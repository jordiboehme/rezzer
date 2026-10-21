// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Form logic in the model: fields with role, format and calculation order, the calculations
//! as formulas, the graph of which field reads which, scripts listed with their text, and
//! signatures with the saves after them.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::append_update;
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Bytes, Confidence, Detection, FormField, FormatId, LossReason, ReadOptions,
    ReadOutcome, Reader, Role, Severity, Source, Value,
};
use rezzer_read_pdf::PdfReader;

fn corpus(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/corpus/pdf");
    std::fs::read(path.join(name)).expect("corpus file")
}

fn read(bytes: &[u8]) -> ReadOutcome {
    let src = Source {
        path: None,
        name: "file.pdf".into(),
        bytes: Bytes::Owned(Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format: FormatId::Pdf,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "%PDF-".into(),
        },
        modified: None,
    };
    PdfReader.read(
        &src,
        &ReadOptions::default(),
        &Budget::new(Limits::default()),
    )
}

fn field<'a>(out: &'a ReadOutcome, name: &str) -> &'a FormField {
    out.document
        .facets
        .fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name}: {:?}", out.document.facets.fields))
}

fn anchor(name: &str) -> Anchor {
    Anchor(format!("field:{name}"))
}

#[test]
fn form_table_roles_and_order() {
    let out = read(&corpus("pdfjs-text-field-calc.pdf"));
    let fields = &out.document.facets.fields;
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Source", "Mirror"]);
    let source = field(&out, "Source");
    assert_eq!(source.anchor, anchor("Source"));
    assert_eq!(source.kind, "text");
    assert_eq!(source.role, Some(Role::Input));
    assert_eq!(source.calc_order, None);
    assert_eq!(source.pages, [1]);
    let mirror = field(&out, "Mirror");
    assert_eq!(mirror.role, Some(Role::Result));
    assert_eq!(mirror.calc_order, Some(1));
    assert_eq!(mirror.flags, ["read-only"]);
    assert_eq!(mirror.pages, [3]);
    let calc = mirror
        .actions
        .iter()
        .find(|a| a.trigger == "calculate")
        .expect("a calculate action");
    assert_eq!(
        calc.script,
        r#"event.value = this.getField("Source").value;"#
    );
    // The one calculation as a formula that reads Source.
    let formulas = &out.document.facets.formulas;
    assert_eq!(formulas.len(), 1, "{formulas:?}");
    let f = &formulas[0];
    assert_eq!(f.anchor, anchor("Mirror"));
    assert_eq!(f.dialect, "pdf-js");
    assert_eq!(f.expression, calc.script);
    assert_eq!(f.references, [anchor("Source")]);
    assert_eq!(f.role, Some(Role::Result));
    assert_eq!(f.note, None);
    let facts = &out.document.properties.facts;
    assert_eq!(facts.form_fields, Some(2));
    assert_eq!(facts.has_javascript, Some(true));
    assert_eq!(facts.signed, Some(false));
    assert_eq!(facts.signatures, None);
}

#[test]
fn form_graph_from_reads() {
    let out = read(&corpus("pdfjs-text-field-calc.pdf"));
    let graph = out
        .document
        .facets
        .graphs
        .iter()
        .find(|g| g.id == "form")
        .expect("the form graph");
    assert_eq!(graph.kind, "dependency");
    let labels: Vec<&str> = graph.nodes.iter().map(|n| n.label.as_str()).collect();
    assert_eq!(labels, ["Source (input)", "Mirror (result, calc 1)"]);
    let id = |label: &str| {
        let n = graph.nodes.iter().find(|n| n.label == label).unwrap();
        assert_eq!(n.anchor, Some(anchor(label.split(' ').next().unwrap())));
        n.id.clone()
    };
    assert_eq!(graph.edges.len(), 1);
    let e = &graph.edges[0];
    assert_eq!(
        (e.from.clone(), e.to.clone(), e.kind.as_str()),
        (id("Mirror (result, calc 1)"), id("Source (input)"), "reads")
    );
    // A form whose fields read none has no graph.
    let out = read(&corpus("pdfbox-sf1199a-direct-deposit.pdf"));
    assert!(out.document.facets.graphs.iter().all(|g| g.id != "form"));
}

