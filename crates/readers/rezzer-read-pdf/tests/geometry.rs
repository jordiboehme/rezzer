// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The page geometry a read gives when it asks for it: the size of each page and the boxes of
//! its rules, drawn shapes, pictures, tables, cells, headings and text.

mod common;

use common::barfont::page_with_photo;
use common::{Page, markdown, pdf, read, read_in, show};
use std::time::Duration;

use rezzer_limits::{Budget, CancelToken, Clock, Limits};
use rezzer_model::{
    Anchor, Block, BoxKind, Geometry, MAX_BOXES_PER_PAGE, PlacedBox, ReadOptions, ReadOutcome,
};

/// `bytes` read with page geometry asked for.
fn read_geometry(bytes: &[u8]) -> ReadOutcome {
    read_in(
        bytes,
        &ReadOptions {
            geometry: true,
            ..ReadOptions::default()
        },
        &Budget::new(Limits::default()),
    )
}

fn geometry(out: &ReadOutcome) -> &Geometry {
    out.document.geometry.as_deref().expect("page geometry")
}

/// The boxes of page `n`.
fn boxes(out: &ReadOutcome, n: u32) -> &[PlacedBox] {
    let page = geometry(out)
        .pages
        .iter()
        .find(|p| p.page == n)
        .expect("the page has geometry");
    &page.boxes
}

fn of_kind(boxes: &[PlacedBox], kind: BoxKind) -> Vec<&PlacedBox> {
    boxes.iter().filter(|b| b.kind == kind).collect()
}

/// The blocks of page section `n`.
fn page(out: &ReadOutcome, n: u32) -> &[Block] {
    let want = Anchor(format!("p{n}"));
    out.document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section {
                anchor: Some(a),
                blocks,
                ..
            } if *a == want => Some(blocks.as_slice()),
            _ => None,
        })
        .expect("the page section")
}

/// Whether `got` lies within half a point of `want` on every side.
fn near<const N: usize>(got: [f32; N], want: [f32; N]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.5)
}

/// Whether `inner` lies inside `outer`.
fn inside(inner: [f32; 4], outer: [f32; 4]) -> bool {
    inner[0] >= outer[0] && inner[1] >= outer[1] && inner[2] <= outer[2] && inner[3] <= outer[3]
}

/// A ruled table of `rows` rows of two cells, 30 points high and 120 wide each, with every
/// line of the grid drawn, between two paragraphs.
fn ruled_page(rows: &[[&str; 2]]) -> String {
    ruled_at(rows, 0.0)
}

/// [`ruled_page`] moved `down` points down.
fn ruled_at(rows: &[[&str; 2]], down: f32) -> String {
    let (left, top) = (72.0, 700.0 - down);
    let mut c = show(
        "F1",
        10.0,
        72.0,
        760.0 - down,
        "A paragraph before the table.",
    );
    c += "0.5 w ";
    let height = 30.0 * rows.len() as f32;
    for k in 0..=rows.len() {
        let y = top - 30.0 * k as f32;
        c += &format!("{left} {y} m {} {y} l S ", left + 240.0);
    }
    for k in 0..=2 {
        let x = left + 120.0 * k as f32;
        c += &format!("{x} {top} m {x} {} l S ", top - height);
    }
    for (r, row) in rows.iter().enumerate() {
        for (k, text) in row.iter().enumerate() {
            let x = left + 6.0 + 120.0 * k as f32;
            let y = top - 30.0 * r as f32 - 20.0;
            c += &show("F1", 10.0, x, y, text);
        }
    }
    c += &show(
        "F1",
        10.0,
        72.0,
        520.0 - down,
        "A paragraph after the table.",
    );
    c
}

fn ruled_table() -> Vec<u8> {
    pdf(
        &[Page::text(ruled_page(&[
            ["Fruit", "Count"],
            ["Apples", "3"],
            ["Pears", "12"],
        ]))],
        "",
    )
}

