// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! XLSX and XLSM: preflight with zipguard, workbook parts from sheetparts, cells from
//! calamine, formulas through formulakit.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::panic::AssertUnwindSafe;

use calamine::{Reader as _, Xlsx};
use formulakit::quote_sheet;
use rezzer_limits::{Budget, LimitHit, contain};
use rezzer_model::{
    Anchor, Annotation, Block, Cell, Embedded, FormulaKind, Inline, LossReason, ReadOutcome, Rule,
    SectionKind, Severity, StatusColor, Value,
};
use sheetparts::numfmt::{self, DateSystem, NumKind};
use sheetparts::ooxml::{
    AttrKind, CalcMode, FormulaAttr, SheetParts, SheetState, WorkbookParts, read_sheet,
    read_workbook,
};
use sheetparts::{Issue, IssueKind, PropValue};
use zipguard::Archive;

use crate::Out;
use crate::grid::{self, RawCell, SheetCells, Stop, Val, note};
use crate::logic::{FormulaProblem, Formulas};
use crate::package::{self, Package, Refusal, zip_reason};

/// Formula problems listed one by one per sheet; the rest are counted in one loss.
const LISTED_PROBLEMS: usize = 20;

/// Reads an XLSX or XLSM package.
pub(crate) fn read(bytes: &[u8], budget: &Budget, mut out: Out) -> ReadOutcome {
    let lim = budget.limits();
    let stop = || budget.check_time().is_err();
    // One entry may unpack to the unpacked limit, within the ratio limit. Everything
    // zipguard unpacks (the preflight and sheetparts' reads) and every part calamine
    // unpacks again is charged to the budget, so the unpacked limit bounds the real total.
    let zip_limits = zipguard::Limits {
        max_entry_bytes: lim.max_unpacked_bytes,
        max_total_bytes: lim.max_unpacked_bytes,
        max_ratio: u64::from(lim.max_ratio),
        ..zipguard::Limits::default()
    };
    let mut zip = match Archive::open(bytes, zip_limits) {
        Ok(z) => z,
        Err(e) => {
            let why = zip_reason(&e);
            out.loss(
                "document",
                format!("the whole file: the package could not be opened: {e}"),
                why,
                Severity::Error,
            );
            return out.failed();
        }
    };
    // Every entry is measured before anything else reads it.
    let pkg = match Package::preflight(&mut zip, budget) {
        Ok(p) => p,
        Err(refusal) => return refuse(out, refusal),
    };
    let mut charged = zip.unpacked();
    let part_limits = sheetparts::Limits {
        max_depth: usize::try_from(lim.max_depth).unwrap_or(usize::MAX),
        max_items: usize::try_from(lim.max_elements).unwrap_or(usize::MAX),
    };

    let wb = read_workbook(&mut zip, &part_limits, &stop);
    issues_to_losses(&mut out, &wb.issues);
    if wb.workbook_part.is_empty() {
        out.loss(
            "document",
            "the whole file: the package has no workbook part",
            LossReason::Damaged,
            Severity::Error,
        );
        return out.failed();
    }
    if let Err(refusal) = package::check_structure(&mut zip, &wb) {
        return refuse(out, refusal);
    }
    if let Err(hit) = charge_reads(&zip, &mut charged, budget) {
        return refuse(out, (LossReason::Limit, hit.to_string()));
    }

    // calamine unpacks these again when it opens the workbook.
    let xl = match wb.workbook_part.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/"),
        None => String::new(),
    };
    let open_parts = [
        "_rels/.rels".to_string(),
        format!("{xl}workbook.xml"),
        format!("{xl}_rels/workbook.xml.rels"),
        format!("{xl}sharedStrings.xml"),
        format!("{xl}styles.xml"),
    ];
    for part in &open_parts {
        if let Err(hit) = pkg.charge(budget, part) {
            return refuse(out, (LossReason::Limit, format!("{part}: {hit}")));
        }
    }
    let opened = contain(AssertUnwindSafe(|| Xlsx::new(Cursor::new(bytes))));
    let mut cal = match opened {
        Ok(Ok(c)) => c,
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

    properties(&mut out, &wb);
    let vba = vba_modules(&mut out, &pkg, &mut cal, &wb, &xl, budget);

    let mut formulas = Formulas::new(&wb.names, lim.max_text_bytes);
    let system = if wb.date1904 {
        DateSystem::Date1904
    } else {
        DateSystem::Date1900
    };
    let mut halted: Option<&'static str> = None;
    for sheet in &wb.sheets {
        let q = quote_sheet(&sheet.name);
        let mut blocks = Vec::new();
        match sheet.state {
            SheetState::Visible => {}
            SheetState::Hidden => blocks.push(status("hidden", StatusColor::Grey)),
            SheetState::VeryHidden => blocks.push(status("very hidden", StatusColor::Yellow)),
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
        // calamine unpacks the sheet for its cells, sheetparts for its logic.
        let charged_sheet = pkg.charge(budget, &sheet.part).and_then(|()| {
            let parts = read_sheet(&mut zip, sheet, &wb, &part_limits, &stop);
            charge_reads(&zip, &mut charged, budget).map(|()| parts)
        });
        let parts = match charged_sheet {
            Ok(p) => p,
            Err(hit) => {
                out.loss(
                    q.as_str(),
                    format!("sheet not read: {hit}"),
                    LossReason::Limit,
                    Severity::Warning,
                );
                blocks.push(note("This sheet was not read."));
                out.doc.body.push(section(&sheet.name, &q, blocks));
                halted = Some("unpacked limit reached");
                continue;
            }
        };
        issues_to_losses(&mut out, &parts.issues);

        let read = contain(AssertUnwindSafe(|| {
            grid::read_cells(&mut cal, &sheet.name, budget, &mut formulas.meter)
        }));
        let cells = match read {
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
                SheetCells::default()
            }
        };
        cell_losses(&mut out, &q, &cells);
        match cells.stop {
            Some(Stop::Time) => halted = Some("time limit reached"),
            Some(Stop::Elements) => halted = Some("element limit reached"),
            None => {}
        }
        // calamine unpacks the sheet a second time for its merged cells.
        let merged = match pkg.charge(budget, &sheet.part) {
            Ok(()) => contain(AssertUnwindSafe(|| {
                cal.merge_cells_by_sheet_name(&sheet.name)
            })),
            Err(hit) => {
                out.loss(
                    q.as_str(),
                    format!("merged cells not read: {hit}"),
                    LossReason::Limit,
                    Severity::Warning,
                );
                Ok(Ok(Vec::new()))
            }
        };
        let merges = match merged {
            Ok(Ok(m)) => m,
            Ok(Err(e)) => {
                out.loss(
                    q.as_str(),
                    format!("merged cells not read: {e}"),
                    LossReason::Damaged,
                    Severity::Info,
                );
                Vec::new()
            }
            Err(panic) => {
                out.loss(
                    q.as_str(),
                    format!("merged cells not read: internal error: {panic}"),
                    LossReason::Internal,
                    Severity::Info,
                );
                Vec::new()
            }
        };

        let styles: HashMap<(u32, u32), u32> = parts
            .cell_styles
            .iter()
            .map(|(p, s)| ((p.row, p.col), *s))
            .collect();
        let fmt = Formatter {
            wb: &wb,
            styles: &styles,
            system,
            unapplied: RefCell::new(BTreeMap::new()),
            bad_style: RefCell::new(0),
        };

        add_tables(&mut out, &mut formulas, &sheet.name, &parts);
        let (problems, stopped) = sheet_formulas(
            &mut out,
            &mut formulas,
            &sheet.name,
            &cells,
            &parts,
            &fmt,
            budget,
        );
        if stopped {
            halted = Some("time limit reached");
        }
        problem_losses(&mut out, &q, problems);

        // The value table.
        let (table, notes) =
            grid::value_table(&q, &cells.cells, &merges, budget, &|c| fmt.shown(c));
        match table {
            Some(t) => blocks.push(t),
            None => blocks.push(note("The sheet is empty.")),
        }
        table_losses(&mut out, &q, &notes);
        if notes.element_stop.is_some() {
            halted = Some("element limit reached");
        }
        format_losses(&mut out, &q, &fmt);
        if !parts.hidden_rows.is_empty() {
            blocks.push(note(format!(
                "Hidden rows: {}.",
                grid::span_list(&parts.hidden_rows, false)
            )));
        }
        if !parts.hidden_cols.is_empty() {
            blocks.push(note(format!(
                "Hidden columns: {}.",
                grid::span_list(&parts.hidden_cols, true)
            )));
        }
        objects(&mut blocks, &q, &parts);
        rules(&mut out, &q, &parts, budget);
        annotations(&mut out, &q, &parts, budget);
        out.doc.body.push(section(&sheet.name, &q, blocks));
    }

    for (i, n) in wb.names.iter().enumerate() {
        let over = if i % 1_024 == 0 {
            budget.check_time().err()
        } else {
            None
        };
        if let Some(hit) = over.or_else(|| budget.add_elements(1).err()) {
            out.loss(
                format!("name:{}", n.name),
                format!(
                    "{hit}: this and {} more defined names not read",
                    wb.names.len() - i - 1
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        let (f, problem) = formulas.name(n);
        out.doc.facets.formulas.push(f);
        if let Some(p) = problem {
            problem_loss(&mut out, p);
        }
    }
    embedded(&mut out, &wb, &vba, &formulas);
    // The logic inputs stay with the formulas for the workbook logic pass.
    drop(formulas);
    out.finish()
}

/// Charges what zipguard unpacked since the last call to the budget.
fn charge_reads(zip: &Archive, charged: &mut u64, budget: &Budget) -> Result<(), LimitHit> {
    let now = zip.unpacked();
    let delta = now.saturating_sub(*charged);
    *charged = now;
    budget.add_unpacked(delta)
}

/// A failed outcome for a package that is not read at all.
fn refuse(mut out: Out, (why, what): Refusal) -> ReadOutcome {
    out.loss(
        "document",
        format!("the whole file: the package is not read: {what}"),
        why,
        Severity::Error,
    );
    out.failed()
}

fn issues_to_losses(out: &mut Out, issues: &[Issue]) {
    for i in issues {
        let (why, severity) = match i.kind {
            IssueKind::Damaged => (LossReason::Damaged, Severity::Warning),
            IssueKind::Limit => (LossReason::Limit, Severity::Warning),
            IssueKind::Unsupported => (LossReason::Unsupported, Severity::Info),
        };
        out.loss(i.part.as_str(), i.what.as_str(), why, severity);
    }
}

fn status(text: &str, color: StatusColor) -> Block {
    Block::Paragraph {
        inlines: vec![Inline::Status {
            text: text.to_string(),
            color,
        }],
        role: Some("status".into()),
        anchor: None,
    }
}

fn section(name: &str, q: &str, blocks: Vec<Block>) -> Block {
    Block::Section {
        kind: SectionKind::Sheet,
        title: Some(name.to_string()),
        anchor: Some(Anchor(q.to_string())),
        blocks,
    }
}

/// Per number format id without a known code: how many cells, and the first cell.
type Unapplied = BTreeMap<u32, (u64, (u32, u32))>;

/// Shows cell values as the file formats them.
struct Formatter<'a> {
    wb: &'a WorkbookParts,
    styles: &'a HashMap<(u32, u32), u32>,
    system: DateSystem,
    /// Format ids without a known code: cells and the first cell.
    unapplied: RefCell<Unapplied>,
    /// Cells whose style index is not in the style list.
    bad_style: RefCell<u64>,
}

impl Formatter<'_> {
    /// The number as shown, with its kind. A format the file names but that is not known
    /// (a locale-dependent built-in) is not applied: the raw value is shown.
    fn number(&self, row: u32, col: u32, v: f64) -> (String, NumKind) {
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

    fn shown(&self, c: &RawCell) -> String {
        match &c.val {
            Val::Empty => String::new(),
            Val::Num(v) => self.number(c.row, c.col, *v).0,
            Val::Text(s) | Val::Error(s) | Val::Iso(s) => s.clone(),
            Val::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        }
    }

    /// The cached value of a formula cell as a typed value.
    fn cached(&self, c: &RawCell) -> Option<Value> {
        match &c.val {
            Val::Empty => None,
            Val::Num(v) => {
                let styled = self
                    .styles
                    .get(&(c.row, c.col))
                    .and_then(|s| self.wb.styles.number_format(*s));
                match styled.map(numfmt::kind_of) {
                    Some(NumKind::Percent) => Some(Value::percent(*v)),
                    _ => Some(Value::number(*v, None)),
                }
            }
            Val::Text(s) | Val::Error(s) => Some(Value::Text { value: s.clone() }),
            Val::Iso(s) => Some(Value::DateTime { value: s.clone() }),
            Val::Bool(b) => Some(Value::Bool { value: *b }),
        }
    }
}

fn format_losses(out: &mut Out, q: &str, fmt: &Formatter) {
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

fn cell_losses(out: &mut Out, q: &str, cells: &SheetCells) {
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

fn table_losses(out: &mut Out, q: &str, notes: &grid::TableNotes) {
    if let Some((first, last)) = notes.rows_cut {
        let rows = if first == last {
            format!("row {first}")
        } else {
            format!("rows {first} to {last}")
        };
        out.loss(
            q,
            format!("{rows} not shown in the value table (at most {} cells); their formulas are still listed", grid::MAX_TABLE_CELLS),
            LossReason::Limit,
            Severity::Info,
        );
    }
    if let Some((first, last)) = notes.cols_cut {
        out.loss(
            q,
            format!(
                "columns {} to {} not shown in the value table (at most {} columns); their formulas are still listed",
                rezzer_model::column_letter(first as usize),
                rezzer_model::column_letter(last as usize),
                grid::MAX_TABLE_COLUMNS
            ),
            LossReason::Limit,
            Severity::Info,
        );
    }
    if let Some(row) = notes.element_stop {
        out.loss(
            q,
            format!(
                "element limit reached, value table rows from row {} on not shown",
                row + 1
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if notes.merges_ignored > 0 {
        out.loss(
            q,
            format!(
                "{} merged areas overlap another one or are too large; shown as single cells",
                notes.merges_ignored
            ),
            LossReason::Damaged,
            Severity::Info,
        );
    }
}

fn problem_loss(out: &mut Out, p: FormulaProblem) {
    let why = if p.unreadable {
        LossReason::Unsupported
    } else {
        LossReason::Damaged
    };
    out.loss(p.anchor, p.what, why, Severity::Warning);
}

fn problem_losses(out: &mut Out, q: &str, problems: Vec<FormulaProblem>) {
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

/// Registers the Excel tables of a sheet with the logic index.
fn add_tables(out: &mut Out, formulas: &mut Formulas, sheet: &str, parts: &SheetParts) {
    for t in &parts.tables {
        let location = format!("table:{}", t.name);
        let Some(area) = grid::parse_area(&t.range) else {
            out.loss(
                location,
                format!(
                    "table {} has an unreadable range {:?}; its references read no cells",
                    t.name, t.range
                ),
                LossReason::Damaged,
                Severity::Warning,
            );
            continue;
        };
        if let Err(why) = formulas.add_table(
            &t.name,
            sheet,
            area,
            t.header_rows,
            t.totals_rows,
            &t.columns,
        ) {
            out.loss(
                location,
                format!("{why}; its references read no cells"),
                LossReason::Damaged,
                Severity::Warning,
            );
        }
    }
}

/// The formulas of a sheet into the formulas facet, and the value cells into the logic
/// index as constants. Returns the formulas that could not be read.
fn sheet_formulas(
    out: &mut Out,
    formulas: &mut Formulas,
    sheet: &str,
    cells: &SheetCells,
    parts: &SheetParts,
    fmt: &Formatter,
    budget: &Budget,
) -> (Vec<FormulaProblem>, bool) {
    let q = quote_sheet(sheet);
    let attrs: HashMap<(u32, u32), &FormulaAttr> = parts
        .formula_attrs
        .iter()
        .map(|a| ((a.at.row, a.at.col), a))
        .collect();
    // Shared formula masters from the XML, for cells calamine met before their master.
    let mut masters: HashMap<u32, (&str, (u32, u32))> = HashMap::new();
    for a in &parts.formula_attrs {
        if let (AttrKind::Shared { index }, Some(text), Some(_)) = (&a.kind, &a.text, &a.range)
            && !text.is_empty()
        {
            masters.insert(*index, (text.as_str(), (a.at.row, a.at.col)));
        }
    }
    // Areas filled by array, spill and data table formulas: their cells are results, not
    // constants. Swept down the rows.
    let mut areas: Vec<((u32, u32), (u32, u32))> = parts
        .formula_attrs
        .iter()
        .filter(|a| !matches!(a.kind, AttrKind::Shared { .. }))
        .filter_map(|a| a.range.as_deref().and_then(grid::parse_area))
        .collect();
    areas.sort_by_key(|a| a.0.0);
    let mut next_area = 0usize;
    let mut active: Vec<((u32, u32), (u32, u32))> = Vec::new();

    let mut problems = Vec::new();
    let mut dropped: (u64, Option<(u32, u32)>) = (0, None);
    let mut lost_shared: Vec<(u32, u32)> = Vec::new();
    let mut stopped = false;
    for (i, c) in cells.cells.iter().enumerate() {
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
        if c.text_dropped {
            dropped.0 += 1;
            dropped.1.get_or_insert(pos);
            continue;
        }
        let attr = attrs.get(&pos).copied();
        let mut text = c.formula.clone();
        if text.is_none()
            && c.shared_unknown
            && let Some(AttrKind::Shared { index }) = attr.map(|a| &a.kind)
            && let Some((t, start)) = masters.get(index)
        {
            text = calamine::expand_shared_formula(t, *start, pos).ok();
        }
        let index = out.doc.facets.formulas.len();
        if let Some(text) = text {
            let (kind, range) = match attr.map(|a| &a.kind) {
                Some(AttrKind::Array) => (FormulaKind::Array, attr.and_then(|a| a.range.clone())),
                Some(AttrKind::DynamicArray) => {
                    (FormulaKind::Spill, attr.and_then(|a| a.range.clone()))
                }
                Some(AttrKind::Shared { .. }) => (FormulaKind::Shared, None),
                _ if c.shared => (FormulaKind::Shared, None),
                _ => (FormulaKind::Cell, None),
            };
            let (f, problem) = formulas.cell(
                sheet,
                c.row,
                c.col,
                &text,
                kind,
                range,
                fmt.cached(c),
                index,
            );
            out.doc.facets.formulas.push(f);
            problems.extend(problem);
            continue;
        }
        if let Some(FormulaAttr {
            kind:
                AttrKind::DataTable {
                    r1,
                    r2,
                    two_d,
                    row_input,
                },
            range,
            ..
        }) = attr
        {
            let f = formulas.data_table(
                sheet,
                c.row,
                c.col,
                range.as_deref(),
                r1.as_deref(),
                r2.as_deref(),
                *two_d,
                *row_input,
                fmt.cached(c),
                index,
            );
            out.doc.facets.formulas.push(f);
            continue;
        }
        if c.shared_unknown {
            lost_shared.push(pos);
            continue;
        }
        // A value cell: a constant unless an array, spill or data table fills it.
        while let Some(a) = areas.get(next_area).filter(|a| a.0.0 <= c.row) {
            active.push(*a);
            next_area += 1;
        }
        active.retain(|a| a.1.0 >= c.row);
        let filled = active.iter().any(|a| (a.0.1..=a.1.1).contains(&c.col));
        if !filled && c.val != Val::Empty {
            formulas.add_constant(sheet, c.row, c.col);
        }
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
    if !lost_shared.is_empty() {
        let first: Vec<String> = lost_shared
            .iter()
            .take(5)
            .map(|(r, c)| grid::a1(*r, *c))
            .collect();
        out.loss(
            q.as_str(),
            format!(
                "{} cells of a shared formula whose master cell is missing; their formula text is not known (first {})",
                lost_shared.len(),
                first.join(", ")
            ),
            LossReason::Damaged,
            Severity::Warning,
        );
    }
    (problems, stopped)
}

/// Excel tables, pivot tables and charts as small tables in the sheet section.
fn objects(blocks: &mut Vec<Block>, q: &str, parts: &SheetParts) {
    let plain = |s: &str| Cell::new(vec![Inline::plain(s)]);
    let head = |cols: &[&str]| cols.iter().map(|c| plain(c)).collect::<Vec<_>>();
    let small = |caption: &str, role: &str, rows: Vec<Vec<Cell>>| Block::Table {
        caption: Some(caption.to_string()),
        header_rows: 1,
        role: Some(role.to_string()),
        rows,
        alignments: Vec::new(),
        anchor: None,
    };
    if !parts.tables.is_empty() {
        let mut rows = vec![head(&["Table", "Range", "Columns"])];
        for t in &parts.tables {
            let mut name = plain(&t.name);
            name.anchor = Some(Anchor(format!("table:{}", t.name)));
            let mut range = plain(&t.range);
            range.anchor = Some(Anchor(format!("{q}!{}", t.range.replace('$', ""))));
            rows.push(vec![name, range, plain(&t.columns.join(", "))]);
        }
        blocks.push(small("Excel tables", "excel-table", rows));
    }
    if !parts.pivots.is_empty() {
        let mut rows = vec![head(&["Pivot table", "Location", "Source"])];
        for p in &parts.pivots {
            let mut location = plain(&p.location);
            location.anchor = Some(Anchor(format!("{q}!{}", p.location.replace('$', ""))));
            rows.push(vec![
                plain(&p.name),
                location,
                plain(p.source.as_deref().unwrap_or("not known")),
            ]);
        }
        blocks.push(small("Pivot tables", "pivot", rows));
    }
    if !parts.charts.is_empty() {
        let mut rows = vec![head(&["Chart", "Title", "Series"])];
        for ch in &parts.charts {
            let series: Vec<String> = ch
                .series
                .iter()
                .map(|(name, values)| match name {
                    Some(n) => format!("{n}: {values}"),
                    None => values.clone(),
                })
                .collect();
            rows.push(vec![
                plain(&ch.kind),
                plain(ch.title.as_deref().unwrap_or("")),
                plain(&series.join("; ")),
            ]);
        }
        blocks.push(small("Charts", "chart", rows));
    }
}

/// Validations and conditional formats, one rule per range they apply to.
fn rules(out: &mut Out, q: &str, parts: &SheetParts, budget: &Budget) {
    let mut list: Vec<Rule> = Vec::new();
    for v in &parts.validations {
        let mut detail = v.kind.clone();
        if let Some(list) = &v.list {
            detail = format!("list: {}", list.join(", "));
        } else if v.kind != "list" && v.kind != "custom" && v.kind != "none" {
            detail.push(' ');
            detail.push_str(v.operator.as_deref().unwrap_or("between"));
        }
        let formulas: Vec<String> = [&v.formula1, &v.formula2]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let message = match (&v.prompt, &v.error) {
            (Some(p), Some(e)) => Some(format!("prompt: {p}; error: {e}")),
            (Some(p), None) => Some(format!("prompt: {p}")),
            (None, Some(e)) => Some(format!("error: {e}")),
            (None, None) => None,
        };
        for range in v.range.split_whitespace() {
            list.push(Rule {
                kind: "validation".into(),
                anchor: Anchor(format!("{q}!{}", range.replace('$', ""))),
                detail: detail.clone(),
                formulas: formulas.clone(),
                message: message.clone(),
            });
        }
    }
    for cf in &parts.cond_formats {
        let mut detail = cf.kind.clone();
        if let Some(op) = &cf.operator {
            detail.push(' ');
            detail.push_str(op);
        }
        detail.push_str(&format!(" (priority {})", cf.priority));
        for range in cf.range.split_whitespace() {
            list.push(Rule {
                kind: "conditional-format".into(),
                anchor: Anchor(format!("{q}!{}", range.replace('$', ""))),
                detail: detail.clone(),
                formulas: cf.formulas.clone(),
                message: None,
            });
        }
    }
    let total = list.len();
    for (i, rule) in list.into_iter().enumerate() {
        if let Err(hit) = budget.add_elements(1) {
            out.loss(
                q,
                format!("{hit}: {} of {total} rules not read", total - i),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        out.doc.facets.rules.push(rule);
    }
}

/// Notes and threaded comments at their cells; replies nested under their thread.
fn annotations(out: &mut Out, q: &str, parts: &SheetParts, budget: &Budget) {
    let max = usize::try_from(budget.limits().max_text_bytes).unwrap_or(usize::MAX);
    let mut cut = 0u64;
    let mut body = |text: &str| {
        if budget.charge_text(text.len()).is_err() {
            cut += 1;
            crate::text_inlines(&grid::cut_text(text.to_string(), max))
        } else {
            crate::text_inlines(text)
        }
    };
    let mut list = Vec::new();
    for n in &parts.notes {
        list.push(Annotation {
            kind: "note".into(),
            author: n.author.clone(),
            date: None,
            anchor: Some(Anchor(format!("{q}!{}", n.cell.replace('$', "")))),
            quoted: None,
            body: body(&n.text),
            replies: Vec::new(),
            resolved: None,
        });
    }
    let mut threads: Vec<Annotation> = Vec::new();
    let mut by_id: HashMap<&str, usize> = HashMap::new();
    let mut orphans = 0u64;
    for t in &parts.threads {
        let a = Annotation {
            kind: "comment".into(),
            author: t.person.clone(),
            date: t.date.clone(),
            anchor: Some(Anchor(format!("{q}!{}", t.cell.replace('$', "")))),
            quoted: None,
            body: body(&t.text),
            replies: Vec::new(),
            resolved: None,
        };
        let root = t.parent.as_deref().and_then(|p| by_id.get(p).copied());
        match root {
            Some(i) => threads[i].replies.push(a),
            None => {
                if t.parent.is_some() {
                    orphans += 1;
                }
                by_id.insert(t.id.as_str(), threads.len());
                threads.push(Annotation {
                    resolved: Some(t.done),
                    ..a
                });
            }
        }
    }
    list.extend(threads);
    let total = list.len();
    for (i, a) in list.into_iter().enumerate() {
        let size = 1 + a.replies.len() as u64;
        if let Err(hit) = budget.add_elements(size) {
            out.loss(
                q,
                format!("{hit}: {} of {total} comments not read", total - i),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        out.doc.facets.annotations.push(a);
    }
    if cut > 0 {
        out.loss(
            q,
            format!("{cut} comments longer than the text limit were cut"),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if orphans > 0 {
        out.loss(
            q,
            format!("{orphans} comment replies point at a thread that is not there; shown as threads of their own"),
            LossReason::Damaged,
            Severity::Info,
        );
    }
}

/// Title, authors, dates, keywords, custom properties and calculation settings.
fn properties(out: &mut Out, wb: &WorkbookParts) {
    let props = &mut out.doc.properties;
    let core = &wb.props.core;
    props.title = core.get("title").cloned();
    props.description = core.get("description").cloned();
    if let Some(a) = core.get("creator") {
        props.authors.push(a.clone());
    }
    props.created = core
        .get("created")
        .map(|v| Value::DateTime { value: v.clone() });
    props.modified = core
        .get("modified")
        .map(|v| Value::DateTime { value: v.clone() });
    props.language = core.get("language").cloned();
    if let Some(k) = core.get("keywords") {
        props.keywords = k
            .split([',', ';'])
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .collect();
    }
    let custom = &mut props.custom;
    let text = |s: &str| Value::Text {
        value: s.to_string(),
    };
    for (key, name) in [
        ("subject", "subject"),
        ("lastModifiedBy", "last_modified_by"),
        ("category", "category"),
    ] {
        if let Some(v) = core.get(key) {
            custom.insert(name.into(), text(v));
        }
    }
    for (key, name) in [("Application", "application"), ("Company", "company")] {
        if let Some(v) = wb.props.app.get(key) {
            custom.insert(name.into(), text(v));
        }
    }
    let mode = match wb.calc.mode {
        CalcMode::Auto => "auto",
        CalcMode::Manual => "manual",
        CalcMode::AutoNoTable => "auto_no_table",
    };
    custom.insert("calc_mode".into(), text(mode));
    custom.insert(
        "iterative_calculation".into(),
        Value::Bool {
            value: wb.calc.iterate,
        },
    );
    if wb.calc.iterate {
        if let Some(n) = wb.calc.iterate_count {
            custom.insert("iteration_count".into(), Value::number(f64::from(n), None));
        }
        if let Some(d) = wb.calc.iterate_delta {
            custom.insert("iteration_delta".into(), Value::number(d, None));
        }
    }
    if wb.date1904 {
        custom.insert("date_system".into(), text("1904"));
    }
    // Custom document properties keep their names; Rezzer's own keys win a clash.
    for (name, value) in &wb.props.custom {
        let v = match value {
            PropValue::Text(s) => text(s),
            PropValue::Integer(i) => Value::number(*i as f64, None),
            PropValue::Number(n) => Value::number(*n, None),
            PropValue::Bool(b) => Value::Bool { value: *b },
            PropValue::DateTime(s) => Value::DateTime { value: s.clone() },
            PropValue::Other { value, .. } => text(value),
        };
        custom.entry(name.clone()).or_insert(v);
    }
}

/// VBA module names and the functions each one declares. The code is never run.
#[derive(Default)]
struct Vba {
    modules: Vec<String>,
    /// Uppercase function name to its module.
    functions: HashMap<String, String>,
}

fn vba_modules(
    out: &mut Out,
    pkg: &Package,
    cal: &mut Xlsx<Cursor<&[u8]>>,
    wb: &WorkbookParts,
    xl: &str,
    budget: &Budget,
) -> Vba {
    let mut vba = Vba::default();
    if !wb.has_vba {
        return vba;
    }
    let part = format!("{xl}vbaProject.bin");
    if let Err(hit) = pkg.charge(budget, &part) {
        out.loss(
            part.as_str(),
            format!("VBA project not read: {hit}"),
            LossReason::Limit,
            Severity::Warning,
        );
        return vba;
    }
    let project = match contain(AssertUnwindSafe(|| cal.vba_project())) {
        Ok(Ok(Some(p))) => p,
        Ok(Ok(None)) => {
            out.loss(
                "document",
                "the package has a VBA project calamine does not find; module names not listed",
                LossReason::Unsupported,
                Severity::Info,
            );
            return vba;
        }
        Ok(Err(e)) => {
            out.loss(
                part.as_str(),
                format!("VBA project not read: {e}"),
                LossReason::Damaged,
                Severity::Warning,
            );
            return vba;
        }
        Err(panic) => {
            out.loss(
                part.as_str(),
                format!("VBA project not read: internal error: {panic}"),
                LossReason::Internal,
                Severity::Warning,
            );
            return vba;
        }
    };
    for name in project.get_module_names() {
        vba.modules.push(name.to_string());
        // The source is read as text to find the functions it declares; never run.
        if let Ok(code) = project.get_module(name) {
            for f in declared_functions(&code) {
                vba.functions
                    .entry(f.to_uppercase())
                    .or_insert_with(|| name.to_string());
            }
        }
    }
    vba
}

/// Names of the `Function` procedures a VBA module declares.
fn declared_functions(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in code.lines() {
        let mut words = line.split_whitespace().peekable();
        while let Some(w) = words.peek() {
            let lower = w.to_ascii_lowercase();
            if matches!(lower.as_str(), "public" | "private" | "friend" | "static") {
                words.next();
            } else {
                break;
            }
        }
        if words
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("function"))
            && let Some(rest) = words.next()
        {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
    out
}

/// VBA modules, external workbooks and add-ins, and the user functions formulas call.
fn embedded(out: &mut Out, wb: &WorkbookParts, vba: &Vba, formulas: &Formulas) {
    let list = &mut out.doc.facets.embedded;
    if wb.has_vba && vba.modules.is_empty() {
        list.push(Embedded {
            kind: "macro".into(),
            name: "VBA project".into(),
            detail: Some("VBA project, never run".into()),
            target: None,
        });
    }
    for m in &vba.modules {
        list.push(Embedded {
            kind: "macro".into(),
            name: m.clone(),
            detail: Some("VBA module, never run".into()),
            target: None,
        });
    }
    for link in &wb.external_links {
        let lower = link.target.to_ascii_lowercase();
        let addin_file = [".xlam", ".xla", ".xll"].iter().any(|e| lower.ends_with(e));
        let kind = match link.kind.as_str() {
            "add-in" => "add-in",
            "workbook" if addin_file => "add-in",
            "workbook" => "external-workbook",
            other => other,
        };
        let name = link
            .target
            .rsplit(['/', '\\'])
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or(&link.target);
        list.push(Embedded {
            kind: kind.to_string(),
            name: name.to_string(),
            detail: Some(format!("[{}] in formulas, never opened", link.index)),
            target: Some(link.target.clone()),
        });
    }
    for (name, _first_use) in &formulas.user_functions {
        let (detail, target) = user_function_source(name, wb, vba);
        list.push(Embedded {
            kind: "user-function".into(),
            name: name.clone(),
            detail: Some(detail),
            target,
        });
    }
}

/// Where a user function comes from: `VBA module` (target the module), the add-in or
/// workbook path of a `[n]!` prefix, or `unknown`.
fn user_function_source(name: &str, wb: &WorkbookParts, vba: &Vba) -> (String, Option<String>) {
    if let Some(rest) = name.strip_prefix('[')
        && let Some((book, _)) = rest.split_once("]!")
    {
        if let Ok(index) = book.parse::<u32>() {
            return match wb.external_links.iter().find(|l| l.index == index) {
                Some(l) => (l.target.clone(), None),
                None => ("unknown".into(), None),
            };
        }
        return (book.to_string(), None);
    }
    match vba.functions.get(&name.to_uppercase()) {
        Some(module) => ("VBA module".into(), Some(module.clone())),
        None => ("unknown".into(), None),
    }
}
