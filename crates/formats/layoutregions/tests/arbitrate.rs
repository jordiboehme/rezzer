// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use layoutregions::*;

fn page() -> PageIn {
    PageIn {
        page: 3,
        size: [600.0, 800.0],
        turn: 0,
        texts: Vec::new(),
        rules: Vec::new(),
        boxes: Vec::new(),
        pictures: Vec::new(),
        codes: Vec::new(),
        given: Vec::new(),
    }
}

fn cand(kind: Kind, prims: Vec<PrimId>) -> Candidate {
    Candidate {
        kind,
        prims,
        bounds: Bounds {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 1.0,
        },
        score: Score(500),
        signals: Vec::new(),
        host: 7,
    }
}

#[test]
fn first_offer_wins() {
    let p = page();
    let mut arb = InOrder::new(&p, &Limits::default());
    let a = arb.offer(cand(Kind::Paragraph, vec![PrimId::Text(0)]));
    let b = arb.offer(cand(Kind::Heading, vec![PrimId::Text(0)]));
    assert_eq!(a, Offer::Accepted(RegionId(0)));
    assert_eq!(b, Offer::Refused { by: RegionId(0) });
    assert!(arb.is_taken(PrimId::Text(0)));
    assert!(!arb.is_taken(PrimId::Text(1)));
    let out = arb.finish();
    assert_eq!(out.page, 3);
    assert_eq!(out.regions.len(), 1);
    assert_eq!(out.regions[0].host, 7);
    assert_eq!(out.regions[0].certainty, Certainty::Rule);
    assert!(out.regions[0].parent.is_none());
}

#[test]
fn disjoint_offers_both_win() {
    let p = page();
    let mut arb = InOrder::new(&p, &Limits::default());
    assert_eq!(
        arb.offer(cand(Kind::Paragraph, vec![PrimId::Text(0)])),
        Offer::Accepted(RegionId(0))
    );
    assert_eq!(
        arb.offer(cand(
            Kind::Paragraph,
            vec![PrimId::Text(1), PrimId::Rule(0)]
        )),
        Offer::Accepted(RegionId(1))
    );
    assert_eq!(arb.finish().regions.len(), 2);
}

#[test]
fn region_limit_is_an_issue() {
    let p = page();
    let limits = Limits {
        max_regions: 1,
        ..Limits::default()
    };
    let mut arb = InOrder::new(&p, &limits);
    assert_eq!(
        arb.offer(cand(Kind::Paragraph, vec![PrimId::Text(0)])),
        Offer::Accepted(RegionId(0))
    );
    assert_eq!(
        arb.offer(cand(Kind::Paragraph, vec![PrimId::Text(1)])),
        Offer::Limit
    );
    assert_eq!(
        arb.offer(cand(Kind::Paragraph, vec![PrimId::Text(2)])),
        Offer::Limit
    );
    let out = arb.finish();
    assert_eq!(out.regions.len(), 1);
    assert_eq!(out.issues.len(), 1);
    assert_eq!(out.issues[0].kind, IssueKind::Limit);
}

struct Named(&'static str);
impl Finder<Vec<&'static str>> for Named {
    fn name(&self) -> &'static str {
        self.0
    }
    fn find(&mut self, _: &PageIn, cx: &mut Vec<&'static str>, _: &mut InOrder) {
        cx.push(self.0);
    }
}

#[test]
fn finders_run_in_slice_order() {
    let p = page();
    let mut arb = InOrder::new(&p, &Limits::default());
    let (mut a, mut b, mut c) = (Named("a"), Named("b"), Named("c"));
    let mut cx = Vec::new();
    find_in_order(
        &p,
        &mut [&mut c, &mut a, &mut b],
        &mut cx,
        &mut arb,
        &|| false,
    );
    assert_eq!(cx, ["c", "a", "b"]);
    assert!(arb.finish().issues.is_empty());
}

#[test]
fn stop_between_finders_is_an_issue() {
    let p = page();
    let mut arb = InOrder::new(&p, &Limits::default());
    let (mut a, mut b) = (Named("a"), Named("b"));
    let mut cx = Vec::new();
    let asked = std::cell::Cell::new(0);
    let stop = || {
        asked.set(asked.get() + 1);
        asked.get() > 1
    };
    find_in_order(&p, &mut [&mut a, &mut b], &mut cx, &mut arb, &stop);
    assert_eq!(cx, ["a"]);
    assert_eq!(asked.get(), 2);
    let out = arb.finish();
    assert_eq!(out.issues.len(), 1);
    assert_eq!(out.issues[0].kind, IssueKind::Stopped);
}

