// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Parameters of cells and names and the dependency graph.

use super::*;

/// The kind of edge in the dependency graph: `from` feeds `to` (`to` reads `from`).
const EDGE_KIND: &str = "feeds";

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
pub(super) fn cell_parameters<'f>(
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
pub(super) fn name_parameters(
    facet: &[Formula],
    literal: Vec<(String, String, Value)>,
) -> Vec<Parameter> {
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
pub(super) fn dependency_graph(
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
