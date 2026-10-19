// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::sync::Arc;

use rezzer_model::{
    Anchor, Annotation, Asset, Block, Bytes, CalloutKind, Cell, Completeness, Confidence,
    Detection, Document, Edge, Embedded, Event, FormatId, Formula, Graph, Inline, Loss, LossReason,
    MODEL_VERSION, Node, Properties, ReaderInfo, Role, SectionKind, Severity, Source, SourceInfo,
    StatusColor, TaskState, Value,
};

fn src() -> SourceInfo {
    SourceInfo {
        path: Some("/tmp/a.xlsx".into()),
        name: "a.xlsx".into(),
        sha256: "00".repeat(32),
        size: 42,
        format: FormatId::Xlsx,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "zip with xl/workbook.xml".into(),
        },
        modified: Some("2026-10-19T10:00:00Z".into()),
        encoding: None,
        reader: Some(ReaderInfo {
            id: "xlsx".into(),
            version: "0.1.0".into(),
        }),
    }
}

fn text(s: &str) -> Inline {
    Inline::Text {
        text: s.into(),
        emphasis: false,
        strong: true,
    }
}

fn full_document() -> Document {
    let mut doc = Document::new(src());
    doc.properties = Properties {
        title: Some("Costs".into()),
        description: Some("Quarterly costs".into()),
        authors: vec!["Ada".into()],
        created: Some(Value::DateTime {
            value: "2026-01-01T00:00:00Z".into(),
        }),
        modified: Some(Value::Date {
            value: "2026-02-01".into(),
        }),
        language: Some("en".into()),
        keywords: vec!["budget".into()],
        custom: BTreeMap::from([
            (
                "weight".to_string(),
                Value::Number {
                    value: 1.5,
                    unit: Some("kg".into()),
                },
            ),
            ("share".to_string(), Value::Percent { value: 0.25 }),
            ("final".to_string(), Value::Bool { value: true }),
            (
                "tags".to_string(),
                Value::List {
                    value: vec![Value::Text { value: "a".into() }],
                },
            ),
        ]),
    };
    let anchor = Some(Anchor("Costs!C12".into()));
    doc.body = vec![
        Block::Section {
            kind: SectionKind::Sheet,
            title: Some("Costs".into()),
            anchor: Some(Anchor("sheet:Costs".into())),
            blocks: vec![Block::Heading {
                level: 2,
                inlines: vec![text("Intro")],
                anchor: Some(Anchor("h:intro".into())),
            }],
        },
        Block::Paragraph {
            inlines: vec![
                text("see"),
                Inline::Code { text: "x".into() },
                Inline::Link {
                    target: "https://example.org".into(),
                    title: Some("Example".into()),
                    internal: false,
                    inlines: vec![text("here")],
                },
                Inline::FootnoteRef { id: "1".into() },
                Inline::Math {
                    latex: "x^2".into(),
                },
                Inline::Status {
                    text: "Done".into(),
                    color: StatusColor::Green,
                },
                Inline::AnchorMark {
                    anchor: Anchor("p4".into()),
                },
            ],
            role: Some("caption".into()),
            anchor: None,
        },
        Block::List {
            ordered: true,
            items: vec![vec![Block::Paragraph {
                inlines: vec![text("one")],
                role: None,
                anchor: None,
            }]],
        },
        Block::Table {
            caption: Some("Totals".into()),
            header_rows: 1,
            rows: vec![vec![Cell {
                inlines: vec![text("12")],
                row_span: 1,
                col_span: 2,
                anchor: anchor.clone(),
            }]],
            anchor: Some(Anchor("table:1".into())),
        },
        Block::Code {
            lang: Some("rust".into()),
            text: "fn main() {}".into(),
        },
        Block::Quote {
            blocks: vec![Block::Paragraph {
                inlines: vec![text("quoted")],
                role: None,
                anchor: None,
            }],
        },
        Block::Math {
            latex: "\\frac{a}{b}".into(),
            source_kind: "omml".into(),
        },
        Block::Media {
            asset: "img1".into(),
        },
        Block::Callout {
            kind: CalloutKind::Warning,
            title: Some("Careful".into()),
            folded: Some(true),
            blocks: vec![],
        },
        Block::Task {
            state: TaskState::Cancelled,
            inlines: vec![text("todo")],
        },
    ];
    doc.facets.formulas.push(Formula {
        anchor: Anchor("Costs!C12".into()),
        expression: "=SUM(C1:C11)".into(),
        dialect: "excel".into(),
        references: vec![Anchor("Costs!C1:C11".into())],
        cached: Some(Value::Number {
            value: 12.0,
            unit: None,
        }),
        role: Some(Role::Result),
        pattern: Some("g1".into()),
        latex: Some("\\sum C".into()),
    });
    doc.facets.graphs.push(Graph {
        id: "deps".into(),
        kind: "dependency".into(),
        nodes: vec![Node {
            id: "n1".into(),
            label: "C12".into(),
            kind: Some("cell".into()),
            anchor: anchor.clone(),
        }],
        edges: vec![Edge {
            from: "n1".into(),
            to: "n2".into(),
            kind: "references".into(),
        }],
    });
    doc.facets.timeline.push(Event {
        time: Some("2026-03-01".into()),
        actor: Some("Ada".into()),
        kind: "revision".into(),
        anchor: anchor.clone(),
        text: Some("changed".into()),
    });
    doc.facets.annotations.push(Annotation {
        kind: "comment".into(),
        author: Some("Bob".into()),
        date: Some("2026-03-02".into()),
        anchor: anchor.clone(),
        quoted: Some("12".into()),
        body: vec![text("Check this")],
        replies: vec![Annotation {
            kind: "comment".into(),
            author: Some("Ada".into()),
            date: None,
            anchor: None,
            quoted: None,
            body: vec![text("Ok")],
            replies: vec![],
            resolved: None,
        }],
        resolved: Some(false),
    });
    doc.facets.embedded.push(Embedded {
        kind: "macro".into(),
        name: "Module1".into(),
        detail: Some("vba, 120 bytes".into()),
        target: None,
    });
    doc.assets.push(Asset {
        id: "img1".into(),
        path: Some("assets/img1.png".into()),
        sha256: "11".repeat(32),
        mime: "image/png".into(),
        caption: Some("Chart".into()),
        alt: None,
        needs_vision: true,
    });
    doc.losses.push(Loss {
        location: "Costs!D1".into(),
        what: "chart".into(),
        why: LossReason::NotImplemented,
        severity: Severity::Warning,
    });
    doc
}

