// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rule G: the recursive XY cut at fully empty rows and columns, and the split of blocks that
//! touch only at a corner.
//!
//! Each piece holds only its own items. A split sorts the piece's items by their first row
//! (or column) and sweeps their intervals; a gap in the union is an empty row (or column)
//! run. A merge never crosses a cut, because its whole span is occupied. The cost is the
//! number of items times the cut depth, times the log of the sort. The corner split is a
//! union-find over the items' rectangles by a sweep, with no grid of positions.

use std::collections::BTreeMap;

use crate::input::{Limits, Pos, Rect};
use crate::occupancy::{Prep, bounds};

/// Why the cut ended before every item was placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trip {
    Regions,
    Stopped,
}

/// One piece that cannot be cut further.
pub(crate) struct Leaf {
    pub ids: Vec<u32>,
    /// Merges that no longer count as occupied (they crossed a line around a fact range) and
    /// belong here, because their anchor lies here.
    pub loose: Vec<u32>,
    pub rows_cut: bool,
    pub cols_cut: bool,
    /// Split from a piece it touched only at a corner.
    pub corner: bool,
    /// Cut apart from a piece that reached around a fact range.
    pub beside: bool,
    /// The piece needed a cut deeper than the limit and stays uncut.
    pub deep: bool,
    /// The corner split needed more work than its budget: the leaf may hold several blocks.
    pub coarse: bool,
}

