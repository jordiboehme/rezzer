// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sections after the content: Logic, Annotations, Changes, Embedded and Not read.
//! Each returns `None` when it has nothing to show. The form parts are in `form.rs`.

use std::collections::HashMap;

use rezzer_model::{
    Annotation, Block, Document, Formula, Inline, LossReason, Role, Severity, StatusColor, Value,
};

use crate::blocks::{self, Ctx, MAX_DEPTH, Mode, Notes, cell_text, code_span, pipe_table};
use crate::form::{self, FORM_GRAPH, PDF_JS};
use crate::mermaid;
use crate::workbook;

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

fn formula_row(f: &Formula, with_note: bool, math: bool) -> Vec<String> {
    let mut expr = String::new();
    code_span(&f.expression, Mode::Cell, &mut expr);
    if let Some(latex) = f.latex.as_deref().filter(|l| math && !l.trim().is_empty()) {
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
fn formula_table(list: &[&Formula], math: bool) -> String {
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
    let rows: Vec<Vec<String>> = list
        .iter()
        .map(|f| formula_row(f, with_note, math))
        .collect();
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

/// `## Logic`. A workbook gets its logic per sheet ([`workbook::logic`]); any other document
/// gets its form first ([`form::section`]: the fields, their graph and calculations, which are
/// then not repeated), then formulas without a pattern (one table per dialect), then one table
/// per pattern (in order of first appearance), then the graphs. Without `math` no LaTeX is
/// written.
pub(crate) fn logic(doc: &Document, math: bool) -> Option<String> {
    if workbook::is_workbook(doc) {
        return workbook::logic(doc, math);
    }
    let form = form::section(doc);
    let with_form = form.is_some();
    let formulas: Vec<&Formula> = doc
        .facets
        .formulas
        .iter()
        .filter(|f| !(with_form && f.dialect == PDF_JS))
        .collect();
    let graphs: Vec<_> = doc
        .facets
        .graphs
        .iter()
        .filter(|g| !(with_form && g.id == FORM_GRAPH))
        .collect();
    if formulas.is_empty() && graphs.is_empty() && form.is_none() {
        return None;
    }
    let mut parts = vec!["## Logic".to_string()];
    parts.extend(form);
    // One pass: formulas without a pattern by dialect, the others by pattern, each list in
    // order of first appearance.
    let mut singles: Vec<(&str, Vec<&Formula>)> = Vec::new();
    let mut groups: Vec<(&str, Vec<&Formula>)> = Vec::new();
    let mut single_at: HashMap<&str, usize> = HashMap::new();
    let mut group_at: HashMap<&str, usize> = HashMap::new();
    for f in formulas {
        let (key, lists, at) = match f.pattern.as_deref() {
            Some(p) => (p, &mut groups, &mut group_at),
            None => (f.dialect.as_str(), &mut singles, &mut single_at),
        };
        match at.get(key) {
            Some(&i) => lists[i].1.push(f),
            None => {
                at.insert(key, lists.len());
                lists.push((key, vec![f]));
            }
        }
    }
    for (_, single) in &singles {
        parts.push(formula_table(single, math));
    }
    for (p, group) in &groups {
        parts.push(blocks::heading_line(
            3,
            &format!("Pattern: {}", blocks::line_text(p)),
        ));
        parts.push(formula_table(group, math));
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
    Inline::plain(t)
}

fn paragraph(inlines: Vec<Inline>) -> Block {
    Block::paragraph(inlines)
}

/// One annotation as list item blocks: who, when and where, the quoted passage, the body
/// and the replies.
pub(crate) fn annotation_item(a: &Annotation, depth: usize) -> Vec<Block> {
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

/// An annotation at its passage: a note callout `<Kind> by <author>, <date>` with the quoted
/// passage as a nested quote, the state, the body and the replies as a list.
pub(crate) fn passage_note(a: &Annotation) -> Block {
    let mut title: String = a
        .kind
        .chars()
        .take(1)
        .flat_map(char::to_uppercase)
        .chain(a.kind.chars().skip(1))
        .collect();
    if let Some(author) = a.author.as_deref().filter(|s| !s.trim().is_empty()) {
        title.push_str(&format!(" by {author}"));
    }
    if let Some(date) = a.date.as_deref().filter(|s| !s.trim().is_empty()) {
        title.push_str(&format!(", {date}"));
    }
    let mut item = Vec::new();
    if let Some(q) = a.quoted.as_deref().filter(|q| !q.trim().is_empty()) {
        item.push(Block::Quote {
            blocks: vec![paragraph(vec![text(q)])],
            anchor: None,
        });
    }
    match a.resolved {
        Some(true) => item.push(paragraph(vec![Inline::Status {
            text: "Resolved".into(),
            color: StatusColor::Green,
        }])),
        Some(false) => item.push(paragraph(vec![Inline::Status {
            text: "Open".into(),
            color: StatusColor::Yellow,
        }])),
        None => {}
    }
    if !a.body.is_empty() {
        item.push(paragraph(a.body.clone()));
    }
    if !a.replies.is_empty() {
        item.push(Block::List {
            ordered: false,
            items: a.replies.iter().map(|r| annotation_item(r, 1)).collect(),
            anchor: None,
        });
    }
    Block::Callout {
        kind: rezzer_model::CalloutKind::Note,
        title: Some(title),
        folded: None,
        blocks: item,
        anchor: None,
    }
}

/// `## Annotations`: comments, notes and highlights with author, date and passage; the ones
/// the content showed at their passage (`notes`) are left out.
pub(crate) fn annotations(doc: &Document, ctx: &Ctx, notes: &Notes) -> Option<String> {
    let items: Vec<Vec<Block>> = doc
        .facets
        .annotations
        .iter()
        .enumerate()
        .filter(|(i, _)| !notes.shown(*i))
        .map(|(_, a)| annotation_item(a, 0))
        .collect();
    if items.is_empty() {
        return None;
    }
    let list = Block::List {
        ordered: false,
        items,
        anchor: None,
    };
    Some(format!(
        "## Annotations\n\n{}",
        blocks::blocks(&[list], ctx, 0, 3, false)
    ))
}

/// `## Changes`: the signatures ([`form::signatures`]), then the timeline as a table, under
/// `### Revisions` when there are signatures.
pub(crate) fn changes(doc: &Document) -> Option<String> {
    let signatures = form::signatures(doc);
    if doc.facets.timeline.is_empty() {
        return signatures.map(|s| format!("## Changes\n\n{s}"));
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
    let table = pipe_table(
        &header(&["When", "Who", "What", "Where", "Text"]),
        &[],
        &rows,
    );
    Some(match signatures {
        Some(s) => format!("## Changes\n\n{s}\n\n### Revisions\n\n{table}"),
        None => format!("## Changes\n\n{table}"),
    })
}

/// `## Embedded`: scripts, macros, objects and references (never run or followed) with the
/// texts of the scripts ([`form::scripts`]), then the pictures the calling agent should look
/// at.
pub(crate) fn embedded(doc: &Document) -> Option<String> {
    let vision: Vec<_> = doc.assets.iter().filter(|a| a.needs_vision).collect();
    if doc.facets.embedded.is_empty() && vision.is_empty() {
        return None;
    }
    let mut parts = vec!["## Embedded".to_string()];
    if !doc.facets.embedded.is_empty() {
        // The k-th attachment is extracted as asset `attachment-<k>`, under its own name.
        let files: HashMap<&str, &rezzer_model::Asset> =
            doc.assets.iter().map(|a| (a.id.as_str(), a)).collect();
        let mut attachments = 0;
        let rows: Vec<Vec<String>> = doc
            .facets
            .embedded
            .iter()
            .map(|e| {
                let mut target = String::new();
                if let Some(t) = &e.target {
                    code_span(t, Mode::Cell, &mut target);
                }
                if e.kind == "attachment" {
                    attachments += 1;
                    let id = format!("attachment-{attachments}");
                    let file = files
                        .get(id.as_str())
                        .filter(|a| a.caption.as_deref() == Some(e.name.as_str()))
                        .and_then(|a| a.path.as_deref());
                    if let Some(path) = file {
                        if !target.is_empty() {
                            target.push(' ');
                        }
                        target.push_str(&blocks::asset_link(path, Mode::Cell));
                    }
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
        parts.extend(form::scripts(doc));
    }
    if !vision.is_empty() {
        parts.push("### Pictures that need vision".into());
        let rows: Vec<Vec<String>> = vision
            .iter()
            .map(|a| {
                let file = match &a.path {
                    Some(p) => blocks::asset_link(p, Mode::Cell),
                    None => "not extracted".to_string(),
                };
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
        LossReason::Internal => "internal error",
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
