// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Spreadsheet formula analysis.
//!
//! [`analyze`] tells which cells, ranges, names, tables and external workbooks an Excel or
//! OpenFormula formula reads, which functions it calls (and which of them are not built in),
//! whether it is volatile, a display form in Excel A1 syntax and an R1C1 pattern key. Copies
//! of one formula share the pattern key, so a sheet of filled-down formulas folds into a few
//! patterns. [`anchor_of`] gives a stable text anchor for each reference.
//!
//! [`logic::Workbook`] puts the formulas of a workbook together: roles, pattern groups,
//! parameter cells and a dependency graph on group level. [`latex`] writes a formula as
//! KaTeX-ready math, and [`worth_math`] tells whether that is worth showing.
//!
//! ```
//! use formulakit::{CellPos, Dialect, analyze, anchor_of};
//!
//! let a = analyze("=SUM(Data!B2:B9)*rate", Dialect::Excel, CellPos { row: 0, col: 3 }).unwrap();
//! let anchors: Vec<String> = a.refs.iter().map(|r| anchor_of("Main", r)).collect();
//! assert_eq!(anchors, ["Data!B2:B9", "name:rate"]);
//! assert_eq!(a.pattern_key, "=SUM(Data!R[1]C[-2]:R[8]C[-2])*rate");
//! ```
//!
//! formulakit does not calculate.

mod engine;
mod functions;
mod latex;
pub mod logic;
mod openformula;
mod pattern;
mod refs;

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::ops::Range;

use engine::{Node, Target};
pub use latex::{latex, worth_math};
pub use refs::{
    CellPos, MAX_COL, MAX_ROW, Ref, SheetSpan, TableRows, Whole, anchor_of, quote_sheet,
};

/// The syntax a formula is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// Excel A1 syntax as stored in XLSX (`_xlfn.` prefixes allowed).
    Excel,
    /// OpenFormula as stored in ODF files (`of:=[.A1]`). A `msoxl:` prefix switches to Excel.
    OpenFormula,
}

/// A formula that could not be read. The caller keeps the formula text and shows this note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "formula not readable: {}", self.message)
    }
}

impl std::error::Error for ParseError {}

/// What a formula reads and does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    /// The formula in Excel A1 syntax with a leading `=`, without the storage prefixes
    /// `_xlfn.`, `_xlws.`, `_xludf.` and `_xlpm.`; OpenFormula converted to A1.
    pub display: String,
    /// Everything the formula reads, each distinct reference once, in order of first
    /// appearance. `A1` and `$A$1` differ in their `$` flags and are both listed (same anchor).
    /// LET and LAMBDA parameters are not in here.
    pub refs: Vec<Ref>,
    /// The functions called, each once. Built-in names are uppercase; user functions are
    /// written as in the formula. Calls of a LET-bound lambda by its local name are not listed.
    pub functions: Vec<String>,
    /// Functions that are not built in: VBA or add-in functions, `[1]!name` from another
    /// workbook, names of lambdas defined in the workbook.
    pub user_functions: Vec<String>,
    /// Whether the formula calls NOW, TODAY, RAND, RANDBETWEEN, RANDARRAY, OFFSET, INDIRECT,
    /// CELL or INFO.
    pub volatile: bool,
    /// LET and LAMBDA parameter names, without `_xlpm.`.
    pub lambda_params: Vec<String>,
    /// `display` with cell and range references in R1C1 form relative to the formula cell, so
    /// copies of one formula share it.
    pub pattern_key: String,
}

/// What one edit puts in place of the original text.
enum Repl {
    Keep,
    /// Drop this many leading bytes (a storage prefix such as `_xlfn.`).
    DropPrefix(usize),
    Text(String),
}

/// One text replacement on the Excel text, for the display form and the pattern key.
struct Edit {
    span: Range<usize>,
    display: Repl,
    key: Repl,
}

#[derive(Default)]
struct Walk {
    at: CellPos,
    refs: Vec<Ref>,
    seen: HashSet<Ref>,
    functions: Vec<String>,
    user_functions: Vec<String>,
    volatile: bool,
    lambda_params: Vec<String>,
    edits: Vec<Edit>,
    /// LET and LAMBDA parameters in scope, uppercase, innermost last.
    scope: Vec<String>,
}

