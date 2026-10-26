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
//! Rezzer names itself (rulings P5-Y and P5-AC): in the assets folder regular files named as
//! Rezzer names pictures ([`is_asset_name`]), in the parts folder Markdown parts that Rezzer
//! wrote for this source ([`holder`]). A file a user put there stays. An empty parts folder
//! is removed, also one Rezzer did not make. Symlinks and folders inside are never followed
//! or removed.
//!
//! Each Markdown and JSON path is looked at again right before its write, and a path that
//! was free is created exclusively without removing anything: a file that appears after the
//! check (a parallel run, a file of the user's) fails the write and stays.
//!
//! Known gap (planned for the known-issues plan): every path is opened by name below the
//! output folder, so its folders are followed at the moment of each call. Someone who can
//! write to the output folder and swaps the parts or assets folder for a symlink between
//! the folder check and the writes redirects them into the folder the link points at. There
//! new parts are only created (never over a file), and only Rezzer's own parts of
//! the same source are replaced or removed. Pictures are replaced by name, and stale
//! picture names are removed, so a file named like a Rezzer picture (`page-3.png`) in that
//! folder can be overwritten or removed. Closing it needs writes relative to an open folder
//! handle (`openat` with `O_NOFOLLOW`), which the standard library does not offer.
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

use std::collections::HashSet;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;

use rezzer_model::{Blob, Document, Loss, LossReason, Severity};
use rezzer_render::MAIN_NAME_MAX;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::config::one_line;

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
    hex::encode(&Sha256::digest(name.as_bytes())[..4])
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

/// The stem of the parts folder `<stem>.parts/` and the assets folder `<stem>.assets/` of
/// `source_name` (decision 8): [`bundle_name`] of the NFC name, plus `-<name hash>` when
/// `bundle_name` changed the name (also a leading `.`), or when the name ends in `.md`,
/// `.json`, `.assets` or `.parts` (any case). So `a b.pdf` and `a_b.pdf` never share a
/// folder, and the folders of one source never land on a file or folder of another (`x` and
/// `x.assets`, `x` and `x.parts`, `a` with its JSON and `a.json`).
pub fn folder_stem(source_name: &str) -> String {
    let name = last_component(source_name);
    let stem = bundle_name(&name);
    let clash = [".md", ".json", ".assets", ".parts"]
        .iter()
        .any(|suffix| ends_with_ignore_case(&name, suffix));
    if stem != name || clash {
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
/// a leading `.` was replaced by `_` (so Crystalline does not skip it as hidden), or when the
/// name was cut.
///
/// A name is cut to [`MAIN_NAME_MAX`] bytes with the hash, and to the char count of the
/// source's last component (at least 4), so it never holds more than that count plus 9
/// chars: the bound the splitter counts with.
pub fn main_name(source_name: &str) -> String {
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

/// The extensions Rezzer gives pictures ([`asset_extension`]).
const ASSET_EXTENSIONS: [&str; 4] = ["png", "jpg", "pdf", "bin"];

/// Whether `name` is a name Rezzer gives a picture: a safe asset id, `.` and one of
/// [`ASSET_EXTENSIONS`].
fn is_asset_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(id, ext)| safe_asset_id(id) && ASSET_EXTENSIONS.contains(&ext))
}

fn asset_extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "application/pdf" => "pdf",
        _ => "bin",
    }
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
        let file = format!("{}.{}", asset.id, asset_extension(&asset.mime));
        asset.path = Some(format!("{folder}/{file}"));
        planned.push(Planned {
            asset: index,
            file,
            bytes: blob.bytes,
        });
    }
    planned
}

