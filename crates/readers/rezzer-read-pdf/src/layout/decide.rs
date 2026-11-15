// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The decisions of a page: its segments ([`layoutregions::segment::segment_page_drawn`]),
//! typed ([`layoutregions::typers::type_page`]) and arbitrated by one rule
//! ([`layoutregions::arbitrate()`]), with what the layout makes of each decision: which
//! extractor reads its runs ([`Class`]), why a spatial one is kept as laid out
//! ([`unsure_why`]), and the region it is ([`regions`]).

use std::collections::{BTreeMap, HashMap};

use layoutregions::extract::text::Demotion;
use layoutregions::file::FileFacts;
use layoutregions::segment::drawn::{Grids, Unsearched};
use layoutregions::segment::{SegId, Segments, Source};
use layoutregions::typers::{FileIndex, PageFacts, Typed};
use layoutregions::{
    Alternative, Bounds, Certainty, Decision, Issue, Kind, Limits, Margins, PageIn, PageRegions,
    PrimId, Region, RegionId, Score, Signal,
};
use rezzer_model::{SpatialLine, SpatialPiece};

/// No decision holds the run.
pub(super) const NONE: u32 = u32::MAX;

/// The bounds of a decision folded into another, which is no region.
const NO_BOUNDS: Bounds = Bounds {
    x0: 0.0,
    y0: 0.0,
    x1: 0.0,
    y1: 0.0,
};

/// Which extractor reads the runs of a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Class {
    /// Headings, paragraphs and lists (the text walk, or the tag tree's blocks), and the text
    /// of a figure, a code or a form field.
    Text,
    /// A running header.
    Header,
    /// A running footer.
    Footer,
    /// A footnote legend of an untagged page.
    Legend,
    /// A table, a group of labels and values, or totals.
    Table,
    /// Kept as laid out.
    Spatial,
    /// A table the tag tree states, read with its rows across pages.
    Tagged,
}

impl Class {
    /// Every class, in the order of their index.
    pub(super) const ALL: [Class; 7] = [
        Class::Text,
        Class::Header,
        Class::Footer,
        Class::Legend,
        Class::Table,
        Class::Spatial,
        Class::Tagged,
    ];

