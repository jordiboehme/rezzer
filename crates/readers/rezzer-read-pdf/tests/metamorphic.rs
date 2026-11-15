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

/// The facts of a file are the same whether they are learned from the runs in the order the
/// reader hands them over (its flow) or in the order of their places (as `engine_decisions`
/// learns them, so that the order of the runs changes no fact). Names what differs.
///
/// This compares two orders only on a page where they differ, and the reader hands nearly
/// every page over in the order of its places (3 of the 1,010 pages of the corpus and the
/// shapes differ); `assert_facts_under_reorder` compares every page
/// with its runs shuffled. The lines of a file are learned with their runs in place order,
/// so what is learned from them holds in every order; the legend of a page is read in the
/// order handed over. For a tagged file the read itself learns from more runs than these
/// pages hold (those an ActualText replaces).
fn assert_facts_in_both_orders(name: &str, pages: &[layoutregions::PageIn], tagged: bool) {
    let [given, placed] = rezzer_read_pdf::engine_facts_by_order(pages, tagged);
    let mut differs = Vec::new();
    if given.body_size != placed.body_size {
        differs.push(format!(
            "body size {} | {}",
            given.body_size, placed.body_size
        ));
    }
    if given.heading_sizes != placed.heading_sizes {
        differs.push(format!(
            "heading sizes {:?} | {:?}",
            given.heading_sizes, placed.heading_sizes
        ));
    }
    if given.running != placed.running {
        differs.push(format!(
            "running {:?} | {:?}",
            given.running, placed.running
        ));
    }
    if given.two_page != placed.two_page {
        differs.push(format!(
            "two-page footers {:?} | {:?}",
            given.two_page, placed.two_page
        ));
    }
    for page in given.legends.keys().chain(placed.legends.keys()) {
        if given.legends.get(page) != placed.legends.get(page) {
            differs.push(format!("legend of page {page}"));
        }
    }
    differs.dedup();
    assert!(
        differs.is_empty(),
        "{name}: facts of the file by the order of the runs (flow | places): {differs:#?}"
    );
}

/// The facts of a file that are learned from its lines are the same with the runs of each
/// page in another order (the change `reorder-runs` of the sweep, each run keeps its line
/// number): the body size, the heading sizes, the running lines and the footers of the
/// two-page rule.
///
/// The legends are not compared. A known limit, owner after 0.1.0: the footnote legend of a page is read from the runs in the
/// order handed in, so with the runs shuffled it can be found differently or not at all.
/// The cause: `RefPlace::Above` (a reference number set at the end of the line before)
/// needs the number to open the next line, and only the order of the runs says that it
/// opens it. Sorting the runs by place would pair the number with the word left of it on
/// its own line, a wrong pair, which is worse than a read that depends on the order. A
/// file hands its runs over in one fixed order; only a shuffle moves the legend. The rule
/// that would lift the limit asks for no order: the number goes to the word whose baseline
/// is nearest over it, whichever line number it has. That is a change of the legend rules
/// and needs its own private check before it lands.
///
/// The body size leaves out the ink of the legend's runs (see `file::learn`), so it
/// follows the legend: where a shuffle loses a legend, its ink counts as body ink. On the
/// corpus and the shapes the body size holds, so it is asserted here; a file whose legend
/// holds more ink than its body size could move it under a shuffle.
fn assert_facts_under_reorder(name: &str, pages: &[layoutregions::PageIn], tagged: bool) {
    let reordered: Vec<layoutregions::PageIn> = pages
        .iter()
        .map(|p| Transform::ReorderRuns.apply(p).0)
        .collect();
    let [plain, _] = rezzer_read_pdf::engine_facts_by_order(pages, tagged);
    let [moved, _] = rezzer_read_pdf::engine_facts_by_order(&reordered, tagged);
    let lines = |f: &layoutregions::file::FileFacts| {
        (
            f.body_size,
            f.heading_sizes.clone(),
            f.running.clone(),
            f.two_page.clone(),
        )
    };
    assert!(
        lines(&plain) == lines(&moved),
        "{name}: the facts of the file change with the order of its runs: {:#?} | {:#?}",
        lines(&plain),
        lines(&moved)
    );
}

