// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small builder for inputs from a text grid, and the checks every test shares.
//!
//! A row is written as cells separated by `|`. A cell token may start with markers: `*` bold,
//! `^` filled. Then `=` makes it a formula cell (the rest is its shown result, which may be
//! empty). A token that parses as a number is a number, `TRUE` and `FALSE` are bools, the
//! rest is text. An empty token is no cell.

#![allow(dead_code)]

use sheetlayout::{
    CellIn, Class, Fact, FactKind, FormulaIn, Input, Layout, Limits, NameIn, Pos, Rect, Region,
    Style, segment,
};

/// `B3` as a 0-based position.
pub fn pos(a1: &str) -> Pos {
    let letters: String = a1.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits = &a1[letters.len()..];
    let mut col = 0u32;
    for c in letters.chars() {
        col = col * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
    }
    let row: u32 = digits.parse().expect("row number");
    Pos {
        row: row - 1,
        col: col - 1,
    }
}

/// `B3:D5` or `B3` as an inclusive 0-based rectangle.
pub fn rect(a1: &str) -> Rect {
    let (a, b) = a1.split_once(':').unwrap_or((a1, a1));
    let (a, b) = (pos(a), pos(b));
    Rect {
        top: a.row,
        left: a.col,
        bottom: b.row,
        right: b.col,
    }
}

fn any_label(_: &str) -> bool {
    true
}

fn no_unit(_: &[&str]) -> bool {
    false
}

fn never() -> bool {
    false
}

#[derive(Default, Clone)]
pub struct Sheet {
    pub cells: Vec<CellIn>,
    pub merges: Vec<Rect>,
    pub facts: Vec<Fact>,
    pub names: Vec<NameIn>,
    pub frozen: Option<(u32, u32)>,
    pub print_titles: Option<(u32, u32)>,
    pub styles_read: bool,
}

impl Sheet {
    /// A sheet without styles (every style flag unknown).
    pub fn new() -> Self {
        Self::default()
    }

    /// A sheet whose styles are read: a cell without a marker is not bold and not filled.
    pub fn styled() -> Self {
        Self {
            styles_read: true,
            ..Self::default()
        }
    }

    /// Writes `rows` from the cell `at` down, one line per row.
    pub fn rows(mut self, at: &str, rows: &[&str]) -> Self {
        let start = pos(at);
        for (i, line) in rows.iter().enumerate() {
            for (j, token) in line.split('|').enumerate() {
                let p = Pos {
                    row: start.row + i as u32,
                    col: start.col + j as u32,
                };
                self.put(p, token);
            }
        }
        self
    }

    pub fn cell(mut self, at: &str, token: &str) -> Self {
        self.put(pos(at), token);
        self
    }

    pub fn put(&mut self, p: Pos, token: &str) {
        let mut t = token.trim();
        if t.is_empty() {
            return;
        }
        let mut bold = false;
        let mut filled = false;
        loop {
            if let Some(rest) = t.strip_prefix('*') {
                bold = true;
                t = rest;
            } else if let Some(rest) = t.strip_prefix('^') {
                filled = true;
                t = rest;
            } else {
                break;
            }
        }
        let mut formula = None;
        if let Some(rest) = t.strip_prefix('=') {
            formula = Some(FormulaIn {
                functions: Vec::new(),
                reads: Vec::new(),
            });
            t = rest;
        }
        let known = |b: bool| if self.styles_read { Some(b) } else { None };
        let style = Style {
            bold: known(bold),
            filled: known(filled),
            ..Style::default()
        };
        let (class, number) = if let Ok(n) = t.parse::<f64>() {
            (Class::Number, Some(n))
        } else if t == "TRUE" || t == "FALSE" {
            (Class::Bool, None)
        } else {
            (Class::Text, None)
        };
        self.cells.retain(|c| c.pos != p);
        self.cells.push(CellIn {
            pos: p,
            class,
            text: t.to_string(),
            number,
            style,
            formula,
        });
    }

    pub fn merge(mut self, r: &str) -> Self {
        self.merges.push(rect(r));
        self
    }

