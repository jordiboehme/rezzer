// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The layout of a sheet: its cells, merges, facts and signals as `sheetlayout` input, the
//! detection, and the regions mapped into blocks by `rezzer_layout`. Past the input builders
//! this is the same for every format.

use std::collections::HashMap;

use rezzer_layout::{Caps, CellText, ColumnNames, Mapped};
use rezzer_limits::Budget;
use sheetlayout::{CellIn, Class, Fact, FactKind, FormulaIn, Input, NameIn, Pos, Rect, Style};
use sheetparts::{DefinedName, Look};

use crate::grid::{self, RawCell, Val};
use crate::logic;

/// Empty cells with center across selection passed to the detector per sheet, at most. They
/// only tell how far a text to their left spreads.
const MAX_CENTER_CELLS: usize = 100_000;

/// What a sheet says about its layout beyond its cells.
#[derive(Debug, Default)]
pub(crate) struct Signals {
    pub facts: Vec<Fact>,
    pub names: Vec<NameIn>,
    pub outline: Vec<(u32, u32, u8)>,
    pub frozen: Option<(u32, u32)>,
    pub print_titles: Option<(u32, u32)>,
    /// Whether the style flags were read at all.
    pub styles_read: bool,
}

/// One cell as the detector and the tables see it.
pub(crate) struct Shown {
    pub text: String,
    pub class: Class,
    pub number: Option<f64>,
}

/// The layout of one sheet, mapped into blocks, and the number of merged areas ignored
/// because they overlap one before them.
pub(crate) struct SheetLayout {
    pub mapped: Mapped,
    pub merges_ignored: u64,
}

/// The style flags of a look; every flag is known.
pub(crate) fn style_of(look: Look) -> Style {
    Style {
        bold: Some(look.bold),
        filled: Some(look.filled),
        indent: Some(look.indent),
        center_across: Some(look.center_across),
        rule_above: Some(look.rule_above),
        rule_below: Some(look.rule_below),
    }
}

/// Detects the layout of one sheet and maps it into blocks. `cells` are sorted and each
/// position is there once; `shown` gives a cell's shown text and class, `style` its style
/// flags; `centered` lists empty positions with center across selection; `formulas` are
/// the formula facts of the sheet.
#[allow(clippy::too_many_arguments)]
pub(crate) fn map_sheet(
    sheet_q: &str,
    cells: &[RawCell],
    shown: &dyn Fn(&RawCell) -> Shown,
    style: &dyn Fn(u32, u32) -> Style,
    centered: &[(u32, u32)],
    merges: &[Rect],
    mut formulas: HashMap<(u32, u32), FormulaIn>,
    signals: Signals,
    caption_names: &[String],
    budget: &Budget,
) -> SheetLayout {
    let (merges, merges_ignored) = rezzer_layout::clean_merges(merges);
    let at: Vec<(u32, u32)> = cells.iter().map(|c| (c.row, c.col)).collect();
    let hidden = rezzer_layout::covered(&at, &merges);
    let mut texts = Vec::with_capacity(cells.len());
    let mut input_cells = Vec::with_capacity(cells.len());
    for (c, covered) in cells.iter().zip(hidden) {
        let s = shown(c);
        // A formula cell, even when its text was not kept or its result is empty.
        let has_formula = c.formula.is_some()
            || c.shared
            || c.shared_unknown
            || c.text_dropped
            || c.val == Val::Empty;
        if !covered {
            input_cells.push(CellIn {
                pos: Pos {
                    row: c.row,
                    col: c.col,
                },
                class: s.class,
                text: s.text.clone(),
                number: s.number,
                style: style(c.row, c.col),
                formula: has_formula.then(|| formulas.remove(&(c.row, c.col)).unwrap_or_default()),
            });
        }
        texts.push(CellText {
            row: c.row,
            col: c.col,
            text: s.text,
        });
    }
    // Empty cells with center across selection, where no cell and no merge is.
    let free = rezzer_layout::covered(centered, &merges);
    let mut extra: Vec<CellIn> = centered
        .iter()
        .zip(free)
        .filter(|(p, under)| !under && at.binary_search(p).is_err())
        .map(|(p, _)| p)
        .take(MAX_CENTER_CELLS)
        .map(|&(row, col)| CellIn {
            pos: Pos { row, col },
            class: Class::Text,
            text: String::new(),
            number: None,
            style: style(row, col),
            formula: None,
        })
        .collect();
    if !extra.is_empty() {
        input_cells.append(&mut extra);
        input_cells.sort_by_key(|c| c.pos);
    }
    let input = Input {
        cells: input_cells,
        merges: merges.clone(),
        facts: signals.facts,
        names: signals.names,
        outline: signals.outline,
        frozen: signals.frozen,
        print_titles: signals.print_titles,
        header_guess: None,
        styles_read: signals.styles_read,
        is_label: &logic::is_label,
        is_unit_column: &logic::is_unit_column_texts,
    };
    let stop = || budget.check_time().is_err();
    let layout = sheetlayout::detect(&input, &rezzer_layout::limits(budget), &stop);
    drop(input);
    let mut caps = Caps::default();
    let mapped = rezzer_layout::blocks(
        sheet_q,
        &layout,
        &texts,
        &merges,
        ColumnNames::Letters,
        caption_names,
        &mut caps,
        budget,
    );
    SheetLayout {
        mapped,
        merges_ignored,
    }
}

