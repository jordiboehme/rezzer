// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The public test corpus in `tests/corpus/`, run through the real pipeline (text, Markdown,
//! CSV, spreadsheet and PDF readers, no cache, `generated.at` fixed). Each corpus file has a
//! `<file>.facts.json` next to it; `corpus_facts` checks them all and reports every failure
//! at once, `corpus_golden` keeps `insta` snapshots of the model JSON and the Markdown of the
//! text files, of [`GOLDEN_OFFICE`] and of [`GOLDEN_PDF`]. The named spreadsheet and PDF
//! tests below check what the files prove: knowledge, not formatting. The large files of
//! `remote.tsv` are not walked; ignored tests read them after `scripts/fetch-corpus.sh`.
//!
//! A fact is `{ "pointer": "/json/pointer", "equals"?: v, "len"?: n, "min_len"?: n,
//! "contains"?: "text", "absent"?: true }`. `absent`: the pointer must not exist. `pointer` points into the Document JSON; `contains` is a substring
//! of the rendered Markdown (without a `pointer`) or of the JSON value at the pointer
//! (serialized, for a non-string value). Corpus files are untrusted data and are only read.
#![cfg(all(
    feature = "fmt-text",
    feature = "fmt-markdown",
    feature = "fmt-csv",
    feature = "fmt-xlsx",
    feature = "fmt-ods",
    feature = "fmt-pdf"
))]

mod common;

use std::path::{Path, PathBuf};

use common::{
    GENERATED_AT, PIPELINE_FOLDERS, corpus_dir, corpus_files, pipeline, relative, remote_rows,
};
use rezzer_model::{Block, Document, plain_text};
#[cfg(unix)]
use rezzer_pipeline::Config;
use rezzer_pipeline::{OutputKind, Pipeline, Request};
use serde_json::Value;

/// The allocator of the Rezzer binary: the memory these tests measure is the memory it
/// ships with.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

fn request() -> Request {
    Request {
        output: OutputKind::Json,
        generated_at: Some(GENERATED_AT.to_string()),
        ..Request::default()
    }
}

/// The spreadsheets with golden files: one of each format and the main kinds of logic
/// (engineering chains, folded add-in calls, data tables, LAMBDA and spills, threaded
/// comments, OpenFormula across sheets, pivot tables) and the sheet layouts (titles and a
/// two-row header, group rows, a named range beside a table, two tables on one sheet).
const GOLDEN_OFFICE: [&str; 12] = [
    "tunnel_design_calculations.xlsx",
    "NST-EST2023-POP.xlsx",
    "table_1_01.xlsx",
    "ExcelTables.xlsx",
    "table-multiple.xlsx",
    "ex012.PVT_z_factor.xlsm",
    "one-variable.xlsx",
    "LAMBDA.xlsx",
    "threadedComment.xlsx",
    "pivots.xlsx",
    "formula-across-sheets.ods",
    "testEXCEL_various.xlsb",
];

/// The PDFs with golden files: form logic (a calculation, formats in words, a LibreOffice
/// form, buttons with scripts), an annotation at its passage, two columns, page labels with
/// an outline, a scan that needs vision, and an invoice with a logo.
const GOLDEN_PDF: [&str; 9] = [
    "pdfjs-text-field-calc.pdf",
    "pdfbox-sf1199a-direct-deposit.pdf",
    "pdfjs-annotation-highlight.pdf",
    "pypdf-multicolumn.pdf",
    "qpdf-page-labels-and-outlines.pdf",
    "pdfjs-js-buttons.pdf",
    "pypdf-libreoffice-form.pdf",
    "scan-truman-letter-nara.pdf",
    "mustang-gnuaccounting-invoice-508.pdf",
];

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
    assert!(files.len() >= 50, "corpus not found: {} files", files.len());
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

/// File system facts that exist on some machines and not on others: their lines are left
/// out of a snapshot, so it is the same on every machine.
const MACHINE_KEYS: [&str; 3] = ["file_created: ", "downloaded_from: ", "file_tags: "];

