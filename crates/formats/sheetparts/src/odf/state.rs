// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The parts of a sheet while its XML is read: cell rectangles, validations, columns and notes.

use super::OdsSheet;
use crate::a1::a1;

/// A rectangle of cells, 0-based and inclusive.
#[derive(Debug, Clone, Copy)]
pub(super) struct Rect {
    pub(super) r0: u32,
    pub(super) r1: u32,
    pub(super) c0: u32,
    pub(super) c1: u32,
}

impl Rect {
    pub(super) fn a1(self) -> String {
        if self.r0 == self.r1 && self.c0 == self.c1 {
            a1(self.r0, self.c0)
        } else {
            format!("{}:{}", a1(self.r0, self.c0), a1(self.r1, self.c1))
        }
    }
}

#[derive(Default)]
pub(super) struct ValDef {
    pub(super) condition: String,
    pub(super) prompt: String,
    pub(super) error: String,
    pub(super) rects: Vec<Rect>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Msg {
    Help,
    Error,
}

pub(super) struct ColDef {
    pub(super) first: u32,
    pub(super) last: u32,
    pub(super) style: Option<String>,
}

#[derive(Default)]
pub(super) struct AnnState {
    pub(super) rect: Option<Rect>,
    pub(super) open: bool,
    pub(super) author: String,
    pub(super) date: String,
    pub(super) field: Option<bool>,
    pub(super) paras: Vec<String>,
    pub(super) para: Option<String>,
}

pub(super) struct SheetState {
    pub(super) sheet: OdsSheet,
    pub(super) row: u64,
    pub(super) col: u64,
    pub(super) col_defs: u64,
    pub(super) cols: Vec<ColDef>,
    pub(super) in_row: bool,
    pub(super) in_header: bool,
    pub(super) group_depth: u8,
    /// A row group opened since the last outline span: the next span starts a new entry.
    pub(super) group_break: bool,
    pub(super) row_rep: u64,
    pub(super) row_style: Option<String>,
    pub(super) rows_clamped: bool,
    pub(super) cols_clamped: bool,
    pub(super) cell: Option<Rect>,
    pub(super) ann: AnnState,
}
