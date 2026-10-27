// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What readers read, the nodes of the dependency graph and the cells reached through names.

use std::collections::HashMap;
use std::rc::Rc;

use super::geometry::{Rect, merge};
use super::stop::Stopper;
use crate::CellPos;

/// Something a reader reads that is not a cell: a name, a table or another workbook.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Sym {
    /// A defined name added with [`Workbook::add_name`].
    Name(usize),
    /// A name no [`Workbook::add_name`] defined, by its anchor.
    Unknown(String),
    /// A table reference that could not be resolved to cells (unknown table or column).
    Table(String),
    External(String),
}

/// What one reader (a formula, or a defined name) reads.
#[derive(Default)]
pub(super) struct Reads {
    pub(super) rects: Vec<Rect>,
    pub(super) syms: Vec<Sym>,
}

/// Where a reader's references are read from.
#[derive(Clone, Copy)]
pub(super) struct Ctx<'a> {
    /// The sheet unqualified cell references point into.
    pub(super) sheet: Option<u32>,
    /// The sheet name lookups start from.
    pub(super) scope: Option<&'a str>,
    /// The formula cell, for `[@col]` and a table reference without a table name.
    pub(super) at: Option<CellPos>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Group,
    Formula,
    Parameters,
    Name,
    External,
}

impl Kind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Kind::Group => "group",
            Kind::Formula => "formula",
            Kind::Parameters => "parameters",
            Kind::Name => "name",
            Kind::External => "external",
        }
    }

    pub(super) fn prefix(self) -> char {
        match self {
            Kind::Group => 'p',
            Kind::Formula => 'f',
            Kind::Parameters => 'v',
            Kind::Name => 'n',
            Kind::External => 'x',
        }
    }

    pub(super) fn reads(self) -> bool {
        matches!(self, Kind::Group | Kind::Formula | Kind::Name)
    }
}

pub(super) struct Node {
    pub(super) kind: Kind,
    pub(super) label: String,
    /// For readers: the nodes this one brings in, in order of first use.
    pub(super) brings: Vec<usize>,
}

/// The most rectangles kept for name closures at one time.
const CLOSURE_MEMO_RECTS: usize = 1 << 20;

/// The node of each sym, made on first use; a new node is brought in by its reader.
pub(super) struct SymNodes {
    pub(super) map: HashMap<Sym, usize>,
    /// Names that got a node, with that node, in order: they are readers too.
    pub(super) names: Vec<(usize, usize)>,
}

/// The cells reached through sets of names, following names that refer to names. The walk
/// uses an explicit stack, so a long chain of names cannot overflow the thread stack, and a
/// cycle is walked once (every name in it reaches all of its cells). Results for repeated
/// name sets are kept while they stay small in total.
pub(super) struct Closures {
    memo: HashMap<Vec<usize>, Rc<[Rect]>>,
    kept: usize,
    /// The walk number that last visited each name.
    seen: Vec<u32>,
    walk: u32,
    stack: Vec<usize>,
}

impl Closures {
    pub(super) fn new(names: usize) -> Self {
        Closures {
            memo: HashMap::new(),
            kept: 0,
            seen: vec![0; names],
            walk: 0,
            stack: Vec::new(),
        }
    }

    /// The cells reached from `start`, or `None` when the stop callback asks to stop.
    pub(super) fn get(
        &mut self,
        start: &[usize],
        reads: &[Reads],
        stopper: &mut Stopper,
    ) -> Option<Rc<[Rect]>> {
        let mut key = start.to_vec();
        key.sort_unstable();
        key.dedup();
        if let Some(done) = self.memo.get(&key) {
            return Some(Rc::clone(done));
        }
        self.walk = self.walk.wrapping_add(1);
        if self.walk == 0 {
            self.seen.iter_mut().for_each(|s| *s = 0);
            self.walk = 1;
        }
        let mut cells = Vec::new();
        self.stack.clear();
        self.stack.extend(key.iter().copied());
        while let Some(i) = self.stack.pop() {
            if stopper.tick() {
                return None;
            }
            match self.seen.get_mut(i) {
                Some(s) if *s != self.walk => *s = self.walk,
                _ => continue,
            }
            let Some(r) = reads.get(i) else {
                continue;
            };
            cells.extend(r.rects.iter().copied());
            for s in &r.syms {
                if let Sym::Name(j) = s {
                    self.stack.push(*j);
                }
            }
        }
        let cells: Rc<[Rect]> = merge(cells).into();
        if self.kept + cells.len() <= CLOSURE_MEMO_RECTS {
            self.kept += cells.len();
            self.memo.insert(key, Rc::clone(&cells));
        }
        Some(cells)
    }
}
