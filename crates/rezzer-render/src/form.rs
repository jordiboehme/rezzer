// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Form logic, scripts and signatures: the `### Form` part of `## Logic` with its graph and
//! calculations, the `### Scripts` part of `## Embedded` and the `### Signatures` part of
//! `## Changes`. Scripts are shown as text in fences, never run; signatures are described,
//! never verified.

use std::collections::HashMap;

use rezzer_model::{Document, Formula};

use crate::blocks::{cell_text, fence, finish_lines, heading_line, line_text, pipe_table};
use crate::facets::{role_name, value_text};
use crate::mermaid;

/// The dialect of form calculations.
pub(crate) const PDF_JS: &str = "pdf-js";

/// The id of the graph of which field reads which.
pub(crate) const FORM_GRAPH: &str = "form";

/// A Reads cell names at most this many fields, then how many more.
const MAX_READS: usize = 20;

fn header(cols: &[&str]) -> Vec<String> {
    cols.iter().map(|c| (*c).to_string()).collect()
}

/// The field name of a field anchor `field:<name>`.
fn field_name(anchor: &str) -> &str {
    anchor.strip_prefix("field:").unwrap_or(anchor)
}

/// `### Form`: one row per field, then the graph `form` and `### Calculations`. `None` for a
/// document without fields.
pub(crate) fn section(doc: &Document) -> Option<String> {
    let fields = &doc.facets.fields;
    if fields.is_empty() {
        return None;
    }
    let mut calcs: HashMap<&str, &Formula> = HashMap::new();
    for f in doc.facets.formulas.iter().filter(|f| f.dialect == PDF_JS) {
        calcs.entry(f.anchor.0.as_str()).or_insert(f);
    }
    let rows: Vec<Vec<String>> = fields
        .iter()
        .map(|f| {
            let mut name = cell_text(&f.name);
            if let Some(label) = f
                .label
                .as_deref()
                .filter(|l| !l.trim().is_empty() && *l != f.name)
            {
                name.push_str(&format!(" ({})", cell_text(label)));
            }
            let role = match (f.role, f.calc_order) {
                (Some(r), Some(n)) => format!("{} (calc {n})", role_name(r)),
                (Some(r), None) => role_name(r).to_string(),
                (None, Some(n)) => format!("calc {n}"),
                (None, None) => String::new(),
            };
            let rule = match f.actions.iter().find(|a| a.trigger == "calculate") {
                Some(a) => match a.words.as_deref().filter(|w| !w.trim().is_empty()) {
                    Some(w) => cell_text(w),
                    None => "script, see below".to_string(),
                },
                None => String::new(),
            };
            let reads = calcs
                .get(f.anchor.0.as_str())
                .map(|c| reads_cell(c))
                .unwrap_or_default();
            vec![
                name,
                cell_text(&f.kind),
                role,
                f.value
                    .as_ref()
                    .map(|v| cell_text(&value_text(v)))
                    .unwrap_or_default(),
                f.format.as_deref().map(cell_text).unwrap_or_default(),
                rule,
                reads,
                cell_text(&f.flags.join(", ")),
            ]
        })
        .collect();
    let mut parts = vec![
        heading_line(3, "Form"),
        pipe_table(
            &header(&[
                "Field", "Type", "Role", "Value", "Format", "Rule", "Reads", "Flags",
            ]),
            &[],
            &rows,
        ),
    ];
    if let Some(g) = doc.facets.graphs.iter().find(|g| g.id == FORM_GRAPH) {
        parts.push(mermaid::render(g));
    }
    let order: HashMap<&str, Option<u32>> = fields
        .iter()
        .map(|f| (f.anchor.0.as_str(), f.calc_order))
        .collect();
    let entries: Vec<String> = doc
        .facets
        .formulas
        .iter()
        .filter(|f| f.dialect == PDF_JS)
        .map(|c| {
            let name = field_name(&c.anchor.0);
            let line = match order.get(c.anchor.0.as_str()).copied().flatten() {
                Some(n) => format!("{}, calc {n}:", line_text(name)),
                None => format!("{}:", line_text(name)),
            };
            format!(
                "{}\n{}",
                finish_lines(&line),
                fence(Some("javascript"), &c.expression)
            )
        })
        .collect();
    if !entries.is_empty() {
        parts.push(heading_line(3, "Calculations"));
        parts.push(entries.join("\n\n"));
    }
    Some(parts.join("\n\n"))
}

/// The fields a calculation reads, at most [`MAX_READS`] by name, then `unknown, see script`
/// when it builds names at run time.
fn reads_cell(c: &Formula) -> String {
    let mut reads: Vec<String> = c
        .references
        .iter()
        .take(MAX_READS)
        .map(|r| cell_text(field_name(&r.0)))
        .collect();
    if c.references.len() > MAX_READS {
        reads.push(format!("and {} more", c.references.len() - MAX_READS));
    }
    if c.note.is_some() {
        reads.push("unknown, see script".into());
    }
    reads.join(", ")
}

/// `### Scripts`: every embedded entry with a text, on a line that names it, then its text in
/// a fence longer than any backtick run in it. `None` when no entry has a text.
pub(crate) fn scripts(doc: &Document) -> Option<String> {
    let entries: Vec<String> = doc
        .facets
        .embedded
        .iter()
        .filter_map(|e| {
            let text = e.text.as_deref()?;
            let line = match e.detail.as_deref().filter(|d| !d.trim().is_empty()) {
                Some(d) => format!("{} ({}), listed, not run.", e.name, d),
                None => format!("{}, listed, not run.", e.name),
            };
            let lang = e
                .detail
                .as_deref()
                .is_some_and(|d| d.split(',').next() == Some("javascript"))
                .then_some("javascript");
            Some(format!(
                "{}\n{}",
                finish_lines(&line_text(&line)),
                fence(lang, text)
            ))
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    Some(format!(
        "{}\n\n{}",
        heading_line(3, "Scripts"),
        entries.join("\n\n")
    ))
}

/// `### Signatures`: one row per signature. None is verified. `None` without signatures.
pub(crate) fn signatures(doc: &Document) -> Option<String> {
    let sigs = &doc.facets.signatures;
    if sigs.is_empty() {
        return None;
    }
    let opt = |v: &Option<String>| v.as_deref().map(cell_text).unwrap_or_default();
    let rows: Vec<Vec<String>> = sigs
        .iter()
        .map(|s| {
            vec![
                s.anchor
                    .as_ref()
                    .map_or_else(|| "no field".to_string(), |a| cell_text(field_name(&a.0))),
                cell_text(&s.kind),
                opt(&s.signer),
                opt(&s.date),
                opt(&s.reason),
                if s.covers_whole_file { "yes" } else { "no" }.to_string(),
                s.later_revisions.to_string(),
                "no".to_string(),
            ]
        })
        .collect();
    Some(format!(
        "{}\n\n{}",
        heading_line(3, "Signatures"),
        pipe_table(
            &header(&[
                "Signature",
                "Kind",
                "Signer",
                "Date",
                "Reason",
                "Whole file",
                "Saves after",
                "Verified",
            ]),
            &[],
            &rows,
        )
    ))
}
