// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Metamorphic tests of the region engine on the pages of the public corpus: each page as the
//! reader hands it to the engine (`page_in`, before any given region), changed in a way that
//! keeps its meaning (see `meta`), must keep the engine's decisions or move a typed region to
//! a spatial one; never a different typed read, a lost run or a run read twice. The breaks
//! found today are pinned by name with the task that owns them. The sweep over every page of
//! the corpus is ignored by default; a fixed sample runs every time.

mod common;

#[path = "../../../formats/layoutregions/tests/meta/mod.rs"]
mod meta;

use std::path::PathBuf;

use meta::{Decisions, Transform};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/corpus/pdf")
}

/// The engine's input of each page of corpus file `name`, as `page_in` makes it (before any
/// given region), and whether the file is read by its tags.
fn pages_of(name: &str) -> (Vec<layoutregions::PageIn>, bool) {
    let bytes = std::fs::read(corpus_dir().join(name)).expect(name);
    let (_, pages) = common::read_engine_pages(&bytes);
    let tagged = pages.first().is_some_and(|p| p.tagged);
    let inputs = pages
        .into_iter()
        .map(|p| {
            let mut input = p.input;
            input.given.clear();
            input
        })
        .collect();
    (inputs, tagged)
}

/// Every PDF of the corpus, by name.
fn corpus_files() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(corpus_dir())
        .expect("corpus")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".pdf"))
        .collect();
    names.sort();
    names
}

/// The pinned breaks of the corpus (see `meta::pin_lines`).
fn pins() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/metamorphic-corpus.tsv")
}

/// The pin lines of the corpus files `names`; each break printed (with `--no-capture`).
fn sweep_files(names: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    for name in names {
        let (pages, tagged) = pages_of(name);
        let decide = |p: &[layoutregions::PageIn]| -> Vec<Decisions> {
            rezzer_read_pdf::engine_decisions(p, tagged)
        };
        let findings = meta::sweep(&pages, &Transform::ALL, &decide);
        for l in meta::break_lines(name, &findings) {
            println!("BREAK\t{l}");
        }
        for l in meta::move_lines(name, &findings) {
            println!("MOVE\t{l}");
        }
        let same = findings.iter().filter(|f| f.verdict.same()).count();
        println!("PAGES\t{name}\t{}\t{same}", findings.len());
        lines.extend(meta::pin_lines(name, &findings));
    }
    lines
}

/// The default sample: four small corpus files in git (a tax form, an invoice, a direct
/// deposit form and a page in columns).
const SAMPLE: [&str; 4] = [
    "irs-f1040-2025.pdf",
    "mustang-gnuaccounting-invoice-508.pdf",
    "pdfbox-sf1199a-direct-deposit.pdf",
    "pypdf-multicolumn.pdf",
];

/// A fixed sample of small corpus files (a tax form, a direct deposit form, an invoice and a
/// page in columns), every page under every change, against the pinned breaks.
#[test]
fn metamorphic_corpus_sample_holds_its_pins() {
    // `metamorphic_corpus_sweep_every_page` writes the pins of every file.
    if std::env::var_os("REZZER_WRITE_META").is_some() {
        return;
    }
    let names: Vec<String> = SAMPLE.map(String::from).to_vec();
    meta::check_pins(&pins(), &names, &sweep_files(&names));
}

/// Every page of every corpus file under every change, against the pinned breaks; each break
/// printed as a tab-separated line (see `meta::break_lines`). `REZZER_WRITE_META` writes the
/// pins instead. Run with `--run-ignored only --no-capture`.
#[test]
#[ignore = "sweeps every page of the corpus"]
fn metamorphic_corpus_sweep_every_page() {
    let names = corpus_files();
    // A pinned file that is not there (a large file not fetched) is not checked: say so.
    for n in meta::pinned_names(&pins()) {
        if !names.contains(&n) {
            eprintln!("not checked, the file is not there: {n}");
        }
    }
    meta::check_pins(&pins(), &names, &sweep_files(&names));
}

/// The decisions `engine_decisions` gives on the pages as the reader hands them to the engine
/// (with their given regions) are those the reader reads: every region of the read is one of
/// them, with its kind and primitives. The exception: a region the writing of the page keeps
/// as laid out (`decide::agree`, after the decisions), counted per file.
#[test]
fn engine_decisions_are_the_decisions_of_the_read() {
    for (name, written) in SAMPLE.into_iter().zip([0, 0, 3, 0]) {
        let bytes = std::fs::read(corpus_dir().join(name)).expect(name);
        let (_, pages) = common::read_engine_pages(&bytes);
        let tagged = pages.first().is_some_and(|p| p.tagged);
        let inputs: Vec<layoutregions::PageIn> = pages.iter().map(|p| p.input.clone()).collect();
        let decided = rezzer_read_pdf::engine_decisions(&inputs, tagged);
        let (mut found, mut spatial, mut other) = (0usize, 0usize, Vec::new());
        for (p, d) in pages.iter().zip(&decided) {
            for r in &p.regions.regions {
                let mut prims = r.prims.clone();
                prims.sort_unstable();
                if d.iter().any(|(k, q, _)| *k == r.kind && *q == prims) {
                    found += 1;
                } else if r.kind == layoutregions::Kind::Spatial {
                    spatial += 1;
                } else {
                    other.push(format!("page {} {:?} {:?}", p.regions.page, r.kind, prims));
                }
            }
        }
        assert_eq!(
            spatial, written,
            "{name}: regions kept as laid out when written"
        );
        eprintln!("{name}: {found} regions found, {spatial} kept as laid out when written");
        assert!(found > 0 && other.is_empty(), "{name}: {other:#?}");
    }
}
