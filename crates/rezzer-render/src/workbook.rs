// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `## Logic` for a workbook (XLSX, XLSM, XLSB, ODS, XLS): one `### <sheet>` part per sheet
//! in sheet order with its parameters, formulas (copies folded into one row per pattern),
//! results and rules; then `### Names`, `### Dependencies` and `### Hidden logic`.
//!
//! Inside a sheet's part, cells of that sheet are written without the sheet name; cells of
//! other sheets keep it. Anchors that belong to no sheet of the body go to `### Other`.
//! Every pass over the formulas is linear, so a workbook with millions of formulas renders
//! in one sweep.

use std::collections::HashMap;

use rezzer_model::{
    Block, CalloutKind, Document, FormatId, Formula, FormulaKind, Graph, Inline, Node, Parameter,
    Role, Rule, SectionKind, Value, column_letter,
};

use crate::blocks::{self, Ctx, Mode, cell_text, code_span, heading_line, line_text, pipe_table};
use crate::facets::{role_name, value_text};
use crate::mermaid::{self, MAX_EDGES, MAX_NODES};

/// A `Reads` cell names at most this many references, then how many more.
const MAX_READS: usize = 20;

/// `### Hidden logic` lists at most this many data tables, spill ranges or array formulas
/// each; a line says how many more there are.
const MAX_AREAS: usize = 200;

/// A volatile function row names at most this many cells.
const MAX_VOLATILE_CELLS: usize = 5;

/// The start of the loss the sheet reader writes when cached results may not match.
const STALE: &str = "stored results may be old";

/// Functions whose result can change on every recalculation (as formulakit lists them).
const VOLATILE: [&str; 9] = [
    "CELL",
    "INDIRECT",
    "INFO",
    "NOW",
    "OFFSET",
    "RAND",
    "RANDARRAY",
    "RANDBETWEEN",
    "TODAY",
];

/// True for the formats whose logic is laid out per sheet.
pub(crate) fn is_workbook(doc: &Document) -> bool {
    matches!(
        doc.source.format,
        FormatId::Xlsx | FormatId::Xlsm | FormatId::Xlsb | FormatId::Ods | FormatId::Xls
    )
}

/// The sheet part of an anchor and the rest after its `!`: `'My ''x'''!A1` gives
/// (`'My ''x'''`, `A1`). `None` for anchors without a sheet (`name:`, `ext:`, `table:`).
pub(crate) fn split_sheet(anchor: &str) -> Option<(&str, &str)> {
    if anchor.starts_with("name:") || anchor.starts_with("ext:") || anchor.starts_with("table:") {
        return None;
    }
    if let Some(inner) = anchor.strip_prefix('\'') {
        let mut chars = inner.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c != '\'' {
                continue;
            }
            if chars.peek().map(|&(_, n)| n) == Some('\'') {
                chars.next();
                continue;
            }
            let end = i + 2;
            return anchor[end..]
                .strip_prefix('!')
                .map(|rest| (&anchor[..end], rest));
        }
        return None;
    }
    let (sheet, rest) = anchor.split_once('!')?;
    (!sheet.is_empty()).then_some((sheet, rest))
}

/// A sheet name without its quotes: `'It''s'` gives `It's`.
fn unquote(sheet: &str) -> String {
    match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => sheet.to_string(),
    }
}

/// The sheets of the body in order, as (title, anchor), and a map from anchor to index.
struct Sheets<'a> {
    list: Vec<(String, &'a str)>,
    index: HashMap<&'a str, usize>,
}

impl<'a> Sheets<'a> {
    fn of(body: &'a [Block]) -> Sheets<'a> {
        let mut list = Vec::new();
        let mut index = HashMap::new();
        for b in body {
            if let Block::Section {
                kind: SectionKind::Sheet,
                title,
                anchor: Some(anchor),
                ..
            } = b
            {
                if index.contains_key(anchor.0.as_str()) {
                    continue;
                }
                let title = title
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map_or_else(|| unquote(&anchor.0), str::to_string);
                index.insert(anchor.0.as_str(), list.len());
                list.push((title, anchor.0.as_str()));
            }
        }
        Sheets { list, index }
    }

    /// The index of the sheet an anchor lies on.
    fn of_anchor(&self, anchor: &str) -> Option<usize> {
        split_sheet(anchor).and_then(|(s, _)| self.index.get(s).copied())
    }

    /// A readable sheet name for a quoted sheet part.
    fn title(&self, sheet: &str) -> String {
        match self.index.get(sheet) {
            Some(&i) => self.list[i].0.clone(),
            None => unquote(sheet),
        }
    }
}

/// An anchor as written inside the part of `sheet`: without the sheet when it is that sheet.
fn local<'b>(anchor: &'b str, sheet: Option<&str>) -> &'b str {
    match (split_sheet(anchor), sheet) {
        (Some((s, rest)), Some(own)) if s == own => rest,
        _ => anchor,
    }
}

fn is_name(f: &Formula) -> bool {
    matches!(f.kind, Some(FormulaKind::Name | FormulaKind::Lambda))
        || f.anchor.0.starts_with("name:")
}

