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
use rezzer_limits::{Budget, contain};
use rezzer_model::{
    Anchor, Annotation, Block, Cell, Embedded, FormulaKind, Inline, LossReason, ReadOutcome, Rule,
    SectionKind, Severity, StatusColor, Value,
};
use sheetparts::numfmt::{self, DateSystem, NumKind};
use sheetparts::ooxml::{
    AttrKind, CalcMode, CalcSettings, CondFormat, DocProps, ExternalLink, FormulaAttr, SheetParts,
    SheetState, Validation, WorkbookParts, print_title_rows, read_sheet, read_workbook,
};
use sheetparts::{DefinedName, Issue, IssueKind, PropValue};
use zipguard::Archive;

use crate::grid::{self, RawCell, SheetCells, Stop, Val, note};
use crate::logic::{FormulaProblem, Formulas, ReadState, SheetFacts};
use crate::package::{self, Package, Refusal, zip_reason};
use crate::{Out, layout};

/// Formula problems listed one by one per sheet; the rest are counted in one loss.
const LISTED_PROBLEMS: usize = 20;

/// Reads an XLSX or XLSM package.
pub(crate) fn read(bytes: &[u8], budget: &Budget, mut out: Out) -> ReadOutcome {
    let lim = budget.limits();
    let stop = || budget.check_time().is_err();
    // One entry may unpack to the unpacked limit, within the ratio limit. The preflight
    // measures every entry once and charges it to the budget, so the unpacked limit bounds
    // what the package really unpacks to. Later reads of a part (sheetparts, calamine) are
    // not charged again: they cost memory for a while, bounded by the entry size and the
    // memory limit, not unpacked size.
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
    if let Err(refusal) = Package::preflight(&mut zip, budget) {
        return refuse(out, refusal);
    }
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
    // The part calamine reads for each sheet.
    let calamine_parts = match package::check_structure(&mut zip, &wb) {
        Ok(parts) => parts,
        Err(refusal) => return refuse(out, refusal),
    };

    let xl = match wb.workbook_part.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/"),
        None => String::new(),
    };
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

    // calamine finds a sheet by its name, so its names must be sheetparts' names, in order
    // and each once; otherwise one section could mix the parts of two sheets.
    let cal_names = cal.sheet_names();
    let ours: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    if let Err(refusal) = package::check_names(&cal_names, &ours) {
        return refuse(out, refusal);
    }

    properties(&mut out, &wb.props, Some(&wb.calc), wb.date1904);
    let vba = if wb.has_vba {
        vba_modules(&mut out, &mut cal, &format!("{xl}vbaProject.bin"))
    } else {
        Vba::default()
    };

    let mut formulas = Formulas::new(&wb.names, lim.max_text_bytes);
    let system = if wb.date1904 {
        DateSystem::Date1904
    } else {
        DateSystem::Date1900
    };
    let mut halted: Option<&'static str> = None;
    // The cells of each sheet read, for the labels and values of the parameters.
    let mut facts: Vec<SheetFacts> = Vec::new();
    // Some formula cells were not read or not listed.
    let mut incomplete = false;
    for sheet in &wb.sheets {
        // The part calamine reads for this sheet: the first of its sheets with this name
        // (its own lookup), joined from that sheet's relationship. calamine keeps the path
        // private, so the join is check_structure's; check_names made the names one to one.
        let Some(cal_part) = cal_names
            .iter()
            .position(|n| n == &sheet.name)
            .and_then(|i| calamine_parts.get(i))
        else {
            return refuse(
                out,
                (
                    LossReason::Damaged,
                    format!("calamine has no sheet named {:?}", sheet.name),
                ),
            );
        };
        debug_assert_eq!(
            cal_part, &sheet.part,
            "calamine and sheetparts read one part"
        );
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
        let parts = read_sheet(&mut zip, sheet, &wb, &part_limits, &stop);
        issues_to_losses(&mut out, &parts.issues);
        // A sheet part cut at a limit may hide formula details the logic index never sees.
        incomplete |= parts
            .issues
            .iter()
            .any(|i| matches!(i.kind, IssueKind::Limit));

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
        incomplete |= cells.error.is_some();
        match cells.stop {
            Some(Stop::Time) => halted = Some("time limit reached"),
            Some(Stop::Elements) => halted = Some("element limit reached"),
            None => {}
        }
        let merged = contain(AssertUnwindSafe(|| {
            cal.merge_cells_by_sheet_name(&sheet.name)
        }));
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

        // The regions of the sheet: tables, headers and report blocks.
        let laid = sheet_layout(
            &q,
            &sheet.name,
            &wb,
            &parts,
            &cells,
            &merges,
            &styles,
            &fmt,
            formulas.layout_facts(&sheet.name),
            budget,
        );
        if laid.mapped.blocks.is_empty() {
            blocks.push(note("The sheet is empty."));
        }
        blocks.extend(laid.mapped.blocks);
        for l in laid.mapped.losses {
            out.loss(l.location, l.what, l.why, l.severity);
        }
        merge_loss(&mut out, &q, laid.merges_ignored);
        if laid.mapped.elements_out {
            halted = Some("element limit reached");
        }
        format_losses(&mut out, &q, &fmt);
        drop(fmt);
        facts.push(SheetFacts::new(
            &sheet.name,
            cells.cells,
            styles,
            laid.paths,
        ));
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
        rules(
            &mut out,
            &q,
            &parts.validations,
            &parts.cond_formats,
            budget,
        );
        annotations(&mut out, &q, &parts, budget);
        out.doc.body.push(section(&sheet.name, &q, blocks));
    }

    defined_names(&mut out, &mut formulas, &wb.names, budget);
    embedded(&mut out, wb.has_vba, &wb.external_links, &vba, &formulas);
    let formats = |s: u32| wb.styles.number_format(s);
    let state = ReadState {
        calc: Some(&wb.calc),
        formats: &formats,
        incomplete: incomplete || halted.is_some(),
    };
    crate::logic::workbook_logic(&mut out, formulas, facts, state, budget);
    out.finish()
}

