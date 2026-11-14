// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The segmentation of a page: what its drawing and its text part it into. [`drawn`] finds
//! the grids and boxes the drawn lines make, once per page; [`cut`] orders and parts text by
//! white space; [`style`] parts it by steps of style. [`segment_page`] nests them into one
//! tree of candidate regions per page.
//!
//! Segmentation proposes and never types: a segment says what parted it ([`Source`]), not
//! what it is.

pub mod cut;
pub mod drawn;
pub mod style;

use crate::input::{GivenRegion, PageIn, PrimId, RuleIn, TextRun};
use crate::limits::Limits;
use crate::line::PIECE_GAP;
use crate::region::{Bounds, Issue, IssueKind, Kind};
use crate::segment::cut::{CutShort, Piece, Split};
use crate::segment::drawn::{Line, SNAP};
use crate::segment::style::style_steps;
use crate::work;
use std::collections::HashMap;

/// A segment of a page: its index in [`Segments::all`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SegId(pub u32);

/// What made a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// The whole page: the root.
    Page,
    /// A region the file states itself, with the kind the file gives it. It is one leaf.
    Given(Kind),
    /// A closed box of drawn lines that holds text.
    Box,
    /// A grid of drawn lines of at least two cells: a row or a column of cells (whose cells
    /// that hold text are [`Source::Box`] segments under it), or rows and columns. A grid
    /// over most of the page (a form drawn in its frame) is one too; whether a grid is a
    /// table, a frame or a row of fields is the typers' decision.
    Grid { rows: u32, cols: u32 },
    /// A group of text that a column gutter of the whitespace cut parts off.
    Gutter,
    /// A group of text that a band of white space across parts off.
    Band,
    /// A group of text that a drawn rule across a gutter parts off.
    Rule,
    /// A group of text of one style (size, weight, colour) inside a group of the cut.
    Style,
    /// The text of a segment beside its other parts (drawn children, given regions, its
    /// drawing): what the whitespace cut and the style steps part further.
    Rest,
    /// Drawn primitives of a segment that no drawn child holds: its rules and boxes together,
    /// or one picture, or one code. It is one leaf.
    Drawing,
    /// The text between two drawn rules one over the other, where both reach (a strip of
    /// fields, a band of a table's rows under its header): it is kept together before the
    /// whitespace cut, which would part its columns (see [`strips`]). Style steps part it.
    Strip,
    /// A row of labels over the row of their values, as the whitespace cut found them (see
    /// `cut::Cut::cells`): each label with the value under it, or a label alone.
    Cell,
    /// Lines of a text leaf that the text walk reads as one kind (a heading, paragraphs, list
    /// items), where the walk reads the leaf's lines as more than one kind (see
    /// [`crate::typers::walk_cut`]).
    Walk,
    /// A part of a data line that a small colon label is set into (see
    /// `Builder::labels_before_values`): the label with the text after it on that line, or
    /// what stands before the label. One line of text, kept as it is laid out. No typer
    /// reads it as a label and its value. It is one leaf.
    Line,
}

/// A candidate region: the primitives it holds and what parted it from the rest.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub id: SegId,
    pub bounds: Bounds,
    /// Every primitive of the segment, its children's included, sorted.
    pub prims: Vec<PrimId>,
    pub source: Source,
    pub parent: Option<SegId>,
    /// The main cut of the segment. Under the root the given regions come first. Then the
    /// drawn children (top to bottom, then left to right), the text and the drawing. For
    /// text the children are the groups of the cut in reading order, or of style steps.
    pub children: Vec<SegId>,
    /// Other cuts of the same primitives, each a list of segments whose leaves hold every
    /// primitive of this segment once. Their segments have this segment as parent but are
    /// not among its children.
    pub alternatives: Vec<Vec<SegId>>,
}

/// The segment tree of one page.
#[derive(Debug, Clone, PartialEq)]
pub struct Segments {
    /// Every segment, the root first; [`SegId`] is the index.
    pub all: Vec<Segment>,
    pub root: SegId,
    /// What a limit or the stop callback cut short. Nothing is dropped then: the rest is one
    /// leaf that holds every primitive of it.
    pub issues: Vec<Issue>,
    /// The segments a limit or the stop callback left one leaf where they would have been
    /// parted further (see the bounds of [`segment_page`]), ascending: their kind was not
    /// looked for in full.
    pub short: Vec<SegId>,
}

impl Segments {
    /// For each of `runs` text runs of the page (by its id), whether it stands in a segment
    /// of [`Segments::short`].
    pub fn short_texts(&self, runs: usize) -> Vec<bool> {
        let mut short = vec![false; runs];
        for s in &self.short {
            let Some(seg) = self.all.get(s.0 as usize) else {
                continue;
            };
            work::add(seg.prims.len() as u64);
            for p in &seg.prims {
                if let PrimId::Text(i) = *p
                    && let Some(slot) = short.get_mut(i as usize)
                {
                    *slot = true;
                }
            }
        }
        short
    }
}

/// The segment tree of `page`, in this order of proposal:
///
/// 1. Given regions, in their `order`: each one leaf with [`Source::Given`]; a primitive two
///    of them name stays with the first, a primitive the page does not have is skipped.
/// 2. The drawn structure ([`drawn::drawn`]): grids of at least two cells and closed boxes
///    that hold text, nested where one holds another; rectangles drawn side by side that
///    share their sides are the cells of one grid, as their lines meet. The cells of a grid
///    of one row or one column that hold text are its boxes and nest under it. A grid over
///    more than [`drawn::PAGE_SHARE`] of the page is a segment too; a box that large is the
///    page's frame and no segment (see [`drawn::drawn`]). Text belongs to the innermost one
///    that holds its centre, any other primitive to the innermost one that holds it whole
///    (within [`SNAP`]).
/// 3. The whitespace cut ([`cut::xy_cut`]) of the text of each segment, with its own
///    horizontal rules: each group a segment of [`Source::Gutter`], [`Source::Band`] or
///    [`Source::Rule`] (the split before it; the first group takes the split after it).
/// 4. Style steps ([`style::style_steps`]) inside each group of the cut.
///
/// Where a segment has drawn children, the whitespace cut of all its text with the drawn
/// structure left out is a second cut of it when one of its column gutters holds a vertical
/// side of a drawn child beside its text (a gutter that a box also bounds): that cut is kept
/// in `alternatives`. Its groups are leaves; the given regions and the drawing leaves complete
/// it. At most [`Limits::max_alternatives`] are kept per segment.
///
/// Bounds: no segment nests deeper than [`Limits::max_cut_depth`] below the root. A segment
/// at the bound that would be parted further is one leaf that holds every primitive of it,
/// and an [`Issue`] of kind [`IssueKind::Limit`] says so. A whitespace cut that reaches its
/// own depth bound keeps the rest of its region as one group ordered by line and then by x,
/// not by column ([`cut::CutShort::Depth`]); that is an `Issue` of kind `Limit` too. `stop`
/// is asked once per segment that would be parted and passed to the cut; once it says stop,
/// every segment not yet parted is one leaf and an `Issue` of kind [`IssueKind::Stopped`]
/// says so. A grid search that was not done, or boxes that were not used, are `Limit` issues
/// with the reason. A page makes at most [`Limits::max_regions`] segments: a segment whose
/// parts would pass that stays one leaf, and an `Issue` of kind `Limit` says so. Each kind of
/// issue is said once per page.
///
/// Every primitive of the page is in exactly one leaf of the main cut, and every primitive of
/// a segment in exactly one leaf of each of its alternatives.
///
/// ```
/// use layoutregions::segment::{Source, segment_page};
/// use layoutregions::{Limits, PageIn};
///
/// let page = PageIn {
///     page: 1,
///     size: [595.0, 842.0],
///     turn: 0,
///     texts: Vec::new(),
///     rules: Vec::new(),
///     boxes: Vec::new(),
///     pictures: Vec::new(),
///     codes: Vec::new(),
///     given: Vec::new(),
/// };
/// let s = segment_page(&page, &Limits::default(), &|| false);
/// assert_eq!(s.all[s.root.0 as usize].source, Source::Page);
/// assert!(s.issues.is_empty());
/// ```
pub fn segment_page(page: &PageIn, limits: &Limits, stop: &dyn Fn() -> bool) -> Segments {
    segment_with(page, None, None, limits, stop).0
}

