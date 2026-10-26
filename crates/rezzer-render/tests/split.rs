// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The splitter: a document over the part size is cut into units at its own borders, each
//! within the limit (plan 5 Task 5, spec 5, decisions 9, 10 and 12).

use std::collections::BTreeMap;

use rezzer_model::{
    Anchor, Annotation, Asset, Block, Cell, Certainty, Completeness, Confidence, Detection,
    Document, Embedded, Event, Evidence, FormatId, Formula, Inline, LayoutKind, OutlineEntry,
    ReaderInfo, Role, SectionKind, SourceInfo, TableLayout, Value,
};
use std::sync::Arc;

use rezzer_limits::{Budget, Limits};
use rezzer_model::{Bytes, ReadOptions, Reader, Source};
use rezzer_read_markdown::MarkdownReader;
use rezzer_render::{RenderOptions, Unit, render, split};

fn source(name: &str, format: FormatId) -> SourceInfo {
    SourceInfo {
        path: Some(format!("/data/{name}")),
        name: name.into(),
        sha256: "abc123".into(),
        size: 42,
        format,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "test".into(),
        },
        modified: Some("2026-01-05T10:00:00Z".into()),
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
        encoding: Some("utf-8".into()),
        reader: Some(ReaderInfo {
            id: "test".into(),
            version: "0.1.0".into(),
        }),
    }
}

fn doc(format: FormatId) -> Document {
    let name = match format {
        FormatId::Pdf => "report.pdf",
        FormatId::Xlsx => "book.xlsx",
        _ => "report.md",
    };
    Document::new(source(name, format))
}

fn opts() -> RenderOptions {
    RenderOptions {
        generated_at: "2026-10-20T12:00:00Z".into(),
        rezzer_version: "0.1.0".into(),
        ..RenderOptions::default()
    }
}

/// `n` characters of plain words, no character that the renderer escapes.
fn words(n: usize) -> String {
    let mut out = String::with_capacity(n + 8);
    let mut i = 0usize;
    while out.len() < n {
        out.push_str(["lorem ", "ipsum ", "dolor ", "sit ", "amet "][i % 5]);
        i += 1;
    }
    out.truncate(n);
    if out.ends_with(' ') {
        out.pop();
        out.push('x');
    }
    out
}

fn para(text: &str) -> Block {
    Block::Paragraph {
        inlines: vec![Inline::plain(text)],
        role: None,
        anchor: None,
    }
}

fn heading(level: u8, text: &str) -> Block {
    Block::Heading {
        level,
        inlines: vec![Inline::plain(text)],
        anchor: Some(Anchor(format!(
            "h:{}",
            text.to_lowercase().replace(' ', "-")
        ))),
    }
}

fn page(n: u32, blocks: Vec<Block>) -> Block {
    Block::Section {
        kind: SectionKind::Page,
        title: None,
        anchor: Some(Anchor(format!("p{n}"))),
        blocks,
    }
}

fn sheet(title: &str, blocks: Vec<Block>) -> Block {
    Block::Section {
        kind: SectionKind::Sheet,
        title: Some(title.into()),
        anchor: Some(Anchor(title.into())),
        blocks,
    }
}

fn units(d: &Document, part_size: usize) -> Vec<Unit> {
    split(d, &opts(), part_size, "b").expect("the document is over the part size")
}

fn names(list: &[Unit]) -> Vec<&str> {
    list.iter().map(|u| u.file.as_str()).collect()
}

/// Every unit is within the part size by Crystalline's measure (characters / 4).
fn assert_within(list: &[Unit], part_size: usize) {
    for u in list {
        let tokens = u.markdown.chars().count() / 4;
        assert!(
            tokens <= part_size,
            "{} has {tokens} tokens, over {part_size}",
            u.file
        );
    }
}

/// The body of a rendered file: what follows the frontmatter.
fn body(md: &str) -> &str {
    let rest = md.strip_prefix("---\n").expect("frontmatter");
    let end = rest.find("\n---\n").expect("frontmatter end");
    &rest[end + 5..]
}

#[test]
fn fits_gives_none() {
    // A body of exactly 2,500 tokens (10,003 characters) fits; one character more does not.
    let with = |n: usize| {
        let mut d = doc(FormatId::Markdown);
        d.body = vec![para(&words(n))];
        d
    };
    let base = body(&render(&with(1_000), Completeness::Complete, &opts()))
        .chars()
        .count();
    let n = 1_000 + 10_003 - base;
    let exact = with(n);
    let md = render(&exact, Completeness::Complete, &opts());
    assert_eq!(body(&md).chars().count(), 10_003);
    assert_eq!(split(&exact, &opts(), 2_500, "b"), None);
    assert!(split(&with(n + 1), &opts(), 2_500, "b").is_some());
    // Part size 0 never splits.
    assert_eq!(split(&with(200_000), &opts(), 0, "b"), None);
}

fn cell(text: &str, at: &str) -> Cell {
    let mut c = Cell::new(vec![Inline::plain(text)]);
    c.anchor = Some(Anchor(at.into()));
    c
}

/// A sheet table with header row 1 and data rows 2 to `last` in columns A and B.
fn sheet_table(sheet: &str, last: u32) -> Block {
    let mut rows = vec![vec![
        cell("Item", &format!("{sheet}!A1")),
        cell("Amount", &format!("{sheet}!B1")),
    ]];
    rows.extend((2..=last).map(|r| {
        vec![
            cell(&format!("item number {r}"), &format!("{sheet}!A{r}")),
            cell(&format!("{}", r * 7), &format!("{sheet}!B{r}")),
        ]
    }));
    Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: Some(Anchor(format!("{sheet}!A1:B{last}"))),
        layout: Some(Box::new(TableLayout {
            kind: LayoutKind::Table,
            key: format!("{sheet}!A1"),
            range: format!("A1:B{last}"),
            data: Some(format!("A2:B{last}")),
            header_rows: vec![1],
            header_paths: vec!["Item".into(), "Amount".into()],
            label_cols: 1,
            total_columns: Vec::new(),
            certainty: Certainty::Rule,
            found_by: vec![Evidence::Rule {
                id: "H1".into(),
                detail: "header by type change".into(),
            }],
            unsure: Vec::new(),
            rows: Vec::new(),
            hidden_header: false,
        })),
    }
}

fn workbook() -> Document {
    let mut d = doc(FormatId::Xlsx);
    for i in 1..=20 {
        d.body
            .push(sheet(&format!("S{i}"), vec![para(&words(100))]));
    }
    d.body.push(sheet("Big", vec![sheet_table("Big", 2_001)]));
    for i in 21..=40 {
        d.body
            .push(sheet(&format!("S{i}"), vec![para(&words(100))]));
    }
    d
}

#[test]
fn sheets_become_units_and_pack() {
    let list = units(&workbook(), 2_500);
    assert_within(&list, 2_500);
    // The small sheets before and after the large one are packed; the large one is cut into
    // row ranges.
    let (rows, other): (Vec<&Unit>, Vec<&Unit>) =
        list.iter().partition(|u| u.file.starts_with("big-rows-"));
    assert_eq!(
        other.iter().map(|u| u.file.as_str()).collect::<Vec<_>>(),
        ["s1", "s21"]
    );
    assert_eq!(other[0].location, "S1 to S20");
    assert_eq!(other[1].location, "S21 to S40");
    assert!(other[0].markdown.contains("## S20"));
    assert!(rows.len() > 2, "{:?}", names(&list));
    assert_eq!(list[0].file, "s1");
    assert_eq!(list.last().map(|u| u.file.as_str()), Some("s21"));
    // The ranges cover every data row once, in order, each with the header row.
    let mut next = 2;
    for u in &rows {
        let range = u.file.strip_prefix("big-rows-").expect("a row range");
        let (a, b) = range.split_once('-').expect("two row numbers");
        let (a, b): (u32, u32) = (a.parse().expect("first"), b.parse().expect("last"));
        assert_eq!(a, next, "{:?}", names(&list));
        assert!(b >= a);
        next = b + 1;
        assert_eq!(u.location, format!("Big!A{a}:B{b}"));
        assert!(u.markdown.contains("Item"), "{}", u.file);
        assert!(u.markdown.contains(&format!("item number {a} ")));
        assert!(u.markdown.contains(&format!("item number {b} ")));
    }
    assert_eq!(next, 2_002);
}

#[test]
fn outline_chapters_group_pages() {
    // Five chapters of 25,000 characters each: each fits the part size of 10,000 tokens
    // alone, no two fit together, so packing never joins two chapters.
    let tops = [
        (1, "Intro"),
        (4, "Method"),
        (9, "Results"),
        (20, "Discussion"),
    ];
    let starts = [1u32, 4, 9, 20, 31, 41];
    let mut d = doc(FormatId::Pdf);
    for k in 0..5 {
        let (from, to) = (starts[k], starts[k + 1]);
        let per_page = 25_000 / (to - from) as usize;
        for n in from..to {
            d.body.push(page(n, vec![para(&words(per_page))]));
        }
    }
    for (p, t) in tops {
        d.facets.outline.push(OutlineEntry {
            level: 1,
            title: t.into(),
            page: Some(p),
        });
        // Nested entries are no chapter borders.
        d.facets.outline.push(OutlineEntry {
            level: 2,
            title: format!("{t} detail"),
            page: Some(p + 1),
        });
    }
    // The outline need not be in page order.
    d.facets.outline.insert(
        0,
        OutlineEntry {
            level: 1,
            title: "Appendix".into(),
            page: Some(31),
        },
    );
    let list = units(&d, 10_000);
    assert_within(&list, 10_000);
    assert_eq!(
        names(&list),
        ["intro", "method", "results", "discussion", "appendix"]
    );
    let locations: Vec<&str> = list.iter().map(|u| u.location.as_str()).collect();
    assert_eq!(
        locations,
        [
            "pages 1 to 3",
            "pages 4 to 8",
            "pages 9 to 19",
            "pages 20 to 30",
            "pages 31 to 40"
        ]
    );
    assert_eq!(list[2].title, "Results");
}

#[test]
fn pages_without_outline_pack() {
    // Pages of 3,000 characters: two fit a part of 2,500 tokens, three do not.
    let mut d = doc(FormatId::Pdf);
    for n in 1..=30 {
        d.body.push(page(n, vec![para(&words(3_000))]));
    }
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    let expected: Vec<String> = (0..15)
        .map(|k| format!("pages-{}-{}", 2 * k + 1, 2 * k + 2))
        .collect();
    assert_eq!(names(&list), expected);
    assert_eq!(list[0].location, "pages 1 to 2");
    assert_eq!(list[0].title, "pages 1 to 2");
}

#[test]
fn headings_then_blocks() {
    // Every chunk of about 5,000 characters fits a part of 2,500 tokens alone but not with
    // its neighbour. "Two" holds 12 paragraphs of 1,500 characters: 4 fit, 5 do not.
    let mut d = doc(FormatId::Markdown);
    d.body.push(para(&words(5_000)));
    d.body.push(heading(2, "Alpha"));
    d.body.push(para(&words(5_000)));
    d.body.push(heading(2, "Beta"));
    d.body.push(heading(3, "One"));
    d.body.push(para(&words(5_000)));
    d.body.push(heading(3, "Two"));
    for _ in 0..12 {
        d.body.push(para(&words(1_500)));
    }
    d.body.push(heading(2, "Gamma"));
    d.body.push(para(&words(5_000)));
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    assert_eq!(
        names(&list),
        ["start", "alpha", "beta", "two", "two-2", "two-3", "gamma"]
    );
    assert!(list[2].markdown.starts_with("## Beta"));
    assert!(list[2].markdown.contains("### One"));
    assert!(list[3].markdown.starts_with("### Two"));
    assert_eq!(list[3].location, "Two");
    assert_eq!(list[4].title, "Two, part 2");
}