/// A page with a large bold heading over lines of body text.
fn heading_page() -> Vec<u8> {
    let mut c = show("F2", 20.0, 72.0, 760.0, "Quarterly Garden Notes");
    let lines = [
        "The orchard gave more apples this year than in any year before.",
        "Most of the pears were picked in the second week of the month.",
        "The plums stayed small, and the figs did not ripen at all.",
        "Next spring the beds by the wall get new soil and more light.",
        "The old shed is cleared out so the tools have a dry place.",
    ];
    for (k, line) in lines.iter().enumerate() {
        c += &show("F1", 10.0, 72.0, 720.0 - 14.0 * k as f32, line);
    }
    pdf(&[Page::text(c)], "")
}

#[test]
fn no_geometry_unless_asked() {
    let out = read(&ruled_table());
    assert!(out.document.geometry.is_none());
}

#[test]
fn geometry_has_page_size_rules_and_pictures() {
    let mut c = show("F1", 10.0, 72.0, 780.0, "A line of quiet words.");
    c += "0.5 w 72 700 m 300 700 l S\n";
    c += "72 650 m 300 650 l S\n";
    c += "200 300 120 60 re f\n";
    c += "q 100 0 0 80 300 400 cm /Photo Do Q\n";
    let out = read_geometry(&page_with_photo(&c, &[]));
    let g = geometry(&out);
    assert_eq!(g.pages.len(), 1);
    assert_eq!(g.pages[0].page, 1);
    assert_eq!(g.pages[0].size, [595.0, 842.0]);
    let all = boxes(&out, 1);
    let rules = of_kind(all, BoxKind::Rule);
    assert_eq!(rules.len(), 2, "{all:?}");
    for want in [[72.0, 700.0, 300.0, 700.0], [72.0, 650.0, 300.0, 650.0]] {
        assert!(rules.iter().any(|r| near(r.bounds, want)), "{rules:?}");
    }
    let drawn = of_kind(all, BoxKind::Drawn);
    assert_eq!(drawn.len(), 1, "{all:?}");
    assert!(
        near(drawn[0].bounds, [200.0, 300.0, 320.0, 360.0]),
        "{drawn:?}"
    );
    let pictures = of_kind(all, BoxKind::Picture);
    assert_eq!(pictures.len(), 1, "{all:?}");
    assert!(
        near(pictures[0].bounds, [300.0, 400.0, 400.0, 480.0]),
        "{pictures:?}"
    );
}

#[test]
fn ruled_table_gives_table_and_cells() {
    let out = read_geometry(&ruled_table());
    let table = page(&out, 1)
        .iter()
        .find_map(|b| match b {
            Block::Table { anchor, rows, .. } => Some((anchor.clone(), rows.len())),
            _ => None,
        })
        .expect("the page has a table");
    assert_eq!(table.1, 3);
    let anchor = table.0.expect("the table has an anchor");
    let found = geometry(&out).box_of(&anchor).expect("the table has a box");
    assert_eq!(found.kind, BoxKind::Table);
    let all = boxes(&out, 1);
    assert_eq!(of_kind(all, BoxKind::Table).len(), 1, "{all:?}");
    let cells = of_kind(all, BoxKind::Cell);
    assert_eq!(cells.len(), 6, "{all:?}");
    // The cells follow their table, inside its box.
    let at = all
        .iter()
        .position(|b| b.kind == BoxKind::Table)
        .expect("a table box");
    assert!(
        all[at + 1..at + 7].iter().all(|b| b.kind == BoxKind::Cell),
        "{all:?}"
    );
    assert!(
        cells.iter().all(|c| inside(c.bounds, found.bounds)),
        "{all:?}"
    );
    // Each cell is the box of its text: from where its run starts, from the baseline up the
    // size of the text, row by row and left to right.
    for (k, cell) in cells.iter().enumerate() {
        let (r, c) = (k / 2, k % 2);
        let (x, y) = (78.0 + 120.0 * c as f32, 680.0 - 30.0 * r as f32);
        let b = cell.bounds;
        assert!(near([b[0], b[1]], [x, y]), "cell {k}: {b:?}");
        assert!((b[3] - b[1] - 10.0).abs() <= 0.5, "cell {k}: {b:?}");
        assert!(b[2] > b[0] && b[2] < x + 114.0, "cell {k}: {b:?}");
    }
}

