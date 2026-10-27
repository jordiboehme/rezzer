// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Cell value formatting and the losses it reports.

use super::*;

/// Shows cell values as the file formats them.
pub(super) struct Formatter<'a> {
    pub(super) wb: &'a WorkbookParts,
    pub(super) styles: &'a HashMap<(u32, u32), u32>,
    pub(super) system: DateSystem,
    /// Format ids without a known code: cells and the first cell.
    pub(super) unapplied: RefCell<Unapplied>,
    /// Cells whose style index is not in the style list.
    pub(super) bad_style: RefCell<u64>,
}

impl Formatter<'_> {
    /// The number as shown, with its kind. A format the file names but that is not known
    /// (a locale-dependent built-in) is not applied: the raw value is shown.
    pub(super) fn number(&self, row: u32, col: u32, v: f64) -> (String, NumKind) {
        let general = || numfmt::format_with("General", v, self.system);
        let Some(&s) = self.styles.get(&(row, col)) else {
            let f = general();
            return (f.text, f.kind);
        };
        let f = match self.wb.styles.format_id(s) {
            None => {
                *self.bad_style.borrow_mut() += 1;
                general()
            }
            Some(id) => match self.wb.styles.number_format(s) {
                Some(code) => numfmt::format_with(code, v, self.system),
                None => {
                    let mut map = self.unapplied.borrow_mut();
                    let e = map.entry(id).or_insert((0, (row, col)));
                    e.0 += 1;
                    general()
                }
            },
        };
        (f.text, f.kind)
    }

    pub(super) fn shown(&self, c: &RawCell) -> String {
        match &c.val {
            Val::Empty => String::new(),
            Val::Num(v) => self.number(c.row, c.col, *v).0,
            Val::Text(s) | Val::Error(s) | Val::Iso(s) => s.clone(),
            Val::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        }
    }

    /// The cached value of a formula cell as a typed value.
    pub(super) fn cached(&self, c: &RawCell) -> Option<Value> {
        let code = self
            .styles
            .get(&(c.row, c.col))
            .and_then(|s| self.wb.styles.number_format(*s));
        crate::logic::cell_value(&c.val, code)
    }
}

pub(super) fn format_losses(out: &mut Out, q: &str, fmt: &Formatter) {
    for (id, (count, (row, col))) in fmt.unapplied.borrow().iter() {
        let cells = if *count == 1 {
            "1 cell".to_string()
        } else {
            format!("{count} cells")
        };
        out.loss(
            format!("{q}!{}", grid::a1(*row, *col)),
            format!(
                "number format {id} depends on the locale and is not known: format not applied to {cells} (first {}), the raw value is shown",
                grid::a1(*row, *col)
            ),
            LossReason::Unsupported,
            Severity::Info,
        );
    }
    let bad = *fmt.bad_style.borrow();
    if bad > 0 {
        out.loss(
            q,
            format!(
                "{bad} cells name a cell style that does not exist; shown without a number format"
            ),
            LossReason::Damaged,
            Severity::Info,
        );
    }
}

pub(crate) fn cell_losses(out: &mut Out, q: &str, cells: &SheetCells) {
    if let Some(e) = &cells.error {
        let after = cells
            .cells
            .iter()
            .max_by_key(|c| (c.row, c.col))
            .map(|c| format!(" after cell {}", grid::a1(c.row, c.col)))
            .unwrap_or_default();
        out.loss(
            q,
            format!("the sheet XML could not be read{after}: {e}; the cells before are kept, the rest of the sheet is not read"),
            LossReason::Damaged,
            Severity::Warning,
        );
    }
    match cells.stop {
        Some(Stop::Time) => out.loss(
            q,
            "time limit reached, the rest of the sheet and the later sheets not read",
            LossReason::Limit,
            Severity::Warning,
        ),
        Some(Stop::Elements) => out.loss(
            q,
            "element limit reached, the rest of the sheet and the later sheets not read",
            LossReason::Limit,
            Severity::Warning,
        ),
        None => {}
    }
    if cells.cut.0 > 0 {
        let first: Vec<String> = cells.cut.1.iter().map(|(r, c)| grid::a1(*r, *c)).collect();
        out.loss(
            q,
            format!(
                "{} text cells longer than the text limit were cut (first {})",
                cells.cut.0,
                first.join(", ")
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if cells.duplicates > 0 {
        out.loss(
            q,
            format!(
                "{} cells appear twice in the sheet XML; the first is kept",
                cells.duplicates
            ),
            LossReason::Damaged,
            Severity::Warning,
        );
    }
}

pub(crate) fn problem_loss(out: &mut Out, p: FormulaProblem) {
    let why = if p.unreadable {
        LossReason::Unsupported
    } else {
        LossReason::Damaged
    };
    out.loss(p.anchor, p.what, why, Severity::Warning);
}

pub(crate) fn problem_losses(out: &mut Out, q: &str, problems: Vec<FormulaProblem>) {
    let total = problems.len();
    for p in problems.into_iter().take(LISTED_PROBLEMS) {
        problem_loss(out, p);
    }
    if total > LISTED_PROBLEMS {
        out.loss(
            q,
            format!(
                "{} more formulas could not be read (texts kept)",
                total - LISTED_PROBLEMS
            ),
            LossReason::Unsupported,
            Severity::Warning,
        );
    }
}