#[test]
fn stop_before_the_first_finder_runs_none() {
    let p = page();
    let mut arb = InOrder::new(&p, &Limits::default());
    let (mut a, mut b) = (Named("a"), Named("b"));
    let mut cx = Vec::new();
    find_in_order(&p, &mut [&mut a, &mut b], &mut cx, &mut arb, &|| true);
    assert!(cx.is_empty());
    let out = arb.finish();
    assert_eq!(out.issues.len(), 1);
    assert_eq!(out.issues[0].kind, IssueKind::Stopped);
}

#[test]
fn counts_by_kind_in_kind_order() {
    let p = page();
    let mut arb = InOrder::new(&p, &Limits::default());
    arb.offer(cand(Kind::Spatial, vec![PrimId::Text(0)]));
    arb.offer(cand(Kind::Table, vec![PrimId::Text(1)]));
    arb.offer(cand(Kind::Table, vec![PrimId::Text(2)]));
    arb.offer(cand(Kind::HeaderBand, vec![PrimId::Text(3)]));
    let out = arb.finish();
    let counts = count_by_kind(&out.regions);
    let v: Vec<(Kind, usize)> = counts.into_iter().collect();
    assert_eq!(
        v,
        [(Kind::HeaderBand, 1), (Kind::Table, 2), (Kind::Spatial, 1)]
    );
}

#[test]
fn kind_names_are_snake_case() {
    assert_eq!(Kind::LabelValue.name(), "label_value");
    for k in Kind::ALL {
        assert!(
            k.name()
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{}",
            k.name()
        );
    }
}

#[test]
fn all_kinds_are_in_kind_order() {
    assert!(Kind::ALL.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(Kind::ALL.first(), Some(&Kind::HeaderBand));
    assert_eq!(Kind::ALL.last(), Some(&Kind::Spatial));
}

#[test]
fn an_offer_keeps_its_certainty() {
    let p = page();
    let mut arb = InOrder::new(&p, &Limits::default());
    arb.offer_as(cand(Kind::Table, vec![PrimId::Text(0)]), Certainty::Exact);
    arb.offer(cand(Kind::Paragraph, vec![PrimId::Text(1)]));
    let out = arb.finish();
    assert_eq!(out.regions[0].certainty, Certainty::Exact);
    assert_eq!(out.regions[1].certainty, Certainty::Rule);
}

/// The kind after `k` in `Kind` order: an exhaustive match, so a new kind must be placed here,
/// and then `Kind::ALL` must hold it too.
fn next(k: Kind) -> Option<Kind> {
    match k {
        Kind::HeaderBand => Some(Kind::FooterBand),
        Kind::FooterBand => Some(Kind::TitleBlock),
        Kind::TitleBlock => Some(Kind::Heading),
        Kind::Heading => Some(Kind::Paragraph),
        Kind::Paragraph => Some(Kind::List),
        Kind::List => Some(Kind::Table),
        Kind::Table => Some(Kind::LabelValue),
        Kind::LabelValue => Some(Kind::Totals),
        Kind::Totals => Some(Kind::Figure),
        Kind::Figure => Some(Kind::Code),
        Kind::Code => Some(Kind::FootnoteLegend),
        Kind::FootnoteLegend => Some(Kind::Spatial),
        Kind::Spatial => None,
    }
}

#[test]
fn all_kinds_holds_every_kind() {
    let mut chain = vec![Kind::HeaderBand];
    while let Some(k) = next(*chain.last().unwrap()) {
        chain.push(k);
    }
    assert_eq!(chain, Kind::ALL);
}

#[test]
fn count_lines_print_every_kind_then_the_total() {
    let mut a = std::collections::BTreeMap::new();
    a.insert(Kind::Table, 2);
    a.insert(Kind::HeaderBand, 1);
    let b = std::collections::BTreeMap::new();
    let lines = count_lines(&[a, b]);
    assert_eq!(lines.len(), Kind::ALL.len() + 1);
    assert_eq!(lines[0], "header_band\t1\t0");
    assert!(lines.contains(&"table\t2\t0".to_string()));
    assert_eq!(lines.last().unwrap(), "total\t3\t0");
}

#[test]
fn a_moved_line_names_its_old_and_new_line() {
    let old = vec!["table\t2".to_string(), "total\t2".to_string()];
    let new = vec!["table\t3".to_string(), "total\t3".to_string()];
    assert_eq!(
        moved_lines(&old, &new),
        [
            "old: table\t2\nnew: table\t3",
            "old: total\t2\nnew: total\t3"
        ]
    );
    assert!(moved_lines(&old, &old).is_empty());
    assert_eq!(moved_lines(&old[..1], &old), ["old: (none)\nnew: total\t2"]);
}

// ---- Scored arbitration (Task 18) ----

use std::collections::BTreeMap;

use layoutregions::segment::{SegId, Segment, Segments, Source};
use layoutregions::typers::Typed;

fn sig(id: &'static str, weight: u16) -> Signal {
    Signal {
        id,
        detail: String::new(),
        weight,
    }
}

/// A candidate of `kind` scored `score`, with one signal of that weight.
fn typed(kind: Kind, score: u16) -> Typed {
    Typed {
        kind,
        score: Score(score),
        signals: vec![sig("test.evidence", score)],
    }
}

/// A segment tree built by hand.
struct Build {
    all: Vec<Segment>,
}

impl Build {
    fn new() -> Build {
        let mut b = Build { all: Vec::new() };
        b.add(None, Source::Page, Vec::new());
        b
    }

    /// A segment under `parent` holding `prims` (a leaf), its prims added up the tree.
    fn add(&mut self, parent: Option<u32>, source: Source, prims: Vec<PrimId>) -> u32 {
        let id = self.all.len() as u32;
        self.all.push(Segment {
            id: SegId(id),
            bounds: Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
            },
            prims: Vec::new(),
            source,
            parent: parent.map(SegId),
            children: Vec::new(),
            alternatives: Vec::new(),
        });
        if let Some(p) = parent {
            self.all[p as usize].children.push(SegId(id));
        }
        self.give(id, &prims);
        id
    }

    /// A segment of an other cut of `parent`: its parent is set, but it is no child.
    fn add_alt(&mut self, parent: u32, prims: Vec<PrimId>) -> u32 {
        let id = self.all.len() as u32;
        self.all.push(Segment {
            id: SegId(id),
            bounds: Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
            },
            prims: prims.clone(),
            source: Source::Gutter,
            parent: Some(SegId(parent)),
            children: Vec::new(),
            alternatives: Vec::new(),
        });
        id
    }

    /// `prims` to segment `id` and every segment above it.
    fn give(&mut self, mut id: u32, prims: &[PrimId]) {
        loop {
            let s = &mut self.all[id as usize];
            s.prims.extend_from_slice(prims);
            s.prims.sort_unstable();
            match s.parent {
                Some(p) => id = p.0,
                None => break,
            }
        }
    }

    fn done(self) -> Segments {
        Segments {
            all: self.all,
            root: SegId(0),
            issues: Vec::new(),
        }
    }
}

