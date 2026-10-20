// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! XLSB: the binary workbook. calamine reads values, cached results, formula texts, defined
//! names and VBA module names; the XML parts XLSB still has give the document properties.
//! Before calamine sees a part, every entry is measured, the package structure is checked
//! and the BIFF12 records are walked (see [`crate::biff12`]).

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::panic::AssertUnwindSafe;

use calamine::{DataRef, Reader as _, SheetType, SheetVisible, Xlsb, XlsbError};
use formulakit::quote_sheet;
use rezzer_limits::{Budget, contain};
use rezzer_model::{FormulaKind, LossReason, ReadOutcome, Severity, StatusColor};
use sheetparts::DefinedName;
use sheetparts::numfmt::{self, DateSystem};
use sheetparts::ooxml::read_doc_props;
use zipguard::Archive;

use crate::Out;
use crate::biff12::{self, Book, SheetScan, Widths};
use crate::grid::{self, RawCell, SheetCells, Stop, Val, note};
use crate::logic::{FormulaProblem, Formulas, ReadState, SheetFacts};
use crate::package::{self, Package, Refusal, zip_reason};
use crate::xlsx::{
    Vba, cell_losses, embedded, issues_to_losses, problem_losses, properties, refuse, section,
    status, table_losses, vba_modules,
};

/// The parts calamine reads by these fixed names.
const WORKBOOK: &str = "xl/workbook.bin";
const WORKBOOK_RELS: &str = "xl/_rels/workbook.bin.rels";
const OPEN_PARTS: [&str; 4] = [
    "xl/sharedStrings.bin",
    "xl/styles.bin",
    WORKBOOK_RELS,
    WORKBOOK,
];
const VBA_PART: &str = "xl/vbaProject.bin";

/// What is not read from an XLSB workbook, said once per file.
const NOT_READ: &str = "not read for XLSB: validation, conditional formats, notes and threads, names with scope, calc settings, data tables, number formats beyond dates; also merged cells, hidden rows and columns, Excel tables, pivot tables, charts and external links";

/// The formats Rezzer gives the dates calamine finds, by format key.
const DATE_CODES: [&str; 4] = ["yyyy-mm-dd", "yyyy-mm-dd hh:mm:ss", "hh:mm:ss", "[h]:mm:ss"];

/// Cells read between two checks of the time limit.
const CHECK_EVERY: u64 = 1_024;

/// A sheet as the checks found it.
struct SheetEntry {
    name: String,
    /// The part calamine reads: `xl/` and the relationship target.
    part: String,
}

/// Checks the package before calamine opens it: canonical relationship parts, the workbook
/// at `xl/workbook.bin`, every sheet relationship internal, plain and present, sheet names
/// unique and without control characters.
fn check_package(zip: &mut Archive) -> Result<(Book, Vec<SheetEntry>), Refusal> {
    let damaged = |what: String| (LossReason::Damaged, what);
    let parsed = package::all_rels(zip)?;
    let doc = package::office_document(&parsed)?;
    let part = package::join("", &doc.target);
    if part != WORKBOOK {
        return Err(damaged(format!(
            "the workbook part is {part}, not {WORKBOOK}, which calamine reads"
        )));
    }
    if !zip.contains(WORKBOOK) {
        return Err(damaged(format!("{WORKBOOK} is not in the package")));
    }
    let bytes = zip
        .read(WORKBOOK)
        .map_err(|e| (zip_reason(&e), format!("{WORKBOOK}: {e}")))?;
    let book = biff12::read_book(&bytes).map_err(|e| damaged(format!("{WORKBOOK}: {e}")))?;
    let by_id: HashMap<&str, &package::Rel> = parsed
        .get(WORKBOOK_RELS)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|r| (r.id.as_str(), r))
        .collect();
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for s in &book.sheets {
        if package::has_control(&s.name) {
            return Err(damaged(format!(
                "the sheet name {:?} holds a control character",
                s.name
            )));
        }
        if !seen.insert(s.name.to_lowercase()) {
            return Err(damaged(format!(
                "the sheet name {:?} is used twice",
                s.name
            )));
        }
        let Some(id) = &s.rel_id else {
            return Err(damaged(format!(
                "sheet {:?} has no relationship id",
                s.name
            )));
        };
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(damaged(format!(
                "sheet {:?} has the relationship id {id:?}, which is not plain letters and digits",
                s.name
            )));
        }
        let Some(rel) = by_id.get(id.as_str()) else {
            return Err(damaged(format!(
                "sheet {:?} names the relationship {id:?}, which is not there",
                s.name
            )));
        };
        if rel.external || rel.target.starts_with('/') || !package::clean(&rel.target) {
            return Err(damaged(format!(
                "the target {:?} of sheet {:?} is external or not a plain path",
                rel.target, s.name
            )));
        }
        let part = format!("xl/{}", rel.target);
        if !matches!(
            part.split('/').nth(1),
            Some("worksheets" | "chartsheets" | "dialogsheets")
        ) {
            return Err(damaged(format!(
                "the part {part} of sheet {:?} is not in a sheet folder",
                s.name
            )));
        }
        if !zip.contains(&part) {
            return Err(damaged(format!(
                "the part {part} of sheet {:?} is not in the package",
                s.name
            )));
        }
        entries.push(SheetEntry {
            name: s.name.clone(),
            part,
        });
    }
    Ok((book, entries))
}

