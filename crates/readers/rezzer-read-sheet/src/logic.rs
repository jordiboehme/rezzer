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

/// The arguments of a call, given the text after its `(`. `None` when the closing `)` is not
/// the end of the text.
fn top_level_args(text: &str) -> Option<Vec<String>> {
    let (args, rest) = call_args(text)?;
    rest.trim().is_empty().then_some(args)
}

/// The arguments of a call, given the text after its `(`, and the text after its `)`.
/// `None` when the call is not closed.
fn call_args(text: &str) -> Option<(Vec<String>, &str)> {
    let mut depth = 0usize;
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' | '\'' => {
                current.push(c);
                // Quoted text or sheet name; a doubled quote stays inside.
                for (_, d) in chars.by_ref() {
                    current.push(d);
                    if d == c {
                        break;
                    }
                }
            }
            '(' | '{' | '[' => {
                depth += 1;
                current.push(c);
            }
            ')' | '}' | ']' if depth > 0 => {
                depth -= 1;
                current.push(c);
            }
            ')' => {
                args.push(std::mem::take(&mut current));
                return Some((args, &text[i + 1..]));
            }
            ',' if depth == 0 => args.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    None
}

/// The arguments of every call of `function` in a formula text, outside text literals and
/// quoted sheet names.
fn calls_of(text: &str, function: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut prev: Option<char> = None;
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            prev = Some(c);
            continue;
        }
        if c == '"' || c == '\'' {
            quote = Some(c);
        } else if !prev.is_some_and(|p| p.is_alphanumeric() || p == '_' || p == '.')
            && let Some(head) = text.get(i..i + function.len() + 1)
            && head
                .get(..function.len())
                .is_some_and(|h| h.eq_ignore_ascii_case(function))
            && head.ends_with('(')
            && let Some((args, _)) = call_args(&text[i + function.len() + 1..])
        {
            out.push(args);
        }
        prev = Some(c);
    }
    out
}

/// The text of a string literal argument (`"mi"`), with doubled quotes undone.
fn text_literal(arg: &str) -> Option<String> {
    let inner = arg.trim().strip_prefix('"')?.strip_suffix('"')?;
    let text = inner.replace("\"\"", "\"");
    (!text.is_empty()).then_some(text)
}

/// The one cell a formula consists of (`=B5`, `=$L$3`, `=z!$L$3`), with its sheet. A
/// reference without a sheet points into `sheet`; `None` when there is none to point to.
fn plain_cell(a: &Analysis, sheet: Option<&str>) -> Option<(String, CellPos)> {
    let [
        Ref::Cell {
            sheet: span, pos, ..
        },
    ] = a.refs.as_slice()
    else {
        return None;
    };
    if !a.functions.is_empty() || span.as_ref().is_some_and(|s| s.last.is_some()) {
        return None;
    }
    let target = match span {
        Some(s) => s.first.clone(),
        None => sheet?.to_string(),
    };
    let cell = crate::grid::a1(pos.row, pos.col);
    let shown = a.display.trim_start_matches('=').replace('$', "");
    let alone = shown == cell || shown == format!("{}!{cell}", quote_sheet(&target));
    alone.then_some((target, *pos))
}

/// The value of a name whose whole text is one number (`0.05`, `=-3`, `1.5E-3`, `5%`).
fn literal_number(text: &str) -> Option<Value> {
    let t = text.trim().trim_start_matches('=').trim();
    let (digits, percent) = match t.strip_suffix('%') {
        Some(d) => (d.trim_end(), true),
        None => (t, false),
    };
    let body = digits.strip_prefix(['-', '+']).unwrap_or(digits);
    if !body.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        || !body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'E' | 'e' | '+' | '-'))
    {
        return None;
    }
    let v: f64 = digits.parse().ok().filter(|v: &f64| v.is_finite())?;
    Some(if percent {
        Value::percent(v / 100.0)
    } else {
        Value::number(v, None)
    })
}

/// The cells of one sheet the logic pass looks at for labels, values and number formats.
pub(crate) struct SheetFacts {
    pub name: String,
    /// Sorted by row, then column, each position once (formula texts dropped).
    pub cells: Vec<RawCell>,
    /// The format key of each cell that has one (an OOXML style index, or an index into the
    /// reader's own format list); [`ReadState::formats`] gives its code.
    pub styles: HashMap<(u32, u32), u32>,
    /// What names the cells of the sheet's regions.
    pub paths: SheetPaths,
}

impl SheetFacts {
    pub fn new(
        name: &str,
        mut cells: Vec<RawCell>,
        styles: HashMap<(u32, u32), u32>,
        paths: SheetPaths,
    ) -> SheetFacts {
        // Only whether a cell holds a formula matters here; the texts are in the facet.
        for c in &mut cells {
            if c.formula.is_some() {
                c.formula = Some(String::new());
            }
        }
        SheetFacts {
            name: name.to_string(),
            cells,
            styles,
            paths,
        }
    }
}

/// How the workbook was read, for the logic pass.
pub(crate) struct ReadState<'a> {
    /// The calculation settings, when the file gives them.
    pub calc: Option<&'a CalcSettings>,
    /// The number format code of a format key in [`SheetFacts::styles`].
    pub formats: &'a dyn Fn(u32) -> Option<&'a str>,
    /// Some sheet or part of one was not read, or some formula cells were not listed.
    pub incomplete: bool,
}

