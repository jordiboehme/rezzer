// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The reading order of a page's regions, in one pass.
//!
//! Header bands come first, top to bottom; footnote legends and then footer bands come last,
//! each in the same XY order as the body. The body regions between them are put in XY order
//! over their boxes: the whitespace cut of [`crate::segment::cut`], on regions instead of
//! pieces.
//!
//! - A wide drawn rule across a set parts it into above and below.
//! - A gutter at least one body em wide, from the set's top to its bottom, parts it into
//!   columns, read left to right, each down to its end. Only a region that stands in a band of
//!   its own may reach across the gutter (a title, a figure or a table across the columns): it
//!   parts the columns into bands, as a piece across a gutter does in the cut. A gutter only a
//!   drawn box makes is taken after the bands of white space across, as in the cut.
//! - Otherwise the widest band of white space across the set parts it into above and below.
//! - What is left is read top to bottom, then left to right.
//!
//! Body regions the file gives an order keep that order among themselves. A body region without
//! one goes with the given regions of the column the XY order read it in (or of the column that
//! column is part of); a region across the columns, or of a column without given regions,
//! stands after the given regions the XY order put before it.
//!
//! Flowing text reads down each column because each column is its own cut; a comparison reads
//! row by row because it is one `Table` region. Figures and codes are regions like the others
//! and take their place in the same pass. Every length is measured in the body em of the page
//! (the size most of its ink is set in), so the order is the same at every scale.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::input::{PageIn, PrimId};
use crate::limits::Limits;
use crate::region::{Bounds, Certainty, Kind, Region, RegionId};
use crate::segment::cut::SIZE_STEP;
use crate::work;

/// A drawn rule across this share of a set's width or more parts the set into above and below
/// (as the cut's wide rule).
const WIDE_RULE: f32 = 2.0 / 3.0;

/// A column gutter is at least this many body ems wide (P6-Y).
const GUTTER: f32 = 1.0;

/// The two sides of a gutter stand side by side: their heights overlap by this share of the
/// smaller one (as in the cut).
const SIDE_BY_SIDE: f32 = 0.3;

/// Two regions whose tops are at most this share of the smaller line height apart stand in
/// one row.
const ROW_TOLERANCE: f32 = 0.5;

/// A rule is horizontal when its two ends differ in height by at most this many body ems.
const FLAT: f32 = 0.25;

/// Drawn lines frame a region when they reach each edge of its box within this many
/// body ems.
const FRAME: f32 = 0.5;

/// A page with no text measures its lengths in this share of its height.
const NO_TEXT_EM: f32 = 1.0 / 66.0;

/// The regions of a page in reading order: every region of `regions` once.
///
/// Header bands first; then the body regions in XY order, those the file gives an order (see
/// [`PageIn::given`]) in that order among themselves with the untagged regions of their
/// columns (see `Cx::with_given`); then the footnote legends, then the
/// footer bands. A region with a parent among `regions` follows its parent, its siblings in XY
/// order. A region with no usable box (a bound that is not a number) stands after the placed
/// regions of its group, in the order it was handed over.
pub fn order(page: &PageIn, regions: &[Region]) -> Vec<RegionId> {
    work::add(1 + regions.len() as u64);
    let em = body_em(page);
    let given = given_orders(page, regions);
    let frames = Frames::new(page, em);
    let cx = Cx {
        page,
        em,
        boxes: regions
            .iter()
            .map(|r| Info::new(page, r, em, &given, &frames))
            .collect(),
        max_depth: Limits::default().max_cut_depth as usize,
        cells: RefCell::new(vec![0; regions.len()]),
        tree: RefCell::new(vec![(0, false)]),
    };
    let at: HashMap<RegionId, usize> = regions.iter().enumerate().map(|(k, r)| (r.id, k)).collect();
    // The children of each region, and the regions without a parent among `regions`.
    let mut kids: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut top: Vec<usize> = Vec::new();
    for (k, r) in regions.iter().enumerate() {
        match r.parent.and_then(|p| at.get(&p)).copied() {
            Some(p) if p != k => kids.entry(p).or_default().push(k),
            _ => top.push(k),
        }
    }
    let (mut head, mut body, mut legend, mut foot) = (vec![], vec![], vec![], vec![]);
    for k in top {
        match regions[k].kind {
            Kind::HeaderBand => head.push(k),
            Kind::FootnoteLegend => legend.push(k),
            Kind::FooterBand => foot.push(k),
            _ => body.push(k),
        }
    }
    let mut ordered = cx.place(head);
    ordered.extend(cx.with_given(cx.place(body)));
    ordered.extend(cx.place(legend));
    ordered.extend(cx.place(foot));
    // Each region, then its children, depth first; a region met twice stands once.
    let mut seen = vec![false; regions.len()];
    let mut out = Vec::with_capacity(regions.len());
    let mut stack: Vec<usize> = ordered.into_iter().rev().collect();
    while let Some(k) = stack.pop() {
        if std::mem::replace(&mut seen[k], true) {
            continue;
        }
        out.push(regions[k].id);
        if let Some(c) = kids.get(&k) {
            stack.extend(cx.place(c.clone()).into_iter().rev());
        }
    }
    // Regions whose parents only point at each other: in the order handed over.
    for (k, r) in regions.iter().enumerate() {
        if !seen[k] {
            out.push(r.id);
        }
    }
    out
}