/// One row of the Formulas table: a formula of its own or a pattern group.
struct Entry<'a> {
    members: Vec<&'a Formula>,
}

impl<'a> Entry<'a> {
    fn first(&self) -> &'a Formula {
        self.members[0]
    }

    fn has_role(&self, role: Role) -> bool {
        self.members.iter().any(|f| f.role == Some(role))
    }
}

/// The formulas in order of first appearance, members of one pattern folded into one entry.
fn entries<'a>(list: &[&'a Formula]) -> Vec<Entry<'a>> {
    let mut out: Vec<Entry<'a>> = Vec::new();
    let mut by_pattern: HashMap<&str, usize> = HashMap::new();
    for &f in list {
        match f.pattern.as_deref() {
            Some(p) => match by_pattern.get(p) {
                Some(&i) => out[i].members.push(f),
                None => {
                    by_pattern.insert(p, out.len());
                    out.push(Entry { members: vec![f] });
                }
            },
            None => out.push(Entry { members: vec![f] }),
        }
    }
    out
}

/// A cell or range part (`$B$2`, `B2:C9`) as 0-based (column, row) corners.
fn corners(rest: &str) -> Option<((u32, u32), (u32, u32))> {
    let cell = |s: &str| -> Option<(u32, u32)> {
        let s = s.replace('$', "");
        let letters = s.chars().take_while(char::is_ascii_alphabetic).count();
        if letters == 0 || letters > 3 || letters == s.len() {
            return None;
        }
        let mut col: u32 = 0;
        for c in s[..letters].chars() {
            col = col * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
        }
        let row: u32 = s[letters..].parse().ok()?;
        (row >= 1).then(|| (col - 1, row - 1))
    };
    match rest.split_once(':') {
        Some((a, b)) => Some((cell(a)?, cell(b)?)),
        None => cell(rest).map(|c| (c, c)),
    }
}

/// The area a row covers. A formula of its own: the area it fills, else its cell. A pattern:
/// the area all members share, else the group's node in the dependency graph, else the box
/// around its members.
fn area(entry: &Entry<'_>, groups: &HashMap<&str, &str>) -> Vec<String> {
    let found = group_area(entry, groups);
    let n = entry.members.len();
    if n > 1 && n <= MAX_LISTED && cell_count(&found) != Some(n as u64) {
        // A small group that does not fill its area: its cells, not the area.
        return entry.members.iter().map(|f| f.anchor.0.clone()).collect();
    }
    vec![found]
}

