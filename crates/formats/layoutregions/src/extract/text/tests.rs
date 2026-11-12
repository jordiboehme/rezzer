// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use crate::input::{FontKind, Shift, TextRun};
use crate::line::{text_line, text_lines};
use std::ops::Range;

fn run(text: &str, line: u32, y: f32, size: f32) -> TextRun {
    TextRun {
        text: text.into(),
        x: 72.0,
        y,
        w: 100.0,
        h: size,
        size,
        font: 0,
        font_kind: FontKind::Unknown,
        bold: false,
        italic: false,
        color: None,
        shift: Shift::Base,
        line,
        off_axis: false,
        gaps: Vec::new(),
    }
}

/// The blocks of `lines` as a reader walks them: the list stays open after an item, a heading
/// or a paragraph closes it, a line without text leaves it as it is.
fn walk(lines: &[TextLine<'_, TextRun>], cx: &TextCx<'_>) -> Vec<(TextBlock, Range<usize>)> {
    let mut out = Vec::new();
    let mut open = false;
    let mut i = 0;
    while i < lines.len() {
        let (block, j) = next_block(lines, i, open, cx);
        assert!(j > i, "the walk goes on");
        match &block {
            TextBlock::Empty => {}
            TextBlock::Item { .. } => open = true,
            _ => open = false,
        }
        out.push((block, i..j));
        i = j;
    }
    out
}

fn no_amount(_: &str) -> bool {
    false
}

/// Heading levels against a body of 10 points, with the heading sizes `heading_sizes`.
fn levels(heading_sizes: &[f32]) -> Levels<'_> {
    Levels {
        body_size: 10.0,
        heading_sizes,
        text_sizes: &[],
        ends_in_amount: &no_amount,
    }
}

#[test]
fn markers() {
    assert_eq!(list_marker("\u{2022} one"), Some((false, 4)));
    assert_eq!(list_marker("- two"), Some((false, 2)));
    assert_eq!(list_marker("\u{2013} three"), Some((false, 4)));
    assert_eq!(list_marker("12) four"), Some((true, 4)));
    assert_eq!(list_marker("3. five"), Some((true, 3)));
    assert_eq!(list_marker("-5 degrees"), None);
    assert_eq!(list_marker("2024 was"), None);
    assert_eq!(list_marker("1.5 litres"), None);
    // Padding after a star or a dash: a mark set into a form field, no bullet.
    assert_eq!(list_marker("*     Betrag"), None);
    assert_eq!(list_marker("-  keine"), None);
    // A star closed by a lone star on its line encloses an item, no bullet (part 2e-4,
    // item 6); a star bullet over text with a star in a word still is one.
    assert_eq!(list_marker("* A123456B * x/y"), None);
    assert_eq!(list_marker("* Erste Position"), Some((false, 2)));
    assert_eq!(list_marker("* Rabatt 5* auf alles"), Some((false, 2)));
    assert_eq!(list_marker("\u{2013}\t\tkeine"), None);
    assert_eq!(list_marker("* Punkt"), Some((false, 2)));
}

/// The minus after an amount with cents ends the amount; a hyphen after a word or a whole
/// number cuts a word or a range (part 2e-4, item 3). Text of other scripts is read char by
/// char, never cut inside a char.
#[test]
fn a_minus_after_an_amount_is_no_word_cut() {
    for signed in ["10,00-", "Rabatt 1 10,00-", "1.234,56-", "12.50-"] {
        assert!(signed_amount(signed), "{signed}");
        assert!(!word_cut(signed), "{signed}");
    }
    for cut in [
        "Page-",
        "Rechnungs-",
        "2025-",
        "1,5-",
        "\u{201c}ab-",
        "\u{20ac}1,00-",
        "-",
    ] {
        assert!(!signed_amount(cut), "{cut}");
        assert!(word_cut(cut), "{cut}");
    }
}

#[test]
fn a_star_parted_by_a_piece_gap_is_no_bullet() {
    let mark = TextRun {
        w: 6.0,
        ..run("* ", 0, 700.0, 10.0)
    };
    let far = TextRun {
        x: 120.0,
        ..run("Betrag", 0, 700.0, 10.0)
    };
    let near = TextRun {
        x: 80.0,
        ..run("Punkt", 0, 700.0, 10.0)
    };
    let padded = [mark.clone(), far];
    assert!(padded_mark(&text_line(&padded)));
    let bullet = [mark, near];
    assert!(!padded_mark(&text_line(&bullet)));
    // A dash bullet at a tab stop: its space stands before the text.
    let dash = TextRun {
        w: 6.0,
        ..run("-", 0, 700.0, 10.0)
    };
    let tabbed = TextRun {
        x: 120.0,
        ..run(" Punkt", 0, 700.0, 10.0)
    };
    let tab_stop = [dash, tabbed];
    assert!(!padded_mark(&text_line(&tab_stop)));
}

