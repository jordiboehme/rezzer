// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Helpers over the regions of a sheet that the sheet reader and the renderer share: which
//! region holds a cell, and which author headings a region's table stands under.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::{Block, plain_text};

/// A rectangle of cells, 0-based, edges included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellArea {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

/// The index of the area that holds each `(row, column)` point, 0-based. Areas normally do
/// not overlap. When some do (ranges of a damaged file), a point goes to the smallest area
/// that holds it: the innermost range. Between areas of equal size the one that starts
/// higher, then further left, wins, and between equal rectangles the lower index, so the
/// answer does not depend on the order of the list. Every point lies in at most one area
/// either way.
///
/// The layout detector gives cells to its regions by the same rule with its own copy of this
/// sweep (`sheetlayout`'s `locate`), so the detector and the reader agree on every cell; the
/// two keep the same structure.
///
/// One sweep by row. The areas that have begun are kept in a segment tree over their column
/// edges: each area sits in the O(log R) nodes that cover its columns, in a heap ordered by
/// size, and a point looks only at the nodes on the path to its column. Areas that ended
/// above are dropped from a heap when they come to its top, once. So the cost is
/// O((P + R log R) log R) for P points and R areas, never the cells the areas cover and never
/// P times R, however the areas overlap.
pub fn locate_cells(areas: &[CellArea], points: &[(u32, u32)]) -> Vec<Option<usize>> {
    let mut out = vec![None; points.len()];
    if areas.is_empty() {
        return out;
    }
    // Column edges: a leaf is the columns from one edge up to the next.
    let mut edges: Vec<u64> = areas
        .iter()
        .flat_map(|a| [u64::from(a.left), u64::from(a.right) + 1])
        .collect();
    edges.sort_unstable();
    edges.dedup();
    let leaves = edges.len() - 1;
    let leaf_of = |col: u64| edges.partition_point(|&e| e <= col).checked_sub(1);
    let mut tree = Tree {
        heaps: vec![BinaryHeap::new(); 4 * leaves.max(1)],
        leaves,
    };
    let mut by_top: Vec<usize> = (0..areas.len()).collect();
    by_top.sort_unstable_by_key(|&k| areas[k].top);
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.sort_unstable_by_key(|&i| points[i]);
    let mut next = 0;
    for i in order {
        let (row, col) = points[i];
        while let Some(&k) = by_top.get(next)
            && areas[k].top <= row
        {
            let a = areas[k];
            let from = leaf_of(u64::from(a.left)).unwrap_or(0);
            let to = leaf_of(u64::from(a.right)).unwrap_or(0);
            tree.insert(
                1,
                0,
                leaves,
                from,
                to,
                Reverse((size(&a), a.top, a.left, k)),
            );
            next += 1;
        }
        let Some(leaf) = leaf_of(u64::from(col)).filter(|&l| l < leaves) else {
            continue;
        };
        out[i] = tree.smallest(leaf, |k| areas[k].bottom >= row);
    }
    out
}

/// The order of areas for a point: size, top row, left column, index; the least wins.
type Key = (u64, u32, u32, usize);

/// A segment tree over column leaves; each node holds the areas that cover all its leaves
/// (and not all of its parent's), smallest first.
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

    /// The smallest live area on the path from the root to `leaf`. Areas that are not live
    /// any more (`live` is false: they ended above the sweep row) leave their heap for good.
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

/// The number of cells of an area.
fn size(a: &CellArea) -> u64 {
    (u64::from(a.bottom) - u64::from(a.top) + 1) * (u64::from(a.right) - u64::from(a.left) + 1)
}

/// For each table with a sheet layout among `blocks`, in order: its layout range and the
/// author headings open above it, wide to narrow, each trimmed. An anchored heading is
/// author text (a caption heading, a defined name standing as a caption, a section
/// heading); a generated heading (no anchor) closes the deeper ones and adds nothing.
pub fn table_headings(blocks: &[Block]) -> Vec<(&str, Vec<String>)> {
    let mut open: Vec<(u8, Option<String>)> = Vec::new();
    let mut out = Vec::new();
    for b in blocks {
        match b {
            Block::Heading {
                level,
                inlines,
                anchor,
            } => {
                open.retain(|(l, _)| l < level);
                let text = anchor
                    .as_ref()
                    .map(|_| plain_text(inlines).trim().to_string())
                    .filter(|t| !t.is_empty());
                open.push((*level, text));
            }
            Block::Table {
                layout: Some(l), ..
            } => out.push((
                l.range.as_str(),
                open.iter().filter_map(|(_, t)| t.clone()).collect(),
            )),
            _ => {}
        }
    }
    out
}
