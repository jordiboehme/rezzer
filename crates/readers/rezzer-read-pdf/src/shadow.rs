// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The scored reading of a page beside today's in-order one, so the switch to the region
//! engine lands a known difference.
//!
//! In a debug build with the environment variable `REZZER_SHADOW` set, the layout of each page
//! also segments the page the engine took with the regions the page states
//! ([`crate::engine::given_regions`], [`layoutregions::segment::segment_page_drawn`]), cuts
//! mixed text leaves and types every segment ([`layoutregions::typers::type_page`]) and
//! arbitrates
//! ([`layoutregions::arbitrate()`]), and keeps the decisions beside the in-order regions. The
//! read itself does not change: the shadow asks no budget and decides nothing the reader uses.
//! [`lines`] and [`records`] compare the two readings for the region report and the tests.

use std::collections::{BTreeMap, HashMap};

use layoutregions::segment::{SegId, Segment, Segments, Source};
use layoutregions::typers::Typed;
use layoutregions::{Certainty, Decision, Kind, PrimId, Region, Signal};

use crate::EnginePage;

/// The scored reading of one page: its segment tree, every typer's candidates by segment and
/// the decisions of the arbitration.
#[derive(Debug)]
pub struct Shadow {
    /// Whether the page is laid out by its tag tree: its in-order headings, lists and
    /// paragraphs come from the tags, which the scored reading does not take yet.
    pub tagged: bool,
    pub segs: Segments,
    pub typed: BTreeMap<SegId, Vec<Typed>>,
    pub decisions: Vec<Decision>,
}

/// Whether the shadow runs: a debug build with `REZZER_SHADOW` set, read once.
#[cfg(debug_assertions)]
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("REZZER_SHADOW").is_some())
}

/// The scored reading of `page` with the facts of its file, or `None` when the shadow is off
/// or the segmentation stopped before it searched the drawn lines. It never stops on time:
/// the read it shadows keeps its own budget. Its work is counted with `work::add` on the same
/// counter as the reader's (`layoutregions::work_steps`), so a test that bounds the work of a
/// read sees more with `REZZER_SHADOW` set.
#[cfg(debug_assertions)]
pub(crate) fn run(
    page: &layoutregions::PageIn,
    given: Vec<layoutregions::GivenRegion>,
    facts: &layoutregions::file::FileFacts,
    index: &layoutregions::typers::FileIndex,
    limits: &layoutregions::Limits,
    ends_in_amount: &dyn Fn(&str) -> bool,
    tagged: bool,
) -> Option<Shadow> {
    if !enabled() {
        return None;
    }
    let never = || false;
    // The page with the regions it states, as Task 19 gives them.
    let with_given;
    let page = if given.is_empty() {
        page
    } else {
        with_given = layoutregions::PageIn {
            given,
            ..page.clone()
        };
        &with_given
    };
    let (segs, drawn) = layoutregions::segment::segment_page_drawn(page, limits, &never);
    let drawn = drawn?;
    let (segs, typed) =
        layoutregions::typers::type_page(page, &drawn, segs, facts, index, ends_in_amount, limits);
    let decisions = layoutregions::arbitrate(&segs, &typed, &layoutregions::Margins::of(limits));
    Some(Shadow {
        tagged,
        segs,
        typed,
        decisions,
    })
}

fn certainty_name(c: Certainty) -> &'static str {
    match c {
        Certainty::Exact => "exact",
        Certainty::Rule => "rule",
        Certainty::Unsure => "unsure",
    }
}

/// The ids of `signals`, heaviest first, each with its weight: `id:weight,...`.
fn signal_list(signals: &[Signal]) -> String {
    let mut s: Vec<&Signal> = signals.iter().collect();
    s.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.id.cmp(b.id)));
    s.iter()
        .map(|s| format!("{}:{}", s.id, s.weight))
        .collect::<Vec<_>>()
        .join(",")
}

/// The source of a segment without blanks: `Grid{rows:1,cols:3}`, `Style`, `Given(Table)`.
fn source_name(s: Source) -> String {
    format!("{s:?}").replace(' ', "")
}

