// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small builder for inputs from a text grid, and the checks every test shares.
//!
//! A row is written as cells separated by `|`. A cell token may start with markers: `*` bold,
//! `^` filled, `>` center across selection, `~` one step of indent (a token of markers only is
//! an empty styled cell).
//! Then `=` makes it a formula cell (the rest is its shown result, which may be
//! empty). A token that parses as a number is a number, `TRUE` and `FALSE` are bools, the
//! rest is text. An empty token is no cell.

#![allow(dead_code)]

use sheetlayout::{
    CellIn, Class, Fact, FactKind, FormulaIn, Input, Layout, Limits, NameIn, Pos, Rect, Region,
    Style, detect, segment,
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
    pub header_guess: Option<(u32, bool)>,
    pub styles_read: bool,
    pub outline: Vec<(u32, u32, u8)>,
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
        let mut across = false;
        let mut indent = 0u8;
        loop {
            if let Some(rest) = t.strip_prefix('*') {
                bold = true;
                t = rest;
            } else if let Some(rest) = t.strip_prefix('^') {
                filled = true;
                t = rest;
            } else if let Some(rest) = t.strip_prefix('>') {
                across = true;
                t = rest;
            } else if let Some(rest) = t.strip_prefix('~') {
                indent += 1;
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
            center_across: known(across),
            indent: if indent > 0 || self.styles_read {
                Some(indent)
            } else {
                None
            },
            ..Style::default()
        };
        let (class, number) = if let Ok(n) = t.parse::<f64>() {
            (Class::Number, Some(n))
        } else if t == "TRUE" || t == "FALSE" {
            (Class::Bool, None)
        } else {
            (Class::Text, None)
        };
        self.cells.push(CellIn {
            pos: p,
            class,
            text: t.to_string(),
            number,
            style,
            formula,
        });
    }

    /// Gives the cell at `at` a formula that calls `functions` and reads the ranges `reads`.
    pub fn formula(mut self, at: &str, functions: &[&str], reads: &[&str]) -> Self {
        let p = pos(at);
        let cell = self
            .cells
            .iter_mut()
            .rev()
            .find(|c| c.pos == p)
            .expect("a cell at the formula position");
        cell.formula = Some(FormulaIn {
            functions: functions.iter().map(|f| f.to_string()).collect(),
            reads: reads.iter().map(|r| rect(r)).collect(),
        });
        self
    }

    /// A row outline group from the first to the last sheet row (1-based) at `level`.
    pub fn group(mut self, first: u32, last: u32, level: u8) -> Self {
        self.outline.push((first - 1, last - 1, level));
        self
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
        // The last cell put at a position wins.
        let mut cells = self.cells.clone();
        cells.reverse();
        cells.sort_by_key(|c| (c.pos.row, c.pos.col));
        cells.dedup_by_key(|c| c.pos);
        Input {
            cells,
            merges: self.merges.clone(),
            facts: self.facts.clone(),
            names: self.names.clone(),
            outline: self.outline.clone(),
            frozen: self.frozen,
            print_titles: self.print_titles,
            header_guess: self.header_guess,
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

    /// Detects with the default limits and no stop, with the same checks as `segment`.
    pub fn detect(&self) -> Layout {
        let input = self.input();
        let layout = detect(&input, &Limits::default(), &never);
        every_cell_once(&input, &layout);
        no_overlap(&layout);
        layout
    }

    /// Detects with other label and unit callbacks, with the same checks.
    pub fn detect_with(
        &self,
        is_label: &dyn Fn(&str) -> bool,
        is_unit_column: &dyn Fn(&[&str]) -> bool,
    ) -> Layout {
        let input = Input {
            is_label,
            is_unit_column,
            ..self.input()
        };
        let layout = detect(&input, &Limits::default(), &never);
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
        || r.above_header.contains(&p.row)
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

/// The longest a work bound test may take in thread CPU time. It only catches a runaway:
/// the bound itself is [`assert_work`] on a count of work steps, which does not depend on how
/// fast the runner is (a CI runner took 2.4 s for work that takes 1.3 s on a laptop, against a
/// time bound of 2 s).
pub const CPU_SAFETY: std::time::Duration = std::time::Duration::from_secs(10);

/// Runs `f` with a stop callback that never stops, and gives the result, the steps of layout
/// work it did ([`sheetlayout::work_steps`], `None` in a release build) and the thread CPU
/// time it took.
///
/// The steps are the work bound of these tests: the layout code counts the cells it sorts,
/// the items of every piece the cut looks at, the visits the corner split stops on and the
/// steps of the other walks. The count is the same on every machine for the same input, so a
/// slow runner cannot fail the test and a change that makes the work grow cannot pass it.
pub fn counted<T>(f: impl FnOnce(&dyn Fn() -> bool) -> T) -> (T, Option<u64>, std::time::Duration) {
    let before = sheetlayout::work_steps();
    let start = thread_cpu();
    let out = f(&|| false);
    let used = thread_cpu() - start;
    let steps = sheetlayout::work_steps().zip(before).map(|(a, b)| a - b);
    (out, steps, used)
}

/// Checks a work bound: at most `max` steps of work (checked in debug builds, which count
/// them), and thread CPU time under the safety stop. Each test's bound is the count measured
/// when the bound was set (2026-10-27), with half of it again as margin; a growth past that
/// is a change in the work, not a slow machine.
pub fn assert_work(what: &str, steps: Option<u64>, max: u64, used: std::time::Duration) {
    eprintln!("{what}: {steps:?} steps (bound {max}), {used:?} CPU");
    if let Some(steps) = steps {
        assert!(steps <= max, "{what}: {steps} steps of work, bound {max}");
    }
    assert!(used < CPU_SAFETY, "{what}: {used:?} CPU");
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