/// [`segment_page`] on the grids the host found on the page's rules ([`drawn::grids`] of
/// `page.rules`, so a page's lines are searched once), with the drawn structure it found from
/// them (its boxes from the text the given regions leave), so a host can type the segments
/// with the structure they were cut by; `None` when a bound or the stop came before the
/// search.
pub fn segment_page_drawn(
    page: &PageIn,
    grids: &Result<drawn::Grids, drawn::Unsearched>,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> (Segments, Option<drawn::DrawnStructure>) {
    segment_with(page, Some(grids), None, limits, stop)
}

/// [`segment_page`] on the drawn structure a host found itself, in the frame of `page`: its
/// grids and the boxes its own test found text in (a host that reads text into boxes keeps
/// the boxes it read). The host says what its own search left out: a grid search that was not
/// done, or boxes that were not used, are no issue here.
pub fn segment_drawn(
    page: &PageIn,
    found: &drawn::DrawnStructure,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> Segments {
    segment_with(page, None, Some(found), limits, stop).0
}

/// The segments of `page` and the drawn structure found for them when the host gave none.
fn segment_with(
    page: &PageIn,
    grids: Option<&Result<drawn::Grids, drawn::Unsearched>>,
    found: Option<&drawn::DrawnStructure>,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> (Segments, Option<drawn::DrawnStructure>) {
    let mut b = Builder {
        page,
        grids,
        limits,
        stop,
        max: limits.max_cut_depth as usize,
        all: Vec::new(),
        issues: Vec::new(),
        noted: [false; 5],
        stopped: false,
        given: Vec::new(),
        short: Vec::new(),
        joined: Vec::new(),
        joints: Vec::new(),
    };
    (b.joined, b.joints) = page_joints(page);
    // By height, once: each cut takes the stretch of its own heights (see `cut_runs`). The
    // cut sorts what it gets the same way and keeps the order of equal heights, so it sees
    // the rules in the order it saw them in before.
    work::add(b.joined.len() as u64);
    b.joined.sort_by(|a, c| a.at.total_cmp(&c.at));
    let every = all_prims(page);
    work::add(every.len() as u64);
    let frame = Bounds {
        x0: 0.0,
        y0: 0.0,
        x1: page.size[0],
        y1: page.size[1],
    };
    let root = b.push(Source::Page, every, Some(frame), None, false);
    let own = b.segment(root, found);
    b.labels_by_fields();
    b.values_beside_pairs();
    b.labels_before_values();
    let mut short = b.short;
    short.sort_unstable();
    short.dedup();
    let out = Segments {
        all: b.all,
        root,
        issues: b.issues,
        short,
    };
    debug_assert_eq!(cover_problem(&out, page), None);
    (out, own)
}

/// The issues a page gets at most once each.
#[derive(Clone, Copy)]
enum Note {
    Depth,
    CutDepth,
    Alternatives,
    Stopped,
    Segments,
}

struct Builder<'a> {
    page: &'a PageIn,
    /// The grids of the page's rules when the host found them (see [`segment_page_drawn`]).
    grids: Option<&'a Result<drawn::Grids, drawn::Unsearched>>,
    limits: &'a Limits,
    stop: &'a dyn Fn() -> bool,
    max: usize,
    all: Vec<Segment>,
    issues: Vec<Issue>,
    noted: [bool; 5],
    stopped: bool,
    /// The segments of the given regions.
    given: Vec<SegId>,
    /// The segments a limit or the stop left one leaf (see [`Segments::short`]).
    short: Vec<SegId>,
    /// The page's rules drawn in pieces, joined, and their joints (see [`cut::joined`]): made
    /// once for the page, for every cut of its text. The rules are sorted by height.
    joined: Vec<Line>,
    joints: Vec<(f32, f32)>,
}

/// A grid or box of the page, or the page itself (item 0).
struct Item {
    bounds: Bounds,
    source: Source,
    depth: usize,
    /// Items it holds directly, top to bottom, then left to right.
    children: Vec<usize>,
    /// Text runs it holds and none of its children does.
    texts: Vec<u32>,
    /// Other primitives it holds and none of its children does.
    others: Vec<PrimId>,
    /// The item it nests in (itself for the page).
    parent: usize,
    /// For an item one level past the depth bound: whether it holds a primitive (which its
    /// parent at the bound keeps). An item the given regions took holds none.
    held: bool,
}

impl Builder<'_> {
    fn push(
        &mut self,
        source: Source,
        prims: Vec<PrimId>,
        bounds: Option<Bounds>,
        parent: Option<SegId>,
        child: bool,
    ) -> SegId {
        work::add(1 + prims.len() as u64);
        let id = SegId(self.all.len() as u32);
        let bounds = bounds.unwrap_or_else(|| self.extent(&prims));
        self.all.push(Segment {
            id,
            bounds,
            prims,
            source,
            parent,
            children: Vec::new(),
            alternatives: Vec::new(),
        });
        if child && let Some(p) = parent {
            self.all[p.0 as usize].children.push(id);
        }
        id
    }

    /// Parts a leaf of label lines whose values stand in drawn fields beside it into its
    /// lines. A print form sets a label over the upper left of its field: the boxes of the
    /// digits of an amount, lower and to the right. The fields are drawn structure and are
    /// taken out of the text before it is cut, so the labels of two such fields under each
    /// other are left as one block of two lines, and the text of each field stands between
    /// those lines. Where the labels and the fields are rows of a label and its value (see
    /// [`cut::labels_beside_values`]: one label and one field a row, two rows or more set
    /// alike, each field a value), each label is a segment of its own ([`Source::Band`]), to
    /// be read with its field.
    fn labels_by_fields(&mut self) {
        let page = self.page;
        let texts = |prims: &[PrimId]| -> Option<Vec<u32>> {
            prims
                .iter()
                .map(|p| match *p {
                    PrimId::Text(t) => Some(t),
                    _ => None,
                })
                .collect()
        };
        // The fields: the drawn grids of one row that hold text, by their baseline.
        let mut fields: Vec<(f32, Vec<u32>)> = Vec::new();
        for seg in &self.all {
            if !matches!(seg.source, Source::Grid { rows: 1, .. }) {
                continue;
            }
            work::add(seg.prims.len() as u64);
            let runs: Vec<u32> = seg
                .prims
                .iter()
                .filter_map(|p| match *p {
                    PrimId::Text(t) => Some(t),
                    _ => None,
                })
                .collect();
            if let Some(&first) = runs.first() {
                fields.push((page.texts[first as usize].y, runs));
            }
        }
        if fields.len() < 2 {
            return;
        }
        fields.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut parted: Vec<(SegId, Vec<u32>)> = Vec::new();
        for seg in &self.all {
            let leaf = seg.children.is_empty()
                && seg.alternatives.is_empty()
                && matches!(
                    seg.source,
                    Source::Gutter | Source::Band | Source::Rest | Source::Style
                )
                && (2..=fields.len()).contains(&seg.prims.len());
            if !leaf {
                continue;
            }
            let Some(labels) = texts(&seg.prims) else {
                continue;
            };
            work::add(labels.len() as u64);
            let ys = || labels.iter().map(|&t| page.texts[t as usize].y);
            let (low, high) = (ys().fold(f32::MAX, f32::min), ys().fold(f32::MIN, f32::max));
            // The fields from one step of the labels under the lowest label up to the
            // highest one, right of the labels.
            let step = (high - low) / (labels.len() - 1) as f32;
            let right = seg.bounds.x1;
            let from = fields.partition_point(|f| f.0 <= low - step);
            let mut pieces: Vec<Piece> = labels
                .iter()
                .map(|&t| piece(&page.texts[t as usize]))
                .collect();
            let left: Vec<usize> = (0..pieces.len()).collect();
            for f in fields[from..].iter().take_while(|f| f.0 < high) {
                work::add(f.1.len() as u64);
                if f.1.iter().all(|&t| page.texts[t as usize].x >= right) {
                    pieces.extend(f.1.iter().map(|&t| piece(&page.texts[t as usize])));
                }
            }
            let values: Vec<usize> = (left.len()..pieces.len()).collect();
            if cut::labels_beside_values(&pieces, &left, &values) {
                parted.push((seg.id, labels));
            }
        }
        for (s, labels) in parted {
            for t in labels {
                self.push(Source::Band, vec![PrimId::Text(t)], None, Some(s), true);
            }
        }
    }

    /// Parts a leaf of one line that is a value and, a cell gap right of it, a label with
    /// its own value into the two: a print run sets a date, a wide blank and `Sheet:  4` in
    /// one line. The first run reads as an amount or a date ([`cut::reads_as_value`]), so
    /// it is no label of anything, and the second run alone reads as a short label, its
    /// colon and its value ([`crate::typers::pairs::line_pair_cut`]). Each is a segment of
    /// its own ([`Source::Band`]): the label is read with its value, the first run for
    /// itself. A leaf of any other two runs stays whole.
    fn values_beside_pairs(&mut self) {
        let page = self.page;
        let mut parted: Vec<(SegId, [u32; 2])> = Vec::new();
        for seg in &self.all {
            let leaf = seg.children.is_empty()
                && seg.alternatives.is_empty()
                && matches!(
                    seg.source,
                    Source::Gutter | Source::Band | Source::Rest | Source::Style
                );
            let [PrimId::Text(a), PrimId::Text(b)] = seg.prims[..] else {
                continue;
            };
            let (Some(ra), Some(rb)) = (page.texts.get(a as usize), page.texts.get(b as usize))
            else {
                continue;
            };
            work::add(2);
            // Left to right, as they stand.
            let ((a, ra), (b, rb)) = if ra.x <= rb.x {
                ((a, ra), (b, rb))
            } else {
                ((b, rb), (a, ra))
            };
            let size = ra.size.max(rb.size);
            let one_line = (ra.y - rb.y).abs() <= cut::BASELINE_NEAR * size;
            let apart = rb.x - (ra.x + ra.w) >= crate::extract::table::CELL_GAP * size.max(1.0);
            if leaf
                && one_line
                && apart
                && !ra.off_axis
                && !rb.off_axis
                && cut::reads_as_value(ra.text.trim())
                && crate::typers::pairs::line_pair_cut(std::slice::from_ref(rb), true).is_some()
            {
                parted.push((seg.id, [a, b]));
            }
        }
        for (s, runs) in parted {
            for t in runs {
                self.push(Source::Band, vec![PrimId::Text(t)], None, Some(s), true);
            }
        }
    }

    /// Parts a leaf at a small label that ends with a colon and is set into a data line right
    /// before the last text of that line: a form sets `Sheet:` in its small label face into
    /// the blank of a data line, before the number the print run puts there at the end of
    /// the line, and the same line holds a title and a date. The label and the text after
    /// it are a segment of their own, what stands before them on the line another one
    /// (both [`Source::Line`]): the date is no part of `Sheet: 1`, and no value of the
    /// title before it.
    ///
    /// The rule parts the line and claims no pair. The line `Sheet: 1` is kept as one
    /// plain line in reading order, the label right before the text after it: whether that
    /// text is the value of the label, the layout of one line does not say (the value may
    /// stand under the label, or the text may be the cell of a column), and a plain line
    /// states nothing that can be wrong.
    ///
    /// A line is looked at stretch by stretch: runs in one size (no size step,
    /// [`cut::SIZE_STEP`], between them) that follow each other within [`PIECE_GAP`] sizes
    /// are one stretch, however the producer cut its words into runs. The line is parted
    /// when, by where the text stands and never by the order of the runs:
    ///
    /// - the stretch before the last one ends with a colon and is set a size step smaller
    ///   than the last one, on one baseline with it (within [`cut::BASELINE_NEAR`] of the
    ///   size of the larger face);
    /// - the label is set into a data line: a stretch of the larger face stands before it
    ///   on that baseline, farther from it than the text after it is.
    ///
    /// A line is the runs of one line number of the host. A leaf may hold more lines (a
    /// line of codes over the data line, which the cut keeps with it): each line is looked
    /// at for itself, and the lines that are not parted stay together, those before the
    /// line a segment and those after it. Among other lines the text after the label must
    /// read as an amount, a date or a number ([`cut::reads_as_value`]), what a print run
    /// fills in: a block of lines may be running text, which is not parted at a small word
    /// with a colon.
    ///
    /// A row of a table or of a grid is not parted (Task 26 fix round 6): parted, the row
    /// would be two lines and its label would stand before the cell of another column. A
    /// line is such a row when the line right over it or right under it in the leaf shares
    /// its columns (see [`shares_columns`]); it stays whole, with the lines around it.
    fn labels_before_values(&mut self) {
        let page = self.page;
        // The new segments, in their order: the leaf each is a part of, what it is and
        // its runs.
        let mut parted: Vec<(SegId, Source, Vec<u32>)> = Vec::new();
        for seg in &self.all {
            let leaf = seg.children.is_empty()
                && seg.alternatives.is_empty()
                && matches!(
                    seg.source,
                    Source::Gutter | Source::Band | Source::Rest | Source::Style
                )
                && seg.prims.len() >= 3;
            if !leaf {
                continue;
            }
            let runs: Option<Vec<(u32, &TextRun)>> = seg
                .prims
                .iter()
                .map(|p| match *p {
                    PrimId::Text(t) => page.texts.get(t as usize).map(|r| (t, r)),
                    _ => None,
                })
                .collect();
            let Some(runs) = runs else {
                continue;
            };
            work::add(3 * runs.len() as u64);
            // Runs with ink along the page.
            if runs
                .iter()
                .any(|(_, r)| r.off_axis || r.text.trim().is_empty())
            {
                continue;
            }
            // The lines of the leaf, as the host numbers them, in the order of the leaf: a
            // title beside a data line stands a little higher than the label does, and is
            // on the line all the same.
            // Each line with the place of its first run in the leaf.
            let mut lines: Vec<(usize, Vec<(u32, &TextRun)>)> = Vec::new();
            let mut by_line: Vec<usize> = (0..runs.len()).collect();
            by_line.sort_by_key(|&k| (runs[k].1.line, k));
            for &k in &by_line {
                match lines.last_mut() {
                    Some((_, l)) if l[0].1.line == runs[k].1.line => l.push(runs[k]),
                    _ => lines.push((k, vec![runs[k]])),
                }
            }
            lines.sort_by_key(|l| l.0);
            let alone = lines.len() == 1;
            let mut groups: Vec<(Source, Vec<u32>)> = Vec::new();
            // The lines that are not parted so far: one segment.
            let mut kept: Vec<u32> = Vec::new();
            for (_, line) in &mut lines {
                line.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
            }
            // The lines from the top of the leaf down, and the place of each among them.
            let mut down: Vec<usize> = (0..lines.len()).collect();
            down.sort_by(|&a, &b| lines[b].1[0].1.y.total_cmp(&lines[a].1[0].1.y));
            let mut place = vec![0usize; lines.len()];
            for (p, &l) in down.iter().enumerate() {
                place[l] = p;
            }
            for (k, (_, line)) in lines.iter().enumerate() {
                let parts = label_parts(line, alone).filter(|_| {
                    // A row among the rows of a table stays whole.
                    let beside = [place[k].checked_sub(1), Some(place[k] + 1)];
                    !beside
                        .into_iter()
                        .flatten()
                        .filter_map(|p| down.get(p))
                        .any(|&o| shares_columns(line, &lines[o].1))
                });
                let Some([before, labelled]) = parts else {
                    kept.extend(line.iter().map(|r| r.0));
                    continue;
                };
                if !kept.is_empty() {
                    groups.push((Source::Band, std::mem::take(&mut kept)));
                }
                // The head of the line claims no pair either: its last stretch (a date after
                // a title or a code) is no value of the stretch before it.
                groups.push((Source::Line, before));
                groups.push((Source::Line, labelled));
            }
            // No line was parted: the leaf stays as it is.
            if groups.is_empty() {
                continue;
            }
            if !kept.is_empty() {
                groups.push((Source::Band, kept));
            }
            parted.extend(groups.into_iter().map(|g| (seg.id, g.0, g.1)));
        }
        for (s, source, runs) in parted {
            let prims = runs.into_iter().map(PrimId::Text).collect();
            self.push(source, prims, None, Some(s), true);
        }
    }

    fn extent(&self, prims: &[PrimId]) -> Bounds {
        prims
            .iter()
            .filter_map(|&p| prim_bounds(self.page, p))
            .reduce(union)
            .unwrap_or(Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: 0.0,
                y1: 0.0,
            })
    }

    fn note(&mut self, n: Note) {
        let k = n as usize;
        if self.noted[k] {
            return;
        }
        self.noted[k] = true;
        let max = self.limits.max_cut_depth;
        let (what, kind) = match n {
            Note::Depth => (
                format!(
                    "segments nest deeper than {max} levels: each segment at the bound is one \
                     leaf that holds every primitive of it"
                ),
                IssueKind::Limit,
            ),
            Note::CutDepth => (
                format!(
                    "the whitespace cut reached its depth bound of {max} levels: the rest of \
                     its region stays one group, ordered by line and then by x, not by column"
                ),
                IssueKind::Limit,
            ),
            Note::Alternatives => (
                format!(
                    "a segment has more than {} other cuts: the rest of them are not kept",
                    self.limits.max_alternatives
                ),
                IssueKind::Limit,
            ),
            Note::Segments => (
                format!(
                    "the page would make more than {} segments: each segment not yet parted \
                     is one leaf that holds every primitive of it",
                    self.limits.max_regions
                ),
                IssueKind::Limit,
            ),
            Note::Stopped => (
                "segmentation was stopped: each segment not yet parted is one leaf that holds \
                 every primitive of it"
                    .to_string(),
                IssueKind::Stopped,
            ),
        };
        self.issues.push(Issue { what, kind });
    }

    /// Whether `n` more segments fit under [`Limits::max_regions`]; when they do not, the
    /// page says so once.
    fn room(&mut self, n: usize) -> bool {
        if n == 0 || self.all.len() + n <= self.limits.max_regions {
            return true;
        }
        self.note(Note::Segments);
        false
    }

    /// Whether the work stops here: the stop callback says so now, or said so before.
    fn halted(&mut self) -> bool {
        if !self.stopped && (self.stop)() {
            self.stopped = true;
        }
        if self.stopped {
            self.note(Note::Stopped);
        }
        self.stopped
    }

    /// Parts the root: given regions, then the drawn structure, the cut and the style steps.
    /// Segments the page under `root`; the drawn structure it found itself when `found` is
    /// `None` and the search ran.
    fn segment(
        &mut self,
        root: SegId,
        found: Option<&drawn::DrawnStructure>,
    ) -> Option<drawn::DrawnStructure> {
        let page = self.page;
        if self.halted() {
            self.short.push(root);
            return None;
        }
        // Given regions first; their primitives are taken out of the rest.
        let mut taken = Taken::new(page);
        let mut given: Vec<&GivenRegion> = page.given.iter().collect();
        given.sort_by_key(|g| g.order);
        let mut regions: Vec<(Kind, Vec<PrimId>)> = Vec::new();
        for g in given {
            work::add(1 + g.prims.len() as u64);
            let mut prims: Vec<PrimId> =
                g.prims.iter().copied().filter(|&p| taken.take(p)).collect();
            prims.sort_unstable();
            if !prims.is_empty() {
                regions.push((g.kind, prims));
            }
        }
        // The drawn structure the host found, or from the text the given regions leave: a box
        // holds text when the centre of a run lies in it.
        let mut own = None;
        let found = match found {
            Some(found) => found,
            None => own.insert(self.drawn(&taken)),
        };
        let mut items = self.items(found);
        self.assign(&mut items, &taken);
        // Every item's primitives, its children's included; then the segments, top down. The
        // root makes its given regions and its own parts at once, or none.
        let mut sub = subtree_prims(&items, self.max);
        if self.max == 0 {
            let parts = regions.len() + own_parts(&items, 0, &sub, regions.len());
            let text = !regions.is_empty() || self.text_apart(&items, 0, &sub);
            if text || parts > 0 {
                self.note(Note::Depth);
            }
            if text {
                self.short.push(root);
            }
            return own;
        }
        if !self.room(regions.len() + own_parts(&items, 0, &sub, regions.len())) {
            if !regions.is_empty() || self.text_apart(&items, 0, &sub) {
                self.short.push(root);
            }
            return own;
        }
        for (kind, prims) in regions {
            let id = self.push(Source::Given(kind), prims, None, Some(root), true);
            self.given.push(id);
        }
        let mut stack: Vec<(usize, SegId)> = vec![(0, root)];
        while let Some((i, s)) = stack.pop() {
            let next = self.item(&items, i, s, &mut sub);
            stack.extend(next.into_iter().rev());
        }
        own
    }

    /// The drawn structure of the page, from the text the given regions leave (`taken`): a box
    /// holds text when the centre of a run lies in it. What the search left out is an issue.
    fn drawn(&mut self, taken: &Taken) -> drawn::DrawnStructure {
        let page = self.page;
        let mut centres: Vec<(f32, f32)> = (0..page.texts.len())
            .filter(|&i| !taken.has(PrimId::Text(i as u32)))
            .map(|i| centre(text_bounds(&page.texts[i])))
            .collect();
        centres.sort_by(|a, b| a.0.total_cmp(&b.0));
        work::add(centres.len() as u64);
        let inked = |b: Bounds| {
            let lo = centres.partition_point(|c| c.0 < b.x0);
            let mut steps = 0u64;
            let mut found = false;
            for c in &centres[lo..] {
                steps += 1;
                if c.0 > b.x1 {
                    break;
                }
                if c.1 >= b.y0 && c.1 <= b.y1 {
                    found = true;
                    break;
                }
            }
            work::add(steps);
            found
        };
        let found = match self.grids {
            Some(grids) => {
                if let Ok(g) = grids {
                    work::add((g.hs.len() + g.vs.len()) as u64);
                }
                drawn::drawn_on(page, grids.clone(), &inked)
            }
            None => drawn::drawn(page, &inked),
        };
        match (&found.grids, &found.boxes_note) {
            (Err(why), _) => self.issues.push(Issue {
                what: format!("grids and boxes were not searched: {}", why.why()),
                kind: IssueKind::Limit,
            }),
            (Ok(_), Some(note)) => self.issues.push(Issue {
                what: format!("drawn boxes are not used: {note}"),
                kind: IssueKind::Limit,
            }),
            _ => {}
        }
        found
    }

    /// The grids and boxes of the page as items, nested; item 0 is the page.
    fn items(&self, found: &drawn::DrawnStructure) -> Vec<Item> {
        let page = self.page;
        let mut drawn: Vec<(Bounds, Source)> = Vec::new();
        if let Ok(g) = &found.grids {
            for group in &g.groups {
                work::add((group.0.len() + group.1.len()) as u64);
                // One cell is a box (see `drawn::boxes`), not a grid. The cells of a grid of
                // one row or one column are boxes too (`drawn::boxes` skips only grids of two
                // rows and two columns or more), so they nest under this grid.
                let Some((b, rows, cols)) = drawn::grid_frame(g, group) else {
                    continue;
                };
                let source = Source::Grid { rows, cols };
                drawn.push((b, source));
            }
        }
        drawn.extend(found.boxes.iter().map(|&b| (b, Source::Box)));
        drawn.sort_by(|a, b| nesting_order(&a.0, &b.0));
        let mut items = vec![Item {
            bounds: Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: page.size[0],
                y1: page.size[1],
            },
            source: Source::Page,
            depth: 0,
            children: Vec::new(),
            texts: Vec::new(),
            others: Vec::new(),
            parent: 0,
            held: false,
        }];
        let all: Vec<Bounds> = drawn.iter().map(|d| d.0).collect();
        let holders = holders(&all);
        for ((bounds, source), node) in drawn.into_iter().zip(holders) {
            let at = items.len();
            let depth = items[node].depth + 1;
            items[node].children.push(at);
            items.push(Item {
                bounds,
                source,
                depth,
                children: Vec::new(),
                texts: Vec::new(),
                others: Vec::new(),
                parent: node,
                held: false,
            });
        }
        for i in 0..items.len() {
            let mut kids = std::mem::take(&mut items[i].children);
            kids.sort_by(|&a, &b| {
                items[b]
                    .bounds
                    .y1
                    .total_cmp(&items[a].bounds.y1)
                    .then(items[a].bounds.x0.total_cmp(&items[b].bounds.x0))
            });
            items[i].children = kids;
        }
        items
    }

    /// Gives each primitive the given regions leave to the innermost item that holds it, down
    /// to the depth bound: an item at the bound keeps what the items in it hold. The search
    /// goes one level past the bound, to tell which items there hold anything.
    fn assign(&self, items: &mut [Item], taken: &Taken) {
        let page = self.page;
        let index: Vec<Option<Index>> = items
            .iter()
            .map(|it| Index::new(items, &it.children))
            .collect();
        let place = |bounds: Bounds, text: bool| -> usize {
            let point = centre(bounds);
            let fits = |c: usize| {
                work::add(1);
                if text {
                    holds_point(items[c].bounds, point)
                } else {
                    holds(items[c].bounds, bounds)
                }
            };
            let mut node = 0;
            while items[node].depth <= self.max {
                let kids = &items[node].children;
                let inner = match &index[node] {
                    Some(ix) => ix
                        .at(point)
                        .iter()
                        .map(|&k| kids[k as usize])
                        .find(|&c| fits(c)),
                    None => kids.iter().copied().find(|&c| fits(c)),
                };
                match inner {
                    Some(c) => node = c,
                    None => break,
                }
            }
            node
        };
        let mut texts: Vec<(usize, u32)> = Vec::new();
        let mut others: Vec<(usize, PrimId)> = Vec::new();
        let every = all_prims(page);
        work::add(every.len() as u64);
        for id in every {
            if taken.has(id) {
                continue;
            }
            match id {
                PrimId::Text(t) => {
                    texts.push((place(text_bounds(&page.texts[t as usize]), true), t));
                }
                _ => {
                    let b = prim_bounds(page, id).expect("a primitive of the page");
                    others.push((place(b, false), id));
                }
            }
        }
        let keep = |items: &mut [Item], node: usize| {
            if items[node].depth > self.max {
                items[node].held = true;
                items[node].parent
            } else {
                node
            }
        };
        for (node, t) in texts {
            let at = keep(items, node);
            items[at].texts.push(t);
        }
        for (node, p) in others {
            let at = keep(items, node);
            items[at].others.push(p);
        }
    }

    /// Parts the segment `s` of item `i`; returns the segments of its drawn children, to be
    /// parted next.
    fn item(
        &mut self,
        items: &[Item],
        i: usize,
        s: SegId,
        sub: &mut [Vec<PrimId>],
    ) -> Vec<(usize, SegId)> {
        let it = &items[i];
        let d = it.depth;
        // The root was asked before its given regions were made.
        if i != 0 && self.halted() {
            self.short.push(s);
            return Vec::new();
        }
        let given = if i == 0 {
            self.given.clone()
        } else {
            Vec::new()
        };
        if d >= self.max {
            let text = self.text_apart(items, i, sub);
            if text || own_parts(items, i, sub, 0) > 0 {
                self.note(Note::Depth);
            }
            if text {
                self.short.push(s);
            }
            return Vec::new();
        }
        // The root's parts were reserved with its given regions in `segment`, in one step:
        // the root never returns here once they are made.
        if i != 0 && !self.room(own_parts(items, i, sub, 0)) {
            if self.text_apart(items, i, sub) {
                self.short.push(s);
            }
            return Vec::new();
        }
        // A grid or box whose primitives the given regions took is no segment.
        let mut next = Vec::with_capacity(it.children.len());
        for &c in &it.children {
            let prims = std::mem::take(&mut sub[c]);
            if prims.is_empty() {
                continue;
            }
            let id = self.push(items[c].source, prims, Some(items[c].bounds), Some(s), true);
            next.push((c, id));
        }
        let rules = rules_of(self.page, &it.others);
        let parted = !next.is_empty() || !given.is_empty();
        if parted
            || (!it.texts.is_empty() && !it.others.is_empty())
            || drawing_count(&it.others) > 1
        {
            // All parts are made before the text is parted, so that the room they took stays
            // theirs.
            let rest = (!it.texts.is_empty()).then(|| {
                let mut prims: Vec<PrimId> = it.texts.iter().map(|&t| PrimId::Text(t)).collect();
                prims.sort_unstable();
                self.push(Source::Rest, prims, None, Some(s), true)
            });
            for prims in drawing_parts(&it.others) {
                self.push(Source::Drawing, prims, None, Some(s), true);
            }
            if let Some(r) = rest {
                self.text(r, &it.texts, d + 1, &rules);
            }
        } else if !it.texts.is_empty() {
            self.text(s, &it.texts, d, &rules);
        }
        if !next.is_empty() {
            let kids: Vec<usize> = next.iter().map(|&(c, _)| c).collect();
            self.alternative(items, &kids, s, d, &given, &rules);
        }
        next
    }

    /// Whether the text of item `i` would be parted: some of it stands in its drawn children
    /// (`sub`, or `held` past the depth bound), or the strips between its rules, the cut or
    /// style steps would part it (see [`Builder::text_parts`]). A bound that leaves only
    /// drawings or pictures unparted leaves its text as it would read.
    fn text_apart(&mut self, items: &[Item], i: usize, sub: &[Vec<PrimId>]) -> bool {
        let it = &items[i];
        it.children.iter().any(|&c| {
            work::add(1 + sub[c].len() as u64);
            items[c].held || sub[c].iter().any(|p| matches!(p, PrimId::Text(_)))
        }) || {
            let rules = rules_of(self.page, &it.others);
            self.text_parts(&it.texts, &rules)
        }
    }

    /// Whether the text runs `ids` would be parted as [`Builder::text`] parts them: by the
    /// strips between `rules` first (see [`strips`]; one strip alone by style steps only), else
    /// by the cut or by style steps. A stop during the probe stops the page and says no.
    fn text_parts(&mut self, ids: &[u32], rules: &[Line]) -> bool {
        if ids.len() < 2 {
            return false;
        }
        if let Some((groups, rest)) = strips(self.page, ids, rules) {
            return match (groups.as_slice(), rest.is_empty()) {
                ([one], true) => style_steps(&self.page.texts, one).len() > 1,
                _ => true,
            };
        }
        let bound = Limits {
            max_cut_depth: 0,
            ..*self.limits
        };
        let none = cut::Ruled {
            rules: &[],
            joined: &[],
            joints: &[],
        };
        let found = cut_runs(&self.page.texts, ids, &none, &bound, self.stop).0;
        if found.short == Some(CutShort::Stopped) {
            self.cut_short(found.short);
            return false;
        }
        if found.groups.len() > 1 || found.short.is_some() {
            return true;
        }
        let order: Vec<u32> = found.groups.concat().iter().map(|&k| ids[k]).collect();
        style_steps(&self.page.texts, &order).len() > 1
    }

    /// Parts the text runs `ids` of segment `s` (at depth `d`): the strips between rules first
    /// (see [`strips`]), each parted by style steps only, and the text outside them by the
    /// whitespace cut, then each group by style steps.
    fn text(&mut self, s: SegId, ids: &[u32], d: usize, rules: &[Line]) {
        if ids.len() < 2 {
            return;
        }
        if self.halted() {
            self.short.push(s);
            return;
        }
        work::add(ids.len() as u64);
        if let Some((groups, rest)) = strips(self.page, ids, rules) {
            // The segment is one strip: it is parted by style steps only.
            if groups.len() == 1 && rest.is_empty() {
                self.style(s, &groups[0], d);
                return;
            }
            if d >= self.max {
                self.note(Note::Depth);
                self.short.push(s);
                return;
            }
            let parts = groups.len() + usize::from(!rest.is_empty());
            if !self.room(parts) {
                self.short.push(s);
                return;
            }
            let mut made = Vec::with_capacity(parts);
            for g in groups {
                let mut prims: Vec<PrimId> = g.iter().map(|&t| PrimId::Text(t)).collect();
                prims.sort_unstable();
                made.push((
                    self.push(Source::Strip, prims, None, Some(s), true),
                    g,
                    true,
                ));
            }
            if !rest.is_empty() {
                let mut prims: Vec<PrimId> = rest.iter().map(|&t| PrimId::Text(t)).collect();
                prims.sort_unstable();
                made.push((
                    self.push(Source::Rest, prims, None, Some(s), true),
                    rest,
                    false,
                ));
            }
            for (id, order, strip) in made {
                if strip {
                    self.style(id, &order, d + 1);
                } else {
                    self.text(id, &order, d + 1, rules);
                }
            }
            return;
        }
        let bound = Limits {
            max_cut_depth: (self.max - d) as u32,
            ..*self.limits
        };
        let ruled = cut::Ruled {
            rules,
            joined: &self.joined,
            joints: &self.joints,
        };
        let found = cut_runs(&self.page.texts, ids, &ruled, &bound, self.stop).0;
        self.cut_short(found.short);
        if found.groups.len() < 2 {
            if !found.kept.is_empty() {
                self.short.push(s);
            }
            let order: Vec<u32> = found.groups.concat().iter().map(|&k| ids[k]).collect();
            self.style(s, &order, d);
            return;
        }
        if d >= self.max {
            self.note(Note::Depth);
            self.short.push(s);
            return;
        }
        if !self.room(found.groups.len()) {
            self.short.push(s);
            return;
        }
        // Every group is made before any is parted, so that the room they took stays theirs.
        let mut made = Vec::with_capacity(found.groups.len());
        for (g, group) in found.groups.iter().enumerate() {
            let order: Vec<u32> = group.iter().map(|&k| ids[k]).collect();
            let mut prims: Vec<PrimId> = order.iter().map(|&t| PrimId::Text(t)).collect();
            prims.sort_unstable();
            let id = self.push(source_of(&found, g), prims, None, Some(s), true);
            if found.kept.binary_search(&g).is_ok() {
                self.short.push(id);
            }
            made.push((id, order));
        }
        for (id, order) in made {
            self.style(id, &order, d + 1);
        }
    }

    /// Turns a cut that stopped early into its issue.
    fn cut_short(&mut self, short: Option<CutShort>) {
        match short {
            Some(CutShort::Depth) => self.note(Note::CutDepth),
            Some(CutShort::Stopped) => {
                self.stopped = true;
                self.note(Note::Stopped);
            }
            None => {}
        }
    }

    /// Parts the text runs `order` (in reading order) of segment `s`, at depth `d`, by style
    /// steps.
    fn style(&mut self, s: SegId, order: &[u32], d: usize) {
        if order.len() < 2 {
            return;
        }
        if self.stopped {
            self.short.push(s);
            return;
        }
        work::add(order.len() as u64);
        let groups = style_steps(&self.page.texts, order);
        if groups.len() < 2 {
            return;
        }
        if d >= self.max {
            self.note(Note::Depth);
            self.short.push(s);
            return;
        }
        if !self.room(groups.len()) {
            self.short.push(s);
            return;
        }
        for g in groups {
            let mut prims: Vec<PrimId> = g.into_iter().map(PrimId::Text).collect();
            prims.sort_unstable();
            self.push(Source::Style, prims, None, Some(s), true);
        }
    }

    /// The whitespace cut of all text of segment `s` (at depth `d`) with its drawn children
    /// `kids` (items) left out, kept as an alternative when one of its column gutters holds a
    /// vertical side of a drawn child beside its text: the gutter that a box also bounds. Its
    /// groups are leaves; the given regions and the drawing leaves of the segment (see
    /// [`drawing_parts`]) complete it.
    fn alternative(
        &mut self,
        items: &[Item],
        kids: &[usize],
        s: SegId,
        d: usize,
        given: &[SegId],
        rules: &[Line],
    ) {
        if self.stopped {
            return;
        }
        let mut givens: Vec<PrimId> = given
            .iter()
            .flat_map(|g| self.all[g.0 as usize].prims.iter().copied())
            .collect();
        givens.sort_unstable();
        let free = |p: &PrimId| givens.binary_search(p).is_err();
        let prims = &self.all[s.0 as usize].prims;
        let ids: Vec<u32> = prims
            .iter()
            .filter(|p| free(p))
            .filter_map(|p| match p {
                PrimId::Text(t) => Some(*t),
                _ => None,
            })
            .collect();
        if ids.len() < 2 {
            return;
        }
        let drawing: Vec<PrimId> = prims
            .iter()
            .filter(|p| !matches!(p, PrimId::Text(_)) && free(p))
            .copied()
            .collect();
        work::add(ids.len() as u64);
        // A gutter can hold a side of a drawn child only when some text ends left of that side
        // and some starts right of it: without that, the cut is not made at all.
        let (mut ends, mut starts) = (f32::MAX, f32::MIN);
        for &t in &ids {
            let b = text_bounds(&self.page.texts[t as usize]);
            ends = ends.min(b.x1);
            starts = starts.max(b.x0);
        }
        let between = |x: f32| ends <= x + SNAP && starts >= x - SNAP;
        work::add(kids.len() as u64);
        if !kids
            .iter()
            .any(|&c| between(items[c].bounds.x0) || between(items[c].bounds.x1))
        {
            return;
        }
        let bound = Limits {
            max_cut_depth: (self.max - d) as u32,
            ..*self.limits
        };
        let ruled = cut::Ruled {
            rules,
            joined: &self.joined,
            joints: &self.joints,
        };
        let (found, extents) = cut_runs(&self.page.texts, &ids, &ruled, &bound, self.stop);
        if found.short == Some(CutShort::Stopped) {
            self.cut_short(found.short);
            return;
        }
        if found.groups.len() < 2 {
            return;
        }
        let extent = |g: usize| extents[g];
        let bounded = found.splits.iter().enumerate().any(|(j, split)| {
            if !matches!(split, Split::Gutter | Split::Boxed) {
                return false;
            }
            let (a, b) = (extent(j), extent(j + 1));
            let (g0, g1) = (a.x1, b.x0);
            if g1 <= g0 {
                return false;
            }
            let (lo, hi) = (a.y0.min(b.y0), a.y1.max(b.y1));
            let side = |x: f32| x >= g0 - SNAP && x <= g1 + SNAP;
            kids.iter().any(|&c| {
                let cb = items[c].bounds;
                (side(cb.x0) || side(cb.x1)) && cb.y0 < hi && cb.y1 > lo
            })
        });
        if !bounded {
            return;
        }
        self.cut_short(found.short);
        if self.all[s.0 as usize].alternatives.len() >= self.limits.max_alternatives {
            self.note(Note::Alternatives);
            return;
        }
        if !self.room(found.groups.len() + drawing_count(&drawing)) {
            return;
        }
        let mut alt: Vec<SegId> = given.to_vec();
        for (g, group) in found.groups.iter().enumerate() {
            let mut prims: Vec<PrimId> = group.iter().map(|&k| PrimId::Text(ids[k])).collect();
            prims.sort_unstable();
            let id = self.push(source_of(&found, g), prims, None, Some(s), false);
            if found.kept.binary_search(&g).is_ok() {
                self.short.push(id);
            }
            alt.push(id);
        }
        for prims in drawing_parts(&drawing) {
            alt.push(self.push(Source::Drawing, prims, None, Some(s), false));
        }
        self.all[s.0 as usize].alternatives.push(alt);
    }
}

