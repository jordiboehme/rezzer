// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bounded access to untrusted ZIP archives.
//!
//! Limits are enforced on the bytes that are really unpacked, not on the sizes the
//! archive declares.
//!
//! ```
//! use zipguard::{Archive, Limits};
//! assert!(Archive::open(b"not a zip", Limits::default()).is_err());
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, Cursor, Read};

use zip::ZipArchive;
use zip::result::ZipError as RawError;

/// Bounds for one archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Most entries the archive may list.
    pub max_entries: usize,
    /// Most unpacked bytes of one entry.
    pub max_entry_bytes: u64,
    /// Most unpacked bytes of the whole archive: the sum over its distinct entries. An
    /// entry read again counts once.
    pub max_total_bytes: u64,
    /// Most unpacked bytes per compressed byte of one entry.
    pub max_ratio: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_entries: 10_000,
            max_entry_bytes: 1 << 30,
            max_total_bytes: 4 << 30,
            max_ratio: 100,
        }
    }
}

/// Why an archive or an entry could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipError {
    /// The bytes are not a ZIP archive.
    NotZip,
    /// The archive is a ZIP but broken or unsafe; the text says how.
    Damaged(String),
    /// The entry is encrypted; the text is its name.
    Encrypted(String),
    /// No entry has this name.
    Missing(String),
    /// A limit was hit; `what` names it.
    Limit { what: String },
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZipError::NotZip => write!(f, "not a ZIP archive"),
            ZipError::Damaged(why) => write!(f, "damaged ZIP archive: {why}"),
            ZipError::Encrypted(name) => write!(f, "entry {name} is encrypted"),
            ZipError::Missing(name) => write!(f, "entry {name} is not in the archive"),
            ZipError::Limit { what } => write!(f, "limit reached: {what}"),
        }
    }
}

impl std::error::Error for ZipError {}

/// A ZIP archive held in memory, read within limits.
pub struct Archive<'a> {
    zip: ZipArchive<Cursor<&'a [u8]>>,
    names: Vec<String>,
    index: HashSet<String>,
    limits: Limits,
    /// The sum of `seen`.
    unpacked: u64,
    /// The most bytes unpacked so far from each entry.
    seen: HashMap<String, u64>,
}

impl<'a> Archive<'a> {
    /// Opens the archive and checks the entry count, every entry name and that the archive
    /// can be read only one way: no two entries with one name, every local header naming
    /// the entry its central directory record names, no entries that overlap.
    pub fn open(bytes: &'a [u8], limits: Limits) -> Result<Archive<'a>, ZipError> {
        let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(open_error)?;
        if zip.len() > limits.max_entries {
            return Err(ZipError::Limit {
                what: format!("{} entries, the limit is {}", zip.len(), limits.max_entries),
            });
        }
        zip.validate_header_consistency()
            .map_err(|e| ZipError::Damaged(e.to_string()))?;
        let mut names = Vec::with_capacity(zip.len());
        for name in zip.file_names() {
            let name = name.map_err(|e| ZipError::Damaged(format!("entry name: {e}")))?;
            if let Some(why) = unsafe_name(&name) {
                return Err(ZipError::Damaged(format!(
                    "entry name {} {why}",
                    name.escape_debug()
                )));
            }
            names.push(name.into_owned());
        }
        let index = names.iter().cloned().collect();
        Ok(Archive {
            zip,
            names,
            index,
            limits,
            unpacked: 0,
            seen: HashMap::new(),
        })
    }

    /// Entry names in archive order (directories included).
    pub fn names(&self) -> Vec<&str> {
        self.names.iter().map(String::as_str).collect()
    }

    /// Whether an entry has exactly this name.
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains(name)
    }

