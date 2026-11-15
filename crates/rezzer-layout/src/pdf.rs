// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Maps the regions the region engine (`layoutregions`) found on the pages of a PDF into the
//! model's [`RegionNote`]s, and the read's budget into the engine's limits.

use layoutregions::{Bounds, Kind, PageRegions, Region};
use rezzer_limits::Budget;
use rezzer_model::{Anchor, Certainty, Evidence, RegionAlternative, RegionKind, RegionNote};

/// The engine's limits for a read: the engine's defaults, the regions and candidates of one
/// page also bounded by the elements the read may make.
pub fn engine_limits(budget: &Budget) -> layoutregions::Limits {
    let d = layoutregions::Limits::default();
    let elements = usize::try_from(budget.limits().max_elements).unwrap_or(usize::MAX);
    layoutregions::Limits {
        max_regions: d.max_regions.min(elements),
        max_candidates: d.max_candidates.min(elements),
        ..d
    }
}

/// The notes of the regions of `pages`, page by page in their order, each page's regions in
/// the order the engine accepted them. `anchors(page, host)` names the block a region became
/// (its `host`); a region it names none for stands at the page (`p<N>`), so no region is left
/// out. `turn_back(page, bounds)` maps a region's bounds from the frame the engine saw (turned,
/// for a page whose text was turned to read left to right) back to the page box the reader
/// reports.
pub fn pdf_regions(
    pages: &[PageRegions],
    anchors: &dyn Fn(u32, u32) -> Option<Anchor>,
    turn_back: &dyn Fn(u32, Bounds) -> [f32; 4],
) -> Vec<RegionNote> {
    pages
        .iter()
        .flat_map(|p| p.regions.iter())
        .map(|r| note(r, anchors, turn_back))
        .collect()
}

fn note(
    r: &Region,
    anchors: &dyn Fn(u32, u32) -> Option<Anchor>,
    turn_back: &dyn Fn(u32, Bounds) -> [f32; 4],
) -> RegionNote {
    RegionNote {
        anchor: anchors(r.page, r.host).unwrap_or_else(|| Anchor(format!("p{}", r.page))),
        page: r.page,
        kind: region_kind(r.kind),
        bounds: rounded(turn_back(r.page, r.bounds)),
        certainty: match r.certainty {
            layoutregions::Certainty::Exact => Certainty::Exact,
            layoutregions::Certainty::Rule => Certainty::Rule,
            layoutregions::Certainty::Unsure => Certainty::Unsure,
        },
        score: r.score.0,
        evidence: r
            .signals
            .iter()
            .map(|s| Evidence::Rule {
                id: s.id.to_string(),
                detail: s.detail.clone(),
            })
            .collect(),
        alternatives: r
            .alternatives
            .iter()
            .map(|a| RegionAlternative {
                kind: region_kind(a.kind),
                score: a.score.0,
            })
            .collect(),
    }
}

/// The model's name of an engine kind; both are `Kind::name` in snake case.
pub fn region_kind(kind: Kind) -> RegionKind {
    match kind {
        Kind::HeaderBand => RegionKind::HeaderBand,
        Kind::FooterBand => RegionKind::FooterBand,
        Kind::TitleBlock => RegionKind::TitleBlock,
        Kind::Heading => RegionKind::Heading,
        Kind::Paragraph => RegionKind::Paragraph,
        Kind::List => RegionKind::List,
        Kind::Table => RegionKind::Table,
        Kind::LabelValue => RegionKind::LabelValue,
        Kind::Totals => RegionKind::Totals,
        Kind::Figure => RegionKind::Figure,
        Kind::Code => RegionKind::Code,
        Kind::FootnoteLegend => RegionKind::FootnoteLegend,
        Kind::Spatial => RegionKind::Spatial,
    }
}

/// `b` with `x0 <= x1` and `y0 <= y1`, each rounded to 0.01; a value that is no number is 0,
/// so the JSON always holds four numbers.
fn rounded(b: [f32; 4]) -> [f32; 4] {
    let r = |v: f32| {
        if v.is_finite() {
            (v * 100.0).round() / 100.0
        } else {
            0.0
        }
    };
    let [x0, y0, x1, y1] = b.map(r);
    [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use layoutregions::{Alternative, PrimId, RegionId, Score, Signal};
    use rezzer_limits::Limits;

    fn region(page: u32, kind: Kind, host: u32) -> Region {
        Region {
            id: RegionId(0),
            page,
            kind,
            bounds: Bounds {
                x0: 10.004,
                y0: 20.0,
                x1: 110.126,
                y1: 40.5,
            },
            prims: vec![PrimId::Text(0)],
            certainty: layoutregions::Certainty::Rule,
            score: Score(1000),
            signals: vec![Signal {
                id: "pdf.borderless.totals_under",
                detail: String::new(),
                weight: 1000,
            }],
            alternatives: Vec::new(),
            parent: None,
            unsure: Vec::new(),
            host,
        }
    }

    #[test]
    fn engine_limits_are_the_defaults_for_the_default_budget() {
        let budget = Budget::new(Limits::default());
        assert_eq!(engine_limits(&budget), layoutregions::Limits::default());
        let small = Budget::new(Limits {
            max_elements: 50,
            ..Limits::default()
        });
        let l = engine_limits(&small);
        assert_eq!((l.max_regions, l.max_candidates), (50, 50));
        assert_eq!(
            l.max_cut_depth,
            layoutregions::Limits::default().max_cut_depth
        );
    }

    #[test]
    fn every_kind_has_its_engine_name() {
        for kind in Kind::ALL {
            assert_eq!(
                serde_json::to_value(region_kind(kind)).unwrap(),
                kind.name(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_region_maps_with_anchor_bounds_and_evidence() {
        let mut totals = region(2, Kind::Totals, 3);
        totals.alternatives.push(Alternative {
            kind: Kind::LabelValue,
            score: Score(400),
            signals: Vec::new(),
        });
        let pages = [PageRegions {
            page: 2,
            regions: vec![totals, region(2, Kind::Figure, 9)],
            issues: Vec::new(),
        }];
        let anchors = |page: u32, host: u32| (host == 3).then(|| Anchor(format!("p{page}:t1")));
        // A turn back that moves the box by 1 point.
        let turn_back = |_: u32, b: Bounds| [b.x0 + 1.0, b.y0, b.x1 + 1.0, b.y1];
        let notes = pdf_regions(&pages, &anchors, &turn_back);
        assert_eq!(notes.len(), 2, "no region is left out");
        let n = &notes[0];
        assert_eq!(n.anchor, Anchor("p2:t1".into()));
        assert_eq!(n.page, 2);
        assert_eq!(n.kind, RegionKind::Totals);
        assert_eq!(n.bounds, [11.0, 20.0, 111.13, 40.5]);
        assert_eq!(n.certainty, Certainty::Rule);
        assert_eq!(n.score, 1000);
        assert_eq!(
            n.evidence,
            vec![Evidence::Rule {
                id: "pdf.borderless.totals_under".into(),
                detail: String::new(),
            }]
        );
        assert_eq!(
            n.alternatives,
            vec![RegionAlternative {
                kind: RegionKind::LabelValue,
                score: 400,
            }]
        );
        // No block named: the region stands at its page.
        assert_eq!(notes[1].anchor, Anchor("p2".into()));
    }

    #[test]
    fn bounds_are_ordered_and_finite() {
        assert_eq!(
            rounded([5.0, f32::NAN, 1.0, f32::INFINITY]),
            [1.0, 0.0, 5.0, 0.0]
        );
    }
}
