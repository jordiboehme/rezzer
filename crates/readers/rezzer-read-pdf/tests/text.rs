// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

mod common;

use std::path::PathBuf;

use common::{Builder, Page, all_text, body_text, pdf, show, text, zeros_packed};
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Block, CalloutKind, Completeness, FormatId, Inline, LossReason, ReadOptions,
    ReadOutcome, Reader, SectionKind, Severity, StatusColor, plain_text,
};
use rezzer_read_pdf::PdfReader;

/// The allocator of the Rezzer binary: the memory these tests measure is the memory it
/// ships with.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

fn corpus(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/corpus/pdf");
    std::fs::read(path.join(name)).expect("corpus file")
}

fn read_with(bytes: &[u8], limits: Limits) -> ReadOutcome {
    common::read_in(bytes, &ReadOptions::default(), &Budget::new(limits))
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

/// A page of two columns of numbered lines, each column one text that flows from its top to
/// its bottom past whatever is drawn across the page between `gap` (top and bottom of a
/// stretch where no line stands, or none): `drawn` is added to the page.
fn two_column_page(gap: Option<(f32, f32)>, drawn: &str) -> String {
    let mut c = String::from(drawn);
    for (side, x) in [("L", 56.0), ("R", 310.0)] {
        let mut y = 760.0;
        for k in 1..=40 {
            if let Some((top, bottom)) = gap
                && y < top
                && y > bottom
            {
                y = bottom;
            }
            if y < 80.0 {
                break;
            }
            c += &format!(
                "BT /F1 10 Tf {x} {y} Td (Zeile {side}{k:02} der Spalte mit Text) Tj ET\n"
            );
            y -= 12.0;
        }
    }
    c
}

/// Whether the numbered lines of [`two_column_page`] around a drawing across both columns
/// between lines 20 and 21 of each column read in bands: the lines above it column by column,
/// then the lines under it column by column, each column in its order.
fn in_bands(t: &str) -> bool {
    let at = |side: &str, k: usize| t.find(&format!("Zeile {side}{k:02} "));
    let band = |from: usize, to: usize| -> Option<(usize, usize)> {
        let mut seen = Vec::new();
        for side in ["L", "R"] {
            for k in from..=to {
                if let Some(p) = at(side, k) {
                    seen.push(p);
                }
            }
        }
        let ordered = seen.len() >= 2 && seen.windows(2).all(|w| w[0] < w[1]);
        ordered.then(|| (seen[0], *seen.last().expect("a line")))
    };
    matches!((band(1, 20), band(21, 40)), (Some(above), Some(below)) if above.1 < below.0)
}

/// A framed figure across both columns in the middle of the page, with no text across the
/// gutter: the frame's top and bottom part the columns into bands, so the lines above it are
/// read column by column, then the lines under it. Column flow past a figure is read in bands
/// until text evidence decides it (a word or a sentence that goes on in the column under the
/// figure): two-column reports with a figure across the page read so, and their columns may
/// as well flow past it.
#[test]
fn columns_past_a_framed_figure_across_them_read_in_bands() {
    let frame = "q 0.5 w 56 390 484 130 re S Q\n";
    let out = read(&pdf(
        &[Page::text(two_column_page(Some((532.0, 380.0)), frame))],
        "",
    ));
    let t = all_text(&out);
    assert!(in_bands(&t), "{t}");
}

/// The same with the frame drawn as four lines that meet.
#[test]
fn columns_past_a_figure_framed_by_four_lines_read_in_bands() {
    let frame = "q 0.5 w 56 390 m 540 390 l S 56 520 m 540 520 l S \
                 56 390 m 56 520 l S 540 390 m 540 520 l S Q\n";
    let out = read(&pdf(
        &[Page::text(two_column_page(Some((532.0, 380.0)), frame))],
        "",
    ));
    let t = all_text(&out);
    assert!(in_bands(&t), "{t}");
}

/// A framed chart across both columns with one rule across it and one up it inside the frame
/// (a grid with rules inside, but no text in its cells): it parts the columns into bands as a
/// framed figure does.
#[test]
fn columns_past_a_framed_chart_with_inner_rules_read_in_bands() {
    let frame = "q 0.5 w 56 390 484 130 re S 56 455 m 540 455 l S 298 390 m 298 520 l S Q\n";
    let out = read(&pdf(
        &[Page::text(two_column_page(Some((532.0, 380.0)), frame))],
        "",
    ));
    let t = all_text(&out);
    assert!(in_bands(&t), "{t}");
}

/// Guard: a shaded band across both columns behind their text is a box the text in it is read
/// in, as it was before drawn rules parted bands of columns: the columns above it, then the
/// lines in it, then the columns under it, each column by column. Its edges add no band of
/// their own.
#[test]
fn columns_read_through_a_shaded_band_across_them() {
    let band = "q 0.9 g 50 400 496 100 re f Q\n";
    let out = read(&pdf(&[Page::text(two_column_page(None, band))], ""));
    let t = all_text(&out);
    let order = [
        "Zeile L01 ",
        "Zeile L22 ",
        "Zeile R01 ",
        "Zeile R22 ",
        "Zeile L23 ",
        "Zeile L31 ",
        "Zeile R23 ",
        "Zeile R31 ",
        "Zeile L32 ",
        "Zeile L40 ",
        "Zeile R32 ",
        "Zeile R40 ",
    ];
    let at: Vec<Option<usize>> = order.iter().map(|w| t.find(w)).collect();
    assert!(
        at.iter().all(Option::is_some) && at.windows(2).all(|w| w[0] < w[1]),
        "{t}"
    );
}

/// Guard: a ruled grid of two columns of fields across a page (a rule under each row of fields
/// over both columns, the grid's sides and a rule between the columns) is a table, read row by
/// row as before drawn rules parted bands of columns; its rules add no band of their own.
#[test]
fn a_ruled_grid_of_two_field_columns_stays_a_table() {
    let mut drawn = String::from("q 0.5 w\n");
    for k in 0..=10 {
        let y = 772.0 - 12.0 * k as f32;
        drawn += &format!("50 {y} m 546 {y} l S\n");
    }
    for x in [50.0, 298.0, 546.0] {
        drawn += &format!("{x} 652 m {x} 772 l S\n");
    }
    drawn += "Q\n";
    let mut c = drawn;
    for (side, x) in [("L", 56.0), ("R", 310.0)] {
        for k in 1..=10 {
            let y = 762.0 - 12.0 * (k - 1) as f32;
            c += &format!("BT /F1 10 Tf {x} {y} Td (Zeile {side}{k:02} Feld) Tj ET\n");
        }
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let t = all_text(&out);
    for k in 1..=10 {
        assert!(
            t.contains(&format!("Zeile L{k:02} Feld|Zeile R{k:02} Feld|")),
            "{t}"
        );
    }
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
            assert!(text(blocks).contains("Hidden words on a mixed page"));
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
    assert_eq!(infos, ["p1", "p2"], "{:?}", out.document.losses);
    // The mode 3 run on the mixed page is read into the callout: nothing is left unread.
    assert!(
        out.document
            .losses
            .iter()
            .all(|l| !l.what.contains("not read")),
        "{:?}",
        out.document.losses
    );
}

/// Two hidden texts that touch stay two texts, as on the page: an amount written over the
/// `0,00` of its field is not read as one number. Words of one text with a gap keep the
/// space the reader gave them.
#[test]
fn hidden_texts_that_touch_stay_apart() {
    let hidden = Page::text(format!(
        "{}BT 3 Tr /F1 10 Tf 1 0 0 1 100 650 Tm (0,00) Tj ET\n\
         BT 3 Tr /F1 10 Tf 1 0 0 1 100 650 Tm (1.234,56) Tj ET\n\
         BT 3 Tr /F1 10 Tf 1 0 0 1 100 600 Tm (Erika) Tj 1 0 0 1 130 600 Tm (Mustermann) Tj ET\n",
        show("F1", 12.0, 72.0, 700.0, "Visible words on page one."),
    ));
    let out = read(&pdf(&[hidden], ""));
    let lines: Vec<String> = match page(&out, 1).last() {
        Some(Block::Callout {
            kind: CalloutKind::Details,
            blocks,
            ..
        }) => blocks
            .iter()
            .map(|b| text(std::slice::from_ref(b)).trim().to_string())
            .collect(),
        other => panic!("no invisible text callout: {other:?}"),
    };
    assert_eq!(lines, ["0,00 1.234,56", "Erika Mustermann"]);
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
    let out = common::read_in(
        &corpus("qpdf-user-password.pdf"),
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

#[test]
fn link_covers_only_its_box() {
    // One line drawn as one text object; the link box covers only the address in it.
    let page = Page {
        content: "BT /F1 12 Tf 72 700 Td [(see ) -300 (www.example.org) -300 ( now)] TJ ET\n"
            .into(),
        annots: vec![
            "<< /Type /Annot /Subtype /Link /Rect [96 690 196 715] \
             /A << /S /URI /URI (http://www.example.org) >> >>"
                .into(),
        ],
        ..Page::default()
    };
    let out = read(&pdf(&[page], ""));
    assert_eq!(
        all_links(&out.document.body),
        [(
            "http://www.example.org".to_string(),
            false,
            "www.example.org".to_string()
        )]
    );
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    assert!(
        md.contains("see [www.example.org](http://www.example.org) now"),
        "{md}"
    );
}

/// A one-page file with Helvetica in WinAnsiEncoding as `/F1`, where byte 240 (octal) is a
/// no-break space.
fn win_ansi_page(content: &str) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font =
        b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    let c = b.stream("", content.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn nbsp_padding_splits_pieces() {
    // Label and value lines, each one string: the label, no-break spaces as padding, the
    // value.
    let pad = |n: usize| "\\240".repeat(n);
    let mut c = String::new();
    for (k, (label, n, value)) in [
        ("Points:", 12, "1200"),
        ("Level:", 13, "Gold"),
        ("Status:", 12, "Active"),
    ]
    .into_iter()
    .enumerate()
    {
        let y = 700.0 - 14.0 * k as f32;
        c += &format!("BT /F1 12 Tf 72 {y} Td ({label}{}{value}) Tj ET\n", pad(n));
    }
    let out = read(&win_ansi_page(&c));
    let all = all_text(&out);
    assert!(!all.contains('\u{a0}'), "{all:?}");
    // Label and value stand apart: three lines of two pieces in columns are a table.
    assert_eq!(
        all, "Page 1\nPoints:|1200|\nLevel:|Gold|\nStatus:|Active|\n",
        "{all:?}"
    );
}

/// Lines of five words at 10 points from `top` down, the word spacing of line `l` is `tw(l)`.
fn justified(x: f32, top: f32, lines: usize, seed: usize, tw: impl Fn(usize) -> f32) -> String {
    let vocab = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
        "juliet", "kilo", "lima", "mike", "november",
    ];
    let mut c = String::new();
    for l in 0..lines {
        let words: Vec<&str> = (0..5)
            .map(|k| vocab[(seed + l * 5 + k) % vocab.len()])
            .collect();
        c += &format!(
            "BT /F1 10 Tf {} Tw {x} {} Td ({}) Tj ET\n",
            tw(l),
            top - 12.0 * l as f32,
            words.join(" ")
        );
    }
    c
}

#[test]
fn loose_justified_columns_read_column_by_column() {
    // Two columns of justified prose with wide word spacing (5.5 points at 10 points, wider
    // than the piece gap from the ink, narrower from the end of the space), then the same
    // with only every third line loose.
    let left = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike \
                november alpha";
    for tw in [
        |_: usize| 5.5f32,
        |l: usize| if l.is_multiple_of(3) { 6.0 } else { 1.0 },
    ] {
        let c = justified(50.0, 760.0, 20, 0, tw) + &justified(320.0, 760.0, 20, 3, tw);
        let out = read(&pdf(&[Page::text(c)], ""));
        let all = all_text(&out);
        assert!(all.contains(left), "{all}");
    }
}

#[test]
fn double_spaces_inside_a_line_stay() {
    // Two spaces between words open no piece gap: they stay as drawn.
    let c = "BT /F1 10 Tf 50 760 Td (let  x  =  1;) Tj ET\n";
    let out = read(&pdf(&[Page::text(c)], ""));
    assert!(
        all_text(&out).contains("let  x  =  1;"),
        "{}",
        all_text(&out)
    );
}

#[test]
fn a_link_box_that_cuts_a_word_takes_the_whole_word() {
    // The box covers most of the address only: the link reaches the word's edges.
    let page = Page {
        content: show("F1", 12.0, 72.0, 700.0, "see www.example.org now"),
        annots: vec![
            "<< /Type /Annot /Subtype /Link /Rect [92 696 173 712] \
             /A << /S /URI /URI (http://www.example.org) >> >>"
                .into(),
        ],
        ..Page::default()
    };
    let out = read(&pdf(&[page], ""));
    assert_eq!(
        all_links(&out.document.body),
        [(
            "http://www.example.org".to_string(),
            false,
            "www.example.org".to_string()
        )]
    );
    // A box that ends after "www.exam": the link reaches the end of the word, but not the
    // full stop after it.
    let page = Page {
        content: show("F1", 12.0, 72.0, 700.0, "see www.example.org."),
        annots: vec![
            "<< /Type /Annot /Subtype /Link /Rect [92 696 150 712] \
             /A << /S /URI /URI (http://www.example.org) >> >>"
                .into(),
        ],
        ..Page::default()
    };
    let out = read(&pdf(&[page], ""));
    assert_eq!(
        all_links(&out.document.body),
        [(
            "http://www.example.org".to_string(),
            false,
            "www.example.org".to_string()
        )]
    );
}

/// A ToUnicode CMap, deflated, that maps the bytes A to Z to the Hebrew letters from U+05D0 on
/// and every other printable byte and the space to itself.
fn hebrew_cmap() -> Vec<u8> {
    use std::io::Write;
    let mut chars: Vec<String> = Vec::new();
    for c in 0x20u8..=0x7e {
        let u = if c.is_ascii_uppercase() {
            0x05d0 + u32::from(c - b'A')
        } else {
            u32::from(c)
        };
        chars.push(format!("<{c:02X}> <{u:04X}>"));
    }
    let mut body = String::new();
    for chunk in chars.chunks(90) {
        body += &format!(
            "{} beginbfchar\n{}\nendbfchar\n",
            chunk.len(),
            chunk.join("\n")
        );
    }
    let cmap = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /X def \
         1 begincodespacerange <00> <FF> endcodespacerange\n{body}endcmap CMapName \
         currentdict /CMap defineresource pop end end\n"
    );
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(cmap.as_bytes()).expect("deflate");
    z.finish().expect("deflate")
}

#[test]
fn a_link_in_a_right_to_left_line_stays_on_its_word() {
    // A Hebrew line in visual order, a link box over two letters of its leftmost word; then
    // the same line drawn glyph by glyph from right to left. The link covers that word (the
    // last one in reading order), not the line.
    let word = "ABCDEFGH IJK LMN";
    let mut reverse = String::from("BT /F1 12 Tf ");
    let mut x = 72.0f32;
    let mut xs = Vec::new();
    for ch in word.chars() {
        xs.push((x, ch));
        x += if ch == ' ' { 3.34 } else { 8.0 };
    }
    for (x, ch) in xs.iter().rev() {
        reverse += &format!("1 0 0 1 {x} 700 Tm ({ch}) Tj ");
    }
    reverse += "ET\n";
    for content in [show("F1", 12.0, 72.0, 700.0, word), reverse] {
        let page = Page {
            content,
            annots: vec![
                "<< /Type /Annot /Subtype /Link /Rect [90 696 110 712] \
                 /A << /S /URI /URI (http://example.org) >> >>"
                    .into(),
            ],
            packed_cmap: Some(hebrew_cmap()),
        };
        let out = read(&pdf(&[page], ""));
        let links = all_links(&out.document.body);
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(links[0].0, "http://example.org");
        assert_eq!(
            links[0].2, "\u{5d7}\u{5d6}\u{5d5}\u{5d4}\u{5d3}\u{5d2}\u{5d1}\u{5d0}",
            "{links:?}"
        );
    }
}

#[test]
fn padding_in_a_right_to_left_line_is_one_space() {
    // A Hebrew label, twelve spaces of padding, a Hebrew value, in one string.
    let page = Page {
        content: show("F1", 12.0, 72.0, 700.0, "ABC:            DEF"),
        packed_cmap: Some(hebrew_cmap()),
        ..Page::default()
    };
    let out = read(&pdf(&[page], ""));
    let all = all_text(&out);
    // The glyphs stand in visual order: the value, read first, then the label.
    assert!(
        all.contains("\u{5d5}\u{5d4}\u{5d3} :\u{5d2}\u{5d1}\u{5d0}"),
        "{all:?}"
    );
    assert!(!all.contains("  "), "{all:?}");
}

#[test]
fn an_exact_link_box_stays_exact() {
    // A box exactly over "example.org", before an apostrophe and before a hyphen.
    for (text, rest) in [
        ("see example.org's page", "\u{2019}s page"),
        ("see example.org-based tool", "-based tool"),
    ] {
        let page = Page {
            content: show("F1", 12.0, 72.0, 700.0, text),
            annots: vec![
                "<< /Type /Annot /Subtype /Link /Rect [94 696 161.3 712] \
                 /A << /S /URI /URI (http://example.org) >> >>"
                    .into(),
            ],
            ..Page::default()
        };
        let out = read(&pdf(&[page], ""));
        let links = all_links(&out.document.body);
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(links[0].2, "example.org", "{links:?}");
        assert!(all_text(&out).contains(rest), "{}", all_text(&out));
    }
}

#[test]
fn a_link_over_the_end_of_an_actual_text_span_links_the_span() {
    // The span's text stands on its first glyph; the box covers only its later glyphs.
    let page = Page {
        content: "BT /F1 12 Tf 72 700 Td (see ) Tj /Span << /ActualText (www.example.org) >> \
                  BDC (xxx.yyyyyyy.zzz) Tj EMC ( now) Tj ET\n"
            .into(),
        annots: vec![
            "<< /Type /Annot /Subtype /Link /Rect [140 696 178 712] \
             /A << /S /URI /URI (http://www.example.org) >> >>"
                .into(),
        ],
        ..Page::default()
    };
    let out = read(&pdf(&[page], ""));
    assert_eq!(
        all_links(&out.document.body),
        [(
            "http://www.example.org".to_string(),
            false,
            "www.example.org".to_string()
        )]
    );
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("not placed")),
        "{:?}",
        out.document.losses
    );
}

/// A box from `(x0, y0)` to `(x1, y1)` with corners rounded by `r` (curves), stroked.
fn rounded_box(x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> String {
    let k = 0.552 * r;
    format!(
        "{a} {y0} m {b} {y0} l {c} {y0} {x1} {d} {x1} {e} c {x1} {f} l {x1} {g} {c} {y1} {b} {y1} c \
         {a} {y1} l {h} {y1} {x0} {g} {x0} {f} c {x0} {e} l {x0} {d} {h} {y0} {a} {y0} c h S\n",
        a = x0 + r,
        b = x1 - r,
        c = x1 - r + k,
        d = y0 + r - k,
        e = y0 + r,
        f = y1 - r,
        g = y1 - r + k,
        h = x0 + r - k,
    )
}

#[test]
fn rounded_boxes_side_by_side() {
    // Two rounded boxes side by side, 4 and 5 short lines (invented sender and document
    // facts). First at offset baselines, then with the lines of both boxes on the same
    // baselines, where only the boxes keep them apart. Each box keeps its lines.
    let left = ["Sample Air Ltd", "Main Road 1", "1000 Town", "Country"];
    let right = [
        "Invoice 4711",
        "Date 2030-01-02",
        "Customer 12345",
        "Page 1 of 1",
        "Currency EUR",
    ];
    for (pitch, start) in [(11.6f32, 703.0f32), (14.0, 700.0)] {
        let mut c = String::from("0.8 w ");
        c += &rounded_box(40.0, 600.0, 240.0, 720.0, 10.0);
        c += &rounded_box(300.0, 600.0, 520.0, 720.0, 10.0);
        for (k, t) in left.iter().enumerate() {
            c += &show("F1", 10.0, 50.0, 700.0 - 14.0 * k as f32, t);
        }
        for (k, t) in right.iter().enumerate() {
            c += &show("F1", 10.0, 310.0, start - pitch * k as f32, t);
        }
        let out = read(&pdf(&[Page::text(c)], ""));
        let blocks = page(&out, 1);
        assert!(
            !blocks.iter().any(|b| matches!(b, Block::Table { .. })),
            "{blocks:#?}"
        );
        assert!(
            !out.document
                .losses
                .iter()
                .any(|l| l.what.contains("Possible table")),
            "{:?}",
            out.document.losses
        );
        // Each box keeps its line breaks.
        let md = rezzer_render::render(
            &out.document,
            out.completeness,
            &rezzer_render::RenderOptions::default(),
        );
        for lines in [&left[..], &right[..]] {
            let broken = lines.join("\\\n");
            assert!(md.contains(&format!("\n{broken}\n")), "{broken} in {md}");
        }
        // Box by box, each box whole and in its order.
        let body = body_text(blocks);
        let at = |t: &str| body.find(t).unwrap_or_else(|| panic!("{t} in {body}"));
        let mut last = 0;
        for t in left.iter().chain(right.iter()) {
            assert!(at(t) >= last, "{t} out of order in {body}");
            last = at(t);
        }
    }
}

#[test]
fn known_limit_a_side_heading_lands_inside_the_question_above_it() {
    // A known limit, pinned so a later change notices: on the tax form's first page the
    // sidebar heading "Dependents" stands at other baselines than the form text beside it, so
    // the sidebar is read as a column of its own. The sidebar label "Digital Assets" and the
    // first line of the question beside it are one piece that crosses the gutter, so the
    // heading is read between the two lines of that question. A cut at the top of each side
    // heading fixes this here but mixes the instructions with the sample check of the direct
    // deposit form again, so it is not made.
    let out = read(&corpus("irs-f1040-2025.pdf"));
    let body = text(page(&out, 1));
    let at = |t: &str| body.find(t).unwrap_or_else(|| panic!("{t} in {body}"));
    let (first, heading, second) = (
        at("(b) sell,"),
        at("Dependents"),
        at("exchange, or otherwise"),
    );
    assert!(first < heading && heading < second, "{body}");
}

fn markdown_of(out: &ReadOutcome) -> String {
    rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    )
}

#[test]
fn short_and_aligned_lines_keep_their_breaks() {
    // A paragraph whose second line ends where the next line's first word would still have
    // fit: a break. A block of lines set flush right: a break after each. Ragged prose that
    // fills its measure: one paragraph of joined lines.
    let mut c = String::new();
    for (k, t) in [
        "A long first line of the text sets the measure of this paragraph",
        "Short line.",
        "Next line goes on to the end",
    ]
    .iter()
    .enumerate()
    {
        c += &show("F1", 10.0, 72.0, 760.0 - 12.0 * k as f32, t);
    }
    for (k, t) in ["Sample Hotel", "Lake Road 7", "2000 Village"]
        .iter()
        .enumerate()
    {
        let x = 520.0 - common::helvetica_width(t, 10.0);
        c += &show("F1", 10.0, x, 660.0 - 12.0 * k as f32, t);
    }
    for (k, t) in ["Johann von der Heide", "Am Markt 3", "20095 Hamburg"]
        .iter()
        .enumerate()
    {
        c += &show("F1", 10.0, 72.0, 610.0 - 12.0 * k as f32, t);
    }
    for (k, t) in [
        "Prose that runs from one line to the next fills the measure of",
        "its column, and the next word never fits at the end of a line,",
        "so the lines of the paragraph are joined into one text again as",
        "they were written.",
    ]
    .iter()
    .enumerate()
    {
        c += &show("F1", 10.0, 72.0, 560.0 - 12.0 * k as f32, t);
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let md = markdown_of(&out);
    assert!(
        md.contains(
            "A long first line of the text sets the measure of this paragraph Short line.\\\n\
             Next line goes on to the end\n"
        ),
        "{md}"
    );
    assert!(
        md.contains("\nSample Hotel\\\nLake Road 7\\\n2000 Village\n"),
        "{md}"
    );
    assert!(
        md.contains("\nJohann von der Heide\\\nAm Markt 3\\\n20095 Hamburg\n"),
        "{md}"
    );
    assert!(
        md.contains(
            "\nProse that runs from one line to the next fills the measure of its column, and \
             the next word never fits at the end of a line, so the lines of the paragraph are \
             joined into one text again as they were written.\n"
        ),
        "{md}"
    );
}

#[test]
fn running_footers_are_found_on_turned_pages() {
    // Three pages whose text is drawn upward, each with the same footer line at the bottom of
    // the turned frame: the footer is a running footer, not body text.
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut kids = Vec::new();
    for n in 1..=3 {
        let content = format!(
            "q 0 1 -1 0 595 0 cm BT /F1 10 Tf 72 500 Td (Body text of page {n}.) Tj ET \
             BT /F1 8 Tf 72 20 Td (Sample Company - confidential) Tj ET Q\n"
        );
        let c = b.stream("", content.as_bytes());
        kids.push(b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /Rotate 90 \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
        )));
    }
    let refs: Vec<String> = kids.iter().map(|k| format!("{k} 0 R")).collect();
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{}] /Count 3 >>", refs.join(" ")),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    for n in 1..=3 {
        let footer = page(&out, n).iter().any(|b| {
            matches!(b, Block::Paragraph { inlines, role, .. }
                if role.as_deref() == Some("page-footer")
                    && plain_text(inlines).contains("confidential"))
        });
        assert!(footer, "page {n}: {:#?}", page(&out, n));
    }
}

