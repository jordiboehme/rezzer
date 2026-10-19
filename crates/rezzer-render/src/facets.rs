// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sections after the content: Logic, Annotations, Changes, Embedded and Not read.
//! Each returns `None` when it has nothing to show.

use rezzer_model::{
    Annotation, Block, Document, Formula, Inline, LossReason, Role, Severity, StatusColor, Value,
};

use crate::blocks::{self, Ctx, MAX_DEPTH, Mode, cell_text, code_span, pipe_table};
use crate::mermaid;

fn header(cols: &[&str]) -> Vec<String> {
    cols.iter().map(|c| (*c).to_string()).collect()
}

pub(crate) fn role_name(role: Role) -> &'static str {
    match role {
        Role::Input => "input",
        Role::Constant => "constant",
        Role::Result => "result",
        Role::Intermediate => "intermediate",
    }
}

fn formula_row(f: &Formula, with_note: bool) -> Vec<String> {
    let mut expr = String::new();
    code_span(&f.expression, Mode::Cell, &mut expr);
    if let Some(latex) = f.latex.as_deref().filter(|l| !l.trim().is_empty()) {
        expr.push(' ');
        expr.push_str(&blocks::inlines(
            &[Inline::Math {
                latex: latex.to_string(),
            }],
            Mode::Cell,
        ));
    }
    let reads: Vec<String> = f.references.iter().map(|r| cell_text(&r.0)).collect();
    let mut row = vec![
        cell_text(&f.anchor.0),
        expr,
        reads.join(", "),
        f.role.map(role_name).unwrap_or("").to_string(),
        f.cached
            .as_ref()
            .map(|v| cell_text(&value_text(v)))
            .unwrap_or_default(),
    ];
    if with_note {
        row.push(f.note.as_deref().map(cell_text).unwrap_or_default());
    }
    row
}

/// The dialect in words.
fn dialect_words(dialect: &str) -> String {
    match dialect {
        "excel" => "Spreadsheet formulas".into(),
        "openformula" => "OpenDocument formulas".into(),
        "derived" => "Found in the data, not written as formulas in the file".into(),
        other => blocks::line_text(other),
    }
}

/// A line naming the dialects, then the table. The `Note` column is there only when a
/// formula of the group has a note.
fn formula_table(list: &[&Formula]) -> String {
    let mut dialects: Vec<&str> = Vec::new();
    for f in list {
        if !dialects.contains(&f.dialect.as_str()) {
            dialects.push(&f.dialect);
        }
    }
    let words: Vec<String> = dialects
        .iter()
        .filter(|d| !d.trim().is_empty())
        .map(|d| dialect_words(d))
        .collect();
    let with_note = list.iter().any(|f| f.note.is_some());
    let rows: Vec<Vec<String>> = list.iter().map(|f| formula_row(f, with_note)).collect();
    let mut cols = vec!["Cells", "Formula", "Reads", "Role", "Value"];
    if with_note {
        cols.push("Note");
    }
    let table = pipe_table(&header(&cols), &[], &rows);
    if words.is_empty() {
        table
    } else {
        format!(
            "{}\n\n{table}",
            blocks::finish_lines(&format!("{}.", words.join(", ")))
        )
    }
}

/// `## Logic`: formulas without a pattern (one table per dialect), then one table per
/// pattern (in order of first appearance), then the graphs.
pub(crate) fn logic(doc: &Document) -> Option<String> {
    let formulas = &doc.facets.formulas;
    let graphs = &doc.facets.graphs;
    if formulas.is_empty() && graphs.is_empty() {
        return None;
    }
    let mut parts = vec!["## Logic".to_string()];
    let mut dialects: Vec<&str> = Vec::new();
    for f in formulas.iter().filter(|f| f.pattern.is_none()) {
        if !dialects.contains(&f.dialect.as_str()) {
            dialects.push(&f.dialect);
        }
    }
    for d in dialects {
        let single: Vec<&Formula> = formulas
            .iter()
            .filter(|f| f.pattern.is_none() && f.dialect == d)
            .collect();
        parts.push(formula_table(&single));
    }
    let mut patterns: Vec<&str> = Vec::new();
    for f in formulas {
        if let Some(p) = f.pattern.as_deref()
            && !patterns.contains(&p)
        {
            patterns.push(p);
        }
    }
    for p in patterns {
        let group: Vec<&Formula> = formulas
            .iter()
            .filter(|f| f.pattern.as_deref() == Some(p))
            .collect();
        parts.push(blocks::heading_line(
            3,
            &format!("Pattern: {}", blocks::line_text(p)),
        ));
        parts.push(formula_table(&group));
    }
    for g in graphs {
        parts.push(mermaid::render(g));
    }
    Some(parts.join("\n\n"))
}

