// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The detectors of a page as region finders (see `layoutregions`): tagged tables, running
//! headers and footers, the footnote legend, underlined titles, box pairs and the tables
//! without tags, run by [`layoutregions::find_in_order`] in the order and behind the gates
//! [`super::page_blocks_at`] always had. Each finder offers what its detector took, one
//! candidate per table, title or band, under the detector's own signal id (see
//! [`Detector`]); the arbitration keeps the first claim on a run. After the layout, the blocks
//! laid out from the runs no finder took, the pictures and the codes of the page are offered
//! too, so every run of the page is in one region; the text of a grid, band or region refused
//! as a table carries the reason in its regions.

use std::collections::{HashMap, HashSet};

use layoutregions::{
    Bounds, Candidate, Certainty, Finder, InOrder, Kind, Offer, PageIn, PrimId, Score, Signal,
};
use rezzer_model::{Block, LossReason, Severity};

use super::{
    Band, Built, Context, Drawn, Flow, Placed, Run, Slot, footnotes, furniture_key_of, lines,
    page_text_sizes_of, refused_quietly, time_over,
};
use crate::engine;
use crate::tables::{self, BoxTitle, Detector, Doubts, Planned};

/// The score of a region a detector found: a detector's claim is not weighed yet.
const FOUND: u16 = 1000;

/// The signal id of the blocks laid out from the runs no finder took.
const LINES: &str = "pdf.lines";

/// The `host` of a region no [`Placed`] stands for (a band, a footnote legend).
pub(super) const NO_PLACED: u32 = u32::MAX;

/// Whether the regions of the signal `id` name a [`Placed`] by `host`: the tables of every
/// detector (see [`Detector`]) and the titles.
pub(super) fn placed_by(id: &str) -> bool {
    id == UnderlinedTitles::ID || Detector::ALL.iter().any(|d| d.id() == id)
}

/// What the detectors of one page share while they run: the flow they take runs from, the
/// tables and titles they place, their notes, and what the page is.
pub(crate) struct PageCx<'p> {
    pub(super) page: u32,
    pub(super) height: Option<f32>,
    pub(super) cx: &'p Context<'p>,
    pub(super) drawn: &'p Drawn<'p>,
    /// What a bound or the stop cut short in the segmentation of the page (see
    /// [`UntaggedTables`]).
    pub(super) segment_issues: Vec<layoutregions::Issue>,
    pub(super) flow: Flow,
    pub(super) placed: Vec<Placed>,
    pub(super) notes: Vec<(String, LossReason, Severity)>,
    /// Whether the geometric detectors run: false on a tagged page whose tag tree holds.
    pub(super) geometry: bool,
    /// The text sizes of the page (see [`text_sizes`]), once the footnotes are taken out.
    pub(super) text_sizes: Option<Vec<f32>>,
    pub(super) legend: footnotes::Legend,
    pub(super) headers: Vec<Run>,
    pub(super) footers: Vec<Run>,
    /// The blocks of the page: the running headers are built as soon as they are found.
    pub(super) built: Built,
    /// In a debug build: the runs the tagged tables took and dropped as repeated header rows,
    /// which no region holds.
    pub(super) repeats: Vec<u32>,
    /// The grids, bands and regions refused as tables whose text stays as it stands: the ids
    /// of their runs and why (see [`mark_doubts`]).
    pub(super) doubts: Doubts,
}

/// The text sizes of a page, counted over all its text but its legend, also what tables take
/// later (the values of a form are its data lines), and a footer of a file of two pages (see
/// [`super::Style::two_page`]).
pub(super) fn text_sizes(p: &PageCx<'_>) -> Vec<f32> {
    let two_page_lines = lines(&p.footers);
    let two_page = two_page_lines
        .iter()
        .filter(|l| furniture_key_of(l, p.height).is_some_and(|k| p.cx.style.two_page.contains(&k)))
        .flat_map(|l| l.runs.iter());
    page_text_sizes_of(p.flow.runs.iter().chain(two_page))
}

