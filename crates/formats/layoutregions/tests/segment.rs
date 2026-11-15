// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One segment tree per page: given regions, drawn grids and boxes, the whitespace cut and
//! style steps, nested; every primitive in exactly one leaf of the main cut and of each
//! alternative.

use std::collections::BTreeMap;

use layoutregions::segment::drawn::drawn;
use layoutregions::segment::{SegId, Segment, Segments, Source, segment_drawn, segment_page};
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
            level: None,
            role: None,
            group: None,
        },
        GivenRegion {
            kind: Kind::Table,
            prims: vec![PrimId::Text(2), PrimId::Text(3), PrimId::Rule(0)],
            order: 1,
            level: None,
            role: None,
            group: None,
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
        level: None,
        role: None,
        group: None,
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
        // Each segment the cap left whole is named, and only when the cap was hit.
        let hit = limit_says(&s, "segments");
        assert_eq!(
            hit,
            !s.short.is_empty(),
            "{cap}: {:?} {:?}",
            s.issues,
            s.short
        );
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
/// 12,862,944 measured, about seven in a hundred more allowed (11,230,144 before the cut looked at the pieces of a gutter:
/// the cut now looks once more over the pieces of each gutter for a piece that stands
/// between two lines of a column, one step a piece, see `between_lines`, and each cut
/// takes the joined rules at the heights of its pieces, one step a piece, see
/// `cut::at_heights`). The cut
/// looks for bands only a few levels deep, so each level of boxes cuts its text in work in
/// proportion to it.
const STAIRCASE_BOUND: u64 = 13_800_000;

/// `n` boxes side by side, each 6 points wide and as tall as the page, with an upright line
/// down its middle (a grid of one row and two cells) and one run in its left cell.
fn boxes_side_by_side(n: usize) -> PageIn {
    let mut rules = Vec::new();
    let mut texts = Vec::new();
    for k in 0..n {
        let x = 10.0 + 12.0 * k as f32;
        rules.push(rect(x, 100.0, x + 6.0, 700.0));
        rules.push(line(x + 3.0, 100.0, x + 3.0, 700.0));
        texts.push(run("a", x + 0.5, 400.0, 2.0, 4.0, 0));
    }
    page([12.0 * n as f32 + 20.0, 800.0], texts, rules)
}

/// The grid search walks across the page and counts the lines that meet, so grids
/// side by side whose lines share their heights cost about their lines. Before, each upright
/// line was tested against every flat line in its height: 400 grids took 960,000 steps and
/// 800 took 3,840,000, four times as many.
#[test]
fn grids_side_by_side_are_searched_in_work_about_their_lines() {
    let work = |n: usize| {
        let p = boxes_side_by_side(n);
        let before = work_steps();
        let found = layoutregions::segment::drawn::grids(&p.rules).expect("searched");
        let after = work_steps();
        assert_eq!(found.groups.len(), n);
        assert!(
            found
                .groups
                .iter()
                .all(|(h, v)| h.len() == 2 && v.len() == 3)
        );
        before.zip(after).map(|(b, a)| a - b)
    };
    let (Some(small), Some(large)) = (work(400), work(800)) else {
        // A release build does not count work.
        return;
    };
    assert!(
        large * 10 < 22 * small,
        "{small} steps for 400, {large} for 800"
    );
    assert!(large < 200_000, "{large} steps for 800");
}

/// The lines that meet are found whatever their order, and lines that only come near are
/// not: a flat line that ends more than a snap before an upright one, and one that lies more
/// than a snap over its top, stay out of its grid.
#[test]
fn the_grid_walk_finds_the_lines_that_meet_and_no_others() {
    let rules = vec![
        // A frame of four lines, given from right to left and top to bottom.
        line(100.0, 0.0, 100.0, 50.0),
        line(0.0, 50.0, 100.0, 50.0),
        line(0.0, 0.0, 100.0, 0.0),
        line(0.0, 0.0, 0.0, 50.0),
        // Ends 3 points before the right side, at its height: meets the left side only.
        line(-20.0, 25.0, 97.0, 25.0),
        // 3 points over the top of the frame: meets nothing.
        line(0.0, 53.0, 100.0, 53.0),
        // An upright line of its own, away from all.
        line(200.0, 0.0, 200.0, 50.0),
    ];
    let found = layoutregions::segment::drawn::grids(&rules).expect("searched");
    assert_eq!(found.groups.len(), 1);
    let (h, v) = &found.groups[0];
    let heights: Vec<f32> = h.iter().map(|&j| found.hs[j].at).collect();
    let places: Vec<f32> = v.iter().map(|&k| found.vs[k].at).collect();
    assert_eq!(heights, [0.0, 25.0, 50.0]);
    assert_eq!(places, [0.0, 100.0]);
}

/// A box nests in the boxes at its own place, found through an index of all boxes,
/// so boxes side by side are nested in work about their number. Before, each box was tested
/// against every box before it at its depth.
#[test]
fn boxes_side_by_side_are_nested_in_work_about_their_number() {
    let work = |n: usize| {
        let p = boxes_side_by_side(n);
        let before = work_steps();
        let s = segment_page(&p, &Limits::default(), &|| false);
        let after = work_steps();
        check_cover(&s, &p);
        let grids = with_source(&s, |x| matches!(x, Source::Grid { .. }));
        assert_eq!(grids.len(), n);
        before.zip(after).map(|(b, a)| a - b)
    };
    let (Some(small), Some(large)) = (work(400), work(800)) else {
        // A release build does not count work.
        return;
    };
    eprintln!("side by side: {small} steps for 400, {large} for 800");
    assert!(
        large * 10 < 22 * small,
        "{small} steps for 400, {large} for 800"
    );
}

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
    // The page is the leaf the bound left whole.
    assert_eq!(s.short, [s.root]);
    // A page of one line has nothing to part: no issue, nothing left short.
    let p = page([595.0, 842.0], column(50.0, 700.0, 1, 0), Vec::new());
    let s = segment_page(&p, &zero, &|| false);
    assert!(s.issues.is_empty(), "{:?}", s.issues);
    assert!(s.short.is_empty());
}

