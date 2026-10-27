// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a run with an output directory writes, and the names it writes under (decision 8).
//!
//! For a source named `<name>`:
//! - `<dir>/<main name>.md`, the main document (or the whole document when it fits);
//! - `<dir>/<main name>.json` with the model JSON;
//! - `<dir>/<stem>.parts/` with the parts of a split document (ruling P5-AA);
//! - `<dir>/<stem>.assets/` with the pictures.
//!
//! The suffixes keep both folders off the source's own name, so a source can be written into
//! its own folder (`notes.txt` next to `notes.txt.md`, `notes.txt.parts/` and
//! `notes.txt.assets/`).
//!
//! Every name comes from the last path component of the source name in Unicode NFC (ruling
//! P5-W), so a name that macOS gives in NFD gets the same files and links as its NFC form.
//! The name hash is the first 8 hex of the SHA-256 of that NFC name, not of the content, so
//! an edited file keeps its addresses. No name starts with `.` (ruling P5-Z): Crystalline
//! skips hidden files.
//!
//! On Windows the main name and the folder stem are made legal there ([`legal_name`], Jordi,
//! 2026-10-27): each of `: < > " | ? *` and the control characters, each trailing dot or
//! space, and a device name before the first `.` (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to
//! `COM9`, `LPT1` to `LPT9`, any case) become `_`, and a changed name gets the name hash of
//! the original name, so `a:b.txt` and `a?b.txt` never share a file. On Unix and macOS the
//! names stay as they are. The frontmatter's `resource` keeps the original name either way.
//!
//! Rezzer replaces a Markdown file or JSON only when it wrote it for the same source (ruling
//! P5-AG): its frontmatter has `generated: { by: rezzer/` (a part also its `part:` key), or
//! the JSON starts with `model_version`; and it names the same source. The same source is the
//! same `resource`, the same canonical path (symlinks, `..` and letter case resolved by
//! `fs::canonicalize`), or the same `source_sha256` (a moved folder, unchanged content). A
//! main document, JSON or part path held by anything else, a file of the user's or the
//! output of another source with other content (`a/report.pdf` and `b/report.pdf`, or
//! `Note.md` and `note.md` on a file system that ignores case), fails the run before a file
//! of it is written (rulings P5-AB and P5-AG). The message names the file and, for another
//! source, both sources.
//!
//! Files an earlier run left that this run does not write are removed, but only files
//! Rezzer names itself (rulings P5-Y, P5-AC and P5-AK): in the assets folder regular files
//! named as Rezzer names pictures and attachments ([`is_asset_name`]), in the parts folder
//! Markdown parts that Rezzer wrote for this source ([`holder`]). A file a user put there
//! stays. An empty parts folder is removed, also one Rezzer did not make. Symlinks and
//! folders inside are never followed or removed.
//!
//! Each Markdown and JSON path is looked at again right before its write, and a path that
//! was free is created exclusively without removing anything: a file that appears after the
//! check (a parallel run, a file of the user's) fails the write and stays.
//!
//! On Unix every write, read and removal under the output folder goes through a handle of
//! the folder opened once, without following a symlink ([`Folder`]): the output folder is
//! opened once per run, and each write, decision and removal in the parts or assets folder
//! goes through one handle of that folder. Someone who can write to the output folder and
//! swaps the parts or assets folder for a symlink or another folder after it was opened
//! redirects nothing: the run writes into the folder it opened, and the cleanup removes only
//! there. The output folder itself may be a symlink the user chose. Known Windows limit: on
//! other systems each call still opens its path by name, so such a swap between the check
//! and a call can redirect it there.
//!
//! The assets folder is found by its name only, not by its source, so a name that looks
//! like a hashed stem (it ends in `-` and 8 hex digits, in any case) gets its own name hash
//! too ([`folder_stem`]): `a_b.pdf-<hash>` next to `a b.pdf` gets `a_b.pdf-<hash>-<hash>`,
//! and neither run removes the other one's pictures as stale, also with the hash in upper
//! case on a file system that ignores case. Two sources still share a folder when their
//! names give the same 32-bit name hash, which takes a collision made on purpose, as for
//! every hashed name. That is accepted.
//!
//! Order: the pictures are planned while the document is read, because the Markdown links
//! name them. After the render the part paths are checked, so a run that fails there has
//! written and removed nothing. Then the pictures, the parts, the main document and the
//! JSON are written; the stale parts and the stale pictures go last. A write error on the
//! way leaves the new files written so far and keeps every old file, so the old links of an
//! earlier run still resolve; the next run that works cleans up. A failed read removes no
//! old picture. When a picture cannot be written, the Markdown is rendered again; should
//! the new render name a part path that the check then refuses, the run fails with the
//! pictures already written: only pictures of the same name were replaced, nothing is lost.

