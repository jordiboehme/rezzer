// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Formulas of cells and defined names, read through formulakit, plus the inputs of the
//! workbook logic index (formulas, constants, names and tables), and the workbook logic
//! pass: roles, pattern groups, parameters with units and the dependency graph.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;

use formulakit::logic::{FormulaId, GroupGraph, ParamCell, Role as KitRole, TableArea, Workbook};
use formulakit::{Analysis, CellPos, Dialect, Ref, anchor_of, quote_sheet};
use rezzer_limits::{Budget, contain};
use rezzer_model::{
    Anchor, Edge, Formula, FormulaKind, Graph, LossReason, Node, Parameter, Role, STALE_RESULTS,
    Severity, Value,
};
use sheetparts::DefinedName;
use sheetparts::numfmt::{self, NumKind};
use sheetparts::ooxml::{CalcMode, CalcSettings};

use crate::Out;
use crate::grid::{RawCell, Val};
use crate::layout::SheetPaths;
use crate::units;

mod calls;
mod labels;
mod params;

pub(crate) use calls::*;
pub(crate) use labels::*;
pub(crate) use params::*;

/// Excel's own maximum length of a formula, in characters. Longer texts are refused.
pub(crate) const MAX_FORMULA_CHARS: usize = 8_192;

/// Distinct formula texts whose LaTeX is remembered. The cache is cleared when full.
const LATEX_CACHE: usize = 65_536;

/// What the logic pass of the workbook needs: every formula, constant cell, defined name
/// and table, fed to formulakit's index in workbook order, plus which model formula each
/// index entry is. [`workbook_logic`] runs `finish` on it.
pub(crate) struct LogicInputs {
    pub workbook: Workbook,
    /// Position in `Facets.formulas` and the id formulakit gave the formula.
    pub formula_ids: Vec<(usize, FormulaId)>,
}

/// Formula text kept over the whole workbook, against `max_text_bytes`. Shared formulas
/// turn a few bytes per cell into a copy of the master's text, so the sum is bounded, not
/// only each text. Once full it stays full, so the texts kept are a prefix of the workbook.
pub(crate) struct TextMeter {
    used: u64,
    max: u64,
    full: bool,
}

impl TextMeter {
    pub fn new(max: u64) -> TextMeter {
        TextMeter {
            used: 0,
            max,
            full: false,
        }
    }

    /// Takes `bytes` of text if they fit.
    pub fn take(&mut self, bytes: usize) -> bool {
        let n = u64::try_from(bytes).unwrap_or(u64::MAX);
        if self.full || self.used.saturating_add(n) > self.max {
            self.full = true;
            return false;
        }
        self.used += n;
        true
    }

    /// Whether `bytes` would still fit; takes nothing, but a miss makes the meter full.
    pub fn fits(&mut self, bytes: usize) -> bool {
        let n = u64::try_from(bytes).unwrap_or(u64::MAX);
        if self.full || self.used.saturating_add(n) > self.max {
            self.full = true;
            return false;
        }
        true
    }

    pub fn is_full(&self) -> bool {
        self.full
    }

    pub fn max(&self) -> u64 {
        self.max
    }
}

/// A table of the workbook with its place.
struct TableLoc {
    name: String,
    sheet: String,
    start: (u32, u32),
    end: (u32, u32),
}

/// One problem with a formula, for a loss.
pub(crate) struct FormulaProblem {
    pub anchor: String,
    pub what: String,
    /// A parse failure (Unsupported) rather than a damaged text.
    pub unreadable: bool,
}