/// What the order looks at of a region.
struct Info {
    /// The box, `y` upward; `None` when a bound is not a number.
    b: Option<Bounds>,
    /// The median size of its text, `None` without text.
    line: Option<f32>,
    /// A drawn box: the lines it holds, or a drawn box or four rules of the page, frame its box.
    drawn: bool,
    /// The order the file gives it, if any.
    given: Option<u32>,
}

impl Info {
    fn new(
        page: &PageIn,
        r: &Region,
        em: f32,
        given: &HashMap<RegionId, u32>,
        frames: &Frames,
    ) -> Info {
        work::add(1 + r.prims.len() as u64);
        let b = r.bounds;
        let b = [b.x0, b.y0, b.x1, b.y1]
            .iter()
            .all(|v| v.is_finite())
            .then(|| normal(b));
        let mut sizes: Vec<f32> = r
            .prims
            .iter()
            .filter_map(|p| match p {
                PrimId::Text(i) => page.texts.get(*i as usize).map(|t| t.size.abs()),
                _ => None,
            })
            .filter(|s| s.is_finite() && *s > 0.0)
            .collect();
        sizes.sort_by(f32::total_cmp);
        Info {
            b,
            line: sizes.get(sizes.len() / 2).copied(),
            drawn: b.is_some_and(|b| framed(page, r, b, em) || frames.frame(b)),
            given: given.get(&r.id).copied(),
        }
    }
}

/// The bounds with `x0 <= x1` and `y0 <= y1`.
fn normal(b: Bounds) -> Bounds {
    Bounds {
        x0: b.x0.min(b.x1),
        y0: b.y0.min(b.y1),
        x1: b.x0.max(b.x1),
        y1: b.y0.max(b.y1),
    }
}

/// Whether the drawn lines the region `r` holds frame its box `b`: its drawn boxes and rules
/// reach each edge of `b` together.
fn framed(page: &PageIn, r: &Region, b: Bounds, em: f32) -> bool {
    let reach = r
        .prims
        .iter()
        .filter_map(|p| match *p {
            PrimId::Box(i) => page.boxes.get(i as usize).map(|x| normal(x.bounds)),
            PrimId::Rule(i) => page.rules.get(i as usize).map(|x| {
                normal(Bounds {
                    x0: x.x0,
                    y0: x.y0,
                    x1: x.x1,
                    y1: x.y1,
                })
            }),
            _ => None,
        })
        .reduce(|a, f| Bounds {
            x0: a.x0.min(f.x0),
            y0: a.y0.min(f.y0),
            x1: a.x1.max(f.x1),
            y1: a.y1.max(f.y1),
        });
    let near = FRAME * em;
    reach.is_some_and(|f| {
        (f.x0 - b.x0).abs() <= near
            && (f.x1 - b.x1).abs() <= near
            && (f.y0 - b.y0).abs() <= near
            && (f.y1 - b.y1).abs() <= near
    })
}

/// The drawn boxes and rules of a page, whoever holds them, sorted once so that the frame of
/// a region is looked up in a few steps (as the cut decides a boxed piece by the page's boxes,
/// not by who holds them).
struct Frames {
    /// The drawn boxes and rectangles, by their top.
    boxes: Vec<Bounds>,
    /// The horizontal rules as (height, from, to), by height.
    flat: Vec<(f32, f32, f32)>,
    /// The vertical rules as (place, from, to), by place.
    upright: Vec<(f32, f32, f32)>,
    /// How far an edge of a frame may lie from the edge of a region's box.
    near: f32,
}

