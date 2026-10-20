// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::time::Instant;

use rezzer_enrich::{EnrichOptions, enrich};
use rezzer_model::{
    Anchor, Annotation, Asset, Block, Cell, Confidence, Detection, Document, Edge, Event, FormatId,
    Formula, Graph, Inline, Loss, LossReason, Node, Parameter, Role, Rule, SectionKind, Severity,
    SourceInfo, Value,
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
        kind: None,
        range: None,
        original: None,
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
            role: None,
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
    // A broken reference is no content left unread: uncertain, not damaged.
    assert_eq!(l.why, LossReason::Uncertain);
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
                    wiki: false,
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
            kind: "links_to".into(),
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
    // Ten table rows, one of them the header.
    assert_eq!(o.parts[0].size, "9 rows x 3 columns");
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

#[test]
fn numeric_table_gets_no_language() {
    let mut d = doc();
    let rows: Vec<Vec<Cell>> = std::iter::once(vec!["Year", "Total", "Gas Fuel", "Liquid Fuel"])
        .chain(std::iter::repeat_n(
            vec!["1999", "6.5e3", "2025-01-02", "A12-B7"],
            300,
        ))
        .map(|r| r.into_iter().map(|c| Cell::new(vec![txt(c)])).collect())
        .collect();
    d.body = vec![Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: vec![],
        anchor: None,
    }];
    enrich(&mut d, &only(&[]));
    assert_eq!(d.properties.language, None);
}

#[test]
fn links_fact_counts_only_links_to_edges() {
    let mut d = doc();
    d.body = vec![para("x", None)];
    let edge = |kind: &str| Edge {
        from: "a".into(),
        to: "b".into(),
        kind: kind.into(),
        anchor: None,
    };
    d.facets.graphs.push(Graph {
        id: "l".into(),
        kind: "link".into(),
        nodes: vec![],
        edges: vec![edge("links_to"), edge("embeds"), edge("links_to")],
    });
    enrich(&mut d, &only(&[]));
    let facts = d.overview.unwrap().facts;
    assert!(
        facts.contains(&("Links".to_string(), "2".to_string())),
        "{facts:?}"
    );
}

#[test]
fn footnote_wrapper_is_not_a_part() {
    let mut d = doc();
    let note = Block::Section {
        kind: SectionKind::Part,
        title: None,
        anchor: Some(a("fn:1")),
        blocks: vec![para("note", None)],
    };
    d.body = vec![
        Block::Section {
            kind: SectionKind::Chapter,
            title: Some("Main".into()),
            anchor: None,
            blocks: vec![para("x", None)],
        },
        Block::Section {
            kind: SectionKind::Other,
            title: Some("Footnotes".into()),
            anchor: None,
            blocks: vec![note],
        },
    ];
    enrich(&mut d, &only(&[]));
    let parts = d.overview.unwrap().parts;
    assert_eq!(parts.len(), 1, "{parts:?}");
    assert_eq!(parts[0].name, "Main");
}

#[test]
fn odd_sheet_names_resolve() {
    for name in ["a!b.csv", "'q.csv", "it's data", "my data"] {
        let mut d = doc();
        d.body = vec![sheet(name, 2, 2)];
        d.facets.formulas = vec![formula(
            &format!("{name}!B2"),
            &[&format!("{name}!A:A"), &format!("{name}!A1:B2")],
        )];
        enrich(&mut d, &only(&[]));
        assert!(d.losses.is_empty(), "{name}: {:?}", d.losses);
    }
}

#[test]
fn parts_are_linked_by_id_not_name() {
    let mut d = doc();
    let sec = |anchor: &str| Block::Section {
        kind: SectionKind::Sheet,
        title: Some("Same".into()),
        anchor: Some(a(anchor)),
        blocks: vec![para("x", Some(&format!("{anchor}!A1")))],
    };
    d.body = vec![sec("s1"), sec("s2")];
    d.facets.formulas = vec![formula("s2!A1", &["s1!A1"])];
    enrich(&mut d, &only(&[]));
    let parts = d.overview.unwrap().parts;
    assert_eq!(parts[0].id, "s1");
    assert_eq!(parts[1].reads, vec!["s1".to_string()]);
    assert!(parts[0].reads.is_empty());
}

