// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Formulas and defined names of an XLSB workbook.

use super::*;

/// What the formula pass found for one formula cell.
enum FText {
    Text(String),
    /// The workbook's formula text limit was reached.
    Dropped,
    /// The tokens refer to a shared or array formula, whose text calamine does not read.
    Shared,
    /// calamine could not parse the tokens.
    Unreadable(String),
}

#[derive(Default)]
pub(super) struct FormulaPass {
    texts: HashMap<(u32, u32), FText>,
    /// calamine stopped reading the part with this error.
    pub(super) error: Option<String>,
    pub(super) stopped: bool,
}

/// Reads the formula texts of a sheet. Each text is charged to the formula text meter.
pub(super) fn read_formulas(
    cal: &mut Xlsb<Cursor<&[u8]>>,
    sheet: &str,
    scan: &SheetScan,
    formulas: &mut Formulas,
    budget: &Budget,
) -> FormulaPass {
    let mut pass = FormulaPass::default();
    let mut reader = match cal.worksheet_cells_reader(sheet) {
        Ok(r) => r,
        Err(e) => {
            pass.error = Some(e.to_string());
            return pass;
        }
    };
    // calamine returns the formula records in the order the walk found them; a record it
    // cannot parse is consumed, so the walk names its cell.
    let mut k = 0usize;
    loop {
        // One formula may take calamine a while to rebuild: the time is checked for each.
        if budget.check_time().is_err() {
            pass.stopped = true;
            break;
        }
        match reader.next_formula() {
            Ok(Some(c)) => {
                k += 1;
                let text = c.get_value();
                let entry = if text.is_empty() {
                    FText::Shared
                } else if formulas.meter.take(text.len()) {
                    FText::Text(text.clone())
                } else {
                    FText::Dropped
                };
                pass.texts.insert(c.get_position(), entry);
            }
            Ok(None) => break,
            Err(XlsbError::Io(e)) => {
                pass.error = Some(e.to_string());
                break;
            }
            Err(e) => {
                let Some(pos) = scan.formulas.get(k).copied() else {
                    pass.error = Some(e.to_string());
                    break;
                };
                k += 1;
                pass.texts.insert(pos, FText::Unreadable(e.to_string()));
            }
        }
    }
    pass
}

/// Every formula cell the walk found is marked as one, also when its text was not read, so
/// its cached result is never taken for a constant. Formula cells whose result is an error
/// are not in calamine's value pass: they are added with the error the walk read. Returns
/// the formula cells left out at the element limit.
pub(super) fn merge_formula_cells(
    cells: &mut SheetCells,
    pass: &FormulaPass,
    scan: &SheetScan,
    budget: &Budget,
) -> u64 {
    let have: HashSet<(u32, u32)> = cells.cells.iter().map(|c| (c.row, c.col)).collect();
    let mut added = false;
    let mut missing = 0u64;
    for pos in &scan.formulas {
        if have.contains(pos) {
            continue;
        }
        if budget.add_elements(1).is_err() {
            missing += 1;
            continue;
        }
        let val = match scan.error_results.get(pos) {
            Some(e) => Val::Error((*e).to_string()),
            None => Val::Empty,
        };
        cells.cells.push(RawCell {
            row: pos.0,
            col: pos.1,
            val,
            formula: None,
            shared: false,
            shared_unknown: false,
            text_dropped: false,
        });
        added = true;
    }
    if added {
        cells.cells.sort_by_key(|c| (c.row, c.col));
        cells.cells.dedup_by_key(|c| (c.row, c.col));
    }
    let formula_cells: HashSet<&(u32, u32)> = scan.formulas.iter().collect();
    for c in &mut cells.cells {
        let pos = (c.row, c.col);
        if !formula_cells.contains(&pos) && !pass.texts.contains_key(&pos) {
            continue;
        }
        // Only "is a formula" matters from here on; the texts stay in the pass.
        c.formula = Some(String::new());
        c.shared_unknown = matches!(pass.texts.get(&pos), Some(FText::Shared));
        c.text_dropped = matches!(pass.texts.get(&pos), Some(FText::Dropped));
    }
    missing
}

/// Calls in a formula text that calamine writes for tokens it does not resolve: an add-in
/// or newer Excel function (`EXTERNAL_WB_NAME`) or a user-defined function (`User(`).
fn unresolved_call(text: &str) -> bool {
    if text.contains("EXTERNAL_WB_NAME") {
        return true;
    }
    text.match_indices("User(").any(|(i, _)| {
        text[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.'))
    })
}

/// What `sheet_formulas` reads for one sheet.
pub(super) struct SheetIn<'a> {
    pub(super) name: &'a str,
    pub(super) cells: &'a SheetCells,
    pub(super) pass: &'a FormulaPass,
    pub(super) scan: &'a SheetScan,
    pub(super) dates: &'a HashMap<(u32, u32), u32>,
    pub(super) quoting: &'a Quoting,
}

