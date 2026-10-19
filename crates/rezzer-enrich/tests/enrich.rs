// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::time::Instant;

use rezzer_enrich::{EnrichOptions, enrich};
use rezzer_model::{
    Anchor, Annotation, Asset, Block, Cell, Confidence, Detection, Document, Edge, Event, FormatId,
    Formula, Graph, Inline, Loss, LossReason, Node, Role, SectionKind, Severity, SourceInfo, Value,
};

fn doc() -> Document {
    Document::new(SourceInfo {
        path: None,
        name: "t.csv".into(),
        sha256: "00".into(),
        size: 1,
        format: FormatId::Csv,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "test".into(),
        },
        modified: None,
        encoding: None,
        reader: None,
    })
}

fn a(s: &str) -> Anchor {
    Anchor(s.into())
}

fn txt(s: &str) -> Inline {
    Inline::Text {
        text: s.into(),
        emphasis: false,
        strong: false,
        strike: false,
    }
}

fn para(s: &str, anchor: Option<&str>) -> Block {
    Block::Paragraph {
        inlines: vec![txt(s)],
        role: None,
        anchor: anchor.map(a),
    }
}

fn formula(anchor: &str, refs: &[&str]) -> Formula {
    Formula {
        anchor: a(anchor),
        expression: "=A1".into(),
        dialect: "excel".into(),
        references: refs.iter().map(|r| a(r)).collect(),
        cached: None,
        role: None,
        pattern: None,
        latex: None,
        note: None,
    }
}

fn sheet(name: &str, rows: usize, cols: usize) -> Block {
    Block::Section {
        kind: SectionKind::Sheet,
        title: Some(name.into()),
        anchor: Some(a(name)),
        blocks: vec![Block::Table {
            caption: None,
            header_rows: 1,
            rows: (0..rows)
                .map(|r| {
                    (0..cols)
                        .map(|c| {
                            let mut cell = Cell::new(vec![txt("x")]);
                            cell.anchor =
                                Some(a(&format!("{name}!{}{}", (b'A' + c as u8) as char, r + 1)));
                            cell
                        })
                        .collect()
                })
                .collect(),
            alignments: vec![],
            anchor: None,
        }],
    }
}

fn only(disabled: &[&str]) -> EnrichOptions {
    // Helper: disable everything except what the test names is not needed; plain options.
    EnrichOptions {
        disabled: disabled.iter().map(|s| (*s).to_string()).collect(),
    }
}

#[test]
fn dangling_reference_becomes_loss() {
    let mut d = doc();
    d.body = vec![para("hello", Some("p1"))];
    d.facets.annotations.push(Annotation {
        kind: "comment".into(),
        author: None,
        date: None,
        anchor: Some(a("p1")),
        quoted: None,
        body: vec![],
        replies: vec![],
        resolved: None,
    });
    d.facets.annotations.push(Annotation {
        kind: "comment".into(),
        author: None,
        date: None,
        anchor: Some(a("gone")),
        quoted: None,
        body: vec![],
        replies: vec![],
        resolved: None,
    });
    enrich(&mut d, &only(&[]));
    assert_eq!(d.losses.len(), 1, "{:?}", d.losses);
    let l = &d.losses[0];
    assert_eq!(l.what, "reference to gone not found");
    assert_eq!(l.why, LossReason::Damaged);
    assert_eq!(l.severity, Severity::Warning);
}

