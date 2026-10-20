// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use rezzer_model::{
    Align, Anchor, Annotation, Asset, Block, CalloutKind, Cell, Completeness, Confidence,
    Detection, Document, Edge, Embedded, Event as TimelineEvent, FormatId, Formula, FormulaKind,
    Graph, Inline, Loss, LossReason, Node, Overview, OverviewPart, Parameter, ReaderInfo, Role,
    Rule, SectionKind, Severity, SourceInfo, StatusColor, TaskState, Value,
};
use rezzer_render::{OutputSection, RenderOptions, paginate, render, render_overview};

fn source() -> SourceInfo {
    SourceInfo {
        path: Some("/data/report.md".into()),
        name: "report.md".into(),
        sha256: "abc123".into(),
        size: 42,
        format: FormatId::Markdown,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "extension .md and text".into(),
        },
        modified: Some("2026-01-05T10:00:00Z".into()),
        encoding: Some("utf-8".into()),
        reader: Some(ReaderInfo {
            id: "markdown".into(),
            version: "0.1.0".into(),
        }),
    }
}

fn doc() -> Document {
    Document::new(source())
}

fn opts() -> RenderOptions {
    RenderOptions {
        sections: None,
        generated_at: "2026-10-20T12:00:00Z".into(),
        rezzer_version: "0.1.0".into(),
        archive_ref: None,
        math: true,
    }
}

fn txt(text: &str) -> Inline {
    Inline::Text {
        text: text.into(),
        emphasis: false,
        strong: false,
        strike: false,
    }
}

fn para(text: &str) -> Block {
    Block::Paragraph {
        inlines: vec![txt(text)],
        role: None,
        anchor: None,
    }
}

fn body_of(blocks: Vec<Block>) -> String {
    let mut d = doc();
    d.body = blocks;
    render(&d, Completeness::Complete, &opts())
}

/// The part after the frontmatter.
fn after_frontmatter(md: &str) -> &str {
    let rest = md.strip_prefix("---\n").expect("frontmatter");
    let end = rest.find("\n---\n").expect("frontmatter end");
    &rest[end + 5..]
}

fn frontmatter_of(md: &str) -> &str {
    let rest = md.strip_prefix("---\n").expect("frontmatter");
    let end = rest.find("\n---\n").expect("frontmatter end");
    &rest[..end + 1]
}

fn loss(location: &str, what: &str, why: LossReason, severity: Severity) -> Loss {
    Loss {
        location: location.into(),
        what: what.into(),
        why,
        severity,
    }
}

fn formula(anchor: &str, expr: &str, refs: &[&str], pattern: Option<&str>) -> Formula {
    Formula {
        anchor: Anchor(anchor.into()),
        expression: expr.into(),
        dialect: "excel".into(),
        references: refs.iter().map(|r| Anchor((*r).into())).collect(),
        cached: None,
        role: Some(Role::Result),
        pattern: pattern.map(Into::into),
        latex: None,
        note: None,
        kind: None,
        range: None,
        original: None,
    }
}

fn node(id: &str, label: &str) -> Node {
    Node {
        id: id.into(),
        label: label.into(),
        kind: None,
        anchor: None,
    }
}

fn edge(from: &str, to: &str, kind: &str) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        kind: kind.into(),
        anchor: None,
    }
}

#[test]
fn frontmatter_key_order() {
    let mut d = doc();
    d.properties.title = Some("Report".into());
    d.properties.description = Some("A test".into());
    d.properties.keywords = vec!["alpha".into(), "beta".into()];
    d.properties.authors = vec!["Ann \"A\"".into(), "Bob".into()];
    d.properties.language = Some("de".into());
    d.properties.created = Some(Value::DateTime {
        value: "2025-11-30T08:00:00Z".into(),
    });
    d.properties.modified = Some(Value::Date {
        value: "2025-12-01".into(),
    });
    d.losses.push(loss(
        "page 1",
        "a picture",
        LossReason::Unsupported,
        Severity::Info,
    ));
    let mut o = opts();
    o.archive_ref = Some("rezzer://sha256/abc123/report.md".into());
    let md = render(&d, Completeness::Partial, &o);
    let expected = "---\n\
type: document\n\
title: \"Report\"\n\
description: \"A test\"\n\
resource: \"/data/report.md\"\n\
tags: [\"alpha\", \"beta\"]\n\
generated: { by: rezzer/0.1.0, at: 2026-10-20T12:00:00Z }\n\
source_date: 2025-12-01\n\
temporal_confidence: high\n\
sources: [\"rezzer://sha256/abc123/report.md\"]\n\
source_sha256: \"abc123\"\n\
source_size: 42\n\
format: markdown\n\
detected_by: \"extension .md and text\"\n\
encoding: \"utf-8\"\n\
reader: \"markdown/0.1.0\"\n\
completeness: partial\n\
losses: 1\n\
authors: [\"Ann \\\"A\\\"\", \"Bob\"]\n\
language: \"de\"\n\
keywords: [\"alpha\", \"beta\"]\n\
created: \"2025-11-30T08:00:00Z\"\n\
modified: \"2025-12-01\"\n\
---\n\
\n\
# Report\n";
    assert!(md.starts_with(expected), "{md}");
}

#[test]
fn frontmatter_minimal_uses_file_name_and_file_date() {
    let mut s = source();
    s.path = None;
    s.encoding = None;
    s.reader = None;
    let d = Document::new(s);
    let md = render(&d, Completeness::Complete, &opts());
    let expected = "---\n\
type: document\n\
title: \"report.md\"\n\
resource: \"report.md\"\n\
generated: { by: rezzer/0.1.0, at: 2026-10-20T12:00:00Z }\n\
source_date: 2026-01-05\n\
temporal_confidence: medium\n\
source_sha256: \"abc123\"\n\
source_size: 42\n\
format: markdown\n\
detected_by: \"extension .md and text\"\n\
completeness: complete\n\
losses: 0\n\
---\n\
\n\
# report.md\n";
    assert_eq!(md, expected);
}

#[test]
fn frontmatter_escapes_file_values() {
    let mut d = doc();
    d.properties.title = Some("He said: \"hi\"\nbye".into());
    let md = render(&d, Completeness::Complete, &opts());
    assert!(
        md.contains("title: \"He said: \\\"hi\\\"\\nbye\"\n"),
        "{md}"
    );
    // The heading is one line.
    assert!(md.contains("\n# He said: \"hi\" bye\n"), "{md}");
}

#[test]
fn frontmatter_round_trips_hostile_values_through_yaml() {
    let hostile = "a\\b \"q\"\n\t\u{1}\u{7f}\u{85}\u{2028}\u{feff}---\n# x: y";
    let mut d = doc();
    d.properties.title = Some(hostile.into());
    d.properties.keywords = vec![hostile.into()];
    d.source.detection.evidence = hostile.into();
    let md = render(&d, Completeness::Complete, &opts());
    let fm = frontmatter_of(&md);
    let mut scalars = Vec::new();
    for item in saphyr_parser::Parser::new_from_str(fm) {
        let (event, _) = item.expect("valid yaml");
        if let saphyr_parser::Event::Scalar(value, _, _, _) = event {
            scalars.push(value.into_owned());
        }
    }
    let pos = scalars
        .iter()
        .position(|s| s == "title")
        .expect("title key");
    assert_eq!(scalars[pos + 1], hostile);
    let pos = scalars.iter().position(|s| s == "tags").expect("tags key");
    assert_eq!(scalars[pos + 1], hostile);
    let pos = scalars
        .iter()
        .position(|s| s == "detected_by")
        .expect("detected_by key");
    assert_eq!(scalars[pos + 1], hostile);
}

#[test]
fn callout_folded() {
    let md = body_of(vec![
        Block::Callout {
            kind: CalloutKind::Details,
            title: Some("Rollback".into()),
            folded: Some(true),
            blocks: vec![para("Stop it."), para("Restore.")],
            anchor: None,
        },
        Block::Callout {
            kind: CalloutKind::Note,
            title: Some("Background".into()),
            folded: Some(false),
            blocks: vec![para("Open.")],
            anchor: None,
        },
        Block::Callout {
            kind: CalloutKind::Warning,
            title: None,
            folded: None,
            blocks: vec![para("Careful.")],
            anchor: None,
        },
    ]);
    assert!(
        md.contains("> [!DETAILS]- Rollback\n> Stop it.\n>\n> Restore.\n"),
        "{md}"
    );
    assert!(md.contains("> [!NOTE]+ Background\n> Open.\n"), "{md}");
    assert!(md.contains("> [!WARNING]\n> Careful.\n"), "{md}");
}

#[test]
fn status_label() {
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![
            txt("The rollout is "),
            Inline::Status {
                text: "In review".into(),
                color: StatusColor::Blue,
            },
            txt("."),
        ],
        role: None,
        anchor: None,
    }]);
    assert!(
        md.contains("The rollout is :status[In review]{blue}.\n"),
        "{md}"
    );
}

#[test]
fn math_inline_and_block() {
    let md = body_of(vec![
        Block::Paragraph {
            inlines: vec![
                txt("Inline "),
                Inline::Math {
                    latex: "a^2 + b^2".into(),
                },
                txt(" and "),
                Inline::Math {
                    latex: "a`b".into(),
                },
            ],
            role: None,
            anchor: None,
        },
        Block::Math {
            latex: "E = mc^2".into(),
            source_kind: "latex".into(),
            anchor: None,
        },
        Block::Math {
            latex: "a\n$$\nb".into(),
            source_kind: "latex".into(),
            anchor: None,
        },
    ]);
    assert!(md.contains("Inline $`a^2 + b^2`$ and $$a`b$$\n"), "{md}");
    assert!(md.contains("\n$$\nE = mc^2\n$$\n"), "{md}");
    assert!(md.contains("\n```math\na\n$$\nb\n```\n"), "{md}");
}

#[test]
fn task_states() {
    let task = |state, text: &str| Block::Task {
        state,
        inlines: vec![txt(text)],
        anchor: None,
    };
    let md = body_of(vec![
        task(TaskState::Open, "open"),
        task(TaskState::Done, "done"),
        task(TaskState::Cancelled, "not applicable"),
    ]);
    assert!(
        md.contains("\n- [ ] open\n- [x] done\n- [~] not applicable\n"),
        "{md}"
    );
}

#[test]
fn lists_nest_and_neighbours_do_not_merge() {
    let list = |ordered, items: Vec<Vec<Block>>| Block::List {
        ordered,
        items,
        anchor: None,
    };
    let md = body_of(vec![
        list(
            false,
            vec![
                vec![para("one"), list(true, vec![vec![para("inner")]])],
                vec![para("two")],
            ],
        ),
        list(false, vec![vec![para("three")]]),
    ]);
    assert!(
        md.contains("\n- one\n  1. inner\n- two\n\n* three\n"),
        "{md}"
    );
}