/// Adds `item` unless the list has it already in any letter case (function and parameter
/// names are case-insensitive); the first spelling stays.
fn push_unique(list: &mut Vec<String>, item: &str) {
    let upper = item.to_uppercase();
    if !list.iter().any(|x| x.to_uppercase() == upper) {
        list.push(item.to_string());
    }
}

impl Walk {
    fn add_ref(&mut self, r: Ref) {
        if self.seen.insert(r.clone()) {
            self.refs.push(r);
        }
    }

    /// The parameter name if `raw` is a LET or LAMBDA parameter here.
    fn param<'a>(&self, raw: &'a str) -> Option<&'a str> {
        let (bare, _) = functions::strip_prefixes(raw);
        let tagged = raw
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case("_xlpm."));
        (tagged || self.scope.contains(&bare.to_uppercase())).then_some(bare)
    }

    /// Records a parameter name at a binding position and returns it for the scope.
    fn bind(&mut self, node: &Node) -> Option<String> {
        let Node::Ref {
            span,
            target: Target::Name(raw),
        } = node
        else {
            return None;
        };
        let (bare, _) = functions::strip_prefixes(raw);
        if bare.is_empty() || bare.contains('!') {
            return None;
        }
        push_unique(&mut self.lambda_params, bare);
        self.edits.push(Edit {
            span: span.clone(),
            display: Repl::Text(bare.to_string()),
            key: Repl::Text(bare.to_string()),
        });
        Some(bare.to_uppercase())
    }

    fn visit(&mut self, node: &Node) {
        match node {
            Node::Literal(_) | Node::Empty => {}
            Node::Unary { expr, .. } => self.visit(expr),
            Node::Binary { left, right, .. } => {
                self.visit(left);
                self.visit(right);
            }
            Node::Call { callee, args } => {
                self.visit(callee);
                args.iter().for_each(|a| self.visit(a));
            }
            Node::Array(rows) => rows.iter().flatten().for_each(|c| self.visit(c)),
            Node::Ref { span, target } => self.visit_ref(span, target),
            Node::Area { parts, folded } => {
                for (span, part) in parts {
                    self.key_edit(span, part);
                }
                self.add_ref(folded.clone());
            }
            Node::Func { name, span, args } => {
                let (bare, udf) = functions::strip_prefixes(name);
                let upper = bare.to_uppercase();
                // The prefixes are ASCII and come first, so their byte length is the difference.
                let prefix = name.len().saturating_sub(bare.len());
                self.edits.push(Edit {
                    span: span.clone(),
                    display: Repl::DropPrefix(prefix),
                    key: Repl::DropPrefix(prefix),
                });
                let local = self.scope.contains(&upper);
                if !local {
                    let builtin = !udf && functions::is_builtin(&upper);
                    if builtin {
                        push_unique(&mut self.functions, &upper);
                        self.volatile |= functions::is_volatile(&upper);
                    } else {
                        push_unique(&mut self.functions, bare);
                        push_unique(&mut self.user_functions, bare);
                    }
                }
                match upper.as_str() {
                    "LET" if !local => self.visit_let(args),
                    "LAMBDA" if !local => self.visit_lambda(args),
                    _ => args.iter().for_each(|a| self.visit(a)),
                }
            }
        }
    }

    /// LET(name1, value1, name2, value2, ..., body): each name is in scope after its value.
    fn visit_let(&mut self, args: &[Node]) {
        let depth = self.scope.len();
        let last = args.len().saturating_sub(1);
        let mut i = 0;
        while i < last {
            let value = args.get(i + 1).filter(|_| i + 1 < last);
            let bound = args.get(i).and_then(|n| self.bind(n));
            if bound.is_none()
                && let Some(n) = args.get(i)
            {
                self.visit(n);
            }
            if let Some(v) = value {
                self.visit(v);
            }
            if let Some(name) = bound {
                self.scope.push(name);
            }
            i += 2;
        }
        if let Some(body) = args.get(last) {
            self.visit(body);
        }
        self.scope.truncate(depth);
    }

    /// LAMBDA(param1, ..., body).
    fn visit_lambda(&mut self, args: &[Node]) {
        let depth = self.scope.len();
        let last = args.len().saturating_sub(1);
        for p in args.get(..last).unwrap_or(&[]) {
            match self.bind(p) {
                Some(name) => self.scope.push(name),
                None => self.visit(p),
            }
        }
        if let Some(body) = args.get(last) {
            self.visit(body);
        }
        self.scope.truncate(depth);
    }

    fn visit_ref(&mut self, span: &Range<usize>, target: &Target) {
        match target {
            Target::Name(raw) => {
                if let Some(bare) = self.param(raw) {
                    self.edits.push(Edit {
                        span: span.clone(),
                        display: Repl::Text(bare.to_string()),
                        key: Repl::Text(bare.to_string()),
                    });
                } else {
                    self.add_ref(refs::name_ref(raw));
                }
            }
            Target::Ref(r) => {
                self.key_edit(span, r);
                self.add_ref(r.clone());
            }
        }
    }

    /// Writes a cell or range reference in R1C1 form in the pattern key.
    fn key_edit(&mut self, span: &Range<usize>, r: &Ref) {
        if let Some(key) = pattern::r1c1(r, self.at) {
            self.edits.push(Edit {
                span: span.clone(),
                display: Repl::Keep,
                key: Repl::Text(key),
            });
        }
    }
}