#[test]
fn heading_box_has_depth_and_anchor() {
    let out = read_geometry(&heading_page());
    let (depth, anchor) = page(&out, 1)
        .iter()
        .find_map(|b| match b {
            Block::Heading {
                level: depth,
                anchor: Some(anchor),
                ..
            } => Some((*depth, anchor.clone())),
            _ => None,
        })
        .expect("the page has a heading");
    let found = geometry(&out)
        .box_of(&anchor)
        .expect("the heading has a box");
    assert_eq!(found.kind, BoxKind::Heading(depth));
    assert!(near(
        found.bounds,
        [72.0, 760.0, found.bounds[2], found.bounds[3]]
    ));
    assert!(
        found.bounds[2] > 200.0 && found.bounds[3] > 770.0,
        "{found:?}"
    );
    // The lines of body text are text boxes with their anchors.
    let texts = of_kind(boxes(&out, 1), BoxKind::Text);
    assert!(!texts.is_empty());
    assert!(texts.iter().all(|t| t.anchor.is_some()), "{texts:?}");
}

/// A page with `n` short drawn lines and a line of text.
fn many_lines(n: usize) -> Vec<u8> {
    let mut c = show("F1", 10.0, 72.0, 800.0, "A page of many short lines.");
    c += "0.5 w ";
    for k in 0..n {
        let (x, y) = (20.0 + 5.5 * (k % 100) as f32, 40.0 + 7.0 * (k / 100) as f32);
        c += &format!("{x} {y} m {} {y} l S ", x + 3.0);
    }
    pdf(&[Page::text(c)], "")
}

#[test]
fn geometry_caps_hold() {
    let bytes = many_lines(10_000);
    let pdf_steps = || rezzer_read_pdf::work_steps().unwrap_or(0);
    let model_steps = || rezzer_model::work_steps().unwrap_or(0);
    let (pdf0, model0) = (pdf_steps(), model_steps());
    let plain = read(&bytes);
    let (pdf1, model1) = (pdf_steps(), model_steps());
    let out = read_geometry(&bytes);
    let (pdf2, model2) = (pdf_steps(), model_steps());
    let all = boxes(&out, 1);
    assert_eq!(all.len(), MAX_BOXES_PER_PAGE);
    // The text comes before the rules, so the cap cuts rules only.
    assert_eq!(all[0].kind, BoxKind::Text, "{:?}", &all[..4]);
    assert_eq!(of_kind(all, BoxKind::Rule).len(), MAX_BOXES_PER_PAGE - 1);
    assert_eq!(markdown(&out), markdown(&plain));
    // The reader does no more work of its own for the geometry; the pushes count one step
    // each for the one page they look at, and stop at the page's cap.
    assert_eq!(pdf2 - pdf1, pdf1 - pdf0);
    let pushes = (model2 - model1).saturating_sub(model1 - model0);
    assert!(pushes <= MAX_BOXES_PER_PAGE as u64 + 8, "{pushes}");
}

#[test]
fn markdown_same_with_and_without_geometry() {
    let mut c = ruled_page(&[["Fruit", "Count"], ["Apples", "3"], ["Pears", "12"]]);
    c += &show("F2", 20.0, 72.0, 800.0, "Quarterly Garden Notes");
    c += "200 300 120 60 re f\n";
    c += "q 100 0 0 80 300 400 cm /Photo Do Q\n";
    for bytes in [page_with_photo(&c, &[]), heading_page(), ruled_table()] {
        let plain = read(&bytes);
        let with = read_geometry(&bytes);
        assert!(with.document.geometry.is_some());
        assert_eq!(markdown(&with), markdown(&plain));
        assert_eq!(
            serde_json::to_string(&with.document).unwrap(),
            serde_json::to_string(&plain.document).unwrap()
        );
    }
}

