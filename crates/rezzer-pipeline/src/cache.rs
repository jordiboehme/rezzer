// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The result cache: one JSON file per key holding the enriched document and its
//! completeness, so a hit skips reading and enriching.
//!
//! The key is the SHA-256 of a canonical JSON object (sorted keys) with the source hash, the
//! Rezzer and model versions, the request without `generated_at`, and what else changes the
//! document: the detected format (the name and MIME hints can change it), the reader id and
//! version (the reader set depends on cargo features), the limits and the disabled enrich
//! passes. Entries are written atomically (temporary file, then rename). An entry that does
//! not parse is ignored and rewritten. The cache is best effort: a write that fails is
//! dropped, the run goes on.

use std::io::Write;
use std::path::{Path, PathBuf};

use rezzer_model::{Completeness, Document, MODEL_VERSION, Source};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{Config, OutputKind, Request, VERSION};

pub(crate) struct Cache {
    dir: PathBuf,
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
        Some(Cache { dir })
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
        let _ = write_atomic(&self.dir, &self.path(key), &entry);
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

/// The cache key of one run, as lowercase hex.
pub(crate) fn key(
    src: &Source,
    reader: Option<(&str, &str)>,
    config: &Config,
    req: &Request,
) -> String {
    let mut disabled = config.enrich.disabled.clone();
    disabled.sort();
    disabled.dedup();
    let request = json!({
        "output": match req.output {
            OutputKind::Markdown => "markdown",
            OutputKind::Json => "json",
        },
        "sections": req
            .sections
            .as_ref()
            .map(|list| list.iter().map(|s| s.name()).collect::<Vec<_>>()),
    });
    let material = json!({
        "sha256": src.sha256,
        "rezzer": VERSION,
        "model_version": MODEL_VERSION,
        "format": rezzer_render::format_name(src.format),
        "reader": reader.map(|(id, version)| format!("{id}@{version}")),
        "limits": config.limits,
        "enrich_disabled": disabled,
        "request": request,
    });
    // serde_json keeps object keys sorted, so the text is canonical.
    hex::encode(Sha256::digest(material.to_string().as_bytes()))
}