/// The probe of whether a bound left text whole parts the text
/// as the walk does, strips between rules first. Three lines of one size and face, evenly
/// apart, the first two between two rules: the whitespace cut and the style steps would not
/// part them, the strip does, so a depth bound of zero leaves the page short.
#[test]
fn a_bound_that_leaves_a_strip_whole_leaves_the_page_short() {
    let texts = vec![
        run("Name Muster", 50.0, 700.0, 60.0, 9.0, 0),
        run("Street Beispielweg", 50.0, 688.0, 90.0, 9.0, 1),
        run("Further text below", 50.0, 676.0, 90.0, 9.0, 2),
    ];
    let rules = vec![
        line(40.0, 711.0, 300.0, 711.0),
        line(40.0, 684.0, 300.0, 684.0),
    ];
    let p = page([595.0, 842.0], texts.clone(), rules);
    let zero = Limits {
        max_cut_depth: 0,
        ..Limits::default()
    };
    let s = segment_page(&p, &zero, &|| false);
    check_cover(&s, &p);
    assert_eq!(s.short, [s.root], "{:?}", s.issues);
    // Without the rules nothing parts the three lines: nothing is left short.
    let p = page([595.0, 842.0], texts, Vec::new());
    let s = segment_page(&p, &zero, &|| false);
    assert!(s.short.is_empty(), "{:?}", s.issues);
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
        level: None,
        role: None,
        group: None,
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
        level: None,
        role: None,
        group: None,
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

/// The text runs a segment holds.
fn texts_of(g: &Segment) -> Vec<u32> {
    g.prims
        .iter()
        .filter_map(|p| match p {
            PrimId::Text(t) => Some(*t),
            _ => None,
        })
        .collect()
}

/// Asserts that the one grid of `s` is `want` and that its cells are boxes nested under it,
/// each holding the text `cells` names, in order of the boxes.
fn grid_holds_cells(s: &Segments, want: Source, cells: &[u32]) {
    let grids = with_source(s, |x| matches!(x, Source::Grid { .. }));
    assert_eq!(grids.len(), 1, "one grid");
    assert_eq!(grids[0].source, want);
    let boxes = with_source(s, |x| *x == Source::Box);
    assert_eq!(boxes.len(), cells.len(), "one box per cell with text");
    for b in &boxes {
        assert_eq!(b.parent, Some(grids[0].id), "a cell nests under its grid");
    }
    let mut held: Vec<u32> = boxes.iter().flat_map(|b| texts_of(b)).collect();
    held.sort_unstable();
    assert_eq!(held, cells);
}

#[test]
fn a_one_row_grid_holds_its_cells() {
    // A row of three cells drawn with lines, a word in each, and prose below.
    let rules = vec![
        line(50.0, 600.0, 350.0, 600.0),
        line(50.0, 620.0, 350.0, 620.0),
        line(50.0, 600.0, 50.0, 620.0),
        line(150.0, 600.0, 150.0, 620.0),
        line(250.0, 600.0, 250.0, 620.0),
        line(350.0, 600.0, 350.0, 620.0),
    ];
    let mut texts = vec![
        run("Muster GmbH", 55.0, 606.0, 50.0, 9.0, 0),
        run("12345", 155.0, 606.0, 25.0, 9.0, 0),
        run("1.234,56", 255.0, 606.0, 35.0, 9.0, 0),
    ];
    texts.extend(column(50.0, 500.0, 3, 1));
    let p = page([595.0, 842.0], texts, rules);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    grid_holds_cells(&s, Source::Grid { rows: 1, cols: 3 }, &[0, 1, 2]);
}

#[test]
fn a_one_column_grid_holds_its_cells() {
    // A column of three fields drawn with lines, a value in each.
    let rules = vec![
        line(200.0, 600.0, 300.0, 600.0),
        line(200.0, 620.0, 300.0, 620.0),
        line(200.0, 640.0, 300.0, 640.0),
        line(200.0, 660.0, 300.0, 660.0),
        line(200.0, 600.0, 200.0, 660.0),
        line(300.0, 600.0, 300.0, 660.0),
    ];
    let texts = vec![
        run("101", 205.0, 646.0, 15.0, 9.0, 0),
        run("nein", 205.0, 626.0, 20.0, 9.0, 1),
        run("01.01.2011", 205.0, 606.0, 45.0, 9.0, 2),
    ];
    let p = page([595.0, 842.0], texts, rules);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    grid_holds_cells(&s, Source::Grid { rows: 3, cols: 1 }, &[0, 1, 2]);
}

#[test]
fn abutting_boxes_are_one_grid() {
    // Three rectangles drawn side by side that share their sides are the cells of one row;
    // two rectangles a gap apart below them stay two boxes.
    let rules = vec![
        rect(50.0, 600.0, 150.0, 620.0),
        rect(150.0, 600.0, 250.0, 620.0),
        rect(250.0, 600.0, 350.0, 620.0),
        rect(50.0, 500.0, 150.0, 520.0),
        rect(156.0, 500.0, 256.0, 520.0),
    ];
    let texts = vec![
        run("Art", 55.0, 606.0, 15.0, 9.0, 0),
        run("Basis", 155.0, 606.0, 22.0, 9.0, 0),
        run("Betrag", 255.0, 606.0, 27.0, 9.0, 0),
        run("Erika", 55.0, 506.0, 22.0, 9.0, 1),
        run("Mustermann", 160.0, 506.0, 50.0, 9.0, 1),
    ];
    let p = page([595.0, 842.0], texts, rules);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    let grids = with_source(&s, |x| matches!(x, Source::Grid { .. }));
    assert_eq!(grids.len(), 1, "one grid");
    assert_eq!(grids[0].source, Source::Grid { rows: 1, cols: 3 });
    assert_eq!(texts_of(grids[0]), [0, 1, 2]);
    let boxes = with_source(&s, |x| *x == Source::Box);
    assert_eq!(boxes.len(), 5);
    let apart: Vec<&&Segment> = boxes.iter().filter(|b| b.parent == Some(s.root)).collect();
    assert_eq!(apart.len(), 2, "boxes a gap apart are no grid");
    for b in boxes.iter().filter(|b| b.parent != Some(s.root)) {
        assert_eq!(b.parent, Some(grids[0].id));
    }
}

#[test]
fn a_grid_over_most_of_the_page_is_a_segment() {
    // A grid of three rows and three columns that covers most of the page (a form drawn in
    // its frame): it is a segment; whether it is a table is decided later.
    let (x0, y0, x1, y1) = (20.0, 20.0, 575.0, 820.0);
    let mut rules = vec![rect(x0, y0, x1, y1)];
    rules.push(line(205.0, y0, 205.0, y1));
    rules.push(line(390.0, y0, 390.0, y1));
    rules.push(line(x0, 286.0, x1, 286.0));
    rules.push(line(x0, 553.0, x1, 553.0));
    let texts = vec![
        run("Muster GmbH", 30.0, 800.0, 50.0, 9.0, 0),
        run("12345 Musterstadt", 215.0, 800.0, 75.0, 9.0, 0),
        run("1.234,56", 30.0, 500.0, 35.0, 9.0, 1),
    ];
    let p = page([595.0, 842.0], texts, rules);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    let grids = with_source(&s, |x| matches!(x, Source::Grid { .. }));
    assert_eq!(grids.len(), 1, "the grid is a segment");
    assert_eq!(grids[0].source, Source::Grid { rows: 3, cols: 3 });
    assert_eq!(grids[0].parent, Some(s.root));
    assert_eq!(texts_of(grids[0]), [0, 1, 2]);
}

#[test]
fn a_host_found_structure_decides_the_boxes() {
    // Two drawn boxes; the host found text in the second only (its own test of where text
    // lies), so only the second is a box segment, though a run's centre lies in each. The
    // host names what its own search left out: no issue for it here.
    let rules = vec![
        rect(50.0, 600.0, 250.0, 640.0),
        rect(300.0, 600.0, 500.0, 640.0),
    ];
    let texts = vec![
        run("Muster GmbH", 60.0, 615.0, 50.0, 9.0, 0),
        run("12345 Musterstadt", 310.0, 615.0, 75.0, 9.0, 0),
    ];
    let p = page([595.0, 842.0], texts, rules);
    let mut found = drawn(&p, &|b: Bounds| b.x0 > 280.0);
    assert_eq!(found.boxes.len(), 1);
    let s = segment_drawn(&p, &found, &Limits::default(), &|| false);
    check_cover(&s, &p);
    let boxes = with_source(&s, |x| *x == Source::Box);
    assert_eq!(boxes.len(), 1);
    assert_eq!(texts_of(boxes[0]), [1]);
    assert!(s.issues.is_empty());
    // A structure whose boxes were not used: the host says so, the segments do not.
    found.boxes.clear();
    found.boxes_note = Some("its boxes overlap too much".into());
    let s = segment_drawn(&p, &found, &Limits::default(), &|| false);
    check_cover(&s, &p);
    assert!(with_source(&s, |x| *x == Source::Box).is_empty());
    assert!(s.issues.is_empty());
    // Without a structure the segmentation finds its own and says what it left out.
    let s = segment_page(&p, &Limits::default(), &|| false);
    assert_eq!(with_source(&s, |x| *x == Source::Box).len(), 2);
}

#[test]
fn many_one_row_grids_are_bounded() {
    // 200 rows of three drawn cells each, a word in each cell: a grid and three boxes per
    // row. A small cap on the segments holds, says so once, and keeps every prim in one leaf.
    let mut rules = Vec::new();
    let mut texts = Vec::new();
    for k in 0..200u32 {
        let y = 4_000.0 - 20.0 * k as f32;
        for c in 0..3u32 {
            let x = 50.0 + 100.0 * c as f32;
            rules.push(rect(x, y, x + 100.0, y + 16.0));
            texts.push(run("Muster", x + 5.0, y + 5.0, 30.0, 9.0, k));
        }
    }
    let p = page([595.0, 4_100.0], texts, rules);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    assert_eq!(
        with_source(&s, |x| *x == Source::Grid { rows: 1, cols: 3 }).len(),
        200
    );
    assert_eq!(with_source(&s, |x| *x == Source::Box).len(), 600);
    let small = Limits {
        max_regions: 300,
        ..Limits::default()
    };
    let s = segment_page(&p, &small, &|| false);
    check_cover(&s, &p);
    assert!(s.all.len() <= 300, "{}", s.all.len());
    let hits: Vec<&Issue> = s
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Limit)
        .collect();
    assert_eq!(hits.len(), 1, "{:?}", s.issues);
    assert!(hits[0].what.contains("segments"), "{}", hits[0].what);
}