#[test]
fn address_shapes() {
    assert!(address_shaped(&[
        "Erika Mustermann",
        "Musterweg 1",
        "12345 Musterstadt"
    ]));
    // An IBAN in groups is one word.
    assert!(address_shaped(&[
        "Muster GmbH",
        "IBAN DE00 0000 0000 0000 0000 00",
        "BIC MUSTDEXXXXX"
    ]));
    assert_eq!(word_count("IBAN DE00 0000 0000 0000 0000 00"), 2);
    // Two lines, no digit after the first, prose.
    assert!(!address_shaped(&["Erika Mustermann", "Musterweg 1"]));
    assert!(!address_shaped(&[
        "Muster GmbH",
        "Abteilung Einkauf",
        "Musterstadt"
    ]));
    assert!(!address_shaped(&[
        "die Ware wird",
        "am 1. Mai geliefert",
        "und bezahlt"
    ]));
    assert!(!address_shaped(&["Muster GmbH", "12345", "00000"]));
}

/// A list between two paragraphs: an item takes the indented line that goes on it, the
/// second item joins the first, and the paragraph after the gap stands on its own.
#[test]
fn a_list_between_paragraphs() {
    let runs = vec![
        run("Shopping", 0, 700.0, 10.0),
        run("- apples", 1, 688.0, 10.0),
        run("- pears and", 2, 676.0, 10.0),
        TextRun {
            x: 80.0,
            ..run("plums", 3, 664.0, 10.0)
        },
        run("After the list.", 4, 630.0, 10.0),
    ];
    let lines = text_lines(&runs);
    let block = vec![None; lines.len()];
    let cx = TextCx {
        levels: &levels(&[]),
        block: &block,
    };
    assert_eq!(
        walk(&lines, &cx),
        [
            (TextBlock::Paragraph { breaks: Vec::new() }, 0..1),
            (
                TextBlock::Item {
                    ordered: false,
                    cut: 2,
                    joins: false
                },
                1..2
            ),
            (
                TextBlock::Item {
                    ordered: false,
                    cut: 2,
                    joins: true
                },
                2..4
            ),
            (TextBlock::Paragraph { breaks: Vec::new() }, 4..5),
        ]
    );
}