/// The fence lines of `md` (openers and closers).
fn fence_lines(md: &str) -> usize {
    md.lines().filter(|l| l.starts_with("```")).count()
}

#[test]
fn oversized_units_are_cut_within_limit() {
    let mut d = doc(FormatId::Markdown);
    d.body.push(heading(2, "Paragraph"));
    d.body.push(para(&words(60_000)));
    d.body.push(heading(2, "Fence"));
    let code: String = (0..625)
        .map(|i| format!("let x{i:04} = {};\n", words(60)))
        .collect();
    d.body.push(Block::Code {
        lang: Some("rust".into()),
        text: code,
        anchor: None,
    });
    d.body.push(heading(2, "Table"));
    d.body.push(Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows: vec![
            vec![
                Cell::new(vec![Inline::plain("Key")]),
                Cell::new(vec![Inline::plain("Text")]),
            ],
            vec![
                Cell::new(vec![Inline::plain("k1")]),
                Cell::new(vec![Inline::plain(words(45_000))]),
            ],
        ],
        alignments: Vec::new(),
        anchor: None,
        layout: None,
    });
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);

    let fenced: Vec<&Unit> = list
        .iter()
        .filter(|u| u.markdown.contains("let x"))
        .collect();
    assert!(fenced.len() > 1);
    for (i, u) in fenced.iter().enumerate() {
        // Each piece opens the fence with the same opener and closes it.
        assert!(u.markdown.contains("```rust\n"), "{}", u.markdown);
        assert_eq!(fence_lines(&u.markdown) % 2, 0, "{}", u.file);
        if i > 0 {
            assert!(u.markdown.starts_with("(continued)\n"), "{}", u.file);
        }
        if i + 1 < fenced.len() {
            assert!(u.markdown.ends_with("\n(continued)"), "{}", u.file);
        }
    }

    let tables: Vec<&Unit> = list
        .iter()
        .filter(|u| u.markdown.contains("| k1 |") || u.markdown.contains("(continued) "))
        .filter(|u| u.markdown.contains('|'))
        .collect();
    assert!(tables.len() > 1);
    for u in &tables {
        // Each piece is a valid table: the header and separator rows, then rows of the same
        // column count.
        let rows: Vec<&str> = u.markdown.lines().filter(|l| l.starts_with('|')).collect();
        assert!(
            rows[0].contains("Key") && rows[0].contains("Text"),
            "{}",
            u.file
        );
        assert!(rows[1].starts_with("| ---"), "{}", u.file);
        for row in &rows {
            assert_eq!(row.matches(" | ").count(), 1, "{row}");
        }
        assert!(u.markdown.contains("(continued)"));
    }

    let prose: Vec<&Unit> = list
        .iter()
        .filter(|u| !u.markdown.contains('|') && !u.markdown.contains("let x"))
        .collect();
    assert!(prose.len() > 1);
    // Nothing is lost: the paragraph's words are all there, in order.
    let joined: String = prose
        .iter()
        .map(|u| {
            u.markdown
                .replace("(continued)", "")
                .replace("## Paragraph", "")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let squeezed: Vec<&str> = joined.split_whitespace().collect();
    let original = words(60_000);
    let expected: Vec<&str> = original.split_whitespace().collect();
    assert_eq!(squeezed, expected);
}

#[test]
fn links_follow_their_targets_across_parts() {
    let mut d = doc(FormatId::Markdown);
    d.assets.push(Asset {
        id: "pic".into(),
        path: Some("b.assets/pic.png".into()),
        sha256: "00".into(),
        mime: "image/png".into(),
        caption: None,
        alt: Some("A picture".into()),
        needs_vision: false,
        codes: Vec::new(),
        code_unread: false,
    });
    d.body.push(heading(2, "Alpha"));
    d.body.push(para(&words(5_000)));
    d.body.push(heading(2, "Beta"));
    d.body.push(Block::Paragraph {
        inlines: vec![
            Inline::Link {
                target: "#alpha".into(),
                title: None,
                internal: true,
                wiki: false,
                inlines: vec![Inline::plain("see alpha")],
            },
            Inline::plain(" and "),
            Inline::Link {
                target: "#beta".into(),
                title: None,
                internal: true,
                wiki: false,
                inlines: vec![Inline::plain("this one")],
            },
            Inline::FootnoteRef { id: "n1".into() },
        ],
        role: None,
        anchor: None,
    });
    d.body.push(Block::Media {
        asset: "pic".into(),
        alt: None,
        anchor: None,
    });
    d.body.push(para(&words(5_000)));
    d.body.push(heading(2, "Gamma"));
    d.body.push(Block::Paragraph {
        inlines: vec![
            Inline::plain("Gamma cites"),
            Inline::FootnoteRef { id: "n1".into() },
        ],
        role: None,
        anchor: Some(Anchor("g1".into())),
    });
    d.body.push(para(&words(5_000)));
    d.body.push(Block::Section {
        kind: SectionKind::Other,
        title: None,
        anchor: Some(Anchor("fn:n1".into())),
        blocks: vec![para("The footnote text.")],
    });
    d.facets.annotations.push(Annotation {
        kind: "comment".into(),
        author: Some("Ann".into()),
        date: None,
        anchor: Some(Anchor("g1".into())),
        quoted: None,
        body: vec![Inline::plain("A note on gamma.")],
        replies: Vec::new(),
        resolved: None,
    });
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    assert_eq!(names(&list), ["alpha", "beta", "gamma"]);
    let [alpha, beta, gamma] = &list[..] else {
        panic!("three units");
    };
    // A link to another unit names its file; one inside the unit stays an anchor.
    assert!(
        beta.markdown.contains("[see alpha](alpha.md#alpha)"),
        "{}",
        beta.markdown
    );
    assert!(beta.markdown.contains("[this one](#beta)"));
    // Pictures are one folder up.
    assert!(
        beta.markdown.contains("](../b.assets/pic.png)"),
        "{}",
        beta.markdown
    );
    // The footnote definition is in each unit that cites it.
    for u in [beta, gamma] {
        assert!(
            u.markdown.contains("[^n1]: The footnote text."),
            "{}",
            u.markdown
        );
    }
    assert!(!alpha.markdown.contains("[^n1]"));
    // The annotation stands at its passage, only there.
    assert!(gamma.markdown.contains("A note on gamma."));
    assert!(!alpha.markdown.contains("A note on gamma."));
    assert!(!beta.markdown.contains("A note on gamma."));
}

#[test]
fn asset_rule_counts_import_length() {
    // 100 pictures: the written links are short, the links an import writes are longer
    // (`assets/<file path>/<name>`), and that length decides.
    let mut d = doc(FormatId::Markdown);
    for i in 0..100 {
        let id = format!("a{i:03}");
        d.assets.push(Asset {
            id: id.clone(),
            path: Some(format!("b.assets/{id}.png")),
            sha256: "00".into(),
            mime: "image/png".into(),
            caption: None,
            alt: Some("p".into()),
            needs_vision: false,
            codes: Vec::new(),
            code_unread: false,
        });
        d.body.push(Block::Media {
            asset: id,
            alt: None,
            anchor: None,
        });
    }
    let base = body(&render(&d, Completeness::Complete, &opts()))
        .chars()
        .count();
    // Fill the body to 9,600 characters, 2,400 tokens as written.
    d.body.push(para(&words(9_600 - base - 2)));
    let md = render(&d, Completeness::Complete, &opts());
    assert_eq!(body(&md).chars().count(), 9_600);
    let list = units(&d, 2_500);
    assert!(list.len() > 1);
    assert_within(&list, 2_500);
    // Without the pictures' import length the same text fits.
    let mut plain = d.clone();
    plain.assets.clear();
    plain.body.retain(|b| !matches!(b, Block::Media { .. }));
    plain.body.push(para(&words(
        9_600
            - 2
            - body(&render(&plain, Completeness::Complete, &opts()))
                .chars()
                .count(),
    )));
    assert_eq!(split(&plain, &opts(), 2_500, "b"), None);
}

#[test]
fn file_names_slug_unique_and_not_reserved() {
    // Six chapters of 5,000 characters: each fits a part of 2,500 tokens alone, no two fit
    // together, so each gets a file of its own.
    let mut d = doc(FormatId::Markdown);
    for title in ["Index", "Log", "Über", "Summary", "Summary", "!!!"] {
        d.body.push(heading(2, title));
        d.body.push(para(&words(5_000)));
    }
    let list = units(&d, 2_500);
    assert_eq!(
        names(&list),
        ["index-2", "log-2", "ber", "summary", "summary-2", "part-6"]
    );
    for u in &list {
        assert!(u.file.len() <= 60);
        assert!(
            u.file
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        );
    }
    // A long title is cut at a hyphen, within 60 bytes.
    let mut long = doc(FormatId::Markdown);
    let title = "a very long chapter title that goes on and on well past the sixty byte limit";
    long.body.push(heading(2, title));
    long.body.push(para(&words(5_000)));
    long.body.push(heading(2, "Next"));
    long.body.push(para(&words(5_000)));
    let list = units(&long, 2_500);
    assert_eq!(
        list[0].file,
        "a-very-long-chapter-title-that-goes-on-and-on-well-past-the"
    );
    assert!(list[0].file.len() <= 60);
}

#[test]
fn trailing_sections_are_units() {
    let mut d = doc(FormatId::Pdf);
    d.body.push(Block::Section {
        kind: SectionKind::Other,
        title: Some("Outline".into()),
        anchor: None,
        blocks: vec![Block::List {
            ordered: false,
            items: vec![vec![para("Intro, page 1")], vec![para("End, page 2")]],
            anchor: None,
        }],
    });
    d.body.push(page(1, vec![para(&words(6_000))]));
    d.body.push(page(2, vec![para(&words(6_000))]));
    d.properties.custom = BTreeMap::from([(
        "Project".to_string(),
        Value::Text {
            value: "Apollo".into(),
        },
    )]);
    d.facets.formulas.push(Formula {
        anchor: Anchor("field:total".into()),
        expression: "a + b".into(),
        dialect: "excel".into(),
        references: Vec::new(),
        cached: None,
        role: Some(Role::Result),
        pattern: None,
        latex: None,
        note: None,
        kind: None,
        range: None,
        original: None,
    });
    d.facets.annotations.push(Annotation {
        kind: "comment".into(),
        author: Some("Ann".into()),
        date: None,
        anchor: None,
        quoted: None,
        body: vec![Inline::plain("Loose note.")],
        replies: Vec::new(),
        resolved: None,
    });
    d.facets.timeline.push(Event {
        time: Some("2026-01-01".into()),
        actor: Some("Bob".into()),
        kind: "revision".into(),
        anchor: None,
        text: None,
    });
    d.facets.embedded.push(Embedded {
        kind: "javascript".into(),
        name: "calc".into(),
        detail: None,
        target: None,
        text: None,
    });
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    assert_eq!(
        names(&list),
        [
            "page-1",
            "page-2",
            "properties",
            "logic",
            "annotations",
            "changes",
            "embedded",
            "outline"
        ]
    );
    let by = |name: &str| list.iter().find(|u| u.file == name).expect(name);
    assert!(by("properties").markdown.contains("Apollo"));
    assert_eq!(by("logic").location, "Logic");
    assert!(by("annotations").markdown.contains("Loose note."));
    assert!(by("outline").markdown.contains("End, page 2"));
    assert!(!by("page-1").markdown.contains("End, page 2"));
}

#[test]
fn logic_is_cut_per_sheet() {
    let mut d = doc(FormatId::Xlsx);
    d.body.push(sheet("Alpha", vec![para(&words(5_000))]));
    d.body.push(sheet("Beta", vec![para(&words(5_000))]));
    // About 5,000 characters of logic per sheet: one sheet's part fits 2,500 tokens, both
    // together do not.
    for (s, n) in [("Alpha", 0), ("Beta", 1)] {
        for i in 1..=25 {
            d.facets.formulas.push(Formula {
                anchor: Anchor(format!("{s}!C{i}")),
                expression: format!("SUM({s}!A{i}:B{i}) + {}", words(60 + n)),
                dialect: "excel".into(),
                references: Vec::new(),
                cached: None,
                role: Some(Role::Result),
                pattern: None,
                latex: None,
                note: None,
                kind: None,
                range: None,
                original: None,
            });
        }
    }
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    let logic: Vec<&str> = names(&list)
        .into_iter()
        .filter(|n| n.starts_with("logic"))
        .collect();
    assert_eq!(logic, ["logic-alpha", "logic-beta"], "{:?}", names(&list));
    let beta = list.iter().find(|u| u.file == "logic-beta").expect("beta");
    assert!(beta.markdown.starts_with("### Beta"), "{}", beta.markdown);
    assert_eq!(beta.location, "Logic, Beta");
}

#[test]
fn same_document_same_units() {
    let book = workbook();
    assert_eq!(units(&book, 2_500), units(&book, 2_500));
    let mut d = doc(FormatId::Markdown);
    for i in 0..20 {
        d.body.push(heading(2, &format!("Chapter {i}")));
        d.body.push(para(&words(1_000 + 37 * i)));
    }
    let first = units(&d, 2_500);
    assert_eq!(first, units(&d, 2_500));
    assert!(first.len() > 1);
}

#[test]
fn a_word_without_spaces_is_cut_between_characters() {
    let mut d = doc(FormatId::Markdown);
    let word = "x".repeat(30_000);
    d.body.push(para(&word));
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    assert!(list.len() > 2);
    let joined: String = list
        .iter()
        .map(|u| u.markdown.replace("(continued)", "").replace('\n', ""))
        .collect();
    assert_eq!(joined, word);
}

#[test]
fn sheet_selection_applies_before_the_split() {
    let mut chosen = opts();
    chosen.sheets = Some(vec!["Big".into()]);
    let list = split(&workbook(), &chosen, 2_500, "b").expect("Big alone is over the size");
    assert!(
        list.iter().all(|u| u.file.starts_with("big-rows-")),
        "{:?}",
        names(&list)
    );
    // Sections: only the chosen ones are cut into units.
    let mut content_only = opts();
    content_only.sections = Some(vec![rezzer_render::OutputSection::Content]);
    let mut d = doc(FormatId::Markdown);
    for i in 0..4 {
        d.body.push(heading(2, &format!("Part {i}")));
        d.body.push(para(&words(5_000)));
    }
    d.properties.custom = BTreeMap::from([(
        "Project".to_string(),
        Value::Text {
            value: "Apollo".into(),
        },
    )]);
    let list = split(&d, &content_only, 2_500, "b").expect("over the size");
    assert_eq!(names(&list), ["part-0", "part-1", "part-2", "part-3"]);
}

#[test]
fn a_large_document_splits_in_bounded_time() {
    // 40,000 paragraphs under 400 headings, every tenth paragraph citing one of 100
    // footnotes, and 2,000 annotations: the cut stays fast (linear in the size).
    let mut d = doc(FormatId::Markdown);
    for h in 0..400 {
        d.body.push(heading(2, &format!("Heading {h}")));
        for p in 0..100 {
            let n = h * 100 + p;
            let mut inlines = vec![Inline::plain(words(120))];
            if n % 10 == 0 {
                inlines.push(Inline::FootnoteRef {
                    id: format!("f{}", n % 100),
                });
            }
            d.body.push(Block::Paragraph {
                inlines,
                role: None,
                anchor: Some(Anchor(format!("a{n}"))),
            });
            if n % 20 == 0 {
                d.facets.annotations.push(Annotation {
                    kind: "comment".into(),
                    author: None,
                    date: None,
                    anchor: Some(Anchor(format!("a{n}"))),
                    quoted: None,
                    body: vec![Inline::plain("note")],
                    replies: Vec::new(),
                    resolved: None,
                });
            }
        }
    }
    for f in 0..100 {
        d.body.push(Block::Section {
            kind: SectionKind::Other,
            title: None,
            anchor: Some(Anchor(format!("fn:f{f}"))),
            blocks: vec![para("A footnote.")],
        });
    }
    let started = std::time::Instant::now();
    let list = units(&d, 10_000);
    assert_within(&list, 10_000);
    assert!(list.len() >= 100);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    // Every annotation stands at its passage, in one unit only.
    let shown: usize = list
        .iter()
        .map(|u| u.markdown.matches("Comment").count())
        .sum();
    assert_eq!(shown, 2_000);
}

#[test]
fn a_trailing_section_over_the_limit_is_cut_into_numbered_parts() {
    let mut d = doc(FormatId::Markdown);
    d.body.push(para(&words(9_000)));
    d.properties.custom = (0..400)
        .map(|i| (format!("Key {i:03}"), Value::Text { value: words(40) }))
        .collect();
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    let props: Vec<&Unit> = list
        .iter()
        .filter(|u| u.file.starts_with("properties"))
        .collect();
    assert!(props.len() > 1, "{:?}", names(&list));
    assert_eq!(props[0].file, "properties");
    assert_eq!(props[0].title, "Properties");
    assert_eq!(props[1].file, "properties-2");
    assert_eq!(props[1].title, "Properties, part 2");
    // A cut table repeats its header in every piece.
    for u in &props {
        assert!(u.markdown.contains("| Key | Value |"), "{}", u.file);
    }
}

#[test]
fn long_fence_openers_stay_bounded() {
    // An info string of 20,000 letters, and code with a run of 20,000 backticks (so the
    // fence is longer still): both are cut within the limit into a few pieces.
    let lines: String = (0..300)
        .map(|i| format!("let y{i:03} = {};\n", words(40)))
        .collect();
    for (lang, code) in [
        (Some("x".repeat(20_000)), lines.clone()),
        (
            Some("rust".to_string()),
            format!("{}\n{lines}", "`".repeat(20_000)),
        ),
    ] {
        let mut d = doc(FormatId::Markdown);
        d.body.push(Block::Code {
            lang,
            text: code,
            anchor: None,
        });
        let started = std::time::Instant::now();
        let list = units(&d, 2_500);
        assert_within(&list, 2_500);
        assert!(list.len() < 20, "{} units", list.len());
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        // The code lines are all there, and the language is still named.
        let joined: String = list.iter().map(|u| u.markdown.as_str()).collect();
        assert!(joined.contains("let y000 ") && joined.contains("let y299 "));
        assert!(list[0].markdown.contains("xxxx") || list[0].markdown.contains("rust"));
    }
}

#[test]
fn a_heading_never_stands_alone() {
    // "One" alone is over the part size: the chapter heading "Beta" goes with its first
    // piece instead of making a part of its own.
    let mut d = doc(FormatId::Markdown);
    d.body.push(heading(2, "Alpha"));
    d.body.push(para(&words(5_000)));
    d.body.push(heading(2, "Beta"));
    d.body.push(heading(3, "One"));
    for _ in 0..6 {
        d.body.push(para(&words(1_500)));
    }
    d.body.push(heading(3, "Two"));
    d.body.push(para(&words(5_000)));
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    for u in &list {
        let content = u
            .markdown
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'));
        assert!(content.count() > 0, "{} holds only headings", u.file);
    }
    let first_of_one = list
        .iter()
        .find(|u| u.markdown.contains("### One"))
        .expect("One");
    assert!(
        first_of_one.markdown.starts_with("## Beta"),
        "{}",
        first_of_one.markdown
    );
}

#[test]
fn packed_logic_parts_name_their_range() {
    let mut d = doc(FormatId::Xlsx);
    d.body.push(sheet("Alpha", vec![para(&words(5_000))]));
    // Three sheets of small logic and one large: the small ones pack into one part.
    for (s, count) in [("Aa", 3), ("Bb", 3), ("Cc", 3), ("Dd", 60)] {
        d.body.push(sheet(s, vec![para("x")]));
        for i in 1..=count {
            d.facets.formulas.push(Formula {
                anchor: Anchor(format!("{s}!C{i}")),
                expression: format!("SUM({s}!A{i}:B{i}) + {}", words(60)),
                dialect: "excel".into(),
                references: Vec::new(),
                cached: None,
                role: Some(Role::Result),
                pattern: None,
                latex: None,
                note: None,
                kind: None,
                range: None,
                original: None,
            });
        }
    }
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    let first = list
        .iter()
        .find(|u| u.file.starts_with("logic"))
        .expect("logic");
    assert_eq!(first.file, "logic-aa", "{:?}", names(&list));
    assert_eq!(first.location, "Logic, Aa to Cc");
}

/// A document as the Markdown reader makes it.
fn read_markdown(text: &str) -> Document {
    let src = Source {
        path: None,
        name: "note.md".into(),
        bytes: Bytes::Owned(Arc::new(text.as_bytes().to_vec())),
        sha256: "00".repeat(32),
        size: text.len() as u64,
        format: FormatId::Markdown,
        detection: Detection {
            confidence: Confidence::Likely,
            evidence: "md".into(),
        },
        modified: None,
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
    };
    MarkdownReader
        .read(
            &src,
            &ReadOptions::default(),
            &Budget::new(Limits::default()),
        )
        .document
}

#[test]
fn footnotes_from_the_markdown_reader_follow_their_citations() {
    // The reader puts every definition into one `Footnotes` section at the end.
    let text = format!(
        "## Alpha\n\n{}\n\nAlpha cites[^a].\n\n## Beta\n\n{}\n\nBeta cites[^a] and[^b].\n\n\
         [^a]: Note A.\n\n[^b]: Note B cites[^c].\n\n[^c]: Note C.\n",
        words(5_000),
        words(5_000)
    );
    let d = read_markdown(&text);
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    assert_eq!(names(&list), ["alpha", "beta"]);
    assert!(
        list[0].markdown.contains("[^a]: Note A."),
        "{}",
        list[0].markdown
    );
    assert!(!list[0].markdown.contains("Note B"));
    for note in ["[^a]: Note A.", "[^b]: Note B cites[^c]", "[^c]: Note C."] {
        assert!(
            list[1].markdown.contains(note),
            "{note}: {}",
            list[1].markdown
        );
    }
    // A definition nothing cites stays in a footnotes part; the cited ones are not repeated
    // there.
    let d = read_markdown(&format!("{text}\n[^z]: Lonely.\n"));
    let list = units(&d, 2_500);
    let lonely = list
        .iter()
        .find(|u| u.markdown.contains("Lonely."))
        .expect("the uncited note");
    assert!(!lonely.markdown.contains("Note A."), "{}", lonely.markdown);
}

/// A sheet table with data rows 2 to `last`; row `big` holds a cell of 45,000 characters.
fn sheet_table_with_big_row(sheet: &str, last: u32, big: Option<u32>) -> Block {
    let mut t = sheet_table(sheet, last);
    if let (Some(big), Block::Table { rows, .. }) = (big, &mut t) {
        rows[(big - 1) as usize][0] = cell(&words(45_000), &format!("{sheet}!A{big}"));
    }
    t
}

#[test]
fn one_huge_row_leaves_the_other_ranges_as_they_were() {
    let count = |big: Option<u32>| {
        let mut d = doc(FormatId::Xlsx);
        d.body.push(sheet(
            "Data",
            vec![sheet_table_with_big_row("Data", 2_001, big)],
        ));
        let list = units(&d, 2_500);
        assert_within(&list, 2_500);
        list.len()
    };
    let normal = count(None);
    let with_big = count(Some(1_000));
    // The big row is cut into about 6 pieces of its own; the other ranges keep their size.
    assert!(
        with_big <= normal * 12 / 10 + 8,
        "{with_big} units with the big row, {normal} without"
    );
}

#[test]
fn part_names_stay_within_60_bytes() {
    let title = "A sheet with a very long name that goes on and on for many words";
    let key = "Sales_figures_for_every_region_and_every_quarter_of_the_whole_year_and_more";
    let mut d = doc(FormatId::Xlsx);
    let mut tables = Vec::new();
    for k in 0..2 {
        let mut t = sheet_table(title, 2_001);
        if let Block::Table {
            layout: Some(layout),
            ..
        } = &mut t
        {
            layout.key = format!("table:{key}{k}");
        }
        tables.push(t);
    }
    d.body.push(sheet(title, tables));
    let list = units(&d, 2_500);
    let mut seen = std::collections::HashSet::new();
    for u in &list {
        assert!(u.file.len() <= 60, "{} is {} bytes", u.file, u.file.len());
        assert!(seen.insert(u.file.clone()), "{} twice", u.file);
        assert!(
            !u.file.starts_with('-') && !u.file.ends_with('-'),
            "{}",
            u.file
        );
    }
    assert!(list.len() > 4);
}

#[test]
fn math_blocks_are_closed_and_opened_again() {
    let latex: String = (0..2_000)
        .map(|i| format!("a_{{{i}}} + b_{{{i}}} = c \\\\\n"))
        .collect();
    let mut d = doc(FormatId::Markdown);
    d.body.push(Block::Math {
        latex,
        source_kind: "latex".into(),
        anchor: None,
    });
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    assert!(list.len() > 1);
    for u in &list {
        let mut open = false;
        for line in u.markdown.lines() {
            if line == "$$" {
                open = !open;
            } else if line == "(continued)" {
                assert!(!open, "{}: a continued line inside the math", u.file);
            }
        }
        assert!(!open, "{}: math left open", u.file);
        assert!(u.markdown.contains("$$"), "{}", u.file);
    }
}

#[test]
fn a_repeated_heading_is_linked_by_its_slug_in_its_part() {
    let text = format!(
        "## Summary\n\n{}\n\nSee [the later one](#summary-1).\n\n## Summary\n\n{}\n",
        words(5_000),
        words(5_000)
    );
    let list = units(&read_markdown(&text), 2_500);
    // The reader's link graph adds a Logic part after them.
    assert_eq!(names(&list)[..2], ["summary", "summary-2"]);
    assert!(
        list[0]
            .markdown
            .contains("[the later one](summary-2.md#summary)"),
        "{}",
        list[0].markdown
    );
}

#[test]
fn links_in_trailing_sections_follow_their_targets() {
    let mut d = doc(FormatId::Markdown);
    d.body.push(heading(2, "Alpha"));
    d.body.push(para(&words(5_000)));
    d.body.push(heading(2, "Beta"));
    d.body.push(para(&words(5_000)));
    d.facets.annotations.push(Annotation {
        kind: "comment".into(),
        author: None,
        date: None,
        anchor: None,
        quoted: None,
        body: vec![Inline::Link {
            target: "#alpha".into(),
            title: None,
            internal: true,
            wiki: false,
            inlines: vec![Inline::plain("see alpha")],
        }],
        replies: Vec::new(),
        resolved: None,
    });
    let list = units(&d, 2_500);
    let notes = list
        .iter()
        .find(|u| u.file == "annotations")
        .expect("annotations");
    assert!(
        notes.markdown.contains("](alpha.md#alpha)"),
        "{}",
        notes.markdown
    );
}

#[test]
fn a_section_inside_a_section_is_cut_at_its_headings() {
    let mut inner = Vec::new();
    for i in 0..6 {
        inner.push(heading(3, &format!("Part {i}")));
        inner.push(para(&words(4_000)));
    }
    let mut d = doc(FormatId::Markdown);
    d.body.push(Block::Section {
        kind: SectionKind::Chapter,
        title: Some("Outer".into()),
        anchor: None,
        blocks: vec![Block::Section {
            kind: SectionKind::Chapter,
            title: Some("Inner".into()),
            anchor: None,
            blocks: inner,
        }],
    });
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    // The parts are named by the inner section's headings, two to a part.
    assert_eq!(names(&list), ["part-0", "part-2", "part-4"]);
    for u in &list {
        assert!(
            !u.markdown.contains("(continued)"),
            "{} was cut as text",
            u.file
        );
    }
}

#[test]
fn a_long_prefix_is_not_repeated_on_every_piece() {
    // One code line of 2,000 (repeated: under a quarter of the room) or 4,000 spaces (not
    // repeated) and about 10 MB of text: the output stays within twice the input and the
    // cut stays fast.
    for spaces in [2_000, 4_000] {
        let text = format!("{}{}\n", " ".repeat(spaces), words(10_000_000));
        let mut d = doc(FormatId::Markdown);
        d.body.push(Block::Code {
            lang: Some("text".into()),
            text,
            anchor: None,
        });
        let started = std::time::Instant::now();
        let list = units(&d, 2_500);
        assert_within(&list, 2_500);
        let out: usize = list.iter().map(|u| u.markdown.len()).sum();
        assert!(
            out < 20_000_000,
            "{out} bytes of output for {spaces} spaces"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "{:?}",
            started.elapsed()
        );
        if spaces == 4_000 {
            assert!(
                !list[1].markdown.contains(&" ".repeat(1_000)),
                "the prefix is repeated"
            );
        }
    }
}

#[test]
fn a_table_without_layout_is_cut_again_with_its_own_row_numbers() {
    // Escaped cells take twice the characters the row estimate counts, so the first ranges
    // are over the size and are cut again; their numbers stay the table's own rows.
    let mut rows = vec![vec![
        Cell::new(vec![Inline::plain("Key")]),
        Cell::new(vec![Inline::plain("Stars")]),
    ]];
    for i in 0..40 {
        rows.push(vec![
            Cell::new(vec![Inline::plain(format!("k{i}"))]),
            Cell::new(vec![Inline::plain("*".repeat(600))]),
        ]);
    }
    let mut d = doc(FormatId::Markdown);
    d.body.push(Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: None,
    });
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    let mut next = 2;
    for u in &list {
        let range = u.file.strip_prefix("start-rows-").expect("a row range");
        let (a, b) = range.split_once('-').expect("two numbers");
        let (a, b): (u32, u32) = (a.parse().expect("first"), b.parse().expect("last"));
        assert_eq!(a, next, "{:?}", names(&list));
        next = b + 1;
        assert!(
            u.markdown.contains(&format!("| k{} |", a - 2)),
            "{}",
            u.file
        );
    }
    assert_eq!(next, 42);
    assert!(list.len() >= 4, "{:?}", names(&list));
}

