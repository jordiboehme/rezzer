// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a run with an output directory writes, and the names it writes under (decision 8).
//!
//! For a source named `<name>`:
//! - `<dir>/<main name>.md`, the main document (or the whole document when it fits);
//! - `<dir>/<main name>.json` with the model JSON;
//! - `<dir>/<stem>/` with the parts of a split document;
//! - `<dir>/<stem>.assets/` with the pictures.
//!
//! Every name comes from the last path component of the source name in Unicode NFC (ruling
//! P5-W), so a name that macOS gives in NFD gets the same files and links as its NFC form.
//! The name hash is the first 8 hex of the SHA-256 of that NFC name, not of the content, so
//! an edited file keeps its addresses.
//!
//! Files an earlier run left that this run does not write are removed: in the assets folder
//! every regular file but Markdown, in the parts folder only Markdown parts that Rezzer wrote
//! (a `part:` key in the frontmatter). Symlinks and folders are never followed or removed.
//! The check of a folder and the removal of its files are separate steps, so a folder that
//! is swapped for a symlink in between is a known gap (planned for the known-issues plan).

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
/// `/` and a backslash), every char outside `[A-Za-z0-9._-]` replaced by `_`, a leading `.` replaced
/// by `_`, `file` when empty, at most 120 bytes. It never holds a separator, so the bundle
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

/// The stem of the parts folder `<stem>/` and the assets folder `<stem>.assets/` of
/// `source_name` (decision 8): [`bundle_name`] of the NFC name, plus `-<name hash>` when
/// `bundle_name` changed the name, or when the name ends in `.md`, `.json` or `.assets`
/// (any case). So `a b.pdf` and `a_b.pdf` never share a folder, and the folders of one
/// source never land on a file or folder of another (`a` and `a.md`, `x` and `x.assets`,
/// `a` with its JSON and `a.json`).
pub fn folder_stem(source_name: &str) -> String {
    let name = last_component(source_name);
    let stem = bundle_name(&name);
    let clash = [".md", ".json", ".assets"]
        .iter()
        .any(|suffix| ends_with_ignore_case(&name, suffix));
    if stem != name || clash {
        format!("{stem}-{}", name_hash(&name))
    } else {
        stem
    }
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
/// when it is `index` or `log` before the first `.` (any case), when a control character
/// was replaced by `_`, or when the name was cut.
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

fn asset_extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "application/pdf" => "pdf",
        _ => "bin",
    }
}

/// Writes each blob to `<dir>/<stem>.assets/<asset id>.<ext>` and sets the asset's path
/// (relative to `dir`). A bad asset id or a write error is a loss, never a failed read; an
/// asset without a written blob keeps no path. A blob without an asset is dropped. Files an
/// earlier run left in the folder that this run does not write are removed (see
/// [`remove_stale`]), so the folder holds what the Markdown names.
pub(crate) fn write_bundle(document: &mut Document, blobs: Vec<Blob>, dir: &Path, stem: &str) {
    let folder = format!("{stem}.assets");
    let mut losses = Vec::new();
    let mut written_names: HashSet<OsString> = HashSet::new();
    let folder_ready = if blobs.is_empty() {
        Ok(())
    } else {
        prepare_folder(dir, &folder)
    };
    for blob in blobs {
        let Some(asset) = document.assets.iter_mut().find(|a| a.id == blob.asset) else {
            continue;
        };
        let location = format!("asset {}", one_line(&asset.id));
        if !safe_asset_id(&asset.id) {
            losses.push(Loss {
                location,
                what: "the picture was not written: its id is not a safe file name".into(),
                why: LossReason::Internal,
                severity: Severity::Warning,
            });
            continue;
        }
        let relative = format!("{folder}/{}.{}", asset.id, asset_extension(&asset.mime));
        let target = dir.join(&relative);
        let written = match &folder_ready {
            Ok(()) => write_new(&target, &blob.bytes),
            Err(e) => Err(std::io::Error::new(e.kind(), e.to_string())),
        };
        let name = target.file_name().map(std::ffi::OsStr::to_os_string);
        match written {
            Ok(()) => {
                asset.path = Some(relative);
                written_names.extend(name);
            }
            Err(e) => losses.push(Loss {
                location,
                what: format!("the picture was not written: {}", one_line(&e.to_string())),
                why: LossReason::Internal,
                severity: Severity::Warning,
            }),
        }
    }
    if folder_ready.is_ok()
        && let Err(e) = remove_stale(&dir.join(&folder), &written_names, Stale::Assets)
    {
        losses.push(Loss {
            location: "assets".into(),
            what: format!(
                "files of an earlier run in {} were not all removed: {}",
                one_line(&folder),
                one_line(&e.to_string())
            ),
            why: LossReason::Internal,
            severity: Severity::Warning,
        });
    }
    document.losses.extend(losses);
}

