// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{Builder, Page, pdf, show, zeros_packed};
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Block, Bytes, CalloutKind, Completeness, Confidence, Detection, FormatId, Inline,
    LossReason, ReadOptions, ReadOutcome, Reader, SectionKind, Severity, Source, StatusColor,
    plain_text,
};
use rezzer_read_pdf::PdfReader;

fn corpus(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/corpus/pdf");
    std::fs::read(path.join(name)).expect("corpus file")
}

fn source(bytes: &[u8]) -> Source {
    Source {
        path: None,
        name: "file.pdf".into(),
        bytes: Bytes::Owned(Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format: FormatId::Pdf,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "%PDF-".into(),
        },
        modified: None,
    }
}

fn read_with(bytes: &[u8], limits: Limits) -> ReadOutcome {
    PdfReader.read(
        &source(bytes),
        &ReadOptions::default(),
        &Budget::new(limits),
    )
}

fn read(bytes: &[u8]) -> ReadOutcome {
    read_with(bytes, Limits::default())
}

/// The blocks of page section `n`.
fn page(out: &ReadOutcome, n: u32) -> &[Block] {
    let want = Anchor(format!("p{n}"));
    out.document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section {
                kind: SectionKind::Page,
                anchor: Some(a),
                blocks,
                ..
            } if *a == want => Some(blocks.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no section p{n} in {:#?}", out.document.body))
}

/// The text of a block and the blocks inside it, one line per paragraph or heading.
fn block_text(block: &Block, out: &mut String) {
    match block {
        Block::Heading { inlines, .. }
        | Block::Paragraph { inlines, .. }
        | Block::Task { inlines, .. } => {
            out.push_str(&plain_text(inlines));
            out.push('\n');
        }
        Block::Section { blocks, .. }
        | Block::Quote { blocks, .. }
        | Block::Callout { blocks, .. } => blocks.iter().for_each(|b| block_text(b, out)),
        Block::List { items, .. } => items.iter().flatten().for_each(|b| block_text(b, out)),
        Block::Table { rows, .. } => {
            for row in rows {
                for cell in row {
                    out.push_str(&plain_text(&cell.inlines));
                    out.push('|');
                }
                out.push('\n');
            }
        }
        _ => {}
    }
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    blocks.iter().for_each(|b| block_text(b, &mut out));
    out
}

/// The text of the page sections without the folded callouts (what an agent reads as the
/// page's body).
fn body_text(blocks: &[Block]) -> String {
    let kept: Vec<Block> = blocks
        .iter()
        .filter(|b| !matches!(b, Block::Callout { .. }))
        .cloned()
        .collect();
    text(&kept)
}

fn all_text(out: &ReadOutcome) -> String {
    text(&out.document.body)
}

fn headings(blocks: &[Block]) -> Vec<(u8, String)> {
    let mut found = Vec::new();
    for b in blocks {
        match b {
            Block::Heading { level, inlines, .. } => found.push((*level, plain_text(inlines))),
            Block::Section { blocks, .. } => found.extend(headings(blocks)),
            _ => {}
        }
    }
    found
}

#[test]
fn two_columns_read_in_order() {
    let out = read(&corpus("pypdf-multicolumn.pdf"));
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
    let t = all_text(&out);
    // The last words of the first column on page 1 and the first words of the second.
    let first_end = t
        .find("Vivamus viverra fermentum felis.")
        .expect("end of column 1");
    let second_start = t
        .find("pellentesque ante. Phasellus adipiscing semper elit.")
        .expect("start of column 2");
    assert!(first_end < second_start, "{t}");
    // Lines of one column are not mixed with lines of the other.
    assert!(
        !t.contains("Vivamus viverra fermentum felis. pellentesque"),
        "{t}"
    );
}

#[test]
fn tagged_headings_from_roles() {
    let out = read(&corpus("pdfjs-structure-simple.pdf"));
    assert_eq!(out.document.properties.facts.tagged, Some(true));
    assert_eq!(out.document.properties.facts.pages, Some(1));
    let blocks = page(&out, 1);
    assert_eq!(
        headings(blocks),
        vec![(1, "Heading 1".to_string()), (2, "Heading 2".to_string())]
    );
    let paragraphs: Vec<String> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph {
                inlines,
                role: None,
                ..
            } => Some(plain_text(inlines)),
            _ => None,
        })
        .collect();
    assert_eq!(paragraphs, ["This paragraph 1.", "This paragraph 2."]);
}