impl Frames {
    fn new(page: &PageIn, em: f32) -> Frames {
        work::add((page.boxes.len() + page.rules.len()) as u64);
        let finite = |b: &Bounds| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite());
        let mut boxes: Vec<Bounds> = page.boxes.iter().map(|x| normal(x.bounds)).collect();
        let (mut flat, mut upright) = (Vec::new(), Vec::new());
        let thin = FLAT * em;
        for r in &page.rules {
            let b = normal(Bounds {
                x0: r.x0,
                y0: r.y0,
                x1: r.x1,
                y1: r.y1,
            });
            if !finite(&b) {
                continue;
            }
            if r.rect {
                boxes.push(b);
            } else if b.y1 - b.y0 <= thin {
                flat.push((0.5 * (b.y0 + b.y1), b.x0, b.x1));
            } else if b.x1 - b.x0 <= thin {
                upright.push((0.5 * (b.x0 + b.x1), b.y0, b.y1));
            }
        }
        boxes.retain(finite);
        boxes.sort_by(|a, b| a.y1.total_cmp(&b.y1));
        flat.sort_by(|a, b| a.0.total_cmp(&b.0));
        upright.sort_by(|a, b| a.0.total_cmp(&b.0));
        Frames {
            boxes,
            flat,
            upright,
            near: FRAME * em,
        }
    }

    /// Whether a drawn box, or four rules, frame `b`: each edge within [`FRAME`] body ems of
    /// the edge of `b`.
    fn frame(&self, b: Bounds) -> bool {
        let near = self.near;
        let at = |v: f32, w: f32| (v - w).abs() <= near;
        let lo = self.boxes.partition_point(|x| x.y1 < b.y1 - near);
        let boxed = self.boxes[lo..]
            .iter()
            .take_while(|x| x.y1 <= b.y1 + near)
            .inspect(|_| work::add(1))
            .any(|x| at(x.x0, b.x0) && at(x.x1, b.x1) && at(x.y0, b.y0));
        if boxed {
            return true;
        }
        // A rule at `place` within reach that spans `from` to `to`.
        let spans = |list: &[(f32, f32, f32)], place: f32, from: f32, to: f32| {
            let lo = list.partition_point(|r| r.0 < place - near);
            list[lo..]
                .iter()
                .take_while(|r| r.0 <= place + near)
                .inspect(|_| work::add(1))
                .any(|r| r.1 <= from + near && r.2 >= to - near)
        };
        spans(&self.flat, b.y1, b.x0, b.x1)
            && spans(&self.flat, b.y0, b.x0, b.x1)
            && spans(&self.upright, b.x0, b.y0, b.y1)
            && spans(&self.upright, b.x1, b.y0, b.y1)
    }
}

/// The body em of a page: the text size most ink characters are set in (sizes less than
/// [`SIZE_STEP`] above the smallest of a group count as one), or a share of the page height on
/// a page with no text.
fn body_em(page: &PageIn) -> f32 {
    work::add(page.texts.len() as u64);
    let mut sizes: Vec<(f32, usize)> = page
        .texts
        .iter()
        .map(|t| {
            let ink = t.text.chars().filter(|c| !c.is_whitespace()).count();
            (t.size.abs(), ink)
        })
        .filter(|(s, n)| s.is_finite() && *s > 0.0 && *n > 0)
        .collect();
    sizes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut best: Option<(usize, f32)> = None;
    let mut k = 0;
    while k < sizes.len() {
        let first = sizes[k].0;
        let mut n = 0;
        let mut j = k;
        while j < sizes.len() && sizes[j].0 < first * SIZE_STEP {
            n += sizes[j].1;
            j += 1;
        }
        if best.is_none_or(|(m, _)| n > m) {
            best = Some((n, first));
        }
        k = j;
    }
    match best {
        Some((_, s)) => s,
        None => {
            let h = page.size[1].abs();
            if h.is_finite() && h > 0.0 {
                NO_TEXT_EM * h
            } else {
                1.0
            }
        }
    }
}