/// Pictures and codes are each a leaf of their own, beside one leaf of the rules: under a
/// page with text, and under a page of pictures alone.
#[test]
fn each_picture_and_code_is_a_leaf_of_its_own() {
    let pic = |x0: f32, y0: f32| PictureIn {
        bounds: Bounds {
            x0,
            y0,
            x1: x0 + 100.0,
            y1: y0 + 80.0,
        },
    };
    let mut p = page(
        [595.0, 842.0],
        column(72.0, 760.0, 3, 0),
        vec![line(72.0, 700.0, 500.0, 700.0)],
    );
    p.pictures = vec![pic(72.0, 500.0), pic(300.0, 500.0)];
    p.codes = vec![CodeIn {
        bounds: Bounds {
            x0: 72.0,
            y0: 300.0,
            x1: 172.0,
            y1: 340.0,
        },
    }];
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    let drawings: Vec<Vec<PrimId>> = with_source(&s, |x| *x == Source::Drawing)
        .iter()
        .map(|g| g.prims.clone())
        .collect();
    assert_eq!(
        drawings,
        [
            vec![PrimId::Rule(0)],
            vec![PrimId::Picture(0)],
            vec![PrimId::Picture(1)],
            vec![PrimId::Code(0)],
        ]
    );
    // Pictures alone: one leaf each, the root parted; one picture alone is the root itself.
    let mut only = page([595.0, 842.0], Vec::new(), Vec::new());
    only.pictures = vec![pic(72.0, 500.0), pic(300.0, 500.0)];
    let s = segment_page(&only, &Limits::default(), &|| false);
    check_cover(&s, &only);
    assert_eq!(with_source(&s, |x| *x == Source::Drawing).len(), 2);
    only.pictures.truncate(1);
    let s = segment_page(&only, &Limits::default(), &|| false);
    assert_eq!(s.all.len(), 1);
}

/// The lines between two rules that span them stay one strip before the
/// whitespace cut, which would part its columns; a strip of one line is a row and is cut as
/// text; the text outside the strips is cut as before; every primitive in one leaf.
#[test]
fn lines_between_two_rules_are_one_strip() {
    for k in [0.75f32, 1.0, 1.5] {
        let mut texts = Vec::new();
        // A strip of two columns of fields, far apart: labels over values.
        for (c, (label, value)) in [("Personalnummer", "00123"), ("Eintritt", "01.01.2020")]
            .iter()
            .enumerate()
        {
            let x = (72.0 + 250.0 * c as f32) * k;
            texts.push(run(label, x, 690.0 * k, 60.0 * k, 6.0 * k, 10));
            texts.push(run(value, x, 678.0 * k, 50.0 * k, 9.0 * k, 11));
        }
        // A row between rules of its own, and text under the rules.
        texts.push(run("Muster", 72.0 * k, 655.0 * k, 40.0 * k, 9.0 * k, 12));
        texts.push(run("12,00", 322.0 * k, 655.0 * k, 30.0 * k, 9.0 * k, 12));
        texts.extend(column(72.0 * k, 600.0 * k, 3, 20).into_iter().map(|mut r| {
            r.size *= k;
            r.h *= k;
            r.w *= k;
            r
        }));
        let rules: Vec<RuleIn> = [700.0, 670.0, 645.0]
            .iter()
            .map(|&y| line(70.0 * k, y * k, 520.0 * k, y * k))
            .collect();
        let p = page([600.0 * k, 800.0 * k], texts, rules);
        let s = segment_page(&p, &Limits::default(), &|| false);
        check_cover(&s, &p);
        let strips = with_source(&s, |x| *x == Source::Strip);
        assert_eq!(strips.len(), 1, "at {k}x: {:#?}", s.all);
        let mut lines: Vec<u32> = strips[0]
            .prims
            .iter()
            .filter_map(|q| match q {
                PrimId::Text(i) => Some(p.texts[*i as usize].line),
                _ => None,
            })
            .collect();
        lines.sort_unstable();
        lines.dedup();
        assert_eq!(lines, [10, 11], "at {k}x");
        // No gutter of the cut parts the strip.
        assert!(
            strips[0]
                .children
                .iter()
                .all(|c| seg(&s, *c).source == Source::Style),
            "at {k}x"
        );
    }
}

/// A strip holds two to six lines of text between its rules; with seven the
/// lines between the rules are a page section, cut as text. The count is of lines, which a
/// page printed larger or smaller keeps.
#[test]
fn a_strip_holds_six_lines_at_most() {
    for (n, strips) in [(6u32, 1usize), (7, 0)] {
        let texts: Vec<TextRun> = (0..n)
            .map(|k| run("Zeile", 72.0, 690.0 - 10.0 * k as f32, 40.0, 8.0, 10 + k))
            .chain([run(PROSE, 72.0, 500.0, 200.0, 8.0, 40)])
            .collect();
        let top = 700.0;
        let bottom = 690.0 - 10.0 * n as f32;
        let p = page(
            [600.0, 800.0],
            texts,
            vec![
                line(70.0, top, 520.0, top),
                line(70.0, bottom, 520.0, bottom),
            ],
        );
        let s = segment_page(&p, &Limits::default(), &|| false);
        check_cover(&s, &p);
        assert_eq!(
            with_source(&s, |x| *x == Source::Strip).len(),
            strips,
            "{n} lines"
        );
    }
}

