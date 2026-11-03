// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One segment tree per page: given regions, drawn grids and boxes, the whitespace cut and
//! style steps, nested; every primitive in exactly one leaf of the main cut and of each
//! alternative.

use std::collections::BTreeMap;

use layoutregions::segment::drawn::drawn;
use layoutregions::segment::{SegId, Segment, Segments, Source, segment_page};
use layoutregions::*;

fn run(text: &str, x: f32, y: f32, w: f32, size: f32, line: u32) -> TextRun {
    TextRun {
        text: text.to_string(),
        x,
        y,
        w,
        h: size,
        size,
        font: 1,
        font_kind: FontKind::Proportional,
        bold: false,
        italic: false,
        color: None,
        shift: Shift::Base,
        line,
        off_axis: false,
        gaps: Vec::new(),
    }
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> RuleIn {
    RuleIn {
        x0,
        y0,
        x1,
        y1,
        thick: 0.5,
        dashed: false,
        rect: true,
    }
}

fn line(x0: f32, y0: f32, x1: f32, y1: f32) -> RuleIn {
    RuleIn {
        rect: false,
        ..rect(x0, y0, x1, y1)
    }
}

fn page(size: [f32; 2], texts: Vec<TextRun>, rules: Vec<RuleIn>) -> PageIn {
    PageIn {
        page: 1,
        size,
        turn: 0,
        texts,
        rules,
        boxes: Vec::new(),
        pictures: Vec::new(),
        codes: Vec::new(),
        given: Vec::new(),
    }
}

const PROSE: &str = "Erika Mustermann wohnt in 12345 Musterstadt";

/// A column of `n` lines of prose at `x`, from `top` down, 12 points apart, rows numbered
/// from `line`.
fn column(x: f32, top: f32, n: u32, line: u32) -> Vec<TextRun> {
    (0..n)
        .map(|k| run(PROSE, x, top - 12.0 * k as f32, 210.0, 9.0, line + k))
        .collect()
}

fn seg(s: &Segments, id: SegId) -> &Segment {
    &s.all[id.0 as usize]
}

/// Every primitive of the page.
fn all_prims(p: &PageIn) -> Vec<PrimId> {
    let mut out = Vec::new();
    out.extend((0..p.texts.len() as u32).map(PrimId::Text));
    out.extend((0..p.rules.len() as u32).map(PrimId::Rule));
    out.extend((0..p.boxes.len() as u32).map(PrimId::Box));
    out.extend((0..p.pictures.len() as u32).map(PrimId::Picture));
    out.extend((0..p.codes.len() as u32).map(PrimId::Code));
    out
}

/// The primitives of the leaves under `ids`, each as often as a leaf holds it.
fn leaf_prims(s: &Segments, ids: &[SegId]) -> BTreeMap<PrimId, usize> {
    let mut out = BTreeMap::new();
    let mut stack: Vec<SegId> = ids.to_vec();
    while let Some(id) = stack.pop() {
        let g = seg(s, id);
        if g.children.is_empty() {
            for p in &g.prims {
                *out.entry(*p).or_insert(0) += 1;
            }
        } else {
            stack.extend(g.children.iter().copied());
        }
    }
    out
}

/// Each primitive of the page in exactly one leaf of the main cut, and each primitive of a
/// segment in exactly one leaf of each of its alternatives. Parents and children agree.
fn check_cover(s: &Segments, p: &PageIn) {
    let want: BTreeMap<PrimId, usize> = all_prims(p).into_iter().map(|p| (p, 1)).collect();
    assert_eq!(leaf_prims(s, &[s.root]), want, "main cut");
    for (i, g) in s.all.iter().enumerate() {
        assert_eq!(g.id, SegId(i as u32));
        for c in &g.children {
            assert_eq!(seg(s, *c).parent, Some(g.id), "parent of {c:?}");
        }
        let mine: BTreeMap<PrimId, usize> = g.prims.iter().map(|p| (*p, 1)).collect();
        assert_eq!(mine.len(), g.prims.len(), "a prim twice in {:?}", g.id);
        if !g.children.is_empty() {
            assert_eq!(leaf_prims(s, &g.children), mine, "children of {:?}", g.id);
        }
        for alt in &g.alternatives {
            assert_eq!(leaf_prims(s, alt), mine, "alternative of {:?}", g.id);
        }
    }
}

/// Whether a `Limit` issue of `s` says `words`.
fn limit_says(s: &Segments, words: &str) -> bool {
    s.issues
        .iter()
        .any(|i| i.kind == IssueKind::Limit && i.what.contains(words))
}

fn depth(s: &Segments, mut id: SegId) -> usize {
    let mut d = 0;
    while let Some(p) = seg(s, id).parent {
        id = p;
        d += 1;
    }
    d
}

fn with_source(s: &Segments, f: impl Fn(&Source) -> bool) -> Vec<&Segment> {
    s.all.iter().filter(|g| f(&g.source)).collect()
}

/// A ruled grid of three rows and three columns, a box holding two lines, two text columns
/// below and a footer across them.
fn mixed_page() -> PageIn {
    let mut texts = Vec::new();
    let mut rules = vec![rect(50.0, 600.0, 300.0, 700.0)];
    rules.push(line(133.0, 600.0, 133.0, 700.0));
    rules.push(line(216.0, 600.0, 216.0, 700.0));
    rules.push(line(50.0, 633.0, 300.0, 633.0));
    rules.push(line(50.0, 666.0, 300.0, 666.0));
    for r in 0..3u32 {
        for c in 0..3u32 {
            texts.push(run(
                "1.234,56",
                60.0 + 83.0 * c as f32,
                680.0 - 33.0 * r as f32,
                40.0,
                9.0,
                r,
            ));
        }
    }
    rules.push(rect(350.0, 600.0, 545.0, 700.0));
    texts.push(run("Muster GmbH", 360.0, 680.0, 80.0, 9.0, 0));
    texts.push(run("12345 Musterstadt", 360.0, 668.0, 110.0, 9.0, 1));
    texts.extend(column(50.0, 560.0, 10, 3));
    texts.extend(column(315.0, 560.0, 10, 3));
    texts.push(run("Seite 1 von 1", 250.0, 40.0, 80.0, 7.0, 13));
    page([595.0, 842.0], texts, rules)
}

#[test]
fn every_prim_in_one_leaf() {
    let p = mixed_page();
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    assert_eq!(seg(&s, s.root).source, Source::Page);
    assert!(s.issues.is_empty(), "{:?}", s.issues);
    // The grid is found with its rows and columns and holds its nine cells.
    let grids = with_source(&s, |x| matches!(x, Source::Grid { .. }));
    assert_eq!(grids.len(), 1);
    assert_eq!(grids[0].source, Source::Grid { rows: 3, cols: 3 });
    let texts = |g: &Segment| -> Vec<u32> {
        g.prims
            .iter()
            .filter_map(|p| match p {
                PrimId::Text(i) => Some(*i),
                _ => None,
            })
            .collect()
    };
    assert_eq!(texts(grids[0]), (0..9).collect::<Vec<_>>());
    // The box holds its two lines and its outline.
    let boxes = with_source(&s, |x| *x == Source::Box);
    assert_eq!(boxes.len(), 1);
    assert_eq!(texts(boxes[0]), vec![9, 10]);
    assert!(boxes[0].prims.contains(&PrimId::Rule(5)));
    // The two columns are parted by a gutter: no leaf holds text of both.
    for g in s.all.iter().filter(|g| g.children.is_empty()) {
        let t = texts(g);
        let left = t.iter().any(|i| (11..21).contains(i));
        let right = t.iter().any(|i| (21..31).contains(i));
        assert!(!(left && right), "{:?} holds both columns", g.id);
    }
    assert!(!with_source(&s, |x| *x == Source::Gutter).is_empty());
}

#[test]
fn given_regions_come_first_and_whole() {
    let mut p = page(
        [595.0, 842.0],
        column(50.0, 700.0, 6, 0),
        vec![line(50.0, 600.0, 300.0, 600.0)],
    );
    // Listed second but ordered first; the other one shares Text(3) with it, and names a
    // primitive the page does not have.
    p.given = vec![
        GivenRegion {
            kind: Kind::Paragraph,
            prims: vec![PrimId::Text(3), PrimId::Text(4), PrimId::Text(999)],
            order: 2,
        },
        GivenRegion {
            kind: Kind::Table,
            prims: vec![PrimId::Text(2), PrimId::Text(3), PrimId::Rule(0)],
            order: 1,
        },
    ];
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    let root = seg(&s, s.root);
    let first = seg(&s, root.children[0]);
    let second = seg(&s, root.children[1]);
    assert_eq!(first.source, Source::Given(Kind::Table));
    assert_eq!(
        first.prims,
        vec![PrimId::Text(2), PrimId::Text(3), PrimId::Rule(0)]
    );
    assert_eq!(second.source, Source::Given(Kind::Paragraph));
    assert_eq!(second.prims, vec![PrimId::Text(4)]);
    // Whole: a given region is one leaf, and its primitives are in no other segment below
    // the root.
    assert!(first.children.is_empty() && second.children.is_empty());
    for g in &s.all {
        if g.id == s.root || g.id == first.id || g.id == second.id {
            continue;
        }
        for p in [2, 3, 4] {
            assert!(!g.prims.contains(&PrimId::Text(p)), "{:?}", g.id);
        }
    }
}

#[test]
fn box_holding_a_grid_nests() {
    let mut texts = vec![run("Muster GmbH", 60.0, 680.0, 80.0, 9.0, 0)];
    let rules = vec![
        rect(40.0, 400.0, 400.0, 700.0),
        rect(80.0, 450.0, 300.0, 600.0),
        line(190.0, 450.0, 190.0, 600.0),
        line(80.0, 525.0, 300.0, 525.0),
    ];
    for (r, y) in [570.0f32, 495.0].into_iter().enumerate() {
        for x in [90.0f32, 200.0] {
            texts.push(run("1.234,56", x, y, 40.0, 9.0, 1 + r as u32));
        }
    }
    let p = page([595.0, 842.0], texts, rules);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    let boxes = with_source(&s, |x| *x == Source::Box);
    let grids = with_source(&s, |x| matches!(x, Source::Grid { .. }));
    assert_eq!((boxes.len(), grids.len()), (1, 1));
    assert_eq!(grids[0].source, Source::Grid { rows: 2, cols: 2 });
    assert_eq!(grids[0].parent, Some(boxes[0].id));
    assert_eq!(boxes[0].parent, Some(s.root));
    for k in 1..5 {
        assert!(grids[0].prims.contains(&PrimId::Text(k)));
    }
    assert!(!grids[0].prims.contains(&PrimId::Text(0)));
    assert!(boxes[0].prims.contains(&PrimId::Text(0)));
}

/// Two columns of prose; the left one stands in a drawn box whose right side lies in the
/// gutter.
fn boxed_column_page() -> PageIn {
    let mut texts = column(50.0, 680.0, 10, 0);
    texts.extend(column(320.0, 680.0, 10, 0));
    page([595.0, 842.0], texts, vec![rect(40.0, 550.0, 280.0, 700.0)])
}

#[test]
fn two_cuts_give_an_alternative() {
    let p = boxed_column_page();
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    assert!(s.issues.is_empty(), "{:?}", s.issues);
    let root = seg(&s, s.root);
    // The main cut: the box holds the left column.
    let boxes = with_source(&s, |x| *x == Source::Box);
    assert_eq!(boxes.len(), 1);
    assert_eq!(boxes[0].parent, Some(s.root));
    assert!((0..10).all(|k| boxes[0].prims.contains(&PrimId::Text(k))));
    // The gutter the box also bounds is the other cut of the same primitives.
    assert_eq!(root.alternatives.len(), 1);
    let alt = &root.alternatives[0];
    let text_groups: Vec<&Segment> = alt
        .iter()
        .map(|&id| seg(&s, id))
        .filter(|g| g.prims.iter().any(|p| matches!(p, PrimId::Text(_))))
        .collect();
    assert_eq!(text_groups.len(), 2);
    assert!(text_groups.iter().all(|g| g.source == Source::Gutter));
    let left: Vec<PrimId> = (0..10).map(PrimId::Text).collect();
    let right: Vec<PrimId> = (10..20).map(PrimId::Text).collect();
    assert_eq!(text_groups[0].prims, left);
    assert_eq!(text_groups[1].prims, right);
    // Alternatives hang under the segment they cut, but are not its children.
    for &id in alt {
        assert_eq!(seg(&s, id).parent, Some(s.root));
        assert!(!root.children.contains(&id));
    }
}

#[test]
fn a_heading_above_a_box_is_no_alternative() {
    // A title above a boxed block, no gutter: the box explains a band, no column.
    let mut texts = vec![run("Muster GmbH", 50.0, 720.0, 80.0, 12.0, 0)];
    texts.extend(column(50.0, 680.0, 5, 1));
    let p = page([595.0, 842.0], texts, vec![rect(40.0, 600.0, 280.0, 700.0)]);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    assert!(s.all.iter().all(|g| g.alternatives.is_empty()));
}

/// Concentric boxes, 10 points apart, on a page large enough that the outermost is no frame;
/// one run in the top strip of each ring and the rest of `runs` in the innermost box.
fn nested_page(boxes: usize, runs: usize) -> PageIn {
    let side = 10_000.0f32;
    let outer = 7_700.0f32;
    let lo = (side - outer) / 2.0;
    let mut rules = Vec::with_capacity(boxes);
    let mut texts = Vec::with_capacity(runs);
    for k in 0..boxes {
        let a = lo + 10.0 * k as f32;
        rules.push(rect(a, a, side - a, side - a));
        // In the ring between box k and box k + 1, in its top strip.
        texts.push(run("Muster", a + 20.0, side - a - 8.0, 4.0, 4.0, k as u32));
    }
    let inner = lo + 10.0 * (boxes - 1) as f32;
    let cols = 224usize;
    let rest = runs - boxes;
    for i in 0..rest {
        let (r, c) = (i / cols, i % cols);
        texts.push(run(
            "1.234,56",
            inner + 20.0 + 16.0 * c as f32,
            side - inner - 20.0 - 16.0 * r as f32,
            10.0,
            8.0,
            (boxes + r) as u32,
        ));
    }
    page([side, side], texts, rules)
}

#[test]
fn alternatives_and_depth_are_bounded() {
    let p = nested_page(200, 50_000);
    assert_eq!(p.texts.len(), 50_000);
    assert_eq!(
        drawn(&p, &|_| true).boxes.len(),
        200,
        "the builder makes 200 boxes"
    );
    let limits = Limits::default();
    let before = work_steps();
    let started = std::time::Instant::now();
    let s = segment_page(&p, &limits, &|| false);
    let took = started.elapsed();
    let after = work_steps();
    check_cover(&s, &p);
    // Depth: no segment below the bound; the boxes past it are one leaf holding every
    // primitive of them, and the page says so.
    let deepest = s.all.iter().map(|g| depth(&s, g.id)).max().unwrap();
    assert!(deepest <= limits.max_cut_depth as usize, "{deepest}");
    assert!(limit_says(&s, "nest deeper"), "{:?}", s.issues);
    let boxes = with_source(&s, |x| *x == Source::Box);
    assert_eq!(boxes.len(), limits.max_cut_depth as usize);
    let last = boxes
        .iter()
        .find(|g| depth(&s, g.id) == limits.max_cut_depth as usize)
        .expect("a box at the bound");
    assert!(last.children.is_empty());
    // Box 63 (counted from 0) and every box in it, with the runs in their rings and the
    // inner runs.
    let held = (200 - 63) + (50_000 - 200);
    let texts = last
        .prims
        .iter()
        .filter(|p| matches!(p, PrimId::Text(_)))
        .count();
    assert_eq!(texts, held);
    assert_eq!(last.prims.len(), held + (200 - 63));
    // Work: pinned with a little headroom over the measured count.
    if let (Some(b), Some(a)) = (before, after) {
        eprintln!("work steps {} in {took:?}", a - b);
        assert!(a - b <= WORK_BOUND, "{} steps", a - b);
    }

    // Deeper bounds walk all 200 boxes, still inside the limits and without a depth issue.
    let deep = Limits {
        max_cut_depth: 256,
        ..Limits::default()
    };
    let s = segment_page(&p, &deep, &|| false);
    check_cover(&s, &p);
    assert_eq!(with_source(&s, |x| *x == Source::Box).len(), 200);
    assert!(s.issues.is_empty(), "{:?}", s.issues);

    // A small depth bound: the rest is one leaf at the bound.
    let p2 = mixed_page();
    let shallow = Limits {
        max_cut_depth: 1,
        ..Limits::default()
    };
    let s = segment_page(&p2, &shallow, &|| false);
    check_cover(&s, &p2);
    assert!(s.all.iter().all(|g| depth(&s, g.id) <= 1));
    assert!(limit_says(&s, "nest deeper"), "{:?}", s.issues);

    // Alternatives: past the cap none is kept, the page says so, the main cut stays whole.
    let p3 = boxed_column_page();
    let none = Limits {
        max_alternatives: 0,
        ..Limits::default()
    };
    let s = segment_page(&p3, &none, &|| false);
    check_cover(&s, &p3);
    assert!(s.all.iter().all(|g| g.alternatives.is_empty()));
    assert!(limit_says(&s, "other cuts"), "{:?}", s.issues);
}

/// Work steps of `segment_page` on the page of 50,000 runs in 200 nested boxes, the cut's own
/// work included: 13,228,429 measured, about ten in a hundred more allowed. Most of it is each
/// primitive once per level of nesting it stands in (64 levels at the bound): placed and held
/// by its segment. No text stands beside a side of an inner box, so no level cuts again for an
/// alternative.
const WORK_BOUND: u64 = 14_500_000;

#[test]
fn a_stop_keeps_the_rest_whole() {
    let p = mixed_page();
    let s = segment_page(&p, &Limits::default(), &|| true);
    check_cover(&s, &p);
    assert!(s.issues.iter().any(|i| i.kind == IssueKind::Stopped));
    assert!(seg(&s, s.root).children.is_empty());
}

#[test]
fn unsearched_lines_are_an_issue() {
    // More straight lines than the grid search looks at, upright ones among them.
    let mut rules: Vec<RuleIn> = (0..20_001)
        .map(|k| line(10.0, 10.0 + k as f32 * 0.03, 500.0, 10.0 + k as f32 * 0.03))
        .collect();
    rules.push(line(20.0, 10.0, 20.0, 600.0));
    let p = page([595.0, 842.0], column(50.0, 700.0, 3, 0), rules);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    let limits: Vec<&Issue> = s
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Limit)
        .collect();
    assert_eq!(limits.len(), 1, "{:?}", s.issues);
    assert!(
        limits[0].what.contains("straight lines"),
        "{}",
        limits[0].what
    );
}