#[test]
fn inline_styles_links_images_breaks() {
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![
            Inline::Text {
                text: "bold ".into(),
                emphasis: false,
                strong: true,
                strike: false,
            },
            Inline::Text {
                text: "gone".into(),
                emphasis: false,
                strong: false,
                strike: true,
            },
            Inline::LineBreak,
            Inline::Link {
                target: "https://example.org/a b".into(),
                title: Some("T \"q\"".into()),
                internal: false,
                wiki: false,
                inlines: vec![txt("site")],
            },
            txt(" "),
            Inline::Image {
                target: "img/p.png".into(),
                alt: "a [pic]".into(),
                title: Some("cap".into()),
            },
            txt(" "),
            Inline::Code { text: "a`b".into() },
            Inline::FootnoteRef { id: "1".into() },
            Inline::LineBreak,
        ],
        role: None,
        anchor: None,
    }]);
    assert!(
        md.contains(
            "**bold** ~~gone~~\\\n[site](https://example.org/a%20b \"T \\\"q\\\"\") \
![a \\[pic\\]](img/p.png \"cap\") ``a`b``[^1]\n"
        ),
        "{md}"
    );
}

#[test]
fn table_alignment_merged_cells_and_pipes() {
    let mut merged = Cell::new(vec![txt("wide")]);
    merged.col_span = 2;
    let md = body_of(vec![Block::Table {
        caption: Some("Costs".into()),
        header_rows: 1,
        role: None,
        rows: vec![
            vec![
                Cell::new(vec![txt("A")]),
                Cell::new(vec![txt("B")]),
                Cell::new(vec![txt("C")]),
            ],
            vec![Cell::new(vec![txt("x | y")]), merged],
            vec![
                Cell::new(vec![Inline::Code { text: "a|b".into() }]),
                Cell::new(vec![txt("-5")]),
            ],
        ],
        alignments: vec![Align::Left, Align::Center, Align::Right],
        anchor: None,
    }]);
    let expected = "**Costs**\n\n\
| A | B | C |\n\
| :--- | :---: | ---: |\n\
| x \\| y | wide | wide |\n\
| `a\\|b` | -5 |  |\n\
\n\
Merged cells: B2:C2\n";
    assert!(md.contains(expected), "{md}");
}

#[test]
fn rule_is_never_frontmatter_or_setext() {
    let md = body_of(vec![
        para("above"),
        Block::Rule { anchor: None },
        para("below"),
    ]);
    assert!(md.contains("\nabove\n\n---\n\nbelow\n"), "{md}");
}

#[test]
fn code_fence_is_longer_than_inner_backticks() {
    let md = body_of(vec![Block::Code {
        lang: Some("rust".into()),
        text: "let a = 1;\n````\n".into(),
        anchor: None,
    }]);
    assert!(
        md.contains("\n`````rust\nlet a = 1;\n````\n`````\n"),
        "{md}"
    );
}

