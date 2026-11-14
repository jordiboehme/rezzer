// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The whole engine on huge pages: the facts of the file, the segment tree, the typers, the
//! arbitration and the order, in work that is counted and pinned. Every run stands in exactly
//! one decision.

use std::collections::BTreeMap;

use layoutregions::file::{FilePage, learn};
use layoutregions::segment::drawn::grids;
use layoutregions::segment::segment_page_drawn;
use layoutregions::typers::{FileIndex, type_page};
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

/// The page of `alternatives_and_depth_are_bounded` (tests/segment.rs): concentric boxes, 10
/// points apart, on a page large enough that the outermost is no frame; one run in the top
/// strip of each ring and the rest of `runs` in the innermost box, 224 to a line.
fn nested_page(boxes: usize, runs: usize) -> PageIn {
    let side = 10_000.0f32;
    let outer = 7_700.0f32;
    let lo = (side - outer) / 2.0;
    let mut rules = Vec::with_capacity(boxes);
    let mut texts = Vec::with_capacity(runs);
    for k in 0..boxes {
        let a = lo + 10.0 * k as f32;
        rules.push(rect(a, a, side - a, side - a));
        texts.push(run("Muster", a + 20.0, side - a - 8.0, 4.0, 4.0, k as u32));
    }
    let inner = lo + 10.0 * (boxes - 1) as f32;
    let cols = 224usize;
    for i in 0..runs - boxes {
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

/// What the engine made of a page: the kind and the primitives of each decision in the
/// engine's order, the issues of its segment tree, and the work steps it took (`None` in a
/// release build).
struct Read {
    decisions: Vec<(Kind, Vec<PrimId>)>,
    issues: Vec<Issue>,
    steps: Option<u64>,
}

/// The whole engine on `p`, as a reader runs it: the file's facts from the page, the segments
/// on the page's grids, every typer, one arbitration and the order of the regions.
fn engine(p: &PageIn, limits: &Limits) -> Read {
    let none = |_: &str| false;
    let before = work_steps();
    let file = [FilePage::of(p)];
    let facts = learn(&file, false, &none);
    let index = FileIndex::new(&facts);
    let (segs, drawn) = segment_page_drawn(p, &grids(&p.rules), limits, &|| false);
    let (segs, typed) = match drawn {
        Some(d) => type_page(p, &d, segs, &facts, &index, &none, limits),
        None => (segs, BTreeMap::new()),
    };
    let decided = arbitrate(&segs, &typed, &Margins::of(limits));
    let regions: Vec<Region> = decided
        .iter()
        .enumerate()
        .map(|(k, d)| {
            let prims = segs.all[d.seg.0 as usize].prims.clone();
            Region {
                id: RegionId(k as u32),
                kind: d.kind,
                bounds: layoutregions::segment::bounds_of(p, &prims).unwrap_or(Bounds {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 0.0,
                    y1: 0.0,
                }),
                prims,
                page: p.page,
                certainty: Certainty::Rule,
                score: Score(0),
                signals: Vec::new(),
                alternatives: Vec::new(),
                parent: None,
                unsure: Vec::new(),
                host: 0,
            }
        })
        .collect();
    let placed = order(p, &regions);
    assert_eq!(placed.len(), regions.len(), "every region has its place");
    let steps = before.zip(work_steps()).map(|(b, a)| a - b);
    Read {
        decisions: regions.into_iter().map(|r| (r.kind, r.prims)).collect(),
        issues: segs.issues,
        steps,
    }
}

/// Every text run of `p` stands in exactly one decision of `read`.
fn assert_every_run_once(p: &PageIn, read: &Read) {
    let mut seen = vec![0u32; p.texts.len()];
    for (_, prims) in &read.decisions {
        for q in prims {
            if let PrimId::Text(t) = *q {
                seen[t as usize] += 1;
            }
        }
    }
    let off: Vec<usize> = (0..seen.len()).filter(|&t| seen[t] != 1).take(8).collect();
    assert!(off.is_empty(), "runs not in exactly one decision: {off:?}");
}

/// Work steps of the whole engine on the page of 50,000 runs in 200 nested boxes: measured,
/// with about ten in a hundred more allowed (see the test).
const ENGINE_BOUND: u64 = 69_000_000;

/// Task 37: the page of Task 8 (50,000 runs in 200 nested boxes) through the whole engine,
/// not the segmentation alone. The depth bound leaves the boxes past it whole with a `Limit`
/// issue, every run stands in one decision, and the work of all steps together is pinned.
#[test]
fn engine_work_is_bounded() {
    let p = nested_page(200, 50_000);
    let started = std::time::Instant::now();
    let read = engine(&p, &Limits::default());
    let took = started.elapsed();
    assert_every_run_once(&p, &read);
    assert!(
        read.issues
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.what.contains("nest deeper")),
        "{:?}",
        read.issues
    );
    if let Some(steps) = read.steps {
        eprintln!(
            "engine work steps {steps} in {took:?}, {} decisions",
            read.decisions.len()
        );
        assert!(steps <= ENGINE_BOUND, "{steps} steps");
        // Half the runs in the same boxes: the work grows about with the runs.
        let half = engine(&nested_page(200, 25_000), &Limits::default());
        let half = half.steps.expect("a debug build counts");
        eprintln!("engine work steps {half} for half the runs");
        assert!(
            steps * 10 <= half * 23,
            "{half} steps for 25,000 runs, {steps} for 50,000"
        );
    }
}