    pub fn fact(mut self, r: &str, kind: FactKind) -> Self {
        self.facts.push(Fact {
            rect: rect(r),
            kind,
        });
        self
    }

    pub fn name(mut self, name: &str, r: &str) -> Self {
        self.names.push(NameIn {
            name: name.to_string(),
            rect: rect(r),
            hidden: false,
            builtin: false,
        });
        self
    }

    pub fn input(&self) -> Input<'static> {
        let mut cells = self.cells.clone();
        cells.sort_by_key(|c| (c.pos.row, c.pos.col));
        Input {
            cells,
            merges: self.merges.clone(),
            facts: self.facts.clone(),
            names: self.names.clone(),
            outline: Vec::new(),
            frozen: self.frozen,
            print_titles: self.print_titles,
            header_guess: None,
            styles_read: self.styles_read,
            is_label: &any_label,
            is_unit_column: &no_unit,
        }
    }

    /// Segments with the default limits and no stop, and checks that every cell is placed once.
    pub fn segment(&self) -> Layout {
        let input = self.input();
        let layout = segment(&input, &Limits::default(), &never);
        every_cell_once(&input, &layout);
        no_overlap(&layout);
        layout
    }
}

/// A list object fact with one header row.
pub fn list_object(name: &str) -> FactKind {
    FactKind::ListObject {
        name: name.to_string(),
        header_rows: 1,
        totals_rows: 0,
        columns: Vec::new(),
    }
}

/// The ranges of the regions in reading order.
pub fn ranges(layout: &Layout) -> Vec<String> {
    layout.regions.iter().map(|r| r.rect.a1()).collect()
}

fn holds(r: &Region, p: Pos) -> bool {
    let inside =
        |x: &Rect| x.top <= p.row && p.row <= x.bottom && x.left <= p.col && p.col <= x.right;
    if !inside(&r.rect) {
        return false;
    }
    r.captions.contains(&p.row)
        || r.header_rows.contains(&p.row)
        || r.notes.contains(&p.row)
        || r.heading.map(|h| h.0) == Some(p.row)
        || r.data.as_ref().is_some_and(inside)
}

/// Every non-empty cell lies in exactly one region: in a caption, header, note or heading row
/// or in the data area.
pub fn every_cell_once(input: &Input, layout: &Layout) {
    let mut cells: Vec<Pos> = input
        .cells
        .iter()
        .filter(|c| !c.text.is_empty() || c.formula.is_some())
        .map(|c| c.pos)
        .collect();
    cells.sort();
    cells.dedup();
    let mut count = vec![0u32; cells.len()];
    for r in &layout.regions {
        let from = cells.partition_point(|p| p.row < r.rect.top);
        let to = cells.partition_point(|p| p.row <= r.rect.bottom);
        for k in from..to.max(from) {
            if holds(r, cells[k]) {
                count[k] += 1;
            }
        }
    }
    for (p, n) in cells.iter().zip(&count) {
        assert_eq!(
            *n, 1,
            "cell at row {} col {} is in {n} regions",
            p.row, p.col
        );
    }
}

/// No two regions share a position.
pub fn no_overlap(layout: &Layout) {
    let mut rects: Vec<Rect> = layout.regions.iter().map(|r| r.rect).collect();
    rects.sort_by_key(|r| r.top);
    for (i, a) in rects.iter().enumerate() {
        for b in rects[i + 1..].iter().take_while(|b| b.top <= a.bottom) {
            let apart = b.right < a.left || a.right < b.left;
            assert!(apart, "regions {} and {} overlap", a.a1(), b.a1());
        }
    }
}

/// The CPU time the calling thread has used: work bounds do not depend on a busy machine.
#[cfg(unix)]
pub fn thread_cpu() -> std::time::Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime fills the timespec we pass.
    let r = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    assert_eq!(r, 0, "clock_gettime");
    std::time::Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

#[cfg(not(unix))]
pub fn thread_cpu() -> std::time::Duration {
    std::time::Duration::ZERO
}