#[test]
fn references_resolve_through_every_anchor_place() {
    let mut d = doc();
    d.body = vec![
        sheet("Costs", 2, 2),
        Block::Quote {
            blocks: vec![Block::Paragraph {
                inlines: vec![Inline::Link {
                    target: "x".into(),
                    title: None,
                    internal: false,
                    inlines: vec![Inline::AnchorMark { anchor: a("deep") }],
                }],
                role: None,
                anchor: None,
            }],
            anchor: Some(a("q1")),
        },
    ];
    d.facets.graphs.push(Graph {
        id: "g".into(),
        kind: "feature-tree".into(),
        nodes: vec![Node {
            id: "n".into(),
            label: "N".into(),
            kind: None,
            anchor: Some(a("q1")),
        }],
        edges: vec![],
    });
    d.facets.formulas = vec![
        formula(
            "Costs!B2",
            &["Costs!A1", "Costs!A:A", "Costs!A1:B2", "'Costs'!A1"],
        ),
        formula("Costs!C9", &["deep", "q1"]),
    ];
    d.facets.timeline.push(Event {
        time: None,
        actor: None,
        kind: "note".into(),
        anchor: Some(a("deep")),
        text: None,
    });
    enrich(&mut d, &only(&[]));
    assert!(d.losses.is_empty(), "{:?}", d.losses);

    // A range on an unknown sheet is not resolved.
    d.facets.formulas.push(formula("Costs!A1", &["Other!A:A"]));
    enrich(&mut d, &only(&[]));
    assert_eq!(d.losses.len(), 1);
    assert_eq!(d.losses[0].what, "reference to Other!A:A not found");
}

#[test]
fn unresolved_references_are_capped() {
    let mut d = doc();
    d.body = vec![para("x", Some("p1"))];
    let refs: Vec<String> = (0..250).map(|i| format!("missing{i}")).collect();
    let refs: Vec<&str> = refs.iter().map(String::as_str).collect();
    d.facets.formulas = vec![formula("p1", &refs)];
    enrich(&mut d, &only(&[]));
    assert_eq!(d.losses.len(), 101);
    assert!(d.losses[100].what.contains("150"), "{:?}", d.losses[100]);
}

const GERMAN: &str = "Dies ist ein längerer deutscher Text über die Entwicklung der Software in \
    den letzten Jahren. Die Anwendung liest viele Dateien und macht daraus Wissen für Agenten, \
    damit sie die Inhalte besser verstehen können. Das Ergebnis ist ein klarer Text.";

#[test]
fn language_detected_for_german_text() {
    let mut d = doc();
    d.body = vec![para(GERMAN, None)];
    enrich(&mut d, &only(&[]));
    assert_eq!(d.properties.language.as_deref(), Some("de"));
}

#[test]
fn language_not_set_for_short_text() {
    let mut d = doc();
    d.body = vec![para("Das ist kurz.", None)];
    enrich(&mut d, &only(&[]));
    assert_eq!(d.properties.language, None);
}

#[test]
fn language_kept_when_already_set() {
    let mut d = doc();
    d.properties.language = Some("fr".into());
    d.body = vec![para(GERMAN, None)];
    enrich(&mut d, &only(&[]));
    assert_eq!(d.properties.language.as_deref(), Some("fr"));
}

#[test]
fn overview_lists_sections_and_counts() {
    let mut d = doc();
    d.properties.created = Some(Value::Date {
        value: "2026-01-01".into(),
    });
    d.properties.modified = Some(Value::DateTime {
        value: "2026-02-03T04:05:06Z".into(),
    });
    d.body = vec![
        sheet("Input", 10, 3),
        sheet("Calc", 4, 2),
        Block::Section {
            kind: SectionKind::Chapter,
            title: None,
            anchor: None,
            blocks: vec![para("a", None), para("b", None)],
        },
        Block::Media {
            asset: "img1".into(),
            anchor: None,
        },
    ];
    d.assets.push(Asset {
        id: "img1".into(),
        path: None,
        sha256: "00".into(),
        mime: "image/png".into(),
        caption: None,
        alt: None,
        needs_vision: false,
    });
    d.facets.formulas = vec![formula("Calc!A1", &["Input!A1", "Input!B2"])];
    d.facets.graphs.push(Graph {
        id: "l".into(),
        kind: "link".into(),
        nodes: vec![],
        edges: vec![Edge {
            from: "a".into(),
            to: "b".into(),
            kind: "link".into(),
            anchor: None,
        }],
    });
    d.facets.timeline.push(Event {
        time: None,
        actor: None,
        kind: "save".into(),
        anchor: None,
        text: None,
    });
    d.losses.push(Loss {
        location: "x".into(),
        what: "y".into(),
        why: LossReason::Unsupported,
        severity: Severity::Info,
    });
    enrich(&mut d, &only(&[]));
    let o = d.overview.expect("overview");
    assert_eq!(o.parts.len(), 3);
    assert_eq!(o.parts[0].name, "Input");
    assert_eq!(o.parts[0].kind, "sheet");
    assert_eq!(o.parts[0].size, "10 rows x 3 columns");
    assert_eq!(o.parts[0].role, Some(Role::Input));
    assert!(o.parts[0].reads.is_empty());
    assert_eq!(o.parts[1].role, Some(Role::Result));
    assert_eq!(o.parts[1].reads, vec!["Input".to_string()]);
    assert_eq!(o.parts[2].name, "Part 3");
    assert_eq!(o.parts[2].kind, "chapter");
    assert_eq!(o.parts[2].size, "2 paragraphs");
    assert_eq!(o.parts[2].role, None);
    let facts: Vec<(&str, &str)> = o
        .facts
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        facts,
        vec![
            ("Formulas", "1"),
            ("Graphs", "1"),
            ("Links", "1"),
            ("Images", "1"),
            ("Changes", "1"),
            ("Not read", "1"),
            ("Last changed", "2026-02-03T04:05:06Z"),
        ]
    );
}

