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
        own: &[],
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
        own: &[],
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
        own: &[],
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
        line_breaks(&lines, 0, 2, &[], &no_amount)
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

/// Task 36a part 1 (the letter head with a larger name): a bold name a little larger than the
/// body over the smaller lines of its address at the address's own pitch is the first line of
/// the address, a paragraph and no heading. A heading that stands farther over the lines than
/// they stand from each other, lines of one shape, lines at another edge, one line under the
/// name or lines without a number keep the heading.
#[test]
fn a_name_over_its_smaller_address_lines_opens_the_address() {
    // The name at `size` and `x`, its first address line `lead` under it, the address lines
    // at 8 points and `pitch` apart.
    let page = |texts: &[&str], size: f32, x: f32, lead: f32, pitch: f32| -> Vec<TextRun> {
        let mut runs = vec![TextRun {
            bold: true,
            x,
            ..run(texts[0], 0, 700.0, size)
        }];
        for (k, t) in texts[1..].iter().enumerate() {
            runs.push(run(t, 1 + k as u32, 700.0 - lead - pitch * k as f32, 8.0));
        }
        runs
    };
    let levels = levels(&[12.0]);
    let read = |runs: &[TextRun]| {
        let lines = text_lines(runs);
        let block = vec![None; lines.len()];
        let cx = TextCx {
            levels: &levels,
            block: &block,
            own: &[],
        };
        match next_block(&lines, 0, false, &cx) {
            (
                TextBlock::Heading {
                    demoted, breaks, ..
                },
                j,
            ) => (j, demoted, breaks.len()),
            other => panic!("{other:?}"),
        }
    };
    let address = ["Sample Trading", "Mill Lane 4", "20481 Oldtown"];
    // The name is a heading by its own style, and the three lines are one address with a
    // break before each line under the name.
    assert_eq!(
        read(&page(&address, 12.0, 72.0, 10.4, 10.4)),
        (3, Some(Demotion::Address), 2)
    );
    // The pitch of the lines against the lead of the name, each in its own size: the lead
    // may be `TITLE_STEP` (1.1) times the pitch. 10.4 / 8 = 1.3 sizes, so 1.43 sizes of 12
    // is the bound: 17.1 points holds, 17.2 is a heading over the address.
    assert_eq!(
        read(&page(&address, 12.0, 72.0, 17.1, 10.4)).1,
        Some(Demotion::Address)
    );
    assert_eq!(read(&page(&address, 12.0, 72.0, 17.2, 10.4)), (1, None, 0));
    // The name at another left edge than its lines (an edge is 0.1 sizes: 1.2 points).
    assert_eq!(read(&page(&address, 12.0, 73.1, 10.4, 10.4)).0, 3);
    assert_eq!(read(&page(&address, 12.0, 73.3, 10.4, 10.4)), (1, None, 0));
    // One line under the name shows no pitch and no address.
    assert_eq!(
        read(&page(&address[..2], 12.0, 72.0, 10.4, 10.4)),
        (1, None, 0)
    );
    // Lines without a number, and lines of one shape (a title over fields set alike).
    let plain = ["Sample Trading", "Mill Lane", "Oldtown"];
    assert_eq!(read(&page(&plain, 12.0, 72.0, 10.4, 10.4)), (1, None, 0));
    let fields = ["Chapter 1", "Part 2", "Annex 3"];
    assert_eq!(read(&page(&fields, 12.0, 72.0, 10.4, 10.4)), (1, None, 0));
    // The shape of an address without a street and a town: a heading over rows of short
    // fields with numbers, and over a short note that wraps, keeps its heading.
    let rows = [
        "Staff and office",
        "Number: 20481 Born: 01.01.1990",
        "Class: 1 Rate: 0.0",
    ];
    assert_eq!(read(&page(&rows, 12.0, 72.0, 10.4, 10.4)), (1, None, 0));
    let note = ["Income", "Attach Form A-2", "here. Also", "attach Forms"];
    assert_eq!(read(&page(&note, 12.0, 72.0, 10.4, 10.4)), (1, None, 0));
    // Fix round 1 (review I2): a bold line over a complete address, one that has its own
    // name line (a line without a digit over its street), is a heading over it: an address
    // has one name. The label of an invoice (`Delivery address`) keeps its heading.
    let headed = [
        "Delivery address",
        "Sample Trading",
        "Mill Lane 4",
        "20481 Oldtown",
    ];
    assert_eq!(read(&page(&headed, 12.0, 72.0, 10.4, 10.4)), (1, None, 0));
    // The same label over an address without a name line reads as the first line of the
    // address, by ruling (controller, fix round 1): the geometry cannot tell such a label
    // from a name (both are short lines without a digit at the address's edge and pitch).
    // One block with the bold first line kept bold: no value and no order moves, and the
    // heading lost is the accepted cost (recorded for after 0.1.0).
    let label = ["Delivery address", "Mill Lane 4", "20481 Oldtown"];
    assert_eq!(
        read(&page(&label, 12.0, 72.0, 10.4, 10.4)),
        (3, Some(Demotion::Address), 2)
    );
    // The longest run of lines that is an address: prose after the town is no line of it,
    // and a country under the town is.
    let longer = [
        "Sample Trading",
        "Mill Lane 4",
        "20481 Oldtown",
        "Northland",
        "we send the goods as agreed in the order of the first of the month",
    ];
    assert_eq!(read(&page(&longer, 12.0, 72.0, 10.4, 10.4)).0, 4);
    // A line under the address at another pitch is no line of it.
    let mut apart = page(&longer[..4], 12.0, 72.0, 10.4, 10.4);
    apart[3].y -= 2.0;
    assert_eq!(read(&apart).0, 3);
    // At most `NAME_ADDRESS_LINES` (8) lines, the name counted.
    let many = [
        "Sample Trading",
        "Hall 1",
        "Gate 2",
        "Dock 3",
        "Mill Lane 4",
        "20481 Oldtown",
        "Room 6",
        "Desk 7",
        "Tray 8",
    ];
    assert_eq!(
        read(&page(&many, 12.0, 72.0, 10.4, 10.4)).0,
        NAME_ADDRESS_LINES
    );
    // A name much larger than the body (1.5 times the body of 10 and over) keeps its
    // heading: a company's name set large over its address is the head of the letter.
    assert_eq!(
        read(&page(&address, 14.9, 72.0, 10.4, 10.4)).1,
        Some(Demotion::Address)
    );
    assert_eq!(read(&page(&address, 15.0, 72.0, 10.4, 10.4)), (1, None, 0));
    // The address in another block of its own than the name (a box between them): the name
    // stays what it was.
    let runs = page(&address, 12.0, 72.0, 10.4, 10.4);
    let lines = text_lines(&runs);
    let block = [Some(0), Some(1), Some(1)];
    let cx = TextCx {
        levels: &levels,
        block: &block,
        own: &[],
    };
    assert!(matches!(
        next_block(&lines, 0, false, &cx),
        (TextBlock::Heading { demoted: None, .. }, 1)
    ));
}