/// The order the file gives each region it states: the smallest `order` of a given region that
/// shares a primitive with it. Only a region the file states (certainty `Exact`, or a `given.`
/// signal) takes one.
fn given_orders(page: &PageIn, regions: &[Region]) -> HashMap<RegionId, u32> {
    let mut of: HashMap<PrimId, u32> = HashMap::new();
    for g in &page.given {
        work::add(1 + g.prims.len() as u64);
        for p in &g.prims {
            let e = of.entry(*p).or_insert(g.order);
            *e = (*e).min(g.order);
        }
    }
    let mut out = HashMap::new();
    if of.is_empty() {
        return out;
    }
    for r in regions {
        let stated =
            r.certainty == Certainty::Exact || r.signals.iter().any(|s| s.id.starts_with("given."));
        if !stated {
            continue;
        }
        work::add(r.prims.len() as u64);
        if let Some(o) = r.prims.iter().filter_map(|p| of.get(p)).min() {
            out.insert(r.id, *o);
        }
    }
    out
}

struct Cx<'a> {
    page: &'a PageIn,
    em: f32,
    /// What the order looks at of each region, by its place in the slice handed over.
    boxes: Vec<Info>,
    max_depth: usize,
    /// The cell the XY order put each region in: the column it was read in (a new cell for
    /// each side of a gutter), or the cell of the set it was cut from.
    cells: RefCell<Vec<u32>>,
    /// For each cell, the cell it was cut from and whether it is a side of a gutter (a column);
    /// cell 0 is the page.
    tree: RefCell<Vec<(u32, bool)>>,
}

/// A column gutter of a set: the white space it spans.
struct Gutter {
    from: f32,
    to: f32,
    /// Only a drawn box makes it: the regions of one side are all drawn boxes.
    boxed: bool,
}

