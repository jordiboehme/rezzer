// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Helpers over the regions of a sheet that the sheet reader and the renderer share: which
//! region holds a cell, and which author headings a region's table stands under.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Block, plain_text};

/// A rectangle of cells, 0-based, edges included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellArea {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

/// How many open areas left of a point the search looks at when areas overlap. A damaged
/// file can stack many overlapping ranges; past this many the smallest area found so far
/// holds the point, so the cost stays bounded.
pub const OVERLAP_SCAN: usize = 64;

/// The index of the area that holds each `(row, column)` point, 0-based. One sweep by row
/// with the areas that have begun, by left column, so the cost follows the number of points
/// and areas, never the cells they cover.
///
/// Areas normally do not overlap. When some do (ranges of a damaged file), a point goes to
/// the smallest area that holds it: the innermost range. Every point lies in at most one
/// area either way.
pub fn locate_cells(areas: &[CellArea], points: &[(u32, u32)]) -> Vec<Option<usize>> {
    let mut by_top: Vec<usize> = (0..areas.len()).collect();
    by_top.sort_unstable_by_key(|&k| (areas[k].top, areas[k].left));
    let overlap = any_overlap(areas, &by_top);
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.sort_unstable_by_key(|&i| points[i]);
    let mut next = 0;
    // Keyed by left column and index, so areas with the same left column all stay open.
    let mut open: BTreeSet<(u32, usize)> = BTreeSet::new();
    let mut ended: Vec<(u32, usize)> = Vec::new();
    let mut out = vec![None; points.len()];
    for i in order {
        let (row, col) = points[i];
        while let Some(&k) = by_top.get(next)
            && areas[k].top <= row
        {
            open.insert((areas[k].left, k));
            next += 1;
        }
        let mut best: Option<usize> = None;
        let mut looked = 0usize;
        for &(left, k) in open.range(..=(col, usize::MAX)).rev() {
            let a = areas[k];
            if a.bottom < row {
                // Ended above: it can never hold a point again.
                ended.push((left, k));
                continue;
            }
            if a.right >= col && best.is_none_or(|b| size(&a) < size(&areas[b])) {
                best = Some(k);
            }
            looked += 1;
            // Without overlaps the nearest open area left of the point is the only one that
            // can hold it.
            if !overlap || looked >= OVERLAP_SCAN {
                break;
            }
        }
        for e in ended.drain(..) {
            open.remove(&e);
        }
        out[i] = best;
    }
    out
}

/// The number of cells of an area.
fn size(a: &CellArea) -> u64 {
    (u64::from(a.bottom) - u64::from(a.top) + 1) * (u64::from(a.right) - u64::from(a.left) + 1)
}

/// Whether any two areas overlap, by a sweep in `by_top` order. The areas open at the top
/// row of a new area do not overlap each other (else the answer is already yes), so only
/// the nearest open one left of its right edge can reach it.
fn any_overlap(areas: &[CellArea], by_top: &[usize]) -> bool {
    let mut open: BTreeMap<u32, usize> = BTreeMap::new();
    for &k in by_top {
        let a = areas[k];
        while let Some((&left, &j)) = open.range(..=a.right).next_back() {
            let b = areas[j];
            if b.bottom < a.top {
                open.remove(&left);
                continue;
            }
            if b.right >= a.left {
                return true;
            }
            break;
        }
        if open
            .insert(a.left, k)
            .is_some_and(|j| areas[j].bottom >= a.top)
        {
            // An open area with the same left column overlaps it.
            return true;
        }
    }
    false
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