#[test]
fn untagged_headings_from_size() {
    let out = read(&corpus("pypdf-outline.pdf"));
    assert_eq!(out.document.properties.facts.tagged, Some(false));
    let found = headings(&out.document.body);
    assert!(found.contains(&(1, "Contents".to_string())), "{found:?}");
    assert!(found.contains(&(1, "1 Foo".to_string())), "{found:?}");
    assert!(found.contains(&(1, "2 Bar".to_string())), "{found:?}");
    // Body text is never a heading.
    assert!(
        found.iter().all(|(_, t)| !t.starts_with("Hello")),
        "{found:?}"
    );
}

#[test]
fn repeated_line_inside_the_text_stays_text() {
    let words = ["alpha", "beta", "gamma", "delta"];
    let pages: Vec<Page> = words
        .iter()
        .map(|w| {
            Page::text(format!(
                "{}{}{}{}",
                show("F1", 9.0, 72.0, 815.0, "ACME Quarterly Report"),
                show(
                    "F1",
                    11.0,
                    72.0,
                    790.0,
                    &format!("The {w} section starts here.")
                ),
                show("F1", 11.0, 72.0, 760.0, "Control:"),
                show(
                    "F1",
                    11.0,
                    72.0,
                    500.0,
                    &format!("More about {w} in plain words.")
                ),
            ))
        })
        .collect();
    let out = read(&pdf(&pages, ""));
    for n in 1..=4u32 {
        let roles: Vec<(Option<String>, String)> = page(&out, n)
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph { inlines, role, .. } => Some((role.clone(), plain_text(inlines))),
                _ => None,
            })
            .collect();
        assert_eq!(
            roles[1],
            (Some("page-header".into()), "ACME Quarterly Report".into())
        );
        // In the top band and on every page, but below text of the page: not a header.
        assert!(
            roles
                .iter()
                .any(|(r, t)| r.is_none() && t.contains("Control:")),
            "page {n}: {roles:?}"
        );
    }
}

/// The paragraphs of a page with their roles, without the marker.
fn roles_of(out: &ReadOutcome, n: u32) -> Vec<(Option<String>, String)> {
    page(out, n)
        .iter()
        .skip(1)
        .filter_map(|b| match b {
            Block::Paragraph { inlines, role, .. } => Some((role.clone(), plain_text(inlines))),
            Block::Heading { inlines, .. } => Some((Some("heading".into()), plain_text(inlines))),
            _ => None,
        })
        .collect()
}

#[test]
fn edge_line_that_is_also_body_text_stays_text() {
    // As in a control catalog: a table continues over pages 1 to 3 and repeats its column
    // titles below the running header; a section label ends pages 1 to 3 above the footer.
    // On page 4 the same lines stand in the middle of the text.
    let pages: Vec<Page> = (1..=4u32)
        .map(|n| {
            let mut c = show("F1", 9.0, 72.0, 815.0, "ACME Control Catalog");
            if n < 4 {
                c += &show("F1", 10.0, 72.0, 780.0, "CONTROL NAME");
                c += &show(
                    "F1",
                    10.0,
                    72.0,
                    500.0,
                    &format!("Row text {n} of the table."),
                );
                c += &show("F1", 10.0, 72.0, 110.0, "Related Controls: None.");
            } else {
                c += &show("F1", 10.0, 72.0, 600.0, "Some text before the table.");
                c += &show("F1", 10.0, 72.0, 500.0, "CONTROL NAME");
                c += &show("F1", 10.0, 72.0, 480.0, "Related Controls: None.");
                c += &show("F1", 10.0, 72.0, 460.0, "Some text after the label.");
            }
            c += &show("F1", 9.0, 280.0, 30.0, &format!("Page {n} of 4"));
            Page::text(c)
        })
        .collect();
    let out = read(&pdf(&pages, ""));
    for n in 1..=3u32 {
        let roles = roles_of(&out, n);
        assert_eq!(
            roles.first(),
            Some(&(Some("page-header".into()), "ACME Control Catalog".into())),
            "page {n}: {roles:?}"
        );
        assert_eq!(
            roles.last(),
            Some(&(Some("page-footer".into()), format!("Page {n} of 4"))),
            "page {n}: {roles:?}"
        );
        for text in ["CONTROL NAME", "Related Controls: None."] {
            assert!(
                roles.iter().any(|(r, t)| r.is_none() && t == text),
                "page {n}: {text} is not body text: {roles:?}"
            );
        }
    }
}

