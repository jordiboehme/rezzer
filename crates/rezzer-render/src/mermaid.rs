// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Graphs as Mermaid `flowchart LR` fences.

use std::collections::{HashMap, HashSet};

use rezzer_model::Graph;

use crate::blocks::{heading_line, line_text};

/// A graph with more nodes shows only its first nodes and says how many were left out.
pub(crate) const MAX_NODES: usize = 200;

/// At most this many edges are drawn; a line says how many more there are.
pub(crate) const MAX_EDGES: usize = 1_000;

/// A Mermaid id: `n_` plus the id with every character outside `[A-Za-z0-9_]` as `_`. The
/// prefix keeps ids clear of keywords such as `end` and of leading digits.
fn sanitize(id: &str) -> String {
    let mut out = String::from("n_");
    out.extend(id.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '_' {
            c
        } else {
            '_'
        }
    }));
    out
}

/// A label for inside `"..."`: characters Mermaid would read as markup become entity codes.
fn label(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("#quot;"),
            '#' => out.push_str("#35;"),
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            '`' => out.push_str("#96;"),
            '%' => out.push_str("#37;"),
            '\n' | '\r' => out.push(' '),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// `### Graph: <id>`, a sentence, the fence and, when something is left out, a line saying
/// what. The sentence, fence and notes have no blank line between them, so paging never
/// separates them.
pub(crate) fn render(graph: &Graph) -> String {
    let mut ids: HashMap<&str, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut lines = vec!["flowchart LR".to_string()];
    let shown = graph.nodes.len().min(MAX_NODES);
    for node in &graph.nodes[..shown] {
        if ids.contains_key(node.id.as_str()) {
            // A repeated node id is the same node.
            continue;
        }
        let base = sanitize(&node.id);
        let mut id = base.clone();
        let mut n = 2;
        while used.contains(&id) {
            id = format!("{base}_{n}");
            n += 1;
        }
        used.insert(id.clone());
        lines.push(format!("    {id}[\"{}\"]", label(&node.label)));
        ids.insert(node.id.as_str(), id);
    }
    let known: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut cut_edges = 0;
    let mut drawn = 0;
    let mut over_edges = 0;
    let mut dangling = 0;
    for edge in &graph.edges {
        match (ids.get(edge.from.as_str()), ids.get(edge.to.as_str())) {
            (Some(_), Some(_)) if drawn >= MAX_EDGES => over_edges += 1,
            (Some(from), Some(to)) => {
                drawn += 1;
                if edge.kind.trim().is_empty() {
                    lines.push(format!("    {from} --> {to}"));
                } else {
                    lines.push(format!("    {from} -->|\"{}\"| {to}", label(&edge.kind)));
                }
            }
            _ if known.contains(edge.from.as_str()) && known.contains(edge.to.as_str()) => {
                cut_edges += 1;
            }
            _ => dangling += 1,
        }
    }

    let mut out = heading_line(3, &format!("Graph: {}", line_text(&graph.id)));
    out.push_str("\n\n");
    out.push_str(&format!(
        "Kind {}, {}, {}.\n",
        line_text(&graph.kind),
        plural(graph.nodes.len(), "node", "nodes"),
        plural(graph.edges.len(), "edge", "edges"),
    ));
    out.push_str("```mermaid\n");
    out.push_str(&lines.join("\n"));
    out.push_str("\n```");
    if graph.nodes.len() > shown {
        out.push_str(&format!(
            "\nOnly the first {shown} of {} nodes are shown: {} and {} are left out.",
            graph.nodes.len(),
            plural(graph.nodes.len() - shown, "node", "nodes"),
            plural(cut_edges, "edge", "edges"),
        ));
    }
    if over_edges > 0 {
        out.push_str(&format!(
            "\nOnly the first {MAX_EDGES} of {} edges are shown: {} are left out.",
            drawn + over_edges,
            plural(over_edges, "edge", "edges"),
        ));
    }
    if dangling == 1 {
        out.push_str("\n1 edge points to a node that is not in the graph and is left out.");
    } else if dangling > 1 {
        out.push_str(&format!(
            "\n{dangling} edges point to nodes that are not in the graph and are left out."
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_prefixes_and_replaces() {
        assert_eq!(sanitize("end"), "n_end");
        assert_eq!(sanitize("Sheet 1!A1"), "n_Sheet_1_A1");
        assert_eq!(sanitize(""), "n_");
        assert_eq!(sanitize("é"), "n__");
    }

    #[test]
    fn label_escapes_markup() {
        assert_eq!(
            label("a \"b\" #1 <i> `c` 5%\nx"),
            "a #quot;b#quot; #35;1 #lt;i#gt; #96;c#96; 5#37; x"
        );
    }
}