fn text(i: u32) -> PrimId {
    PrimId::Text(i)
}

fn decide_all(segs: &Segments, typed: &BTreeMap<SegId, Vec<Typed>>) -> Vec<Decision> {
    let out = arbitrate(segs, typed, &Margins::default());
    check_cover(segs, &out);
    out
}

/// Every primitive of the page in exactly one decision.
fn check_cover(segs: &Segments, out: &[Decision]) {
    let mut got: Vec<PrimId> = out
        .iter()
        .flat_map(|d| segs.all[d.seg.0 as usize].prims.iter().copied())
        .collect();
    got.sort_unstable();
    assert_eq!(got, segs.all[segs.root.0 as usize].prims, "the cover");
}

fn kinds(a: &[Alternative]) -> Vec<Kind> {
    a.iter().map(|x| x.kind).collect()
}

/// A page of one leaf under the root, with `cands` on the leaf.
fn one_leaf(cands: Vec<Typed>) -> Decision {
    let mut b = Build::new();
    let leaf = b.add(Some(0), Source::Gutter, vec![text(0)]);
    let segs = b.done();
    let typed = BTreeMap::from([(SegId(leaf), cands)]);
    let out = decide_all(&segs, &typed);
    assert_eq!(out.len(), 1);
    out.into_iter().next().unwrap()
}

#[test]
fn margins_start_at_300_and_150() {
    let m = Margins::default();
    assert_eq!((m.min, m.margin), (Score(300), Score(150)));
    assert_eq!(m.max_alternatives, Limits::default().max_alternatives);
    let tight = Limits {
        max_alternatives: 1,
        ..Limits::default()
    };
    assert_eq!(Margins::of(&tight).max_alternatives, 1);
}

#[test]
fn clear_winner_wins() {
    let d = one_leaf(vec![typed(Kind::Paragraph, 300), typed(Kind::Table, 900)]);
    assert_eq!(d.kind, Kind::Table);
    assert_eq!(d.certainty, Certainty::Rule);
    assert_eq!(d.score, Score(900));
    assert_eq!(d.signals, [sig("test.evidence", 900)]);
    assert_eq!(kinds(&d.alternatives), [Kind::Paragraph]);
    assert_eq!(d.alternatives[0].score, Score(300));
}

