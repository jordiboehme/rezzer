// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! ODS: calamine reads values and formula texts; sheetparts reads hidden sheets, rows and
//! columns, annotations, validations, number formats, named ranges with their scope and the
//! document properties. Formulas are OpenFormula, shown in A1 form with the stored text kept.
//!
//! calamine builds every sheet as a dense range when it opens the file, so `content.xml` is
//! checked and measured first (see [`crate::odscan`]): a file whose ranges would pass the
//! memory limit is not read.

use std::collections::HashMap;
use std::io::Cursor;
use std::panic::AssertUnwindSafe;

use calamine::{Data, Ods, OdsError, Reader as _};
use formulakit::quote_sheet;
use quick_xml::Reader;
use quick_xml::events::Event;
use rezzer_limits::{Budget, contain};
use rezzer_model::{
    Anchor, Annotation, Embedded, FormulaKind, LossReason, ReadOutcome, Severity, StatusColor,
};
use sheetparts::IssueKind;
use sheetparts::numfmt::{self, DateSystem};
use sheetparts::odf::{self, OdsSheet};
use zipguard::Archive;

use crate::Out;
use crate::grid::{self, RawCell, SheetCells, Stop, Val, note};
use crate::logic::{Formulas, ReadState, SheetFacts};
use crate::odscan::{self, NotCanonical};
use crate::package::{self, Package, zip_reason};
use crate::xlsx::{
    Vba, add_annotations, cell_losses, comment_body, defined_names, embedded, issues_to_losses,
    problem_losses, properties, refuse, rules, section, status, table_losses,
};

const MIMETYPE: &str = "mimetype";
const MANIFEST: &str = "META-INF/manifest.xml";
const CONTENT: &str = "content.xml";
const SPREADSHEET: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// What is not read from an ODS workbook, said once per file.
const NOT_READ: &str = "not read for ODS: conditional formats, merged cells, database ranges, pivot tables (data pilot), charts and calculation settings";

/// Cells read between two checks of the time limit.
const CHECK_EVERY: u64 = 1_024;

/// Whether the manifest lists encryption data for an entry.
fn encrypted(manifest: &[u8]) -> bool {
    let mut reader = Reader::from_reader(manifest);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e))
                if e.local_name().as_ref() == "encryption-data" =>
            {
                return true;
            }
            Ok(Event::Eof) | Err(_) => return false,
            _ => {}
        }
    }
}