/// The formulas of a sheet into the facet, the value cells into the logic index. Returns
/// the problems, whether a formula text has an area or a 3D reference, and whether the time
/// limit stopped it.
pub(super) fn sheet_formulas(
    out: &mut Out,
    formulas: &mut Formulas,
    sheet: &SheetIn,
    budget: &Budget,
) -> (Vec<FormulaProblem>, bool, bool) {
    let name = sheet.name;
    let q = quote_sheet(name);
    let formula_cells: HashSet<&(u32, u32)> = sheet.scan.formulas.iter().collect();
    let mut problems = Vec::new();
    let mut shared: Vec<(u32, u32)> = Vec::new();
    let mut dropped: (u64, Option<(u32, u32)>) = (0, None);
    let mut unread = 0u64;
    let mut any = false;
    let mut stopped = false;
    for (i, c) in sheet.cells.cells.iter().enumerate() {
        if i % 1_024 == 1_023 && budget.check_time().is_err() {
            out.loss(
                format!("{q}!{}", grid::a1(c.row, c.col)),
                "time limit reached: the formulas from this cell on and the later sheets not read",
                LossReason::Limit,
                Severity::Warning,
            );
            stopped = true;
            break;
        }
        let pos = (c.row, c.col);
        let code = sheet
            .dates
            .get(&pos)
            .and_then(|k| DATE_CODES.get(*k as usize))
            .copied();
        let cached = crate::logic::cell_value(&c.val, code);
        match sheet.pass.texts.get(&pos) {
            Some(FText::Text(text)) => {
                // Areas and 3D references are where calamine's text differs from Excel's.
                any |= text.contains([':', '!']);
                let index = out.doc.facets.formulas.len();
                let quoted = sheet.quoting.apply(text);
                let why = match &quoted {
                    _ if unresolved_call(text) => Some(
                        "calamine does not name an add-in, user-defined or newer Excel function in it"
                            .to_string(),
                    ),
                    Err(ambiguous) => Some(format!(
                        "calamine writes the sheet name {ambiguous:?} unquoted, and another sheet name ends the same way"
                    )),
                    Ok(_) => None,
                };
                if let Some(why) = why {
                    let (f, p) = formulas.cell_not_analyzed(name, c.row, c.col, text, &why, cached);
                    out.doc.facets.formulas.push(f);
                    problems.push(p);
                    continue;
                }
                let text = quoted.unwrap_or_else(|_| text.clone());
                let (f, p) = formulas.cell(
                    name,
                    c.row,
                    c.col,
                    &text,
                    FormulaKind::Cell,
                    None,
                    cached,
                    index,
                );
                out.doc.facets.formulas.push(f);
                problems.extend(p);
            }
            Some(FText::Shared) => shared.push(pos),
            Some(FText::Dropped) => {
                dropped.0 += 1;
                dropped.1.get_or_insert(pos);
            }
            Some(FText::Unreadable(e)) => {
                formulas.unindexed_formulas += 1;
                problems.push(FormulaProblem {
                    anchor: format!("{q}!{}", grid::a1(c.row, c.col)),
                    what: format!("formula not read: {e}"),
                    unreadable: false,
                });
            }
            // A formula cell whose text was not read: never a constant.
            None if formula_cells.contains(&pos) => unread += 1,
            None if c.val != Val::Empty => formulas.add_constant(name, c.row, c.col),
            None => {}
        }
    }
    formulas.unindexed_formulas += unread;
    formulas.unindexed_formulas += shared.len() as u64 + dropped.0;
    if !shared.is_empty() {
        let first: Vec<String> = shared
            .iter()
            .take(5)
            .map(|(r, c)| grid::a1(*r, *c))
            .collect();
        out.loss(
            q.as_str(),
            format!(
                "{} formula cells belong to a shared or array formula, whose text calamine does not read for XLSB (first {}); their values are shown",
                shared.len(),
                first.join(", ")
            ),
            LossReason::Unsupported,
            Severity::Warning,
        );
    }
    if let (n, Some((r, c))) = dropped {
        out.loss(
            format!("{q}!{}", grid::a1(r, c)),
            format!(
                "formula text limit reached ({} bytes in all): {n} formula cells from {} on not listed; their values are shown",
                formulas.meter.max(),
                grid::a1(r, c)
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    (problems, any, stopped)
}

/// The defined names calamine read. XLSB names carry no scope Rezzer reads, so a name used
/// on several sheets is listed once.
pub(super) fn defined_names(
    out: &mut Out,
    formulas: &mut Formulas,
    names: &[DefinedName],
    budget: &Budget,
) {
    let mut count: HashMap<String, u64> = HashMap::new();
    let mut first = Vec::new();
    for n in names {
        let c = count.entry(n.name.to_uppercase()).or_insert(0);
        *c += 1;
        if *c == 1 {
            first.push(n.clone());
        }
    }
    for n in &first {
        let c = count.get(&n.name.to_uppercase()).copied().unwrap_or(1);
        if c > 1 {
            out.loss(
                format!("name:{}", n.name),
                format!(
                    "the defined name {} appears {c} times; sheet scopes are not read for XLSB, so it is listed once",
                    n.name
                ),
                LossReason::Unsupported,
                Severity::Info,
            );
        }
    }
    crate::xlsx::defined_names(out, formulas, &first, budget);
}
