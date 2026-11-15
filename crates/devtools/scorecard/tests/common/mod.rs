// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small hand-built documents for the tests, written as the model JSON. All text is invented.

#![allow(dead_code)]

use rezzer_model::Document;
use serde_json::{Value, json};

/// A document with `body`, `regions` and form `fields`, on one page.
pub fn document(body: Vec<Value>, regions: Vec<Value>, fields: Vec<Value>) -> Document {
    serde_json::from_value(json!({
        "model_version": 1,
        "source": {
            "name": "muster.pdf",
            "sha256": "",
            "size": 0,
            "format": "pdf",
            "detection": { "confidence": "certain", "evidence": "%PDF-" }
        },
        "properties": { "facts": { "tagged": true } },
        "body": [{ "type": "section", "kind": "page", "anchor": "p1", "blocks": body }],
        "facets": { "fields": fields },
        "regions": regions
    }))
    .expect("the test document is model JSON")
}

/// A document with `body` only.
pub fn doc(body: Vec<Value>) -> Document {
    document(body, Vec::new(), Vec::new())
}

/// Inlines of `text`: a line break at each `\n`.
pub fn inlines(text: &str) -> Value {
    let mut out = Vec::new();
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push(json!({ "type": "line_break" }));
        }
        if !line.is_empty() {
            out.push(json!({ "type": "text", "text": line }));
        }
    }
    Value::Array(out)
}

pub fn paragraph(anchor: &str, text: &str) -> Value {
    json!({ "type": "paragraph", "inlines": inlines(text), "anchor": anchor })
}

pub fn heading(anchor: &str, level: u8, text: &str) -> Value {
    json!({ "type": "heading", "level": level, "inlines": inlines(text), "anchor": anchor })
}

pub fn cell(text: &str) -> Value {
    spanned(text, 1, 1)
}

/// A cell that spans `rows` rows and `cols` columns.
pub fn spanned(text: &str, rows: u32, cols: u32) -> Value {
    json!({ "inlines": inlines(text), "row_span": rows, "col_span": cols })
}

/// A table of `rows` of cell texts, each cell of one row and one column.
pub fn table(anchor: &str, header_rows: usize, rows: &[&[&str]]) -> Value {
    let rows: Vec<Vec<Value>> = rows
        .iter()
        .map(|r| r.iter().map(|c| cell(c)).collect())
        .collect();
    table_of(anchor, header_rows, rows)
}

pub fn table_of(anchor: &str, header_rows: usize, rows: Vec<Vec<Value>>) -> Value {
    json!({ "type": "table", "header_rows": header_rows, "rows": rows, "anchor": anchor })
}

/// A block kept as laid out: each line is its pieces.
pub fn spatial(anchor: &str, lines: &[&[&str]], links: &[&str]) -> Value {
    let lines: Vec<Value> = lines
        .iter()
        .map(|l| {
            let pieces: Vec<Value> = l
                .iter()
                .enumerate()
                .map(|(n, t)| json!({ "col": n * 20, "text": t }))
                .collect();
            json!({ "pieces": pieces })
        })
        .collect();
    json!({ "type": "spatial", "lines": lines, "links": links, "anchor": anchor })
}

/// A region at `anchor` of `kind`, found by the rule `id`.
pub fn region(anchor: &str, kind: &str, certainty: &str, id: &str) -> Value {
    json!({
        "anchor": anchor,
        "page": 1,
        "kind": kind,
        "bounds": [10.0, 20.0, 110.5, 220.25],
        "certainty": certainty,
        "score": 1000,
        "evidence": [{ "kind": "rule", "id": id, "detail": "Geheimer Wortlaut" }]
    })
}

/// A filled text field.
pub fn field(name: &str, label: Option<&str>, value: &str) -> Value {
    let mut field = json!({
        "anchor": format!("field:{name}"),
        "name": name,
        "kind": "text",
        "value": { "type": "text", "value": value },
        "pages": [1]
    });
    if let Some(label) = label {
        field["label"] = json!(label);
    }
    field
}