/// The segments item `i` makes for its own parts, beside `given` given regions (the root's):
/// its drawn children that hold anything (see `sub`, or `held` past the depth bound), and when
/// it is parted, one segment for its text and its drawing leaves (see [`drawing_parts`]). It
/// is parted when it has drawn children or given regions, text and drawing, or more than one
/// drawing leaf.
fn own_parts(items: &[Item], i: usize, sub: &[Vec<PrimId>], given: usize) -> usize {
    let it = &items[i];
    let kids = it
        .children
        .iter()
        .filter(|&&c| !sub[c].is_empty() || items[c].held)
        .count();
    let drawings = drawing_count(&it.others);
    let split = kids > 0 || given > 0 || (!it.texts.is_empty() && drawings > 0) || drawings > 1;
    if !split {
        return 0;
    }
    kids + usize::from(!it.texts.is_empty()) + drawings
}

/// How many drawing leaves `others` (primitives other than text) make (see
/// [`drawing_parts`]).
fn drawing_count(others: &[PrimId]) -> usize {
    let rest = others
        .iter()
        .any(|p| matches!(p, PrimId::Rule(_) | PrimId::Box(_)));
    let own = others
        .iter()
        .filter(|p| matches!(p, PrimId::Picture(_) | PrimId::Code(_)))
        .count();
    own + usize::from(rest)
}

