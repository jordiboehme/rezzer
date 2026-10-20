// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use rezzer_limits::Budget;

use crate::document::{Detection, Document, SourceInfo};
use crate::format::{Completeness, FormatId};

/// Reads one family of formats into a [`Document`].
///
/// A reader never fails on bad input: it returns what it has plus [`crate::Loss`] entries and
/// a [`Completeness`]. It checks `budget` at loop boundaries.
pub trait Reader: Send + Sync {
    /// Short stable id, for example `xlsx` or `pdf`.
    fn id(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn formats(&self) -> &'static [FormatId];
    fn read(&self, src: &Source, opts: &ReadOptions, budget: &Budget) -> ReadOutcome;
}

/// The result of one read. Debug and Clone only: it is not serialized, the [`Document`] is.
#[derive(Debug, Clone)]
pub struct ReadOutcome {
    pub document: Document,
    pub completeness: Completeness,
}

/// Options for one read. Debug and Clone only: never serialized, and Debug hides the password.
#[derive(Clone, Default)]
pub struct ReadOptions {
    pub password: Option<String>,
}

impl fmt::Debug for ReadOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let password = self.password.as_ref().map(|_| "<redacted>");
        f.debug_struct("ReadOptions")
            .field("password", &password)
            .finish()
    }
}

/// The raw bytes of a source, memory-mapped or owned. Cheap to clone.
#[derive(Clone)]
pub enum Bytes {
    Mapped(Arc<memmap2::Mmap>),
    Owned(Arc<Vec<u8>>),
}

impl Bytes {
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Bytes::Mapped(m) => m,
            Bytes::Owned(v) => v,
        }
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Bytes::Mapped(_) => "Mapped",
            Bytes::Owned(_) => "Owned",
        };
        write!(f, "Bytes::{kind}({} bytes)", self.as_slice().len())
    }
}

/// A file to read, with its detected format. Debug and Clone only: it holds the raw bytes
/// and is never serialized; [`crate::SourceInfo`] is its serialized form.
#[derive(Debug, Clone)]
pub struct Source {
    pub path: Option<PathBuf>,
    pub name: String,
    pub bytes: Bytes,
    /// Lowercase hex.
    pub sha256: String,
    pub size: u64,
    pub format: FormatId,
    pub detection: Detection,
    pub modified: Option<String>,
}

impl Source {
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

/// Builds the [`SourceInfo`] of a [`Source`]: name, path, hash, size, format, detection and
/// modification time are copied. `encoding` and `reader` stay empty for the reader to fill.
pub fn source_info(src: &Source) -> SourceInfo {
    SourceInfo {
        path: src.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        name: src.name.clone(),
        sha256: src.sha256.clone(),
        size: src.size,
        format: src.format,
        detection: src.detection.clone(),
        modified: src.modified.clone(),
        encoding: None,
        reader: None,
    }
}