#[test]
fn hostile_text_injects_no_structure() {
    let hostile = [
        "---\ntitle: x\n---",
        "<!-- -->",
        "| a |",
        "# not a heading",
        "$`x`$",
        "1. not a list\n> not a quote\n    - not code\n===",
        "[!WARNING] not a callout *x* _y_ ~z~ [l](u) \\",
    ];
    let md = body_of(hostile.iter().map(|t| para(t)).collect());
    assert!(!md.contains("<!--"), "{md}");
    let body = after_frontmatter(&md);
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM;
    let mut texts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_para = false;
    for event in Parser::new_ext(body, opts) {
        match event {
            Event::Start(Tag::Heading { .. }) | Event::End(TagEnd::Heading(_)) => {}
            Event::Start(Tag::Paragraph) => {
                in_para = true;
                current.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                in_para = false;
                texts.push(current.clone());
            }
            Event::Text(t) if in_para => current.push_str(&t),
            Event::Text(_) => {}
            Event::SoftBreak => current.push('\n'),
            other => panic!("unexpected event {other:?} in\n{body}"),
        }
    }
    // The first paragraph-free event is the title heading; the rest are the paragraphs.
    let expected: Vec<String> = hostile
        .iter()
        .map(|t| {
            t.lines()
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect();
    assert_eq!(texts, expected, "{body}");
}

#[test]
fn logic_table_and_mermaid() {
    let mut d = doc();
    d.facets.formulas = vec![
        Formula {
            cached: Some(Value::Number {
                value: 12.0,
                unit: Some("kg".into()),
            }),
            ..formula(
                "Sheet1!D1",
                "=SUM(C2:C3)",
                &["Sheet1!C2", "Sheet1!C3"],
                None,
            )
        },
        formula(
            "Sheet1!C2",
            "=A2*B2",
            &["Sheet1!A2", "Sheet1!B2"],
            Some("p1"),
        ),
        formula(
            "Sheet1!C3",
            "=A3*B3",
            &["Sheet1!A3", "Sheet1!B3"],
            Some("p1"),
        ),
    ];
    d.facets.graphs = vec![Graph {
        id: "deps".into(),
        kind: "dependency".into(),
        nodes: vec![
            node("Sheet1!C2", "C2 \"total\" <b>"),
            node("Sheet1 C2", "other"),
            node("end", "End"),
        ],
        edges: vec![
            edge("Sheet1!C2", "Sheet1 C2", "reads"),
            edge("Sheet1 C2", "end", ""),
            edge("end", "missing", "reads"),
        ],
    }];
    let md = render(&d, Completeness::Complete, &opts());
    let expected = "## Logic\n\n\
Spreadsheet formulas.\n\n\
| Cells | Formula | Reads | Role | Value |\n\
| --- | --- | --- | --- | --- |\n\
| Sheet1!D1 | `=SUM(C2:C3)` | Sheet1!C2, Sheet1!C3 | result | 12 kg |\n\
\n\
### Pattern: p1\n\n\
Spreadsheet formulas.\n\n\
| Cells | Formula | Reads | Role | Value |\n\
| --- | --- | --- | --- | --- |\n\
| Sheet1!C2 | `=A2*B2` | Sheet1!A2, Sheet1!B2 | result |  |\n\
| Sheet1!C3 | `=A3*B3` | Sheet1!A3, Sheet1!B3 | result |  |\n\
\n\
### Graph: deps\n\n\
Kind dependency, 3 nodes, 3 edges.\n\
```mermaid\n\
flowchart LR\n    \
n_Sheet1_C2[\"C2 #quot;total#quot; #lt;b#gt;\"]\n    \
n_Sheet1_C2_2[\"other\"]\n    \
n_end[\"End\"]\n    \
n_Sheet1_C2 -->|\"reads\"| n_Sheet1_C2_2\n    \
n_Sheet1_C2_2 --> n_end\n\
```\n\
1 edge points to a node that is not in the graph and is left out.\n";
    assert!(md.contains(expected), "{md}");
}

#[test]
fn mermaid_caps_at_200_nodes() {
    let mut d = doc();
    let nodes: Vec<Node> = (0..205).map(|i| node(&format!("v{i}"), "x")).collect();
    let edges: Vec<Edge> = (0..204)
        .map(|i| edge(&format!("v{i}"), &format!("v{}", i + 1), "next"))
        .collect();
    d.facets.graphs = vec![Graph {
        id: "big".into(),
        kind: "link".into(),
        nodes,
        edges,
    }];
    let md = render(&d, Completeness::Complete, &opts());
    let fence_start = md.find("```mermaid\n").expect("fence");
    let fence = &md[fence_start..];
    let fence_end = fence[11..].find("```").expect("fence end") + 11;
    let inside = &fence[..fence_end];
    assert_eq!(inside.matches("[\"x\"]").count(), 200, "{md}");
    assert_eq!(inside.matches("-->").count(), 199, "{md}");
    assert!(
        md.contains(
            "```\nOnly the first 200 of 205 nodes are shown: 5 nodes and 5 edges are left out."
        ),
        "{md}"
    );
}

#[test]
fn not_read_table() {
    let mut d = doc();
    d.losses = vec![
        loss(
            "page 3",
            "table | odd",
            LossReason::NotImplemented,
            Severity::Warning,
        ),
        loss(
            "A1",
            "header guessed",
            LossReason::Uncertain,
            Severity::Info,
        ),
        loss(
            "document",
            "reader stopped",
            LossReason::Internal,
            Severity::Error,
        ),
    ];
    let md = render(&d, Completeness::Partial, &opts());
    let expected = "## Not read\n\n\
| Where | What | Why | Severity |\n\
| --- | --- | --- | --- |\n\
| page 3 | table \\| odd | not implemented | warning |\n\
| A1 | header guessed | uncertain | info |\n\
| document | reader stopped | internal error | error |\n";
    assert!(md.ends_with(expected), "{md}");
}

fn rich_document() -> Document {
    let mut d = doc();
    d.properties.title = Some("Quarterly report".into());
    d.properties.authors = vec!["Ann".into()];
    d.properties.keywords = vec!["finance".into()];
    d.properties.custom.insert(
        "department".into(),
        Value::Text {
            value: "Sales".into(),
        },
    );
    d.properties.created = Some(Value::DateTime {
        value: "2025-11-30T08:00:00Z".into(),
    });
    d.overview = Some(Overview {
        parts: vec![
            part("Numbers", "3 rows x 2 columns", Some(Role::Input), &[]),
            part("Sheet 1", "1 paragraph", Some(Role::Result), &["Numbers"]),
        ],
        facts: vec![
            ("Formulas".into(), "1".into()),
            ("Not read".into(), "2".into()),
            ("Last changed".into(), "2025-11-30T08:00:00Z".into()),
        ],
    });
    d.body = vec![
        Block::Heading {
            level: 1,
            inlines: vec![txt("Quarterly report")],
            anchor: Some(Anchor("h:quarterly-report".into())),
        },
        para("Intro text with <!-- hidden --> and a pipe |."),
        Block::Heading {
            level: 2,
            inlines: vec![txt("Numbers")],
            anchor: Some(Anchor("h:numbers".into())),
        },
        Block::Table {
            caption: None,
            header_rows: 1,
            role: None,
            rows: vec![
                vec![Cell::new(vec![txt("Item")]), Cell::new(vec![txt("Cost")])],
                vec![Cell::new(vec![txt("Pen")]), Cell::new(vec![txt("2")])],
            ],
            alignments: vec![Align::None, Align::Right],
            anchor: None,
        },
        Block::Callout {
            kind: CalloutKind::Tip,
            title: Some("Hint".into()),
            folded: None,
            blocks: vec![para("Check the totals.")],
            anchor: None,
        },
        Block::Quote {
            blocks: vec![para("Quoted words.")],
            anchor: None,
        },
        Block::Task {
            state: TaskState::Done,
            inlines: vec![txt("Review")],
            anchor: None,
        },
        Block::Math {
            latex: "x = y".into(),
            source_kind: "latex".into(),
            anchor: None,
        },
        Block::Media {
            asset: "img1".into(),
            anchor: None,
        },
        Block::Media {
            asset: "img2".into(),
            anchor: None,
        },
        Block::Section {
            kind: SectionKind::Sheet,
            title: Some("Sheet 1".into()),
            anchor: Some(Anchor("Sheet 1".into())),
            blocks: vec![para("Inside the sheet.")],
        },
        Block::Section {
            kind: SectionKind::Other,
            title: Some("Footnotes".into()),
            anchor: None,
            blocks: vec![Block::Section {
                kind: SectionKind::Other,
                title: None,
                anchor: Some(Anchor("fn:1".into())),
                blocks: vec![para("A note."), para("More of it.")],
            }],
        },
    ];
    d.assets = vec![Asset {
        id: "img1".into(),
        path: Some("assets/img1.png".into()),
        sha256: "ff".into(),
        mime: "image/png".into(),
        caption: Some("Figure 1".into()),
        alt: Some("A chart".into()),
        needs_vision: true,
    }];
    d.facets.formulas = vec![formula("Sheet 1!B2", "=A2*2", &["Sheet 1!A2"], None)];
    d.facets.annotations = vec![Annotation {
        kind: "comment".into(),
        author: Some("Bob".into()),
        date: Some("2025-12-02".into()),
        anchor: Some(Anchor("p2".into())),
        quoted: Some("Intro text".into()),
        body: vec![txt("Please check <!-- this -->.")],
        replies: vec![Annotation {
            kind: "comment".into(),
            author: Some("Ann".into()),
            date: None,
            anchor: None,
            quoted: None,
            body: vec![txt("Done.")],
            replies: Vec::new(),
            resolved: None,
        }],
        resolved: Some(true),
    }];
    d.facets.timeline = vec![TimelineEvent {
        time: Some("2025-12-01T09:00:00Z".into()),
        actor: Some("Ann".into()),
        kind: "insert".into(),
        anchor: Some(Anchor("p2".into())),
        text: Some("and a pipe".into()),
    }];
    d.facets.embedded = vec![Embedded {
        kind: "macro".into(),
        name: "AutoOpen".into(),
        detail: Some("VBA".into()),
        target: Some("https://example.org/x".into()),
    }];
    d.losses = vec![loss(
        "page 9",
        "an embedded font",
        LossReason::Unsupported,
        Severity::Info,
    )];
    d
}

#[test]
fn no_html_comments() {
    let md = render(&rich_document(), Completeness::Partial, &opts());
    assert!(!md.contains("<!--"), "{md}");
}

#[test]
fn sections_filter() {
    let mut o = opts();
    o.sections = Some(vec![OutputSection::NotRead]);
    let md = render(&rich_document(), Completeness::Partial, &o);
    assert!(md.contains("## Not read"), "{md}");
    assert!(!md.contains("Intro text"), "{md}");
    assert!(!md.contains("## Logic"), "{md}");
    assert!(md.contains("\n# Quarterly report\n"), "{md}");
}

fn part(name: &str, size: &str, role: Option<Role>, reads: &[&str]) -> OverviewPart {
    OverviewPart {
        id: name.into(),
        name: name.into(),
        kind: "sheet".into(),
        size: size.into(),
        role,
        reads: reads.iter().map(|r| (*r).to_string()).collect(),
    }
}

#[test]
fn overview_is_none_without_data() {
    let mut d = doc();
    assert_eq!(render_overview(&d), None);
    d.overview = Some(Overview::default());
    assert_eq!(render_overview(&d), None);
}

#[test]
fn overview_table_facts_and_graph() {
    let mut d = doc();
    d.overview = Some(Overview {
        parts: vec![
            part("Input", "10 rows x 3 columns", Some(Role::Input), &[]),
            part("Calc", "4 rows x 2 columns", Some(Role::Result), &["Input"]),
        ],
        facts: vec![
            ("Formulas".into(), "2".into()),
            ("Last changed".into(), "2026-02-03".into()),
        ],
    });
    let expected = "## Overview\n\n\
| Part | Kind | Size | Role | Reads from |\n\
| --- | --- | --- | --- | --- |\n\
| Input | sheet | 10 rows x 3 columns | input |  |\n\
| Calc | sheet | 4 rows x 2 columns | result | Input |\n\n\
- Formulas: 2\n\
- Last changed: 2026-02-03\n\n\
```mermaid\n\
flowchart LR\n    p0[\"Input\"]\n    p1[\"Calc\"]\n    p0 --> p1\n```";
    assert_eq!(render_overview(&d).unwrap(), expected);
    let md = render(&d, Completeness::Complete, &opts());
    assert!(md.contains("\n# report.md\n\n## Overview\n"), "{md}");
}

#[test]
fn overview_single_plain_part_has_a_table() {
    let mut d = doc();
    d.overview = Some(Overview {
        parts: vec![part("Only", "3 paragraphs", None, &[])],
        facts: vec![("Not read".into(), "1".into())],
    });
    assert_eq!(
        render_overview(&d).unwrap(),
        "## Overview\n\n\
| Part | Kind | Size | Role | Reads from |\n\
| --- | --- | --- | --- | --- |\n\
| Only | sheet | 3 paragraphs |  |  |\n\n\
- Not read: 1"
    );
}

#[test]
fn overview_text_is_escaped() {
    let mut d = doc();
    d.overview = Some(Overview {
        parts: vec![
            part("a | b <!-- x -->\n# h", "1", Some(Role::Input), &[]),
            part(
                "Q\"]\nend",
                "1",
                Some(Role::Result),
                &["a | b <!-- x -->\n# h"],
            ),
        ],
        facts: vec![("L".into(), "<script>\n# no".into())],
    });
    let out = render_overview(&d).unwrap();
    assert!(!out.contains("<!--"), "{out}");
    assert!(!out.contains("<script>"), "{out}");
    assert!(!out.contains("\n# "), "{out}");
    // The table has exactly header, separator and two rows.
    let table_lines = out.lines().filter(|l| l.starts_with('|')).count();
    assert_eq!(table_lines, 4, "{out}");
    let graph = out.split("```mermaid").nth(1).unwrap();
    assert!(!graph.contains("\"]\nend"), "{out}");
}

#[test]
fn snapshot_rich_document() {
    let md = render(&rich_document(), Completeness::Partial, &opts());
    insta::assert_snapshot!(md);
}

#[test]
fn paginate_splits_before_h2_not_in_fence() {
    let filler = "word ".repeat(40); // 200 chars, 50 tokens
    let md = format!(
        "---\ntype: document\n---\n\n# T\n\n{filler}\n\n## A\n\n{filler}\n\n```text\n## not a heading\n{filler}\n```\n\n## B\n\n{filler}\n"
    );
    let pages = paginate(&md, 150);
    assert_eq!(pages.concat(), md);
    assert!(pages.len() >= 3, "{pages:#?}");
    for page in &pages[1..] {
        assert!(
            page.starts_with("## A\n") || page.starts_with("## B\n"),
            "{pages:#?}"
        );
    }
    // The fence stays whole on one page.
    assert!(
        pages
            .iter()
            .any(|p| p.contains("```text\n## not a heading\n"))
    );
}

#[test]
fn paginate_keeps_frontmatter_on_page_1() {
    let long = "x".repeat(400);
    let md =
        format!("---\ntype: document\ntitle: \"{long}\"\n---\n\n# T\n\nshort\n\n## A\n\nmore\n");
    let pages = paginate(&md, 10);
    assert_eq!(pages.concat(), md);
    assert!(pages[0].starts_with("---\ntype: document\n"));
    // The title keeps its first block with it.
    assert!(
        pages[0].ends_with("\n---\n\n# T\n\nshort\n\n"),
        "{pages:#?}"
    );
    assert_eq!(pages.len(), 2, "{pages:#?}");
    assert!(pages[1].starts_with("## A\n"), "{pages:#?}");
}

#[test]
fn paginate_splits_large_section_at_h3_then_blank_line() {
    let filler = "word ".repeat(40);
    let md = format!(
        "# T\n\n## A\n\n{filler}\n\n### A1\n\n{filler}\n\n{filler}\n\n    indented\n\n---\n\nend\n"
    );
    let pages = paginate(&md, 60);
    assert_eq!(pages.concat(), md);
    assert!(
        pages.iter().any(|p| p.starts_with("### A1\n")),
        "{pages:#?}"
    );
    for page in &pages {
        assert!(!page.starts_with("    "), "{pages:#?}");
        assert!(!page.starts_with("---"), "{pages:#?}");
    }
    assert!(pages.len() >= 3, "{pages:#?}");
}

#[test]
fn paginate_small_or_zero_is_one_page() {
    let md = "# T\n\n## A\n\nx\n";
    assert_eq!(paginate(md, 20_000), vec![md.to_string()]);
    assert_eq!(paginate(md, 0), vec![md.to_string()]);
}

#[test]
fn headings_shift_below_the_title() {
    let heading = |level, text: &str| Block::Heading {
        level,
        inlines: vec![txt(text)],
        anchor: None,
    };
    let mut d = doc();
    d.properties.title = Some("Report".into());
    d.body = vec![heading(1, "Other"), heading(2, "C#"), heading(6, "Deep")];
    let md = render(&d, Completeness::Complete, &opts());
    assert!(
        md.contains("\n# Report\n\n## Other\n\n### C\\#\n\n###### Deep\n"),
        "{md}"
    );
}

#[test]
fn deep_nesting_is_cut_visibly() {
    let mut block = para("core");
    for _ in 0..1000 {
        block = Block::Quote {
            blocks: vec![block],
            anchor: None,
        };
    }
    let md = body_of(vec![block]);
    assert!(md.contains("Nesting too deep"), "{}", &md[..500]);
    assert!(!md.contains("core"));
}

#[test]
fn dollar_signs_never_open_math() {
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![
            txt("costs $5 and $$x$$ and $"),
            Inline::Code { text: "y".into() },
            txt("$"),
        ],
        role: None,
        anchor: None,
    }]);
    assert!(
        md.contains("costs \\$5 and \\$\\$x\\$\\$ and \\$`y`\\$\n"),
        "{md}"
    );
}

#[test]
fn hostile_spans_stay_bounded() {
    let mut huge = Cell::new(vec![txt("h")]);
    huge.col_span = u32::MAX;
    huge.row_span = u32::MAX;
    let row = || vec![Cell::new(vec![txt("a")]), Cell::new(vec![txt("b")])];
    let md = body_of(vec![Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows: vec![row(), vec![huge], row()],
        alignments: Vec::new(),
        anchor: None,
    }]);
    assert!(md.contains("Merged cells: A2:B3"), "{}", &md[..300]);
    assert!(md.len() < 1_000_000, "{}", md.len());
}

#[test]
fn inline_math_fallbacks_stay_safe_without_a_math_parser() {
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![
            Inline::Math {
                latex: "a`<!-- x -->".into(),
            },
            txt(" "),
            Inline::Math {
                latex: "a`$b".into(),
            },
        ],
        role: None,
        anchor: None,
    }]);
    for event in Parser::new_ext(after_frontmatter(&md), Options::ENABLE_TABLES) {
        assert!(
            !matches!(event, Event::Html(_) | Event::InlineHtml(_)),
            "{event:?} in {md}"
        );
    }
    assert!(md.contains("``a`<!-- x -->`` ``a`$b``"), "{md}");
}