/// The Cells text of a row, written for the part of `sheet`.
fn area_text(entry: &Entry<'_>, groups: &HashMap<&str, &str>, sheet: Option<&str>) -> String {
    area(entry, groups)
        .iter()
        .map(|a| cell_text(local(a, sheet)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A group with up to this many members that does not fill its area lists its cells.
const MAX_LISTED: usize = 5;

/// The number of cells of a `Sheet!A1:B9` area.
fn cell_count(anchor: &str) -> Option<u64> {
    let ((c0, r0), (c1, r1)) = corners(split_sheet(anchor)?.1)?;
    Some((u64::from(c1.abs_diff(c0)) + 1) * (u64::from(r1.abs_diff(r0)) + 1))
}

fn group_area(entry: &Entry<'_>, groups: &HashMap<&str, &str>) -> String {
    let first = entry.first();
    if entry.members.len() == 1 {
        return first
            .range
            .as_ref()
            .map_or_else(|| first.anchor.0.clone(), |r| r.0.clone());
    }
    if let Some(range) = &first.range
        && range.0 != first.anchor.0
        && entry
            .members
            .iter()
            .all(|f| f.range.as_ref() == Some(range))
    {
        return range.0.clone();
    }
    if let Some(node) = first.pattern.as_deref().and_then(|p| groups.get(p)) {
        return (*node).to_string();
    }
    let Some((sheet, _)) = split_sheet(&first.anchor.0) else {
        return first.anchor.0.clone();
    };
    let mut bounds: Option<((u32, u32), (u32, u32))> = None;
    for f in &entry.members {
        let Some((s, rest)) = split_sheet(&f.anchor.0) else {
            return first.anchor.0.clone();
        };
        let Some((a, b)) = corners(rest).filter(|_| s == sheet) else {
            return first.anchor.0.clone();
        };
        bounds = Some(match bounds {
            None => (a, b),
            Some((lo, hi)) => (
                (lo.0.min(a.0), lo.1.min(a.1)),
                (hi.0.max(b.0), hi.1.max(b.1)),
            ),
        });
    }
    let Some(((c0, r0), (c1, r1))) = bounds else {
        return first.anchor.0.clone();
    };
    format!(
        "{sheet}!{}{}:{}{}",
        column_letter(c0 as usize),
        r0 + 1,
        column_letter(c1 as usize),
        r1 + 1
    )
}

fn code(text: &str) -> String {
    let mut out = String::new();
    code_span(text, Mode::Cell, &mut out);
    out
}

fn math(latex: &str) -> String {
    blocks::inlines(
        &[Inline::Math {
            latex: latex.to_string(),
        }],
        Mode::Cell,
    )
}

/// The stored result, or that the file has none.
fn cached(f: &Formula) -> String {
    match &f.cached {
        Some(v) => cell_text(&value_text(v)),
        None if is_name(f) => String::new(),
        None => "not calculated in file".to_string(),
    }
}

fn reads(f: &Formula, sheet: Option<&str>) -> String {
    let mut list: Vec<String> = f
        .references
        .iter()
        .take(MAX_READS)
        .map(|r| cell_text(local(&r.0, sheet)))
        .collect();
    if f.references.len() > MAX_READS {
        list.push(format!("and {} more", f.references.len() - MAX_READS));
    }
    list.join(", ")
}

fn roles(entry: &Entry<'_>) -> String {
    let mut seen: Vec<Role> = Vec::new();
    for f in &entry.members {
        if let Some(r) = f.role
            && !seen.contains(&r)
        {
            seen.push(r);
        }
    }
    seen.iter()
        .map(|r| role_name(*r))
        .collect::<Vec<_>>()
        .join(", ")
}

fn header(cols: &[&str]) -> Vec<String> {
    cols.iter().map(|c| (*c).to_string()).collect()
}

/// `#### Formulas`: one row per formula of its own or pattern.
fn formulas_table(
    list: &[Entry<'_>],
    sheet: Option<&str>,
    groups: &HashMap<&str, &str>,
    with_math: bool,
) -> String {
    let with_note = list.iter().any(|e| e.first().note.is_some());
    let with_math = with_math
        && list.iter().any(|e| {
            e.first()
                .latex
                .as_deref()
                .is_some_and(|l| !l.trim().is_empty())
        });
    let mut cols = vec!["Cells", "Count", "Formula", "Reads", "Role", "Value"];
    if with_note {
        cols.push("Note");
    }
    if with_math {
        cols.push("Math");
    }
    let rows: Vec<Vec<String>> = list
        .iter()
        .map(|e| {
            let f = e.first();
            let mut row = vec![
                area_text(e, groups, sheet),
                e.members.len().to_string(),
                code(&f.expression),
                reads(f, sheet),
                roles(e),
                cached(f),
            ];
            if with_note {
                row.push(f.note.as_deref().map(cell_text).unwrap_or_default());
            }
            if with_math {
                row.push(
                    f.latex
                        .as_deref()
                        .filter(|l| !l.trim().is_empty())
                        .map(math)
                        .unwrap_or_default(),
                );
            }
            row
        })
        .collect();
    let mut out = String::from("#### Formulas\n\n");
    if list.iter().any(|e| e.members.len() > 1) {
        out.push_str(
            "A row with a count above 1 stands for one formula copied over its cells. It shows the first cell.\n\n",
        );
    }
    out.push_str(&pipe_table(&header(&cols), &[], &rows));
    out
}

/// `#### Results`: the formulas marked as results with their stored values.
fn results_table(
    list: &[Entry<'_>],
    sheet: Option<&str>,
    groups: &HashMap<&str, &str>,
) -> Option<String> {
    let results: Vec<&Entry<'_>> = list.iter().filter(|e| e.has_role(Role::Result)).collect();
    if results.is_empty() {
        return None;
    }
    let rows: Vec<Vec<String>> = results
        .iter()
        .map(|e| {
            let f = e.first();
            vec![area_text(e, groups, sheet), code(&f.expression), cached(f)]
        })
        .collect();
    let mut out = String::from("#### Results\n\n");
    if results.iter().any(|e| e.members.len() > 1) {
        out.push_str("For formulas copied over several cells the first cell is shown.\n\n");
    }
    out.push_str(&pipe_table(
        &header(&["Cell", "Formula", "Value"]),
        &[],
        &rows,
    ));
    Some(out)
}

fn unit_source_words(source: &str) -> String {
    match source {
        "label" => "label".into(),
        "unit-cell" => "cell beside".into(),
        "number-format" => "number format".into(),
        "convert" => "CONVERT".into(),
        "header" => "column header".into(),
        other => cell_text(other),
    }
}

/// The 0-based (column, row) of a single-cell anchor on `sheet`.
fn cell_of(anchor: &str, sheet: &str) -> Option<(u32, u32)> {
    let (s, rest) = split_sheet(anchor)?;
    let (a, b) = corners(rest)?;
    (s == sheet && a == b).then_some(a)
}

/// Runs of parameters in list order that sit next to each other in one row or one column,
/// have no name, hold quantities of one kind and share their unit and its source; later
/// cells have the first cell's label or none. A run of [`MIN_RUN`] or more becomes one row.
fn parameter_runs<'a, 'b>(list: &'b [&'a Parameter]) -> Vec<&'b [&'a Parameter]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < list.len() {
        let first = list[i];
        let mut end = i + 1;
        let start = split_sheet(&first.anchor.0)
            .and_then(|(s, _)| cell_of(&first.anchor.0, s).map(|c| (s, c)));
        if let Some((sheet, (c0, r0))) = start
            && first.name.is_none()
            && is_quantity(first.value.as_ref())
        {
            let mut step: Option<(u32, u32)> = None;
            let mut prev = (c0, r0);
            while end < list.len() {
                let p = list[end];
                let Some(cell) = cell_of(&p.anchor.0, sheet) else {
                    break;
                };
                let this = match (cell.0.checked_sub(prev.0), cell.1.checked_sub(prev.1)) {
                    (Some(1), Some(0)) => (1, 0),
                    (Some(0), Some(1)) => (0, 1),
                    _ => break,
                };
                let fits = step.is_none_or(|s| s == this)
                    && p.name.is_none()
                    && p.unit == first.unit
                    && p.unit_source == first.unit_source
                    && same_kind(p.value.as_ref(), first.value.as_ref())
                    && (p.label.is_none() || p.label == first.label);
                if !fits {
                    break;
                }
                step = Some(this);
                prev = cell;
                end += 1;
            }
        }
        if end - i < MIN_RUN {
            end = i + 1;
        }
        out.push(&list[i..end]);
        i = end;
    }
    out
}

/// Numbers, percentages and dates can form a run; texts and flags stand on their own.
fn is_quantity(v: Option<&Value>) -> bool {
    matches!(
        v,
        Some(
            Value::Number { .. }
                | Value::Percent { .. }
                | Value::Date { .. }
                | Value::DateTime { .. }
        )
    )
}

fn same_kind(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => std::mem::discriminant(a) == std::mem::discriminant(b),
        _ => false,
    }
}

/// A run of parameters folds into one row from this many cells on.
const MIN_RUN: usize = 3;

/// `#### Parameters`. Runs of unnamed values (a row of inputs, a column of test values) are
/// one row each, with the first three values and the last.
fn parameters_table(list: &[&Parameter], sheet: Option<&str>) -> String {
    let opt = |v: &Option<String>| v.as_deref().map(cell_text).unwrap_or_default();
    let value = |p: &Parameter| p.value.as_ref().map(value_text).unwrap_or_default();
    let rows: Vec<Vec<String>> = parameter_runs(list)
        .into_iter()
        .map(|run| {
            let p = run[0];
            let last = run[run.len() - 1];
            if run.len() == 1 {
                return vec![
                    cell_text(local(&p.anchor.0, sheet)),
                    opt(&p.name),
                    opt(&p.label),
                    cell_text(&value(p)),
                    opt(&p.unit),
                    p.unit_source
                        .as_deref()
                        .map(unit_source_words)
                        .unwrap_or_default(),
                    p.read_by.to_string(),
                ];
            }
            let cells = format!(
                "{}:{}",
                local(&p.anchor.0, sheet),
                split_sheet(&last.anchor.0).map_or(last.anchor.0.as_str(), |(_, r)| r)
            );
            let shown: Vec<String> = run.iter().take(3).map(|p| value(p)).collect();
            let mut values = shown.join(", ");
            if run.len() > 4 {
                values.push_str(" ...");
            }
            if run.len() > 3 {
                values.push_str(", ");
                values.push_str(&value(last));
            }
            values.push_str(&format!(" ({} values)", run.len()));
            let low = run.iter().map(|p| p.read_by).min().unwrap_or(0);
            let high = run.iter().map(|p| p.read_by).max().unwrap_or(0);
            let read_by = if low == high {
                format!("{low} each")
            } else {
                format!("{low} to {high} each")
            };
            vec![
                cell_text(&cells),
                String::new(),
                opt(&p.label),
                cell_text(&values),
                opt(&p.unit),
                p.unit_source
                    .as_deref()
                    .map(unit_source_words)
                    .unwrap_or_default(),
                read_by,
            ]
        })
        .collect();
    let mut out = String::from("#### Parameters\n\n");
    if rows.len() < list.len() {
        out.push_str(
            "A row with a cell range stands for values next to each other that play one part.\n\n",
        );
    }
    out.push_str(&pipe_table(
        &header(&[
            "Cell",
            "Name",
            "Label",
            "Value",
            "Unit",
            "Unit from",
            "Read by",
        ]),
        &[],
        &rows,
    ));
    out
}

/// `#### Rules`: validation and conditional formats.
fn rules_table(list: &[&Rule], sheet: Option<&str>) -> String {
    let rows: Vec<Vec<String>> = list
        .iter()
        .map(|r| {
            vec![
                cell_text(&r.kind),
                cell_text(local(&r.anchor.0, sheet)),
                cell_text(&r.detail),
                r.formulas
                    .iter()
                    .map(|f| code(f))
                    .collect::<Vec<_>>()
                    .join(", "),
                r.message.as_deref().map(cell_text).unwrap_or_default(),
            ]
        })
        .collect();
    format!(
        "#### Rules\n\n{}",
        pipe_table(
            &header(&["Kind", "Cells", "Rule", "Formulas", "Message"]),
            &[],
            &rows
        )
    )
}

/// Everything that lies on one sheet (or on none, for `### Other`).
#[derive(Default)]
struct Part<'a> {
    formulas: Vec<&'a Formula>,
    parameters: Vec<&'a Parameter>,
    rules: Vec<&'a Rule>,
}