#[test]
fn style_steps_part_a_leaf() {
    let mut texts = vec![run("Muster GmbH", 50.0, 711.0, 80.0, 14.0, 0)];
    texts[0].bold = true;
    texts.extend(column(50.0, 700.0, 4, 1));
    let p = page([595.0, 842.0], texts, Vec::new());
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    let styles = with_source(&s, |x| *x == Source::Style);
    assert_eq!(styles.len(), 2);
    assert_eq!(styles[0].prims, vec![PrimId::Text(0)]);
}

/// Two columns of prose of 9,500 lines each, just under the size up to which the cut looks
/// for columns, inside 70 nested boxes: every level of nesting cuts all of its text again for
/// the alternative.
#[test]
fn nested_columns_are_cut_in_time() {
    let boxes = 70usize;
    let lines = 9_500u32;
    let (w, h) = (4_000.0f32, 300_000.0f32);
    let mut rules = Vec::new();
    for k in 0..boxes {
        let a = 100.0 + 10.0 * k as f32;
        rules.push(rect(a, a, 2_000.0 - a + 100.0, 120_000.0 - a));
    }
    let inner = 100.0 + 10.0 * (boxes - 1) as f32;
    let top = 120_000.0 - inner - 30.0;
    let mut texts = column(inner + 20.0, top, lines, 0);
    texts.extend(column(inner + 280.0, top, lines, 0));
    let p = page([w, h], texts, rules);
    assert_eq!(drawn(&p, &|_| true).boxes.len(), boxes);
    let started = std::time::Instant::now();
    let s = segment_page(&p, &Limits::default(), &|| false);
    eprintln!("nested columns in {:?}", started.elapsed());
    check_cover(&s, &p);
    assert!(s.issues.iter().any(|i| i.kind == IssueKind::Limit));
    // With room for every level the innermost box parts its two columns at the gutter.
    let deep = Limits {
        max_cut_depth: 256,
        ..Limits::default()
    };
    let started = std::time::Instant::now();
    let s = segment_page(&p, &deep, &|| false);
    eprintln!("nested columns, every level, in {:?}", started.elapsed());
    check_cover(&s, &p);
    let gutters = with_source(&s, |x| *x == Source::Gutter);
    assert_eq!(gutters.len(), 2);
    assert_eq!(gutters[0].prims.len(), lines as usize);
}