/// A 0-based area as a rectangle.
pub(crate) fn rect_of(area: ((u32, u32), (u32, u32))) -> Rect {
    let ((top, left), (bottom, right)) = area;
    Rect {
        top,
        left,
        bottom,
        right,
    }
}

/// The rectangle a defined name refers to when its whole text is one area on `sheet`
/// (`Sheet1!$A$4:$F$62`, `'My sheet'!B2:C9`).
pub(crate) fn name_rect(refers_to: &str, sheet: &str) -> Option<Rect> {
    let text = refers_to.trim();
    let text = text.strip_prefix('=').unwrap_or(text);
    // The reference follows the last `!` outside quotes; a comma outside quotes makes a
    // list of areas.
    let mut quoted = false;
    let mut at = None;
    for (i, c) in text.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            '!' if !quoted => at = Some(i),
            ',' | ' ' | '(' if !quoted => return None,
            _ => {}
        }
    }
    let at = at?;
    let (name, area) = (&text[..at], &text[at + 1..]);
    let name = match name.strip_prefix('\'').and_then(|n| n.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => name.to_string(),
    };
    if name.contains(['[', ':']) || name.to_lowercase() != sheet.to_lowercase() {
        return None;
    }
    grid::parse_area(area).map(rect_of)
}

/// The defined names that are one area on `sheet`, and the ones among them that may stand
/// as a caption (not hidden, not written by the application for itself).
pub(crate) fn sheet_names(names: &[DefinedName], sheet: &str) -> (Vec<NameIn>, Vec<String>) {
    let mut list = Vec::new();
    let mut captions = Vec::new();
    for n in names {
        let Some(rect) = name_rect(&n.refers_to, sheet) else {
            continue;
        };
        let builtin = n
            .name
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case("_xlnm."));
        if !n.hidden && !builtin {
            captions.push(n.name.clone());
        }
        list.push(NameIn {
            name: n.name.clone(),
            rect,
            hidden: n.hidden,
            builtin,
        });
    }
    (list, captions)
}

/// Runs of rows with one outline level, from `(row, level)` pairs.
pub(crate) fn outline_runs(rows: &[(u32, u8)]) -> Vec<(u32, u32, u8)> {
    let mut sorted = rows.to_vec();
    sorted.sort_unstable();
    let mut out: Vec<(u32, u32, u8)> = Vec::new();
    for (row, level) in sorted {
        match out.last_mut() {
            Some(last) if last.2 == level && last.1.checked_add(1) == Some(row) => last.1 = row,
            _ => out.push((row, row, level)),
        }
    }
    out
}

/// An Excel table as a fact.
pub(crate) fn table_fact(
    area: ((u32, u32), (u32, u32)),
    name: &str,
    header_rows: u32,
    totals_rows: u32,
    columns: &[String],
) -> Fact {
    Fact {
        rect: rect_of(area),
        kind: FactKind::ListObject {
            name: name.to_string(),
            header_rows,
            totals_rows,
            columns: columns.to_vec(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_on_this_sheet_only() {
        let r = |t: &str| name_rect(t, "NST-EST2023-POP");
        assert_eq!(
            r("'NST-EST2023-POP'!$A$4:$F$62"),
            Some(Rect {
                top: 3,
                left: 0,
                bottom: 61,
                right: 5
            })
        );
        assert_eq!(r("=nst-est2023-pop!A1").map(|x| x.a1()), Some("A1".into()));
        assert_eq!(r("Other!$A$1:$B$2"), None);
        assert_eq!(r("'NST-EST2023-POP'!A1:B2,'NST-EST2023-POP'!C1"), None);
        assert_eq!(r("[1]'NST-EST2023-POP'!A1"), None);
        assert_eq!(r("{1,2}"), None);
        assert_eq!(
            name_rect("'it''s'!B2:C3", "it's").map(|x| x.a1()),
            Some("B2:C3".into())
        );
    }

    #[test]
    fn outline_rows_fold_into_runs() {
        assert_eq!(
            outline_runs(&[(5, 1), (2, 1), (3, 1), (4, 2), (7, 1)]),
            [(2, 3, 1), (4, 4, 2), (5, 5, 1), (7, 7, 1)]
        );
    }
}
