// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The result cache: one JSON file per key holding the enriched document and its
//! completeness, so a hit skips reading and enriching.
//!
//! The key is the SHA-256 of a canonical JSON object (sorted keys) with the source hash, the
//! Rezzer and model versions and what else changes the document: the file name (readers use it, for example as the CSV sheet name, and it is a
//! detection hint), the detected format (a MIME hint can change it), the reader id and
//! version (the reader set depends on cargo features), the limits and the disabled enrich
//! passes. The request's output kind, sections and `generated_at` are not in the key: they
//! only change rendering, so Markdown and JSON runs share an entry. A request with a password
//! is never cached (decided by the pipeline).
//!
//! Entries are written atomically (temporary file, then rename). An entry that does not parse
//! is ignored and rewritten. After a write that takes the cache over
//! [`crate::Config::cache_max_bytes`], the oldest entries (by modification time) are removed
//! until it is under 90 % of the cap. The cache is best effort: any cache error is skipped
//! silently and the run goes on.

use std::io::Write;
use std::path::{Path, PathBuf};

use rezzer_model::{Completeness, Document, MODEL_VERSION, Source};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{Config, VERSION};

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
        let text = std::fs::read(self.path(key)).ok()?;
        let entry: Entry = serde_json::from_slice(&text).ok()?;
        let fits = entry.document.model_version == MODEL_VERSION
            && entry.document.source.sha256 == src.sha256
            && entry.completeness != Completeness::Failed;
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
    serde_json::to_writer(&mut tmp, entry)?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// The cache key of one read, as lowercase hex.
pub(crate) fn key(src: &Source, reader: Option<(&str, &str)>, config: &Config) -> String {
    let mut disabled = config.enrich.disabled.clone();
    disabled.sort();
    disabled.dedup();
    let material = json!({
        "sha256": src.sha256,
        "name": src.name,
        "rezzer": VERSION,
        "model_version": MODEL_VERSION,
        "format": rezzer_render::format_name(src.format),
        "reader": reader.map(|(id, version)| format!("{id}@{version}")),
        "limits": config.limits,
        "enrich_disabled": disabled,
    });
    // serde_json keeps object keys sorted, so the text is canonical.
    hex::encode(Sha256::digest(material.to_string().as_bytes()))
}
