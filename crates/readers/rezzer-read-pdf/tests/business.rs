// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Business documents (invoices, offers, orders, delivery notes), built in code with invented
//! values only. Unless a test says otherwise, a page has a body of at least 600 ink characters
//! of plain 7 pt text, so that 7 pt is the body size.

mod common;

use common::{FormPage, print_form, show};
use rezzer_model::{Block, Inline, ReadOutcome, plain_text};

/// The allocator of the Rezzer binary, so the reads run as they run in it.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

/// One line of the body text, in small letters mostly, so it reads as prose.
const BODY_LINE: &str = "die Muster GmbH liefert die bestellte Ware gemäß den Bedingungen";

/// Lines of plain `size` pt body text from `top` down, at least 600 ink characters.
fn body(size: f32, top: f32) -> String {
    let mut out = String::new();
    let mut ink = 0;
    let mut y = top;
    while ink < 600 {
        out.push_str(&show("FL", size, 40.0, y, BODY_LINE));
        ink += BODY_LINE.chars().filter(|c| !c.is_whitespace()).count();
        y -= size * 1.3;
    }
    out
}

/// A file of one page per template, each with the 7 pt body under it.
fn pages(templates: &[String]) -> Vec<u8> {
    let pages: Vec<FormPage> = templates
        .iter()
        .map(|t| FormPage {
            template: format!("{t}{}", body(7.0, 500.0)),
            ..FormPage::default()
        })
        .collect();
    print_form(&pages, "", "")
}

/// Every block of the body, sections opened.
fn blocks(out: &ReadOutcome) -> Vec<&Block> {
    fn walk<'a>(blocks: &'a [Block], found: &mut Vec<&'a Block>) {
        for b in blocks {
            match b {
                Block::Section { blocks, .. } => walk(blocks, found),
                _ => found.push(b),
            }
        }
    }
    let mut found = Vec::new();
    walk(&out.document.body, &mut found);
    found
}

fn headings(out: &ReadOutcome) -> Vec<(u8, String)> {
    blocks(out)
        .into_iter()
        .filter_map(|b| match b {
            Block::Heading { level, inlines, .. } => Some((*level, plain_text(inlines))),
            _ => None,
        })
        .collect()
}

/// The paragraph whose text starts with `start`, if one does.
fn paragraph<'a>(out: &'a ReadOutcome, start: &str) -> Option<&'a [Inline]> {
    blocks(out).into_iter().find_map(|b| match b {
        Block::Paragraph { inlines, .. } if plain_text(inlines).starts_with(start) => {
            Some(inlines.as_slice())
        }
        _ => None,
    })
}

fn line_breaks(inlines: &[Inline]) -> usize {
    inlines
        .iter()
        .filter(|i| matches!(i, Inline::LineBreak))
        .count()
}

#[test]
fn address_in_larger_size_is_no_heading() {
    let address = show("FB", 9.0, 40.0, 700.0, "Erika Mustermann")
        + &show("FB", 9.0, 40.0, 689.0, "Musterweg 1")
        + &show("FB", 9.0, 40.0, 678.0, "12345 Musterstadt");
    let out = common::read(&pages(&[address]));
    assert!(headings(&out).is_empty(), "{:?}", headings(&out));
    let found = paragraph(&out, "Erika Mustermann").expect("the address paragraph");
    assert_eq!(
        plain_text(found),
        "Erika Mustermann Musterweg 1 12345 Musterstadt"
    );
    assert_eq!(line_breaks(found), 2, "{found:?}");
}

#[test]
fn two_line_sentence_in_larger_size_is_a_paragraph() {
    let text = show(
        "FB",
        9.0,
        40.0,
        700.0,
        "Bitte zahlen Sie den Betrag bis zum 01.03.2026 auf das Konto unten.",
    ) + &show("FB", 9.0, 40.0, 689.0, "Vielen Dank für Ihren Auftrag.");
    let out = common::read(&pages(&[text]));
    assert!(headings(&out).is_empty(), "{:?}", headings(&out));
    let found = paragraph(&out, "Bitte zahlen").expect("the paragraph");
    assert!(
        plain_text(found).ends_with("Vielen Dank für Ihren Auftrag."),
        "{found:?}"
    );
}

#[test]
fn bold_label_then_value_is_no_heading() {
    let line = "BT /FB 9 Tf 40 700 Td (Kundennummer:) Tj /FL 9 Tf ( 00000) Tj ET\n".to_string();
    let out = common::read(&pages(&[line]));
    assert!(headings(&out).is_empty(), "{:?}", headings(&out));
    let found = paragraph(&out, "Kundennummer:").expect("the label line");
    assert_eq!(plain_text(found), "Kundennummer: 00000");
    match &found[0] {
        Inline::Text { text, strong, .. } => {
            assert_eq!(text.trim(), "Kundennummer:");
            assert!(strong, "{found:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn amount_line_is_no_heading() {
    let out = common::read(&pages(&[
        show("FB", 9.0, 40.0, 700.0, "Gesamtbetrag 1.234,56 EUR"),
        show("FB", 14.0, 40.0, 700.0, "Gesamtbetrag 1.234,56 EUR"),
    ]));
    assert!(headings(&out).is_empty(), "{:?}", headings(&out));
    assert!(paragraph(&out, "Gesamtbetrag 1.234,56 EUR").is_some());
}

/// Guard: a bold title a little larger than the body stays a heading.
#[test]
fn bold_title_stays_a_heading() {
    let out = common::read(&pages(&[show("FB", 9.0, 40.0, 700.0, "Rechnung")]));
    assert_eq!(headings(&out), [(1, "Rechnung".to_string())]);
}

/// Guard: two lines of a title at 1.5 times the body size or more are one heading.
#[test]
fn two_line_large_title_stays_a_heading() {
    let template = show("FL", 14.0, 40.0, 760.0, "Jahresbericht")
        + &show("FL", 14.0, 40.0, 744.0, "Muster GmbH")
        + &body(9.0, 500.0);
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = common::read(&print_form(&[page], "", ""));
    assert_eq!(
        headings(&out),
        [(1, "Jahresbericht Muster GmbH".to_string())]
    );
}

/// Guard: a number in a large title does not make it data.
#[test]
fn heading_with_a_year_stays_a_heading() {
    let out = common::read(&pages(&[show("FB", 14.0, 40.0, 700.0, "Bericht 2025")]));
    assert_eq!(headings(&out), [(1, "Bericht 2025".to_string())]);
}

/// Guard: the title of a cover page with little other text stays a heading, also when it sets
/// a large share of the page's ink.
#[test]
fn cover_title_stays_a_heading() {
    let cover = FormPage {
        template: show("FL", 24.0, 40.0, 600.0, "Jahresbericht")
            + &show("FL", 10.0, 40.0, 400.0, "Muster GmbH"),
        ..FormPage::default()
    };
    let text = FormPage {
        template: body(10.0, 700.0),
        ..FormPage::default()
    };
    let out = common::read(&print_form(&[cover, text], "", ""));
    assert_eq!(headings(&out), [(1, "Jahresbericht".to_string())]);
}
