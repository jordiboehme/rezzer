// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The edge score: reads the edges of Mermaid diagrams and compares them with the edges of the
//! ground truth, with their direction.

use std::collections::{BTreeSet, HashMap};

/// How an edge is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dir {
    Directed,
    Both,
    Undirected,
}

/// An edge between two normalised labels. For `Both` and `Undirected` the ends are sorted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub dir: Dir,
}

impl Edge {
    fn new(from: String, to: String, dir: Dir) -> Self {
        if dir != Dir::Directed && to < from {
            Edge {
                from: to,
                to: from,
                dir,
            }
        } else {
            Edge { from, to, dir }
        }
    }
}

/// The edges of a ground truth.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TruthEdges {
    pub edges: Vec<Edge>,
    /// `(label, from, to)` for `from -->|label| to`: a label a model may draw as its own node.
    pub arrow_labels: Vec<(String, String, String)>,
    /// `(from, to, text)`: arrow text of a truth with a node map. It is not collapsed.
    pub edge_text: Vec<(String, String, String)>,
    /// The normalised node labels of a truth with a node map. When present, predicted labels
    /// are matched against these.
    pub node_labels: Option<Vec<String>>,
}

/// The score of the edges of one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EdgeScore {
    pub truth: u32,
    pub predicted: u32,
    pub correct: u32,
}

/// Lowercases a label and keeps only `[a-z0-9äöüß ]`.
fn norm_label(s: &str) -> String {
    let s = s
        .replace("<br/>", " ")
        .replace("<br />", " ")
        .replace("<br>", " ");
    let s = s.trim();
    let s = s.trim_start_matches(['[', '(', '{', '>']);
    let s = s.trim_end_matches([']', ')', '}']);
    let s = s.trim_matches(|c| c == '"' || c == '\'' || c == ' ');
    let kept: String = s
        .to_lowercase()
        .chars()
        .filter(|c| matches!(c, 'a'..='z' | '0'..='9' | 'ä' | 'ö' | 'ü' | 'ß' | ' '))
        .collect();
    kept.trim().to_string()
}

fn unescape_html(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// The sources of all ```` ```mermaid ```` blocks of a Markdown page. A block that is not closed
/// runs to the end of the page.
fn mermaid_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in markdown.lines() {
        let t = line.trim();
        match &mut current {
            None => {
                if t.starts_with("```mermaid") {
                    current = Some(String::new());
                }
            }
            Some(buf) => {
                if t.starts_with("```") {
                    blocks.push(unescape_html(buf));
                    current = None;
                } else {
                    buf.push_str(line);
                    buf.push('\n');
                }
            }
        }
    }
    if let Some(buf) = current {
        blocks.push(unescape_html(&buf));
    }
    blocks
}

/// All edges of all mermaid blocks in a Markdown page, sorted and without duplicates.
pub fn mermaid_edges(markdown: &str) -> Vec<Edge> {
    let all: BTreeSet<Edge> = mermaid_blocks(markdown)
        .iter()
        .flat_map(|b| parse_mermaid(b))
        .collect();
    all.into_iter().collect()
}

/// The edges of one mermaid source, sorted and without duplicates.
pub fn parse_mermaid(src: &str) -> Vec<Edge> {
    let first = src
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("%%"))
        .unwrap_or("")
        .to_lowercase();
    let set: BTreeSet<Edge> = if first.starts_with("sequencediagram") {
        parse_sequence(src)
    } else if first.starts_with("pie") {
        BTreeSet::new()
    } else {
        parse_flowchart(src)
    };
    set.into_iter().collect()
}

fn first_word(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or("")
}

// ---- flowcharts ----

const ARROWS: [&str; 10] = [
    "<-->", "<-.->", "<==>", "-->", "==>", "-.->", "---", "-.-", "->>", "->",
];

