// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use rezzer_model::{
    Align, Anchor, Annotation, Asset, Block, CalloutKind, Cell, Completeness, Confidence,
    Detection, Document, Edge, Embedded, Event as TimelineEvent, FormatId, Formula, Graph, Inline,
    Loss, LossReason, Node, ReaderInfo, Role, SectionKind, Severity, SourceInfo, StatusColor,
    TaskState, Value,
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
        formula(
            "Sheet1!D1",
            "=SUM(C2:C3)",
            &["Sheet1!C2", "Sheet1!C3"],
            None,
        ),
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
| Cells | Formula | Reads | Role |\n\
| --- | --- | --- | --- |\n\
| Sheet1!D1 | `=SUM(C2:C3)` | Sheet1!C2, Sheet1!C3 | result |\n\
\n\
### Pattern: p1\n\n\
| Cells | Formula | Reads | Role |\n\
| --- | --- | --- | --- |\n\
| Sheet1!C2 | `=A2*B2` | Sheet1!A2, Sheet1!B2 | result |\n\
| Sheet1!C3 | `=A3*B3` | Sheet1!A3, Sheet1!B3 | result |\n\
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
    ];
    let md = render(&d, Completeness::Partial, &opts());
    let expected = "## Not read\n\n\
| Where | What | Why | Severity |\n\
| --- | --- | --- | --- |\n\
| page 3 | table \\| odd | not implemented | warning |\n\
| A1 | header guessed | uncertain | info |\n";
    assert!(md.ends_with(expected), "{md}");
}

fn rich_document() -> Document {
    let mut d = doc();
    d.properties.title = Some("Quarterly report".into());
    d.properties.authors = vec!["Ann".into()];
    d.properties.keywords = vec!["finance".into()];
    d.properties.created = Some(Value::DateTime {
        value: "2025-11-30T08:00:00Z".into(),
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

#[test]
fn overview_hook_is_empty_for_now() {
    assert_eq!(render_overview(&rich_document()), None);
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
        md.contains("costs $5 and \\$\\$x\\$\\$ and \\$`y`\\$\n"),
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
        rows: vec![row(), vec![huge], row()],
        alignments: Vec::new(),
        anchor: None,
    }]);
    assert!(md.contains("Merged cells: A2:XFD3"), "{}", &md[..300]);
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