pub(crate) struct Cut {
    pub leaves: Vec<Leaf>,
    /// The limit that ended the cut, with the items and loose merges not placed.
    pub rest: Option<(Trip, Vec<u32>, Vec<u32>)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    Both,
    Rows,
    Cols,
    /// A part from the corner split: no cut applies to it.
    Leaf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum How {
    Rows,
    Cols,
    Fact,
    Corner,
}

/// A merge that is no longer occupied, with the item of its anchor cell when there is one.
#[derive(Clone, Copy)]
struct Loose {
    merge: u32,
    anchor: Option<u32>,
}

struct Piece {
    ids: Vec<u32>,
    loose: Vec<Loose>,
    depth: u32,
    next: Next,
    rows_cut: bool,
    cols_cut: bool,
    corner: bool,
    beside: bool,
}

/// Splits the items at every run of empty rows (or columns), in order, or gives `None` when
/// the piece holds no such run.
fn split(prep: &Prep, ids: &mut [u32], rows: bool) -> Option<Vec<Vec<u32>>> {
    let span = |id: u32| {
        let r = prep.item_rect(id);
        if rows {
            (r.top, r.bottom)
        } else {
            (r.left, r.right)
        }
    };
    ids.sort_unstable_by_key(|&id| span(id).0);
    let mut cuts = Vec::new();
    let mut end = span(*ids.first()?).1;
    for (k, &id) in ids.iter().enumerate().skip(1) {
        let (a, b) = span(id);
        // `a > end + 1` without overflow: the row after `end` is empty.
        if a > end && a - end > 1 {
            cuts.push(k);
        }
        end = end.max(b);
    }
    if cuts.is_empty() {
        return None;
    }
    let mut parts = Vec::with_capacity(cuts.len() + 1);
    let mut from = 0;
    for k in cuts {
        parts.push(ids[from..k].to_vec());
        from = k;
    }
    parts.push(ids[from..].to_vec());
    Some(parts)
}

/// Gives each loose merge to the part that holds its anchor cell, or to the first part when
/// it has none.
fn hand_down(
    loose: Vec<Loose>,
    parts: &[Vec<u32>],
    owner: &mut Vec<u32>,
    items: usize,
) -> Vec<Vec<Loose>> {
    let mut out: Vec<Vec<Loose>> = parts.iter().map(|_| Vec::new()).collect();
    if loose.is_empty() || out.is_empty() {
        return out;
    }
    if owner.is_empty() {
        *owner = vec![0; items];
    }
    for (k, part) in parts.iter().enumerate() {
        for &id in part {
            owner[id as usize] = k as u32;
        }
    }
    for l in loose {
        let k = l.anchor.map_or(0, |a| owner[a as usize] as usize);
        out[k].push(l);
    }
    out
}

/// Cuts the free items into leaves in reading order. `room` is how many regions the cut may
/// still give, the rest region included. `facts` are the fact ranges no leaf may reach.
pub(crate) fn cut(
    prep: &Prep,
    free: Vec<u32>,
    facts: &[Rect],
    limits: &Limits,
    room: usize,
    stop: &dyn Fn() -> bool,
) -> Cut {
    let mut leaves = Vec::new();
    let mut stack = Vec::new();
    let mut owner = Vec::new();
    if !free.is_empty() {
        stack.push(Piece {
            ids: free,
            loose: Vec::new(),
            depth: 0,
            next: Next::Both,
            rows_cut: false,
            cols_cut: false,
            corner: false,
            beside: false,
        });
    }
    let rest = |p: Piece, stack: Vec<Piece>, why: Trip| {
        let mut ids = p.ids;
        let mut loose: Vec<u32> = p.loose.iter().map(|l| l.merge).collect();
        for q in stack {
            ids.extend(q.ids);
            loose.extend(q.loose.iter().map(|l| l.merge));
        }
        Some((why, ids, loose))
    };
    while let Some(mut p) = stack.pop() {
        if stop() {
            return Cut {
                leaves,
                rest: rest(p, stack, Trip::Stopped),
            };
        }
        let mut parts = None;
        let mut how = How::Rows;
        if p.next == Next::Both || p.next == Next::Rows {
            parts = split(prep, &mut p.ids, true);
        }
        if parts.is_none() && (p.next == Next::Both || p.next == Next::Cols) {
            parts = split(prep, &mut p.ids, false);
            how = How::Cols;
        }
        if parts.is_none()
            && let Some(f) =
                bounds(prep, &p.ids).and_then(|b| facts.iter().find(|f| f.intersects(&b)))
        {
            let (groups, dropped) = around(prep, &p.ids, f);
            p.loose.extend(dropped);
            parts = Some(groups);
            how = How::Fact;
        }
        let mut coarse = false;
        if parts.is_none() && p.next != Next::Leaf {
            match corners(prep, &p.ids, stop) {
                Corners::Parts(v) => {
                    parts = Some(v);
                    how = How::Corner;
                }
                Corners::One => {}
                Corners::Skipped => coarse = true,
                Corners::Stopped => {
                    return Cut {
                        leaves,
                        rest: rest(p, stack, Trip::Stopped),
                    };
                }
            }
        }
        // A cut around a fact range never reaches that range again, so it needs no depth
        // limit. A piece past the limit is cut around the facts it reaches first, so an uncut
        // piece never encloses a fact.
        let mut deep = parts.is_some() && how != How::Fact && p.depth >= limits.max_cut_depth;
        if deep
            && let Some(f) =
                bounds(prep, &p.ids).and_then(|b| facts.iter().find(|f| f.intersects(&b)))
        {
            let (groups, dropped) = around(prep, &p.ids, f);
            p.loose.extend(dropped);
            parts = Some(groups);
            how = How::Fact;
            deep = false;
        }
        match parts {
            Some(parts) if !deep => {
                let loose = hand_down(
                    std::mem::take(&mut p.loose),
                    &parts,
                    &mut owner,
                    prep.items(),
                );
                for (ids, loose) in parts.into_iter().zip(loose).rev() {
                    stack.push(Piece {
                        ids,
                        loose,
                        depth: if how == How::Fact {
                            p.depth
                        } else {
                            p.depth + 1
                        },
                        next: match how {
                            How::Rows => Next::Cols,
                            How::Cols => Next::Rows,
                            How::Fact => Next::Both,
                            How::Corner => Next::Leaf,
                        },
                        rows_cut: p.rows_cut || how == How::Rows,
                        cols_cut: p.cols_cut || how == How::Cols,
                        corner: p.corner || how == How::Corner,
                        beside: p.beside || how == How::Fact,
                    });
                }
            }
            _ => {
                let full = if stack.is_empty() {
                    leaves.len() >= room
                } else {
                    leaves.len() + 1 >= room
                };
                if full {
                    return Cut {
                        leaves,
                        rest: rest(p, stack, Trip::Regions),
                    };
                }
                leaves.push(Leaf {
                    ids: p.ids,
                    loose: p.loose.iter().map(|l| l.merge).collect(),
                    rows_cut: p.rows_cut,
                    cols_cut: p.cols_cut,
                    corner: p.corner,
                    beside: p.beside,
                    deep,
                    coarse,
                });
            }
        }
    }
    Cut { leaves, rest: None }
}

/// Cuts a piece whose bounds reach a fact range into the items above it, left of it, right
/// of it and below it. A merge that crosses one of these lines is no longer counted as
/// occupied; it is handed on with the part that holds its anchor cell. The parts never reach
/// the fact range, so a rule region never holds a fact's cells.
fn around(prep: &Prep, ids: &[u32], f: &Rect) -> (Vec<Vec<u32>>, Vec<Loose>) {
    let mut groups: [Vec<u32>; 4] = Default::default();
    let mut dropped = Vec::new();
    for &id in ids {
        let r = prep.item_rect(id);
        let g = if r.bottom < f.top {
            0
        } else if r.top > f.bottom {
            3
        } else if r.top < f.top || r.bottom > f.bottom {
            usize::MAX
        } else if r.right < f.left {
            1
        } else if r.left > f.right {
            2
        } else {
            usize::MAX
        };
        if g == usize::MAX {
            if let Some(merge) = prep.merge_of(id) {
                let anchor = prep
                    .cell_at(Pos {
                        row: r.top,
                        col: r.left,
                    })
                    .map(|k| k as u32);
                dropped.push(Loose { merge, anchor });
            }
            continue;
        }
        groups[g].push(id);
    }
    (
        groups.into_iter().filter(|g| !g.is_empty()).collect(),
        dropped,
    )
}

/// What the corner split found.
enum Corners {
    One,
    /// The split needed more work than its budget, or the rejoin did not settle: the piece
    /// stays one region, and an issue says so.
    Skipped,
    Parts(Vec<Vec<u32>>),
    Stopped,
}

enum Halt {
    Stopped,
    Budget,
}

/// Union-find with path halving.
struct Dsu(Vec<u32>);

impl Dsu {
    fn new(n: usize) -> Self {
        Self((0..n as u32).collect())
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            let up = self.0[self.0[x as usize] as usize];
            self.0[x as usize] = up;
            x = up;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) -> bool {
        let (a, b) = (self.find(a), self.find(b));
        if a == b {
            return false;
        }
        self.0[a.max(b) as usize] = a.min(b);
        true
    }
}

/// A run of columns and the rectangle that covers it to the lowest row.
#[derive(Clone, Copy)]
struct Seg {
    right: u32,
    bottom: u32,
    item: u32,
}

/// Visits of the column map a corner split may make, per item and in all: a shape that needs
/// more (many long merges crossed by many wide ones) is not split at corners.
const VISITS_PER_ITEM: u64 = 64;
const VISITS_BASE: u64 = 4096;
/// Rectangles between two questions to the stop callback.
const STOP_EVERY: usize = 4096;
/// Rounds of joining parts whose bounds overlap.
const REJOIN_ROUNDS: usize = 64;

/// Joins rectangles that overlap; with `touch` also rectangles side by side or stacked, never
/// on a diagonal only. A sweep by top row keeps a map of column runs, each holding the
/// rectangle that covers it down to the lowest row; a new rectangle joins the runs it meets.
/// Gives the number of joins.
fn connect(
    rect: &dyn Fn(u32) -> Rect,
    n: usize,
    touch: bool,
    dsu: &mut Dsu,
    visits: &mut u64,
    stop: &dyn Fn() -> bool,
) -> Result<usize, Halt> {
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_unstable_by_key(|&i| {
        let r = rect(i);
        (r.top, r.left)
    });
    let mut map: BTreeMap<u32, Seg> = BTreeMap::new();
    let mut joins = 0usize;
    let mut met: Vec<(u32, Seg)> = Vec::new();
    for (k, &b) in order.iter().enumerate() {
        if k % STOP_EVERY == STOP_EVERY - 1 && stop() {
            return Err(Halt::Stopped);
        }
        let r = rect(b);
        let (lo, hi) = if touch {
            (r.left.saturating_sub(1), r.right.saturating_add(1))
        } else {
            (r.left, r.right)
        };
        let start = match map.range(..=lo).next_back() {
            Some((&key, s)) if s.right >= lo => key,
            _ => lo,
        };
        met.clear();
        met.extend(map.range(start..=hi).map(|(&key, s)| (key, *s)));
        *visits = visits
            .checked_sub(met.len() as u64 + 1)
            .ok_or(Halt::Budget)?;
        for &(key, s) in &met {
            map.remove(&key);
            if u64::from(s.bottom) + 1 < u64::from(r.top) {
                // Ended above the row before: it meets nothing any more.
                continue;
            }
            let meets = if s.bottom >= r.top {
                true
            } else {
                // Ends in the row above: stacked only when the columns overlap.
                touch && s.right >= r.left && key <= r.right
            };
            if meets && dsu.union(b, s.item) {
                joins += 1;
            }
        }
        // Write the new rectangle over its columns where it reaches lower than what is there.
        let mut cur = u64::from(r.left);
        let put = |map: &mut BTreeMap<u32, Seg>, left: u64, seg: Seg| {
            map.insert(left as u32, seg);
        };
        for &(key, s) in &met {
            if u64::from(s.bottom) + 1 < u64::from(r.top) {
                continue;
            }
            if s.right < r.left || key > r.right {
                // Outside the new columns (the side runs of `touch`): unchanged.
                put(&mut map, u64::from(key), s);
                continue;
            }
            if key < r.left {
                put(
                    &mut map,
                    u64::from(key),
                    Seg {
                        right: r.left - 1,
                        ..s
                    },
                );
            }
            let a = u64::from(key.max(r.left));
            let z = s.right.min(r.right);
            if a > cur {
                put(
                    &mut map,
                    cur,
                    Seg {
                        right: (a - 1) as u32,
                        bottom: r.bottom,
                        item: b,
                    },
                );
            }
            if s.bottom >= r.bottom {
                put(&mut map, a, Seg { right: z, ..s });
            } else {
                put(
                    &mut map,
                    a,
                    Seg {
                        right: z,
                        bottom: r.bottom,
                        item: b,
                    },
                );
            }
            cur = u64::from(z) + 1;
            if s.right > r.right {
                put(
                    &mut map,
                    u64::from(r.right) + 1,
                    Seg {
                        right: s.right,
                        ..s
                    },
                );
            }
        }
        if cur <= u64::from(r.right) {
            put(
                &mut map,
                cur,
                Seg {
                    right: r.right,
                    bottom: r.bottom,
                    item: b,
                },
            );
        }
    }
    Ok(joins)
}

/// Splits a piece that no empty row or column cuts into its parts that touch only at
/// corners: the orthogonally connected parts of its occupied positions. Parts whose bounds
/// overlap are joined again, so regions never overlap. Gives `One` when the piece is one
/// part, and `Skipped` when the shape needs more work than the visit budget allows or the
/// rejoin does not settle.
fn corners(prep: &Prep, ids: &[u32], stop: &dyn Fn() -> bool) -> Corners {
    let k = ids.len();
    if k < 2 {
        return Corners::One;
    }
    let item = |i: u32| prep.item_rect(ids[i as usize]);
    let mut visits = (k as u64).saturating_mul(VISITS_PER_ITEM) + VISITS_BASE;
    let mut dsu = Dsu::new(k);
    match connect(&item, k, true, &mut dsu, &mut visits, stop) {
        Ok(_) => {}
        Err(Halt::Stopped) => return Corners::Stopped,
        Err(Halt::Budget) => return Corners::Skipped,
    }
    // Number the parts and take their bounds.
    let mut part_of = vec![u32::MAX; k];
    let mut boxes: Vec<Rect> = Vec::new();
    for i in 0..k as u32 {
        let root = dsu.find(i) as usize;
        if part_of[root] == u32::MAX {
            part_of[root] = boxes.len() as u32;
            boxes.push(item(i));
        }
        let p = part_of[root];
        part_of[i as usize] = p;
        boxes[p as usize] = boxes[p as usize].union(&item(i));
    }
    drop(dsu);
    let mut joined_all = false;
    for _ in 0..REJOIN_ROUNDS {
        if boxes.len() < 2 {
            return Corners::One;
        }
        if stop() {
            return Corners::Stopped;
        }
        let mut d2 = Dsu::new(boxes.len());
        let b = &boxes;
        match connect(
            &|i| b[i as usize],
            boxes.len(),
            false,
            &mut d2,
            &mut visits,
            stop,
        ) {
            Ok(0) => {
                joined_all = true;
                break;
            }
            Ok(_) => {}
            Err(Halt::Stopped) => return Corners::Stopped,
            Err(Halt::Budget) => return Corners::Skipped,
        }
        let mut renumber = vec![u32::MAX; boxes.len()];
        let mut next: Vec<Rect> = Vec::new();
        for (p, bx) in boxes.iter().enumerate() {
            let root = d2.find(p as u32) as usize;
            if renumber[root] == u32::MAX {
                renumber[root] = next.len() as u32;
                next.push(*bx);
            }
            let q = renumber[root] as usize;
            renumber[p] = q as u32;
            next[q] = next[q].union(bx);
        }
        for p in part_of.iter_mut() {
            *p = renumber[*p as usize];
        }
        boxes = next;
    }
    if !joined_all {
        return Corners::Skipped;
    }
    if boxes.len() < 2 {
        return Corners::One;
    }
    let mut parts: Vec<Vec<u32>> = vec![Vec::new(); boxes.len()];
    for (i, &p) in part_of.iter().enumerate() {
        parts[p as usize].push(ids[i]);
    }
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_unstable_by_key(|&p| (boxes[p].top, boxes[p].left));
    let mut slots: Vec<Option<Vec<u32>>> = parts.into_iter().map(Some).collect();
    Corners::Parts(order.into_iter().filter_map(|p| slots[p].take()).collect())
}

/// The bounding rectangle of a leaf.
pub(crate) fn leaf_rect(prep: &Prep, leaf: &Leaf) -> Rect {
    bounds(prep, &leaf.ids).expect("a leaf holds at least one item")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{CellIn, Class, Input, Style};

    #[test]
    fn corners_ask_the_stop() {
        let mut cells = Vec::new();
        for row in 0..300u32 {
            for col in (row % 2..300).step_by(2) {
                cells.push(CellIn {
                    pos: Pos { row, col },
                    class: Class::Text,
                    text: "x".to_string(),
                    number: None,
                    style: Style::default(),
                    formula: None,
                });
            }
        }
        let input = Input {
            cells,
            merges: Vec::new(),
            facts: Vec::new(),
            names: Vec::new(),
            outline: Vec::new(),
            frozen: None,
            print_titles: None,
            header_guess: None,
            styles_read: false,
            is_label: &|_| true,
            is_unit_column: &|_| false,
        };
        let prep = Prep::new(&input);
        let ids: Vec<u32> = (0..prep.items() as u32).collect();
        assert!(matches!(corners(&prep, &ids, &|| false), Corners::Parts(_)));
        // The first question comes inside the sweep and says go on; the second ends it,
        // before the rejoin round would ask.
        let calls = std::cell::Cell::new(0);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > 1
        };
        assert!(matches!(corners(&prep, &ids, &stop), Corners::Stopped));
        assert_eq!(calls.get(), 2);
    }
}