/// A ruled table drawn in a frame turned a quarter onto the page: its text reads upward, so
/// the page is read in a turned frame, 842 points wide and 595 high. The size and the boxes
/// are in that frame, and the cells stand where the table is drawn in it.
#[test]
fn a_turned_page_gives_its_boxes_in_the_frame_of_its_text() {
    let mut c = String::from("q 0 1 -1 0 595 0 cm\n");
    c += &ruled_at(
        &[["Fruit", "Count"], ["Apples", "3"], ["Pears", "12"]],
        200.0,
    );
    c += "Q\n";
    let out = read_geometry(&pdf(&[Page::text(c)], ""));
    let g = geometry(&out);
    assert_eq!(g.pages[0].size, [842.0, 595.0]);
    let all = boxes(&out, 1);
    let tables = of_kind(all, BoxKind::Table);
    assert_eq!(tables.len(), 1, "{all:?}");
    let cells = of_kind(all, BoxKind::Cell);
    assert_eq!(cells.len(), 6, "{all:?}");
    for b in all {
        let [x0, y0, x1, y1] = b.bounds;
        assert!(
            x0 >= -0.5 && y0 >= -0.5 && x1 <= 842.5 && y1 <= 595.5,
            "{b:?}"
        );
    }
    for (k, cell) in cells.iter().enumerate() {
        let (r, c) = (k / 2, k % 2);
        let (x, y) = (78.0 + 120.0 * c as f32, 480.0 - 30.0 * r as f32);
        assert!(
            near([cell.bounds[0], cell.bounds[1]], [x, y]),
            "cell {k}: {cell:?}"
        );
    }
    assert!(!of_kind(all, BoxKind::Text).is_empty(), "{all:?}");
}

/// `pages` pages, each of a line of text and a drawn rule.
fn pages_with_a_rule(pages: usize) -> Vec<u8> {
    let page = |k: usize| {
        let mut c = show(
            "F1",
            10.0,
            72.0,
            760.0,
            &format!("Page number {k} of the file."),
        );
        for line in 0..20 {
            let y = 700.0 - 14.0 * line as f32;
            c += &show(
                "F1",
                10.0,
                72.0,
                y,
                "Some quiet words of the body text here.",
            );
        }
        c += "0.5 w 72 300 m 300 300 l S\n";
        Page::text(c)
    };
    let all: Vec<Page> = (1..=pages).map(page).collect();
    pdf(&all, "")
}

/// Reads `bytes` with page geometry on a clock of one millisecond a reading under
/// `max_time`, with a look at the clock after each page written as plain lines; gives the
/// read and the readings it took.
fn read_on_steps(bytes: &[u8], max_time: Duration) -> (ReadOutcome, u64) {
    let clock = Clock::steps(Duration::from_millis(1));
    let limits = Limits {
        max_time,
        ..Limits::default()
    };
    let budget = Budget::with_clock(limits, CancelToken::default(), clock.clone());
    let opts = ReadOptions {
        geometry: true,
        ..ReadOptions::default()
    };
    let out = rezzer_read_pdf::with_plain_check(1, || read_in(bytes, &opts, &budget));
    (out, clock.reads())
}

/// A page the time limit names as not read gives no geometry: each page with geometry has
/// its text written, laid out or as plain lines.
#[test]
fn a_page_the_time_stops_gives_no_geometry() {
    let bytes = pages_with_a_rule(120);
    let (_, reads) = read_on_steps(&bytes, Duration::from_secs(360_000));
    let mut halted = 0;
    for k in 1..=12u64 {
        let limit = Duration::from_millis(reads * k / 16);
        let (out, _) = read_on_steps(&bytes, limit);
        let Some(g) = out.document.geometry.as_deref() else {
            continue;
        };
        let written: Vec<u32> = out
            .document
            .body
            .iter()
            .filter_map(|b| match b {
                Block::Section {
                    anchor: Some(a),
                    blocks,
                    ..
                } if blocks.len() > 1 => a.0.strip_prefix('p')?.parse().ok(),
                _ => None,
            })
            .collect();
        if written.len() < 120 {
            halted += 1;
        }
        for p in &g.pages {
            assert!(
                written.contains(&p.page),
                "limit {limit:?}: page {} has geometry but no text",
                p.page
            );
        }
    }
    assert!(halted > 0, "no limit stopped the read");
}