#[test]
fn a_stop_at_any_point_keeps_every_prim() {
    // Stop after the n-th question: inside the parts of the page, the cut and the
    // alternative alike, every primitive stays in one leaf.
    for p in [mixed_page(), boxed_column_page(), nested_page(80, 2_000)] {
        let mut asked_total = 0;
        for n in 1..40u32 {
            let asked = std::cell::Cell::new(0u32);
            let stop = || {
                asked.set(asked.get() + 1);
                asked.get() > n
            };
            let s = segment_page(&p, &Limits::default(), &stop);
            check_cover(&s, &p);
            let stopped = s.issues.iter().any(|i| i.kind == IssueKind::Stopped);
            assert_eq!(stopped, asked.get() > n, "{n}");
            asked_total = asked_total.max(asked.get());
        }
        assert!(asked_total > 1);
    }
}

#[test]
fn a_grid_the_given_regions_took_is_no_segment() {
    // The grid is still drawn, but a given region holds its lines and its cells.
    let mut p = mixed_page();
    let mut prims: Vec<PrimId> = (0..9).map(PrimId::Text).collect();
    prims.extend((0..5).map(PrimId::Rule));
    p.given = vec![GivenRegion {
        kind: Kind::Table,
        prims,
        order: 0,
    }];
    assert_eq!(
        drawn(&p, &|_| true).grids.map(|g| g.groups.len()),
        Ok(2),
        "the grid and the box are drawn"
    );
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    assert!(with_source(&s, |x| matches!(x, Source::Grid { .. })).is_empty());
    assert_eq!(with_source(&s, |x| *x == Source::Box).len(), 1);
}