/// A line the reader names as a line of its own (a line of blocks side by side read as
/// columns, see [`TextCx::own`]) starts after a line break, though it goes on the sentence of
/// the line over it; the lines stay one paragraph. Without the name the sentence is joined,
/// and a name for the first line alone breaks nothing.
#[test]
fn a_line_of_its_own_is_never_joined_to_the_line_over_it() {
    let runs = vec![
        run("washer for the nut and the bolt, flat, set", 0, 730.0, 10.0),
        run("of twelve pieces, zinc plated, in a paper", 1, 718.0, 10.0),
        run("bag of the mill", 2, 706.0, 10.0),
    ];
    let lines = text_lines(&runs);
    let block = vec![None; lines.len()];
    let breaks = |own: &[bool]| {
        let cx = TextCx {
            levels: &levels(&[]),
            block: &block,
            own,
        };
        match walk(&lines, &cx).as_slice() {
            [(TextBlock::Paragraph { breaks }, range)] if *range == (0..3) => breaks.clone(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(breaks(&[]), []);
    assert_eq!(breaks(&[false, false, false]), []);
    assert_eq!(breaks(&[true, false, false]), []);
    assert_eq!(breaks(&[false, true, false]), [(0, 1)]);
    assert_eq!(breaks(&[true, true, true]), [(0, 1), (0, 2)]);
}