/// The box of the runs, pictures and codes `prims` of `page`: a run's box reaches from its
/// left edge over its advance, and from a third of its size below the baseline to its size
/// above it. `None` when `prims` names nothing of the page.
fn bounds_of(page: &PageIn, prims: &[PrimId]) -> Option<Bounds> {
    let mut out: Option<Bounds> = None;
    for &p in prims {
        let b = match p {
            PrimId::Text(_) => {
                let r = engine::run_of(p).and_then(|i| page.texts.get(i))?;
                Bounds {
                    x0: r.x.min(r.x + r.w),
                    y0: r.y - r.h / 3.0,
                    x1: r.x.max(r.x + r.w),
                    y1: r.y + r.h,
                }
            }
            PrimId::Picture(i) => page.pictures.get(i as usize)?.bounds,
            PrimId::Code(i) => page.codes.get(i as usize)?.bounds,
            PrimId::Rule(_) | PrimId::Box(_) => continue,
        };
        out = Some(match out {
            None => b,
            Some(o) => Bounds {
                x0: o.x0.min(b.x0),
                y0: o.y0.min(b.y0),
                x1: o.x1.max(b.x1),
                y1: o.y1.max(b.y1),
            },
        });
    }
    out
}

/// The candidate of `kind` for `prims` (sorted, each once), found by the finder `id` with the
/// found-by text `detail`; `None` for no pieces.
fn candidate(
    page: &PageIn,
    kind: Kind,
    mut prims: Vec<PrimId>,
    id: &'static str,
    detail: &str,
    host: u32,
) -> Option<Candidate> {
    prims.sort_unstable();
    prims.dedup();
    let bounds = bounds_of(page, &prims)?;
    Some(Candidate {
        kind,
        prims,
        bounds,
        score: Score(FOUND),
        signals: vec![Signal {
            id,
            detail: detail.to_string(),
            weight: FOUND,
        }],
        host,
    })
}

/// The text pieces of the runs `ids`.
fn texts(ids: &[u32]) -> Vec<PrimId> {
    ids.iter().map(|&i| PrimId::Text(i)).collect()
}

/// Offers `c`. A refusal is a bug: two detectors took one run, or one detector put parts of a
/// run in two places; a debug build says which.
fn offer(page: &PageIn, arb: &mut InOrder, c: Option<Candidate>, certainty: Certainty) {
    let Some(c) = c else { return };
    let held: Vec<PrimId> = if cfg!(debug_assertions) {
        c.prims
            .iter()
            .copied()
            .filter(|p| arb.is_taken(*p))
            .collect()
    } else {
        Vec::new()
    };
    let what = (c.kind, c.signals.first().map(|s| s.id), c.prims.len());
    let answer = arb.offer_as(c, certainty);
    debug_assert!(
        !matches!(answer, Offer::Refused { .. }),
        "page {}: the offer {what:?} is refused, {answer:?} holds {held:?}",
        page.page
    );
}

/// Offers a table a detector planned as the `host`th placed part, under its detector's id and
/// of the kind its detector gives it (see [`Detector`]).
fn offer_table(page: &PageIn, arb: &mut InOrder, t: &Planned, host: u32, certainty: Certainty) {
    let c = candidate(
        page,
        t.by.region_kind(t.kind),
        texts(&t.ids),
        t.by.id(),
        t.found_by.unwrap_or(""),
        host,
    );
    offer(page, arb, c, certainty);
}

/// Offers a title over its lines as a heading, as the `host`th placed part.
fn offer_title(page: &PageIn, arb: &mut InOrder, t: &BoxTitle, id: &'static str, host: u32) {
    let c = candidate(page, Kind::Heading, texts(&t.ids), id, "", host);
    offer(page, arb, c, Certainty::Rule);
}

/// The place the next part takes among the placed ones.
fn next_host(p: &PageCx<'_>) -> u32 {
    u32::try_from(p.placed.len()).unwrap_or(NO_PLACED)
}