// ---------------------------------------------------------------------------------------
// The main document and the part files (plan 5 Task 6, spec 5 and 7, decisions 10 and 13).

use rezzer_model::{FieldAction, FormField, Loss, LossReason, OverviewPart, Severity, Signature};
use rezzer_render::{Output, PartFile, render_output};

/// Ruling P5-AF: a source chapter named `Contents` keeps its name (Rezzer's own list parts
/// are `List of parts`), and its pieces are numbered as one unit.
#[test]
fn a_source_contents_chapter_keeps_its_name() {
    let mut d = doc(FormatId::Markdown);
    d.body.push(heading(1, "Contents"));
    d.body.push(para(&words(30_000)));
    d.body.push(heading(1, "Chapter 1"));
    d.body.push(para(&words(8_000)));
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    let all = titles(&list);
    assert_eq!(&all[..2], ["Contents", "Contents, part 2"], "{all:?}");
    assert!(all.contains(&"Chapter 1"), "{all:?}");
}

/// Ruling P5-AF: units that share a title get their location in words.
#[test]
fn chapters_sharing_a_title_get_their_pages() {
    let mut d = doc(FormatId::Pdf);
    for n in 1..=4 {
        d.body.push(page(n, vec![para(&words(4_000))]));
    }
    for p in [1, 3] {
        d.facets.outline.push(OutlineEntry {
            level: 1,
            title: "Notes".into(),
            page: Some(p),
        });
    }
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    assert_eq!(
        titles(&list),
        ["Notes (pages 1 to 2)", "Notes (pages 3 to 4)"]
    );
}

