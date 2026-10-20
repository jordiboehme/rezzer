// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The public test corpus in `tests/corpus/`, run through the real pipeline (text, Markdown,
//! CSV and spreadsheet readers, no cache, `generated.at` fixed). Each corpus file has a
//! `<file>.facts.json` next to it; `corpus_facts` checks them all and reports every failure
//! at once, `corpus_golden` keeps `insta` snapshots of the model JSON and the Markdown of the
//! text files and of [`GOLDEN_OFFICE`]. The named spreadsheet tests below check what the
//! workbooks prove: knowledge, not formatting.
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
    feature = "fmt-ods"
))]

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
    registry.register(Box::new(rezzer_read_sheet::SheetReader));
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

/// Folders whose files run through the pipeline with facts. `office/malformed/` is not walked:
/// the hostile tests read those files.
const PIPELINE_FOLDERS: [&str; 2] = ["web-text", "office"];

/// The spreadsheets with golden files: one of each format and the main kinds of logic
/// (engineering chains, folded add-in calls, data tables, LAMBDA and spills, threaded
/// comments, OpenFormula across sheets, pivot tables).
const GOLDEN_OFFICE: [&str; 8] = [
    "tunnel_design_calculations.xlsx",
    "ex012.PVT_z_factor.xlsm",
    "one-variable.xlsx",
    "LAMBDA.xlsx",
    "threadedComment.xlsx",
    "pivots.xlsx",
    "formula-across-sheets.ods",
    "testEXCEL_various.xlsb",
];

/// Corpus files, sorted: the files right in the pipeline folders, except facts files.
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
            if file.is_file() && !name.ends_with(".facts.json") && !name.starts_with('.') {
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
            let office = file.parent().is_some_and(|p| p.ends_with("office"));
            if office && !GOLDEN_OFFICE.contains(&name.as_str()) {
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

/// Every file in `pdf/` is listed in MANIFEST.md with its full SHA-256, every `pdf/` row has a
/// file, and no file is over 2 MB. The two large files that are not in the repo are listed in
/// `remote.tsv` only: they may be absent, and when present they must match their hash.
#[test]
fn pdf_corpus_matches_manifest() {
    let mut listed = manifest_rows();
    listed.retain(|k, _| k.starts_with("pdf/"));
    assert_eq!(listed.len(), 32, "expected 32 pdf rows");
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus_dir().join("pdf"))
        .unwrap()
        .map(|e| e.unwrap().path())
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

/// The `pdf/` rows of `remote.tsv` (path, SHA-256): large files that are fetched, not committed.
fn remote_rows() -> std::collections::BTreeMap<String, String> {
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

// The spreadsheet corpus: what each workbook proves. The checks read the model JSON (and the
// Markdown only for what exists only there), so a change of layout does not break them.

/// The model JSON of `office/<name>`.
fn office(name: &str) -> Value {
    run(&pipeline(), &corpus_dir().join("office").join(name)).json
}

fn office_markdown(name: &str) -> String {
    run(&pipeline(), &corpus_dir().join("office").join(name)).markdown
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
    // Inputs with their label and the unit in the cell beside them.
    for (anchor, label, value, unit) in [
        ("Structural_Lining!B5", "Excavated diameter, D", 10.0, "m"),
        (
            "Structural_Lining!B6",
            "Trial lining thickness, t",
            0.35,
            "m",
        ),
        ("Structural_Lining!B7", "Overburden depth, H", 25.0, "m"),
        (
            "Structural_Lining!B8",
            "Unit weight of ground, gamma",
            22.0,
            "kN/m3",
        ),
        ("Ventilation!B5", "Tunnel length, L", 800.0, "m"),
        ("Drainage!B12", "Trial pipe diameter", 0.3, "m"),
    ] {
        let p = at(&doc, "parameters", anchor);
        assert_eq!(p["label"], label, "{anchor}");
        assert_eq!(number(&p["value"]), Some(value), "{anchor}");
        assert_eq!(p["unit"], unit, "{anchor}");
        assert_eq!(p["unit_source"], "unit-cell", "{anchor}");
    }
    // The trial thickness column takes label and unit from its header `t (m)`.
    let t = at(&doc, "parameters", "Structural_Lining!E49");
    assert_eq!(t["label"], "t");
    assert_eq!(t["unit"], "m");
    assert_eq!(t["unit_source"], "header");
    // A dimensionless input gets no unit.
    let fos = at(&doc, "parameters", "Structural_Lining!B13");
    assert_eq!(fos["label"], "Target Factor of Safety");
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
    for (anchor, label, code) in [
        ("Weight!B2", "Gram", "g"),
        ("Weight!B4", "Pound mass (avoirdupois)", "lbm"),
        ("distance!B2", "Meter", "m"),
        ("distance!B6", "Foot", "ft"),
    ] {
        let p = at(&doc, "parameters", anchor);
        assert_eq!(p["label"], label, "{anchor}");
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
    assert_eq!(cells, 3_466, "cell formulas in the spreadsheet corpus");
    assert!(
        failures.is_empty(),
        "{} formulas do not parse ({limit_refusals} by the parser limits):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
