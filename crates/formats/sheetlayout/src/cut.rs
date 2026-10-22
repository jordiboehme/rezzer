// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rule G: the recursive XY cut at fully empty rows and columns.
//!
//! Each piece holds only its own items. A split sorts the piece's items by their first row
//! (or column) and sweeps their intervals; a gap in the union is an empty row (or column)
//! run. A merge never crosses a cut, because its whole span is occupied. The cost is the
//! number of items times the cut depth, times the log of the sort.

use crate::input::{Limits, Rect};
use crate::occupancy::{Prep, bounds};

/// Why the cut ended before every item was placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trip {
    Depth,
    Regions,
    Stopped,
}

/// One piece that cannot be cut further.
pub(crate) struct Leaf {
    pub ids: Vec<u32>,
    pub rows_cut: bool,
    pub cols_cut: bool,
    /// Split from a piece it touched only at a corner.
    pub corner: bool,
}

pub(crate) struct Cut {
    pub leaves: Vec<Leaf>,
    /// The limit that ended the cut, with the items not placed.
    pub rest: Option<(Trip, Vec<u32>)>,
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
    Corner,
}

struct Piece {
    ids: Vec<u32>,
    depth: u32,
    next: Next,
    rows_cut: bool,
    cols_cut: bool,
    corner: bool,
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

/// Cuts the free items into leaves in reading order. `room` is how many regions the cut may
/// still give, the rest region included.
pub(crate) fn cut(
    prep: &Prep,
    free: Vec<u32>,
    limits: &Limits,
    room: usize,
    stop: &dyn Fn() -> bool,
) -> Cut {
    let mut leaves = Vec::new();
    let mut stack = Vec::new();
    if !free.is_empty() {
        stack.push(Piece {
            ids: free,
            depth: 0,
            next: Next::Both,
            rows_cut: false,
            cols_cut: false,
            corner: false,
        });
    }
    let rest = |p: Piece, stack: Vec<Piece>, why: Trip| {
        let mut ids = p.ids;
        for q in stack {
            ids.extend(q.ids);
        }
        Some((why, ids))
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
        if parts.is_none() && p.next != Next::Leaf {
            parts = corners(prep, &p.ids);
            how = How::Corner;
        }
        match parts {
            Some(parts) => {
                if p.depth >= limits.max_cut_depth {
                    return Cut {
                        leaves,
                        rest: rest(p, stack, Trip::Depth),
                    };
                }
                for ids in parts.into_iter().rev() {
                    stack.push(Piece {
                        ids,
                        depth: p.depth + 1,
                        next: match how {
                            How::Rows => Next::Cols,
                            How::Cols => Next::Rows,
                            How::Corner => Next::Leaf,
                        },
                        rows_cut: p.rows_cut || how == How::Rows,
                        cols_cut: p.cols_cut || how == How::Cols,
                        corner: p.corner || how == How::Corner,
                    });
                }
            }
            None => {
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
                    rows_cut: p.rows_cut,
                    cols_cut: p.cols_cut,
                    corner: p.corner,
                });
            }
        }
    }
    Cut { leaves, rest: None }
}

/// The largest grid of elementary cells the corner split builds, and its size per item: a
/// sparser piece is not split at corners and stays one region.
const CORNER_AREA_MAX: u64 = 1 << 20;
const CORNER_AREA_PER_ITEM: u64 = 64;

