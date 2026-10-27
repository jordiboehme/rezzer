// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The tables of a sheet part: parameters, formulas, results, rules and names.

use super::*;

pub(super) fn code(text: &str) -> String {
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

/// The `Reads` cell. A read of a labelled parameter on another sheet names its full label,
/// as the short label under its group title stands only in that sheet's part.
fn reads(f: &Formula, sheet: Option<&str>, labels: &Labels) -> String {
    let mut list: Vec<String> = f
        .references
        .iter()
        .take(MAX_READS)
        .map(|r| {
            let shown = local(&r.0, sheet);
            match labels.get(&r.0).filter(|_| shown == r.0) {
                Some(label) => format!("{} ({})", cell_text(shown), cell_text(label)),
                None => cell_text(shown),
            }
        })
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

pub(super) fn header(cols: &[&str]) -> Vec<String> {
    cols.iter().map(|c| (*c).to_string()).collect()
}

/// `#### Formulas`: one row per formula of its own or pattern.
fn formulas_table(
    list: &[Entry<'_>],
    sheet: Option<&str>,
    groups: &HashMap<&str, &str>,
    with_math: bool,
    labels: &Labels,
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
                reads(f, sheet, labels),
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
    // Only the members that are results: a group with mixed roles gives its result cells.
    let results: Vec<Entry<'_>> = list
        .iter()
        .filter(|e| e.has_role(Role::Result))
        .map(|e| {
            let members: Vec<&Formula> = e
                .members
                .iter()
                .copied()
                .filter(|f| f.role == Some(Role::Result))
                .collect();
            Entry {
                partial: members.len() < e.members.len(),
                members,
            }
        })
        .collect();
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
/// have no name, hold quantities of one kind and share their label, unit and unit source
/// (the model's facts for each cell; no label is passed on to a neighbour). A run of [`MIN_RUN`] or more becomes one row.
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
                    && p.label == first.label;
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

/// `#### Parameters`: one group per sheet region in reading order, the parameters outside
/// every region last, each group one table (spec 8.7). The start the paths of a group share
/// is written once as its title (`Parameters in <start>:`) when it holds a heading of the
/// region; the Label column holds the shortest end of each path that is unique in the
/// group. Runs of unnamed values (a row of inputs, a column of test values) are one row
/// each, with the first three values and the last.
fn parameters_table(list: &[&Parameter], sheet: Option<&str>, regions: &[Region]) -> String {
    // The parameters on this sheet's cells, as (row, column) points.
    let cells: Vec<(usize, (u32, u32))> = list
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let (col, row) = sheet.and_then(|s| cell_of(&p.anchor.0, s))?;
            Some((i, (row, col)))
        })
        .collect();
    let areas: Vec<CellArea> = regions.iter().map(|r| r.area).collect();
    let points: Vec<(u32, u32)> = cells.iter().map(|c| c.1).collect();
    let mut region_of = vec![None; list.len()];
    for ((i, _), at) in cells.iter().zip(locate_cells(&areas, &points)) {
        region_of[*i] = at;
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<&Parameter>> =
        std::collections::BTreeMap::new();
    for (p, at) in list.iter().zip(region_of) {
        groups.entry(at.unwrap_or(usize::MAX)).or_default().push(p);
    }
    let mut folded = false;
    let mut parts: Vec<String> = Vec::new();
    for (k, group) in &groups {
        let runs = parameter_runs(group);
        folded |= runs.len() < group.len();
        let headings = regions.get(*k).map_or(&[][..], |r| r.headings.as_slice());
        let (title, labels) = short_labels(&runs, headings);
        if let Some(title) = title {
            parts.push(format!("Parameters in {}:", line_text(&title)));
        }
        parts.push(group_table(&runs, &labels, sheet));
    }
    let mut out = String::from("#### Parameters\n\n");
    if folded {
        out.push_str(
            "A row with a cell range stands for values next to each other that play one part.\n\n",
        );
    }
    out.push_str(&parts.join("\n\n"));
    out
}

/// The title of a group and the label of each row. The title is the longest start the
/// paths share, as long as every path keeps at least one part after it, and only when that
/// start holds one of the region's `headings` (a column header alone is no title). Each
/// label is the shortest end (whole parts) no other row's path ends with; when none is,
/// the parts after the title written, else the last part. A row without a label stays
/// without one.
fn short_labels(runs: &[&[&Parameter]], headings: &[String]) -> (Option<String>, Vec<String>) {
    let paths: Vec<Vec<&str>> = runs.iter().map(|r| parts_of(r[0])).collect();
    // A parameter without a label has no say in the title.
    let named: Vec<&Vec<&str>> = paths.iter().filter(|p| !p.is_empty()).collect();
    let shortest = named.iter().map(|p| p.len()).min().unwrap_or(0);
    let mut shared = 0;
    while shared + 1 < shortest && named.iter().all(|p| p[shared] == named[0][shared]) {
        shared += 1;
    }
    let titled = named
        .first()
        .is_some_and(|p| p[..shared].iter().any(|t| headings.iter().any(|h| h == t)));
    let shared = if titled { shared } else { 0 };
    let title = titled.then(|| named[0][..shared].join(" / "));
    // How many paths end with each end of each length.
    let mut ends: HashMap<(usize, Vec<&str>), usize> = HashMap::new();
    for p in &paths {
        for k in 1..=p.len() {
            *ends.entry((k, p[p.len() - k..].to_vec())).or_default() += 1;
        }
    }
    let labels = paths
        .iter()
        .map(|p| {
            // The title is written, so the parts after it are enough; never the title again.
            let after = p.len().saturating_sub(shared).max(1).min(p.len());
            let k = (1..=after)
                .find(|&k| ends.get(&(k, p[p.len() - k..].to_vec())) == Some(&1))
                .unwrap_or(after);
            p[p.len() - k..].join(" / ")
        })
        .collect();
    (title, labels)
}

/// One group's table: a row per run, with the run's label.
fn group_table(runs: &[&[&Parameter]], labels: &[String], sheet: Option<&str>) -> String {
    let opt = |v: &Option<String>| v.as_deref().map(cell_text).unwrap_or_default();
    let value = |p: &Parameter| p.value.as_ref().map(value_text).unwrap_or_default();
    let rows: Vec<Vec<String>> = runs
        .iter()
        .zip(labels)
        .map(|(run, label)| {
            let p = run[0];
            let last = run[run.len() - 1];
            if run.len() == 1 {
                return vec![
                    cell_text(local(&p.anchor.0, sheet)),
                    opt(&p.name),
                    cell_text(label),
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
                cell_text(label),
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
    pipe_table(
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
    )
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
pub(super) struct Part<'a> {
    pub(super) formulas: Vec<&'a Formula>,
    pub(super) parameters: Vec<&'a Parameter>,
    pub(super) rules: Vec<&'a Rule>,
}

impl Part<'_> {
    pub(super) fn is_empty(&self) -> bool {
        self.formulas.is_empty() && self.parameters.is_empty() && self.rules.is_empty()
    }
}

/// What every part of the Logic section looks things up in.
pub(super) struct Lookup<'a> {
    pub(super) groups: HashMap<&'a str, &'a str>,
    pub(super) labels: Labels,
    pub(super) with_math: bool,
}

pub(super) fn part(
    title: &str,
    p: &Part<'_>,
    sheet: Option<&str>,
    regions: &[Region],
    look: &Lookup,
) -> String {
    let groups = &look.groups;
    let mut out = vec![heading_line(3, &line_text(title))];
    if !p.parameters.is_empty() {
        out.push(parameters_table(&p.parameters, sheet, regions));
    }
    if !p.formulas.is_empty() {
        let list = entries(&p.formulas);
        out.push(formulas_table(
            &list,
            sheet,
            groups,
            look.with_math,
            &look.labels,
        ));
        out.extend(results_table(&list, sheet, groups));
    }
    if !p.rules.is_empty() {
        out.push(rules_table(&p.rules, sheet));
    }
    out.join("\n\n")
}

/// `### Names`: defined names and LAMBDA names, with the value and read count of names that
/// hold a plain value. A name that stands for one labelled parameter cell names its full
/// label in a `Label` column (unless the label is the name), which is there only when some
/// name has one.
pub(super) fn names_table(
    names: &[&Formula],
    params: &HashMap<&str, &Parameter>,
    sheets: &Sheets,
    labels: &Labels,
) -> String {
    // A label that only repeats the name says nothing more.
    let label_of = |f: &Formula| {
        let rest = f.anchor.0.strip_prefix("name:").unwrap_or(&f.anchor.0);
        let name = split_sheet(rest).map_or(rest, |(_, n)| n);
        match f.references.as_slice() {
            [one] => labels.get(&one.0).filter(|l| *l != name),
            _ => None,
        }
    };
    let with_label = names.iter().any(|f| label_of(f).is_some());
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
            let mut row = vec![
                cell_text(name),
                cell_text(&scope),
                kind.to_string(),
                code(&f.expression),
            ];
            if with_label {
                row.push(label_of(f).map(cell_text).unwrap_or_default());
            }
            row.extend([
                p.and_then(|p| p.value.as_ref())
                    .map(|v| cell_text(&value_text(v)))
                    .unwrap_or_default(),
                p.map(|p| p.read_by.to_string()).unwrap_or_default(),
                f.note.as_deref().map(cell_text).unwrap_or_default(),
            ]);
            row
        })
        .collect();
    let mut cols = vec!["Name", "Scope", "Kind", "Refers to"];
    if with_label {
        cols.push("Label");
    }
    cols.extend(["Value", "Read by", "Note"]);
    format!("### Names\n\n{}", pipe_table(&header(&cols), &[], &rows))
}

/// Calls `found` with the name of every function called in `expr` (the text right before a
/// `(`), outside string literals and quoted sheet names.
pub(super) fn called_functions(expr: &str, mut found: impl FnMut(&str)) {
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

pub(super) fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}