/// The tables of the tag tree: the file states them. A tagged page whose tag tree holds gets
/// no geometric detection after them.
pub(super) struct TaggedTables;

impl TaggedTables {
    const ID: &'static str = Detector::Tagged.id();
}

impl Finder<PageCx<'_>> for TaggedTables {
    fn name(&self) -> &'static str {
        Self::ID
    }

    fn find(&mut self, page: &PageIn, p: &mut PageCx<'_>, arb: &mut InOrder) {
        let cx = p.cx;
        let Some(tree) = cx.tree else {
            return;
        };
        // Tagged tables are taken out before running headers and footers are looked for: the
        // column titles a long table repeats at the top of each page are table text.
        let before: Vec<u32> = if cfg!(debug_assertions) {
            p.flow.ids.clone()
        } else {
            Vec::new()
        };
        let (found, fallbacks) =
            tables::tagged(p.page, &mut p.flow, tree, cx.tables, cx.in_part, &|| {
                time_over(cx.budget).is_err()
            });
        if cfg!(debug_assertions) {
            // The runs of a tagged table that adds only repeated header rows to a table begun
            // on an earlier page are taken and dropped as repeats: no table and no region
            // holds them (until Task 19 gives every run a region).
            let after: HashSet<u32> = p.flow.ids.iter().copied().collect();
            let owned: HashSet<u32> = found.iter().flat_map(|t| t.ids.iter().copied()).collect();
            p.repeats.extend(
                before
                    .into_iter()
                    .filter(|i| !after.contains(i) && !owned.contains(i)),
            );
        }
        for t in found {
            offer_table(page, arb, &t, next_host(p), Certainty::Exact);
            p.placed.push(Placed::Table(t));
        }
        for what in cx.tables.take_issues() {
            p.notes.push((what, LossReason::Damaged, Severity::Warning));
        }
        p.geometry = !fallbacks.is_empty();
        p.notes.extend(
            fallbacks
                .into_iter()
                .map(|w| (w, LossReason::Uncertain, Severity::Info)),
        );
    }
}

/// Running headers and footers (see [`super::Style::furniture`]): one band of each per page.
/// The header lines are built as paragraphs at once, the first blocks of the page.
pub(super) struct Furniture;

impl Furniture {
    const ID: &'static str = "pdf.furniture";
}

impl Finder<PageCx<'_>> for Furniture {
    fn name(&self) -> &'static str {
        Self::ID
    }

    fn find(&mut self, page: &PageIn, p: &mut PageCx<'_>, arb: &mut InOrder) {
        let cx = p.cx;
        let bands = {
            let all = lines(&p.flow.runs);
            // Text the tag tree places is content, wherever it stands.
            let content: Vec<bool> = if cx.tree.is_some() {
                let mut at = 0;
                all.iter()
                    .map(|l| {
                        let s = &p.flow.slots[at..at + l.runs.len()];
                        at += l.runs.len();
                        s.iter().any(|s| *s != Slot::Free)
                    })
                    .collect()
            } else {
                vec![false; all.len()]
            };
            let by_line = cx.style.furniture(&all, p.height, &content);
            let mut bands = Vec::with_capacity(p.flow.runs.len());
            for (l, b) in all.iter().zip(by_line) {
                bands.extend(std::iter::repeat_n(b, l.runs.len()));
            }
            bands
        };
        let edges = p.flow.take(|i, _| bands[i].is_some());
        let (mut top, mut bottom) = (Vec::new(), Vec::new());
        for (t, b) in edges.into_iter().zip(bands.iter().flatten()) {
            match b {
                Band::Top => {
                    p.headers.push(t.run);
                    top.push(t.id);
                }
                Band::Bottom => {
                    p.footers.push(t.run);
                    bottom.push(t.id);
                }
            }
        }
        // One list of blocks for the whole page, each charged as it is built: a stop keeps
        // the blocks before it.
        for l in lines(&p.headers) {
            p.built
                .paragraph(l.runs.iter().collect(), Some("page-header"), cx);
        }
        for (kind, ids) in [(Kind::HeaderBand, top), (Kind::FooterBand, bottom)] {
            let c = candidate(page, kind, texts(&ids), Self::ID, "", NO_PLACED);
            offer(page, arb, c, Certainty::Rule);
        }
    }
}