fn apply(out: &mut String, original: &str, repl: &Repl) {
    match repl {
        Repl::Keep => out.push_str(original),
        Repl::DropPrefix(n) => out.push_str(original.get(*n..).unwrap_or(original)),
        Repl::Text(t) => out.push_str(t),
    }
}

/// Applies the edits (sorted, not overlapping) to `text`: the display form and the key.
fn splice(text: &str, edits: &[Edit]) -> (String, String) {
    let mut display = String::with_capacity(text.len());
    let mut key = String::with_capacity(text.len());
    let mut pos = 0;
    for edit in edits {
        let (Some(gap), Some(original)) = (
            text.get(pos..edit.span.start),
            text.get(edit.span.start..edit.span.end),
        ) else {
            continue;
        };
        display.push_str(gap);
        key.push_str(gap);
        apply(&mut display, original, &edit.display);
        apply(&mut key, original, &edit.key);
        pos = edit.span.end;
    }
    let tail = text.get(pos..).unwrap_or("");
    display.push_str(tail);
    key.push_str(tail);
    (display, key)
}

/// Splits a namespace prefix (`of:`, `msoxl:`) from ODF formula text.
fn namespace(text: &str, dialect: Dialect) -> Result<(&str, Dialect), ParseError> {
    let letters = text.chars().take_while(char::is_ascii_lowercase).count();
    match (text.get(..letters), text.get(letters..)) {
        (Some(ns), Some(rest)) if letters > 0 && rest.starts_with(":=") => {
            let rest = rest.get(1..).unwrap_or("");
            match ns {
                "of" | "oooc" => Ok((rest, Dialect::OpenFormula)),
                "msoxl" => Ok((rest, Dialect::Excel)),
                other => Err(ParseError {
                    message: format!("unknown formula namespace {other}:"),
                }),
            }
        }
        _ => Ok((text, dialect)),
    }
}

/// Formula text as Excel A1 text with a leading `=`: the namespace prefix removed and
/// OpenFormula converted.
pub(crate) fn excel_text(text: &str, dialect: Dialect) -> Result<Cow<'_, str>, ParseError> {
    let (body, dialect) = namespace(text.trim_start(), dialect)?;
    let body: Cow<'_, str> = if body.starts_with('=') {
        Cow::Borrowed(body)
    } else {
        Cow::Owned(format!("={body}"))
    };
    match dialect {
        Dialect::Excel => Ok(body),
        Dialect::OpenFormula => openformula::to_excel(&body).map(Cow::Owned),
    }
}

/// Analyzes one formula in the cell `at` (0-based; relative references in the pattern key are
/// offsets from it). The text may come with or without its leading `=` and, for ODF, with its
/// namespace prefix (`of:=`). A formula that cannot be read returns a [`ParseError`]; this
/// function never panics on any input.
pub fn analyze(text: &str, dialect: Dialect, at: CellPos) -> Result<Analysis, ParseError> {
    let excel = excel_text(text, dialect)?;
    let excel = excel.as_ref();
    let tree = engine::parse(excel)?;
    let mut walk = Walk {
        at,
        ..Walk::default()
    };
    walk.visit(&tree);
    walk.edits.sort_by_key(|e| e.span.start);
    let (display, pattern_key) = splice(excel, &walk.edits);
    Ok(Analysis {
        display,
        refs: walk.refs,
        functions: walk.functions,
        user_functions: walk.user_functions,
        volatile: walk.volatile,
        lambda_params: walk.lambda_params,
        pattern_key,
    })
}
