// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Align, Anchor, Block, Bytes, CalloutKind, Completeness, Confidence, Detection, FormatId,
    Inline, LossReason, ReadOptions, ReadOutcome, Reader, SectionKind, Source, TaskState, Value,
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
            Inline::Image { alt, .. } => alt.clone(),
            Inline::LineBreak => "\n".into(),
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
        [
            Block::Code { lang: Some(l), text, .. },
            Block::Paragraph { inlines, .. }
        ] if l == "yaml" && text.contains("unclosed") && plain(inlines) == "Still here."
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
            "h:hello",
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
fn images_are_references_with_graph_and_embedded() {
    let out = read(
        "![a cat](cat.png \"Tom\") and ![[pic.png]]\n\n[![badge](https://img.example/b.svg)](https://ci.example.com)\n",
    );
    let Block::Paragraph { inlines, .. } = &out.document.body[0] else {
        panic!("{:?}", out.document.body);
    };
    assert!(inlines.contains(&Inline::Image {
        target: "cat.png".into(),
        alt: "a cat".into(),
        title: Some("Tom".into()),
    }));
    assert!(
        inlines
            .iter()
            .any(|i| matches!(i, Inline::Image { target, .. } if target == "pic.png"))
    );
    let g = &out.document.facets.graphs[0];
    let node = |id: &str| {
        g.nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("{id}"))
    };
    assert_eq!(node("cat.png").kind.as_deref(), Some("image"));
    assert!(
        g.edges
            .iter()
            .any(|e| e.to == "cat.png" && e.kind == "embeds")
    );
    assert!(
        g.edges
            .iter()
            .any(|e| e.to == "https://img.example/b.svg" && e.kind == "embeds")
    );
    assert!(
        g.edges
            .iter()
            .any(|e| e.to == "https://ci.example.com" && e.kind == "links_to")
    );
    let emb = &out.document.facets.embedded;
    assert_eq!(emb.len(), 3);
    assert_eq!(emb[0].kind, "image");
    assert_eq!(emb[0].name, "a cat");
    assert_eq!(emb[0].target.as_deref(), Some("cat.png"));
    assert!(out.document.losses.is_empty());
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn flow_nesting_bomb_in_yaml_is_fast_and_kept_as_text() {
    let input = format!("---\nk: {}\n---\n\nBody\n", "[".repeat(300_000));
    let start = std::time::Instant::now();
    let out = read(&input);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| matches!(l.why, LossReason::Damaged | LossReason::Limit))
    );
    assert!(matches!(&out.document.body[0], Block::Code { lang: Some(l), .. } if l == "yaml"));
    assert!(matches!(
        out.document.body.last(),
        Some(Block::Paragraph { .. })
    ));
}

#[test]
fn block_nesting_bomb_in_yaml_is_rejected() {
    let mut fm = String::new();
    for i in 0..300 {
        fm.push_str(&" ".repeat(i));
        fm.push_str("a:\n");
    }
    let out = read(&format!("---\n{fm}---\n\nBody\n"));
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged)
    );
}

#[test]
fn rule_block_in_the_middle_is_not_frontmatter() {
    let out = read("Para\n\n---\nfoo: bar\ntitle: Hijack\n---\n\nAfter\n");
    let p = &out.document.properties;
    assert_eq!(p.title, None);
    assert!(p.custom.is_empty());
    assert!(
        out.document
            .body
            .iter()
            .any(|b| matches!(b, Block::Rule { .. }))
    );
    let all = format!("{:?}", out.document.body);
    assert!(all.contains("Hijack"));
    assert!(all.contains("After"));
}

#[test]
fn unreadable_frontmatter_text_is_kept() {
    let out = read("---\nHello world\n---\n\nBody\n");
    assert!(format!("{:?}", out.document.body).contains("Hello world"));
    let out = read("---\ntitle: A\ntitle: B\nother: 1\n---\n\nBody\n");
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged)
    );
    assert!(matches!(&out.document.body[0], Block::Code { text, .. } if text.contains("other: 1")));
}