/// The pin lines of the corpus files `names`; each break printed (with `--no-capture`).
fn sweep_files(names: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    for name in names {
        let (pages, tagged) = pages_of(name);
        assert_facts_in_both_orders(name, &pages, tagged);
        assert_facts_under_reorder(name, &pages, tagged);
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

/// Every file the pin file names is a file of the corpus: one in its folder, or one the
/// fetch list names (`tests/corpus/remote.tsv`, a large file that may not be fetched). The
/// pins of a file that was renamed or taken out are stale, and no sweep would check them.
#[test]
fn metamorphic_pins_name_corpus_files_that_are_there() {
    let remote = std::fs::read_to_string(corpus_dir().join("../remote.tsv")).unwrap_or_default();
    let fetched: Vec<&str> = remote
        .lines()
        .filter_map(|l| l.split('\t').next())
        .filter_map(|p| p.strip_prefix("pdf/"))
        .collect();
    let here = corpus_files();
    let stale: Vec<String> = meta::pinned_names(&pins())
        .into_iter()
        .filter(|n| !here.contains(n) && !fetched.contains(&n.as_str()))
        .collect();
    assert!(
        stale.is_empty(),
        "pins of files that are gone, take them out of {}: {stale:#?}",
        pins().display()
    );
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

/// A run half its size wide for each character.
fn run_at(text: &str, line: u32, x: f32, y: f32, size: f32) -> layoutregions::TextRun {
    layoutregions::TextRun {
        text: text.into(),
        x,
        y,
        w: text.len() as f32 * size * 0.5,
        h: size,
        size,
        font: 0,
        font_kind: layoutregions::FontKind::Unknown,
        bold: false,
        italic: false,
        color: None,
        shift: layoutregions::Shift::Base,
        line,
        off_axis: false,
        gaps: Vec::new(),
    }
}

/// Page `n` of a file of three, built by hand: a running line at the top, lines of body text
/// and, on page 1, a heading, a reference number raised inside a run (`unit` 2), one that
/// opens its own line at the end of the line before (`month` 3) and the legend of the two.
/// With `pad`, a run of white space ends each of these lines and stands left of them, as the
/// reader hands one over on a page of nist-sp800-53r5: the order of the runs is not the
/// order of their places, and the runs of the references and their words have other numbers
/// in the two orders.
fn page_with_notes(n: u32, pad: bool) -> layoutregions::PageIn {
    fn line(texts: &mut Vec<layoutregions::TextRun>, pad: bool, runs: Vec<layoutregions::TextRun>) {
        let (number, y) = (runs[0].line, runs[0].y);
        texts.extend(runs);
        if pad {
            texts.push(run_at(" ", number, 30.0, y, 9.0));
        }
    }
    let mut texts = Vec::new();
    let top = run_at("Works report of the north plant", 0, 40.0, 770.0, 9.0);
    line(&mut texts, pad, vec![top]);
    if n == 1 {
        let mut head = run_at("Review of the year", 1, 40.0, 730.0, 14.0);
        head.bold = true;
        line(&mut texts, pad, vec![head]);
        let word = run_at("unit charge monthly", 2, 40.0, 700.0, 9.0);
        line(
            &mut texts,
            pad,
            vec![word, run_at("2", 2, 58.5, 702.5, 6.0)],
        );
        let above = run_at("the values are reported each month", 3, 40.0, 686.0, 9.0);
        line(&mut texts, pad, vec![above]);
        texts.push(run_at("3", 4, 193.5, 688.5, 6.0));
    }
    for k in 0..4 {
        let y = 660.0 - 12.0 * k as f32;
        let body = run_at(
            "the plant ran on every day of the week",
            5 + k,
            40.0,
            y,
            9.0,
        );
        line(&mut texts, pad, vec![body]);
    }
    if n == 1 {
        texts.push(run_at("2 per month", 9, 40.0, 70.0, 6.0));
        texts.push(run_at("3 per piece", 10, 40.0, 62.0, 6.0));
    }
    layoutregions::PageIn {
        page: n,
        size: [600.0, 800.0],
        turn: 0,
        texts,
        rules: Vec::new(),
        boxes: Vec::new(),
        pictures: Vec::new(),
        codes: Vec::new(),
        given: Vec::new(),
    }
}

/// On pages whose runs do not stand in the order of their places the facts of the file are
/// the same in both orders: the running line, the heading size, the body size and the legend
/// with a reference inside a run and one above its line, each named by the same runs. The
/// pages are built by hand after an order the reader was measured to hand over (a run of
/// white space after the ink of its line and left of it), so the two orders differ on every
/// page. The same pages without those runs stand in place order.
#[test]
fn the_facts_of_pages_out_of_place_order_are_the_same_in_both_orders() {
    use layoutregions::file::RefPlace;
    let pages: Vec<layoutregions::PageIn> = (0..3).map(|n| page_with_notes(n, true)).collect();
    for p in &pages {
        assert!(
            layoutregions::RunOrder::of(p).is_some(),
            "page {}: the runs stand in the order of their places",
            p.page
        );
    }
    let [given, placed] = rezzer_read_pdf::engine_facts_by_order(&pages, false);
    // The facts are there: a comparison of two empty sets would say nothing.
    assert_eq!(given.body_size, 9.0);
    assert_eq!(given.heading_sizes, [14.0]);
    let running: Vec<&str> = given.running.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(running, ["Works report of the north plant"]);
    let places = |f: &layoutregions::file::FileFacts| -> Vec<(u32, Option<u32>, bool)> {
        f.legends[&1]
            .raised
            .iter()
            .map(|r| match r.place {
                RefPlace::End => (r.run, None, false),
                RefPlace::Inside { word, .. } => (r.run, Some(word), false),
                RefPlace::Above { word } => (r.run, Some(word), true),
            })
            .collect()
    };
    // `2` (run 5) goes into run 4, `3` (run 9) after run 7 of the line above.
    assert_eq!(places(&given), [(5, Some(4), false), (9, Some(7), true)]);
    assert_eq!(given.legends[&1].entries.len(), 2);
    assert_eq!(given, placed);
    assert_facts_in_both_orders("pages built by hand", &pages, false);
    // In place order (no run of white space) the pages are compared with themselves.
    let plain: Vec<layoutregions::PageIn> = (0..3).map(|n| page_with_notes(n, false)).collect();
    assert!(
        plain
            .iter()
            .all(|p| layoutregions::RunOrder::of(p).is_none())
    );
    let [flow, _] = rezzer_read_pdf::engine_facts_by_order(&plain, false);
    assert_eq!(places(&flow), [(3, Some(2), false), (5, Some(4), true)]);
}

/// Page `n` of a file of three with ink out of place order, as the reader hands over two
/// pieces of one row on arxiv-2408.09869 page 7 (the right one first): the running line at
/// the top and each line of body text in two runs, the right one first.
fn page_with_ink_out_of_place(n: u32) -> layoutregions::PageIn {
    let mut texts = vec![
        run_at(" of the north plant", 0, 94.0, 770.0, 9.0),
        run_at("Works report", 0, 40.0, 770.0, 9.0),
    ];
    for k in 0..4 {
        let y = 660.0 - 12.0 * k as f32;
        texts.push(run_at(" on every day of the week", 1 + k, 103.0, y, 9.0));
        texts.push(run_at("the plant ran", 1 + k, 40.0, y, 9.0));
    }
    layoutregions::PageIn {
        page: n,
        size: [600.0, 800.0],
        turn: 0,
        texts,
        rules: Vec::new(),
        boxes: Vec::new(),
        pictures: Vec::new(),
        codes: Vec::new(),
        given: Vec::new(),
    }
}

/// The lines of a file are learned with their runs in the order of their places: a running
/// line whose ink is handed over out of that order
/// gets the key its places give, in both orders and under a shuffle of the runs. Learned in
/// the order handed over, the key read `of the north plant Works report`.
#[test]
fn a_running_line_with_ink_out_of_place_order_is_learned_by_place() {
    let pages: Vec<layoutregions::PageIn> = (0..3).map(page_with_ink_out_of_place).collect();
    assert!(
        pages
            .iter()
            .all(|p| layoutregions::RunOrder::of(p).is_some())
    );
    let [given, placed] = rezzer_read_pdf::engine_facts_by_order(&pages, false);
    let running: Vec<&str> = given.running.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(running, ["Works report of the north plant"]);
    assert_eq!(given.body_size, 9.0);
    assert_eq!(given, placed);
    assert_facts_in_both_orders("ink out of place order", &pages, false);
    assert_facts_under_reorder("ink out of place order", &pages, false);
}

/// A page of the corpus the reader hands over in another order than the order of its places
/// (runs at one left edge on one line), in the default run: its facts are the same in both
/// orders. The other two such pages of the corpus are in files that are fetched, and only the
/// sweep of every page reaches them.
#[test]
fn the_facts_of_a_corpus_page_out_of_place_order_are_the_same_in_both_orders() {
    let name = "pdfbox-actualtext-din91379.pdf";
    let (pages, tagged) = pages_of(name);
    let moved = pages
        .iter()
        .filter(|p| layoutregions::RunOrder::of(p).is_some())
        .count();
    assert_eq!(
        moved, 1,
        "{name}: pages the reader hands over out of place order"
    );
    assert_facts_in_both_orders(name, &pages, tagged);
}