/// What the files of one run are: the main document, its parts and the JSON.
pub(crate) struct Files<'a> {
    /// The main document's name without `.md` ([`main_name`]).
    pub main: &'a str,
    /// The parts folder ([`folder_stem`]).
    pub stem: &'a str,
    pub main_markdown: &'a str,
    /// `(<stem>/<file>.md, markdown)` in order.
    pub parts: Vec<(&'a str, &'a str)>,
    pub json: Option<&'a str>,
}

/// Writes the files of one run into `dir` (made when missing): the parts into `<stem>/`,
/// then the main document and the JSON. Parts of an earlier run that this one does not
/// write are removed, and the parts folder too when it is empty then. A symlink is never
/// written through. The error says which file could not be written.
pub(crate) fn write_files(dir: &Path, files: &Files<'_>) -> Result<(), String> {
    let shown = |path: &Path, e: std::io::Error| {
        format!(
            "cannot write {}: {}",
            path.display(),
            one_line(&e.to_string())
        )
    };
    std::fs::create_dir_all(dir).map_err(|e| shown(dir, e))?;
    let folder = dir.join(files.stem);
    let mut keep: HashSet<OsString> = HashSet::new();
    if !files.parts.is_empty() {
        prepare_folder(dir, files.stem).map_err(|e| shown(&folder, e))?;
    }
    for (path, markdown) in &files.parts {
        let target = dir.join(path);
        write_new(&target, markdown.as_bytes()).map_err(|e| shown(&target, e))?;
        keep.extend(target.file_name().map(std::ffi::OsStr::to_os_string));
    }
    let main = dir.join(format!("{}.md", files.main));
    write_new(&main, files.main_markdown.as_bytes()).map_err(|e| shown(&main, e))?;
    if let Some(json) = files.json {
        let target = dir.join(format!("{}.json", files.main));
        let mut text = json.to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        write_new(&target, text.as_bytes()).map_err(|e| shown(&target, e))?;
    }
    remove_stale(&folder, &keep, Stale::Parts).map_err(|e| {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stale {
    /// An assets folder: every regular file but Markdown (a parts folder of another source
    /// may have the same name).
    Assets,
    /// A parts folder: only Markdown parts Rezzer wrote, so a folder of the user's own notes
    /// that happens to have the name loses nothing.
    Parts,
}

/// The most bytes read to find the `part:` key of a part file. Rezzer's own frontmatter
/// can be long (keyword tags, file tags), so this is generous; it only bounds a hostile file.
const PART_HEAD_MAX: u64 = 16 * 1024 * 1024;

/// Whether `path` is a part file Rezzer wrote: frontmatter with a `part:` key. Read line by
/// line up to the closing `---`, without following a symlink (the caller checked the entry
/// type).
fn is_rezzer_part(path: &Path) -> bool {
    use std::io::BufRead;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut lines = std::io::BufReader::new(file.take(PART_HEAD_MAX)).split(b'\n');
    if !matches!(lines.next(), Some(Ok(first)) if first == b"---") {
        return false;
    }
    for line in lines {
        let Ok(line) = line else {
            return false;
        };
        if line == b"---" {
            return false;
        }
        if line.starts_with(b"part: ") {
            return true;
        }
    }
    false
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
        let markdown = Path::new(&entry.file_name())
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
        let remove = match which {
            Stale::Assets => !markdown,
            Stale::Parts => markdown && is_rezzer_part(&entry.path()),
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

/// Writes a new file at `path`. An existing entry (also a symlink) is removed without being
/// followed, and the file is created exclusively, so a symlink is never written through.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            return Err(std::io::Error::other("a directory is in the way"));
        }
        Ok(_) => std::fs::remove_file(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)
}
