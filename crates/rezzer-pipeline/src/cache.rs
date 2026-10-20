// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The result cache: one JSON file per key holding the enriched document and its
//! completeness, so a hit skips reading and enriching.
//!
//! The key is the SHA-256 of a canonical JSON object (sorted keys) with the source hash, the
//! Rezzer version and build id, the model version and what else changes the document: the file name (readers use it, for example as the CSV sheet name, and it is a
//! detection hint), the detected format (a MIME hint can change it), the reader id and
//! version (the reader set depends on cargo features), the limits except the page size and
//! the disabled enrich passes. The request's output kind, sections and `generated_at` are not in the key: they
//! only change rendering, so Markdown and JSON runs share an entry. A request with a password
//! is never cached (decided by the pipeline).
//!
//! Entries are written atomically (temporary file, then rename). An entry that does not parse
//! is ignored and rewritten. After a write that takes the cache over
//! [`crate::Config::cache_max_bytes`], the least recently used entries (by modification
//! time, which a hit refreshes) are removed
//! until it is under 90 % of the cap. The cache is best effort: any cache error is skipped
//! silently and the run goes on.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rezzer_model::{Completeness, Document, MODEL_VERSION, Source};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{BUILD_ID, Config, VERSION};

pub(crate) struct Cache {
    dir: PathBuf,
    max_bytes: u64,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    completeness: Completeness,
    document: Document,
}

impl Cache {
    /// The cache of `config`, or None when caching is off or there is no cache dir.
    pub(crate) fn of(config: &Config) -> Option<Cache> {
        if !config.cache {
            return None;
        }
        let dir = match &config.cache_dir {
            Some(dir) => dir.clone(),
            None => directories::ProjectDirs::from("", "", "rezzer")?
                .cache_dir()
                .to_path_buf(),
        };
        Some(Cache {
            dir,
            max_bytes: config.cache_max_bytes,
        })
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// The entry for `key`, if there is one that parses and belongs to `src`.
    pub(crate) fn get(&self, key: &str, src: &Source) -> Option<(Document, Completeness)> {
        let path = self.path(key);
        let file = File::open(&path).ok()?;
        let entry: Entry = serde_json::from_reader(BufReader::new(file)).ok()?;
        let fits = entry.document.model_version == MODEL_VERSION
            && entry.document.source.sha256 == src.sha256
            && entry.completeness != Completeness::Failed;
        if fits {
            // A hit makes the entry the most recently used one for eviction (LRU).
            let _ = File::options()
                .write(true)
                .open(&path)
                .and_then(|f| f.set_modified(SystemTime::now()));
        }
        fits.then_some((entry.document, entry.completeness))
    }

    /// Stores an entry. Failed results are never stored.
    pub(crate) fn put(&self, key: &str, document: &Document, completeness: Completeness) {
        if completeness == Completeness::Failed {
            return;
        }
        let entry = EntryRef {
            completeness,
            document,
        };
        // Best effort: an unwritable cache only costs speed.
        let path = self.path(key);
        if write_atomic(&self.dir, &path, &entry).is_ok() {
            self.evict(&path);
        }
    }

    /// Removes the oldest entries while the cache is over its cap, down to 90 % of it. The
    /// entry just written goes last, and only if it alone is over the cap.
    fn evict(&self, newest: &Path) {
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut entries: Vec<(bool, std::time::SystemTime, PathBuf, u64)> = dir
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| {
                let meta = e.metadata().ok().filter(|m| m.is_file())?;
                let path = e.path();
                Some((path == newest, meta.modified().ok()?, path, meta.len()))
            })
            .collect();
        let mut total: u64 = entries.iter().map(|e| e.3).sum();
        if total <= self.max_bytes {
            return;
        }
        let target = self.max_bytes - self.max_bytes / 10;
        // Oldest first; the newest entry sorts last whatever its time says.
        entries.sort();
        for (is_newest, _, path, len) in entries {
            if total <= target || (is_newest && total <= self.max_bytes) {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }
}

#[derive(Serialize)]
struct EntryRef<'a> {
    completeness: Completeness,
    document: &'a Document,
}

fn write_atomic(dir: &Path, path: &Path, entry: &EntryRef<'_>) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".entry-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    write_entry(tmp.as_file_mut(), entry)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Serializes `entry` through a buffer: serde_json writes token by token, which unbuffered
/// is one system call per token and made a cache miss several times slower than no cache.
fn write_entry<W: Write>(out: W, entry: &EntryRef<'_>) -> std::io::Result<()> {
    let mut buffered = BufWriter::with_capacity(64 * 1024, out);
    serde_json::to_writer(&mut buffered, entry)?;
    buffered.flush()
}

/// The cache key of one read, as lowercase hex.
pub(crate) fn key(src: &Source, reader: Option<(&str, &str)>, config: &Config) -> String {
    // The page size only changes pagination, which runs after the cache.
    let mut limits = serde_json::to_value(&config.limits).unwrap_or_default();
    if let Some(map) = limits.as_object_mut() {
        map.remove("page_tokens");
    }
    let mut disabled = config.enrich.disabled.clone();
    disabled.sort();
    disabled.dedup();
    let material = json!({
        "sha256": src.sha256,
        "name": src.name,
        "rezzer": VERSION,
        "build": BUILD_ID,
        "model_version": MODEL_VERSION,
        "format": rezzer_render::format_name(src.format),
        "reader": reader.map(|(id, version)| format!("{id}@{version}")),
        "limits": limits,
        "enrich_disabled": disabled,
    });
    // serde_json keeps object keys sorted, so the text is canonical.
    hex::encode(Sha256::digest(material.to_string().as_bytes()))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use rezzer_model::{Block, Confidence, Detection, FormatId, Inline, SourceInfo};

    use super::*;

    /// Counts write calls and bytes.
    #[derive(Default)]
    struct Counter {
        calls: usize,
        bytes: usize,
    }

    impl Write for Counter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            self.bytes += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn large_entries_are_written_in_big_chunks() {
        let mut document = Document::new(SourceInfo {
            path: None,
            name: "big.txt".into(),
            sha256: "0".repeat(64),
            size: 0,
            format: FormatId::Text,
            detection: Detection {
                confidence: Confidence::Likely,
                evidence: "test".into(),
            },
            modified: None,
            encoding: None,
            reader: None,
        });
        for i in 0..50_000 {
            document.body.push(Block::Paragraph {
                inlines: vec![Inline::Text {
                    text: format!("paragraph {i}"),
                    emphasis: false,
                    strong: false,
                    strike: false,
                }],
                role: None,
                anchor: None,
            });
        }
        let entry = EntryRef {
            completeness: Completeness::Complete,
            document: &document,
        };
        let mut counter = Counter::default();
        write_entry(&mut counter, &entry).unwrap();
        assert!(counter.bytes > 2_000_000, "{} bytes", counter.bytes);
        // Unbuffered, serde_json makes several write calls per paragraph (hundreds of
        // thousands here); buffered, it is about one call per 64 KiB.
        assert!(
            counter.calls <= counter.bytes / (32 * 1024) + 2,
            "{} write calls for {} bytes",
            counter.calls,
            counter.bytes
        );
    }
}