/// Every line of a page of a heading, prose, a list and a line without text stands in one
/// block, once, in order; a line in another block of its own goes on no paragraph.
#[test]
fn extractors_return_every_prim_once() {
    let runs = vec![
        run("Einleitung", 0, 760.0, 16.0),
        run("Dies ist ein Satz, der weiter", 1, 730.0, 10.0),
        run("geht und endet hier.", 2, 718.0, 10.0),
        run("1. Erster Punkt", 3, 690.0, 10.0),
        run("2. Zweiter Punkt", 4, 678.0, 10.0),
        run(" ", 5, 666.0, 10.0),
        run("Muster GmbH", 6, 640.0, 10.0),
        run("Musterweg 1", 7, 628.0, 10.0),
    ];
    let lines = text_lines(&runs);
    // The last line stands in a block of its own the table pass read.
    let mut block = vec![None; lines.len()];
    block[7] = Some(0);
    let cx = TextCx {
        levels: &levels(&[16.0]),
        block: &block,
    };
    let blocks = walk(&lines, &cx);
    let mut seen = vec![0; lines.len()];
    for (_, range) in &blocks {
        for k in range.clone() {
            seen[k] += 1;
        }
    }
    assert_eq!(seen, vec![1; lines.len()]);
    let ends: Vec<usize> = blocks.iter().map(|b| b.1.end).collect();
    assert!(ends.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(
        blocks,
        [
            (
                TextBlock::Heading {
                    level: 1,
                    demoted: None,
                    breaks: Vec::new()
                },
                0..1
            ),
            (TextBlock::Paragraph { breaks: Vec::new() }, 1..3),
            (
                TextBlock::Item {
                    ordered: true,
                    cut: 3,
                    joins: false
                },
                3..4
            ),
            (
                TextBlock::Item {
                    ordered: true,
                    cut: 3,
                    joins: true
                },
                4..5
            ),
            (TextBlock::Empty, 5..6),
            (TextBlock::Paragraph { breaks: Vec::new() }, 6..7),
            (TextBlock::Paragraph { breaks: Vec::new() }, 7..8),
        ]
    );
}

/// The blocks of `runs` with no blocks of their own, on a body of 10 points with headings of
/// 16 and 12 points.
fn blocks_of(runs: &[TextRun]) -> Vec<(TextBlock, Range<usize>)> {
    let lines = text_lines(runs);
    let block = vec![None; lines.len()];
    let cx = TextCx {
        levels: &levels(&[16.0, 12.0]),
        block: &block,
    };
    walk(&lines, &cx)
}

/// The short lines of an address at one left edge are one paragraph with a break before each
/// line after the first.
#[test]
fn an_address_keeps_its_lines() {
    let runs = vec![
        run("Muster GmbH", 0, 700.0, 10.0),
        run("Musterweg 1", 1, 688.0, 10.0),
        run("12345 Musterstadt", 2, 676.0, 10.0),
    ];
    assert_eq!(
        blocks_of(&runs),
        [(
            TextBlock::Paragraph {
                breaks: vec![(0, 1), (0, 2)]
            },
            0..3
        )]
    );
}

/// Bold lines a little larger than the body that are the lines of an address are no heading:
/// a paragraph with its breaks.
#[test]
fn a_small_bold_address_is_no_heading() {
    let bold = |text: &str, line: u32, y: f32| TextRun {
        bold: true,
        ..run(text, line, y, 12.0)
    };
    let runs = vec![
        bold("Muster GmbH", 0, 700.0),
        bold("Musterweg 1", 1, 686.0),
        bold("12345 Musterstadt", 2, 672.0),
    ];
    assert_eq!(
        blocks_of(&runs),
        [(
            TextBlock::Heading {
                level: 2,
                demoted: Some(Demotion::LinesApart),
                breaks: vec![(0, 1), (0, 2)]
            },
            0..3
        )]
    );
}

/// A bold title under plain lines, farther from them than their pitch, ends their paragraph
/// though it is near enough to go on it.
#[test]
fn a_bold_title_under_a_block_stands_alone() {
    let runs = vec![
        run("Muster GmbH", 0, 700.0, 10.0),
        run("Musterweg 1", 1, 690.0, 10.0),
        TextRun {
            bold: true,
            ..run("Rechnung", 2, 677.0, 10.0)
        },
    ];
    assert_eq!(
        blocks_of(&runs),
        [
            (
                TextBlock::Paragraph {
                    breaks: vec![(0, 1)]
                },
                0..2
            ),
            (TextBlock::Paragraph { breaks: Vec::new() }, 2..3),
        ]
    );
}

/// The address branch of [`heading_block`]: bold lines a little larger than the body that
/// share a left edge and have the shape of an address read as a paragraph. It is reached only
/// where the column over the block (a line a box cut off it) is no list of short items, so
/// that no pair of lines is apart, and every line wraps.
#[test]
fn an_address_under_a_line_of_prose_is_demoted_as_an_address() {
    let bold = |text: &str, line: u32, y: f32| TextRun {
        bold: true,
        ..run(text, line, y, 12.0)
    };
    let runs = vec![
        bold("die Firma liefert die Ware an", 0, 714.0),
        bold("Muster GmbH", 1, 700.0),
        bold("Musterweg 1", 2, 686.0),
        bold("12345 Musterstadt", 3, 672.0),
    ];
    let lines = text_lines(&runs);
    let levels = levels(&[16.0, 12.0]);
    assert_eq!(levels.level(&lines[1]), Some(2));
    assert_eq!(
        heading_block(&lines, 1, 2, &levels),
        (4, Some(Demotion::Address))
    );
    // Without the line over it the short lines are apart: the same block is demoted for that.
    assert_eq!(
        heading_block(&lines[1..], 0, 2, &levels),
        (3, Some(Demotion::LinesApart))
    );
}

/// Review R3 of Task 31b, the path of a public form (irs-f6744 untagged page 174). A value
/// at the right end of a row of boxes, and far left of it, 4.4 points lower at a size of
/// 5.4, the second line of the label of the first box. Read one after the other they are
/// lines of one paragraph for the walk, but no text of one line: the second starts after a
/// line break. The lines of a block stand a size apart or more, and these two share no
/// stretch of the width.
#[test]
fn a_line_left_of_the_line_over_it_and_closer_than_a_line_starts_after_a_break() {
    let at = |text: &str, line: u32, x: f32, w: f32, y: f32| TextRun {
        x,
        w,
        ..run(text, line, y, 5.4)
    };
    let breaks = |second: TextRun| {
        let runs = [at("3", 0, 218.6, 2.0, 389.0), second];
        let lines = text_lines(&runs);
        assert!(continues(&lines[0], &lines[1]));
        line_breaks(&lines, 0, 2, &no_amount)
    };
    assert_eq!(breaks(at("code", 1, 45.0, 12.0, 384.6)), [(0, 1)]);
    // The step moved both ways (`LINE_STEP`, one size: 5.4 points): 0.1 points closer than
    // a size the line stands in another column, 0.1 points farther it goes on as before.
    assert_eq!(breaks(at("code", 1, 45.0, 12.0, 389.0 - 5.3)), [(0, 1)]);
    assert_eq!(breaks(at("code", 1, 45.0, 12.0, 389.0 - 5.5)), []);
    // The white space moved both ways (`BESIDE_GAP`, one size): a line that ends 5.5 points
    // before the value starts is apart from it, one that ends 5.3 points before it is not.
    assert_eq!(breaks(at("code", 1, 201.1, 12.0, 384.6)), [(0, 1)]);
    assert_eq!(breaks(at("code", 1, 201.3, 12.0, 384.6)), []);
    // The predicate alone: a line under the line over it, however close, is a line of its
    // block (the two lines of a label set tight), and so is a line right of it.
    assert!(other_column((218.6, 220.6), (45.0, 57.0), 4.4, 5.4));
    assert!(!other_column((45.0, 59.0), (45.0, 61.0), 4.4, 5.4));
    assert!(!other_column((45.0, 57.0), (218.6, 220.6), 4.4, 5.4));
}