#[test]
fn paginate_keeps_caption_table_and_merge_note_together() {
    let filler = "word ".repeat(40);
    let md = format!(
        "# T\n\n## A\n\n{filler}\n\n**Costs**\n\n| a |\n| --- |\n| x |\n\nMerged cells: A1:A2\n\n{filler}\n"
    );
    let pages = paginate(&md, 30);
    assert_eq!(pages.concat(), md);
    for page in &pages {
        assert!(!page.starts_with('|'), "{pages:#?}");
        assert!(!page.starts_with("Merged"), "{pages:#?}");
    }
}

#[test]
fn logic_names_dialects_and_shows_notes() {
    let mut d = doc();
    let derived = |anchor: &str, note: Option<&str>| Formula {
        dialect: "derived".into(),
        note: note.map(Into::into),
        ..formula(anchor, "Total = A + B", &["data!A:A"], None)
    };
    d.facets.formulas = vec![
        derived("data!C:C", Some("holds 20 of 20 rows")),
        derived("data!D:D", None),
        Formula {
            dialect: "openformula".into(),
            cached: Some(Value::Percent { value: 0.07 }),
            ..formula("B1", "of:=A1", &[], Some("g"))
        },
        Formula {
            dialect: "mathcad".into(),
            cached: Some(Value::Bool { value: true }),
            ..formula("B2", "x:=1", &[], Some("g"))
        },
    ];
    let md = render(&d, Completeness::Complete, &opts());
    let expected = "## Logic\n\n\
Found in the data, not written as formulas in the file.\n\n\
| Cells | Formula | Reads | Role | Value | Note |\n\
| --- | --- | --- | --- | --- | --- |\n\
| data!C:C | `Total = A + B` | data!A:A | result |  | holds 20 of 20 rows |\n\
| data!D:D | `Total = A + B` | data!A:A | result |  |  |\n\
\n\
### Pattern: g\n\n\
OpenDocument formulas, mathcad.\n\n\
| Cells | Formula | Reads | Role | Value |\n\
| --- | --- | --- | --- | --- |\n\
| B1 | `of:=A1` |  | result | 7 % |\n\
| B2 | `x:=1` |  | result | true |\n";
    assert!(md.contains(expected), "{md}");
}

#[test]
fn custom_properties_get_a_section_before_the_content() {
    let mut d = doc();
    d.properties.custom.insert(
        "project | x".into(),
        Value::Text {
            value: "Apollo".into(),
        },
    );
    d.properties.custom.insert(
        "sizes".into(),
        Value::List {
            value: vec![
                Value::Number {
                    value: 1.5,
                    unit: None,
                },
                Value::Date {
                    value: "2025-01-02".into(),
                },
            ],
        },
    );
    d.body = vec![para("Body text.")];
    let md = render(&d, Completeness::Complete, &opts());
    let expected = "\n# report.md\n\n## Properties\n\n\
| Key | Value |\n\
| --- | --- |\n\
| project \\| x | Apollo |\n\
| sizes | 1.5, 2025-01-02 |\n\
\n\
Body text.\n";
    assert!(md.contains(expected), "{md}");
    let mut o = opts();
    o.sections = Some(vec![OutputSection::Content]);
    let only = render(&d, Completeness::Complete, &o);
    assert!(!only.contains("## Properties"), "{only}");
}

fn styled(text: &str, emphasis: bool, strong: bool) -> Inline {
    Inline::Text {
        text: text.into(),
        emphasis,
        strong,
        strike: false,
    }
}

#[test]
fn emphasis_flanks_in_common_cases() {
    let p = |inlines: Vec<Inline>| {
        let md = body_of(vec![Block::Paragraph {
            inlines,
            role: None,
            anchor: None,
        }]);
        after_frontmatter(&md)
            .trim_start_matches("\n# report.md\n\n")
            .to_string()
    };
    assert_eq!(p(vec![styled("word", true, false), txt(".")]), "*word*.\n");
    assert_eq!(
        p(vec![txt("("), styled("word", true, false), txt(")")]),
        "(*word*)\n"
    );
    assert_eq!(
        p(vec![txt("a"), styled("b", true, false), txt("c")]),
        "a*b*c\n"
    );
    // Not left-flanking: the markers move out to the whole word run.
    assert_eq!(
        p(vec![
            txt("see foo"),
            styled("(bar)", false, true),
            txt(" now")
        ]),
        "see **foo(bar)** now\n"
    );
    assert_eq!(
        p(vec![txt("x "), styled("(bar)", false, true), txt("baz y")]),
        "x **(bar)baz** y\n"
    );
    // Punctuation on both sides flanks.
    assert_eq!(
        p(vec![
            Inline::Code { text: "c".into() },
            styled("(bar)", true, false),
            Inline::Code { text: "d".into() },
        ]),
        "`c`*(bar)*`d`\n"
    );
    // Every case parses as emphasis where markers were written.
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![txt("see foo"), styled("(bar)", false, true), txt(" now")],
        role: None,
        anchor: None,
    }]);
    let strong = Parser::new_ext(after_frontmatter(&md), Options::empty())
        .filter(|e| matches!(e, Event::Start(Tag::Strong)))
        .count();
    assert_eq!(strong, 1, "{md}");
}

#[test]
fn loose_list_items_keep_a_blank_line_before_nested_lists() {
    let md = body_of(vec![Block::List {
        ordered: false,
        items: vec![vec![
            para("a"),
            para("b"),
            Block::List {
                ordered: false,
                items: vec![vec![para("c")]],
                anchor: None,
            },
        ]],
        anchor: None,
    }]);
    assert!(md.contains("\n- a\n\n  b\n\n  - c\n"), "{md}");
}

fn cells_of_rows(md: &str) -> Vec<usize> {
    let mut rows = Vec::new();
    let mut n = 0;
    for e in Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES) {
        match e {
            Event::Start(Tag::TableRow) | Event::Start(Tag::TableHead) => n = 0,
            Event::Start(Tag::TableCell) => n += 1,
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => rows.push(n),
            _ => {}
        }
    }
    rows
}

#[test]
fn table_cells_keep_links_images_and_footnotes_intact() {
    let link = Inline::Link {
        target: "http://a/b|c".into(),
        title: Some("x|y".into()),
        internal: false,
        wiki: false,
        inlines: vec![txt("L")],
    };
    let image = Inline::Image {
        target: "i|j.png".into(),
        alt: "p|q".into(),
        title: Some("t|u".into()),
    };
    let md = body_of(vec![Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows: vec![
            vec![
                Cell::new(vec![txt("a")]),
                Cell::new(vec![txt("b")]),
                Cell::new(vec![txt("c")]),
            ],
            vec![
                Cell::new(vec![link]),
                Cell::new(vec![Inline::FootnoteRef { id: "a|b".into() }]),
                Cell::new(vec![image]),
            ],
        ],
        alignments: Vec::new(),
        anchor: None,
    }]);
    let body = after_frontmatter(&md);
    assert_eq!(cells_of_rows(body), vec![3, 3], "{body}");
    let mut link_ok = false;
    let mut image_ok = false;
    for e in Parser::new_ext(body, Options::ENABLE_TABLES) {
        match e {
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => link_ok = &*dest_url == "http://a/b%7Cc" && &*title == "x|y",
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => image_ok = &*dest_url == "i%7Cj.png" && &*title == "t|u",
            _ => {}
        }
    }
    assert!(link_ok && image_ok, "{body}");
}

#[test]
fn wide_spans_stay_small() {
    let mut rows: Vec<Vec<Cell>> = (0..400)
        .map(|r| vec![Cell::new(vec![txt(&format!("r{r}"))])])
        .collect();
    for _ in 0..5 {
        let mut wide = Cell::new(vec![txt("w")]);
        wide.col_span = 16_384;
        rows[0].push(wide);
    }
    let md = body_of(vec![Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
    }]);
    assert!(md.len() < 100_000, "{}", md.len());
    assert!(md.contains("| r399 |"), "{}", &md[md.len() - 300..]);
}

#[test]
fn table_grid_is_capped_in_rows_and_columns() {
    let row = |n: usize| {
        (0..n)
            .map(|i| Cell::new(vec![txt(&i.to_string())]))
            .collect()
    };
    let md = body_of(vec![Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows: (0..300).map(|_| row(1000)).collect(),
        alignments: Vec::new(),
        anchor: None,
    }]);
    assert!(
        md.contains("100 more rows not shown (see the model JSON)."),
        "tail"
    );
    assert_eq!(cells_of_rows(after_frontmatter(&md)).len(), 200);
    let md = body_of(vec![Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows: vec![row(1200), row(3)],
        alignments: Vec::new(),
        anchor: None,
    }]);
    assert!(
        md.contains("200 more columns not shown (see the model JSON)."),
        "cols"
    );
    assert_eq!(cells_of_rows(after_frontmatter(&md)), vec![1000, 1000]);
}

#[test]
fn deeply_nested_link_text_is_kept() {
    let mut inline = txt("leaf");
    for i in 0..70 {
        inline = Inline::Link {
            target: format!("u{i}"),
            title: None,
            internal: false,
            wiki: false,
            inlines: vec![txt("t"), inline],
        };
    }
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![inline],
        role: None,
        anchor: None,
    }]);
    assert!(
        md.contains(&format!("[{}leaf](u69)", "t".repeat(70))),
        "{md}"
    );
}

#[test]
fn dialect_line_is_escaped() {
    let mut d = doc();
    d.facets.formulas = vec![Formula {
        dialect: "# x".into(),
        ..formula("A1", "=1", &[], None)
    }];
    let md = render(&d, Completeness::Complete, &opts());
    assert!(md.contains("\n\\# x.\n"), "{md}");
}

#[test]
fn impossible_dates_are_quoted() {
    let date = |v: &str| {
        let mut d = doc();
        d.properties.modified = Some(Value::Date { value: v.into() });
        render(&d, Completeness::Complete, &opts())
    };
    assert!(date("2024-02-29").contains("\nsource_date: 2024-02-29\ntemporal_confidence: high\n"));
    // An impossible document date falls back to the file date.
    let file_date = "\nsource_date: 2026-01-05\ntemporal_confidence: medium\n";
    for bad in ["2026-02-30", "2025-13-01", "2026-04-31", "2100-02-29"] {
        assert!(date(bad).contains(file_date), "{bad}");
    }
    // ... or to the created date when that one is real.
    let mut d = doc();
    d.properties.modified = Some(Value::Date {
        value: "2026-02-30".into(),
    });
    d.properties.created = Some(Value::Date {
        value: "2025-03-04".into(),
    });
    let md = render(&d, Completeness::Complete, &opts());
    assert!(
        md.contains("\nsource_date: 2025-03-04\ntemporal_confidence: high\n"),
        "{md}"
    );
    // An impossible file date is quoted.
    let mut d = doc();
    d.source.modified = Some("2026-02-30T00:00:00Z".into());
    let md = render(&d, Completeness::Complete, &opts());
    assert!(md.contains("\nsource_date: \"2026-02-30\"\n"), "{md}");
}