/// The drawing leaves of `others` (primitives other than text), each sorted: the rules and
/// boxes together first, then one leaf per picture and one per code, in the order of their
/// ids. A picture and a code are each a region of their own, as a reader shows each.
fn drawing_parts(others: &[PrimId]) -> Vec<Vec<PrimId>> {
    work::add(others.len() as u64);
    let mut rest: Vec<PrimId> = others
        .iter()
        .copied()
        .filter(|p| matches!(p, PrimId::Rule(_) | PrimId::Box(_)))
        .collect();
    rest.sort_unstable();
    let mut own: Vec<PrimId> = others
        .iter()
        .copied()
        .filter(|p| matches!(p, PrimId::Picture(_) | PrimId::Code(_)))
        .collect();
    own.sort_unstable();
    let mut out = Vec::with_capacity(own.len() + 1);
    if !rest.is_empty() {
        out.push(rest);
    }
    out.extend(own.into_iter().map(|p| vec![p]));
    out
}

/// The source of group `g` of the cut `found`: a cell of labels over values, else the split
/// before it; the first group takes the split after it.
fn source_of(found: &cut::Cut, g: usize) -> Source {
    if found.cells.binary_search(&g).is_ok() {
        return Source::Cell;
    }
    let splits = &found.splits;
    let split = if g == 0 {
        splits.first()
    } else {
        splits.get(g - 1)
    };
    match split {
        Some(Split::Band) => Source::Band,
        Some(Split::Rule) => Source::Rule,
        _ => Source::Gutter,
    }
}

