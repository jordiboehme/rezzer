// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Opens a file safely, hashes it and decides its format from the bytes.
//!
//! Detection order: empty, magic bytes, container listing (ZIP, OLE), pattern signatures
//! (PDF after a binary prefix, IGES, ASCII DXF, SolidWorks blocks, STL), text sniffing, a ZIP
//! after a stub, else unknown. The file name's extension and a declared MIME type are hints:
//! they only break ties between candidates with equal evidence (CSV, TSV or Markdown when
//! sniffing declines) and move the confidence of a text result, never override content
//! evidence. Every check reads a bounded prefix, so detection stays cheap on large files.

mod container;
mod magic;
mod source;
mod text;
mod zipwalk;

use std::path::Path;

use rezzer_limits::Limits;
use rezzer_model::{Confidence, Detection, FormatId, Source};

/// Why a file could not be opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenError {
    #[error("file not found: {0}")]
    NotFound(String),
    #[error("not a regular file: {0}")]
    NotAFile(String),
    #[error("file is too large: {size} bytes, the limit is {max} bytes")]
    TooLarge { size: u64, max: u64 },
    #[error("cannot read file: {0}")]
    Io(String),
}

/// Opens `path`: refuses directories and other non-files, refuses files larger than
/// `limits.max_file_bytes`, maps the file, hashes it (SHA-256, lowercase hex), records the
/// modification time (RFC 3339, UTC) and detects its format.
pub fn open(path: &Path, limits: &Limits) -> Result<Source, OpenError> {
    source::open(path, limits, &detect_with_hints)
}

/// A format detector: file name, declared MIME type and bytes in, format and evidence out.
/// [`detect_with_hints`] is the one Rezzer uses.
pub type Detector<'a> = dyn Fn(&str, Option<&str>, &[u8]) -> (FormatId, Detection) + 'a;

/// Like [`open`], with another detector, for example [`detect_with_hints`] wrapped in panic
/// containment by the pipeline, or a test double.
pub fn open_with_detector(
    path: &Path,
    limits: &Limits,
    detector: &Detector<'_>,
) -> Result<Source, OpenError> {
    source::open(path, limits, detector)
}

/// Like [`from_bytes_with_hints`], with another detector. See [`open_with_detector`].
pub fn from_bytes_with_detector(
    name: &str,
    mime: Option<&str>,
    bytes: Vec<u8>,
    limits: &Limits,
    detector: &Detector<'_>,
) -> Result<Source, OpenError> {
    source::from_bytes(name, mime, bytes, limits, detector)
}

/// Builds a [`Source`] from bytes already in memory (an upload, a container member).
/// `name` is used for display and as the extension tie-breaker.
pub fn from_bytes(name: &str, bytes: Vec<u8>, limits: &Limits) -> Result<Source, OpenError> {
    source::from_bytes(name, None, bytes, limits, &detect_with_hints)
}

/// Like [`from_bytes`], with the MIME type the sender declared (for example the type of a
/// data: URI) as a further hint. See [`detect_with_hints`].
pub fn from_bytes_with_hints(
    name: &str,
    mime: Option<&str>,
    bytes: Vec<u8>,
    limits: &Limits,
) -> Result<Source, OpenError> {
    source::from_bytes(name, mime, bytes, limits, &detect_with_hints)
}

/// Formats a time as RFC 3339 in UTC with whole seconds, for example
/// `2026-10-19T08:30:00Z`. Times before 1970 give None.
pub fn rfc3339(t: std::time::SystemTime) -> Option<String> {
    source::rfc3339(t)
}

/// Detects the format of `bytes` by content. `name` only breaks ties.
pub fn detect(name: &str, bytes: &[u8]) -> (FormatId, Detection) {
    detect_with_hints(name, None, bytes)
}

/// Detects the format of `bytes` by content, with the file name and a declared MIME type as
/// hints. Hints are used after the content checks: a MIME type naming CSV, TSV or Markdown
/// decides when sniffing declines (before the extension) and raises or lowers the confidence
/// of a text result. Magic bytes, containers and other clear evidence always win: a PDF named
/// `.txt` and sent as `text/plain` stays a PDF. Parameters such as `; charset=utf-8` and case
/// are ignored.
pub fn detect_with_hints(name: &str, mime: Option<&str>, bytes: &[u8]) -> (FormatId, Detection) {
    let mime = mime.map(normalize_mime).filter(|m| !m.is_empty());
    let mime = mime.as_deref();
    if bytes.is_empty() {
        return found(FormatId::Empty, Confidence::Certain, "file is empty");
    }
    if let Some(f) = magic::detect(bytes) {
        return f;
    }
    if bytes.starts_with(magic::OLE_MAGIC) {
        return container::ole(bytes);
    }
    if bytes.starts_with(magic::ZIP_MAGIC) || bytes.starts_with(magic::ZIP_EMPTY_MAGIC) {
        return container::zip(bytes);
    }
    if let Some(f) = magic::detect_patterns(bytes) {
        return f;
    }
    if let Some(f) = text::detect(name, mime, bytes) {
        return f;
    }
    // Self-extracting archives: a program stub in front of a ZIP. Only the directory is
    // checked; the layout behind the stub is not inspected.
    if zipwalk::has_directory(bytes) {
        return found(
            FormatId::Zip,
            Confidence::Likely,
            "zip archive after a stub",
        );
    }
    found(
        FormatId::Unknown,
        Confidence::Likely,
        "no known signature and not text (binary or control bytes)",
    )
}

/// `Text/CSV; charset=utf-8` -> `text/csv`.
fn normalize_mime(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

pub(crate) type Found = (FormatId, Detection);

pub(crate) fn found(
    format: FormatId,
    confidence: Confidence,
    evidence: impl Into<String>,
) -> Found {
    (
        format,
        Detection {
            confidence,
            evidence: evidence.into(),
        },
    )
}

/// Lowercased extension of `name`, if any.
pub(crate) fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// Text taken from a file for evidence: control characters dropped, at most 100 characters.
pub(crate) fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(100).collect()
}