#[test]
fn only_safe_schemes_become_links() {
    let link = |target: &str| Inline::Link {
        target: target.into(),
        title: None,
        internal: false,
        wiki: false,
        inlines: vec![txt("go")],
    };
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![
            link("javascript:alert(1)"),
            txt(" "),
            link("java\tscript:x"),
            txt(" "),
            Inline::Image {
                target: "data:image/png;base64,AA".into(),
                alt: "pic".into(),
                title: None,
            },
            txt(" "),
            link("HTTPS://ok.example/a[[b]]"),
            txt(" "),
            link("mailto:a@b.c"),
            txt(" "),
            link("docs/x.md#y"),
            txt(" "),
            link("C:/dir/file"),
        ],
        role: None,
        anchor: None,
    }]);
    assert!(
        md.contains(
            "go `javascript:alert(1)` go `java\tscript:x` pic `data:image/png;base64,AA` \
[go](HTTPS://ok.example/a%5B%5Bb%5D%5D) [go](mailto:a@b.c) [go](docs/x.md#y) \
[go](C:/dir/file)\n"
        ),
        "{md}"
    );
}

#[test]
fn single_dollar_pairs_do_not_form_math() {
    let md = body_of(vec![para("costs $x$ y and $5")]);
    assert!(md.contains("costs \\$x\\$ y and \\$5\n"), "{md}");
    let body = after_frontmatter(&md);
    for e in Parser::new_ext(body, Options::ENABLE_MATH) {
        assert!(
            !matches!(e, Event::InlineMath(_) | Event::DisplayMath(_)),
            "{body}"
        );
    }
}

#[test]
fn mermaid_caps_edges() {
    let mut d = doc();
    let nodes: Vec<Node> = (0..50).map(|i| node(&format!("v{i}"), "x")).collect();
    let edges: Vec<Edge> = (0..1500)
        .map(|i| edge(&format!("v{}", i % 50), &format!("v{}", (i / 50) % 50), "e"))
        .collect();
    d.facets.graphs = vec![Graph {
        id: "g".into(),
        kind: "link".into(),
        nodes,
        edges,
    }];
    let md = render(&d, Completeness::Complete, &opts());
    assert_eq!(md.matches("-->").count(), 1000);
    assert!(
        md.contains("Only the first 1000 of 1500 edges are shown: 500 edges are left out."),
        "{}",
        &md[md.len() - 300..]
    );
}

#[test]
fn character_references_cannot_sneak_a_scheme_in() {
    let link = |target: &str| Inline::Link {
        target: target.into(),
        title: Some("see [[Target]]".into()),
        internal: false,
        wiki: false,
        inlines: vec![txt("go")],
    };
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![
            link("javascript&#58;alert(1)"),
            txt(" "),
            link("javascript&colon;alert(1)"),
            txt(" "),
            link("jav&#x61;script:alert(1)"),
            txt(" "),
            link("https://ok.example/?a=1&b=2"),
        ],
        role: None,
        anchor: None,
    }]);
    assert!(
        md.contains(
            "go `javascript&#58;alert(1)` go `javascript&colon;alert(1)` \
go `jav&#x61;script:alert(1)` \
[go](https://ok.example/?a=1\\&b=2 \"see \\[\\[Target\\]\\]\")\n"
        ),
        "{md}"
    );
    let body = after_frontmatter(&md);
    let mut links = Vec::new();
    for e in Parser::new_ext(body, Options::empty()) {
        if let Event::Start(Tag::Link { dest_url, .. }) = e {
            links.push(dest_url.to_string());
        }
    }
    assert_eq!(links, ["https://ok.example/?a=1&b=2"], "{body}");
}

#[test]
fn dollar_pairs_across_runs_are_escaped() {
    let md = body_of(vec![
        Block::Paragraph {
            inlines: vec![
                txt("a $x "),
                Inline::Code { text: "c$".into() },
                txt(" y$ b"),
            ],
            role: None,
            anchor: None,
        },
        Block::Paragraph {
            inlines: vec![txt("p $q "), styled("strong", false, true), txt(" r$ s")],
            role: None,
            anchor: None,
        },
        para("only $5 here"),
        Block::Paragraph {
            inlines: vec![txt("e $f"), Inline::Math { latex: "x".into() }, txt("g")],
            role: None,
            anchor: None,
        },
    ]);
    assert!(md.contains("e \\$f$`x`$g\n"), "{md}");
    assert!(md.contains("a \\$x `c$` y\\$ b\n"), "{md}");
    assert!(md.contains("p \\$q **strong** r\\$ s\n"), "{md}");
    assert!(md.contains("only $5 here\n"), "{md}");
    // The only math is the real formula; no text `$` pairs into a fake one.
    for e in Parser::new_ext(after_frontmatter(&md), Options::ENABLE_MATH) {
        match e {
            Event::InlineMath(m) => assert!(m.contains('x') && !m.contains('f'), "{m} in {md}"),
            Event::DisplayMath(m) => panic!("{m} in {md}"),
            _ => {}
        }
    }
}

#[test]
fn inline_code_keeps_tabs() {
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![Inline::Code {
            text: "a\tb".into(),
        }],
        role: None,
        anchor: None,
    }]);
    assert!(md.contains("\n`a\tb`\n"), "{md}");
}

#[test]
fn merge_note_names_only_shown_rows() {
    let row = |n: usize| -> Vec<Cell> {
        (0..n)
            .map(|i| Cell::new(vec![txt(&i.to_string())]))
            .collect()
    };
    let mut rows: Vec<Vec<Cell>> = (0..300).map(|_| row(1000)).collect();
    rows[0][0].row_span = 300;
    let md = body_of(vec![Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
    }]);
    assert!(
        md.contains("Merged cells: A1:A200\n"),
        "{}",
        &md[md.len() - 200..]
    );
}

#[test]
fn scheme_check_sees_references_and_ampersands() {
    let link = |target: &str| Inline::Link {
        target: target.into(),
        title: None,
        internal: false,
        wiki: false,
        inlines: vec![txt("go")],
    };
    let long = format!("javascript&#{}58;x", "0".repeat(40));
    let md = body_of(vec![Block::Paragraph {
        inlines: vec![
            link("javascript&#58alert"),
            txt(" "),
            link(&long),
            txt(" "),
            link("javascript&sol;:x"),
            txt(" "),
            link("a&b;c.md"),
        ],
        role: None,
        anchor: None,
    }]);
    assert!(
        md.contains(&format!(
            "go `javascript&#58alert` go `{long}` go `javascript&sol;:x` [go](a\\&b;c.md)\n"
        )),
        "{md}"
    );
}

#[test]
fn overview_duplicate_names_get_no_self_arrow() {
    let mut d = doc();
    let mut first = part("Same", "1", Some(Role::Input), &[]);
    first.id = "a".into();
    let mut second = part("Same", "1", Some(Role::Result), &[]);
    second.id = "b".into();
    second.reads = vec!["a".into()];
    d.overview = Some(Overview {
        parts: vec![first, second],
        facts: vec![],
    });
    let out = render_overview(&d).unwrap();
    assert!(out.contains("p0 --> p1"), "{out}");
    assert!(!out.contains("p1 --> p1"), "{out}");
    assert!(
        out.contains("| Same | sheet | 1 | result | Same |"),
        "{out}"
    );
}

#[test]
fn overview_empty_document_has_no_heading() {
    let d = doc();
    let md = render(&d, Completeness::Complete, &opts());
    assert!(!md.contains("## Overview"), "{md}");
}

#[test]
fn output_section_names_round_trip() {
    use rezzer_render::OutputSection;
    for section in OutputSection::ALL {
        assert_eq!(section.name().parse::<OutputSection>(), Ok(section));
    }
    assert_eq!(
        " Not-Read ".parse::<OutputSection>(),
        Ok(OutputSection::NotRead)
    );
    let err = "body".parse::<OutputSection>().unwrap_err();
    assert!(
        err.contains("\"body\"") && err.contains("not_read"),
        "{err}"
    );
}

#[test]
fn format_name_is_snake_case() {
    assert_eq!(
        rezzer_render::format_name(rezzer_model::FormatId::ThreeMf),
        "three_mf"
    );
}

fn wiki_link(target: &str, text: &str) -> Inline {
    Inline::Link {
        target: target.into(),
        title: None,
        internal: true,
        wiki: true,
        inlines: vec![Inline::Text {
            text: text.into(),
            emphasis: false,
            strong: false,
            strike: false,
        }],
    }
}

fn render_inline(inline: Inline) -> String {
    let doc = {
        let mut d = Document::new(source());
        d.body.push(Block::Paragraph {
            inlines: vec![inline],
            role: None,
            anchor: None,
        });
        d
    };
    let md = render(&doc, Completeness::Complete, &opts());
    let (_, body) = md.split_once("\n---\n\n").expect("frontmatter");
    body.trim().replace("# ", "").to_string()
}

#[test]
fn wikilinks_are_written_as_wikilinks() {
    assert!(render_inline(wiki_link("graph-view", "graph-view")).ends_with("[[graph-view]]"));
    assert!(render_inline(wiki_link("a#sec", "label")).ends_with("[[a#sec|label]]"));
}

#[test]
fn unsafe_wikilinks_fall_back_to_a_normal_link() {
    for (target, text) in [
        ("a]]b", "a]]b"),
        ("a|b", "a|b"),
        ("a\nb", "x"),
        ("ok", "x|y"),
        ("ok", "x]]y"),
    ] {
        let out = render_inline(wiki_link(target, text));
        assert!(!out.contains("[["), "{out}");
    }
}

/// Parses the rendered Markdown back: no live javascript link and no raw HTML.
fn assert_inert(md: &str) {
    for event in Parser::new_ext(md, Options::all()) {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                let url = dest_url.to_ascii_lowercase();
                assert!(
                    !url.trim_start().starts_with("javascript:"),
                    "live link {dest_url:?} in {md:?}"
                );
            }
            Event::Html(h) | Event::InlineHtml(h) => panic!("raw html {h:?} in {md:?}"),
            _ => {}
        }
    }
}

#[test]
fn hostile_wikilinks_stay_inert() {
    let cases = [
        ("<javascript:alert(1)>", "<javascript:alert(1)>"),
        ("<script>alert(1)</script>", "x"),
        ("x", "<img src=x onerror=alert(1)>"),
        ("x", "a <b>bold</b> c"),
        ("a] [evil](javascript:x) [b", "a] [evil](javascript:x) [b"),
        ("javascript:alert(1)", "javascript:alert(1)"),
        ("a\\", "b"),
        ("x", "`code` *em* $m$ &amp; ![i](u)"),
    ];
    for (target, text) in cases {
        let out = render_inline(wiki_link(target, text));
        assert!(!out.contains("[["), "{out}");
        assert_inert(&out);
    }
}

