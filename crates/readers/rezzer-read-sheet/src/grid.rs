// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The cells of one sheet, read sparsely from calamine, and helpers for cell positions.

use std::collections::HashMap;
use std::io::{Read, Seek};

use calamine::{DataRef, Xlsx, XlsxFormulaMetadata};
use rezzer_limits::Budget;
use rezzer_model::{Block, Inline, column_letter};

use crate::logic::TextMeter;

/// Cells read between two checks of the time and the stop flag.
const CHECK_EVERY: u64 = 1_024;

/// A cell value as stored in the file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    Empty,
    Num(f64),
    Text(String),
    Bool(bool),
    Error(String),
    /// A date, time or duration that the file stores as ISO 8601 text.
    Iso(String),
}

/// One cell with a value or a formula.
#[derive(Debug, Clone)]
pub(crate) struct RawCell {
    pub row: u32,
    pub col: u32,
    pub val: Val,
    /// The formula text without `=`, as stored (shared formulas expanded).
    pub formula: Option<String>,
    /// The cell belongs to a shared formula group.
    pub shared: bool,
    /// A shared formula cell whose master cell was not seen first, so its text is unknown.
    pub shared_unknown: bool,
    /// The formula text was not kept: the workbook's formula text limit was reached.
    pub text_dropped: bool,
}

/// Why reading cells stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    Time,
    Elements,
}

/// The cells read from one sheet, sorted by row and column, each position once.
#[derive(Debug, Default)]
pub(crate) struct SheetCells {
    pub cells: Vec<RawCell>,
    /// calamine stopped with this error after the cells in `cells`.
    pub error: Option<String>,
    pub stop: Option<Stop>,
    /// Text cells cut at the text limit: count and the first cells.
    pub cut: (u64, Vec<(u32, u32)>),
    /// Cells that appeared twice; the first one is kept.
    pub duplicates: u64,
}

/// Reads the cells of a sheet with calamine's streaming cell reader. Empty cells without a
/// formula are skipped; every kept cell costs one budget element.
pub(crate) fn read_cells<RS: Read + Seek>(
    cal: &mut Xlsx<RS>,
    sheet: &str,
    budget: &Budget,
    meter: &mut TextMeter,
) -> SheetCells {
    let mut out = SheetCells::default();
    let mut reader = match cal.worksheet_cells_reader(sheet) {
        Ok(r) => r,
        Err(e) => {
            out.error = Some(e.to_string());
            return out;
        }
    };
    let max_text = usize::try_from(budget.limits().max_text_bytes).unwrap_or(usize::MAX);
    // Shared formula masters: index to (text, master position).
    let mut shared: HashMap<usize, (String, (u32, u32))> = HashMap::new();
    let mut seen: u64 = 0;
    loop {
        seen += 1;
        if seen.is_multiple_of(CHECK_EVERY) && budget.check_time().is_err() {
            out.stop = Some(Stop::Time);
            break;
        }
        let record = match reader.next_cell_with_formula_metadata() {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => {
                out.error = Some(e.to_string());
                break;
            }
        };
        let (row, col) = record.pos;
        // An `f` element marks a formula cell even when its text is empty (a data table
        // master or an array cell) and no value was stored.
        let has_f = record.formula.is_some();
        let (mut formula, mut is_shared, mut unknown, mut dropped) = (None, false, false, false);
        match record.formula {
            Some(XlsxFormulaMetadata::Normal { formula: f }) => {
                if meter.take(f.len()) {
                    formula = Some(f);
                } else {
                    dropped = true;
                }
            }
            Some(XlsxFormulaMetadata::Shared {
                shared_index,
                range,
                formula: f,
            }) => {
                is_shared = true;
                let start = range.map_or((row, col), |r| r.start);
                // The text applies to the master cell itself relative to the range start.
                if meter.fits(f.len()) {
                    formula = if start == (row, col) {
                        Some(f.clone())
                    } else {
                        calamine::expand_shared_formula(&f, start, (row, col)).ok()
                    };
                    unknown = formula.is_none();
                } else {
                    dropped = true;
                }
                shared.insert(shared_index, (f, start));
            }
            Some(XlsxFormulaMetadata::SharedDerived { shared_index }) => {
                is_shared = true;
                match shared.get(&shared_index) {
                    // A copy is about as long as the master: check before expanding, charge
                    // the real length after.
                    Some((f, _)) if !meter.fits(f.len()) => dropped = true,
                    Some((f, start)) => {
                        match calamine::expand_shared_formula(f, *start, (row, col)) {
                            Ok(text) => formula = Some(text),
                            Err(_) => unknown = true,
                        }
                    }
                    None => unknown = true,
                }
            }
            Some(_) => {}
            None => {}
        }
        if is_shared && !dropped && !unknown {
            match &formula {
                Some(f) if !meter.take(f.len()) => {
                    formula = None;
                    dropped = true;
                }
                _ => {}
            }
        }
        // Data table masters and the inner cells of an array come back with empty text;
        // their meaning comes from the formula attributes.
        let formula = formula.filter(|f| !f.is_empty());
        let val = match record.value {
            DataRef::Empty => Val::Empty,
            DataRef::Int(i) => Val::Num(i as f64),
            DataRef::Float(f) => Val::Num(f),
            DataRef::Bool(b) => Val::Bool(b),
            DataRef::DateTime(d) => Val::Num(d.as_f64()),
            DataRef::DateTimeIso(s) | DataRef::DurationIso(s) => Val::Iso(s),
            DataRef::Error(e) => Val::Error(e.to_string()),
            DataRef::String(s) => Val::Text(s),
            DataRef::SharedString(s) => Val::Text(s.to_string()),
        };
        if val == Val::Empty && !has_f {
            continue;
        }
        let val = match val {
            Val::Text(s) if budget.charge_text(s.len()).is_err() => {
                out.cut.0 += 1;
                if out.cut.1.len() < 5 {
                    out.cut.1.push((row, col));
                }
                Val::Text(cut_text(s, max_text))
            }
            other => other,
        };
        if budget.add_elements(1).is_err() {
            out.stop = Some(Stop::Elements);
            break;
        }
        out.cells.push(RawCell {
            row,
            col,
            val,
            formula,
            shared: is_shared,
            shared_unknown: unknown,
            text_dropped: dropped,
        });
    }
    // Hostile files may repeat or reorder cells: sort, keep the first of each position.
    out.cells.sort_by_key(|c| (c.row, c.col));
    let before = out.cells.len();
    out.cells.dedup_by_key(|c| (c.row, c.col));
    out.duplicates = (before - out.cells.len()) as u64;
    out
}