use std::borrow::Cow;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;

use rezzer_model::{Blob, Document, Loss, LossReason, Severity};
use rezzer_render::MAIN_NAME_MAX;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::config::{CacheOs, one_line};
use crate::folder::{EntryKind, Folder};
use crate::names::legal_name;

/// The longest bundle name, in bytes.
const BUNDLE_NAME_MAX: usize = 120;

/// `-` and 8 hex.
const HASH_LEN: usize = 9;

/// The last path component of `source_name` (split at `/` and a backslash) in NFC.
fn last_component(source_name: &str) -> String {
    source_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .nfc()
        .collect()
}

/// The first 8 hex of the SHA-256 of `name`.
fn name_hash(name: &str) -> String {
    rezzer_model::text::hex(&Sha256::digest(name.as_bytes())[..4])
}

/// The directory name stem for the assets of `source_name`: the last path component (split at
/// `/` and a backslash), every char outside `[A-Za-z0-9._-]` replaced by `_`, a leading `.`
/// replaced by `_`, `file` when empty, at most 120 bytes. It never holds a separator, so the bundle
/// stays inside the output directory.
pub fn bundle_name(source_name: &str) -> String {
    let last = source_name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut name: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(BUNDLE_NAME_MAX)
        .collect();
    if name.starts_with('.') {
        name.replace_range(..1, "_");
    }
    if name.is_empty() {
        name.push_str("file");
    }
    name
}