/// The footnote legend of an untagged page (see `footnotes::take`); a tagged file says its
/// notes. The page's text sizes are counted once the legend is out.
pub(super) struct Footnotes;

impl Footnotes {
    const ID: &'static str = "pdf.footnotes";
}

impl Finder<PageCx<'_>> for Footnotes {
    fn name(&self) -> &'static str {
        Self::ID
    }

    fn find(&mut self, page: &PageIn, p: &mut PageCx<'_>, arb: &mut InOrder) {
        let cx = p.cx;
        // Footnotes are found before tables and lists: a legend set in columns is no table,
        // and a star that opens a footnote is no bullet.
        if cx.tree.is_none() {
            p.legend = footnotes::take(p.page, &mut p.flow, cx.style.legends.get(&p.page));
        }
        if let Some(why) = p.legend.refused.take() {
            p.notes.push((why, LossReason::Uncertain, Severity::Info));
        }
        let c = candidate(
            page,
            Kind::FootnoteLegend,
            texts(&p.legend.ids),
            Self::ID,
            "",
            NO_PLACED,
        );
        offer(page, arb, c, Certainty::Rule);
        p.text_sizes = Some(text_sizes(p));
    }
}

/// A bold title alone over a rule no grid takes, and the lines under it, on a page drawn as a
/// form (see `tables::form_drawn`). Only where the geometric detectors run and the page has
/// no form fields.
pub(super) struct UnderlinedTitles;

impl UnderlinedTitles {
    const ID: &'static str = "pdf.underlined_title";
}

impl Finder<PageCx<'_>> for UnderlinedTitles {
    fn name(&self) -> &'static str {
        Self::ID
    }

    fn find(&mut self, page: &PageIn, p: &mut PageCx<'_>, arb: &mut InOrder) {
        // A title over a bar is a form's: a page of prose keeps its headings and paragraphs.
        if !p.geometry
            || p.height.is_none()
            || !p.drawn.widgets.is_empty()
            || !tables::form_drawn(p.drawn.grids(), p.drawn.boxes)
        {
            return;
        }
        let cx = p.cx;
        let stop = || time_over(cx.budget).is_err();
        let drawn = (p.drawn.rules, p.drawn.grids());
        let (titles, halted) = tables::underlined_titles(&mut p.flow, drawn, &stop);
        for t in titles {
            offer_title(page, arb, &t, Self::ID, next_host(p));
            p.placed.push(Placed::Title(t));
        }
        if let Some(why) = halted {
            p.notes.push((
                format!(
                    "underlined titles on page {} are looked for only in part: {why}; the \
                     text past that is read as it stands",
                    p.page
                ),
                LossReason::Limit,
                Severity::Info,
            ));
        }
    }
}

/// Drawn boxes of a label over its value, and the titles beside them; taken before the
/// tables without tags, as a row of them is no header band. Only where the geometric
/// detectors run and the page has no form fields (a page with form fields is the form's).
pub(super) struct BoxPairs;

impl BoxPairs {
    const ID: &'static str = Detector::BoxPairs.id();
}

