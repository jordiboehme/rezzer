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
//! - A gutter at least 0.8 body em wide (as in the cut), from the set's top to its bottom,
//!   parts it into columns, read left to right, each down to its end. Only a region that
//!   stands in a band of its own may reach across the gutter (a title, a figure or a table
//!   across the columns): it parts the columns into bands, as a piece across a gutter does in
//!   the cut. A gutter only a drawn box makes is taken after the bands of white space across,
//!   as in the cut.
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
//!
//! [`order_by_source`] is the order for a file whose source order (the order it sets its
//! content down in) is its reading order: the body reads in that order, and only what the
//! source has no place for (a figure or code without text) is placed, by its column and height.

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

/// A column gutter is at least this many body ems wide: the cut's gutter, so a gutter
/// the cut reads between the lines of two columns parts their regions too (a column gap of
/// one em, which the ends of the lines of a justified column narrow a little).
const GUTTER: f32 = crate::segment::cut::GUTTER;

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

/// Placing the regions the source lacks by their column looks at the sourced regions once for
/// each of them, and at the content of each that can reach into its width (see `Width`). On
/// a page of many pictures and many regions that is their product; it may take at most this
/// many steps for each region and each box of content of the set, and the regions left past
/// that are placed by their height alone (as a page head is), so the work grows with the
/// page, not with pictures times regions. The busiest page of the corpus, forms and generated
/// sets (133 pictures beside 179 regions) took about a fifth of it when every box of content
/// left of a picture was looked at; it takes less now.
const PLACE_WORK: u64 = 1024;

thread_local! {
    /// A test's stand-in for [`PLACE_WORK`] on this thread (see [`with_place_work`]).
    static PLACE_WORK_SET: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with `per` steps of work for placing pictures by their column in place of
/// [`PLACE_WORK`], on this thread; `u64::MAX` places every picture by its column. For the tests
/// of the work bound only: they compare the order past the bound with the order without it,
/// and reach the bound with a small page.
#[doc(hidden)]
pub fn with_place_work<T>(per: u64, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            PLACE_WORK_SET.with(|w| w.set(self.0));
        }
    }
    let _restore = Restore(PLACE_WORK_SET.with(|w| w.replace(Some(per))));
    f()
}

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
    ordered(page, regions, &[], false)
}

/// The regions of a page in their source order (`source`: the order the file sets them down
/// in, for a PDF the order of its content), for a file whose source order is its reading
/// order: furniture first and last as in [`order`], the body in the source order (the regions
/// a file states too, as the source has them), with no XY cut. A region not in `source` (a
/// figure or code without text) stands before the first of them that stands below it (see
/// `Cx::by_source`).
pub fn order_by_source(page: &PageIn, regions: &[Region], source: &[RegionId]) -> Vec<RegionId> {
    ordered(page, regions, source, true)
}