#[test]
fn narrow_german_prose_keeps_joining() {
    // Paragraphs of German prose in a narrow column: nouns start with capitals, so every line
    // does, yet the lines of each paragraph are one text, whatever its last word (a small
    // word, a year, an abbreviation in capitals, a capital word part, or none: the paragraph
    // goes on in the next column).
    let paras = [
        [
            "Am Montag hat der Vorstand die neue",
            "Satzung beschlossen und den Termin der",
            "Mitgliederversammlung auf Mai gelegt.",
        ],
        [
            "Die Kasse wurde von zwei Mitgliedern",
            "Geprueft und ohne Beanstandung",
            "Entlastet, wie es die Satzung will.",
        ],
        [
            "Am Montag hat der Vorstand die neue",
            "Satzung beschlossen und den Termin der",
            "Mitgliederversammlung auf den 5. Mai 2027.",
        ],
        [
            "Die Kasse wurde von zwei Mitgliedern",
            "Geprueft und ohne Beanstandung an die",
            "Revisoren der Sparkasse Musterstadt AG.",
        ],
        [
            "Der neue Beitrag gilt ab dem Januar",
            "Fuer alle Mitglieder laut Beschluss",
            "Nummer 12 der Versammlung im EU-Raum.",
        ],
        [
            "Die Mitglieder haben ueber die neue",
            "Satzung und den Haushalt des Vereins",
            "Abgestimmt, und der Vorstand hat die",
        ],
    ];
    let mut c = String::new();
    let mut y = 760.0;
    for para in &paras {
        for line in para {
            c += &show("F1", 10.0, 72.0, y, line);
            y -= 12.0;
        }
        y -= 10.0;
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let md = markdown_of(&out);
    for para in &paras {
        let joined = format!("\n{}\n", para.join(" "));
        assert!(md.contains(&joined), "{joined} in {md}");
    }
}

#[test]
fn two_centred_lines_keep_their_break() {
    // A title block of two lines centred on the page: a break between them.
    let mut c = String::new();
    for (k, t) in ["Sample Research Group", "Lake Town, Country"]
        .iter()
        .enumerate()
    {
        let x = 297.5 - common::helvetica_width(t, 10.0) / 2.0;
        c += &show("F1", 10.0, x, 700.0 - 12.0 * k as f32, t);
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let md = markdown_of(&out);
    assert!(
        md.contains("\nSample Research Group\\\nLake Town, Country\n"),
        "{md}"
    );
}

#[test]
fn two_lines_set_flush_right_keep_their_break() {
    // A name over its register line at the head of a letter, set flush right: two short
    // lines in the shape of an address are two lines. The same edges with wording that shows
    // a wrap are one text: a line that ends in a comma, and two lines that read as a sentence.
    let block = |lines: [&str; 2]| {
        let mut c = String::new();
        for (k, t) in lines.iter().enumerate() {
            let x = 560.0 - common::helvetica_width(t, 10.0);
            c += &show("F1", 10.0, x, 700.0 - 12.0 * k as f32, t);
        }
        markdown_of(&read(&pdf(&[Page::text(c)], "")))
    };
    let md = block(["Sample Supply Ltd", "Register Lake Town 4521"]);
    assert!(
        md.contains("\nSample Supply Ltd\\\nRegister Lake Town 4521\n"),
        "{md}"
    );
    let md = block([
        "The old lamp near the door,",
        "Which nobody has lit for years",
    ]);
    assert!(
        md.contains("\nThe old lamp near the door, Which nobody has lit for years\n"),
        "{md}"
    );
    let md = block([
        "Nobody has lit the lamp for",
        "Years. The room stays dark and cold.",
    ]);
    assert!(
        md.contains("\nNobody has lit the lamp for Years. The room stays dark and cold.\n"),
        "{md}"
    );
}

#[test]
fn very_narrow_german_prose_keeps_joining() {
    // German prose in a column of three or four words a line, every line starting with a
    // capital: its lines hold words in small letters, an address does not, so each paragraph
    // is one text.
    let paras: [&[&str]; 3] = [
        &[
            "Der Vorstand hat am",
            "Montag die Satzung",
            "Beschlossen, gueltig ab",
            "Januar 2027.",
        ],
        &[
            "Die Kasse pruefen",
            "Mitglieder der Sparkasse",
            "Musterstadt AG.",
        ],
        &[
            "Die neue Halle",
            "Wird im Fruehjahr",
            "Gebaut und kostet",
            "Rund 40.000 Euro",
        ],
    ];
    let mut c = String::new();
    let mut y = 700.0;
    for para in &paras {
        for line in *para {
            c += &show("F1", 10.0, 72.0, y, line);
            y -= 12.0;
        }
        y -= 10.0;
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let md = markdown_of(&out);
    for para in &paras {
        let joined = format!("\n{}\n", para.join(" "));
        assert!(md.contains(&joined), "{joined} in {md}");
    }
}

/// Guard: prose whose first lines start with two space glyphs (an indent typed as spaces),
/// and a line inside a paragraph that starts with one, keeps its paragraphs and its words.
#[test]
fn indented_prose_keeps_its_paragraphs() {
    let paragraphs = [
        vec![
            "  The payroll office of Muster GmbH sends the statement at the end",
            "of every month to each employee by post, together with the notes",
            " on taxes and social insurance that apply to the month in question,",
            "and keeps a copy of every statement for the records of the firm.",
        ],
        vec![
            "  Erika Mustermann reads the statement and checks the amounts, the",
            "days of leave and the hours worked against her own notes for the",
            "month before she signs the form and sends it back to the office.",
        ],
    ];
    let mut c = String::new();
    let mut y = 760.0;
    for lines in &paragraphs {
        for l in lines {
            c += &show("F1", 10.0, 72.0, y, l);
            y -= 12.0;
        }
        y -= 12.0;
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let found: Vec<String> = roles_of(&out, 1).into_iter().map(|(_, t)| t).collect();
    let want: Vec<String> = paragraphs
        .iter()
        .map(|lines| lines.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(found, want);
}

/// A page of body text with `extra` drawn on it.
fn page_with(extra: &str) -> Page {
    Page::text(format!(
        "{}{extra}",
        show(
            "F1",
            10.0,
            72.0,
            600.0,
            "The text of the page in plain words, as a letter has it."
        )
    ))
}

/// The footer line of the tests of two-page furniture.
const FIRM_LINE: &str = "Muster GmbH, Musterweg 1, 12345 Musterstadt";

/// A line at the foot of both pages of a file of two pages is a running footer: a letter or
/// invoice of two pages has no third page to repeat it on.
#[test]
fn two_page_footer_is_furniture() {
    let pages: Vec<Page> = (0..2)
        .map(|_| page_with(&show("F1", 8.0, 72.0, 40.0, FIRM_LINE)))
        .collect();
    let out = read(&pdf(&pages, ""));
    for n in 1..=2u32 {
        let roles = roles_of(&out, n);
        assert_eq!(
            roles.last(),
            Some(&(Some("page-footer".into()), FIRM_LINE.into())),
            "page {n}: {roles:?}"
        );
        assert_eq!(
            roles.iter().filter(|(_, t)| t == FIRM_LINE).count(),
            1,
            "page {n}: {roles:?}"
        );
    }
}

/// Guard: a line at the foot of two pages of three is body text: it repeats on too few pages.
#[test]
fn line_on_two_of_three_pages_is_not_furniture() {
    let pages: Vec<Page> = (0..3)
        .map(|n| {
            if n < 2 {
                page_with(&show("F1", 8.0, 72.0, 40.0, FIRM_LINE))
            } else {
                page_with("")
            }
        })
        .collect();
    let out = read(&pdf(&pages, ""));
    for n in 1..=2u32 {
        let roles = roles_of(&out, n);
        // Task 19a-5b: the lone line may be kept as laid out (no region type has clear
        // evidence for one short line); it is body text there too, never furniture.
        let laid_out = page(&out, n).iter().any(|b| match b {
            Block::Spatial { lines, .. } => lines
                .iter()
                .any(|l| l.pieces.iter().any(|p| p.text == FIRM_LINE)),
            _ => false,
        });
        assert!(
            laid_out || roles.iter().any(|(r, t)| r.is_none() && t == FIRM_LINE),
            "page {n}: {roles:?} {:#?}",
            page(&out, n)
        );
        assert!(
            roles
                .iter()
                .all(|(r, _)| r.as_deref() != Some("page-footer")),
            "page {n}: {roles:?}"
        );
    }
}

/// A page number at the foot of every page, far under a page of few lines in the same font,
/// is a running footer: the few lines are not parted by white space (too few to look for
/// bands), so the number shared a leaf with the line above it and was typed with it
/// (arxiv-2408.09869 p7, whose figures are drawn beside them). The running line, apart from
/// the text above by its gap, is cut off.
#[test]
fn page_number_under_few_lines_is_furniture() {
    let figure = "q 0.5 w 320 200 m 540 200 l S 320 300 m 540 300 l S 320 400 m 540 400 l S Q\n";
    let pages: Vec<Page> = (1..=3)
        .map(|n| page_with(&(show("F1", 10.0, 300.0, 40.0, &n.to_string()) + figure)))
        .collect();
    let out = read(&pdf(&pages, ""));
    for n in 1..=3u32 {
        let roles = roles_of(&out, n);
        assert_eq!(
            roles.last(),
            Some(&(Some("page-footer".into()), n.to_string())),
            "page {n}: {roles:?} {:#?}",
            page(&out, n)
        );
    }
}

/// Task 31c: the same page with the page number written with a word, as a count of the
/// pages, or both (`Page 2`, `2 / 3`, `Page 2 of 3`): a running line of the file below all
/// other text, far under the few lines it shares its leaf with, and in the shape of a page
/// number. It is cut off and is a footer, as the bare number is. A line of that place and
/// shape that is no running line of the file (the text differs from page to page in more than
/// its digits) stays with its text.
#[test]
fn a_page_number_with_a_word_or_a_count_under_few_lines_is_furniture() {
    let figure = "q 0.5 w 320 200 m 540 200 l S 320 300 m 540 300 l S 320 400 m 540 400 l S Q\n";
    let read_with = |foot: &dyn Fn(u32) -> String| {
        let pages: Vec<Page> = (1..=3)
            .map(|n| page_with(&(show("F1", 10.0, 280.0, 40.0, &foot(n)) + figure)))
            .collect();
        read(&pdf(&pages, ""))
    };
    let shapes: [&dyn Fn(u32) -> String; 4] = [
        &|n| format!("Page {n}"),
        &|n| format!("{n} / 3"),
        &|n| format!("Page {n} of 3"),
        &|n| format!("{n}/3"),
    ];
    for foot in shapes {
        let out = read_with(foot);
        for n in 1..=3u32 {
            let roles = roles_of(&out, n);
            assert_eq!(
                roles.last(),
                Some(&(Some("page-footer".into()), foot(n))),
                "page {n}: {roles:?}"
            );
        }
    }
    let words = ["Crate", "Barrel", "Sack"];
    let out = read_with(&|n| format!("{} {n}", words[n as usize - 1]));
    for n in 1..=3u32 {
        let roles = roles_of(&out, n);
        assert!(
            roles
                .iter()
                .all(|(r, _)| r.as_deref() != Some("page-footer")),
            "page {n}: {roles:?}"
        );
    }
}

/// The roles and texts of each page of a file of `pages` pages like the page of
/// [`page_number_under_few_lines_is_furniture`], with the line `foot(n)` at the foot of page
/// `n` and `body(n)` over it.
fn foot_roles(
    pages: u32,
    body: &dyn Fn(u32) -> String,
    foot: &dyn Fn(u32) -> String,
) -> Vec<Vec<(Option<String>, String)>> {
    let figure = "q 0.5 w 320 200 m 540 200 l S 320 300 m 540 300 l S 320 400 m 540 400 l S Q\n";
    let all: Vec<Page> = (1..=pages)
        .map(|n| page_with(&(show("F1", 10.0, 280.0, 40.0, &foot(n)) + figure + &body(n))))
        .collect();
    let out = read(&pdf(&all, ""));
    (1..=pages).map(|n| roles_of(&out, n)).collect()
}

/// Fix round 1 of part 2b-2 of Task 31c (I2): a line at the foot of every page with a word
/// and a number is a value, not a page number, when its number does not follow the pages. A
/// total of each page (`Total 120`, `Total 140`, `Total 160`), the same total on every page
/// and an amount on both pages of a file of two (`Amount due: 120`) are running lines by
/// their text with the digits taken out, below all other text and far under it, and have the
/// shape of a page number: they got the role of a page footer. They stay text.
#[test]
fn a_value_at_the_foot_of_every_page_is_no_page_number() {
    let cases: [(u32, &dyn Fn(u32) -> String); 6] = [
        (3, &|n| format!("Total {}", 100 + 20 * n)),
        (3, &|_| "Total 120".into()),
        (2, &|_| "Amount due: 120".into()),
        // A number that goes up by two, one that goes down, and a number over its count.
        (3, &|n| format!("Lot {}", 2 * n)),
        (3, &|n| format!("Step {}", 9 - n)),
        (3, &|n| format!("{} / 3", n + 3)),
    ];
    for (pages, foot) in cases {
        for (k, roles) in foot_roles(pages, &|_| String::new(), foot)
            .iter()
            .enumerate()
        {
            let n = k as u32 + 1;
            assert!(
                roles
                    .iter()
                    .all(|(r, _)| r.as_deref() != Some("page-footer")),
                "page {n}: {roles:?}"
            );
            assert!(
                roles.iter().any(|(r, t)| r.is_none() && *t == foot(n)),
                "page {n}: {roles:?}"
            );
        }
    }
}

/// The page numbers that follow the pages keep the role: from another number than 1 (a file
/// cut out of a longer one), with a count that is the same on every page, and on both pages
/// of a file of two.
#[test]
fn a_page_number_that_follows_the_pages_from_an_offset_is_furniture() {
    let cases: [(u32, &dyn Fn(u32) -> String); 5] = [
        (3, &|n| format!("Page {}", n + 4)),
        (3, &|n| format!("{} / 12", n + 6)),
        (3, &|n| format!("Page {} of 9", n + 6)),
        (2, &|n| format!("Page {n}")),
        (2, &|n| format!("{n} / 2")),
    ];
    for (pages, foot) in cases {
        for (k, roles) in foot_roles(pages, &|_| String::new(), foot)
            .iter()
            .enumerate()
        {
            let n = k as u32 + 1;
            assert_eq!(
                roles.last(),
                Some(&(Some("page-footer".into()), foot(n))),
                "page {n}: {roles:?}"
            );
        }
    }
}

/// A known limit, pinned so that a change of the evidence is seen (fix round 2 of part 2b-2
/// of Task 31c, R3; owner after 0.1.0): a value at the foot of every page that goes up by
/// exactly one from page to page (`Total 121`, `Total 122`, `Total 123`) has all the
/// evidence of a page number that starts at an offset, and gets the role of a page footer.
/// Its text is kept. Nothing on the pages tells the two apart.
#[test]
fn a_value_that_goes_up_by_one_with_the_pages_reads_as_a_page_number() {
    let foot = |n: u32| format!("Total {}", 120 + n);
    for (k, roles) in foot_roles(3, &|_| String::new(), &foot).iter().enumerate() {
        let n = k as u32 + 1;
        assert_eq!(
            roles.last(),
            Some(&(Some("page-footer".into()), foot(n))),
            "page {n}: {roles:?}"
        );
    }
}

/// `7 / 12` as the last row of a table without rules at the foot of every page, under the
/// white space of its empty rows: the same on every page, so it counts no pages and is no
/// page number. It stays text.
#[test]
fn a_count_as_the_last_row_of_a_table_is_no_page_number() {
    let names = [["Crate", "Barrel"], ["Sack", "Basket"], ["Bottle", "Drum"]];
    let table = |n: u32| {
        let [first, second] = names[n as usize - 1];
        let mut rows = String::new();
        for (k, row) in [
            ["Item", "Units", "Share"],
            [first, "4", "2 / 12"],
            [second, "9", "5 / 12"],
        ]
        .iter()
        .enumerate()
        {
            for (cell, x) in row.iter().zip([200.0, 280.0, 340.0]) {
                rows += &show("F1", 10.0, x, 140.0 - 13.0 * k as f32, cell);
            }
        }
        rows
    };
    for pages in [2, 3] {
        let out = foot_roles(pages, &table, &|_| "7 / 12".into());
        for (k, roles) in out.iter().enumerate() {
            assert!(
                roles
                    .iter()
                    .all(|(r, _)| r.as_deref() != Some("page-footer")),
                "page {}: {roles:?}",
                k + 1
            );
        }
    }
}

/// Guard: a letterhead at the top of both pages of a file of two pages stays content: the
/// title and the fields of an invoice repeat on its second page, and they are the reader's to
/// read.
#[test]
fn two_page_letterhead_stays_content() {
    let head = show("F2", 14.0, 72.0, 800.0, "Rechnung")
        + &show("F1", 9.0, 72.0, 784.0, "Rechnungsnummer 0000-0001")
        + &show("F1", 9.0, 72.0, 773.0, "Kundennummer 00000");
    let pages: Vec<Page> = (0..2).map(|_| page_with(&head)).collect();
    let out = read(&pdf(&pages, ""));
    for n in 1..=2u32 {
        let roles = roles_of(&out, n);
        assert!(
            roles
                .iter()
                .all(|(r, _)| r.as_deref() != Some("page-header")),
            "page {n}: {roles:?}"
        );
        let all = roles
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for t in [
            "Rechnung",
            "Rechnungsnummer 0000-0001",
            "Kundennummer 00000",
        ] {
            assert!(all.contains(t), "page {n}: {t} in {roles:?}");
        }
    }
}

#[test]
fn tagged_file_without_a_title_gets_its_heading() {
    let out = read(&corpus("pdfjs-structure-simple.pdf"));
    let p = &out.document.properties;
    assert_eq!(p.title.as_deref(), Some("Heading 1"));
    assert_eq!(
        p.title_from.as_deref(),
        Some("largest heading in the top third of page 1")
    );
}

/// A tagged page: an `H1` in `title_size` pt Helvetica-Bold on top, then a `P` of prose
/// lines in 10 pt.
fn tagged_report(title_size: f32) -> Vec<u8> {
    let mut c = format!(
        "/H1 <</MCID 0>> BDC BT /F2 {title_size} Tf 72 780 Td (Musterbericht 2026) Tj ET EMC\n\
         /P <</MCID 1>> BDC BT /F1 10 Tf 72 700 Td 13 TL\n"
    );
    for _ in 0..12 {
        c.push_str("(die Muster GmbH liefert die bestellte Ware wie vereinbart aus) ' \n");
    }
    c.push_str("ET EMC\n");
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let kid = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let bold = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>");
    let cs = b.stream("", c.as_bytes());
    let root = b.reserve();
    let doc = b.reserve();
    let h1 = b.add(format!(
        "<< /Type /StructElem /S /H1 /P {doc} 0 R /Pg {kid} 0 R /K 0 >>"
    ));
    let p = b.add(format!(
        "<< /Type /StructElem /S /P /P {doc} 0 R /Pg {kid} 0 R /K 1 >>"
    ));
    b.set(
        doc,
        format!("<< /Type /StructElem /S /Document /P {root} 0 R /K [{h1} 0 R {p} 0 R] >>"),
    );
    b.set(
        root,
        format!(
            "<< /Type /StructTreeRoot /K {doc} 0 R /ParentTree << /Nums [0 [{h1} 0 R {p} 0 R]] >> >>"
        ),
    );
    b.set(
        kid,
        format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /StructParents 0 \
             /Resources << /Font << /F1 {font} 0 R /F2 {bold} 0 R >> >> /Contents {cs} 0 R >>"
        ),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R /MarkInfo << /Marked true >> \
             /StructTreeRoot {root} 0 R >>"
        ),
    );
    b.finish(catalog)
}

#[test]
fn tagged_heading_on_top_is_the_title() {
    for size in [14.0f32, 18.0, 24.0] {
        let out = read(&tagged_report(size));
        assert_eq!(out.document.properties.facts.tagged, Some(true));
        assert_eq!(
            out.document.properties.title.as_deref(),
            Some("Musterbericht 2026"),
            "{size} pt"
        );
    }
}

/// The columns of [`tagged_footer`]: a company, its managers under a title (one paragraph of
/// names wrapped over three lines), the chair of its board under a title.
const TAGGED_FOOTER: [&[&str]; 3] = [
    &[
        "Oakfield Rail Ltd",
        "Head office in Oldtown",
        "Registered at Oldtown HRB 12 345",
        "Tax-No. DE123456789",
    ],
    &[
        "Managers",
        "Dr. Robin Sample (Chair), Ann Specimen",
        "(Sales), Alex Other (Staff), Max Sampleton",
        "(Works)",
    ],
    &["Chair of the supervisory board", "Morgan Example"],
];

/// Task 36a part 1b (class D on the tagged path): `pages` tagged pages, each a title and
/// three body paragraphs across the page (in 10 points times `k`) over a footer of three columns without rules
/// at 7 points times `k`, the columns at an even pitch from x 85, the lines 1.2 sizes apart
/// from the baseline 64.85. The pitch is the width of the second line of names less a fifth
/// of a point (that line reaches that little past the start of the third column, where that
/// column is empty). The structure tree holds one paragraph element per line, row by row
/// across the columns, and the page number one line under the footer as the last element.
fn tagged_footer(pages: usize, k: f32) -> Vec<u8> {
    let size = 7.0 * k;
    let step = 1.2 * size;
    // The width with the brackets as the font sets them (333 units; `helvetica_width` counts
    // them as 278).
    let width = |text: &str| {
        let brackets = text.chars().filter(|c| matches!(c, '(' | ')')).count() as f32;
        common::helvetica_width(text, size) + brackets * 55.0 * size / 1000.0
    };
    let pitch = width(TAGGED_FOOTER[1][2]) - 0.2;
    let numbers: Vec<String> = (1..=pages)
        .map(|n| format!("Sheet {n} / {pages}"))
        .collect();
    let sheets: Vec<Vec<common::TaggedLine<'_>>> = numbers
        .iter()
        .map(|number| {
            let mut lines: Vec<common::TaggedLine<'_>> = vec![
                (85.0, 760.0, 14.0 * k, true, "Carriage report"),
                (85.0, 730.0, 10.0 * k, false, BODY[0]),
                (85.0, 730.0 - 14.0 * k, 10.0 * k, false, BODY[1]),
                (85.0, 730.0 - 28.0 * k, 10.0 * k, false, BODY[2]),
            ];
            for row in 0..4 {
                for (c, column) in TAGGED_FOOTER.iter().enumerate() {
                    if let Some(text) = column.get(row) {
                        let x = 85.0 + pitch * c as f32;
                        lines.push((x, 64.85 - step * row as f32, size, false, text));
                    }
                }
            }
            lines.push((521.3, 64.85 - step * 4.0, size, false, number));
            lines
        })
        .collect();
    common::tagged_paragraphs(&sheets)
}

/// The body paragraphs of [`tagged_footer`], each one line across the page.
const BODY: [&str; 3] = [
    "The wagons of the morning run were checked and found in good order.",
    "Two axles were marked for service and one brake hose was changed.",
    "The run left on time and reached the harbour siding without a stop.",
];

/// The texts of the paragraphs and headings of `out` in reading order.
fn block_texts(out: &ReadOutcome) -> Vec<String> {
    fn walk(blocks: &[Block], texts: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Section { blocks, .. } => walk(blocks, texts),
                b => {
                    let mut t = String::new();
                    common::block_text(b, &mut t);
                    texts.push(t.trim().to_string());
                }
            }
        }
    }
    let mut texts = Vec::new();
    walk(&out.document.body, &mut texts);
    texts
}