#[test]
fn bare_page_numbers_stay_footers_beside_numbers_in_the_text() {
    let pages: Vec<Page> = (1..=3u32)
        .map(|n| {
            Page::text(format!(
                "{}{}{}",
                show(
                    "F1",
                    11.0,
                    72.0,
                    500.0,
                    &format!("Body {} words.", ["one", "two", "three"][n as usize - 1])
                ),
                show("F1", 11.0, 72.0, 450.0, &format!("{}", n + 4)),
                show("F1", 9.0, 290.0, 30.0, &n.to_string()),
            ))
        })
        .collect();
    let out = read(&pdf(&pages, ""));
    for n in 1..=3u32 {
        assert_eq!(
            roles_of(&out, n).last(),
            Some(&(Some("page-footer".into()), n.to_string())),
            "page {n}"
        );
    }
}

#[test]
fn header_heights_a_fraction_apart_are_one_header() {
    // On exactly three pages, with middles 0.2 points apart across a multiple of 4 points.
    let ys = [797.4, 797.6, 797.4];
    let pages: Vec<Page> = ys
        .iter()
        .enumerate()
        .map(|(i, y)| {
            Page::text(format!(
                "{}{}",
                show("F1", 9.0, 72.0, *y, "ACME Quarterly Report"),
                show(
                    "F1",
                    11.0,
                    72.0,
                    500.0,
                    &format!("Body {} words.", ["one", "two", "three"][i])
                ),
            ))
        })
        .collect();
    let out = read(&pdf(&pages, ""));
    for n in 1..=3u32 {
        assert_eq!(
            roles_of(&out, n).first(),
            Some(&(Some("page-header".into()), "ACME Quarterly Report".into())),
            "page {n}"
        );
    }
}

#[test]
fn page_sections_count_against_the_element_limit() {
    // The page walk of the open stays under the limit; two elements per page do not.
    let pages = vec![Page::text(String::new()); 900];
    let limits = Limits {
        max_elements: 1_000,
        ..Limits::default()
    };
    let out = read_with(&pdf(&pages, ""), limits);
    assert_eq!(out.completeness, Completeness::Partial);
    let sections = out
        .document
        .body
        .iter()
        .filter(|b| matches!(b, Block::Section { .. }))
        .count();
    assert!(sections <= 500, "{sections} sections");
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("elements")),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn latex_page_numbers_are_footers() {
    // pdflatex sets the page number about 90 points above the bottom of an A4 page.
    let out = read(&corpus("pypdf-outline.pdf"));
    for n in 1..=4u32 {
        let blocks = page(&out, n);
        match blocks.last() {
            Some(Block::Paragraph {
                inlines,
                role: Some(role),
                ..
            }) => {
                assert_eq!(role, "page-footer", "page {n}");
                assert_eq!(plain_text(inlines), n.to_string());
            }
            other => panic!("page {n}: {other:?}"),
        }
    }
}

