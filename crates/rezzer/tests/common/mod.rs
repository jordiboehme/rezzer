// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the corpus tests share: the corpus files and the pipeline that reads them. Every item
//! here is used by each test that includes this module, so none of it is dead code there.
//! Corpus files are untrusted data and are only read.

use std::path::{Path, PathBuf};

use rezzer_model::Registry;
use rezzer_pipeline::{Config, Pipeline};

pub const GENERATED_AT: &str = "2026-01-01T00:00:00Z";

/// Folders whose files run through the pipeline with facts. `office/malformed/` is not walked:
/// the hostile tests read those files.
pub const PIPELINE_FOLDERS: [&str; 3] = ["web-text", "office", "pdf"];

pub fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

/// The text, Markdown, CSV, spreadsheet and PDF readers, without the cache.
pub fn pipeline() -> Pipeline {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_text::TextReader));
    registry.register(Box::new(rezzer_read_markdown::MarkdownReader));
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    registry.register(Box::new(rezzer_read_sheet::SheetReader));
    registry.register(Box::new(rezzer_read_pdf::PdfReader));
    let config = Config {
        cache: false,
        ..Config::default()
    };
    Pipeline::new(config, registry)
}

/// Corpus files, sorted: the files right in the pipeline folders, except facts files and the
/// large files of `remote.tsv` (they may be absent).
pub fn corpus_files() -> Vec<PathBuf> {
    let remote = remote_rows();
    let mut files = Vec::new();
    for folder in std::fs::read_dir(corpus_dir()).unwrap() {
        let folder = folder.unwrap().path();
        if !folder.is_dir()
            || !PIPELINE_FOLDERS
                .iter()
                .any(|f| folder.file_name().is_some_and(|n| n == *f))
        {
            continue;
        }
        for file in std::fs::read_dir(&folder).unwrap() {
            let file = file.unwrap().path();
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            if file.is_file()
                && !name.ends_with(".facts.json")
                && !name.starts_with('.')
                && !remote.contains_key(&relative(&file))
            {
                files.push(file);
            }
        }
    }
    files.sort();
    files
}

/// The path of a corpus file below the corpus folder, with `/`.
pub fn relative(file: &Path) -> String {
    file.strip_prefix(corpus_dir())
        .unwrap()
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// The `pdf/` rows of `remote.tsv` (path, SHA-256): large files that are fetched, not committed.
pub fn remote_rows() -> std::collections::BTreeMap<String, String> {
    let list = std::fs::read_to_string(corpus_dir().join("remote.tsv")).unwrap();
    let mut rows = std::collections::BTreeMap::new();
    for line in list
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let cells: Vec<&str> = line.split('\t').collect();
        assert_eq!(cells.len(), 3, "bad remote.tsv row: {line}");
        assert!(cells[1].starts_with("https://"), "not https: {line}");
        assert_eq!(cells[2].len(), 64, "not a full SHA-256: {line}");
        rows.insert(cells[0].to_string(), cells[2].to_string());
    }
    rows
}