/// `text` cut to at most `max` bytes at a char boundary.
pub(crate) fn cut_text(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// The A1 address of a 0-based position.
pub(crate) fn a1(row: u32, col: u32) -> String {
    format!("{}{}", column_letter(col as usize), u64::from(row) + 1)
}

/// A 0-based position from an A1 address (`B5`, `$B$5`).
pub(crate) fn parse_cell(text: &str) -> Option<(u32, u32)> {
    let text = text.trim().replace('$', "");
    let letters = text.chars().take_while(char::is_ascii_alphabetic).count();
    if letters == 0 || letters > 3 {
        return None;
    }
    let (l, d) = text.split_at(letters);
    if d.is_empty() || d.len() > 7 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut col = 0u32;
    for c in l.bytes() {
        col = col * 26 + u32::from(c.to_ascii_uppercase() - b'A' + 1);
    }
    let row: u32 = d.parse().ok()?;
    if !(1..=16_384).contains(&col) || !(1..=1_048_576).contains(&row) {
        return None;
    }
    Some((row - 1, col - 1))
}

/// A 0-based area from `A1:C9` or a single `A1`, start before end.
pub(crate) fn parse_area(text: &str) -> Option<((u32, u32), (u32, u32))> {
    let (a, b) = match text.split_once(':') {
        Some((a, b)) => (parse_cell(a)?, parse_cell(b)?),
        None => {
            let a = parse_cell(text)?;
            (a, a)
        }
    };
    Some(((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1))))
}

/// `8 to 11, 15` from 0-based spans; `letters` writes columns as letters.
pub(crate) fn span_list(spans: &[sheetparts::RowSpan], letters: bool) -> String {
    let name = |n: u32| {
        if letters {
            column_letter(n as usize)
        } else {
            (u64::from(n) + 1).to_string()
        }
    };
    spans
        .iter()
        .map(|s| {
            if s.first == s.last {
                name(s.first)
            } else {
                format!("{} to {}", name(s.first), name(s.last))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A paragraph of plain text.
pub(crate) fn note(text: impl Into<String>) -> Block {
    Block::paragraph(vec![Inline::plain(text)])
}
