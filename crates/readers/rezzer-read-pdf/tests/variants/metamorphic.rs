// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Metamorphic tests of the region engine on every generated shape, one base variant each:
//! each page as the reader hands it to the engine (before any given region), changed in a way
//! that keeps its meaning (see `meta`), must keep the engine's decisions or move a typed
//! region to a spatial one. The breaks found today are pinned per shape and page in
//! `metamorphic-shapes.tsv`, with the task that owns them (see `meta::pin_lines`). The
//! pictures family is read in a fixed sample by default (every [`PICTURE_STEP`]th shape: its
//! shapes are rendered and scanned for codes); `metamorphic_shapes_every_family` (ignored)
//! runs every shape.

use crate::meta::{self, Decisions, Transform};
use crate::{all_shapes, axes, inv};

/// The pictures family's default sample takes every this many shapes.
const PICTURE_STEP: usize = 4;

/// The engine's input of each page of `shape` in the base variant, before any given region,
/// and whether the file is read by its tags. A shape whose axes draw files of two pages only
/// (a box in another width on the second page) is read with both pages: the base variant
/// with the first page count of the shape's own axes.
fn pages_of(shape: &axes::Shape) -> (Vec<layoutregions::PageIn>, bool) {
    let base = axes::Variant {
        pages: shape
            .axes
            .pages
            .first()
            .copied()
            .unwrap_or(axes::BASE.pages),
        ..axes::BASE
    };
    let page = axes::page_of(shape, base);
    let (_, pages) = inv::read_engine(&page);
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

/// The pinned breaks of the shapes (see `meta::pin_lines`).
fn pins() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/variants/metamorphic-shapes.tsv")
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

/// The pin lines of the shapes of `family`; each break printed (with `--no-capture`).
fn sweep_shapes(family: &[axes::Shape]) -> (Vec<String>, Vec<String>) {
    let (mut names, mut lines) = (Vec::new(), Vec::new());
    for shape in family {
        let (pages, tagged) = pages_of(shape);
        assert_facts_in_both_orders(shape.name, &pages, tagged);
        assert_facts_under_reorder(shape.name, &pages, tagged);
        let decide = |p: &[layoutregions::PageIn]| -> Vec<Decisions> {
            rezzer_read_pdf::engine_decisions(p, tagged)
        };
        let findings = meta::sweep(&pages, &Transform::ALL, &decide);
        for l in meta::break_lines(shape.name, &findings) {
            println!("BREAK\t{l}");
        }
        for l in meta::move_lines(shape.name, &findings) {
            println!("MOVE\t{l}");
        }
        names.push(shape.name.to_string());
        lines.extend(meta::pin_lines(shape.name, &findings));
    }
    (names, lines)
}

macro_rules! family_tests {
    ($($test:ident => $family:path,)*) => {
        $(
            /// Every shape of the family in its base variant under every change, against the
            /// pinned breaks (`metamorphic_shapes_every_family` writes them).
            #[test]
            fn $test() {
                if std::env::var_os("REZZER_WRITE_META").is_some() {
                    return;
                }
                let (names, lines) = sweep_shapes(&$family());
                meta::check_pins(&pins(), &names, &lines);
            }
        )*
    };
}

family_tests! {
    metamorphic_forms_hold_their_pins => crate::forms::shapes,
    metamorphic_docs_hold_their_pins => crate::docs::shapes,
    metamorphic_pictures_hold_their_pins => picture_sample,
    metamorphic_classes_hold_their_pins => crate::classes::shapes,
}

/// Every shape of every family under every change, against the pinned breaks, each break
/// printed (with `--no-capture`). `REZZER_WRITE_META` writes the pins instead.
#[test]
#[ignore = "the family tests run the same shapes; run it to print or write the pins"]
fn metamorphic_shapes_every_family() {
    let (names, lines) = sweep_shapes(&all_shapes());
    meta::check_pins(&pins(), &names, &lines);
}

/// The default sample of the pictures family: every [`PICTURE_STEP`]th shape.
fn picture_sample() -> Vec<axes::Shape> {
    crate::pictures::shapes()
        .into_iter()
        .step_by(PICTURE_STEP)
        .collect()
}

/// Every shape the pin file names is a shape of a family: the pins of a shape that was
/// renamed or taken out are stale, and no sweep would ever check them.
#[test]
fn metamorphic_pins_name_shapes_that_are_there() {
    let shapes: Vec<&'static str> = all_shapes().iter().map(|s| s.name).collect();
    let stale: Vec<String> = meta::pinned_names(&pins())
        .into_iter()
        .filter(|n| !shapes.contains(&n.as_str()))
        .collect();
    assert!(
        stale.is_empty(),
        "pins of shapes that are gone, take them out of {}: {stale:#?}",
        pins().display()
    );
}

/// Prints the path of one page through the engine before and after one change, for the cause
/// of a pin: each decision with its kind, what made its segment, its reasons (see
/// `rezzer_read_pdf::engine_decisions_why`) and its first words, the decisions that stand on
/// one side only marked with `<` (the page) or `>` (the changed page). The page is the base
/// variant of the shape `REZZER_TRACE_SHAPE`, or page `REZZER_TRACE_PAGE` (from 1, default 1)
/// of the file at `REZZER_TRACE_FILE`; the change is `REZZER_TRACE_CHANGE` (a name of
/// `Transform::name`). With `REZZER_TRACE_RUNS` the runs of both pages are printed too.
/// Without a change it does nothing. Run with `--no-capture`.
#[test]
#[ignore = "prints the path of one page under one change when REZZER_TRACE_CHANGE is set"]
fn trace_change() {
    let Ok(change) = std::env::var("REZZER_TRACE_CHANGE") else {
        return;
    };
    let t = Transform::ALL
        .into_iter()
        .find(|t| t.name() == change)
        .unwrap_or_else(|| panic!("no change {change}"));
    let (pages, tagged) = if let Ok(file) = std::env::var("REZZER_TRACE_FILE") {
        let (_, pages) = crate::common::read_engine_pages(&std::fs::read(file).unwrap());
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
    } else {
        let name = std::env::var("REZZER_TRACE_SHAPE").expect("a shape or a file");
        let shape = all_shapes()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no shape {name}"));
        pages_of(&shape)
    };
    let at: usize = std::env::var("REZZER_TRACE_PAGE")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(1);
    let (changed, origins): (Vec<layoutregions::PageIn>, Vec<meta::Origin>) =
        pages.iter().map(|p| t.apply(p)).unzip();
    let page = &pages[at - 1];
    let lines = |ps: &[layoutregions::PageIn], origin: &meta::Origin| -> Vec<String> {
        rezzer_read_pdf::engine_decisions_why(ps, tagged)[at - 1]
            .iter()
            .map(|(kind, prims, made, why)| {
                let mut ids: Vec<u32> = prims
                    .iter()
                    .flat_map(|&p| origin.of(p))
                    .filter_map(|p| match p {
                        layoutregions::PrimId::Text(i) => Some(i),
                        _ => None,
                    })
                    .collect();
                ids.sort_unstable();
                ids.dedup();
                let text: Vec<&str> = ids
                    .iter()
                    .filter_map(|&i| page.texts.get(i as usize))
                    .map(|r| r.text.trim())
                    .collect();
                let words: String = text.join(" | ").chars().take(70).collect();
                format!("{} {made} {why} {} runs {words:?}", kind.name(), ids.len())
            })
            .collect()
    };
    if std::env::var_os("REZZER_TRACE_RUNS").is_some() {
        for (name, p) in [("PAGE", page), ("CHANGED", &changed[at - 1])] {
            println!("{name} RUNS");
            for (i, r) in p.texts.iter().enumerate() {
                println!(
                    "  run {i}: x {:.2}..{:.2} y {:.2} size {:.2} line {}{} {:?}",
                    r.x,
                    r.x + r.w,
                    r.y,
                    r.size,
                    r.line,
                    if r.bold { " bold" } else { "" },
                    r.text.chars().take(40).collect::<String>()
                );
            }
        }
    }
    let before = lines(&pages, &meta::Origin::same(page));
    let after = lines(&changed, &origins[at - 1]);
    println!("PAGE");
    for l in &before {
        println!("{} {l}", if after.contains(l) { ' ' } else { '<' });
    }
    println!("CHANGED {change}");
    for l in &after {
        println!("{} {l}", if before.contains(l) { ' ' } else { '>' });
    }
}
