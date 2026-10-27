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

mod format;
mod parts;
mod sheet;

pub(crate) use format::*;
pub(crate) use parts::*;
use sheet::*;

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
        let (paths, elements_out) = laid.place(&q, &mut blocks, &mut out);
        if elements_out {
            halted = Some("element limit reached");
        }
        format_losses(&mut out, &q, &fmt);
        drop(fmt);
        facts.push(SheetFacts::new(&sheet.name, cells.cells, styles, paths));
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