/// Task 36a part 1b (class D on the tagged path): the tags of a footer of three columns give
/// one paragraph per line, row by row across the columns. Where the page shows the columns
/// (starts shared by the title line and later lines at an even pitch) the paragraphs are
/// placed column by column, each column top to bottom: a line of one column between the
/// lines of another breaks what the column says. The elements stay as the tags give them,
/// one paragraph each.
#[test]
fn tagged_paragraphs_of_columns_read_column_by_column() {
    // The scales the page holds: at 1.25 the third column ends near the right edge.
    for (pages, k) in [(1, 1.0f32), (2, 1.0), (1, 0.5), (1, 1.25)] {
        let out = read(&tagged_footer(pages, k));
        assert_eq!(out.document.properties.facts.tagged, Some(true));
        let texts = block_texts(&out);
        let at = |t: &str| {
            texts
                .iter()
                .position(|b| b == t)
                .unwrap_or_else(|| panic!("{pages} pages, scale {k}: no block {t:?} in {texts:#?}"))
        };
        for column in TAGGED_FOOTER {
            let first = at(column[0]);
            for (n, line) in column.iter().enumerate() {
                assert_eq!(
                    at(line),
                    first + n,
                    "{pages} pages, scale {k}: {line:?} in {texts:#?}"
                );
            }
        }
        // The body keeps its order, before the footer, and the page number stands last.
        let body: Vec<usize> = BODY.iter().map(|t| at(t)).collect();
        assert!(
            body.is_sorted() && body[2] < at(TAGGED_FOOTER[0][0]),
            "{texts:#?}"
        );
        assert!(texts.last().is_some_and(|t| t.starts_with("Sheet ")));
    }
}