/// Turns stored formulas into model formulas and collects the logic inputs.
pub(crate) struct Formulas {
    /// Defined names by (uppercase name, uppercase scope sheet).
    names: HashSet<(String, Option<String>)>,
    /// Uppercase names of the workbook's LAMBDA names.
    lambdas: HashSet<String>,
    tables: Vec<TableLoc>,
    worth: HashMap<String, bool>,
    latex: HashMap<String, Option<String>>,
    /// User functions in order of first use, with the first cell that calls them.
    pub user_functions: Vec<(String, String)>,
    user_seen: HashSet<String>,
    pub inputs: LogicInputs,
    pub meter: TextMeter,
    /// Cells a `CONVERT` call reads as its value, by (uppercase sheet, row, column), with
    /// the unit it converts from.
    convert_units: HashMap<(String, u32, u32), String>,
    /// Cells a defined name refers to on its own, with the first such name.
    named_cells: HashMap<(String, u32, u32), String>,
    /// Defined names whose whole text is a number: anchor, name and value.
    literal_names: Vec<(String, String, Value)>,
    /// Formula cells and names the logic index does not know (not analyzed, not kept), so
    /// a cell may be read by more formulas than the index sees.
    pub unindexed_formulas: u64,
    pub unindexed_names: u64,
    /// The syntax cell formulas are stored in, and its name in the model.
    dialect: Dialect,
    dialect_name: &'static str,
    /// What each formula cell calls and reads on its own sheet, for the layout detector: by
    /// uppercase sheet name, then (row, column). Handed out by [`Formulas::layout_facts`].
    layout: HashMap<String, HashMap<(u32, u32), sheetlayout::FormulaIn>>,
}

impl Formulas {
    pub fn new(names: &[DefinedName], max_text: u64) -> Formulas {
        let mut set = HashSet::new();
        let mut lambdas = HashSet::new();
        for n in names {
            set.insert((
                n.name.to_uppercase(),
                n.scope.as_deref().map(str::to_uppercase),
            ));
            if lambda_params(&n.refers_to).is_some() {
                lambdas.insert(n.name.to_uppercase());
            }
        }
        Formulas {
            names: set,
            lambdas,
            tables: Vec::new(),
            worth: HashMap::new(),
            latex: HashMap::new(),
            user_functions: Vec::new(),
            user_seen: HashSet::new(),
            inputs: LogicInputs {
                workbook: Workbook::new(),
                formula_ids: Vec::new(),
            },
            meter: TextMeter::new(max_text),
            convert_units: HashMap::new(),
            named_cells: HashMap::new(),
            literal_names: Vec::new(),
            unindexed_formulas: 0,
            unindexed_names: 0,
            dialect: Dialect::Excel,
            dialect_name: "excel",
            layout: HashMap::new(),
        }
    }

    /// The layout facts of the formula cells of `sheet` (matched without case), handed out
    /// once: the functions each one calls and the ranges it reads on its own sheet. A
    /// formula that was not analyzed is listed with neither.
    pub fn layout_facts(&mut self, sheet: &str) -> HashMap<(u32, u32), sheetlayout::FormulaIn> {
        self.layout
            .remove(&sheet.to_uppercase())
            .unwrap_or_default()
    }

    /// Notes a formula cell for the layout detector, from its analysis when there is one.
    fn note_layout(&mut self, sheet: &str, row: u32, col: u32, a: Option<&Analysis>) {
        let fact = a.map_or_else(sheetlayout::FormulaIn::default, |a| {
            sheetlayout::FormulaIn {
                functions: a.functions.iter().map(|f| f.to_uppercase()).collect(),
                reads: same_sheet_reads(sheet, &a.refs),
            }
        });
        self.layout
            .entry(sheet.to_uppercase())
            .or_default()
            .insert((row, col), fact);
    }

    /// Cell formulas stored as OpenFormula (`of:=[.A1]`), as in ODS files. Their stored text
    /// is kept in `original` as written, with its namespace prefix.
    pub fn open_formula(mut self) -> Formulas {
        self.dialect = Dialect::OpenFormula;
        self.dialect_name = "openformula";
        self
    }

