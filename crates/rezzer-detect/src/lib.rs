// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Opens a file safely, hashes it and decides its format from the bytes.
//!
//! Detection order: empty, magic bytes, container listing (ZIP, OLE), pattern signatures
//! (IGES, STL), text sniffing, else
//! unknown. The file name's extension only breaks ties between candidates with equal
//! evidence (CSV or TSV when sniffing declines). Every check reads a bounded prefix, so
//! detection stays cheap on large files.

mod container;
mod magic;
mod source;
mod text;

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
    source::open(path, limits)
}

/// Builds a [`Source`] from bytes already in memory (an upload, a container member).
/// `name` is used for display and as the extension tie-breaker.
pub fn from_bytes(name: &str, bytes: Vec<u8>, limits: &Limits) -> Result<Source, OpenError> {
    source::from_bytes(name, bytes, limits)
}

/// Detects the format of `bytes` by content. `name` only breaks ties.
pub fn detect(name: &str, bytes: &[u8]) -> (FormatId, Detection) {
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
    if let Some(f) = text::detect(name, bytes) {
        return f;
    }
    found(
        FormatId::Unknown,
        Confidence::Likely,
        "no known signature and not text (binary or control bytes)",
    )
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