#[test]
fn wikilinks_with_formatting_in_the_text_fall_back() {
    let mut link = wiki_link("x", "label");
    if let Inline::Link { inlines, .. } = &mut link {
        *inlines = vec![Inline::Text {
            text: "label".into(),
            emphasis: false,
            strong: true,
            strike: false,
        }];
    }
    assert!(!render_inline(link).contains("[["));
    let mut link = wiki_link("x", "label");
    if let Inline::Link { inlines, .. } = &mut link {
        *inlines = vec![Inline::Code {
            text: "label".into(),
        }];
    }
    assert!(!render_inline(link).contains("[["));
}

#[test]
fn plain_wikilinks_in_any_script_are_written() {
    assert!(
        render_inline(wiki_link("Straße/日本語 note-1x.md#Sec, it's", "Straße"))
            .ends_with("[[Straße/日本語 note-1x.md#Sec, it's|Straße]]")
    );
    assert!(!render_inline(wiki_link("http://x", "http://x")).contains("[["));
}

fn plain_link(target: &str, text: &str) -> Inline {
    Inline::Link {
        target: target.into(),
        title: None,
        internal: false,
        wiki: false,
        inlines: vec![txt(text)],
    }
}

#[test]
fn headings_keep_their_links() {
    let mut d = doc();
    d.properties.title = Some("H a".into());
    d.body = vec![
        Block::Heading {
            level: 1,
            inlines: vec![txt("H "), plain_link("https://b.example/x", "a")],
            anchor: Some(Anchor("h:h-a".into())),
        },
        Block::Heading {
            level: 2,
            inlines: vec![txt("Sub "), plain_link("https://b.example/y", "b")],
            anchor: None,
        },
    ];
    let md = render(&d, Completeness::Complete, &opts());
    assert!(md.contains("\n# H [a](https://b.example/x)\n"), "{md}");
    assert!(md.contains("\n## Sub [b](https://b.example/y)\n"), "{md}");
}

#[test]
fn underscores_keep_a_wikilink_from_being_written() {
    for name in ["_b_", "__init__", "a_b"] {
        let out = render_inline(wiki_link(name, name));
        assert!(!out.contains("[["), "{out}");
    }
}

#[test]
fn wikilinks_with_a_different_text_are_not_written_in_table_cells() {
    let mut d = doc();
    d.body = vec![Block::Table {
        caption: None,
        header_rows: 0,
        role: None,
        rows: vec![vec![
            Cell::new(vec![wiki_link("x", "y")]),
            Cell::new(vec![wiki_link("same", "same")]),
        ]],
        alignments: Vec::new(),
        anchor: None,
    }];
    let md = render(&d, Completeness::Complete, &opts());
    assert!(!md.contains("[[x|y]]"), "{md}");
    let row = md.lines().find(|l| l.contains("same")).expect("row");
    assert_eq!(row.matches(" | ").count(), 1, "{row}");
    assert!(row.contains("[[same]]"), "{row}");
}

#[test]
fn task_items_inside_lists_get_one_marker() {
    let task = |state, text: &str| Block::Task {
        state,
        inlines: vec![txt(text)],
        anchor: None,
    };
    let list = |ordered, items: Vec<Vec<Block>>| Block::List {
        ordered,
        items,
        anchor: None,
    };
    let md = body_of(vec![
        list(
            false,
            vec![
                vec![task(TaskState::Open, "a")],
                vec![task(TaskState::Done, "b")],
            ],
        ),
        list(false, vec![vec![task(TaskState::Open, "c")]]),
        para("between"),
        list(true, vec![vec![task(TaskState::Done, "d"), para("more")]]),
    ]);
    assert!(md.contains("\n- [ ] a\n- [x] b\n\n* [ ] c\n"), "{md}");
    assert!(md.contains("\n1. [x] d\n\n   more\n"), "{md}");
}

// ---------------------------------------------------------------------------------------
// Spreadsheet logic

fn workbook() -> Document {
    let mut d = doc();
    d.source.format = FormatId::Xlsx;
    d.source.name = "calc.xlsx".into();
    d.body = vec![sheet("Inputs", "Inputs"), sheet("Calc", "Calc")];
    d
}

fn sheet(title: &str, anchor: &str) -> Block {
    Block::Section {
        kind: SectionKind::Sheet,
        title: Some(title.into()),
        anchor: Some(Anchor(anchor.into())),
        blocks: vec![para("values")],
    }
}

fn num(value: f64) -> Option<Value> {
    Some(Value::Number { value, unit: None })
}

fn cell_formula(anchor: &str, expr: &str, refs: &[&str], role: Role, cached: f64) -> Formula {
    Formula {
        role: Some(role),
        cached: num(cached),
        kind: Some(FormulaKind::Cell),
        ..formula(anchor, expr, refs, None)
    }
}

fn copied(anchor: &str, expr: &str, refs: &[&str], pattern: &str, range: &str) -> Formula {
    Formula {
        role: Some(Role::Intermediate),
        cached: num(2.0),
        kind: Some(FormulaKind::Shared),
        pattern: Some(pattern.into()),
        range: Some(Anchor(range.into())),
        ..formula(anchor, expr, refs, None)
    }
}

fn parameter(anchor: &str, label: Option<&str>, value: f64, read_by: u32) -> Parameter {
    Parameter {
        anchor: Anchor(anchor.into()),
        name: None,
        label: label.map(Into::into),
        value: num(value),
        unit: None,
        unit_source: None,
        read_by,
    }
}

fn logic_of(d: &Document, math: bool) -> String {
    let md = render(
        d,
        Completeness::Complete,
        &RenderOptions {
            math,
            sections: Some(vec![OutputSection::Logic]),
            ..opts()
        },
    );
    let start = md.find("## Logic").expect("a Logic section");
    md[start..].to_string()
}

#[test]
fn logic_per_sheet_with_patterns_and_counts() {
    let mut d = workbook();
    d.facets.formulas = vec![
        cell_formula("Calc!C1", "=SUM(C2:C4)", &["Calc!C2:C4"], Role::Result, 6.0),
        copied(
            "Calc!C2",
            "=Inputs!B1*B2",
            &["Inputs!B1", "Calc!B2"],
            "p1",
            "Calc!C2:C4",
        ),
        copied(
            "Calc!C3",
            "=Inputs!B1*B3",
            &["Inputs!B1", "Calc!B3"],
            "p1",
            "Calc!C2:C4",
        ),
        copied(
            "Calc!C4",
            "=Inputs!B1*B4",
            &["Inputs!B1", "Calc!B4"],
            "p1",
            "Calc!C2:C4",
        ),
        Formula {
            role: None,
            kind: Some(FormulaKind::Name),
            ..formula("name:rate", "Inputs!$B$1", &["Inputs!B1"], None)
        },
        Formula {
            role: None,
            kind: Some(FormulaKind::Lambda),
            note: Some("parameters: x".into()),
            ..formula("name:Calc!Twice", "LAMBDA(x, 2*x)", &[], None)
        },
    ];
    d.facets.parameters = vec![parameter("Inputs!B1", Some("Rate"), 2.0, 3)];
    let logic = logic_of(&d, true);
    let inputs = logic.find("\n### Inputs\n").expect("Inputs heading");
    let calc = logic.find("\n### Calc\n").expect("Calc heading");
    let names = logic.find("\n### Names\n").expect("Names heading");
    assert!(inputs < calc && calc < names, "{logic}");
    let expected = "### Calc\n\n\
#### Formulas\n\n\
A row with a count above 1 stands for one formula copied over its cells. It shows the first cell.\n\n\
| Cells | Count | Formula | Reads | Role | Value |\n\
| --- | --- | --- | --- | --- | --- |\n\
| C1 | 1 | `=SUM(C2:C4)` | C2:C4 | result | 6 |\n\
| C2:C4 | 3 | `=Inputs!B1*B2` | Inputs!B1, B2 | intermediate | 2 |\n\
\n\
#### Results\n\n\
| Cell | Formula | Value |\n\
| --- | --- | --- |\n\
| C1 | `=SUM(C2:C4)` | 6 |\n";
    assert!(logic.contains(expected), "{logic}");
    let expected_names = "### Names\n\n\
| Name | Scope | Kind | Refers to | Value | Read by | Note |\n\
| --- | --- | --- | --- | --- | --- | --- |\n\
| rate | workbook | name | `Inputs!$B$1` |  |  |  |\n\
| Twice | Calc | LAMBDA | `LAMBDA(x, 2*x)` |  |  | parameters: x |\n";
    assert!(logic.contains(expected_names), "{logic}");
    // The formulas of a copied group are one row, not one row per cell.
    assert_eq!(logic.matches("`=Inputs!B1*").count(), 1, "{logic}");
}

#[test]
fn parameters_table_shows_unit_source() {
    let mut d = workbook();
    d.facets.parameters = vec![
        Parameter {
            unit: Some("m".into()),
            unit_source: Some("unit-cell".into()),
            ..parameter("Inputs!B5", Some("Diameter"), 10.0, 2)
        },
        Parameter {
            name: Some("rate".into()),
            unit: Some("%".into()),
            unit_source: Some("number-format".into()),
            ..parameter("Inputs!B6", Some("Rate"), 0.05, 1)
        },
        Parameter {
            unit: Some("kN".into()),
            unit_source: Some("header".into()),
            ..parameter("Inputs!C9", None, 4.0, 1)
        },
    ];
    let logic = logic_of(&d, true);
    let expected = "### Inputs\n\n\
#### Parameters\n\n\
| Cell | Name | Label | Value | Unit | Unit from | Read by |\n\
| --- | --- | --- | --- | --- | --- | --- |\n\
| B5 |  | Diameter | 10 | m | cell beside | 2 |\n\
| B6 | rate | Rate | 0.05 | % | number format | 1 |\n\
| C9 |  |  | 4 | kN | column header | 1 |\n";
    assert!(logic.contains(expected), "{logic}");
    assert!(!logic.contains("### Calc"), "{logic}");
}

#[test]
fn math_column_only_when_enabled() {
    let mut d = workbook();
    d.facets.formulas = vec![
        Formula {
            latex: Some(r"\frac{\mathrm{B1}}{2}".into()),
            ..cell_formula("Calc!C1", "=B1/2", &["Calc!B1"], Role::Result, 1.0)
        },
        cell_formula("Calc!C2", "=B2", &["Calc!B2"], Role::Result, 1.0),
    ];
    let with = logic_of(&d, true);
    assert!(
        with.contains("| Cells | Count | Formula | Reads | Role | Value | Math |"),
        "{with}"
    );
    assert!(
        with.contains("| C1 | 1 | `=B1/2` | B1 | result | 1 | $`\\frac{\\mathrm{B1}}{2}`$ |"),
        "{with}"
    );
    assert!(
        with.contains("| C2 | 1 | `=B2` | B2 | result | 1 |  |"),
        "{with}"
    );
    let without = logic_of(&d, false);
    assert!(!without.contains("Math"), "{without}");
    assert!(!without.contains('$'), "{without}");
    assert!(
        without.contains("| Cells | Count | Formula | Reads | Role | Value |\n"),
        "{without}"
    );
    // A workbook without LaTeX has no Math column either.
    d.facets.formulas[0].latex = None;
    assert!(!logic_of(&d, true).contains("Math"));
}