/// Every item's primitives, its children's included, sorted; empty past the depth bound
/// (those items keep nothing: their primitives are in the item at the bound).
fn subtree_prims(items: &[Item], max: usize) -> Vec<Vec<PrimId>> {
    let mut sub: Vec<Vec<PrimId>> = vec![Vec::new(); items.len()];
    // Children always come after their parent: walk back.
    for i in (0..items.len()).rev() {
        let it = &items[i];
        if it.depth > max {
            continue;
        }
        let mut prims: Vec<PrimId> = it.texts.iter().map(|&t| PrimId::Text(t)).collect();
        prims.extend(it.others.iter().copied());
        for &c in &it.children {
            prims.extend(sub[c].iter().copied());
        }
        work::add(prims.len() as u64);
        prims.sort_unstable();
        sub[i] = prims;
    }
    sub
}

/// A piece reaches into a frame or under a strip when it reaches past the edge by more than
/// this share of its text size (the slack the cut gives a piece at a gutter: half a point at
/// 8 points).
const REACH: f32 = 1.0 / 16.0;

/// A strip holds at most this many lines of text: more between two rules is a section of a
/// page, not a strip of fields or a band of rows.
const STRIP_LINES: usize = 6;

/// The strips of the runs `ids` (in reading order) between the horizontal `rules`: rules at
/// one height (within [`SNAP`]) are one level, and a piece of one level over a piece of the
/// next level under it bounds a strip where both reach. A piece of a line (see [`pieces_of`])
/// stands in the first strip that holds its middle, with all its runs. A strip that another
/// line of text runs through from the side holds nothing. A strip with ink on two lines to [`STRIP_LINES`] lines is a group; the runs
/// of the other strips and outside every strip are the rest. Each group and the rest keep the
/// order of `ids`. `None` when no strip holds ink.
fn strips(page: &PageIn, ids: &[u32], rules: &[Line]) -> Option<(Vec<Vec<u32>>, Vec<u32>)> {
    if rules.len() < 2 {
        return None;
    }
    let mut sorted: Vec<&Line> = rules.iter().collect();
    sorted.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
    let mut levels: Vec<Vec<&Line>> = Vec::new();
    for l in sorted {
        match levels.last_mut() {
            Some(level) if l.at - level[0].at <= SNAP => level.push(l),
            _ => levels.push(vec![l]),
        }
    }
    // The frames (bottom, top, x0, x1), top to bottom, then left to right.
    let mut frames: Vec<(f32, f32, f32, f32)> = Vec::new();
    for w in levels.windows(2) {
        for low in &w[0] {
            for up in &w[1] {
                work::add(1);
                let (a, b) = (low.from.max(up.from), low.to.min(up.to));
                if b - a > SNAP {
                    frames.push((low.at, up.at, a, b));
                }
            }
        }
    }
    frames.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.total_cmp(&b.2)));
    // A piece of a line stands in a strip whole, by its middle: its runs go together, however
    // the line is cut into runs.
    let mut at: Vec<Option<usize>> = vec![None; ids.len()];
    let (pieces, members) = pieces_of(&page.texts, ids);
    for (p, m) in pieces.iter().zip(&members) {
        work::add(frames.len() as u64);
        let (cx, cy) = centre(p.bounds);
        let f = frames
            .iter()
            .position(|f| cy > f.0 && cy < f.1 && cx > f.2 && cx < f.3);
        for &k in m {
            at[k] = f;
        }
    }
    // A frame that a line of text runs through is no strip: a piece at the frame's height
    // that reaches into it from the side and has its middle outside (a full line of a
    // paragraph across a narrow frame) shows that the rules bound no band of this text. The
    // few short lines whose middles do lie inside are lines of those paragraphs. And the
    // lines of a strip follow each other: a line of other text between two of them (at a
    // height between theirs, under or over their extent) shows that the frame picks lines
    // out of a longer text, each of which goes on with its own neighbours. Only the frames
    // that hold a piece are looked at: each with its bounds and the extent of its pieces.
    let mut extent: Vec<Option<Bounds>> = vec![None; frames.len()];
    for (p, m) in pieces.iter().zip(&members) {
        if let (true, Some(k)) = (p.inked, at[m[0]]) {
            extent[k] = Some(extent[k].map_or(p.bounds, |e| union(e, p.bounds)));
        }
    }
    let held: Vec<(usize, Bounds)> = extent
        .iter()
        .enumerate()
        .filter_map(|(k, e)| e.map(|e| (k, e)))
        .collect();
    let mut crossed = vec![false; frames.len()];
    if !held.is_empty() {
        for (p, m) in pieces.iter().zip(&members) {
            if !p.inked {
                continue;
            }
            work::add(held.len() as u64);
            let (_, cy) = centre(p.bounds);
            let own = at[m[0]];
            for &(k, e) in &held {
                if own == Some(k) {
                    continue;
                }
                let f = frames[k];
                // A piece reaches in when it reaches past the edge by more than a small share
                // of its size (see [`REACH`]): ink that touches the start of a rule by a hair
                // runs through nothing.
                let reach = REACH * p.size;
                let through =
                    cy > f.0 && cy < f.1 && p.bounds.x0 < f.3 - reach && p.bounds.x1 > f.2 + reach;
                let between = cy > e.y0
                    && cy < e.y1
                    && p.bounds.x0 < e.x1 - reach
                    && p.bounds.x1 > e.x0 + reach;
                if through || between {
                    crossed[k] = true;
                }
            }
        }
    }
    for f in at.iter_mut() {
        if f.is_some_and(|k| crossed[k]) {
            *f = None;
        }
    }
    let mut groups: Vec<Vec<u32>> = vec![Vec::new(); frames.len()];
    let mut rest: Vec<u32> = Vec::new();
    for (&t, f) in ids.iter().zip(&at) {
        match f {
            Some(f) => groups[*f].push(t),
            None => rest.push(t),
        }
    }
    let mut kept = Vec::new();
    for g in groups {
        let mut lines: Vec<u32> = g
            .iter()
            .map(|&t| &page.texts[t as usize])
            .filter(|r| crate::line::ink(&r.text) > 0)
            .map(|r| r.line)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        // One line between two rules is a row (of a table with row rules, or of labels beside
        // values): the cut reads it with the lines around it.
        if lines.len() < 2 || lines.len() > STRIP_LINES {
            rest.extend(g);
        } else {
            kept.push(g);
        }
    }
    if kept.is_empty() {
        return None;
    }
    // The rest in the order of `ids`.
    let order: HashMap<u32, usize> = ids.iter().enumerate().map(|(k, &t)| (t, k)).collect();
    rest.sort_by_key(|t| order.get(t).copied().unwrap_or(usize::MAX));
    Some((kept, rest))
}

