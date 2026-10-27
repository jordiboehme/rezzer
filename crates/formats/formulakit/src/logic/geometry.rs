// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rectangles of cells, the sorted per-sheet index of cells and the sweep that counts reads.

use super::stop::Stopper;
use crate::CellPos;
use crate::refs::{MAX_COL, col_letters};

/// A rectangle of cells on one sheet, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct Rect {
    pub(super) sheet: u32,
    pub(super) r1: u32,
    pub(super) c1: u32,
    pub(super) r2: u32,
    pub(super) c2: u32,
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        self.sheet == o.sheet
            && self.r1 <= o.r2
            && o.r1 <= self.r2
            && self.c1 <= o.c2
            && o.c1 <= self.c2
    }

    fn is_cell(&self) -> bool {
        self.r1 == self.r2 && self.c1 == self.c2
    }

    /// `self` without the cells of `o`, as up to four rectangles.
    fn subtract(self, o: &Rect, out: &mut Vec<Rect>) {
        if !self.overlaps(o) {
            out.push(self);
            return;
        }
        if self.r1 < o.r1 {
            out.push(Rect {
                r2: o.r1 - 1,
                ..self
            });
        }
        if self.r2 > o.r2 {
            out.push(Rect {
                r1: o.r2 + 1,
                ..self
            });
        }
        let (r1, r2) = (self.r1.max(o.r1), self.r2.min(o.r2));
        if self.c1 < o.c1 {
            out.push(Rect {
                r1,
                r2,
                c2: o.c1 - 1,
                ..self
            });
        }
        if self.c2 > o.c2 {
            out.push(Rect {
                r1,
                r2,
                c1: o.c2 + 1,
                ..self
            });
        }
    }
}

/// The same cells as `rects` in fewer rectangles: rectangles with the same columns whose
/// rows overlap or touch become one, then the same for rows. Nested ranges (running
/// totals) and copies that read the next cell down collapse this way.
pub(super) fn merge(mut rects: Vec<Rect>) -> Vec<Rect> {
    rects.sort_unstable_by_key(|r| (r.sheet, r.c1, r.c2, r.r1, r.r2));
    let mut by_cols: Vec<Rect> = Vec::with_capacity(rects.len());
    for r in rects {
        match by_cols.last_mut() {
            Some(l)
                if l.sheet == r.sheet
                    && l.c1 == r.c1
                    && l.c2 == r.c2
                    && r.r1 <= l.r2.saturating_add(1) =>
            {
                l.r2 = l.r2.max(r.r2);
            }
            _ => by_cols.push(r),
        }
    }
    by_cols.sort_unstable_by_key(|r| (r.sheet, r.r1, r.r2, r.c1, r.c2));
    let mut out: Vec<Rect> = Vec::with_capacity(by_cols.len());
    for r in by_cols {
        match out.last_mut() {
            Some(l)
                if l.sheet == r.sheet
                    && l.r1 == r.r1
                    && l.r2 == r.r2
                    && r.c1 <= l.c2.saturating_add(1) =>
            {
                l.c2 = l.c2.max(r.c2);
            }
            _ => out.push(r),
        }
    }
    out
}

