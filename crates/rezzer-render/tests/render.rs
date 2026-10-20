// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use rezzer_model::{
    Align, Anchor, Annotation, Asset, Block, CalloutKind, Cell, Completeness, Confidence,
    Detection, Document, Edge, Embedded, Event as TimelineEvent, FormatId, Formula, Graph, Inline,
    Loss, LossReason, Node, Overview, OverviewPart, ReaderInfo, Role, SectionKind, Severity,
    SourceInfo, StatusColor, TaskState, Value,
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