#[test]
fn close_scores_are_spatial() {
    let d = one_leaf(vec![typed(Kind::Paragraph, 500), typed(Kind::Table, 600)]);
    assert_eq!(d.kind, Kind::Spatial);
    assert_eq!(d.certainty, Certainty::Unsure);
    assert_eq!(kinds(&d.alternatives), [Kind::Table, Kind::Paragraph]);
}

#[test]
fn no_evidence_is_spatial() {
    let d = one_leaf(Vec::new());
    assert_eq!((d.kind, d.certainty), (Kind::Spatial, Certainty::Unsure));
    assert!(d.alternatives.is_empty());
    assert!(d.signals.is_empty());
}

#[test]
fn weak_winner_is_spatial() {
    let d = one_leaf(vec![typed(Kind::Heading, 200)]);
    assert_eq!((d.kind, d.certainty), (Kind::Spatial, Certainty::Unsure));
    assert_eq!(kinds(&d.alternatives), [Kind::Heading]);
}

/// A winner needs a signal: a score without one is no evidence.
#[test]
fn a_score_without_signals_is_no_evidence() {
    let mut t = typed(Kind::Table, 900);
    t.signals.clear();
    let d = one_leaf(vec![t]);
    assert_eq!(d.kind, Kind::Spatial);
}

/// Spatial refusals explain an unsure decision; they never win and are no alternative.
#[test]
fn spatial_signals_explain_an_unsure_decision() {
    let refusal = Typed {
        kind: Kind::Spatial,
        score: Score(0),
        signals: vec![sig("spatial.page_frame", 0)],
    };
    let d = one_leaf(vec![refusal.clone(), typed(Kind::Paragraph, 200)]);
    assert_eq!((d.kind, d.certainty), (Kind::Spatial, Certainty::Unsure));
    assert_eq!(d.signals, [sig("spatial.page_frame", 0)]);
    assert_eq!(kinds(&d.alternatives), [Kind::Paragraph]);
    // A clear winner keeps its own signals.
    let d = one_leaf(vec![refusal, typed(Kind::Paragraph, 700)]);
    assert_eq!(d.kind, Kind::Paragraph);
    assert!(d.alternatives.is_empty());
}

#[test]
fn alternatives_are_capped() {
    let cands = vec![
        typed(Kind::Heading, 500),
        typed(Kind::Paragraph, 510),
        typed(Kind::List, 490),
        typed(Kind::Table, 480),
        typed(Kind::LabelValue, 470),
        typed(Kind::Totals, 460),
    ];
    let d = one_leaf(cands);
    assert_eq!(
        kinds(&d.alternatives),
        [Kind::Paragraph, Kind::Heading, Kind::List, Kind::Table]
    );
}

#[test]
fn given_stays_given() {
    let mut b = Build::new();
    let given = b.add(Some(0), Source::Given(Kind::Table), vec![text(0), text(1)]);
    let leaf = b.add(Some(0), Source::Gutter, vec![text(2)]);
    let segs = b.done();
    let typed = BTreeMap::from([
        (SegId(given), vec![typed(Kind::Paragraph, 1000)]),
        (SegId(leaf), vec![typed(Kind::Paragraph, 700)]),
        // The root's strong reading never folds a given region.
        (SegId(0), vec![typed(Kind::Paragraph, 1000)]),
    ]);
    let out = decide_all(&segs, &typed);
    assert_eq!(out[0].seg, SegId(given));
    assert_eq!(out[0].kind, Kind::Table);
    assert_eq!(out[0].certainty, Certainty::Exact);
    assert_eq!(out[0].score, Score(1000));
    assert_eq!(
        out[0].signals.iter().map(|s| s.id).collect::<Vec<_>>(),
        ["given.file_states"]
    );
    assert!(out[0].alternatives.is_empty());
    assert_eq!(out[1].kind, Kind::Paragraph);
}

/// A tree: root > parent (List) > two leaves: a Heading 400 and an unsure one (Paragraph 400
/// against Heading 350).
fn parent_tree(parent: u16) -> (Segments, BTreeMap<SegId, Vec<Typed>>) {
    let mut b = Build::new();
    let p = b.add(Some(0), Source::Gutter, Vec::new());
    let a = b.add(Some(p), Source::Style, vec![text(0)]);
    let c = b.add(Some(p), Source::Style, vec![text(1)]);
    let segs = b.done();
    let typed = BTreeMap::from([
        (SegId(p), vec![typed(Kind::List, parent)]),
        (SegId(a), vec![typed(Kind::Heading, 400)]),
        (
            SegId(c),
            vec![typed(Kind::Paragraph, 400), typed(Kind::Heading, 350)],
        ),
    ]);
    (segs, typed)
}