/// The defined names into the formulas facet, their texts charged to the formula text meter.
pub(crate) fn defined_names(
    out: &mut Out,
    formulas: &mut Formulas,
    names: &[DefinedName],
    budget: &Budget,
) {
    let mut names_dropped = 0u64;
    for (i, n) in names.iter().enumerate() {
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
                    names.len() - i - 1
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        if n.refers_to.trim().is_empty() {
            let f = formulas.name_not_analyzed(
            n,
            "the file gives no formula text for this name that is read (for XLSB: an add-in or external name)",
        );
            out.doc.facets.formulas.push(f);
            continue;
        }
        // A name's text counts against the same limit as the formula texts.
        if !formulas.meter.take(n.refers_to.len()) {
            names_dropped += 1;
            formulas.name_not_kept(n);
            continue;
        }
        let (f, problem) = formulas.name(n);
        out.doc.facets.formulas.push(f);
        if let Some(p) = problem {
            problem_loss(out, p);
        }
    }
    if names_dropped > 0 {
        out.loss(
        "document",
        format!(
            "formula text limit reached ({} bytes in all): {names_dropped} defined names not listed",
            formulas.meter.max()
        ),
        LossReason::Limit,
        Severity::Warning,
    );
    }
}

/// A failed outcome for a package that is not read at all.
pub(crate) fn refuse(mut out: Out, (why, what): Refusal) -> ReadOutcome {
    out.loss(
        "document",
        format!("the whole file: the package is not read: {what}"),
        why,
        Severity::Error,
    );
    out.failed()
}

pub(crate) fn issues_to_losses(out: &mut Out, issues: &[Issue]) {
    for i in issues {
        let (why, severity) = match i.kind {
            IssueKind::Damaged => (LossReason::Damaged, Severity::Warning),
            IssueKind::Limit => (LossReason::Limit, Severity::Warning),
            IssueKind::Unsupported => (LossReason::Unsupported, Severity::Info),
        };
        out.loss(i.part.as_str(), i.what.as_str(), why, severity);
    }
}

pub(crate) fn status(text: &str, color: StatusColor) -> Block {
    Block::Paragraph {
        inlines: vec![Inline::Status {
            text: text.to_string(),
            color,
        }],
        role: Some("status".into()),
        anchor: None,
    }
}