#[test]
fn oversize_frontmatter_is_truncated_text_with_limit_loss() {
    let big = "x".repeat(2 * 1024 * 1024);
    let out = read(&format!("---\nk: {big}\n---\n\nBody\n"));
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit)
    );
    let Block::Code { text, .. } = &out.document.body[0] else {
        panic!("not code");
    };
    assert_eq!(text.len(), 1024 * 1024);
}

#[test]
fn rules_breaks_strike_and_alignment() {
    let out = read(
        "one\n\n---\n\nline  \nbreak ~~gone~~ soft\nwrap\n\n| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |\n",
    );
    let body = &out.document.body;
    assert!(matches!(&body[1], Block::Rule { .. }));
    let Block::Paragraph { inlines, .. } = &body[2] else {
        panic!("{body:?}");
    };
    assert!(inlines.contains(&Inline::LineBreak));
    assert!(
        inlines
            .iter()
            .any(|i| matches!(i, Inline::Text { text, strike: true, .. } if text == "gone"))
    );
    assert!(plain(inlines).contains("soft wrap"));
    let Block::Table { alignments, .. } = &body[3] else {
        panic!("{body:?}");
    };
    assert_eq!(alignments, &[Align::Left, Align::Center, Align::Right]);
}

#[test]
fn display_math_flag_does_not_leak() {
    let out = read("| $$x$$ |\n|---|\n| y |\n\n$y$\n\nText $$z$$ more\n");
    let body = &out.document.body;
    assert!(
        matches!(&body[1], Block::Paragraph { inlines, .. } if matches!(inlines[0], Inline::Math { .. }))
    );
    assert!(matches!(&body[2], Block::Paragraph { .. }));
}

#[test]
fn rezzer_callout_dialect_is_read_back() {
    let out = read(
        "> [!NOTE] My *Title*\n> body text\n\n> [!tip]- Folded\n> x\n\n> [!DETAILS]+ Open\n> y\n\n> [!unknown] Not a callout\n> z\n",
    );
    let body = &out.document.body;
    match &body[0] {
        Block::Callout {
            kind,
            title,
            folded,
            blocks,
            ..
        } => {
            assert_eq!(*kind, CalloutKind::Note);
            assert_eq!(title.as_deref(), Some("My Title"));
            assert_eq!(*folded, None);
            assert!(
                matches!(&blocks[..], [Block::Paragraph { inlines, .. }] if plain(inlines) == "body text")
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(&body[1], Block::Callout { kind: CalloutKind::Tip, title: Some(t), folded: Some(true), .. } if t == "Folded")
    );
    assert!(matches!(
        &body[2],
        Block::Callout {
            kind: CalloutKind::Details,
            folded: Some(false),
            ..
        }
    ));
    assert!(matches!(&body[3], Block::Quote { .. }));
}

#[test]
fn frontmatter_edge_cases() {
    let out = read("---\ndate: true\nempty:\nn: 1\n---\n");
    let p = &out.document.properties;
    assert_eq!(p.created, None);
    assert_eq!(p.custom["date"], Value::Bool { value: true });
    assert_eq!(p.custom["empty"], text(""));
}

#[test]
fn footnote_in_quote_leaves_no_empty_quote() {
    let out = read("x[^1]\n\n> [^1]: in a quote\n");
    assert!(
        !out.document
            .body
            .iter()
            .any(|b| matches!(b, Block::Quote { .. }))
    );
    assert!(format!("{:?}", out.document.body).contains("in a quote"));
}

#[test]
fn slug_ignores_html_tags() {
    let out = read("# H with <span>html</span> x\n");
    assert!(matches!(&out.document.body[0],
        Block::Heading { anchor: Some(a), .. } if a.0 == "h:h-with-html-x"));
}

#[test]
fn depth_loss_names_the_line() {
    let input = format!("a\n\nb\n\n{} deep\n", ">".repeat(300));
    let limits = Limits {
        max_depth: 16,
        ..Limits::default()
    };
    let out = read_with(input.as_bytes(), limits);
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.why == LossReason::Limit)
        .unwrap();
    assert!(loss.location.contains("line 5"), "{}", loss.location);
}

fn assert_bomb_rejected(frontmatter_value: &str) {
    let input = format!("---\nk: {frontmatter_value}\n---\n\nBody\n");
    let start = std::time::Instant::now();
    let out = read(&input);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| matches!(l.why, LossReason::Damaged | LossReason::Limit))
    );
    assert!(matches!(&out.document.body[0], Block::Code { lang: Some(l), .. } if l == "yaml"));
}