/// [`order`], or [`order_by_source`] when `follow`.
fn ordered(page: &PageIn, regions: &[Region], source: &[RegionId], follow: bool) -> Vec<RegionId> {
    work::add(1 + regions.len() as u64 + source.len() as u64);
    let em = body_em(page);
    // In the source order the file's order of its content holds, given regions too.
    let given = if follow {
        HashMap::new()
    } else {
        given_orders(page, regions)
    };
    let frames = Frames::new(page, em);
    let cx = Cx {
        page,
        em,
        boxes: regions
            .iter()
            .map(|r| Info::new(page, r, em, &given, &frames, follow))
            .collect(),
        max_depth: Limits::default().max_cut_depth as usize,
        cells: RefCell::new(vec![0; regions.len()]),
        tree: RefCell::new(vec![(0, false)]),
        follow,
        source: {
            let rank: HashMap<RegionId, u32> = source
                .iter()
                .enumerate()
                .map(|(k, id)| (*id, u32::try_from(k).unwrap_or(u32::MAX)))
                .collect();
            regions.iter().map(|r| rank.get(&r.id).copied()).collect()
        },
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
    let body = cx.place(body);
    ordered.extend(if follow { body } else { cx.with_given(body) });
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
    /// The boxes of its content (lines, cells, rules); only in the source order, where a
    /// picture's column is judged by the content in its width. They stand class by class
    /// (see `widths`), and by their left edge within a class.
    content: Vec<Bounds>,
    /// The classes of `content` by width, each a stretch of it.
    widths: Vec<Width>,
    /// What all of `content` spans, for looks that can then leave the region out; `None`
    /// without content.
    span: Option<Span>,
}

/// The boxes of one class of width in the content of a region (see `Info::content`): those
/// whose widths have one binary exponent, so the widest is less than twice the narrowest. A
/// box can reach into a width only when it starts less than its own width left of it, so in
/// each class the boxes that can are one short stretch, found by two searches. With all boxes
/// in one list, one wide box (a rule across the page, a line set as one run) would make that
/// stretch everything left of the width. The classes are those of the number format, at most
/// one for each exponent, so there is no step to choose.
#[derive(Debug, Clone, Copy)]
struct Width {
    /// The stretch of `Info::content`.
    from: usize,
    to: usize,
    /// A width no box of the class is wider than (see [`width_over`]).
    wide: f64,
}

/// A width that is not under the width of the box, in `f64`: the difference of its edges,
/// one step up. The difference is exact for the coordinates of a page, but not for every
/// pair of numbers (an edge at -1e20 and one at 1 are 1e20 apart in `f64`, which loses the
/// 1), and a width rounded down would let [`reach`] leave out a box that reaches into a
/// width. The step up is at most one box more to look at.
fn width_over(c: &Bounds) -> f64 {
    (f64::from(c.x1) - f64::from(c.x0)).next_up()
}

/// The stretch of `class` (boxes of one class of width, by their left edge, none wider than
/// `wide`) that can reach into the width of `lb`: the boxes that start left of its right
/// edge, without those whose left edge plus `wide` is at or left of its left edge.
///
/// No box outside the stretch shares the width (see [`shares`]), for every finite input: a
/// box that shares it starts left of the right edge of `lb` and ends right of its left edge.
/// The sum of a left edge and `wide` is not under the right edge of the box, as `wide` is
/// not under its width; rounding the sum to `f64` keeps that, as the right edge is a number
/// of `f64` and rounding keeps the order. So a box whose rounded sum is at or left of the
/// left edge of `lb` ends there or before it. The rounded sum rises with the left edge, so
/// the boxes left out are the start of the class.
fn reach_of(class: &[Bounds], wide: f64, lb: &Bounds) -> (usize, usize) {
    let from = class.partition_point(|c| f64::from(c.x0) + wide <= f64::from(lb.x0));
    let to = class.partition_point(|c| c.x0 < lb.x1).max(from);
    (from, to)
}

/// Whether two spans share a column: they overlap across by half the narrower one or more
/// (boxes that only touch at their edges stand side by side).
fn shares(a: Bounds, b: Bounds) -> bool {
    let over = a.x1.min(b.x1) - a.x0.max(b.x0);
    over > 0.0 && over >= 0.5 * (a.x1 - a.x0).min(b.x1 - b.x0)
}

/// The class of a box by its width: the sign and exponent bits of the width, which rise with
/// it (the width of a box put in order is zero or more).
fn width_class(c: &Bounds) -> u32 {
    (c.x1 - c.x0).max(0.0).to_bits() >> 23
}

/// The sourced regions of a set by where they start across, to find those that reach a
/// width (see `Cx::by_source`).
struct Reach {
    /// The left edge and the place among the sourced regions of each whose edges are finite
    /// numbers, by the left edge.
    by_left: Vec<(f32, usize)>,
    /// The right edge of each of those, by its place among the sourced regions.
    right: Vec<f32>,
    /// The widest of them.
    widest: f32,
    /// The regions with an edge that is no finite number: they reach every width.
    loose: Vec<usize>,
}

impl Reach {
    /// `spans` gives the left and the right edge of each sourced region, in their order.
    fn new(spans: impl Iterator<Item = (f32, f32)>) -> Reach {
        let mut out = Reach {
            by_left: Vec::new(),
            right: Vec::new(),
            widest: 0.0,
            loose: Vec::new(),
        };
        for (i, (x0, x1)) in spans.enumerate() {
            out.right.push(x1);
            if x0.is_finite() && x1.is_finite() {
                out.by_left.push((x0, i));
                out.widest = out.widest.max(x1 - x0);
            } else {
                out.loose.push(i);
            }
        }
        out.by_left.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    /// Puts into `near` (empty) the places of the sourced regions that reach the width
    /// `x0..x1`, in their order: every region that starts at or left of `x1` and ends at
    /// or right of `x0` (one that only touches the width counts, so no region with content
    /// in the width or on both sides of it is left out). A region that reaches the width
    /// starts no more than the widest region left of it; the search starts a little
    /// further left (a ten-thousandth of the numbers at hand), so no rounding of the widths
    /// leaves one out. A width with an edge that is no finite number is reached by all.
    /// Returns the steps it took.
    fn over(&self, x0: f32, x1: f32, near: &mut Vec<usize>) -> u64 {
        let all = !(x0.is_finite() && x1.is_finite() && self.widest.is_finite());
        let left = x0 - self.widest;
        let left = left - 1e-4 * (left.abs() + self.widest + 1.0);
        let from = if all {
            0
        } else {
            self.by_left.partition_point(|r| r.0 < left)
        };
        let mut steps = 1 + u64::from(self.by_left.len().max(1).ilog2());
        for &(left, i) in &self.by_left[from..] {
            if !all && left > x1 {
                break;
            }
            steps += 1;
            if all || self.right[i] >= x0 {
                near.push(i);
            }
        }
        near.extend(&self.loose);
        steps += self.loose.len() as u64;
        near.sort_unstable();
        steps
    }
}

/// What the content of a region spans (see `Info::content`).
#[derive(Debug, Clone, Copy)]
struct Span {
    /// The lowest bottom and the highest top.
    y0: f32,
    y1: f32,
    /// The leftmost right edge and the rightmost left edge.
    x1_min: f32,
    x0_max: f32,
}

impl Span {
    fn of(content: &[Bounds]) -> Option<Span> {
        let first = content.first()?;
        let start = Span {
            y0: first.y0,
            y1: first.y1,
            x1_min: first.x1,
            x0_max: first.x0,
        };
        Some(content.iter().fold(start, |s, c| Span {
            y0: s.y0.min(c.y0),
            y1: s.y1.max(c.y1),
            x1_min: s.x1_min.min(c.x1),
            x0_max: s.x0_max.max(c.x0),
        }))
    }
}

impl Info {
    fn new(
        page: &PageIn,
        r: &Region,
        em: f32,
        given: &HashMap<RegionId, u32>,
        frames: &Frames,
        follow: bool,
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
        let mut content: Vec<Bounds> = if follow {
            r.prims
                .iter()
                .filter_map(|&p| crate::segment::prim_bounds(page, p))
                .map(normal)
                .filter(|c| [c.x0, c.y0, c.x1, c.y1].iter().all(|v| v.is_finite()))
                .collect()
        } else {
            Vec::new()
        };
        content.sort_by(|a, c| {
            width_class(a)
                .cmp(&width_class(c))
                .then(a.x0.total_cmp(&c.x0))
        });
        let mut widths: Vec<Width> = Vec::new();
        for (i, c) in content.iter().enumerate() {
            let wide = width_over(c);
            match widths.last_mut() {
                Some(w) if width_class(&content[w.from]) == width_class(c) => {
                    w.to = i + 1;
                    w.wide = w.wide.max(wide);
                }
                _ => widths.push(Width {
                    from: i,
                    to: i + 1,
                    wide,
                }),
            }
        }
        Info {
            widths,
            span: Span::of(&content),
            b,
            line: sizes.get(sizes.len() / 2).copied(),
            drawn: b.is_some_and(|b| framed(page, r, b, em) || frames.frame(b)),
            given: given.get(&r.id).copied(),
            content,
        }
    }
}

/// The sourced regions of a set by their height, for the height rule of `Cx::by_source`: the
/// place before the first region (in the source order) whose top is lower than a middle; with
/// none, just after the last one whose middle is above it; else the end. A binary search, so
/// that a page of many pictures places each in steps of the log of its regions.
struct Heights {
    /// The tops, rising, with the first place in the source order among those up to each.
    tops: Vec<(f32, usize)>,
    /// The middles, rising, with the last place in the source order among those from each on.
    mids: Vec<(f32, usize)>,
    len: usize,
}

impl Heights {
    fn new(sourced: &[usize], top: impl Fn(usize) -> f32, mid: impl Fn(usize) -> f32) -> Heights {
        work::add(2 * sourced.len() as u64);
        let mut tops: Vec<(f32, usize)> = sourced
            .iter()
            .enumerate()
            .map(|(i, &k)| (top(k), i))
            .collect();
        tops.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut first = usize::MAX;
        for t in &mut tops {
            first = first.min(t.1);
            t.1 = first;
        }
        let mut mids: Vec<(f32, usize)> = sourced
            .iter()
            .enumerate()
            .map(|(i, &k)| (mid(k), i))
            .collect();
        mids.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut last = 0;
        for t in mids.iter_mut().rev() {
            last = last.max(t.1);
            t.1 = last;
        }
        Heights {
            tops,
            mids,
            len: sourced.len(),
        }
    }

    /// The place in the source order a region whose middle is `m` stands before.
    fn place(&self, m: f32) -> usize {
        let lower = self.tops.partition_point(|t| t.0 < m);
        if lower > 0 {
            return self.tops[lower - 1].1;
        }
        let above = self.mids.partition_point(|t| t.0 <= m);
        self.mids.get(above).map_or(self.len, |t| t.1 + 1)
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
    /// The drawn boxes and rectangles, by their top and then their left edge.
    boxes: Vec<Bounds>,
    /// The horizontal rules, by height.
    flat: Rules,
    /// The vertical rules, by place.
    upright: Rules,
    /// How far an edge of a frame may lie from the edge of a region's box.
    near: f32,
}

/// The rules of a page in one direction as (place, from, to), by their place and then their
/// start, to ask whether one of those near a place spans a stretch. A row of small grids has
/// as many rules at one height as it has grids, and each region asked for all of them.
struct Rules {
    list: Vec<(f32, f32, f32)>,
    /// For each rule the farthest end of the rules at its own place up to it: the rules at
    /// a place that start at or before a point span a stretch when the farthest end among
    /// them reaches its end.
    ends: Vec<f32>,
}

impl Rules {
    /// `list` holds finite numbers only.
    fn new(mut list: Vec<(f32, f32, f32)>) -> Rules {
        list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut ends: Vec<f32> = Vec::with_capacity(list.len());
        for (k, r) in list.iter().enumerate() {
            let end = match k.checked_sub(1) {
                Some(j) if list[j].0.to_bits() == r.0.to_bits() => ends[j].max(r.2),
                _ => r.2,
            };
            ends.push(end);
        }
        Rules { list, ends }
    }

    /// Whether a rule within `near` of `place` spans `from` to `to`: it starts at or before
    /// `from + near` and ends at or after `to - near`. One search for each place with rules
    /// near `place`.
    fn spans(&self, place: f32, from: f32, to: f32, near: f32) -> bool {
        let list = &self.list;
        let mut at = list.partition_point(|r| r.0 < place - near);
        while at < list.len() && list[at].0 <= place + near {
            work::add(1);
            let here = list[at].0;
            let end = at
                + list[at..]
                    .partition_point(|r| r.0.total_cmp(&here).is_le())
                    .max(1);
            let starts = at + list[at..end].partition_point(|r| r.1 <= from + near);
            if starts > at && self.ends[starts - 1] >= to - near {
                return true;
            }
            at = end;
        }
        false
    }
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
        boxes.sort_by(|a, b| a.y1.total_cmp(&b.y1).then(a.x0.total_cmp(&b.x0)));
        Frames {
            boxes,
            flat: Rules::new(flat),
            upright: Rules::new(upright),
            near: FRAME * em,
        }
    }

    /// Whether a drawn box, or four rules, frame `b`: each edge within [`FRAME`] body ems of
    /// the edge of `b`.
    fn frame(&self, b: Bounds) -> bool {
        let near = self.near;
        let at = |v: f32, w: f32| (v - w).abs() <= near;
        // The boxes with their top near its top, top by top (they stand by their left edge
        // at each top): those whose left edge is near its left edge. The stretch searched
        // is twice as wide as `near`, so no rounding leaves a box out; each is tested.
        let boxes = &self.boxes;
        let mut lo = boxes.partition_point(|x| x.y1 < b.y1 - near);
        while lo < boxes.len() && boxes[lo].y1 <= b.y1 + near {
            work::add(1);
            let top = boxes[lo].y1;
            let end = lo
                + boxes[lo..]
                    .partition_point(|x| x.y1.total_cmp(&top).is_le())
                    .max(1);
            let from = lo + boxes[lo..end].partition_point(|x| x.x0 < b.x0 - 2.0 * near);
            let boxed = boxes[from..end]
                .iter()
                .take_while(|x| x.x0 <= b.x0 + 2.0 * near)
                .inspect(|_| work::add(1))
                .any(|x| at(x.x0, b.x0) && at(x.x1, b.x1) && at(x.y0, b.y0));
            if boxed {
                return true;
            }
            lo = end;
        }
        self.flat.spans(b.y1, b.x0, b.x1, near)
            && self.flat.spans(b.y0, b.x0, b.x1, near)
            && self.upright.spans(b.x0, b.y0, b.y1, near)
            && self.upright.spans(b.x1, b.y0, b.y1, near)
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
    /// The place of each region in the source order, if it has one.
    source: Vec<Option<u32>>,
    /// The source order is the reading order: no cut is made where it holds.
    follow: bool,
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
        if self.follow {
            return self.by_source(set);
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
        // fails, cuts blocks apart that read together.
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
    /// its right side; a region across the gutter is a band of its own. So is a region alone in
    /// its strip whose box shares no width with the regions of its side that stand beside the
    /// other side: it stands above or below that column, not in it (a pair further left under a
    /// column of labels beside their values), and the columns end at it.
    fn columns(&self, mut set: Vec<usize>, g: &Gutter, depth: usize, cell: u32) -> Vec<usize> {
        let alone = self.alone(&set);
        // The widths the regions of each side that share a strip with another take, merged and
        // sorted.
        let widths = |left: bool| {
            let mut xs: Vec<(f32, f32)> = set
                .iter()
                .zip(&alone)
                .filter(|&(_, &a)| !a)
                .map(|(&k, _)| self.b(k))
                .filter(|b| if left { b.x1 <= g.from } else { b.x0 >= g.to })
                .map(|b| (b.x0, b.x1))
                .collect();
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut merged: Vec<(f32, f32)> = Vec::with_capacity(xs.len());
            for (a, b) in xs {
                match merged.last_mut() {
                    Some(m) if a <= m.1 => m.1 = m.1.max(b),
                    _ => merged.push((a, b)),
                }
            }
            merged
        };
        let (left_widths, right_widths) = (widths(true), widths(false));
        // Whether the box from `x0` to `x1` shares width with one of `merged`.
        let shares = |merged: &[(f32, f32)], x0: f32, x1: f32| {
            let i = merged.partition_point(|m| m.1 <= x0);
            merged.get(i).is_some_and(|m| m.0 < x1)
        };
        let mut off: Vec<usize> = set
            .iter()
            .zip(&alone)
            .filter(|&(&k, &a)| {
                let b = self.b(k);
                a && if b.x1 <= g.from {
                    !shares(&left_widths, b.x0, b.x1)
                } else if b.x0 >= g.to {
                    !shares(&right_widths, b.x0, b.x1)
                } else {
                    false
                }
            })
            .map(|(&k, _)| k)
            .collect();
        off.sort_unstable();
        work::add(2 * set.len() as u64);
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
            if off.binary_search(&k).is_err() && b.x1 <= g.from {
                left.push(k);
            } else if off.binary_search(&k).is_err() && b.x0 >= g.to {
                right.push(k);
            } else {
                flush(&mut left, &mut right, &mut out);
                // A region across the gutter, or off its side, is a cell of its own: it binds
                // to no given region of the set above or below the columns.
                self.cells.borrow_mut()[k] = self.new_cell(cell, false);
                out.push(k);
            }
        }
        flush(&mut left, &mut right, &mut out);
        out
    }

    /// The set in the source order: the regions with a place in it in that order. Each other
    /// one (a figure or code without text) is placed by its column: the content (lines, cells,
    /// rules) of the sourced regions that overlaps its width by half the narrower of the two.
    /// A region is of its column by that content, and stands over or under it by that
    /// content's top, not by the region's box (a region the segmenter joined across a gutter
    /// is of a column only by its content there).
    ///
    /// - A region holds it (its content stands in its width over its top and under its
    ///   bottom, or at its height on its left and on its right): it stands before that region,
    ///   which is not cut. A region whose box only reaches around it does not hold it.
    /// - Its column's reading began over it (content of its width starts above its top): it
    ///   stands after the last such region and before the first region under it in its column
    ///   (the middle or the foot of a column). When the source reads, between those two, a
    ///   region of another column that stands across its top (a column read down through its
    ///   height), it reads column by column there: the picture stands just before the first
    ///   region under it. Else (labels beside it, regions below it: a source read band by
    ///   band) before the first of them below its middle. A source that reads past it in its
    ///   column before all that is over it (a figure across the columns, the source read column
    ///   by column) meets it at the first crossing: before the first region under it.
    /// - Nothing of its width is over it, but text of other columns is (the top of a column):
    ///   it stands before the first region under it in its column.
    /// - Nothing on the page is over it (a page head), or nothing of its width is under it:
    ///   before the first region whose top is lower than its middle; with none, just after the
    ///   last one whose middle is above its own; else at the end, highest middle first.
    ///
    /// At the top of its column (text of its width under it), text of another column stands
    /// across its top, or over it, when it starts above its top, or within one line of it (that
    /// region's text size, else the body em) and its column is read down past the picture: it,
    /// or a region of its width the source reads after it (a heading over the column's first
    /// paragraph) before it reaches the picture's column, reaches lower than where the
    /// picture's column goes on under it. Columns start level, and a line's box starts at its
    /// ascender, lower than the frame edge a picture is set at by less than a line. A label
    /// beside it, or a page head read band by band, ends above where its column goes on.
    /// A region of the picture's own width counts the same way: a paragraph whose box starts
    /// level with the picture's top runs beside it and then under it (the picture is set into
    /// its top), and the picture stands before it, as it does when that box starts above the
    /// top. Leaving it out only within the line would turn the place on a fraction of a point.
    /// At the foot of its column (nothing of its width under it) only text that starts above
    /// its top stands across it.
    fn by_source(&self, set: Vec<usize>) -> Vec<usize> {
        work::add(2 * set.len() as u64);
        let (mut sourced, mut loose): (Vec<usize>, Vec<usize>) =
            set.into_iter().partition(|&k| self.source[k].is_some());
        sourced.sort_by_key(|&k| self.source[k]);
        let mid = |k: usize| 0.5 * (self.b(k).y0 + self.b(k).y1);
        loose.sort_by(|&a, &b| mid(b).total_cmp(&mid(a)).then(a.cmp(&b)));
        // Each loose region goes before the sourced place `before[k]`, or at the end.
        let mut before: Vec<Vec<usize>> = vec![Vec::new(); sourced.len() + 1];
        let by_height = Heights::new(&sourced, |k| self.b(k).y1, mid);
        let content: usize = sourced.iter().map(|&k| self.boxes[k].content.len()).sum();
        let per = PLACE_WORK_SET.with(|w| w.get()).unwrap_or(PLACE_WORK);
        let budget = per.saturating_mul((sourced.len() + content + loose.len()) as u64);
        // The steps spent placing by column so far.
        let spent = std::cell::Cell::new(0u64);
        let add = |n: u64| {
            work::add(n);
            spent.set(spent.get().saturating_add(n));
        };
        // Set when the budget runs out while a picture is placed by its column (in the scan of
        // `level`): that picture is then placed by its height, as those after it are.
        let out_of_work = std::cell::Cell::new(false);
        // The sourced regions by where their content starts across (the region's box for
        // one without content), with the widest of them: only those that reach the width of
        // a loose region can be of its column, hold it or stand around it, so each loose
        // region looks at those and not at every sourced region (a row of small grids has a
        // picture and a region for each grid). A region whose content has an edge that is
        // no finite number is looked at by all.
        let reach = Reach::new(sourced.iter().map(|&k| {
            let info = &self.boxes[k];
            if info.content.is_empty() {
                let b = self.b(k);
                (b.x0, b.x1)
            } else {
                info.content
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(x0, x1), c| {
                        (x0.min(c.x0), x1.max(c.x1))
                    })
            }
        }));
        work::add((sourced.len() + content) as u64);
        // The parts of the sourced regions in the width of the loose region before, with that
        // width and the regions that reach it: the parts depend on the width alone, and the
        // pictures of one column, or the rows of one stack of boxes, share theirs. Every
        // region that does not reach the width has no part.
        let mut parts: Vec<Option<Bounds>> = vec![None; sourced.len()];
        let mut near: Vec<usize> = Vec::new();
        let mut kept: Option<(f32, f32)> = None;
        for &l in &loose {
            let (m, top) = (mid(l), self.b(l).y1);
            if spent.get() > budget {
                work::add(1 + sourced.len().max(1).ilog2() as u64);
                before[by_height.place(m)].push(l);
                continue;
            }
            let lb = self.b(l);
            if kept != Some((lb.x0, lb.x1)) {
                for &i in &near {
                    parts[i] = None;
                }
                near.clear();
                add(reach.over(lb.x0, lb.x1, &mut near));
            }
            add(1 + near.len() as u64);
            // The part of each sourced region in its column: the bounds of its content (lines,
            // cells, rules) that shares the loose region's width; a region without content
            // stands by its box. A region whose box spans the gutter but whose content in this
            // width is one short paragraph is of this column by that paragraph only.
            // Only the content that can reach into its width is looked at: in each class of
            // width, the stretch of [`reach`]. The parts of the width before are kept: they
            // depend on the width alone.
            if kept != Some((lb.x0, lb.x1)) {
                for &i in &near {
                    let k = sourced[i];
                    let info = &self.boxes[k];
                    if info.content.is_empty() {
                        parts[i] = shares(lb, self.b(k)).then(|| self.b(k));
                        continue;
                    }
                    let mut part: Option<Bounds> = None;
                    for w in &info.widths {
                        let class = &info.content[w.from..w.to];
                        let (from, to) = reach_of(class, w.wide, &lb);
                        add((to - from) as u64 + 2 * u64::from(class.len().ilog2()) + 1);
                        for c in class[from..to].iter().filter(|c| shares(lb, **c)) {
                            part = Some(part.map_or(*c, |a| Bounds {
                                x0: a.x0.min(c.x0),
                                y0: a.y0.min(c.y0),
                                x1: a.x1.max(c.x1),
                                y1: a.y1.max(c.y1),
                            }));
                        }
                    }
                    parts[i] = part;
                }
                kept = Some((lb.x0, lb.x1));
            }
            let (parts, near) = (&parts, &near);
            // Below it: a region whose top is lower than its middle.
            let below = |k: usize| self.b(k).y1 < m;
            // Of its column: content in its width; over it: that content starts above its top;
            // under it: that content starts below its middle.
            let over = |i: usize| parts[i].is_some_and(|p| p.y1 > top);
            let under = |i: usize| parts[i].is_some_and(|p| p.y1 < m);
            // The sourced region `i` stands at or over its top: it starts above its top, or
            // within one line of it (that region's text size, else the body em) and its column
            // is read down past the picture: it, or a region of its width the source reads
            // after it and before `end` (the first region under the picture in its column),
            // reaches lower than the top of `end`. Columns start level, and a line's box starts
            // at its ascender, lower than the frame edge a picture is set at by less than a
            // line, so a picture flush with the top of its column stands up to a line above the
            // other column's first line. A label beside the picture, or a page head the source
            // reads band by band, ends above where the picture's column goes on. `i` may be of
            // the picture's own width (`end` itself: a paragraph the picture is set into the top
            // of, beside it and then under it); it counts as it does when its box starts above
            // the top.
            let level = |i: usize, end: usize| {
                let k = sourced[i];
                let (b, line) = (self.b(k), self.boxes[k].line.unwrap_or(self.em));
                if b.y1 > top {
                    return true;
                }
                if b.y1 <= top - line {
                    return false;
                }
                let Some(floor) = parts.get(end).copied().flatten().map(|p| p.y1) else {
                    return false;
                };
                if b.y0 < floor {
                    return true;
                }
                // The scan is counted in the budget too: many regions in a row level with its
                // top each look at the rest of the row.
                if spent.get() > budget {
                    out_of_work.set(true);
                    return false;
                }
                add(end.saturating_sub(i) as u64);
                (i + 1..end).any(|j| {
                    let c = self.b(sourced[j]);
                    parts[j].is_none() && c.y0 < floor && shares(b, c)
                })
            };
            // Only a region that reaches its width has a part there.
            let last_over = near.iter().rev().copied().find(|&i| over(i));
            let first_under = |from: usize| {
                near[near.partition_point(|&i| i < from)..]
                    .iter()
                    .copied()
                    .find(|&i| under(i))
            };
            // A region holds it when its content stands around it: in its width over its top
            // and under its bottom (a picture inside a block of text kept as laid out), or at
            // its height on its left and on its right (a code in a row of a table, between its
            // cells). The region is not cut, and the picture stands before it. A region whose
            // box only reaches around it (content of two columns joined) does not hold it.
            // A region whose content does not reach its height, or stands all on one side of
            // it, is left out by what its content spans. In the others, class by class, the
            // content on its right is the end of the class (sorted by the left edge) and the
            // content on its left is in the start of it, and each look ends at the first box
            // at its height.
            let row_around = |k: usize| {
                add(1);
                let info = &self.boxes[k];
                let Some(span) = info.span else {
                    return false;
                };
                if span.y0 >= lb.y1
                    || span.y1 <= lb.y0
                    || span.x1_min > lb.x0
                    || span.x0_max < lb.x1
                {
                    return false;
                }
                let at_height = |b: &Bounds| b.y0 < lb.y1 && b.y1 > lb.y0;
                let mut seen = 0u64;
                let mut classes = info.widths.iter().map(|w| &info.content[w.from..w.to]);
                let right = classes.clone().any(|c| {
                    seen += 2 * u64::from(c.len().ilog2()) + 1;
                    c[c.partition_point(|b| b.x0 < lb.x1)..]
                        .iter()
                        .inspect(|_| seen += 1)
                        .any(at_height)
                });
                let around = right
                    && classes.any(|c| {
                        seen += 2 * u64::from(c.len().ilog2()) + 1;
                        c[..c.partition_point(|b| b.x0 <= lb.x0)]
                            .iter()
                            .inspect(|_| seen += 1)
                            .any(|b| at_height(b) && b.x1 <= lb.x0)
                    });
                add(seen);
                around
            };
            // A region with content on its left and on its right reaches its width too.
            let holder = near.iter().copied().find(|&i| {
                parts[i].is_some_and(|p| p.y1 >= lb.y1 && p.y0 <= lb.y0) || row_around(sourced[i])
            });
            let at = match (holder, last_over) {
                (Some(h), _) => h,
                // Its column's reading began over it. A source that reads on past it in its
                // column before reaching all that is over it reads across it column by column
                // (a figure across the columns): before the first region under it.
                (None, Some(i)) if first_under(0).is_some_and(|u| u < i) => {
                    first_under(0).unwrap_or(sourced.len())
                }
                // After the last region over it, before the first under it in its column. When
                // the source reads, between those, a region of another column that stands across
                // its top (over its top and under it: a column read down through its height), it
                // reads column by column there: just before the first under it. Else (a label
                // beside it, regions below it: a source read band by band) before the first of
                // them below its middle.
                (None, Some(i)) => {
                    let from = i + 1;
                    let next_under = first_under(from);
                    let to = next_under.unwrap_or(sourced.len());
                    // Level with its top counts only at the top of a column, with text of its
                    // width under it; at the foot of its column only text over its top does.
                    let across = |j: usize| {
                        let b = self.b(sourced[j]);
                        (if next_under.is_some() {
                            level(j, to)
                        } else {
                            b.y1 > top
                        }) && b.y0 < top
                    };
                    // The regions the source reads between the two are looked at one by one.
                    add(to.saturating_sub(from) as u64);
                    if (from..to).any(across) {
                        to
                    } else {
                        (from..to).find(|&j| below(sourced[j])).unwrap_or(to)
                    }
                }
                // The top of its column: other columns have text over it, or level with its
                // top (or the paragraph under it starts level with its top, beside it). Before
                // the first region under it in its column.
                (None, None)
                    if first_under(0).is_some_and(|end| {
                        // Any sourced region may stand level with its top: they are looked
                        // at one by one, up to the first that does.
                        let mut looked = 0u64;
                        let level_with = (0..sourced.len()).any(|i| {
                            looked += 1;
                            level(i, end)
                        });
                        add(looked);
                        level_with
                    }) =>
                {
                    first_under(0).unwrap_or(sourced.len())
                }
                // A page head (nothing over it), or nothing of its width: by its height.
                (None, None) => by_height.place(m),
            };
            let at = if out_of_work.get() {
                by_height.place(m)
            } else {
                at
            };
            before[at].push(l);
        }
        let mut out = Vec::with_capacity(sourced.len() + loose.len());
        for (i, k) in sourced.into_iter().enumerate() {
            out.append(&mut before[i]);
            out.push(k);
        }
        out.append(&mut before.last_mut().map(std::mem::take).unwrap_or_default());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether a drawn box, or four rules, frame `b`, as it was asked by a plain scan: every
    /// box and every rule near an edge looked at.
    fn frame_one_by_one(
        boxes: &[Bounds],
        flat: &[(f32, f32, f32)],
        upright: &[(f32, f32, f32)],
        near: f32,
        b: Bounds,
    ) -> bool {
        let at = |v: f32, w: f32| (v - w).abs() <= near;
        let boxed = boxes.iter().any(|x| {
            x.y1 >= b.y1 - near
                && x.y1 <= b.y1 + near
                && at(x.x0, b.x0)
                && at(x.x1, b.x1)
                && at(x.y0, b.y0)
        });
        let spans = |list: &[(f32, f32, f32)], place: f32, from: f32, to: f32| {
            list.iter().any(|r| {
                r.0 >= place - near && r.0 <= place + near && r.1 <= from + near && r.2 >= to - near
            })
        };
        boxed
            || (spans(flat, b.y1, b.x0, b.x1)
                && spans(flat, b.y0, b.x0, b.x1)
                && spans(upright, b.x0, b.y0, b.y1)
                && spans(upright, b.x1, b.y0, b.y1))
    }

    /// The frame of a region found by a search among the boxes and rules at its
    /// edges is the frame every box and rule gives: on pages of boxes and rules on a grid
    /// of 10 points, some a little off it (within the reach of a frame, at it and past it),
    /// asked for boxes on the same grid.
    #[test]
    fn a_frame_found_by_search_is_the_frame_every_box_and_rule_gives() {
        fn roll(seed: &mut u64, n: u32) -> f32 {
            *seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((*seed >> 33) % u64::from(n)) as f32
        }
        // A place on the grid moved by -1, 0, 1 or 1.5 points: the reach of a frame is 2.
        fn off(seed: &mut u64, v: f32) -> f32 {
            v + [0.0, 0.0, 1.0, 2.0, 2.5][roll(seed, 5) as usize] - 1.0
        }
        fn place(seed: &mut u64) -> (f32, f32, f32, f32) {
            let (x, y) = (10.0 * roll(seed, 8), 10.0 * roll(seed, 8));
            (
                x,
                y,
                x + 10.0 * (1.0 + roll(seed, 3)),
                y + 10.0 * (1.0 + roll(seed, 3)),
            )
        }
        let seed = &mut 7u64;
        let (mut yes, mut no) = (0, 0);
        for _ in 0..30 {
            let em = 4.0;
            let near = FRAME * em;
            let mut page = PageIn {
                page: 1,
                size: [200.0, 200.0],
                turn: 0,
                texts: Vec::new(),
                rules: Vec::new(),
                boxes: Vec::new(),
                pictures: Vec::new(),
                codes: Vec::new(),
                given: Vec::new(),
            };
            for _ in 0..12 {
                let (x0, y0, x1, y1) = place(seed);
                page.boxes.push(crate::BoxIn {
                    bounds: Bounds {
                        x0: off(seed, x0),
                        y0: off(seed, y0),
                        x1: off(seed, x1),
                        y1: off(seed, y1),
                    },
                    filled: false,
                    stroked: true,
                    widget: false,
                });
            }
            for k in 0..60 {
                let (x0, y0, x1, y1) = place(seed);
                // Flat rules along a top or a bottom, upright ones along a side.
                let (x0, y0, x1, y1) = match k % 4 {
                    0 => {
                        let y = off(seed, y1);
                        (x0, y, x1, y)
                    }
                    1 => {
                        let y = off(seed, y0);
                        (x0, y, x1, y)
                    }
                    2 => {
                        let x = off(seed, x0);
                        (x, y0, x, y1)
                    }
                    _ => {
                        let x = off(seed, x1);
                        (x, y0, x, y1)
                    }
                };
                page.rules.push(crate::RuleIn {
                    x0,
                    y0,
                    x1,
                    y1,
                    thick: 0.5,
                    dashed: false,
                    rect: false,
                });
            }
            let frames = Frames::new(&page, em);
            let boxes: Vec<Bounds> = page.boxes.iter().map(|x| normal(x.bounds)).collect();
            let flat: Vec<(f32, f32, f32)> = page
                .rules
                .iter()
                .filter(|r| r.y0 == r.y1)
                .map(|r| (r.y0, r.x0, r.x1))
                .collect();
            let upright: Vec<(f32, f32, f32)> = page
                .rules
                .iter()
                .filter(|r| r.x0 == r.x1)
                .map(|r| (r.x0, r.y0, r.y1))
                .collect();
            for _ in 0..300 {
                let (x0, y0, x1, y1) = place(seed);
                let b = Bounds { x0, y0, x1, y1 };
                let want = frame_one_by_one(&boxes, &flat, &upright, near, b);
                assert_eq!(frames.frame(b), want, "{b:?}");
                if want {
                    yes += 1;
                } else {
                    no += 1;
                }
            }
        }
        assert!(yes > 30 && no > 30, "{yes} framed, {no} not");
    }

    fn span(x0: f32, x1: f32) -> Bounds {
        Bounds {
            x0,
            y0: 0.0,
            x1,
            y1: 1.0,
        }
    }

    /// The classes of `boxes` as [`Info::new`] builds them: each a stretch by left edge, with
    /// a width none of its boxes is wider than.
    fn classes(mut boxes: Vec<Bounds>) -> Vec<(Vec<Bounds>, f64)> {
        boxes.sort_by(|a, c| {
            width_class(a)
                .cmp(&width_class(c))
                .then(a.x0.total_cmp(&c.x0))
        });
        boxes
            .chunk_by(|a, c| width_class(a) == width_class(c))
            .map(|class| {
                let wide = class.iter().map(width_over).fold(f64::MIN, f64::max);
                (class.to_vec(), wide)
            })
            .collect()
    }

    /// A box from -1e20 to 1 reaches into the
    /// width from 0.5 to 0.9. Its width in `f64` is 1e20, which loses the 1, and its left
    /// edge plus that width is 0: with the width as it was the search left the box out. The
    /// width one step up keeps it.
    #[test]
    fn a_box_from_far_off_the_page_that_reaches_into_a_width_is_looked_at() {
        let (far, lb) = (span(-1e20, 1.0), span(0.5, 0.9));
        assert!(shares(lb, far));
        let rounded = f64::from(far.x1) - f64::from(far.x0);
        // Left out: the stretch starts after it.
        assert_eq!(reach_of(&[far], rounded, &lb), (1, 1));
        assert!(width_over(&far) > rounded);
        assert_eq!(reach_of(&[far], width_over(&far), &lb), (0, 1));
        // A box that ends at the left edge of the width, or starts at its right edge, only
        // touches it; the search may look at it, the answer is the same.
        assert!(!shares(lb, span(0.0, 0.5)) && !shares(lb, span(0.9, 2.0)));
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

        /// A coordinate: mostly on a page (a lattice of half points, so that edges meet),
        /// sometimes far off it, tiny, or huge.
        fn place(&mut self) -> f32 {
            let odd = [
                1e20,
                -1e20,
                3e38,
                -3e38,
                1e-30,
                -1e-30,
                16_777_216.0,
                16_777_217.0,
                1e9,
                -1e9,
            ];
            match self.next() % 8 {
                0 => odd[(self.next() % odd.len() as u64) as usize],
                1 => (self.next() % 2_000) as f32 * 0.37 - 100.0,
                _ => (self.next() % 80) as f32 * 0.5,
            }
        }

        fn span(&mut self) -> Bounds {
            let (a, b) = (self.place(), self.place());
            span(a.min(b), a.max(b))
        }
    }

    /// Every box that shares a width is in the stretch the search looks at, as the walk of
    /// every box finds it: boxes on a page, with edges that meet, and boxes with edges far
    /// off the page, where sums and differences round.
    #[test]
    fn the_search_by_width_class_leaves_out_no_box_that_shares_the_width() {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        let (mut shared, mut left_out) = (0usize, 0usize);
        for round in 0..2_000 {
            let boxes: Vec<Bounds> = (0..1 + rng.next() % 24).map(|_| rng.span()).collect();
            let classes = classes(boxes);
            for _ in 0..8 {
                let lb = rng.span();
                for (class, wide) in &classes {
                    let (from, to) = reach_of(class, *wide, &lb);
                    assert!(from <= to && to <= class.len());
                    for (i, c) in class.iter().enumerate() {
                        let inside = (from..to).contains(&i);
                        assert!(
                            inside || !shares(lb, *c),
                            "round {round}: {c:?} shares {lb:?}, outside {from}..{to} of {class:?}"
                        );
                        shared += usize::from(shares(lb, *c));
                        left_out += usize::from(!inside);
                    }
                }
            }
        }
        // The test has boxes that share a width, and the search leaves boxes out.
        assert!(shared > 1_000 && left_out > 1_000, "{shared} {left_out}");
    }
}