/// Labels longer than this are text, not labels.
const MAX_LABEL: usize = 200;

/// Empty cells a label or header may be away from its value.
const LABEL_GAP: u32 = 3;

/// The kind of edge in the dependency graph: `from` feeds `to` (`to` reads `from`).
const EDGE_KIND: &str = "feeds";

/// The workbook logic: roles, pattern groups, parameters, the dependency graph and the
/// stored-results warning. Runs after every sheet and name has been read.
pub(crate) fn workbook_logic(
    out: &mut Out,
    formulas: Formulas,
    sheets: Vec<SheetFacts>,
    state: ReadState,
    budget: &Budget,
) {
    stale_results(out, state.calc);
    let Formulas {
        inputs,
        convert_units,
        named_cells,
        literal_names,
        unindexed_formulas,
        unindexed_names,
        tables,
        ..
    } = formulas;
    let LogicInputs {
        workbook,
        formula_ids,
    } = inputs;
    let stop = || budget.check_time().is_err();
    let logic = match contain(AssertUnwindSafe(|| workbook.finish(&stop))) {
        Ok(l) => l,
        Err(panic) => {
            out.loss(
                "document",
                format!("roles, patterns, parameters and the dependency graph not worked out: internal error: {panic}"),
                LossReason::Internal,
                Severity::Warning,
            );
            return;
        }
    };
    let mut facet_of: Vec<Option<usize>> = Vec::new();
    for (index, id) in &formula_ids {
        let i = id.0 as usize;
        if facet_of.len() <= i {
            facet_of.resize(i + 1, None);
        }
        facet_of[i] = Some(*index);
    }
    let facet = |id: FormulaId| facet_of.get(id.0 as usize).copied().flatten();

    // A formula the index does not know may read any cell: no cell is surely a result.
    let mut gaps = Vec::new();
    match unindexed_formulas {
        0 => {}
        1 => gaps.push("1 formula could not be analyzed".to_string()),
        n => gaps.push(format!("{n} formulas could not be analyzed")),
    }
    match unindexed_names {
        0 => {}
        1 => gaps.push("1 defined name could not be analyzed".to_string()),
        n => gaps.push(format!("{n} defined names could not be analyzed")),
    }
    if state.incomplete {
        gaps.push("not every formula cell was read".to_string());
    }
    let certain = gaps.is_empty();
    if !certain {
        out.loss(
            "document",
            format!(
                "the workbook logic is incomplete: {}; no formula is marked as a result and read counts may be low",
                gaps.join("; ")
            ),
            LossReason::Uncertain,
            Severity::Warning,
        );
    }

    let formulas_facet = &mut out.doc.facets.formulas;
    for (id, role) in &logic.roles {
        let role = match role {
            KitRole::Constant => Role::Constant,
            KitRole::Intermediate => Role::Intermediate,
            KitRole::Result if certain => Role::Result,
            KitRole::Result => continue,
        };
        if let Some(f) = facet(*id).and_then(|i| formulas_facet.get_mut(i)) {
            f.role = Some(role);
        }
    }
    for g in &logic.groups {
        if g.members.len() < 2 {
            continue;
        }
        let range = Anchor(format!("{}!{}", quote_sheet(&g.sheet), g.area));
        for m in &g.members {
            if let Some(f) = facet(*m).and_then(|i| formulas_facet.get_mut(i)) {
                f.pattern = Some(g.id.clone());
                // An array, spill or data table keeps the area it fills.
                if f.range.is_none() {
                    f.range = Some(range.clone());
                }
            }
        }
    }
    if logic.stopped {
        let kept = if logic.parameters.is_empty() {
            "patterns and constant formulas are kept; other roles, parameters and the dependency graph are not set"
        } else {
            "roles, patterns and parameters are kept; the dependency graph is not set"
        };
        out.loss(
            "document",
            format!("time limit reached while the workbook logic was worked out: {kept}"),
            LossReason::Limit,
            Severity::Warning,
        );
    }

    let mut path_text = TextMeter::new(budget.limits().max_text_bytes);
    let (mut params, unit_notes) = cell_parameters(
        &logic.parameters,
        &sheets,
        state.formats,
        &convert_units,
        &named_cells,
        &mut path_text,
    );
    if path_text.is_full() {
        out.loss(
            "document",
            format!(
                "text limit of {} bytes reached: later parameters keep their one label, not their path of headings",
                path_text.max()
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    for (anchor, what) in unit_notes {
        out.loss(anchor, what, LossReason::Uncertain, Severity::Info);
    }
    params.extend(name_parameters(&out.doc.facets.formulas, literal_names));
    let total = params.len();
    for (i, p) in params.into_iter().enumerate() {
        if let Err(hit) = budget.add_elements(1) {
            out.loss(
                "document",
                format!("{hit}: {} of {total} parameters not listed", total - i),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        out.doc.facets.parameters.push(p);
    }

    if !logic.stopped
        && !logic.graph.nodes.is_empty()
        && let Some(g) = dependency_graph(out, &logic.graph, &tables, budget)
    {
        out.doc.facets.graphs.push(g);
    }
}

/// One `Uncertain` warning when the stored results may not match the formulas: manual
/// calculation, data tables left out of automatic calculation, iterative calculation, a file
/// that asks for a full recalculation, or formulas without a stored result.
fn stale_results(out: &mut Out, calc: Option<&CalcSettings>) {
    let formulas = &out.doc.facets.formulas;
    let mut why = Vec::new();
    let default = CalcSettings::default();
    let calc = calc.unwrap_or(&default);
    if calc.mode == CalcMode::Manual {
        why.push("calculation is set to manual".to_string());
    }
    if calc.mode == CalcMode::AutoNoTable
        && formulas
            .iter()
            .any(|f| f.kind == Some(FormulaKind::DataTable))
    {
        why.push("data tables are not recalculated automatically".to_string());
    }
    if calc.iterate {
        why.push("iterative calculation is on".to_string());
    }
    if calc.full_calc_on_load {
        why.push("the file asks for a full recalculation when opened".to_string());
    }
    let missing = formulas
        .iter()
        .filter(|f| {
            f.cached.is_none() && !matches!(f.kind, Some(FormulaKind::Name | FormulaKind::Lambda))
        })
        .count();
    match missing {
        0 => {}
        1 => why.push("1 formula has no stored result".to_string()),
        n => why.push(format!("{n} formulas have no stored result")),
    }
    if !why.is_empty() {
        out.loss(
            "document",
            format!("{STALE_RESULTS}: {}", why.join("; ")),
            LossReason::Uncertain,
            Severity::Warning,
        );
    }
}

/// The texts around a value: the label to its left, the text right beside it (where a unit
/// sits in `Diameter | 10.0 | m`) when its column reads as a unit column, the header above
/// it.
#[derive(Clone, Copy, Default)]
struct Around<'a> {
    left: Option<&'a str>,
    /// The text on the left is a value of the row (a category like `blue`), not the name of
    /// this value: it occurs more than once in its column, or its column is a column of
    /// categories. A header above wins over it.
    left_is_value: bool,
    beside: Option<&'a str>,
    header: Option<&'a str>,
}

/// What the sweep knows about one column.
#[derive(Clone, Copy)]
struct Column<'a> {
    /// The last row with a cell, and whether that cell holds a value or formula.
    last: u32,
    last_data: bool,
    /// The last text that heads the cells below it, with its row.
    head: Option<(&'a str, u32)>,
    /// Every row from the head down to `last` holds a value or formula: an unbroken run.
    solid: bool,
    /// The header at the top of the current run of values.
    block: Option<&'a str>,
}

/// Words a column header uses for a column of units.
const UNIT_HEADERS: &[&str] = &["unit", "units", "uom", "einheit", "einheiten", "dimension"];

/// The texts around each wanted cell, in one sweep over the sorted cells.
/// - The label to the left is at most [`LABEL_GAP`] empty cells away; a value or formula in
///   between stops the search. Labels name one value each and categories repeat: a text
///   that occurs more than once in its column, or that stands in a column where at least
///   half of the texts repeat an earlier one (four texts or more), is marked as a value of
///   the row.
/// - The header is the first text above the value when an unbroken run of values or
///   formulas joins them (any length, so every value under `t (m)` finds it). Across empty
///   cells it is at most [`LABEL_GAP`] rows above. A text right under values of the same
///   column (`see note` inside a number column) is a value, not a header.
fn labels_of<'a>(
    cells: &'a [RawCell],
    wanted: &HashSet<(u32, u32)>,
) -> HashMap<(u32, u32), Around<'a>> {
    let label = |t: &'a str| is_label(t).then_some(t);
    let mut columns: HashMap<u32, Column<'a>> = HashMap::new();
    let mut out = HashMap::new();
    // How often each text occurs in its column, and how many texts and distinct texts
    // each column has.
    let mut in_column: HashMap<(u32, &'a str), u32> = HashMap::new();
    let mut texts: HashMap<u32, (u32, u32)> = HashMap::new();
    for c in cells {
        if let Some(t) = text_of(c) {
            let n = in_column.entry((c.col, t.trim())).or_default();
            *n += 1;
            let column = texts.entry(c.col).or_default();
            column.0 += 1;
            if *n == 1 {
                column.1 += 1;
            }
        }
    }
    let categories = |col: u32| {
        texts
            .get(&col)
            .is_some_and(|&(all, distinct)| all >= 4 && distinct * 2 <= all)
    };
    // The verdict on each text run beside values, kept for every cell of the run.
    let mut unit_columns: HashMap<usize, UnitColumn> = HashMap::new();
    for (i, c) in cells.iter().enumerate() {
        let near = columns
            .get(&c.col)
            .copied()
            .filter(|s| c.row - s.last - 1 <= LABEL_GAP);
        let text = text_of(c);
        let data = text.is_none() && !(c.val == Val::Empty && c.formula.is_none());
        if wanted.contains(&(c.row, c.col)) {
            let left_cell = i
                .checked_sub(1)
                .and_then(|j| cells.get(j))
                .filter(|p| p.row == c.row && c.col - p.col - 1 <= LABEL_GAP);
            let left = left_cell.and_then(text_of).and_then(label);
            let left_is_value = left_cell.zip(left).is_some_and(|(p, t)| {
                in_column.get(&(p.col, t.trim())).copied().unwrap_or(0) > 1 || categories(p.col)
            });
            let unbroken = near.is_some_and(|s| s.solid && c.row == s.last + 1);
            let header = near
                .and_then(|s| s.head)
                .filter(|(_, row)| unbroken || c.row - row <= LABEL_GAP)
                .map(|(t, _)| t);
            // The value heads no table column of its own: `label | value | unit` rows.
            let in_table = near.and_then(|s| s.block).is_some();
            let beside = cells
                .get(i + 1)
                .filter(|n| n.row == c.row && n.col == c.col + 1)
                .and_then(|n| {
                    let t = text_of(n)?;
                    let column = match unit_columns.get(&(i + 1)) {
                        Some(v) => *v,
                        None => {
                            let run = text_run(cells, i + 1);
                            let v = reads_as_unit_column(cells, &run);
                            unit_columns.extend(run.into_iter().map(|j| (j, v)));
                            v
                        }
                    };
                    let ok = match column {
                        UnitColumn::Declared => true,
                        UnitColumn::Likely => left.is_some() && !in_table,
                        UnitColumn::No => false,
                    };
                    ok.then_some(t)
                });
            out.insert(
                (c.row, c.col),
                Around {
                    left,
                    left_is_value,
                    beside,
                    header,
                },
            );
        }
        let state = match text {
            Some(t) => {
                // A text right under values is one more value of the column.
                let heads = !near.is_some_and(|s| s.last_data);
                let head = label(t).filter(|_| heads);
                Column {
                    last: c.row,
                    last_data: false,
                    head: head.map(|h| (h, c.row)),
                    solid: head.is_some(),
                    block: head,
                }
            }
            None => Column {
                last: c.row,
                last_data: data,
                head: near.and_then(|s| s.head),
                solid: data && near.is_some_and(|s| s.solid && c.row == s.last + 1),
                block: near.and_then(|s| s.block),
            },
        };
        columns.insert(c.col, state);
    }
    out
}

/// A text cell's text, for cells that hold a text and no formula.
fn text_of(c: &RawCell) -> Option<&str> {
    match &c.val {
        Val::Text(t) if c.formula.is_none() && !c.shared_unknown && !c.text_dropped => {
            Some(t.as_str())
        }
        _ => None,
    }
}

/// How a column of texts beside values reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnitColumn {
    /// Its header says it holds units (`Unit`, `Einheit`).
    Declared,
    /// It reads as units: at least two known units of two or more characters, most of its
    /// texts known units, and at least one that no code looks like (`kN/m3`, `m2`, `°C`).
    Likely,
    No,
}

/// The cells above and below `i` in its column that hold text, with no gap, top first.
fn text_run(cells: &[RawCell], i: usize) -> Vec<usize> {
    let col = cells[i].col;
    let find = |row: u32| {
        cells
            .binary_search_by_key(&(row, col), |c| (c.row, c.col))
            .ok()
            .filter(|&j| text_of(&cells[j]).is_some())
    };
    // The whole run: its verdict is kept for every cell of it, so each run is walked once.
    let mut up = Vec::new();
    let mut row = cells[i].row;
    while let Some(r) = row.checked_sub(1)
        && let Some(j) = find(r)
    {
        up.push(j);
        row = r;
    }
    up.reverse();
    up.push(i);
    let mut row = cells[i].row;
    while let Some(j) = row.checked_add(1).and_then(find) {
        up.push(j);
        row += 1;
    }
    up
}

/// Whether a text reads as a label: not empty after trimming and at most [`MAX_LABEL`]
/// bytes. The rule of [`labels_of`], also handed to the layout detector.
pub(crate) fn is_label(text: &str) -> bool {
    text.len() <= MAX_LABEL && !text.trim().is_empty()
}

/// Whether the texts of a column, top first, read as units: declared by its first text or
/// likely (see [`UnitColumn`]). The rule of [`reads_as_unit_column`], also handed to the
/// layout detector.
pub(crate) fn is_unit_column_texts(texts: &[&str]) -> bool {
    let texts: Vec<&str> = texts.iter().map(|t| t.trim()).collect();
    unit_column_of(texts) != UnitColumn::No
}

/// How the text run `run` reads (see [`UnitColumn`]).
fn reads_as_unit_column(cells: &[RawCell], run: &[usize]) -> UnitColumn {
    let texts: Vec<&str> = run
        .iter()
        .filter_map(|&j| text_of(&cells[j]))
        .map(str::trim)
        .collect();
    unit_column_of(texts)
}

/// Unit symbols that read as units on their own, also without a digit or a sign: SI base
/// and derived units with the usual prefixes and common engineering units. A column of
/// these (`m`, `kPa`, `kN`) is a unit column even though no text in it looks like `kN/m3`.
const UNIT_SYMBOLS: &[&str] = &[
    "mm", "cm", "dm", "m", "km", "mm2", "cm2", "m2", "km2", "mm3", "cm3", "m3", "mg", "g", "kg",
    "t", "ms", "s", "min", "h", "hr", "d", "N", "kN", "MN", "Nm", "kNm", "Pa", "kPa", "MPa", "GPa",
    "bar", "mbar", "%", "deg", "°", "°C", "K", "mW", "W", "kW", "MW", "GW", "Wh", "kWh", "MWh",
    "J", "kJ", "MJ", "ml", "mL", "l", "L", "mV", "V", "kV", "mA", "A", "kA", "Hz", "kHz", "MHz",
    "rad", "ha", "ppm",
];

/// Words that count things (`nos` for numbers, `pcs` for pieces). They read as a unit only
/// in a column that also holds a real unit symbol: a column of `no` alone is a yes/no
/// column.
const COUNT_WORDS: &[&str] = &["nos", "no", "pcs", "ea"];

/// Whether `t` is one of [`UNIT_SYMBOLS`] or a quotient of two of them, like `m/s` or
/// `kN/m3`. A quotient of two upper case letters is a code (`N/A`, `A/C`), never a unit.
fn is_unit_symbol(t: &str) -> bool {
    let known = |p: &str| UNIT_SYMBOLS.contains(&p);
    match t.split_once('/') {
        Some((a, b)) => {
            let code = |p: &str| p.len() == 1 && p.chars().all(|c| c.is_ascii_uppercase());
            known(a) && known(b) && !(code(a) && code(b))
        }
        None => known(t),
    }
}

/// How trimmed texts of a column, top first, read (see [`UnitColumn`]).
fn unit_column_of(texts: Vec<&str>) -> UnitColumn {
    if texts
        .first()
        .is_some_and(|t| UNIT_HEADERS.iter().any(|h| t.eq_ignore_ascii_case(h)))
    {
        return UnitColumn::Declared;
    }
    let counted: Vec<&str> = texts.into_iter().filter(|t| *t != "-").collect();
    // Every text is a unit symbol or a count word: units, also when none is marked by a
    // digit or a sign. One-letter symbols alone (`A`, `N`, `m` and `d`) are as often classes,
    // flags or codes, so the column needs a symbol of two or more characters, or a count
    // word beside a real symbol.
    let count = |t: &str| COUNT_WORDS.contains(&t);
    if counted.len() >= 2 && counted.iter().all(|t| is_unit_symbol(t) || count(t)) {
        let real = || counted.iter().filter(|t| !count(t));
        let long = real().any(|t| t.chars().count() >= 2);
        let counts = counted.iter().any(|t| count(t)) && real().next().is_some();
        if long || counts {
            return UnitColumn::Likely;
        }
    }
    let known: Vec<&str> = counted
        .iter()
        .copied()
        .filter(|t| units::is_unit(t, true) || is_unit_symbol(t))
        .collect();
    let long = known.iter().filter(|t| t.chars().count() > 1).count();
    let marked = known.iter().any(|t| {
        t.chars()
            .any(|c| c.is_ascii_digit() || "/^²³°%·".contains(c))
    });
    if long >= 2 && known.len() * 2 >= counted.len() && marked {
        UnitColumn::Likely
    } else {
        UnitColumn::No
    }
}

/// The value of a cell as a typed value; a number with a percent format is a percent.
pub(crate) fn cell_value(val: &Val, code: Option<&str>) -> Option<Value> {
    match val {
        Val::Empty => None,
        Val::Num(v) => match code.map(numfmt::kind_of) {
            Some(NumKind::Percent) => Some(Value::percent(*v)),
            _ => Some(Value::number(*v, None)),
        },
        Val::Text(s) | Val::Error(s) => Some(Value::Text { value: s.clone() }),
        Val::Iso(s) => Some(Value::DateTime { value: s.clone() }),
        Val::Bool(b) => Some(Value::Bool { value: *b }),
    }
}

/// Parameters for the constant cells formulas read, with label, path, value, name and unit,
/// plus a note (anchor, text) for each value whose unit sources disagree. The path texts of
/// all parameters together are bounded by `paths`; past it a parameter keeps its one label.
fn cell_parameters<'f>(
    found: &[ParamCell],
    sheets: &[SheetFacts],
    formats: &dyn Fn(u32) -> Option<&'f str>,
    convert_units: &HashMap<(String, u32, u32), String>,
    named_cells: &HashMap<(String, u32, u32), String>,
    paths: &mut TextMeter,
) -> (Vec<Parameter>, Vec<(String, String)>) {
    let mut out = Vec::with_capacity(found.len());
    let mut notes = Vec::new();
    let mut start = 0;
    while start < found.len() {
        let sheet = &found[start].sheet;
        let end = found[start..]
            .iter()
            .position(|p| &p.sheet != sheet)
            .map_or(found.len(), |n| start + n);
        let group = &found[start..end];
        start = end;
        let upper = sheet.to_uppercase();
        let facts = sheets
            .iter()
            .find(|s| &s.name == sheet)
            .or_else(|| sheets.iter().find(|s| s.name.to_uppercase() == upper));
        let wanted: HashSet<(u32, u32)> = group.iter().map(|p| (p.at.row, p.at.col)).collect();
        let labels = facts
            .map(|f| labels_of(&f.cells, &wanted))
            .unwrap_or_default();
        let empty = SheetPaths::default();
        let regions = facts.map_or(&empty, |f| &f.paths);
        let points: Vec<(u32, u32)> = group.iter().map(|p| (p.at.row, p.at.col)).collect();
        let in_data = regions.locate(&points, false);
        let in_region = regions.locate(&points, true);
        let q = quote_sheet(sheet);
        for ((p, data), whole) in group.iter().zip(in_data).zip(in_region) {
            let (row, col) = (p.at.row, p.at.col);
            let anchor = format!("{q}!{}", crate::grid::a1(row, col));
            let cell = facts.and_then(|f| {
                f.cells
                    .binary_search_by_key(&(row, col), |c| (c.row, c.col))
                    .ok()
                    .map(|i| &f.cells[i])
            });
            let code = facts
                .and_then(|f| f.styles.get(&(row, col)))
                .and_then(|s| formats(*s));
            let value = cell.and_then(|c| cell_value(&c.val, code));
            // Units and column headers describe quantities. Above a text value the text is
            // as likely the value before it (`g`, `sg`, `lbm`) as a header.
            let quantity = matches!(value, Some(Value::Number { .. } | Value::Percent { .. }));
            let mut around = labels.get(&(row, col)).copied().unwrap_or_default();
            if !quantity {
                around.beside = None;
                around.header = None;
            }
            // A row value on the left (a category) says less than the column header. A label
            // with a unit in brackets, like `Load [kN]`, names a quantity, so it is never a
            // row value; a bare unit or a size like `DN 50 mm` can be one.
            if around.left_is_value
                && around.header.is_some()
                && !around.left.is_some_and(units::is_bracketed_label)
            {
                around.left = None;
            }
            let (left_label, left_unit) = around.left.map(units::split_label).unwrap_or_default();
            let (head_label, head_unit) = around.header.map(units::split_label).unwrap_or_default();
            let key = (upper.clone(), row, col);
            let mut found_units: Vec<(String, &str)> = Vec::new();
            if quantity {
                found_units.extend(left_unit.map(|u| (u, "label")));
                found_units.extend(
                    around
                        .beside
                        .map(str::trim)
                        .filter(|t| units::is_unit(t, true))
                        .map(|u| (u.to_string(), "unit-cell")),
                );
                found_units.extend(
                    code.and_then(units::from_number_format)
                        .map(|u| (u, "number-format")),
                );
                found_units.extend(convert_units.get(&key).map(|u| (u.clone(), "convert")));
                found_units.extend(head_unit.map(|u| (u, "header")));
            }
            let (unit, unit_source) = match found_units.split_first() {
                None => (None, None),
                Some(((u, s), rest)) if rest.iter().all(|(r, _)| r == u) => {
                    (Some(u.clone()), Some(s.to_string()))
                }
                Some(_) => {
                    let named: Vec<String> = found_units
                        .iter()
                        .map(|(u, s)| format!("{u} ({s})"))
                        .collect();
                    notes.push((
                        anchor.clone(),
                        format!("units disagree: {}; no unit set", named.join(", ")),
                    ));
                    (None, None)
                }
            };
            let label = left_label.clone().or(head_label);
            // A cell of a caption, header or note row stands under the region's headings
            // with the label found beside or above it.
            let mut path = match (data, whole) {
                (Some(k), _) => region_path(regions, k, (row, col), &around, &left_label, &label),
                (None, Some(k)) => with_base(regions, k, &around, &left_label, label.clone()),
                (None, None) => Vec::new(),
            };
            if path.is_empty() {
                path.extend(label.clone());
            }
            let bytes: usize = path.iter().map(|t| t.len() + 3).sum();
            if path.len() > 1 && !paths.take(bytes) {
                path = label.into_iter().collect();
            }
            out.push(Parameter {
                anchor: Anchor(anchor),
                name: named_cells.get(&key).cloned(),
                label: (!path.is_empty()).then(|| path.join(" / ")),
                value,
                unit,
                unit_source,
                read_by: p.read_by,
                path,
            });
        }
    }
    (out, notes)
}

/// The path of a parameter at `at` in region `k` (spec 8.7): the region's base, the parts
/// of its column's header path and the label of its row. A row label the label rule took as
/// this value's label (`left_label`) is written as that label, without its unit; a text it
/// took as a value of the row (a category, a size) stands as it is. Outside the label
/// columns, a row without a label there is named by the label on the left. When neither
/// names the cell, `label` (the one found beside or above) does; empty when there is none.
fn region_path(
    regions: &SheetPaths,
    k: usize,
    at: (u32, u32),
    around: &Around,
    left_label: &Option<String>,
    label: &Option<String>,
) -> Vec<String> {
    let (row, col) = at;
    let (mut narrow, row_labels) = regions.narrow_parts(k, row, col);
    let left_text = around.left.map(str::trim);
    if row_labels.is_empty() && !regions.in_label_cols(k, col) {
        narrow.extend(left_label.clone());
    }
    for t in row_labels {
        if Some(t) == left_text {
            narrow.extend(left_label.clone());
        } else {
            narrow.push(t.to_string());
        }
    }
    if narrow.is_empty() {
        narrow.extend(label.clone());
    }
    with_base(regions, k, around, left_label, narrow)
}

/// `narrow` under the base of region `k`; empty when `narrow` is. The text of the label
/// found beside or above is not repeated when it is a heading the region stands under
/// (`t (m)` as a caption over its values).
fn with_base(
    regions: &SheetPaths,
    k: usize,
    around: &Around,
    left_label: &Option<String>,
    narrow: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut narrow = narrow.into_iter().peekable();
    if narrow.peek().is_none() {
        return Vec::new();
    }
    let source = if left_label.is_some() {
        around.left
    } else {
        around.header
    };
    let mut path: Vec<String> = regions.regions[k]
        .base
        .iter()
        .filter(|b| source.map(str::trim) != Some(b.as_str()))
        .cloned()
        .collect();
    path.extend(narrow);
    path
}

/// Parameters for defined names whose whole text is a number; read by the cell formulas
/// that name them.
fn name_parameters(facet: &[Formula], literal: Vec<(String, String, Value)>) -> Vec<Parameter> {
    if literal.is_empty() {
        return Vec::new();
    }
    // A formula may write the name in another case than its definition.
    let mut counts: HashMap<String, u32> = literal
        .iter()
        .map(|(a, _, _)| (a.to_uppercase(), 0))
        .collect();
    for f in facet {
        if matches!(f.kind, Some(FormulaKind::Name | FormulaKind::Lambda)) {
            continue;
        }
        for r in &f.references {
            if r.0.starts_with("name:")
                && let Some(n) = counts.get_mut(&r.0.to_uppercase())
            {
                *n = n.saturating_add(1);
            }
        }
    }
    literal
        .iter()
        .map(|(anchor, name, value)| Parameter {
            anchor: Anchor(anchor.clone()),
            name: Some(name.clone()),
            label: None,
            value: Some(value.clone()),
            unit: None,
            unit_source: None,
            read_by: counts.get(&anchor.to_uppercase()).copied().unwrap_or(0),
            path: Vec::new(),
        })
        .collect()
}

/// The group-level dependency graph. Node anchors are set where they resolve: groups,
/// formulas, parameter blocks and other workbooks always, names and tables when the
/// workbook defines them.
fn dependency_graph(
    out: &mut Out,
    graph: &GroupGraph,
    tables: &[TableLoc],
    budget: &Budget,
) -> Option<Graph> {
    let size = (graph.nodes.len() + graph.edges.len()) as u64;
    if let Err(hit) = budget.add_elements(size) {
        out.loss(
            "document",
            format!(
                "{hit}: the dependency graph ({} nodes, {} edges) not listed",
                graph.nodes.len(),
                graph.edges.len()
            ),
            LossReason::Limit,
            Severity::Warning,
        );
        return None;
    }
    let names: HashMap<String, &str> = out
        .doc
        .facets
        .formulas
        .iter()
        .filter(|f| matches!(f.kind, Some(FormulaKind::Name | FormulaKind::Lambda)))
        .map(|f| (f.anchor.0.to_uppercase(), f.anchor.0.as_str()))
        .collect();
    let nodes = graph
        .nodes
        .iter()
        .map(|(id, label, kind)| {
            let anchor = match kind.as_str() {
                "name" => match label.strip_prefix("table:") {
                    Some(t) => tables
                        .iter()
                        .find(|x| x.name.to_uppercase() == t.to_uppercase())
                        .map(|x| format!("table:{}", x.name)),
                    None => names.get(&label.to_uppercase()).map(|a| a.to_string()),
                },
                _ => Some(label.clone()),
            };
            Node {
                id: id.clone(),
                label: label.clone(),
                kind: Some(kind.clone()),
                anchor: anchor.map(Anchor),
            }
        })
        .collect();
    let edges = graph
        .edges
        .iter()
        .map(|(from, to)| Edge {
            from: from.clone(),
            to: to.clone(),
            kind: EDGE_KIND.to_string(),
            anchor: None,
        })
        .collect();
    Some(Graph {
        id: "dependencies".into(),
        kind: "dependency".into(),
        nodes,
        edges,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit_symbols_make_a_unit_column() {
        // Plain symbols without a digit or a sign.
        assert!(is_unit_column_texts(&["m", "m", "-", "kPa", "kPa"]));
        assert!(is_unit_column_texts(&["m2", "m"]));
        assert!(is_unit_column_texts(&["l/s", "l/s", "l/s"]));
        assert!(is_unit_column_texts(&["m/s", "m/s"]));
        // Quotients of symbols, beside a text that is not a unit.
        assert!(is_unit_column_texts(&[
            "veh/hr", "veh/hr", "g/s", "g/m3", "m3/s", "m/s"
        ]));
        // Count words beside a real symbol (the tunnel jet fan block).
        assert!(is_unit_column_texts(&["Pa", "N", "nos"]));
        assert!(is_unit_column_texts(&["kg", "pcs"]));
        assert!(is_unit_column_texts(&["m", "ea"]));
        // Words, codes and a single symbol are not a unit column.
        assert!(!is_unit_column_texts(&["m"]));
        assert!(!is_unit_column_texts(&["OK", "N/A", "yes"]));
        assert!(!is_unit_column_texts(&["N/A", "N/A"]));
        assert!(!is_unit_column_texts(&["A/C", "S/N"]));
        assert!(!is_unit_column_texts(&["A", "B", "C"]));
        assert!(!is_unit_column_texts(&["red", "blue", "green"]));
        // A class column, a Y/N column, a gender column (m and d) and a yes/no column are
        // data, not units.
        assert!(!is_unit_column_texts(&["A", "A", "A"]));
        assert!(!is_unit_column_texts(&["N", "N", "N"]));
        assert!(!is_unit_column_texts(&["m", "d", "m", "d"]));
        assert!(!is_unit_column_texts(&["no", "no"]));
        assert!(!is_unit_column_texts(&["yes", "no", "no"]));
    }

    use rezzer_limits::Limits;
    use rezzer_model::{Confidence, Detection, Document, FormatId, SourceInfo};

    use super::*;

    fn out() -> Out {
        Out::new(Document::new(SourceInfo {
            path: None,
            name: "t.xlsx".into(),
            sha256: String::new(),
            size: 0,
            format: FormatId::Xlsx,
            detection: Detection {
                confidence: Confidence::Certain,
                evidence: "test".into(),
            },
            modified: None,
            encoding: None,
            reader: None,
        }))
    }

    /// A1:A3 values and B1:B3 copies of `=A1*2`.
    fn copies(out: &mut Out) -> Formulas {
        let mut formulas = Formulas::new(&[], u64::MAX);
        for row in 0..3 {
            formulas.add_constant("S", row, 0);
            let index = out.doc.facets.formulas.len();
            let (f, problem) = formulas.cell(
                "S",
                row,
                1,
                &format!("A{}*2", row + 1),
                FormulaKind::Cell,
                None,
                Some(Value::number(2.0, None)),
                index,
            );
            assert!(problem.is_none());
            out.doc.facets.formulas.push(f);
        }
        formulas
    }

    #[test]
    fn stopped_logic_keeps_patterns_and_says_so() {
        let mut out = out();
        let formulas = copies(&mut out);
        let budget = Budget::new(Limits::default());
        budget.cancel();
        let state = ReadState {
            calc: None,
            formats: &|_| None,
            incomplete: false,
        };
        workbook_logic(&mut out, formulas, Vec::new(), state, &budget);
        let doc = &out.doc;
        for f in &doc.facets.formulas {
            assert_eq!(f.pattern.as_deref(), Some("p1"));
            assert_eq!(f.range.as_ref().map(|a| a.0.as_str()), Some("S!B1:B3"));
            assert_eq!(f.role, None);
        }
        assert!(doc.facets.parameters.is_empty());
        assert!(doc.facets.graphs.is_empty());
        assert!(
            doc.losses.iter().any(|l| l.why == LossReason::Limit
                && l.what
                    .starts_with("time limit reached while the workbook logic")),
            "{:#?}",
            doc.losses
        );
        assert!(out.partial);
    }

    #[test]
    fn label_callback_is_the_label_rule() {
        assert!(is_label("Span"));
        assert!(is_label("  Span  "));
        assert!(!is_label("   "));
        assert!(!is_label(""));
        assert!(is_label(&"a".repeat(MAX_LABEL)));
        assert!(!is_label(&"a".repeat(MAX_LABEL + 1)));
    }

    #[test]
    fn unit_column_callback_reads_units() {
        assert!(is_unit_column_texts(&["Unit", "whatever"]));
        assert!(is_unit_column_texts(&[" kN/m3 ", "m2", "mm"]));
        assert!(!is_unit_column_texts(&["Alice", "Bob", "Carol"]));
        assert!(!is_unit_column_texts(&[]));
    }

    #[test]
    fn layout_facts_name_functions_and_same_sheet_reads() {
        let mut out = out();
        let mut formulas = Formulas::new(&[], u64::MAX);
        let texts = [
            (4, "SUBTOTAL(9,B2:B4)"),
            (5, "SUM(B2:B5)+Data!B2+'S'!C1"),
            (6, "SUM(Data!B2:B5)"),
            (7, "1+"),
        ];
        for (row, text) in texts {
            let index = out.doc.facets.formulas.len();
            let (f, _) = formulas.cell("S", row, 1, text, FormulaKind::Cell, None, None, index);
            out.doc.facets.formulas.push(f);
        }
        let (f, _) = formulas.cell("Data", 0, 0, "SUM(A2:A3)", FormulaKind::Cell, None, None, 9);
        out.doc.facets.formulas.push(f);
        let rect = |top, left, bottom, right| sheetlayout::Rect {
            top,
            left,
            bottom,
            right,
        };
        let facts = formulas.layout_facts("S");
        assert_eq!(facts.len(), 4, "{facts:?}");
        assert_eq!(facts[&(4, 1)].functions, ["SUBTOTAL"]);
        assert_eq!(facts[&(4, 1)].reads, [rect(1, 1, 3, 1)]);
        assert_eq!(facts[&(5, 1)].functions, ["SUM"]);
        assert_eq!(facts[&(5, 1)].reads, [rect(1, 1, 4, 1), rect(0, 2, 0, 2)]);
        // A read on another sheet never marks a row of this one.
        assert!(facts[&(6, 1)].reads.is_empty());
        // A formula that does not parse still counts as a formula.
        assert_eq!(facts[&(7, 1)], sheetlayout::FormulaIn::default());
        // Handed out once per sheet.
        assert!(formulas.layout_facts("S").is_empty());
        assert_eq!(formulas.layout_facts("data").len(), 1);
    }

    #[test]
    fn convert_calls_and_literals() {
        let calls = calls_of(
            "=A1+convert(B1,\"m\",\"ft\")*CONVERT(C1,\"g\",\"kg\")",
            "CONVERT",
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ["B1", "\"m\"", "\"ft\""]);
        assert!(calls_of("=\"CONVERT(B1,1,2)\"&MYCONVERT(B1,1,2)", "CONVERT").is_empty());
        assert_eq!(text_literal("\"a\"\"b\""), Some("a\"b".into()));
        assert_eq!(text_literal("B1"), None);
        assert_eq!(literal_number("0.05"), Some(Value::number(0.05, None)));
        assert_eq!(literal_number("=-3"), Some(Value::number(-3.0, None)));
        assert_eq!(literal_number("5%"), Some(Value::percent(0.05)));
        assert_eq!(literal_number("1.5E-3"), Some(Value::number(0.0015, None)));
        for t in ["inf", "NaN", "A1", "1+2", "\"5\"", ""] {
            assert_eq!(literal_number(t), None, "{t}");
        }
    }
}