impl Finder<PageCx<'_>> for BoxPairs {
    fn name(&self) -> &'static str {
        Self::ID
    }

    fn find(&mut self, page: &PageIn, p: &mut PageCx<'_>, arb: &mut InOrder) {
        if !p.geometry || p.height.is_none() || !p.drawn.widgets.is_empty() {
            return;
        }
        let cx = p.cx;
        let stop = || time_over(cx.budget).is_err();
        let (found, titles, (unsure, doubts), skipped) = tables::box_pairs(
            p.page,
            &mut p.flow,
            p.drawn.boxes,
            p.drawn.grids(),
            p.text_sizes.as_deref().unwrap_or_default(),
            &stop,
        );
        for t in found {
            offer_table(page, arb, &t, next_host(p), Certainty::Rule);
            p.placed.push(Placed::Table(t));
        }
        for t in titles {
            offer_title(page, arb, &t, Self::ID, next_host(p));
            p.placed.push(Placed::Title(t));
        }
        p.notes.extend(
            unsure
                .into_iter()
                .map(|w| (w, LossReason::Uncertain, Severity::Info)),
        );
        p.doubts.extend(doubts);
        p.notes
            .extend(skipped.map(|w| (w, LossReason::Limit, Severity::Info)));
    }
}

/// The tables without tags (see `tables::untagged`), only where the geometric detectors run.
/// One pass runs their detectors in their order: on the drawn rules the columns of totals
/// beside ruled sections (`pdf.side_totals`), the sections of ruled grids (`pdf.ruled`), the
/// header bands (`pdf.banded`) and the rows of fields between rules (`pdf.strips`), whose runs
/// leave the flow together once all of them have claimed theirs; then the detector without
/// rules by its classes (`pdf.borderless.*`). The runs of a table are known once they left the
/// flow, so each table is offered after the pass, under its own detector, in that order.
pub(super) struct UntaggedTables;

impl UntaggedTables {
    /// No region has this id: each has its detector's.
    const NAME: &'static str = "pdf.untagged_tables";
}

/// Where a detector's tables are offered among those of the tables without tags: the
/// detectors on drawn rules in their order, then the detector without rules, whose classes
/// are decided region by region down the page.
fn offer_rank(by: Detector) -> usize {
    if by.borderless() {
        Detector::ALL.len()
    } else {
        Detector::ALL.iter().position(|d| *d == by).unwrap_or(0)
    }
}

impl Finder<PageCx<'_>> for UntaggedTables {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn find(&mut self, page: &PageIn, p: &mut PageCx<'_>, arb: &mut InOrder) {
        let (true, Some(height)) = (p.geometry, p.height) else {
            return;
        };
        let cx = p.cx;
        let stop = || time_over(cx.budget).is_err();
        // In a debug build: how often each id stands in the flow before the pass, to tell the
        // runs it cut in pieces.
        let before: HashMap<u32, usize> = if cfg!(debug_assertions) {
            counts(p.flow.ids.iter())
        } else {
            HashMap::new()
        };
        // The segments of the page, once and only where the geometric detectors run, on the
        // drawn structure the text pass read the page in (its boxes are those its runs were
        // read in): where the table detectors look.
        let segments = layoutregions::segment::segment_drawn(
            page,
            p.drawn.structure,
            &layoutregions::Limits::default(),
            &stop,
        );
        let found = tables::untagged(p.page, &mut p.flow, p.drawn, &segments, height, &stop);
        p.segment_issues = segments.issues;
        let cut: HashSet<u32> = if cfg!(debug_assertions) {
            counts(
                p.flow
                    .ids
                    .iter()
                    .chain(found.tables.iter().flat_map(|t| &t.ids)),
            )
            .into_iter()
            .filter(|(id, n)| *n > before.get(id).copied().unwrap_or(0))
            .map(|(id, _)| id)
            .collect()
        } else {
            HashSet::new()
        };
        let first = next_host(p);
        let mut order: Vec<usize> = (0..found.tables.len()).collect();
        order.sort_by_key(|&k| offer_rank(found.tables[k].by));
        for k in order {
            let t = &found.tables[k];
            let host = first.saturating_add(u32::try_from(k).unwrap_or(NO_PLACED));
            // A run cut in pieces keeps one id for all of them (`ruled::apply_cuts`): when two
            // tables took pieces of it, the run is the first table's, the detector on the
            // drawn rules that cut it. Anything else claimed twice is a refusal.
            let ids: Vec<u32> = t
                .ids
                .iter()
                .copied()
                .filter(|&i| {
                    let taken = arb.is_taken(PrimId::Text(i));
                    debug_assert!(
                        !taken || cut.contains(&i),
                        "page {}: run {i} of a {} table is claimed, but it is not cut",
                        page.page,
                        t.by.id()
                    );
                    !taken
                })
                .collect();
            let c = candidate(
                page,
                t.by.region_kind(t.kind),
                texts(&ids),
                t.by.id(),
                t.found_by.unwrap_or(""),
                host,
            );
            offer(page, arb, c, Certainty::Rule);
        }
        p.placed.extend(found.tables.into_iter().map(Placed::Table));
        p.placed
            .extend(found.unsure.into_iter().map(Placed::Unsure));
        p.doubts.extend(found.doubts);
        p.notes.extend(
            found
                .skipped
                .into_iter()
                .map(|w| (w, LossReason::Limit, Severity::Info)),
        );
        p.notes.extend(
            found
                .uncertain
                .into_iter()
                .map(|w| (w, LossReason::Uncertain, Severity::Info)),
        );
        p.notes.extend(
            refused_quietly(p.page, found.quiet)
                .into_iter()
                .map(|w| (w, LossReason::Uncertain, Severity::Info)),
        );
    }
}

