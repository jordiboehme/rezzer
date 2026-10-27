// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The calculation chain, hidden logic and the dependency graph of a workbook.

use super::*;

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
pub(super) fn hidden_logic(doc: &Document, cells: &[&Formula]) -> Option<String> {
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
pub(super) fn stale_callout(doc: &Document) -> Option<String> {
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
        codes_shown: Default::default(),
        assets: &doc.assets,
        link_base: "",
        heading_offset: 0,
        notes: None,
        in_sheet: Default::default(),
        table_style: Default::default(),
        column_numbers: false,
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
pub(super) fn dependencies(graph: &Graph, doc: &Document, sheets: &Sheets) -> String {
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

/// The dependency graph without the nodes and edges of sheets that are not selected: an
/// edge stays when one of its ends lies on a chosen sheet, a node when it lies on a chosen
/// sheet or an edge that stays names it.
pub(super) fn selected_graph(graph: &Graph, sheets: &Sheets, selection: &Selection) -> Graph {
    let on_chosen = |n: &Node| {
        let anchor = n.anchor.as_ref().map_or(n.label.as_str(), |a| a.0.as_str());
        split_sheet(anchor)
            .is_some_and(|(s, _)| sheets.index.contains_key(s) && selection.chosen(s))
    };
    let chosen: HashMap<&str, bool> = graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), on_chosen(n)))
        .collect();
    let is_chosen = |id: &str| chosen.get(id).copied().unwrap_or(false);
    let edges: Vec<rezzer_model::Edge> = graph
        .edges
        .iter()
        .filter(|e| is_chosen(&e.from) || is_chosen(&e.to))
        .cloned()
        .collect();
    let named: std::collections::HashSet<&str> = edges
        .iter()
        .flat_map(|e| [e.from.as_str(), e.to.as_str()])
        .collect();
    Graph {
        id: graph.id.clone(),
        kind: graph.kind.clone(),
        nodes: graph
            .nodes
            .iter()
            .filter(|n| is_chosen(&n.id) || named.contains(n.id.as_str()))
            .cloned()
            .collect(),
        edges,
    }
}