/// The page with every run cut into its words: each word a run of its own, placed by its
/// share of the characters of the run.
fn word_by_word(p: &PageIn) -> PageIn {
    let mut texts = Vec::new();
    for r in &p.texts {
        let n = r.text.chars().count().max(1) as f32;
        let mut at = 0usize;
        for word in r.text.split(' ') {
            let len = word.chars().count();
            if len > 0 {
                texts.push(TextRun {
                    text: word.to_string(),
                    x: r.x + r.w * at as f32 / n,
                    w: r.w * len as f32 / n,
                    ..r.clone()
                });
            }
            at += len + 1;
        }
    }
    PageIn { texts, ..p.clone() }
}

/// The text of each leaf of the main cut with text, its runs by line and place, sorted.
fn leaf_texts(s: &Segments, p: &PageIn) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![s.root];
    while let Some(id) = stack.pop() {
        let g = seg(s, id);
        if !g.children.is_empty() {
            stack.extend(g.children.iter().copied());
            continue;
        }
        let mut runs: Vec<&TextRun> = texts_of(g).iter().map(|&t| &p.texts[t as usize]).collect();
        runs.sort_by(|a, b| a.line.cmp(&b.line).then(a.x.total_cmp(&b.x)));
        if !runs.is_empty() {
            out.push(
                runs.iter()
                    .map(|r| r.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
    }
    out.sort();
    out
}

const SENTENCE: &str = "the green lamp stands beside a wooden shelf";

/// Two columns of prose under a line across them are parted the same
/// whether each line is one run or a run per word. The column gutter needs lines of prose on
/// both sides and few pieces across it; both count the pieces of the lines, not their runs.
#[test]
fn columns_are_cut_the_same_word_by_word() {
    let mut texts = vec![run(
        "a note across both columns of the page",
        200.0,
        700.0,
        190.0,
        9.0,
        0,
    )];
    for k in 0..6u32 {
        let y = 670.0 - 12.0 * k as f32;
        texts.push(run(SENTENCE, 72.0, y, 200.0, 9.0, 1 + k));
        texts.push(run(SENTENCE, 320.0, y, 200.0, 9.0, 1 + k));
    }
    let whole = page([600.0, 800.0], texts, Vec::new());
    let words = word_by_word(&whole);
    assert!(words.texts.len() > 4 * whole.texts.len());
    let (a, b) = (
        segment_page(&whole, &Limits::default(), &|| false),
        segment_page(&words, &Limits::default(), &|| false),
    );
    check_cover(&a, &whole);
    check_cover(&b, &words);
    let column = [SENTENCE; 6].join(" ");
    assert_eq!(
        leaf_texts(&a, &whole),
        vec![
            "a note across both columns of the page".to_string(),
            column.clone(),
            column
        ]
    );
    assert_eq!(leaf_texts(&b, &words), leaf_texts(&a, &whole));
    assert_eq!(with_source(&b, |x| *x == Source::Gutter).len(), 3);
}

/// The lines between two rules stand in their strip whole, by the middle
/// of the line, also when the line reaches past the rules and is cut into words: the words
/// past the end of the rules stay with their line.
#[test]
fn a_line_stands_in_its_strip_whole_word_by_word() {
    let mut texts = vec![
        run(SENTENCE, 72.0, 690.0, 258.0, 9.0, 10),
        run(SENTENCE, 72.0, 678.0, 258.0, 9.0, 11),
    ];
    texts.extend(column(72.0, 600.0, 3, 20));
    // The rules end at 240, the lines at 330: their middles lie between the rules.
    let rules = vec![
        line(70.0, 702.0, 240.0, 702.0),
        line(70.0, 670.0, 240.0, 670.0),
    ];
    let whole = page([600.0, 800.0], texts, rules);
    let words = word_by_word(&whole);
    for p in [&whole, &words] {
        let s = segment_page(p, &Limits::default(), &|| false);
        check_cover(&s, p);
        let strips = with_source(&s, |x| *x == Source::Strip);
        assert_eq!(strips.len(), 1, "{:#?}", s.all);
        let mut held: Vec<&TextRun> = texts_of(strips[0])
            .iter()
            .map(|&t| &p.texts[t as usize])
            .collect();
        held.sort_by(|a, b| a.line.cmp(&b.line).then(a.x.total_cmp(&b.x)));
        let text: Vec<&str> = held.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text.join(" "), [SENTENCE, SENTENCE].join(" "));
    }
}

/// The page with every line rule drawn in three pieces that touch.
fn rules_in_pieces(p: &PageIn) -> PageIn {
    let mut rules = Vec::new();
    for r in &p.rules {
        if r.rect {
            rules.push(r.clone());
            continue;
        }
        for k in 0..3 {
            let at = |t: f32| (r.x0 + (r.x1 - r.x0) * t, r.y0 + (r.y1 - r.y0) * t);
            let ((x0, y0), (x1, y1)) = (at(k as f32 / 3.0), at((k + 1) as f32 / 3.0));
            rules.push(RuleIn {
                x0,
                y0,
                x1,
                y1,
                ..r.clone()
            });
        }
    }
    PageIn { rules, ..p.clone() }
}

/// The source and the text runs of every segment that holds text, in the order of the tree.
fn text_tree(s: &Segments) -> Vec<(Source, Vec<u32>)> {
    s.all
        .iter()
        .map(|g| (g.source, texts_of(g)))
        .filter(|(_, t)| !t.is_empty())
        .collect()
}

/// A rule drawn in pieces that touch bounds the same strip and parts the
/// same bands of two columns as the rule drawn in one: the pieces are joined before the
/// strips and the rule cut look at them.
#[test]
fn rules_in_pieces_bound_the_same_strips_and_bands() {
    // A strip of two fields between two rules, a row between rules of its own.
    let mut texts = vec![
        run("first label", 72.0, 690.0, 60.0, 6.0, 10),
        run("00123", 72.0, 678.0, 50.0, 9.0, 11),
        run("second label", 322.0, 690.0, 60.0, 6.0, 10),
        run("00456", 322.0, 678.0, 50.0, 9.0, 11),
    ];
    // Two columns of prose, a rule across their gutter between the eighth and ninth line:
    // too many lines over it for a strip.
    for k in 0..12u32 {
        let y = 600.0 - 12.0 * k as f32;
        texts.push(run(SENTENCE, 72.0, y, 200.0, 9.0, 20 + k));
        texts.push(run(SENTENCE, 320.0, y, 200.0, 9.0, 20 + k));
    }
    let rules: Vec<RuleIn> = [700.0, 670.0, 514.5]
        .iter()
        .map(|&y| line(70.0, y, 522.0, y))
        .collect();
    let whole = page([600.0, 800.0], texts, rules);
    let pieces = rules_in_pieces(&whole);
    assert_eq!(pieces.rules.len(), 9);
    let (a, b) = (
        segment_page(&whole, &Limits::default(), &|| false),
        segment_page(&pieces, &Limits::default(), &|| false),
    );
    check_cover(&a, &whole);
    check_cover(&b, &pieces);
    assert_eq!(with_source(&a, |x| *x == Source::Strip).len(), 1);
    assert!(!with_source(&a, |x| *x == Source::Rule).is_empty());
    assert_eq!(text_tree(&b), text_tree(&a));
}

/// White space after a label, set up to the next column (the
/// padding of a form field), joins the label's piece and bridges nothing: the label of the
/// next column is a piece of its own, so the gutter between the two columns of labels over
/// values is still found, with or without the padding.
#[test]
fn padding_up_to_the_next_column_bridges_no_gutter() {
    let page_of = |padded: bool| {
        let mut texts = Vec::new();
        for k in 0..6u32 {
            let y = 690.0 - 12.0 * k as f32;
            texts.push(run(SENTENCE, 72.0, y, 200.0, 9.0, k));
            if padded {
                // From the end of the left line to the start of the right one.
                texts.push(run("      ", 272.0, y, 46.0, 9.0, k));
            }
            texts.push(run(SENTENCE, 320.0, y, 200.0, 9.0, k));
        }
        page([600.0, 800.0], texts, Vec::new())
    };
    let inked = |s: &Segments, p: &PageIn| -> Vec<String> {
        leaf_texts(s, p)
            .into_iter()
            .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|t| !t.is_empty())
            .collect()
    };
    let (plain, padded) = (page_of(false), page_of(true));
    let (a, b) = (
        segment_page(&plain, &Limits::default(), &|| false),
        segment_page(&padded, &Limits::default(), &|| false),
    );
    check_cover(&b, &padded);
    let column = [SENTENCE; 6].join(" ");
    assert_eq!(inked(&a, &plain), vec![column.clone(), column]);
    assert_eq!(inked(&b, &padded), inked(&a, &plain));
}