/// Whether `name` ends in `suffix`, ignoring ASCII case.
fn ends_with_ignore_case(name: &str, suffix: &str) -> bool {
    name.len() >= suffix.len()
        && name.is_char_boundary(name.len() - suffix.len())
        && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// Whether `name` looks like a hashed stem: it ends in `-` and 8 ASCII hex digits, in any
/// case.
fn looks_hashed(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= HASH_LEN
        && bytes[bytes.len() - HASH_LEN] == b'-'
        && bytes[bytes.len() - HASH_LEN + 1..]
            .iter()
            .all(u8::is_ascii_hexdigit)
}

/// The stem of the parts folder `<stem>.parts/` and the assets folder `<stem>.assets/` of
/// `source_name` (decision 8): [`bundle_name`] of the NFC name, plus `-<name hash>` when
/// `bundle_name` changed the name (also a leading `.`), when the name ends in `.md`,
/// `.json`, `.assets` or `.parts` (any case), or when it looks like a hashed stem (ends in
/// `-` and 8 hex digits, any case), or, on Windows, when the stem had to be made legal there
/// (a trailing dot or a device name such as `con` became `_`). So `a b.pdf` and `a_b.pdf`
/// never share a folder, nor `a b.pdf` and a source named after its stem `a_b.pdf-<hash>`,
/// and the folders of one source never land on a file or folder of another (`x` and
/// `x.assets`, `x` and `x.parts`, `a` with its JSON and `a.json`).
pub fn folder_stem(source_name: &str) -> String {
    folder_stem_on(source_name, CacheOs::current())
}

/// [`folder_stem`] as written on `os`: on Windows the stem is made legal there first
/// ([`legal_name`]), and a stem that changed gets the name hash too.
pub(crate) fn folder_stem_on(source_name: &str, os: CacheOs) -> String {
    let name = last_component(source_name);
    let stem = legal_name(&bundle_name(&name), os).into_owned();
    let clash = [".md", ".json", ".assets", ".parts"]
        .iter()
        .any(|suffix| ends_with_ignore_case(&name, suffix));
    if stem != name || clash || looks_hashed(&name) {
        format!("{stem}-{}", name_hash(&name))
    } else {
        stem
    }
}

/// The parts folder for a [`folder_stem`]: `<stem>.parts`.
pub fn parts_folder(stem: &str) -> String {
    format!("{stem}.parts")
}

/// Whether Crystalline would read `name` as one of its reserved names: `index` or `log`
/// before the first `.`, in any case.
fn reserved(name: &str) -> bool {
    let first = name.split('.').next().unwrap_or_default();
    first.eq_ignore_ascii_case("index") || first.eq_ignore_ascii_case("log")
}

/// The name of the main document without `.md` (decision 8): the last path component of
/// `source_name` in NFC, so `budget.xlsx` gives `budget.xlsx.md`. `.`, `..` or an empty name
/// become `file`. `-<name hash>` is added once when the name holds no ASCII letter or digit,
/// when it is `index` or `log` before the first `.` (any case), when a control character or
/// a leading `.` was replaced by `_` (so Crystalline does not skip it as hidden), when the
/// name was cut, or, on Windows, when it had to be made legal there (a character Windows
/// refuses, a trailing dot or space, or a device name such as `CON` became `_`).
///
/// A name is cut to [`MAIN_NAME_MAX`] bytes with the hash, and to the char count of the
/// source's last component (at least 4), so it never holds more than that count plus 9
/// chars: the bound the splitter counts with.
pub fn main_name(source_name: &str) -> String {
    main_name_on(source_name, CacheOs::current())
}

/// [`main_name`] as written on `os`: on Windows the name is made legal there
/// ([`legal_name`]), and a name that changed gets the name hash.
pub(crate) fn main_name_on(source_name: &str, os: CacheOs) -> String {
    let given = source_name.rsplit(['/', '\\']).next().unwrap_or_default();
    let nfc = last_component(source_name);
    let mut hashed = false;
    let mut name: String = nfc
        .chars()
        .map(|c| {
            if c.is_control() {
                hashed = true;
                '_'
            } else {
                c
            }
        })
        .collect();
    if matches!(name.as_str(), "" | "." | "..") {
        name = "file".to_string();
    }
    if name.starts_with('.') {
        name.replace_range(..1, "_");
        hashed = true;
    }
    if let Cow::Owned(legal) = legal_name(&name, os) {
        name = legal;
        hashed = true;
    }
    if !name.bytes().any(|b| b.is_ascii_alphanumeric()) || reserved(&name) {
        hashed = true;
    }
    let max_chars = given.chars().count().max(4);
    let max_bytes = if hashed {
        MAIN_NAME_MAX - HASH_LEN
    } else {
        MAIN_NAME_MAX
    };
    if name.chars().count() > max_chars || name.len() > max_bytes {
        let max_bytes = MAIN_NAME_MAX - HASH_LEN;
        let mut end = 0;
        for (count, (i, c)) in name.char_indices().enumerate() {
            if count == max_chars || i + c.len_utf8() > max_bytes {
                break;
            }
            end = i + c.len_utf8();
        }
        name.truncate(end);
        hashed = true;
    }
    if hashed {
        name.push('-');
        name.push_str(&name_hash(&nfc));
    }
    name
}

/// Whether `id` is safe as a file name stem: `^[a-z0-9-]{1,64}$`.
fn safe_asset_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The media types whose files Rezzer writes under their own extension (ruling P5-AJ), each
/// one Crystalline 0.29.1 copies on import (its `attachment_mime` list). Any other picture
/// or attachment is written as [`OTHER_EXTENSION`].
const ASSET_TYPES: [(&str, &str); 13] = [
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
    ("application/pdf", "pdf"),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "docx",
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xlsx",
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pptx",
    ),
    ("application/vnd.oasis.opendocument.text", "odt"),
    ("application/vnd.oasis.opendocument.spreadsheet", "ods"),
    ("application/vnd.oasis.opendocument.presentation", "odp"),
    ("application/json", "json"),
    ("text/plain", "txt"),
];

/// The extension of a picture or attachment whose type its bytes do not confirm.
const OTHER_EXTENSION: &str = "bin";

