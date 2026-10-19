// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Block, Bytes, CalloutKind, Completeness, Confidence, Detection, FormatId, Inline,
    LossReason, ReadOptions, ReadOutcome, Reader, SectionKind, Source, TaskState, Value,
};
use rezzer_read_markdown::MarkdownReader;

fn source(bytes: &[u8]) -> Source {
    Source {
        path: None,
        name: "note.md".into(),
        bytes: Bytes::Owned(Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format: FormatId::Markdown,
        detection: Detection {
            confidence: Confidence::Likely,
            evidence: "md".into(),
        },
        modified: None,
    }
}

fn read_with(bytes: &[u8], limits: Limits) -> ReadOutcome {
    MarkdownReader.read(
        &source(bytes),
        &ReadOptions::default(),
        &Budget::new(limits),
    )
}

fn read(text: &str) -> ReadOutcome {
    read_with(text.as_bytes(), Limits::default())
}

fn plain(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .map(|i| match i {
            Inline::Text { text, .. } | Inline::Code { text } => text.clone(),
            Inline::Link { inlines, .. } => plain(inlines),
            Inline::Math { latex } => format!("${latex}$"),
            Inline::FootnoteRef { id } => format!("[^{id}]"),
            other => panic!("unexpected inline {other:?}"),
        })
        .collect()
}

fn text(s: &str) -> Value {
    Value::Text { value: s.into() }
}

#[test]
fn yaml_frontmatter_to_properties() {
    let out = read(
        "---\ntitle: My Note\ntags: [a, b]\nversions:\n  fpt: '*'\n  ghes: '>=3.0'\nauthor: Jordi\ndate: 2024-05-01\ndraft: true\nweight: 3\nlang: de\n---\n\n# Heading\n",
    );
    let p = &out.document.properties;
    assert_eq!(p.title.as_deref(), Some("My Note"));
    assert_eq!(p.keywords, vec!["a", "b"]);
    assert_eq!(p.authors, vec!["Jordi"]);
    assert_eq!(p.language.as_deref(), Some("de"));
    assert_eq!(
        p.created,
        Some(Value::Date {
            value: "2024-05-01".into()
        })
    );
    assert_eq!(p.custom["versions.fpt"], text("*"));
    assert_eq!(p.custom["versions.ghes"], text(">=3.0"));
    assert_eq!(p.custom["draft"], Value::Bool { value: true });
    assert_eq!(p.custom["weight"], Value::number(3.0, None));
    assert!(!p.custom.contains_key("title"));
    assert_eq!(out.completeness, Completeness::Complete);
    // The frontmatter is not repeated in the body.
    assert!(matches!(
        out.document.body[0],
        Block::Heading { level: 1, .. }
    ));
}

#[test]
fn toml_frontmatter_to_properties() {
    let out = read(
        "+++\ntitle = \"Traits\"\ndescription = \"About traits\"\nauthors = [\"A\", \"B\"]\ndate = 2024-05-01\n[extra]\nmath = true\n+++\n\nBody\n",
    );
    let p = &out.document.properties;
    assert_eq!(p.title.as_deref(), Some("Traits"));
    assert_eq!(p.description.as_deref(), Some("About traits"));
    assert_eq!(p.authors, vec!["A", "B"]);
    assert_eq!(
        p.created,
        Some(Value::Date {
            value: "2024-05-01".into()
        })
    );
    assert_eq!(p.custom["extra.math"], Value::Bool { value: true });
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn broken_frontmatter_is_loss_not_failure() {
    let out = read("---\ntitle: [unclosed\nfoo: : :\n---\n\nStill here.\n");
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged && l.location == "frontmatter")
    );
    assert!(matches!(
        &out.document.body[..],
        [Block::Paragraph { inlines, .. }] if plain(inlines) == "Still here."
    ));
}