fn read_csv(bytes: &[u8]) -> Document {
    use rezzer_model::{Bytes, ReadOptions, Reader, Source};
    let source = Source {
        path: None,
        name: "data.csv".into(),
        bytes: Bytes::Owned(std::sync::Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format: FormatId::Csv,
        detection: Detection {
            confidence: Confidence::Likely,
            evidence: "csv".into(),
        },
        modified: None,
    };
    rezzer_read_csv::CsvReader
        .read(
            &source,
            &ReadOptions::default(),
            &rezzer_limits::Budget::new(rezzer_limits::Limits::default()),
        )
        .document
}

#[test]
fn csv_columns_summary_does_not_set_a_language() {
    let names = [
        "Year",
        "Total",
        "Gas Fuel",
        "Liquid Fuel",
        "Solid Fuel",
        "Cement",
        "Flaring",
        "Per Capita",
        "Bunker Fuels",
        "Gas Flaring",
        "Coal Burning",
        "Oil Burning",
    ];
    let mut csv = names.join(",");
    csv.push('\n');
    for y in 1900..2100 {
        let row: Vec<String> = (0..names.len())
            .map(|c| format!("{}.{}", y * (c + 1), c % 7))
            .collect();
        csv.push_str(&row.join(","));
        csv.push('\n');
    }
    let mut d = read_csv(csv.as_bytes());
    enrich(&mut d, &only(&[]));
    assert_eq!(d.properties.language, None);
}

#[test]
fn csv_of_english_sentences_is_english() {
    let mut csv = String::from("id,text\n");
    for i in 0..12 {
        csv.push_str(&format!(
            "{i},\"The quick brown fox jumps over the lazy dog while the weather stays warm and everybody enjoys the long afternoon.\"\n"
        ));
    }
    let mut d = read_csv(csv.as_bytes());
    enrich(&mut d, &only(&[]));
    assert_eq!(d.properties.language.as_deref(), Some("en"));
}

fn custom_number(d: &mut Document, key: &str, n: f64) {
    d.properties
        .custom
        .insert(key.into(), rezzer_model::Value::number(n, None));
}

#[test]
fn a_single_sheet_takes_its_size_from_the_reader() {
    let mut d = doc();
    d.body.push(sheet("data", 11, 4));
    custom_number(&mut d, "rows", 30_000.0);
    custom_number(&mut d, "columns", 6.0);
    enrich(&mut d, &only(&[]));
    let o = d.overview.expect("overview");
    assert_eq!(o.parts[0].size, "30000 rows x 6 columns (10 shown)");
}

#[test]
fn a_sheet_with_every_row_shown_has_no_shown_note() {
    let mut d = doc();
    d.body.push(sheet("data", 10, 9));
    custom_number(&mut d, "rows", 9.0);
    custom_number(&mut d, "columns", 9.0);
    enrich(&mut d, &only(&[]));
    assert_eq!(d.overview.unwrap().parts[0].size, "9 rows x 9 columns");
}

#[test]
fn a_summary_table_is_not_the_sheet_data() {
    // After a time cut the sheet holds only the reader's Columns table.
    let mut d = doc();
    d.body.push(Block::Section {
        kind: SectionKind::Sheet,
        title: Some("data".into()),
        anchor: Some(a("data")),
        blocks: vec![Block::Table {
            caption: Some("Columns".into()),
            header_rows: 1,
            role: Some("summary".into()),
            rows: vec![vec![Cell::new(vec![txt("Column")])]; 5],
            alignments: vec![],
            anchor: None,
        }],
    });
    custom_number(&mut d, "rows", 4094.0);
    custom_number(&mut d, "columns", 2.0);
    enrich(&mut d, &only(&[]));
    assert_eq!(
        d.overview.unwrap().parts[0].size,
        "4094 rows x 2 columns (none shown)"
    );
}

#[test]
fn workbook_names_and_external_references_resolve() {
    use rezzer_model::FormulaKind;
    let mut d = doc();
    d.body = vec![sheet("S", 1, 1), sheet("'My T'", 1, 1)];
    let mut gg = formula("name:gg", &["S!A1"]);
    gg.kind = Some(FormulaKind::Name);
    let mut twice = formula("name:S!twice", &[]);
    twice.kind = Some(FormulaKind::Lambda);
    d.facets.formulas = vec![
        gg,
        twice,
        formula(
            "S!A1",
            &[
                "name:gg",
                "name:S!twice",
                "ext:[1]Sheet1!A1",
                "ext:[2]!fn",
                "S:'My T'!A1:B2",
                "'S:My T'!B5",
            ],
        ),
    ];
    enrich(&mut d, &only(&[]));
    assert!(d.losses.is_empty(), "{:?}", d.losses);

    // A name the workbook does not define stays a loss, also when a cell formula has it as
    // its own anchor.
    d.facets
        .formulas
        .push(formula("S!A1", &["name:zz", "S:Other!A1"]));
    d.facets.formulas.push(formula("name:cell", &[]));
    enrich(&mut d, &only(&[]));
    let what: Vec<_> = d.losses.iter().map(|l| l.what.as_str()).collect();
    assert_eq!(
        what,
        [
            "reference to name:zz not found",
            "reference to S:Other!A1 not found",
            "reference to name:cell not found"
        ]
    );
}

#[test]
fn parameter_rule_and_formula_range_anchors_are_checked() {
    use rezzer_model::FormulaKind;
    let mut d = doc();
    d.body = vec![sheet("S", 2, 2)];
    let mut gg = formula("name:gg", &[]);
    gg.kind = Some(FormulaKind::Name);
    let mut spill = formula("S!A1", &[]);
    spill.range = Some(a("S!A1:B2"));
    d.facets.formulas = vec![gg, spill];
    let param = |anchor: &str| Parameter {
        anchor: a(anchor),
        name: None,
        label: None,
        value: Some(Value::number(1.0, None)),
        unit: None,
        unit_source: None,
        read_by: 1,
    };
    let rule = |anchor: &str| Rule {
        kind: "validation".into(),
        anchor: a(anchor),
        detail: "whole number".into(),
        formulas: vec![],
        message: None,
    };
    d.facets.parameters = vec![param("S!B1"), param("name:gg")];
    d.facets.rules = vec![rule("S!A1:A2 B1:B2")];
    enrich(&mut d, &only(&[]));
    assert!(d.losses.is_empty(), "{:?}", d.losses);

    let mut gone = formula("S!B2", &[]);
    gone.range = Some(a("Gone!A1:A3"));
    d.facets.formulas.push(gone);
    d.facets.parameters.push(param("Gone!C3"));
    d.facets.rules.push(Rule {
        kind: "conditional-format".into(),
        ..rule("Lost!A1:A9")
    });
    enrich(&mut d, &only(&[]));
    let found: Vec<_> = d
        .losses
        .iter()
        .map(|l| (l.location.as_str(), l.what.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("S!B2", "reference to Gone!A1:A3 not found"),
            ("parameter", "reference to Gone!C3 not found"),
            (
                "rule (conditional-format)",
                "reference to Lost!A1:A9 not found"
            ),
        ]
    );
}

#[test]
fn workbook_overview_counts() {
    let mut d = doc();
    d.source.format = FormatId::Xlsx;
    let hidden = |name: &str, state: &str| Block::Section {
        kind: SectionKind::Sheet,
        title: Some(name.into()),
        anchor: Some(a(name)),
        blocks: vec![Block::Paragraph {
            inlines: vec![Inline::Status {
                text: state.into(),
                color: rezzer_model::StatusColor::Grey,
            }],
            role: Some("status".into()),
            anchor: None,
        }],
    };
    d.body = vec![
        sheet("Input", 3, 2),
        sheet("Calc", 3, 2),
        hidden("Old", "hidden"),
        hidden("Secret", "very hidden"),
    ];
    let with = |f: Formula, role: Option<Role>, pattern: Option<&str>| Formula {
        role,
        pattern: pattern.map(Into::into),
        kind: Some(rezzer_model::FormulaKind::Cell),
        ..f
    };
    d.facets.formulas = vec![
        with(formula("Calc!A1", &["Input!A1"]), Some(Role::Result), None),
        with(
            formula("Calc!A2", &["Input!A2"]),
            Some(Role::Intermediate),
            Some("p1"),
        ),
        with(
            formula("Calc!A3", &["Input!A3"]),
            Some(Role::Intermediate),
            Some("p1"),
        ),
        Formula {
            kind: Some(rezzer_model::FormulaKind::Name),
            ..formula("name:rate", &["Input!A1"])
        },
        Formula {
            kind: Some(rezzer_model::FormulaKind::Lambda),
            ..formula("name:Twice", &[])
        },
    ];
    d.facets.parameters = vec![rezzer_model::Parameter {
        anchor: a("Input!A1"),
        name: None,
        label: None,
        value: None,
        unit: None,
        unit_source: None,
        read_by: 1,
    }];
    d.facets.annotations = vec![Annotation {
        kind: "comment".into(),
        author: None,
        date: None,
        anchor: Some(a("Calc!A1")),
        quoted: None,
        body: vec![txt("check")],
        replies: Vec::new(),
        resolved: None,
    }];
    let embedded = |kind: &str| rezzer_model::Embedded {
        text: None,
        kind: kind.into(),
        name: "x".into(),
        detail: None,
        target: None,
    };
    d.facets.embedded = vec![
        embedded("external-workbook"),
        embedded("add-in"),
        embedded("macro"),
    ];
    d.facets.graphs.push(Graph {
        id: "dependencies".into(),
        kind: "dependency".into(),
        nodes: vec![],
        edges: vec![],
    });
    enrich(&mut d, &only(&[]));
    let facts: Vec<(String, String)> = d.overview.expect("overview").facts;
    let facts: Vec<(&str, &str)> = facts
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        facts,
        vec![
            ("Sheets", "4 (hidden: Old, Secret)"),
            ("Formulas", "3"),
            ("Patterns", "1"),
            ("Parameters", "1"),
            ("Results", "1"),
            ("Names", "2"),
            ("Comments", "1"),
            ("External links", "2"),
            ("Embedded", "3"),
        ]
    );
}