#[test]
fn parent_beats_unsure_children_only_by_margin() {
    // The weakest child is unsure (0): a clear parent replaces both.
    let (segs, typed) = parent_tree(800);
    let out = decide_all(&segs, &typed);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].seg, SegId(1));
    assert_eq!(out[0].kind, Kind::List);
    // It carries its children's kinds.
    assert_eq!(kinds(&out[0].alternatives), [Kind::Heading]);
    // Sure children (400 each) stand against a parent 100 ahead of them.
    let (segs, mut typed) = parent_tree(500);
    typed.insert(SegId(3), vec![typed_of(Kind::Paragraph, 400)]);
    let out = decide_all(&segs, &typed);
    assert_eq!(
        out.iter().map(|d| (d.seg, d.kind)).collect::<Vec<_>>(),
        [(SegId(2), Kind::Heading), (SegId(3), Kind::Paragraph)]
    );
    // The parent's best candidate is an alternative on each child.
    assert!(out.iter().all(|d| kinds(&d.alternatives) == [Kind::List]));
    // At the margin the parent replaces them.
    let (segs, mut typed) = parent_tree(550);
    typed.insert(SegId(3), vec![typed_of(Kind::Paragraph, 400)]);
    let out = decide_all(&segs, &typed);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, Kind::List);
}

fn typed_of(kind: Kind, score: u16) -> Typed {
    typed(kind, score)
}

/// Guard 1: a leaf of rules alone (no text, picture or code) is decided Spatial but does not
/// count in the weakest score: it does not let its parent fold sure children.
#[test]
fn a_rules_only_leaf_lets_no_parent_fold() {
    let mut b = Build::new();
    let p = b.add(Some(0), Source::Box, Vec::new());
    let a = b.add(Some(p), Source::Rest, vec![text(0)]);
    let rules = b.add(
        Some(p),
        Source::Drawing,
        vec![PrimId::Rule(0), PrimId::Box(0)],
    );
    let segs = b.done();
    // The child is of the parent's kind, so only the weakest score can block the fold.
    let typed = BTreeMap::from([
        (SegId(p), vec![typed(Kind::Paragraph, 800)]),
        (SegId(a), vec![typed(Kind::Paragraph, 700)]),
    ]);
    let out = decide_all(&segs, &typed);
    assert_eq!(
        out.iter()
            .map(|d| (d.seg, d.kind, d.certainty))
            .collect::<Vec<_>>(),
        [
            (SegId(a), Kind::Paragraph, Certainty::Rule),
            (SegId(rules), Kind::Spatial, Certainty::Unsure),
        ]
    );
    // A leaf of text with no evidence counts 0: then the parent folds.
    let mut b = Build::new();
    let p = b.add(Some(0), Source::Box, Vec::new());
    let a = b.add(Some(p), Source::Rest, vec![text(0)]);
    b.add(Some(p), Source::Style, vec![text(1)]);
    let segs = b.done();
    let typed = BTreeMap::from([
        (SegId(p), vec![typed_of(Kind::Paragraph, 800)]),
        (SegId(a), vec![typed_of(Kind::Paragraph, 700)]),
    ]);
    let out = decide_all(&segs, &typed);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].seg, SegId(p));
}

/// Guard 2: the page root never replaces its children, however strong its reading; its
/// candidates go on them as alternatives.
#[test]
fn a_root_with_a_strong_reading_stays_split() {
    let mut b = Build::new();
    let a = b.add(Some(0), Source::Gutter, vec![text(0)]);
    let c = b.add(Some(0), Source::Gutter, vec![text(1)]);
    let segs = b.done();
    let typed = BTreeMap::from([
        (
            SegId(0),
            vec![typed(Kind::Paragraph, 1000), typed(Kind::Table, 600)],
        ),
        (SegId(a), vec![typed(Kind::Table, 700)]),
        (SegId(c), Vec::new()),
    ]);
    let out = decide_all(&segs, &typed);
    assert_eq!(
        out.iter().map(|d| (d.seg, d.kind)).collect::<Vec<_>>(),
        [(SegId(a), Kind::Table), (SegId(c), Kind::Spatial)]
    );
    assert_eq!(kinds(&out[0].alternatives), [Kind::Paragraph]);
    assert_eq!(kinds(&out[1].alternatives), [Kind::Paragraph, Kind::Table]);
}

/// A segment with two cuts: its main cut (two leaves) and an other cut (two leaves across).
fn cut_tree(main: [u16; 2], other: [u16; 2]) -> Vec<Decision> {
    let mut b = Build::new();
    let s = b.add(Some(0), Source::Box, Vec::new());
    let m0 = b.add(Some(s), Source::Rest, vec![text(0), text(1)]);
    let m1 = b.add(Some(s), Source::Rest, vec![text(2), text(3)]);
    let o0 = b.add_alt(s, vec![text(0), text(2)]);
    let o1 = b.add_alt(s, vec![text(1), text(3)]);
    b.all[s as usize]
        .alternatives
        .push(vec![SegId(o0), SegId(o1)]);
    let segs = b.done();
    let typed = BTreeMap::from([
        (SegId(m0), vec![typed(Kind::Paragraph, main[0])]),
        (SegId(m1), vec![typed(Kind::Paragraph, main[1])]),
        (SegId(o0), vec![typed(Kind::Table, other[0])]),
        (SegId(o1), vec![typed(Kind::Table, other[1])]),
    ]);
    decide_all(&segs, &typed)
}

