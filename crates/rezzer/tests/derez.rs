// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Grids from real files: every format the binary reads gives its own grid. The samples are
//! public corpus files, listed with source and license in `tests/corpus/MANIFEST.md`; they
//! are untrusted data and are only read.
#![cfg(all(
    feature = "fmt-text",
    feature = "fmt-markdown",
    feature = "fmt-csv",
    feature = "fmt-xlsx",
    feature = "fmt-ods",
    feature = "fmt-pdf"
))]

mod common;

use assert_cmd::cargo::cargo_bin_cmd;
use common::{GENERATED_AT, corpus_dir, corpus_files, pipeline};
use rezzer_derez::grid::{Dir, Grid, Pos};
use rezzer_derez::plan::{Fitted, Refused, START_RUN, fit};
use rezzer_derez::rounds;
use rezzer_pipeline::Request;

/// One sample per format, below `tests/corpus`. A format the binary lists that is missing
/// here fails the test.
const SAMPLES: [(&str, &str); 9] = [
    ("text", "web-text/text-utf8-gutenberg-faust.txt"),
    (
        "markdown",
        "web-text/md-toml-frontmatter-rust-blog-traits.md",
    ),
    ("csv", "web-text/csv-pandas-fdic-banklist.csv"),
    ("tsv", "web-text/tsv-noheader-geonames-admin1.txt"),
    ("xlsx", "office/ExcelTables.xlsx"),
    ("xlsm", "office/ex012.PVT_z_factor.xlsm"),
    ("xlsb", "office/testEXCEL_various.xlsb"),
    ("ods", "office/merged.ods"),
    ("pdf", "pdf/pdfjs-structure-simple.pdf"),
];

/// The formats of `rezzer formats`, with HOME and the config dir in a temp dir.
fn listed_formats() -> Vec<String> {
    let home = tempfile::tempdir().unwrap();
    let out = cargo_bin_cmd!("rezzer")
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .arg("formats")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines
            .next()
            .map(|l| l.split_whitespace().collect::<Vec<_>>()),
        Some(vec!["format", "reader", "version"]),
        "{text}"
    );
    lines
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// The free cells `from` reaches, by row.
fn flood(g: &Grid, from: Pos) -> Vec<bool> {
    let w = usize::from(g.width());
    let mut seen = vec![false; w * usize::from(g.height())];
    let mut todo = vec![from];
    seen[usize::from(from.y) * w + usize::from(from.x)] = true;
    while let Some(p) = todo.pop() {
        for d in Dir::ALL {
            if let Some(q) = g.step(p, d).filter(|&q| g.is_free(q)) {
                let i = usize::from(q.y) * w + usize::from(q.x);
                if !seen[i] {
                    seen[i] = true;
                    todo.push(q);
                }
            }
        }
    }
    seen
}

/// Problems of a fitted grid: a free cell or a start the user program's start does not
/// reach, or a start without its run of free cells.
fn problems(f: &Fitted) -> Vec<String> {
    let g = &f.grid;
    let w = usize::from(g.width());
    let seen = flood(g, f.starts[0].0);
    let mut out = Vec::new();
    for y in 0..g.height() {
        for x in 0..g.width() {
            let p = Pos { x, y };
            if g.is_free(p) && !seen[usize::from(y) * w + usize::from(x)] {
                out.push(format!("free cell {x},{y} not reached"));
            }
        }
    }
    for (k, &(start, dir)) in f.starts.iter().enumerate() {
        if !seen[usize::from(start.y) * w + usize::from(start.x)] {
            out.push(format!("start {k} not reached"));
        }
        let mut at = start;
        for _ in 0..START_RUN {
            match g.step(at, dir).filter(|&q| g.is_free(q)) {
                Some(q) => at = q,
                None => {
                    out.push(format!("start {k} has no free run"));
                    break;
                }
            }
        }
    }
    out
}

#[test]
fn every_format_gives_its_own_grid() {
    let formats = listed_formats();
    assert!(!formats.is_empty());
    let pipeline = pipeline();
    let request = Request {
        generated_at: Some(GENERATED_AT.to_string()),
        geometry: true,
        ..Request::default()
    };
    let corpus = corpus_files();
    let mut failures = Vec::new();
    for format in &formats {
        let Some(&(_, sample)) = SAMPLES.iter().find(|(f, _)| f == format) else {
            failures.push(format!("{format}: no sample"));
            continue;
        };
        let path = corpus_dir().join(sample);
        assert!(corpus.contains(&path), "{sample} is not a corpus file");
        let result = pipeline.run_path(&path, &request);
        let Some(doc) = &result.document else {
            failures.push(format!("{format}: {sample} not read: {:?}", result.message));
            continue;
        };
        let read_as = serde_json::to_value(doc.source.format).unwrap();
        if read_as != format.as_str() {
            failures.push(format!("{format}: {sample} read as {read_as}"));
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let r = rounds(&name, Some(doc), None);
        if let Some(note) = &r.note {
            failures.push(format!("{format}: {note}"));
        }
        for plan in &r.plans {
            let at = format!("{format}: {}", plan.label);
            if plan.label == "the Grid" {
                failures.push(format!("{at}: the built-in grid"));
            }
            match fit(plan, 80, 40) {
                Ok(f) => failures.extend(problems(&f).into_iter().map(|p| format!("{at}: {p}"))),
                Err(e) => failures.push(format!("{at}: {e:?} at 80x40")),
            }
            match fit(plan, 30, 18) {
                Ok(f) => failures.extend(
                    problems(&f)
                        .into_iter()
                        .map(|p| format!("{at}: {p} at 30x18")),
                ),
                Err(Refused::Crowded) => {}
                Err(e) => failures.push(format!("{at}: {e:?} at 30x18")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