#[test]
fn title_falls_back_to_first_h1() {
    let out = read("Intro\n\n# The Real *Title*\n\n# Second\n");
    assert_eq!(
        out.document.properties.title.as_deref(),
        Some("The Real Title")
    );
}

#[test]
fn alert_becomes_callout() {
    let out = read("> [!WARNING]\n> Careful with *this*.\n\n> plain quote\n");
    match &out.document.body[..] {
        [
            Block::Callout { kind, blocks, .. },
            Block::Quote { blocks: q, .. },
        ] => {
            assert_eq!(*kind, CalloutKind::Warning);
            assert!(matches!(
                &blocks[..],
                [Block::Paragraph { inlines, .. }] if plain(inlines) == "Careful with this."
            ));
            assert_eq!(q.len(), 1);
        }
        other => panic!("unexpected body {other:?}"),
    }
}

#[test]
fn wikilinks_in_link_graph() {
    let out = read(
        "See [[Getting Started]] and [site](https://example.com) and [up](#hello) and [rel](../a.md).\n\nNot a link: `[[In Code]]`. Again [[Getting Started]].\n",
    );
    let graph = &out.document.facets.graphs[0];
    assert_eq!(graph.id, "links");
    assert_eq!(graph.kind, "link");
    let ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "self",
            "Getting Started",
            "https://example.com",
            "#hello",
            "../a.md"
        ]
    );
    assert!(!ids.contains(&"In Code"));
    assert_eq!(graph.edges.len(), 4);
    assert!(
        graph
            .edges
            .iter()
            .all(|e| e.from == "self" && e.kind == "links_to")
    );
    let Block::Paragraph { inlines, .. } = &out.document.body[0] else {
        panic!("not a paragraph");
    };
    let links: Vec<(&str, bool)> = inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Link {
                target, internal, ..
            } => Some((target.as_str(), *internal)),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        vec![
            ("Getting Started", true),
            ("https://example.com", false),
            ("#hello", true),
            ("../a.md", true)
        ]
    );
}

#[test]
fn no_links_no_graph() {
    assert!(read("plain text\n").document.facets.graphs.is_empty());
}

#[test]
fn heading_anchor_slug() {
    let out = read("## Hello, World!\n\n## Hello, World!\n\n### `Code` & more\n");
    let anchors: Vec<_> = out
        .document
        .body
        .iter()
        .map(|b| match b {
            Block::Heading { anchor, .. } => anchor.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        anchors,
        vec![
            Some(Anchor("h:hello-world".into())),
            Some(Anchor("h:hello-world-1".into())),
            Some(Anchor("h:code--more".into())),
        ]
    );
}

#[test]
fn deep_nesting_hits_depth_limit() {
    let input = format!("{} deep\n\nafter\n", ">".repeat(10_000));
    let limits = Limits {
        max_depth: 256,
        ..Limits::default()
    };
    let out = read_with(input.as_bytes(), limits);
    assert_eq!(out.completeness, Completeness::Partial);
    let limit_losses: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(limit_losses.len(), 1);
    // The text after the nested quote is still read.
    assert!(
        out.document
            .body
            .iter()
            .any(|b| matches!(b, Block::Paragraph { inlines, .. } if plain(inlines) == "after"))
    );
}

#[test]
fn deep_lists_hit_depth_limit() {
    let mut input = String::new();
    for i in 0..2000 {
        input.push_str(&" ".repeat(i * 2));
        input.push_str("- x\n");
    }
    let limits = Limits {
        max_depth: 64,
        ..Limits::default()
    };
    let out = read_with(input.as_bytes(), limits);
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit)
    );
}