#[test]
fn segments_are_bounded() {
    // 20,000 lines of one column whose size alternates: style steps would make one segment
    // per line, more than the cap.
    let texts: Vec<TextRun> = (0..20_000u32)
        .map(|k| {
            let size = if k % 2 == 0 { 9.0 } else { 12.0 };
            run(PROSE, 50.0, 300_000.0 - 14.0 * k as f32, 210.0, size, k)
        })
        .collect();
    let p = page([600.0, 300_100.0], texts, Vec::new());
    let limits = Limits::default();
    let s = segment_page(&p, &limits, &|| false);
    check_cover(&s, &p);
    assert!(s.all.len() <= limits.max_regions, "{}", s.all.len());
    let hits: Vec<&Issue> = s
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Limit)
        .collect();
    assert_eq!(hits.len(), 1, "{:?}", s.issues);
    assert!(hits[0].what.contains("segments"), "{}", hits[0].what);
    // A cap of a few segments on the mixed page: inside it, every prim in one leaf.
    let p = mixed_page();
    for cap in 1..12 {
        let small = Limits {
            max_regions: cap,
            ..Limits::default()
        };
        let s = segment_page(&p, &small, &|| false);
        check_cover(&s, &p);
        assert!(s.all.len() <= cap.max(1), "{cap}: {}", s.all.len());
    }
}

