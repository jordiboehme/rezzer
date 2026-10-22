// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sparse view of a sheet: occupied cells in order and merges as rectangles.
//!
//! Nothing here is per row or per position of the used area. An item is either one occupied
//! cell or one merge; occupancy of a band comes from the items' row or column intervals.

use crate::input::{CellIn, Input, Rect};

pub(crate) struct Prep<'a> {
    pub input: &'a Input<'a>,
    /// Indices into `input.cells` of the occupied cells, sorted by position, one per position.
    pub cells: Vec<u32>,
    /// The merges, with their corners in order, sorted by top and left, without repeats.
    pub merges: Vec<Rect>,
}

impl<'a> Prep<'a> {
    pub fn new(input: &'a Input<'a>) -> Self {
        let n = input.cells.len().min(u32::MAX as usize);
        let mut cells: Vec<u32> = (0..n as u32)
            .filter(|&i| input.cells[i as usize].occupied())
            .collect();
        let sorted = cells
            .windows(2)
            .all(|w| input.cells[w[0] as usize].pos < input.cells[w[1] as usize].pos);
        if !sorted {
            // A stable sort keeps the first cell at a repeated position in front.
            cells.sort_by_key(|&i| input.cells[i as usize].pos);
            cells.dedup_by_key(|i| input.cells[*i as usize].pos);
        }
        let mut merges: Vec<Rect> = input.merges.iter().map(|m| m.normalized()).collect();
        merges.sort_unstable_by_key(|m| (m.top, m.left, m.bottom, m.right));
        merges.dedup();
        Self {
            input,
            cells,
            merges,
        }
    }

    pub fn cell(&self, k: usize) -> &'a CellIn {
        &self.input.cells[self.cells[k] as usize]
    }

    /// The number of items: cells first, then merges.
    pub fn items(&self) -> usize {
        self.cells.len() + self.merges.len()
    }

    /// The rectangle of an item.
    pub fn item_rect(&self, id: u32) -> Rect {
        let id = id as usize;
        if id < self.cells.len() {
            Rect::cell(self.cell(id).pos)
        } else {
            self.merges[id - self.cells.len()]
        }
    }

    /// Whether an item is a merge, and which.
    pub fn merge_of(&self, id: u32) -> Option<u32> {
        let id = id as usize;
        (id >= self.cells.len()).then(|| (id - self.cells.len()) as u32)
    }

    /// The range of `cells` whose rows lie in `top..=bottom`.
    pub fn rows(&self, top: u32, bottom: u32) -> std::ops::Range<usize> {
        let from = self
            .cells
            .partition_point(|&i| self.input.cells[i as usize].pos.row < top);
        let to = self
            .cells
            .partition_point(|&i| self.input.cells[i as usize].pos.row <= bottom);
        from..to.max(from)
    }

    /// The range of `cells` in one row between two columns.
    pub fn row_span(&self, row: u32, left: u32, right: u32) -> std::ops::Range<usize> {
        let r = self.rows(row, row);
        let slice = &self.cells[r.clone()];
        let from = slice.partition_point(|&i| self.input.cells[i as usize].pos.col < left);
        let to = slice.partition_point(|&i| self.input.cells[i as usize].pos.col <= right);
        r.start + from..r.start + to.max(from)
    }
}

/// The bounding rectangle of some items.
pub(crate) fn bounds(prep: &Prep, ids: &[u32]) -> Option<Rect> {
    ids.iter()
        .map(|&id| prep.item_rect(id))
        .reduce(|a, b| a.union(&b))
}