/// The horizontal rules among `prims`, as the strips and the cut take them; a rectangle gives
/// its top and bottom. Pieces of one rule that touch or overlap are one rule, joined as the
/// grid search joins them ([`drawn::merged`]): a rule drawn in pieces (cell by cell, or
/// dash by dash) bounds the same strips and parts the same bands as the rule drawn in one.
fn rules_of(page: &PageIn, prims: &[PrimId]) -> Vec<Line> {
    let mut out = Vec::new();
    for p in prims {
        let PrimId::Rule(r) = p else { continue };
        let r: &RuleIn = &page.rules[*r as usize];
        let (from, to) = (r.x0.min(r.x1), r.x0.max(r.x1));
        if r.rect {
            out.push(Line { at: r.y0, from, to });
            out.push(Line { at: r.y1, from, to });
        } else if (r.y0 - r.y1).abs() <= SNAP / 2.0 {
            out.push(Line {
                at: (r.y0 + r.y1) / 2.0,
                from,
                to,
            });
        }
    }
    work::add(out.len() as u64);
    drawn::merged(out)
}

/// The stretches of one line `runs` (its runs with ink, left to right), each the places of
/// its runs in `runs`: runs in one size (no size step, [`cut::SIZE_STEP`], between them) that
/// follow each other within [`PIECE_GAP`] sizes are one stretch.
fn stretches_of(runs: &[(u32, &TextRun)]) -> Vec<std::ops::Range<usize>> {
    let mut stretches: Vec<std::ops::Range<usize>> = Vec::new();
    for k in 0..runs.len() {
        let r = runs[k].1;
        let goes_on = stretches.last().is_some_and(|s| {
            let (head, last) = (runs[s.start].1, runs[k - 1].1);
            r.size < cut::SIZE_STEP * head.size
                && head.size < cut::SIZE_STEP * r.size
                && r.x - (last.x + last.w) <= PIECE_GAP * head.size.max(r.size)
        });
        match stretches.last_mut() {
            Some(s) if goes_on => s.end = k + 1,
            _ => stretches.push(k..k + 1),
        }
    }
    work::add(runs.len() as u64);
    stretches
}

/// The two parts of one line `runs` (its runs with ink, left to right) at a small label that
/// ends with a colon and is set into the line right before its last stretch (see
/// `labels_before_values`): the run ids of what stands before the label, and those of the
/// label with the stretch after it; `None` when the line holds no such label. With `alone`
/// the line is the only one of its leaf; else the last stretch must read as an amount, a
/// date or a number.
fn label_parts(runs: &[(u32, &TextRun)], alone: bool) -> Option<[Vec<u32>; 2]> {
    let stretches = stretches_of(runs);
    if stretches.len() < 3 {
        return None;
    }
    // The one place a label can have: before the last stretch, after another one.
    let n = stretches.len() - 2;
    let (label, last) = (&stretches[n], &stretches[n + 1]);
    let (small, large) = (runs[label.start].1, runs[last.start].1);
    // The white space between stretch `k` and the next one.
    let gap = |k: usize| {
        let end = runs[stretches[k].end - 1].1;
        runs[stretches[k + 1].start].1.x - (end.x + end.w)
    };
    let near = cut::BASELINE_NEAR * large.size;
    let step = cut::SIZE_STEP * small.size;
    // The words after the label read as an amount, a date or a number.
    let filled = || {
        let text: Vec<&str> = runs[last.clone()].iter().map(|r| r.1.text.trim()).collect();
        cut::reads_as_value(&text.join(" "))
    };
    // A stretch of a larger face than the label before it on the baseline of the last one.
    let on_data_line = || {
        stretches[..n].iter().any(|s| {
            let head = runs[s.start].1;
            head.size >= step && (head.y - large.y).abs() <= near
        })
    };
    let parts = (small.y - large.y).abs() <= near
        && runs[label.end - 1].1.text.trim_end().ends_with(':')
        && large.size >= step
        && gap(n) < gap(n - 1)
        && on_data_line()
        && (alone || filled());
    let ids = |from: usize, to: usize| -> Vec<u32> {
        runs[stretches[from].start..stretches[to - 1].end]
            .iter()
            .map(|r| r.0)
            .collect()
    };
    parts.then(|| [ids(0, n), ids(n, n + 2)])
}

/// Whether the line `other` shares the columns of the line `line` that holds a small label
/// before its last stretch (see [`label_parts`]; both are the runs with ink of one line, left
/// to right): `other` has text in the column of the stretch after the label and in the column
/// of a stretch before it, so `line` is a row of a table or of a grid whose rows or titles
/// `other` is one of. Two stretches stand in one column when their left or their right ends
/// are within [`cut::ROW_EDGE`] of the size of the stretch of `line`. One shared column is no
/// table: a line of codes over a data line ends where the data line ends.
fn shares_columns(line: &[(u32, &TextRun)], other: &[(u32, &TextRun)]) -> bool {
    let mine = stretches_of(line);
    if mine.len() < 3 {
        return false;
    }
    let ends = |runs: &[(u32, &TextRun)], s: &std::ops::Range<usize>| {
        let (first, last) = (runs[s.start].1, runs[s.end - 1].1);
        (first.x, last.x + last.w)
    };
    let (mut lefts, mut rights): (Vec<f32>, Vec<f32>) =
        stretches_of(other).iter().map(|s| ends(other, s)).unzip();
    lefts.sort_by(f32::total_cmp);
    rights.sort_by(f32::total_cmp);
    work::add((line.len() + other.len()) as u64);
    // Whether one of the sorted ends `at` stands within `near` of `x`.
    let has = |at: &[f32], x: f32, near: f32| {
        let k = at.partition_point(|e| *e < x - near);
        at.get(k).is_some_and(|e| *e <= x + near)
    };
    let shared = |s: &std::ops::Range<usize>| {
        let (left, right) = ends(line, s);
        let near = cut::ROW_EDGE * line[s.start].1.size;
        has(&lefts, left, near) || has(&rights, right, near)
    };
    let label = mine.len() - 2;
    shared(&mine[label + 1]) && mine[..label].iter().any(shared)
}

/// A text run as a piece of the cut.
fn piece(r: &TextRun) -> Piece {
    let inked = r.text.chars().any(|c| !c.is_whitespace());
    Piece {
        bounds: text_bounds(r),
        line: r.line,
        baseline: r.y,
        size: r.size,
        bold: inked && r.bold,
        inked,
        chars: r.text.trim().chars().count(),
        runs: 1,
        colon: r.text.trim_end().ends_with(':'),
        value: cut::reads_as_value(&r.text),
        number: cut::whole_number(&r.text),
        boxed: None,
        rows: 1,
        // A run stands where the host says its text stands: a host that hands the advance
        // of blanks after the text with the run has them in the extent of the piece.
        trail: 0.0,
    }
}

/// The pieces of the runs `ids` of `texts` as the cut takes them, each with the places in
/// `ids` of its runs from left to right: the runs of one line that follow each other within
/// [`PIECE_GAP`] sizes are one piece, so that a line reads the same to the cut whether its
/// producer wrote it as one run, word by word or glyph by glyph (the share of pieces that
/// cross a gutter and the length of a line of prose count pieces). A piece stands where its
/// ink stands: a run of white space joins a piece without widening it and without carrying
/// it to the next run (the gap is measured from the ink); alone it is a piece of white space. A piece knows how many runs it is made of (see [`Piece::runs`]).
fn pieces_of(texts: &[TextRun], ids: &[u32]) -> (Vec<Piece>, Vec<Vec<usize>>) {
    work::add(ids.len() as u64);
    let mut order: Vec<usize> = (0..ids.len()).collect();
    let run = |k: usize| &texts[ids[k] as usize];
    order.sort_by(|&a, &b| {
        let (ra, rb) = (run(a), run(b));
        ra.line
            .cmp(&rb.line)
            .then(ra.x.min(ra.x + ra.w).total_cmp(&rb.x.min(rb.x + rb.w)))
            .then(a.cmp(&b))
    });
    let mut pieces: Vec<Piece> = Vec::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    // Where the last run of the open piece ends, white space too.
    let mut end = f32::MIN;
    // How the ink of the open piece reads so far.
    let mut reading = None;
    for k in order {
        let one = piece(run(k));
        let text = &run(k).text;
        if let (Some(p), Some(m)) = (pieces.last_mut(), members.last_mut())
            && p.line == one.line
            && one.bounds.x0 - end <= PIECE_GAP * p.size.max(one.size)
        {
            // The gap to the next run is measured from the end of the ink once the piece has
            // ink: white space after it (padding a form sets up to the next column) joins the
            // piece and bridges nothing, as in the reader's own pieces.
            if one.inked || !p.inked {
                end = end.max(one.bounds.x1);
            }
            if one.inked && !p.inked {
                // The first ink of the piece: it stands where its ink stands.
                let (chars, runs) = (p.chars + one.chars, p.runs);
                *p = Piece { chars, runs, ..one };
                reading = Some(cut::Reading::of(text, one.value));
            } else if one.inked {
                p.bounds = union(p.bounds, one.bounds);
                p.size = p.size.max(one.size);
                p.bold = p.bold && one.bold;
                p.chars += one.chars;
                p.colon = one.colon;
                // The piece reads the same in one run and in several: a currency and its
                // figures in two runs are an amount, a number goes on with its digits.
                let r = reading.map_or(cut::Reading::of(text, one.value), |r| {
                    r.goes_on(text, one.value)
                });
                (p.value, p.number) = (r.value, r.number);
                reading = Some(r);
            }
            p.runs += 1;
            m.push(k);
            continue;
        }
        end = one.bounds.x1;
        reading = one.inked.then(|| cut::Reading::of(text, one.value));
        pieces.push(one);
        members.push(vec![k]);
    }
    (pieces, members)
}

/// The rules of `page` that are drawn in pieces, joined, and their joints (see
/// [`cut::joined`]): from the flat rules and the tops and bottoms of its rectangles, with the
/// upright rules and the sides of the rectangles as the lines that make a joint a corner.
fn page_joints(page: &PageIn) -> (Vec<Line>, Vec<(f32, f32)>) {
    let strokes = page
        .rules
        .iter()
        .map(|r| ([r.x0, r.y0, r.x1, r.y1], r.rect));
    let (flat, upright) = cut::flat_and_upright(strokes);
    cut::joined(&flat, &upright)
}

/// The whitespace cut of the runs `ids` of `texts` (see [`xy_cut`]) on their pieces (see
/// [`pieces_of`]): its groups hold places in `ids`, and the extent of the ink of each group
/// comes with it.
///
/// `ruled.joined` is sorted by height. The cut looks only at the rules between the lowest and
/// the highest of its pieces, so it is handed that stretch of the page's joined rules and
/// not all of them: it sorts what it gets on every call, and a page of many boxes is cut
/// once for each box.
fn cut_runs(
    texts: &[TextRun],
    ids: &[u32],
    ruled: &cut::Ruled<'_>,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> (cut::Cut, Vec<Bounds>) {
    let (pieces, members) = pieces_of(texts, ids);
    let ruled = &cut::Ruled {
        joined: cut::at_heights(ruled.joined, &pieces),
        ..*ruled
    };
    let mut found = cut::xy_cut_ruled(&pieces, ruled, limits, stop);
    let mut extents = Vec::with_capacity(found.groups.len());
    for group in &mut found.groups {
        extents.push(
            group
                .iter()
                .map(|&k| pieces[k].bounds)
                .reduce(union)
                .expect("a group has a piece"),
        );
        *group = group
            .iter()
            .flat_map(|&k| members[k].iter().copied())
            .collect();
    }
    (found, extents)
}