/// Whether `name` is a name Rezzer gives a picture or attachment (ruling P5-AK): an id of a
/// shape the readers make, `page-<n>`, `page-<n>-pic-<m>`, `page-<n>-code-<m>` or
/// `attachment-<n>`, then `.` and an extension [`asset_extension`] gives. Only such files are
/// removed as stale, so a user's `logo.png` in the folder stays.
fn is_asset_name(name: &str) -> bool {
    let number = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    let own_id = |id: &str| {
        if let Some(n) = id.strip_prefix("attachment-") {
            return number(n);
        }
        let Some(rest) = id.strip_prefix("page-") else {
            return false;
        };
        match rest.split_once('-') {
            None => number(rest),
            Some((n, inner)) => {
                number(n)
                    && ["pic-", "code-"]
                        .iter()
                        .any(|p| inner.strip_prefix(p).is_some_and(number))
            }
        }
    };
    name.rsplit_once('.').is_some_and(|(id, ext)| {
        own_id(id) && (ext == OTHER_EXTENSION || ASSET_TYPES.iter().any(|(_, e)| *e == ext))
    })
}

/// The extension of a picture or attachment, from what its bytes confirm (ruling P5-AJ):
/// a PNG attachment named `image.png` gets `png`, a file named `x.png` with other bytes
/// gets `bin`. Neither the name nor the declared type counts.
fn asset_extension(bytes: &[u8]) -> &'static str {
    rezzer_detect::confirmed_media_type(bytes)
        .and_then(|mime| ASSET_TYPES.iter().find(|(m, _)| *m == mime))
        .map_or(OTHER_EXTENSION, |(_, ext)| ext)
}

/// A picture to write: its file name in `<stem>.assets/` and its bytes.
pub(crate) struct Planned {
    asset: usize,
    file: String,
    bytes: Vec<u8>,
}

/// Plans the pictures of a run without writing them: each blob of a known asset with a safe
/// id gets the path `<stem>.assets/<asset id>.<ext>` (relative to the output directory),
/// so the Markdown can link it. A bad asset id is a loss and keeps no path; a blob without
/// an asset is dropped. [`write_assets`] writes them once the part paths are checked.
pub(crate) fn plan_assets(document: &mut Document, blobs: Vec<Blob>, stem: &str) -> Vec<Planned> {
    let folder = format!("{stem}.assets");
    let mut planned = Vec::new();
    for blob in blobs {
        let Some(index) = document.assets.iter().position(|a| a.id == blob.asset) else {
            continue;
        };
        let asset = &mut document.assets[index];
        if !safe_asset_id(&asset.id) {
            let location = format!("asset {}", one_line(&asset.id));
            document.losses.push(Loss {
                location,
                what: "the picture was not written: its id is not a safe file name".into(),
                why: LossReason::Internal,
                severity: Severity::Warning,
            });
            continue;
        }
        let file = format!("{}.{}", asset.id, asset_extension(&blob.bytes));
        asset.path = Some(format!("{folder}/{file}"));
        planned.push(Planned {
            asset: index,
            file,
            bytes: blob.bytes,
        });
    }
    planned
}

/// Writes the planned pictures to `<dir>/<stem>.assets/`, made when missing (a symlink or a
/// file there is refused). A picture that cannot be written loses its path and is a loss,
/// never a failed read. Returns the names written and the assets folder it opened, for
/// [`clean_assets`], and whether a loss was added (the Markdown must then be rendered again).
pub(crate) fn write_assets(
    document: &mut Document,
    planned: Vec<Planned>,
    dir: &Folder,
    stem: &str,
) -> (HashSet<OsString>, bool, Option<Folder>) {
    let mut written = HashSet::new();
    let mut lost = false;
    if planned.is_empty() {
        return (written, lost, None);
    }
    let assets = dir.sub(&format!("{stem}.assets"), true);
    for p in planned {
        let result = match &assets {
            // Pictures are replaced by name (ruling P5-AC).
            Ok(assets) => write_new(assets, &p.file, &p.bytes, true),
            Err(e) => Err(std::io::Error::new(e.kind(), e.to_string())),
        };
        match result {
            Ok(()) => {
                written.insert(OsString::from(&p.file));
            }
            Err(e) => {
                let asset = &mut document.assets[p.asset];
                asset.path = None;
                let location = format!("asset {}", one_line(&asset.id));
                document.losses.push(Loss {
                    location,
                    what: format!("the picture was not written: {}", one_line(&e.to_string())),
                    why: LossReason::Internal,
                    severity: Severity::Warning,
                });
                lost = true;
            }
        }
    }
    (written, lost, assets.ok())
}