/// The probe of the review: a parent paragraph 550 over a caption line (paragraph 300), lines
/// of labels and values (900) and prose (550). The weakest check passes (550 >= 300 + 150),
/// but the label/value child of another kind blocks the fold; so does a label/value
/// grandchild under a child that kept its own children, and an unsure child whose best
/// alternative of another kind is ahead.
#[test]
fn a_sure_child_of_another_kind_blocks_the_fold() {
    let mut b = Build::new();
    let p = b.add(Some(0), Source::Rest, Vec::new());
    let cap = b.add(Some(p), Source::Band, vec![text(0)]);
    let kv = b.add(Some(p), Source::Band, vec![text(1), text(2)]);
    let prose = b.add(Some(p), Source::Band, vec![text(3)]);
    let segs = b.done();
    let mut typed = BTreeMap::from([
        (SegId(p), vec![typed_of(Kind::Paragraph, 550)]),
        (SegId(cap), vec![typed_of(Kind::Paragraph, 300)]),
        (
            SegId(kv),
            vec![
                typed_of(Kind::LabelValue, 900),
                typed_of(Kind::Paragraph, 300),
            ],
        ),
        (SegId(prose), vec![typed_of(Kind::Paragraph, 550)]),
    ]);
    let out = decide_all(&segs, &typed);
    assert_eq!(
        out.iter().map(|d| (d.seg, d.kind)).collect::<Vec<_>>(),
        [
            (SegId(cap), Kind::Paragraph),
            (SegId(kv), Kind::LabelValue),
            (SegId(prose), Kind::Paragraph)
        ]
    );
    // An unsure child of a label/value and a table tied at 850 blocks a parent of 900.
    typed.insert(
        SegId(kv),
        vec![typed_of(Kind::LabelValue, 850), typed_of(Kind::Table, 850)],
    );
    typed.insert(SegId(p), vec![typed_of(Kind::Paragraph, 900)]);
    let out = decide_all(&segs, &typed);
    assert_eq!(out.len(), 3);
    assert_eq!(out[1].kind, Kind::Spatial);
    // A grandchild: the child kept its own children (it is unsure of itself), and its
    // label/value child blocks the parent.
    let mut b = Build::new();
    let p = b.add(Some(0), Source::Rest, Vec::new());
    let c = b.add(Some(p), Source::Band, Vec::new());
    let g1 = b.add(Some(c), Source::Style, vec![text(0)]);
    let g2 = b.add(Some(c), Source::Style, vec![text(1)]);
    let segs = b.done();
    let typed = BTreeMap::from([
        (SegId(p), vec![typed_of(Kind::Paragraph, 700)]),
        (SegId(g1), vec![typed_of(Kind::LabelValue, 900)]),
        (SegId(g2), vec![typed_of(Kind::Paragraph, 300)]),
    ]);
    let out = decide_all(&segs, &typed);
    assert_eq!(
        out.iter().map(|d| (d.seg, d.kind)).collect::<Vec<_>>(),
        [(SegId(g1), Kind::LabelValue), (SegId(g2), Kind::Paragraph)]
    );
}

#[test]
fn first_cut_stands_on_a_tie() {
    let out = cut_tree([700, 600], [800, 600]);
    assert_eq!(
        out.iter().map(|d| (d.seg, d.kind)).collect::<Vec<_>>(),
        [(SegId(2), Kind::Paragraph), (SegId(3), Kind::Paragraph)]
    );
    // The other cut's kinds are alternatives on the regions they share prims with.
    assert!(out.iter().all(|d| kinds(&d.alternatives) == [Kind::Table]));
    // Ahead by the margin, the other cut wins and the first cut's kinds go on it.
    let out = cut_tree([700, 600], [900, 750]);
    assert_eq!(
        out.iter().map(|d| (d.seg, d.kind)).collect::<Vec<_>>(),
        [(SegId(4), Kind::Table), (SegId(5), Kind::Table)]
    );
    assert!(
        out.iter()
            .all(|d| kinds(&d.alternatives) == [Kind::Paragraph])
    );
}