/// Rewrites `-- text -->` as `-->|text|`.
fn rewrite_text_arrows(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let mut replaced = false;
        let mut from = 0;
        while let Some(rel) = rest[from..].find("--") {
            let i = from + rel;
            let after = &rest[i + 2..];
            if let Some(end) = after.find("-->") {
                let text = &after[..end];
                let t = text.trim();
                let after_dash = rest[..i].ends_with('-');
                if !after_dash
                    && !t.is_empty()
                    && !t.starts_with('-')
                    && !t.ends_with('-')
                    && !text.contains("--")
                    && !text.contains(['>', '|', '[', ']'])
                {
                    out.push_str(&rest[..i]);
                    out.push_str("-->|");
                    out.push_str(t);
                    out.push('|');
                    rest = &after[end + 3..];
                    replaced = true;
                    break;
                }
            }
            from = i + 1;
        }
        if !replaced {
            out.push_str(rest);
            return out;
        }
    }
}

/// Splits a statement into node pieces and arrows, ignoring arrows inside brackets and quotes.
fn split_arrows(line: &str) -> (Vec<&str>, Vec<&'static str>) {
    let mut nodes = Vec::new();
    let mut arrows = Vec::new();
    let mut depth = 0usize;
    let mut quote = false;
    let mut last = 0;
    let mut i = 0;
    while i < line.len() {
        let c = line[i..].chars().next().unwrap_or(' ');
        match c {
            '"' => quote = !quote,
            '[' | '(' | '{' if !quote => depth += 1,
            ']' | ')' | '}' if !quote => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0
            && !quote
            && let Some(a) = ARROWS.iter().find(|a| line[i..].starts_with(**a))
        {
            nodes.push(&line[last..i]);
            arrows.push(*a);
            let mut j = i + a.len();
            let tail = line[j..].trim_start();
            j = line.len() - tail.len();
            if tail.starts_with('|')
                && let Some(close) = tail[1..].find('|')
            {
                j += close + 2;
                j = line.len() - line[j..].trim_start().len();
            }
            last = j;
            i = j;
            continue;
        }
        i += c.len_utf8();
    }
    nodes.push(&line[last..]);
    (nodes, arrows)
}

/// The end of the bracket group that starts at the beginning of `s`.
fn bracket_group(s: &str) -> Option<&str> {
    if let Some(r) = s.strip_prefix('>') {
        return r.find(']').map(|e| &s[..e + 2]);
    }
    let mut depth = 0usize;
    let mut quote = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quote = !quote,
            '[' | '(' | '{' if !quote => depth += 1,
            ']' | ')' | '}' if !quote => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&s[..i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// A node piece like `db[(DB)]` as its id and its label, if it has one.
fn parse_node(piece: &str) -> Option<(String, Option<String>)> {
    let piece = piece.trim();
    let id_len = piece
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(piece.len());
    if id_len == 0 {
        return None;
    }
    let id = &piece[..id_len];
    let rest = piece[id_len..].trim_start();
    let label = if rest.starts_with(['[', '(', '{', '>']) {
        bracket_group(rest)
            .map(norm_label)
            .filter(|l| !l.is_empty())
    } else {
        None
    };
    Some((id.to_string(), label))
}

fn parse_flowchart(src: &str) -> BTreeSet<Edge> {
    let mut labels: HashMap<String, String> = HashMap::new();
    let mut raw: Vec<(String, String, &'static str)> = Vec::new();
    for stmt in src.split(['\n', ';']) {
        let line = rewrite_text_arrows(stmt.trim());
        let line = line.trim();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        if matches!(
            first_word(line),
            "graph" | "flowchart" | "style" | "classDef" | "class" | "subgraph" | "end"
        ) {
            continue;
        }
        let (nodes, arrows) = split_arrows(line);
        let mut ids: Vec<Option<String>> = Vec::new();
        for n in nodes {
            match parse_node(n) {
                Some((id, label)) => {
                    let entry = labels.entry(id.clone()).or_insert_with(|| norm_label(&id));
                    if let Some(l) = label {
                        *entry = l;
                    }
                    ids.push(Some(id));
                }
                None => ids.push(None),
            }
        }
        for (i, a) in arrows.iter().enumerate() {
            if let (Some(Some(f)), Some(Some(t))) = (ids.get(i), ids.get(i + 1)) {
                raw.push((f.clone(), t.clone(), a));
            }
        }
    }
    let label_of = |id: &str| labels.get(id).cloned().unwrap_or_else(|| norm_label(id));
    raw.into_iter()
        .map(|(f, t, a)| {
            let dir = if a.starts_with('<') {
                Dir::Both
            } else if a == "---" || a == "-.-" {
                Dir::Undirected
            } else {
                Dir::Directed
            };
            Edge::new(label_of(&f), label_of(&t), dir)
        })
        .collect()
}

// ---- sequence diagrams ----

const SEQ_ARROWS: [&str; 8] = ["-->>", "->>", "-->", "->", "--x", "-x", "--)", "-)"];

fn parse_sequence(src: &str) -> BTreeSet<Edge> {
    let mut alias: HashMap<String, String> = HashMap::new();
    let lines: Vec<&str> = src.split(['\n', ';']).map(str::trim).collect();
    for line in &lines {
        let mut words = line.splitn(2, char::is_whitespace);
        if matches!(words.next(), Some("participant" | "actor"))
            && let Some(rest) = words.next()
        {
            let rest = rest.trim();
            let (id, name) = match rest.split_once(char::is_whitespace) {
                Some((id, tail)) => {
                    let tail = tail.trim_start();
                    match tail
                        .strip_prefix("as")
                        .filter(|t| t.starts_with(char::is_whitespace))
                    {
                        Some(n) => (id, n.trim()),
                        None => continue,
                    }
                }
                None => (rest, rest),
            };
            alias.insert(id.to_string(), norm_label(name));
        }
    }
    let name_of = |s: &str| {
        let s = s.trim();
        alias.get(s).cloned().unwrap_or_else(|| norm_label(s))
    };
    let mut edges = BTreeSet::new();
    for line in lines {
        let word = first_word(line).to_lowercase();
        if matches!(
            word.as_str(),
            "note" | "loop" | "alt" | "end" | "activate" | "deactivate" | "participant" | "actor"
        ) {
            continue;
        }
        let found = line.match_indices('-').find_map(|(d, _)| {
            let left = &line[..d];
            if left.trim().is_empty() || left.contains(['<', '>', ':']) {
                return None;
            }
            SEQ_ARROWS
                .iter()
                .find(|a| line[d..].starts_with(**a))
                .map(|a| (d, *a))
        });
        let Some((dash, arrow)) = found else { continue };
        let left = &line[..dash];
        let mut right = line[dash + arrow.len()..].trim_start();
        right = right.strip_prefix(['+', '-']).unwrap_or(right).trim_start();
        let right = right.split(':').next().unwrap_or("").trim();
        if right.is_empty() {
            continue;
        }
        edges.insert(Edge::new(name_of(left), name_of(right), Dir::Directed));
    }
    edges
}

// ---- truth ----

/// Splits `a --> b`, `a <--> b` or `a -->|label| b`. The label is `None` for the plain forms.
fn split_truth_line(line: &str) -> Option<(String, Dir, Option<String>, String)> {
    let line = line.trim();
    if let Some(i) = line.find("-->|") {
        let a = line[..i].trim();
        let rest = &line[i + 4..];
        let j = rest.find('|')?;
        let (label, b) = (&rest[..j], rest[j + 1..].trim());
        if a.is_empty() || label.is_empty() || b.is_empty() {
            return None;
        }
        return Some((a.into(), Dir::Directed, Some(label.into()), b.into()));
    }
    let i = line.find("-->")?;
    let (start, dir) = if i > 0 && line.as_bytes()[i - 1] == b'<' {
        (i - 1, Dir::Both)
    } else {
        (i, Dir::Directed)
    };
    let (a, b) = (line[..start].trim(), line[i + 3..].trim());
    if a.is_empty() || b.is_empty() {
        return None;
    }
    Some((a.into(), dir, None, b.into()))
}

/// Reads a truth file: one edge per line, as `a --> b`, `a <--> b` or `a -->|label| b`.
pub fn parse_truth(text: &str) -> TruthEdges {
    let mut set = BTreeSet::new();
    let mut arrow_labels = Vec::new();
    for line in text.lines() {
        let Some((a, dir, label, b)) = split_truth_line(line) else {
            continue;
        };
        let (a, b) = (norm_label(&a), norm_label(&b));
        if let Some(l) = label {
            arrow_labels.push((norm_label(&l), a.clone(), b.clone()));
        }
        set.insert(Edge::new(a, b, dir));
    }
    TruthEdges {
        edges: set.into_iter().collect(),
        arrow_labels,
        ..TruthEdges::default()
    }
}

/// Reads a truth whose nodes are named in a separate text (`ID<TAB>label` per line) and whose
/// edges use the ids, as `A -->|label| B`. Arrow text is kept as edge text, not as a label to
/// collapse.
pub fn parse_truth_with_nodes(edges: &str, nodes: &str) -> TruthEdges {
    let ids: HashMap<&str, String> = nodes
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_once('\t'))
        .map(|(id, label)| (id.trim(), norm_label(label)))
        .collect();
    let name = |id: &str| ids.get(id).cloned().unwrap_or_else(|| norm_label(id));
    let mut set = BTreeSet::new();
    let mut edge_text = Vec::new();
    for line in edges.lines() {
        let Some((a, dir, label, b)) = split_truth_line(line) else {
            continue;
        };
        let (a, b) = (name(&a), name(&b));
        if let Some(l) = label {
            edge_text.push((a.clone(), b.clone(), norm_label(&l)));
        }
        set.insert(Edge::new(a, b, dir));
    }
    let labels: BTreeSet<String> = ids.values().cloned().collect();
    TruthEdges {
        edges: set.into_iter().collect(),
        arrow_labels: Vec::new(),
        edge_text,
        node_labels: Some(labels.into_iter().collect()),
    }
}

// ---- scoring ----

fn dice(a: &str, b: &str) -> f64 {
    let ta: BTreeSet<&str> = a.split_whitespace().collect();
    let tb: BTreeSet<&str> = b.split_whitespace().collect();
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    2.0 * ta.intersection(&tb).count() as f64 / (ta.len() + tb.len()) as f64
}

/// The key with the highest Dice coefficient, if it is at least 0.5.
fn best_dice<'a>(label: &str, keys: impl Iterator<Item = &'a String>) -> Option<&'a String> {
    keys.map(|k| (dice(label, k), k))
        .max_by(|x, y| x.0.total_cmp(&y.0).then_with(|| x.1.cmp(y.1)))
        .filter(|(d, _)| *d >= 0.5)
        .map(|(_, k)| k)
}

fn canon(label: &str, keys: &BTreeSet<String>, node_labels: Option<&[String]>) -> String {
    if let Some(nodes) = node_labels {
        return best_dice(label, nodes.iter()).map_or_else(|| label.to_string(), Clone::clone);
    }
    let padded = format!(" {label} ");
    let squashed = label.replace(' ', "");
    let contained = keys
        .iter()
        .filter(|k| {
            !k.is_empty()
                && (padded.contains(&format!(" {k} "))
                    || (k.chars().count() > 3 && squashed.contains(&k.replace(' ', ""))))
        })
        .max_by(|a, b| a.len().cmp(&b.len()).then_with(|| b.cmp(a)));
    contained
        .or_else(|| best_dice(label, keys.iter()))
        .map_or_else(|| label.to_string(), Clone::clone)
}

/// The predicted edges with their labels mapped to the truth labels, and an arrow label drawn
/// as its own node collapsed into the arrow.
fn canonical_edges(predicted: &[Edge], truth: &TruthEdges) -> BTreeSet<Edge> {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for e in &truth.edges {
        keys.insert(e.from.clone());
        keys.insert(e.to.clone());
    }
    for (l, _, _) in &truth.arrow_labels {
        keys.insert(l.clone());
    }
    let nodes = truth.node_labels.as_deref();
    let mut pred: BTreeSet<Edge> = predicted
        .iter()
        .map(|e| {
            Edge::new(
                canon(&e.from, &keys, nodes),
                canon(&e.to, &keys, nodes),
                e.dir,
            )
        })
        .collect();
    for (label, a, b) in &truth.arrow_labels {
        let into = Edge::new(a.clone(), label.clone(), Dir::Directed);
        let out = Edge::new(label.clone(), b.clone(), Dir::Directed);
        if pred.contains(&into) && pred.contains(&out) {
            pred.remove(&into);
            pred.remove(&out);
            pred.insert(Edge::new(a.clone(), b.clone(), Dir::Directed));
        }
    }
    pred
}

/// Scores predicted edges against a truth. An edge is correct with the same ends and the same
/// direction, counted once. A predicted label maps to a truth label (whole word, or a squashed
/// substring for a truth label longer than 3 characters, longest first, then Dice of at least
/// 0.5), and an arrow label drawn as its own node counts as the arrow.
pub fn score_edges(predicted: &[Edge], truth: &TruthEdges) -> EdgeScore {
    let pred = canonical_edges(predicted, truth);
    let want: BTreeSet<Edge> = truth
        .edges
        .iter()
        .map(|e| Edge::new(e.from.clone(), e.to.clone(), e.dir))
        .collect();
    EdgeScore {
        truth: want.len() as u32,
        predicted: pred.len() as u32,
        correct: pred.intersection(&want).count() as u32,
    }
}

/// How the node labels of a predicted diagram compare with a truth.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeCheck {
    /// Truth node labels that no predicted label maps to: misread or missing labels.
    pub misread: Vec<String>,
    /// Predicted labels that map to no truth label (arrow labels count as truth labels).
    pub unknown: Vec<String>,
    /// Predicted edges between two truth nodes that the truth does not join in any direction.
    pub invented_arrows: Vec<Edge>,
}