/// Ruling P5-AF: units that share title and location get ` (2)`, not `, part 2`.
#[test]
fn units_at_one_place_get_a_number() {
    let mut d = doc(FormatId::Markdown);
    for _ in 0..2 {
        d.body.push(heading(1, "Notes"));
        d.body.push(para(&words(8_000)));
    }
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    assert_eq!(titles(&list), ["Notes", "Notes (2)"]);
}

/// No title is used twice, also not in another letter case or as a piece title.
#[test]
fn titles_stay_unique_against_own_parts_and_pieces() {
    let mut d = doc(FormatId::Markdown);
    for (title, chars) in [
        ("Logic", 8_000),
        ("Abstract", 30_000),
        ("abstract, part 2", 8_000),
    ] {
        d.body.push(heading(1, title));
        d.body.push(para(&words(chars)));
    }
    let list = units(&d, 2_500);
    assert_within(&list, 2_500);
    let all = titles(&list);
    // The document has no Logic part, so the source chapter keeps its title.
    assert_eq!(all[0], "Logic", "{all:?}");
    let unique: std::collections::HashSet<_> = all.iter().map(|t| t.to_lowercase()).collect();
    assert_eq!(unique.len(), all.len(), "{all:?}");
}

/// Ruling P5-AF: a source chapter `Overview` gets a word only when the main document writes
/// its own `Overview` part.
#[test]
fn a_source_overview_chapter_gets_a_word_only_beside_an_overview_part() {
    let mut d = doc(FormatId::Markdown);
    for title in ["Overview", "Chapter 2", "Chapter 3"] {
        d.body.push(heading(1, title));
        d.body.push(para(&words(6_000)));
    }
    d.overview = Some(rezzer_model::Overview {
        parts: (1..=30)
            .map(|i| OverviewPart {
                id: format!("p{i}"),
                name: format!("Part {i}"),
                kind: "chapter".into(),
                size: "1 paragraph".into(),
                role: None,
                reads: Vec::new(),
            })
            .collect(),
        facts: Vec::new(),
    });
    let out = output(&d, 2_500, "report.md.md", "b");
    let all: Vec<&str> = out.parts.iter().map(|p| p.title.as_str()).collect();
    assert!(all.contains(&"report.md: Overview (section)"), "{all:?}");
    assert!(all.contains(&"report.md: Overview"), "{all:?}");
    let unique: std::collections::HashSet<_> = all.iter().map(|t| t.to_lowercase()).collect();
    assert_eq!(unique.len(), all.len(), "{all:?}");
    // With the whole overview in the main document there is no Overview part.
    d.overview.as_mut().expect("overview").parts.truncate(5);
    let out = output(&d, 2_500, "report.md.md", "b");
    let all: Vec<&str> = out.parts.iter().map(|p| p.title.as_str()).collect();
    assert!(all.contains(&"report.md: Overview"), "{all:?}");
    assert!(!all.iter().any(|t| t.contains("(section)")), "{all:?}");
}