/// How often each id stands in `ids`.
fn counts<'a>(ids: impl Iterator<Item = &'a u32>) -> HashMap<u32, usize> {
    let mut out = HashMap::new();
    for &id in ids {
        *out.entry(id).or_insert(0) += 1;
    }
    out
}

/// Adds the reason of each grid, band or region refused as a table (`doubts`: the ids of its
/// runs and why) to the `unsure` notes of the regions laid out from its runs as text, once
/// each.
pub(super) fn mark_doubts(regions: &mut layoutregions::PageRegions, doubts: &[(Vec<u32>, &str)]) {
    if doubts.is_empty() {
        return;
    }
    let mut why_of: HashMap<u32, Vec<&str>> = HashMap::new();
    for (ids, why) in doubts {
        for &id in ids {
            let all = why_of.entry(id).or_default();
            if !all.contains(why) {
                all.push(why);
            }
        }
    }
    for r in &mut regions.regions {
        if r.signals.first().map(|s| s.id) != Some(LINES) {
            continue;
        }
        for p in &r.prims {
            let PrimId::Text(id) = p else { continue };
            for why in why_of.get(id).into_iter().flatten() {
                if !r.unsure.iter().any(|u| u == why) {
                    r.unsure.push((*why).to_string());
                }
            }
        }
    }
}

/// The text of a run as it was read and as a piece of it: white space at its ends left out.
fn core(text: &str) -> &str {
    text.trim()
}

/// Checks that each run of `flow` is the run its id names in `page`, or a piece of it (a cut,
/// or a footnote reference whose text is its label).
pub(super) fn check_ids(page: &PageIn, flow: &Flow) {
    for (r, &id) in flow.runs.iter().zip(&flow.ids) {
        let whole = page.texts.get(id as usize).map(|t| t.text.as_str());
        let piece = core(&r.text);
        assert!(
            r.note || piece.is_empty() || whole.is_some_and(|w| w.contains(piece)),
            "page {}: the run {:?} carries the id {id} of {whole:?}",
            page.page,
            r.text
        );
    }
}

/// The ids of the runs of `flow` that are pieces of a cut run: an id that stands more than once
/// in the flow, or a run whose text is less than its id's run.
pub(super) fn cut_runs(page: &PageIn, flow: &Flow) -> HashSet<u32> {
    let mut seen = HashSet::new();
    let mut cut = HashSet::new();
    for (r, &id) in flow.runs.iter().zip(&flow.ids) {
        let whole = page.texts.get(id as usize).map(|t| core(&t.text));
        if !seen.insert(id) || whole.is_some_and(|w| w != core(&r.text)) {
            cut.insert(id);
        }
    }
    cut
}