/// The other way (Task 36a part 1b): an ordinary tagged page keeps the order of its tags.
/// Paragraphs across the full width, and rows of three short paragraphs side by side at an
/// even pitch with no paragraph wrapped in a column (rows of a list of parts, tagged as
/// paragraphs): nothing shows that the rows are one text, so they read row by row.
#[test]
fn tagged_paragraphs_keep_the_order_of_their_tags() {
    let rows = [
        ["Part", "Shelf", "Count"],
        ["Brake hose", "North bay", "12"],
        ["Axle box", "South bay", "4"],
        ["Buffer", "Yard store", "9"],
    ];
    let mut lines: Vec<common::TaggedLine<'_>> = vec![
        (85.0, 760.0, 14.0, true, "Carriage report"),
        (85.0, 730.0, 10.0, false, BODY[0]),
        (85.0, 716.0, 10.0, false, BODY[1]),
        (85.0, 702.0, 10.0, false, BODY[2]),
    ];
    for (n, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            let (x, y) = (85.0 + 170.0 * c as f32, 64.85 - 8.4 * n as f32);
            lines.push((x, y, 7.0, false, text));
        }
    }
    let out = read(&common::tagged_paragraphs(&[lines.clone()]));
    let texts = block_texts(&out);
    let given: Vec<&str> = lines.iter().map(|l| l.4).collect();
    let read: Vec<&str> = texts.iter().skip(1).map(String::as_str).collect();
    assert_eq!(read, given);
}