#[test]
fn quoted_bracket_bombs_are_rejected_fast() {
    // Over the parse size cap.
    assert_bomb_rejected(&"[\"]\",".repeat(200_000));
    assert_bomb_rejected(&"[']',".repeat(200_000));
    // Under the cap: the depth scan itself must see the nesting.
    assert_bomb_rejected(&"[\"]\",".repeat(40_000));
    assert_bomb_rejected(&"[']',".repeat(40_000));
    // A plain scalar with a quote character does not hide real nesting.
    assert_bomb_rejected(&format!("[a '{}", "[".repeat(5_000)));
    // A stray quote in a block scalar does not flip the quote parity afterwards.
    let input = format!(
        "---\nk: |\n  \"\nz: {}\n---\n\nBody\n",
        "[\"]\",".repeat(40_000)
    );
    let out = read(&input);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged)
    );
}

#[test]
fn quoted_brackets_in_normal_frontmatter_still_parse() {
    let out = read("---\ntitle: \"a ] b [ c\"\nk: ['x]', \"y[\", it's]\nnote: it's # [[[\n---\n");
    assert_eq!(out.document.properties.title.as_deref(), Some("a ] b [ c"));
    assert!(out.document.losses.is_empty());
}

#[test]
fn later_dash_block_is_normal_markdown() {
    let out = read("Para\n\n---\n- a\n- b\n\nSee [l](x.md)\n\n---\n\nAfter\n");
    let body = &out.document.body;
    assert!(
        body.iter()
            .any(|b| matches!(b, Block::List { items, .. } if items.len() == 2))
    );
    assert_eq!(
        body.iter()
            .filter(|b| matches!(b, Block::Rule { .. }))
            .count(),
        2
    );
    let g = &out.document.facets.graphs[0];
    assert!(g.nodes.iter().any(|n| n.id == "x.md"));
}