/// Reads an ODS package.
pub(crate) fn read(bytes: &[u8], budget: &Budget, mut out: Out) -> ReadOutcome {
    let lim = budget.limits();
    let stop = || budget.check_time().is_err();
    let zip_limits = zipguard::Limits {
        max_entry_bytes: lim.max_unpacked_bytes,
        max_total_bytes: lim.max_unpacked_bytes,
        max_ratio: u64::from(lim.max_ratio),
        ..zipguard::Limits::default()
    };
    let mut zip = match Archive::open(bytes, zip_limits) {
        Ok(z) => z,
        Err(e) => {
            out.loss(
                "document",
                format!("the whole file: the package could not be opened: {e}"),
                zip_reason(&e),
                Severity::Error,
            );
            return out.failed();
        }
    };
    // The preflight measures every entry once and charges it to the budget; later reads of
    // a part are not charged again.
    if let Err(refusal) = Package::preflight(&mut zip, budget) {
        return refuse(out, refusal);
    }
    let damaged = |what: String| (LossReason::Damaged, what);

    // The parts calamine opens by these exact names; case variants were refused above.
    for part in [MIMETYPE, MANIFEST, CONTENT] {
        if !zip.contains(part) {
            return refuse(out, damaged(format!("{part} is not in the package")));
        }
    }
    let mime = match zip.read(MIMETYPE) {
        Ok(m) => m,
        Err(e) => return refuse(out, (zip_reason(&e), format!("{MIMETYPE}: {e}"))),
    };
    if mime != SPREADSHEET.as_bytes() {
        return refuse(
            out,
            damaged(format!(
                "the mimetype is {:?}, not {SPREADSHEET}",
                String::from_utf8_lossy(&mime)
            )),
        );
    }
    let manifest = match zip.read(MANIFEST) {
        Ok(m) => m,
        Err(e) => return refuse(out, (zip_reason(&e), format!("{MANIFEST}: {e}"))),
    };
    if encrypted(&manifest) {
        out.loss(
            "document",
            "the whole file: the workbook is protected with a password (the manifest lists encryption data); it is not decrypted",
            LossReason::Encrypted,
            Severity::Error,
        );
        return out.failed();
    }
    let content = match zip.read(CONTENT) {
        Ok(c) => c,
        Err(e) => return refuse(out, (zip_reason(&e), format!("{CONTENT}: {e}"))),
    };
    let max_depth = usize::try_from(lim.max_depth).unwrap_or(usize::MAX);
    match odscan::canonical(&content, max_depth, &stop) {
        Ok(()) => {}
        Err(NotCanonical::Damaged(why)) => return refuse(out, damaged(why)),
        Err(NotCanonical::Limit(why)) => return refuse(out, (LossReason::Limit, why)),
    }
    let cell = (std::mem::size_of::<Data>() + std::mem::size_of::<String>()) as u64;
    let held = match odscan::calamine_bytes(&content, cell, &stop) {
        Ok(n) => n,
        Err(why) => return refuse(out, (LossReason::Limit, why)),
    };
    if held > lim.max_memory_bytes {
        return refuse(
            out,
            (
                LossReason::Limit,
                format!(
                    "the cells would take about {held} bytes of memory in calamine's ranges, more than the memory limit of {} bytes",
                    lim.max_memory_bytes
                ),
            ),
        );
    }
    drop(content);

    let part_limits = sheetparts::Limits {
        max_depth,
        max_items: usize::try_from(lim.max_elements).unwrap_or(usize::MAX),
    };
    let parts = odf::read(&mut zip, &part_limits, &stop);
    issues_to_losses(&mut out, &parts.issues);

    let mut cal = match contain(AssertUnwindSafe(|| Ods::new(Cursor::new(bytes)))) {
        Ok(Ok(c)) => c,
        Ok(Err(OdsError::Password)) => {
            out.loss(
                "document",
                "the whole file: the workbook is protected with a password; it is not decrypted",
                LossReason::Encrypted,
                Severity::Error,
            );
            return out.failed();
        }
        Ok(Err(e)) => {
            out.loss(
                "document",
                format!("the whole file: the workbook could not be opened: {e}"),
                LossReason::Damaged,
                Severity::Error,
            );
            return out.failed();
        }
        Err(panic) => {
            out.loss(
                "document",
                format!(
                    "the whole file: opening the workbook stopped with an internal error: {panic}"
                ),
                LossReason::Internal,
                Severity::Error,
            );
            return out.failed();
        }
    };
    let cal_names = cal.sheet_names();
    let ours: Vec<&str> = parts.sheets.iter().map(|s| s.name.as_str()).collect();
    if let Err(refusal) = package::check_names(&cal_names, &ours) {
        // When the ODF part reader stopped inside content.xml (a limit, broken XML), it
        // found fewer sheets than calamine. That stop is the reason, not a second reading.
        let stopped = parts
            .issues
            .iter()
            .find(|i| i.part == CONTENT && matches!(i.kind, IssueKind::Limit | IssueKind::Damaged));
        return match stopped {
            Some(i) => {
                let why = match i.kind {
                    IssueKind::Limit => LossReason::Limit,
                    _ => LossReason::Damaged,
                };
                refuse(
                    out,
                    (
                        why,
                        format!("{CONTENT}: {}, so not every sheet could be read", i.what),
                    ),
                )
            }
            None => refuse(out, refusal),
        };
    }

    properties(&mut out, &parts.props, None, false);
    out.loss(
        "document",
        NOT_READ,
        LossReason::Unsupported,
        Severity::Info,
    );

    // Number format codes, by the key SheetFacts keeps per cell.
    let mut codes: Vec<String> = Vec::new();
    let mut code_keys: HashMap<String, u32> = HashMap::new();
    let mut formulas = Formulas::new(&parts.names, lim.max_text_bytes).open_formula();
    let mut halted: Option<&'static str> = None;
    let mut facts: Vec<SheetFacts> = Vec::new();
    let mut incomplete = false;
    for sheet in &parts.sheets {
        let q = quote_sheet(&sheet.name);
        let mut blocks = Vec::new();
        if sheet.hidden {
            blocks.push(status("hidden", StatusColor::Grey));
        }
        if halted.is_none() && stop() {
            halted = Some("time limit reached");
        }
        if let Some(why) = halted {
            out.loss(
                q.as_str(),
                format!("sheet not read: {why}"),
                LossReason::Limit,
                Severity::Warning,
            );
            blocks.push(note("This sheet was not read."));
            out.doc.body.push(section(&sheet.name, &q, blocks));
            continue;
        }
        let read = contain(AssertUnwindSafe(|| {
            read_cells(&mut cal, &sheet.name, budget, &mut formulas)
        }));
        let OdsCells {
            cells,
            texts,
            formulas_known,
            missing,
        } = match read {
            Ok(r) => r,
            Err(panic) => {
                out.loss(
                    q.as_str(),
                    format!(
                        "cells not read: the cell reader stopped with an internal error: {panic}"
                    ),
                    LossReason::Internal,
                    Severity::Warning,
                );
                incomplete = true;
                OdsCells::default()
            }
        };
        if missing > 0 {
            out.loss(
                q.as_str(),
                format!("element limit reached: {missing} formula cells not shown"),
                LossReason::Limit,
                Severity::Warning,
            );
            halted = Some("element limit reached");
        }
        cell_losses(&mut out, &q, &cells);
        incomplete |= cells.error.is_some();
        match cells.stop {
            Some(Stop::Time) => halted = Some("time limit reached"),
            Some(Stop::Elements) => halted = Some("element limit reached"),
            None => {}
        }

        let mut styles: HashMap<(u32, u32), u32> = HashMap::new();
        for (pos, code) in &sheet.cell_formats {
            let next = codes.len() as u32;
            let key = *code_keys.entry(code.clone()).or_insert_with(|| {
                codes.push(code.clone());
                next
            });
            styles.insert((pos.row, pos.col), key);
        }
        let code_of = |c: &RawCell| {
            styles
                .get(&(c.row, c.col))
                .and_then(|k| codes.get(*k as usize))
                .map(String::as_str)
        };

        let mut problems = Vec::new();
        let mut dropped: (u64, Option<(u32, u32)>) = (0, None);
        for (i, c) in cells.cells.iter().enumerate() {
            if i % 1_024 == 1_023 && budget.check_time().is_err() {
                out.loss(
                    format!("{q}!{}", grid::a1(c.row, c.col)),
                    "time limit reached: the formulas from this cell on and the later sheets not read",
                    LossReason::Limit,
                    Severity::Warning,
                );
                halted = Some("time limit reached");
                break;
            }
            let pos = (c.row, c.col);
            match texts.get(&pos) {
                Some(Some(text)) => {
                    let index = out.doc.facets.formulas.len();
                    let cached = crate::logic::cell_value(&c.val, code_of(c));
                    let (f, p) = formulas.cell(
                        &sheet.name,
                        c.row,
                        c.col,
                        text,
                        FormulaKind::Cell,
                        None,
                        cached,
                        index,
                    );
                    out.doc.facets.formulas.push(f);
                    problems.extend(p);
                }
                Some(None) => {
                    dropped.0 += 1;
                    dropped.1.get_or_insert(pos);
                }
                // Without the formula range no cell is known to be a constant.
                None if c.val != Val::Empty && formulas_known => {
                    formulas.add_constant(&sheet.name, c.row, c.col)
                }
                None => {}
            }
        }
        formulas.unindexed_formulas += dropped.0;
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
        problem_losses(&mut out, &q, problems);

        let shown = |c: &RawCell| match &c.val {
            Val::Empty => String::new(),
            Val::Num(v) => {
                numfmt::format_with(code_of(c).unwrap_or("General"), *v, DateSystem::Date1900).text
            }
            Val::Text(s) | Val::Error(s) | Val::Iso(s) => s.clone(),
            Val::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        };
        let (table, notes) = grid::value_table(&q, &cells.cells, &[], budget, &shown);
        match table {
            Some(t) => blocks.push(t),
            None => blocks.push(note("The sheet is empty.")),
        }
        table_losses(&mut out, &q, &notes);
        if notes.element_stop.is_some() {
            halted = Some("element limit reached");
        }
        if !sheet.hidden_rows.is_empty() {
            blocks.push(note(format!(
                "Hidden rows: {}.",
                grid::span_list(&sheet.hidden_rows, false)
            )));
        }
        if !sheet.hidden_cols.is_empty() {
            blocks.push(note(format!(
                "Hidden columns: {}.",
                grid::span_list(&sheet.hidden_cols, true)
            )));
        }
        rules(&mut out, &q, &sheet.validations, &[], budget);
        annotations(&mut out, &q, sheet, budget);
        facts.push(SheetFacts::new(&sheet.name, cells.cells, styles));
        out.doc.body.push(section(&sheet.name, &q, blocks));
    }

    defined_names(&mut out, &mut formulas, &parts.names, budget);
    let vba = Vba::default();
    embedded(&mut out, false, &[], &vba, &formulas);
    if zip.names().iter().any(|n| n.starts_with("Basic/")) {
        out.doc.facets.embedded.push(Embedded {
            kind: "macro".into(),
            name: "Basic".into(),
            detail: Some("LibreOffice Basic macros, never run".into()),
            target: None,
        });
    }
    let formats = |k: u32| codes.get(k as usize).map(String::as_str);
    let state = ReadState {
        calc: None,
        formats: &formats,
        incomplete: incomplete || halted.is_some(),
    };
    crate::logic::workbook_logic(&mut out, formulas, facts, state, budget);
    out.finish()
}