/// Task 36a fix round 3 (re-review 2, I2, on the tagged path): the rows of a table without
/// rules, tagged one paragraph per cell line, whose last item has a second line of its name
/// beside the wrapped end of its description, while only the first item has a source. The
/// lines show all that blocks side by side with a wrapped paragraph show, so they are read
/// column by column. The elements stay as the tags give them: each line is a block of its
/// own, and no block holds the descriptions of two items.
#[test]
fn tagged_cells_read_as_columns_stay_blocks_of_their_own() {
    let rows = [
        ["Item", "Description", "Source"],
        [
            "Bolt",
            "steel bolt for the frame, zinc plated, size M8",
            "Mill",
        ],
        ["Nut", "nut for the bolt, zinc plated, size M8, hex", ""],
        [
            "Washer",
            "washer for the nut and the bolt, flat, set of",
            "",
        ],
        ["DIN 125", "twelve pieces", ""],
    ];
    let mut lines: Vec<common::TaggedLine<'_>> = vec![
        (40.0, 760.0, 14.0, true, "Carriage report"),
        (40.0, 730.0, 10.0, false, BODY[0]),
    ];
    for (n, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate().filter(|(_, t)| !t.is_empty()) {
            let (x, y) = (40.0 + 161.0 * c as f32, 640.0 - 9.6 * n as f32);
            lines.push((x, y, 8.0, false, text));
        }
    }
    let out = read(&common::tagged_paragraphs(&[lines]));
    let texts = block_texts(&out);
    let at = |t: &str| {
        texts
            .iter()
            .position(|b| b == t)
            .unwrap_or_else(|| panic!("no block {t:?} in {texts:#?}"))
    };
    // Through the path: the columns were read, the names before the descriptions.
    assert!(at("DIN 125") < at("Description"), "{texts:#?}");
    // Each cell line is a block of its own, the descriptions in their order.
    let described: Vec<usize> = rows[1..].iter().map(|r| at(r[1])).collect();
    assert!(described.is_sorted(), "{texts:#?}");
    for cell in rows.iter().flatten().filter(|t| !t.is_empty()) {
        at(cell);
    }
}