/// Paragraphs of three lines, the last one short, 12 points from line to line and 30 from
/// paragraph to paragraph, from line `first` on.
fn short_ended_paragraphs(n: u32, top: f32, first: u32) -> Vec<TextRun> {
    let mut texts = Vec::new();
    for k in 0..n {
        let y = top - 54.0 * k as f32;
        let wide = [SENTENCE, SENTENCE].join(" ");
        texts.push(run(&wide, 72.0, y, 440.0, 9.0, first + 3 * k));
        texts.push(run(&wide, 72.0, y - 12.0, 440.0, 9.0, first + 3 * k + 1));
        texts.push(run(
            "beside a wooden shelf",
            72.0,
            y - 24.0,
            105.0,
            9.0,
            first + 3 * k + 2,
        ));
    }
    texts
}

/// Two rules far apart, one over the other and both short (the
/// edges of a bar, a mark in the margin), bound a tall narrow frame. Full lines of the
/// paragraphs run through it; only the short last lines have their middles in it. The frame
/// is no strip: the short lines stay with the lines of their paragraphs, where the strip
/// took two of them out as one region across the paragraphs between them.
#[test]
fn a_frame_that_lines_of_text_run_through_is_no_strip() {
    let rules = vec![
        line(100.0, 712.0, 150.0, 712.0),
        line(100.0, 540.0, 150.0, 540.0),
    ];
    let p = page([600.0, 800.0], short_ended_paragraphs(3, 700.0, 0), rules);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    assert!(
        with_source(&s, |x| *x == Source::Strip).is_empty(),
        "{:#?}",
        s.all
    );
    // Every leaf with text holds lines that follow each other.
    for g in s.all.iter().filter(|g| g.children.is_empty()) {
        let mut lines: Vec<u32> = texts_of(g)
            .iter()
            .map(|&t| p.texts[t as usize].line)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        assert!(lines.windows(2).all(|w| w[1] == w[0] + 1), "{lines:?}");
    }
}

/// The lines of a strip follow each other. Two lines whose
/// middles lie in a frame, with a shorter line of the same text between them that ends left
/// of the frame, are lines of that text, not a strip of two.
#[test]
fn the_lines_of_a_strip_follow_each_other() {
    let wide = [SENTENCE, SENTENCE].join(" ");
    let texts = vec![
        run(&wide, 72.0, 690.0, 440.0, 9.0, 0),
        run("a short line", 72.0, 678.0, 60.0, 9.0, 1),
        run("another short one", 72.0, 666.0, 85.0, 9.0, 2),
        run(&wide, 72.0, 654.0, 440.0, 9.0, 3),
        run("the end", 72.0, 642.0, 40.0, 9.0, 4),
    ];
    // The frame reaches from 200 to 400: the middles of the two full lines lie in it, the
    // short lines end left of it.
    let rules = vec![
        line(200.0, 702.0, 400.0, 702.0),
        line(200.0, 636.0, 400.0, 636.0),
    ];
    let p = page([600.0, 800.0], texts, rules);
    let s = segment_page(&p, &Limits::default(), &|| false);
    check_cover(&s, &p);
    assert!(
        with_source(&s, |x| *x == Source::Strip).is_empty(),
        "{:#?}",
        s.all
    );
}

/// A piece whose ink touches the side of a frame by a hair (a
/// label in the margin that ends where the rules start) runs through nothing: the lines
/// between the two rules stay one strip.
#[test]
fn ink_that_touches_a_frame_by_a_hair_leaves_the_strip() {
    let strip = |margin_end: f32| {
        let mut texts = vec![
            run("first label", 122.0, 690.0, 60.0, 6.0, 10),
            run("00123", 122.0, 678.0, 50.0, 9.0, 11),
            run("second label", 322.0, 690.0, 60.0, 6.0, 10),
            run("00456", 322.0, 678.0, 50.0, 9.0, 11),
            // A label in the margin at the height of the strip's second line.
            run(
                "a note in the margin",
                margin_end - 90.0,
                684.0,
                90.0,
                9.0,
                12,
            ),
        ];
        texts.extend(column(72.0, 600.0, 3, 20));
        let rules = vec![
            line(120.0, 700.0, 520.0, 700.0),
            line(120.0, 670.0, 520.0, 670.0),
        ];
        let p = page([600.0, 800.0], texts, rules);
        let s = segment_page(&p, &Limits::default(), &|| false);
        check_cover(&s, &p);
        with_source(&s, |x| *x == Source::Strip).len()
    };
    // The note ends 0.3 points inside the frame: a hair. Ending 30 points inside, with its
    // middle outside, it runs into the frame.
    assert_eq!(strip(120.3), 1);
    assert_eq!(strip(150.0), 0);
}