    pub(super) fn index(self) -> usize {
        Class::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

/// The segments, candidates and decisions of a page, and the decision of each of its runs.
pub(super) struct Decided {
    pub segs: Segments,
    pub typed: BTreeMap<SegId, Vec<Typed>>,
    pub decisions: Vec<Decision>,
    /// The decision of each run of the page (by its id), or [`NONE`].
    pub of_run: Vec<u32>,
    pub margins: Margins,
    /// The segmentation stopped before it searched the drawn lines: nothing was typed, every
    /// leaf is spatial.
    pub untyped: bool,
    /// For each decision, the part of today's reading it is given for (see `today`), if any,
    /// and whether that part is text where it stands (a refusal) rather than a table or title.
    pub pre: Vec<Option<(usize, bool)>>,
    /// The unsure decisions laid out together with a neighbour ([`join_rows`]): the unsure
    /// one, the neighbour, and how they stand (in words).
    pub joined: Vec<(usize, usize, &'static str)>,
    /// For each decision whose runs differ from its segment's once the page is written (see
    /// [`agree`]), and each decision of the whole page, its primitives and their bounds.
    pub own: Vec<Option<(Vec<PrimId>, Bounds)>>,
    /// The decisions folded into another (see [`agree`]): they are no region of their own.
    pub gone: Vec<bool>,
    /// For each decision, whether a bound or the stop left its segment whole before it was
    /// parted (see [`cut_short`]): it is kept as laid out with a Limit loss.
    pub limit: Vec<bool>,
}

impl Decided {
    /// What made the segment of decision `k`.
    pub(super) fn source(&self, k: usize) -> Source {
        let d = &self.decisions[k];
        self.segs
            .all
            .get(d.seg.0 as usize)
            .map_or(Source::Page, |s| s.source)
    }

    /// The primitives of decision `k`.
    pub(super) fn prims(&self, k: usize) -> &[PrimId] {
        if let Some(Some((prims, _))) = self.own.get(k) {
            return prims;
        }
        let d = &self.decisions[k];
        self.segs
            .all
            .get(d.seg.0 as usize)
            .map_or(&[][..], |s| s.prims.as_slice())
    }

    /// The bounds of decision `k`, if it has a segment.
    pub(super) fn bounds(&self, k: usize) -> Option<Bounds> {
        if let Some(Some((_, b))) = self.own.get(k) {
            return Some(*b);
        }
        self.segs
            .all
            .get(self.decisions[k].seg.0 as usize)
            .map(|s| s.bounds)
    }

    /// Which extractor reads the runs of decision `k`; `tagged` for a page laid out by its tag
    /// tree, whose footnotes the tags say.
    pub(super) fn class(&self, k: usize, tagged: bool) -> Class {
        let kind = self.decisions[k].kind;
        match self.pre.get(k).copied().flatten() {
            Some((_, true)) => return Class::Text,
            Some((_, false)) => return Class::Table,
            None => {}
        }
        match self.source(k) {
            Source::Given(Kind::Table) => Class::Tagged,
            Source::Given(Kind::Totals | Kind::LabelValue) => Class::Table,
            Source::Given(_) => Class::Text,
            _ => match kind {
                Kind::HeaderBand => Class::Header,
                Kind::FooterBand => Class::Footer,
                Kind::FootnoteLegend if !tagged => Class::Legend,
                Kind::Table | Kind::LabelValue | Kind::Totals => Class::Table,
                Kind::Spatial => Class::Spatial,
                _ => Class::Text,
            },
        }
    }

    /// The class of the run `id`.
    pub(super) fn class_of_run(&self, id: u32, tagged: bool) -> Class {
        match self.of_run.get(id as usize) {
            Some(&k) if k != NONE => self.class(k as usize, tagged),
            _ => Class::Text,
        }
    }

    /// The kind of the decision of the run `id`, if a decision holds it.
    pub(super) fn kind_of_run(&self, id: u32) -> Option<Kind> {
        match self.of_run.get(id as usize) {
            // Text a reader of today refused is laid out as today: the walk decides.
            Some(&k) if k != NONE && matches!(self.pre[k as usize], Some((_, true))) => None,
            Some(&k) if k != NONE => Some(self.decisions[k as usize].kind),
            _ => None,
        }
    }

    /// The decision of the run `id`, when the engine itself decided it as text (a paragraph,
    /// a list, a heading or a title block): the region the run is placed with (Task 31c).
    /// Text a reader of today gave or refused has none: it is laid out as today.
    pub(super) fn text_region_of_run(&self, id: u32) -> Option<u32> {
        let k = *self.of_run.get(id as usize)?;
        (k != NONE
            && self.pre[k as usize].is_none()
            && matches!(
                self.decisions[k as usize].kind,
                Kind::Paragraph | Kind::List | Kind::Heading | Kind::TitleBlock
            ))
        .then_some(k)
    }

    /// The given region of today's reading that holds the run `id`: the number of its table.
    pub(super) fn given_of_run(&self, id: u32) -> Option<u32> {
        let k = *self.of_run.get(id as usize)?;
        let (table, _) = (*self.pre.get(k as usize)?)?;
        u32::try_from(table).ok()
    }

    /// The candidates of the segment of decision `k`.
    pub(super) fn candidates(&self, k: usize) -> &[Typed] {
        self.typed
            .get(&self.decisions[k].seg)
            .map_or(&[][..], Vec::as_slice)
    }
}

/// The given regions of today's reading (see [`decide`]): the table of each of its runs, and
/// the signal of each table with whether it is text a reader refused.
pub(super) type Pre<'a> = (&'a HashMap<u32, usize>, &'a dyn Fn(usize) -> (Signal, bool));

/// Segments, types and arbitrates `page` (with its given regions) with the facts of its file.
/// A segmentation stopped before it searched the drawn lines types nothing: every leaf of it is
/// spatial. A given region of today's reading (`pre`: the table of each of its runs, and the
/// signal of each table; see `today`) is decided by its reader: certainty `Rule`, the reader's
/// signal. Text a reader refused is `Unsure` with the refusal as its signal.
///
/// The engine reads the runs in the order of their places ([`layoutregions::RunOrder`]): a
/// page whose runs stand in another order is decided in that order, and the decisions name
/// the runs by the ids of `page`. So the order of the runs handed in decides no read.
pub(super) fn decide(
    page: &PageIn,
    grids: &Result<Grids, Unsearched>,
    file: (&FileFacts, &FileIndex),
    ends_in_amount: &dyn Fn(&str) -> bool,
    pre: Pre<'_>,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> Decided {
    let Some(order) = layoutregions::RunOrder::of(page) else {
        let facts = PageFacts::of(file.0, file.1, page.page);
        return decide_in_order(page, grids, facts, ends_in_amount, pre, limits, stop);
    };
    let pre_of: HashMap<u32, usize> = pre
        .0
        .iter()
        .filter_map(|(&run, &t)| Some((*order.to.get(run as usize)?, t)))
        .collect();
    // The legend of the page names its runs by the ids of `page`: the typers look them up
    // on the page in place order. Only this page's legend is renamed and handed on; the
    // facts of the file are not copied (a file may hold a legend on every page).
    let legend = file.0.legends.get(&page.page).map(|l| order.legend(l));
    let mut d = decide_in_order(
        &order.page,
        grids,
        PageFacts {
            file: file.0,
            index: file.1,
            legend: legend.as_ref(),
        },
        ends_in_amount,
        (&pre_of, pre.1),
        limits,
        stop,
    );
    order.segments_back(&mut d.segs);
    for (prims, _) in d.own.iter_mut().flatten() {
        order.back(prims);
    }
    let mut of_run = vec![NONE; d.of_run.len()];
    for (k, &old) in d.of_run.iter().zip(&order.back) {
        if let Some(slot) = of_run.get_mut(old as usize) {
            *slot = *k;
        }
    }
    d.of_run = of_run;
    d
}

/// [`decide`] on a page whose runs stand in the order of their places, with the facts of its
/// file and the page's own legend named by the ids of `page`.
fn decide_in_order(
    page: &PageIn,
    grids: &Result<Grids, Unsearched>,
    facts: PageFacts<'_>,
    ends_in_amount: &dyn Fn(&str) -> bool,
    pre: Pre<'_>,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> Decided {
    let margins = Margins::of(limits);
    let (segs, drawn) = layoutregions::segment::segment_page_drawn(page, grids, limits, stop);
    let untyped = drawn.is_none();
    let (segs, typed) = match drawn {
        Some(drawn) => {
            layoutregions::typers::type_page_with(page, &drawn, segs, facts, ends_in_amount, limits)
        }
        None => (segs, BTreeMap::new()),
    };
    let mut decisions = layoutregions::arbitrate(&segs, &typed, &margins);
    // A given region of today's reading is decided by its reader: certainty `Rule`, with the
    // reader's signal. Its first run names its table (the first given region that lists a run
    // holds it).
    let mut pre_of = vec![None; decisions.len()];
    for (k, d) in decisions.iter_mut().enumerate() {
        let Some(seg) = segs.all.get(d.seg.0 as usize) else {
            continue;
        };
        if !matches!(seg.source, Source::Given(_)) {
            continue;
        }
        let first = seg.prims.iter().find_map(|p| match *p {
            PrimId::Text(i) => Some(i),
            _ => None,
        });
        if let Some(t) = first.and_then(|i| pre.0.get(&i).copied()) {
            let (signal, text) = (pre.1)(t);
            if text {
                // Text its reader refused: the refusal speaks against a table or pairs and
                // for no other type, so the region is unsure and has no score. Its text is
                // still written by the walk.
                d.certainty = Certainty::Unsure;
                d.score = Score(0);
            } else {
                d.certainty = Certainty::Rule;
            }
            d.signals = vec![signal];
            pre_of[k] = Some((t, text));
        }
    }
    // A decision of the whole page (its root segment, the page's frame) stands where its
    // primitives stand, not over the frame.
    let mut own: Vec<Option<(Vec<PrimId>, Bounds)>> = decisions
        .iter()
        .map(|d| {
            let seg = segs.all.get(d.seg.0 as usize)?;
            if seg.source != Source::Page {
                return None;
            }
            let bounds = layoutregions::segment::bounds_of(page, &seg.prims)?;
            Some((seg.prims.clone(), bounds))
        })
        .collect();
    let (mut limit, mut apart) = cut_short(page, &segs, &mut decisions, &mut pre_of, &mut own);
    part_pictures(
        page,
        &segs,
        &mut decisions,
        &mut pre_of,
        &mut own,
        (&mut limit, &mut apart),
    );
    let mut of_run = vec![NONE; page.texts.len()];
    for (k, d) in decisions.iter().enumerate() {
        let prims = match &own[k] {
            Some((prims, _)) => prims,
            None => match segs.all.get(d.seg.0 as usize) {
                Some(seg) => &seg.prims,
                None => continue,
            },
        };
        for p in prims {
            if let PrimId::Text(i) = *p
                && let Some(slot) = of_run.get_mut(i as usize)
            {
                *slot = u32::try_from(k).unwrap_or(NONE);
            }
        }
    }
    let joined = join_rows(page, &segs, &decisions, &pre_of, &apart, &mut of_run);
    let n = decisions.len();
    Decided {
        segs,
        typed,
        decisions,
        of_run,
        margins,
        untyped,
        pre: pre_of,
        joined,
        own,
        gone: vec![false; n],
        limit,
    }
}

/// The signal of a region a bound or the stop left whole before it was parted.
const CUT_SHORT: &str = "pdf.limit.unparted";

/// Keeps as laid out the text of the segments a bound of the segmentation or the stop left
/// whole where they would have been parted further (`segs.short`): its type was not looked for
/// in full, so no reading of it is sure. A decision whose text runs all stand in such a
/// segment becomes `Spatial` (certainty `Unsure`, its kind an alternative); one that holds some
/// of them gives those runs to a `Spatial` decision of their own (`own`). Decisions the file
/// or today's reading give (`pre`) are kept. Gives, for each decision, whether it is such a
/// spatial one, and whether it holds other runs than its segment's (such a spatial one, or one
/// that gave runs away), which no neighbour is laid out with ([`join_rows`]).
fn cut_short(
    page: &PageIn,
    segs: &Segments,
    decisions: &mut Vec<Decision>,
    pre: &mut Vec<Option<(usize, bool)>>,
    own: &mut Vec<Option<(Vec<PrimId>, Bounds)>>,
) -> (Vec<bool>, Vec<bool>) {
    let mut limit = vec![false; decisions.len()];
    if segs.short.is_empty() {
        let apart = limit.clone();
        return (limit, apart);
    }
    let mut apart = limit.clone();
    let short = segs.short_texts(page.texts.len());
    for k in 0..decisions.len() {
        let Some(seg) = segs.all.get(decisions[k].seg.0 as usize) else {
            continue;
        };
        if matches!(seg.source, Source::Given(_)) || pre[k].is_some() {
            continue;
        }
        let prims = own[k].as_ref().map_or(&seg.prims, |o| &o.0);
        let is_short = |p: &PrimId| matches!(*p, PrimId::Text(i) if short[i as usize]);
        if !prims.iter().any(is_short) {
            continue;
        }
        let d = &decisions[k];
        let mut alternatives = Vec::new();
        if d.kind != Kind::Spatial {
            alternatives.push(Alternative {
                kind: d.kind,
                score: d.score,
                signals: d.signals.clone(),
            });
        }
        alternatives.extend(
            d.alternatives
                .iter()
                .filter(|a| a.kind != d.kind && a.kind != Kind::Spatial)
                .cloned(),
        );
        let spatial = Decision {
            seg: d.seg,
            kind: Kind::Spatial,
            certainty: Certainty::Unsure,
            score: Score(0),
            signals: vec![Signal {
                id: CUT_SHORT,
                detail: "a bound of the segmentation, or the time limit, left it whole before \
                         it was parted: its region type was not looked for in full"
                    .to_string(),
                weight: 0,
            }],
            alternatives,
        };
        let rest_has_text = prims
            .iter()
            .any(|p| matches!(p, PrimId::Text(_)) && !is_short(p));
        if !rest_has_text {
            decisions[k] = spatial;
            limit[k] = true;
            apart[k] = true;
            continue;
        }
        let (cut, rest): (Vec<PrimId>, Vec<PrimId>) = prims.iter().partition(|p| is_short(p));
        let (Some(at_cut), Some(at_rest)) = (
            layoutregions::segment::bounds_of(page, &cut),
            layoutregions::segment::bounds_of(page, &rest),
        ) else {
            continue;
        };
        own[k] = Some((rest, at_rest));
        apart[k] = true;
        decisions.push(spatial);
        pre.push(None);
        own.push(Some((cut, at_cut)));
        limit.push(true);
        apart.push(true);
    }
    (limit, apart)
}

/// The primitives and bounds of a decision whose runs differ from its segment's (see
/// [`Decided::own`]).
type Own = Option<(Vec<PrimId>, Bounds)>;

/// The signal of a figure region of the pictures and codes a bound of the segmentation left
/// in one region with text.
pub(super) const PICTURES_UNPARTED: &str = "pdf.limit.pictures_unparted";

/// On a page whose segmentation a bound or the stop cut short (`segs.issues`), a decision of
/// another kind than a figure or a code that holds pictures or codes beside its text gives them
/// to a `Figure` decision of their own (certainty `Unsure`, signal [`PICTURES_UNPARTED`]), so
/// they are never hidden in a region of text: the bound left them unparted and their type was
/// not looked for. The decision keeps its text and its type. Decisions the file or today's
/// reading give (`pre`) are kept as they are. A new decision is kept apart from its neighbours
/// and is no limit leaf (see [`cut_short`]): its pictures stand where they are drawn.
fn part_pictures(
    page: &PageIn,
    segs: &Segments,
    decisions: &mut Vec<Decision>,
    pre: &mut Vec<Option<(usize, bool)>>,
    own: &mut Vec<Own>,
    (limit, apart): (&mut Vec<bool>, &mut Vec<bool>),
) {
    if segs.issues.is_empty() {
        return;
    }
    for k in 0..decisions.len() {
        if matches!(decisions[k].kind, Kind::Figure | Kind::Code) || pre[k].is_some() {
            continue;
        }
        let Some(seg) = segs.all.get(decisions[k].seg.0 as usize) else {
            continue;
        };
        if matches!(seg.source, Source::Given(_)) {
            continue;
        }
        let prims = own[k].as_ref().map_or(&seg.prims, |o| &o.0);
        let (media, rest): (Vec<PrimId>, Vec<PrimId>) = prims
            .iter()
            .partition(|p| matches!(p, PrimId::Picture(_) | PrimId::Code(_)));
        if media.is_empty() || !rest.iter().any(|p| matches!(p, PrimId::Text(_))) {
            continue;
        }
        let (Some(at_media), Some(at_rest)) = (
            layoutregions::segment::bounds_of(page, &media),
            layoutregions::segment::bounds_of(page, &rest),
        ) else {
            continue;
        };
        let count = media.len();
        own[k] = Some((rest, at_rest));
        apart[k] = true;
        decisions.push(Decision {
            seg: decisions[k].seg,
            kind: Kind::Figure,
            certainty: Certainty::Unsure,
            score: Score(0),
            signals: vec![Signal {
                id: PICTURES_UNPARTED,
                detail: format!(
                    "a bound of the segmentation, or the time limit, left {count} pictures or \
                     codes in one region with text: their region type was not looked for"
                ),
                weight: 0,
            }],
            alternatives: Vec::new(),
        });
        pre.push(None);
        own.push(Some((media, at_media)));
        limit.push(false);
        apart.push(true);
    }
}

/// How a neighbour laid out with an unsure decision stands: on its rows, side by side.
const SHARED_ROWS: &str = "whose rows it shares";

/// How a neighbour laid out with an unsure decision stands: its lines between the decision's
/// lines in one column.
const INTERLEAVED: &str = "whose lines stand between its lines";

/// Pairs of decisions and primitives a page weighs for [`join_rows`], over all pairs of
/// decisions it compares; past it the rest stay apart (each still read as decided).
const JOIN_WORK: usize = 4_000_000;

/// An unsure area (a `Spatial` decision) whose rows a neighbouring text region (a `Paragraph`
/// or another `Spatial` decision) shares, side by side across a gap
/// ([`layoutregions::extract::spatial::shares_rows`]), is laid out together with it, row by
/// row: the neighbour's runs go to the unsure decision (`of_run`), so a column of lone amounts
/// stands on the lines of the names beside it instead of a block of its own. So is another
/// unsure area whose lines stand between its lines in one column
/// ([`layoutregions::extract::spatial::interleaves`]): read one after the other, a line would
/// come after lines it stands below. Gives each pair (the unsure decision, the neighbour, how
/// they stand); once the page is written the neighbour is folded into
/// the unsure one ([`agree`]). Decisions given by the file or by today's reading (`pre`) are
/// neither. Each decision is laid out with at most one other, and one that took others is
/// laid with none.
fn join_rows(
    page: &PageIn,
    segs: &Segments,
    decisions: &[Decision],
    pre: &[Option<(usize, bool)>],
    apart: &[bool],
    of_run: &mut [u32],
) -> Vec<(usize, usize, &'static str)> {
    let seg = |k: usize| {
        segs.all
            .get(decisions[k].seg.0 as usize)
            .filter(|s| !matches!(s.source, Source::Given(_)) && pre[k].is_none() && !apart[k])
    };
    let mut joined = vec![false; decisions.len()];
    let mut took = vec![false; decisions.len()];
    let mut work = 0usize;
    let mut pairs = Vec::new();
    // The decisions that can be laid out with another, by what both tests ask of each side
    // (see `Ink`), found once: a decision is then weighed only against those that share its
    // lines (for `shares_rows`) or whose ink overlaps its ink across (for `interleaves`),
    // not against every decision of the page.
    let ink: Vec<Option<Ink>> = (0..decisions.len())
        .map(|k| {
            let kind = decisions[k].kind;
            if !matches!(kind, Kind::Paragraph | Kind::Spatial) {
                return None;
            }
            let s = seg(k)?;
            work += 1 + s.prims.len();
            Ink::of(page, &s.prims)
        })
        .collect();
    if work > JOIN_WORK {
        return pairs;
    }
    let unsure: Vec<bool> = decisions.iter().map(|d| d.kind == Kind::Spatial).collect();
    let near = Near::new(&ink, &unsure);
    let mut found: Vec<usize> = Vec::new();
    for d in 0..decisions.len() {
        if decisions[d].kind != Kind::Spatial || joined[d] {
            continue;
        }
        let Some(a) = seg(d) else { continue };
        let Some(own) = ink[d].as_ref() else { continue };
        found.clear();
        work += near.of(own, &mut found);
        if work > JOIN_WORK {
            return pairs;
        }
        // In the order of the decisions, as when every decision was looked at.
        found.sort_unstable();
        found.dedup();
        for &k in &found {
            // Every pair looked at counts, the pairs apart in height too.
            work += 1;
            if work > JOIN_WORK {
                return pairs;
            }
            if k == d || joined[k] || took[k] {
                continue;
            }
            let Some(b) = seg(k) else { continue };
            if b.bounds.y1 < a.bounds.y0 || b.bounds.y0 > a.bounds.y1 {
                continue;
            }
            work += a.prims.len() + b.prims.len();
            if work > JOIN_WORK {
                return pairs;
            }
            let how = if layoutregions::extract::spatial::shares_rows(page, &a.prims, &b.prims) {
                Some(SHARED_ROWS)
            } else if decisions[k].kind == Kind::Spatial
                && layoutregions::extract::spatial::interleaves(page, &a.prims, &b.prims)
            {
                Some(INTERLEAVED)
            } else {
                None
            };
            if let Some(how) = how {
                joined[k] = true;
                took[d] = true;
                pairs.push((d, k, how));
                let to = u32::try_from(d).unwrap_or(NONE);
                for p in &b.prims {
                    if let PrimId::Text(i) = *p
                        && let Some(slot) = of_run.get_mut(i as usize)
                    {
                        *slot = to;
                    }
                }
            }
        }
    }
    pairs
}

/// What [`join_rows`] needs of the text of a decision to tell which others it can be laid
/// out with: the lines of its runs with ink, sorted, and how far that ink reaches across.
/// `None` for a decision the two tests refuse with every other: no run with ink, a run off
/// the line's axis, or a run the page does not have.
struct Ink {
    lines: Vec<u32>,
    x0: f32,
    x1: f32,
}

impl Ink {
    fn of(page: &PageIn, prims: &[PrimId]) -> Option<Ink> {
        let mut lines = Vec::new();
        let (mut x0, mut x1) = (f32::MAX, f32::MIN);
        for p in prims {
            if let PrimId::Text(i) = *p {
                let r = page.texts.get(i as usize)?;
                if r.off_axis {
                    return None;
                }
                if !r.text.trim().is_empty() {
                    lines.push(r.line);
                    x0 = x0.min(r.x.min(r.x + r.w));
                    x1 = x1.max(r.x.max(r.x + r.w));
                }
            }
        }
        lines.sort_unstable();
        lines.dedup();
        (!lines.is_empty()).then_some(Ink { lines, x0, x1 })
    }
}

/// The decisions with ink by what [`join_rows`] can join them on: those of the same lines
/// ([`layoutregions::extract::spatial::shares_rows`] asks that the rows of the two are the
/// same rows), and the unsure ones by where their ink starts across
/// ([`layoutregions::extract::spatial::interleaves`] asks that the ink of the two overlaps).
struct Near<'a> {
    ink: &'a [Option<Ink>],
    /// The decisions of each set of lines.
    same: HashMap<&'a [u32], Vec<usize>>,
    /// The unsure decisions whose ink has finite ends, by its left end, and the widest of
    /// them: ink that overlaps a stretch starts less than that left of it.
    by_left: Vec<(f32, usize)>,
    widest: f32,
    /// The unsure decisions whose ink has an end that is no finite number: weighed with all.
    loose: Vec<usize>,
}

impl<'a> Near<'a> {
    /// `unsure` tells which decisions are unsure areas (`Spatial`).
    fn new(ink: &'a [Option<Ink>], unsure: &[bool]) -> Near<'a> {
        let mut same: HashMap<&[u32], Vec<usize>> = HashMap::new();
        let (mut by_left, mut loose) = (Vec::new(), Vec::new());
        let mut widest = 0.0f32;
        for (k, i) in ink.iter().enumerate() {
            let Some(i) = i else { continue };
            same.entry(i.lines.as_slice()).or_default().push(k);
            if !unsure[k] {
                continue;
            }
            if i.x0.is_finite() && i.x1.is_finite() {
                by_left.push((i.x0, k));
                widest = widest.max(i.x1 - i.x0);
            } else {
                loose.push(k);
            }
        }
        by_left.sort_by(|a, b| a.0.total_cmp(&b.0));
        Near {
            ink,
            same,
            by_left,
            widest,
            loose,
        }
    }

    /// Adds to `found` the decisions an unsure decision with the ink `own` may be laid
    /// out with: every one of its lines, and every unsure one whose ink overlaps its ink
    /// across (all unsure ones when its own ends are no finite numbers). A superset of those
    /// the two tests accept; the decision itself is among them. Returns the steps it took.
    fn of(&self, own: &Ink, found: &mut Vec<usize>) -> usize {
        let mut steps = 1;
        if let Some(list) = self.same.get(own.lines.as_slice()) {
            steps += list.len();
            found.extend(list);
        }
        found.extend(&self.loose);
        steps += self.loose.len();
        if !(own.x0.is_finite() && own.x1.is_finite() && self.widest.is_finite()) {
            steps += self.by_left.len();
            found.extend(self.by_left.iter().map(|e| e.1));
            return steps;
        }
        // Ink that overlaps `own` starts left of its right end, and not more than the
        // widest ink left of its left end. The search starts a little further left than
        // that (a ten-thousandth of the numbers at hand), so that no rounding of the widths
        // leaves one out: each one found is tested below.
        let left = own.x0 - self.widest;
        let left = left - 1e-4 * (left.abs() + self.widest + 1.0);
        let from = self.by_left.partition_point(|e| e.0 < left);
        for &(_, k) in self.by_left[from..].iter().take_while(|e| e.0 <= own.x1) {
            steps += 1;
            if self.ink[k]
                .as_ref()
                .is_some_and(|i| i.x1 > own.x0 && i.x0 < own.x1)
            {
                found.push(k);
            }
        }
        steps
    }
}

/// Why the walk laid out the lines of a heading decision as a paragraph, in words.
fn demotion_why(d: Demotion) -> &'static str {
    match d {
        Demotion::LinesApart => {
            "its lines are short items of one column or fields in the same two faces"
        }
        Demotion::Unwrapped => "a line of its column does not go on in the next as wrapped text",
        Demotion::Cells => "its lines each stand in pieces apart by a cell gap",
        Demotion::Address => "it has the shape of an address",
        Demotion::Sentences => "it reads as sentences",
    }
}

/// What [`agree`] takes of the page written: the runs of the footnote legend, the runs of
/// heading decisions written as paragraphs with why, the drawn box of each table of today's
/// reading, and the runs of each line a table's reader left that was read as a pair.
pub(super) type Done<'a> = (
    &'a [u32],
    &'a [(u32, Demotion)],
    &'a [Option<Bounds>],
    &'a [Vec<u32>],
);

/// Makes each decision say what the page wrote of it, once the page is written:
/// - an unsure decision laid out with a neighbour whose rows it shares ([`join_rows`]) holds
///   the neighbour's primitives too, and a signal names the neighbour's region, which is no
///   region of its own any more;
/// - runs of a heading decision that the walk wrote as a paragraph for their geometry
///   (`demoted`, by run id) are a paragraph decision with that reason (the heading its
///   alternative): the whole decision when all its runs are, else a decision of its own;
/// - the runs of the page's footnote legend (`legend`, taken by today's reader from the
///   page's whole flow) are one footnote legend decision: an engine legend decision that holds
///   only legend runs takes the others, else a decision of its own; the regions they came
///   from are named in its signal, and a region left with no primitive is gone;
/// - a table of today's reading reaches over the drawn box it was read by (`drawn`, by the
///   part of today's reading), past the box of its runs;
/// - each line a table's reader left that the line pair rule read as a pair (`pairs`, its
///   runs) is a pairs decision of its own, its decision's kind an alternative;
/// - last, every decision's kind is the kind of the blocks its runs were written in
///   (`written`, the [`crate::block_word`] of the block of each run by its id): runs written
///   as another kind than their decision's (a table refused and kept as laid out, the lines
///   under a title written as paragraphs, text a reader refused that the walk wrote as a
///   heading or list, a line of a list that opens no item) are a decision of the kind
///   written, with why (`why`, by run id, else a reason by the two kinds), the decision's
///   kind an alternative: the whole decision when none of its runs was written as its kind,
///   else a decision of their own per kind written, parted where runs written as its kind
///   stand between them in the page's flow. Figures and codes keep their kind (their
///   media block is placed later; the text in them is written by the walk), and runs without
///   ink or written in no block stay where they are.
pub(super) fn agree<'w>(
    decided: &mut Decided,
    page: &PageIn,
    (legend, demoted, drawn, pairs): Done<'_>,
    written: &[Option<&'static str>],
    why: &dyn Fn(u32) -> Option<&'w str>,
) {
    let text_of = |prims: &[PrimId]| -> Vec<u32> {
        prims
            .iter()
            .filter_map(|p| match *p {
                PrimId::Text(i) => Some(i),
                _ => None,
            })
            .collect()
    };
    let set_own = |decided: &mut Decided, k: usize, prims: Vec<PrimId>| {
        let fallback = decided.bounds(k);
        let bounds = layoutregions::segment::bounds_of(page, &prims).or(fallback);
        match bounds {
            Some(b) if !prims.is_empty() => decided.own[k] = Some((prims, b)),
            _ => {
                decided.own[k] = Some((Vec::new(), fallback.unwrap_or(NO_BOUNDS)));
                decided.gone[k] = true;
            }
        }
    };
    let push = |decided: &mut Decided, d: Decision, prims: Vec<PrimId>| -> usize {
        let k = decided.decisions.len();
        decided.decisions.push(d);
        decided.pre.push(None);
        decided.own.push(None);
        decided.gone.push(false);
        decided.limit.push(false);
        for i in text_of(&prims) {
            if let Some(slot) = decided.of_run.get_mut(i as usize) {
                *slot = u32::try_from(k).unwrap_or(NONE);
            }
        }
        set_own(decided, k, prims);
        k
    };
    // Tables over their drawn boxes.
    for k in 0..decided.decisions.len() {
        if let Some((t, false)) = decided.pre[k]
            && let Some(Some(b)) = drawn.get(t)
            && let Some(at) = decided.bounds(k)
        {
            let prims = decided.prims(k).to_vec();
            let reach = Bounds {
                x0: at.x0.min(b.x0),
                y0: at.y0.min(b.y0),
                x1: at.x1.max(b.x1),
                y1: at.y1.max(b.y1),
            };
            decided.own[k] = Some((prims, reach));
        }
    }
    // Folds.
    for (d, k, how) in std::mem::take(&mut decided.joined) {
        let mut prims = decided.prims(d).to_vec();
        prims.extend_from_slice(decided.prims(k));
        prims.sort_unstable();
        prims.dedup();
        let kind = words(decided.decisions[k].kind);
        decided.decisions[d].signals.push(Signal {
            id: "pdf.spatial.shared_rows",
            detail: format!(
                "laid out row by row with region {k} ({kind}), {how}; that region is part of \
                 this one"
            ),
            weight: 0,
        });
        set_own(decided, d, prims);
        decided.own[k] = Some((Vec::new(), decided.bounds(k).unwrap_or(NO_BOUNDS)));
        decided.gone[k] = true;
        decided.joined.push((d, k, how));
    }
    // Headings written as paragraphs.
    let mut why_of: BTreeMap<usize, (Demotion, Vec<u32>)> = BTreeMap::new();
    for &(id, why) in demoted {
        let Some(&k) = decided.of_run.get(id as usize) else {
            continue;
        };
        if k == NONE || decided.decisions[k as usize].kind != Kind::Heading {
            continue;
        }
        why_of
            .entry(k as usize)
            .or_insert((why, Vec::new()))
            .1
            .push(id);
    }
    for (k, (why, ids)) in why_of {
        let ids: std::collections::BTreeSet<u32> = ids.into_iter().collect();
        let all = text_of(decided.prims(k));
        let heading = decided.decisions[k].clone();
        let alternative = Alternative {
            kind: Kind::Heading,
            score: heading.score,
            signals: heading.signals.clone(),
        };
        let signal = |of: Option<usize>| Signal {
            id: "pdf.walk.demoted",
            detail: match of {
                None => format!(
                    "a heading by its size, written as a paragraph: {}",
                    demotion_why(why)
                ),
                Some(k) => format!(
                    "lines of the heading region {k}, written as a paragraph: {}",
                    demotion_why(why)
                ),
            },
            weight: 0,
        };
        if all.iter().all(|i| ids.contains(i)) {
            let d = &mut decided.decisions[k];
            d.kind = Kind::Paragraph;
            d.signals = vec![signal(None)];
            d.alternatives.insert(0, alternative);
        } else {
            let (moved, kept): (Vec<PrimId>, Vec<PrimId>) = decided
                .prims(k)
                .iter()
                .partition(|p| matches!(p, PrimId::Text(i) if ids.contains(i)));
            set_own(decided, k, kept);
            let d = Decision {
                kind: Kind::Paragraph,
                signals: vec![signal(Some(k))],
                alternatives: vec![alternative],
                ..heading
            };
            push(decided, d, moved);
        }
    }
    // Lines a table's reader left, read as pairs.
    for ids in pairs {
        let Some(&k) = ids.first().and_then(|&i| decided.of_run.get(i as usize)) else {
            continue;
        };
        if k == NONE {
            continue;
        }
        let k = k as usize;
        let ids: std::collections::BTreeSet<u32> = ids.iter().copied().collect();
        let was = decided.decisions[k].clone();
        let (moved, kept): (Vec<PrimId>, Vec<PrimId>) = decided
            .prims(k)
            .iter()
            .partition(|p| matches!(p, PrimId::Text(i) if ids.contains(i)));
        if moved.is_empty() {
            continue;
        }
        set_own(decided, k, kept);
        let d = Decision {
            kind: Kind::LabelValue,
            certainty: Certainty::Rule,
            signals: vec![Signal {
                id: "pdf.walk.left_pair",
                detail: format!(
                    "a line the reader of the {} left, a pair by the line pair rule as a lone \
                     line",
                    words(was.kind)
                ),
                weight: 0,
            }],
            alternatives: vec![Alternative {
                kind: was.kind,
                score: was.score,
                signals: was.signals.clone(),
            }],
            ..was
        };
        push(decided, d, moved);
    }
    // The footnote legend.
    if !legend.is_empty() {
        legend_decision(decided, legend, &set_own, &push);
    }
    // The kinds written.
    agree_written(decided, page, written, why, &set_own, &push);
}

/// The footnote legend of [`agree`]: its runs (`legend`) are one footnote legend decision.
fn legend_decision(
    decided: &mut Decided,
    legend: &[u32],
    set_own: &dyn Fn(&mut Decided, usize, Vec<PrimId>),
    push: &dyn Fn(&mut Decided, Decision, Vec<PrimId>) -> usize,
) {
    let text_of = |prims: &[PrimId]| -> Vec<u32> {
        prims
            .iter()
            .filter_map(|p| match *p {
                PrimId::Text(i) => Some(i),
                _ => None,
            })
            .collect()
    };
    let ids: std::collections::BTreeSet<u32> = legend.iter().copied().collect();
    let mut from: Vec<usize> = ids
        .iter()
        .filter_map(|&id| decided.of_run.get(id as usize).copied())
        .filter(|&k| k != NONE)
        .map(|k| k as usize)
        .collect();
    from.sort_unstable();
    from.dedup();
    let own_legend = from.iter().copied().find(|&k| {
        decided.decisions[k].kind == Kind::FootnoteLegend
            && text_of(decided.prims(k)).iter().all(|i| ids.contains(i))
    });
    let mut moved: Vec<PrimId> = Vec::new();
    for &k in &from {
        if Some(k) == own_legend {
            continue;
        }
        let (taken, kept): (Vec<PrimId>, Vec<PrimId>) = decided
            .prims(k)
            .iter()
            .partition(|p| matches!(p, PrimId::Text(i) if ids.contains(i)));
        moved.extend(taken);
        set_own(decided, k, kept);
    }
    let others: Vec<String> = from
        .iter()
        .filter(|&&k| Some(k) != own_legend)
        .map(|k| k.to_string())
        .collect();
    let mut detail = "the footnote legend of the page: lines at its foot that open with the \
                      markers the text refers to"
        .to_string();
    if !others.is_empty() {
        detail.push_str(&format!("; its lines were in region {}", others.join(", ")));
    }
    let signal = Signal {
        id: "pdf.footnotes.legend",
        detail,
        weight: layoutregions::typers::MAX_SCORE,
    };
    match own_legend {
        Some(k) => {
            let mut prims = decided.prims(k).to_vec();
            prims.extend(moved);
            prims.sort_unstable();
            set_own(decided, k, prims);
            decided.decisions[k].signals.push(signal);
            for i in text_of(decided.prims(k)) {
                if let Some(slot) = decided.of_run.get_mut(i as usize) {
                    *slot = u32::try_from(k).unwrap_or(NONE);
                }
            }
        }
        None if !moved.is_empty() => {
            moved.sort_unstable();
            let d = Decision {
                seg: decided.segs.root,
                kind: Kind::FootnoteLegend,
                certainty: Certainty::Rule,
                score: Score(layoutregions::typers::MAX_SCORE),
                signals: vec![signal],
                alternatives: Vec::new(),
            };
            push(decided, d, moved);
        }
        None => {}
    }
}

/// Why the lines under a title over its box or bar are a paragraph region of their own.
pub(super) const TITLE_LINES: &str =
    "a line in the box or under the bar of a title, written as a paragraph after it";

/// The kind of a region whose runs were written in a block of the [`crate::block_word`]
/// `word`, if it says one.
fn kind_written(word: &str) -> Option<Kind> {
    Some(match word {
        "heading" => Kind::Heading,
        "paragraph" => Kind::Paragraph,
        "list" => Kind::List,
        "table" => Kind::Table,
        "spatial" => Kind::Spatial,
        "page-header" => Kind::HeaderBand,
        "page-footer" => Kind::FooterBand,
        "section" | "footnote" => Kind::FootnoteLegend,
        _ => return None,
    })
}

/// Whether a region of `kind` agrees with runs written as `written`: the same kind, a title
/// block written as a heading, or pairs and totals written as a table.
fn agrees(kind: Kind, written: Kind) -> bool {
    kind == written
        || matches!(
            (kind, written),
            (Kind::TitleBlock, Kind::Heading) | (Kind::LabelValue | Kind::Totals, Kind::Table)
        )
}

/// Why runs of a decision of kind `was` were written as `kind`, when the writing gave no
/// reason of its own.
fn written_why(was: Kind, kind: Kind) -> String {
    match (was, kind) {
        (Kind::List, Kind::Paragraph) => {
            "a line of the list that opens no item, written as a paragraph".to_string()
        }
        (_, Kind::Spatial) => "it is kept as laid out".to_string(),
        (_, Kind::Table) => format!(
            "read as {} by the reader of a table on the page",
            words(was)
        ),
        _ => format!("the text walk wrote it as {}", words(kind)),
    }
}

/// The last step of [`agree`]: each decision's kind is the kind its runs were written as.
fn agree_written<'w>(
    decided: &mut Decided,
    page: &PageIn,
    written: &[Option<&'static str>],
    why: &dyn Fn(u32) -> Option<&'w str>,
    set_own: &dyn Fn(&mut Decided, usize, Vec<PrimId>),
    push: &dyn Fn(&mut Decided, Decision, Vec<PrimId>) -> usize,
) {
    let inked = |i: u32| {
        page.texts
            .get(i as usize)
            .is_some_and(|t| !t.text.trim().is_empty())
    };
    for k in 0..decided.decisions.len() {
        let was = decided.decisions[k].kind;
        if decided.gone[k] || matches!(was, Kind::Figure | Kind::Code) {
            continue;
        }
        // The runs of the decision written as another kind, by that kind, in the order of the
        // kinds' first runs; and whether any run was written as its own kind. Runs written as
        // its own kind that stand between two runs of another kind in the page's flow part
        // them: a heading over a table and a line under it, both kept as laid out, are two
        // regions, not one that the table stands in.
        let mut other: Vec<(Kind, Vec<u32>)> = Vec::new();
        let mut kept = false;
        // By kind written: the group its runs go to now; a run written as the decision's own
        // kind closes every group.
        let mut open: Vec<(Kind, usize)> = Vec::new();
        let mut ids: Vec<u32> = decided
            .prims(k)
            .iter()
            .filter_map(|p| match *p {
                PrimId::Text(i) => Some(i),
                _ => None,
            })
            .collect();
        ids.sort_unstable();
        for i in ids {
            if !inked(i) {
                continue;
            }
            let Some(kind) = written
                .get(i as usize)
                .copied()
                .flatten()
                .and_then(kind_written)
            else {
                continue;
            };
            if agrees(was, kind) {
                kept = true;
                open.clear();
            } else {
                match open.iter().find(|o| o.0 == kind) {
                    Some(&(_, g)) => other[g].1.push(i),
                    None => {
                        open.push((kind, other.len()));
                        other.push((kind, vec![i]));
                    }
                }
            }
        }
        if other.is_empty() {
            continue;
        }
        let d = decided.decisions[k].clone();
        let refused = matches!(decided.pre[k], Some((_, true)));
        let made = |kind: Kind, first: u32| -> Decision {
            let reason = why(first).map_or_else(|| written_why(was, kind), str::to_string);
            let mut alternatives = vec![Alternative {
                kind: was,
                score: d.score,
                signals: d.signals.clone(),
            }];
            alternatives.extend(
                d.alternatives
                    .iter()
                    .filter(|a| a.kind != kind && a.kind != was)
                    .cloned(),
            );
            let certainty = match (kind, d.certainty) {
                (Kind::Spatial, _) => Certainty::Unsure,
                // Text a finder refused stays unsure whatever the walk wrote it as.
                _ if refused => Certainty::Unsure,
                (_, Certainty::Exact | Certainty::Unsure) => Certainty::Rule,
                (_, c) => c,
            };
            Decision {
                kind,
                certainty,
                signals: vec![Signal {
                    id: "pdf.walk.written",
                    detail: format!(
                        "read as {}, written as {}: {reason}",
                        words(was),
                        words(kind)
                    ),
                    weight: 0,
                }],
                alternatives,
                ..d.clone()
            }
        };
        // With no run written as its kind, the decision takes the kind most of its runs were
        // written as; every other kind written is a decision of its own.
        if !kept {
            let most = other
                .iter()
                .enumerate()
                .max_by(|(a, x), (b, y)| x.1.len().cmp(&y.1.len()).then(b.cmp(a)))
                .map_or(0, |(j, _)| j);
            let (kind, ids) = other.remove(most);
            decided.decisions[k] = made(kind, ids[0]);
        }
        for (kind, ids) in other {
            let moved: std::collections::BTreeSet<u32> = ids.iter().copied().collect();
            let (taken, rest): (Vec<PrimId>, Vec<PrimId>) = decided
                .prims(k)
                .iter()
                .partition(|p| matches!(p, PrimId::Text(i) if moved.contains(i)));
            set_own(decided, k, rest);
            push(decided, made(kind, ids[0]), taken);
        }
    }
}

/// A kind's name in words: `label value` for `label_value`.
fn words(kind: Kind) -> String {
    kind.name().replace('_', " ")
}

/// Why a spatial decision is kept as laid out, from the candidates of its own segment: two
/// kinds that both reach the least score and stand within the margin read `about equally`; a
/// kind that reaches the least score alone reads as that kind, but not clearly enough (it does
/// not lead the next by the margin); otherwise no kind has clear evidence.
pub(super) fn unsure_why(candidates: &[Typed], m: &Margins) -> String {
    let mut own: Vec<&Typed> = candidates
        .iter()
        .filter(|t| t.kind != Kind::Spatial)
        .collect();
    own.sort_by(|a, b| b.score.cmp(&a.score).then(a.kind.cmp(&b.kind)));
    match (own.first(), own.get(1)) {
        (Some(a), Some(b)) if b.score >= m.min && a.score.0 - b.score.0 < m.margin.0 => format!(
            "it reads as {} or as {} about equally ({} and {} of 1000)",
            words(a.kind),
            words(b.kind),
            a.score.0,
            b.score.0
        ),
        (Some(a), _) if a.score >= m.min => format!(
            "it reads as {} ({} of 1000), but not clearly enough",
            words(a.kind),
            a.score.0
        ),
        _ => NO_EVIDENCE.to_string(),
    }
}

/// Why a spatial decision is kept as laid out when no kind reaches the least score (see
/// [`unsure_why`]): the one reason that names no kind.
pub(super) const NO_EVIDENCE: &str = "no region type has clear evidence for it";

/// Why runs of a decision of `kind` that its extractor read no table, pairs or totals from are
/// kept as laid out: the extractor's own reason when it gave one.
pub(super) fn unread_why(kind: Kind, reason: Option<&str>) -> String {
    match reason {
        Some(why) => format!("it reads as {}, but {why}", words(kind)),
        None => format!(
            "it reads as {}, but no {} could be built from it",
            words(kind),
            words(kind)
        ),
    }
}

/// A gap between two runs of a piece wider than this many sizes of their text is a space
/// between words (a run need not hold the space itself).
const WORD_GAP: f32 = 0.15;

/// The parts of a run between its word gaps ([`layoutregions::GapIn`]), each with the x
/// range of its ink, so a run drawn in the gap of another (a note marker after the first of
/// two words the reader kept in one run) stands in that gap, not after the run. Each part keeps
/// the white space around it, so the parts of one run joined give its text. A run whose gaps
/// do not stand inside it in order (a turned or right to left run) stays one part.
fn parts(r: &layoutregions::TextRun) -> Vec<layoutregions::TextRun> {
    let (x0, x1) = (r.x, r.x + r.w);
    let inside = r.w > 0.0
        && !r.off_axis
        && r.gaps
            .windows(2)
            .all(|g| g[0].words < g[1].words && g[0].to <= g[1].from)
        && r.gaps
            .iter()
            .all(|g| x0 <= g.from && g.from <= g.to && g.to <= x1);
    if r.gaps.is_empty() || !inside {
        return vec![r.clone()];
    }
    // The byte where the text after the first `n` words starts (its white space included).
    let cut = |n: u32| -> usize {
        let mut words = 0;
        let mut in_word = false;
        for (i, c) in r.text.char_indices() {
            if c.is_whitespace() {
                if in_word {
                    words += 1;
                    in_word = false;
                    if words == n {
                        return i;
                    }
                }
            } else {
                in_word = true;
            }
        }
        r.text.len()
    };
    let mut out = Vec::with_capacity(r.gaps.len() + 1);
    let (mut at, mut from) = (0usize, x0);
    for g in r.gaps.iter() {
        let to = cut(g.words).max(at);
        out.push(layoutregions::TextRun {
            text: r.text[at..to].to_string(),
            x: from,
            w: g.from - from,
            gaps: Vec::new(),
            ..r.clone()
        });
        (at, from) = (to, g.to);
    }
    out.push(layoutregions::TextRun {
        text: r.text[at..].to_string(),
        x: from,
        w: x1 - from,
        gaps: Vec::new(),
        ..r.clone()
    });
    out.retain(|p| !p.text.is_empty());
    out
}

/// What a run kept as laid out carries besides its text: its link and, for a reference to a
/// footnote, the footnote's label.
#[derive(Clone, Copy, Default)]
pub(super) struct RunMarks<'a> {
    pub uri: Option<&'a std::rc::Rc<super::UriTarget>>,
    pub note: Option<&'a str>,
}