#[test]
fn actual_text_replaces_glyphs() {
    let out = read(&corpus("pdfbox-actualtext-din91379.pdf"));
    let t = all_text(&out);
    // The file draws base letters and combining marks as separate glyphs; its ActualText
    // spans give the sequences as text.
    assert!(t.contains("A\u{30b} C\u{300} C\u{304}"), "{t}");
    assert!(t.contains("K\u{35f}H"), "{t}");
}

#[test]
fn arabic_text_kept() {
    let out = read(&corpus("pypdf-arabic-habibi.pdf"));
    let t = all_text(&out);
    assert!(t.contains("habibi"), "{t}");
    assert!(!t.contains("ibibah"), "{t}");
    assert!(t.contains('\u{62d}'), "{t}");
}

fn marker(blocks: &[Block]) -> String {
    match blocks.first() {
        Some(Block::Paragraph {
            inlines,
            role: Some(role),
            ..
        }) if role == "page" => match inlines.as_slice() {
            [Inline::Status { text, color }] => {
                assert_eq!(*color, StatusColor::Grey);
                text.clone()
            }
            other => panic!("page marker inlines {other:?}"),
        },
        other => panic!("no page marker first: {other:?}"),
    }
}

#[test]
fn page_sections_with_labels() {
    let out = read(&corpus("qpdf-page-labels-and-outlines.pdf"));
    assert_eq!(out.document.properties.facts.pages, Some(30));
    let sections = out
        .document
        .body
        .iter()
        .filter(|b| {
            matches!(
                b,
                Block::Section {
                    kind: SectionKind::Page,
                    ..
                }
            )
        })
        .count();
    assert_eq!(sections, 30);
    // An empty label shows the page number alone.
    assert_eq!(marker(page(&out, 1)), "Page 1");
    assert_eq!(marker(page(&out, 3)), "Page i (3)");
    assert!(text(page(&out, 3)).contains("Potato 2"));
    // Block anchors count the page's blocks after the marker, which has none.
    match &page(&out, 3)[..2] {
        [
            Block::Paragraph { anchor: None, .. },
            Block::Heading { anchor, .. } | Block::Paragraph { anchor, .. },
        ] => assert_eq!(anchor.as_ref(), Some(&Anchor("p3:b1".into()))),
        other => panic!("{other:?}"),
    }
}

fn links(inlines: &[Inline], found: &mut Vec<(String, bool, String)>) {
    for i in inlines {
        if let Inline::Link {
            target,
            internal,
            inlines,
            ..
        } = i
        {
            found.push((target.clone(), *internal, plain_text(inlines)));
        }
    }
}

fn all_links(blocks: &[Block]) -> Vec<(String, bool, String)> {
    let mut found = Vec::new();
    for b in blocks {
        match b {
            Block::Paragraph { inlines, .. } | Block::Heading { inlines, .. } => {
                links(inlines, &mut found)
            }
            Block::Section { blocks, .. } => found.extend(all_links(blocks)),
            Block::List { items, .. } => {
                for item in items {
                    found.extend(all_links(item));
                }
            }
            _ => {}
        }
    }
    found
}

#[test]
fn uri_link_is_external_link() {
    let content = format!(
        "{}{}",
        show("F1", 12.0, 72.0, 700.0, "Read the guide"),
        show("F1", 12.0, 72.0, 600.0, "See chapter two")
    );
    let page = Page {
        content,
        annots: vec![
            // Over "Read the guide".
            "<< /Type /Annot /Subtype /Link /Rect [70 695 160 715] \
             /A << /S /URI /URI (https://example.org/guide) >> >>"
                .into(),
            // An internal link: the text stays text.
            "<< /Type /Annot /Subtype /Link /Rect [70 595 170 615] /Dest [3 0 R /Fit] >>".into(),
        ],
        ..Page::default()
    };
    let out = read(&pdf(&[page], ""));
    let found = all_links(&out.document.body);
    assert_eq!(
        found,
        [(
            "https://example.org/guide".to_string(),
            false,
            "Read the guide".to_string()
        )]
    );
    assert!(all_text(&out).contains("See chapter two"));
}