    /// Registers an Excel table. An area formulakit refuses comes back as the message for a
    /// loss; the table then still resolves `[@col]` names but reads no cells.
    pub fn add_table(
        &mut self,
        name: &str,
        sheet: &str,
        area: ((u32, u32), (u32, u32)),
        header_rows: u32,
        totals_rows: u32,
        columns: &[String],
    ) -> Result<(), String> {
        self.tables.push(TableLoc {
            name: name.to_string(),
            sheet: sheet.to_uppercase(),
            start: area.0,
            end: area.1,
        });
        let table = TableArea {
            start: CellPos {
                row: area.0.0,
                col: area.0.1,
            },
            end: CellPos {
                row: area.1.0,
                col: area.1.1,
            },
            header_rows,
            totals_rows,
            columns: columns.to_vec(),
        };
        self.inputs
            .workbook
            .add_table(name, sheet, table)
            .map_err(|e| e.to_string())
    }

    pub fn add_constant(&mut self, sheet: &str, row: u32, col: u32) {
        self.inputs
            .workbook
            .add_constant(sheet, CellPos { row, col });
    }

    /// The name of the table on `sheet` that holds the cell, if any.
    fn table_at(&self, sheet: &str, row: u32, col: u32) -> Option<&str> {
        let sheet = sheet.to_uppercase();
        self.tables
            .iter()
            .find(|t| {
                t.sheet == sheet
                    && (t.start.0..=t.end.0).contains(&row)
                    && (t.start.1..=t.end.1).contains(&col)
            })
            .map(|t| t.name.as_str())
    }

    fn table_named(&self, name: &str) -> Option<&str> {
        self.tables
            .iter()
            .find(|t| t.name.to_uppercase() == name.to_uppercase())
            .map(|t| t.name.as_str())
    }

    /// The anchor of something a formula on `sheet` reads. An unqualified name that is
    /// local to `sheet` gets the sheet; a name that is no defined name but a table becomes
    /// the table.
    fn ref_anchor(&self, sheet: Option<&str>, r: &Ref) -> String {
        if let Ref::Name { name, sheet: None } = r {
            let upper = name.to_uppercase();
            if let Some(s) = sheet
                && self
                    .names
                    .contains(&(upper.clone(), Some(s.to_uppercase())))
            {
                return format!("name:{}!{name}", quote_sheet(s));
            }
            if !self.names.contains(&(upper, None))
                && let Some(t) = self.table_named(name)
            {
                return format!("table:{t}");
            }
        }
        anchor_of(sheet.unwrap_or(""), r)
    }