/// The formula text of each formula cell (None when the formula text limit was reached).
type FormulaTexts = HashMap<(u32, u32), Option<String>>;

/// The cells of a sheet from calamine's ranges.
#[derive(Default)]
struct OdsCells {
    cells: SheetCells,
    texts: FormulaTexts,
    /// The formula range was read, so a cell without a formula text is a value.
    formulas_known: bool,
    /// Formula cells without a stored value left out at the element limit.
    missing: u64,
}

fn read_cells(
    cal: &mut Ods<Cursor<&[u8]>>,
    sheet: &str,
    budget: &Budget,
    formulas: &mut Formulas,
) -> OdsCells {
    let mut read = OdsCells::default();
    let out = &mut read.cells;
    let texts = &mut read.texts;
    let values = match cal.worksheet_range(sheet) {
        Ok(r) => r,
        Err(e) => {
            out.error = Some(e.to_string());
            return read;
        }
    };
    let max_text = usize::try_from(budget.limits().max_text_bytes).unwrap_or(usize::MAX);
    let (r0, c0) = values.start().unwrap_or((0, 0));
    let mut seen = 0u64;
    for (r, c, v) in values.used_cells() {
        seen += 1;
        if seen.is_multiple_of(CHECK_EVERY) && budget.check_time().is_err() {
            out.stop = Some(Stop::Time);
            break;
        }
        let (row, col) = (r0 + r as u32, c0 + c as u32);
        let val = match v {
            Data::Empty => continue,
            Data::Int(i) => Val::Num(*i as f64),
            Data::Float(f) => Val::Num(*f),
            Data::Bool(b) => Val::Bool(*b),
            Data::DateTime(d) => Val::Num(d.as_f64()),
            Data::DateTimeIso(s) | Data::DurationIso(s) => Val::Iso(s.clone()),
            Data::Error(e) => Val::Error(e.to_string()),
            Data::String(s) if budget.charge_text(s.len()).is_err() => {
                out.cut.0 += 1;
                if out.cut.1.len() < 5 {
                    out.cut.1.push((row, col));
                }
                Val::Text(grid::cut_text(s.clone(), max_text))
            }
            Data::String(s) => Val::Text(s.clone()),
        };
        if budget.add_elements(1).is_err() {
            out.stop = Some(Stop::Elements);
            break;
        }
        out.cells.push(raw(row, col, val));
    }
    drop(values);
    if out.stop.is_some() {
        return read;
    }
    let found = match cal.worksheet_formula(sheet) {
        Ok(r) => r,
        Err(e) => {
            out.error = Some(e.to_string());
            return read;
        }
    };
    let (r0, c0) = found.start().unwrap_or((0, 0));
    for (r, c, text) in found.used_cells() {
        seen += 1;
        if seen.is_multiple_of(CHECK_EVERY) && budget.check_time().is_err() {
            out.stop = Some(Stop::Time);
            break;
        }
        let pos = (r0 + r as u32, c0 + c as u32);
        let kept = formulas.meter.take(text.len()).then(|| text.clone());
        texts.insert(pos, kept);
    }
    read.formulas_known = out.stop.is_none();
    // Formula cells without a stored value.
    let have: std::collections::HashSet<(u32, u32)> =
        out.cells.iter().map(|c| (c.row, c.col)).collect();
    for pos in texts.keys() {
        if have.contains(pos) {
            continue;
        }
        if budget.add_elements(1).is_ok() {
            out.cells.push(raw(pos.0, pos.1, Val::Empty));
        } else {
            read.missing += 1;
        }
    }
    out.cells.sort_by_key(|c| (c.row, c.col));
    for c in &mut out.cells {
        if let Some(t) = texts.get(&(c.row, c.col)) {
            c.formula = Some(String::new());
            c.text_dropped = t.is_none();
        }
    }
    read
}

fn raw(row: u32, col: u32, val: Val) -> RawCell {
    RawCell {
        row,
        col,
        val,
        formula: None,
        shared: false,
        shared_unknown: false,
        text_dropped: false,
    }
}

/// The annotations of a sheet: comments with author and date at their cells.
fn annotations(out: &mut Out, q: &str, sheet: &OdsSheet, budget: &Budget) {
    let mut cut = 0u64;
    let list: Vec<Annotation> = sheet
        .annotations
        .iter()
        .map(|a| Annotation {
            kind: "comment".into(),
            author: a.author.clone(),
            date: a.date.clone(),
            anchor: Some(Anchor(format!("{q}!{}", a.cell))),
            quoted: None,
            body: comment_body(budget, &a.text, &mut cut),
            replies: Vec::new(),
            resolved: None,
        })
        .collect();
    add_annotations(out, q, list, budget, cut);
}