/// The same cells as `rects`, each covered once, so a formula counts a cell once. Returns
/// `None` when the stop callback asks to stop.
pub(super) fn disjoint(rects: Vec<Rect>, stopper: &mut Stopper) -> Option<Vec<Rect>> {
    let rects = merge(rects);
    let (cells, ranges): (Vec<Rect>, Vec<Rect>) = rects.into_iter().partition(Rect::is_cell);
    let mut out: Vec<Rect> = Vec::new();
    let mut pieces = Vec::new();
    let mut next = Vec::new();
    for r in ranges {
        pieces.clear();
        pieces.push(r);
        for o in &out {
            if stopper.tick() {
                return None;
            }
            next.clear();
            for p in pieces.drain(..) {
                p.subtract(o, &mut next);
            }
            std::mem::swap(&mut pieces, &mut next);
            if pieces.is_empty() {
                break;
            }
        }
        out.extend(pieces.iter().copied());
    }
    let range_count = out.len();
    for c in cells {
        if stopper.tick() {
            return None;
        }
        // `merge` leaves no duplicate cells, so only ranges can cover a cell twice.
        if !out[..range_count].iter().any(|o| c.overlaps(o)) {
            out.push(c);
        }
    }
    Some(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PointKind {
    Formula(u32),
    Constant,
}

/// The formula and constant cells of one sheet, sorted by row, then column.
#[derive(Default)]
pub(super) struct SheetIndex {
    pub(super) points: Vec<(CellPos, PointKind)>,
    /// Each row that holds a cell, with the index of its first cell in `points`.
    rows: Vec<(u32, usize)>,
}

impl SheetIndex {
    pub(super) fn build(mut points: Vec<(CellPos, PointKind)>) -> Self {
        points.sort_by_key(|(p, k)| {
            (
                *p,
                match k {
                    PointKind::Formula(f) => u64::from(*f),
                    PointKind::Constant => u64::MAX,
                },
            )
        });
        points.dedup();
        let mut rows = Vec::new();
        for (i, (p, _)) in points.iter().enumerate() {
            if rows.last().is_none_or(|(r, _)| *r != p.row) {
                rows.push((p.row, i));
            }
        }
        SheetIndex { points, rows }
    }

    /// Calls `visit` with the index of each cell inside the rectangle. Visits only rows
    /// that hold cells; returns false when `visit` asks to stop.
    fn each_in(&self, r: &Rect, mut visit: impl FnMut(usize) -> bool) -> bool {
        let first = self.rows.partition_point(|(row, _)| *row < r.r1);
        for (n, (row, start)) in self.rows.iter().enumerate().skip(first) {
            if *row > r.r2 {
                break;
            }
            let end = self
                .rows
                .get(n + 1)
                .map_or(self.points.len(), |(_, next)| *next);
            let Some(segment) = self.points.get(*start..end) else {
                continue;
            };
            let from = segment.partition_point(|(p, _)| p.col < r.c1);
            for (i, (p, _)) in segment.iter().enumerate().skip(from) {
                if p.col > r.c2 {
                    break;
                }
                if !visit(start + i) {
                    return false;
                }
            }
        }
        true
    }
}

/// A Fenwick tree over the columns of a sheet: add a value to a column range, read the
/// sum at one column.
pub(super) struct Fenwick {
    tree: Vec<i64>,
}

impl Fenwick {
    pub(super) fn new() -> Self {
        // Columns 0..=MAX_COL, one past the end for the range end, 1-based inside.
        Fenwick {
            tree: vec![0; MAX_COL as usize + 3],
        }
    }

    fn clear(&mut self) {
        self.tree.iter_mut().for_each(|v| *v = 0);
    }

    fn add_at(&mut self, col: u32, delta: i64) {
        let mut i = col as usize + 1;
        while let Some(v) = self.tree.get_mut(i) {
            *v += delta;
            i += i.isolate_lowest_one();
        }
    }

    /// Adds `delta` to columns `c1..=c2`.
    fn add(&mut self, c1: u32, c2: u32, delta: i64) {
        self.add_at(c1, delta);
        self.add_at(c2.saturating_add(1), -delta);
    }

    fn at(&self, col: u32) -> i64 {
        let mut i = col as usize + 1;
        let mut sum = 0;
        while i > 0 {
            sum += self.tree.get(i).copied().unwrap_or(0);
            i -= i.isolate_lowest_one();
        }
        sum
    }
}

/// A rectangle starts (`delta` +1) or ends (`delta` -1, one row past its end) for the sweep.
#[derive(Clone, Copy)]
pub(super) struct Event {
    row: u32,
    c1: u32,
    c2: u32,
    delta: i64,
}

/// The start and end events of a rectangle that `weight` readers read.
pub(super) fn push_events(events: &mut [Vec<Event>], r: &Rect, weight: i64) {
    if let Some(list) = events.get_mut(r.sheet as usize) {
        list.push(Event {
            row: r.r1,
            c1: r.c1,
            c2: r.c2,
            delta: weight,
        });
        list.push(Event {
            row: r.r2.saturating_add(1),
            c1: r.c1,
            c2: r.c2,
            delta: -weight,
        });
    }
}

/// The bounding box of some cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Bounds {
    lo: CellPos,
    hi: CellPos,
}

impl Bounds {
    pub(super) fn of(p: CellPos) -> Self {
        Bounds { lo: p, hi: p }
    }

    pub(super) fn add(&mut self, o: Bounds) {
        self.lo.row = self.lo.row.min(o.lo.row);
        self.lo.col = self.lo.col.min(o.lo.col);
        self.hi.row = self.hi.row.max(o.hi.row);
        self.hi.col = self.hi.col.max(o.hi.col);
    }

    pub(super) fn a1(&self) -> String {
        let cell = |p: CellPos| format!("{}{}", col_letters(p.col), u64::from(p.row) + 1);
        if self.lo == self.hi {
            cell(self.lo)
        } else {
            format!("{}:{}", cell(self.lo), cell(self.hi))
        }
    }
}

pub(super) fn add_bounds(slot: &mut Option<Bounds>, b: Bounds) {
    match slot {
        Some(x) => x.add(b),
        None => *slot = Some(b),
    }
}

/// What one rectangle holds: the formula nodes and the bounding box of the constants.
pub(super) struct Hits {
    pub(super) targets: Vec<usize>,
    pub(super) constants: Option<Bounds>,
}

/// The read counts of each cell of one sheet (formulas that read it) and whether a name
/// refers to it, from one sweep over the rows.
pub(super) fn sweep(
    idx: &SheetIndex,
    events: &mut [Event],
    named_events: &mut [Event],
    tree: &mut Fenwick,
    named_tree: &mut Fenwick,
    stopper: &mut Stopper,
) -> (Vec<u32>, Vec<bool>) {
    tree.clear();
    named_tree.clear();
    events.sort_unstable_by_key(|e| e.row);
    named_events.sort_unstable_by_key(|e| e.row);
    let mut counts = Vec::with_capacity(idx.points.len());
    let mut named = Vec::with_capacity(idx.points.len());
    let (mut e, mut ne) = (0, 0);
    for (p, _) in &idx.points {
        while let Some(ev) = events.get(e).filter(|ev| ev.row <= p.row) {
            tree.add(ev.c1, ev.c2, ev.delta);
            e += 1;
            if stopper.tick() {
                return (counts, named);
            }
        }
        while let Some(ev) = named_events.get(ne).filter(|ev| ev.row <= p.row) {
            named_tree.add(ev.c1, ev.c2, ev.delta);
            ne += 1;
        }
        counts.push(u32::try_from(tree.at(p.col).max(0)).unwrap_or(u32::MAX));
        named.push(named_tree.at(p.col) > 0);
        if stopper.tick() {
            return (counts, named);
        }
    }
    (counts, named)
}

/// The formula nodes and constants inside one rectangle, with the number of cells visited.
/// `None` when the stop callback asks to stop.
pub(super) fn hits(
    index: &[SheetIndex],
    rect: &Rect,
    formula_node: &[usize],
    stopper: &mut Stopper,
) -> Option<(Hits, usize)> {
    let mut h = Hits {
        targets: Vec::new(),
        constants: None,
    };
    let Some(sheet) = index.get(rect.sheet as usize) else {
        return Some((h, 0));
    };
    let mut cost = 0;
    let finished = sheet.each_in(rect, |i| {
        cost += 1;
        match sheet.points[i] {
            (_, PointKind::Formula(f)) => {
                if let Some(n) = formula_node.get(f as usize)
                    && h.targets.last() != Some(n)
                {
                    h.targets.push(*n);
                }
            }
            (p, PointKind::Constant) => add_bounds(&mut h.constants, Bounds::of(p)),
        }
        !stopper.tick()
    });
    if !finished {
        return None;
    }
    h.targets.sort_unstable();
    h.targets.dedup();
    Some((h, cost))
}
