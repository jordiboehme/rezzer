// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Formulas of cells and defined names, read through formulakit, plus the inputs of the
//! workbook logic index (formulas, constants, names and tables).

use std::collections::{HashMap, HashSet};

use formulakit::logic::{FormulaId, TableArea, Workbook};
use formulakit::{Analysis, CellPos, Dialect, Ref, anchor_of, quote_sheet};
use rezzer_model::{Anchor, Formula, FormulaKind, Value};
use sheetparts::DefinedName;

/// Excel's own maximum length of a formula, in characters. Longer texts are refused.
pub(crate) const MAX_FORMULA_CHARS: usize = 8_192;

/// Distinct formula texts whose LaTeX is remembered. The cache is cleared when full.
const LATEX_CACHE: usize = 65_536;

/// What the logic pass of the workbook needs: every formula, constant cell, defined name
/// and table, fed to formulakit's index in workbook order, plus which model formula each
/// index entry is. Built here and kept for the logic pass, which runs `finish` on it.
#[allow(
    dead_code,
    reason = "read by the workbook logic pass (roles, patterns, graph)"
)]
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
        }
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

    /// The model formula of one cell. `text` is the stored text without `=`; `index` is the
    /// position the formula will take in `Facets.formulas`.
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
        let stored = format!("={text}");
        let mut formula = Formula {
            anchor: Anchor(anchor.clone()),
            expression: stored.clone(),
            dialect: "excel".into(),
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
            match formulakit::analyze(text, Dialect::Excel, CellPos { row, col }) {
                Ok(mut a) => {
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
                    let id = self
                        .inputs
                        .workbook
                        .add_formula(sheet, CellPos { row, col }, &a);
                    self.inputs.formula_ids.push((index, id));
                }
                Err(e) => {
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
        // The logic index sees the inputs the table reads.
        if let Ok(a) = formulakit::analyze(&expression, Dialect::Excel, CellPos { row, col }) {
            let id = self
                .inputs
                .workbook
                .add_formula(sheet, CellPos { row, col }, &a);
            self.inputs.formula_ids.push((index, id));
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
        let mut formula = Formula {
            anchor: Anchor(anchor.clone()),
            expression: n.refers_to.clone(),
            dialect: "excel".into(),
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
                    self.inputs
                        .workbook
                        .add_name(&n.name, n.scope.as_deref(), Some(&a));
                }
                Err(e) => {
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
    let mut depth = 0usize;
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars();
    let mut closed = false;
    while let Some(c) = chars.next() {
        if closed {
            if c.is_whitespace() {
                continue;
            }
            return None;
        }
        match c {
            '"' | '\'' => {
                current.push(c);
                // Quoted text or sheet name; a doubled quote stays inside.
                for d in chars.by_ref() {
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
                closed = true;
            }
            ',' if depth == 0 => args.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    closed.then_some(args)
}