#[test]
fn formats_in_words_in_fields() {
    let out = read(&corpus("pdfbox-sf1199a-direct-deposit.pdf"));
    let amount = field(&out, "Allottment Amount");
    assert_eq!(
        amount.format.as_deref(),
        Some(r#"number, 2 decimals, "$" before the number"#)
    );
    assert_eq!(amount.role, Some(Role::Input));
    assert_eq!(amount.flags, ["do-not-spell-check", "do-not-scroll"]);
    let format = amount
        .actions
        .iter()
        .find(|a| a.trigger == "format")
        .unwrap();
    assert_eq!(format.script, r#"AFNumber_Format(2, 0, 0, 0, "$", true);"#);
    assert_eq!(format.words.as_deref(), amount.format.as_deref());
    // A keystroke script Rezzer has no words for keeps its text and no words.
    let keystroke = amount
        .actions
        .iter()
        .find(|a| a.trigger == "keystroke")
        .unwrap();
    assert_eq!(keystroke.words, None);
    assert_eq!(
        field(&out, "Bank Rep Date").format.as_deref(),
        Some("date mm/dd/yyyy")
    );
    assert_eq!(field(&out, "Phone").format.as_deref(), Some("phone number"));
    // Fields without scripts have no format; no field calculates.
    assert!(out.document.facets.formulas.is_empty());
    assert!(
        out.document
            .facets
            .fields
            .iter()
            .all(|f| f.role != Some(Role::Result))
    );
}

#[test]
fn scripts_embedded_with_text() {
    let out = read(&corpus("pdfjs-js-buttons.pdf"));
    let scripts: Vec<_> = out
        .document
        .facets
        .embedded
        .iter()
        .filter(|e| e.kind == "script")
        .collect();
    let callback = scripts
        .iter()
        .find(|e| e.name == "document")
        .expect("the document script");
    assert_eq!(callback.detail.as_deref(), Some("javascript, callback"));
    assert!(
        callback
            .text
            .as_deref()
            .unwrap()
            .starts_with("function callback(x)"),
        "{callback:?}"
    );
    let button = scripts
        .iter()
        .find(|e| e.name == "field Button3 activate")
        .expect("the button script");
    assert_eq!(button.detail.as_deref(), Some("javascript"));
    assert!(button.text.as_deref().unwrap().contains("getField"));
    assert_eq!(
        scripts
            .iter()
            .filter(|e| e.name == "field Group1 activate")
            .count(),
        4
    );
    assert_eq!(out.document.properties.facts.has_javascript, Some(true));
    // The fields keep their scripts too.
    let button = field(&out, "Button3");
    assert_eq!(button.kind, "button");
    assert!(button.actions.iter().any(|a| a.trigger == "activate"));
}

#[test]
fn signature_with_later_revisions() {
    let bytes = corpus("irs-f1040-2025.pdf");
    let out = read(&bytes);
    let sigs = &out.document.facets.signatures;
    assert_eq!(sigs.len(), 1, "{sigs:?}");
    let s = &sigs[0];
    assert_eq!(s.kind, "usage-rights");
    assert_eq!(s.anchor, None);
    assert_eq!(s.sub_filter.as_deref(), Some("adbe.pkcs7.detached"));
    assert!(s.covers_whole_file);
    // The save that signed ends where the signature does: no save came after it.
    assert_eq!(s.later_revisions, 0);
    let facts = &out.document.properties.facts;
    assert_eq!(facts.signed, Some(true));
    assert_eq!(facts.signatures, Some(1));
    assert_eq!(facts.form_fields, Some(199));
    let xfa = out
        .document
        .losses
        .iter()
        .find(|l| l.what == "XFA form data present, not read")
        .expect("the XFA loss");
    assert_eq!(
        (xfa.why, xfa.severity),
        (LossReason::NotImplemented, Severity::Warning)
    );
    // One more save after the signature.
    let mut later = bytes.clone();
    append_update(
        &mut later,
        &[(2454, b"<< /Note (a later save) >>".to_vec())],
        "/Size 2455 /Root 2399 0 R /Info 2397 0 R",
    );
    let out = read(&later);
    let s = &out.document.facets.signatures[0];
    assert_eq!(s.later_revisions, 1);
    assert!(!s.covers_whole_file);
    assert_eq!(out.document.properties.facts.revisions, Some(3));
}

#[test]
fn field_values_and_options() {
    let out = read(&corpus("pypdf-libreoffice-form.pdf"));
    let fields = &out.document.facets.fields;
    assert!(!fields.is_empty());
    for f in fields {
        assert_eq!(f.anchor, anchor(&f.name));
        if let Some(v) = &f.value {
            assert!(matches!(v, Value::Text { .. }), "{f:?}");
        }
    }
}