/// The anchor of the block an in-order region became, or `-`.
fn anchor_of(page: &EnginePage, r: &Region) -> String {
    page.anchors
        .get(r.host as usize)
        .cloned()
        .flatten()
        .unwrap_or_else(|| "-".to_string())
}

/// The in-order region of each primitive (the first that holds it).
fn region_of(page: &EnginePage) -> HashMap<PrimId, usize> {
    let mut of = HashMap::new();
    for (i, r) in page.regions.regions.iter().enumerate() {
        for &p in &r.prims {
            of.entry(p).or_insert(i);
        }
    }
    of
}

/// The in-order regions `prims` touch, with how many of them each holds, most first, then in
/// region order.
fn touched(of: &HashMap<PrimId, usize>, prims: &[PrimId]) -> Vec<(usize, usize)> {
    let mut count: BTreeMap<usize, usize> = BTreeMap::new();
    for p in prims {
        if let Some(&r) = of.get(p) {
            *count.entry(r).or_insert(0) += 1;
        }
    }
    let mut out: Vec<(usize, usize)> = count.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// What the evidence of a decision is, for the report: a decided kind's signals, or for an
/// unsure one the two best candidates of its own segment (kind, score, signals; the
/// alternatives it carries from other cuts and parents are left out) and its `spatial.*`
/// reasons.
fn evidence(shadow: &Shadow, d: &Decision) -> String {
    if d.certainty != Certainty::Unsure {
        return format!("{} {}", d.score.0, signal_list(&d.signals));
    }
    let mut own: Vec<&Typed> = shadow
        .typed
        .get(&d.seg)
        .map(|ts| ts.iter().filter(|t| t.kind != Kind::Spatial).collect())
        .unwrap_or_default();
    own.sort_by(|a, b| b.score.cmp(&a.score).then(a.kind.cmp(&b.kind)));
    let mut parts: Vec<String> = own
        .iter()
        .take(2)
        .map(|a| {
            format!(
                "{} {} {}",
                a.kind.name(),
                a.score.0,
                signal_list(&a.signals)
            )
        })
        .collect();
    if parts.is_empty() {
        parts.push("no candidate".to_string());
    }
    if !d.signals.is_empty() {
        parts.push(format!("spatial {}", signal_list(&d.signals)));
    }
    parts.join(" | ")
}

/// The comparison of one page: for each in-order region whose kind or certainty differs from
/// the scored decisions that hold its primitives, one line
/// `page\tanchor\tin-order kind\tscored kind\tcertainty\tbest signals\textent`.
///
/// - scored kind: the kinds of the decisions that hold the region's primitives, the one that
///   holds most first, joined by `+`;
/// - certainty: the scored certainty of that decision, `in-order>scored` when they differ
///   (a region the file states, exact, against the in-order rule of the same kind is no
///   difference);
/// - best signals: that decision's evidence (see [`evidence`]);
/// - extent: `same`, `split` (the region's primitives stand in several decisions), `merged`
///   (its decision holds primitives of other regions too) or `split+merged`.
///
/// With `extent` true a region whose kind and certainty agree but whose extent differs gets
/// a line too. A page without a shadow gives none.
pub fn lines(page: &EnginePage, extent: bool) -> Vec<String> {
    let Some(shadow) = &page.shadow else {
        return Vec::new();
    };
    let of = region_of(page);
    // The decisions that hold each region's primitives, with how many each holds.
    let mut held: Vec<BTreeMap<usize, usize>> = vec![BTreeMap::new(); page.regions.regions.len()];
    let mut spans: Vec<usize> = Vec::with_capacity(shadow.decisions.len());
    for (k, d) in shadow.decisions.iter().enumerate() {
        let seg = &shadow.segs.all[d.seg.0 as usize];
        let t = touched(&of, &seg.prims);
        spans.push(t.len());
        for (r, n) in t {
            *held[r].entry(k).or_insert(0) += n;
        }
    }
    let mut out = Vec::new();
    for (i, r) in page.regions.regions.iter().enumerate() {
        let mut ds: Vec<(usize, usize)> = held[i].iter().map(|(&k, &n)| (k, n)).collect();
        ds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let Some(&(main, _)) = ds.first() else {
            out.push(format!(
                "{}\t{}\t{}\tnone\t-\t-\t-",
                page.regions.page,
                anchor_of(page, r),
                r.kind.name()
            ));
            continue;
        };
        let d = &shadow.decisions[main];
        let mut kinds: Vec<Kind> = Vec::new();
        for (k, _) in &ds {
            let kind = shadow.decisions[*k].kind;
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        let split = ds.len() > 1;
        let merged = spans[main] > 1;
        let shape = match (split, merged) {
            (false, false) => "same",
            (true, false) => "split",
            (false, true) => "merged",
            (true, true) => "split+merged",
        };
        let same_kind = kinds == [r.kind];
        // A region the file states is exact in the scored reading; the in-order layout reads
        // the same tag block with a rule. That is no difference of the reading.
        let same_certainty = d.certainty == r.certainty
            || (same_kind && r.certainty == Certainty::Rule && d.certainty == Certainty::Exact);
        if same_kind && same_certainty && (!extent || shape == "same") {
            continue;
        }
        let certainty = if same_certainty {
            certainty_name(d.certainty).to_string()
        } else {
            format!(
                "{}>{}",
                certainty_name(r.certainty),
                certainty_name(d.certainty)
            )
        };
        out.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            page.regions.page,
            anchor_of(page, r),
            r.kind.name(),
            kinds.iter().map(|k| k.name()).collect::<Vec<_>>().join("+"),
            certainty,
            evidence(shadow, d),
            shape
        ));
    }
    out
}