#[test]
fn document_roundtrips_through_json() {
    let doc = full_document();
    let json = serde_json::to_string(&doc).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(doc, back);
}

#[test]
fn model_version_is_1() {
    assert_eq!(MODEL_VERSION, 1);
    assert_eq!(Document::new(src()).model_version, 1);
}

#[test]
fn format_ids_serialize_snake_case() {
    assert_eq!(
        serde_json::to_string(&FormatId::ThreeMf).unwrap(),
        "\"three_mf\""
    );
    assert_eq!(
        serde_json::to_string(&FormatId::KicadPcb).unwrap(),
        "\"kicad_pcb\""
    );
    assert_eq!(
        serde_json::to_string(&FormatId::PostScript).unwrap(),
        "\"post_script\""
    );
}

#[test]
fn json_schema_generates() {
    let schema = schemars::schema_for!(Document);
    let value = serde_json::to_value(&schema).unwrap();
    assert!(value.is_object());
    assert!(value["properties"]["facets"].is_object());
}

#[test]
fn serde_shapes_are_tagged_snake_case() {
    let v = serde_json::to_value(Value::Number {
        value: 2.0,
        unit: None,
    })
    .unwrap();
    assert_eq!(v["type"], "number");
    assert_eq!(v["value"], 2.0);
    let v = serde_json::to_value(Value::DateTime { value: "x".into() }).unwrap();
    assert_eq!(v["type"], "date_time");
    let b = serde_json::to_value(Block::Media { asset: "a".into() }).unwrap();
    assert_eq!(b["type"], "media");
    let i = serde_json::to_value(Inline::FootnoteRef { id: "1".into() }).unwrap();
    assert_eq!(i["type"], "footnote_ref");
    assert_eq!(
        serde_json::to_value(Anchor("p4".into())).unwrap(),
        serde_json::json!("p4")
    );
    assert_eq!(
        serde_json::to_value(LossReason::NotImplemented).unwrap(),
        "not_implemented"
    );
    assert_eq!(
        serde_json::to_value(Completeness::Partial).unwrap(),
        "partial"
    );
}

#[test]
fn source_exposes_bytes() {
    let src = Source {
        path: None,
        name: "a.txt".into(),
        bytes: Bytes::Owned(Arc::new(b"hello".to_vec())),
        sha256: String::new(),
        size: 5,
        format: FormatId::Text,
        detection: Detection {
            confidence: Confidence::Guess,
            evidence: "utf-8".into(),
        },
        modified: None,
    };
    assert_eq!(src.bytes(), b"hello");
    assert!(format!("{src:?}").contains("5 bytes"));
}
