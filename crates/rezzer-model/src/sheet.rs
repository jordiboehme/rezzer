// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Helpers over the regions of a sheet that the sheet reader and the renderer share: which
//! region holds a cell, and which author headings a region's table stands under.

use std::collections::BTreeMap;

use crate::{Block, plain_text};

/// A rectangle of cells, 0-based, edges included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellArea {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

/// The index of the area that holds each `(row, column)` point, 0-based. The areas must not
/// overlap. One sweep by row with the areas that have begun, by left column, so the cost
/// follows the number of points and areas, never the cells they cover.
pub fn locate_cells(areas: &[CellArea], points: &[(u32, u32)]) -> Vec<Option<usize>> {
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.sort_unstable_by_key(|&i| points[i]);
    let mut by_top: Vec<usize> = (0..areas.len()).collect();
    by_top.sort_unstable_by_key(|&k| (areas[k].top, areas[k].left));
    let mut next = 0;
    let mut open: BTreeMap<u32, usize> = BTreeMap::new();
    let mut out = vec![None; points.len()];
    for i in order {
        let (row, col) = points[i];
        while let Some(&k) = by_top.get(next)
            && areas[k].top <= row
        {
            open.insert(areas[k].left, k);
            next += 1;
        }
        // An area that ended above is closed; an open area left of the point that does not
        // reach it means no area holds it, as areas never overlap.
        while let Some((&left, &k)) = open.range(..=col).next_back() {
            let a = areas[k];
            if a.bottom < row {
                open.remove(&left);
                continue;
            }
            if a.right >= col {
                out[i] = Some(k);
            }
            break;
        }
    }
    out
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
