// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Form logic in the model: fields with role, format and calculation order, the calculations
//! as formulas, the graph of which field reads which, scripts listed with their text, and
//! signatures with the saves after them.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{Builder, Page, append_update, pdf, show};
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
    // A usage rights signature unlocks features of a viewer; it does not sign the content.
    assert_eq!(facts.signed, Some(false));
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

#[test]
fn link_actions_without_text_are_listed() {
    // Two script links without a readable script, a Launch after a URI and a Launch after a
    // script: every action is listed once, none is followed.
    let link = |rect: &str, action: &str| {
        format!("<< /Type /Annot /Subtype /Link /Rect [{rect}] /A << {action} >> >>")
    };
    let page = Page {
        content: show("F1", 12.0, 72.0, 700.0, "Hello"),
        annots: vec![
            link("0 0 50 50", "/S /JavaScript"),
            link("60 0 110 50", "/S /JavaScript /JS 99"),
            link(
                "120 0 170 50",
                "/S /URI /URI (http://x.example/) /Next << /S /Launch /F (evil.exe) >>",
            ),
            link(
                "180 0 230 50",
                r"/S /JavaScript /JS (app.alert\(1\)) /Next << /S /Launch /F (evil2.exe) >>",
            ),
            link("240 0 290 50", "/S /Launch /F (first.exe)"),
        ],
        ..Page::default()
    };
    let out = read(&pdf(&[page], ""));
    let listed: Vec<(&str, &str, Option<&str>, bool)> = out
        .document
        .facets
        .embedded
        .iter()
        .map(|e| {
            (
                e.kind.as_str(),
                e.detail.as_deref().unwrap_or_default(),
                e.target.as_deref(),
                e.text.is_some(),
            )
        })
        .collect();
    let count = |kind: &str, detail: &str, target: Option<&str>| {
        listed
            .iter()
            .filter(|(k, d, t, _)| *k == kind && *d == detail && *t == target)
            .count()
    };
    assert_eq!(count("script", "launch", Some("evil.exe")), 1, "{listed:?}");
    assert_eq!(
        count("script", "launch", Some("evil2.exe")),
        1,
        "{listed:?}"
    );
    // The Launch a link starts with is listed by the links, once.
    assert_eq!(
        count("launch-action", "from page 1", Some("first.exe")),
        1,
        "{listed:?}"
    );
    assert_eq!(
        count("script", "launch", Some("first.exe")),
        0,
        "{listed:?}"
    );
    let scripts: Vec<bool> = listed
        .iter()
        .filter(|(k, d, _, _)| *k == "script" && *d == "javascript")
        .map(|(_, _, _, text)| *text)
        .collect();
    assert_eq!(scripts, [false, false, true], "{listed:?}");
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.location == "scripts" && l.what.contains("2 JavaScript actions")),
        "{:?}",
        out.document.losses
    );
}

/// A one-page file with a signature field. The signed range leaves out the last `unsigned`
/// bytes; without `final_eol` the file ends right after `%%EOF`.
fn signed(unsigned: usize, final_eol: bool) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let contents = b.stream("", b"BT ET");
    let page = b.reserve();
    let sig = b.add(format!(
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /Name (Ann) \
         /ByteRange [0 AAAAAAAAAA BBBBBBBBBB CCCCCCCCCC] /Contents <{}> >>",
        "0".repeat(64)
    ));
    let field = b.add(format!(
        "<< /FT /Sig /T (Sig1) /V {sig} 0 R /Type /Annot /Subtype /Widget /Rect [0 0 0 0] \
         /P {page} 0 R >>"
    ));
    b.set(
        page,
        format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /Contents {contents} 0 R \
             /Annots [{field} 0 R] >>"
        ),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"),
    );
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R /AcroForm << /Fields [{field} 0 R] /SigFlags 3 \
             >> >>"
        ),
    );
    let mut f = b.finish(catalog);
    if !final_eol {
        assert_eq!(f.pop(), Some(b'\n'));
    }
    let find = |f: &[u8], pat: &[u8]| f.windows(pat.len()).position(|w| w == pat).unwrap();
    let lt = find(&f, b"/Contents <") + 10;
    let gt = find(&f[lt..], b">") + lt + 1;
    let rest = f.len() - gt - unsigned;
    for (mark, v) in [("AAAAAAAAAA", lt), ("BBBBBBBBBB", gt), ("CCCCCCCCCC", rest)] {
        let at = find(&f, mark.as_bytes());
        f[at..at + 10].copy_from_slice(format!("{v:010}").as_bytes());
    }
    f
}

fn saves_after(bytes: &[u8]) -> (u32, bool) {
    let out = read(bytes);
    let s = &out.document.facets.signatures[0];
    (s.later_revisions, s.covers_whole_file)
}

fn one_more_save(mut f: Vec<u8>) -> Vec<u8> {
    append_update(&mut f, &[(99, b"<< >>".to_vec())], "/Size 100 /Root 1 0 R");
    f
}

#[test]
fn saves_after_a_signature_count_from_its_end() {
    assert_eq!(saves_after(&signed(0, true)), (0, true));
    assert_eq!(saves_after(&one_more_save(signed(0, true))), (1, false));
    assert_eq!(saves_after(&signed(0, false)), (0, true));
    // The update starts with the line end the signed file did not have.
    assert_eq!(saves_after(&one_more_save(signed(0, false))).0, 1);
    // The signed range stops at %%EOF; the line end after it is not a save.
    assert_eq!(saves_after(&signed(1, true)).0, 0);
}

#[test]
fn saves_after_a_signature_past_the_revision_cap_are_a_lower_bound() {
    let mut f = signed(0, true);
    for k in 0..80u32 {
        append_update(
            &mut f,
            &[(100 + k, b"<< >>".to_vec())],
            &format!("/Size {} /Root 1 0 R", 101 + k),
        );
    }
    let out = read(&f);
    let s = &out.document.facets.signatures[0];
    assert!(s.later_revisions >= 63, "{}", s.later_revisions);
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.location == "signatures")
        .unwrap_or_else(|| panic!("{:?}", out.document.losses));
    assert!(loss.what.contains("at least"), "{loss:?}");
    assert_eq!(loss.why, LossReason::Limit);
}