/// Sheet names calamine writes without quotes before `!`, quoted the way Excel does.
struct Quoting {
    /// The names that need quotes with their quoted form, longest first.
    names: Vec<(String, String)>,
    /// Every sheet name.
    all: Vec<String>,
}

impl Quoting {
    fn new(sheets: &[&str]) -> Quoting {
        let mut names: Vec<(String, String)> = sheets
            .iter()
            .map(|s| (s.to_string(), quote_sheet(s)))
            .filter(|(s, q)| s != q)
            .collect();
        names.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));
        Quoting {
            names,
            all: sheets.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// The text with every `name!` of a sheet that needs quotes quoted, outside string
    /// literals. Err with the name when the text could also be read as an expression that
    /// ends in another sheet's name (`Q1-2024!A1` with sheets `Q1-2024` and `2024`).
    fn apply(&self, text: &str) -> Result<String, String> {
        if self.names.is_empty() || !text.contains('!') {
            return Ok(text.to_string());
        }
        let mut out = String::with_capacity(text.len() + 8);
        let mut in_string = false;
        let mut prev: Option<char> = None;
        let mut i = 0;
        while i < text.len() {
            let rest = &text[i..];
            let boundary = prev.is_none_or(|c| {
                !(c.is_alphanumeric() || matches!(c, '_' | '.' | '\'' | '"' | '!' | '$' | '\\'))
            });
            if !in_string
                && boundary
                && let Some((name, quoted)) = self
                    .names
                    .iter()
                    .find(|(n, _)| rest.starts_with(n.as_str()) && rest[n.len()..].starts_with('!'))
            {
                if self
                    .all
                    .iter()
                    .any(|m| m != name && name.ends_with(m.as_str()))
                {
                    return Err(name.clone());
                }
                out.push_str(quoted);
                out.push('!');
                i += name.len() + 1;
                prev = Some('!');
                continue;
            }
            let c = rest.chars().next().unwrap_or_default();
            if c == '"' {
                in_string = !in_string;
            }
            out.push(c);
            prev = Some(c);
            i += c.len_utf8();
        }
        Ok(out)
    }
}

/// Byte widths of the texts a formula token can name (UTF-8 takes at most three bytes per
/// UTF-16 unit).
fn widths(book: &Book) -> Widths {
    let width = |s: &str| 3 * s.encode_utf16().count() as u64;
    Widths {
        // calamine also writes `#InvalidWorkSheet` for a sheet it cannot name.
        sheet: book
            .sheets
            .iter()
            .map(|s| width(&s.name))
            .max()
            .unwrap_or(0)
            .max(17),
        name: book.names.iter().map(|(n, _)| width(n)).max().unwrap_or(0),
    }
}

