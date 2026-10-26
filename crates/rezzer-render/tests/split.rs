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
    let mut logic_only = opts();
    logic_only.sections = Some(vec![rezzer_render::OutputSection::Content]);
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
    let list = split(&d, &logic_only, 2_500, "b").expect("over the size");
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
        // The code lines are all there.
        let joined: String = list.iter().map(|u| u.markdown.as_str()).collect();
        assert!(joined.contains("let y000 ") && joined.contains("let y299 "));
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