#[test]
fn results_table_lists_cached_values() {
    let mut d = workbook();
    d.facets.formulas = vec![
        Formula {
            cached: Some(Value::Text {
                value: "SAFE".into(),
            }),
            ..cell_formula(
                "Calc!B9",
                "=IF(B8>1,\"SAFE\",\"NOT SAFE\")",
                &["Calc!B8"],
                Role::Result,
                0.0,
            )
        },
        Formula {
            cached: None,
            ..cell_formula("Calc!B10", "=B9", &["Calc!B9"], Role::Result, 0.0)
        },
        cell_formula("Calc!B8", "=B7*2", &["Calc!B7"], Role::Intermediate, 1.5),
        Formula {
            role: Some(Role::Result),
            cached: num(4.0),
            ..copied("Calc!D2", "=C2*2", &["Calc!C2"], "p1", "Calc!D2:D3")
        },
        Formula {
            role: Some(Role::Result),
            cached: num(6.0),
            ..copied("Calc!D3", "=C3*2", &["Calc!C3"], "p1", "Calc!D2:D3")
        },
    ];
    let logic = logic_of(&d, true);
    let expected = "#### Results\n\n\
For formulas copied over several cells the first cell is shown.\n\n\
| Cell | Formula | Value |\n\
| --- | --- | --- |\n\
| B9 | `=IF(B8>1,\"SAFE\",\"NOT SAFE\")` | SAFE |\n\
| B10 | `=B9` | not calculated in file |\n\
| D2:D3 | `=C2*2` | 4 |\n";
    assert!(logic.contains(expected), "{logic}");
    assert!(!logic[logic.find("#### Results").unwrap()..].contains("| B8 |"));
}

#[test]
fn hidden_logic_lists_udf_and_external() {
    let mut d = workbook();
    d.facets.formulas = vec![
        cell_formula(
            "Calc!C1",
            "=myFunc(B1)+[1]!addin_f(B2)",
            &["Calc!B1"],
            Role::Result,
            1.0,
        ),
        cell_formula("Calc!C2", "=MYFUNC(B2)", &["Calc!B2"], Role::Result, 1.0),
        cell_formula("Calc!C3", "=NOW()+\"RAND()\"", &[], Role::Constant, 1.0),
        Formula {
            kind: Some(FormulaKind::DataTable),
            range: Some(Anchor("Calc!B5:B11".into())),
            note: Some("what-if data table: column input A2".into()),
            ..cell_formula("Calc!B5", "=TABLE(,A2)", &["Calc!A2"], Role::Result, 2.0)
        },
        Formula {
            kind: Some(FormulaKind::Spill),
            range: Some(Anchor("Calc!E1:E9".into())),
            ..cell_formula("Calc!E1", "=SEQUENCE(9)", &[], Role::Constant, 1.0)
        },
        Formula {
            kind: Some(FormulaKind::Spill),
            range: Some(Anchor("Calc!F1".into())),
            ..cell_formula("Calc!F1", "=SUM(E1#)", &[], Role::Result, 45.0)
        },
    ];
    let embedded = |kind: &str, name: &str, detail: &str, target: Option<&str>| Embedded {
        kind: kind.into(),
        name: name.into(),
        detail: Some(detail.into()),
        target: target.map(Into::into),
    };
    d.facets.embedded = vec![
        embedded("macro", "Module1", "VBA module, never run", None),
        embedded("user-function", "myFunc", "VBA module", Some("Module1")),
        embedded(
            "external-workbook",
            "data.xlsx",
            "[2] in formulas, never opened",
            Some("data.xlsx"),
        ),
        embedded(
            "add-in",
            "Tools.xlam",
            "[1] in formulas, never opened",
            Some("C:\\Tools.xlam"),
        ),
        embedded("user-function", "[1]!addin_f", "C:\\Tools.xlam", None),
    ];
    d.properties.custom.insert(
        "calc_mode".into(),
        Value::Text {
            value: "manual".into(),
        },
    );
    d.properties
        .custom
        .insert("iterative_calculation".into(), Value::Bool { value: true });
    d.properties.custom.insert(
        "iteration_count".into(),
        Value::Number {
            value: 100.0,
            unit: None,
        },
    );
    let logic = logic_of(&d, true);
    let expected = "### Hidden logic\n\n\
| What | Where | Detail |\n\
| --- | --- | --- |\n\
| user function | `myFunc` | VBA module `Module1`, used in 2 formulas |\n\
| user function | `[1]!addin_f` | C:\\\\Tools.xlam, used in 1 formula |\n\
| add-in | Tools.xlam | \\[1\\] in formulas, never opened `C:\\Tools.xlam` |\n\
| external workbook | data.xlsx | \\[2\\] in formulas, never opened `data.xlsx` |\n\
| VBA module | Module1 | VBA module, never run |\n\
| data table | Calc!B5:B11 | `=TABLE(,A2)` what-if data table: column input A2 |\n\
| spill range | Calc!E1:E9 | `=SEQUENCE(9)` |\n\
| volatile function | NOW | in 1 formula: Calc!C3 |\n\
| calculation | workbook | manual, iterative calculation on (100 steps) |\n";
    assert!(logic.contains(expected), "{logic}");
    // A spill formula that fills only its own cell is no spill range.
    assert!(!logic.contains("| spill range | Calc!F1 |"), "{logic}");
}

#[test]
fn stale_results_warning_is_a_callout_at_the_top() {
    let mut d = workbook();
    d.facets.formulas = vec![cell_formula(
        "Calc!C1",
        "=B1",
        &["Calc!B1"],
        Role::Result,
        1.0,
    )];
    d.losses = vec![loss(
        "document",
        "stored results may be old: calculation is set to manual; 1 formula has no stored result",
        LossReason::Uncertain,
        Severity::Warning,
    )];
    let logic = logic_of(&d, true);
    assert!(
        logic.starts_with(
            "## Logic\n\n\
> [!WARNING] Stored results may be old\n\
> Calculation is set to manual; 1 formula has no stored result. Rezzer does not calculate: the values shown are the ones stored in the file.\n\n\
### Calc\n"
        ),
        "{logic}"
    );
}

/// The lines inside the first mermaid fence of `md`.
fn mermaid_lines(md: &str) -> Vec<&str> {
    let start = md.find("```mermaid\n").expect("a mermaid fence") + 11;
    let end = md[start..].find("\n```").expect("fence end") + start;
    md[start..end].lines().collect()
}

/// A node line is `    <id><open>"<label>"<close>` with an id of `[A-Za-z0-9_]` and no raw
/// quote in the label; an edge line joins two such ids.
fn assert_valid_mermaid(lines: &[&str]) {
    assert_eq!(lines[0], "flowchart LR");
    let id_ok =
        |id: &str| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    for line in &lines[1..] {
        let line = line.strip_prefix("    ").expect("indented");
        if let Some((from, rest)) = line.split_once(" -->") {
            assert!(id_ok(from), "{line}");
            let to = match rest.strip_prefix("|\"") {
                Some(r) => {
                    let (label, to) = r.split_once("\"| ").expect("edge label");
                    assert!(!label.contains('"'), "{line}");
                    to
                }
                None => rest.trim_start(),
            };
            assert!(id_ok(to), "{line}");
            continue;
        }
        let open = line.find('"').expect("a quoted label");
        let close = line.rfind('"').expect("a quoted label");
        assert!(
            id_ok(line[..open].trim_end_matches(['[', '(', '{'])),
            "{line}"
        );
        assert!(!line[open + 1..close].contains('"'), "{line}");
    }
}

#[test]
fn sheet_name_with_quote_renders_valid_mermaid() {
    let mut d = workbook();
    let q = "'It''s \"x\" [1]'";
    d.body = vec![sheet("It's \"x\" [1]", q)];
    let a1 = format!("{q}!A1");
    let b1 = format!("{q}!B1");
    d.facets.formulas = vec![cell_formula(&b1, "=A1*2", &[&a1], Role::Result, 2.0)];
    d.facets.graphs = vec![Graph {
        id: "dependencies".into(),
        kind: "dependency".into(),
        nodes: vec![
            Node {
                kind: Some("parameters".into()),
                anchor: Some(Anchor(a1.clone())),
                ..node("v1", &a1)
            },
            Node {
                kind: Some("formula".into()),
                anchor: Some(Anchor(b1.clone())),
                ..node("f1", &b1)
            },
        ],
        edges: vec![edge("v1", "f1", "feeds")],
    }];
    let logic = logic_of(&d, true);
    assert!(logic.contains("### It's \"x\" \\[1\\]\n"), "{logic}");
    assert!(
        logic.contains("| B1 | 1 | `=A1*2` | A1 | result | 2 |"),
        "{logic}"
    );
    let lines = mermaid_lines(&logic);
    assert_valid_mermaid(&lines);
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[1].contains("#quot;x#quot;"), "{lines:?}");
}

fn big_graph(sheets: usize, per_sheet: usize) -> Graph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for s in 0..sheets {
        for i in 0..per_sheet {
            let a = format!("'S {s}'!A{i}");
            nodes.push(Node {
                kind: Some("formula".into()),
                anchor: Some(Anchor(a.clone())),
                ..node(&format!("f{s}_{i}"), &a)
            });
            if i > 0 {
                edges.push(edge(
                    &format!("f{s}_{}", i - 1),
                    &format!("f{s}_{i}"),
                    "feeds",
                ));
            }
        }
        if s > 0 {
            edges.push(edge(&format!("f{}_0", s - 1), &format!("f{s}_0"), "feeds"));
            edges.push(edge(&format!("f{}_1", s - 1), &format!("f{s}_1"), "feeds"));
        }
    }
    nodes.push(Node {
        kind: Some("name".into()),
        anchor: Some(Anchor("name:rate".into())),
        ..node("n1", "name:rate")
    });
    edges.push(edge("n1", "f0_0", "feeds"));
    Graph {
        id: "dependencies".into(),
        kind: "dependency".into(),
        nodes,
        edges,
    }
}