/// Removes the pictures an earlier run left in `<dir>/<stem>.assets/` that this run did not
/// write ([`remove_stale`]), through `assets`, the folder [`write_assets`] opened. When the
/// run wrote no picture the folder is opened here; a missing one, or one that is not a plain
/// directory, is left as it is. Runs last, after every file of the run is written, so a run
/// that fails earlier leaves the old output whole.
pub(crate) fn clean_assets(
    dir: &Folder,
    assets: Option<&Folder>,
    stem: &str,
    written: &HashSet<OsString>,
) -> Result<(), String> {
    let folder = format!("{stem}.assets");
    let shown = |e: std::io::Error| {
        format!(
            "files of an earlier run in {} were not all removed: {}",
            one_line(&folder),
            one_line(&e.to_string())
        )
    };
    let opened;
    let assets = match assets {
        Some(assets) => assets,
        None => match dir.sub(&folder, false) {
            Ok(found) => {
                opened = found;
                &opened
            }
            Err(e) if is_absent(&e) => return Ok(()),
            Err(e) => return Err(shown(e)),
        },
    };
    remove_stale(assets, written, Stale::Assets).map_err(shown)
}

/// Whether `e`, from [`Folder::sub`] without `create`, says there is no plain folder to clean:
/// it is missing, or a symlink or a file is there.
fn is_absent(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

/// What the files of one run are: the main document, its parts and the JSON.
pub(crate) struct Files<'a> {
    /// The source as the frontmatter's `resource` names it, unquoted: its path, else its
    /// name.
    pub source: &'a str,
    /// The source's SHA-256 in hex, as `source_sha256` holds it.
    pub sha256: &'a str,
    /// The main document's name without `.md` ([`main_name`]).
    pub main: &'a str,
    /// The parts folder ([`parts_folder`]).
    pub folder: &'a str,
    pub main_markdown: &'a str,
    /// `(<folder>/<file>.md, markdown)` in order ([`part_file`] cuts the file name).
    pub parts: Vec<(&'a str, &'a str)>,
    pub json: Option<&'a str>,
}

/// The file name of the part path `path` (`<folder>/<file>.md`): the one component the parts
/// folder handle takes.
fn part_file<'a>(files: &Files<'_>, path: &'a str) -> Result<&'a str, String> {
    path.strip_prefix(files.folder)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| {
            format!(
                "cannot write the parts: {} is not in {}",
                one_line(path),
                one_line(files.folder)
            )
        })
}

/// Writes the files of one run into `dir`: the parts into `<folder>/` (made when missing,
/// opened once), then the main document and the JSON. Parts of an earlier run that this one
/// does not write are removed, and the parts folder too when it is empty then. A symlink is
/// never written through. Before anything is written, every path is checked
/// ([`check_paths`]).
pub(crate) fn write_files(dir: &Folder, files: &Files<'_>) -> Result<(), String> {
    let shown = |path: &Path, e: std::io::Error| {
        format!(
            "cannot write {}: {}",
            path.display(),
            one_line(&e.to_string())
        )
    };
    let folder = dir.path().join(files.folder);
    check_paths(Some(dir), files)?;
    let source = SourceId::of(files);
    let parts = if files.parts.is_empty() {
        // Nothing to write: only an existing plain folder is cleaned.
        match dir.sub(files.folder, false) {
            Ok(parts) => Some(parts),
            Err(e) if is_absent(&e) => None,
            Err(e) => return Err(shown(&folder, e)),
        }
    } else {
        Some(dir.sub(files.folder, true).map_err(|e| shown(&folder, e))?)
    };
    // Each path is looked at again right before its write: only Rezzer's own output for
    // this source is replaced. A file that appeared since the check (another run at the same
    // time, a file of the user's) makes the exclusive create fail instead of being replaced.
    let put = |at: &Folder, name: &str, bytes: &[u8], what: Target| -> Result<(), String> {
        let replace = decide(at, OsStr::new(name), &source, what)?;
        write_new(at, name, bytes, replace).map_err(|e| shown(&at.path().join(name), e))
    };
    let mut keep: HashSet<OsString> = HashSet::new();
    if let Some(parts) = &parts {
        for (path, markdown) in &files.parts {
            let name = part_file(files, path)?;
            put(parts, name, markdown.as_bytes(), Target::Part)?;
            keep.insert(OsString::from(name));
        }
    }
    let main = format!("{}.md", files.main);
    put(dir, &main, files.main_markdown.as_bytes(), Target::Main)?;
    if let Some(json) = files.json {
        let mut text = json.to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        put(
            dir,
            &format!("{}.json", files.main),
            text.as_bytes(),
            Target::Json,
        )?;
    }
    if let Some(parts) = &parts {
        remove_stale(parts, &keep, Stale::Parts(&source)).map_err(|e| {
            format!(
                "cannot remove the parts of an earlier run in {}: {}",
                folder.display(),
                one_line(&e.to_string())
            )
        })?;
    }
    if keep.is_empty() {
        remove_if_empty(dir, files.folder);
    }
    Ok(())
}

