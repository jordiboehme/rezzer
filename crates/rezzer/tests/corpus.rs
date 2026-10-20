// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The public test corpus in `tests/corpus/`, run through the real pipeline (text, Markdown
//! and CSV readers, no cache, `generated.at` fixed). Each corpus file has a
//! `<file>.facts.json` next to it; `corpus_facts` checks them all and reports every failure
//! at once, `corpus_golden` keeps `insta` snapshots of the model JSON and the Markdown.
//!
//! A fact is `{ "pointer": "/json/pointer", "equals"?: v, "len"?: n, "min_len"?: n,
//! "contains"?: "text", "absent"?: true }`. `absent`: the pointer must not exist. `pointer` points into the Document JSON; `contains` is a substring
//! of the rendered Markdown (without a `pointer`) or of the JSON value at the pointer
//! (serialized, for a non-string value). Corpus files are untrusted data and are only read.
#![cfg(all(feature = "fmt-text", feature = "fmt-markdown", feature = "fmt-csv"))]

use std::path::{Path, PathBuf};

use rezzer_model::Registry;
use rezzer_pipeline::{Config, OutputKind, Pipeline, Request};
use serde_json::Value;

const GENERATED_AT: &str = "2026-01-01T00:00:00Z";

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn pipeline() -> Pipeline {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_text::TextReader));
    registry.register(Box::new(rezzer_read_markdown::MarkdownReader));
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    let config = Config {
        cache: false,
        ..Config::default()
    };
    Pipeline::new(config, registry)
}

fn request() -> Request {
    Request {
        output: OutputKind::Json,
        generated_at: Some(GENERATED_AT.to_string()),
        ..Request::default()
    }
}

/// Folders whose files run through the pipeline with facts and snapshots. `office/` holds the
/// spreadsheet corpus; it is checked by `office_corpus_matches_manifest` until the spreadsheet
/// readers get facts and snapshots.
const PIPELINE_FOLDERS: [&str; 1] = ["web-text"];

/// Corpus files, sorted: everything in the pipeline folders except facts files.
fn corpus_files() -> Vec<PathBuf> {
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
            if !name.ends_with(".facts.json") && !name.starts_with('.') {
                files.push(file);
            }
        }
    }
    files.sort();
    files
}

struct Output {
    json: Value,
    markdown: String,
}

fn run(pipeline: &Pipeline, file: &Path) -> Output {
    let result = pipeline.run_path(file, &request());
    let json_text = result
        .json
        .unwrap_or_else(|| panic!("{}: no document: {:?}", file.display(), result.message));
    Output {
        json: serde_json::from_str(&json_text).unwrap(),
        markdown: result.markdown_pages.join(""),
    }
}

/// Checks one fact; returns why it fails.
fn check(fact: &Value, out: &Output) -> Result<(), String> {
    let pointer = fact.get("pointer").and_then(Value::as_str);
    if fact.get("absent").and_then(Value::as_bool) == Some(true) {
        return match pointer.and_then(|p| out.json.pointer(p)) {
            None => Ok(()),
            Some(v) => Err(format!(
                "{}: expected to be absent, found {v}",
                pointer.unwrap_or("")
            )),
        };
    }
    let found = match pointer {
        Some(p) => Some(
            out.json
                .pointer(p)
                .ok_or_else(|| format!("{p}: not found"))?,
        ),
        None => None,
    };
    let at = pointer.unwrap_or("markdown");
    if let Some(expected) = fact.get("equals")
        && found != Some(expected)
    {
        return Err(format!("{at}: expected {expected}, found {found:?}"));
    }
    let size = found.and_then(|v| match v {
        Value::Array(a) => Some(a.len()),
        Value::Object(o) => Some(o.len()),
        Value::String(s) => Some(s.chars().count()),
        _ => None,
    });
    if let Some(len) = fact.get("len").and_then(Value::as_u64)
        && size != Some(len as usize)
    {
        return Err(format!("{at}: expected len {len}, found {size:?}"));
    }
    if let Some(min) = fact.get("min_len").and_then(Value::as_u64)
        && size.is_none_or(|s| (s as u64) < min)
    {
        return Err(format!("{at}: expected min_len {min}, found {size:?}"));
    }
    if let Some(needle) = fact.get("contains").and_then(Value::as_str) {
        let hay = match found {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => out.markdown.clone(),
        };
        if !hay.contains(needle) {
            return Err(format!("{at}: does not contain {needle:?}"));
        }
    }
    Ok(())
}