    /// Unpacks one entry into memory.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, ZipError> {
        let mut out = Vec::new();
        self.pump(name, Some(&mut out))?;
        Ok(out)
    }

    /// Unpacks one entry into a counter and returns its real size.
    pub fn measure(&mut self, name: &str) -> Result<u64, ZipError> {
        self.pump(name, None)
    }

    /// Bytes unpacked so far by `read` and `measure`, each entry counted once: a second
    /// read of an entry adds only what it unpacks beyond the first, normally nothing.
    pub fn unpacked(&self) -> u64 {
        self.unpacked
    }

    fn pump(&mut self, name: &str, mut sink: Option<&mut Vec<u8>>) -> Result<u64, ZipError> {
        if !self.contains(name) {
            return Err(ZipError::Missing(name.to_string()));
        }
        let limits = self.limits;
        // What this entry unpacked before is already counted, so a re-read may use it again.
        let before = self.seen.get(name).copied().unwrap_or(0);
        let remaining = limits
            .max_total_bytes
            .saturating_sub(self.unpacked - before);
        let mut file = self.zip.by_name(name).map_err(|e| match e {
            RawError::FileNotFound => ZipError::Missing(name.to_string()),
            other => entry_error(name, other),
        })?;
        if file.encrypted() {
            return Err(ZipError::Encrypted(name.to_string()));
        }
        let ratio_cap = file
            .compressed_size()
            .max(1)
            .saturating_mul(limits.max_ratio);
        // Each cap allows its limit plus one byte, so a stream that exceeds it is seen.
        let caps = [
            (limits.max_entry_bytes, "entry size"),
            (ratio_cap, "compression ratio"),
            (remaining, "total unpacked size"),
        ];
        let (cap, what) = caps
            .iter()
            .min_by_key(|(cap, _)| *cap)
            .copied()
            .expect("three caps");
        let mut count: u64 = 0;
        let mut buf = vec![0u8; 64 * 1024];
        let mut failure = None;
        loop {
            let want = (cap.saturating_add(1) - count).min(buf.len() as u64) as usize;
            let n = match file.read(&mut buf[..want]) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.to_string().contains("larger than its declared") => {
                    // The zip crate stops an entry that outgrows its header; that is a size lie.
                    failure = Some(ZipError::Limit {
                        what: format!("{name} is larger than its declared size"),
                    });
                    break;
                }
                Err(e) => {
                    failure = Some(ZipError::Damaged(format!("entry {name}: {e}")));
                    break;
                }
            };
            count += n as u64;
            if count > cap {
                // The extra byte proves the limit; it is not counted as unpacked.
                count = cap;
                failure = Some(ZipError::Limit {
                    what: format!("{name} exceeds the {what} limit"),
                });
                break;
            }
            if let Some(out) = sink.as_deref_mut() {
                out.extend_from_slice(&buf[..n]);
            }
        }
        if count > before {
            self.unpacked = self.unpacked.saturating_add(count - before);
            self.seen.insert(name.to_string(), count);
        }
        match failure {
            Some(err) => Err(err),
            None => Ok(count),
        }
    }
}

fn open_error(err: RawError) -> ZipError {
    match err {
        RawError::InvalidArchive(msg)
            if msg.contains("EOCD") || msg.contains("central directory end") =>
        {
            ZipError::NotZip
        }
        RawError::InvalidArchive(msg) => ZipError::Damaged(msg.into_owned()),
        RawError::UnsupportedArchive(msg) => ZipError::Damaged(msg.to_string()),
        RawError::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof => ZipError::NotZip,
        other => ZipError::Damaged(other.to_string()),
    }
}

fn entry_error(name: &str, err: RawError) -> ZipError {
    match err {
        RawError::UnsupportedArchive(msg) if msg.to_ascii_lowercase().contains("password") => {
            ZipError::Encrypted(name.to_string())
        }
        RawError::InvalidPassword => ZipError::Encrypted(name.to_string()),
        other => ZipError::Damaged(format!("entry {name}: {other}")),
    }
}

/// Why a name is not safe to use, or None.
fn unsafe_name(name: &str) -> Option<&'static str> {
    if name.contains('\0') {
        return Some("contains a NUL byte");
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return Some("is an absolute path");
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some("is an absolute path");
    }
    if name.split(['/', '\\']).any(|part| part == "..") {
        return Some("climbs out with ..");
    }
    None
}