/// Checks every path of a run before anything is written (rulings P5-AB and P5-AG): the
/// main document, the JSON when the run writes one, the parts folder (missing or a plain
/// directory) and each part path. Each must be missing or a file Rezzer wrote for this
/// source. `dir` is `None` when the output folder is missing: then every path is free.
pub(crate) fn check_paths(dir: Option<&Folder>, files: &Files<'_>) -> Result<(), String> {
    let Some(dir) = dir else {
        return Ok(());
    };
    let source = SourceId::of(files);
    decide(
        dir,
        OsStr::new(&format!("{}.md", files.main)),
        &source,
        Target::Main,
    )?;
    if files.json.is_some() {
        let json = format!("{}.json", files.main);
        decide(dir, OsStr::new(&json), &source, Target::Json)?;
    }
    if files.parts.is_empty() {
        return Ok(());
    }
    let folder = dir.path().join(files.folder);
    let parts = match dir.sub(files.folder, false) {
        Ok(parts) => parts,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => {
            return Err(format!(
                "cannot write the parts: {} exists and is not a plain directory",
                folder.display()
            ));
        }
        Err(e) => {
            return Err(format!(
                "cannot write the parts: {}: {}",
                folder.display(),
                one_line(&e.to_string())
            ));
        }
    };
    for (path, _) in &files.parts {
        let name = part_file(files, path)?;
        decide(&parts, OsStr::new(name), &source, Target::Part)?;
    }
    Ok(())
}

/// What a path of a run is: the main document, the JSON or a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Main,
    Json,
    Part,
}

/// Whether Rezzer may write `name` in `folder` for `source` now: `Ok(false)` when nothing is
/// there, `Ok(true)` when it holds Rezzer's own output for this source (so it is replaced),
/// else the message that stops the run.
fn decide(folder: &Folder, name: &OsStr, source: &SourceId, what: Target) -> Result<bool, String> {
    let found = match what {
        Target::Main => holder(folder, name, source, Kind::Main),
        Target::Part => holder(folder, name, source, Kind::Part),
        Target::Json => json_holder(folder, name, source),
    };
    let target = folder.path().join(name);
    match found {
        Holder::Free => Ok(false),
        Holder::Same => Ok(true),
        Holder::Foreign if what == Target::Part => Err(format!(
            "cannot write the parts: {} is in the way and is not a part Rezzer wrote",
            target.display()
        )),
        Holder::Foreign => Err(format!(
            "cannot write {}: it is in the way and is not a file Rezzer wrote",
            target.display()
        )),
        Holder::Other(other) => Err(format!(
            "cannot write {}: it holds Rezzer's output for {other}, and this read is of \
             {}. Sources with the same name, or names that differ only in letter case on a \
             file system that ignores case, need different output folders",
            target.display(),
            source.resource
        )),
    }
}

/// Removes the folder `name` in `dir` when it is a plain directory and empty. Anything else
/// stays: removing a folder fails for a symlink or a folder with entries, also one that a
/// file lands in right now, which is right.
fn remove_if_empty(dir: &Folder, name: &str) {
    let _ = dir.remove_dir(name);
}

/// Which files [`remove_stale`] may remove.
#[derive(Debug, Clone, Copy)]
enum Stale<'a> {
    /// An assets folder: only files named as Rezzer names pictures, never Markdown (a parts
    /// folder of another source may have the same name) or a file a user put there.
    Assets,
    /// A parts folder: only Markdown parts Rezzer wrote for this source, so a folder of the
    /// user's own notes, or the parts of another source, lose nothing.
    Parts(&'a SourceId),
}