impl NodeCheck {
    /// Nodes the model added: unknown labels beyond the one each misread label explains.
    pub fn invented_nodes(&self) -> usize {
        self.unknown.len().saturating_sub(self.misread.len())
    }
}

/// Compares the node labels and the joins of predicted edges with a truth, with the label
/// mapping of [`score_edges`]. A reversed arrow that replaces the right one is a wrong arrow,
/// not an invented one; an extra arrow next to the right one is invented.
pub fn check_nodes(predicted: &[Edge], truth: &TruthEdges) -> NodeCheck {
    let pred = canonical_edges(predicted, truth);
    let truth_nodes: BTreeSet<&str> = truth
        .edges
        .iter()
        .flat_map(|e| [e.from.as_str(), e.to.as_str()])
        .collect();
    let arrow_labels: BTreeSet<&str> = truth
        .arrow_labels
        .iter()
        .map(|(l, _, _)| l.as_str())
        .collect();
    let pred_nodes: BTreeSet<&str> = pred
        .iter()
        .flat_map(|e| [e.from.as_str(), e.to.as_str()])
        .collect();
    // The truth edges per pair of nodes, in any direction.
    let pair = |e: &Edge| {
        if e.from <= e.to {
            (e.from.clone(), e.to.clone())
        } else {
            (e.to.clone(), e.from.clone())
        }
    };
    let mut truth_pairs: HashMap<(String, String), usize> = HashMap::new();
    for t in &truth.edges {
        *truth_pairs.entry(pair(t)).or_default() += 1;
    }
    let mut pred_pairs: HashMap<(String, String), usize> = HashMap::new();
    for p in &pred {
        *pred_pairs.entry(pair(p)).or_default() += 1;
    }
    let want: BTreeSet<&Edge> = truth.edges.iter().collect();
    // An arrow between two truth nodes is invented when the truth does not join them, or when
    // the pair has more arrows than the truth and this one is not a truth arrow. A reversed arrow
    // that replaces the right one is wrong, not invented.
    let invented = |e: &Edge| {
        let k = pair(e);
        match truth_pairs.get(&k) {
            None => true,
            Some(&n) => pred_pairs[&k] > n && !want.contains(e),
        }
    };
    NodeCheck {
        misread: truth_nodes
            .iter()
            .filter(|n| !pred_nodes.contains(*n))
            .map(|n| n.to_string())
            .collect(),
        unknown: pred_nodes
            .iter()
            .filter(|n| !truth_nodes.contains(*n) && !arrow_labels.contains(*n))
            .map(|n| n.to_string())
            .collect(),
        invented_arrows: pred
            .iter()
            .filter(|e| {
                truth_nodes.contains(e.from.as_str())
                    && truth_nodes.contains(e.to.as_str())
                    && invented(e)
            })
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(a: &str, b: &str, dir: Dir) -> Edge {
        Edge::new(a.into(), b.into(), dir)
    }

    fn fenced(src: &str) -> String {
        format!("text\n\n```mermaid\n{src}\n```\n")
    }

    const TEST2_TRUTH: &str =
        "search --> Learn\nLearn --> teach\nteach --> search\nApp --> DB\nMCP <--> DB\n";

    /// The Markdown a right reading of the test2 page gives.
    const TEST2_RIGHT: &str = "This note is without\\\nreal-time recognition.\n\n```mermaid\ngraph TD\n    search --> Learn\n    Learn --> teach\n    teach --> search\n```\n\n> A cyclic diagram with three nodes.\n\n```mermaid\ngraph LR\n    App --> DB\n    DB <--> MCP\n```\n\n> App points to DB, and DB and MCP are connected by a double-headed arrow.\n";

    /// What notedown's Chandra converter gives for the HTML of `edge_check_sees_wrong_arrows`:
    /// the right nodes, the cycle reversed and db -> mcp directed.
    const WRONG_ARROWS: &str = "```mermaid\ngraph TD\n    search((search)) --> teach((teach))\n    teach --> learn((learn))\n    learn --> search\n```\n\n```mermaid\ngraph LR\n    App((App)) --> DB[(DB)]\n    DB --> MCP[MCP]\n```\n";

    #[test]
    fn test2_right_arrows() {
        let mut edges = mermaid_edges(TEST2_RIGHT);
        edges.sort();
        let mut want = vec![
            e("search", "learn", Dir::Directed),
            e("learn", "teach", Dir::Directed),
            e("teach", "search", Dir::Directed),
            e("app", "db", Dir::Directed),
            e("db", "mcp", Dir::Both),
        ];
        want.sort();
        assert_eq!(edges, want);
        let score = score_edges(&edges, &parse_truth(TEST2_TRUTH));
        assert_eq!(
            score,
            EdgeScore {
                truth: 5,
                predicted: 5,
                correct: 5
            }
        );
    }

    #[test]
    fn wrong_arrows_are_seen() {
        let edges = mermaid_edges(WRONG_ARROWS);
        assert_eq!(edges.len(), 5);
        let score = score_edges(&edges, &parse_truth(TEST2_TRUTH));
        assert_eq!(score.correct, 1);
        assert_eq!(score.truth, 5);
        assert_eq!(score.predicted, 5);
        assert!(!edges.contains(&e("db", "mcp", Dir::Both)));
    }

    #[test]
    fn semicolons_and_br() {
        let edges = mermaid_edges(&fenced("graph TD; A[Notedown<br/>CLI] --> B[(Server)];"));
        assert_eq!(edges, vec![e("notedown cli", "server", Dir::Directed)]);
    }

    #[test]
    fn edge_label_as_node_collapses() {
        let truth = parse_truth("md -->|mcp| claude ai agent\n");
        let edges = mermaid_edges(&fenced(
            "graph LR\n  A[md] --> B[mcp] --> C[claude ai agent]",
        ));
        assert_eq!(edges.len(), 2);
        let score = score_edges(&edges, &truth);
        assert_eq!(
            score,
            EdgeScore {
                truth: 1,
                predicted: 1,
                correct: 1
            }
        );
    }

    #[test]
    fn text_arrow() {
        let edges = mermaid_edges(&fenced("graph LR\n  A -- uses --> B"));
        assert_eq!(edges, vec![e("a", "b", Dir::Directed)]);
    }

    #[test]
    fn sequence_diagram() {
        let edges = mermaid_edges(&fenced(
            "sequenceDiagram; participant U as User; U->>UI: click; UI-->>U: done",
        ));
        assert_eq!(
            edges,
            vec![
                e("ui", "user", Dir::Directed),
                e("user", "ui", Dir::Directed)
            ]
        );
    }

    #[test]
    fn fuzzy_label() {
        let truth = parse_truth("notedown --> ocr model\n");
        let edges = mermaid_edges(&fenced("graph LR\n  A[Notedown] --> B[OCR model (qwen)]"));
        assert_eq!(edges, vec![e("notedown", "ocr model qwen", Dir::Directed)]);
        assert_eq!(score_edges(&edges, &truth).correct, 1);
    }

    #[test]
    fn squashed_label_matches_longer_truth_labels_only() {
        let truth = parse_truth("notedown <--> supernote\n");
        let edges = mermaid_edges(&fenced("graph LR\n  A[Notedown] <--> B[Super Note]"));
        assert_eq!(score_edges(&edges, &truth).correct, 1);
        // "ocr" has 3 characters: no squashing inside an unrelated word
        let truth = parse_truth("notedown --> ocr\n");
        let edges = mermaid_edges(&fenced("graph LR\n  A[Notedown] --> B[Soc Rate]"));
        assert_eq!(score_edges(&edges, &truth).correct, 0);
    }

    #[test]
    fn mixed_arrow_chain_keeps_both_edges() {
        let edges = mermaid_edges(&fenced("graph TD; A --- B --> C"));
        assert_eq!(
            edges,
            vec![e("a", "b", Dir::Undirected), e("b", "c", Dir::Directed)]
        );
    }

    #[test]
    fn hyphens_in_text_arrow_and_participant() {
        let edges = mermaid_edges(&fenced("graph LR\n  A -- e-mail --> B"));
        assert_eq!(edges, vec![e("a", "b", Dir::Directed)]);
        let edges = mermaid_edges(&fenced("sequenceDiagram\n  Web-Server->>DB: query"));
        assert_eq!(edges, vec![e("webserver", "db", Dir::Directed)]);
    }

    #[test]
    fn pie_has_no_edges() {
        let edges = mermaid_edges(&fenced("pie title Pets\n  \"Dogs\" : 5\n  \"Cats\" : 3"));
        assert!(edges.is_empty());
    }

    #[test]
    fn arrow_kinds_and_node_ids_resolved_anywhere() {
        let src = "graph TD\n  A --- B\n  B <--> C\n  C -.-> D\n  D ==> E\n  A[First]\n  endpoint --> A\n";
        let edges = mermaid_edges(&fenced(src));
        assert!(edges.contains(&e("first", "b", Dir::Undirected)));
        assert!(edges.contains(&e("b", "c", Dir::Both)));
        assert!(edges.contains(&e("c", "d", Dir::Directed)));
        assert!(edges.contains(&e("d", "e", Dir::Directed)));
        assert!(edges.contains(&e("endpoint", "first", Dir::Directed)));
    }

    #[test]
    fn labelled_arrow_and_skipped_statements() {
        let src = "flowchart LR\n  %% a comment\n  style A fill:#fff\n  subgraph S\n  A -->|via| B\n  end";
        let edges = mermaid_edges(&fenced(src));
        assert_eq!(edges, vec![e("a", "b", Dir::Directed)]);
    }

    #[test]
    fn truth_with_node_map() {
        let nodes = "n1\tAlpha Service\nn2\tBeta Store\n";
        let truth = parse_truth_with_nodes("n1 -->|sync| n2\n", nodes);
        assert_eq!(
            truth.edges,
            vec![e("alpha service", "beta store", Dir::Directed)]
        );
        assert!(truth.arrow_labels.is_empty());
        assert_eq!(
            truth.edge_text,
            vec![("alpha service".into(), "beta store".into(), "sync".into())]
        );
        assert_eq!(
            truth.node_labels,
            Some(vec!["alpha service".to_string(), "beta store".to_string()])
        );
        let edges = mermaid_edges(&fenced(
            "graph LR\n  A[Alpha Service API] --> B[The Beta Store]",
        ));
        assert_eq!(score_edges(&edges, &truth).correct, 1);
        let unrelated = mermaid_edges(&fenced("graph LR\n  A[Gamma] --> B[Beta Store]"));
        assert_eq!(score_edges(&unrelated, &truth).correct, 0);
    }

    #[test]
    fn truth_parsing() {
        let t = parse_truth("a <--> b\nb --> a\nx -->|mcp| y\nnot an edge\n");
        assert_eq!(
            t.edges,
            vec![
                e("a", "b", Dir::Both),
                e("b", "a", Dir::Directed),
                e("x", "y", Dir::Directed)
            ]
        );
        assert_eq!(t.arrow_labels, vec![("mcp".into(), "x".into(), "y".into())]);
    }

    /// From notedown's `edge_check_sees_wrong_arrows`: Chandra's raw Mermaid for test2 from a
    /// newer llama.cpp, HTML-escaped and on one line each, with the right nodes and mostly wrong
    /// arrows.
    #[test]
    fn edge_check_sees_wrong_arrows() {
        let md = "```mermaid\ngraph TD; search((search)) --&gt; teach((teach)); teach --&gt; learn((learn)); learn --&gt; search;\n```\n\n```mermaid\ngraph LR; App((App)) --&gt; DB[(DB)]; DB --&gt; MCP[MCP];\n```\n";
        let edges = mermaid_edges(md);
        let mut want = vec![
            e("search", "teach", Dir::Directed),
            e("teach", "learn", Dir::Directed),
            e("learn", "search", Dir::Directed),
            e("app", "db", Dir::Directed),
            e("db", "mcp", Dir::Directed),
        ];
        want.sort();
        assert_eq!(edges, want);
        assert!(!edges.contains(&e("db", "mcp", Dir::Both)));
        let truth = parse_truth(TEST2_TRUTH);
        assert_eq!(score_edges(&edges, &truth).correct, 1);
        // The nodes are right, the arrows are wrong: nothing is invented.
        let check = check_nodes(&edges, &truth);
        assert!(
            check.misread.is_empty() && check.unknown.is_empty(),
            "{check:?}"
        );
        assert!(check.invented_arrows.is_empty(), "{check:?}");
    }

    /// An arrow added next to a right one between the same two nodes is invented: test2's cycle
    /// as Qwen read it from a crop, with all three truth arrows and two reversed extras.
    #[test]
    fn extra_arrow_beside_a_right_one_is_invented() {
        let md = fenced(
            "graph TD\n  A[search] --> B[teach]\n  B --> A\n  C[Learn] --> B\n  A --> C\n  C --> A",
        );
        let check = check_nodes(&mermaid_edges(&md), &parse_truth(TEST2_TRUTH));
        assert_eq!(
            check.invented_arrows,
            [
                e("learn", "search", Dir::Directed),
                e("search", "teach", Dir::Directed)
            ]
        );
    }

    #[test]
    fn node_check_counts_misread_and_invented() {
        let truth =
            parse_truth("notedown --> ocr model\nocr model --> md\nmd -->|mcp| claude ai agent\n");
        // All right, mcp drawn as a node.
        let right = mermaid_edges(&fenced(
            "graph LR\n  A[Notedown] --> B[OCR Model] --> C[.md] --> D[MCP] --> E[Claude AI Agent]",
        ));
        assert_eq!(check_nodes(&right, &truth), NodeCheck::default());
        // One misread label: "OCR Model" read as "QCB Mdl".
        let misread = mermaid_edges(&fenced(
            "graph LR\n  A[Notedown] --> B[QCB Mdl] --> C[md] -->|mcp| E[Claude AI Agent]",
        ));
        let check = check_nodes(&misread, &truth);
        assert_eq!(check.misread, ["ocr model"]);
        assert_eq!(check.unknown, ["qcb mdl"]);
        assert_eq!(check.invented_nodes(), 0);
        // An invented node and an invented arrow.
        let invented = mermaid_edges(&fenced(
            "graph LR\n  A[Notedown] --> B[OCR Model] --> C[md] -->|mcp| E[Claude AI Agent]\n  E --> F[Database]\n  A --> C",
        ));
        let check = check_nodes(&invented, &truth);
        assert!(check.misread.is_empty());
        assert_eq!(check.invented_nodes(), 1);
        assert_eq!(check.invented_arrows, [e("notedown", "md", Dir::Directed)]);
        // A reversed arrow is wrong but not invented.
        let reversed = mermaid_edges(&fenced("graph LR\n  A[OCR Model] --> B[Notedown]"));
        assert!(check_nodes(&reversed, &truth).invented_arrows.is_empty());
    }
}