/// What a segment holds: its text runs, those with ink, its pictures and codes, its rules
/// and boxes, the lines of its runs with ink, and how many of those lines its parent holds
/// runs with ink of outside it (a segment that cuts its lines).
fn holds(page: &EnginePage, shadow: &Shadow, seg: &Segment) -> String {
    let inked_line = |p: &PrimId| match *p {
        PrimId::Text(i) => page
            .input
            .texts
            .get(i as usize)
            .filter(|t| layoutregions::line::ink(&t.text) > 0)
            .map(|t| t.line),
        _ => None,
    };
    let lines: std::collections::BTreeSet<u32> = seg.prims.iter().filter_map(inked_line).collect();
    let cut = seg.parent.map_or(0, |p| {
        let own: std::collections::HashSet<&PrimId> = seg.prims.iter().collect();
        let outside: std::collections::BTreeSet<u32> = shadow.segs.all[p.0 as usize]
            .prims
            .iter()
            .filter(|q| !own.contains(q))
            .filter_map(inked_line)
            .collect();
        lines.intersection(&outside).count()
    });
    let (mut texts, mut inked, mut pictures, mut codes, mut drawn) = (0, 0, 0, 0, 0);
    for p in &seg.prims {
        match *p {
            PrimId::Text(i) => {
                texts += 1;
                if page
                    .input
                    .texts
                    .get(i as usize)
                    .is_some_and(|t| layoutregions::line::ink(&t.text) > 0)
                {
                    inked += 1;
                }
            }
            PrimId::Picture(_) => pictures += 1,
            PrimId::Code(_) => codes += 1,
            _ => drawn += 1,
        }
    }
    format!(
        "t{texts} i{inked} p{pictures} c{codes} d{drawn} l{} x{cut}",
        lines.len()
    )
}

/// The text of the runs of `prims`, in id order, the first 80 characters, blanks for tabs and
/// line ends.
fn text_of(page: &EnginePage, prims: &[PrimId]) -> String {
    let mut text = String::new();
    for p in prims {
        if let PrimId::Text(i) = *p
            && let Some(t) = page.input.texts.get(i as usize)
        {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t.text.trim());
            if text.chars().count() > 80 {
                break;
            }
        }
    }
    text.chars()
        .take(80)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The in-order regions a segment's primitives stand in: `kind/by:count;...`, `by` the id of