/// Reads an XLSB package.
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
    let pkg = match Package::preflight(&mut zip, budget) {
        Ok(p) => p,
        Err(refusal) => return refuse(out, refusal),
    };
    let mut charged = zip.unpacked();
    let part_limits = sheetparts::Limits {
        max_depth: usize::try_from(lim.max_depth).unwrap_or(usize::MAX),
        max_items: usize::try_from(lim.max_elements).unwrap_or(usize::MAX),
    };
    let (book, entries) = match check_package(&mut zip) {
        Ok(c) => c,
        Err(refusal) => return refuse(out, refusal),
    };
    if let Err(hit) = charge_reads(&zip, &mut charged, budget) {
        return refuse(out, (LossReason::Limit, hit.to_string()));
    }
    // calamine parses the formulas of the defined names when it opens the workbook.
    let widths = widths(&book);
    let mut names_text = 0u64;
    let mut names_work = 0u64;
    for (name, rgce) in &book.names {
        let left = biff12::MAX_OPEN_WORK - names_work;
        match biff12::check_tokens_within(rgce, widths, left) {
            Ok((n, work)) => {
                names_text = names_text.saturating_add(n);
                names_work += work;
            }
            Err(why) => {
                return refuse(
                    out,
                    (
                        LossReason::Limit,
                        format!("defined name {name:?}: {}", why.what()),
                    ),
                );
            }
        }
    }
    if names_text > lim.max_text_bytes {
        return refuse(
            out,
            (
                LossReason::Limit,
                format!(
                    "the defined names could produce {names_text} bytes of formula text, more than the text limit of {}",
                    lim.max_text_bytes
                ),
            ),
        );
    }
    for part in OPEN_PARTS {
        if let Err(hit) = pkg.charge(budget, part) {
            return refuse(out, (LossReason::Limit, format!("{part}: {hit}")));
        }
    }
    let mut cal = match contain(AssertUnwindSafe(|| Xlsb::new(Cursor::new(bytes)))) {
        Ok(Ok(c)) => c,
        Ok(Err(XlsbError::Password)) => {
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
    let ours: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    if let Err(refusal) = package::check_names(&cal_names, &ours) {
        return refuse(out, refusal);
    }
    let kinds: Vec<(SheetType, SheetVisible)> = cal
        .sheets_metadata()
        .iter()
        .map(|s| (s.typ, s.visible))
        .collect();

    let (props, issues) = read_doc_props(&mut zip, &part_limits, &stop);
    issues_to_losses(&mut out, &issues);
    if let Err(hit) = charge_reads(&zip, &mut charged, budget) {
        return refuse(out, (LossReason::Limit, hit.to_string()));
    }
    let system = if cal.has_1904_epoch() {
        DateSystem::Date1904
    } else {
        DateSystem::Date1900
    };
    properties(&mut out, &props, None, system == DateSystem::Date1904);
    out.loss(
        "document",
        NOT_READ,
        LossReason::Unsupported,
        Severity::Info,
    );

    let has_vba = pkg.contains(VBA_PART);
    let vba = if has_vba {
        vba_modules(&mut out, &pkg, &mut cal, VBA_PART, budget)
    } else {
        Vba::default()
    };

    let names: Vec<DefinedName> = cal
        .defined_names()
        .iter()
        .map(|(name, text)| DefinedName {
            name: name.clone(),
            scope: None,
            refers_to: text.clone(),
            hidden: false,
            comment: None,
        })
        .collect();
    // calamine writes sheet names unquoted in references; Rezzer quotes them as Excel does.
    let quoting = Quoting::new(&ours);
    let mut ambiguous_names = Vec::new();
    let names: Vec<DefinedName> = names
        .into_iter()
        .filter_map(|mut n| match quoting.apply(&n.refers_to) {
            Ok(text) => {
                n.refers_to = text;
                Some(n)
            }
            Err(sheet) => {
                ambiguous_names.push((n, sheet));
                None
            }
        })
        .collect();
    let mut formulas = Formulas::new(&names, lim.max_text_bytes);
    let mut halted: Option<&'static str> = None;
    let mut facts: Vec<SheetFacts> = Vec::new();
    let mut incomplete = false;
    let mut caveat = false;
    for (i, entry) in entries.iter().enumerate() {
        let q = quote_sheet(&entry.name);
        let mut blocks = Vec::new();
        let (kind, visible) = kinds
            .get(i)
            .copied()
            .unwrap_or((SheetType::WorkSheet, SheetVisible::Visible));
        match visible {
            SheetVisible::Visible => {}
            SheetVisible::Hidden => blocks.push(status("hidden", StatusColor::Grey)),
            SheetVisible::VeryHidden => blocks.push(status("very hidden", StatusColor::Yellow)),
        }
        if kind != SheetType::WorkSheet {
            out.loss(
                q.as_str(),
                "chart and dialog sheets are not read for XLSB",
                LossReason::Unsupported,
                Severity::Info,
            );
            blocks.push(note("This is a chart or dialog sheet; it is not read."));
            out.doc.body.push(section(&entry.name, &q, blocks));
            continue;
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
            out.doc.body.push(section(&entry.name, &q, blocks));
            continue;
        }
        // The walk over the records reads the part through zipguard.
        let scan = zip
            .read(&entry.part)
            .map_err(|e| (zip_reason(&e), e.to_string()))
            .and_then(|b| {
                charge_reads(&zip, &mut charged, budget)
                    .map_err(|hit| (LossReason::Limit, hit.to_string()))?;
                biff12::scan_sheet(&b, widths).map_err(|e| (LossReason::Damaged, e))
            });
        let scan = match scan {
            Ok(s) => s,
            Err((why, what)) => {
                out.loss(
                    q.as_str(),
                    format!("sheet not read: {}: {what}", entry.part),
                    why,
                    Severity::Warning,
                );
                blocks.push(note("This sheet was not read."));
                out.doc.body.push(section(&entry.name, &q, blocks));
                incomplete = true;
                if why == LossReason::Limit {
                    halted = Some("unpacked limit reached");
                }
                continue;
            }
        };

        // Values: calamine unpacks the sheet once for them.
        if let Err(hit) = pkg.charge(budget, &entry.part) {
            out.loss(
                q.as_str(),
                format!("sheet not read: {hit}"),
                LossReason::Limit,
                Severity::Warning,
            );
            blocks.push(note("This sheet was not read."));
            out.doc.body.push(section(&entry.name, &q, blocks));
            halted = Some("unpacked limit reached");
            continue;
        }
        let mut dates: HashMap<(u32, u32), u32> = HashMap::new();
        let read = contain(AssertUnwindSafe(|| {
            read_values(&mut cal, &entry.name, budget, &mut dates)
        }));
        let mut cells = match read {
            Ok(c) => c,
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
                SheetCells::default()
            }
        };
        cell_losses(&mut out, &q, &cells);
        incomplete |= cells.error.is_some();
        match cells.stop {
            Some(Stop::Time) => halted = Some("time limit reached"),
            Some(Stop::Elements) => halted = Some("element limit reached"),
            None => {}
        }

        // Formulas: a second pass, unless the walk found tokens calamine must not parse.
        let mut pass = FormulaPass::default();
        if let Some((why, (row, col))) = scan.unsafe_formula {
            out.loss(
                q.as_str(),
                format!(
                    "formulas of this sheet not read: {} (cell {}); the values are shown",
                    why.what(),
                    grid::a1(row, col)
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            incomplete = true;
        } else if !scan.formulas.is_empty() && halted.is_none() {
            match pkg.charge(budget, &entry.part) {
                Ok(()) => {
                    let read = contain(AssertUnwindSafe(|| {
                        read_formulas(&mut cal, &entry.name, &scan, &mut formulas, budget)
                    }));
                    match read {
                        Ok(p) => pass = p,
                        Err(panic) => {
                            out.loss(
                                q.as_str(),
                                format!(
                                    "formulas not read: the formula reader stopped with an internal error: {panic}"
                                ),
                                LossReason::Internal,
                                Severity::Warning,
                            );
                            incomplete = true;
                        }
                    }
                }
                Err(hit) => {
                    out.loss(
                        q.as_str(),
                        format!("formulas not read: {hit}"),
                        LossReason::Limit,
                        Severity::Warning,
                    );
                    halted = Some("unpacked limit reached");
                }
            }
        }
        if let Some(e) = &pass.error {
            out.loss(
                q.as_str(),
                format!("the formulas could not be read to the end: {e}"),
                LossReason::Damaged,
                Severity::Warning,
            );
            incomplete = true;
        }
        if pass.stopped {
            halted = Some("time limit reached");
            incomplete = true;
        }
        let missing = merge_formula_cells(&mut cells, &pass, &scan, budget);
        if missing > 0 {
            out.loss(
                q.as_str(),
                format!("element limit reached: {missing} formula cells not shown"),
                LossReason::Limit,
                Severity::Warning,
            );
            halted = Some("element limit reached");
        }

        let shown = |c: &RawCell| shown(c, &dates, system);
        let sheet_in = SheetIn {
            name: &entry.name,
            cells: &cells,
            pass: &pass,
            scan: &scan,
            dates: &dates,
            quoting: &quoting,
        };
        let (problems, has_areas, stopped) =
            sheet_formulas(&mut out, &mut formulas, &sheet_in, budget);
        caveat |= has_areas;
        if stopped {
            halted = Some("time limit reached");
        }
        problem_losses(&mut out, &q, problems);
        let (table, notes) = grid::value_table(&q, &cells.cells, &[], budget, &shown);
        match table {
            Some(t) => blocks.push(t),
            None => blocks.push(note("The sheet is empty.")),
        }
        table_losses(&mut out, &q, &notes);
        if notes.element_stop.is_some() {
            halted = Some("element limit reached");
        }
        facts.push(SheetFacts::new(&entry.name, cells.cells, dates));
        out.doc.body.push(section(&entry.name, &q, blocks));
    }
    if caveat {
        out.loss(
            "document",
            "XLSB formula texts are rebuilt from binary tokens: area and 3D references always show $ whether or not the file marks them absolute",
            LossReason::Uncertain,
            Severity::Info,
        );
    }

    defined_names(&mut out, &mut formulas, &names, budget);
    for (n, sheet) in &ambiguous_names {
        let why = format!(
            "not analyzed: calamine writes the sheet name {sheet:?} unquoted, and another sheet name ends the same way"
        );
        let f = formulas.name_not_analyzed(n, &why);
        out.loss(
            f.anchor.0.clone(),
            format!("defined name {why}"),
            LossReason::Unsupported,
            Severity::Warning,
        );
        out.doc.facets.formulas.push(f);
    }
    embedded(&mut out, has_vba, &[], &vba, &formulas);
    let formats = |k: u32| DATE_CODES.get(k as usize).copied();
    let state = ReadState {
        calc: None,
        formats: &formats,
        incomplete: incomplete || halted.is_some(),
    };
    crate::logic::workbook_logic(&mut out, formulas, facts, state, budget);
    out.finish()
}

/// Charges what zipguard unpacked since the last call to the budget.
fn charge_reads(
    zip: &Archive,
    charged: &mut u64,
    budget: &Budget,
) -> Result<(), rezzer_limits::LimitHit> {
    let now = zip.unpacked();
    let delta = now.saturating_sub(*charged);
    *charged = now;
    budget.add_unpacked(delta)
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
fn read_values(
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
struct FormulaPass {
    texts: HashMap<(u32, u32), FText>,
    /// calamine stopped reading the part with this error.
    error: Option<String>,
    stopped: bool,
}

/// Reads the formula texts of a sheet. Each text is charged to the formula text meter.
fn read_formulas(
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
fn merge_formula_cells(
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

/// The text of a cell as shown: dates in ISO form, numbers in general form.
fn shown(c: &RawCell, dates: &HashMap<(u32, u32), u32>, system: DateSystem) -> String {
    match &c.val {
        Val::Empty => String::new(),
        Val::Num(v) => {
            let code = dates
                .get(&(c.row, c.col))
                .and_then(|k| DATE_CODES.get(*k as usize))
                .copied()
                .unwrap_or("General");
            numfmt::format_with(code, *v, system).text
        }
        Val::Text(s) | Val::Error(s) | Val::Iso(s) => s.clone(),
        Val::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
    }
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
struct SheetIn<'a> {
    name: &'a str,
    cells: &'a SheetCells,
    pass: &'a FormulaPass,
    scan: &'a SheetScan,
    dates: &'a HashMap<(u32, u32), u32>,
    quoting: &'a Quoting,
}

/// The formulas of a sheet into the facet, the value cells into the logic index. Returns
/// the problems, whether a formula text has an area or a 3D reference, and whether the time
/// limit stopped it.
fn sheet_formulas(
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
fn defined_names(out: &mut Out, formulas: &mut Formulas, names: &[DefinedName], budget: &Budget) {
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
