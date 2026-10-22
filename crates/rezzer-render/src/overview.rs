// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `## Overview`: a table of the parts, the facts as a list and a Mermaid graph of which part
//! reads from which. All text goes through the shared escaping helpers.

use std::collections::HashMap;

use rezzer_model::{Document, OverviewPart};

use crate::Selection;
use crate::blocks::{cell_text, line_text, pipe_table};
use crate::facets::role_name;
use crate::mermaid::{self, MAX_EDGES, MAX_NODES};

/// The table shows at most this many parts; a line says how many were left out.
const MAX_PARTS: usize = MAX_NODES;

/// A "Reads from" cell names at most this many parts, then how many more.
const MAX_READS: usize = 20;

/// With a selection, the size of every part that is not a chosen sheet ends with
/// `, not selected`.
pub(crate) fn render(doc: &Document, selection: Option<&Selection>) -> Option<String> {
    let overview = doc.overview.as_ref()?;
    if overview.parts.is_empty() && overview.facts.is_empty() {
        return None;
    }
    let mut blocks = vec!["## Overview".to_string()];
    let parts = &overview.parts;
    let any_reads = parts.iter().any(|p| !p.reads.is_empty());
    if !parts.is_empty() {
        let header: Vec<String> = ["Part", "Kind", "Size", "Role", "Reads from"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let names: HashMap<&str, &str> = parts
            .iter()
            .map(|p| (p.id.as_str(), p.name.as_str()))
            .collect();
        let rows: Vec<Vec<String>> = parts
            .iter()
            .take(MAX_PARTS)
            .map(|p| row(p, &names, selection.is_some_and(|s| !s.chosen(&p.id))))
            .collect();
        let mut table = pipe_table(&header, &[], &rows);
        if parts.len() > MAX_PARTS {
            table.push_str(&format!(
                "\n\nOnly the first {MAX_PARTS} of {} parts are shown.",
                parts.len()
            ));
        }
        blocks.push(table);
    }
    if !overview.facts.is_empty() {
        let lines: Vec<String> = overview
            .facts
            .iter()
            .map(|(k, v)| format!("- {}: {}", line_text(k), line_text(v)))
            .collect();
        blocks.push(lines.join("\n"));
    }
    if any_reads {
        blocks.push(graph(parts));
    }
    Some(blocks.join("\n\n"))
}

fn row(part: &OverviewPart, names: &HashMap<&str, &str>, unselected: bool) -> Vec<String> {
    let mut reads: Vec<String> = part
        .reads
        .iter()
        .take(MAX_READS)
        .map(|r| cell_text(names.get(r.as_str()).copied().unwrap_or(r)))
        .collect();
    if part.reads.len() > MAX_READS {
        reads.push(format!("and {} more", part.reads.len() - MAX_READS));
    }
    vec![
        cell_text(&part.name),
        cell_text(&part.kind),
        if unselected {
            cell_text(&format!("{}, not selected", part.size))
        } else {
            cell_text(&part.size)
        },
        part.role.map(role_name).unwrap_or("").to_string(),
        reads.join(", "),
    ]
}

/// Parts as nodes, an arrow from a part to every part that reads from it.
fn graph(parts: &[OverviewPart]) -> String {
    let shown = parts.len().min(MAX_NODES);
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, p) in parts[..shown].iter().enumerate() {
        index.entry(p.id.as_str()).or_insert(i);
    }
    let mut lines = vec!["flowchart LR".to_string()];
    for (i, p) in parts[..shown].iter().enumerate() {
        lines.push(format!("    p{i}[\"{}\"]", mermaid::label(&p.name)));
    }
    let mut drawn = 0;
    let mut over = 0;
    for (i, p) in parts[..shown].iter().enumerate() {
        for r in &p.reads {
            let Some(&from) = index.get(r.as_str()) else {
                continue;
            };
            if from == i {
                continue;
            }
            if drawn >= MAX_EDGES {
                over += 1;
            } else {
                drawn += 1;
                lines.push(format!("    p{from} --> p{i}"));
            }
        }
    }
    let mut out = format!("```mermaid\n{}\n```", lines.join("\n"));
    if over > 0 {
        out.push_str(&format!(
            "\nOnly the first {MAX_EDGES} arrows are shown: {over} more are left out."
        ));
    }
    if parts.len() > shown {
        out.push_str(&format!(
            "\nOnly the first {shown} of {} parts are shown.",
            parts.len()
        ));
    }
    out
}
