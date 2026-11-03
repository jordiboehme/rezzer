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