impl Part<'_> {
    fn is_empty(&self) -> bool {
        self.formulas.is_empty() && self.parameters.is_empty() && self.rules.is_empty()
    }
}

fn part(
    title: &str,
    p: &Part<'_>,
    sheet: Option<&str>,
    groups: &HashMap<&str, &str>,
    with_math: bool,
) -> String {
    let mut out = vec![heading_line(3, &line_text(title))];
    if !p.parameters.is_empty() {
        out.push(parameters_table(&p.parameters, sheet));
    }
    if !p.formulas.is_empty() {
        let list = entries(&p.formulas);
        out.push(formulas_table(&list, sheet, groups, with_math));
        out.extend(results_table(&list, sheet, groups));
    }
    if !p.rules.is_empty() {
        out.push(rules_table(&p.rules, sheet));
    }
    out.join("\n\n")
}

/// `### Names`: defined names and LAMBDA names, with the value and read count of names that
/// hold a plain value.
fn names_table(names: &[&Formula], params: &HashMap<&str, &Parameter>, sheets: &Sheets) -> String {
    let rows: Vec<Vec<String>> = names
        .iter()
        .map(|f| {
            let rest = f.anchor.0.strip_prefix("name:").unwrap_or(&f.anchor.0);
            let (scope, name) = match split_sheet(rest) {
                Some((s, n)) => (sheets.title(s), n),
                None => ("workbook".to_string(), rest),
            };
            let kind = if f.kind == Some(FormulaKind::Lambda) {
                "LAMBDA"
            } else {
                "name"
            };
            let p = params.get(f.anchor.0.as_str());
            vec![
                cell_text(name),
                cell_text(&scope),
                kind.to_string(),
                code(&f.expression),
                p.and_then(|p| p.value.as_ref())
                    .map(|v| cell_text(&value_text(v)))
                    .unwrap_or_default(),
                p.map(|p| p.read_by.to_string()).unwrap_or_default(),
                f.note.as_deref().map(cell_text).unwrap_or_default(),
            ]
        })
        .collect();
    format!(
        "### Names\n\n{}",
        pipe_table(
            &header(&[
                "Name",
                "Scope",
                "Kind",
                "Refers to",
                "Value",
                "Read by",
                "Note"
            ]),
            &[],
            &rows
        )
    )
}