pub(crate) fn section(name: &str, q: &str, blocks: Vec<Block>) -> Block {
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
        let code = self
            .styles
            .get(&(c.row, c.col))
            .and_then(|s| self.wb.styles.number_format(*s));
        crate::logic::cell_value(&c.val, code)
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

/// The layout of an XLSX sheet: its cells with their classes and looks, its merges, Excel
/// tables, pivots, autofilter, names, outline, panes and print titles.
#[allow(clippy::too_many_arguments)]
fn sheet_layout(
    q: &str,
    name: &str,
    wb: &WorkbookParts,
    parts: &SheetParts,
    cells: &SheetCells,
    merges: &[calamine::Dimensions],
    styles: &HashMap<(u32, u32), u32>,
    fmt: &Formatter,
    formulas: HashMap<(u32, u32), sheetlayout::FormulaIn>,
    budget: &Budget,
) -> layout::SheetLayout {
    let looks = wb.styles.has_looks();
    let style = |row: u32, col: u32| {
        if !looks {
            return sheetlayout::Style::default();
        }
        let index = styles.get(&(row, col)).copied().unwrap_or(0);
        wb.styles
            .look(index)
            .map(layout::style_of)
            .unwrap_or_default()
    };
    let shown = |c: &RawCell| {
        let (text, class, number) = match &c.val {
            Val::Num(v) => {
                let (text, kind) = fmt.number(c.row, c.col, *v);
                let class = if matches!(kind, NumKind::Date | NumKind::Time) {
                    sheetlayout::Class::Date
                } else {
                    sheetlayout::Class::Number
                };
                (text, class, Some(*v))
            }
            Val::Iso(_) => (fmt.shown(c), sheetlayout::Class::Date, None),
            Val::Bool(_) => (fmt.shown(c), sheetlayout::Class::Bool, None),
            Val::Error(_) => (fmt.shown(c), sheetlayout::Class::Error, None),
            Val::Text(_) | Val::Empty => (fmt.shown(c), sheetlayout::Class::Text, None),
        };
        layout::Shown {
            text,
            class,
            number,
        }
    };
    let centered: Vec<(u32, u32)> = if looks {
        let mut list: Vec<(u32, u32)> = parts
            .cell_styles
            .iter()
            .filter(|(_, s)| wb.styles.look(*s).is_some_and(|l| l.center_across))
            .map(|(p, _)| (p.row, p.col))
            .collect();
        list.sort_unstable();
        list
    } else {
        Vec::new()
    };
    let mut facts = Vec::new();
    for t in &parts.tables {
        if let Some(area) = grid::parse_area(&t.range) {
            facts.push(layout::table_fact(
                area,
                &t.name,
                t.header_rows,
                t.totals_rows,
                &t.columns,
            ));
        }
    }
    for p in &parts.pivots {
        if let Some(area) = grid::parse_area(&p.location) {
            // Decision 13: `colGrandTotals` draws the total row at the bottom, when the pivot
            // has row fields; `rowGrandTotals` the total column at the right, when it has
            // column fields.
            facts.push(sheetlayout::Fact {
                rect: layout::rect_of(area),
                kind: sheetlayout::FactKind::Pivot {
                    name: p.name.clone(),
                    first_header_row: p.first_header_row,
                    first_data_row: p.first_data_row,
                    first_data_col: p.first_data_col,
                    total_row: p.col_grand_totals && p.row_fields > 0,
                    total_col: p.row_grand_totals && p.col_fields > 0,
                },
            });
        }
    }
    if let Some(area) = parts.auto_filter.as_deref().and_then(grid::parse_area) {
        facts.push(sheetlayout::Fact {
            rect: layout::rect_of(area),
            kind: sheetlayout::FactKind::AutoFilter,
        });
    }
    let (names, caption_names) = layout::sheet_names(&wb.names, name);
    let signals = layout::Signals {
        facts,
        names,
        outline: layout::outline_runs(&parts.outline_rows),
        frozen: parts.frozen.map(|f| (f.rows, f.cols)),
        print_titles: print_title_rows(&wb.names, name).map(|s| (s.first, s.last)),
        styles_read: looks,
    };
    let merges: Vec<sheetlayout::Rect> = merges
        .iter()
        .map(|d| layout::rect_of((d.start, d.end)).normalized())
        .collect();
    layout::map_sheet(
        q,
        &cells.cells,
        &shown,
        &style,
        &centered,
        &merges,
        formulas,
        signals,
        &caption_names,
        budget,
    )
}

/// The loss for merged areas that overlap one before them.
pub(crate) fn merge_loss(out: &mut Out, q: &str, ignored: u64) {
    if ignored > 0 {
        out.loss(
            q,
            format!(
                "{ignored} merged areas overlap another one or are too large; shown as single cells"
            ),
            LossReason::Damaged,
            Severity::Info,
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
            // The same meter as every other formula text: checked before the copy is made,
            // charged with its real length after.
            let copy = if formulas.meter.fits(t.len()) {
                calamine::expand_shared_formula(t, *start, pos).ok()
            } else {
                None
            };
            match copy {
                Some(f) if formulas.meter.take(f.len()) => text = Some(f),
                Some(_) | None if formulas.meter.is_full() => {
                    dropped.0 += 1;
                    dropped.1.get_or_insert(pos);
                    continue;
                }
                _ => {}
            }
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
    formulas.unindexed_formulas += dropped.0 + lost_shared.len() as u64;
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
        layout: None,
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
pub(crate) fn rules(
    out: &mut Out,
    q: &str,
    validations: &[Validation],
    cond_formats: &[CondFormat],
    budget: &Budget,
) {
    let mut list: Vec<Rule> = Vec::new();
    for v in validations {
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
    for cf in cond_formats {
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

/// The inlines of a comment text, cut at the text limit (counted in `cut`).
pub(crate) fn comment_body(budget: &Budget, text: &str, cut: &mut u64) -> Vec<Inline> {
    if budget.charge_text(text.len()).is_err() {
        *cut += 1;
        let max = usize::try_from(budget.limits().max_text_bytes).unwrap_or(usize::MAX);
        crate::text_inlines(&grid::cut_text(text.to_string(), max))
    } else {
        crate::text_inlines(text)
    }
}

/// Notes and threaded comments at their cells; replies nested under their thread.
fn annotations(out: &mut Out, q: &str, parts: &SheetParts, budget: &Budget) {
    let mut cut = 0u64;
    let mut body = |text: &str| comment_body(budget, text, &mut cut);
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
    add_annotations(out, q, list, budget, cut);
    if orphans > 0 {
        out.loss(
            q,
            format!("{orphans} comment replies point at a thread that is not there; shown as threads of their own"),
            LossReason::Damaged,
            Severity::Info,
        );
    }
}

/// Adds the annotations of a sheet within the element budget; `cut` texts were cut at the
/// text limit.
pub(crate) fn add_annotations(
    out: &mut Out,
    q: &str,
    list: Vec<Annotation>,
    budget: &Budget,
    cut: u64,
) {
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
}

/// Title, authors, dates, keywords, custom properties and, when the file has them, the
/// calculation settings and the date system.
pub(crate) fn properties(
    out: &mut Out,
    doc: &DocProps,
    calc: Option<&CalcSettings>,
    date1904: bool,
) {
    let props = &mut out.doc.properties;
    let core = &doc.core;
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
        if let Some(v) = doc.app.get(key) {
            custom.insert(name.into(), text(v));
        }
    }
    if let Some(calc) = calc {
        let mode = match calc.mode {
            CalcMode::Auto => "auto",
            CalcMode::Manual => "manual",
            CalcMode::AutoNoTable => "auto_no_table",
        };
        custom.insert("calc_mode".into(), text(mode));
        custom.insert(
            "iterative_calculation".into(),
            Value::Bool {
                value: calc.iterate,
            },
        );
        if calc.iterate {
            if let Some(n) = calc.iterate_count {
                custom.insert("iteration_count".into(), Value::number(f64::from(n), None));
            }
            if let Some(d) = calc.iterate_delta {
                custom.insert("iteration_delta".into(), Value::number(d, None));
            }
        }
    }
    if date1904 {
        custom.insert("date_system".into(), text("1904"));
    }
    // Custom document properties keep their names; Rezzer's own keys win a clash.
    for (name, value) in &doc.custom {
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
pub(crate) struct Vba {
    modules: Vec<String>,
    /// Uppercase function name to its module.
    functions: HashMap<String, String>,
}

pub(crate) fn vba_modules<RS, R>(out: &mut Out, cal: &mut R, part: &str) -> Vba
where
    RS: std::io::Read + std::io::Seek,
    R: calamine::Reader<RS>,
    R::Error: std::fmt::Display,
{
    let mut vba = Vba::default();
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
                part,
                format!("VBA project not read: {e}"),
                LossReason::Damaged,
                Severity::Warning,
            );
            return vba;
        }
        Err(panic) => {
            out.loss(
                part,
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
pub(crate) fn declared_functions(code: &str) -> Vec<String> {
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
pub(crate) fn embedded(
    out: &mut Out,
    has_vba: bool,
    links: &[ExternalLink],
    vba: &Vba,
    formulas: &Formulas,
) {
    let list = &mut out.doc.facets.embedded;
    if has_vba && vba.modules.is_empty() {
        list.push(Embedded {
            text: None,
            kind: "macro".into(),
            name: "VBA project".into(),
            detail: Some("VBA project, never run".into()),
            target: None,
        });
    }
    for m in &vba.modules {
        list.push(Embedded {
            text: None,
            kind: "macro".into(),
            name: m.clone(),
            detail: Some("VBA module, never run".into()),
            target: None,
        });
    }
    for link in links {
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
            text: None,
            kind: kind.to_string(),
            name: name.to_string(),
            detail: Some(format!("[{}] in formulas, never opened", link.index)),
            target: Some(link.target.clone()),
        });
    }
    for (name, _first_use) in &formulas.user_functions {
        let (detail, target) = user_function_source(name, links, vba);
        list.push(Embedded {
            text: None,
            kind: "user-function".into(),
            name: name.clone(),
            detail: Some(detail),
            target,
        });
    }
}

/// Where a user function comes from: `VBA module` (target the module), the add-in or
/// workbook path of a `[n]!` prefix, or `unknown`.
fn user_function_source(name: &str, links: &[ExternalLink], vba: &Vba) -> (String, Option<String>) {
    if let Some(rest) = name.strip_prefix('[')
        && let Some((book, _)) = rest.split_once("]!")
    {
        if let Ok(index) = book.parse::<u32>() {
            return match links.iter().find(|l| l.index == index) {
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