#[test]
fn same_scores_same_decision_in_any_input_order() {
    let (segs, typed) = parent_tree(500);
    let base = decide_all(&segs, &typed);
    let mut turned = typed.clone();
    for v in turned.values_mut() {
        v.reverse();
    }
    assert_eq!(decide_all(&segs, &turned), base);
    // Three candidates in every rotation give one decision.
    let three = vec![
        typed_of(Kind::Table, 500),
        typed_of(Kind::Heading, 450),
        typed_of(Kind::List, 500),
    ];
    let first = one_leaf(three.clone());
    for k in 1..3 {
        let mut c = three.clone();
        c.rotate_left(k);
        assert_eq!(one_leaf(c), first, "rotated by {k}");
    }
    assert_eq!(
        kinds(&first.alternatives),
        [Kind::List, Kind::Table, Kind::Heading]
    );
    // Equal scores: the kind order settles the sort, so the same kind leads either way.
    let a = one_leaf(vec![
        typed_of(Kind::Table, 500),
        typed_of(Kind::Heading, 500),
    ]);
    let b = one_leaf(vec![
        typed_of(Kind::Heading, 500),
        typed_of(Kind::Table, 500),
    ]);
    assert_eq!(a, b);
    assert_eq!(kinds(&a.alternatives), [Kind::Heading, Kind::Table]);
}