impl Cx<'_> {
    /// The regions `set` (places in the slice handed over) in XY order; those with no usable
    /// box after the others, in their order.
    fn place(&self, set: Vec<usize>) -> Vec<usize> {
        let (placed, loose): (Vec<usize>, Vec<usize>) =
            set.into_iter().partition(|&k| self.boxes[k].b.is_some());
        let mut out = self.xy(placed, 0, 0);
        out.extend(loose);
        out
    }

    /// The body in XY order with the regions the file gives an order put in that order. A
    /// region without one goes with the given regions of its cell (the column the XY order read
    /// it in): just after the given region before it in that cell, or, when it opens its cell,
    /// just before the next one. A column without given regions that is itself part of a column
    /// (a sub-column) takes those of the enclosing column: the region goes after the given
    /// region before it there, or after the last of them in the given order. Any other region
    /// without a given region in its cell (an untagged figure across the page, which is a cell
    /// of its own) stands after the groups of the given regions the XY order put before it, the
    /// last of them in the given order; before all groups when there are none. So an untagged
    /// figure or heading stays in its column, and an untagged band stays between the bands
    /// above and below it, however the file orders the columns.
    fn with_given(&self, set: Vec<usize>) -> Vec<usize> {
        work::add(2 * set.len() as u64);
        if set.iter().all(|&k| self.boxes[k].given.is_none()) {
            return set;
        }
        let cells = self.cells.borrow();
        let tree = self.tree.borrow();
        let side = |c: u32| tree.get(c as usize).is_some_and(|t| t.1);
        let up = |c: u32| tree.get(c as usize).map_or(0, |t| t.0);
        let n = set.len();
        let stated = |i: usize| self.boxes[set[i]].given;
        let cell = |i: usize| cells[set[i]];
        // The place of the given region before and after each place, in its cell.
        let (mut prev, mut next) = (vec![None; n], vec![None; n]);
        let mut seen: HashMap<u32, usize> = HashMap::new();
        for (i, p) in prev.iter_mut().enumerate() {
            if stated(i).is_some() {
                seen.insert(cell(i), i);
            } else {
                *p = seen.get(&cell(i)).copied();
            }
        }
        seen.clear();
        for (i, q) in next.iter_mut().enumerate().rev() {
            if stated(i).is_some() {
                seen.insert(cell(i), i);
            } else {
                *q = seen.get(&cell(i)).copied();
            }
        }
        // The given places in the given order (stable: one given region's parts keep their XY
        // order), and the rank of each.
        let mut ranked: Vec<usize> = (0..n).filter(|&i| stated(i).is_some()).collect();
        ranked.sort_by_key(|&i| stated(i));
        let mut rank = vec![0usize; n];
        for (r, &i) in ranked.iter().enumerate() {
            rank[i] = r;
        }
        // The given places of each column inside a column, from the columns they stand in up.
        let mut inner: HashMap<u32, Vec<usize>> = HashMap::new();
        for &i in &ranked {
            let mut c = cell(i);
            for _ in 0..=self.max_depth {
                if !side(c) || !side(up(c)) {
                    break;
                }
                c = up(c);
                inner.entry(c).or_default().push(i);
            }
        }
        // The given region of the enclosing columns an untagged region at `i` goes after.
        let enclosing = |i: usize| -> Option<usize> {
            let mut c = cell(i);
            for _ in 0..=self.max_depth {
                if !side(c) || !side(up(c)) {
                    return None;
                }
                c = up(c);
                if let Some(list) = inner.get(&c) {
                    let before = list.iter().copied().filter(|&g| g < i).max();
                    return before.or_else(|| list.iter().copied().max_by_key(|&g| rank[g]));
                }
            }
            None
        };
        let (mut before, mut after) = (vec![Vec::new(); n], vec![Vec::new(); n]);
        // The regions of cells without a given region: those before every given region, and
        // those after the group of each rank.
        let mut lead = Vec::new();
        let mut trail = vec![Vec::new(); ranked.len()];
        let mut highest: Option<usize> = None;
        for i in 0..n {
            if stated(i).is_some() {
                highest = highest.max(Some(rank[i]));
            } else if let Some(p) = prev[i] {
                after[p].push(set[i]);
            } else if let Some(q) = next[i] {
                before[q].push(set[i]);
            } else if let Some(g) = enclosing(i) {
                after[g].push(set[i]);
            } else if let Some(r) = highest {
                trail[r].push(set[i]);
            } else {
                lead.push(set[i]);
            }
        }
        let mut out = lead;
        for (r, &i) in ranked.iter().enumerate() {
            out.append(&mut before[i]);
            out.push(set[i]);
            out.append(&mut after[i]);
            out.append(&mut trail[r]);
        }
        out
    }

    /// A new cell cut from `parent`; `side` when it is a side of a gutter.
    fn new_cell(&self, parent: u32, side: bool) -> u32 {
        let mut tree = self.tree.borrow_mut();
        let id = u32::try_from(tree.len()).unwrap_or(u32::MAX);
        tree.push((parent, side));
        id
    }

    /// The box of the region at `k` (a placed one).
    fn b(&self, k: usize) -> Bounds {
        self.boxes[k].b.unwrap_or(Bounds {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        })
    }

    /// The places of `set` sorted top down, then left to right.
    fn by_top(&self, set: &mut [usize]) {
        set.sort_by(|&a, &b| {
            self.b(b)
                .y1
                .total_cmp(&self.b(a).y1)
                .then(self.b(a).x0.total_cmp(&self.b(b).x0))
                .then(a.cmp(&b))
        });
    }

    /// The XY order of `set`, `depth` cuts down, in the cell `cell`. At the bound of the depth
    /// the set is read by rows.
    fn xy(&self, set: Vec<usize>, depth: usize, cell: u32) -> Vec<usize> {
        work::add(1 + set.len() as u64);
        {
            let mut cells = self.cells.borrow_mut();
            for &k in &set {
                cells[k] = cell;
            }
        }
        if set.len() <= 1 {
            return set;
        }
        if depth >= self.max_depth {
            return self.by_rows(set);
        }
        if let Some((a, b)) = self.at_wide_rule(&set) {
            return [self.xy(a, depth + 1, cell), self.xy(b, depth + 1, cell)].concat();
        }
        if let Some(g) = self.gutter(&set) {
            // A gutter only a drawn box makes is taken after the bands of white space across:
            // text above or below the boxes is not part of their columns.
            if g.boxed
                && let Some((a, b)) = self.widest_band(&set)
            {
                return [self.xy(a, depth + 1, cell), self.xy(b, depth + 1, cell)].concat();
            }
            return self.columns(set, &g, depth, cell);
        }
        if let Some((a, b)) = self.widest_band(&set) {
            return [self.xy(a, depth + 1, cell), self.xy(b, depth + 1, cell)].concat();
        }
        self.by_rows(set)
    }

    /// The set parted at a wide drawn rule: the highest horizontal rule across [`WIDE_RULE`] of
    /// the set's width or more that no region of the set reaches across, with regions above
    /// and below it.
    fn at_wide_rule(&self, set: &[usize]) -> Option<(Vec<usize>, Vec<usize>)> {
        work::add((set.len() + self.page.rules.len()) as u64);
        let (lo, hi) = set.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &k| {
            (lo.min(self.b(k).x0), hi.max(self.b(k).x1))
        });
        let width = hi - lo;
        if width.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return None;
        }
        // The spans of the regions by their bottom, and the highest top of those up to each.
        let mut spans: Vec<(f32, f32)> =
            set.iter().map(|&k| (self.b(k).y0, self.b(k).y1)).collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut reach: Vec<f32> = Vec::with_capacity(spans.len());
        for s in &spans {
            let last = reach.last().copied().unwrap_or(f32::MIN);
            reach.push(last.max(s.1));
        }
        let flat = FLAT * self.em;
        let mut best: Option<f32> = None;
        for r in &self.page.rules {
            let y = 0.5 * (r.y0 + r.y1);
            let over = r.x0.max(r.x1).min(hi) - r.x0.min(r.x1).max(lo);
            let wide = over.is_finite() && over >= WIDE_RULE * width;
            let flat = (r.y1 - r.y0).abs() <= flat;
            if r.rect || !wide || !flat || !y.is_finite() || best.is_some_and(|b| b >= y) {
                continue;
            }
            // The regions whose bottom is below the rule; none of them may reach above it.
            let below = spans.partition_point(|s| s.0 < y);
            if below > 0 && below < spans.len() && reach[below - 1] <= y {
                best = Some(y);
            }
        }
        let y = best?;
        let (above, below): (Vec<usize>, Vec<usize>) =
            set.iter().partition(|&&k| self.b(k).y0 >= y);
        (!above.is_empty() && !below.is_empty()).then_some((above, below))
    }

    /// For each place in `set`, whether its region stands alone in its strip: regions whose
    /// heights overlap, directly or through others, share a strip.
    fn alone(&self, set: &[usize]) -> Vec<bool> {
        let mut order: Vec<usize> = (0..set.len()).collect();
        order.sort_by(|&a, &b| self.b(set[b]).y1.total_cmp(&self.b(set[a]).y1));
        let mut alone = vec![false; set.len()];
        let mut strip: Vec<usize> = Vec::new();
        let mut low = f32::MAX;
        for i in order {
            let b = self.b(set[i]);
            if !strip.is_empty() && b.y1 <= low {
                if let [one] = strip[..] {
                    alone[one] = true;
                }
                strip.clear();
                low = f32::MAX;
            }
            strip.push(i);
            low = low.min(b.y0);
        }
        if let [one] = strip[..] {
            alone[one] = true;
        }
        alone
    }

    /// A column gutter of `set`: white space at least [`GUTTER`] body ems wide from the set's
    /// top to its bottom, with regions on both sides that stand side by side. The columns are
    /// made of the regions that share their strip with another; only a region alone in its
    /// strip may reach across the gutter (a title, a figure or a table across the columns), and
    /// it parts the columns into bands. Of several gaps, only the widest (the leftmost of equal
    /// ones) is tried.
    fn gutter(&self, set: &[usize]) -> Option<Gutter> {
        work::add(2 * set.len() as u64);
        let alone = self.alone(set);
        let core: Vec<usize> = (0..set.len())
            .filter(|&i| !alone[i])
            .map(|i| set[i])
            .collect();
        if core.is_empty() {
            return None;
        }
        let mut xs: Vec<(f32, f32)> = core.iter().map(|&k| (self.b(k).x0, self.b(k).x1)).collect();
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let min = GUTTER * self.em;
        let mut gaps: Vec<(f32, f32)> = Vec::new();
        let mut reach = xs[0].1;
        for &(a, b) in &xs[1..] {
            if a - reach >= min {
                gaps.push((reach, a));
            }
            reach = reach.max(b);
        }
        // Only the widest gap is tried, as in the cut: a gap after it, where the widest one
        // fails, cuts blocks apart that read together (ruling P6-X3).
        let (g0, g1) = gaps
            .into_iter()
            .reduce(|best, g| if g.1 - g.0 > best.1 - best.0 { g } else { best })?;
        let span = |side: &[usize]| {
            side.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &k| {
                (lo.min(self.b(k).y0), hi.max(self.b(k).y1))
            })
        };
        let (left, right): (Vec<usize>, Vec<usize>) =
            core.iter().partition(|&&k| self.b(k).x1 <= g0);
        let (a, b) = (span(&left), span(&right));
        let overlap = a.1.min(b.1) - a.0.max(b.0);
        let smaller = (a.1 - a.0).min(b.1 - b.0);
        let side_by_side = if smaller > 0.0 {
            overlap >= SIDE_BY_SIDE * smaller
        } else {
            overlap >= 0.0
        };
        let drawn = |side: &[usize]| side.iter().all(|&k| self.boxes[k].drawn);
        side_by_side.then(|| Gutter {
            from: g0,
            to: g1,
            boxed: drawn(&left) || drawn(&right),
        })
    }

    /// The set read in columns at the gutter `g`: band by band, each band's left side and then
    /// its right side; a region across the gutter is a band of its own.
    fn columns(&self, mut set: Vec<usize>, g: &Gutter, depth: usize, cell: u32) -> Vec<usize> {
        self.by_top(&mut set);
        let mut out = Vec::with_capacity(set.len());
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let flush = |left: &mut Vec<usize>, right: &mut Vec<usize>, out: &mut Vec<usize>| {
            // Each side of the gutter is a cell of its own, a new one for each band.
            for side in [left, right] {
                let column = self.new_cell(cell, true);
                out.extend(self.xy(std::mem::take(side), depth + 1, column));
            }
        };
        for k in set {
            let b = self.b(k);
            if b.x1 <= g.from {
                left.push(k);
            } else if b.x0 >= g.to {
                right.push(k);
            } else {
                flush(&mut left, &mut right, &mut out);
                // A region across the gutter is a cell of its own: it binds to no given region
                // of the set above or below the columns.
                self.cells.borrow_mut()[k] = self.new_cell(cell, false);
                out.push(k);
            }
        }
        flush(&mut left, &mut right, &mut out);
        out
    }

    /// The set parted at its widest band of white space across: the regions above it and
    /// those below. Of bands of one width, the middle one, so that a column of evenly spaced
    /// regions is cut in halves and the cut stays shallow.
    fn widest_band(&self, set: &[usize]) -> Option<(Vec<usize>, Vec<usize>)> {
        work::add(2 * set.len() as u64);
        let mut set = set.to_vec();
        self.by_top(&mut set);
        let mut bands: Vec<(f32, usize)> = Vec::new();
        let mut low = f32::MAX;
        for i in 1..set.len() {
            low = low.min(self.b(set[i - 1]).y0);
            let gap = low - self.b(set[i]).y1;
            if gap > 0.0 {
                bands.push((gap, i));
            }
        }
        let widest = bands.iter().map(|b| b.0).fold(f32::MIN, f32::max);
        let near = 1e-3 * self.em;
        let wide: Vec<usize> = bands
            .iter()
            .filter(|b| b.0 >= widest - near)
            .map(|b| b.1)
            .collect();
        let at = *wide.get(wide.len() / 2)?;
        let below = set.split_off(at);
        Some((set, below))
    }

    /// Top to bottom, then left to right: a region whose top is within [`ROW_TOLERANCE`] of
    /// the smaller line height below the top of the first region of a row stands in that row.
    fn by_rows(&self, mut set: Vec<usize>) -> Vec<usize> {
        work::add(2 * set.len() as u64);
        self.by_top(&mut set);
        let line = |k: usize| self.boxes[k].line;
        let mut out = Vec::with_capacity(set.len());
        let mut row: Vec<usize> = Vec::new();
        let close = |row: &mut Vec<usize>, out: &mut Vec<usize>| {
            row.sort_by(|&a, &b| self.b(a).x0.total_cmp(&self.b(b).x0).then(a.cmp(&b)));
            out.append(row);
        };
        for k in set {
            let joins = row.first().is_some_and(|&first| {
                let h = match (line(first), line(k)) {
                    (Some(a), Some(b)) => a.min(b),
                    (Some(a), None) | (None, Some(a)) => a,
                    (None, None) => self.em,
                };
                self.b(first).y1 - self.b(k).y1 <= ROW_TOLERANCE * h
            });
            if !joins {
                close(&mut row, &mut out);
            }
            row.push(k);
        }
        close(&mut row, &mut out);
        out
    }
}