/// The most bytes read to find the frontmatter keys of a Markdown file, or the source of a
/// JSON file. Rezzer's own frontmatter can be long (keyword tags, file tags), so this is
/// generous; it only bounds a hostile file.
const HEAD_MAX: u64 = 16 * 1024 * 1024;

/// What tells one source from another in a file Rezzer wrote (ruling P5-AG).
#[derive(Debug)]
pub(crate) struct SourceId {
    /// The quoted `resource` value, as the frontmatter writes it.
    resource: String,
    /// The canonical form of an absolute source path; `None` for a name (bytes, a `data:`
    /// URI) or a path that cannot be resolved.
    canonical: Option<std::path::PathBuf>,
    /// The SHA-256 in hex.
    sha256: String,
}

impl SourceId {
    fn of(files: &Files<'_>) -> SourceId {
        SourceId {
            resource: rezzer_render::frontmatter_value(files.source),
            canonical: canonical(files.source),
            sha256: files.sha256.to_string(),
        }
    }

    /// Whether a file written for the unquoted `resource` with `sha256` is of this source:
    /// the same resource, the same canonical path or the same content.
    fn same(&self, resource: &str, sha256: Option<&str>) -> bool {
        rezzer_render::frontmatter_value(resource) == self.resource
            || (self.canonical.is_some() && canonical(resource) == self.canonical)
            || sha256.is_some_and(|h| !h.is_empty() && h == self.sha256)
    }
}

/// `fs::canonicalize` of an absolute path; `None` for a relative name, which must not be
/// resolved against the working directory, or when it fails (a file that is gone). Like
/// [`crate::canonical_path`], without the Windows verbatim prefix where that is safe.
fn canonical(resource: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(resource);
    if !path.is_absolute() {
        return None;
    }
    std::fs::canonicalize(path).ok().map(crate::strip_verbatim)
}

/// A frontmatter value as Rezzer quotes it, unquoted again; `None` for any other shape.
fn unquoted(value: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(value).ok()?;
    if !text.starts_with('"') {
        return None;
    }
    serde_json::from_str(text).ok()
}

/// Who holds a path Rezzer wants to write.
#[derive(Debug, PartialEq, Eq)]
enum Holder {
    /// Nothing is there.
    Free,
    /// A file Rezzer wrote for the same source.
    Same,
    /// A file Rezzer wrote for another source, named by its quoted `resource`.
    Other(String),
    /// Anything else: a file of the user's, a symlink, a folder.
    Foreign,
}

/// Which Markdown file [`holder`] looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A main document: no `part:` key.
    Main,
    /// A part: a `part:` key.
    Part,
}

/// The regular file `name` in `folder`, opened without following a symlink, else who holds
/// the path: [`Holder::Free`] when nothing is there, [`Holder::Foreign`] for anything else.
fn regular(folder: &Folder, name: &OsStr) -> Result<std::fs::File, Holder> {
    match folder.kind(name) {
        Ok(None) => return Err(Holder::Free),
        Ok(Some(EntryKind::File)) => {}
        Ok(Some(_)) | Err(_) => return Err(Holder::Foreign),
    }
    match folder.open_file(name) {
        Ok(Some(file)) => Ok(file),
        Ok(None) | Err(_) => Err(Holder::Foreign),
    }
}

/// Who holds the Markdown file `name` in `folder`, for `source`. The frontmatter is read line
/// by line up to the closing `---`, without following a symlink: a symlink, a folder or a
/// file without Rezzer's `generated` line is [`Holder::Foreign`].
fn holder(folder: &Folder, name: &OsStr, source: &SourceId, kind: Kind) -> Holder {
    use std::io::BufRead;
    let file = match regular(folder, name) {
        Ok(file) => file,
        Err(found) => return found,
    };
    let mut lines = std::io::BufReader::new(file.take(HEAD_MAX)).split(b'\n');
    if !matches!(lines.next(), Some(Ok(first)) if first == b"---") {
        return Holder::Foreign;
    }
    let (mut part, mut generated) = (false, false);
    let (mut written_for, mut sha256): (Option<Vec<u8>>, Option<String>) = (None, None);
    for line in lines {
        let Ok(line) = line else {
            return Holder::Foreign;
        };
        if line == b"---" {
            break;
        }
        part |= line.starts_with(b"part: ");
        generated |= line.starts_with(b"generated: { by: rezzer/");
        if written_for.is_none()
            && let Some(value) = line.strip_prefix(b"resource: ")
        {
            written_for = Some(value.to_vec());
        }
        if sha256.is_none()
            && let Some(value) = line.strip_prefix(b"source_sha256: ")
        {
            sha256 = unquoted(value);
        }
    }
    match written_for {
        Some(value) if generated && part == (kind == Kind::Part) => {
            let same = match unquoted(&value) {
                Some(resource) => source.same(&resource, sha256.as_deref()),
                None => false,
            };
            if same {
                Holder::Same
            } else {
                Holder::Other(String::from_utf8_lossy(&value).into_owned())
            }
        }
        _ => Holder::Foreign,
    }
}