    /// The anchors of `refs`, each once.
    fn anchors(&self, sheet: Option<&str>, refs: &[Ref]) -> Vec<Anchor> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for r in refs {
            if sheet.is_none()
                && matches!(
                    r,
                    Ref::Cell { sheet: None, .. } | Ref::Range { sheet: None, .. }
                )
            {
                // A workbook name reading cells of the active sheet: no fixed place.
                continue;
            }
            let a = self.ref_anchor(sheet, r);
            if seen.insert(a.clone()) {
                out.push(Anchor(a));
            }
        }
        out
    }

    /// LaTeX for a formula worth math, parsed at most once per pattern key (worth) and per
    /// text (LaTeX: it names the cells, so copies cannot share it).
    fn latex_of(&mut self, a: &Analysis) -> Option<String> {
        let worth = match self.worth.get(&a.pattern_key) {
            Some(w) => *w,
            None => {
                let w = formulakit::worth_math(a);
                self.worth.insert(a.pattern_key.clone(), w);
                w
            }
        };
        if !worth {
            return None;
        }
        if let Some(l) = self.latex.get(&a.display) {
            return l.clone();
        }
        let l = formulakit::latex(&a.display, Dialect::Excel);
        if self.latex.len() >= LATEX_CACHE {
            self.latex.clear();
        }
        self.latex.insert(a.display.clone(), l.clone());
        l
    }

    fn note_user_functions(&mut self, a: &Analysis, anchor: &str) {
        for f in &a.user_functions {
            let bare = f.strip_prefix("_xludf.").unwrap_or(f);
            let upper = bare.to_uppercase();
            if self.lambdas.contains(&upper) {
                continue;
            }
            if self.user_seen.insert(upper) {
                self.user_functions
                    .push((bare.to_string(), anchor.to_string()));
            }
        }
    }

    /// Remembers the unit of each cell a `CONVERT` call in `display` converts: the first
    /// argument must be one plain cell and the second a text literal Excel knows as a unit.
    fn note_convert(&mut self, sheet: &str, at: CellPos, display: &str) {
        for args in calls_of(display, "CONVERT") {
            let [value, from, _to] = args.as_slice() else {
                continue;
            };
            let Some(unit) = text_literal(from).filter(|u| units::is_convert_unit(u)) else {
                continue;
            };
            let Ok(a) = formulakit::analyze(&format!("={}", value.trim()), Dialect::Excel, at)
            else {
                continue;
            };
            if let Some((target, pos)) = plain_cell(&a, Some(sheet)) {
                self.convert_units
                    .entry((target.to_uppercase(), pos.row, pos.col))
                    .or_insert(unit);
            }
        }
    }

    /// The model formula of one cell. `text` is the stored text: without `=` for Excel, as
    /// written (`of:=...`) for OpenFormula; `index` is the position the formula will take in
    /// `Facets.formulas`.
    #[allow(clippy::too_many_arguments)]
    pub fn cell(
        &mut self,
        sheet: &str,
        row: u32,
        col: u32,
        text: &str,
        kind: FormulaKind,
        range: Option<String>,
        cached: Option<Value>,
        index: usize,
    ) -> (Formula, Option<FormulaProblem>) {
        let anchor = format!("{}!{}", quote_sheet(sheet), crate::grid::a1(row, col));
        let stored = match self.dialect {
            Dialect::Excel => format!("={text}"),
            Dialect::OpenFormula => text.to_string(),
        };
        let mut formula = Formula {
            anchor: Anchor(anchor.clone()),
            expression: stored.clone(),
            dialect: self.dialect_name.into(),
            references: Vec::new(),
            role: None,
            pattern: None,
            latex: None,
            note: None,
            kind: Some(kind),
            range: range.map(|r| Anchor(format!("{}!{}", quote_sheet(sheet), r.replace('$', "")))),
            original: None,
            cached: None,
        };
        let mut notes = Vec::new();
        if cached.is_none() {
            notes.push("not calculated in file".to_string());
        }
        formula.cached = cached;
        let mut problem = None;
        if too_long(text) {
            self.note_layout(sheet, row, col, None);
            self.unindexed_formulas += 1;
            notes.push(format!(
                "longer than {MAX_FORMULA_CHARS} characters, Excel's maximum: not analyzed"
            ));
            problem = Some(FormulaProblem {
                anchor: anchor.clone(),
                what: format!(
                    "formula text of {} characters is longer than Excel's maximum of {MAX_FORMULA_CHARS}; kept as text, not analyzed",
                    text.chars().count()
                ),
                unreadable: false,
            });
        } else {
            match formulakit::analyze(text, self.dialect, CellPos { row, col }) {
                Ok(mut a) => {
                    self.note_layout(sheet, row, col, Some(&a));
                    // `[@col]` inside a table names no table: it is the one holding the cell.
                    if let Some(t) = self.table_at(sheet, row, col).map(str::to_string) {
                        for r in &mut a.refs {
                            if let Ref::Table { name, .. } = r
                                && name.is_empty()
                            {
                                *name = t.clone();
                            }
                        }
                    }
                    formula.references = self.anchors(Some(sheet), &a.refs);
                    formula.latex = self.latex_of(&a);
                    if a.display != stored {
                        // The display form is a second copy of the text.
                        if self.meter.take(a.display.len()) {
                            formula.original = Some(stored);
                            formula.expression = a.display.clone();
                        } else {
                            notes.push(
                                "display form not kept: formula text limit reached".to_string(),
                            );
                        }
                    }
                    self.note_user_functions(&a, &anchor);
                    if a.functions.iter().any(|f| f == "CONVERT") {
                        self.note_convert(sheet, CellPos { row, col }, &a.display);
                    }
                    let id = self
                        .inputs
                        .workbook
                        .add_formula(sheet, CellPos { row, col }, &a);
                    self.inputs.formula_ids.push((index, id));
                }
                Err(e) => {
                    self.note_layout(sheet, row, col, None);
                    self.unindexed_formulas += 1;
                    notes.push(e.to_string());
                    problem = Some(FormulaProblem {
                        anchor: anchor.clone(),
                        what: format!("{e}; the text is kept"),
                        unreadable: true,
                    });
                }
            }
        }
        if !notes.is_empty() {
            formula.note = Some(notes.join("; "));
        }
        (formula, problem)
    }

    /// A cell formula whose text is kept but not analyzed, because `why` makes the text
    /// unreliable. The logic index does not know it.
    #[allow(clippy::too_many_arguments)]
    pub fn cell_not_analyzed(
        &mut self,
        sheet: &str,
        row: u32,
        col: u32,
        text: &str,
        why: &str,
        cached: Option<Value>,
    ) -> (Formula, FormulaProblem) {
        let anchor = format!("{}!{}", quote_sheet(sheet), crate::grid::a1(row, col));
        self.note_layout(sheet, row, col, None);
        self.unindexed_formulas += 1;
        let mut notes = Vec::new();
        if cached.is_none() {
            notes.push("not calculated in file".to_string());
        }
        notes.push(format!("not analyzed: {why}"));
        let formula = Formula {
            anchor: Anchor(anchor.clone()),
            expression: match self.dialect {
                Dialect::Excel => format!("={text}"),
                Dialect::OpenFormula => text.to_string(),
            },
            dialect: self.dialect_name.into(),
            references: Vec::new(),
            role: None,
            pattern: None,
            latex: None,
            note: Some(notes.join("; ")),
            kind: Some(FormulaKind::Cell),
            range: None,
            original: None,
            cached,
        };
        let problem = FormulaProblem {
            anchor,
            what: format!("formula not analyzed: {why}; the text is kept"),
            unreadable: true,
        };
        (formula, problem)
    }

    /// A defined name whose text is not analyzed (`why`): listed with its text as written,
    /// unknown to the logic index.
    pub fn name_not_analyzed(&mut self, n: &DefinedName, why: &str) -> Formula {
        self.name_not_kept(n);
        let anchor = match &n.scope {
            Some(s) => format!("name:{}!{}", quote_sheet(s), n.name),
            None => format!("name:{}", n.name),
        };
        Formula {
            anchor: Anchor(anchor),
            expression: n.refers_to.clone(),
            dialect: "excel".into(),
            references: Vec::new(),
            role: None,
            pattern: None,
            latex: None,
            note: Some(why.to_string()),
            kind: Some(FormulaKind::Name),
            range: None,
            original: None,
            cached: None,
        }
    }

    /// A what-if data table: Excel shows it as `{=TABLE(row input, column input)}`.
    #[allow(clippy::too_many_arguments)]
    pub fn data_table(
        &mut self,
        sheet: &str,
        row: u32,
        col: u32,
        range: Option<&str>,
        r1: Option<&str>,
        r2: Option<&str>,
        two_d: bool,
        row_input: bool,
        cached: Option<Value>,
        index: usize,
    ) -> Formula {
        let q = quote_sheet(sheet);
        let (row_cell, col_cell) = if two_d {
            (r1, r2)
        } else if row_input {
            (r1, None)
        } else {
            (None, r1)
        };
        let expression = format!(
            "=TABLE({},{})",
            row_cell.unwrap_or(""),
            col_cell.unwrap_or("")
        );
        let mut inputs = Vec::new();
        let mut references = Vec::new();
        for (label, c) in [("row input", row_cell), ("column input", col_cell)] {
            if let Some(c) = c {
                inputs.push(format!("{label} {}", c.replace('$', "")));
                references.push(Anchor(format!("{q}!{}", c.replace('$', ""))));
            }
        }
        let mut notes = vec![format!("what-if data table: {}", inputs.join(", "))];
        if cached.is_none() {
            notes.push("not calculated in file".to_string());
        }
        self.note_layout(sheet, row, col, None);
        // The logic index sees the inputs the table reads.
        match formulakit::analyze(&expression, Dialect::Excel, CellPos { row, col }) {
            Ok(a) => {
                let id = self
                    .inputs
                    .workbook
                    .add_formula(sheet, CellPos { row, col }, &a);
                self.inputs.formula_ids.push((index, id));
            }
            Err(_) => self.unindexed_formulas += 1,
        }
        Formula {
            anchor: Anchor(format!("{q}!{}", crate::grid::a1(row, col))),
            expression,
            dialect: "excel".into(),
            references,
            cached,
            role: None,
            pattern: None,
            latex: None,
            note: Some(notes.join("; ")),
            kind: Some(FormulaKind::DataTable),
            range: range.map(|r| Anchor(format!("{q}!{}", r.replace('$', "")))),
            original: None,
        }
    }

    /// A defined name whose text is not kept: the logic index still knows the name.
    pub fn name_not_kept(&mut self, n: &DefinedName) {
        self.unindexed_names += 1;
        self.inputs
            .workbook
            .add_name(&n.name, n.scope.as_deref(), None);
    }

    /// The model formula of a defined name: `name:gg` or `name:Sheet1!gg`, kind `name` or
    /// `lambda` (with its parameters in the note).
    pub fn name(&mut self, n: &DefinedName) -> (Formula, Option<FormulaProblem>) {
        let anchor = match &n.scope {
            Some(s) => format!("name:{}!{}", quote_sheet(s), n.name),
            None => format!("name:{}", n.name),
        };
        let params = lambda_params(&n.refers_to);
        let mut notes = Vec::new();
        if let Some(p) = &params {
            notes.push(format!("parameters: {}", p.join(", ")));
        }
        if n.hidden {
            notes.push("hidden".to_string());
        }
        if let Some(c) = n.comment.as_ref().filter(|c| !c.trim().is_empty()) {
            notes.push(c.trim().to_string());
        }
        // ODF named expressions carry their namespace (`of:=`); named ranges and OOXML names
        // are Excel text. formulakit switches on the prefix.
        let open_formula = ["of:", "oooc:"]
            .iter()
            .any(|p| n.refers_to.trim_start().starts_with(p));
        let mut formula = Formula {
            anchor: Anchor(anchor.clone()),
            expression: n.refers_to.clone(),
            dialect: if open_formula { "openformula" } else { "excel" }.into(),
            references: Vec::new(),
            cached: None,
            role: None,
            pattern: None,
            latex: None,
            note: None,
            kind: Some(if params.is_some() {
                FormulaKind::Lambda
            } else {
                FormulaKind::Name
            }),
            range: None,
            original: None,
        };
        let mut problem = None;
        if too_long(&n.refers_to) {
            problem = Some(FormulaProblem {
                anchor: anchor.clone(),
                what: format!(
                    "name text longer than Excel's maximum of {MAX_FORMULA_CHARS} characters; kept as text, not analyzed"
                ),
                unreadable: false,
            });
            self.unindexed_names += 1;
            self.inputs
                .workbook
                .add_name(&n.name, n.scope.as_deref(), None);
        } else {
            match formulakit::analyze(&n.refers_to, Dialect::Excel, CellPos::default()) {
                Ok(a) => {
                    formula.references = self.anchors(n.scope.as_deref(), &a.refs);
                    // The text as written has no `=` and may carry `_xlfn.` prefixes.
                    let shown = a.display.strip_prefix('=').unwrap_or(&a.display);
                    if shown != n.refers_to {
                        if self.meter.take(shown.len()) {
                            formula.original = Some(n.refers_to.clone());
                            formula.expression = shown.to_string();
                        } else {
                            notes.push(
                                "display form not kept: formula text limit reached".to_string(),
                            );
                        }
                    }
                    self.note_user_functions(&a, &anchor);
                    if let Some((sheet, pos)) = plain_cell(&a, n.scope.as_deref()) {
                        self.named_cells
                            .entry((sheet.to_uppercase(), pos.row, pos.col))
                            .or_insert_with(|| n.name.clone());
                    }
                    if a.refs.is_empty()
                        && a.functions.is_empty()
                        && let Some(v) = literal_number(&n.refers_to)
                    {
                        self.literal_names.push((anchor.clone(), n.name.clone(), v));
                    }
                    self.inputs
                        .workbook
                        .add_name(&n.name, n.scope.as_deref(), Some(&a));
                }
                Err(e) => {
                    self.unindexed_names += 1;
                    notes.push(e.to_string());
                    problem = Some(FormulaProblem {
                        anchor: anchor.clone(),
                        what: format!("{e}; the text is kept"),
                        unreadable: true,
                    });
                    self.inputs
                        .workbook
                        .add_name(&n.name, n.scope.as_deref(), None);
                }
            }
        }
        if !notes.is_empty() {
            formula.note = Some(notes.join("; "));
        }
        (formula, problem)
    }
}