/// Checks that every run of `page` is in exactly one region: the text pieces of the regions,
/// with the runs `repeats` dropped as repeated header rows, are the ids 0 up to the page's
/// runs, each once.
pub(super) fn check_cover(page: &PageIn, regions: &layoutregions::PageRegions, repeats: &[u32]) {
    let mut ids: Vec<u32> = regions
        .regions
        .iter()
        .flat_map(|r| &r.prims)
        .filter_map(|p| match p {
            PrimId::Text(i) => Some(*i),
            _ => None,
        })
        .chain(repeats.iter().copied())
        .collect();
    ids.sort_unstable();
    let all = (0..page.texts.len()).map(|i| i as u32);
    if ids.iter().copied().eq(all) {
        return;
    }
    let twice: Vec<u32> = ids
        .windows(2)
        .filter(|w| w[0] == w[1])
        .map(|w| w[0])
        .collect();
    let held: HashSet<u32> = ids.into_iter().collect();
    let none: Vec<(u32, &str)> = (0..page.texts.len() as u32)
        .filter(|i| !held.contains(i))
        .take(8)
        .map(|i| (i, page.texts[i as usize].text.as_str()))
        .collect();
    panic!(
        "page {}: of {} runs, these are in no region: {none:?}; in two: {twice:?}",
        page.page,
        page.texts.len()
    );
}

/// What a block laid out from the runs no finder took is as a region.
fn kind_of(block: &Block) -> Kind {
    match block {
        Block::Heading { .. } => Kind::Heading,
        Block::List { .. } => Kind::List,
        Block::Table { .. } => Kind::Table,
        Block::Section { .. } => Kind::FootnoteLegend,
        Block::Paragraph { role, .. } => match role.as_deref() {
            Some("footnote") => Kind::FootnoteLegend,
            Some("page-header") => Kind::HeaderBand,
            Some("page-footer") => Kind::FooterBand,
            _ => Kind::Paragraph,
        },
        _ => Kind::Paragraph,
    }
}

/// Offers each block laid out from runs no finder took (`ids`, by block), with the block's
/// place among `blocks` as its host; then each picture and each code of the page, the
/// picture's or code's place as its host.
pub(super) fn offer_rest(
    page: &PageIn,
    arb: &mut InOrder,
    blocks: &[Block],
    ids: &[Vec<u32>],
    cut: &HashSet<u32>,
) {
    for (k, (block, ids)) in blocks.iter().zip(ids).enumerate() {
        // A run cut in pieces keeps one id for all of them (`ruled::apply_cuts`): when a
        // detector took some pieces into its table and others stay text, the run is the
        // detector's, the first claim; a run whose pieces stand in two blocks is the first
        // block's. The pieces get ids of their own in Task 19.
        let ids: Vec<u32> = ids
            .iter()
            .copied()
            .filter(|&i| {
                let taken = arb.is_taken(PrimId::Text(i));
                // Only a run cut in pieces may be claimed twice; anything else is a refusal.
                debug_assert!(
                    !taken || cut.contains(&i),
                    "page {}: run {i} is laid out as text and claimed, but it is not cut",
                    page.page
                );
                !taken
            })
            .collect();
        if ids.is_empty() {
            continue;
        }
        let host = u32::try_from(k).unwrap_or(NO_PLACED);
        let c = candidate(page, kind_of(block), texts(&ids), LINES, "", host);
        offer(page, arb, c, Certainty::Rule);
    }
    for i in 0..page.pictures.len() {
        let i = u32::try_from(i).unwrap_or(u32::MAX);
        let c = candidate(
            page,
            Kind::Figure,
            vec![PrimId::Picture(i)],
            "pdf.pictures",
            "",
            i,
        );
        offer(page, arb, c, Certainty::Rule);
    }
    for i in 0..page.codes.len() {
        let i = u32::try_from(i).unwrap_or(u32::MAX);
        let c = candidate(page, Kind::Code, vec![PrimId::Code(i)], "pdf.codes", "", i);
        offer(page, arb, c, Certainty::Rule);
    }
}
