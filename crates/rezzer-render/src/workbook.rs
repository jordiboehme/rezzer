// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `## Logic` for a workbook (XLSX, XLSM, XLSB, ODS, XLS): one `### <sheet>` part per sheet
//! in sheet order with its parameters, formulas (copies folded into one row per pattern),
//! results and rules; then `### Names`, `### Dependencies` and `### Hidden logic`.
//!
//! Inside a sheet's part, cells of that sheet are written without the sheet name; cells of
//! other sheets keep it. Anchors that belong to no sheet of the body go to `### Other`.
//! Parameters are grouped by sheet region; a group's shared path start is its title, and
//! each label is the shortest unique end of its path (spec 8.7). Where a label stands
//! without that title (a read from another sheet, `### Names`) the full path is written.
//! Every pass over the formulas is linear, so a workbook with millions of formulas renders
//! in one sweep.
//!
//! With a sheet selection only the parts of the chosen sheets are written, their reads of
//! other sheets as written; the graph keeps what touches a chosen sheet, names stay all.

use std::collections::HashMap;

use rezzer_model::{
    Block, CalloutKind, CellArea, Document, FormatId, Formula, FormulaKind, Graph, Inline, Node,
    Parameter, Role, Rule, SectionKind, Value, column_letter, locate_cells, table_headings,
};

use crate::Selection;
use crate::blocks::{self, Ctx, Mode, cell_text, code_span, heading_line, line_text, pipe_table};
use crate::facets::{role_name, value_text};
use crate::mermaid::{self, MAX_EDGES, MAX_NODES};

mod areas;
mod logic;
mod tables;

use areas::*;
use logic::*;
use tables::*;

/// A `Reads` cell names at most this many references, then how many more.
const MAX_READS: usize = 20;

/// `### Hidden logic` lists at most this many data tables, spill ranges or array formulas
/// each; a line says how many more there are.
const MAX_AREAS: usize = 200;

/// A volatile function row names at most this many cells.
const MAX_VOLATILE_CELLS: usize = 5;

/// The start of the loss the sheet reader writes when cached results may not match; shared
/// with the reader through the model.
const STALE: &str = rezzer_model::STALE_RESULTS;

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

/// The sheets of the body in order, as (title, anchor), a map from anchor to index, and the
/// regions of each sheet in reading order.
struct Sheets<'a> {
    list: Vec<(String, &'a str)>,
    index: HashMap<&'a str, usize>,
    regions: Vec<Vec<Region>>,
}

impl<'a> Sheets<'a> {
    fn of(body: &'a [Block]) -> Sheets<'a> {
        let mut list = Vec::new();
        let mut index = HashMap::new();
        let mut regions = Vec::new();
        for b in body {
            if let Block::Section {
                kind: SectionKind::Sheet,
                title,
                anchor: Some(anchor),
                blocks,
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
                regions.push(regions_of(blocks));
            }
        }
        Sheets {
            list,
            index,
            regions,
        }
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

/// `## Logic` for a workbook. `None` when there is nothing to show. With a selection, only
/// the parts of the chosen sheets and the graph around them; names stay.
pub(crate) fn logic(
    doc: &Document,
    with_math: bool,
    selection: Option<&Selection>,
) -> Option<String> {
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
    let selected = dependency
        .zip(selection)
        .map(|(g, selection)| selected_graph(g, &sheets, selection));
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
    let look = Lookup {
        groups,
        labels: Labels::of(&doc.facets.parameters),
        with_math,
    };

    let hidden = hidden_logic(doc, &cells);
    // The headings of the parts after the sheets; a sheet with one of these names gets
    // `Sheet: <name>`, so each heading leads to one place.
    let mut fixed: Vec<&str> = Vec::new();
    if !other.is_empty() {
        fixed.push("Other");
    }
    if !names.is_empty() {
        fixed.push("Names");
    }
    if dependency.is_some() {
        fixed.push("Dependencies");
    }
    if hidden.is_some() {
        fixed.push("Hidden logic");
    }

    let mut out: Vec<String> = Vec::new();
    out.extend(stale_callout(doc));
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            continue;
        }
        let (title, anchor) = &sheets.list[i];
        if selection.is_some_and(|s| !s.chosen(anchor)) {
            continue;
        }
        let heading = if fixed.iter().any(|f| f.eq_ignore_ascii_case(title.trim())) {
            format!("Sheet: {title}")
        } else {
            title.clone()
        };
        out.push(part(&heading, p, Some(anchor), &sheets.regions[i], &look));
    }
    if !other.is_empty() {
        out.push(part("Other", &other, None, &[], &look));
    }
    if !names.is_empty() {
        out.push(names_table(&names, &name_params, &sheets, &look.labels));
    }
    if let Some(g) = selected.as_ref().or(dependency)
        && !g.nodes.is_empty()
    {
        out.push(dependencies(g, doc, &sheets));
    }
    out.extend(hidden);
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
mod tests;