/// Ruling P5-AF: a lone source chapter titled like one of Rezzer's trailing parts keeps its
/// title when the document has no such part, and gets a word when it has one.
#[test]
fn a_source_properties_chapter_gets_a_word_only_beside_a_properties_part() {
    let mut d = doc(FormatId::Markdown);
    for title in ["Properties", "Chapter 2"] {
        d.body.push(heading(1, title));
        d.body.push(para(&words(8_000)));
    }
    assert_eq!(titles(&units(&d, 2_500)), ["Properties", "Chapter 2"]);
    d.properties.custom = BTreeMap::from([(
        "Project".to_string(),
        Value::Text {
            value: "Apollo".into(),
        },
    )]);
    assert_eq!(
        titles(&units(&d, 2_500)),
        ["Properties (section)", "Chapter 2", "Properties"]
    );
}

/// Ruling P5-AD with the Markdown reader: a wikilink in the text becomes escaped text, one in
/// a code span or a fence stays as it is.
#[test]
fn wikilinks_in_code_stay_and_in_text_are_escaped() {
    let d = read_markdown("# T\n\nA `[[x]]` b.\n\n```\n[[y]]\n```\n\nSee [[z]] and [[p_q|R]].\n");
    let md = render(&d, Completeness::Complete, &opts());
    assert!(md.contains("A `[[x]]` b."), "{md}");
    assert!(md.contains("\n[[y]]\n"), "{md}");
    assert!(md.contains(r"See \[\[z\]\] and \[\[p\_q\|R\]\]."), "{md}");
}

fn titles(list: &[Unit]) -> Vec<&str> {
    list.iter().map(|u| u.title.as_str()).collect()
}

fn output(d: &Document, part_size: usize, main: &str, bundle: &str) -> Output {
    render_output(d, Completeness::Complete, &opts(), part_size, main, bundle)
}

/// A Markdown document with `n` chapters of `chars` characters each, titled `Chapter <i>`.
fn chapters_doc(n: usize, chars: usize) -> Document {
    let mut d = doc(FormatId::Markdown);
    for i in 1..=n {
        d.body.push(heading(1, &format!("Chapter {i}")));
        d.body.push(para(&words(chars)));
    }
    d
}

/// The lines of the section `## <title>` in `md`, without the heading and blank lines.
fn section_lines<'a>(md: &'a str, title: &str) -> Vec<&'a str> {
    let head = format!("\n## {title}\n");
    let Some(at) = md.find(&head) else {
        return Vec::new();
    };
    let rest = &md[at + head.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    rest[..end]
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect()
}

/// The value of a top-level frontmatter key, as written.
fn key<'a>(md: &'a str, name: &str) -> Option<&'a str> {
    let front = md.strip_prefix("---\n")?;
    let front = &front[..front.find("\n---\n")?];
    front
        .lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(": "))
}

/// Every written file within the part size: the body after the frontmatter, characters / 4,
/// as Crystalline's Q002 counts it.
fn assert_files_within(out: &Output, part_size: usize) {
    let tokens = body(&out.main).chars().count() / 4;
    assert!(tokens <= part_size, "main has {tokens} tokens");
    for p in &out.parts {
        let tokens = body(&p.markdown).chars().count() / 4;
        assert!(tokens <= part_size, "{} has {tokens} tokens", p.path);
    }
}

/// The part files in the order the reader finds them: the main document's `## Parts` list,
/// or the lists of the `contents` parts it leads to, then the `contents` parts themselves.
fn listed_order(out: &Output, bundle: &str) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    // The destination as written: in angle brackets up to the `>`, else up to the `)`.
    let link_target = |line: &str| -> Option<String> {
        let at = line.find("](")?;
        let rest = &line[at + 2..];
        match rest.strip_prefix('<') {
            Some(inner) => Some(inner[..inner.find('>')?].to_string()),
            None => Some(rest[..rest.find(')')?].to_string()),
        }
    };
    let lines = section_lines(&out.main, "Parts");
    let contents: Vec<&PartFile> = out
        .parts
        .iter()
        .filter(|p| {
            let file = p.path.rsplit('/').next().unwrap_or("");
            file == "contents.md" || file.starts_with("contents-")
        })
        .collect();
    if contents.is_empty() {
        for line in lines {
            order.push(link_target(line).expect("a link"));
        }
        return order;
    }
    for c in &contents {
        for line in body(&c.markdown).lines().filter(|l| l.starts_with("- [")) {
            order.push(format!("{bundle}/{}", link_target(line).expect("a link")));
        }
    }
    order.extend(contents.iter().map(|c| c.path.clone()));
    order
}

#[test]
fn no_split_gives_the_whole_render() {
    let d = chapters_doc(2, 100);
    let out = output(&d, 10_000, "report.md.md", "report-md");
    assert_eq!(out.main, render(&d, Completeness::Complete, &opts()));
    assert!(out.parts.is_empty());
    assert!(
        output(&chapters_doc(40, 6_000), 0, "r.md", "r")
            .parts
            .is_empty()
    );
}

#[test]
fn main_document_lists_parts_in_order() {
    let d = chapters_doc(6, 6_000);
    let out = output(&d, 2_500, "report.md.md", "report-md");
    assert_files_within(&out, 2_500);
    let files: Vec<&str> = out.parts.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(
        files,
        [
            "report-md/chapter-1.md",
            "report-md/chapter-2.md",
            "report-md/chapter-3.md",
            "report-md/chapter-4.md",
            "report-md/chapter-5.md",
            "report-md/chapter-6.md",
        ]
    );
    assert_eq!(key(&out.main, "parts"), Some("6"));
    assert!(body(&out.main).starts_with("\n# report.md\n\n"));
    let lines = section_lines(&out.main, "Parts");
    let want: Vec<String> = (1..=6)
        .map(|i| format!("- [report.md: Chapter {i}](report-md/chapter-{i}.md): Chapter {i}"))
        .collect();
    assert_eq!(lines, want);
    for (i, p) in out.parts.iter().enumerate() {
        assert_eq!(p.title, format!("report.md: Chapter {}", i + 1));
        assert_eq!(p.location, format!("Chapter {}", i + 1));
    }
    // The main document holds no content of its own beyond the extracts.
    assert!(!out.main.contains("lorem ipsum"));
}

fn field(name: &str, calc: bool) -> FormField {
    FormField {
        anchor: Anchor(format!("field:{name}")),
        name: name.into(),
        label: None,
        kind: "text".into(),
        value: None,
        role: None,
        format: None,
        calc_order: None,
        actions: if calc {
            vec![FieldAction {
                trigger: "calculate".into(),
                script: "AFSimple_Calculate(\"SUM\", [\"a\"]);".into(),
                words: None,
            }]
        } else {
            Vec::new()
        },
        flags: Vec::new(),
        options: Vec::new(),
        pages: Vec::new(),
    }
}

