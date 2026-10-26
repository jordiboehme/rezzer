// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `## Overview`: a table of the parts, the facts as a list and a Mermaid graph of which part
//! reads from which. All text goes through the shared escaping helpers.

use std::borrow::Cow;
use std::collections::HashMap;

use rezzer_model::{Document, OverviewPart};

use crate::Selection;
use crate::blocks::{cell_text, line_text, pipe_table};
use crate::facets::role_name;
use crate::mermaid::{self, MAX_EDGES, MAX_NODES};
use crate::split::{TITLE_CAP, capped};

/// The table shows at most this many parts; a line says how many were left out.
const MAX_PARTS: usize = MAX_NODES;

/// A "Reads from" cell names at most this many parts, then how many more.
const MAX_READS: usize = 20;

/// With a selection, the size of every sheet part that is not chosen ends with
/// `, not selected`; other parts are never marked.
pub(crate) fn render(doc: &Document, selection: Option<&Selection>) -> Option<String> {
    render_with(doc, selection, Limit::Plain)
}

/// The whole overview, for the `overview` part of a split document (P5-V): every row, fact
/// and "Reads from" name, no text cut. Only the graph keeps its limits; the table holds what
/// it draws.
pub(crate) fn render_whole(doc: &Document, selection: Option<&Selection>) -> Option<String> {
    render_with(doc, selection, Limit::Whole)
}

/// How much of the overview a render shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limit {
    /// One file: at most [`MAX_PARTS`] rows and [`MAX_READS`] names a cell.
    Plain,
    /// A split document's main document: at most this many entries, texts cut.
    Capped(usize),
    /// A split document's `overview` part: everything.
    Whole,
}

impl Limit {
    fn rows(self) -> usize {
        match self {
            Limit::Plain => MAX_PARTS,
            Limit::Capped(c) => c,
            Limit::Whole => usize::MAX,
        }
    }

    fn reads(self) -> usize {
        match self {
            Limit::Plain => MAX_READS,
            Limit::Capped(c) => c.min(MAX_READS),
            Limit::Whole => usize::MAX,
        }
    }

    fn facts(self) -> usize {
        match self {
            Limit::Capped(c) => c,
            _ => usize::MAX,
        }
    }

    fn graph(self) -> (usize, usize) {
        match self {
            Limit::Capped(c) => (c, c),
            _ => (MAX_NODES, MAX_EDGES),
        }
    }
}

/// The overview of a split document's main document (decision 13): at most `cap` parts,
/// facts, names in a "Reads from" cell and arrows, each text cut to [`TITLE_CAP`]
/// characters, then a line or item that says how many more there are.
pub(crate) fn render_capped(
    doc: &Document,
    selection: Option<&Selection>,
    cap: usize,
) -> Option<String> {
    render_with(doc, selection, Limit::Capped(cap))
}

fn render_with(doc: &Document, selection: Option<&Selection>, limit: Limit) -> Option<String> {
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
        let shown = limit.rows();
        let rows: Vec<Vec<String>> = parts
            .iter()
            .take(shown)
            .map(|p| {
                let unselected = selection.is_some_and(|s| s.left_out(&p.id));
                row(p, &names, unselected, limit)
            })
            .collect();
        let mut table = pipe_table(&header, &[], &rows);
        if matches!(limit, Limit::Capped(_)) && parts.len() > shown {
            table.push_str(&format!("\n\nAnd {} more rows.", parts.len() - shown));
        } else if parts.len() > shown {
            table.push_str(&format!(
                "\n\nOnly the first {MAX_PARTS} of {} parts are shown.",
                parts.len()
            ));
        }
        blocks.push(table);
    }
    if !overview.facts.is_empty() {
        let shown = limit.facts();
        let mut lines: Vec<String> = overview
            .facts
            .iter()
            .take(shown)
            .map(|(k, v)| {
                format!(
                    "- {}: {}",
                    line_text(&cut(k, limit)),
                    line_text(&cut(v, limit))
                )
            })
            .collect();
        if overview.facts.len() > shown {
            lines.push(format!("- and {} more", overview.facts.len() - shown));
        }
        blocks.push(lines.join("\n"));
    }
    if any_reads {
        let (nodes, edges) = limit.graph();
        blocks.push(graph(parts, nodes, edges, limit));
    }
    Some(blocks.join("\n\n"))
}

/// `text` cut to [`TITLE_CAP`] characters when the overview is capped.
fn cut(text: &str, limit: Limit) -> Cow<'_, str> {
    match limit {
        Limit::Capped(_) => Cow::Owned(capped(text, TITLE_CAP)),
        _ => Cow::Borrowed(text),
    }
}

fn row(
    part: &OverviewPart,
    names: &HashMap<&str, &str>,
    unselected: bool,
    limit: Limit,
) -> Vec<String> {
    let most = limit.reads();
    let mut reads: Vec<String> = part
        .reads
        .iter()
        .take(most)
        .map(|r| cell_text(&cut(names.get(r.as_str()).copied().unwrap_or(r), limit)))
        .collect();
    if part.reads.len() > most {
        reads.push(format!("and {} more", part.reads.len() - most));
    }
    vec![
        cell_text(&cut(&part.name, limit)),
        cell_text(&cut(&part.kind, limit)),
        if unselected {
            cell_text(&format!("{}, not selected", cut(&part.size, limit)))
        } else {
            cell_text(&cut(&part.size, limit))
        },
        part.role.map(role_name).unwrap_or("").to_string(),
        reads.join(", "),
    ]
}

/// Parts as nodes, an arrow from a part to every part that reads from it; at most
/// `max_nodes` nodes and `max_edges` arrows, the names cut as in a capped overview.
fn graph(parts: &[OverviewPart], max_nodes: usize, max_edges: usize, limit: Limit) -> String {
    let shown = parts.len().min(max_nodes);
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, p) in parts[..shown].iter().enumerate() {
        index.entry(p.id.as_str()).or_insert(i);
    }
    let mut lines = vec!["flowchart LR".to_string()];
    for (i, p) in parts[..shown].iter().enumerate() {
        lines.push(format!(
            "    p{i}[\"{}\"]",
            mermaid::label(&cut(&p.name, limit))
        ));
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
            if drawn >= max_edges {
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
            "\nOnly the first {max_edges} arrows are shown: {over} more are left out."
        ));
    }
    if parts.len() > shown {
        // Say that it is the graph that is cut, so it never reads as if the table were.
        out.push_str(&format!(
            "\nThe graph shows only the first {shown} of {} parts.",
            parts.len()
        ));
        if matches!(limit, Limit::Whole) {
            out.push_str(" The table lists them all.");
        }
    }
    out
}