/// One line of a date and, a cell gap right of it, a label with its
/// colon and its value is two segments, the date for itself and the label with its value.
/// A first run that is words (a label of its own), a second run that is no label with a
/// value, and a second run a word space after the first stay one segment with the first.
#[test]
fn a_value_beside_a_label_with_its_value_is_parted_from_it() {
    let leaves = |first: &str, second: &str, gap: f32| {
        let size = 10.0;
        let w = |t: &str| 0.6 * size * t.chars().count() as f32;
        let mut texts = vec![
            run(first, 40.0, 700.0, w(first), size, 0),
            run(second, 40.0 + w(first) + gap, 700.0, w(second), size, 0),
        ];
        // A column of prose far under the line: the line is a band of the page.
        texts.extend(column(40.0, 500.0, 4, 1));
        let p = page([595.0, 842.0], texts, Vec::new());
        let s = segment_page(&p, &Limits::default(), &|| false);
        check_cover(&s, &p);
        // The leaves that hold a run of the line, by their number of runs.
        s.all
            .iter()
            .filter(|g| g.children.is_empty())
            .filter(|g| {
                g.prims
                    .iter()
                    .any(|p| matches!(p, PrimId::Text(t) if *t < 2))
            })
            .map(|g| g.prims.len())
            .collect::<Vec<_>>()
    };
    assert_eq!(leaves("03.04.2026", "Sheet:  4", 150.0), [1, 1]);
    assert_eq!(leaves("127,50", "Sheet:  4", 150.0), [1, 1]);
    assert_eq!(leaves("Ledger", "Sheet:  4", 150.0), [2], "words");
    assert_eq!(leaves("03.04.2026", "Sheet", 150.0), [2], "no pair");
    // The cell gap is one text size (`CELL_GAP`).
    assert_eq!(leaves("03.04.2026", "Sheet:  4", 10.5), [1, 1]);
    assert_eq!(leaves("03.04.2026", "Sheet:  4", 9.5), [2], "a word space");
}

/// Two labels of a print form, each over the upper left of its
/// field (a drawn row of digit boxes, lower and to the right), are parted into a segment
/// each, to be read with their fields. The fields are asked from one step of the labels
/// under the lowest label up to the highest one, right of the labels: a field at the step
/// under the lowest label, a field at the height of the highest label and a field that
/// starts left of the labels' right edge are no fields of these labels, and with them
/// left out the rows do not match.
#[test]
fn labels_beside_drawn_fields_are_parted_into_their_lines() {
    // A field: three digit boxes in a row from `x`, with a digit in each, on baseline `y`.
    let field = |x: f32, y: f32, line: u32| {
        let rules: Vec<RuleIn> = (0..3)
            .map(|k| {
                let x0 = x + 12.0 * k as f32;
                rect(x0, y - 3.0, x0 + 12.0, y + 9.0)
            })
            .collect();
        let texts: Vec<TextRun> = (0..3)
            .map(|k| run("7", x + 3.0 + 12.0 * k as f32, y, 5.5, 9.5, line))
            .collect();
        (texts, rules)
    };
    // The labels at 470 and 440 (a step of 30, their right edge at 320), their fields on
    // the baselines 460 and 430 from 382, and one more field at (`x`, `y`) when given.
    let leaves = |third: Option<(f32, f32)>| {
        let mut texts = vec![
            run("Amount", 300.0, 470.0, 20.0, 6.0, 0),
            run("Due on", 300.0, 440.0, 20.0, 6.0, 1),
        ];
        let mut rules = Vec::new();
        let at = [Some((382.0, 460.0)), Some((382.0, 430.0)), third];
        for (k, (x, y)) in at.into_iter().flatten().enumerate() {
            let (t, r) = field(x, y, 2 + k as u32);
            texts.extend(t);
            rules.extend(r);
        }
        texts.extend(column(40.0, 300.0, 4, 10));
        let p = page([595.0, 842.0], texts, rules);
        let s = segment_page(&p, &Limits::default(), &|| false);
        check_cover(&s, &p);
        // The leaves of the main cut that hold a label, by their labels.
        let mut found: Vec<Vec<u32>> = Vec::new();
        let mut stack = vec![s.root];
        while let Some(id) = stack.pop() {
            let g = seg(&s, id);
            stack.extend(g.children.iter().copied());
            let labels: Vec<u32> = texts_of(g).into_iter().filter(|&t| t < 2).collect();
            if g.children.is_empty() && !labels.is_empty() {
                found.push(labels);
            }
        }
        found.sort();
        found
    };
    let (apart, whole) = (vec![vec![0], vec![1]], vec![vec![0, 1]]);
    assert_eq!(leaves(None), apart);
    // One step of the labels (30) under the lowest label (440): a field on 410 or lower
    // is the field of another label, one just over it is asked and has no label here.
    assert_eq!(leaves(Some((382.0, 410.0))), apart);
    assert_eq!(leaves(Some((382.0, 410.1))), whole);
    // A field on the baseline of the highest label (470) or over it is none of theirs.
    assert_eq!(leaves(Some((450.0, 470.0))), apart);
    assert_eq!(leaves(Some((450.0, 469.9))), whole);
    // A field whose first digit starts left of the labels' right edge (320) is none.
    assert_eq!(leaves(Some((316.9, 452.0))), apart);
    assert_eq!(leaves(Some((317.1, 452.0))), whole);
}