/// The bounds of the regions of a turned page lie in the page box the reader reports (the
/// visible box, `/Rotate` not applied), not in the turned frame the layout read the page in:
/// a page with `/Rotate 90` whose text is drawn upward, and a page without `/Rotate` whose
/// text a `cm` turns upward and stands past the box's width in the turned frame.
#[test]
fn bounds_of_a_turned_page_lie_in_its_box() {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut kids = Vec::new();
    for rotate in ["/Rotate 90", ""] {
        let content = "q 0 1 -1 0 595 0 cm BT /F1 10 Tf 72 500 Td (Body text of the page.) Tj ET \
             BT /F1 10 Tf 700 100 Td (Far right in the turned frame) Tj ET Q\n";
        let c = b.stream("", content.as_bytes());
        kids.push(b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] {rotate} \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
        )));
    }
    let refs: Vec<String> = kids.iter().map(|k| format!("{k} 0 R")).collect();
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{}] /Count 2 >>", refs.join(" ")),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let bytes = b.finish(catalog);
    // The engine saw the turned frame: there a region reaches past the box's width.
    let (_, pages) = common::read_regions(&bytes);
    for p in &pages {
        assert!(
            p.regions.iter().any(|r| r.bounds.x1 > 595.0),
            "page {}: the frame is not turned: {:#?}",
            p.page,
            p.regions
        );
    }
    let out = read(&bytes);
    // The two lines stand apart: never one line of text.
    let md = common::markdown(&out);
    assert!(
        !md.lines()
            .any(|l| l.contains("Body text of the page.") && l.contains("Far right")),
        "{md}"
    );
    for page in [1, 2] {
        let notes: Vec<&rezzer_model::RegionNote> = out
            .document
            .regions
            .iter()
            .filter(|n| n.page == page)
            .collect();
        // Task 19: the engine may decide both lines one region (a finder of today gave one
        // region a line); each region lies in the box all the same.
        assert!(!notes.is_empty(), "page {page}: {notes:#?}");
        for n in notes {
            let [x0, y0, x1, y1] = n.bounds;
            assert!(
                0.0 <= x0 && x0 <= x1 && x1 <= 595.0 && 0.0 <= y0 && y0 <= y1 && y1 <= 842.0,
                "page {page}: {n:#?}"
            );
        }
    }
    // Each line, drawn upward by the `cm`, stands where the page shows it: a line at x, y of
    // the turned frame (baseline y, a box from a third of the size below it to the size above
    // it) lies at 595 - y - 10 to 595 - y + 10 / 3 across and from x up over its width. A wrong
    // turn back gives other numbers, not a box clamped onto an edge.
    let expected = |text: &str, x: f32, y: f32| {
        [
            595.0 - y - 10.0,
            x,
            595.0 - y + 10.0 / 3.0,
            x + common::helvetica_width(text, 10.0),
        ]
    };
    let lines = [
        expected("Body text of the page.", 72.0, 500.0),
        expected("Far right in the turned frame", 700.0, 100.0),
    ];
    for page in [1, 2] {
        for want in &lines {
            // Task 19: the engine may decide both lines one region; then that region holds
            // the box of each line (a finder of today gave each line its own). The engine's
            // box of a run reaches from its baseline up by its size (no part below the
            // baseline), so a region it bounds by its runs holds each line from the baseline up.
            let run_box = [want[0], want[1], want[2] - 10.0 / 3.0, want[3]];
            let found = out.document.regions.iter().find(|n| {
                n.page == page
                    && (n.bounds.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.5)
                        || (n.bounds[0] <= run_box[0] + 0.5
                            && n.bounds[1] <= run_box[1] + 0.5
                            && n.bounds[2] >= run_box[2] - 0.5
                            && n.bounds[3] >= run_box[3] - 0.5))
            });
            assert!(
                found.is_some(),
                "page {page}: no region at {want:?}: {:#?}",
                out.document.regions
            );
        }
    }
}