#[test]
fn corpus_facts() {
    let pipeline = pipeline();
    let files = corpus_files();
    assert!(files.len() >= 19, "corpus not found: {} files", files.len());
    let mut failures = Vec::new();
    for file in &files {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let facts_path = file.with_file_name(format!("{name}.facts.json"));
        let Ok(text) = std::fs::read_to_string(&facts_path) else {
            failures.push(format!("{name}: no facts file"));
            continue;
        };
        let facts: Vec<Value> = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{}: invalid facts: {e}", facts_path.display()));
        if facts.is_empty() {
            failures.push(format!("{name}: facts file is empty"));
        }
        let out = run(&pipeline, file);
        for fact in &facts {
            if let Err(why) = check(fact, &out) {
                failures.push(format!("{name}: {why}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failed facts:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Makes a snapshot independent of the checkout location and of the file's modification time.
fn redact(text: &str, corpus: &str) -> String {
    let text = text.replace(corpus, "<corpus>");
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            if line.starts_with("source_date: ") {
                "source_date: <modified>".to_string()
            } else {
                line.to_string()
            }
        })
        .collect();
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Arrays longer than this are cut in the JSON snapshot, so that snapshots of big tables and
/// books stay readable and small. The facts see the whole document, the Markdown snapshot
/// is complete.
const SNAPSHOT_ARRAY_LIMIT: usize = 12;

fn shorten(value: &mut Value) {
    match value {
        Value::Array(items) => {
            if items.len() > SNAPSHOT_ARRAY_LIMIT {
                let more = items.len() - SNAPSHOT_ARRAY_LIMIT;
                items.truncate(SNAPSHOT_ARRAY_LIMIT);
                items.push(Value::String(format!("<{more} more items>")));
            }
            items.iter_mut().for_each(shorten);
        }
        Value::Object(map) => map.values_mut().for_each(shorten),
        _ => {}
    }
}

fn snapshot_json(out: &Output, corpus: &str) -> String {
    let mut value = out.json.clone();
    if let Some(modified) = value.pointer_mut("/source/modified") {
        *modified = Value::String("<modified>".into());
    }
    shorten(&mut value);
    redact(&serde_json::to_string_pretty(&value).unwrap(), corpus)
}

#[test]
fn corpus_golden() {
    let pipeline = pipeline();
    let corpus = std::fs::canonicalize(corpus_dir()).unwrap();
    let corpus = corpus.to_string_lossy().to_string();
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(corpus_dir().join("snapshots"));
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| {
        for file in corpus_files() {
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            let out = run(&pipeline, &file);
            insta::assert_snapshot!(format!("{name}.json"), snapshot_json(&out, &corpus));
            insta::assert_snapshot!(format!("{name}.md"), redact(&out.markdown, &corpus));
        }
    });
}

/// The rows of MANIFEST.md: relative path to SHA-256. Only rows whose first cell is a path.
fn manifest_rows() -> std::collections::BTreeMap<String, String> {
    let manifest = std::fs::read_to_string(corpus_dir().join("MANIFEST.md")).unwrap();
    let mut listed = std::collections::BTreeMap::new();
    for line in manifest.lines().filter(|l| {
        l.starts_with("| ")
            && l[2..]
                .split(" |")
                .next()
                .is_some_and(|c| c.contains('/') && !c.contains(' '))
    }) {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(cells.len(), 4, "bad row: {line}");
        assert!(
            listed
                .insert(cells[0].to_string(), cells[3].to_string())
                .is_none(),
            "listed twice: {line}"
        );
        assert!(cells[1].contains("http"), "no source URL: {line}");
    }
    listed
}

fn relative(file: &Path) -> String {
    file.strip_prefix(corpus_dir())
        .unwrap()
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256_of(file: &Path) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(std::fs::read(file).unwrap()))
}

/// Every pipeline corpus file is listed in MANIFEST.md with its SHA-256, and every row has a file.
#[test]
fn corpus_manifest_matches_the_files() {
    let mut listed = manifest_rows();
    listed.retain(|k, _| {
        PIPELINE_FOLDERS
            .iter()
            .any(|f| k.starts_with(&format!("{f}/")))
    });
    let mut problems = Vec::new();
    for file in corpus_files() {
        let rel = relative(&file);
        let hash = sha256_of(&file);
        match listed.remove(&rel) {
            None => problems.push(format!("{rel}: not in MANIFEST.md")),
            Some(h) if h != hash => problems.push(format!("{rel}: hash differs")),
            Some(_) => {}
        }
    }
    problems.extend(listed.keys().map(|k| format!("{k}: no such file")));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Every file in `office/` (including `office/malformed/`) is listed in MANIFEST.md with its
/// full SHA-256, every `office/` row has a file, and no file is over 2 MB. The files are never
/// skipped: a missing file fails the test.
#[test]
fn office_corpus_matches_manifest() {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else {
                files.push(path);
            }
        }
    }
    let mut listed = manifest_rows();
    listed.retain(|k, _| k.starts_with("office/"));
    assert_eq!(listed.len(), 33, "expected 33 office rows");
    let mut files = Vec::new();
    walk(&corpus_dir().join("office"), &mut files);
    files.sort();
    let mut problems = Vec::new();
    for file in &files {
        let rel = relative(file);
        let len = std::fs::metadata(file).unwrap().len();
        if len > 2 * 1024 * 1024 {
            problems.push(format!("{rel}: {len} bytes, over 2 MB"));
        }
        match listed.remove(&rel) {
            None => problems.push(format!("{rel}: not in MANIFEST.md")),
            Some(h) if h.len() != 64 => problems.push(format!("{rel}: not a full SHA-256")),
            Some(h) if h != sha256_of(file) => problems.push(format!("{rel}: hash differs")),
            Some(_) => {}
        }
    }
    problems.extend(listed.keys().map(|k| format!("{k}: no such file")));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