#[test]
fn running_header_and_footer_get_roles() {
    let pages: Vec<Page> = (1..=4)
        .map(|n| {
            Page::text(format!(
                "{}{}{}",
                show("F1", 9.0, 72.0, 815.0, "ACME Quarterly Report"),
                show(
                    "F1",
                    11.0,
                    72.0,
                    500.0,
                    &format!("Body text of page {n} goes here, with a few more words to read.")
                ),
                show("F1", 9.0, 280.0, 30.0, &format!("Page {n} of 4")),
            ))
        })
        .collect();
    let out = read(&pdf(&pages, ""));
    for n in 1..=4u32 {
        let blocks = page(&out, n);
        let roles: Vec<(Option<String>, String)> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph { inlines, role, .. } => Some((role.clone(), plain_text(inlines))),
                _ => None,
            })
            .collect();
        assert_eq!(
            roles,
            [
                (Some("page".to_string()), format!("Page {n}")),
                (
                    Some("page-header".to_string()),
                    "ACME Quarterly Report".to_string()
                ),
                (
                    None,
                    format!("Body text of page {n} goes here, with a few more words to read.")
                ),
                (Some("page-footer".to_string()), format!("Page {n} of 4")),
            ],
            "page {n}"
        );
    }
}

#[test]
fn invisible_text_is_reported_not_mixed() {
    let visible_and_hidden = Page::text(format!(
        "{}BT /F1 12 Tf 7 Tr 72 650 Td (Clipped words nobody sees) Tj ET\n\
         BT /F1 12 Tf 3 Tr 72 600 Td (Hidden words on a mixed page) Tj ET\n",
        show("F1", 12.0, 72.0, 700.0, "Visible words on page one."),
    ));
    // A page that is only an invisible text layer, as an OCR program writes it.
    let layer = Page::text(
        "BT /F1 12 Tf 3 Tr 72 700 Td (The quick brown fox jumps over the lazy dog) Tj \
         0 -20 Td (Pack my box with five dozen liquor jugs today) Tj ET\n",
    );
    let out = read(&pdf(&[visible_and_hidden, layer], ""));

    let one = page(&out, 1);
    let body = body_text(one);
    assert!(body.contains("Visible words on page one."), "{body}");
    assert!(!body.contains("Clipped words"), "{body}");
    assert!(!body.contains("Hidden words"), "{body}");
    match one.last() {
        Some(Block::Callout {
            kind: CalloutKind::Details,
            title: Some(title),
            folded: Some(true),
            blocks,
            ..
        }) => {
            assert_eq!(title, "Invisible text on this page");
            assert!(text(blocks).contains("Clipped words nobody sees"));
        }
        other => panic!("no invisible text callout: {other:?}"),
    }

    let two = page(&out, 2);
    assert!(!body_text(two).contains("quick brown fox"), "{two:?}");
    match two.last() {
        Some(Block::Callout {
            kind: CalloutKind::Details,
            blocks,
            ..
        }) => {
            let t = text(blocks);
            assert!(
                t.contains("The quick brown fox jumps over the lazy dog"),
                "{t}"
            );
            assert!(
                t.contains("Pack my box with five dozen liquor jugs today"),
                "{t}"
            );
        }
        other => panic!("no invisible text callout: {other:?}"),
    }

    let infos: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.severity == Severity::Info && l.what.contains("invisible"))
        .map(|l| l.location.clone())
        .collect();
    assert_eq!(infos, ["p1", "p1", "p2"], "{:?}", out.document.losses);
    // The mode 3 run on the mixed page is not given back by the text parser: it is counted.
    assert!(
        out.document.losses.iter().any(|l| l.location == "p1"
            && l.severity == Severity::Info
            && l.what
                .starts_with("1 run of invisible text (render mode 3) not read")),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn refused_page_is_skipped_with_loss() {
    // Page 2's font reads a CMap that unpacks to 16 MiB, over the 8 MiB stream limit.
    let pages = vec![
        Page::text(show("F1", 12.0, 72.0, 700.0, "First page text.")),
        Page {
            content: show("F1", 12.0, 72.0, 700.0, "Second page text."),
            packed_cmap: Some(zeros_packed(16 << 20)),
            ..Page::default()
        },
    ];
    let limits = Limits {
        max_unpacked_bytes: 8 << 20,
        ..Limits::default()
    };
    let out = read_with(&pdf(&pages, ""), limits);
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(!all_text(&out).contains("Second page text."));
    // The other page is read.
    assert!(text(page(&out, 1)).contains("First page text."));
    let refused: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.location == "p2" && l.why == LossReason::Limit)
        .collect();
    assert_eq!(refused.len(), 1, "{:?}", out.document.losses);
    assert!(refused[0].what.contains("not read"), "{:?}", refused[0]);
    // Both page sections are there, each with its marker.
    assert_eq!(marker(page(&out, 1)), "Page 1");
    assert_eq!(marker(page(&out, 2)), "Page 2");
}