/// Splits a piece that no empty row or column cuts into its parts that touch only at
/// corners: the orthogonally connected parts of its occupied positions. Parts whose bounds
/// overlap stay together, so the regions never overlap. Works on the grid of elementary
/// cells between the items' edges, not on sheet positions; gives `None` when the piece is
/// one part or that grid would be too large for the piece.
fn corners(prep: &Prep, ids: &[u32]) -> Option<Vec<Vec<u32>>> {
    if ids.len() < 2 {
        return None;
    }
    let rects: Vec<Rect> = ids.iter().map(|&id| prep.item_rect(id)).collect();
    let edges = |a: fn(&Rect) -> u32, b: fn(&Rect) -> u32| {
        let mut v: Vec<u64> = rects
            .iter()
            .flat_map(|r| [u64::from(a(r)), u64::from(b(r)) + 1])
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let rows = edges(|r| r.top, |r| r.bottom);
    let cols = edges(|r| r.left, |r| r.right);
    let (h, w) = (rows.len() - 1, cols.len() - 1);
    let budget = CORNER_AREA_MAX.min((ids.len() as u64).saturating_mul(CORNER_AREA_PER_ITEM));
    if (h as u64) * (w as u64) > budget.max(4096) {
        return None;
    }
    let at = |v: &[u64], x: u64| v.partition_point(|&e| e < x);
    // Coverage by a 2D difference array, then prefix sums.
    let stride = w + 1;
    let mut grid = vec![0i32; (h + 1) * stride];
    let mut corner_of = Vec::with_capacity(rects.len());
    for r in &rects {
        let (r0, r1) = (
            at(&rows, u64::from(r.top)),
            at(&rows, u64::from(r.bottom) + 1),
        );
        let (c0, c1) = (
            at(&cols, u64::from(r.left)),
            at(&cols, u64::from(r.right) + 1),
        );
        grid[r0 * stride + c0] += 1;
        grid[r0 * stride + c1] -= 1;
        grid[r1 * stride + c0] -= 1;
        grid[r1 * stride + c1] += 1;
        corner_of.push(r0 * stride + c0);
    }
    for i in 0..=h {
        for j in 0..=w {
            let mut v = grid[i * stride + j];
            if i > 0 {
                v += grid[(i - 1) * stride + j];
            }
            if j > 0 {
                v += grid[i * stride + j - 1];
            }
            if i > 0 && j > 0 {
                v -= grid[(i - 1) * stride + j - 1];
            }
            grid[i * stride + j] = v;
        }
    }
    // Label the covered cells by flood fill; a label is stored as -(label + 1).
    let mut labels = 0i32;
    let mut queue: Vec<u32> = Vec::new();
    for start in 0..h * stride {
        if start % stride == w || grid[start] <= 0 {
            continue;
        }
        labels += 1;
        grid[start] = -labels;
        queue.push(start as u32);
        while let Some(k) = queue.pop() {
            let k = k as usize;
            let (i, j) = (k / stride, k % stride);
            let mut visit = |n: usize| {
                if grid[n] > 0 {
                    grid[n] = -labels;
                    queue.push(n as u32);
                }
            };
            if i > 0 {
                visit(k - stride);
            }
            if i + 1 < h {
                visit(k + stride);
            }
            if j > 0 {
                visit(k - 1);
            }
            if j + 1 < w {
                visit(k + 1);
            }
        }
    }
    if labels < 2 {
        return None;
    }
    let mut parts: Vec<(Rect, Vec<u32>)> = Vec::new();
    let mut index = vec![usize::MAX; labels as usize];
    for ((&id, r), &k) in ids.iter().zip(&rects).zip(&corner_of) {
        let l = (-grid[k] - 1) as usize;
        if index[l] == usize::MAX {
            index[l] = parts.len();
            parts.push((*r, Vec::new()));
        }
        let part = &mut parts[index[l]];
        part.0 = part.0.union(r);
        part.1.push(id);
    }
    drop(grid);
    // Parts whose bounds overlap are joined until none do.
    for _ in 0..64 {
        parts.sort_by_key(|p| (p.0.top, p.0.left));
        let mut joined = false;
        let mut out: Vec<(Rect, Vec<u32>)> = Vec::with_capacity(parts.len());
        for part in parts {
            match out.iter_mut().rev().find(|q| q.0.intersects(&part.0)) {
                Some(q) => {
                    q.0 = q.0.union(&part.0);
                    q.1.extend(part.1);
                    joined = true;
                }
                None => out.push(part),
            }
        }
        parts = out;
        if parts.len() < 2 {
            return None;
        }
        if !joined {
            return Some(parts.into_iter().map(|p| p.1).collect());
        }
    }
    None
}

/// The bounding rectangle of a leaf.
pub(crate) fn leaf_rect(prep: &Prep, leaf: &Leaf) -> crate::input::Rect {
    bounds(prep, &leaf.ids).expect("a leaf holds at least one item")
}