/// Nested boxes, each with a line of its own beside the left side of the next one, around
/// 5,000 lines that each stand apart by a band of white space, so that the cut parts them one
/// by one, as deep as it may.
fn staircase_page(boxes: usize, lines: u32) -> PageIn {
    let top = 110_000.0f32;
    let mut rules = Vec::new();
    let mut texts = Vec::new();
    for k in 0..boxes {
        let (x0, y0) = (100.0 + 30.0 * k as f32, 100.0 + 10.0 * k as f32);
        rules.push(rect(
            x0,
            y0,
            9_000.0 - 10.0 * k as f32,
            top - 10.0 * k as f32,
        ));
        texts.push(run("Muster", x0 + 2.0, 50_000.0, 10.0, 6.0, 0));
    }
    let inner = 100.0 + 30.0 * (boxes - 1) as f32;
    for k in 0..lines {
        texts.push(run(
            PROSE,
            inner + 100.0,
            top - 2_000.0 - 20.0 * k as f32,
            210.0,
            6.0,
            1 + k,
        ));
    }
    page([20_000.0, 120_000.0], texts, rules)
}

#[test]
fn nested_boxes_around_deep_text_are_cut_in_bounded_work() {
    let p = staircase_page(70, 5_000);
    assert_eq!(drawn(&p, &|_| true).boxes.len(), 70);
    let before = work_steps();
    let started = std::time::Instant::now();
    let s = segment_page(&p, &Limits::default(), &|| false);
    let took = started.elapsed();
    let after = work_steps();
    check_cover(&s, &p);
    if let (Some(b), Some(a)) = (before, after) {
        eprintln!("staircase work steps {} in {took:?}", a - b);
        assert!(a - b <= STAIRCASE_BOUND, "{} steps", a - b);
    }
    assert!(limit_says(&s, "nest deeper"), "{:?}", s.issues);
}