/// One part of a piece's text while it is built: its bytes, its link (an index into the
/// block's links), its footnote and its style.
#[derive(Clone, PartialEq)]
struct Part<'a> {
    from: usize,
    to: usize,
    link: Option<u32>,
    note: Option<&'a str>,
    strong: bool,
    emphasis: bool,
}

/// The spans of a piece from its parts (byte ranges of the untrimmed `text`, `lead` bytes
/// trimmed off its start): parts of one link and style next to each other, or with only white
/// space between them, are one span; a footnote reference is a span of its own; plain text
/// is none.
fn spans_of(text: &str, lead: usize, parts: &[Part<'_>]) -> Vec<rezzer_model::SpatialSpan> {
    let mut merged: Vec<Part<'_>> = Vec::new();
    for p in parts {
        if p.link.is_none() && p.note.is_none() && !p.strong && !p.emphasis {
            continue;
        }
        if let Some(last) = merged.last_mut()
            && p.note.is_none()
            && last.note.is_none()
            && (last.link, last.strong, last.emphasis) == (p.link, p.strong, p.emphasis)
            && text
                .get(last.to..p.from)
                .is_some_and(|between| between.chars().all(char::is_whitespace))
        {
            last.to = p.to;
            continue;
        }
        merged.push(p.clone());
    }
    let trimmed = &text[lead..];
    let end = trimmed.len();
    let chars = |b: usize| {
        let b = b.saturating_sub(lead).min(end);
        u32::try_from(trimmed[..b].chars().count()).unwrap_or(u32::MAX)
    };
    merged
        .into_iter()
        .filter_map(|p| {
            // White space at the edges of a span is outside it.
            let body = text.get(p.from..p.to)?;
            let from = p.from + (body.len() - body.trim_start().len());
            let to = p.from + body.trim_end().len();
            let (start, end) = (chars(from), chars(to));
            (start < end).then(|| rezzer_model::SpatialSpan {
                start,
                end,
                link: p.link,
                footnote: p.note.map(str::to_string),
                emphasis: p.emphasis,
                strong: p.strong,
            })
        })
        .collect()
}

/// The lines of the text runs `prims` of `page` as laid out (see
/// [`layoutregions::extract::spatial::spatial`]): each piece the text of its runs, white space
/// at its ends left out. In a piece the runs are cut at their word gaps ([`parts`]) and stand
/// left to right; two parts are two words when a gap of [`WORD_GAP`] sizes stands between them
/// or when one is another text than the one before ([`layoutregions::line::apart`]: printed
/// over it, or a label and its value set against each other). A note reference stands right
/// after its word, as in a paragraph.
///
/// The text of a run with a link, a run that is a footnote reference (`marks`, by the run's
/// id) and bold or italic text are spans of the piece, so they stay where they stand; `link`
/// gives the index of a target in the block's links, or none when it has no room.
pub(super) fn spatial_lines<'m>(
    page: &PageIn,
    prims: &[PrimId],
    marks: &dyn Fn(u32) -> RunMarks<'m>,
    link: &mut dyn FnMut(&std::rc::Rc<super::UriTarget>) -> Option<u32>,
) -> Vec<SpatialLine> {
    layoutregions::extract::spatial::spatial(page, prims)
        .into_iter()
        .map(|l| SpatialLine {
            pieces: l
                .pieces
                .into_iter()
                .filter_map(|(col, ids)| {
                    let mut runs: Vec<(RunMarks<'m>, layoutregions::TextRun)> = ids
                        .iter()
                        .filter_map(|p| match *p {
                            PrimId::Text(i) => page.texts.get(i as usize).map(|r| (marks(i), r)),
                            _ => None,
                        })
                        .flat_map(|(m, r)| parts(r).into_iter().map(move |p| (m, p)))
                        .collect();
                    runs.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
                    let mut text = String::new();
                    let mut spans: Vec<Part<'m>> = Vec::new();
                    let mut ink_end = f32::MIN;
                    let mut before: Option<&layoutregions::TextRun> = None;
                    for (m, r) in &runs {
                        let note = r.shift == layoutregions::Shift::Raised;
                        let link_at = m.uri.and_then(&mut *link);
                        if note {
                            let end = text.trim_end().len();
                            text.truncate(end);
                            let from = text.len();
                            text.push_str(r.text.trim());
                            spans.push(Part {
                                from,
                                to: text.len(),
                                link: link_at,
                                note: m.note,
                                strong: r.bold,
                                emphasis: r.italic,
                            });
                            ink_end = ink_end.max(r.x + r.w);
                            continue;
                        }
                        if let Some(prev) = before {
                            let gap = r.x - ink_end > WORD_GAP * prev.size.max(r.size);
                            let apart = gap || layoutregions::line::apart(before, r, ink_end);
                            let spaced = text.ends_with(char::is_whitespace)
                                || r.text.starts_with(char::is_whitespace);
                            if apart && !spaced {
                                text.push(' ');
                            }
                        }
                        let from = text.len();
                        text.push_str(&r.text);
                        spans.push(Part {
                            from,
                            to: text.len(),
                            link: link_at,
                            note: m.note,
                            strong: r.bold,
                            emphasis: r.italic,
                        });
                        if !r.text.trim().is_empty() {
                            ink_end = ink_end.max(r.x + r.w);
                            before = Some(r);
                        }
                    }
                    let lead = text.len() - text.trim_start().len();
                    let spans = spans_of(&text, lead, &spans);
                    let text = text.trim();
                    (!text.is_empty()).then(|| SpatialPiece {
                        col,
                        text: text.to_string(),
                        spans,
                    })
                })
                .collect(),
        })
        .filter(|l| !l.pieces.is_empty())
        .collect()
}

/// The regions of a page: one per decision, in the order of the decisions, with the kind,
/// certainty, score, signals and alternatives of its decision, the primitives and bounds of
/// its segment, and `hosts[k]` (the block decision `k` became) as its host. `unsure` holds the
/// reasons the layout gives for a region kept as laid out. `issues` are the page's.
pub(super) fn regions(
    page: u32,
    decided: Decided,
    hosts: &[u32],
    unsure: &BTreeMap<usize, Vec<String>>,
    issues: Vec<Issue>,
) -> PageRegions {
    let at: Vec<Option<(Vec<PrimId>, Bounds)>> = (0..decided.decisions.len())
        .map(|k| {
            (!decided.gone[k])
                .then(|| Some((decided.prims(k).to_vec(), decided.bounds(k)?)))
                .flatten()
        })
        .collect();
    let regions = decided
        .decisions
        .into_iter()
        .zip(at)
        .enumerate()
        .filter_map(|(k, (d, at))| {
            let (prims, bounds) = at?;
            // A region holds something of the page: a decision over no primitives (the root of
            // an empty page) is no region, nor is a spatial one over no text, picture or code
            // (a leaf of rules or boxes alone, which has nothing to read); a region the page
            // states stays, and so does a drawing typed as a figure.
            let readable = prims
                .iter()
                .any(|p| matches!(p, PrimId::Text(_) | PrimId::Picture(_) | PrimId::Code(_)));
            let given = d.signals.iter().any(|s| s.id.starts_with("given."));
            if !given && (prims.is_empty() || (!readable && d.kind == Kind::Spatial)) {
                return None;
            }
            Some(Region {
                id: RegionId(u32::try_from(k).unwrap_or(u32::MAX)),
                page,
                kind: d.kind,
                bounds,
                prims,
                certainty: d.certainty,
                score: if d.certainty == Certainty::Unsure {
                    Score(0)
                } else {
                    d.score
                },
                signals: d.signals,
                alternatives: d.alternatives,
                parent: None,
                unsure: unsure.get(&k).cloned().unwrap_or_default(),
                host: hosts.get(k).copied().unwrap_or(u32::MAX),
            })
        })
        .collect();
    PageRegions {
        page,
        regions,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fix round 1 of Task 37, M4: the regions an unsure region is weighed against hold
    /// every region the two tests accept with it (`join_rows` before Task 37 weighed it
    /// against every region of the page). 60 seeded pages of regions of one to three lines,
    /// some sharing their lines side by side, some with lines between each other's lines in
    /// one column; every pair is asked both ways.
    #[test]
    fn the_regions_weighed_for_a_join_hold_every_pair_the_tests_accept() {
        use layoutregions::extract::spatial::{interleaves, shares_rows};
        let mut seed = 17u64;
        let mut roll = |n: u32| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % u64::from(n)
        };
        let (mut shared, mut between) = (0, 0);
        for _ in 0..60 {
            let mut texts = Vec::new();
            let mut regions: Vec<Vec<PrimId>> = Vec::new();
            for _ in 0..24 {
                // A region of a few lines out of eight, at one of six places across.
                let first = roll(6) as u32;
                let step = 1 + roll(2) as u32;
                let lines = 1 + roll(3) as u32;
                let x = 60.0 * roll(6) as f32 + 5.0 * roll(3) as f32;
                let mut prims = Vec::new();
                for l in 0..lines {
                    let line = first + l * step;
                    texts.push(layoutregions::TextRun {
                        text: if roll(12) == 0 {
                            " ".into()
                        } else {
                            "Muster".into()
                        },
                        x,
                        y: 700.0 - 12.0 * line as f32,
                        w: 30.0 + 10.0 * roll(4) as f32,
                        h: 9.0,
                        size: 9.0,
                        font: 1,
                        font_kind: layoutregions::FontKind::Proportional,
                        bold: false,
                        italic: false,
                        color: None,
                        shift: layoutregions::Shift::Base,
                        line,
                        off_axis: roll(40) == 0,
                        gaps: Vec::new(),
                    });
                    prims.push(PrimId::Text(texts.len() as u32 - 1));
                }
                regions.push(prims);
            }
            let page = PageIn {
                page: 1,
                size: [612.0, 792.0],
                turn: 0,
                texts,
                rules: Vec::new(),
                boxes: Vec::new(),
                pictures: Vec::new(),
                codes: Vec::new(),
                given: Vec::new(),
            };
            let unsure: Vec<bool> = (0..regions.len()).map(|_| roll(3) != 0).collect();
            let ink: Vec<Option<Ink>> = regions.iter().map(|p| Ink::of(&page, p)).collect();
            let near = Near::new(&ink, &unsure);
            for d in 0..regions.len() {
                let mut found = Vec::new();
                if let Some(own) = &ink[d] {
                    near.of(own, &mut found);
                }
                for k in 0..regions.len() {
                    if k == d {
                        continue;
                    }
                    let rows = shares_rows(&page, &regions[d], &regions[k]);
                    let lines = unsure[k] && interleaves(&page, &regions[d], &regions[k]);
                    if rows || lines {
                        assert!(found.contains(&k), "region {k} for region {d}");
                    }
                    shared += usize::from(rows);
                    between += usize::from(lines);
                }
            }
        }
        assert!(
            shared > 20 && between > 20,
            "{shared} share rows, {between} interleave"
        );
    }

    fn typed(kind: Kind, score: u16) -> Typed {
        Typed {
            kind,
            score: Score(score),
            signals: Vec::new(),
        }
    }

    /// Two kinds within the margin read about equally; a kind over the least score that does
    /// not lead the next by the margin reads as it, but not clearly enough; a kind under the
    /// least score is no clear evidence.
    #[test]
    fn the_loss_says_why_a_region_is_spatial() {
        let m = Margins::default();
        assert_eq!(
            unsure_why(
                &[typed(Kind::Paragraph, 600), typed(Kind::LabelValue, 550)],
                &m
            ),
            "it reads as paragraph or as label value about equally (600 and 550 of 1000)"
        );
        assert_eq!(
            unsure_why(&[typed(Kind::Paragraph, 250)], &m),
            "no region type has clear evidence for it"
        );
        // Fix round 1b of Task 19 (M10): the best kind reaches the least score, the second
        // does not, and the lead is under the margin, so arbitration left it spatial.
        assert_eq!(
            unsure_why(&[typed(Kind::Table, 350), typed(Kind::Paragraph, 250)], &m),
            "it reads as table (350 of 1000), but not clearly enough"
        );
        assert_eq!(
            unsure_why(&[], &m),
            "no region type has clear evidence for it"
        );
    }

    fn run(text: &str, x: f32, y: f32, size: f32, line: u32) -> layoutregions::TextRun {
        layoutregions::TextRun {
            text: text.to_string(),
            x,
            y,
            w: text.chars().count() as f32 * size * 0.5,
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

    /// A decision: its kind, its runs and its part of today's reading.
    type Read = (Kind, Vec<u32>, Option<(usize, bool)>);

    /// A page handed in with its runs in another order is decided as the page in the order
    /// of their places, and every decision names the runs by the ids of the page handed in:
    /// the primitives of the segments, the decision of each run and the part of today's
    /// reading a given region stands for.
    #[test]
    fn a_page_is_decided_in_the_order_of_its_places_whatever_order_its_runs_have() {
        // A title, a paragraph of four lines of two runs each, a given table of two rows and
        // a table drawn with rules, three rows of three cells. The drawn table is the part
        // whose read moves with the order of the runs when they are read as handed in:
        // without `RunOrder` in `decide` the reversed order leaves a rule out of the table,
        // a region of its own kept as laid out (review M2 of part 2b-1 of Task 31c).
        let mut texts = vec![run("Notes on the garden wall", 60.0, 740.0, 16.0, 0)];
        for k in 0..4u32 {
            let y = 700.0 - 12.0 * k as f32;
            texts.push(run("the wall was built of the stone", 60.0, y, 10.0, 1 + k));
            texts.push(run("that came from the old quarry", 220.0, y, 10.0, 1 + k));
        }
        for k in 0..2u32 {
            let y = 600.0 - 14.0 * k as f32;
            texts.push(run("Stone", 60.0, y, 10.0, 5 + k));
            texts.push(run("12", 300.0, y, 10.0, 5 + k));
        }
        let mut rules = Vec::new();
        let rule = |x0: f32, y0: f32, x1: f32, y1: f32| layoutregions::RuleIn {
            x0,
            y0,
            x1,
            y1,
            thick: 0.0,
            dashed: false,
            rect: false,
        };
        for (r, y) in [500.0f32, 480.0, 460.0, 440.0].into_iter().enumerate() {
            rules.push(rule(72.0, y, 372.0, y));
            if r < 3 {
                let cells = [["Name", "Place", "Sum"], ["Alder", "Oakfield", "1.234,51"]];
                for (c, x) in [76.0f32, 176.0, 276.0].into_iter().enumerate() {
                    texts.push(run(cells[r.min(1)][c], x, y - 14.0, 9.0, 7 + r as u32));
                }
            }
        }
        for x in [72.0f32, 172.0, 272.0, 372.0] {
            rules.push(rule(x, 440.0, x, 500.0));
        }
        let n = texts.len() as u32;
        let page_of = |order: &[u32]| {
            let at = |old: u32| order.iter().position(|&i| i == old).unwrap() as u32;
            PageIn {
                page: 0,
                size: [600.0, 800.0],
                turn: 0,
                texts: order.iter().map(|&i| texts[i as usize].clone()).collect(),
                rules: rules.clone(),
                boxes: Vec::new(),
                pictures: Vec::new(),
                codes: Vec::new(),
                given: vec![layoutregions::GivenRegion {
                    kind: Kind::Table,
                    prims: (9..13).map(|i| PrimId::Text(at(i))).collect(),
                    order: 0,
                    level: None,
                    role: None,
                    group: None,
                }],
            }
        };
        let signal = |_: usize| {
            (
                Signal {
                    id: "test.table",
                    detail: String::new(),
                    weight: 1000,
                },
                false,
            )
        };
        let decided = |order: &[u32]| {
            let page = page_of(order);
            let placed = page_of(&(0..n).collect::<Vec<_>>());
            let file = [layoutregions::file::FilePage::of(&placed)];
            let facts = layoutregions::file::learn(&file, false, &|_| false);
            let index = FileIndex::new(&facts);
            // The part of today's reading: the table, named by the ids of the page handed in.
            let pre: HashMap<u32, usize> = (0..n)
                .filter(|&i| (9..13).contains(&order[i as usize]))
                .map(|i| (i, 7))
                .collect();
            let grids = layoutregions::segment::drawn::grids(&page.rules);
            let d = decide(
                &page,
                &grids,
                (&facts, &index),
                &|_| false,
                (&pre, &signal),
                &Limits::default(),
                &|| false,
            );
            // Each decision as its kind, the runs it holds (by the ids of the page in the
            // order of its places) and its part of today's reading.
            let mut out: Vec<Read> = Vec::new();
            for k in 0..d.decisions.len() {
                let mut runs: Vec<u32> = d
                    .prims(k)
                    .iter()
                    .filter_map(|p| match *p {
                        PrimId::Text(i) => Some(order[i as usize]),
                        // The rules of the drawn table keep their ids in every order.
                        _ => None,
                    })
                    .collect();
                assert!(d.prims(k).is_sorted(), "sorted by the ids handed in");
                for p in d.prims(k) {
                    let PrimId::Text(i) = *p else { continue };
                    assert_eq!(d.of_run[i as usize], k as u32, "run {i} of {order:?}");
                }
                runs.sort_unstable();
                out.push((d.decisions[k].kind, runs, d.pre[k]));
            }
            out
        };
        let in_order: Vec<u32> = (0..n).collect();
        let base = decided(&in_order);
        assert!(
            base.iter().any(|(k, runs, pre)| *k == Kind::Table
                && *runs == [9, 10, 11, 12]
                && *pre == Some((7, false))),
            "{base:?}"
        );
        assert!(
            base.iter()
                .any(|(k, runs, _)| *k == Kind::Paragraph && runs.len() == 8),
            "{base:?}"
        );
        assert!(
            base.iter()
                .any(|(k, runs, pre)| *k == Kind::Table && runs.len() == 9 && pre.is_none()),
            "{base:?}"
        );
        let reversed: Vec<u32> = (0..n).rev().collect();
        let mixed: Vec<u32> = (0..n).map(|i| (i * 5 + 3) % n).collect();
        for order in [reversed, mixed] {
            assert_eq!(decided(&order), base, "order {order:?}");
        }
    }

    /// Page `n` of a file whose every page has a footnote legend and runs out of the order
    /// of their places: a line with a raised number, four body lines in two runs each with
    /// the right run first, and the legend of one footnote. The words differ by page, so no
    /// line is a running line.
    fn page_with_legend(n: u32) -> PageIn {
        const PARTS: [&str; 8] = ["bar", "den", "fol", "gim", "hul", "jor", "kes", "lim"];
        let word = |k: u32| {
            let at = |d: u32| PARTS[((n * 5 + k) / 8u32.pow(d) % 8) as usize];
            format!("{}{}{}", at(0), at(1), at(2))
        };
        let mut texts = vec![
            run(&format!("the {} charge", word(0)), 40.0, 720.0, 9.0, 0),
            run(" 2", 112.0, 724.0, 6.0, 0),
        ];
        for k in 0..4u32 {
            let y = 690.0 - 12.0 * k as f32;
            texts.push(run("on every day of the week", 240.0, y, 9.0, 1 + k));
            let left = format!("the {} mill ran", word(1 + k));
            texts.push(run(&left, 40.0, y, 9.0, 1 + k));
        }
        texts.push(run(&format!("2 per {}", word(5)), 40.0, 70.0, 6.0, 5));
        PageIn {
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

    /// The steps of work [`decide`] takes for a file of `pages` pages of
    /// [`page_with_legend`]; `None` in a release build.
    fn legend_pages_steps(pages: u32) -> Option<u64> {
        let ins: Vec<PageIn> = (1..=pages).map(page_with_legend).collect();
        let file: Vec<_> = ins.iter().map(layoutregions::file::FilePage::of).collect();
        let facts = layoutregions::file::learn(&file, false, &|_| false);
        assert_eq!(
            facts.legends.len(),
            pages as usize,
            "a legend on every page"
        );
        let index = FileIndex::new(&facts);
        let pre = HashMap::new();
        let signal = |_: usize| -> (Signal, bool) { unreachable!("no given region") };
        let before = crate::work::work_steps();
        for page in &ins {
            assert!(layoutregions::RunOrder::of(page).is_some(), "out of order");
            let grids = layoutregions::segment::drawn::grids(&page.rules);
            let d = decide(
                page,
                &grids,
                (&facts, &index),
                &|_| false,
                (&pre, &signal),
                &Limits::default(),
                &|| false,
            );
            // The legend is found by its renamed runs: the last run handed in.
            let last = page.texts.len() as u32 - 1;
            let k = d.of_run[last as usize] as usize;
            assert_eq!(
                d.decisions[k].kind,
                Kind::FootnoteLegend,
                "page {}",
                page.page
            );
            assert_eq!(d.prims(k), [PrimId::Text(last)]);
        }
        crate::work::work_steps().zip(before).map(|(a, b)| a - b)
    }

    /// A page out of the order of its places with a footnote legend gets its own legend
    /// renamed, and no other page's: the work of a file of such pages grows with its pages,
    /// not with their square (review L2 of part 2b-1 of Task 31c: every such page copied the
    /// legends of the whole file). The renaming counts its steps (`RunOrder::legend`).
    #[test]
    fn pages_out_of_place_order_with_a_legend_each_take_work_in_line_with_the_pages() {
        let (few, many) = (legend_pages_steps(8), legend_pages_steps(128));
        eprintln!("legend pages: 8 pages {few:?} steps, 128 pages {many:?} steps");
        if let Some((few, many)) = few.zip(many) {
            // 16 times the pages: 16 times the steps, with an eighth of the small file's
            // steps of room. A copy of every legend per page would add 128 * 128 legends.
            assert!(
                many <= 16 * few + few / 8,
                "{few} steps for 8, {many} for 128"
            );
        }
    }
}