/// Makes a snapshot independent of the checkout location and of the file's file system
/// facts: modification times are replaced, birth time, download address and tags left out.
fn redact(text: &str, corpus: &str) -> String {
    let text = if cfg!(windows) {
        redact_windows_corpus(text, corpus)
    } else {
        text.replace(corpus, "<corpus>")
    };
    let lines: Vec<String> = text
        .lines()
        .filter(|line| !MACHINE_KEYS.iter().any(|key| line.starts_with(key)))
        .map(|line| {
            if line.starts_with("source_date: ") {
                "source_date: <modified>".to_string()
            } else if line.starts_with("file_modified: ") {
                "file_modified: <modified>".to_string()
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

/// The corpus path of [`redact`] on Windows, so a snapshot reads as on Unix. There a quoted
/// value (JSON and frontmatter) holds the path with each backslash doubled; the path is
/// replaced, and the rest of it up to the closing quote gets `/` as its separator.
fn redact_windows_corpus(text: &str, corpus: &str) -> String {
    let quoted = corpus.replace('\\', r"\\");
    let mut pieces = text.split(quoted.as_str());
    let mut out = pieces.next().unwrap_or_default().to_string();
    for piece in pieces {
        let end = piece.find(['"', '\n']).unwrap_or(piece.len());
        out.push_str("<corpus>");
        out.push_str(&piece[..end].replace(r"\\", "/"));
        out.push_str(&piece[end..]);
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
    if let Some(Value::Object(source)) = value.get_mut("source") {
        for key in ["created", "downloaded_from", "file_tags"] {
            source.remove(key);
        }
    }
    shorten(&mut value);
    redact(&serde_json::to_string_pretty(&value).unwrap(), corpus)
}

#[test]
fn corpus_golden() {
    let pipeline = pipeline();
    // The form Rezzer writes: canonical, without the Windows verbatim prefix.
    let corpus = rezzer_pipeline::strip_verbatim(std::fs::canonicalize(corpus_dir()).unwrap());
    let corpus = corpus.to_string_lossy().to_string();
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(corpus_dir().join("snapshots"));
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| {
        for file in corpus_files() {
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            let office = file.parent().is_some_and(|p| p.ends_with("office"));
            if office && !GOLDEN_OFFICE.contains(&name.as_str()) {
                continue;
            }
            let pdf = file.parent().is_some_and(|p| p.ends_with("pdf"));
            if pdf && !GOLDEN_PDF.contains(&name.as_str()) {
                continue;
            }
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

fn sha256_of(file: &Path) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(std::fs::read(file).unwrap()))
}

/// Every pipeline corpus file is listed in MANIFEST.md with its SHA-256, and every row right
/// in a pipeline folder has a file (`office/malformed/` is checked by
/// `office_corpus_matches_manifest`).
#[test]
fn corpus_manifest_matches_the_files() {
    let mut listed = manifest_rows();
    listed.retain(|k, _| {
        PIPELINE_FOLDERS.iter().any(|f| {
            k.strip_prefix(&format!("{f}/"))
                .is_some_and(|rest| !rest.contains('/'))
        })
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

/// Every file in `office/` (including `office/malformed/`, without the facts files) is listed
/// in MANIFEST.md with its full SHA-256, every `office/` row has a file, and no file is over
/// 2 MB. The files are never skipped: a missing file fails the test.
#[test]
fn office_corpus_matches_manifest() {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else if !path.to_string_lossy().ends_with(".facts.json") {
                files.push(path);
            }
        }
    }
    let mut listed = manifest_rows();
    listed.retain(|k, _| k.starts_with("office/"));
    assert_eq!(listed.len(), 51, "expected 51 office rows");
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

/// Every file in `pdf/` (without the facts files) is listed in MANIFEST.md with its full
/// SHA-256, every `pdf/` row has a file, and no file is over 2 MB. The two large files that are
/// not in the repo are listed in `remote.tsv` only: they may be absent, and when present they
/// must match their hash.
#[test]
fn pdf_corpus_matches_manifest() {
    let mut listed = manifest_rows();
    listed.retain(|k, _| k.starts_with("pdf/"));
    assert_eq!(listed.len(), 39, "expected 39 pdf rows");
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus_dir().join("pdf"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.to_string_lossy().ends_with(".facts.json"))
        .collect();
    files.sort();
    let remote = remote_rows();
    let mut problems = Vec::new();
    for file in &files {
        let rel = relative(file);
        if let Some(expected) = remote.get(&rel) {
            if sha256_of(file) != *expected {
                problems.push(format!("{rel}: hash differs from remote.tsv"));
            }
            continue;
        }
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
    for name in [
        "pdf/nist-sp800-53r5.pdf",
        "pdf/arxiv-2408.09869-docling-report.pdf",
    ] {
        if !remote.contains_key(name) {
            problems.push(format!("{name}: not in remote.tsv"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

// The spreadsheet corpus: what each workbook proves. The checks read the model JSON (and the
// Markdown only for what exists only there), so a change of layout does not break them.

/// The model JSON of `office/<name>`.
fn office(name: &str) -> Value {
    run(&pipeline(), &corpus_dir().join("office").join(name)).json
}

fn office_markdown(name: &str) -> String {
    run(&pipeline(), &corpus_dir().join("office").join(name)).markdown
}

/// `Sheet!A1` shape: a sheet name (quoted or not), `!`, and one cell reference (letters, digits).
fn is_cell_anchor(anchor: &str) -> bool {
    anchor.rsplit_once('!').is_some_and(|(sheet, cell)| {
        let letters = cell.chars().take_while(char::is_ascii_uppercase).count();
        let digits = &cell[letters..];
        !sheet.is_empty()
            && letters > 0
            && !digits.is_empty()
            && digits.chars().all(|c| c.is_ascii_digit())
    })
}

/// The cells of a sheet document as (anchor, shown text): every block is walked (inside
/// sections, quotes, callouts and lists too); the cells of tables without a `role` (so the
/// generated Excel table, pivot and chart summary tables are out), paragraphs and headings
/// count when the anchor has the `Sheet!A1` shape and the plain text is not empty. Sorted by
/// anchor. The layout baseline and the layout checks share this helper.
fn collect_cells(doc: &Document) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut push = |anchor: &Option<rezzer_model::Anchor>, inlines: &[rezzer_model::Inline]| {
        if let Some(anchor) = anchor
            && is_cell_anchor(&anchor.0)
        {
            let text = plain_text(inlines);
            if !text.is_empty() {
                out.push((anchor.0.clone(), text));
            }
        }
    };
    let mut stack: Vec<&Block> = doc.body.iter().rev().collect();
    while let Some(block) = stack.pop() {
        match block {
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => stack.extend(blocks.iter().rev()),
            Block::List { items, .. } => stack.extend(items.iter().flatten().rev()),
            Block::Heading {
                inlines, anchor, ..
            }
            | Block::Paragraph {
                inlines, anchor, ..
            } => push(anchor, inlines),
            Block::Table {
                role: None, rows, ..
            } => {
                for cell in rows.iter().flatten() {
                    push(&cell.anchor, &cell.inlines);
                }
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The baseline of `office/<name>` from before the layout change: `cells` as
/// `[[anchor, text], ...]` plus the `formulas`, `parameters` and `rules` facets as model JSON.
fn baseline(name: &str) -> Value {
    let path = corpus_dir().join("baseline").join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: no baseline: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// The spreadsheet files right in `office/`, sorted.
fn office_spreadsheets() -> Vec<PathBuf> {
    corpus_files()
        .into_iter()
        .filter(|f| f.parent().is_some_and(|p| p.ends_with("office")))
        .filter(|f| {
            f.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| ["xlsx", "xlsm", "xlsb", "ods"].contains(&e))
        })
        .collect()
}

/// Writes `tests/corpus/baseline/<file>.json` for every spreadsheet in `office/` with the
/// current reader. Run on demand:
/// `cargo nextest run -p rezzer --test corpus --run-ignored only write_layout_baseline`.
#[test]
#[ignore = "writes tests/corpus/baseline; run on demand"]
fn write_layout_baseline() {
    let pipeline = pipeline();
    let dir = corpus_dir().join("baseline");
    std::fs::create_dir_all(&dir).unwrap();
    for file in office_spreadsheets() {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let json = run(&pipeline, &file).json;
        let doc: Document = serde_json::from_value(json.clone()).unwrap();
        let cells: Vec<Value> = collect_cells(&doc)
            .into_iter()
            .map(|(a, t)| serde_json::json!([a, t]))
            .collect();
        let facet = |n: &str| {
            json.pointer(&format!("/facets/{n}"))
                .cloned()
                .unwrap_or(Value::Array(Vec::new()))
        };
        let out = serde_json::json!({
            "cells": cells,
            "formulas": facet("formulas"),
            "parameters": facet("parameters"),
            "rules": facet("rules"),
        });
        let mut text = serde_json::to_string_pretty(&out).unwrap();
        text.push('\n');
        std::fs::write(dir.join(format!("{name}.json")), text).unwrap();
    }
}

/// Every spreadsheet in `office/` has a baseline with its four parts.
#[test]
fn layout_baseline_covers_the_spreadsheets() {
    for file in office_spreadsheets() {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let b = baseline(&name);
        for key in ["cells", "formulas", "parameters", "rules"] {
            assert!(b[key].is_array(), "{name}: baseline lacks {key}");
        }
    }
}

/// The layout keeps every cell, value and piece of logic: for every spreadsheet with a
/// baseline (XLSX, XLSM, XLSB, ODS), the cells the blocks hold are the baseline's cells, and
/// the formulas, parameters and rules facets are equal. No cell is written as a stray line
/// outside its region.
///
/// One change is allowed: a cell under a merged area that is not the area's top-left cell
/// (calamine gives the values of ODS covered cells) is not shown. It must be stated as a
/// loss: at its own anchor for the first ones of a sheet, then in the sheet's count loss.
#[test]
fn cells_and_logic_unchanged_on_corpus() {
    let pipeline = pipeline();
    let mut failures = Vec::new();
    let mut checked = 0;
    let mut covered_total = 0;
    for file in office_spreadsheets() {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        checked += 1;
        let base = baseline(&name);
        let json = run(&pipeline, &file).json;
        let doc: Document = serde_json::from_value(json.clone()).unwrap();
        let now: Vec<(String, String)> = collect_cells(&doc);
        let before: Vec<(String, String)> = base["cells"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c[0].as_str().unwrap().to_string(),
                    c[1].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let now_set: std::collections::HashSet<&(String, String)> = now.iter().collect();
        // A cell written twice is never allowed (every cell in exactly one block).
        if now_set.len() != now.len() {
            let mut seen = std::collections::HashSet::new();
            let twice: Vec<_> = now.iter().filter(|c| !seen.insert(*c)).take(10).collect();
            failures.push(format!("{name}: cells written twice: {twice:?}"));
        }
        let before_set: std::collections::HashSet<&(String, String)> = before.iter().collect();
        let extra: Vec<_> = now.iter().filter(|c| !before_set.contains(c)).collect();
        let missing: Vec<&(String, String)> =
            before.iter().filter(|c| !now_set.contains(c)).collect();
        // Missing cells stated as covered: by anchor, else by the count loss of their sheet.
        let covered_at = |anchor: &str, text: &str| {
            let value = format!("{text:?}");
            doc.losses.iter().any(|l| {
                l.location == anchor
                    && l.what.contains("under the merged area")
                    && l.what.contains(&value)
            })
        };
        let mut unstated: Vec<&(String, String)> = Vec::new();
        let mut counted: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for c in &missing {
            if covered_at(&c.0, &c.1) {
                covered_total += 1;
                continue;
            }
            let sheet = c.0.rsplit_once('!').map(|(s, _)| s).unwrap_or("");
            *counted.entry(sheet.to_string()).or_default() += 1;
            unstated.push(c);
        }
        for (sheet, n) in counted {
            let stated: u64 = doc
                .losses
                .iter()
                .filter(|l| l.location == sheet)
                .filter_map(|l| {
                    l.what
                        .strip_suffix(
                            " more cells under merged areas hold a value that is not shown",
                        )
                        .and_then(|k| k.parse::<u64>().ok())
                })
                .sum();
            if stated >= n {
                covered_total += n;
                unstated.retain(|c| !c.0.starts_with(&format!("{sheet}!")));
            }
        }
        if !extra.is_empty() || !unstated.is_empty() {
            failures.push(format!(
                "{name}: cells differ ({} now, {} before); missing {:?}; extra {:?}",
                now.len(),
                before.len(),
                unstated.iter().take(10).collect::<Vec<_>>(),
                extra.iter().take(10).collect::<Vec<_>>()
            ));
        }
        for facet in ["formulas", "parameters", "rules"] {
            let now = json
                .pointer(&format!("/facets/{facet}"))
                .cloned()
                .unwrap_or(Value::Array(Vec::new()));
            // The baseline went through one more print and parse; serde_json's float parse
            // is not exact, so the facet takes the same way before the comparison.
            let mut now: Value =
                serde_json::from_str(&serde_json::to_string(&now).unwrap()).unwrap();
            let mut base_facet = base[facet].clone();
            // Parameter labels are written as paths since spec 8.7 (D2); the rest is equal.
            if facet == "parameters" {
                now = without_label_and_path(&now);
                base_facet = without_label_and_path(&base_facet);
            }
            let base = &base_facet;
            if now != *base {
                let (a, b) = (now.as_array().unwrap(), base.as_array().unwrap());
                let first = a.iter().zip(b).find(|(x, y)| x != y);
                failures.push(format!(
                    "{name}: {facet} differ ({} now, {} before); first {first:?}",
                    a.len(),
                    b.len()
                ));
            }
        }
        let stray = json.to_string().matches("\"role\":\"cell\"").count();
        if stray > 0 {
            failures.push(format!("{name}: {stray} cells written as stray lines"));
        }
    }
    eprintln!("{checked} files checked, {covered_total} covered cells stated as losses");
    assert!(checked >= 48, "only {checked} files checked");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Parameters as model JSON without their `label` and `path`.
fn without_label_and_path(list: &Value) -> Value {
    let mut list = list.clone();
    for p in list.as_array_mut().unwrap() {
        let p = p.as_object_mut().unwrap();
        p.remove("label");
        p.remove("path");
    }
    list
}

/// For every corpus workbook the parameters equal the baseline of Task 1 in anchor, name,
/// value, unit, unit source and read count, in the same order: only `label` and `path`
/// may differ (spec 8.7, D2). A label is the path joined with " / ".
#[test]
fn parameter_values_and_cells_unchanged() {
    let pipeline = pipeline();
    let mut failures = Vec::new();
    let mut total = 0;
    for file in office_spreadsheets() {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let base = baseline(&name);
        let json = run(&pipeline, &file).json;
        let now = json
            .pointer("/facets/parameters")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
        let now: Value = serde_json::from_str(&serde_json::to_string(&now).unwrap()).unwrap();
        total += now.as_array().unwrap().len();
        if without_label_and_path(&now) != without_label_and_path(&base["parameters"]) {
            failures.push(format!("{name}: parameters differ beyond label and path"));
        }
        for p in now.as_array().unwrap() {
            let path: Vec<&str> = list(p, "/path").iter().filter_map(Value::as_str).collect();
            let joined = (!path.is_empty()).then(|| path.join(" / "));
            if p["label"].as_str() != joined.as_deref() {
                failures.push(format!("{name}: label is not the joined path: {p}"));
            }
            if path.iter().any(|part| part.trim().is_empty()) {
                failures.push(format!("{name}: empty path part: {p}"));
            }
        }
    }
    eprintln!("{total} parameters checked");
    assert!(total >= 1_000, "only {total} parameters checked");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn list<'a>(doc: &'a Value, pointer: &str) -> &'a [Value] {
    doc.pointer(pointer)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// The facet entry (formula, parameter, annotation) with this anchor.
fn at<'a>(doc: &'a Value, facet: &str, anchor: &str) -> &'a Value {
    list(doc, &format!("/facets/{facet}"))
        .iter()
        .find(|v| v["anchor"] == anchor)
        .unwrap_or_else(|| panic!("no {facet} entry at {anchor}"))
}

/// Cell formulas: everything in the formulas facet except defined names and LAMBDA names.
fn cell_formulas(doc: &Value) -> Vec<&Value> {
    list(doc, "/facets/formulas")
        .iter()
        .filter(|f| f["kind"] != "name" && f["kind"] != "lambda")
        .collect()
}

fn refs(f: &Value) -> Vec<&str> {
    list(f, "/references")
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

fn number(v: &Value) -> Option<f64> {
    (v["type"] == "number")
        .then(|| v["value"].as_f64())
        .flatten()
}

/// The status text a sheet section starts with (`hidden`, `very hidden`), if any.
fn sheet_status<'a>(doc: &'a Value, title: &str) -> Option<&'a str> {
    let section = list(doc, "/body")
        .iter()
        .find(|s| s["title"] == title)
        .unwrap_or_else(|| panic!("no sheet {title}"));
    let first = section.pointer("/blocks/0/inlines/0")?;
    (first["type"] == "status").then(|| first["text"].as_str())?
}

#[test]
fn tunnel_design_parameters_and_units() {
    let doc = office("tunnel_design_calculations.xlsx");
    let sheets: Vec<&str> = list(&doc, "/body")
        .iter()
        .filter_map(|s| s["title"].as_str())
        .collect();
    assert_eq!(
        sheets,
        [
            "Design_Basis",
            "Structural_Lining",
            "Ventilation",
            "Drainage"
        ]
    );
    let formulas = cell_formulas(&doc);
    assert_eq!(formulas.len(), 86);
    // The formulas sit on the three calculation sheets; the design basis is text only.
    for sheet in ["Structural_Lining!", "Ventilation!", "Drainage!"] {
        assert!(
            formulas
                .iter()
                .any(|f| f["anchor"].as_str().unwrap().starts_with(sheet))
        );
    }
    assert!(
        !formulas
            .iter()
            .any(|f| f["anchor"].as_str().unwrap().starts_with("Design_Basis!"))
    );
    // Inputs with their label under the region's caption `INPUTS` and the unit in the cell
    // beside them.
    for (anchor, label, value, unit) in [
        (
            "Structural_Lining!B5",
            "INPUTS / Excavated diameter, D",
            10.0,
            "m",
        ),
        (
            "Structural_Lining!B6",
            "INPUTS / Trial lining thickness, t",
            0.35,
            "m",
        ),
        (
            "Structural_Lining!B7",
            "INPUTS / Overburden depth, H",
            25.0,
            "m",
        ),
        (
            "Structural_Lining!B8",
            "INPUTS / Unit weight of ground, gamma",
            22.0,
            "kN/m3",
        ),
        ("Ventilation!B5", "INPUTS / Tunnel length, L", 800.0, "m"),
        ("Drainage!B12", "INPUTS / Trial pipe diameter", 0.3, "m"),
    ] {
        let p = at(&doc, "parameters", anchor);
        assert_eq!(p["label"], label, "{anchor}");
        assert_eq!(number(&p["value"]), Some(value), "{anchor}");
        assert_eq!(p["unit"], unit, "{anchor}");
        assert_eq!(p["unit_source"], "unit-cell", "{anchor}");
    }
    // The trial thickness column takes label and unit from its header `t (m)`, under the
    // title of its table.
    let t = at(&doc, "parameters", "Structural_Lining!E49");
    assert_eq!(
        t["label"],
        "LINING THICKNESS SENSITIVITY (trial t vs Factor of Safety) / t"
    );
    assert_eq!(t["unit"], "m");
    assert_eq!(t["unit_source"], "header");
    // A dimensionless input gets no unit.
    let fos = at(&doc, "parameters", "Structural_Lining!B13");
    assert_eq!(fos["label"], "INPUTS / Target Factor of Safety");
    assert!(fos.get("unit").is_none());
    // The safety check is a result, with the cached verdict.
    let status = at(&doc, "formulas", "Structural_Lining!B44");
    assert_eq!(status["role"], "result");
    assert_eq!(status["cached"]["value"], "NOT SAFE - increase thickness");
    assert_eq!(
        refs(status),
        ["Structural_Lining!B43", "Structural_Lining!B13"]
    );
    // Known limit: the sensitivity rows F49:F72 read the fixed cells B42, B30 and B33
    // without `$`, so their R1C1 keys differ and they do not fold. Only B30 and B33 (the
    // same formula twice) share a pattern today.
    let patterns: std::collections::BTreeSet<&str> = formulas
        .iter()
        .filter_map(|f| f["pattern"].as_str())
        .collect();
    assert_eq!(patterns.len(), 1, "{patterns:?}");
    assert!(
        at(&doc, "formulas", "Structural_Lining!F49")
            .get("pattern")
            .is_none()
    );
}

#[test]
fn pvt_z_factor_folds_into_patterns() {
    let doc = office("ex012.PVT_z_factor.xlsm");
    let formulas = cell_formulas(&doc);
    assert_eq!(formulas.len(), 2_133);
    // One row per pattern plus one per formula that is not copied.
    let patterns: std::collections::BTreeSet<&str> = formulas
        .iter()
        .filter_map(|f| f["pattern"].as_str())
        .collect();
    let single = formulas
        .iter()
        .filter(|f| f.get("pattern").is_none())
        .count();
    let rows = patterns.len() + single;
    assert!(
        rows <= 12,
        "{rows} rows: {} patterns, {single} single",
        patterns.len()
    );
    eprintln!(
        "pvt: 2133 formulas in {rows} rows ({} patterns)",
        patterns.len()
    );
    // The gas gravity and the correlation number are named inputs that every z-factor,
    // dZ/dT and dZ/dp call reads.
    for (anchor, name, value) in [("z!L3", "gg", 0.75), ("z!L4", "zcor", 2.0)] {
        let p = at(&doc, "parameters", anchor);
        assert_eq!(p["name"], name);
        assert_eq!(number(&p["value"]), Some(value));
        assert_eq!(p["read_by"], 2_016, "{name}");
    }
    let z = at(&doc, "formulas", "z!F38");
    assert_eq!(z["expression"], "=[1]!unf_pvt_Zgas_d($D38,F$36,gg,zcor)");
    assert!(refs(z).contains(&"name:gg") && refs(z).contains(&"name:zcor"));
    // The add-in that holds the functions is listed with its path, never opened.
    let addins: Vec<&Value> = list(&doc, "/facets/embedded")
        .iter()
        .filter(|e| e["kind"] == "add-in")
        .collect();
    assert!(!addins.is_empty());
    for a in &addins {
        assert_eq!(a["name"], "UniflocVBA_7.xlam");
        assert!(a["target"].as_str().unwrap().ends_with("UniflocVBA_7.xlam"));
        assert!(a["detail"].as_str().unwrap().contains("never opened"));
    }
    let md = office_markdown("ex012.PVT_z_factor.xlsm");
    let hidden = md
        .split("### Hidden logic")
        .nth(1)
        .expect("a Hidden logic section");
    assert!(
        hidden.contains("| add-in | UniflocVBA\\_7.xlam |"),
        "{hidden}"
    );
}

#[test]
fn friction_factor_udfs_listed() {
    let doc = office("ex054.MF_friction_factor.xlsm");
    let udfs: Vec<&str> = list(&doc, "/facets/embedded")
        .iter()
        .filter(|e| e["kind"] == "user-function")
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert_eq!(udfs, ["[1]!unf_version", "[1]!unf_friction_factor"]);
    for e in list(&doc, "/facets/embedded") {
        if e["kind"] == "user-function" {
            assert_eq!(e["detail"], "/unifloc/unifloc_vba/UniflocVBA_7.xlam");
        }
    }
    // The VBA modules are named, never run.
    let macros: Vec<&str> = list(&doc, "/facets/embedded")
        .iter()
        .filter(|e| e["kind"] == "macro")
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert_eq!(macros, ["Module1", "Лист1", "ЭтаКнига"]);
    let f = at(&doc, "formulas", "Лист1!D17");
    assert_eq!(
        f["expression"],
        "=[1]!unf_friction_factor($C17,D$15,type,smooth_transition)"
    );
    // The named switches of the user function are parameters with their label.
    let kind = at(&doc, "parameters", "Лист1!E7");
    assert_eq!(kind["name"], "type");
    assert_eq!(kind["label"], "номер корреляции");
    assert_eq!(kind["read_by"], 115);
    let md = office_markdown("ex054.MF_friction_factor.xlsm");
    assert!(md.contains("used in 115 formulas"));
}

#[test]
fn convert_units_found() {
    // CONVERT.xlsx converts the literal 1 (and 0.05) from the unit code in column B to the
    // first code of each sheet: `CONVERT(1,B2,$B$2)`. The codes are text cells with their
    // unit name beside them. No number in the file carries a unit, so no parameter claims one.
    let doc = office("CONVERT.xlsx");
    let converts = cell_formulas(&doc)
        .into_iter()
        .filter(|f| f["expression"].as_str().unwrap().contains("CONVERT("))
        .count();
    assert!(converts >= 300, "{converts}");
    let c = at(&doc, "formulas", "Weight!C3");
    assert_eq!(c["expression"], "=CONVERT(1,B3,$B$2)");
    assert_eq!(refs(c), ["Weight!B3", "Weight!B2"]);
    // The label is the column header, then the unit name on the left that names the row.
    for (anchor, label, code) in [
        ("Weight!B2", "Gram", "g"),
        ("Weight!B4", "Pound mass (avoirdupois)", "lbm"),
        ("distance!B2", "Meter", "m"),
        ("distance!B6", "Foot", "ft"),
    ] {
        let p = at(&doc, "parameters", anchor);
        assert_eq!(
            p["label"],
            format!("From_unit or to_unit / {label}"),
            "{anchor}"
        );
        assert_eq!(p["value"]["value"], code, "{anchor}");
    }
    assert!(at(&doc, "parameters", "Weight!B2")["read_by"].as_u64() > Some(1));
    for p in list(&doc, "/facets/parameters") {
        assert!(p.get("unit").is_none(), "{p}");
    }
}

#[test]
fn data_table_input_cells() {
    let doc = office("one-variable.xlsx");
    let column = at(&doc, "formulas", "'One-Var'!B5");
    assert_eq!(column["kind"], "data_table");
    assert_eq!(column["range"], "'One-Var'!B5:B11");
    assert_eq!(column["expression"], "=TABLE(,A2)");
    assert_eq!(refs(column), ["'One-Var'!A2"]);
    assert!(column["note"].as_str().unwrap().contains("column input A2"));
    let row = at(&doc, "formulas", "'One-Var'!E5");
    assert_eq!(row["kind"], "data_table");
    assert_eq!(row["range"], "'One-Var'!E5:I5");
    assert_eq!(refs(row), ["'One-Var'!B2"]);
    assert!(row["note"].as_str().unwrap().contains("row input B2"));
    // The input cells are parameters.
    assert_eq!(
        number(&at(&doc, "parameters", "'One-Var'!A2")["value"]),
        Some(1.0)
    );
}

#[test]
fn dynamic_array_spill_ranges() {
    let doc = office("DynamicArrays.xlsx");
    for (anchor, kind, range, expression) in [
        (
            "DynamicArrays!A3",
            "spill",
            "DynamicArrays!A3:B12",
            "=SEQUENCE(10,2,5,2)",
        ),
        (
            "DynamicArrays!F3",
            "spill",
            "DynamicArrays!F3:F5",
            "=TAKE(ANCHORARRAY(A3),3,1)",
        ),
        (
            "DynamicArrays!K3",
            "spill",
            "DynamicArrays!K3:K8",
            "=FILTER(A3:A12,  B3:B12>20)",
        ),
        (
            "DynamicArrays!A19",
            "spill",
            "DynamicArrays!A19:C21",
            "=RANDARRAY(3,3, 0, 100,TRUE)",
        ),
        (
            "ArrayFormulas!A3",
            "array",
            "ArrayFormulas!A3:D3",
            "=SIN({1,2,3,4,5,6,7,8,9})",
        ),
    ] {
        let f = at(&doc, "formulas", anchor);
        assert_eq!(f["kind"], kind, "{anchor}");
        assert_eq!(f["range"], range, "{anchor}");
        assert_eq!(f["expression"], expression, "{anchor}");
    }
    // The storage prefixes are kept as the original text.
    assert_eq!(
        at(&doc, "formulas", "DynamicArrays!K3")["original"],
        "=_xlfn._xlws.FILTER(A3:A12,  B3:B12>20)"
    );
    // The cells of a spill are not formulas of their own.
    assert_eq!(cell_formulas(&doc).len(), 6);
}

#[test]
fn lambda_names_with_params() {
    let doc = office("LAMBDA.xlsx");
    let count_words = at(&doc, "formulas", "name:CountWords");
    assert_eq!(count_words["kind"], "lambda");
    assert_eq!(count_words["note"], "parameters: text");
    let temperature = at(&doc, "formulas", "name:Temperature");
    assert_eq!(temperature["kind"], "lambda");
    assert_eq!(temperature["expression"], "LAMBDA(temp, (5/9) * (temp-32))");
    // The name's comment follows its parameters.
    assert_eq!(temperature["note"], "parameters: temp; Fahrenheit");
    // A call of a workbook lambda is not an unknown user function.
    assert!(
        !list(&doc, "/facets/embedded")
            .iter()
            .any(|e| e["kind"] == "user-function")
    );
    let call = at(&doc, "formulas", "Sheet1!B5");
    assert_eq!(call["expression"], "=Temperature(A5)");
    assert_eq!(number(&call["cached"]).map(|v| v.round()), Some(-18.0));
}

#[test]
fn threaded_comment_person_and_reply() {
    let doc = office("threadedComment.xlsx");
    let thread = list(&doc, "/facets/annotations")
        .iter()
        .find(|a| a["kind"] == "comment")
        .expect("a threaded comment");
    assert_eq!(thread["anchor"], "Sheet1!A2");
    assert_eq!(thread["author"], "Mike Kaganski");
    assert_eq!(thread["date"], "2026-03-24T09:25:49.80");
    assert_eq!(thread["resolved"], false);
    assert_eq!(thread["body"][0]["text"], "a comment on A2");
    let replies = list(thread, "/replies");
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["author"], "Mike Kaganski");
    assert_eq!(replies[0]["date"], "2026-04-07T14:14:05.49");
    assert_eq!(replies[0]["body"][0]["text"], "A reply");
    // The legacy note on A5 stays a note.
    let note = at(&doc, "annotations", "Sheet1!A5");
    assert_eq!(note["kind"], "note");
}

#[test]
fn hidden_and_very_hidden_sheets() {
    // No public test file has a very hidden sheet (state="veryHidden"); the reader tests
    // cover it with a built workbook. ODF has no very hidden state: the sheet named
    // VeryHidden is display="false", like Hidden.
    let doc = office("hidden_sheets.xlsx");
    assert_eq!(sheet_status(&doc, "Sheet1"), Some("hidden"));
    assert_eq!(sheet_status(&doc, "Sheet2"), None);
    assert_eq!(sheet_status(&doc, "Sheet3"), Some("hidden"));
    let ods = office("any_sheets.ods");
    assert_eq!(sheet_status(&ods, "Visible"), None);
    assert_eq!(sheet_status(&ods, "Hidden"), Some("hidden"));
    assert_eq!(sheet_status(&ods, "VeryHidden"), Some("hidden"));
    let facts = list(&ods, "/overview/facts");
    assert!(
        facts
            .iter()
            .any(|f| f[0] == "Sheets" && f[1] == "4 (hidden: Hidden, VeryHidden)"),
        "{facts:?}"
    );
}

#[test]
fn validation_rules_count() {
    let doc = office("DataValidations-49244.xlsx");
    let rules = list(&doc, "/facets/rules");
    assert_eq!(rules.len(), 52);
    assert!(rules.iter().all(|r| r["kind"] == "validation"));
}

#[test]
fn multi_sheet_3d_refs() {
    let doc = office("55906-MultiSheetRefs.xlsx");
    let sum = at(&doc, "formulas", "Sheet1!A3");
    assert_eq!(sum["expression"], "=SUM(Sheet1:Sheet3!A1)");
    assert_eq!(refs(sum), ["Sheet1:Sheet3!A1"]);
    assert_eq!(number(&sum["cached"]), Some(66.0));
    assert_eq!(
        refs(at(&doc, "formulas", "Sheet1!I6")),
        ["Sheet1:Sheet3!A1:B2"]
    );
    // A 3D reference reads every sheet between its ends: the middle sheet's cells are inputs.
    for anchor in ["Sheet1!A1", "Sheet2!A1", "Sheet3!A1"] {
        assert_eq!(at(&doc, "parameters", anchor)["read_by"], 10, "{anchor}");
    }
    assert_eq!(
        number(&at(&doc, "parameters", "Sheet2!A1")["value"]),
        Some(22.0)
    );
}

#[test]
fn external_link_target() {
    let doc = office("external-refs.xlsx");
    let link = list(&doc, "/facets/embedded")
        .iter()
        .find(|e| e["kind"] == "external-workbook")
        .expect("the linked workbook");
    assert_eq!(link["name"], "data.xlsx");
    assert_eq!(link["target"], "data.xlsx");
    assert_eq!(link["detail"], "[1] in formulas, never opened");
    let f = at(&doc, "formulas", "Sheet1!A1");
    assert_eq!(f["expression"], "=[1]Sheet1!A1");
    assert_eq!(refs(f), ["ext:[1]Sheet1!A1"]);
    // The value comes from the cache in the file, the linked file is never read.
    assert_eq!(f["cached"]["value"], "Name");
}

#[test]
fn custom_props_count() {
    let doc = office("testEXCEL_custom_props.xlsx");
    let custom = doc["properties"]["custom"].as_object().unwrap();
    // The reader adds the application and the calculation settings; the rest are the five
    // custom properties of the file, with their type.
    let own: Vec<&str> = custom
        .keys()
        .map(String::as_str)
        .filter(|k| !["application", "calc_mode", "iterative_calculation"].contains(k))
        .collect();
    assert_eq!(own.len(), 5, "{own:?}");
    for (key, kind, value) in [
        ("MyCustomString", "text", serde_json::json!("MyStringValue")),
        ("myCustomNumber", "number", serde_json::json!(3.0)),
        ("myCustomBoolean", "bool", serde_json::json!(true)),
        (
            "MyCustomDate",
            "date_time",
            serde_json::json!("2010-12-30T22:00:00Z"),
        ),
        (
            "myCustomSecondDate",
            "date_time",
            serde_json::json!("2010-12-29T22:00:00Z"),
        ),
    ] {
        assert_eq!(custom[key]["type"], kind, "{key}");
        assert_eq!(custom[key]["value"], value, "{key}");
    }
}

#[test]
fn ods_formulas_across_sheets() {
    let doc = office("formula-across-sheets.ods");
    let formulas = cell_formulas(&doc);
    assert_eq!(formulas.len(), 23);
    assert!(formulas.iter().all(|f| f["dialect"] == "openformula"));
    let b4 = at(&doc, "formulas", "Calculation!B4");
    assert_eq!(b4["expression"], "=Data!A3");
    assert_eq!(b4["original"], "of:=[$Data.A3]");
    assert_eq!(refs(b4), ["Data!A3"]);
    let back = at(&doc, "formulas", "Data!A4");
    assert_eq!(refs(back), ["Calculation!B7"]);
    // Named ranges are read by the formulas that use them.
    let b9 = at(&doc, "formulas", "Calculation!B9");
    assert_eq!(b9["expression"], "=L_nb/(1+Xnb)");
    assert_eq!(refs(b9), ["name:L_nb", "name:Xnb"]);
    assert_eq!(
        at(&doc, "formulas", "name:L_nb")["references"][0],
        "Calculation!E7"
    );
}

#[test]
fn ods_annotation_author_and_date() {
    let doc = office("with-annotation.ods");
    let a = at(&doc, "annotations", "table1!A1");
    assert_eq!(a["kind"], "comment");
    assert_eq!(a["author"], "Marc Schreiber");
    assert_eq!(a["date"], "2024-09-18T18:27:00");
    assert_eq!(a["body"][0]["text"], "This is annotation of a.1");
}

/// The parser's own refusals of deep or huge formulas (formualizer limits).
fn is_limit_refusal(note: &str) -> bool {
    [
        "nesting too deep",
        "AST height limit exceeded",
        "AST node limit exceeded",
        "byte limit exceeded",
    ]
    .iter()
    .any(|m| note.contains(m))
}

#[test]
fn all_corpus_formulas_parse() {
    let pipeline = pipeline();
    let mut cells = 0;
    let mut names = 0;
    let mut failures = Vec::new();
    let mut limit_refusals = 0;
    let mut names_without_text = Vec::new();
    for file in corpus_files()
        .into_iter()
        .filter(|f| f.parent().is_some_and(|p| p.ends_with("office")))
    {
        let file_name = file.file_name().unwrap().to_string_lossy().to_string();
        let doc = run(&pipeline, &file).json;
        let without_text = names_without_text.len();
        for f in list(&doc, "/facets/formulas") {
            let anchor = f["anchor"].as_str().unwrap_or("");
            let name = f["kind"] == "name" || f["kind"] == "lambda";
            let note = f["note"].as_str().unwrap_or("");
            if name && f["expression"] == "" {
                // A name the file gives no formula text for (XLSB add-in names): nothing to parse.
                names_without_text.push(format!("{file_name} {anchor}"));
                continue;
            }
            if name {
                names += 1;
            } else {
                cells += 1;
            }
            if is_limit_refusal(note) {
                limit_refusals += 1;
            }
            if note.contains("formula not readable")
                || note.contains("not analyzed")
                || is_limit_refusal(note)
            {
                failures.push(format!("{file_name} {anchor}: {note}"));
            }
        }
        let without_text = names_without_text.len() - without_text;
        let skipped_names = match without_text {
            0 => String::new(),
            1 => "1 defined name could not be analyzed".to_string(),
            n => format!("{n} defined names could not be analyzed"),
        };
        for loss in list(&doc, "/losses") {
            let what = loss["what"].as_str().unwrap_or("");
            // The logic summary counts the names without text too; they are listed above.
            let what_else = what.replace(&skipped_names, "");
            if what_else.contains("formula not readable")
                || what_else.contains("could not be analyzed")
            {
                failures.push(format!("{file_name} loss: {what}"));
            }
        }
    }
    eprintln!(
        "spreadsheet corpus: {cells} cell formulas and {names} names parsed, \
         {limit_refusals} refused by the parser limits, {} failed; \
         names without text: {names_without_text:?}",
        failures.len()
    );
    assert_eq!(cells, 3_477, "cell formulas in the spreadsheet corpus");
    assert!(
        failures.is_empty(),
        "{} formulas do not parse ({limit_refusals} by the parser limits):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// The PDF corpus: what each file proves. As for the spreadsheets, the checks read the model
// JSON, and the Markdown only for what exists only there.

/// The model JSON and the Markdown of `pdf/<name>`.
fn pdf(name: &str) -> Output {
    run(&pipeline(), &corpus_dir().join("pdf").join(name))
}

fn texts<'a>(values: &'a [Value], key: &str) -> Vec<&'a str> {
    values.iter().filter_map(|v| v[key].as_str()).collect()
}

/// Maximum resident set size of the process: the PDF reader works on more than one thread,
/// so the heap of the calling thread is not enough. It is the figure of one test only under
/// nextest, which runs every test in its own process.
#[cfg(unix)]
fn max_rss_bytes() -> u64 {
    // SAFETY: getrusage fills the struct we pass.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let v = ru.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        v
    } else {
        v * 1024
    }
}

#[test]
fn irs_form_fields_and_xfa() {
    let doc = pdf("irs-f1040-2025.pdf").json;
    let fields = list(&doc, "/facets/fields");
    // Every field of the form is a terminal field: the widgets that hold a value.
    assert_eq!(fields.len(), 199);
    assert!(fields.iter().all(|f| {
        f["name"]
            .as_str()
            .unwrap()
            .starts_with("topmostSubform[0].Page")
    }));
    let kinds: std::collections::BTreeMap<&str, usize> =
        fields.iter().fold(Default::default(), |mut m, f| {
            *m.entry(f["kind"].as_str().unwrap()).or_default() += 1;
            m
        });
    assert_eq!(kinds, [("checkbox", 73), ("text", 126)].into());
    // The XFA data is said, not read.
    assert!(
        list(&doc, "/losses").iter().any(
            |l| l["what"] == "XFA form data present, not read" && l["why"] == "not_implemented"
        ),
        "{:?}",
        doc["losses"]
    );
    // Three document scripts, listed with their text, never run.
    let scripts: Vec<&Value> = list(&doc, "/facets/embedded")
        .iter()
        .filter(|e| e["kind"] == "script")
        .collect();
    assert_eq!(scripts.len(), 3);
    assert!(scripts.iter().all(|s| s["name"] == "document"));
    assert!(
        scripts
            .iter()
            .all(|s| s["text"].as_str().is_some_and(|t| !t.is_empty()))
    );
    assert!(scripts[2]["detail"].as_str().unwrap().contains("XFACheck"));
    // The usage-rights signature covers the whole file. It is listed, not counted as signed.
    let sigs = list(&doc, "/facets/signatures");
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0]["kind"], "usage-rights");
    assert_eq!(sigs[0]["covers_whole_file"], true);
    assert_eq!(sigs[0]["later_revisions"], 0);
    let facts = &doc["properties"]["facts"];
    assert_eq!(facts["signatures"], 1);
    assert_eq!(facts["signed"], false);
    // The file is linearized: the first-page section and the main section are one save, the
    // signature is the second.
    assert_eq!(facts["revisions"], 2);
    let timeline = list(&doc, "/facets/timeline");
    assert_eq!(timeline.len(), 2);
    assert!(timeline[1]["text"].as_str().unwrap().contains("(Sig)"));
}

#[test]
fn sf1199a_formats_in_words() {
    let doc = pdf("pdfbox-sf1199a-direct-deposit.pdf").json;
    let format = |name: &str| {
        list(&doc, "/facets/fields")
            .iter()
            .find(|f| f["name"] == name)
            .unwrap_or_else(|| panic!("no field {name}"))["format"]
            .clone()
    };
    assert_eq!(format("Phone"), "phone number");
    assert_eq!(format("Bank Rep Phone"), "phone number");
    assert_eq!(
        format("Allottment Amount"),
        "number, 2 decimals, \"$\" before the number"
    );
    assert_eq!(format("Check Digit"), "number, 0 decimals");
    assert_eq!(
        format("Prefix for Claim or Payrooo ID Number"),
        "number, 0 decimals, no thousands separator"
    );
    assert_eq!(format("Bank Rep Date"), "date mm/dd/yyyy");
    assert_eq!(format("Name"), Value::Null);
    // The words come from the AF calls, which stay as the script text.
    let date = list(&doc, "/facets/fields")
        .iter()
        .find(|f| f["name"] == "Bank Rep Date")
        .unwrap();
    assert_eq!(date["actions"][0]["trigger"], "format");
    assert_eq!(
        date["actions"][0]["script"],
        "AFDate_FormatEx(\"mm/dd/yyyy\");"
    );
    assert_eq!(date["actions"][0]["words"], "date mm/dd/yyyy");
}

#[test]
fn calc_order_file() {
    let doc = pdf("pdfjs-acroform-calculation-order.pdf").json;
    let fields = list(&doc, "/facets/fields");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["name"], "group");
    assert_eq!(fields[0]["calc_order"], 1);
    // The calculation order of the text-field file comes with its rule.
    let out = pdf("pdfjs-text-field-calc.pdf");
    let mirror = list(&out.json, "/facets/fields")
        .iter()
        .find(|f| f["name"] == "Mirror")
        .unwrap();
    assert_eq!(mirror["calc_order"], 1);
    assert_eq!(mirror["role"], "result");
    let formula = at(&out.json, "formulas", "field:Mirror");
    assert_eq!(formula["dialect"], "pdf-js");
    assert_eq!(refs(formula), ["field:Source"]);
    assert!(out.markdown.contains("Mirror, calc 1:"), "{}", out.markdown);
}

#[test]
fn annotations_by_author() {
    let tim = "Tim van der Meij";
    for (file, kind, author, date, quoted, body) in [
        (
            "pdfjs-annotation-highlight.pdf",
            "highlight",
            tim,
            "2015-12-30T15:22:48+01:00",
            Some("Lorem ipsum dolor sit amet"),
            Some("Highlight content"),
        ),
        (
            "pdfjs-annotation-strikeout.pdf",
            "strikeout",
            tim,
            "2015-12-29T15:05:16+01:00",
            Some("Lorem ipsum dolor sit amet"),
            Some("StrikeOut content"),
        ),
        (
            "pdfjs-annotation-underline.pdf",
            "underline",
            tim,
            "2015-12-28T00:27:36+01:00",
            Some("Lorem ipsum dolor sit amet"),
            Some("Underline content"),
        ),
        (
            "pdfjs-annotation-caret-ink.pdf",
            "caret",
            tim,
            "2015-12-20T23:46:04+01:00",
            None,
            Some("Caret content"),
        ),
        (
            "pdfjs-annotation-caret-ink.pdf",
            "ink",
            tim,
            "2015-12-20T23:46:43+01:00",
            None,
            Some("Ink content"),
        ),
        (
            "pdfjs-annotation-freetext.pdf",
            "free-text",
            tim,
            "2015-12-20T23:32:13+01:00",
            None,
            Some("FreeText content"),
        ),
        (
            "pdfjs-annotation-link-text-popup.pdf",
            "note",
            tim,
            "2015-12-20T23:37:35+01:00",
            None,
            Some("Text content"),
        ),
        (
            "pypdf-annotated.pdf",
            "ink",
            "Lucas",
            "",
            None,
            Some("Hello world!"),
        ),
    ] {
        let out = pdf(file);
        let a = list(&out.json, "/facets/annotations")
            .iter()
            .find(|a| a["kind"] == kind)
            .unwrap_or_else(|| panic!("{file}: no {kind}"));
        assert_eq!(a["author"], author, "{file} {kind}");
        if !date.is_empty() {
            assert_eq!(a["date"], date, "{file} {kind}");
        }
        assert_eq!(a["quoted"].as_str(), quoted, "{file} {kind}");
        assert_eq!(a["body"][0]["text"].as_str(), body, "{file} {kind}");
        // Every annotation sits at a block of its page, and the Markdown shows it there.
        let anchor = a["anchor"].as_str().unwrap();
        assert!(anchor.starts_with("p1:b"), "{file}: {anchor}");
        let mut words = kind.replace('-', " ");
        words[..1].make_ascii_uppercase();
        let head = format!("> [!NOTE] {} by {author}", words.replace(' ', "-"));
        assert!(
            out.markdown.contains(&head),
            "{file}: {head}\n{}",
            out.markdown
        );
    }
    // pypdf's highlight spans two lines and has no author; its date has no zone.
    let doc = pdf("pypdf-annotated.pdf").json;
    let hl = list(&doc, "/facets/annotations")
        .iter()
        .find(|a| a["kind"] == "highlight")
        .unwrap();
    assert_eq!(hl["quoted"], "Line 1 Line 2");
    assert!(hl.get("author").is_none());
    assert_eq!(hl["date"], "1990-04-28T00:00:00");
    assert_eq!(
        texts(list(&doc, "/facets/annotations"), "kind"),
        ["note", "highlight", "ink"]
    );
}

/// The text of every list item of the outline section, depth first.
fn outline_items(doc: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if m.get("type").and_then(Value::as_str) == Some("paragraph") {
                    let text: String = list(v, "/inlines")
                        .iter()
                        .filter_map(|i| i["text"].as_str())
                        .collect();
                    out.push(text);
                }
                m.values().for_each(|v| walk(v, out));
            }
            Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let outline = list(doc, "/body")
        .iter()
        .find(|s| s["anchor"] == "outline")
        .expect("an outline section");
    let mut out = Vec::new();
    walk(&outline["blocks"], &mut out);
    out
}

#[test]
fn outline_and_labels() {
    let out = pdf("qpdf-page-labels-and-outlines.pdf");
    let items = outline_items(&out.json);
    assert_eq!(items.len(), 11, "{items:?}");
    // An outline item names the page by its label; when the label is not unique, the
    // physical page follows.
    assert_eq!(items[0], "Isís 1 -> 5: /XYZ null null null, page iv");
    assert_eq!(
        items[2],
        "Isosicle 1.1.1 -> 12: /FitV 100, page 2 (page 13 of the file)"
    );
    assert_eq!(items[10], "Trepak 2 -> 15: /XYZ 66 756 3, page 6");
    // Every page section carries its label and its physical number.
    let markers: Vec<String> = list(&out.json, "/body")
        .iter()
        .filter(|s| s["kind"] == "page")
        .map(|s| {
            s["blocks"][0]["inlines"][0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(markers.len(), 30);
    assert_eq!(
        markers[..5],
        [
            "Page 1",
            "Page 2",
            "Page i (3)",
            "Page ii (4)",
            "Page iii (5)"
        ]
    );
    assert_eq!(markers[12], "Page 2 (13)");
    // Internal links never render as Markdown links: no target exists.
    let section = out.markdown.split("## Outline").nth(1).unwrap();
    let section = section.split(":status[Page 1]").next().unwrap();
    assert!(!section.contains("]("), "{section}");
    // pypdf's outline is flat, nine items over pages 2 to 4.
    let items = outline_items(&pdf("pypdf-outline.pdf").json);
    assert_eq!(items.len(), 9, "{items:?}");
    assert_eq!(items[0], "Foo, page 2");
    assert_eq!(items[8], "Baz, page 4");
}

#[test]
fn layers_six() {
    let doc = pdf("pypdf-ocg-layers.pdf").json;
    assert_eq!(doc["properties"]["facts"]["layers"], 6);
    let names = doc["properties"]["custom"]["layers"]["value"]
        .as_str()
        .unwrap();
    assert_eq!(
        names,
        "North Arrow (shown), Scale Bar (shown), Map Frame (shown), Map (shown), \
         Light Gray Base (shown), Data Source Attribution (shown)"
    );
}

#[test]
fn threads_titles() {
    let doc = pdf("pdfbox-poems-article-threads.pdf").json;
    let threads: Vec<(&str, &str)> = list(&doc, "/facets/embedded")
        .iter()
        .filter(|e| e["kind"] == "article-thread")
        .map(|e| (e["name"].as_str().unwrap(), e["detail"].as_str().unwrap()))
        .collect();
    // The second title is spelled so in the file.
    assert_eq!(
        threads,
        [("Erlkönig", "page 1"), ("Moulière", "pages 1, 2")]
    );
}

#[test]
fn attachment_listed() {
    let out = pdf("pypdf-attachment.pdf");
    let attachments: Vec<&Value> = list(&out.json, "/facets/embedded")
        .iter()
        .filter(|e| e["kind"] == "attachment")
        .collect();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0]["name"], "image.png");
    assert_eq!(attachments[0]["detail"], "6669 bytes");
    // Listed, never extracted without an output directory.
    assert!(list(&out.json, "/assets").is_empty());
}

#[test]
fn xmp_title() {
    let doc = pdf("pypdf-xmp.pdf").json;
    assert_eq!(doc["properties"]["title"], "Sample PDF with XMP Metadata");
    assert_eq!(
        doc["properties"]["authors"],
        serde_json::json!(["John Doe"])
    );
    assert_eq!(
        doc["properties"]["created"]["value"],
        "2023-04-10T07:46:54Z"
    );
    assert_eq!(
        doc["properties"]["custom"]["subject"]["value"],
        "This is a text"
    );
}

#[test]
fn aes_files_encrypted_fact() {
    for file in [
        "qpdf-aes128-empty-user-password.pdf",
        "qpdf-aes256-empty-user-password.pdf",
    ] {
        let out = pdf(file);
        let facts = &out.json["properties"]["facts"];
        assert_eq!(facts["encrypted"], true, "{file}");
        assert_eq!(facts["pages"], 30, "{file}");
        assert!(list(&out.json, "/losses").is_empty(), "{file}");
        assert!(out.markdown.contains("encrypted: true"), "{file}");
        assert!(out.markdown.contains("Potato 29"), "{file}");
    }
    // The same text without encryption has no such fact.
    let plain = pdf("qpdf-page-labels-and-outlines.pdf").json;
    assert_eq!(plain["properties"]["facts"]["encrypted"], false);
    // A file that needs a password is not read and says so.
    let locked = pdf("qpdf-user-password.pdf");
    assert_eq!(locked.json["losses"][0]["why"], "encrypted");
    assert!(locked.markdown.contains("completeness: failed"));
}

#[test]
fn scan_needs_vision() {
    let out = pdf("scan-truman-letter-nara.pdf");
    let assets = list(&out.json, "/assets");
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0]["id"], "page-1");
    assert_eq!(assets[0]["mime"], "image/png");
    assert_eq!(assets[0]["needs_vision"], true);
    assert_eq!(assets[0]["alt"], "no text layer, 1 picture");
    // The page has no word of its own; the reader says what to look at.
    let page = &list(&out.json, "/body")[0];
    assert!(
        list(page, "/blocks")
            .iter()
            .any(|b| b["type"] == "media" && b["asset"] == "page-1"),
        "{page}"
    );
    // Without an output directory nothing is rendered: the asset has no file.
    assert!(assets[0].get("path").is_none(), "{}", assets[0]);
}

#[test]
fn arabic_not_reversed() {
    let out = pdf("pypdf-arabic-habibi.pdf");
    // حَبيبي in logical order: hah first.
    assert!(
        out.markdown
            .contains("\u{62d}\u{64e}\u{628}\u{64a}\u{628}\u{64a} habibi"),
        "{}",
        out.markdown
    );
    assert!(
        !out.markdown
            .contains("\u{64a}\u{628}\u{64a}\u{628}\u{64e}\u{62d}")
    );
    assert!(!out.markdown.contains("ibibah"));
}

/// The reads of the large remote files run under the default limits; their time and maximum
/// resident set size are printed for the report (`--no-capture`).
#[cfg(unix)]
fn remote_pdf(name: &str) -> PathBuf {
    let path = corpus_dir().join("pdf").join(name);
    assert!(
        path.exists(),
        "{name} is missing: run scripts/fetch-corpus.sh first"
    );
    path
}

#[cfg(unix)]
#[test]
#[ignore = "needs scripts/fetch-corpus.sh"]
fn nist_reads_under_limits() {
    let path = remote_pdf("nist-sp800-53r5.pdf");
    let start = std::time::Instant::now();
    let out = run(&pipeline(), &path);
    let took = start.elapsed();
    let rss = max_rss_bytes();
    eprintln!(
        "nist-sp800-53r5.pdf: {took:?}, max RSS {} MiB, {} bytes of Markdown",
        rss >> 20,
        out.markdown.len()
    );
    let doc = &out.json;
    assert_eq!(doc["properties"]["facts"]["pages"], 492);
    assert_eq!(doc["properties"]["facts"]["tagged"], true);
    let pages = list(doc, "/body")
        .iter()
        .filter(|s| s["kind"] == "page")
        .count();
    assert_eq!(pages, 492);
    // Read whole under the default limits: no page is cut.
    assert!(
        !list(doc, "/losses").iter().any(|l| l["why"] == "limit"),
        "{:?}",
        doc["losses"]
    );
    assert!(
        out.markdown
            .contains("Security and Privacy Controls for Information Systems")
    );
    assert!(took < Config::default().limits.max_time, "{took:?}");
    // The RSS is the figure of this test only under nextest (one process per test).
    assert!(
        std::env::var_os("NEXTEST").is_none() || rss < 1 << 30,
        "max RSS {} MiB",
        rss >> 20
    );
}

#[cfg(unix)]
#[test]
#[ignore = "needs scripts/fetch-corpus.sh"]
fn arxiv_tables_and_pictures() {
    let path = remote_pdf("arxiv-2408.09869-docling-report.pdf");
    let dir = tempfile::tempdir().unwrap();
    let request = Request {
        output_dir: Some(dir.path().to_path_buf()),
        ..request()
    };
    let start = std::time::Instant::now();
    let result = pipeline().run_path(&path, &request);
    let took = start.elapsed();
    let rss = max_rss_bytes();
    eprintln!(
        "arxiv-2408.09869-docling-report.pdf: {took:?}, max RSS {} MiB",
        rss >> 20
    );
    let doc: Value = serde_json::from_str(result.json.as_deref().unwrap()).unwrap();
    assert_eq!(doc["properties"]["facts"]["pages"], 9);
    // The pages with figures need vision and are rendered into the bundle.
    let pictures: Vec<&str> = list(&doc, "/assets")
        .iter()
        .filter(|a| a["needs_vision"] == true)
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert_eq!(pictures, ["page-1", "page-3", "page-8", "page-9"]);
    let bundle = dir
        .path()
        .join("arxiv-2408.09869-docling-report.pdf.assets");
    for id in &pictures {
        let png = std::fs::read(bundle.join(format!("{id}.png"))).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{id}");
    }
    // The tables of the report are drawn with a top and a bottom rule only. They stay
    // paragraphs, and the output says where a table may be.
    let unsure: Vec<&str> = list(&doc, "/losses")
        .iter()
        .filter(|l| l["why"] == "uncertain")
        .filter_map(|l| l["location"].as_str())
        .collect();
    assert_eq!(unsure, ["p5", "p8", "p9"], "{:?}", doc["losses"]);
    let md = result.markdown_pages.join("");
    assert!(md.contains("Table 1: Runtime characteristics of Docling"));
    assert!(md.contains("Apple M3 Max"));
    assert!(took < Config::default().limits.max_time, "{took:?}");
    assert!(
        std::env::var_os("NEXTEST").is_none() || rss < 512 << 20,
        "max RSS {} MiB",
        rss >> 20
    );
}

/// The text of one page section of the model, its blocks joined by blank lines.
fn page_text(doc: &Value, page: u32) -> String {
    fn walk(v: &Value, out: &mut String) {
        match v {
            Value::Object(m) => {
                if let Some(t) = m.get("text").and_then(Value::as_str) {
                    out.push_str(t);
                }
                if m.get("type").and_then(Value::as_str) == Some("line_break") {
                    out.push(' ');
                }
                if let Some(inlines) = m.get("inlines") {
                    walk(inlines, out);
                }
                for key in ["blocks", "items", "rows", "cells"] {
                    if let Some(inner) = m.get(key) {
                        walk(inner, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let section = list(doc, "/body")
        .iter()
        .find(|s| s["anchor"] == format!("p{page}"))
        .unwrap_or_else(|| panic!("no page {page}"));
    let mut out = String::new();
    for block in list(section, "/blocks") {
        walk(block, &mut out);
        out.push_str("\n\n");
    }
    out
}

#[test]
fn sf1199a_back_page_reads_as_prose() {
    // The back of the form is set in a larger size than the front. Its body text is prose,
    // not headings, and its sentences stay whole.
    let doc = pdf("pdfbox-sf1199a-direct-deposit.pdf").json;
    let section = list(&doc, "/body")
        .iter()
        .find(|s| s["anchor"] == "p4")
        .unwrap();
    let headings: Vec<&Value> = list(section, "/blocks")
        .iter()
        .filter(|b| b["type"] == "heading")
        .collect();
    assert!(headings.is_empty(), "{headings:?}");
    let text = page_text(&doc, 4);
    for sentence in [
        "is required under 31 USC 3322, 31 CFR 209 and/or 210.",
        "Failure to provide the requested information may affect the processing of this form",
        "The financial institution cannot cancel the authorization by advice to the Government agency.",
    ] {
        let squeezed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(squeezed.contains(sentence), "{sentence}\n{text}");
    }
    // A number that goes on a sentence is not a list item.
    assert!(
        !list(section, "/blocks").iter().any(|b| b["type"] == "list"),
        "{section}"
    );
}

#[test]
fn multicolumn_reads_column_by_column() {
    // The left column of page 1 ends with "Donec nonummy"; the right column goes on with
    // "pellentesque ante". The left column is read whole before the right one.
    let text = page_text(&pdf("pypdf-multicolumn.pdf").json, 1);
    let end_left = text
        .find("Donec nonummy")
        .expect("the end of the left column");
    let start_right = text
        .find("pellentesque ante. Phasellus")
        .expect("the start of the right column");
    assert!(end_left < start_right, "{text}");
    let abstract_at = text.find("This is a sample document").unwrap();
    assert!(abstract_at < end_left);
}

/// The Census caption is the group title of its sheet's parameters, written once: equal
/// paths show the parts after the title, never the title again (spec 8.7). Its label
/// column cells and header cells all have paths like `<caption> / Geographic Area`.
#[test]
fn census_caption_is_written_once_as_the_group_title() {
    let md = office_markdown("NST-EST2023-POP.xlsx");
    let caption = "Annual Estimates of the Resident Population for the United States, Regions, States, District of Columbia, and Puerto Rico: April 1, 2020 to July 1, 2023";
    let logic = &md[md.find("## Logic").expect("a Logic section")..];
    let head = "#### Parameters\n";
    let rest = &logic[logic.find(head).expect("a Parameters part") + head.len()..];
    let part = &rest[..rest.find("\n#").map_or(rest.len(), |e| e + 1)];
    let title = format!("Parameters in {caption}:");
    assert_eq!(part.matches(title.as_str()).count(), 1, "{part}");
    // No row's label repeats it (cell A2, whose value is the caption, shows it as a value).
    let labels: Vec<&str> = part
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| Cell") && !l.starts_with("| ---"))
        .map(|l| l.split(" | ").nth(2).unwrap_or(""))
        .collect();
    assert!(labels.len() > 300, "{part}");
    assert!(!labels.iter().any(|l| l.contains(caption)), "{part}");
}