#[test]
fn tasks_code_math_and_tables() {
    let out = read(
        "- [ ] open\n- [x] done\n- plain\n\n```rust,ignore\nfn main() {}\n```\n\n$$\nE = mc^2\n$$\n\nInline $a+b$ here.\n\n| A | B |\n|---|---|\n| 1 | **2** |\n",
    );
    let body = &out.document.body;
    let Block::List { items, ordered, .. } = &body[0] else {
        panic!("{body:?}");
    };
    assert!(!ordered);
    assert!(matches!(
        &items[0][..],
        [Block::Task { state: TaskState::Open, inlines, .. }] if plain(inlines) == "open"
    ));
    assert!(matches!(
        &items[1][..],
        [Block::Task {
            state: TaskState::Done,
            ..
        }]
    ));
    assert!(matches!(
        &items[2][..],
        [Block::Paragraph { inlines, .. }] if plain(inlines) == "plain"
    ));
    assert!(matches!(
        &body[1],
        Block::Code { lang: Some(l), text, .. } if l == "rust" && text == "fn main() {}"
    ));
    assert!(matches!(
        &body[2],
        Block::Math { latex, source_kind, .. } if latex.trim() == "E = mc^2" && source_kind == "latex"
    ));
    assert!(matches!(
        &body[3],
        Block::Paragraph { inlines, .. } if plain(inlines) == "Inline $a+b$ here."
    ));
    let Block::Table {
        header_rows, rows, ..
    } = &body[4]
    else {
        panic!("{body:?}");
    };
    assert_eq!(*header_rows, 1);
    assert_eq!(rows.len(), 2);
    assert_eq!(plain(&rows[1][1].inlines), "2");
    assert!(matches!(
        &rows[1][1].inlines[0],
        Inline::Text { strong: true, .. }
    ));
}

#[test]
fn footnotes_become_a_section() {
    let out = read("Text[^a] and more[^b].\n\n[^a]: First note.\n[^b]: Second note.\n");
    let body = &out.document.body;
    let Block::Paragraph { inlines, .. } = &body[0] else {
        panic!("{body:?}");
    };
    assert!(inlines.contains(&Inline::FootnoteRef { id: "a".into() }));
    let Block::Section {
        kind,
        title,
        blocks,
        ..
    } = body.last().unwrap()
    else {
        panic!("{body:?}");
    };
    assert_eq!(*kind, SectionKind::Other);
    assert_eq!(title.as_deref(), Some("Footnotes"));
    assert_eq!(blocks.len(), 2);
    assert!(matches!(
        &blocks[0],
        Block::Section { anchor: Some(a), .. } if a.0 == "fn:a"
    ));
    assert!(matches!(
        &blocks[1],
        Block::Section { anchor: Some(a), .. } if a.0 == "fn:b"
    ));
}

#[test]
fn liquid_tags_and_html_stay_text() {
    let out = read("{% if x %}Hello {{ name }}{% endif %}\n\n<div>raw</div>\n");
    assert!(matches!(
        &out.document.body[0],
        Block::Paragraph { inlines, .. } if plain(inlines) == "{% if x %}Hello {{ name }}{% endif %}"
    ));
    assert!(matches!(
        &out.document.body[1],
        Block::Code { lang: Some(l), text, .. } if l == "html" && text == "<div>raw</div>"
    ));
}

#[test]
fn legacy_encoding_is_decoded_and_recorded() {
    // "Grüße" in windows-1252 plus enough text for the detector.
    let mut bytes = b"# Gr".to_vec();
    bytes.extend_from_slice(&[0xFC, 0xDF, b'e']);
    bytes.extend_from_slice(b"\n\nDas ist ein l\xE4ngerer Text mit Umlauten: \xE4\xF6\xFC\xDF.\n");
    let out = read_with(&bytes, Limits::default());
    assert!(out.document.source.encoding.is_some());
    assert_eq!(out.document.source.reader.as_ref().unwrap().id, "markdown");
    assert_eq!(out.document.properties.title.as_deref(), Some("Grüße"));
}

#[test]
fn images_are_kept_as_text() {
    let out = read("![a cat](cat.png \"Tom\")\n");
    assert!(matches!(
        &out.document.body[0],
        Block::Paragraph { inlines, .. } if plain(inlines) == "![a cat](cat.png \"Tom\")"
    ));
}