fn formula(at: &str) -> Formula {
    Formula {
        anchor: Anchor(at.into()),
        expression: "A1 + 1".into(),
        dialect: "excel".into(),
        references: Vec::new(),
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

fn loss(what: &str) -> Loss {
    Loss {
        location: "p1".into(),
        what: what.into(),
        why: LossReason::Unsupported,
        severity: Severity::Warning,
    }
}

/// A document with every count of the stats line set to `n` (formulas to `formulas`), and
/// enough text to be split at 2,500 tokens.
fn counted_doc(n: usize, formulas: usize) -> Document {
    let mut d = doc(FormatId::Pdf);
    d.properties.facts.pages = Some(n as u32);
    d.properties.facts.revisions = Some(n as u32);
    for i in 1..=n {
        let name = format!("S{i}");
        let mut table = sheet_table(&name, 4);
        if let Block::Table { layout, .. } = &mut table {
            layout.as_mut().expect("layout").key = format!("table:T{i}");
        }
        d.body.push(sheet(&name, vec![para(&words(12_000)), table]));
        d.facets.fields.push(field(&format!("f{i}"), false));
        d.facets.signatures.push(Signature {
            anchor: None,
            kind: "field".into(),
            signer: Some("Ann".into()),
            date: None,
            reason: None,
            location: None,
            sub_filter: None,
            covers_whole_file: true,
            later_revisions: 0,
        });
        d.assets.push(Asset {
            id: format!("a{i}"),
            path: Some(format!("b.assets/pic{i}.png")),
            sha256: "00".into(),
            mime: "image/png".into(),
            caption: None,
            alt: None,
            needs_vision: true,
            codes: Vec::new(),
            code_unread: false,
        });
        d.losses.push(loss(&format!("thing {i}")));
    }
    for i in 1..=formulas {
        d.facets.formulas.push(formula(&format!("S1!C{i}")));
    }
    d
}

#[test]
fn stats_line_names_each_count() {
    let out = output(&counted_doc(2, 2), 2_500, "report.pdf.md", "report-pdf");
    assert_files_within(&out, 2_500);
    assert_eq!(
        section_lines(&out.main, "Stats"),
        [
            "2 pages, 2 sheets, 2 tables, 2 formulas, 2 form fields, 2 signatures, \
             2 revisions, 2 pictures that need vision, 2 parts not read."
        ]
    );
    let out = output(&counted_doc(2, 0), 2_500, "report.pdf.md", "report-pdf");
    let stats = section_lines(&out.main, "Stats").join("\n");
    assert!(!stats.contains("formula"), "{stats}");
    assert!(stats.contains("2 form fields"), "{stats}");
    // One of each is singular.
    let out = output(&counted_doc(1, 1), 2_500, "report.pdf.md", "report-pdf");
    assert_eq!(
        section_lines(&out.main, "Stats"),
        [
            "1 page, 1 sheet, 1 table, 1 formula, 1 form field, 1 signature, 1 revision, \
             1 picture that needs vision, 1 part not read."
        ]
    );
    // The form line and the tables list come from the same facts.
    assert_eq!(
        section_lines(&out.main, "Form"),
        ["1 form field, 1 signature."]
    );
    assert_eq!(section_lines(&out.main, "Tables"), ["- table:T1 S1!A1:B4"]);
}

#[test]
fn form_and_dates_and_people() {
    let mut d = counted_doc(3, 0);
    d.facets.fields[0] = field("total", true);
    d.facets.fields[1].calc_order = Some(1);
    d.properties.authors = vec!["Ann".into(), "Bob".into()];
    d.properties.created = Some(Value::Date {
        value: "2020-01-02".into(),
    });
    d.properties.modified = Some(Value::Date {
        value: "2021-03-04".into(),
    });
    let out = output(&d, 2_500, "report.pdf.md", "report-pdf");
    assert_eq!(
        section_lines(&out.main, "Form"),
        ["3 form fields, 2 calculated, 3 signatures."]
    );
    assert_eq!(
        section_lines(&out.main, "Dates and people"),
        [
            "- Authors: Ann, Bob",
            "- Created: 2020-01-02",
            "- Modified: 2021-03-04"
        ]
    );
    // Without fields and properties the sections are left out.
    let out = output(&chapters_doc(6, 6_000), 2_500, "r.md", "r");
    assert!(!out.main.contains("## Form"));
    assert!(!out.main.contains("## Dates and people"));
    assert!(!out.main.contains("## Stats"), "no count, no stats");
}

#[test]
fn main_document_within_limit_with_many_parts() {
    let d = chapters_doc(2_000, 5_000);
    let out = output(&d, 2_500, "report.md.md", "report-md");
    assert_files_within(&out, 2_500);
    let contents: Vec<&PartFile> = out
        .parts
        .iter()
        .filter(|p| p.path.starts_with("report-md/contents"))
        .collect();
    assert!(contents.len() > 1, "the list needs more than one part");
    assert_eq!(contents[0].path, "report-md/contents.md");
    assert_eq!(contents[1].path, "report-md/contents-2.md");
    assert_eq!(contents[0].title, "report.md: List of parts");
    assert_eq!(contents[1].title, "report.md: List of parts, part 2");
    assert_eq!(contents[0].location, "List of parts");
    // The main document links the first one and lists none of the parts itself.
    let parts = section_lines(&out.main, "Parts");
    assert_eq!(
        parts,
        ["The list of all 2000 parts starts in [List of parts](report-md/contents.md)."]
    );
    // Each contents part but the last leads on to the next one.
    for (k, c) in contents.iter().enumerate() {
        let goes_on = format!(
            "The list goes on in [List of parts, part {}](contents-{}.md).",
            k + 2,
            k + 2
        );
        assert_eq!(
            c.markdown.contains(&goes_on),
            k + 1 < contents.len(),
            "{}",
            c.path
        );
    }
    assert_eq!(key(&out.main, "parts"), Some(&*out.parts.len().to_string()));
    // The lists name every other part once, in the order of the follows chain.
    let order = listed_order(&out, "report-md");
    let paths: Vec<String> = out
        .parts
        .iter()
        .map(|p| format!("report-md/{}", p.path.rsplit('/').next().unwrap_or("")))
        .collect();
    assert_eq!(order, paths);
    assert_eq!(out.parts.len(), 2_000 + contents.len());
    // Each part follows the one before it in that order.
    for pair in out.parts.windows(2) {
        let before = pair[0].path.rsplit('/').next().expect("file");
        assert_eq!(
            relations(&pair[1].markdown)[1],
            ("follows".to_string(), before.to_string())
        );
    }
}

#[test]
fn chosen_sections_shape_the_main_document() {
    let mut d = chapters_doc(6, 6_000);
    d.losses.push(loss("thing"));
    d.overview = Some(rezzer_model::Overview {
        parts: Vec::new(),
        facts: vec![("Chapters".into(), "6".into())],
    });
    let mut o = opts();
    o.sections = Some(vec![rezzer_render::OutputSection::Content]);
    let out = render_output(&d, Completeness::Partial, &o, 2_500, "r.md", "r");
    assert!(!out.main.contains("## Overview"));
    assert!(!out.main.contains("## Not read"));
    assert!(out.main.contains("## Parts"));
    assert!(out.parts.iter().all(|p| !p.path.contains("not-read")));
    assert_eq!(key(&out.main, "completeness"), Some("partial"));
    assert_eq!(key(&out.parts[0].markdown, "completeness"), Some("partial"));
    let all = output(&d, 2_500, "r.md", "r");
    assert_eq!(section_lines(&all.main, "Overview"), ["- Chapters: 6"]);
    assert_eq!(section_lines(&all.main, "Not read").len(), 3);
}

#[test]
fn part_frontmatter_and_relations() {
    let d = chapters_doc(5, 6_000);
    let out = output(&d, 2_500, "report.md.md", "report-md");
    assert_eq!(out.parts.len(), 5);
    let third = &out.parts[2].markdown;
    let front = &third[..third.find("\n---\n").expect("frontmatter end")];
    let at = |k: &str| front.find(&format!("\n{k}: ")).expect(k);
    assert!(at("parts") < at("part") && at("part") < at("location"));
    assert_eq!(key(third, "parts"), Some("5"));
    assert_eq!(key(third, "part"), Some("3"));
    assert_eq!(key(third, "location"), Some("\"Chapter 3\""));
    assert_eq!(key(third, "title"), Some("\"report.md: Chapter 3\""));
    assert_eq!(
        key(third, "description"),
        Some("\"Part 3 of 5 of \\\"report.md\\\": Chapter 3.\"")
    );
    assert_eq!(key(third, "type"), Some("source"));
    assert_eq!(key(third, "status"), Some("stable"));
    assert!(body(third).starts_with("\n# report.md: Chapter 3\n"));
    assert!(
        third.ends_with(
            "\n## Relations\n\n- part_of [report.md](../report.md.md)\n\
             - follows [report.md: Chapter 2](chapter-2.md)\n"
        ),
        "{}",
        &third[third.len() - 200..]
    );
    let first = &out.parts[0].markdown;
    assert!(first.ends_with("\n## Relations\n\n- part_of [report.md](../report.md.md)\n"));
    assert!(!first.contains("- follows"));
    // The main document is no part.
    assert_eq!(key(&out.main, "part"), None);
    assert_eq!(key(&out.main, "location"), None);
}

// A reader of relation lines that follows what Crystalline 0.29.0 reads (`parse.rs`:
// `scan_body`, `bullet_end`, `split_category`, `parse_relation`; `reflection/links.rs`:
// `read_destination`, `code_spans`). Written from those facts, no code taken.

#[derive(Debug, PartialEq)]
enum Read {
    Relation(String, String),
    Observation(String),
}

fn fence_marker(line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let first = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let count = rest.chars().take_while(|c| *c == first).count();
    (count >= 3).then_some((first, count))
}

fn is_heading(line: &str) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    indent <= 3
        && (1..=6).contains(&hashes)
        && rest[hashes..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ' || c == '\t')
}

fn starts_list_item(t: &str) -> bool {
    let gap = |r: &str| r.is_empty() || r.starts_with([' ', '\t']);
    if let Some(r) = t.strip_prefix(['-', '*', '+']) {
        return gap(r);
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    (1..=9).contains(&digits)
        && t[..digits].parse::<u32>() == Ok(1)
        && t[digits..].strip_prefix(['.', ')']).is_some_and(gap)
}

fn continues_bullet(line: &str) -> bool {
    let t = line.trim();
    // Three or more of one of `-`, `*` or `_`, with only spaces and tabs between.
    let thematic = t.chars().next().is_some_and(|m| {
        matches!(m, '-' | '*' | '_')
            && t.chars().all(|c| c == m || c == ' ' || c == '\t')
            && t.chars().filter(|c| *c == m).count() >= 3
    });
    !(t.is_empty()
        || is_heading(line)
        || fence_marker(t).is_some()
        || starts_list_item(t)
        || t.starts_with(['>', '<', '|'])
        || thematic)
}

fn code_spans(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'`' {
            i += 1;
            continue;
        }
        let run_end = i + b[i..].iter().take_while(|c| **c == b'`').count();
        let run = run_end - i;
        let mut k = run_end;
        let mut closed = None;
        while k < b.len() {
            if b[k] == b'`' {
                let m = k + b[k..].iter().take_while(|c| **c == b'`').count();
                if m - k == run {
                    closed = Some(m);
                    break;
                }
                k = m;
            } else {
                k += 1;
            }
        }
        match closed {
            Some(end) => {
                spans.push((i, end));
                i = end;
            }
            None => i = run_end,
        }
    }
    spans
}

/// The `[` that opens the link text closed by the `]` at `close`: the nearest one before it
/// at depth zero; a bracket after a backslash does not count.
fn link_text_start(line: &str, close: usize) -> Option<usize> {
    let b = line.as_bytes();
    let mut depth = 0usize;
    let mut i = close;
    while i > 0 {
        i -= 1;
        let escaped = i > 0 && b[i - 1] == b'\\';
        match b[i] {
            b']' if !escaped => depth += 1,
            b'[' if !escaped => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn closing_paren(line: &str, from: usize) -> Option<usize> {
    let b = line.as_bytes();
    let mut depth = 1usize;
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'\\' if matches!(b.get(i + 1), Some(b'(' | b')' | b'\\')) => i += 1,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The destination after `](` at `open`, decoded: the raw `#` fragment cut off, backslash
/// escapes before ASCII punctuation removed, percent codes decoded. With its closing `)`.
fn read_destination(line: &str, open: usize) -> Option<(String, usize)> {
    let (target, close) = if line[open..].starts_with('<') {
        let gt = open + 1 + line[open + 1..].find('>')?;
        (&line[open + 1..gt], closing_paren(line, gt + 1)?)
    } else {
        let close = closing_paren(line, open)?;
        let inside = &line[open..close];
        let end = inside.find(char::is_whitespace).unwrap_or(inside.len());
        (&inside[..end], close)
    };
    let path = target.split('#').next().unwrap_or("");
    let mut unescaped = String::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            continue;
        }
        unescaped.push(c);
    }
    // Strict: a `%` without two hex digits, or bytes that are no UTF-8, keep the raw path.
    let bytes = unescaped.as_bytes();
    let nibble = |b: Option<&u8>| b.and_then(|b| (*b as char).to_digit(16)).map(|d| d as u8);
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let (Some(hi), Some(lo)) = (nibble(bytes.get(i + 1)), nibble(bytes.get(i + 2))) else {
                return Some((unescaped, close));
            };
            decoded.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let path = String::from_utf8(decoded).unwrap_or(unescaped);
    Some((path, close))
}

fn relation_type(before: &str) -> Option<String> {
    let before = before.trim();
    if before.len() >= 2 && before.starts_with('"') && before.ends_with('"') {
        Some(before[1..before.len() - 1].to_string())
    } else if !before.is_empty()
        && !before.contains(char::is_whitespace)
        && !before.starts_with([':', '['])
    {
        Some(before.to_string())
    } else {
        None
    }
}

fn reserved(path: &str) -> bool {
    matches!(path.rsplit('/').next(), Some("index.md" | "log.md"))
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['#', '/'])
        && !path.contains("://")
        && !path.starts_with("mailto:")
}

fn read_bullet(content: &str) -> Option<Read> {
    // An observation first: `[category]` not followed by `(` or `[`.
    if let Some(rest) = content.strip_prefix('[')
        && let Some(close) = rest.find(']')
    {
        let category = &rest[..close];
        let after = &rest[close + 1..];
        if !category.is_empty()
            && !category.contains('[')
            && !matches!(category, " " | "x" | "X" | "~")
            && !after.starts_with(['(', '['])
        {
            return Some(Read::Observation(category.to_string()));
        }
    }
    // The wikilink form.
    if let Some(open) = content.find("[[")
        && let Some(close) = content[open + 2..].find("]]")
        && let Some(kind) = relation_type(&content[..open])
    {
        let inner = &content[open + 2..open + 2 + close];
        let target = inner.split('|').next().unwrap_or("").trim().to_string();
        if !target.is_empty() && !reserved(&target) {
            return Some(Read::Relation(kind, target));
        }
    }
    // The relative form: the first `[` opens a link to a `.md` file.
    let open = content.find('[')?;
    let spans = code_spans(content);
    let mut cursor = 0;
    while let Some(found) = content[cursor..].find("](") {
        let close_text = cursor + found;
        cursor = close_text + 2;
        let Some(start) = link_text_start(content, close_text) else {
            continue;
        };
        let Some((path, close)) = read_destination(content, close_text + 2) else {
            continue;
        };
        cursor = close;
        let image = content[..start].ends_with('!');
        let note = path.to_ascii_lowercase().ends_with(".md");
        let in_code = spans.iter().any(|(s, e)| (*s..*e).contains(&close_text));
        // Every note link counts; the one that starts at the first `[` is the target.
        if !image && note && relative(&path) && !in_code && !reserved(&path) && start == open {
            return relation_type(&content[..open]).map(|k| Read::Relation(k, path));
        }
    }
    None
}

/// What Crystalline reads from the top-level bullets of `body`.
fn crystalline_reads(body: &str) -> Vec<Read> {
    let lines: Vec<&str> = body.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let mut fence: Option<(char, usize)> = None;
    let in_fence: Vec<bool> = lines
        .iter()
        .map(|l| match fence {
            None => {
                fence = fence_marker(l);
                fence.is_some()
            }
            Some((c, n)) => {
                if let Some((c2, n2)) = fence_marker(l)
                    && c2 == c
                    && n2 >= n
                    && l.trim_start().trim_start_matches(c).trim().is_empty()
                {
                    fence = None;
                }
                true
            }
        })
        .collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if in_fence[i] || is_heading(line) {
            continue;
        }
        let Some(first) = line.strip_prefix("- ") else {
            continue;
        };
        let mut content = first.to_string();
        let open_category = first.starts_with('[') && !first.contains(']');
        if !open_category {
            let mut end = i;
            for (j, next) in lines.iter().enumerate().skip(i + 1) {
                // The last `[[` outside inline code has no `]]` after it.
                let mut masked = lines[end].as_bytes().to_vec();
                for (a, b) in code_spans(lines[end]) {
                    masked[a..b].fill(b' ');
                }
                let masked = String::from_utf8_lossy(&masked).into_owned();
                let inside_link = masked
                    .rfind("[[")
                    .is_some_and(|o| !masked[o + 2..].contains("]]"));
                if in_fence[j] || !continues_bullet(next) || inside_link {
                    break;
                }
                content = format!("{} {}", content.trim_end(), next.trim());
                end = j;
            }
        }
        if let Some(read) = read_bullet(&content) {
            out.push(read);
        }
    }
    out
}

/// The relations Crystalline reads from a file, as (type, target).
fn relations(md: &str) -> Vec<(String, String)> {
    crystalline_reads(body(md))
        .into_iter()
        .filter_map(|r| match r {
            Read::Relation(k, t) => Some((k, t)),
            Read::Observation(_) => None,
        })
        .collect()
}

#[test]
fn the_test_reader_reads_what_crystalline_reads() {
    let reads = |text: &str| crystalline_reads(text);
    let rel = |k: &str, t: &str| Read::Relation(k.into(), t.into());
    assert_eq!(reads("- part_of [A](../a.md)"), [rel("part_of", "../a.md")]);
    assert_eq!(
        reads("- \"two words\" [x](y.md)"),
        [rel("two words", "y.md")]
    );
    assert_eq!(reads("- tok [[x]]"), [rel("tok", "x")]);
    assert_eq!(
        reads("- part_of [A](<../a b (c) %232.md>)"),
        [rel("part_of", "../a b (c) #2.md")]
    );
    // A wrapped bullet is read over its continuation line.
    assert_eq!(reads("- follows\n[A](a.md)"), [rel("follows", "a.md")]);
    assert_eq!(reads("- [cat] text"), [Read::Observation("cat".into())]);
    // Not relations: a bullet that opens with a link, an indented bullet, a fenced line,
    // two words, an image, a web page, a reserved name, a link in code.
    for text in [
        "- [A](a.md): x",
        "  - part_of [A](a.md)",
        "```\n- part_of [A](a.md)\n```",
        "- two words [A](a.md)",
        "- part_of ![A](a.md)",
        "- part_of [A](https://e.org/a.md)",
        "- part_of [A](index.md)",
        "- part_of `[A](a.md)`",
        "- x \\[A\\](a.md)",
    ] {
        assert_eq!(reads(text), [], "{text}");
    }
}

/// A document whose every written place holds text that reads like a relation, a wikilink or
/// an observation.
fn hostile_doc() -> Document {
    let mut d = doc(FormatId::Pdf);
    d.source.name = "Q3 report (final) #2.pdf".into();
    d.properties.title = Some("Q3 `x` [y](z.md) [[w]] \"two words\" [x](y.md) a](b.md) c".into());
    d.properties.description = Some("- part_of [d](e.md)\n- [cat] text".into());
    d.properties.authors = vec!["part_of [x](y.md)".into(), "[[a]]".into()];
    for i in 1..=4u32 {
        d.facets.outline.push(OutlineEntry {
            level: 1,
            title: format!("tok [x](y{i}.md)"),
            page: Some(i),
        });
    }
    for i in 1..=4u32 {
        d.body.push(page(
            i,
            vec![
                heading(1, &format!("tok [[c{i}]] [k](l.md) x](m.md) `")),
                para(&words(9_000)),
                Block::Table {
                    caption: None,
                    header_rows: 1,
                    role: None,
                    rows: vec![vec![Cell::new(vec![Inline::plain("a")])]],
                    alignments: Vec::new(),
                    anchor: None,
                    layout: None,
                },
            ],
        ));
    }
    d.overview = Some(rezzer_model::Overview {
        parts: vec![OverviewPart {
            id: "p1".into(),
            name: "rel [x](y.md)".into(),
            kind: "page".into(),
            size: "1 table".into(),
            role: None,
            reads: Vec::new(),
        }],
        facts: vec![("part_of [x](y.md)".into(), "[[z]]".into())],
    });
    for i in 0..30 {
        d.losses.push(loss(&format!("part_of [x](y{i}.md) [[q]]")));
    }
    d
}

#[test]
fn relation_lines_parse_like_crystalline() {
    let main = "Q3 report (final) #2.pdf.md";
    let out = output(&hostile_doc(), 2_500, main, "q3-report");
    assert_files_within(&out, 2_500);
    assert!(out.parts.len() >= 5, "{:?}", out.parts.len());
    assert_eq!(crystalline_reads(body(&out.main)), [], "the main document");
    let mut previous: Option<String> = None;
    for p in &out.parts {
        let file = p.path.rsplit('/').next().expect("file").to_string();
        let mut want = vec![("part_of".to_string(), format!("../{main}"))];
        if let Some(prev) = &previous {
            want.push(("follows".to_string(), prev.clone()));
        }
        assert_eq!(relations(&p.markdown), want, "{}", p.path);
        // Nothing else is read from the part: no other relation, no observation.
        let reads = crystalline_reads(body(&p.markdown));
        assert_eq!(reads.len(), want.len(), "{}: {reads:?}", p.path);
        previous = Some(file);
    }
    // List items from the file that Crystalline would read (P5-S): a link to a heading in
    // another part becomes a link to that part's file, and a footnote at the start of an item
    // looks like an observation category.
    let out = output(&list_items_doc(), 2_500, "r.md", "r");
    let first = &out.parts[0].markdown;
    assert!(
        first.contains("\n* See [Three](chapter-3.md#chapter-3)\n"),
        "{first}"
    );
    assert!(first.contains("\n* [^1] noted\n"), "{first}");
    assert!(first.contains("\n* plain item\n"), "{first}");
    assert!(
        first.contains("\n* Wrapped\\\n  [Two](chapter-2.md#chapter-2)\n"),
        "{first}"
    );
    for p in &out.parts {
        let reads = crystalline_reads(body(&p.markdown));
        assert!(
            reads.iter().all(|r| matches!(
                r,
                Read::Relation(k, _) if k == "part_of" || k == "follows"
            )),
            "{}: {reads:?}",
            p.path
        );
    }
}

/// Three chapters; the first holds list items that Crystalline would read as a relation or
/// an observation once the document is split.
fn list_items_doc() -> Document {
    let mut d = chapters_doc(3, 6_000);
    let link = |text: &str, target: &str| Inline::Link {
        target: target.into(),
        title: None,
        internal: true,
        wiki: false,
        inlines: vec![Inline::plain(text)],
    };
    let item = |inlines: Vec<Inline>| {
        vec![Block::Paragraph {
            inlines,
            role: None,
            anchor: None,
        }]
    };
    d.body.insert(
        2,
        Block::List {
            ordered: false,
            items: vec![
                item(vec![Inline::plain("See "), link("Three", "#chapter-3")]),
                item(vec![
                    Inline::FootnoteRef { id: "1".into() },
                    Inline::plain(" noted"),
                ]),
                item(vec![Inline::plain("plain item")]),
                item(vec![
                    Inline::plain("Wrapped"),
                    Inline::LineBreak,
                    link("Two", "#chapter-2"),
                ]),
            ],
            anchor: None,
        },
    );
    d
}

#[test]
fn list_items_crystalline_would_read_get_a_star_in_one_file_too() {
    let mut d = doc(FormatId::Markdown);
    d.body.push(Block::List {
        ordered: false,
        items: vec![
            vec![Block::Paragraph {
                inlines: vec![
                    Inline::plain("See "),
                    Inline::Link {
                        target: "other.md".into(),
                        title: None,
                        internal: false,
                        wiki: false,
                        inlines: vec![Inline::plain("other")],
                    },
                ],
                role: None,
                anchor: None,
            }],
            vec![para("plain")],
        ],
        anchor: None,
    });
    let md = render(&d, Completeness::Complete, &opts());
    assert!(md.contains("\n* See [other](other.md)\n* plain\n"), "{md}");
    assert_eq!(crystalline_reads(body(&md)), []);
}

#[test]
fn relation_targets_with_spaces_and_hash() {
    let main = "Q3 report (final) #2.pdf.md";
    let mut d = chapters_doc(3, 6_000);
    d.source.name = "Q3 report (final) #2.pdf".into();
    d.properties.title = Some("Q3 `report".into());
    let out = output(&d, 2_500, main, "Q3 report (final) #2");
    let second = &out.parts[1].markdown;
    assert!(
        second.contains("\n- part_of [Q3 \\`report](<../Q3 report (final) %232.pdf.md>)\n"),
        "{second}"
    );
    assert!(second.contains("\n- follows [Q3 \\`report: Chapter 1](chapter-1.md)\n"));
    assert_eq!(
        relations(second),
        [
            ("part_of".to_string(), format!("../{main}")),
            ("follows".to_string(), "chapter-1.md".to_string())
        ]
    );
    // The main document's links go into the folder, encoded the same way.
    let parts = section_lines(&out.main, "Parts");
    assert_eq!(
        parts[0],
        "- [Q3 \\`report: Chapter 1](<Q3 report (final) %232/chapter-1.md>): Chapter 1"
    );
    assert_eq!(out.parts[0].path, "Q3 report (final) #2/chapter-1.md");
    // A `]` in a title never ends an observation category in a list of parts.
    let mut plain = chapters_doc(3, 6_000);
    plain.properties.title = Some("A] b".into());
    let out = output(&plain, 2_500, "a.md", "a");
    assert!(out.main.contains("- [A&#93; b: Chapter 1](a/chapter-1.md)"));
    assert_eq!(crystalline_reads(body(&out.main)), []);
    // Any whitespace puts the destination in angle brackets: Crystalline cuts a bare one at
    // the first whitespace character.
    for odd in [
        "日本\u{3000}報告.pdf.md",
        "Rechnung\u{a0}12.pdf.md",
        "[[x]].md",
    ] {
        let out = output(&d, 2_500, odd, "x");
        assert_eq!(
            relations(&out.parts[0].markdown),
            [("part_of".to_string(), format!("../{odd}"))],
            "{odd}"
        );
    }
    assert!(
        output(&d, 2_500, "[[x]].md", "x").parts[0]
            .markdown
            .contains("(../%5B%5Bx%5D%5D.md)")
    );
    // `%`, `>` and a backslash are encoded too, so the name reads back as written.
    let odd = "a%b>c\\d`e.md";
    let out = output(&d, 2_500, odd, "x");
    assert_eq!(relations(&out.parts[0].markdown)[0].1, format!("../{odd}"));
}

#[test]
fn not_read_capped_with_link() {
    let mut d = chapters_doc(4, 6_000);
    for i in 1..=45 {
        d.losses.push(loss(&format!("thing {i}")));
    }
    let out = output(&d, 2_500, "report.md.md", "b");
    assert_files_within(&out, 2_500);
    let lines = section_lines(&out.main, "Not read");
    // The header, its separator, 20 rows and the line with the link.
    assert_eq!(lines.len(), 23, "{lines:?}");
    assert!(lines[21].contains("thing 20"));
    assert_eq!(lines[22], "And 25 more in [Not read](b/not-read.md).");
    let not_read = out
        .parts
        .iter()
        .find(|p| p.path == "b/not-read.md")
        .expect("a not-read part");
    assert_eq!(not_read.title, "report.md: Not read");
    assert_eq!(not_read.location, "Not read");
    for i in 1..=45 {
        assert!(
            not_read.markdown.contains(&format!("thing {i} ")),
            "thing {i}"
        );
    }
    // It is a part like the others: listed last and in the follows chain.
    let parts = section_lines(&out.main, "Parts");
    assert_eq!(
        parts.last().copied(),
        Some("- [report.md: Not read](b/not-read.md): Not read")
    );
    assert_eq!(
        out.parts.last().map(|p| p.path.as_str()),
        Some("b/not-read.md")
    );
    assert_eq!(
        relations(&not_read.markdown)[1],
        ("follows".to_string(), "chapter-4.md".to_string())
    );
    // Under the cap the list is whole and there is no not-read part.
    let mut d = chapters_doc(4, 6_000);
    for i in 1..=15 {
        d.losses.push(loss(&format!("thing {i}")));
    }
    let out = output(&d, 2_500, "report.md.md", "b");
    assert_eq!(section_lines(&out.main, "Not read").len(), 17);
    assert!(!out.main.contains("not-read.md"));
    assert!(out.parts.iter().all(|p| !p.path.contains("not-read")));
}

#[test]
fn outline_and_tables_capped_at_20() {
    let mut d = doc(FormatId::Pdf);
    for i in 1..=30u32 {
        d.facets.outline.push(OutlineEntry {
            level: 1,
            title: format!("Chapter {i}"),
            page: Some(i),
        });
        d.facets.outline.push(OutlineEntry {
            level: 3,
            title: format!("Deep {i}"),
            page: Some(i),
        });
        d.body.push(page(
            i,
            vec![
                para(&words(3_000)),
                Block::Table {
                    caption: None,
                    header_rows: 1,
                    role: None,
                    rows: vec![vec![Cell::new(vec![Inline::plain("a")])]],
                    alignments: Vec::new(),
                    anchor: None,
                    layout: None,
                },
            ],
        ));
    }
    d.facets.outline.insert(
        1,
        OutlineEntry {
            level: 2,
            title: "Part 1.1".into(),
            page: Some(1),
        },
    );
    let out = output(&d, 2_500, "report.pdf.md", "b");
    assert_files_within(&out, 2_500);
    let outline = section_lines(&out.main, "Outline");
    assert_eq!(outline.len(), 21, "{outline:?}");
    assert_eq!(outline[0], "- Chapter 1, page 1");
    assert_eq!(outline[1], "  - Part 1.1, page 1");
    assert_eq!(outline[19], "- Chapter 19, page 19");
    assert_eq!(outline[20], "- and 11 more");
    assert!(!outline.iter().any(|l| l.contains("Deep")));
    let tables = section_lines(&out.main, "Tables");
    assert_eq!(tables.len(), 21, "{tables:?}");
    assert_eq!(tables[0], "- Table 1 page 1");
    assert_eq!(tables[19], "- Table 20 page 20");
    assert_eq!(tables[20], "- and 10 more");
}

#[test]
fn overview_is_capped_at_20() {
    let mut d = chapters_doc(6, 6_000);
    d.overview = Some(rezzer_model::Overview {
        parts: (1..=30)
            .map(|i| OverviewPart {
                id: format!("p{i}"),
                name: format!("Part {i}"),
                kind: "chapter".into(),
                size: "1 paragraph".into(),
                role: None,
                reads: Vec::new(),
            })
            .collect(),
        facts: Vec::new(),
    });
    let out = output(&d, 2_500, "report.md.md", "b");
    let lines = section_lines(&out.main, "Overview");
    // The header, its separator, 20 rows, the line that counts the rest and the link.
    assert_eq!(lines.len(), 24, "{lines:?}");
    assert!(lines[21].contains("Part 20 "));
    assert_eq!(lines[22], "And 10 more rows.");
    assert_eq!(
        lines[23],
        "The whole overview is in [Overview](b/overview.md)."
    );
    // The overview part holds every row, and is listed after the content's parts.
    let part = out
        .parts
        .iter()
        .find(|p| p.path == "b/overview.md")
        .expect("an overview part");
    assert_eq!(part.title, "report.md: Overview");
    assert!(part.markdown.contains("| Part 30 |"));
    assert_eq!(
        section_lines(&out.main, "Parts").last().copied(),
        Some("- [report.md: Overview](b/overview.md): Overview")
    );
    // An overview within the cap is whole in the main document, with no part.
    d.overview.as_mut().expect("overview").parts.truncate(5);
    let out = output(&d, 2_500, "report.md.md", "b");
    assert_eq!(section_lines(&out.main, "Overview").len(), 7);
    assert!(out.parts.iter().all(|p| p.path != "b/overview.md"));
    // A cut name is a shortened overview too.
    d.overview.as_mut().expect("overview").parts[0].name = words(300);
    let out = output(&d, 2_500, "report.md.md", "b");
    let part = out.parts.iter().find(|p| p.path == "b/overview.md");
    assert!(
        part.expect("an overview part")
            .markdown
            .contains(&words(300))
    );
}

#[test]
fn not_read_with_a_cut_text_writes_the_part() {
    let mut d = chapters_doc(4, 6_000);
    for i in 1..=15 {
        d.losses.push(loss(&format!("thing {i}")));
    }
    d.losses[3].what = words(500);
    let out = output(&d, 2_500, "report.md.md", "b");
    let lines = section_lines(&out.main, "Not read");
    assert_eq!(lines.len(), 18, "{lines:?}");
    assert_eq!(
        lines[17],
        "The whole texts are in [Not read](b/not-read.md)."
    );
    let part = out
        .parts
        .iter()
        .find(|p| p.path == "b/not-read.md")
        .expect("a not-read part");
    assert!(part.markdown.contains(&words(500)));
}

#[test]
fn pieces_of_a_unit_are_numbered_in_order() {
    let list = units(&counted_doc(1, 0), 2_500);
    let s1: Vec<(&str, &str)> = list
        .iter()
        .filter(|u| u.file.starts_with("s1"))
        .map(|u| (u.file.as_str(), u.title.as_str()))
        .collect();
    assert_eq!(
        s1,
        [("s1", "S1"), ("s1-2", "S1, part 2"), ("s1-3", "S1, part 3")]
    );
}

#[test]
fn trailing_part_names_never_clash() {
    // Headings named like the main document writer's parts take the next free names.
    let mut d = doc(FormatId::Markdown);
    for title in ["Contents", "Not read", "Contents"] {
        d.body.push(heading(1, title));
        d.body.push(para(&words(6_000)));
    }
    for i in 1..=2_000 {
        d.losses.push(loss(&format!("thing {i}")));
    }
    let out = output(&d, 2_500, "report.md.md", "b");
    assert_files_within(&out, 2_500);
    let mut seen = std::collections::HashSet::new();
    for p in &out.parts {
        assert!(seen.insert(p.path.clone()), "{} twice", p.path);
    }
    let files: Vec<&str> = out.parts.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(
        &files[..3],
        ["b/contents-2.md", "b/not-read-2.md", "b/contents-3.md"]
    );
    assert_eq!(files[3], "b/not-read.md");
    assert!(files.contains(&"b/not-read-3.md"), "{files:?}");
}

#[test]
fn main_document_fits_with_huge_own_texts() {
    let mut d = chapters_doc(6, 6_000);
    d.properties.title = Some("<".repeat(20_000));
    d.properties.description = Some("*".repeat(100_000));
    d.properties.authors = (0..5_000).map(|i| format!("Author {i}")).collect();
    for i in 0..500 {
        d.facets.outline.push(OutlineEntry {
            level: 1,
            title: "<".repeat(1_000) + &i.to_string(),
            page: None,
        });
    }
    let out = output(&d, 2_500, "report.md.md", "b");
    assert_files_within(&out, 2_500);
    assert!(out.main.contains("## Parts"));
    assert!(out.main.contains(
        "These lists are left out to keep this file within the size limit: Outline, Dates and \
         people."
    ));
    assert!(out.parts.iter().any(|p| p.path == "b/contents.md"));
}

#[test]
fn a_list_with_one_item_crystalline_would_read_stays_one_list() {
    let mut d = doc(FormatId::Markdown);
    let item = |inlines: Vec<Inline>| {
        vec![Block::Paragraph {
            inlines,
            role: None,
            anchor: None,
        }]
    };
    d.body.push(Block::List {
        ordered: false,
        items: vec![
            item(vec![Inline::plain("first item")]),
            item(vec![
                Inline::FootnoteRef { id: "1".into() },
                Inline::plain(" noted"),
            ]),
            item(vec![Inline::plain("last item")]),
        ],
        anchor: None,
    });
    let md = render(&d, Completeness::Complete, &opts());
    assert!(
        md.contains("\n* first item\n* [^1] noted\n* last item\n"),
        "{md}"
    );
    // CommonMark reads one list with three items.
    use pulldown_cmark::{Event, Parser, Tag};
    let (mut lists, mut items) = (0, 0);
    for event in Parser::new(body(&md)) {
        match event {
            Event::Start(Tag::List(None)) => lists += 1,
            Event::Start(Tag::Item) => items += 1,
            _ => {}
        }
    }
    assert_eq!((lists, items), (1, 3));
    assert_eq!(crystalline_reads(body(&md)), []);
}

#[test]
fn the_overview_part_holds_every_row() {
    // More rows than one file's overview shows (200): the overview parts drop none.
    let mut d = chapters_doc(4, 6_000);
    d.overview = Some(rezzer_model::Overview {
        parts: (1..=250)
            .map(|i| OverviewPart {
                id: format!("p{i}"),
                name: format!("Part {i}"),
                kind: "chapter".into(),
                size: "1 paragraph".into(),
                role: None,
                // One arrow, so the overview has a graph, which shows only 200 parts.
                reads: if i == 2 {
                    vec!["p1".into()]
                } else {
                    Vec::new()
                },
            })
            .collect(),
        facts: vec![("See".into(), "[x](y.md)".into())],
    });
    let out = output(&d, 2_500, "report.md.md", "b");
    assert_files_within(&out, 2_500);
    let overview: Vec<&PartFile> = out
        .parts
        .iter()
        .filter(|p| p.location == "Overview")
        .collect();
    assert!(!overview.is_empty());
    assert_eq!(overview[0].path, "b/overview.md");
    let text: String = overview.iter().map(|p| p.markdown.as_str()).collect();
    for i in 1..=250 {
        assert!(text.contains(&format!("| Part {i} |")), "Part {i}");
    }
    assert!(!text.contains("Only the first"));
    // The note says that the graph is cut, not the table.
    assert!(
        text.contains("The graph shows only the first 200 of 250 parts. The table lists them all."),
        "{text}"
    );
    // Every written file reads as Crystalline reads it: relations only where Rezzer means them.
    assert_eq!(crystalline_reads(body(&out.main)), []);
    for p in &out.parts {
        let reads = crystalline_reads(body(&p.markdown));
        assert!(
            reads.iter().all(|r| matches!(
                r,
                Read::Relation(k, _) if k == "part_of" || k == "follows"
            )),
            "{}: {reads:?}",
            p.path
        );
    }
}