#[test]
fn hard_break_ends_the_callout_title() {
    let out = read("> [!WARNING] Title  \n> body line\n> second\n");
    match &out.document.body[0] {
        Block::Callout { title, blocks, .. } => {
            assert_eq!(title.as_deref(), Some("Title"));
            assert!(
                matches!(&blocks[..], [Block::Paragraph { inlines, .. }] if plain(inlines) == "body line second")
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn image_kind_wins_and_h_prefix_is_not_doubled() {
    let out = read("[x](cat.png) ![c](cat.png) [y](#h:foo)\n");
    let g = &out.document.facets.graphs[0];
    let cat = g.nodes.iter().find(|n| n.id == "cat.png").unwrap();
    assert_eq!(cat.kind.as_deref(), Some("image"));
    assert!(g.nodes.iter().any(|n| n.id == "h:foo"));
    assert!(!g.nodes.iter().any(|n| n.id == "h:h:foo"));
}

#[test]
fn continuation_line_quote_does_not_hide_nesting() {
    // YAML reads the quote on the second line as plain text of the scalar `a "`.
    assert_bomb_rejected(&format!("a\n \"\nz: {}", "[".repeat(250_000)));
    assert_bomb_rejected(&format!("a\n '\nz: {}", "[".repeat(250_000)));
}

#[test]
fn wide_alias_of_a_big_map_is_cut_by_the_node_budget() {
    let keys: Vec<String> = (0..2000).map(|i| format!("k{i}: v")).collect();
    let mut fm = format!("a: &a {{{}}}\nb: [", keys.join(", "));
    while fm.len() < 256 * 1024 - 10 {
        fm.push_str("*a,");
    }
    fm.push(']');
    let input = format!("---\n{fm}\n---\n\nBody\n");
    let start = std::time::Instant::now();
    let out = read(&input);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged)
    );
    assert!(matches!(&out.document.body[0], Block::Code { lang: Some(l), .. } if l == "yaml"));
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn small_aliases_and_anchors_still_map() {
    let out = read(
        "---\nbase: &b\n  x: 1\n  y: [a, b]\ncopy: *b\nlist: [*b, \"q]\"]\nname: &n Jordi\nauthor: *n\nflag: yes\nn: 0x1F\nf: 1.5e2\nneg: -3\nnull_: ~\nstr: !!str 123\nd: 2024-01-02\n---\n",
    );
    let p = &out.document.properties;
    assert!(out.document.losses.is_empty(), "{:?}", out.document.losses);
    assert_eq!(p.authors, vec!["Jordi"]);
    assert_eq!(p.custom["base.x"], Value::number(1.0, None));
    assert_eq!(
        p.custom["copy.y"],
        Value::List {
            value: vec![text("a"), text("b")]
        }
    );
    assert_eq!(p.custom["copy.x"], Value::number(1.0, None));
    assert_eq!(p.custom["flag"], text("yes"));
    assert_eq!(p.custom["n"], Value::number(31.0, None));
    assert_eq!(p.custom["f"], Value::number(150.0, None));
    assert_eq!(p.custom["neg"], Value::number(-3.0, None));
    assert_eq!(p.custom["null_"], text(""));
    assert_eq!(p.custom["str"], text("123"));
    assert_eq!(
        p.custom["d"],
        Value::Date {
            value: "2024-01-02".into()
        }
    );
}

#[test]
fn blank_line_after_opening_dashes_is_no_frontmatter() {
    let out = read("---\n\n# Intro\n\nSome [a link](x.md)\n\n---\n\nMore\n");
    assert!(out.document.losses.is_empty());
    assert!(
        out.document
            .body
            .iter()
            .any(|b| matches!(b, Block::Heading { .. }))
    );
    assert!(
        out.document.facets.graphs[0]
            .nodes
            .iter()
            .any(|n| n.id == "x.md")
    );
    let out = read("---\n---\n\nText\n");
    assert_eq!(
        out.document
            .body
            .iter()
            .filter(|b| matches!(b, Block::Rule { .. }))
            .count(),
        2
    );
}

/// Bytes of keys plus values in `custom`, measured as the reader caps them.
fn custom_bytes(out: &ReadOutcome) -> usize {
    fn value_bytes(v: &Value) -> usize {
        match v {
            Value::Text { value } | Value::Date { value } | Value::DateTime { value } => {
                value.len()
            }
            Value::Number { unit, .. } => 8 + unit.as_ref().map_or(0, String::len),
            Value::Percent { .. } => 8,
            Value::Bool { .. } => 1,
            Value::List { value } => value.iter().map(value_bytes).sum(),
        }
    }
    out.document
        .properties
        .custom
        .iter()
        .map(|(k, v)| k.len() + value_bytes(v))
        .sum()
}

fn assert_flatten_bounded(fm: &str) -> ReadOutcome {
    let input = format!("---\n{fm}\n---\n\nBody\n");
    let start = std::time::Instant::now();
    let out = read(&input);
    let took = start.elapsed();
    assert!(took < std::time::Duration::from_secs(1), "{took:?}");
    assert!(custom_bytes(&out) <= 1024 * 1024, "{}", custom_bytes(&out));
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.location == "frontmatter"),
        "{:?}",
        out.document.losses
    );
    assert!(matches!(&out.document.body[0], Block::Code { lang: Some(l), .. } if l == "yaml"));
    assert_eq!(out.completeness, Completeness::Partial);
    out
}

fn b36(mut i: usize) -> String {
    let d = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut v = vec![b'k'];
    loop {
        v.push(d[i % 36]);
        i /= 36;
        if i == 0 {
            break;
        }
    }
    String::from_utf8(v).unwrap()
}

#[test]
fn long_key_path_times_aliased_leaves_is_bounded() {
    // `l: &l {100 keys}`, one explicit key of 230,000 bytes, 440 aliases of `l` below it.
    let mut fm = String::from("l: &l {");
    for i in 0..100 {
        fm.push_str(&format!("{}: 1,", b36(i)));
    }
    fm.push_str("z: 1}\n");
    fm.push_str(&format!("? {}\n:\n", "K".repeat(230_000)));
    for i in 0..440 {
        fm.push_str(&format!("  {}: *l\n", b36(i)));
    }
    assert!(fm.len() < 256 * 1024);
    let out = assert_flatten_bounded(&fm);
    // What was written before the budget ran out is kept.
    assert!(out.document.properties.custom.contains_key("l.k0"));
}

#[test]
fn long_key_path_without_aliases_is_bounded() {
    let mut fm = format!("? {}\n:\n", "K".repeat(128 * 1024));
    let mut i = 0;
    while fm.len() < 256 * 1024 - 20 {
        fm.push_str(&format!("  {}: 1\n", b36(i)));
        i += 1;
    }
    assert_flatten_bounded(&fm);
}

#[test]
fn flattened_output_is_capped_at_one_mebibyte() {
    // 300 leaves under a 4,000-byte key: about 1.2 MB of key paths, well inside the work budget.
    let mut fm = format!("? {}\n:\n", "K".repeat(4000));
    for i in 0..300 {
        fm.push_str(&format!("  {}: 1\n", b36(i)));
    }
    let out = assert_flatten_bounded(&fm);
    assert!(custom_bytes(&out) > 900 * 1024, "{}", custom_bytes(&out));
    assert!(
        out.document.losses.iter().any(|l| l.what.contains("1 MiB")),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn second_yaml_document_is_a_loss() {
    let out = read("---\na: 1\n--- b: 2\nc: 3\n---\n\nBody\n");
    let p = &out.document.properties;
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged
                && l.what.contains("extra YAML document in frontmatter")),
        "{:?}",
        out.document.losses
    );
    assert_eq!(p.custom["a"], Value::number(1.0, None));
    assert!(!p.custom.contains_key("b") && !p.custom.contains_key("c"));
    assert!(matches!(&out.document.body[0], Block::Code { lang: Some(l), .. } if l == "yaml"));
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn wikilinks_are_marked() {
    let out = read("See [[Getting Started]], [[a#sec|label]] and [web](https://example.com).\n");
    let Block::Paragraph { inlines, .. } = &out.document.body[0] else {
        panic!("not a paragraph");
    };
    let links: Vec<(&str, &str, bool)> = inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Link {
                target,
                wiki,
                inlines,
                ..
            } => Some((
                target.as_str(),
                if inlines.is_empty() { "" } else { "text" },
                *wiki,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        links.iter().map(|l| (l.0, l.2)).collect::<Vec<_>>(),
        vec![
            ("Getting Started", true),
            ("a#sec", true),
            ("https://example.com", false)
        ]
    );
}

#[test]
fn empty_pothole_wikilink_does_not_repeat_the_paragraph() {
    let out = read("A [[x|]] b [[y]] end.\n");
    let Block::Paragraph { inlines, .. } = &out.document.body[0] else {
        panic!("not a paragraph");
    };
    assert_eq!(plain(inlines), "A x b y end.");
    let targets: Vec<&str> = inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Link { target, wiki, .. } => {
                assert!(wiki);
                Some(target.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(targets, ["x", "y"]);
}

#[test]
fn empty_pothole_in_code_is_left_alone() {
    let out = read("`[[x|]]` and\n\n```\n[[x|]]\n```\n");
    let json = serde_json::to_string(&out.document.body).unwrap();
    assert_eq!(json.matches("[[x|]]").count(), 2, "{json}");
}