#[test]
fn big_graph_collapses_to_sheets() {
    let mut d = workbook();
    d.body = (0..3)
        .map(|s| sheet(&format!("S {s}"), &format!("'S {s}'")))
        .collect();
    d.facets.formulas = vec![cell_formula(
        "'S 0'!A1",
        "=A0",
        &["'S 0'!A0"],
        Role::Result,
        1.0,
    )];
    d.facets.graphs = vec![big_graph(3, 100)];
    let logic = logic_of(&d, true);
    assert!(
        logic.contains(
            "The graph has 301 nodes and 302 edges, more than can be drawn, so it shows one box per sheet. A number on an arrow counts the links it stands for.\n```mermaid\n"
        ),
        "{logic}"
    );
    let lines = mermaid_lines(&logic);
    assert_valid_mermaid(&lines);
    let text = lines.join("\n");
    assert!(
        text.contains("[\"S 0\"]") && text.contains("[\"S 2\"]"),
        "{text}"
    );
    assert!(text.contains("{{\"Names\"}}"), "{text}");
    // Two links from S 0 to S 1 become one arrow with their count; links inside a sheet
    // are no arrows.
    assert!(text.contains("-->|\"2\"|"), "{text}");
    assert_eq!(lines.len(), 1 + 4 + 3, "{text}");

    // Too many sheets even for the collapsed graph: a note instead of a fence.
    d.facets.graphs = vec![big_graph(250, 2)];
    let logic = logic_of(&d, true);
    let deps = &logic[logic.find("### Dependencies").expect("Dependencies")..];
    assert!(!deps.contains("```mermaid"), "{deps}");
    assert!(
        deps.contains(
            "The graph has 501 nodes and 749 edges on 250 sheets, too many to draw even with one box per sheet. The model JSON has the whole graph."
        ),
        "{deps}"
    );
}

#[test]
fn small_dependency_graph_uses_shapes_and_pattern_counts() {
    let mut d = workbook();
    d.facets.formulas = vec![
        copied("Calc!C2", "=B2", &["Calc!B2"], "p1", "Calc!C2:C3"),
        copied("Calc!C3", "=B3", &["Calc!B3"], "p1", "Calc!C2:C3"),
    ];
    let typed = |id: &str, label: &str, kind: &str| Node {
        kind: Some(kind.into()),
        anchor: Some(Anchor(label.into())),
        ..node(id, label)
    };
    d.facets.graphs = vec![Graph {
        id: "dependencies".into(),
        kind: "dependency".into(),
        nodes: vec![
            typed("v1", "Calc!B2:B3", "parameters"),
            typed("p1", "Calc!C2:C3", "group"),
            typed("n1", "name:rate", "name"),
        ],
        edges: vec![edge("v1", "p1", "feeds"), edge("n1", "p1", "feeds")],
    }];
    let logic = logic_of(&d, true);
    let expected = "### Dependencies\n\n\
Arrows point from what is read to what reads it. Double boxes are formulas copied over several cells, round boxes are parameters, hexagons are names.\n\
```mermaid\n\
flowchart LR\n    \
n0([\"Calc!B2:B3\"])\n    \
n1[[\"Calc!C2:C3, 2 formulas\"]]\n    \
n2{{\"name:rate\"}}\n    \
n0 --> n1\n    \
n2 --> n1\n\
```\n";
    assert!(logic.contains(expected), "{logic}");
}

#[test]
fn rules_table_per_sheet() {
    let mut d = workbook();
    d.facets.rules = vec![Rule {
        kind: "validation".into(),
        anchor: Anchor("Inputs!C6:C9".into()),
        detail: "whole greaterThan".into(),
        formulas: vec!["E6".into()],
        message: Some("Must be | big".into()),
    }];
    let logic = logic_of(&d, true);
    let expected = "### Inputs\n\n\
#### Rules\n\n\
| Kind | Cells | Rule | Formulas | Message |\n\
| --- | --- | --- | --- | --- |\n\
| validation | C6:C9 | whole greaterThan | `E6` | Must be \\| big |\n";
    assert!(logic.contains(expected), "{logic}");
}

#[test]
fn csv_derived_formulas_keep_the_plain_layout() {
    // A CSV document has a sheet section and sheet-like anchors, but it is no workbook.
    let mut d = doc();
    d.source.format = FormatId::Csv;
    d.body = vec![sheet("data", "data")];
    d.facets.formulas = vec![Formula {
        dialect: "derived".into(),
        ..formula("data!C:C", "Total = A + B", &["data!A:A"], None)
    }];
    let logic = logic_of(&d, true);
    assert!(
        logic.starts_with(
            "## Logic\n\nFound in the data, not written as formulas in the file.\n\n| Cells | Formula | Reads | Role | Value |\n"
        ),
        "{logic}"
    );
}

#[test]
fn parameter_runs_fold_into_one_row() {
    let mut d = workbook();
    let mut list: Vec<Parameter> = (0..5)
        .map(|i| Parameter {
            unit: Some("m".into()),
            unit_source: Some("header".into()),
            ..parameter(
                &format!("Inputs!E{}", 49 + i),
                Some("t"),
                0.1 * f64::from(i + 1),
                2,
            )
        })
        .collect();
    // A row without labels, read counts differ.
    list.extend((0..3).map(|i| {
        parameter(
            &format!("Inputs!{}35", ['F', 'G', 'H'][i]),
            None,
            f64::from(i as u32),
            1 + i as u32,
        )
    }));
    // Two cells are no run; a named cell breaks one.
    list.push(parameter("Inputs!A1", Some("x"), 1.0, 1));
    list.push(parameter("Inputs!A2", Some("x"), 2.0, 1));
    d.facets.parameters = list;
    let logic = logic_of(&d, true);
    let expected = "#### Parameters\n\n\
A row with a cell range stands for values next to each other that play one part.\n\n\
| Cell | Name | Label | Value | Unit | Unit from | Read by |\n\
| --- | --- | --- | --- | --- | --- | --- |\n\
| E49:E53 |  | t | 0.1, 0.2, 0.3 ..., 0.5 (5 values) | m | column header | 2 each |\n\
| F35:H35 |  |  | 0, 1, 2 (3 values) |  |  | 1 to 3 each |\n\
| A1 |  | x | 1 |  |  | 1 |\n\
| A2 |  | x | 2 |  |  | 1 |\n";
    assert!(logic.contains(expected), "{logic}");
}

#[test]
fn small_group_that_skips_cells_lists_them() {
    let mut d = workbook();
    d.facets.formulas = vec![
        copied("Calc!B30", "=A30", &["Calc!A30"], "p1", "Calc!B30:B33"),
        cell_formula("Calc!B31", "=1", &[], Role::Constant, 1.0),
        copied("Calc!B33", "=A33", &["Calc!A33"], "p1", "Calc!B30:B33"),
    ];
    let logic = logic_of(&d, true);
    assert!(logic.contains("| B30, B33 | 2 | `=A30` | A30 |"), "{logic}");
}

#[test]
fn results_of_a_mixed_group_are_only_its_result_cells() {
    let mut d = workbook();
    let member = |anchor: &str, expr: &str, role: Role, value: f64| Formula {
        role: Some(role),
        cached: num(value),
        ..copied(anchor, expr, &[], "p1", "Calc!C2:C4")
    };
    d.facets.formulas = vec![
        member("Calc!C2", "=B2", Role::Intermediate, 1.0),
        member("Calc!C3", "=B3", Role::Result, 2.0),
        member("Calc!C4", "=B4", Role::Result, 3.0),
    ];
    let logic = logic_of(&d, true);
    assert!(
        logic.contains("| C2:C4 | 3 | `=B2` |  | intermediate, result | 1 |"),
        "{logic}"
    );
    let results = &logic[logic.find("#### Results").expect("Results")..];
    assert!(results.contains("| C3:C4 | `=B3` | 2 |"), "{results}");
    assert!(!results.contains("C2"), "{results}");
}

#[test]
fn a_group_that_misses_cells_of_its_area_says_so() {
    let mut d = workbook();
    let mut list: Vec<Formula> = (2..=8)
        .map(|r| {
            copied(
                &format!("Calc!C{r}"),
                &format!("=B{r}"),
                &[],
                "p1",
                "Calc!C2:C9",
            )
        })
        .collect();
    list.push(cell_formula("Calc!C9", "=0", &[], Role::Constant, 0.0));
    d.facets.formulas = list;
    let logic = logic_of(&d, true);
    assert!(
        logic.contains("| C2:C9 (7 of 8 cells) | 7 | `=B2` |"),
        "{logic}"
    );
}

#[test]
fn parameter_runs_need_the_same_label_on_every_cell() {
    // A label left of B1 only: B2:B4 have none in the model, so they keep their own rows.
    let mut d = workbook();
    let mut list = vec![parameter("Inputs!B1", Some("Pressure"), 10.0, 1)];
    list.extend((2..=4).map(|r| parameter(&format!("Inputs!B{r}"), None, f64::from(r * 10), 1)));
    d.facets.parameters = list;
    let logic = logic_of(&d, true);
    let expected = "| Cell | Name | Label | Value | Unit | Unit from | Read by |\n\
| --- | --- | --- | --- | --- | --- | --- |\n\
| B1 |  | Pressure | 10 |  |  | 1 |\n\
| B2:B4 |  |  | 20, 30, 40 (3 values) |  |  | 1 each |\n";
    assert!(logic.contains(expected), "{logic}");
    assert!(!logic.contains("B1:B4"), "{logic}");
}

#[test]
fn mermaid_labels_escape_dollars() {
    let mut d = workbook();
    let q = "'a$$x^$$b'";
    d.body = vec![sheet("a$$x^$$b", q)];
    let a1 = format!("{q}!A1");
    let b1 = format!("{q}!B1");
    d.facets.formulas = vec![cell_formula(&b1, "=A1", &[&a1], Role::Result, 1.0)];
    d.facets.graphs = vec![Graph {
        id: "dependencies".into(),
        kind: "dependency".into(),
        nodes: vec![
            Node {
                kind: Some("parameters".into()),
                ..node("v1", &a1)
            },
            Node {
                kind: Some("formula".into()),
                ..node("f1", &b1)
            },
        ],
        edges: vec![edge("v1", "f1", "feeds")],
    }];
    let logic = logic_of(&d, true);
    let lines = mermaid_lines(&logic);
    assert_valid_mermaid(&lines);
    assert!(lines.iter().all(|l| !l.contains('$')), "{lines:?}");
    assert!(lines[1].contains("a#36;#36;x^#36;#36;b"), "{lines:?}");
}

#[test]
fn sheet_heading_that_clashes_with_a_section_says_sheet() {
    let mut d = workbook();
    d.body = vec![sheet("Names", "Names"), sheet("Calc", "Calc")];
    d.facets.formulas = vec![
        cell_formula("Names!B1", "=A1", &["Names!A1"], Role::Result, 1.0),
        cell_formula("Calc!B1", "=A1", &["Calc!A1"], Role::Result, 1.0),
        Formula {
            role: None,
            kind: Some(FormulaKind::Name),
            ..formula("name:rate", "Names!$A$1", &["Names!A1"], None)
        },
    ];
    let logic = logic_of(&d, true);
    assert!(logic.contains("\n### Sheet: Names\n"), "{logic}");
    assert!(logic.contains("\n### Calc\n"), "{logic}");
    assert_eq!(logic.matches("\n### Names\n").count(), 1, "{logic}");
}