/// A value as plain text: numbers with their unit, percent as `25 %`, lists joined with
/// `, `.
pub(crate) fn value_text(value: &Value) -> String {
    value_text_at(value, 0)
}

fn value_text_at(value: &Value, depth: usize) -> String {
    match value {
        Value::Text { value } | Value::Date { value } | Value::DateTime { value } => value.clone(),
        Value::Number { value, unit } => match unit.as_deref().filter(|u| !u.is_empty()) {
            Some(u) => format!("{} {u}", number(*value)),
            None => number(*value),
        },
        Value::Percent { value } => format!("{} %", number(*value * 100.0)),
        Value::Bool { value } => value.to_string(),
        Value::List { value } => {
            if depth >= MAX_DEPTH {
                return "...".into();
            }
            value
                .iter()
                .map(|v| value_text_at(v, depth + 1))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

/// A number without float noise such as `7.000000000000001`.
pub(crate) fn number(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let rounded = (value * 1e9).round() / 1e9;
    if rounded.is_finite() && (rounded - value).abs() < 1e-9 * value.abs().max(1.0) {
        rounded.to_string()
    } else {
        value.to_string()
    }
}

/// `## Properties`: the custom document properties.
pub(crate) fn properties(doc: &Document) -> Option<String> {
    let custom = &doc.properties.custom;
    if custom.is_empty() {
        return None;
    }
    let rows: Vec<Vec<String>> = custom
        .iter()
        .map(|(k, v)| vec![cell_text(k), cell_text(&value_text(v))])
        .collect();
    Some(format!(
        "## Properties\n\n{}",
        pipe_table(&header(&["Key", "Value"]), &[], &rows)
    ))
}

fn text(t: &str) -> Inline {
    Inline::Text {
        text: t.to_string(),
        emphasis: false,
        strong: false,
        strike: false,
    }
}

fn paragraph(inlines: Vec<Inline>) -> Block {
    Block::Paragraph {
        inlines,
        role: None,
        anchor: None,
    }
}

/// One annotation as list item blocks: who, when and where, the quoted passage, the body
/// and the replies.
fn annotation_item(a: &Annotation, depth: usize) -> Vec<Block> {
    let mut meta = vec![Inline::Text {
        text: a.kind.clone(),
        emphasis: false,
        strong: true,
        strike: false,
    }];
    if let Some(author) = a.author.as_deref().filter(|s| !s.is_empty()) {
        meta.push(text(&format!(" by {author}")));
    }
    if let Some(date) = a.date.as_deref().filter(|s| !s.is_empty()) {
        meta.push(text(&format!(" on {date}")));
    }
    if let Some(anchor) = &a.anchor {
        meta.push(text(" at "));
        meta.push(Inline::Code {
            text: anchor.0.clone(),
        });
    }
    match a.resolved {
        Some(true) => {
            meta.push(text(" "));
            meta.push(Inline::Status {
                text: "Resolved".into(),
                color: StatusColor::Green,
            });
        }
        Some(false) => {
            meta.push(text(" "));
            meta.push(Inline::Status {
                text: "Open".into(),
                color: StatusColor::Yellow,
            });
        }
        None => {}
    }
    let mut item = vec![paragraph(meta)];
    if let Some(q) = a.quoted.as_deref().filter(|q| !q.trim().is_empty()) {
        item.push(Block::Quote {
            blocks: vec![paragraph(vec![text(q)])],
            anchor: None,
        });
    }
    if !a.body.is_empty() {
        item.push(paragraph(a.body.clone()));
    }
    if !a.replies.is_empty() {
        if depth >= MAX_DEPTH {
            item.push(paragraph(vec![text(
                "Replies nested too deep: the rest is not shown.",
            )]));
        } else {
            item.push(Block::List {
                ordered: false,
                items: a
                    .replies
                    .iter()
                    .map(|r| annotation_item(r, depth + 1))
                    .collect(),
                anchor: None,
            });
        }
    }
    item
}

/// `## Annotations`: comments, notes and highlights with author, date and passage.
pub(crate) fn annotations(doc: &Document, ctx: &Ctx) -> Option<String> {
    if doc.facets.annotations.is_empty() {
        return None;
    }
    let list = Block::List {
        ordered: false,
        items: doc
            .facets
            .annotations
            .iter()
            .map(|a| annotation_item(a, 0))
            .collect(),
        anchor: None,
    };
    Some(format!(
        "## Annotations\n\n{}",
        blocks::blocks(&[list], ctx, 0, 3, false)
    ))
}

/// `## Changes`: the timeline as a table.
pub(crate) fn changes(doc: &Document) -> Option<String> {
    if doc.facets.timeline.is_empty() {
        return None;
    }
    let opt = |v: &Option<String>| v.as_deref().map(cell_text).unwrap_or_default();
    let rows: Vec<Vec<String>> = doc
        .facets
        .timeline
        .iter()
        .map(|e| {
            vec![
                opt(&e.time),
                opt(&e.actor),
                cell_text(&e.kind),
                e.anchor
                    .as_ref()
                    .map(|a| cell_text(&a.0))
                    .unwrap_or_default(),
                opt(&e.text),
            ]
        })
        .collect();
    Some(format!(
        "## Changes\n\n{}",
        pipe_table(
            &header(&["When", "Who", "What", "Where", "Text"]),
            &[],
            &rows
        )
    ))
}

/// `## Embedded`: scripts, macros, objects and references (never run or followed), then the
/// pictures the calling agent should look at.
pub(crate) fn embedded(doc: &Document) -> Option<String> {
    let vision: Vec<_> = doc.assets.iter().filter(|a| a.needs_vision).collect();
    if doc.facets.embedded.is_empty() && vision.is_empty() {
        return None;
    }
    let mut parts = vec!["## Embedded".to_string()];
    if !doc.facets.embedded.is_empty() {
        let rows: Vec<Vec<String>> = doc
            .facets
            .embedded
            .iter()
            .map(|e| {
                let mut target = String::new();
                if let Some(t) = &e.target {
                    code_span(t, Mode::Cell, &mut target);
                }
                vec![
                    cell_text(&e.kind),
                    cell_text(&e.name),
                    e.detail.as_deref().map(cell_text).unwrap_or_default(),
                    target,
                ]
            })
            .collect();
        parts.push(pipe_table(
            &header(&["Kind", "Name", "Detail", "Target"]),
            &[],
            &rows,
        ));
    }
    if !vision.is_empty() {
        parts.push("### Pictures that need vision".into());
        let rows: Vec<Vec<String>> = vision
            .iter()
            .map(|a| {
                let mut file = String::new();
                match &a.path {
                    Some(p) => code_span(p, Mode::Cell, &mut file),
                    None => file.push_str("not extracted"),
                }
                vec![
                    cell_text(&a.id),
                    file,
                    a.caption.as_deref().map(cell_text).unwrap_or_default(),
                ]
            })
            .collect();
        parts.push(pipe_table(
            &header(&["Picture", "File", "Caption"]),
            &[],
            &rows,
        ));
    }
    Some(parts.join("\n\n"))
}

fn reason(why: LossReason) -> &'static str {
    match why {
        LossReason::Unsupported => "unsupported",
        LossReason::Damaged => "damaged",
        LossReason::Limit => "limit",
        LossReason::Encrypted => "encrypted",
        LossReason::NotImplemented => "not implemented",
        LossReason::Uncertain => "uncertain",
    }
}

fn severity(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

/// `## Not read`: every loss with where, what, why and how bad.
pub(crate) fn not_read(doc: &Document) -> Option<String> {
    if doc.losses.is_empty() {
        return None;
    }
    let rows: Vec<Vec<String>> = doc
        .losses
        .iter()
        .map(|l| {
            vec![
                cell_text(&l.location),
                cell_text(&l.what),
                reason(l.why).to_string(),
                severity(l.severity).to_string(),
            ]
        })
        .collect();
    Some(format!(
        "## Not read\n\n{}",
        pipe_table(&header(&["Where", "What", "Why", "Severity"]), &[], &rows)
    ))
}