/// Work steps of `segment_page` on the staircase page (70 boxes, 5,000 lines apart):
/// 11,230,144 measured, about ten in a hundred more allowed. The cut
/// looks for bands only a few levels deep, so each level of boxes cuts its text in work in
/// proportion to it.
const STAIRCASE_BOUND: u64 = 12_500_000;

#[test]
fn a_depth_bound_of_zero_keeps_the_page_one_leaf() {
    let p = mixed_page();
    let zero = Limits {
        max_cut_depth: 0,
        ..Limits::default()
    };
    let s = segment_page(&p, &zero, &|| false);
    check_cover(&s, &p);
    assert_eq!(s.all.len(), 1);
    assert!(limit_says(&s, "nest deeper"), "{:?}", s.issues);
    // A page of one line has nothing to part: no issue.
    let p = page([595.0, 842.0], column(50.0, 700.0, 1, 0), Vec::new());
    let s = segment_page(&p, &zero, &|| false);
    assert!(s.issues.is_empty(), "{:?}", s.issues);
}

#[test]
fn a_child_the_given_regions_took_is_no_depth_issue() {
    // The box at the bound holds one line; the grid in it is a given region's: the box would
    // not be parted, so the bound costs nothing and says nothing.
    let mut texts = vec![run("Muster GmbH", 60.0, 680.0, 80.0, 9.0, 0)];
    let rules = vec![
        rect(40.0, 400.0, 400.0, 700.0),
        rect(80.0, 450.0, 300.0, 600.0),
        line(190.0, 450.0, 190.0, 600.0),
        line(80.0, 525.0, 300.0, 525.0),
    ];
    for (r, y) in [570.0f32, 495.0].into_iter().enumerate() {
        for x in [90.0f32, 200.0] {
            texts.push(run("1.234,56", x, y, 40.0, 9.0, 1 + r as u32));
        }
    }
    let mut p = page([595.0, 842.0], texts, rules);
    let mut prims: Vec<PrimId> = (1..5).map(PrimId::Text).collect();
    prims.extend((0..4).map(PrimId::Rule));
    p.given = vec![GivenRegion {
        kind: Kind::Table,
        prims,
        order: 0,
    }];
    let one = Limits {
        max_cut_depth: 1,
        ..Limits::default()
    };
    let s = segment_page(&p, &one, &|| false);
    check_cover(&s, &p);
    assert_eq!(with_source(&s, |x| *x == Source::Box).len(), 1);
    assert!(s.issues.is_empty(), "{:?}", s.issues);
    // Without the given region the grid would be parted off at the bound: that is said.
    p.given.clear();
    let s = segment_page(&p, &one, &|| false);
    check_cover(&s, &p);
    assert!(limit_says(&s, "nest deeper"), "{:?}", s.issues);
}

#[test]
fn given_regions_beside_an_alternative() {
    // The boxed column page with a footer the file states: the root's alternative holds the
    // given region too, and every prim stays in one leaf of both cuts.
    let mut p = boxed_column_page();
    p.texts
        .push(run("Seite 1 von 1", 250.0, 40.0, 80.0, 7.0, 20));
    p.given = vec![GivenRegion {
        kind: Kind::FooterBand,
        prims: vec![PrimId::Text(20)],
        order: 0,
    }];
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    let root = seg(&s, s.root);
    assert_eq!(
        seg(&s, root.children[0]).source,
        Source::Given(Kind::FooterBand)
    );
    assert_eq!(root.alternatives.len(), 1);
    assert!(root.alternatives[0].contains(&root.children[0]));
}