/// Calls `found` with the name of every function called in `expr` (the text right before a
/// `(`), outside string literals and quoted sheet names.
fn called_functions(expr: &str, mut found: impl FnMut(&str)) {
    let mut in_string = false;
    let mut in_sheet = false;
    let mut start: Option<usize> = None;
    for (i, c) in expr.char_indices() {
        if in_string {
            in_string = c != '"';
            continue;
        }
        if in_sheet {
            in_sheet = c != '\'';
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                start = None;
            }
            '\'' => {
                in_sheet = true;
                start = None;
            }
            '(' => {
                if let Some(s) = start.take() {
                    found(&expr[s..i]);
                }
            }
            c if c.is_alphanumeric() || matches!(c, '_' | '.' | '!' | '[' | ']') => {
                start.get_or_insert(i);
            }
            _ => start = None,
        }
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The calculation settings in words, from the reader's properties.
fn calculation(doc: &Document) -> Option<String> {
    let custom = &doc.properties.custom;
    let mut words = Vec::new();
    if let Some(Value::Text { value }) = custom.get("calc_mode") {
        words.push(match value.as_str() {
            "auto" => "automatic".to_string(),
            "manual" => "manual".to_string(),
            "auto_no_table" => "automatic except data tables".to_string(),
            other => cell_text(other),
        });
    }
    match custom.get("iterative_calculation") {
        Some(Value::Bool { value: true }) => {
            let mut limits = Vec::new();
            if let Some(Value::Number { value, .. }) = custom.get("iteration_count") {
                limits.push(format!("{} steps", crate::facets::number(*value)));
            }
            if let Some(Value::Number { value, .. }) = custom.get("iteration_delta") {
                limits.push(format!("max change {}", crate::facets::number(*value)));
            }
            if limits.is_empty() {
                words.push("iterative calculation on".to_string());
            } else {
                words.push(format!("iterative calculation on ({})", limits.join(", ")));
            }
        }
        Some(Value::Bool { value: false }) => words.push("iterative calculation off".to_string()),
        _ => {}
    }
    if matches!(custom.get("date_system"), Some(Value::Text { value }) if value == "1904") {
        words.push("1904 date system".to_string());
    }
    (!words.is_empty()).then(|| words.join(", "))
}

/// `### Hidden logic`: what runs or comes from outside the cells.
fn hidden_logic(doc: &Document, cells: &[&Formula]) -> Option<String> {
    let embedded = &doc.facets.embedded;
    let mut udf_uses: HashMap<String, usize> = embedded
        .iter()
        .filter(|e| e.kind == "user-function")
        .map(|e| (e.name.to_lowercase(), 0))
        .collect();
    let mut volatile: Vec<(&str, usize, Vec<&str>)> = Vec::new();
    for f in &doc.facets.formulas {
        let mut seen_udf: Vec<String> = Vec::new();
        let mut seen_volatile: Vec<&str> = Vec::new();
        called_functions(&f.expression, |name| {
            let lower = name.to_lowercase();
            if udf_uses.contains_key(&lower) && !seen_udf.contains(&lower) {
                seen_udf.push(lower);
            }
            let upper = name.to_ascii_uppercase();
            if let Some(v) = VOLATILE.iter().find(|v| **v == upper)
                && !seen_volatile.contains(v)
            {
                seen_volatile.push(v);
            }
        });
        for u in seen_udf {
            if let Some(n) = udf_uses.get_mut(&u) {
                *n += 1;
            }
        }
        for v in seen_volatile {
            match volatile.iter_mut().find(|(name, _, _)| *name == v) {
                Some((_, n, cells)) => {
                    *n += 1;
                    if cells.len() < MAX_VOLATILE_CELLS {
                        cells.push(&f.anchor.0);
                    }
                }
                None => volatile.push((v, 1, vec![&f.anchor.0])),
            }
        }
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    let detail_of = |detail: &Option<String>, target: &Option<String>| {
        let mut parts = Vec::new();
        if let Some(d) = detail.as_deref().filter(|d| !d.trim().is_empty()) {
            parts.push(cell_text(d));
        }
        if let Some(t) = target.as_deref().filter(|t| !t.trim().is_empty()) {
            parts.push(code(t));
        }
        parts.join(" ")
    };
    for (kind, what) in [
        ("user-function", "user function"),
        ("add-in", "add-in"),
        ("external-workbook", "external workbook"),
        ("macro", "VBA module"),
    ] {
        for e in embedded.iter().filter(|e| e.kind == kind) {
            let mut detail = detail_of(&e.detail, &e.target);
            let name = if kind == "user-function" {
                let uses = udf_uses.get(&e.name.to_lowercase()).copied().unwrap_or(0);
                if uses > 0 {
                    if !detail.is_empty() {
                        detail.push_str(", ");
                    }
                    detail.push_str(&format!("used in {}", plural(uses, "formula", "formulas")));
                }
                code(&e.name)
            } else {
                cell_text(&e.name)
            };
            rows.push(vec![what.to_string(), name, detail]);
        }
    }
    let mut more: Vec<String> = Vec::new();
    for (kind, what, many) in [
        (FormulaKind::DataTable, "data table", "data tables"),
        (FormulaKind::Spill, "spill range", "spill ranges"),
        (FormulaKind::Array, "array formula", "array formulas"),
    ] {
        let list: Vec<&&Formula> = cells
            .iter()
            .filter(|f| {
                f.kind == Some(kind)
                    && (kind == FormulaKind::DataTable
                        || f.range.as_ref().is_some_and(|r| r.0 != f.anchor.0))
            })
            .collect();
        for f in list.iter().take(MAX_AREAS) {
            let range = f.range.as_ref().map_or(&f.anchor.0, |r| &r.0);
            let mut detail = code(&f.expression);
            if let Some(note) = f.note.as_deref().filter(|n| !n.trim().is_empty()) {
                detail.push(' ');
                detail.push_str(&cell_text(note));
            }
            rows.push(vec![what.to_string(), cell_text(range), detail]);
        }
        if list.len() > MAX_AREAS {
            more.push(format!(
                "{} are not listed.",
                plural(
                    list.len() - MAX_AREAS,
                    &format!("more {what}"),
                    &format!("more {many}")
                )
            ));
        }
    }
    for (name, n, at) in &volatile {
        let mut where_ = at
            .iter()
            .map(|a| cell_text(a))
            .collect::<Vec<_>>()
            .join(", ");
        if *n > at.len() {
            where_.push_str(&format!(" and {} more", n - at.len()));
        }
        rows.push(vec![
            "volatile function".to_string(),
            (*name).to_string(),
            format!("in {}: {where_}", plural(*n, "formula", "formulas")),
        ]);
    }
    if !cells.is_empty()
        && let Some(calc) = calculation(doc)
    {
        rows.push(vec!["calculation".into(), "workbook".into(), calc]);
    }
    if rows.is_empty() {
        return None;
    }
    let mut out = format!(
        "### Hidden logic\n\n{}",
        pipe_table(&header(&["What", "Where", "Detail"]), &[], &rows)
    );
    for line in more {
        out.push_str("\n\n");
        out.push_str(&line);
    }
    Some(out)
}

/// The R3 callout: the reader's warning that stored results may be old, at the top.
fn stale_callout(doc: &Document) -> Option<String> {
    let loss = doc
        .losses
        .iter()
        .find(|l| l.what.to_lowercase().starts_with(STALE))?;
    let reasons = loss.what[STALE.len()..]
        .trim_start_matches([':', ' '])
        .trim();
    let mut text = String::new();
    let mut chars = reasons.chars();
    if let Some(c) = chars.next() {
        text.extend(c.to_uppercase());
        text.push_str(chars.as_str());
        if !text.ends_with('.') {
            text.push('.');
        }
        text.push(' ');
    }
    text.push_str("Rezzer does not calculate: the values shown are the ones stored in the file.");
    let callout = Block::Callout {
        kind: CalloutKind::Warning,
        title: Some("Stored results may be old".into()),
        folded: None,
        blocks: vec![Block::paragraph(vec![Inline::plain(&text)])],
        anchor: None,
    };
    let ctx = Ctx {
        assets: &doc.assets,
        heading_offset: 0,
    };
    Some(blocks::blocks(&[callout], &ctx, 0, 3, false))
}

/// Where a graph node goes when the graph is drawn with one box per sheet.
fn bucket(node: &Node, sheets: &Sheets) -> (String, String, &'static str) {
    let anchor = node
        .anchor
        .as_ref()
        .map_or(node.label.as_str(), |a| a.0.as_str());
    if anchor.starts_with("name:") {
        return ("names".into(), "Names".into(), "name");
    }
    if anchor.starts_with("ext:") {
        return ("ext".into(), "Other workbooks".into(), "sheet");
    }
    if anchor.starts_with("table:") {
        return ("tables".into(), "Tables".into(), "sheet");
    }
    match split_sheet(anchor) {
        Some((s, _)) => (format!("sheet:{s}"), sheets.title(s), "sheet"),
        None => ("other".into(), "Other".into(), "sheet"),
    }
}

/// The dependency graph with one node per sheet; edges count the links they stand for.
fn per_sheet(graph: &Graph, sheets: &Sheets) -> Graph {
    let mut nodes: Vec<Node> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut of_node: HashMap<&str, usize> = HashMap::new();
    for n in &graph.nodes {
        let (key, label, kind) = bucket(n, sheets);
        let i = *index.entry(key.clone()).or_insert_with(|| {
            nodes.push(Node {
                id: key,
                label,
                kind: Some(kind.into()),
                anchor: None,
            });
            nodes.len() - 1
        });
        of_node.entry(n.id.as_str()).or_insert(i);
    }
    let mut counts: Vec<((usize, usize), usize)> = Vec::new();
    let mut at: HashMap<(usize, usize), usize> = HashMap::new();
    for e in &graph.edges {
        let (Some(&a), Some(&b)) = (of_node.get(e.from.as_str()), of_node.get(e.to.as_str()))
        else {
            continue;
        };
        if a == b {
            continue;
        }
        match at.get(&(a, b)) {
            Some(&i) => counts[i].1 += 1,
            None => {
                at.insert((a, b), counts.len());
                counts.push(((a, b), 1));
            }
        }
    }
    let edges = counts
        .into_iter()
        .map(|((a, b), n)| rezzer_model::Edge {
            from: nodes[a].id.clone(),
            to: nodes[b].id.clone(),
            kind: n.to_string(),
            anchor: None,
        })
        .collect();
    Graph {
        id: graph.id.clone(),
        kind: graph.kind.clone(),
        nodes,
        edges,
    }
}

fn shapes_sentence(graph: &Graph) -> String {
    let has = |k: &str| graph.nodes.iter().any(|n| n.kind.as_deref() == Some(k));
    let mut shapes = Vec::new();
    if has("group") {
        shapes.push("double boxes are formulas copied over several cells");
    }
    if has("parameters") {
        shapes.push("round boxes are parameters");
    }
    if has("name") {
        shapes.push("hexagons are names");
    }
    let mut out = String::from("Arrows point from what is read to what reads it.");
    if let Some((first, rest)) = shapes.split_first() {
        let mut s = String::new();
        let mut chars = first.chars();
        if let Some(c) = chars.next() {
            s.extend(c.to_uppercase());
            s.push_str(chars.as_str());
        }
        for r in rest {
            s.push_str(", ");
            s.push_str(r);
        }
        out.push(' ');
        out.push_str(&s);
        out.push('.');
    }
    out
}

/// `### Dependencies`: the graph in full when it fits the caps, else one box per sheet, else
/// a note.
fn dependencies(graph: &Graph, doc: &Document, sheets: &Sheets) -> String {
    let mut out = String::from("### Dependencies\n\n");
    if graph.nodes.len() <= MAX_NODES && graph.edges.len() <= MAX_EDGES {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for f in &doc.facets.formulas {
            if let Some(p) = f.pattern.as_deref() {
                *counts.entry(p).or_insert(0) += 1;
            }
        }
        out.push_str(&shapes_sentence(graph));
        out.push('\n');
        out.push_str(&mermaid::flowchart(
            graph,
            &|n: &Node| match counts.get(n.id.as_str()) {
                Some(&c) if c > 1 && n.kind.as_deref() == Some("group") => {
                    format!("{}, {c} formulas", n.label)
                }
                _ => n.label.clone(),
            },
        ));
        return out;
    }
    let small = per_sheet(graph, sheets);
    let sheet_count = small
        .nodes
        .iter()
        .filter(|n| n.id.starts_with("sheet:"))
        .count();
    if small.nodes.len() <= MAX_NODES && small.edges.len() <= MAX_EDGES {
        out.push_str(&format!(
            "The graph has {} and {}, more than can be drawn, so it shows one box per sheet. A number on an arrow counts the links it stands for.\n",
            plural(graph.nodes.len(), "node", "nodes"),
            plural(graph.edges.len(), "edge", "edges"),
        ));
        out.push_str(&mermaid::flowchart(&small, &|n: &Node| n.label.clone()));
    } else {
        out.push_str(&format!(
            "The graph has {} and {} on {}, too many to draw even with one box per sheet. The model JSON has the whole graph.",
            plural(graph.nodes.len(), "node", "nodes"),
            plural(graph.edges.len(), "edge", "edges"),
            plural(sheet_count, "sheet", "sheets"),
        ));
    }
    out
}

/// `## Logic` for a workbook. `None` when there is nothing to show.
pub(crate) fn logic(doc: &Document, with_math: bool) -> Option<String> {
    let sheets = Sheets::of(&doc.body);
    let mut parts: Vec<Part<'_>> = (0..sheets.list.len()).map(|_| Part::default()).collect();
    let mut other = Part::default();
    let mut names: Vec<&Formula> = Vec::new();
    let mut cells: Vec<&Formula> = Vec::new();
    for f in &doc.facets.formulas {
        if is_name(f) {
            names.push(f);
            continue;
        }
        cells.push(f);
        match sheets.of_anchor(&f.anchor.0) {
            Some(i) => parts[i].formulas.push(f),
            None => other.formulas.push(f),
        }
    }
    let mut name_params: HashMap<&str, &Parameter> = HashMap::new();
    for p in &doc.facets.parameters {
        if p.anchor.0.starts_with("name:") {
            name_params.entry(p.anchor.0.as_str()).or_insert(p);
            continue;
        }
        match sheets.of_anchor(&p.anchor.0) {
            Some(i) => parts[i].parameters.push(p),
            None => other.parameters.push(p),
        }
    }
    for r in &doc.facets.rules {
        match sheets.of_anchor(&r.anchor.0) {
            Some(i) => parts[i].rules.push(r),
            None => other.rules.push(r),
        }
    }
    let dependency = doc.facets.graphs.iter().find(|g| g.kind == "dependency");
    let groups: HashMap<&str, &str> = dependency
        .map(|g| {
            g.nodes
                .iter()
                .filter(|n| n.kind.as_deref() == Some("group"))
                .map(|n| {
                    (
                        n.id.as_str(),
                        n.anchor.as_ref().map_or(n.label.as_str(), |a| a.0.as_str()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let mut out: Vec<String> = Vec::new();
    out.extend(stale_callout(doc));
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            continue;
        }
        let (title, anchor) = &sheets.list[i];
        out.push(part(title, p, Some(anchor), &groups, with_math));
    }
    if !other.is_empty() {
        out.push(part("Other", &other, None, &groups, with_math));
    }
    if !names.is_empty() {
        out.push(names_table(&names, &name_params, &sheets));
    }
    if let Some(g) = dependency {
        out.push(dependencies(g, doc, &sheets));
    }
    out.extend(hidden_logic(doc, &cells));
    for g in doc.facets.graphs.iter().filter(|g| g.kind != "dependency") {
        out.push(mermaid::render(g));
    }
    // The callout alone is no Logic.
    let callout = usize::from(out.first().is_some_and(|s| s.starts_with('>')));
    if out.len() <= callout {
        return None;
    }
    Some(format!("## Logic\n\n{}", out.join("\n\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_sheet_handles_quotes() {
        assert_eq!(split_sheet("Calc!A1"), Some(("Calc", "A1")));
        assert_eq!(
            split_sheet("'It''s !x'!A1:B2"),
            Some(("'It''s !x'", "A1:B2"))
        );
        assert_eq!(split_sheet("'open"), None);
        assert_eq!(split_sheet("name:Calc!x"), None);
        assert_eq!(split_sheet("ext:[1]Calc!A1"), None);
        assert_eq!(split_sheet("A1"), None);
        assert_eq!(unquote("'It''s'"), "It's");
    }

    #[test]
    fn corners_read_cells_and_ranges() {
        assert_eq!(corners("$B$2"), Some(((1, 1), (1, 1))));
        assert_eq!(corners("AA10:AB12"), Some(((26, 9), (27, 11))));
        assert_eq!(corners("B:B"), None);
        assert_eq!(corners("B0"), None);
    }

    #[test]
    fn called_functions_skip_strings_and_sheet_names() {
        let mut found = Vec::new();
        called_functions("=NOW()+\"RAND()\"+'Sheet (1)'!A1+[1]!f(SUM(A1))", |n| {
            found.push(n.to_string())
        });
        assert_eq!(found, vec!["NOW", "[1]!f", "SUM"]);
    }
}