/// The extent of a run: its advance, from the baseline up its rendered height.
fn text_bounds(r: &TextRun) -> Bounds {
    norm(Bounds {
        x0: r.x,
        y0: r.y,
        x1: r.x + r.w,
        y1: r.y + r.h,
    })
}

fn norm(b: Bounds) -> Bounds {
    Bounds {
        x0: b.x0.min(b.x1),
        y0: b.y0.min(b.y1),
        x1: b.x0.max(b.x1),
        y1: b.y0.max(b.y1),
    }
}

/// The bounds of `prims` of `page` together; none when none of them is on the page. For a
/// host that moves primitives from one region to another after the decisions.
pub fn bounds_of(page: &PageIn, prims: &[PrimId]) -> Option<Bounds> {
    work::add(prims.len() as u64);
    prims
        .iter()
        .filter_map(|&p| prim_bounds(page, p))
        .reduce(union)
}

pub(crate) fn prim_bounds(page: &PageIn, p: PrimId) -> Option<Bounds> {
    Some(match p {
        PrimId::Text(i) => text_bounds(page.texts.get(i as usize)?),
        PrimId::Rule(i) => {
            let r = page.rules.get(i as usize)?;
            norm(Bounds {
                x0: r.x0,
                y0: r.y0,
                x1: r.x1,
                y1: r.y1,
            })
        }
        PrimId::Box(i) => norm(page.boxes.get(i as usize)?.bounds),
        PrimId::Picture(i) => norm(page.pictures.get(i as usize)?.bounds),
        PrimId::Code(i) => norm(page.codes.get(i as usize)?.bounds),
    })
}

/// Every primitive of the page, sorted.
fn all_prims(page: &PageIn) -> Vec<PrimId> {
    let mut out = Vec::with_capacity(
        page.texts.len() + page.rules.len() + page.boxes.len() + page.pictures.len(),
    );
    out.extend((0..page.texts.len() as u32).map(PrimId::Text));
    out.extend((0..page.rules.len() as u32).map(PrimId::Rule));
    out.extend((0..page.boxes.len() as u32).map(PrimId::Box));
    out.extend((0..page.pictures.len() as u32).map(PrimId::Picture));
    out.extend((0..page.codes.len() as u32).map(PrimId::Code));
    out
}

fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

fn centre(b: Bounds) -> (f32, f32) {
    ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)
}

/// The order the drawn boxes of a page are nested in: larger first, so that a box can only
/// nest in one before it; boxes of one area from the left, then from the top.
fn nesting_order(a: &Bounds, b: &Bounds) -> std::cmp::Ordering {
    let area = |b: &Bounds| (b.x1 - b.x0).max(0.0) * (b.y1 - b.y0).max(0.0);
    area(b)
        .total_cmp(&area(a))
        .then(a.x0.total_cmp(&b.x0))
        .then(b.y1.total_cmp(&a.y1))
}

/// The item each of the boxes `all` (in [`nesting_order`]) nests in: 0 for the page, `c + 1`
/// for the box `all[c]`.
///
/// A box nests in the first box of the page that holds it (the first in the list that
/// nests in the page), then in the first of those nested in that one that holds it, and so
/// on. A box that holds another holds its centre, and comes before it and after the box it
/// nests in. So one walk over the boxes at that centre, in the order of the list, goes down
/// the same way: each box on the walk that nests in the place reached so far and holds the
/// new box is the next place. The boxes at a centre come from one index of all of them, so
/// a page of many boxes side by side does not look at every box for each box.
///
/// The walk leans on the list alone: the boxes at a centre are met in the order of the list,
/// and a box is looked at only after the one it nests in. A test compares it with the walk
/// down the nested boxes it replaced, on seeded random pages.
fn holders(all: &[Bounds]) -> Vec<usize> {
    let index = Index::of(all);
    let mut parent: Vec<usize> = Vec::with_capacity(all.len());
    for (k, &bounds) in all.iter().enumerate() {
        let mut node = 0;
        let mut step = |c: usize| {
            work::add(1);
            if parent[c] == node && holds(all[c], bounds) {
                node = c + 1;
            }
        };
        match &index {
            Some(ix) => ix
                .at(centre(bounds))
                .iter()
                .map(|&c| c as usize)
                .take_while(|&c| c < k)
                .for_each(&mut step),
            None => (0..k).for_each(&mut step),
        }
        parent.push(node);
    }
    parent
}

/// Whether `outer` holds `inner` whole, within [`SNAP`].
fn holds(outer: Bounds, inner: Bounds) -> bool {
    inner.x0 >= outer.x0 - SNAP
        && inner.y0 >= outer.y0 - SNAP
        && inner.x1 <= outer.x1 + SNAP
        && inner.y1 <= outer.y1 + SNAP
}

/// Whether `outer` holds the point `(x, y)`, within [`SNAP`].
fn holds_point(outer: Bounds, (x, y): (f32, f32)) -> bool {
    x >= outer.x0 - SNAP && x <= outer.x1 + SNAP && y >= outer.y0 - SNAP && y <= outer.y1 + SNAP
}

/// The primitives the given regions took.
struct Taken {
    marks: [Vec<bool>; 5],
}

impl Taken {
    fn new(page: &PageIn) -> Taken {
        Taken {
            marks: [
                vec![false; page.texts.len()],
                vec![false; page.rules.len()],
                vec![false; page.boxes.len()],
                vec![false; page.pictures.len()],
                vec![false; page.codes.len()],
            ],
        }
    }

    fn slot(p: PrimId) -> (usize, usize) {
        match p {
            PrimId::Text(i) => (0, i as usize),
            PrimId::Rule(i) => (1, i as usize),
            PrimId::Box(i) => (2, i as usize),
            PrimId::Picture(i) => (3, i as usize),
            PrimId::Code(i) => (4, i as usize),
        }
    }

    fn has(&self, p: PrimId) -> bool {
        let (k, i) = Self::slot(p);
        self.marks[k].get(i).copied().unwrap_or(false)
    }

    /// Takes `p`; false when the page does not have it or it was taken before.
    fn take(&mut self, p: PrimId) -> bool {
        let (k, i) = Self::slot(p);
        match self.marks[k].get_mut(i) {
            Some(m) if !*m => {
                *m = true;
                true
            }
            _ => false,
        }
    }
}

/// The children of an item by where they lie: a grid of cells over their extent, each cell
/// with the children (their places in the list) that reach into it. Only for items with many
/// children; a few are looked at one by one.
struct Index {
    x0: f32,
    y0: f32,
    cw: f32,
    ch: f32,
    /// The cells across and the cells up.
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

/// Up to this many children are looked at one by one.
const FEW_CHILDREN: usize = 8;

impl Index {
    fn new(items: &[Item], kids: &[usize]) -> Option<Index> {
        if kids.len() <= FEW_CHILDREN {
            return None;
        }
        let bounds: Vec<Bounds> = kids.iter().map(|&c| items[c].bounds).collect();
        Index::of(&bounds)
    }

    /// The index of `bounds`; `None` for a few.
    ///
    /// There are about as many cells as boxes (at most 256 times 256), and they are dealt to
    /// the two directions by how many boxes of the mean size fit along each: boxes side by
    /// side in one row get cells side by side, a stack of rows gets cells one over the other.
    /// A square grid of cells would put a whole row of boxes, or a whole stack, in each of
    /// its cells there, and each look would then go over the root of the boxes.
    fn of(bounds: &[Bounds]) -> Option<Index> {
        if bounds.len() <= FEW_CHILDREN {
            return None;
        }
        let all = bounds.iter().copied().reduce(union)?;
        let len = bounds.len() as f64;
        let cells = len.min(256.0 * 256.0);
        // How many boxes of the mean extent fit along a direction: one at least, and one
        // where the extents say nothing (no extent, or one that is not a number).
        let fit = |extent: f32, of: fn(&Bounds) -> f32| -> f64 {
            let mean = bounds
                .iter()
                .map(|b| f64::from(of(b).max(0.0)))
                .sum::<f64>()
                / len;
            let fit = f64::from(extent) / mean;
            if fit.is_finite() { fit.max(1.0) } else { 1.0 }
        };
        let (fx, fy) = (
            fit(all.x1 - all.x0, |b| b.x1 - b.x0),
            fit(all.y1 - all.y0, |b| b.y1 - b.y0),
        );
        work::add(2 * bounds.len() as u64);
        let nx = (cells * fx / fy).sqrt().min(fx.ceil()).clamp(1.0, cells);
        let ny = (cells / nx).min(fy.ceil()).max(1.0);
        let (nx, ny) = (nx as usize, ny as usize);
        let cw = ((all.x1 - all.x0 + 2.0 * SNAP) / nx as f32).max(f32::MIN_POSITIVE);
        let ch = ((all.y1 - all.y0 + 2.0 * SNAP) / ny as f32).max(f32::MIN_POSITIVE);
        let mut ix = Index {
            x0: all.x0 - SNAP,
            y0: all.y0 - SNAP,
            cw,
            ch,
            nx,
            ny,
            cells: vec![Vec::new(); nx * ny],
        };
        for (k, b) in bounds.iter().enumerate() {
            let (a0, b0) = ix.cell(b.x0 - SNAP, b.y0 - SNAP);
            let (a1, b1) = ix.cell(b.x1 + SNAP, b.y1 + SNAP);
            for cy in b0..=b1 {
                for cx in a0..=a1 {
                    work::add(1);
                    ix.cells[cy * nx + cx].push(k as u32);
                }
            }
        }
        Some(ix)
    }

    fn cell(&self, x: f32, y: f32) -> (usize, usize) {
        let f = |v: f32, n: usize| (v.max(0.0) as usize).min(n - 1);
        (
            f((x - self.x0) / self.cw, self.nx),
            f((y - self.y0) / self.ch, self.ny),
        )
    }