/// Who holds the JSON file `name` in `folder`, for `source`. Rezzer's JSON is pretty-printed
/// and starts with `"model_version"`, then the `"source"` object with its `"path"` (when
/// there is one), `"name"` and `"sha256"`; only that shape counts as Rezzer's.
fn json_holder(folder: &Folder, name: &OsStr, source: &SourceId) -> Holder {
    use std::io::BufRead;
    let file = match regular(folder, name) {
        Ok(file) => file,
        Err(found) => return found,
    };
    let mut lines = std::io::BufReader::new(file.take(HEAD_MAX)).lines();
    let mut next = || lines.next().and_then(Result::ok);
    if next().as_deref() != Some("{")
        || !next().is_some_and(|l| l.starts_with("  \"model_version\": "))
        || next().as_deref() != Some("  \"source\": {")
    {
        return Holder::Foreign;
    }
    // A JSON string value of a `"key": value,` line inside the source object.
    let value = |line: &str, key: &str| -> Option<String> {
        let rest = line.strip_prefix(&format!("    \"{key}\": "))?;
        serde_json::from_str::<String>(rest.strip_suffix(',').unwrap_or(rest)).ok()
    };
    let (mut path_value, mut name_value, mut sha256) = (None, None, None);
    while let Some(line) = next() {
        if !line.starts_with("    ") {
            break;
        }
        if path_value.is_none() && name_value.is_none() {
            path_value = value(&line, "path");
        }
        if name_value.is_none() {
            name_value = value(&line, "name");
        }
        if sha256.is_none() {
            sha256 = value(&line, "sha256");
        }
    }
    match path_value.or(name_value) {
        Some(written_for) if source.same(&written_for, sha256.as_deref()) => Holder::Same,
        Some(written_for) => Holder::Other(rezzer_render::frontmatter_value(&written_for)),
        None => Holder::Foreign,
    }
}

/// Removes the regular files in `folder` whose names are not in `keep` and that `which`
/// allows: what an earlier run wrote there and this one does not. Symlinks and folders
/// inside are left alone and never followed.
fn remove_stale(folder: &Folder, keep: &HashSet<OsString>, which: Stale) -> std::io::Result<()> {
    for (name, kind) in folder.entries()? {
        // The entry's own kind: a symlink is a symlink here, never what it points at.
        if kind != EntryKind::File || keep.contains(&name) {
            continue;
        }
        let remove = match which {
            Stale::Assets => name.to_str().is_some_and(is_asset_name),
            Stale::Parts(source) => {
                let markdown = Path::new(&name)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"));
                markdown && holder(folder, &name, source, Kind::Part) == Holder::Same
            }
        };
        if remove {
            folder.remove_file(&name)?;
        }
    }
    Ok(())
}

/// Writes a new file `name` in `folder`, created exclusively, so a symlink is never written
/// through. With `replace` an existing entry (also a symlink) is removed first without being
/// followed; without it nothing is removed, and an entry that is there makes the write fail
/// ("it appeared while Rezzer was writing").
fn write_new(folder: &Folder, name: &str, bytes: &[u8], replace: bool) -> std::io::Result<()> {
    match folder.kind(OsStr::new(name))? {
        Some(EntryKind::Dir) => {
            return Err(std::io::Error::other("a directory is in the way"));
        }
        Some(_) if replace => folder.remove_file(OsStr::new(name))?,
        Some(_) => {
            return Err(std::io::Error::other(
                "it appeared while Rezzer was writing and is not replaced",
            ));
        }
        None => {}
    }
    folder.create_new(name, bytes)
}

#[cfg(test)]
mod tests;