#[test]
fn cmap_bomb_on_a_refused_page_stays_out_of_the_text_parser() {
    // Page 1's font reads a CMap that unpacks to 2 GiB; page 2 is plain.
    let pages = vec![
        Page {
            content: show("F1", 12.0, 72.0, 700.0, "Bomb page text."),
            packed_cmap: Some(zeros_packed(2 << 30)),
            ..Page::default()
        },
        Page::text(show("F1", 12.0, 72.0, 700.0, "Plain page text.")),
    ];
    let out = read(&pdf(&pages, ""));
    assert!(text(page(&out, 2)).contains("Plain page text."));
    assert!(!all_text(&out).contains("Bomb page text."));
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.location == "p1" && l.why == LossReason::Limit),
        "{:?}",
        out.document.losses
    );
    let rss = common::max_rss_bytes();
    assert!(rss < 512 << 20, "max RSS {rss}");
}

#[test]
fn unsafe_page_tree_is_not_given_to_the_text_parser() {
    // One page listed twice in /Kids.
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream(
        "",
        show("F1", 12.0, 72.0, 700.0, "Twice listed.").as_bytes(),
    );
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R {kid} 0 R] /Count 2 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(!all_text(&out).contains("Twice listed."));
    assert!(
        out.document.losses.iter().any(|l| l.location == "document"
            && l.why == LossReason::Damaged
            && l.severity == Severity::Error
            && l.what.contains("page tree")),
        "{:?}",
        out.document.losses
    );
    let sections: Vec<&Block> = out
        .document
        .body
        .iter()
        .filter(|b| matches!(b, Block::Section { .. }))
        .collect();
    assert!(!sections.is_empty());
    for (i, _) in sections.iter().enumerate() {
        let blocks = page(&out, i as u32 + 1);
        assert_eq!(blocks.len(), 1);
        assert_eq!(marker(blocks), format!("Page {}", i + 1));
    }
}

#[test]
fn password_protected_file_is_not_read() {
    let out = read(&corpus("qpdf-user-password.pdf"));
    assert_eq!(out.completeness, Completeness::Failed);
    assert_eq!(
        all_text(&out),
        "This PDF is protected with a password and is not read.\n"
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Encrypted && l.severity == Severity::Error)
    );
}

#[test]
fn wrong_password_is_said() {
    let opts = ReadOptions {
        password: Some("not the password".into()),
        ..ReadOptions::default()
    };
    let out = PdfReader.read(
        &source(&corpus("qpdf-user-password.pdf")),
        &opts,
        &Budget::new(Limits::default()),
    );
    assert_eq!(out.completeness, Completeness::Failed);
    assert_eq!(
        all_text(&out),
        "The given password does not open this PDF.\n"
    );
}

#[test]
fn not_a_pdf_fails_with_damaged_loss() {
    let out = read(b"%PDF-1.7\nthis is not a pdf at all\n%%EOF\n");
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged && l.severity == Severity::Error),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn reader_id_and_formats() {
    assert_eq!(PdfReader.id(), "pdf");
    assert_eq!(PdfReader.formats(), &[FormatId::Pdf]);
}