/// The width of `text` (as a PDF string: a backslash is not shown) in Helvetica-Bold at
/// `size`, with the signs of a code.
fn bold_width(text: &str, size: f32) -> f32 {
    text.chars()
        .map(|c| match c {
            '\\' => 0.0,
            '<' | '>' => 584.0,
            '*' => 389.0,
            '(' | ')' | ':' => 333.0,
            '/' | ' ' => 278.0,
            c => common::font_width("FB", c.encode_utf8(&mut [0; 4]), 1000.0),
        })
        .sum::<f32>()
        * size
        / 1000.0
}

/// The runs of a code line: a label with two blanks after it and a code in runs that each
/// end in a blank.
const CODE_PARTS: [&str; 9] = [
    "Via:  ",
    "<1234>ABCD ",
    "*EF ",
    "*GH ",
    "*\\(JK ",
    "*LMN/OP ",
    "*Q ",
    "*RS\\) ",
    "*TU",
];

/// The text of the middle row of a list of `Label value` rows in Helvetica-Bold of 10
/// points. The row is shown in the runs `parts`, each in a text object of its own with
/// `spacing` (text state operators) before it. Run `i + 1` starts `backs[i]` blank widths
/// left of where run `i` ends (`backs` is taken round again when it is shorter). `mono` shows
/// the row in Courier-Bold (every advance 6 points, so a wide blank) instead.
fn row_in_runs(parts: &[&str], backs: &[f32], spacing: &str, mono: bool) -> String {
    row_in_shows(parts, backs, spacing, mono, (false, ""))
}

