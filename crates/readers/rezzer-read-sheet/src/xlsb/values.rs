// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sheet layout and the values read through calamine.

use super::*;

/// The layout of an XLSB sheet: its cells with their classes, its merges and its defined
/// names. Styles, tables, filters, outline, panes and print titles are not read, so every
/// style flag is unknown.
#[allow(clippy::too_many_arguments)]
pub(super) fn sheet_layout(
    q: &str,
    name: &str,
    names: &[DefinedName],
    builtin: &HashSet<String>,
    cells: &SheetCells,
    dates: &HashMap<(u32, u32), u32>,
    scan: &SheetScan,
    shown: &dyn Fn(&RawCell) -> String,
    formulas: HashMap<(u32, u32), sheetlayout::FormulaIn>,
    budget: &Budget,
) -> layout::SheetLayout {
    let shown = |c: &RawCell| {
        let (class, number) = match &c.val {
            Val::Num(v) if dates.contains_key(&(c.row, c.col)) => {
                (sheetlayout::Class::Date, Some(*v))
            }
            Val::Num(v) => (sheetlayout::Class::Number, Some(*v)),
            Val::Iso(_) => (sheetlayout::Class::Date, None),
            Val::Bool(_) => (sheetlayout::Class::Bool, None),
            Val::Error(_) => (sheetlayout::Class::Error, None),
            Val::Text(_) | Val::Empty => (sheetlayout::Class::Text, None),
        };
        layout::Shown {
            text: shown(c),
            class,
            number,
        }
    };
    let style = |_: u32, _: u32| sheetlayout::Style::default();
    // A name the file marks as built in is never a caption, with or without `_xlnm.`.
    let (mut names, mut caption_names) = layout::sheet_names(names, name);
    for n in &mut names {
        n.builtin |= builtin.contains(&n.name.to_lowercase());
    }
    caption_names.retain(|c| !builtin.contains(&c.name.to_lowercase()));
    let signals = layout::Signals {
        names,
        ..layout::Signals::default()
    };
    let merges: Vec<sheetlayout::Rect> = scan
        .merges
        .iter()
        .map(|&(r0, r1, c0, c1)| layout::rect_of(((r0, c0), (r1, c1))).normalized())
        .collect();
    layout::map_sheet(
        q,
        &cells.cells,
        &shown,
        &style,
        &[],
        &merges,
        formulas,
        signals,
        &caption_names,
        budget,
    )
}

/// The losses of merge records the walk could not use.
pub(super) fn merge_record_losses(out: &mut Out, q: &str, scan: &SheetScan, max_merges: u64) {
    if scan.merges_bad > 0 {
        out.loss(
            q,
            format!(
                "{} merged-cell records are too short or reach past the last row or column; those areas are shown as single cells",
                scan.merges_bad
            ),
            LossReason::Damaged,
            Severity::Info,
        );
    }
    if scan.merges_cut > 0 {
        out.loss(
            q,
            format!(
                "more than {max_merges} merged areas: {} not read; those areas are shown as single cells",
                scan.merges_cut
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if let Some(e) = &scan.merge_error {
        out.loss(
            q,
            format!(
                "merged cells not read past a damaged record: {e}; those areas are shown as single cells"
            ),
            LossReason::Damaged,
            Severity::Info,
        );
    }
}

/// The format key of a date calamine found: date, date and time, time of day, duration.
fn date_key(d: &calamine::ExcelDateTime) -> u32 {
    let v = d.as_f64();
    if d.is_duration() {
        3
    } else if (0.0..1.0).contains(&v) {
        2
    } else if v.fract() == 0.0 {
        0
    } else {
        1
    }
}

/// Reads the values of a sheet with calamine's cell reader. Dates get a format key.
pub(super) fn read_values(
    cal: &mut Xlsb<Cursor<&[u8]>>,
    sheet: &str,
    budget: &Budget,
    dates: &mut HashMap<(u32, u32), u32>,
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
    let mut seen = 0u64;
    loop {
        seen += 1;
        if seen.is_multiple_of(CHECK_EVERY) && budget.check_time().is_err() {
            out.stop = Some(Stop::Time);
            break;
        }
        let cell = match reader.next_cell() {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                out.error = Some(e.to_string());
                break;
            }
        };
        let (row, col) = cell.get_position();
        let val = match cell.get_value() {
            DataRef::Empty => continue,
            DataRef::Int(i) => Val::Num(*i as f64),
            DataRef::Float(f) => Val::Num(*f),
            DataRef::Bool(b) => Val::Bool(*b),
            DataRef::DateTime(d) => {
                dates.insert((row, col), date_key(d));
                Val::Num(d.as_f64())
            }
            DataRef::DateTimeIso(s) | DataRef::DurationIso(s) => Val::Iso(s.clone()),
            DataRef::Error(e) => Val::Error(e.to_string()),
            DataRef::String(s) => Val::Text(s.clone()),
            DataRef::SharedString(s) => Val::Text(s.to_string()),
        };
        let val = match val {
            Val::Text(s) if budget.charge_text(s.len()).is_err() => {
                out.cut.0 += 1;
                if out.cut.1.len() < 5 {
                    out.cut.1.push((row, col));
                }
                Val::Text(grid::cut_text(s, max_text))
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
            formula: None,
            shared: false,
            shared_unknown: false,
            text_dropped: false,
        });
    }
    out.cells.sort_by_key(|c| (c.row, c.col));
    let before = out.cells.len();
    out.cells.dedup_by_key(|c| (c.row, c.col));
    out.duplicates = (before - out.cells.len()) as u64;
    out
}