fn line_run(text: &str, x: f32, y: f32, size: f32, line: u32) -> TextRun {
    TextRun {
        text: text.to_string(),
        x,
        y,
        w: text.chars().count() as f32 * size * 0.5,
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

/// The decisions of the typers and the arbitration on `page`, and the segments.
fn decide_page(page: &PageIn) -> (Segments, Vec<Decision>) {
    use layoutregions::file::{FilePage, learn};
    use layoutregions::segment::drawn::drawn;
    use layoutregions::segment::segment_page;
    use layoutregions::typers::{FileIndex, PageView, type_segments};
    let none = |_: &str| false;
    let file = [FilePage::of(page)];
    let facts = learn(&file, false, &none);
    let index = FileIndex::new(&facts);
    let structure = drawn(page, &|_| true);
    let segs = segment_page(page, &Limits::default(), &|| false);
    let view = PageView::new(page, &structure, &segs, &facts, &index, &none);
    let typed = type_segments(&view);
    let out = arbitrate(&segs, &typed, &Margins::default());
    check_cover(&segs, &out);
    (segs, out)
}

#[test]
fn figure_with_caption() {
    let mut texts: Vec<TextRun> = (0..4)
        .map(|k| {
            line_run(
                "Die Muster GmbH in Musterstadt zeigt hier die Werte des Jahres",
                72.0,
                760.0 - 12.0 * k as f32,
                9.0,
                k,
            )
        })
        .collect();
    texts.push(line_run(
        "Abb. 1: Umsatz der Muster GmbH",
        72.0,
        385.0,
        9.0,
        4,
    ));
    let mut page = PageIn {
        page: 2,
        size: [595.0, 842.0],
        turn: 0,
        texts,
        rules: Vec::new(),
        boxes: Vec::new(),
        pictures: Vec::new(),
        codes: Vec::new(),
        given: Vec::new(),
    };
    page.pictures.push(PictureIn {
        bounds: Bounds {
            x0: 72.0,
            y0: 400.0,
            x1: 372.0,
            y1: 600.0,
        },
    });
    let (segs, out) = decide_page(&page);
    let fig = out
        .iter()
        .find(|d| segs.all[d.seg.0 as usize].prims == [PrimId::Picture(0)])
        .expect("the picture is a region");
    assert_eq!((fig.kind, fig.certainty), (Kind::Figure, Certainty::Rule));
    assert_eq!(
        fig.signals.iter().map(|s| s.id).collect::<Vec<_>>(),
        ["figure.picture", "figure.caption"]
    );
    // Without its caption the picture is still a figure, by the picture alone.
    page.texts.pop();
    let (segs, out) = decide_page(&page);
    let fig = out
        .iter()
        .find(|d| segs.all[d.seg.0 as usize].prims == [PrimId::Picture(0)])
        .expect("the picture is a region");
    assert_eq!(fig.kind, Kind::Figure);
    assert_eq!(
        fig.signals.iter().map(|s| s.id).collect::<Vec<_>>(),
        ["figure.picture"]
    );
}

#[test]
fn a_caption_line_makes_no_figure_of_the_text_around_it() {
    // A caption under a picture, three lines of two pieces under it and prose below.
    let mut texts: Vec<TextRun> = Vec::new();
    texts.push(line_run(
        "Abb. 1: Umsatz der Muster GmbH",
        72.0,
        385.0,
        9.0,
        0,
    ));
    for (k, (a, b)) in [
        ("Umsatz Nord", "1.234,56"),
        ("Umsatz Sued", "2.345,67"),
        ("Umsatz West", "3.456,78"),
    ]
    .iter()
    .enumerate()
    {
        let y = 370.0 - 12.0 * k as f32;
        texts.push(line_run(a, 72.0, y, 9.0, 1 + k as u32));
        texts.push(line_run(b, 300.0, y, 9.0, 1 + k as u32));
    }
    for k in 0..4u32 {
        texts.push(line_run(
            "Die Muster GmbH in Musterstadt zeigt hier die Werte des Jahres und",
            72.0,
            300.0 - 12.0 * k as f32,
            9.0,
            10 + k,
        ));
    }
    let mut page = PageIn {
        page: 2,
        size: [595.0, 842.0],
        turn: 0,
        texts,
        rules: Vec::new(),
        boxes: Vec::new(),
        pictures: Vec::new(),
        codes: Vec::new(),
        given: Vec::new(),
    };
    page.pictures.push(PictureIn {
        bounds: Bounds {
            x0: 72.0,
            y0: 400.0,
            x1: 372.0,
            y1: 600.0,
        },
    });
    let (segs, out) = decide_page(&page);
    // The text beside the caption and the caption line are no figure: only the picture is.
    for d in &out {
        let prims = &segs.all[d.seg.0 as usize].prims;
        if prims == &[PrimId::Picture(0)] {
            assert_eq!(d.kind, Kind::Figure);
            assert_eq!(
                d.signals.iter().map(|s| s.id).collect::<Vec<_>>(),
                ["figure.picture", "figure.caption"]
            );
        } else {
            assert_ne!(d.kind, Kind::Figure, "{d:#?}");
            assert!(
                d.alternatives.iter().all(|a| a.kind != Kind::Figure),
                "{d:#?}"
            );
        }
    }
}

/// A parent of the kind of a sure child only spreads that child's reading over its
/// neighbours: it folds no sure sibling of another kind (an address under a table read as
/// rows), however much more it scores; a sibling with no sure reading folds into it.
#[test]
fn a_parent_that_only_spreads_a_child_folds_no_sibling() {
    let tree = |sibling: Option<u16>| {
        let mut b = Build::new();
        let p = b.add(Some(0), Source::Rest, Vec::new());
        let rows = b.add(Some(p), Source::Band, vec![text(0), text(1)]);
        let other = b.add(Some(p), Source::Band, vec![text(2)]);
        let segs = b.done();
        let mut typed = BTreeMap::from([
            (SegId(p), vec![typed(Kind::Table, 1000)]),
            (SegId(rows), vec![typed(Kind::Table, 800)]),
        ]);
        if let Some(score) = sibling {
            typed.insert(SegId(other), vec![typed_of(Kind::Paragraph, score)]);
        }
        (segs, typed, rows, other)
    };
    // 1000 over 600 and over the table child: still the cut stands.
    let (segs, typed, rows, other) = tree(Some(600));
    let out = decide_all(&segs, &typed);
    assert_eq!(
        out.iter().map(|d| (d.seg, d.kind)).collect::<Vec<_>>(),
        [(SegId(rows), Kind::Table), (SegId(other), Kind::Paragraph)]
    );
    // A sibling with no evidence (Spatial) folds into the table.
    let (segs, typed, _, _) = tree(None);
    let out = decide_all(&segs, &typed);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, Kind::Table);
}

/// A figure keeps the text of its region as text: a figure parent folds no sure decision of a
/// text kind, however strong it is.
#[test]
fn a_figure_folds_no_sure_text() {
    let mut b = Build::new();
    let p = b.add(Some(0), Source::Box, Vec::new());
    let picture = b.add(Some(p), Source::Drawing, vec![PrimId::Picture(0)]);
    let label = b.add(Some(p), Source::Rest, vec![text(0)]);
    let segs = b.done();
    let typed = BTreeMap::from([
        (SegId(p), vec![typed(Kind::Figure, 1000)]),
        (SegId(picture), vec![typed(Kind::Figure, 550)]),
        (SegId(label), vec![typed(Kind::Paragraph, 300)]),
    ]);
    let out = decide_all(&segs, &typed);
    assert_eq!(
        out.iter().map(|d| (d.seg, d.kind)).collect::<Vec<_>>(),
        [
            (SegId(picture), Kind::Figure),
            (SegId(label), Kind::Paragraph)
        ]
    );
    // Text with no sure decision (Spatial) folds into it.
    let unsure = BTreeMap::from([
        (SegId(p), vec![typed_of(Kind::Figure, 1000)]),
        (SegId(picture), vec![typed_of(Kind::Figure, 550)]),
    ]);
    let out = decide_all(&segs, &unsure);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, Kind::Figure);
}