/// More characters than Excel allows in a formula.
/// The ranges `refs` read on `sheet` itself: references without a sheet, or naming this
/// sheet (without case) and no other. References into other sheets, 3D references, other
/// workbooks, names and table references are left out.
fn same_sheet_reads(sheet: &str, refs: &[Ref]) -> Vec<sheetlayout::Rect> {
    let here = |s: &Option<formulakit::SheetSpan>| match s {
        None => true,
        Some(span) => span.last.is_none() && span.first.eq_ignore_ascii_case(sheet),
    };
    refs.iter()
        .filter_map(|r| match r {
            Ref::Cell { sheet: s, pos, .. } if here(s) => Some(sheetlayout::Rect {
                top: pos.row,
                left: pos.col,
                bottom: pos.row,
                right: pos.col,
            }),
            Ref::Range {
                sheet: s,
                start,
                end,
                ..
            } if here(s) => Some(
                sheetlayout::Rect {
                    top: start.row,
                    left: start.col,
                    bottom: end.row,
                    right: end.col,
                }
                .normalized(),
            ),
            _ => None,
        })
        .collect()
}

fn too_long(text: &str) -> bool {
    text.len() > MAX_FORMULA_CHARS && text.chars().count() > MAX_FORMULA_CHARS
}

/// The parameters of a name whose whole text is one LAMBDA call (`LAMBDA(x, y, x*y)` or
/// `_xlfn.LAMBDA(_xlpm.x, ...)`): every argument but the last. `None` for anything else,
/// also for `LET(f, LAMBDA(...), f(1))` or a LAMBDA that is called right away.
pub(crate) fn lambda_params(text: &str) -> Option<Vec<String>> {
    let body = text.trim().trim_start_matches('=').trim_start();
    let body = body.strip_prefix("_xlfn.").unwrap_or(body);
    let head = body.get(..7)?;
    if !head.eq_ignore_ascii_case("LAMBDA(") {
        return None;
    }
    let args = top_level_args(body.get(7..)?)?;
    if args.len() < 2 {
        return None;
    }
    let params = args[..args.len() - 1]
        .iter()
        .map(|p| {
            let p = p.trim();
            p.strip_prefix("_xlpm.").unwrap_or(p).to_string()
        })
        .collect();
    Some(params)
}

#[cfg(test)]
mod tests;
