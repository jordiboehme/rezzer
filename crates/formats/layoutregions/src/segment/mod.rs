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
use crate::region::{Bounds, Issue, IssueKind, Kind};
use crate::segment::cut::{CutShort, Piece, Split, xy_cut};
use crate::segment::drawn::{Line, PAGE_SHARE, SNAP, clustered};
use crate::segment::style::style_steps;
use crate::work;

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
    /// A grid of drawn lines of at least two rows and two columns.
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
    /// The drawn primitives of a segment (rules, boxes, pictures, codes) that no drawn child
    /// holds. It is one leaf.
    Drawing,
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
}

/// The segment tree of `page`, in this order of proposal:
///
/// 1. Given regions, in their `order`: each one leaf with [`Source::Given`]; a primitive two
///    of them name stays with the first, a primitive the page does not have is skipped.
/// 2. The drawn structure ([`drawn::drawn`]): grids of at least two rows and two columns and
///    closed boxes that hold text, nested where one holds another. A grid or box that covers
///    more than [`PAGE_SHARE`] of the page is its frame and no segment. Text belongs to the
///    innermost one that holds its centre, any other primitive to the innermost one that
///    holds it whole (within [`SNAP`]).
/// 3. The whitespace cut ([`cut::xy_cut`]) of the text of each segment, with its own
///    horizontal rules: each group a segment of [`Source::Gutter`], [`Source::Band`] or
///    [`Source::Rule`] (the split before it; the first group takes the split after it).
/// 4. Style steps ([`style::style_steps`]) inside each group of the cut.
///
/// Where a segment has drawn children, the whitespace cut of all its text with the drawn
/// structure left out is a second cut of it when one of its column gutters holds a vertical
/// side of a drawn child beside its text (a gutter that a box also bounds): that cut is kept
/// in `alternatives`. Its groups are leaves; the given regions and one drawing leaf complete
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
    let mut b = Builder {
        page,
        limits,
        stop,
        max: limits.max_cut_depth as usize,
        all: Vec::new(),
        issues: Vec::new(),
        noted: [false; 5],
        stopped: false,
        given: Vec::new(),
    };
    let every = all_prims(page);
    work::add(every.len() as u64);
    let frame = Bounds {
        x0: 0.0,
        y0: 0.0,
        x1: page.size[0],
        y1: page.size[1],
    };
    let root = b.push(Source::Page, every, Some(frame), None, false);
    b.segment(root);
    let out = Segments {
        all: b.all,
        root,
        issues: b.issues,
    };
    debug_assert_eq!(cover_problem(&out, page), None);
    out
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
    limits: &'a Limits,
    stop: &'a dyn Fn() -> bool,
    max: usize,
    all: Vec<Segment>,
    issues: Vec<Issue>,
    noted: [bool; 5],
    stopped: bool,
    /// The segments of the given regions.
    given: Vec<SegId>,
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
    fn segment(&mut self, root: SegId) {
        let page = self.page;
        if self.halted() {
            return;
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
        // The drawn structure, from the text the given regions leave: a box holds text when
        // the centre of a run lies in it.
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
        let found = drawn::drawn(page, &inked);
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
        let mut items = self.items(&found);
        self.assign(&mut items, &taken);
        if self.max == 0 {
            if !regions.is_empty() || parts(&items[0]) || self.text_parts(&items[0].texts) {
                self.note(Note::Depth);
            }
            return;
        }
        // Every item's primitives, its children's included; then the segments, top down. The
        // root makes its given regions and its own parts at once, or none.
        let mut sub = subtree_prims(&items, self.max);
        if !self.room(regions.len() + own_parts(&items[0], &sub, regions.len())) {
            return;
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
    }

    /// The grids and boxes of the page as items, nested; item 0 is the page.
    fn items(&self, found: &drawn::DrawnStructure) -> Vec<Item> {
        let page = self.page;
        let area = |b: &Bounds| (b.x1 - b.x0).max(0.0) * (b.y1 - b.y0).max(0.0);
        let mut drawn: Vec<(Bounds, Source)> = Vec::new();
        if let Ok(g) = &found.grids {
            for (gh, gv) in &g.groups {
                work::add((gh.len() + gv.len()) as u64);
                // Rows and columns as the box search counts them.
                let mut xs: Vec<f32> = gv.iter().map(|&k| g.vs[k].at).collect();
                xs.extend(gh.iter().map(|&j| g.hs[j].from).reduce(f32::min));
                xs.extend(gh.iter().map(|&j| g.hs[j].to).reduce(f32::max));
                let mut ys: Vec<f32> = gh.iter().map(|&j| g.hs[j].at).collect();
                ys.extend(gv.iter().map(|&k| g.vs[k].from).reduce(f32::min));
                ys.extend(gv.iter().map(|&k| g.vs[k].to).reduce(f32::max));
                let (xs, ys) = (clustered(xs), clustered(ys));
                let (cols, rows) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
                if cols < 2 || rows < 2 {
                    continue;
                }
                let b = Bounds {
                    x0: xs[0],
                    y0: ys[0],
                    x1: xs[cols],
                    y1: ys[rows],
                };
                if area(&b) > PAGE_SHARE * page.size[0].max(0.0) * page.size[1].max(0.0) {
                    continue;
                }
                let source = Source::Grid {
                    rows: rows as u32,
                    cols: cols as u32,
                };
                drawn.push((b, source));
            }
        }
        drawn.extend(found.boxes.iter().map(|&b| (b, Source::Box)));
        // Larger first, so that an item can only nest in one before it.
        drawn.sort_by(|a, b| {
            area(&b.0)
                .total_cmp(&area(&a.0))
                .then(a.0.x0.total_cmp(&b.0.x0))
                .then(b.0.y1.total_cmp(&a.0.y1))
        });
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
        }];
        for (bounds, source) in drawn {
            let mut node = 0;
            'down: loop {
                for &c in &items[node].children {
                    work::add(1);
                    if holds(items[c].bounds, bounds) {
                        node = c;
                        continue 'down;
                    }
                }
                break;
            }
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
    /// to the depth bound: an item at the bound keeps what the items in it hold.
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
            while items[node].depth < self.max {
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
        for (node, t) in texts {
            items[node].texts.push(t);
        }
        for (node, p) in others {
            items[node].others.push(p);
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
            return Vec::new();
        }
        let given = if i == 0 {
            self.given.clone()
        } else {
            Vec::new()
        };
        if d >= self.max {
            if parts(it) || self.text_parts(&it.texts) {
                self.note(Note::Depth);
            }
            return Vec::new();
        }
        if !self.room(own_parts(it, sub, given.len())) {
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
        if parted || (!it.texts.is_empty() && !it.others.is_empty()) {
            // Both parts are made before the text is parted, so that the room they took stays
            // theirs.
            let rest = (!it.texts.is_empty()).then(|| {
                let mut prims: Vec<PrimId> = it.texts.iter().map(|&t| PrimId::Text(t)).collect();
                prims.sort_unstable();
                self.push(Source::Rest, prims, None, Some(s), true)
            });
            if !it.others.is_empty() {
                let mut prims = it.others.clone();
                prims.sort_unstable();
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

    /// Whether the text runs `ids` would be parted by the cut or by style steps.
    fn text_parts(&self, ids: &[u32]) -> bool {
        if ids.len() < 2 {
            return false;
        }
        let pieces: Vec<Piece> = ids
            .iter()
            .map(|&t| piece(&self.page.texts[t as usize]))
            .collect();
        let bound = Limits {
            max_cut_depth: 0,
            ..*self.limits
        };
        let found = xy_cut(&pieces, &[], &bound, &|| false);
        if found.groups.len() > 1 || found.short.is_some() {
            return true;
        }
        let order: Vec<u32> = found.groups.concat().iter().map(|&k| ids[k]).collect();
        style_steps(&self.page.texts, &order).len() > 1
    }

    /// Parts the text runs `ids` of segment `s` (at depth `d`) by the whitespace cut, then each
    /// group by style steps.
    fn text(&mut self, s: SegId, ids: &[u32], d: usize, rules: &[Line]) {
        if ids.len() < 2 || self.halted() {
            return;
        }
        work::add(ids.len() as u64);
        let pieces: Vec<Piece> = ids
            .iter()
            .map(|&t| piece(&self.page.texts[t as usize]))
            .collect();
        let bound = Limits {
            max_cut_depth: (self.max - d) as u32,
            ..*self.limits
        };
        let found = xy_cut(&pieces, rules, &bound, self.stop);
        self.short(found.short);
        if found.groups.len() < 2 {
            let order: Vec<u32> = found.groups.concat().iter().map(|&k| ids[k]).collect();
            self.style(s, &order, d);
            return;
        }
        if d >= self.max {
            self.note(Note::Depth);
            return;
        }
        if !self.room(found.groups.len()) {
            return;
        }
        // Every group is made before any is parted, so that the room they took stays theirs.
        let mut made = Vec::with_capacity(found.groups.len());
        for (g, group) in found.groups.iter().enumerate() {
            let order: Vec<u32> = group.iter().map(|&k| ids[k]).collect();
            let mut prims: Vec<PrimId> = order.iter().map(|&t| PrimId::Text(t)).collect();
            prims.sort_unstable();
            let id = self.push(source_of(&found.splits, g), prims, None, Some(s), true);
            made.push((id, order));
        }
        for (id, order) in made {
            self.style(id, &order, d + 1);
        }
    }

    /// Turns a cut that stopped early into its issue.
    fn short(&mut self, short: Option<CutShort>) {
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
        if order.len() < 2 || self.stopped {
            return;
        }
        work::add(order.len() as u64);
        let groups = style_steps(&self.page.texts, order);
        if groups.len() < 2 {
            return;
        }
        if d >= self.max {
            self.note(Note::Depth);
            return;
        }
        if !self.room(groups.len()) {
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
    /// vertical side of a drawn child beside its text: the gutter that a box also bounds. Its groups are
    /// leaves; the given regions and one leaf of all drawing of the segment complete it.
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
        let pieces: Vec<Piece> = ids
            .iter()
            .map(|&t| piece(&self.page.texts[t as usize]))
            .collect();
        let bound = Limits {
            max_cut_depth: (self.max - d) as u32,
            ..*self.limits
        };
        let found = xy_cut(&pieces, rules, &bound, self.stop);
        if found.short == Some(CutShort::Stopped) {
            self.short(found.short);
            return;
        }
        if found.groups.len() < 2 {
            return;
        }
        let extent = |g: &[usize]| {
            g.iter()
                .map(|&k| pieces[k].bounds)
                .reduce(union)
                .expect("a group has a piece")
        };
        let bounded = found.splits.iter().enumerate().any(|(j, split)| {
            if !matches!(split, Split::Gutter | Split::Boxed) {
                return false;
            }
            let (a, b) = (extent(&found.groups[j]), extent(&found.groups[j + 1]));
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
        self.short(found.short);
        if self.all[s.0 as usize].alternatives.len() >= self.limits.max_alternatives {
            self.note(Note::Alternatives);
            return;
        }
        if !self.room(found.groups.len() + usize::from(!drawing.is_empty())) {
            return;
        }
        let mut alt: Vec<SegId> = given.to_vec();
        for (g, group) in found.groups.iter().enumerate() {
            let mut prims: Vec<PrimId> = group.iter().map(|&k| PrimId::Text(ids[k])).collect();
            prims.sort_unstable();
            alt.push(self.push(source_of(&found.splits, g), prims, None, Some(s), false));
        }
        if !drawing.is_empty() {
            alt.push(self.push(Source::Drawing, drawing, None, Some(s), false));
        }
        self.all[s.0 as usize].alternatives.push(alt);
    }
}

/// The segments item `it` makes for its own parts, beside `given` given regions (the root's):
/// its drawn children that hold anything (see `sub`), and when it is parted, one segment for
/// its text and one for its drawing.
fn own_parts(it: &Item, sub: &[Vec<PrimId>], given: usize) -> usize {
    let kids = it.children.iter().filter(|&&c| !sub[c].is_empty()).count();
    let split = kids > 0 || given > 0 || (!it.texts.is_empty() && !it.others.is_empty());
    if !split {
        return 0;
    }
    kids + usize::from(!it.texts.is_empty()) + usize::from(!it.others.is_empty())
}

/// Whether item `it` holds more than text: drawn children, or drawing beside text.
fn parts(it: &Item) -> bool {
    !it.children.is_empty() || (!it.texts.is_empty() && !it.others.is_empty())
}

/// The source of group `g` of a cut with `splits`: the split before it; the first group
/// takes the split after it.
fn source_of(splits: &[Split], g: usize) -> Source {
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

/// The horizontal rules among `prims`, as the cut takes them; a rectangle gives its top and
/// bottom.
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
    out
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
        boxed: None,
        rows: 1,
    }
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

fn prim_bounds(page: &PageIn, p: PrimId) -> Option<Bounds> {
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
    n: usize,
    cells: Vec<Vec<u32>>,
}

/// Up to this many children are looked at one by one.
const FEW_CHILDREN: usize = 8;

impl Index {
    fn new(items: &[Item], kids: &[usize]) -> Option<Index> {
        if kids.len() <= FEW_CHILDREN {
            return None;
        }
        let all = kids.iter().map(|&c| items[c].bounds).reduce(union)?;
        let n = ((kids.len() as f64).sqrt().ceil() as usize).clamp(1, 256);
        let cw = ((all.x1 - all.x0 + 2.0 * SNAP) / n as f32).max(f32::MIN_POSITIVE);
        let ch = ((all.y1 - all.y0 + 2.0 * SNAP) / n as f32).max(f32::MIN_POSITIVE);
        let mut ix = Index {
            x0: all.x0 - SNAP,
            y0: all.y0 - SNAP,
            cw,
            ch,
            n,
            cells: vec![Vec::new(); n * n],
        };
        for (k, &c) in kids.iter().enumerate() {
            let b = items[c].bounds;
            let (a0, b0) = ix.cell(b.x0 - SNAP, b.y0 - SNAP);
            let (a1, b1) = ix.cell(b.x1 + SNAP, b.y1 + SNAP);
            for cy in b0..=b1 {
                for cx in a0..=a1 {
                    work::add(1);
                    ix.cells[cy * n + cx].push(k as u32);
                }
            }
        }
        Some(ix)
    }

    fn cell(&self, x: f32, y: f32) -> (usize, usize) {
        let f = |v: f32| (v.max(0.0) as usize).min(self.n - 1);
        (f((x - self.x0) / self.cw), f((y - self.y0) / self.ch))
    }

    /// The children that may hold the point, in the order of the list.
    fn at(&self, (x, y): (f32, f32)) -> &[u32] {
        let (cx, cy) = self.cell(x, y);
        &self.cells[cy * self.n + cx]
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