    /// The children that may hold the point, in the order of the list.
    fn at(&self, (x, y): (f32, f32)) -> &[u32] {
        let (cx, cy) = self.cell(x, y);
        &self.cells[cy * self.nx + cx]
    }
}

/// What is wrong with the cover of `s`, if anything: every primitive of `page` in exactly one
/// leaf of the main cut, every primitive of a segment in exactly one leaf of each of its
/// alternatives.
fn cover_problem(s: &Segments, page: &PageIn) -> Option<String> {
    let leaves = |ids: &[SegId]| -> Vec<PrimId> {
        let mut out = Vec::new();
        let mut stack: Vec<SegId> = ids.to_vec();
        while let Some(id) = stack.pop() {
            let g = &s.all[id.0 as usize];
            if g.children.is_empty() {
                out.extend(g.prims.iter().copied());
            } else {
                stack.extend(g.children.iter().copied());
            }
        }
        out.sort_unstable();
        out
    };
    let mut every = all_prims(page);
    every.sort_unstable();
    if leaves(&[s.root]) != every {
        return Some("the leaves of the main cut do not hold every primitive once".into());
    }
    for g in &s.all {
        for alt in &g.alternatives {
            if leaves(alt) != g.prims {
                return Some(format!(
                    "an alternative of segment {} does not hold its primitives once",
                    g.id.0
                ));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task 26 fix round 6: a line with a small colon label before its last stretch is a row
    /// of a table when another line has text in the column of the stretch after the label
    /// and in a column of a stretch before it, by their left or their right ends within
    /// `ROW_EDGE` of the text size. One shared column, on either side of the label, is no
    /// table, and a line of codes that ends where a data line ends shares one.
    #[test]
    fn a_line_shares_the_columns_of_a_row_by_a_column_on_each_side_of_its_label() {
        let run = |text: &str, x: f32, size: f32| TextRun {
            text: text.into(),
            x,
            y: 700.0,
            w: 0.6 * size * text.chars().count() as f32,
            h: size,
            size,
            font: 0,
            font_kind: crate::input::FontKind::Unknown,
            bold: false,
            italic: false,
            color: None,
            shift: crate::input::Shift::Base,
            line: 0,
            off_axis: false,
            gaps: Vec::new(),
        };
        let line = |parts: &[(&str, f32, f32)]| -> Vec<TextRun> {
            parts.iter().map(|&(t, x, size)| run(t, x, size)).collect()
        };
        let shares = |a: &[TextRun], b: &[TextRun]| {
            let a: Vec<(u32, &TextRun)> = (0u32..).zip(a).collect();
            let b: Vec<(u32, &TextRun)> = (0u32..).zip(b).collect();
            shares_columns(&a, &b)
        };
        // The amount `12,50` stands from 300 to 330.
        let row = line(&[
            ("1", 50.0, 10.0),
            ("Widget", 90.0, 10.0),
            ("Part no.:", 250.0, 7.0),
            ("12,50", 300.0, 10.0),
        ]);
        // A second row, its amount set to the right end of the column; a row of titles.
        let amount = |x: f32| line(&[("2", 50.0, 10.0), ("Gadget", 90.0, 10.0), ("8,00", x, 10.0)]);
        assert!(shares(&row, &amount(306.0)));
        assert!(shares(
            &row,
            &line(&[
                ("Pos", 50.0, 10.0),
                ("Description", 90.0, 10.0),
                ("Amount", 300.0, 10.0)
            ])
        ));
        // `ROW_EDGE` of the size of 10 is 2.5: an amount that ends 2.4 off is in the column,
        // one that ends 2.6 off (and starts elsewhere) is not.
        assert!(shares(&row, &amount(306.0 + 2.4)));
        assert!(!shares(&row, &amount(306.0 + 2.6)));
        assert!(shares(&row, &amount(306.0 - 2.4)));
        assert!(!shares(&row, &amount(306.0 - 2.6)));
        // One column on one side of the label: the amount's alone (a line of codes that ends
        // where the row ends), or the columns before the label alone.
        assert!(!shares(&row, &line(&[("AB/12 345", 276.0, 10.0)])));
        assert!(!shares(
            &row,
            &line(&[("2", 50.0, 10.0), ("Gadget", 90.0, 10.0)])
        ));
        // The label's own column does not count: a line with text under the label and under
        // the amount shares one column.
        assert!(!shares(
            &row,
            &line(&[("4711", 250.0, 10.0), ("8,00", 306.0, 10.0)])
        ));
        // A line of two stretches holds no label set into it.
        assert!(!shares(&row[2..], &amount(306.0)));
    }

    /// The nesting as it was walked before the index: from the page down, at each place the
    /// first of the boxes nested there, in the order they came, that holds the new box.
    fn walked_down(all: &[Bounds]) -> Vec<usize> {
        let mut children: Vec<Vec<usize>> = vec![Vec::new()];
        let mut parent = Vec::with_capacity(all.len());
        for (k, &bounds) in all.iter().enumerate() {
            let mut node = 0;
            while let Some(&c) = children[node].iter().find(|&&c| holds(all[c - 1], bounds)) {
                node = c;
            }
            children[node].push(k + 1);
            children.push(Vec::new());
            parent.push(node);
        }
        parent
    }

    /// The same numbers on every run (xorshift64*).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: u64) -> f32 {
            (self.next() % n) as f32
        }
    }

    /// A random set of boxes on a lattice of `step` points: boxes anywhere (equal boxes,
    /// shared edges and boxes within a snap of each other come with a small lattice), an
    /// onion of boxes one in the other, a row of boxes side by side, and boxes set into
    /// boxes already there.
    fn boxes(rng: &mut Rng, step: f32) -> Vec<Bounds> {
        let at = |x: f32, y: f32, w: f32, h: f32| Bounds {
            x0: x * step,
            y0: y * step,
            x1: (x + w) * step,
            y1: (y + h) * step,
        };
        let mut out: Vec<Bounds> = Vec::new();
        for _ in 0..rng.next() % 30 {
            let (x, y) = (rng.below(40), rng.below(40));
            out.push(at(x, y, 1.0 + rng.below(20), 1.0 + rng.below(20)));
        }
        let (x, y, rings) = (rng.below(20), rng.below(20), rng.next() % 8);
        for r in 0..rings {
            let (r, size) = (r as f32, 2.0 * (rings - r) as f32);
            out.push(at(x + r, y + r, size, size));
        }
        let (y, w) = (rng.below(40), 1.0 + rng.below(3));
        for k in 0..rng.next() % 30 {
            out.push(at(k as f32 * w, y, w, 2.0));
        }
        for _ in 0..rng.next() % 10 {
            if out.is_empty() {
                break;
            }
            let b = out[(rng.next() % out.len() as u64) as usize];
            let (w, h) = (b.x1 - b.x0, b.y1 - b.y0);
            let (fx, fy) = (rng.below(4) / 4.0, rng.below(4) / 4.0);
            out.push(Bounds {
                x0: b.x0 + fx * w / 2.0,
                y0: b.y0 + fy * h / 2.0,
                x1: b.x1 - rng.below(4) / 8.0 * w,
                y1: b.y1 - rng.below(4) / 8.0 * h,
            });
        }
        out
    }

    /// Fix round 1 of part 2b-2 of Task 31c (M2): the nesting by one walk over the boxes at
    /// the centre of each box gives every box the place the walk down the nested boxes gave
    /// it, on 600 seeded random pages on lattices of 1, 2 and 5 points (gaps of one snap,
    /// shared edges, equal boxes, boxes one in the other, boxes side by side). A change to
    /// the index or to the walk that moves one box fails here. A change to the order of the
    /// list does not: both walks get the same list. The order is pinned by
    /// [`boxes_nest_in_the_innermost_box_before_them`].
    #[test]
    fn the_nesting_by_the_index_is_the_nesting_by_the_walk_down() {
        let mut rng = Rng(0xA076_1D64_78BD_642F);
        let (mut nested, mut indexed, mut deep) = (0usize, 0usize, 0usize);
        for round in 0..600 {
            let step = [1.0, 2.0, 5.0][round % 3];
            let mut all = boxes(&mut rng, step);
            all.sort_by(nesting_order);
            let (new, old) = (holders(&all), walked_down(&all));
            assert_eq!(new, old, "round {round}: {all:?}");
            nested += new.iter().filter(|&&p| p != 0).count();
            indexed += usize::from(Index::of(&all).is_some());
            deep += usize::from(new.iter().any(|&p| p != 0 && new[p - 1] != 0));
        }
        // The pages have boxes in boxes, boxes in those, and enough boxes for the index.
        assert!(
            nested > 5_000 && indexed > 500 && deep > 300,
            "{nested} {indexed} {deep}"
        );
    }

    /// Guard of the order the walk leans on, on one small page: a frame, a second frame
    /// equal to it within a snap, two boxes of one area in it side by side, a box of the
    /// area and the left edge of the right one set less than a snap higher, and a box in the
    /// left one. The larger frame is first and holds the other; boxes of one area go from
    /// the left, and those of one left edge from the top; each box nests in the innermost
    /// box that holds it. The right box and the one set higher hold each other within a
    /// snap: the higher one is first, so the right box nests in it.
    #[test]
    fn boxes_nest_in_the_innermost_box_before_them() {
        let b = |x0: f32, y0: f32, x1: f32, y1: f32| Bounds { x0, y0, x1, y1 };
        let (frame, twin) = (b(0.0, 0.0, 100.0, 100.0), b(1.0, 1.0, 100.0, 100.0));
        let (left, right) = (b(10.0, 10.0, 40.0, 40.0), b(60.0, 10.0, 90.0, 40.0));
        let raised = b(60.0, 11.0, 90.0, 41.0);
        let small = b(15.0, 15.0, 25.0, 25.0);
        let mut all = vec![small, right, twin, left, raised, frame];
        all.sort_by(nesting_order);
        assert_eq!(all, [frame, twin, left, raised, right, small]);
        assert_eq!(holders(&all), [0, 1, 2, 2, 4, 3]);
        assert_eq!(walked_down(&all), [0, 1, 2, 2, 4, 3]);
    }

    /// Task 26 fix round 4 (review 4 I1, M1): a piece reads the same in one run and in
    /// several. The word or sign of a currency before the figures, in a run of its own,
    /// says nothing yet and the next run decides; after the figures it goes on with the
    /// amount; a whole number goes on with its next digits.
    #[test]
    fn a_piece_reads_the_same_in_one_run_and_in_several() {
        // The one piece of `runs`, set one after the other with a blank between them
        // (`touch`: with none): whether it reads as a value, and its whole number.
        let read = |runs: &[&str], touch: bool| {
            let mut x = 40.0;
            let texts: Vec<TextRun> = runs
                .iter()
                .map(|t| {
                    let w = 6.0 * t.chars().count() as f32;
                    let run = TextRun {
                        text: t.to_string(),
                        x,
                        y: 700.0,
                        w,
                        h: 10.0,
                        size: 10.0,
                        font: 1,
                        font_kind: crate::FontKind::Proportional,
                        bold: false,
                        italic: false,
                        color: None,
                        shift: crate::Shift::Base,
                        line: 0,
                        off_axis: false,
                        gaps: Vec::new(),
                    };
                    x += w + if touch { 0.0 } else { 3.0 };
                    run
                })
                .collect();
            let ids: Vec<u32> = (0..texts.len() as u32).collect();
            let (pieces, _) = pieces_of(&texts, &ids);
            assert_eq!(pieces.len(), 1, "{runs:?}");
            (pieces[0].value, pieces[0].number)
        };
        for (runs, one, value) in [
            (&["CHF", "80"][..], "CHF 80", true),
            (&["$", "80"], "$ 80", true),
            (&["CHF", "1.200,50"], "CHF 1.200,50", true),
            (&["80", "CHF"], "80 CHF", true),
            (&["CHF", "Total"], "CHF Total", false),
            (&["TEUR", "80"], "TEUR 80", false),
            (&["CHF", "CHF"], "CHF CHF", false),
            (&["Total", "80"], "Total 80", false),
        ] {
            assert_eq!(read(runs, false), (value, None), "{runs:?}");
            assert_eq!(read(&[one], false), (value, None), "{one}");
        }
        // A blank run between the currency and its figures is no ink and decides nothing.
        assert_eq!(read(&["CHF", " ", "80"], true), (true, None));
        assert_eq!(read(&["CHF", " ", "net"], true), (false, None));
        // M1: a whole number in two runs is the number of its digits in a row; a currency
        // before it makes it an amount, which is no whole number.
        assert_eq!(read(&["20", "24"], true), (true, Some(2024)));
        assert_eq!(read(&["2024"], true), (true, Some(2024)));
        assert_eq!(read(&["20", "2a"], true), (false, None));
        assert_eq!(read(&["$", "2024"], true), (true, None));
    }
}
