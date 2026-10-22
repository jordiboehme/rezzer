// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which region holds a cell.
//!
//! This is the twin of `rezzer_model::locate_cells`, which the sheet reader uses to place the
//! cells of the regions this crate finds. The crate does not depend on Rezzer's model, so it
//! keeps its own copy; the two keep the same structure and the same rule, so the detector and
//! the reader give every cell to the same region.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::input::{Pos, Rect};

/// The index of the rectangle that holds each point. Rectangles normally do not overlap.
/// When some do (facts of a damaged file), a point goes to the smallest rectangle that holds
/// it: the innermost fact. Between rectangles of equal size the one that starts higher, then
/// further left, wins, and between equal rectangles the lower index, so the answer does not
/// depend on the order of the list.
///
/// One sweep by row. The rectangles that have begun are kept in a segment tree over their
/// column edges: each sits in the O(log R) nodes that cover its columns, in a heap ordered by
/// size, and a point looks only at the nodes on the path to its column. Rectangles that ended
/// above leave a heap when they come to its top, once. The cost is O((P + R log R) log R)
/// for P points and R rectangles, however they overlap.
pub fn locate(rects: &[Rect], points: &[Pos]) -> Vec<Option<usize>> {
    locate_until(rects, points, &mut || false).unwrap_or_default()
}

/// [`locate`], asking `stop` once per point; `None` when it says stop.
pub(crate) fn locate_until(
    rects: &[Rect],
    points: &[Pos],
    stop: &mut dyn FnMut() -> bool,
) -> Option<Vec<Option<usize>>> {
    let mut out = vec![None; points.len()];
    if rects.is_empty() {
        return Some(out);
    }
    // Column edges: a leaf is the columns from one edge up to the next.
    let mut edges: Vec<u64> = rects
        .iter()
        .flat_map(|r| [u64::from(r.left), u64::from(r.right) + 1])
        .collect();
    edges.sort_unstable();
    edges.dedup();
    let leaves = edges.len() - 1;
    let leaf_of = |col: u64| edges.partition_point(|&e| e <= col).checked_sub(1);
    let mut tree = Tree {
        heaps: vec![BinaryHeap::new(); 4 * leaves.max(1)],
        leaves,
    };
    let mut by_top: Vec<usize> = (0..rects.len()).collect();
    by_top.sort_unstable_by_key(|&k| rects[k].top);
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.sort_unstable_by_key(|&i| (points[i].row, points[i].col));
    let mut next = 0;
    for i in order {
        if stop() {
            return None;
        }
        let Pos { row, col } = points[i];
        while let Some(&k) = by_top.get(next)
            && rects[k].top <= row
        {
            let r = rects[k];
            let from = leaf_of(u64::from(r.left)).unwrap_or(0);
            let to = leaf_of(u64::from(r.right)).unwrap_or(0);
            tree.insert(
                1,
                0,
                leaves,
                from,
                to,
                Reverse((size(&r), r.top, r.left, k)),
            );
            next += 1;
        }
        let Some(leaf) = leaf_of(u64::from(col)).filter(|&l| l < leaves) else {
            continue;
        };
        out[i] = tree.smallest(leaf, |k| rects[k].bottom >= row);
    }
    Some(out)
}

/// The order of rectangles for a point: size, top row, left column, index; the least wins.
type Key = (u64, u32, u32, usize);

/// A segment tree over column leaves; each node holds the rectangles that cover all its
/// leaves (and not all of its parent's), smallest first.
struct Tree {
    heaps: Vec<BinaryHeap<Reverse<Key>>>,
    leaves: usize,
}

impl Tree {
    /// Puts `item` into the nodes that cover leaves `from..=to` exactly, below `node`, which
    /// spans leaves `lo..hi`.
    fn insert(
        &mut self,
        node: usize,
        lo: usize,
        hi: usize,
        from: usize,
        to: usize,
        item: Reverse<Key>,
    ) {
        if to < lo || hi <= from {
            return;
        }
        if from <= lo && hi - 1 <= to {
            self.heaps[node].push(item);
            return;
        }
        let mid = lo + (hi - lo) / 2;
        self.insert(2 * node, lo, mid, from, to, item);
        self.insert(2 * node + 1, mid, hi, from, to, item);
    }

    /// The smallest live rectangle on the path from the root to `leaf`. Rectangles that are
    /// not live any more (`live` is false: they ended above the sweep row) leave their heap
    /// for good.
    fn smallest(&mut self, leaf: usize, live: impl Fn(usize) -> bool) -> Option<usize> {
        let (mut node, mut lo, mut hi) = (1, 0, self.leaves);
        let mut best: Option<Key> = None;
        loop {
            let heap = &mut self.heaps[node];
            while let Some(&Reverse((_, _, _, k))) = heap.peek() {
                if live(k) {
                    break;
                }
                heap.pop();
            }
            if let Some(&Reverse(top)) = heap.peek()
                && best.is_none_or(|b| top < b)
            {
                best = Some(top);
            }
            if hi - lo <= 1 {
                break;
            }
            let mid = lo + (hi - lo) / 2;
            if leaf < mid {
                (node, hi) = (2 * node, mid);
            } else {
                (node, lo) = (2 * node + 1, mid);
            }
        }
        best.map(|(_, _, _, k)| k)
    }
}

/// The number of cells of a rectangle.
fn size(r: &Rect) -> u64 {
    r.height() * r.width()
}