/// [`row_in_runs`]; with `marked`, each run of the row is a marked-content part of its own
/// (`/Span` with an MCID, as a tagged file writes each show), after a lone blank in the
/// plain face far to the left on the same baseline, a part of its own too. `annots` are the
/// annotation dictionaries of the page, written inline.
fn row_in_shows(
    parts: &[&str],
    backs: &[f32],
    spacing: &str,
    mono: bool,
    (marked, annots): (bool, &str),
) -> String {
    let width = |t: &str| match mono {
        true => 6.0 * t.chars().filter(|c| *c != '\\').count() as f32,
        false => bold_width(t, 10.0),
    };
    let font = if mono { "F3" } else { "F2" };
    let mut content = show("F2", 10.0, 116.5, 667.27, "Start: Musterstadt Nord");
    let mut x = 116.5;
    if marked {
        content += "/Span <</MCID 28>> BDC BT /F1 10 Tf 1 0 0 1 45.64 655.27 Tm ( ) Tj ET EMC\n";
    }
    for (i, part) in parts.iter().enumerate() {
        let shown = format!("BT /{font} 10 Tf {spacing} 1 0 0 1 {x:.2} 655.27 Tm ({part}) Tj ET");
        content += &match marked {
            true => format!("/Span <</MCID {}>> BDC {shown} EMC\n", 29 + i),
            false => format!("{shown}\n"),
        };
        x += width(part) - backs[i % backs.len()] * width(" ");
    }
    content += &show("F2", 10.0, 116.5, 643.27, "Class: 2");
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let fonts: Vec<String> = ["Helvetica", "Helvetica-Bold", "Courier-Bold"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let f = b.add(format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /{name} >>"
            ));
            format!("/F{} {f} 0 R", i + 1)
        })
        .collect();
    let c = b.stream("", content.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << {} >> >> /Contents {c} 0 R /Annots [{annots}] >>",
        fonts.join(" ")
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    // The three rows are one block; the row stands between the other two.
    let all = text(page(&out, 1));
    let row = all
        .split_once("Start: Musterstadt Nord")
        .and_then(|(_, rest)| rest.split_once("Class: 2"))
        .map(|(row, _)| row);
    let row = row.unwrap_or_else(|| panic!("no middle row in {all:?}"));
    // One blank parts the rows on each side; the blanks of the row itself are kept.
    row.strip_prefix(' ')
        .and_then(|r| r.strip_suffix(' '))
        .unwrap_or_else(|| panic!("the rows are not parted in {all:?}"))
        .to_string()
}

/// A line shown run by run, each run ending in a blank and the next one set back so that its
/// first glyph is printed over that blank: the page shows no gap there, so the code reads as
/// printed, with the one blank after the label that nothing is printed over. The next run
/// covers the whole blank or most of it.
#[test]
fn a_blank_the_next_run_is_printed_over_is_no_gap() {
    for back in [1.0, 0.8] {
        assert_eq!(
            row_in_runs(&CODE_PARTS, &[back], "", false),
            "Via: <1234>ABCD*EF*GH*(JK*LMN/OP*Q*RS)*TU",
            "set back by {back} blanks"
        );
    }
}

/// Fix round 1 of Task 38b: the same line in a tagged file, where every show is a marked
/// content part of its own. A change of the part ends no row of shows and parts no word:
/// the code reads as printed, with one blank after the label.
#[test]
fn a_blank_printed_over_by_the_show_of_another_marked_part_is_no_gap() {
    for back in [1.0, 0.8] {
        assert_eq!(
            row_in_shows(&CODE_PARTS, &[back], "", false, (true, "")),
            "Via: <1234>ABCD*EF*GH*(JK*LMN/OP*Q*RS)*TU",
            "set back by {back} blanks"
        );
    }
    // Blanks nothing is printed over stay, part by part.
    assert_eq!(
        row_in_shows(&CODE_PARTS, &[0.0], "", false, (true, "")),
        "Via:  <1234>ABCD *EF *GH *(JK *LMN/OP *Q *RS) *TU"
    );
}

/// Fix round 2 of Task 38b (review M5): a change of link ends a row of shows. The code line
/// with a link over its runs from `*(JK` on is two rows, each with two places or more where
/// every show is printed over the blank before it: their blanks go. The show that starts the
/// link belongs to neither row with the show before it, so the blank between them stays.
#[test]
fn a_change_of_link_ends_a_row_of_shows() {
    let blank = bold_width(" ", 10.0);
    let x: f32 = 116.5
        + CODE_PARTS[..4]
            .iter()
            .map(|p| bold_width(p, 10.0) - blank)
            .sum::<f32>();
    let link = format!(
        "<< /Type /Annot /Subtype /Link /Rect [{:.2} 652 400 667] /Border [0 0 0] \
         /A << /S /URI /URI (https://example.org/) >> >>",
        x - 0.2
    );
    assert_eq!(
        row_in_shows(&CODE_PARTS, &[1.0], "", false, (false, &link)),
        "Via: <1234>ABCD*EF*GH *(JK*LMN/OP*Q*RS)*TU"
    );
}

/// A blank that the next run is not printed over is a gap, and so is one it covers less than
/// half of (`COVERED`, from both sides: 0.47 of the blank stays, 0.6 goes). At 0.47 what is
/// left of the blank is under a word gap, so only the half keeps it.
#[test]
fn a_blank_the_next_run_leaves_white_stays() {
    let spaced = "Via:  <1234>ABCD *EF *GH *(JK *LMN/OP *Q *RS) *TU";
    for back in [0.0, 0.2, 0.47] {
        assert_eq!(
            row_in_runs(&CODE_PARTS, &[back], "", false),
            spaced,
            "set back by {back} blanks"
        );
    }
    assert_eq!(
        row_in_runs(&CODE_PARTS, &[0.6], "", false),
        "Via: <1234>ABCD*EF*GH*(JK*LMN/OP*Q*RS)*TU"
    );
}

/// A wide blank (6 points in Courier of 10 points) that the next run covers by more than
/// half still shows white wider than a word gap: the two words stay apart (0.6 of the blank
/// leaves 2.4 points). Covered by 0.8, 1.2 points are left, less than a word gap: no gap.
#[test]
fn a_wide_blank_printed_over_in_part_still_parts_two_words() {
    let parts = ["Erika ", "Mustermann ", "Musterstadt"];
    assert_eq!(
        row_in_runs(&parts, &[0.6], "", true),
        "Erika Mustermann Musterstadt"
    );
    assert_eq!(
        row_in_runs(&parts, &[0.8], "", true),
        "ErikaMustermannMusterstadt"
    );
}

/// One run printed over a blank is one measurement, not how the row was written
/// (`SET_BACK`, from both sides): a line number kerned against its text keeps its blank; two
/// runs set back with none against lose theirs.
#[test]
fn one_run_printed_over_a_blank_keeps_the_blank() {
    assert_eq!(
        row_in_runs(&["Nr 1 ", "a Total"], &[1.0], "", false),
        "Nr 1 a Total"
    );
    assert_eq!(
        row_in_runs(&["Nr 1 ", "a ", "Total"], &[1.0], "", false),
        "Nr 1aTotal"
    );
}

/// A tight line: some words start inside the blank before them by more than half of it,
/// others by less. The gaps are small but there, so every word keeps its blank. The same
/// holds when one run of the row starts over the letters before its blank.
#[test]
fn a_row_with_a_run_that_leaves_its_blank_keeps_every_blank() {
    let parts = ["Erika ", "Mustermann ", "wohnt ", "hier ", "gern"];
    assert_eq!(
        row_in_runs(&parts, &[0.6, 0.3, 0.7, 0.6], "", false),
        "Erika Mustermann wohnt hier gern"
    );
    // 1.7 blanks back: the run starts over the last letter before the blank.
    assert_eq!(
        row_in_runs(&["Nord ", "*EF ", "*GH"], &[1.7, 1.0], "", false),
        "Nord *EF *GH"
    );
}

/// Text the rule must not touch: a word shown in two runs stays one word, tight word spacing
/// inside a run keeps the words, and kerning into a blank keeps it.
#[test]
fn words_keep_their_blanks_and_split_words_stay_joined() {
    assert_eq!(
        row_in_runs(&["Muster", "stadt ", "Nord"], &[0.0], "", false),
        "Musterstadt Nord"
    );
    // Each word starts 2 points inside the blank of 2.78 before it: the pen moved there.
    assert_eq!(
        row_in_runs(&["Erika Mustermann wohnt hier"], &[0.0], "-2 Tw", false),
        "Erika Mustermann wohnt hier"
    );
    let kerned = "BT /F2 10 Tf 1 0 0 1 116.5 655.27 Tm \
                  [(Erika ) 90 (Mustermann ) 60 (wohnt) 40 ( hier)] TJ ET\n";
    let out = read(&pdf(&[Page::text(kerned)], ""));
    assert_eq!(
        text(page(&out, 1)).lines().last().map(str::trim),
        Some("Erika Mustermann wohnt hier")
    );
}