#[test]
fn last_changed_falls_back_to_created() {
    let mut d = doc();
    d.properties.created = Some(Value::Date {
        value: "2026-01-01".into(),
    });
    d.body = vec![para("x", None)];
    enrich(&mut d, &only(&[]));
    let o = d.overview.expect("overview");
    assert_eq!(
        o.facts,
        vec![("Last changed".to_string(), "2026-01-01".to_string())]
    );
}

#[test]
fn disabled_pass_is_skipped() {
    let mut d = doc();
    d.body = vec![para(GERMAN, None)];
    enrich(&mut d, &only(&["language", "overview"]));
    assert_eq!(d.properties.language, None);
    assert!(d.overview.is_none());
    enrich(&mut d, &only(&["anchors", "overview"]));
    assert_eq!(d.properties.language.as_deref(), Some("de"));
    let mut d2 = doc();
    d2.facets.formulas = vec![formula("zz", &[])];
    enrich(&mut d2, &only(&["anchors"]));
    assert!(d2.losses.is_empty());
}

#[test]
fn big_document_is_fast() {
    let mut d = doc();
    d.body = (0..100_000)
        .map(|i| {
            para(
                &format!("Paragraph number {i} of the big test document."),
                Some(&format!("p{i}")),
            )
        })
        .collect();
    d.facets.formulas = (0..1000)
        .map(|i| formula(&format!("p{i}"), &["p5", "nope"]))
        .collect();
    let start = Instant::now();
    enrich(&mut d, &only(&[]));
    assert!(start.elapsed().as_secs() < 10, "{:?}", start.elapsed());
    assert_eq!(d.losses.len(), 101);
}

#[test]
fn dangling_node_anchor_becomes_loss() {
    let mut d = doc();
    d.body = vec![para("x", Some("h:intro"))];
    d.facets.graphs.push(Graph {
        id: "g".into(),
        kind: "link".into(),
        nodes: vec![
            Node {
                id: "a".into(),
                label: "A".into(),
                kind: None,
                anchor: Some(a("h:intro")),
            },
            Node {
                id: "b".into(),
                label: "B".into(),
                kind: None,
                anchor: Some(a("h:gone")),
            },
        ],
        edges: vec![],
    });
    enrich(&mut d, &only(&[]));
    assert_eq!(d.losses.len(), 1, "{:?}", d.losses);
    assert_eq!(d.losses[0].what, "reference to h:gone not found");
}

#[test]
fn enrich_is_idempotent() {
    let mut d = doc();
    d.body = vec![para(GERMAN, Some("p1")), sheet("S", 2, 2)];
    let refs: Vec<String> = (0..150).map(|i| format!("missing{i}")).collect();
    let refs: Vec<&str> = refs.iter().map(String::as_str).collect();
    d.facets.formulas = vec![formula("p1", &refs)];
    enrich(&mut d, &only(&[]));
    let once = d.clone();
    enrich(&mut d, &only(&[]));
    assert_eq!(d, once);
}