/// A data line of a date and a number, with a small
/// label that ends with a colon set into the blank right before the number, a little over
/// the baseline: the label and the number are a segment of their own, one plain line
/// ([`Source::Line`], no pair is claimed), and the date stays for itself, with a title left
/// of it when there is one. Each sign is needed: the colon, the smaller size of the label,
/// one baseline, the label nearer to the number than to the date, text of the larger face
/// before the label on the baseline, and the number the last of the line.
#[test]
fn a_small_colon_label_on_a_data_line_is_parted_from_what_stands_before_it() {
    /// One run of the line: its text, its left edge, its size and its baseline.
    type Part = (&'static str, f32, f32, f32);
    let leaves = |parts: &[Part]| {
        let mut texts: Vec<TextRun> = parts
            .iter()
            .map(|&(t, x, size, y)| run(t, x, y, 0.6 * size * t.chars().count() as f32, size, 0))
            .collect();
        // A column of prose far under the line: the line is a band of the page.
        texts.extend(column(40.0, 500.0, 4, 1));
        let p = page([595.0, 842.0], texts, Vec::new());
        let s = segment_page(&p, &Limits::default(), &|| false);
        check_cover(&s, &p);
        // The parts of the line are its plain lines (the head claims no pair
        // either): the head from the first run of the line on, and the label with the text
        // after it, a leaf that ends with the last run of the line, the label right before
        // that run's stretch.
        let mut plain: Vec<Vec<u32>> = Vec::new();
        for g in s.all.iter().filter(|g| g.source == Source::Line) {
            let own = texts_of(g);
            assert!(g.children.is_empty(), "{own:?}");
            assert!(own.windows(2).all(|w| w[0] + 1 == w[1]), "{own:?}");
            plain.push(own);
        }
        plain.sort();
        if let [head, labelled] = &plain[..] {
            assert_eq!(head[0], 0, "{plain:?}");
            assert_eq!(head.last().map(|t| t + 1), Some(labelled[0]), "{plain:?}");
            assert!(labelled.len() >= 2, "{plain:?}");
            assert_eq!(
                labelled.last(),
                Some(&(parts.len() as u32 - 1)),
                "{plain:?}"
            );
        }
        let lines = plain.len();
        // The leaves that hold a run of the line, by their runs, left to right.
        let mut found: Vec<Vec<u32>> = s
            .all
            .iter()
            .filter(|g| g.children.is_empty())
            .map(|g| {
                g.prims
                    .iter()
                    .filter_map(|p| match p {
                        PrimId::Text(t) if (*t as usize) < parts.len() => Some(*t),
                        _ => None,
                    })
                    .collect::<Vec<u32>>()
            })
            .filter(|g| !g.is_empty())
            .collect();
        found.sort();
        assert_eq!(lines, 2 * usize::from(found.len() > 1), "{found:?}");
        found
    };
    let date: Part = ("03.04.2026", 300.0, 10.0, 700.0);
    // The label ends at 424.2, six points before the number at 430.
    let label: Part = ("Sheet:", 399.0, 7.0, 701.2);
    let value: Part = ("4", 430.0, 10.0, 700.0);
    let whole = vec![vec![0, 1, 2]];
    assert_eq!(leaves(&[date, label, value]), [vec![0], vec![1, 2]]);
    // With a title left of the date, a little higher than the label.
    let title: Part = ("Ledger of the month", 40.0, 10.0, 701.5);
    assert_eq!(
        leaves(&[title, date, label, value]),
        [vec![0, 1], vec![2, 3]]
    );
    // No colon; a label in the size of the number.
    assert_eq!(leaves(&[date, ("Sheet", 399.0, 7.0, 701.2), value]), whole);
    assert_eq!(
        leaves(&[date, ("Sheet:", 390.0, 10.0, 700.0), value]),
        whole
    );
    // What stands after the label is not asked what it is (the line is only
    // parted, no pair is claimed): a second label after it stays on its line too.
    assert_eq!(
        leaves(&[date, label, ("No:", 430.0, 10.0, 700.0)]),
        [vec![0], vec![1, 2]]
    );
    // The size step (`SIZE_STEP`, 1.08): a label of 9.2 is smaller, one of 9.4 is not.
    assert_eq!(
        leaves(&[date, ("Sheet:", 390.0, 9.2, 700.0), value]),
        [vec![0], vec![1, 2]]
    );
    assert_eq!(leaves(&[date, ("Sheet:", 390.0, 9.4, 700.0), value]), whole);
    // One baseline, within `BASELINE_NEAR` of the size of the number (1.5).
    assert_eq!(
        leaves(&[date, ("Sheet:", 399.0, 7.0, 701.4), value]),
        [vec![0], vec![1, 2]]
    );
    assert_eq!(leaves(&[date, ("Sheet:", 399.0, 7.0, 701.6), value]), whole);
    // The date ends at 360. A label that ends 5 points before the number and starts 6
    // after the date is the number's; one that starts 4 after the date is nearer to it.
    assert_eq!(
        leaves(&[
            date,
            ("Sheet:", 366.0, 7.0, 701.2),
            ("4", 396.2, 10.0, 700.0)
        ]),
        [vec![0], vec![1, 2]]
    );
    assert_eq!(
        leaves(&[
            date,
            ("Sheet:", 364.0, 7.0, 701.2),
            ("4", 394.2, 10.0, 700.0)
        ]),
        whole
    );
    // What follows the number in its size: within `PIECE_GAP` sizes (8 points) the number
    // goes on in it. Farther off it is something else, and the number is the label's when
    // it stands nearer to the label than to that: the label ends 9.8 before a number at
    // 434 that ends at 440, so with what follows 11 after it, not with it 9 after it.
    let more = |x: f32| -> Part { ("of 9", x, 10.0, 700.0) };
    assert_eq!(
        leaves(&[date, label, value, more(441.0)]),
        [vec![0], vec![1, 2, 3]]
    );
    // Farther off it is something else, and the number does not end the line: the line
    // is kept whole.
    let far: Part = ("4", 434.0, 10.0, 700.0);
    assert_eq!(
        leaves(&[date, label, far, more(451.0)]),
        vec![vec![0, 1, 2, 3]]
    );
    assert_eq!(
        leaves(&[date, label, far, more(449.0)]),
        vec![vec![0, 1, 2, 3]]
    );
    // However the words are cut into runs: a title and a label of two runs each.
    assert_eq!(
        leaves(&[
            ("Ledger of", 40.0, 10.0, 701.5),
            ("the month", 100.0, 10.0, 701.5),
            date,
            ("Sheet ", 386.4, 7.0, 701.2),
            ("no:", 411.6, 7.0, 701.2),
            value
        ]),
        [vec![0, 1, 2], vec![3, 4, 5]]
    );
    // A label that heads a row of values takes none of them. The label
    // ends 8 points before the first amount, the amounts stand 14 and more apart.
    let total: Part = ("Total:", 312.0, 6.6, 700.0);
    let first: Part = ("1.234,56", 344.0, 9.5, 700.0);
    let second: Part = ("567,89", 404.0, 9.5, 700.0);
    assert_eq!(leaves(&[total, first, second]), whole);
    assert_eq!(
        leaves(&[total, first, second, ("2.000,00", 464.0, 9.5, 700.0)]),
        vec![vec![0, 1, 2, 3]]
    );
    // Nor with a word after the one amount: the label opens its line and
    // the amount does not end it, so it is not set into a data line.
    assert_eq!(leaves(&[total, first, ("net", 404.0, 9.5, 700.0)]), whole);
    // The data line. What stands before the label is of the larger face:
    // at a label of 7 a word of 7.6 is (`SIZE_STEP`), one of 7.5 is the label's own face.
    let before = |size: f32, y: f32| -> Part { ("Ledger", 300.0, size, y) };
    assert_eq!(
        leaves(&[before(7.6, 700.0), label, value]),
        [vec![0], vec![1, 2]]
    );
    assert_eq!(leaves(&[before(7.5, 700.0), label, value]), whole);
    // And it stands on the baseline of the number (within `BASELINE_NEAR` of its size,
    // 1.5): a title 1.6 higher alone is no data before the label.
    assert_eq!(
        leaves(&[before(10.0, 701.4), label, value]),
        [vec![0], vec![1, 2]]
    );
    assert_eq!(leaves(&[before(10.0, 701.6), label, value]), whole);
}

/// The data line with the small label shares its leaf with a line
/// of codes over it, as a form's title block has it. The label and its number are still a
/// segment of their own (one plain line, no pair); the line of codes and the rest of the
/// data line stay as they are. Among other lines the text after the label must be what a
/// print run fills in (an amount, a date, a number): a word after the label keeps the leaf
/// whole, where a line alone is parted.
#[test]
fn a_small_colon_label_before_a_number_is_parted_under_another_line() {
    /// One run: its text, its left edge, its size, its baseline and its line.
    type Part = (&'static str, f32, f32, f32, u32);
    let leaves = |parts: &[Part]| {
        let mut texts: Vec<TextRun> = parts
            .iter()
            .map(|&(t, x, size, y, line)| {
                run(t, x, y, 0.6 * size * t.chars().count() as f32, size, line)
            })
            .collect();
        texts.extend(column(40.0, 500.0, 4, 5));
        let p = page([595.0, 842.0], texts, Vec::new());
        let s = segment_page(&p, &Limits::default(), &|| false);
        check_cover(&s, &p);
        let mut found: Vec<Vec<u32>> = Vec::new();
        let mut stack = vec![s.root];
        while let Some(id) = stack.pop() {
            let g = seg(&s, id);
            stack.extend(g.children.iter().copied());
            let own: Vec<u32> = texts_of(g)
                .into_iter()
                .filter(|&t| (t as usize) < parts.len())
                .collect();
            if g.children.is_empty() && !own.is_empty() {
                found.push(own);
            }
        }
        found.sort();
        found
    };
    // The line of codes 11 over the data line, over its right part.
    let code: Part = ("00A/00B  00000/00000", 340.0, 10.0, 711.0, 0);
    let title: Part = ("Ledger of the month", 40.0, 10.0, 701.5, 1);
    let date: Part = ("03.04.2026", 300.0, 10.0, 700.0, 1);
    let label: Part = ("Sheet:", 399.0, 7.0, 701.2, 1);
    let value: Part = ("4", 430.0, 10.0, 700.0, 1);
    // Without the rule the two lines are one leaf: that is the leaf the rule reads.
    assert_eq!(
        leaves(&[code, title, date, ("Sheet", 399.0, 7.0, 701.2, 1), value]),
        [vec![0, 1, 2, 3, 4]],
        "no colon"
    );
    assert_eq!(
        leaves(&[code, title, date, label, value]),
        [vec![0], vec![1, 2], vec![3, 4]]
    );
    // The runs of the leaf in another order: the line of codes handed last.
    assert_eq!(
        leaves(&[title, date, label, value, code]),
        [vec![0, 1], vec![2, 3], vec![4]]
    );
    // A word after the label: among other lines the leaf is whole, a line alone is parted.
    let word: Part = ("Draft", 430.0, 10.0, 700.0, 1);
    assert_eq!(
        leaves(&[code, title, date, label, word]),
        [vec![0, 1, 2, 3, 4]]
    );
    assert_eq!(
        leaves(&[title, date, label, word]),
        [vec![0, 1], vec![2, 3]]
    );
    // An amount after the label is a value too; a label of the same size is none.
    let day: Part = ("12,50", 430.0, 10.0, 700.0, 1);
    assert_eq!(
        leaves(&[code, title, date, label, day]),
        [vec![0], vec![1, 2], vec![3, 4]]
    );
    assert_eq!(
        leaves(&[code, title, date, ("Sheet:", 390.0, 10.0, 700.0, 1), value]),
        [vec![0, 1, 2, 3, 4]]
    );
    // A label that heads a row of values takes none of them, under
    // another line as on a line alone; nor with a word after the first amount
    // (the label opens its line and the amount does not end it).
    let total: Part = ("Total:", 312.0, 6.6, 700.0, 1);
    let first: Part = ("1.234,56", 344.0, 9.5, 700.0, 1);
    assert_eq!(
        leaves(&[code, total, first, ("567,89", 404.0, 9.5, 700.0, 1)]),
        [vec![0, 1, 2, 3]]
    );
    assert_eq!(
        leaves(&[code, total, first, ("net", 404.0, 9.5, 700.0, 1)]),
        [vec![0, 1, 2, 3]]
    );
}

/// The parting of a data line at its small colon label does not look
/// at the rest of the page, because it claims no pair. A value under the label
/// or text after the last stretch on another line of
/// the host changes nothing: the label and the text after it are one plain line
/// ([`Source::Line`]) either way, and a line of labels over their values is never parted.
#[test]
fn the_parting_at_a_small_colon_label_asks_nothing_of_the_page() {
    /// One run: its text, its left edge, its size, its baseline and its line.
    type Part = (&'static str, f32, f32, f32, u32);
    // The texts of the plain lines among `parts` that hold a label: the labelled parts of
    // the parted lines (the head of each is a plain line too).
    let lines = |parts: &[Part]| {
        let mut texts: Vec<TextRun> = parts
            .iter()
            .map(|&(t, x, size, y, line)| {
                run(t, x, y, 0.6 * size * t.chars().count() as f32, size, line)
            })
            .collect();
        texts.extend(column(40.0, 500.0, 4, 5));
        let p = page([595.0, 842.0], texts, Vec::new());
        let s = segment_page(&p, &Limits::default(), &|| false);
        check_cover(&s, &p);
        let found: Vec<Vec<&str>> = s
            .all
            .iter()
            .filter(|g| g.source == Source::Line)
            .map(|g| {
                assert!(g.children.is_empty());
                texts_of(g)
                    .into_iter()
                    .map(|t| parts[t as usize].0)
                    .collect()
            })
            .collect();
        // Each parted line gives its head and its labelled part.
        let labelled = |l: &Vec<&str>| l.iter().any(|t| t.ends_with(':'));
        assert_eq!(
            found.len(),
            2 * found.iter().filter(|l| labelled(l)).count()
        );
        found.into_iter().filter(labelled).collect::<Vec<_>>()
    };
    let none: Vec<Vec<&str>> = Vec::new();
    // Labels over their values, a number after the labels: the label line opens with a
    // label, so no larger text stands before `Amount:` on its line. Nothing is parted,
    // wherever the amount stands and in whatever order the runs are handed.
    let day: Part = ("Date:", 50.0, 7.0, 700.0, 0);
    let sum: Part = ("Amount:", 200.0, 7.0, 700.0, 0);
    let number: Part = ("4711/0815/2200", 238.4, 10.0, 700.0, 0);
    let date: Part = ("03.04.2026", 50.0, 10.0, 689.0, 1);
    assert_eq!(
        lines(&[
            day,
            sum,
            ("4711", 350.0, 10.0, 700.0, 0),
            date,
            ("12,50", 200.0, 10.0, 689.0, 1)
        ]),
        none
    );
    for y in [688.0, 689.0, 690.0, 686.0] {
        let amount: Part = ("12,50", 200.0, 10.0, y, 2);
        for parts in [
            [day, sum, number, date, amount],
            [day, sum, number, amount, date],
            [amount, date, day, sum, number],
        ] {
            assert_eq!(lines(&parts), none, "{y}");
        }
    }
    // The label on a data line: a code of the larger face before it, the number the last
    // of the line. One plain line, with an amount under the label at any drop (R5: 11,
    // 14.9, 15.1, 16 and 20 under the line), left or right of it, over the line, and with
    // text after the number on another line of the host. In either order of the runs.
    let code: Part = ("AB 12", 50.0, 10.0, 700.0, 0);
    let line = vec![vec!["Amount:", "4711/0815/2200"]];
    assert_eq!(lines(&[code, sum, number]), line);
    let amount = |x: f32, y: f32| -> Part { ("12,50", x, 10.0, y, 1) };
    for other in [
        amount(200.0, 689.0),
        amount(200.0, 685.1),
        amount(200.0, 684.9),
        amount(200.0, 684.0),
        amount(200.0, 680.0),
        amount(120.0, 689.0),
        amount(260.0, 689.0),
        amount(200.0, 711.0),
        ("77,00", 480.0, 10.0, 700.0, 3),
        ("77,00", 480.0, 10.0, 718.0, 3),
    ] {
        assert_eq!(lines(&[code, sum, number, other]), line, "{other:?}");
        assert_eq!(lines(&[other, code, sum, number]), line, "{other:?}");
    }
}