/// Writes the planned pictures to `<dir>/<stem>.assets/`. A picture that cannot be written
/// loses its path and is a loss, never a failed read. Returns the names written, for
/// [`clean_assets`], and whether a loss was added (the Markdown must then be rendered
/// again).
pub(crate) fn write_assets(
    document: &mut Document,
    planned: Vec<Planned>,
    dir: &Path,
    stem: &str,
) -> (HashSet<OsString>, bool) {
    let folder = format!("{stem}.assets");
    let mut written = HashSet::new();
    let mut lost = false;
    let ready = if planned.is_empty() {
        Ok(())
    } else {
        prepare_folder(dir, &folder)
    };
    for p in planned {
        let result = match &ready {
            // Pictures are replaced by name (ruling P5-AC).
            Ok(()) => write_new(&dir.join(&folder).join(&p.file), &p.bytes, true),
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
    (written, lost)
}

/// Removes the pictures an earlier run left in `<dir>/<stem>.assets/` that this run did not
/// write ([`remove_stale`]). Runs last, after every file of the run is written, so a run
/// that fails earlier leaves the old output whole.
pub(crate) fn clean_assets(
    dir: &Path,
    stem: &str,
    written: &HashSet<OsString>,
) -> Result<(), String> {
    let folder = format!("{stem}.assets");
    remove_stale(&dir.join(&folder), written, Stale::Assets).map_err(|e| {
        format!(
            "files of an earlier run in {} were not all removed: {}",
            one_line(&folder),
            one_line(&e.to_string())
        )
    })
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
    /// `(<folder>/<file>.md, markdown)` in order.
    pub parts: Vec<(&'a str, &'a str)>,
    pub json: Option<&'a str>,
}

/// Writes the files of one run into `dir` (made when missing): the parts into `<folder>/`,
/// then the main document and the JSON. Parts of an earlier run that this one does not
/// write are removed, and the parts folder too when it is empty then. A symlink is never
/// written through. Before anything is written, every path is checked ([`check_paths`]).
pub(crate) fn write_files(dir: &Path, files: &Files<'_>) -> Result<(), String> {
    let shown = |path: &Path, e: std::io::Error| {
        format!(
            "cannot write {}: {}",
            path.display(),
            one_line(&e.to_string())
        )
    };
    let folder = dir.join(files.folder);
    check_paths(dir, files)?;
    let source = SourceId::of(files);
    std::fs::create_dir_all(dir).map_err(|e| shown(dir, e))?;
    let mut keep: HashSet<OsString> = HashSet::new();
    if !files.parts.is_empty() {
        prepare_folder(dir, files.folder).map_err(|e| shown(&folder, e))?;
    }
    // Each path is looked at again right before its write: only Rezzer's own output for
    // this source is replaced. A file that appeared since the check (another run at the same
    // time, a file of the user's) makes the exclusive create fail instead of being replaced.
    let put = |target: &Path, bytes: &[u8], what: Target| -> Result<(), String> {
        let replace = decide(target, &source, what)?;
        write_new(target, bytes, replace).map_err(|e| shown(target, e))
    };
    for (path, markdown) in &files.parts {
        let target = dir.join(path);
        put(&target, markdown.as_bytes(), Target::Part)?;
        keep.extend(target.file_name().map(std::ffi::OsStr::to_os_string));
    }
    let main = dir.join(format!("{}.md", files.main));
    put(&main, files.main_markdown.as_bytes(), Target::Main)?;
    if let Some(json) = files.json {
        let target = dir.join(format!("{}.json", files.main));
        let mut text = json.to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        put(&target, text.as_bytes(), Target::Json)?;
    }
    remove_stale(&folder, &keep, Stale::Parts(&source)).map_err(|e| {
        format!(
            "cannot remove the parts of an earlier run in {}: {}",
            folder.display(),
            one_line(&e.to_string())
        )
    })?;
    if keep.is_empty() {
        remove_if_empty(&folder);
    }
    Ok(())
}

/// Checks every path of a run before anything is written (rulings P5-AB and P5-AG): the
/// main document, the JSON when the run writes one, the parts folder (missing or a plain
/// directory) and each part path. Each must be missing or a file Rezzer wrote for this
/// source.
pub(crate) fn check_paths(dir: &Path, files: &Files<'_>) -> Result<(), String> {
    let source = SourceId::of(files);
    let main = dir.join(format!("{}.md", files.main));
    decide(&main, &source, Target::Main)?;
    if files.json.is_some() {
        let json = dir.join(format!("{}.json", files.main));
        decide(&json, &source, Target::Json)?;
    }
    if files.parts.is_empty() {
        return Ok(());
    }
    let folder = dir.join(files.folder);
    match std::fs::symlink_metadata(&folder) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "cannot write the parts: {} exists and is not a plain directory",
                folder.display()
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(format!(
                "cannot write the parts: {}: {}",
                folder.display(),
                one_line(&e.to_string())
            ));
        }
    }
    for (path, _) in &files.parts {
        decide(&dir.join(path), &source, Target::Part)?;
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

/// Whether Rezzer may write `target` for `source` now: `Ok(false)` when nothing is there,
/// `Ok(true)` when it holds Rezzer's own output for this source (so it is replaced), else
/// the message that stops the run.
fn decide(target: &Path, source: &SourceId, what: Target) -> Result<bool, String> {
    let found = match what {
        Target::Main => holder(target, source, Kind::Main),
        Target::Part => holder(target, source, Kind::Part),
        Target::Json => json_holder(target, source),
    };
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

/// Removes `folder` when it is a plain directory and empty. Anything else stays.
fn remove_if_empty(folder: &Path) {
    let plain = std::fs::symlink_metadata(folder).is_ok_and(|m| m.is_dir());
    let empty =
        plain && std::fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_none());
    if empty {
        // A file that lands in it right now makes this fail, which is right.
        let _ = std::fs::remove_dir(folder);
    }
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
/// resolved against the working directory, or when it fails (a file that is gone).
fn canonical(resource: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(resource);
    if !path.is_absolute() {
        return None;
    }
    std::fs::canonicalize(path).ok()
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

/// Who holds the Markdown path `path`, for `source`. The frontmatter is read line by line
/// up to the closing `---`, without following a symlink: a symlink, a folder or a file
/// without Rezzer's `generated` line is [`Holder::Foreign`].
fn holder(path: &Path, source: &SourceId, kind: Kind) -> Holder {
    use std::io::BufRead;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Holder::Foreign,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Holder::Free,
        Err(_) => return Holder::Foreign,
    }
    let Ok(file) = std::fs::File::open(path) else {
        return Holder::Foreign;
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

/// Who holds the JSON path `path`, for `source`. Rezzer's JSON is pretty-printed and starts
/// with `"model_version"`, then the `"source"` object with its `"path"` (when there is one),
/// `"name"` and `"sha256"`; only that shape counts as Rezzer's.
fn json_holder(path: &Path, source: &SourceId) -> Holder {
    use std::io::BufRead;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Holder::Foreign,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Holder::Free,
        Err(_) => return Holder::Foreign,
    }
    let Ok(file) = std::fs::File::open(path) else {
        return Holder::Foreign;
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
/// inside are left alone and never followed. A folder that is missing, or not a plain
/// directory, is left as it is.
fn remove_stale(folder: &Path, keep: &HashSet<OsString>, which: Stale) -> std::io::Result<()> {
    match std::fs::symlink_metadata(folder) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    }
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        // The entry's own type: a symlink is a symlink here, never what it points at.
        if !entry.file_type()?.is_file() || keep.contains(&entry.file_name()) {
            continue;
        }
        let name = entry.file_name();
        let remove = match which {
            Stale::Assets => name.to_str().is_some_and(is_asset_name),
            Stale::Parts(source) => {
                let markdown = Path::new(&name)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"));
                markdown && holder(&entry.path(), source, Kind::Part) == Holder::Same
            }
        };
        if remove {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Makes `<dir>/<folder>`, a real directory: an existing symlink or file there is refused, so
/// nothing lands outside `dir`.
fn prepare_folder(dir: &Path, folder: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(folder);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(std::io::Error::other(format!(
            "{folder} exists and is not a plain directory"
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir(&path),
        Err(e) => Err(e),
    }
}

/// Writes a new file at `path`, created exclusively, so a symlink is never written through.
/// With `replace` an existing entry (also a symlink) is removed first without being
/// followed; without it nothing is removed, and an entry that is there makes the write
/// fail ("it appeared while Rezzer was writing").
fn write_new(path: &Path, bytes: &[u8], replace: bool) -> std::io::Result<()> {
    use std::io::Write;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            return Err(std::io::Error::other("a directory is in the way"));
        }
        Ok(_) if replace => std::fs::remove_file(path)?,
        Ok(_) => {
            return Err(std::io::Error::other(
                "it appeared while Rezzer was writing and is not replaced",
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                std::io::Error::other("it appeared while Rezzer was writing and is not replaced")
            } else {
                e
            }
        })?;
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::{Files, check_paths, write_files, write_new};

    fn files<'a>(main_markdown: &'a str) -> Files<'a> {
        Files {
            source: "/src/notes.txt",
            sha256: "00",
            main: "notes.txt",
            folder: "notes.txt.parts",
            main_markdown,
            parts: Vec::new(),
            json: None,
        }
    }

    /// Without `replace` nothing is removed: an entry that is there fails the write.
    #[test]
    fn a_free_path_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        write_new(&path, b"first", false).unwrap();
        let err = write_new(&path, b"second", false).unwrap_err();
        assert!(err.to_string().contains("appeared"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        write_new(&path, b"third", true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"third");
    }

    /// A file that appears after the check (another run at the same time, a file of the
    /// user's) is not replaced by the write that follows.
    #[test]
    fn a_file_that_appears_after_the_check_stays() {
        let dir = tempfile::tempdir().unwrap();
        let ours = "---\ntype: source\nresource: \"/src/notes.txt\"\ngenerated: { by: rezzer/0.1.0, at: 2026-10-26T00:00:00Z }\n---\n\nnew\n";
        let f = files(ours);
        check_paths(dir.path(), &f).unwrap();
        let main = dir.path().join("notes.txt.md");
        let other = "---\ntype: source\nresource: \"/other/notes.txt\"\ngenerated: { by: rezzer/0.1.0, at: 2026-10-26T00:00:00Z }\nsource_sha256: \"11\"\n---\n\nother\n";
        for appeared in ["# mine\n", other] {
            std::fs::write(&main, appeared).unwrap();
            let err = write_files(dir.path(), &f).unwrap_err();
            assert!(err.contains("notes.txt.md"), "{err}");
            assert_eq!(std::fs::read_to_string(&main).unwrap(), appeared);
        }
        // Rezzer's own output for this source is replaced.
        std::fs::write(&main, ours.replace("new", "old")).unwrap();
        write_files(dir.path(), &f).unwrap();
        assert_eq!(std::fs::read_to_string(&main).unwrap(), ours);
    }
}