/// the region's first signal.
fn in_order_of(page: &EnginePage, of: &HashMap<PrimId, usize>, seg: &Segment) -> String {
    let t = touched(of, &seg.prims);
    if t.is_empty() {
        return "-".to_string();
    }
    t.iter()
        .map(|&(r, n)| {
            let r = &page.regions.regions[r];
            let by = r.signals.first().map_or("-", |s| s.id);
            format!("{}/{}/{}:{n}", r.kind.name(), by, anchor_of(page, r))
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The raw comparison of one page, for counts the lines do not show; tab-separated, the
/// first field the record's type:
///
/// - `p page tagged|untagged|none widgets`: the page, first: laid out by its tags or not, or
///   `none` when it has no shadow (a stop before the drawn search), and how many form field
///   boxes (widgets) it has, which the scored reading does not take as given regions yet;
/// - `s page seg source parent depth children cuts holds candidates in-order`: every segment
///   of the tree, the segments of other cuts included; `candidates` are `kind score signals`
///   joined by ` | `;
/// - `d page seg source depth children kind certainty score signals alternatives holds
///   in-order text`: every decision, `alternatives` as `kind score signals` joined by ` | `;
/// - `r page anchor kind certainty by decisions text`: every in-order region, `decisions` as
///   the `kind/certainty/seg:count` of the decisions that hold its primitives.
///
/// `text` is the start of the text the decision or region holds (see [`text_of`]).
pub fn records(page: &EnginePage) -> Vec<String> {
    let n = page.regions.page;
    let Some(shadow) = &page.shadow else {
        return vec![format!("p\t{n}\tnone\t-")];
    };
    let of = region_of(page);
    let depth = |s: &Segment| {
        let (mut d, mut up) = (0, s.parent);
        while let Some(p) = up {
            d += 1;
            up = shadow.segs.all[p.0 as usize].parent;
        }
        d
    };
    let widgets = page.input.boxes.iter().filter(|b| b.widget).count();
    let mut out = vec![format!(
        "p\t{n}\t{}\t{widgets}",
        if shadow.tagged { "tagged" } else { "untagged" }
    )];
    for seg in &shadow.segs.all {
        let cands = shadow
            .typed
            .get(&seg.id)
            .map(|ts| {
                ts.iter()
                    .map(|t| {
                        format!(
                            "{} {} {}",
                            t.kind.name(),
                            t.score.0,
                            signal_list(&t.signals)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default();
        out.push(format!(
            "s\t{n}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            seg.id.0,
            source_name(seg.source),
            seg.parent.map_or("-".to_string(), |p| p.0.to_string()),
            depth(seg),
            seg.children.len(),
            seg.alternatives.len(),
            holds(page, shadow, seg),
            if cands.is_empty() {
                "-".to_string()
            } else {
                cands
            },
            in_order_of(page, &of, seg)
        ));
    }
    let mut held: Vec<BTreeMap<usize, usize>> = vec![BTreeMap::new(); page.regions.regions.len()];
    for (k, d) in shadow.decisions.iter().enumerate() {
        let seg = &shadow.segs.all[d.seg.0 as usize];
        for (r, c) in touched(&of, &seg.prims) {
            *held[r].entry(k).or_insert(0) += c;
        }
        let alts = d
            .alternatives
            .iter()
            .map(|a| {
                format!(
                    "{} {} {}",
                    a.kind.name(),
                    a.score.0,
                    signal_list(&a.signals)
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        out.push(format!(
            "d\t{n}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            seg.id.0,
            source_name(seg.source),
            depth(seg),
            seg.children.len(),
            d.kind.name(),
            certainty_name(d.certainty),
            d.score.0,
            if d.signals.is_empty() {
                "-".to_string()
            } else {
                signal_list(&d.signals)
            },
            if alts.is_empty() {
                "-".to_string()
            } else {
                alts
            },
            holds(page, shadow, seg),
            in_order_of(page, &of, seg),
            text_of(page, &seg.prims)
        ));
    }
    for (i, r) in page.regions.regions.iter().enumerate() {
        let ds = held[i]
            .iter()
            .map(|(&k, &c)| {
                let d = &shadow.decisions[k];
                format!(
                    "{}/{}/{}:{c}",
                    d.kind.name(),
                    certainty_name(d.certainty),
                    d.seg.0
                )
            })
            .collect::<Vec<_>>()
            .join(";");
        out.push(format!(
            "r\t{n}\t{}\t{}\t{}\t{}\t{}\t{}",
            anchor_of(page, r),
            r.kind.name(),
            certainty_name(r.certainty),
            r.signals.first().map_or("-", |s| s.id),
            if ds.is_empty() { "-".to_string() } else { ds },
            text_of(page, &r.prims)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use layoutregions::segment::Segment;
    use layoutregions::{Bounds, PageRegions, RegionId, Score};

    fn bounds() -> Bounds {
        Bounds {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 1.0,
        }
    }

    fn segment(id: u32, prims: Vec<PrimId>, parent: Option<u32>, children: Vec<u32>) -> Segment {
        Segment {
            id: SegId(id),
            bounds: bounds(),
            prims,
            source: Source::Rest,
            parent: parent.map(SegId),
            children: children.into_iter().map(SegId).collect(),
            alternatives: Vec::new(),
        }
    }

    fn decision(seg: u32, kind: Kind, certainty: Certainty) -> Decision {
        Decision {
            seg: SegId(seg),
            kind,
            certainty,
            score: Score(500),
            signals: Vec::new(),
            alternatives: Vec::new(),
        }
    }

    fn region(id: u32, kind: Kind, prims: Vec<PrimId>) -> Region {
        Region {
            id: RegionId(id),
            page: 1,
            kind,
            bounds: bounds(),
            prims,
            certainty: Certainty::Rule,
            score: Score(500),
            signals: Vec::new(),
            alternatives: Vec::new(),
            parent: None,
            unsure: Vec::new(),
            host: id,
        }
    }

    /// Fix round 1 (M5): a region whose runs two decisions of other kinds hold gets one line
    /// with both kinds, the one holding most first, and the extent `split`; a region the file
    /// states (exact) against the same kind by rule gives no line.
    #[test]
    fn lines_name_kinds_certainty_and_extent() {
        let t = PrimId::Text;
        let segs = Segments {
            all: vec![
                segment(0, vec![t(0), t(1), t(2), t(3)], None, vec![1, 2, 3]),
                segment(1, vec![t(0), t(1)], Some(0), vec![]),
                segment(2, vec![t(2)], Some(0), vec![]),
                segment(3, vec![t(3)], Some(0), vec![]),
            ],
            root: SegId(0),
            issues: Vec::new(),
        };
        let page = EnginePage {
            input: layoutregions::PageIn {
                page: 1,
                size: [600.0, 800.0],
                turn: 0,
                texts: Vec::new(),
                rules: Vec::new(),
                boxes: Vec::new(),
                pictures: Vec::new(),
                codes: Vec::new(),
                given: Vec::new(),
            },
            regions: PageRegions {
                page: 1,
                regions: vec![
                    region(0, Kind::Paragraph, vec![t(0), t(1), t(2)]),
                    region(1, Kind::Heading, vec![t(3)]),
                ],
                issues: Vec::new(),
            },
            anchors: vec![Some("p1:b1".into()), Some("p1:b2".into())],
            shadow: Some(Shadow {
                tagged: false,
                segs,
                typed: BTreeMap::new(),
                decisions: vec![
                    decision(1, Kind::Table, Certainty::Rule),
                    decision(2, Kind::Spatial, Certainty::Unsure),
                    decision(3, Kind::Heading, Certainty::Exact),
                ],
            }),
        };
        let got = lines(&page, false);
        assert_eq!(got.len(), 1, "{got:?}");
        let cols: Vec<&str> = got[0].split('\t').collect();
        assert_eq!(
            cols[..5],
            ["1", "p1:b1", "paragraph", "table+spatial", "rule"]
        );
        assert_eq!(cols[6], "split");
    }
}
