// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Business documents (invoices, offers, orders, delivery notes), built in code with invented
//! values only. Unless a test says otherwise, a page has a body of at least 600 ink characters
//! of plain 7 pt text, so that 7 pt is the body size.

mod common;

use common::{FormPage, logo, markdown, needing, print_form, show};
use rezzer_model::{Block, Completeness, Inline, LossReason, ReadOutcome, Severity, plain_text};

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

/// How the rules of a generated block are drawn, and how large its page is printed.
#[derive(Debug, Clone, Copy)]
enum Look {
    /// Stroked lines of 0.5 points.
    Plain,
    /// Filled bars of 1 point.
    Bar,
    /// Rows of short filled dashes, a dash and a gap of a fifth and a tenth of the text size.
    Dashed,
    /// Stroked lines with a pattern of dots and round caps.
    Dotted,
    /// The whole page drawn this many times as large.
    Scaled(f32),
}

const LOOKS: [Look; 6] = [
    Look::Plain,
    Look::Bar,
    Look::Dashed,
    Look::Dotted,
    Look::Scaled(0.7),
    Look::Scaled(1.5),
];

/// A horizontal rule from `x0` to `x1` at `y` as `look` draws it; `size` (the text size) sets
/// the dashes and dots.
fn rule(look: Look, x0: f32, x1: f32, y: f32, size: f32) -> String {
    match look {
        Look::Plain | Look::Scaled(_) => common::hrule(x0, x1, y),
        Look::Bar => format!("{x0} {} {} 1 re f\n", y - 0.5, x1 - x0),
        Look::Dashed => {
            let (dash, step) = (0.2 * size, 0.3 * size);
            let mut out = String::new();
            let mut x = x0;
            while x < x1 {
                out += &format!("{x} {} {} 0.5 re f\n", y - 0.25, dash.min(x1 - x));
                x += step;
            }
            out
        }
        Look::Dotted => format!(
            "q 0.5 w 1 J [0 {}] 0 d {x0} {y} m {x1} {y} l S Q\n",
            0.15 * size
        ),
    }
}

/// `text` in Helvetica (`/FL`) or Helvetica-Bold (`/FB`) at `size`, its right end at `x`.
fn right(bold: bool, size: f32, x: f32, y: f32, text: &str) -> String {
    let font = if bold { "FB" } else { "FL" };
    show(font, size, x - common::helvetica_width(text, size), y, text)
}

/// The rules under the line items of [`invoice`].
#[derive(Debug, Clone, Copy, PartialEq)]
enum Rules {
    /// A rule under the header only.
    Header,
    /// A rule under the header and one under the last item, over the totals.
    End,
    /// A rule under every line but the last.
    Rows,
}

/// Line items of an invoice at `k` times 8 pt text, their lines `10 k` apart from the header
/// on the baseline 600 down: the bold header `Pos`, `Artikel`, `Menge` at x 40, 80 and 300 and
/// `Preis` and `Betrag` flush right at x 420 and 540, three items, with the rules of `rules`
/// drawn as `look`. With `End`, the totals follow the rule, their labels at x 340 (between
/// `Menge` and `Preis`) and their values flush right at x 540.
fn invoice(k: f32, rules: Rules, look: Look) -> String {
    let size = 8.0 * k;
    let y = |dy: f32| 600.0 - dy * k;
    let mut out = show("FB", size, 40.0, y(0.0), "Pos")
        + &show("FB", size, 80.0, y(0.0), "Artikel")
        + &show("FB", size, 300.0, y(0.0), "Menge")
        + &right(true, size, 420.0, y(0.0), "Preis")
        + &right(true, size, 540.0, y(0.0), "Betrag");
    out += &rule(look, 40.0, 540.0, y(4.0), size);
    let items = [
        ["1", "Beratung", "2", "500,00", "1.000,00"],
        ["2", "Material", "1", "200,00", "200,00"],
        ["3", "Versand", "1", "34,56", "34,56"],
    ];
    for (n, item) in items.iter().enumerate() {
        let at = y(10.0 * (n as f32 + 1.0));
        out += &show("FL", size, 40.0, at, item[0]);
        out += &show("FL", size, 80.0, at, item[1]);
        out += &show("FL", size, 300.0, at, item[2]);
        out += &right(false, size, 420.0, at, item[3]);
        out += &right(false, size, 540.0, at, item[4]);
        if rules == Rules::Rows && n < 2 {
            out += &rule(look, 40.0, 540.0, y(10.0 * (n as f32 + 1.0) + 4.0), size);
        }
    }
    if rules == Rules::End {
        out += &rule(look, 40.0, 540.0, y(36.0), size);
        let totals = [
            (false, "Nettobetrag", "1.234,56"),
            (false, "USt 19 %", "234,57"),
            (true, "Gesamtbetrag", "1.469,13"),
        ];
        for (n, (bold, label, value)) in totals.iter().enumerate() {
            let at = y(44.0 + 10.0 * n as f32);
            out += &show(if *bold { "FB" } else { "FL" }, size, 340.0, at, label);
            out += &right(*bold, size, 540.0, at, value);
        }
    }
    out
}

/// A file of one page that holds `template` over the 7 pt body, drawn as `look`.
fn page_in(template: &str, look: Look) -> Vec<u8> {
    let page = FormPage {
        template: format!("{template}{}", body(7.0, 420.0)),
        scale: match look {
            Look::Scaled(s) => s,
            _ => 1.0,
        },
        ..FormPage::default()
    };
    print_form(&[page], "", "")
}

/// The tables of the body: header rows and the text of each cell.
fn tables(out: &ReadOutcome) -> Vec<(usize, Vec<Vec<String>>)> {
    blocks(out)
        .into_iter()
        .filter_map(|b| match b {
            Block::Table {
                header_rows, rows, ..
            } => Some((
                *header_rows,
                rows.iter()
                    .map(|r| r.iter().map(|c| plain_text(&c.inlines)).collect())
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

fn owned(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|c| (*c).to_string()).collect())
        .collect()
}

/// The table of the line items of [`invoice`].
fn items_table() -> (usize, Vec<Vec<String>>) {
    (
        1,
        owned(&[
            &["Pos", "Artikel", "Menge", "Preis", "Betrag"],
            &["1", "Beratung", "2", "500,00", "1.000,00"],
            &["2", "Material", "1", "200,00", "200,00"],
            &["3", "Versand", "1", "34,56", "34,56"],
        ]),
    )
}

/// Every size and look of the invoice with `rules`, and what each read gave.
fn invoice_reads(rules: Rules) -> Vec<(String, ReadOutcome)> {
    let mut out = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for look in LOOKS {
            let name = format!("{k}x, {look:?}");
            out.push((name, common::read(&page_in(&invoice(k, rules, look), look))));
        }
    }
    out
}

/// Line items, a rule and the totals under it: the rule ends the table of the items, and the
/// totals are a table of their own, in every size and look.
#[test]
fn rule_ends_a_table_without_rules() {
    let totals = owned(&[
        &["Nettobetrag", "1.234,56"],
        &["USt 19 %", "234,57"],
        &["Gesamtbetrag", "1.469,13"],
    ]);
    for (name, out) in invoice_reads(Rules::End) {
        let got = tables(&out);
        assert_eq!(got.len(), 2, "{name}: {got:?}");
        assert_eq!(got[0], items_table(), "{name}");
        assert_eq!(got[1].1, totals, "{name}");
        assert!(
            out.document.losses.is_empty(),
            "{name}: {:?}",
            out.document.losses
        );
    }
}

/// Guard: the rule under the header row is no end of the table.
#[test]
fn header_rule_is_not_an_end() {
    for (name, out) in invoice_reads(Rules::Header) {
        assert_eq!(tables(&out), [items_table()], "{name}");
        assert!(
            out.document.losses.is_empty(),
            "{name}: {:?}",
            out.document.losses
        );
    }
}

/// Guard: a rule under every line parts rows, not tables.
#[test]
fn row_rules_do_not_end_a_table() {
    for (name, out) in invoice_reads(Rules::Rows) {
        assert_eq!(tables(&out), [items_table()], "{name}");
        assert!(
            out.document.losses.is_empty(),
            "{name}: {:?}",
            out.document.losses
        );
    }
}

/// Four lines of three pieces at x 40, 200 and 360, from the baseline 600 down, the first line
/// with its middle piece at x 150, in the gutter. (The columns are those of the lines under the
/// first, which may be a header that spans them: a piece of a lower line at x 150 would be a
/// column of its own.)
fn columns_block() -> String {
    columns_block_at(600.0)
}

/// The lines of [`columns_block`] from the baseline `top` down.
fn columns_block_at(top: f32) -> String {
    let lines = [
        ["Muster GmbH", "Musterweg 1", "1.234,56"],
        ["Erika Mustermann", "Musterweg 2", "234,57"],
        ["Max Muster", "Musterweg 3", "34,56"],
        ["Muster AG", "Musterweg 4", "200,00"],
    ];
    let mut out = String::new();
    for (n, line) in lines.iter().enumerate() {
        let y = top - 10.0 * n as f32;
        let middle = if n == 0 { 150.0 } else { 200.0 };
        out += &show("FL", 8.0, 40.0, y, line[0]);
        out += &show("FL", 8.0, middle, y, line[1]);
        out += &show("FL", 8.0, 360.0, y, line[2]);
    }
    out
}

/// The loss for the block of [`columns_block`].
const REGION_LOSS: &str = "a block of text in columns on page 1 is not read as a table: some \
                           of its text stands between its columns; its text stays paragraphs";

#[test]
fn refused_region_without_caption_is_a_loss() {
    let out = common::read(&page_in(&columns_block(), Look::Plain));
    let losses = &out.document.losses;
    assert_eq!(losses.len(), 1, "{losses:?}\n{:?}", tables(&out));
    assert_eq!(losses[0].what, REGION_LOSS);
    assert_eq!(losses[0].why, LossReason::Uncertain);
    assert_eq!(losses[0].severity, Severity::Info);
    assert_eq!(out.completeness, Completeness::Complete);
    let text = common::all_text(&out);
    for word in columns_block()
        .lines()
        .filter_map(|l| l.split('(').nth(1)?.split(')').next())
        .flat_map(str::split_whitespace)
    {
        assert!(text.contains(word), "{word} in {text}");
    }
}

/// Two such blocks on a page give one loss that counts them.
#[test]
fn refused_regions_of_a_page_give_one_loss() {
    let template = columns_block_at(700.0) + &columns_block();
    let out = common::read(&page_in(&template, Look::Plain));
    let losses: Vec<&str> = out
        .document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .collect();
    assert_eq!(
        losses,
        [
            "2 blocks of text in columns on page 1 are not read as tables: some of their text \
          stands between their columns; their text stays paragraphs"
        ]
    );
}

/// Guard: a block next to a figure caption is a chart's, and no loss.
#[test]
fn region_beside_a_figure_caption_gives_no_loss() {
    let template = show("FL", 8.0, 40.0, 616.0, "Figure 1: Muster") + &columns_block();
    let out = common::read(&page_in(&template, Look::Plain));
    assert!(out.document.losses.is_empty(), "{:?}", out.document.losses);
}

/// A ruled grid over 70 % of the page, refused without a note, holds the block: its loss
/// says so, and the block gives none of its own.
#[test]
fn quiet_grid_and_region_over_the_same_text_give_one_loss() {
    // Two by two cells: the inner rules stand clear of the text.
    let grid = common::hrule(20.0, 575.0, 800.0)
        + &common::hrule(20.0, 575.0, 120.0)
        + &common::hrule(20.0, 575.0, 100.0)
        + "0.5 w 20 100 m 20 800 l S\n"
        + "0.5 w 560 100 m 560 800 l S\n"
        + "0.5 w 575 100 m 575 800 l S\n";
    let template = grid + &show("FL", 8.0, 40.0, 640.0, "Muster GmbH") + &columns_block();
    let out = common::read(&page_in(&template, Look::Plain));
    let losses = &out.document.losses;
    assert_eq!(losses.len(), 1, "{losses:?}");
    assert!(
        losses[0]
            .what
            .starts_with("a grid of drawn rules on page 1"),
        "{losses:?}"
    );
}

/// The line items of an invoice of `n` rows (1 to 5) at `k` times 8 pt text under plain column
/// titles at `k` times 6 pt, a rule over the titles and one under the items only, drawn as
/// `look`, with a lone upright rule far from them when `upright`.
fn rules_only_items(k: f32, n: usize, look: Look, upright: bool) -> String {
    let (size, ts) = (8.0 * k, 6.0 * k);
    let y = |dy: f32| 600.0 - dy * k;
    let mut t = rule(look, 40.0, 540.0, y(-8.0), size);
    t += &show("FL", ts, 40.0, y(0.0), "Pos");
    t += &show("FL", ts, 80.0, y(0.0), "Artikel");
    t += &show("FL", ts, 300.0, y(0.0), "Menge");
    t += &right(false, ts, 420.0, y(0.0), "Preis");
    t += &right(false, ts, 540.0, y(0.0), "Betrag");
    for (r, item) in ITEMS.iter().take(n).enumerate() {
        let at = y(10.0 * (r as f32 + 1.0));
        t += &show("FL", size, 40.0, at, item[0]);
        t += &show("FL", size, 80.0, at, item[1]);
        t += &show("FL", size, 300.0, at, item[2]);
        t += &right(false, size, 420.0, at, item[3]);
        t += &right(false, size, 540.0, at, item[4]);
    }
    t += &rule(look, 40.0, 540.0, y(10.0 * n as f32 + 6.0), size);
    if upright {
        t += "0.5 w 30 200 m 30 300 l S\n";
    }
    t
}

/// Line items for [`rules_only_items`].
const ITEMS: [[&str; 5]; 5] = [
    ["1", "Beratung", "2", "500,00", "1.000,00"],
    ["2", "Material", "1", "200,00", "200,00"],
    ["3", "Versand", "1", "34,56", "34,56"],
    ["4", "Montage", "3", "10,00", "30,00"],
    ["5", "Pruefung", "1", "99,00", "99,00"],
];

/// The table of the first `n` rows of [`ITEMS`] under their titles.
fn items_of(n: usize) -> Vec<Vec<String>> {
    let mut rows = vec![owned(&[&["Pos", "Artikel", "Menge", "Preis", "Betrag"]])[0].clone()];
    rows.extend(
        ITEMS
            .iter()
            .take(n)
            .map(|r| r.iter().map(|c| (*c).to_string()).collect()),
    );
    rows
}

/// The reviewer's probe R3: line items under small plain column titles between a rule over
/// the titles and one under the items, no upright rule on the page: one table of five columns,
/// header and rows, in three sizes.
#[test]
fn items_between_horizontal_rules_alone_are_a_table() {
    for k in [0.75, 1.0, 1.5] {
        let out = common::read(&page_in(
            &rules_only_items(k, 3, Look::Plain, false),
            Look::Plain,
        ));
        let got: Vec<Vec<Vec<String>>> = tables(&out).into_iter().map(|t| t.1).collect();
        assert_eq!(got, [items_of(3)], "{k}x: {:?}", out.document.losses);
    }
}

/// The reviewer's probe R3b: as R3 with a lone upright rule far from the items.
#[test]
fn items_between_horizontal_rules_with_an_upright_rule_elsewhere_are_a_table() {
    let out = common::read(&page_in(
        &rules_only_items(1.0, 3, Look::Plain, true),
        Look::Plain,
    ));
    let got: Vec<Vec<Vec<String>>> = tables(&out).into_iter().map(|t| t.1).collect();
    assert_eq!(got, [items_of(3)], "{:?}", out.document.losses);
}

/// Line items of two to five rows between two horizontal rules, in three sizes and every
/// look, with and without an upright rule elsewhere: one table, header and rows. One item row
/// under its titles is too little for a table without rules; it reads as fields, each title
/// with the value under it in its own column, never one of another row.
#[test]
fn items_between_horizontal_rules_hold_in_every_size_and_look() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for look in LOOKS {
            for upright in [false, true] {
                for n in 1..=5 {
                    let out = common::read(&page_in(&rules_only_items(k, n, look, upright), look));
                    let got: Vec<Vec<Vec<String>>> =
                        tables(&out).into_iter().map(|t| t.1).collect();
                    let want = items_of(n);
                    let ok = if n == 1 {
                        // Each title over its own value: a table of the two lines, or pairs.
                        got == [want.clone()]
                            || got
                                == [want[0]
                                    .iter()
                                    .cloned()
                                    .zip(want[1].iter().cloned())
                                    .map(|(l, v)| vec![l, v])
                                    .collect::<Vec<_>>()]
                    } else {
                        got == [want]
                    };
                    if !ok {
                        wrong.push(format!(
                            "{k}x, {look:?}, upright {upright}, {n} rows: {got:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Guard: a rule through the glyphs of an item (a cancelled item struck through) lies in no
/// gap between two lines, so it ends no table: one table with header and three rows, in every
/// size and look.
#[test]
fn a_struck_through_item_does_not_end_a_table() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for look in LOOKS {
            let size = 8.0 * k;
            let strike = 600.0 - 20.0 * k + 0.3 * size;
            let t = invoice(k, Rules::Header, look) + &rule(look, 40.0, 540.0, strike, size);
            let out = common::read(&page_in(&t, look));
            if tables(&out) != [items_table()] {
                wrong.push(format!("{k}x, {look:?}: {:?}", tables(&out)));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The invoice of [`invoice`] at `size` pt with its lines `lead` sizes apart, the totals right
/// under the items and a rule from x 40 to 540 midway between the last item and the first
/// total, drawn as `look`.
fn tight_invoice(size: f32, lead: f32, look: Look) -> String {
    let p = lead * size;
    let y = |n: f32| 600.0 - n * p;
    let mut out = show("FB", size, 40.0, y(0.0), "Pos")
        + &show("FB", size, 80.0, y(0.0), "Artikel")
        + &show("FB", size, 300.0, y(0.0), "Menge")
        + &right(true, size, 420.0, y(0.0), "Preis")
        + &right(true, size, 540.0, y(0.0), "Betrag");
    for (n, item) in ITEMS.iter().take(3).enumerate() {
        let at = y(n as f32 + 1.0);
        out += &show("FL", size, 40.0, at, item[0]);
        out += &show("FL", size, 80.0, at, item[1]);
        out += &show("FL", size, 300.0, at, item[2]);
        out += &right(false, size, 420.0, at, item[3]);
        out += &right(false, size, 540.0, at, item[4]);
    }
    out += &rule(look, 40.0, 540.0, y(3.5), size);
    for (n, (label, value)) in [
        ("Nettobetrag", "1.234,56"),
        ("USt 19 %", "234,57"),
        ("Gesamtbetrag", "1.469,13"),
    ]
    .iter()
    .enumerate()
    {
        let at = y(4.0 + n as f32);
        out += &show("FL", size, 340.0, at, label);
        out += &right(false, size, 540.0, at, value);
    }
    out
}

/// A rule midway between the last item and the first total at tight leading (1.1 and 1.2
/// sizes) lies in the gap between the two lines, over the x-height of the lower one: it ends
/// the items' table, and the totals are a table of their own, in every look.
#[test]
fn a_midway_rule_at_tight_leading_ends_a_table() {
    let mut wrong = Vec::new();
    for size in [6.0, 8.0] {
        for lead in [1.1, 1.2] {
            for look in LOOKS {
                let out = common::read(&page_in(&tight_invoice(size, lead, look), look));
                let got = tables(&out);
                let items = got.first().map(|t| t.1.clone());
                if got.len() != 2 || items != Some(items_of(3)) {
                    wrong.push(format!("{size} pt, {lead}, {look:?}: {got:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A row of fields between a rule over it and one under it: small 6 k pt labels at x 40, 170,
/// 300 and 430 over values of `values[c]` lines each at 8 k pt in the fonts `fonts` (plain,
/// bold; the labels bold when `bold`), no dividers unless `dividers`, a lone upright rule far
/// from it when `upright`, the rules drawn as `look`.
fn fields_row(
    k: f32,
    look: Look,
    (fonts, bold): ((&str, &str), bool),
    values: &[&[&str]],
    upright: bool,
    dividers: bool,
) -> String {
    let (size, ts) = (8.0 * k, 6.0 * k);
    let (vf, tf) = (fonts.0, if bold { fonts.1 } else { fonts.0 });
    let y = |dy: f32| 600.0 - dy * k;
    let n = values.iter().map(|v| v.len()).max().unwrap_or(1);
    let bottom = y(10.0 * n as f32 + 6.0);
    let mut t = rule(look, 40.0, 540.0, y(-8.0), size);
    for (c, v) in values.iter().enumerate() {
        t += &show(tf, ts, FIELD_XS[c], y(0.0), FIELD_LABELS[c]);
        for (r, line) in v.iter().enumerate() {
            t += &show(vf, size, FIELD_XS[c], y(10.0 * (r as f32 + 1.0)), line);
        }
        if dividers && c > 0 {
            let x = FIELD_XS[c] - 5.0;
            t += &format!("0.5 w {x} {bottom} m {x} {} l S\n", y(-8.0));
        }
    }
    t += &rule(look, 40.0, 540.0, bottom, size);
    if upright {
        t += "0.5 w 30 200 m 30 300 l S\n";
    }
    t
}

const FIELD_LABELS: [&str; 4] = [
    "Rechnungsanschrift",
    "Lieferanschrift",
    "Bankverbindung",
    "Zahlungsziel",
];
const FIELD_XS: [f32; 4] = [40.0, 170.0, 300.0, 430.0];

/// The rows of fields of [`fields_row`]: values of one, two and three lines, of lines of
/// different counts, and an empty field.
fn field_rows() -> Vec<(&'static str, Vec<&'static [&'static str]>)> {
    vec![
        (
            "one",
            vec![&["Muster GmbH"], &["Erika Mustermann"], &["DE00 1234 5678"]],
        ),
        (
            "two",
            vec![
                &["Muster GmbH", "Musterweg 1"],
                &["Erika Mustermann", "Musterstadt"],
                &["DE00 1234 5678", "Musterbank"],
            ],
        ),
        (
            "three",
            vec![
                &["Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
                &["Erika Mustermann", "Musterweg 2", "54321 Musterdorf"],
                &["DE00 1234 5678", "Musterbank", "Muster AG"],
            ],
        ),
        (
            "four",
            vec![
                &["Muster GmbH", "Musterweg 1"],
                &["Erika Mustermann", "Musterstadt"],
                &["DE00 1234 5678", "Musterbank"],
                &["30 Tage"],
            ],
        ),
        (
            "uneven",
            vec![
                &["Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
                &["Erika Mustermann", "Musterstadt"],
                &["DE00 1234 5678", "Musterbank"],
            ],
        ),
        (
            "uneven2",
            vec![
                &["Muster GmbH", "Musterweg 1"],
                &["Erika Mustermann", "Musterstadt", "Musterland"],
                &["DE00 1234 5678", "Musterbank"],
            ],
        ),
        (
            "gap",
            vec![
                &["Muster GmbH", "Musterweg 1"],
                &[],
                &["DE00 1234 5678", "Musterbank"],
                &["30 Tage", "netto"],
            ],
        ),
        // The reviewer's layouts of round 3 (`rr_probe_round3_h1_more`).
        (
            "4-2-2",
            vec![
                &[
                    "Muster GmbH",
                    "Abt. Einkauf",
                    "Musterweg 1",
                    "12345 Musterstadt",
                ],
                &["Erika Mustermann", "Musterstadt"],
                &["DE00 1234 5678", "Musterbank"],
            ],
        ),
        (
            "3-3-2",
            vec![
                &["Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
                &["Erika Mustermann", "Musterweg 2", "54321 Musterdorf"],
                &["DE00 1234 5678", "Musterbank"],
            ],
        ),
        (
            "2-2-3",
            vec![
                &["Muster GmbH", "Musterweg 1"],
                &["Erika Mustermann", "Musterstadt"],
                &["DE00 1234 5678", "Musterbank", "BIC MUSTDEXX"],
            ],
        ),
        (
            "2-2-2-3",
            vec![
                &["Muster GmbH", "Musterweg 1"],
                &["Erika Mustermann", "Musterstadt"],
                &["DE00 1234 5678", "Musterbank"],
                &["30 Tage", "netto", "ohne Abzug"],
            ],
        ),
    ]
}

/// What breaks the invariant of a row of fields read from `out`: see [`fields_broken_in`].
fn fields_broken(out: &ReadOutcome, values: &[&[&str]]) -> Vec<String> {
    let losses: Vec<String> = out.document.losses.iter().map(|l| l.what.clone()).collect();
    fields_broken_in(&placed_tables(out), &losses, values)
}

/// What breaks the invariant of a row of fields read as `tables` with `losses`:
/// - in a table of labels and values (one label a row), a pair of a label with another value
///   than its own whole value (its lines joined);
/// - in a table whose labels stand in one row over their values, what [`table_broken`] finds:
///   each label heads the column its lines stand in, every line whole in the row it stands in
///   (the last row also the lines past the rows), and no cell holds text of two fields;
/// - a label of a field with a value that is in no table, while no loss names every label of
///   the row (or says that its block in columns stays paragraphs).
fn fields_broken_in(
    tables: &[(usize, Vec<Vec<Placed>>)],
    losses: &[String],
    values: &[&[&str]],
) -> Vec<String> {
    let mut broken = Vec::new();
    let columns: Vec<Column> = values
        .iter()
        .enumerate()
        .map(|(c, v)| {
            (
                FIELD_LABELS[c].to_string(),
                v.iter().map(|l| (*l).to_string()).collect(),
            )
        })
        .collect();
    let mut found: Vec<String> = Vec::new();
    for (header_rows, rows) in tables {
        let titled = rows.iter().any(|r| {
            r.iter()
                .filter(|c| FIELD_LABELS.contains(&c.text.as_str()))
                .count()
                >= 2
        });
        if titled {
            broken.extend(table_broken(rows, &columns, &mut found));
            continue;
        }
        if *header_rows != 0 {
            continue;
        }
        for row in rows {
            let [l, v] = row.as_slice() else { continue };
            if let Some(c) = FIELD_LABELS[..values.len()]
                .iter()
                .position(|f| *f == l.text)
            {
                let want = values[c].join(" ");
                let empty = want.is_empty() && v.text == "empty";
                if v.text != want && !empty {
                    broken.push(format!("wrong pair {}: {}, want {want}", l.text, v.text));
                }
                found.push(l.text.clone());
            }
        }
    }
    // A loss names the row's labels, or says that its block in columns stays paragraphs.
    let named = losses.iter().any(|l| {
        l.starts_with("a block of text in columns")
            || (0..values.len()).all(|c| l.contains(FIELD_LABELS[c]) || values[c].is_empty())
    });
    for (c, v) in values.iter().enumerate() {
        if !v.is_empty() && !found.iter().any(|f| f == FIELD_LABELS[c]) && !named {
            broken.push(format!(
                "{:?} is in no table and no loss names the row",
                FIELD_LABELS[c]
            ));
        }
    }
    broken
}

/// The reviewer's finding R3-M3: the check of a row of fields finds a value line in the cell
/// of another field (`Musterstadt` of `Lieferanschrift` is no part of `12345 Musterstadt`),
/// and a value line in the column of another title.
#[test]
fn the_check_of_fields_finds_lines_in_the_cells_of_other_fields() {
    let values: Vec<&[&str]> = vec![
        &["Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
        &["Erika Mustermann", "Musterstadt"],
        &["DE00 1234 5678", "Musterbank"],
    ];
    let cell = |col: usize, text: &str| Placed {
        col,
        span: 1,
        text: text.to_string(),
    };
    let table = |second: [&str; 3]| {
        vec![(
            1,
            vec![
                vec![
                    cell(0, FIELD_LABELS[0]),
                    cell(1, FIELD_LABELS[1]),
                    cell(2, FIELD_LABELS[2]),
                ],
                vec![
                    cell(0, "Muster GmbH"),
                    cell(1, "Erika Mustermann"),
                    cell(2, "DE00 1234 5678"),
                ],
                vec![cell(0, second[0]), cell(1, second[1]), cell(2, second[2])],
            ],
        )]
    };
    let right = table(["Musterweg 1 12345 Musterstadt", "Musterstadt", "Musterbank"]);
    assert!(fields_broken_in(&right, &[], &values).is_empty());
    let lost = table(["Musterweg 1 12345 Musterstadt", "", "Musterbank"]);
    assert!(!fields_broken_in(&lost, &[], &values).is_empty(), "lost");
    let swapped = table(["Musterweg 1 12345 Musterstadt", "Musterbank", "Musterstadt"]);
    assert!(
        !fields_broken_in(&swapped, &[], &values).is_empty(),
        "swapped"
    );
}

/// Rows of fields with values of many lines, of lines of different counts, and with an empty
/// field, in three sizes, every look, Helvetica and Times, plain and bold labels, alone and
/// with an upright rule elsewhere: see [`fields_broken_in`]. (Dividers between the fields
/// without rules at the row's ends make a row of drawn cells whose outer cells are read
/// otherwise, an older gap left to another task.)
#[test]
fn rows_of_fields_with_values_of_many_lines_keep_every_line() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for (name, values) in field_rows() {
        for k in [0.75, 1.0, 1.5] {
            for look in LOOKS {
                for face in FONTS.iter().flat_map(|f| [(*f, false), (*f, true)]) {
                    for (upright, dividers) in [(false, false), (true, false)] {
                        let page = fields_row(k, look, face, &values, upright, dividers);
                        let out = common::read(&page_in(&page, look));
                        reads += 1;
                        for b in fields_broken(&out, &values) {
                            wrong.push(format!(
                                "{name}, {k}x, {look:?}, {face:?}, upright {upright}, dividers \
                                 {dividers}: {b}"
                            ));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 1584);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Line items of three rows under small titles between two rules, the Artikel of one row
/// (the first, the middle or the last) wrapping to a second line `frei Haus` at its column, in
/// three sizes and every look, with and without an upright rule elsewhere: the wrapped line
/// joins its cell, in the last row as well.
#[test]
fn a_wrapped_item_line_joins_its_cell() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for look in LOOKS {
            for upright in [false, true] {
                for wrap in 0..3 {
                    let (size, ts) = (8.0 * k, 6.0 * k);
                    let mut y = 600.0;
                    let mut t = rule(look, 40.0, 540.0, y + 8.0 * k, size);
                    for (x, s) in [(40.0, "Pos"), (80.0, "Artikel"), (300.0, "Menge")] {
                        t += &show("FL", ts, x, y, s);
                    }
                    t += &right(false, ts, 420.0, y, "Preis");
                    t += &right(false, ts, 540.0, y, "Betrag");
                    for (n, item) in ITEMS.iter().take(3).enumerate() {
                        y -= 10.0 * k;
                        t += &show("FL", size, 40.0, y, item[0]);
                        t += &show("FL", size, 80.0, y, item[1]);
                        t += &show("FL", size, 300.0, y, item[2]);
                        t += &right(false, size, 420.0, y, item[3]);
                        t += &right(false, size, 540.0, y, item[4]);
                        if n == wrap {
                            y -= 10.0 * k;
                            t += &show("FL", size, 80.0, y, "frei Haus");
                        }
                    }
                    t += &rule(look, 40.0, 540.0, y - 6.0 * k, size);
                    if upright {
                        t += "0.5 w 30 200 m 30 300 l S\n";
                    }
                    let out = common::read(&page_in(&t, look));
                    let mut want = items_of(3);
                    want[wrap + 1][1] += " frei Haus";
                    let got: Vec<Vec<Vec<String>>> =
                        tables(&out).into_iter().map(|t| t.1).collect();
                    if got != [want] {
                        wrong.push(format!(
                            "{k}x, {look:?}, upright {upright}, wrap {wrap}: {got:?} {:?}",
                            out.document.losses
                        ));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A cell of a read table: the column it starts in (the spans of the cells before it and the
/// row spans of the rows above counted), its column span and its text.
#[derive(Debug, Clone, PartialEq)]
struct Placed {
    col: usize,
    span: usize,
    text: String,
}

/// The tables of the body, each with its header rows and its cells placed in their columns.
fn placed_tables(out: &ReadOutcome) -> Vec<(usize, Vec<Vec<Placed>>)> {
    blocks(out)
        .into_iter()
        .filter_map(|b| match b {
            Block::Table {
                header_rows, rows, ..
            } => {
                // For each column, the rows a cell above still covers.
                let mut covered: Vec<usize> = Vec::new();
                let placed = rows
                    .iter()
                    .map(|r| {
                        let mut col = 0;
                        let mut out = Vec::new();
                        for c in r {
                            while covered.get(col).is_some_and(|n| *n > 0) {
                                col += 1;
                            }
                            let span = c.col_span.max(1) as usize;
                            if covered.len() < col + span {
                                covered.resize(col + span, 0);
                            }
                            for slot in &mut covered[col..col + span] {
                                *slot = c.row_span.max(1) as usize;
                            }
                            out.push(Placed {
                                col,
                                span,
                                text: plain_text(&c.inlines),
                            });
                            col += span;
                        }
                        for slot in &mut covered {
                            *slot = slot.saturating_sub(1);
                        }
                        out
                    })
                    .collect();
                Some((*header_rows, placed))
            }
            _ => None,
        })
        .collect()
}

/// One column of a generated table or row of fields: its title and the lines under it, top
/// down.
type Column = (String, Vec<String>);

/// What breaks the column invariant in the read table `rows` for `columns`, when one of its
/// rows holds two titles or more (a table of labels and values holds one a row):
/// - each title stands in a cell of its own text, and heads the column its lines stand in:
///   the cells under it (in its columns, down to the last row) hold its lines whole and in
///   order, the cell of each row the line of that row, the last row also the lines past the
///   rows (a value of more lines than the others has);
/// - no cell holds text of two columns: every cell with text is a title or stands under one.
///
/// The titles found are added to `found`.
fn table_broken(rows: &[Vec<Placed>], columns: &[Column], found: &mut Vec<String>) -> Vec<String> {
    let mut broken = Vec::new();
    let is_title = |t: &str| columns.iter().any(|c| c.0 == t);
    let Some(top) = rows
        .iter()
        .position(|r| r.iter().filter(|c| is_title(&c.text)).count() >= 2)
    else {
        return broken;
    };
    let below = &rows[top + 1..];
    // The column range of each title found.
    let mut heads: Vec<(usize, usize)> = Vec::new();
    for (title, lines) in columns {
        let Some(head) = rows[top].iter().find(|c| c.text == *title) else {
            if let Some(c) = rows
                .iter()
                .flatten()
                .find(|c| c.text.contains(title.as_str()))
            {
                broken.push(format!("the title {title:?} is in the cell {:?}", c.text));
            }
            continue;
        };
        found.push(title.clone());
        heads.push((head.col, head.col + head.span));
        let under = |r: &Vec<Placed>| -> String {
            r.iter()
                .filter(|c| c.col >= head.col && c.col + c.span <= head.col + head.span)
                .map(|c| c.text.as_str())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let n = below.len();
        for (k, r) in below.iter().enumerate() {
            let want = if k + 1 == n {
                lines.get(k..).map(|l| l.join(" ")).unwrap_or_default()
            } else {
                lines.get(k).cloned().unwrap_or_default()
            };
            let got = under(r);
            if got != want {
                broken.push(format!(
                    "under {title:?}, row {}: {got:?}, want {want:?}",
                    k + 1
                ));
            }
        }
        if n == 0 && !lines.is_empty() {
            broken.push(format!("no row under {title:?}"));
        }
    }
    for (k, r) in rows.iter().enumerate() {
        for c in r.iter().filter(|c| !c.text.is_empty()) {
            let titled = k == top && is_title(&c.text);
            let under = k > top && heads.iter().any(|h| c.col >= h.0 && c.col + c.span <= h.1);
            if !titled && !under {
                broken.push(format!(
                    "the cell {:?} of row {k} is under no one title",
                    c.text
                ));
            }
        }
    }
    broken
}

/// What breaks the column invariant in `out` for a generated table of `columns`: see
/// [`table_broken`]; and no title is lost or silent: one that heads no table stands in no
/// cell, and a loss says that some text is not read as a table.
fn columns_broken(out: &ReadOutcome, columns: &[Column]) -> Vec<String> {
    let mut broken = Vec::new();
    let mut found = Vec::new();
    for (_, rows) in placed_tables(out) {
        broken.extend(table_broken(&rows, columns, &mut found));
    }
    if found.len() < columns.len() && out.document.losses.is_empty() {
        let lost: Vec<&str> = columns
            .iter()
            .map(|c| c.0.as_str())
            .filter(|t| !found.iter().any(|f| f == t))
            .collect();
        broken.push(format!(
            "the titles {lost:?} head no table and no loss says so"
        ));
    }
    broken
}

/// The text fonts of generated tables: Helvetica and Times, each as plain and bold.
const FONTS: [(&str, &str); 2] = [("FL", "FB"), ("FT", "FTB")];

/// How two close column titles stand over their two columns of values in [`close_titles`].
#[derive(Debug, Clone, Copy)]
enum Close {
    /// One show `Menge Einheit`, the quantities flush left under `Menge`, the units flush left
    /// under `Einheit`.
    OneShow,
    /// Two shows `Menge` and `USt-Satz` 0.4 sizes apart, the quantities (a digit) flush right
    /// under the end of `Menge`, the tax rates (a percentage) flush right under the end of
    /// `USt-Satz`.
    FlushRight,
    /// As `FlushRight` with the titles 0.9 sizes apart: the text layer makes two runs of them
    /// (a gap of more than 0.8 sizes), one piece still (a cell gap is 1 size).
    Apart,
    /// As `FlushRight` with the rate title `MwSt.` over rates set without a space (`19%`).
    Rate,
    /// The geometry of a real invoice (finding L1): `Menge` and `MwSt.` 0.45 sizes apart, the
    /// quantities and the rates (`19%`) flush right 0.2 and 0.3 sizes inside the end of their
    /// title by the standard widths (the reader measures 0.29 to 0.38 and 0.3), so each title
    /// starts well before its values and ends a little after them.
    Inside,
    /// As `Inside` with `Anzahl` over a second column of whole numbers: two columns of one
    /// shape, so a title over both (ruling P6-X2).
    SameShape,
}

/// Rows of line items: position, article, quantity, unit, tax rate and amount.
const CLOSE_ROWS: [[&str; 6]; 5] = [
    ["1", "Beratung", "2", "Std", "19 %", "1.000,00"],
    ["2", "Material", "1", "Stk", "7 %", "200,00"],
    ["3", "Versand", "1", "Stk", "19 %", "34,56"],
    ["4", "Montage", "3", "Std", "0 %", "30,00"],
    ["5", "Pruefung", "1", "Stk", "19 %", "99,00"],
];

/// Line items of `n` rows at `k` times 8 pt in the fonts `fonts` (plain, bold), their titles
/// bold when `bold`: `Pos` and `Artikel` at x 40 and 80, two close titles as `close` sets
/// them from x 300, and `Betrag` flush right at x 540, the rows `10 k` apart from the baseline
/// 600 down. Gives the page's template and its columns.
fn close_titles(
    k: f32,
    fonts: (&str, &str),
    bold: bool,
    n: usize,
    close: Close,
) -> (String, Vec<Column>) {
    let size = 8.0 * k;
    let (vf, tf) = (fonts.0, if bold { fonts.1 } else { fonts.0 });
    let flush_right = |font: &str, x: f32, y: f32, t: &str| {
        show(font, size, x - common::font_width(font, t, size), y, t)
    };
    let rows: Vec<[String; 6]> = CLOSE_ROWS[..n]
        .iter()
        .map(|r| {
            r.map(|c| match close {
                Close::Rate | Close::Inside => c.replace(" %", "%"),
                _ => c.to_string(),
            })
        })
        .collect();
    let (second, a, b) = match close {
        Close::OneShow => ("Einheit", 2, 3),
        Close::FlushRight | Close::Apart => ("USt-Satz", 2, 4),
        Close::Rate | Close::Inside => ("MwSt.", 2, 4),
        Close::SameShape => ("Anzahl", 2, 0),
    };
    let apart = match close {
        Close::Apart => 0.9,
        Close::Inside | Close::SameShape => 0.45,
        _ => 0.4,
    };
    // How far inside the end of its title a value ends: the quantity and the rate.
    let inside = if matches!(close, Close::Inside | Close::SameShape) {
        (0.2 * size, 0.3 * size)
    } else {
        (0.0, 0.0)
    };
    let mut t = show(tf, size, 40.0, 600.0, "Pos") + &show(tf, size, 80.0, 600.0, "Artikel");
    // Where the second title starts, and where the values of each title stand.
    let x2 = match close {
        Close::OneShow => {
            t += &show(tf, size, 300.0, 600.0, "Menge Einheit");
            300.0 + common::font_width(tf, "Menge ", size)
        }
        Close::FlushRight | Close::Apart | Close::Rate | Close::Inside | Close::SameShape => {
            let x2 = 300.0 + common::font_width(tf, "Menge", size) + apart * size;
            t += &show(tf, size, 300.0, 600.0, "Menge");
            t += &show(tf, size, x2, 600.0, second);
            x2
        }
    };
    t += &flush_right(tf, 540.0, 600.0, "Betrag");
    for (r, row) in rows.iter().enumerate() {
        let y = 600.0 - 10.0 * k * (r as f32 + 1.0);
        t += &show(vf, size, 40.0, y, &row[0]);
        t += &show(vf, size, 80.0, y, &row[1]);
        match close {
            Close::OneShow => {
                t += &show(vf, size, 300.0, y, &row[a]);
                t += &show(vf, size, x2, y, &row[b]);
            }
            Close::FlushRight | Close::Apart | Close::Rate | Close::Inside | Close::SameShape => {
                let end = 300.0 + common::font_width(tf, "Menge", size) - inside.0;
                t += &flush_right(vf, end, y, &row[a]);
                let end = x2 + common::font_width(tf, second, size) - inside.1;
                t += &flush_right(vf, end, y, &row[b]);
            }
        }
        t += &flush_right(vf, 540.0, y, &row[5]);
    }
    let column = |title: &str, c: usize| -> Column {
        (
            title.to_string(),
            rows.iter().map(|r| r[c].clone()).collect(),
        )
    };
    let columns = vec![
        column("Pos", 0),
        column("Artikel", 1),
        column("Menge", a),
        column(second, b),
        column("Betrag", 5),
    ];
    (t, columns)
}

/// Header cells of the first table: their text and column span.
fn header_cells(out: &ReadOutcome) -> Vec<(String, u32)> {
    blocks(out)
        .into_iter()
        .find_map(|b| match b {
            Block::Table { rows, .. } => Some(
                rows[0]
                    .iter()
                    .map(|c| (plain_text(&c.inlines), c.col_span))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

/// Two column titles closer than a cell gap, set as one show over two columns of values, are
/// cut at the word space over the gutter between the columns: each heads its own column.
#[test]
fn close_column_titles_are_cut() {
    let mut t = show("FB", 8.0, 40.0, 600.0, "Pos")
        + &show("FB", 8.0, 80.0, 600.0, "Artikel")
        + &show("FB", 8.0, 300.0, 600.0, "Menge Einheit")
        + &right(true, 8.0, 540.0, 600.0, "Betrag");
    let rows = [
        ["1", "Beratung", "2", "Std", "1.000,00"],
        ["2", "Material", "1", "Stk", "200,00"],
        ["3", "Versand", "1", "Stk", "34,56"],
    ];
    for (r, row) in rows.iter().enumerate() {
        let y = 590.0 - 10.0 * r as f32;
        t += &show("FL", 8.0, 40.0, y, row[0]);
        t += &show("FL", 8.0, 80.0, y, row[1]);
        t += &show("FL", 8.0, 300.0, y, row[2]);
        t += &show("FL", 8.0, 328.0, y, row[3]);
        t += &right(false, 8.0, 540.0, y, row[4]);
    }
    let out = common::read(&page_in(&t, Look::Plain));
    let mut want = vec![owned(&[&["Pos", "Artikel", "Menge", "Einheit", "Betrag"]])[0].clone()];
    want.extend(owned(&rows.iter().map(|r| &r[..]).collect::<Vec<_>>()));
    assert_eq!(tables(&out), [(1, want)], "{:?}", out.document.losses);
    let spans: Vec<u32> = header_cells(&out).iter().map(|c| c.1).collect();
    assert_eq!(spans, [1, 1, 1, 1, 1]);
}

/// Close column titles over two columns of values, in every size, font and weight and with 2
/// to 5 rows, set as one show over values flush left and as two shows over short values flush
/// right (a quantity and a tax rate): each title heads the column of its values, no cell
/// holds text of two columns, and no title is lost or silent. Each reads as one table of five
/// columns.
#[test]
fn close_column_titles_head_their_columns_in_every_size_and_font() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for close in [
        Close::OneShow,
        Close::FlushRight,
        Close::Apart,
        Close::Rate,
        Close::Inside,
    ] {
        for k in [0.75, 1.0, 1.5] {
            for fonts in FONTS {
                for bold in [false, true] {
                    for n in 2..=5 {
                        let (t, columns) = close_titles(k, fonts, bold, n, close);
                        let out = common::read(&page_in(&t, Look::Plain));
                        reads += 1;
                        let name = format!("{close:?}, {k}x, {fonts:?}, bold {bold}, {n} rows");
                        for b in columns_broken(&out, &columns) {
                            wrong.push(format!("{name}: {b}"));
                        }
                        let titles: Vec<String> = columns.iter().map(|c| c.0.clone()).collect();
                        let head: Vec<String> =
                            header_cells(&out).into_iter().map(|c| c.0).collect();
                        if head != titles {
                            wrong.push(format!("{name}: header {head:?}"));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 240);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Rows of dates for [`spanned_title`]: position, article, from, to and amount.
const DATE_ROWS: [[&str; 5]; 5] = [
    ["1", "Beratung", "01.01.2026", "31.01.2026", "1.000,00"],
    ["2", "Wartung", "01.02.2026", "28.02.2026", "200,00"],
    ["3", "Support", "01.03.2026", "31.03.2026", "34,56"],
    ["4", "Montage", "01.04.2026", "30.04.2026", "30,00"],
    ["5", "Pruefung", "01.05.2026", "31.05.2026", "99,00"],
];

/// Line items of `n` rows of [`DATE_ROWS`] at `k` times 8 pt in the fonts `fonts` (plain,
/// bold), their titles bold when `bold`: `Pos` and `Artikel` at x 40 and 80, `title` over two
/// columns of dates (from x 300, and 1.25 date widths further), from x 300 or centered over
/// both, and `Betrag` flush right at x 540. Gives the template and the columns (the title's
/// lines are the two dates of a row).
fn spanned_title(
    k: f32,
    (fonts, bold): ((&str, &str), bool),
    n: usize,
    title: &str,
    centered: bool,
) -> (String, Vec<Column>) {
    let size = 8.0 * k;
    let (vf, tf) = (fonts.0, if bold { fonts.1 } else { fonts.0 });
    let w = |font: &str, t: &str| common::font_width(font, t, size);
    let date = w(vf, "01.01.2026");
    let x2 = 300.0 + 1.25 * date;
    let tx = if centered {
        (300.0 + x2 + date - w(tf, title)) / 2.0
    } else {
        300.0
    };
    let mut t = show(tf, size, 40.0, 600.0, "Pos")
        + &show(tf, size, 80.0, 600.0, "Artikel")
        + &show(tf, size, tx, 600.0, title)
        + &show(tf, size, 540.0 - w(tf, "Betrag"), 600.0, "Betrag");
    let rows = &DATE_ROWS[..n];
    for (r, row) in rows.iter().enumerate() {
        let y = 600.0 - 10.0 * k * (r as f32 + 1.0);
        t += &show(vf, size, 40.0, y, row[0]);
        t += &show(vf, size, 80.0, y, row[1]);
        t += &show(vf, size, 300.0, y, row[2]);
        t += &show(vf, size, x2, y, row[3]);
        t += &show(vf, size, 540.0 - w(vf, row[4]), y, row[4]);
    }
    let column = |title: &str, f: &dyn Fn(&[&str; 5]) -> String| -> Column {
        (title.to_string(), rows.iter().map(f).collect())
    };
    let columns = vec![
        column("Pos", &|r| r[0].to_string()),
        column("Artikel", &|r| r[1].to_string()),
        column(title, &|r| format!("{} {}", r[2], r[3])),
        column("Betrag", &|r| r[4].to_string()),
    ];
    (t, columns)
}

/// What a title over two columns of dates breaks in `out`: see [`columns_broken`], and its
/// header cell is the whole title with a span of 2.
fn spanned_broken(out: &ReadOutcome, columns: &[Column]) -> Vec<String> {
    let mut b = columns_broken(out, columns);
    let want: Vec<(String, u32)> = columns
        .iter()
        .enumerate()
        .map(|(c, t)| (t.0.clone(), if c == 2 { 2 } else { 1 }))
        .collect();
    if header_cells(out) != want {
        b.push(format!("header {:?}", header_cells(out)));
    }
    b
}

/// Every font and weight of the generated tables.
fn faces() -> Vec<((&'static str, &'static str), bool)> {
    FONTS
        .iter()
        .flat_map(|f| [(*f, false), (*f, true)])
        .collect()
}

/// Guard: one word over two columns of dates has no word space to cut at: its cell keeps
/// its span of two columns, in every size, font and weight and with 2 to 5 rows, and the
/// dates of each row stand under it.
#[test]
fn one_word_title_over_two_columns_stays_spanned() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for face in faces() {
            for n in 2..=5 {
                let (t, columns) = spanned_title(k, face, n, "Leistungszeitraum", false);
                let out = common::read(&page_in(&t, Look::Plain));
                for b in spanned_broken(&out, &columns) {
                    wrong.push(format!("{k}x, {face:?}, {n} rows: {b}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The review's finding K1: a title of two words or more over two columns of dates, set from
/// the first column's start or centered over both, is one title: it is not cut at a word
/// space over the gutter. In every size, font and weight and with 2 to 5 rows its cell keeps
/// its span of two columns, and the dates of each row stand under it.
#[test]
fn titles_of_many_words_over_two_columns_stay_spanned() {
    let titles = [
        "Zeitraum der Leistung",
        "Service period",
        "Gueltig von bis",
        "Leistung vom bis",
        "Leistungszeitraum",
    ];
    let mut wrong = Vec::new();
    let mut reads = 0;
    for title in titles {
        for centered in [false, true] {
            for k in [0.75, 1.0, 1.5] {
                for face in faces() {
                    for n in 2..=5 {
                        let (t, columns) = spanned_title(k, face, n, title, centered);
                        let out = common::read(&page_in(&t, Look::Plain));
                        reads += 1;
                        for b in spanned_broken(&out, &columns) {
                            wrong.push(format!(
                                "{title}, centered {centered}, {k}x, {face:?}, {n} rows: {b}"
                            ));
                        }
                    }
                }
            }
        }
    }
    // A title of two capitalized words, centered over two columns of dates each wider than its
    // words: neither word lines up with or covers its column.
    for k in [0.75, 1.0, 1.5] {
        for face in faces() {
            for n in 2..=5 {
                let (t, columns) = spanned_title(k, face, n, "Service Period", true);
                let out = common::read(&page_in(&t, Look::Plain));
                reads += 1;
                for b in spanned_broken(&out, &columns) {
                    wrong.push(format!(
                        "Service Period, centered, {k}x, {face:?}, {n} rows: {b}"
                    ));
                }
            }
        }
    }
    assert_eq!(reads, 528);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Guard: tax rates flush right under a short title `USt` 0.9 sizes after `Menge` reach left
/// past the title, to a third of a size from the quantities: each quantity and its rate are
/// one piece of text, so the two are one column. In every size, font and weight and with 2 to
/// 5 rows, the titles are not cut, and each row keeps both its values in the cell under both
/// titles (no title over another's value).
#[test]
fn close_titles_over_values_that_touch_stay_one_column() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for bold in [false, true] {
                for n in 2..=5 {
                    let size = 8.0 * k;
                    let (vf, tf) = (fonts.0, if bold { fonts.1 } else { fonts.0 });
                    let w = |font: &str, t: &str| common::font_width(font, t, size);
                    let x2 = 300.0 + w(tf, "Menge") + 0.9 * size;
                    let mut t = show(tf, size, 40.0, 600.0, "Pos")
                        + &show(tf, size, 80.0, 600.0, "Artikel")
                        + &show(tf, size, 300.0, 600.0, "Menge")
                        + &show(tf, size, x2, 600.0, "USt")
                        + &show(tf, size, 540.0 - w(tf, "Betrag"), 600.0, "Betrag");
                    let rows = &CLOSE_ROWS[..n];
                    for (r, row) in rows.iter().enumerate() {
                        let y = 600.0 - 10.0 * k * (r as f32 + 1.0);
                        let end = 300.0 + w(tf, "Menge");
                        t += &show(vf, size, 40.0, y, row[0]);
                        t += &show(vf, size, 80.0, y, row[1]);
                        t += &show(vf, size, end - w(vf, row[2]), y, row[2]);
                        t += &show(vf, size, x2 + w(tf, "USt") - w(vf, row[4]), y, row[4]);
                        t += &show(vf, size, 540.0 - w(vf, row[5]), y, row[5]);
                    }
                    let out = common::read(&page_in(&t, Look::Plain));
                    let column = |title: &str, f: &dyn Fn(&[&str; 6]) -> String| -> Column {
                        (title.to_string(), rows.iter().map(f).collect())
                    };
                    let columns = [
                        column("Pos", &|r| r[0].to_string()),
                        column("Artikel", &|r| r[1].to_string()),
                        column("Menge USt", &|r| format!("{} {}", r[2], r[4])),
                        column("Betrag", &|r| r[5].to_string()),
                    ];
                    let name = format!("{k}x, {fonts:?}, bold {bold}, {n} rows");
                    for b in columns_broken(&out, &columns) {
                        wrong.push(format!("{name}: {b} {:?}", tables(&out)));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Line items of `n` rows of [`ITEMS`] without any rule at `k` times 8 pt in the fonts
/// `fonts` (plain, bold), their titles bold when `bold`, rows `10 k` apart: the `Artikel` of
/// row `wrap` (when there is one) goes on in a line `frei Haus` under it, and `after` (a text
/// and its x) is a line `10 k` under the last. Gives the template and the columns.
fn unruled_items(
    k: f32,
    fonts: (&str, &str),
    bold: bool,
    n: usize,
    wrap: Option<usize>,
    after: Option<(&str, f32)>,
) -> (String, Vec<Column>) {
    let size = 8.0 * k;
    let (vf, tf) = (fonts.0, if bold { fonts.1 } else { fonts.0 });
    let flush_right = |font: &str, x: f32, y: f32, t: &str| {
        show(font, size, x - common::font_width(font, t, size), y, t)
    };
    let mut y = 600.0;
    let mut t = show(tf, size, 40.0, y, "Pos")
        + &show(tf, size, 80.0, y, "Artikel")
        + &show(tf, size, 300.0, y, "Menge")
        + &flush_right(tf, 420.0, y, "Preis")
        + &flush_right(tf, 540.0, y, "Betrag");
    let mut columns: Vec<Column> = ["Pos", "Artikel", "Menge", "Preis", "Betrag"]
        .iter()
        .map(|c| ((*c).to_string(), Vec::new()))
        .collect();
    for (r, item) in ITEMS.iter().take(n).enumerate() {
        y -= 10.0 * k;
        t += &show(vf, size, 40.0, y, item[0]);
        t += &show(vf, size, 80.0, y, item[1]);
        t += &show(vf, size, 300.0, y, item[2]);
        t += &flush_right(vf, 420.0, y, item[3]);
        t += &flush_right(vf, 540.0, y, item[4]);
        for (c, column) in columns.iter_mut().enumerate() {
            column.1.push(item[c].to_string());
        }
        if wrap == Some(r) {
            y -= 10.0 * k;
            t += &show(vf, size, 80.0, y, "frei Haus");
            columns[1].1[r] += " frei Haus";
        }
    }
    if let Some((text, x)) = after {
        t += &show(vf, size, x, y - 10.0 * k, text);
    }
    (t, columns)
}

/// Line items without rules whose `Artikel` goes on in a second line, in the first, a middle
/// or the last row, in every size, font and weight and with 2 to 5 rows: every title heads
/// the column of its values. A wrap between two rows joins its cell. A wrap of the last row
/// looks like a note under the table: it stays out of the table, its text is read, and a loss
/// says that it may go on in the last row.
#[test]
fn a_wrapped_item_line_without_rules_joins_its_cell_or_is_named() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for bold in [false, true] {
                for n in 2..=5 {
                    for wrap in 0..n {
                        let (t, mut columns) = unruled_items(k, fonts, bold, n, Some(wrap), None);
                        let last = wrap + 1 == n;
                        if last {
                            let item = &mut columns[1].1[wrap];
                            *item = item.trim_end_matches(" frei Haus").to_string();
                        }
                        let out = common::read(&page_in(&t, Look::Plain));
                        reads += 1;
                        let name = format!("{k}x, {fonts:?}, bold {bold}, {n} rows, wrap {wrap}");
                        let mut b = columns_broken(&out, &columns);
                        if tables(&out).len() != 1 {
                            b.push(format!("tables {:?}", tables(&out)));
                        }
                        let losses: Vec<&str> = out
                            .document
                            .losses
                            .iter()
                            .map(|l| l.what.as_str())
                            .collect();
                        let want: &[&str] = if last { &[WRAP_LOSS] } else { &[] };
                        if losses != want {
                            b.push(format!("losses {losses:?}"));
                        }
                        let in_cell = tables(&out)
                            .iter()
                            .any(|t| t.1.iter().flatten().any(|c| c.contains("frei Haus")));
                        if in_cell == last || !common::all_text(&out).contains("frei Haus") {
                            b.push(format!("the wrap is in a cell: {in_cell}"));
                        }
                        wrong.extend(b.into_iter().map(|b| format!("{name}: {b}")));
                    }
                }
            }
        }
    }
    assert_eq!(reads, 168);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Items without rules whose period wraps a date range into a second line (`01.10.2026 -`
/// over `31.10.2026`), in every size and font: the second line is never a row of its own
/// (only an amount under an amount is one); between two rows it goes on in its cell. (The
/// cell's text joins the two lines after the dash without a space, as after a hyphen.) All
/// values are invented.
#[test]
fn a_wrapped_date_range_stays_in_its_cell() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            let size = 8.0 * k;
            let flush_right = |x: f32, y: f32, t: &str| {
                show(
                    fonts.0,
                    size,
                    x - common::font_width(fonts.0, t, size),
                    y,
                    t,
                )
            };
            let mut y = 600.0;
            let mut t = show(fonts.1, size, 40.0, y, "Pos")
                + &show(fonts.1, size, 80.0, y, "Zeitraum")
                + &show(fonts.1, size, 300.0, y, "Art")
                + &flush_right(540.0, y, "Betrag");
            let rows = [
                ("1", "01.10.2026 -", "31.10.2026", "Miete", "1.000,00"),
                ("2", "01.11.2026 -", "30.11.2026", "Miete", "1.000,00"),
                ("3", "01.12.2026 -", "31.12.2026", "Pacht", "200,00"),
            ];
            for (pos, from, to, art, amount) in rows {
                y -= 10.0 * k;
                t += &show(fonts.0, size, 40.0, y, pos);
                t += &show(fonts.0, size, 80.0, y, from);
                t += &show(fonts.0, size, 300.0, y, art);
                t += &flush_right(540.0, y, amount);
                y -= 10.0 * k;
                t += &show(fonts.0, size, 80.0, y, to);
            }
            let out = common::read(&page_in(&t, Look::Plain));
            let found = tables(&out);
            let cells: Vec<String> = found
                .iter()
                .flat_map(|t| t.1.iter().filter_map(|r| r.get(1)))
                .map(|c| c.split_whitespace().collect())
                .collect();
            let alone = found.iter().flat_map(|t| &t.1).any(|r| {
                rows.iter().any(|row| r.get(1).is_some_and(|c| c == row.2))
                    && r.iter().filter(|c| !c.is_empty()).count() == 1
            });
            if alone {
                wrong.push(format!("{k}x, {fonts:?}: a date is a row: {found:?}"));
            }
            for (_, from, to, _, _) in &rows[..2] {
                let cell: String = format!("{from}{to}").split_whitespace().collect();
                if !cells.contains(&cell) {
                    wrong.push(format!("{k}x, {fonts:?}: no cell {from} {to}: {found:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Pay slip lines without rules, a header and three rows, then a blank row a little over one
/// line high (2 to 2.2 pitches), then two or four rows more in the same columns, at sizes 6 to
/// 14 in both fonts: one table with every row, as the rows after the gap stand in the table's
/// own columns. All values are invented.
#[test]
fn a_blank_row_keeps_one_table() {
    let rows = [
        ("1000", "Gehalt", "3.000,00"),
        ("1010", "Zulage", "150,00"),
        ("1020", "Fahrgeld", "45,67"),
        ("2000", "Abzug", "12,34"),
        ("2010", "Kantine", "8,90"),
        ("2020", "Miete", "1,11"),
        ("2030", "Strom", "2,22"),
    ];
    let mut wrong = Vec::new();
    for n in [2usize, 4] {
        for blank in [2.0f32, 2.1, 2.2] {
            for size in [6.0f32, 7.5, 9.0, 11.0, 14.0] {
                for fonts in FONTS {
                    let pitch = 1.25 * size;
                    let fr = |x: f32, y: f32, t: &str| {
                        show(
                            fonts.0,
                            size,
                            x - common::font_width(fonts.0, t, size),
                            y,
                            t,
                        )
                    };
                    let mut y = 700.0;
                    let mut t = show(fonts.1, size, 40.0, y, "Lohnart")
                        + &show(fonts.1, size, 40.0 + 8.0 * size, y, "Bezeichnung")
                        + &fr(40.0 + 40.0 * size, y, "Betrag");
                    for (i, (a, b, c)) in rows[..3 + n].iter().enumerate() {
                        y -= if i == 3 { blank * pitch } else { pitch };
                        t += &show(fonts.0, size, 40.0, y, a);
                        t += &show(fonts.0, size, 40.0 + 8.0 * size, y, b);
                        t += &fr(40.0 + 40.0 * size, y, c);
                    }
                    let out = common::read(&page_in(&t, Look::Plain));
                    let mut want = vec![vec![
                        "Lohnart".to_string(),
                        "Bezeichnung".into(),
                        "Betrag".into(),
                    ]];
                    want.extend(
                        rows[..3 + n]
                            .iter()
                            .map(|r| vec![r.0.to_string(), r.1.into(), r.2.into()]),
                    );
                    let found = tables(&out);
                    if found.len() != 1 || found[0].1 != want {
                        wrong.push(format!(
                            "{n} rows, blank {blank}, {size}, {fonts:?}: {found:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Task 15b group 20: line items with a dashed rule under the header and an end rule over the
/// totals, each dash a fifth of the text size long and half a point thick (thin both ways at
/// small sizes), the page upright or turned a quarter either way: the items and the totals
/// are two tables, as upright. Sizes 6 to 14 in both fonts.
#[test]
fn dashed_end_rules_part_tables_on_a_turned_page() {
    let items = [
        ["Pos", "Artikel", "Menge", "Preis", "Betrag"],
        ["1", "Beratung", "2", "500,00", "1.000,00"],
        ["2", "Material", "1", "200,00", "200,00"],
        ["3", "Versand", "1", "34,56", "34,56"],
    ];
    let totals = [
        ["Nettobetrag", "1.234,56"],
        ["USt 19 %", "234,57"],
        ["Gesamtbetrag", "1.469,13"],
    ];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for fonts in FONTS {
            for (turn, clockwise) in [(false, false), (true, false), (true, true)] {
                let k = size / 8.0;
                let y = |dy: f32| 700.0 - dy * k;
                let right = |font: &str, x: f32, y: f32, t: &str| {
                    show(font, size, x - common::font_width(font, t, size), y, t)
                };
                let dashed = |y: f32| {
                    let mut out = String::new();
                    let mut x = 40.0;
                    while x < 540.0 {
                        let dash = (0.2 * size).min(540.0 - x);
                        out += &format!("{x} {} {dash} 0.5 re f\n", y - 0.25);
                        x += 0.3 * size;
                    }
                    out
                };
                let xs = [40.0, 80.0, 300.0];
                let mut t = String::new();
                for (n, row) in items.iter().enumerate() {
                    let font = if n == 0 { fonts.1 } else { fonts.0 };
                    let at = y(10.0 * n as f32);
                    for (x, c) in xs.iter().zip(&row[..3]) {
                        t += &show(font, size, *x, at, c);
                    }
                    t += &right(font, 420.0, at, row[3]);
                    t += &right(font, 540.0, at, row[4]);
                }
                t += &dashed(y(4.0));
                t += &dashed(y(36.0));
                for (n, row) in totals.iter().enumerate() {
                    let at = y(44.0 + 10.0 * n as f32);
                    t += &show(fonts.0, size, 340.0, at, row[0]);
                    t += &right(fonts.0, 540.0, at, row[1]);
                }
                let page = FormPage {
                    template: format!("{t}{}", body(7.0, 420.0)),
                    turn,
                    clockwise,
                    ..FormPage::default()
                };
                let out = common::read(&print_form(&[page], "", ""));
                let found = tables(&out);
                let owned = |w: &[&[&str]]| -> Vec<Vec<String>> {
                    w.iter()
                        .map(|r| r.iter().map(|c| (*c).to_string()).collect())
                        .collect()
                };
                let item_rows: Vec<&[&str]> = items.iter().map(|r| &r[..]).collect();
                let total_rows: Vec<&[&str]> = totals.iter().map(|r| &r[..]).collect();
                for want in [owned(&item_rows), owned(&total_rows)] {
                    if !found.iter().any(|t| t.1 == want) {
                        wrong.push(format!(
                            "{size}, {fonts:?}, turn {turn} {clockwise}: no {:?} in {found:?}",
                            want[0]
                        ));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Task 15b part 2c re-review r1 (Minor 1): a row of text only after a blank row, over rows of
/// the first table's own shape (`Hinweis | siehe Anlage | Seite` over `3000 | Bonus | 500,00`),
/// is no header of a second table: it is a row of the first table or text of its own, never a
/// table's header row. Sizes 6 to 14, both fonts.
#[test]
fn a_note_row_after_a_blank_row_heads_no_table() {
    let first = [
        ["Lohnart", "Bezeichnung", "Betrag"],
        ["1000", "Gehalt", "3.000,00"],
        ["1010", "Zulage", "150,00"],
        ["1020", "Fahrgeld", "45,67"],
    ];
    let after = [
        ["Hinweis", "siehe Anlage", "Seite"],
        ["3000", "Bonus", "500,00"],
        ["3010", "Praemie", "50,00"],
    ];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for fonts in FONTS {
            let k = size / 9.0;
            let pitch = 1.25 * size;
            let mut t = String::new();
            let mut y = 700.0;
            for (n, row) in first.iter().chain(&after).enumerate() {
                if n == 4 {
                    y -= 1.2 * pitch;
                }
                let font = if n == 0 { fonts.1 } else { fonts.0 };
                t += &show(font, size, 40.0, y, row[0]);
                t += &show(font, size, 40.0 + 72.0 * k, y, row[1]);
                let x = 40.0 + 360.0 * k - common::font_width(font, row[2], size);
                t += &show(font, size, x, y, row[2]);
                y -= pitch;
            }
            let out = common::read(&page_in(&t, Look::Plain));
            let found = tables(&out);
            let headed = found
                .iter()
                .any(|t| t.0 > 0 && t.1.first().is_some_and(|r| r[0] == "Hinweis"));
            if headed {
                wrong.push(format!("{size}, {fonts:?}: {found:?}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Task 15b part 2b re-review (probe C) and part 2c review (Important 4, Minors 1 and 2; ruling
/// P6-X): a second table under the first after a blank row (2.2 pitches), no rule between,
/// whose middle column ends where the first table's middle title ends, so that its lines stand
/// in the first table's columns: its header line starts a table of its own, and no header
/// becomes a row of the first, whether the header is bold, plain, a size step larger, bold
/// after a bold total row, holds years, or has two cells over two columns. Sizes 6 to 14 in
/// both fonts.
#[test]
fn a_header_after_a_blank_row_starts_a_table_of_its_own() {
    let first = [
        ["Lohnart", "Bezeichnung", "Betrag"],
        ["1000", "Gehalt", "3.000,00"],
        ["1010", "Zulage", "150,00"],
        ["1020", "Fahrgeld", "45,67"],
    ];
    let body = [["7001", "12", "45,67"], ["7002", "3", "8,90"]];
    // A name, whether the header is bold, its size factor, a bold total row before the gap,
    // the header cells (an empty middle cell: two columns).
    let cases: [(&str, bool, f32, bool, [&str; 3]); 6] = [
        ("bold", true, 1.0, false, ["Kennung", "Menge", "Wert"]),
        ("plain", false, 1.0, false, ["Kennung", "Menge", "Wert"]),
        ("larger", false, 1.15, false, ["Kennung", "Menge", "Wert"]),
        (
            "after a bold total",
            true,
            1.0,
            true,
            ["Kennung", "Menge", "Wert"],
        ),
        ("years", true, 1.0, false, ["Bezeichnung", "2025", "2026"]),
        ("two columns", true, 1.0, false, ["Kennung", "", "Wert"]),
    ];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for fonts in FONTS {
            for (name, bold, step, total, header) in cases {
                let k = size / 9.0;
                let pitch = 1.25 * size;
                let right = |font: &str, size: f32, x: f32, y: f32, t: &str| {
                    show(font, size, x - common::font_width(font, t, size), y, t)
                };
                let mut t = String::new();
                let mut y = 700.0;
                let mut want_first: Vec<Vec<String>> = Vec::new();
                for (n, row) in first.iter().enumerate() {
                    let font = if n == 0 { fonts.1 } else { fonts.0 };
                    t += &show(font, size, 40.0, y, row[0]);
                    t += &show(font, size, 40.0 + 72.0 * k, y, row[1]);
                    t += &right(font, size, 40.0 + 360.0 * k, y, row[2]);
                    want_first.push(row.iter().map(|c| (*c).to_string()).collect());
                    y -= pitch;
                }
                if total {
                    t += &show(fonts.1, size, 40.0, y, "Summe");
                    t += &right(fonts.1, size, 40.0 + 360.0 * k, y, "3.195,67");
                    want_first.push(vec!["Summe".into(), String::new(), "3.195,67".into()]);
                    y -= pitch;
                }
                y -= 1.2 * pitch;
                let mid = 40.0 + 72.0 * k + common::font_width(fonts.1, "Bezeichnung", size);
                let hf = if bold { fonts.1 } else { fonts.0 };
                let hs = step * size;
                t += &show(hf, hs, 40.0, y, header[0]);
                if !header[1].is_empty() {
                    t += &right(hf, hs, mid, y, header[1]);
                }
                t += &right(hf, hs, 40.0 + 360.0 * k, y, header[2]);
                let two = header[1].is_empty();
                let cells = |r: [&str; 3]| -> Vec<String> {
                    r.iter()
                        .filter(|c| !two || !c.is_empty())
                        .map(|c| (*c).to_string())
                        .collect()
                };
                let mut want_second = vec![cells(header)];
                for row in body {
                    y -= pitch;
                    t += &show(fonts.0, size, 40.0, y, row[0]);
                    if !two {
                        t += &right(fonts.0, size, mid, y, row[1]);
                    }
                    t += &right(fonts.0, size, 40.0 + 360.0 * k, y, row[2]);
                    want_second.push(cells(if two { [row[0], "", row[2]] } else { row }));
                }
                let out = common::read(&page_in(&t, Look::Plain));
                let found = tables(&out);
                for want in [&want_first, &want_second] {
                    if !found.iter().any(|t| t.1 == *want) {
                        wrong.push(format!(
                            "{name}, {size}, {fonts:?}: no table {want:?} in {found:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Line items without rules with a line of amounts under the amounts of the first item, and an
/// amount alone flush right under the last item at the row pitch, at sizes 6 to 14 in both
/// fonts: the line of amounts is a row of its own, and the lone amount (a total, or one more
/// amount: the layout does not tell) stays out of the table (the K2 case).
#[test]
fn a_total_under_stacked_amounts_stays_out() {
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for fonts in FONTS {
            let k = size / 8.0;
            let fr = |x: f32, y: f32, t: &str| {
                show(
                    fonts.0,
                    size,
                    x - common::font_width(fonts.0, t, size),
                    y,
                    t,
                )
            };
            let mut y = 700.0;
            let mut t = show(fonts.1, size, 40.0, y, "Pos")
                + &show(fonts.1, size, 40.0 + 40.0 * k, y, "Artikel")
                + &fr(40.0 + 300.0 * k, y, "Preis")
                + &fr(40.0 + 420.0 * k, y, "Betrag");
            let rows: [(&str, &str, &str, &str); 3] = [
                ("1", "Beratung", "500,00", "1.000,00"),
                ("", "", "50,00", "100,00"),
                ("2", "Material", "200,00", "200,00"),
            ];
            for (a, b, c, d) in rows {
                y -= 10.0 * k;
                if !a.is_empty() {
                    t += &show(fonts.0, size, 40.0, y, a);
                    t += &show(fonts.0, size, 40.0 + 40.0 * k, y, b);
                }
                t += &fr(40.0 + 300.0 * k, y, c);
                t += &fr(40.0 + 420.0 * k, y, d);
            }
            y -= 10.0 * k;
            t += &fr(40.0 + 420.0 * k, y, "1.300,00");
            let out = common::read(&page_in(&t, Look::Plain));
            let found = tables(&out);
            let want: Vec<Vec<String>> = [
                ["Pos", "Artikel", "Preis", "Betrag"],
                ["1", "Beratung", "500,00", "1.000,00"],
                ["", "", "50,00", "100,00"],
                ["2", "Material", "200,00", "200,00"],
            ]
            .iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect();
            if found.len() != 1
                || found[0].1 != want
                || !common::all_text(&out).contains("1.300,00")
            {
                wrong.push(format!("{size}, {fonts:?}: {found:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Line items without rules where a cell holds two whole numbers (two article numbers in one
/// cell) or a percent over a percent, at sizes 6 to 14 in both fonts: the second line goes on
/// in its cell. Only an amount under an amount is a row of its own.
#[test]
fn whole_numbers_and_percents_stay_in_their_cell() {
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for fonts in FONTS {
            let k = size / 8.0;
            let fr = |x: f32, y: f32, t: &str| {
                show(
                    fonts.0,
                    size,
                    x - common::font_width(fonts.0, t, size),
                    y,
                    t,
                )
            };
            let mut y = 700.0;
            let mut t = show(fonts.1, size, 40.0, y, "Artikel")
                + &fr(40.0 + 200.0 * k, y, "Nummer")
                + &fr(40.0 + 300.0 * k, y, "Satz")
                + &fr(40.0 + 420.0 * k, y, "Betrag");
            // The second line of a cell: in the column of numbers, or of rates.
            let rows = [
                (
                    "Beratung",
                    "4711",
                    "19 %",
                    "1.000,00",
                    Some((200.0, "4712")),
                ),
                ("Material", "4720", "19 %", "200,00", Some((300.0, "7 %"))),
                ("Versand", "4730", "7 %", "34,56", None),
            ];
            for (a, n, r, d, second) in rows {
                y -= 10.0 * k;
                t += &show(fonts.0, size, 40.0, y, a);
                t += &fr(40.0 + 200.0 * k, y, n);
                t += &fr(40.0 + 300.0 * k, y, r);
                t += &fr(40.0 + 420.0 * k, y, d);
                if let Some((x, text)) = second {
                    y -= 10.0 * k;
                    t += &fr(40.0 + x * k, y, text);
                }
            }
            let out = common::read(&page_in(&t, Look::Plain));
            let found = tables(&out);
            let want: Vec<Vec<String>> = [
                ["Artikel", "Nummer", "Satz", "Betrag"],
                ["Beratung", "4711 4712", "19 %", "1.000,00"],
                ["Material", "4720", "19 % 7 %", "200,00"],
                ["Versand", "4730", "7 %", "34,56"],
            ]
            .iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect();
            if found.len() != 1 || found[0].1 != want {
                wrong.push(format!("{size}, {fonts:?}: {found:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A table without rules beside a margin column of notes, at sizes 6 to 14 in both fonts:
/// the notes run on over and under the table's rows (one note on each row's baseline and one
/// line over and under them), or stand beside some rows only. The column is no column of the
/// table read apart: no loss says so.
#[test]
fn a_margin_column_beside_a_table_is_no_column_read_apart() {
    let rows = [
        ("1000", "Gehalt", "3.000,00"),
        ("1010", "Zulage", "150,00"),
        ("1020", "Fahrgeld", "45,67"),
        ("2000", "Abzug", "12,34"),
    ];
    let notes = [
        "Hinweise zur",
        "Abrechnung",
        "siehe Anlage",
        "und Merkblatt",
        "der Kasse",
        "im Anhang",
    ];
    let mut wrong = Vec::new();
    for every in [true, false] {
        for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
            for fonts in FONTS {
                let pitch = 1.25 * size;
                let fr = |x: f32, y: f32, t: &str| {
                    show(
                        fonts.0,
                        size,
                        x - common::font_width(fonts.0, t, size),
                        y,
                        t,
                    )
                };
                let top = 700.0;
                let mut t = show(fonts.1, size, 40.0, top, "Lohnart")
                    + &show(fonts.1, size, 40.0 + 8.0 * size, top, "Bezeichnung")
                    + &fr(40.0 + 30.0 * size, top, "Betrag");
                for (i, (a, b, c)) in rows.iter().enumerate() {
                    let y = top - pitch * (i as f32 + 1.0);
                    t += &show(fonts.0, size, 40.0, y, a);
                    t += &show(fonts.0, size, 40.0 + 8.0 * size, y, b);
                    t += &fr(40.0 + 30.0 * size, y, c);
                }
                // The notes from the header's baseline down, a line under the last row too; or
                // beside the first two rows only.
                let x = 40.0 + 36.0 * size;
                for (k, n) in notes.iter().enumerate() {
                    if !every && !(1..=2).contains(&k) {
                        continue;
                    }
                    t += &show(fonts.0, 0.8 * size, x, top - pitch * k as f32, n);
                }
                let out = common::read(&page_in(&t, Look::Plain));
                if out
                    .document
                    .losses
                    .iter()
                    .any(|l| l.what.contains("may go on in a column beside it"))
                {
                    wrong.push(format!(
                        "every {every}, {size}, {fonts:?}: {:?}",
                        out.document.losses
                    ));
                }
                if !common::all_text(&out).contains("siehe Anlage") {
                    wrong.push(format!("every {every}, {size}, {fonts:?}: a note is lost"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// How a line under the items of [`lines_under_items_without_rules_are_named_or_stay_paragraphs`]
/// is set: in the items' font and size, at 0.75 times their size, or in the other family.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Set {
    Same,
    Smaller,
    OtherFamily,
}

/// Lines right under line items without rules, at row pitch, in every size, font and weight
/// and with 2 to 5 rows (the review's finding K2 and its ruling): none joins the table, and
/// each stays text out of it.
/// - A line that starts at a text column's start, in the column's font family and size, may go
///   on in the last row, whatever its first letter: a capitalized wrap `Frei Haus`, a small
///   one `frei Haus`, `Danke` and a signature. A loss says so.
/// - No loss for the others: a page number by its shape (`Seite 1`, `Seite 1 von 2`,
///   `Page 2 of 3` under `Artikel`, `Seite 1/2` flush right), a total or an amount flush right
///   under `Betrag`, `Danke` in a smaller size or in the other family, a note across the
///   gutter between `Pos` and `Artikel`, a line under `Artikel` that runs on into `Menge`, a
///   word under the numbers of `Pos` or `Menge`, and a word that starts half a size into
///   `Artikel`.
#[test]
fn lines_under_items_without_rules_are_named_or_stay_paragraphs() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for bold in [false, true] {
                for (text, x, set, loss) in [
                    ("Frei Haus", Some(80.0), Set::Same, true),
                    ("frei Haus", Some(80.0), Set::Same, true),
                    ("Danke", Some(80.0), Set::Same, true),
                    ("Seite 1", Some(80.0), Set::Same, false),
                    ("Seite 1 von 2", Some(80.0), Set::Same, false),
                    ("Page 2 of 3", Some(80.0), Set::Same, false),
                    ("Seite 1/2", None, Set::Same, false),
                    ("1.469,13", None, Set::Same, false),
                    ("Gesamt 1.469,13", None, Set::Same, false),
                    ("Danke", Some(80.0), Set::Smaller, false),
                    ("Danke", Some(80.0), Set::OtherFamily, false),
                    ("Alle Preise in EUR", Some(40.0), Set::Same, false),
                    (
                        "Zahlbar bis zum 01.03.2026 ohne Abzug, 2 % Skonto bei Zahlung in 10 Tagen",
                        Some(80.0),
                        Set::Same,
                        false,
                    ),
                    ("Hinweis", Some(40.0), Set::Same, false),
                    ("Summe", Some(300.0), Set::Same, false),
                    ("inkl.", Some(80.0 + 4.0 * k), Set::Same, false),
                    ("i.A. Erika Mustermann", Some(80.0), Set::Same, true),
                ] {
                    let size = match set {
                        Set::Smaller => 6.0 * k,
                        _ => 8.0 * k,
                    };
                    let font = match (set, fonts.0) {
                        (Set::OtherFamily, "FL") => "FT",
                        (Set::OtherFamily, _) => "FL",
                        _ => fonts.0,
                    };
                    // Flush right at x 540 when no x is given.
                    let x = x.unwrap_or_else(|| 540.0 - common::font_width(font, text, size));
                    for n in 2..=5 {
                        let (mut t, columns) = unruled_items(k, fonts, bold, n, None, None);
                        t += &show(font, size, x, 600.0 - 10.0 * k * (n as f32 + 1.0), text);
                        let out = common::read(&page_in(&t, Look::Plain));
                        reads += 1;
                        let name =
                            format!("{k}x, {fonts:?}, bold {bold}, {n} rows, {text:?}, {set:?}");
                        let mut b = columns_broken(&out, &columns);
                        if tables(&out).len() != 1 {
                            b.push(format!("tables {:?}", tables(&out)));
                        }
                        let in_cell = tables(&out)
                            .iter()
                            .any(|t| t.1.iter().flatten().any(|c| c.contains(text)));
                        if in_cell || !common::all_text(&out).contains(text) {
                            b.push(format!("{text:?} is in a cell or lost"));
                        }
                        // At the smallest scale with few rows the `Zahlbar bis` line stands
                        // in the top third: its date, which nothing marks, has a loss of its
                        // own.
                        let losses: Vec<&str> = out
                            .document
                            .losses
                            .iter()
                            .map(|l| l.what.as_str())
                            .filter(|w| !w.starts_with("1 date stands in the head"))
                            .collect();
                        let want: &[&str] = if loss { &[WRAP_LOSS] } else { &[] };
                        if losses != want {
                            b.push(format!("losses {losses:?}"));
                        }
                        wrong.extend(b.into_iter().map(|b| format!("{name}: {b}")));
                    }
                }
            }
        }
    }
    assert_eq!(reads, 816);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The loss for a line under a table that may go on in its last row.
const WRAP_LOSS: &str = "a line right under a table on page 1 may go on in its last row: it \
                         is read as a paragraph after the table";

/// Line items of `n` rows of [`ITEMS`] between a rule over their small titles (`6 k` pt,
/// bold when `bold`) and one under the items, at `k` times 8 pt in the fonts `fonts` (plain,
/// bold), drawn as `look`; with `note` (a text, its x and its distance in `k` points), a line
/// over the lower rule under the last item. Gives the template and the columns.
fn framed_items(
    k: f32,
    look: Look,
    fonts: (&str, &str),
    bold: bool,
    n: usize,
    note: Option<(&str, f32, f32)>,
) -> (String, Vec<Column>) {
    let (size, ts) = (8.0 * k, 6.0 * k);
    let (vf, tf) = (fonts.0, if bold { fonts.1 } else { fonts.0 });
    let flush_right = |font: &str, size: f32, x: f32, y: f32, t: &str| {
        show(font, size, x - common::font_width(font, t, size), y, t)
    };
    let mut y = 600.0;
    let mut t = rule(look, 40.0, 540.0, y + 8.0 * k, size);
    for (x, s) in [(40.0, "Pos"), (80.0, "Artikel"), (300.0, "Menge")] {
        t += &show(tf, ts, x, y, s);
    }
    t += &flush_right(tf, ts, 420.0, y, "Preis");
    t += &flush_right(tf, ts, 540.0, y, "Betrag");
    for item in ITEMS.iter().take(n) {
        y -= 10.0 * k;
        t += &show(vf, size, 40.0, y, item[0]);
        t += &show(vf, size, 80.0, y, item[1]);
        t += &show(vf, size, 300.0, y, item[2]);
        t += &flush_right(vf, size, 420.0, y, item[3]);
        t += &flush_right(vf, size, 540.0, y, item[4]);
    }
    if let Some((text, x, gap)) = note {
        y -= gap * k;
        t += &show(vf, size, x, y, text);
    }
    t += &rule(look, 40.0, 540.0, y - 6.0 * k, size);
    let columns = ["Pos", "Artikel", "Menge", "Preis", "Betrag"]
        .iter()
        .enumerate()
        .map(|(c, title)| {
            (
                (*title).to_string(),
                ITEMS.iter().take(n).map(|r| r[c].to_string()).collect(),
            )
        })
        .collect();
    (t, columns)
}

/// The reviewer's probe R3-M1: a note under the last item, over the rule under the items,
/// that runs over the gutter between `Pos` and `Artikel` (from x 40, one row or two under the
/// last item) or between `Artikel` and `Menge` (a long one from x 80 or from x 40), in every
/// size, look, font and weight and with 2 to 5 rows:
/// the note joins no cell, every title heads the column of its values, and the note's text is
/// read.
#[test]
fn a_note_across_a_gutter_in_a_frame_of_items_joins_no_cell() {
    let long =
        "Alle Preise verstehen sich in EUR zuzueglich der gesetzlichen Umsatzsteuer von 19 Prozent";
    let notes = [
        ("Alle Preise in EUR", 40.0, 10.0),
        ("Alle Preise in EUR", 40.0, 18.0),
        (long, 80.0, 10.0),
        (long, 40.0, 10.0),
    ];
    let mut wrong = Vec::new();
    let mut reads = 0;
    for (note, x, gap) in notes {
        {
            for k in [0.75, 1.0, 1.5] {
                for look in LOOKS {
                    for fonts in FONTS {
                        for bold in [false, true] {
                            for n in 2..=5 {
                                let (t, columns) =
                                    framed_items(k, look, fonts, bold, n, Some((note, x, gap)));
                                let out = common::read(&page_in(&t, look));
                                reads += 1;
                                let name = format!(
                                    "{:.12} at {x}, {gap}, {k}x, {look:?}, {fonts:?}, bold {bold}, \
                                     {n} rows",
                                    note
                                );
                                let mut b = columns_broken(&out, &columns);
                                let in_cell = tables(&out)
                                    .iter()
                                    .any(|t| t.1.iter().flatten().any(|c| c.contains("Preise")));
                                if in_cell || !common::all_text(&out).contains(note) {
                                    b.push(format!(
                                        "the note is in a cell or lost: {:?}",
                                        tables(&out)
                                    ));
                                }
                                wrong.extend(b.into_iter().map(|b| format!("{name}: {b}")));
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 1152);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The reviewer's finding R3-M4: a row of fields read as a table whose first row holds the
/// labels has them as its header row, also when no column under them holds numbers (three
/// lines a field), in every size, look, font and weight, alone and with an upright rule
/// elsewhere.
#[test]
fn the_labels_over_a_row_of_fields_read_as_a_table_are_its_header() {
    let mut wrong = Vec::new();
    let mut tabled = 0;
    for (name, values) in field_rows() {
        for k in [0.75, 1.0, 1.5] {
            for (look, face) in LOOKS.iter().flat_map(|l| {
                FONTS
                    .iter()
                    .flat_map(move |f| [(*l, (*f, false)), (*l, (*f, true))])
            }) {
                for upright in [false, true] {
                    let page = fields_row(k, look, face, &values, upright, false);
                    let out = common::read(&page_in(&page, look));
                    for (header_rows, rows) in tables(&out) {
                        let titles = rows[0]
                            .iter()
                            .filter(|c| FIELD_LABELS.contains(&c.as_str()))
                            .count();
                        if titles < 2 {
                            continue;
                        }
                        tabled += 1;
                        if header_rows != 1 {
                            wrong.push(format!(
                                "{name}, {k}x, {look:?}, {face:?}, upright {upright}: header \
                                 rows {header_rows}"
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(tabled >= 800, "{tabled}");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Close column titles as in [`close_column_titles_head_their_columns_in_every_size_and_font`]
/// between a rule over the titles and one under the items, drawn in every look: the frame of
/// titles over items goes to the table pass, and each title heads the column of its values.
#[test]
fn close_column_titles_in_a_frame_head_their_columns() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for close in [
        Close::OneShow,
        Close::FlushRight,
        Close::Apart,
        Close::Rate,
        Close::Inside,
    ] {
        for k in [0.75, 1.0, 1.5] {
            for look in LOOKS {
                for fonts in FONTS {
                    for bold in [false, true] {
                        for n in 2..=5 {
                            let size = 8.0 * k;
                            let (t, columns) = close_titles(k, fonts, bold, n, close);
                            let bottom = 600.0 - 10.0 * k * n as f32 - 6.0 * k;
                            let t = rule(look, 40.0, 540.0, 600.0 + 8.0 * k, size)
                                + &t
                                + &rule(look, 40.0, 540.0, bottom, size);
                            let out = common::read(&page_in(&t, look));
                            reads += 1;
                            let name = format!(
                                "{close:?}, {k}x, {look:?}, {fonts:?}, bold {bold}, {n} rows"
                            );
                            for b in columns_broken(&out, &columns) {
                                wrong.push(format!("{name}: {b}"));
                            }
                            let titles: Vec<String> = columns.iter().map(|c| c.0.clone()).collect();
                            let head: Vec<String> =
                                header_cells(&out).into_iter().map(|c| c.0).collect();
                            if head != titles {
                                wrong.push(format!("{name}: header {head:?}"));
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 1440);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Line items of `n` rows of [`ITEMS`] at `k` times 8 pt in the fonts and weight `face` (the
/// titles bold when it says so), between two rules drawn as `frame` when there is one, with
/// the line `note` at x `x` one row under row `after`. Gives the template and the columns.
fn items_with_note(
    k: f32,
    frame: Option<Look>,
    (fonts, bold): ((&str, &str), bool),
    (n, after): (usize, usize),
    (note, x): (&str, f32),
) -> (String, Vec<Column>) {
    let size = 8.0 * k;
    let (vf, tf) = (fonts.0, if bold { fonts.1 } else { fonts.0 });
    let flush_right = |font: &str, x: f32, y: f32, t: &str| {
        show(font, size, x - common::font_width(font, t, size), y, t)
    };
    let mut y = 600.0;
    let mut t = String::new();
    if let Some(look) = frame {
        t += &rule(look, 40.0, 540.0, y + 8.0 * k, size);
    }
    for (x, s) in [(40.0, "Pos"), (80.0, "Artikel"), (300.0, "Menge")] {
        t += &show(tf, size, x, y, s);
    }
    t += &flush_right(tf, 420.0, y, "Preis");
    t += &flush_right(tf, 540.0, y, "Betrag");
    for (r, item) in ITEMS.iter().take(n).enumerate() {
        y -= 10.0 * k;
        t += &show(vf, size, 40.0, y, item[0]);
        t += &show(vf, size, 80.0, y, item[1]);
        t += &show(vf, size, 300.0, y, item[2]);
        t += &flush_right(vf, 420.0, y, item[3]);
        t += &flush_right(vf, 540.0, y, item[4]);
        if r == after {
            y -= 10.0 * k;
            t += &show(vf, size, x, y, note);
        }
    }
    if let Some(look) = frame {
        t += &rule(look, 40.0, 540.0, y - 6.0 * k, size);
    }
    let columns = ["Pos", "Artikel", "Menge", "Preis", "Betrag"]
        .iter()
        .enumerate()
        .map(|(c, title)| {
            (
                (*title).to_string(),
                ITEMS.iter().take(n).map(|r| r[c].to_string()).collect(),
            )
        })
        .collect();
    (t, columns)
}

/// The review's finding K3: a note between two rows of items that runs over a gutter (from
/// x 40 over `Pos` and `Artikel`, from x 80 over `Artikel` and `Menge`, from x 300 over `Menge`
/// and `Preis`), with no frame and in a frame of plain or dashed rules, in every size, font and
/// weight and with 3 to 5 rows: the note joins no row (no cell holds the values of two
/// columns), every title heads the column of its values, and the note's text is read.
#[test]
fn a_note_across_a_gutter_between_rows_joins_no_row() {
    let notes = [
        ("Alle Preise in EUR", 40.0),
        (
            "Lieferung erfolgt in zwei Teilen ab Lager Musterstadt bis Ende des Monats",
            80.0,
        ),
        (
            "Hinweis: Lieferung erfolgt in zwei Teilen ab Lager Musterstadt",
            40.0,
        ),
        (
            "Rabatt 10 % auf alle Positionen dieser Rechnung gewaehrt",
            300.0,
        ),
    ];
    let mut wrong = Vec::new();
    let mut reads = 0;
    for (note, x) in notes {
        for k in [0.75, 1.0, 1.5] {
            for frame in [None, Some(Look::Plain), Some(Look::Dashed)] {
                for face in faces() {
                    for n in 3..=5 {
                        for after in 0..n - 1 {
                            let (t, columns) =
                                items_with_note(k, frame, face, (n, after), (note, x));
                            let out = common::read(&page_in(&t, frame.unwrap_or(Look::Plain)));
                            reads += 1;
                            let mut b = columns_broken(&out, &columns);
                            let head = note.split(' ').next().unwrap_or(note);
                            let in_cell = tables(&out)
                                .iter()
                                .any(|t| t.1.iter().flatten().any(|c| c.contains(head)));
                            if in_cell || !common::all_text(&out).contains(note) {
                                b.push(format!(
                                    "the note is in a cell or lost: {:?}",
                                    tables(&out)
                                ));
                            }
                            let name = format!(
                                "{:.12} at {x}, {k}x, {frame:?}, {face:?}, {n} rows, after {after}",
                                note
                            );
                            wrong.extend(b.into_iter().map(|b| format!("{name}: {b}")));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 1296);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Two columns of small whole numbers for [`narrow_spanned`].
const NARROW_ROWS: [[&str; 2]; 5] = [
    ["12", "34"],
    ["5", "17"],
    ["20", "8"],
    ["3", "11"],
    ["7", "25"],
];

/// Where a title over two columns stands in [`narrow_spanned`].
#[derive(Debug, Clone, Copy)]
enum Place {
    /// Centered over both columns.
    Center,
    /// From the first column's start.
    Left,
    /// Flush right with the second column's end.
    Right,
}

/// Line items of `n` rows at `k` times 8 pt in `face`: `Pos` and `Artikel` at x 40 and 80,
/// `title` over two columns of small whole numbers (each as wide as `34`, the second `gap`
/// sizes after the first, from x 300; the numbers flush right, or flush left when
/// `flush_left`), set as `place` says, and `Betrag` flush right at x 540, with no line under
/// the title. Gives the template and the columns (the title's lines are both numbers of a
/// row).
fn narrow_spanned(
    k: f32,
    (fonts, bold): ((&str, &str), bool),
    n: usize,
    title: &str,
    (place, flush_left, gap): (Place, bool, f32),
) -> (String, Vec<Column>) {
    let size = 8.0 * k;
    let (vf, tf) = (fonts.0, if bold { fonts.1 } else { fonts.0 });
    let w = |font: &str, t: &str| common::font_width(font, t, size);
    let a0 = 300.0;
    let ea = a0 + w(vf, "34");
    let b0 = ea + gap * size;
    let eb = b0 + w(vf, "34");
    let tx = match place {
        Place::Center => (a0 + eb - w(tf, title)) / 2.0,
        Place::Left => a0,
        Place::Right => eb - w(tf, title),
    };
    let mut t = show(tf, size, 40.0, 600.0, "Pos")
        + &show(tf, size, 80.0, 600.0, "Artikel")
        + &show(tf, size, tx, 600.0, title)
        + &show(tf, size, 540.0 - w(tf, "Betrag"), 600.0, "Betrag");
    let rows = &NARROW_ROWS[..n];
    for (r, v) in rows.iter().enumerate() {
        let y = 600.0 - 10.0 * k * (r as f32 + 1.0);
        t += &show(vf, size, 40.0, y, ITEMS[r][0]);
        t += &show(vf, size, 80.0, y, ITEMS[r][1]);
        if flush_left {
            t += &show(vf, size, a0, y, v[0]);
            t += &show(vf, size, b0, y, v[1]);
        } else {
            t += &show(vf, size, ea - w(vf, v[0]), y, v[0]);
            t += &show(vf, size, eb - w(vf, v[1]), y, v[1]);
        }
        t += &show(vf, size, 540.0 - w(vf, ITEMS[r][4]), y, ITEMS[r][4]);
    }
    let columns = vec![
        (
            "Pos".to_string(),
            ITEMS[..n].iter().map(|r| r[0].to_string()).collect(),
        ),
        (
            "Artikel".to_string(),
            ITEMS[..n].iter().map(|r| r[1].to_string()).collect(),
        ),
        (
            title.to_string(),
            rows.iter().map(|v| format!("{} {}", v[0], v[1])).collect(),
        ),
        (
            "Betrag".to_string(),
            ITEMS[..n].iter().map(|r| r[4].to_string()).collect(),
        ),
    ];
    (t, columns)
}

/// The re-review's finding (ruling P6-X2): a title of two capitalized words over two narrow
/// columns of values of one shape (small whole numbers), centered over both or set from the
/// first column's start, with no line under it, is one title: in every size, font and weight,
/// with 2 to 5 rows, values flush right or flush left and the columns 1.4 or 2 sizes apart,
/// its cell keeps its span of two columns and the numbers of each row stand under it.
#[test]
fn capitalized_titles_over_two_columns_of_one_shape_stay_spanned() {
    let titles = [
        "Gesamt Preis",
        "Date Range",
        "Total Amount",
        "Lager Bestand",
        "Unit Price",
    ];
    let mut wrong = Vec::new();
    let mut reads = 0;
    for title in titles {
        for place in [Place::Center, Place::Left] {
            for flush_left in [false, true] {
                for gap in [1.4, 2.0] {
                    for k in [0.75, 1.0, 1.5] {
                        for face in faces() {
                            for n in 2..=5 {
                                let set = (place, flush_left, gap);
                                let (t, columns) = narrow_spanned(k, face, n, title, set);
                                let out = common::read(&page_in(&t, Look::Plain));
                                reads += 1;
                                for b in spanned_broken(&out, &columns) {
                                    wrong.push(format!(
                                        "{title}, {place:?}, flush left {flush_left}, gap {gap}, \
                                         {k}x, {face:?}, {n} rows: {b}"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 1920);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Ruling P6-X2: close titles set as on the real invoice (`Close::Inside`) over two columns
/// of one shape, a quantity and a count, without and in a frame of every look, in every size,
/// font and weight and with 2 to 5 rows, are one title: the header holds `Menge Anzahl` in one
/// cell over both columns, and the two numbers of each row stand under it.
#[test]
fn close_titles_over_two_columns_of_one_shape_stay_merged() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for k in [0.75, 1.0, 1.5] {
        for frame in std::iter::once(None).chain(LOOKS.map(Some)) {
            for (fonts, bold) in faces() {
                for n in 2..=5 {
                    let (mut t, c) = close_titles(k, fonts, bold, n, Close::SameShape);
                    let look = frame.unwrap_or(Look::Plain);
                    if let Some(look) = frame {
                        let size = 8.0 * k;
                        let bottom = 600.0 - 10.0 * k * n as f32 - 6.0 * k;
                        t = rule(look, 40.0, 540.0, 600.0 + 8.0 * k, size)
                            + &t
                            + &rule(look, 40.0, 540.0, bottom, size);
                    }
                    let out = common::read(&page_in(&t, look));
                    reads += 1;
                    let merged: Vec<String> = c[2]
                        .1
                        .iter()
                        .zip(&c[3].1)
                        .map(|(a, b)| format!("{a} {b}"))
                        .collect();
                    let columns = vec![
                        c[0].clone(),
                        c[1].clone(),
                        ("Menge Anzahl".to_string(), merged),
                        c[4].clone(),
                    ];
                    let name = format!("{k}x, {frame:?}, {fonts:?}, bold {bold}, {n} rows");
                    for b in spanned_broken(&out, &columns) {
                        wrong.push(format!("{name}: {b}"));
                    }
                }
            }
        }
    }
    assert_eq!(reads, 336);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Two columns of values of one kind, as the shape of a column may be misread (the
/// re-review's sets of round 4, ruling of fix round 5): a tie of numbers, whole numbers with
/// thousands marks, amounts with the euro sign after and before, negative amounts, whole
/// and decimal numbers, percentages with and without a space, dates, large numbers with
/// thousands marks, empty cells, a mix of text and numbers, and (round 5) amounts and numbers
/// in forms the shapes do not know, against plain amounts or numbers.
const SHAPE_SETS: [(&str, [&str; 4], [&str; 4]); 16] = [
    (
        "tie",
        ["12", "3,50", "12", "3,50"],
        ["3,50", "12", "3,50", "12"],
    ),
    (
        "thousands",
        ["500", "80", "1.200", "20"],
        ["1.250", "2.400", "90", "1.100"],
    ),
    (
        "euro",
        [
            "12,50 \u{20ac}",
            "3,00 \u{20ac}",
            "7,10 \u{20ac}",
            "1,00 \u{20ac}",
        ],
        [
            "\u{20ac} 12,50",
            "\u{20ac} 3,00",
            "\u{20ac} 7,10",
            "\u{20ac} 1,00",
        ],
    ),
    (
        "negatives",
        ["-6,98", "-1,20", "-3,00", "-0,50"],
        ["6,98", "1,20", "3,00", "0,50"],
    ),
    (
        "whole and decimal",
        ["-6", "-1", "-3", "-2"],
        ["-6,98", "-1,20", "-3,00", "-0,50"],
    ),
    (
        "percent",
        ["19 %", "7 %", "19 %", "0 %"],
        ["19%", "7%", "19%", "0%"],
    ),
    (
        "dates",
        ["01.01.2026", "01.02.2026", "01.03.2026", "01.04.2026"],
        ["31.01.2026", "28.02.2026", "31.03.2026", "30.04.2026"],
    ),
    (
        "large",
        ["1.234.567", "2.000.000", "1.500.000", "3.100.200"],
        ["12.345", "67.890", "10.000", "99.999"],
    ),
    ("empty cells", ["12", "", "5", ""], ["", "3,50", "", "1,20"]),
    (
        "mixed",
        ["12", "Std", "3,50", "x"],
        ["Stk", "4", "1,20", "y"],
    ),
    (
        "dash",
        ["12,-", "5,-", "7,50", "9,-"],
        ["3,50", "4,00", "1,20", "2,00"],
    ),
    (
        "trailing minus",
        ["6,98-", "1,20-", "3,00-", "0,50-"],
        ["6,98", "1,20", "3,00", "0,50"],
    ),
    (
        "parentheses",
        ["(6,98)", "(1,20)", "(3,00)", "(0,50)"],
        ["6,98", "1,20", "3,00", "0,50"],
    ),
    (
        "version",
        ["1.2.24", "2.0.25", "1.10.24", "3.1.25"],
        ["12", "3", "5", "1"],
    ),
    (
        "cents",
        ["50 ct", "20 ct", "10 ct", "5 ct"],
        ["0,50", "0,20", "0,10", "0,05"],
    ),
    (
        "hours",
        ["8:00", "7:30", "6:15", "8:00"],
        ["8,00", "7,50", "6,25", "8,00"],
    ),
];

/// Line items of `n` rows at `k` times 8 pt in `face`: `Pos` and `Artikel` at x 40 and 80,
/// `title` over two columns of the values `a` and `b` (each as wide as its widest value, the
/// second `gap` sizes after the first, from x 300; flush right, or flush left when
/// `flush_left`; an empty value leaves its cell empty), set as `place` says, and `Betrag`
/// flush right at x 540, with no line under the title. Gives the template and the columns
/// (the title's lines are the values of a row).
fn shaped_spanned(
    k: f32,
    (fonts, bold): ((&str, &str), bool),
    n: usize,
    title: &str,
    (a, b): (&[&str; 4], &[&str; 4]),
    (place, flush_left, gap): (Place, bool, f32),
) -> (String, Vec<Column>) {
    let size = 8.0 * k;
    let (vf, tf) = (fonts.0, if bold { fonts.1 } else { fonts.0 });
    let w = |font: &str, t: &str| common::font_width(font, t, size);
    let widest = |v: &[&str; 4]| v.iter().map(|v| w(vf, v)).fold(0.0f32, f32::max);
    let a0 = 300.0;
    let ea = a0 + widest(a);
    let b0 = ea + gap * size;
    let eb = b0 + widest(b);
    let tx = match place {
        Place::Center => (a0 + eb - w(tf, title)) / 2.0,
        Place::Left => a0,
        Place::Right => eb - w(tf, title),
    };
    let mut t = show(tf, size, 40.0, 600.0, "Pos")
        + &show(tf, size, 80.0, 600.0, "Artikel")
        + &show(tf, size, tx, 600.0, title)
        + &show(tf, size, 540.0 - w(tf, "Betrag"), 600.0, "Betrag");
    for r in 0..n {
        let y = 600.0 - 10.0 * k * (r as f32 + 1.0);
        t += &show(vf, size, 40.0, y, ITEMS[r][0]);
        t += &show(vf, size, 80.0, y, ITEMS[r][1]);
        for (v, start, end) in [(a[r], a0, ea), (b[r], b0, eb)] {
            if !v.is_empty() {
                let x = if flush_left { start } else { end - w(vf, v) };
                t += &show(vf, size, x, y, v);
            }
        }
        t += &show(vf, size, 540.0 - w(vf, ITEMS[r][4]), y, ITEMS[r][4]);
    }
    let values = (0..n)
        .map(|r| {
            [a[r], b[r]]
                .iter()
                .filter(|v| !v.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let columns = vec![
        (
            "Pos".to_string(),
            ITEMS[..n].iter().map(|r| r[0].to_string()).collect(),
        ),
        (
            "Artikel".to_string(),
            ITEMS[..n].iter().map(|r| r[1].to_string()).collect(),
        ),
        (title.to_string(), values),
        (
            "Betrag".to_string(),
            ITEMS[..n].iter().map(|r| r[4].to_string()).collect(),
        ),
    ];
    (t, columns)
}

/// The re-review's finding of round 4 (ruling of fix round 5): a title of two capitalized
/// words over two columns of values of one kind ([`SHAPE_SETS`]) is never cut, however the
/// shapes of the columns may be misread: centered, from the first column's start or flush
/// right with the second column's end, values flush right or flush left, the columns 1.4 or
/// 2 sizes apart, in every size, font and weight and with 2 to 4 rows, the title stands whole
/// in one header cell and no header cell holds a part of it. (A title narrower than the first
/// column of wide values stands over that column only; that is no cut.)
#[test]
fn titles_over_two_columns_of_one_kind_of_values_stay_spanned() {
    let titles = [
        "Gesamt Preis",
        "Unit Price",
        "Date Range",
        "Total Amount",
        "Preis EUR",
    ];
    let mut wrong = Vec::new();
    let mut reads = 0;
    for title in titles {
        for (name, a, b) in &SHAPE_SETS {
            for place in [Place::Center, Place::Left, Place::Right] {
                for flush_left in [false, true] {
                    for gap in [1.4, 2.0] {
                        for k in [0.75, 1.0, 1.5] {
                            for face in faces() {
                                for n in 2..=4 {
                                    let set = (place, flush_left, gap);
                                    let (t, _) = shaped_spanned(k, face, n, title, (a, b), set);
                                    let out = common::read(&page_in(&t, Look::Plain));
                                    reads += 1;
                                    let head = header_cells(&out);
                                    let words: Vec<&str> = title.split(' ').collect();
                                    let whole = head.iter().any(|c| c.0 == title);
                                    let part = head.iter().any(|c| words.contains(&c.0.as_str()));
                                    if !whole || part {
                                        wrong.push(format!(
                                            "{title}, {name}, {place:?}, flush left \
                                             {flush_left}, gap {gap}, {k}x, {face:?}, {n} \
                                             rows: header {head:?}"
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 34560);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// How the line over the frame of [`line_over_frame`] is set.
#[derive(Debug, Clone, Copy)]
enum Over {
    /// Plain, in the size of the items.
    Plain,
    /// Bold, in the size of the items.
    Bold,
    /// Plain, one and a half times the size of the items.
    Larger,
    /// Plain, in the size of the column titles (`6 k`).
    Small,
}

/// What breaks the reading of a line of two pieces over the top rule of a frame of `n` line
/// items (see [`framed_items`] for `k`, `look`, `fonts` and `bold`): `Rechnung 4711` at x 40
/// and `Datum 01.01.2026` flush right at x 540, `8 k` over the rule, set as `over`, with a
/// lone upright rule far from them when `upright`. The line must be no part of the table
/// header, every title must head the column of its values, and the line must be read.
fn line_over_frame(
    (k, look, fonts, bold): (f32, Look, (&str, &str), bool),
    n: usize,
    over: Over,
    upright: bool,
) -> Vec<String> {
    let (mut t, columns) = framed_items(k, look, fonts, bold, n, None);
    let (font, size) = match over {
        Over::Plain => (fonts.0, 8.0 * k),
        Over::Bold => (fonts.1, 8.0 * k),
        Over::Larger => (fonts.0, 12.0 * k),
        Over::Small => (fonts.0, 6.0 * k),
    };
    // The top rule of the frame is at 600 + 8 k.
    let y = 600.0 + 16.0 * k;
    let date = "Datum 01.01.2026";
    t += &show(font, size, 40.0, y, "Rechnung 4711");
    t += &show(
        font,
        size,
        540.0 - common::font_width(font, date, size),
        y,
        date,
    );
    if upright {
        t += "0.5 w 30 200 m 30 300 l S\n";
    }
    let out = common::read(&page_in(&t, look));
    let mut broken = columns_broken(&out, &columns);
    let in_cell = tables(&out).iter().any(|t| {
        t.1.iter()
            .flatten()
            .any(|c| c.contains("4711") || c.contains("01.01.2026"))
    });
    let all = common::all_text(&out);
    if in_cell || !all.contains("Rechnung 4711") || !all.contains(date) {
        broken.push(format!("the line is in a cell or lost: {:?}", tables(&out)));
    }
    broken
}

/// The carry C-r3-a: a line of two pieces over the top rule of a frame of line items handed
/// to the table pass (see [`line_over_frame`]) is no part of its header, in every size, look,
/// font and weight of the items, every look of the line, with 2 to 5 rows, alone and with an
/// upright rule elsewhere.
#[test]
fn a_line_over_the_top_rule_of_a_frame_of_items_is_no_header() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for k in [0.75, 1.0, 1.5] {
        for look in LOOKS {
            for fonts in FONTS {
                for bold in [false, true] {
                    for n in 2..=5 {
                        for over in [Over::Plain, Over::Bold, Over::Larger, Over::Small] {
                            for upright in [false, true] {
                                let items = (k, look, fonts, bold);
                                reads += 1;
                                wrong.extend(
                                    line_over_frame(items, n, over, upright)
                                        .into_iter()
                                        .map(|b| {
                                            format!(
                                                "{k}x, {look:?}, {fonts:?}, bold {bold}, {n} rows, \
                                             {over:?}, upright {upright}: {b}"
                                            )
                                        }),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 2304);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// What breaks the reading of column titles of two lines whose first lines (`Anzahl` and
/// `Einzel`, in `font` at `size`, their x in `xs`) stand at `y`, over the top rule of a frame
/// of line items: the template and columns of the frame. The first line must be a paragraph
/// right before the table, holding both words, in no cell, and the titles under the rule must
/// head the columns of their values.
fn first_title_line_broken(
    (mut t, columns): (String, Vec<Column>),
    (font, size): (&str, f32),
    xs: (f32, f32),
    y: f32,
    look: Look,
) -> Vec<String> {
    t += &show(font, size, xs.0, y, "Anzahl");
    t += &show(font, size, xs.1, y, "Einzel");
    let out = common::read(&page_in(&t, look));
    let mut b = columns_broken(&out, &columns);
    let texts = texts_of(&out);
    let line = texts
        .iter()
        .position(|s| s.contains("Anzahl") && s.contains("Einzel") && !s.contains('|'));
    let head = texts.iter().position(|s| s.starts_with("Pos | "));
    let before = matches!((line, head), (Some(l), Some(h)) if l + 1 == h);
    let in_cell = tables(&out).iter().any(|t| {
        t.1.iter()
            .flatten()
            .any(|c| c.contains("Anzahl") || c.contains("Einzel"))
    });
    if !before || in_cell {
        b.push(format!("not a paragraph right before the table: {texts:?}"));
    }
    b
}

/// The finding M1 of the Task 12 review: column titles of two lines over line items between
/// rules (see [`framed_items`]) whose first lines, in the titles' size and font, stand `3 k`
/// over the top rule, `Anzahl` flush left at x 300 over `Menge` and `Einzel` flush right at
/// x 420 over `Preis`. The first line stays a paragraph right before the table, and every
/// title under the rule heads the column of its values: every word is read, only the link of
/// the first line to its titles is lost. Joining it would need a bound that tells it from a
/// short line set over the rule, and none does (see
/// [`a_short_line_over_the_top_rule_joins_no_title`]): a wrong header cell is worse than the
/// lost link (ruling P6-X). In every size, look, font and weight, with 2 to 5 rows; also the
/// reviewer's layout, `Einzel` at x 380, at 1x in Helvetica.
#[test]
fn the_first_line_of_two_line_titles_over_the_top_rule_stays_a_paragraph_before_the_table() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for k in [0.75, 1.0, 1.5] {
        for look in LOOKS {
            for fonts in FONTS {
                for bold in [false, true] {
                    for n in 2..=5 {
                        let (font, size) = (if bold { fonts.1 } else { fonts.0 }, 6.0 * k);
                        let w = common::font_width(font, "Einzel", size);
                        // The top rule of the frame is at 600 + 8 k.
                        let b = first_title_line_broken(
                            framed_items(k, look, fonts, bold, n, None),
                            (font, size),
                            (300.0, 420.0 - w),
                            600.0 + 11.0 * k,
                            look,
                        );
                        reads += 1;
                        wrong.extend(b.into_iter().map(|b| {
                            format!("{k}x, {look:?}, {fonts:?}, bold {bold}, {n} rows: {b}")
                        }));
                    }
                }
            }
        }
    }
    assert_eq!(reads, 288);
    for look in [Look::Plain, Look::Bar] {
        for bold in [false, true] {
            let font = if bold { "FB" } else { "FL" };
            let frame = framed_items(1.0, look, FONTS[0], bold, 3, None);
            let b = first_title_line_broken(frame, (font, 6.0), (300.0, 380.0), 611.0, look);
            wrong.extend(
                b.into_iter()
                    .map(|b| format!("x 380, {look:?}, bold {bold}: {b}")),
            );
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Short lines over the top rule of a frame of line items in the review of fix round 1 (I1),
/// each as its left piece and x, and its right piece and the x it ends at (flush right).
const SHORT_OVER: [(&str, f32, &str, f32); 4] = [
    ("Nr.", 40.0, "Datum", 540.0),
    ("Nr. 12", 40.0, "Seite 1", 540.0),
    ("Kunde", 80.0, "Datum", 420.0),
    ("Nr.", 43.0, "Datum", 537.0),
];

/// The review's guard I1 of fix round 1: a short line of two pieces over the top rule of a
/// frame of line items (see [`framed_items`]), in the titles' size and font and lined up with
/// two of them (see [`SHORT_OVER`]), `d k` over the titles' baseline for d from 9.5 to 18: no
/// header cell holds its words, every title heads the column of its values, and the line is
/// read; in every size, look, font and weight.
#[test]
fn a_short_line_over_the_top_rule_joins_no_title() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for (left, lx, right, rx) in SHORT_OVER {
        for d in [9.5, 11.0, 13.0, 15.0, 18.0] {
            for k in [0.75, 1.0, 1.5] {
                for look in LOOKS {
                    for fonts in FONTS {
                        for bold in [false, true] {
                            let (mut t, columns) = framed_items(k, look, fonts, bold, 3, None);
                            let (font, size) = (if bold { fonts.1 } else { fonts.0 }, 6.0 * k);
                            let y = 600.0 + d * k;
                            t += &show(font, size, lx, y, left);
                            let w = common::font_width(font, right, size);
                            t += &show(font, size, rx - w, y, right);
                            let out = common::read(&page_in(&t, look));
                            reads += 1;
                            let mut b = columns_broken(&out, &columns);
                            let head = header_cells(&out);
                            if head
                                .iter()
                                .any(|c| c.0.contains(left) || c.0.contains(right))
                            {
                                b.push(format!("the line joins the header: {head:?}"));
                            }
                            let texts = texts_of(&out);
                            let read = |p: &str| texts.iter().any(|s| s.contains(p));
                            if !read(left) || !read(right) {
                                b.push(format!("the line is lost: {texts:?}"));
                            }
                            wrong.extend(b.into_iter().map(|b| {
                                format!(
                                    "{left}/{right} at {lx}, d {d}, {k}x, {look:?}, {fonts:?}, \
                                     bold {bold}: {b}"
                                )
                            }));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 1440);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// How the title of [`title_beside_fields`] stands out from its field lines.
#[derive(Debug, Clone, Copy)]
enum Stand {
    /// Bold and one and a half times the size of the fields.
    BoldLarger,
    /// Bold, in the size of the fields.
    Bold,
    /// Plain, one and a half times the size of the fields.
    Larger,
}

/// The field lines beside the title of [`title_beside_fields`]: label and value.
const TITLE_FIELDS: [[&str; 2]; 6] = [
    ["Rechnungsnummer", "0000-0001"],
    ["Kundennummer", "00000"],
    ["Datum", "01.02.2026"],
    ["Seite", "1 von 1"],
    ["Bestellnummer", "4711"],
    ["Lieferdatum", "03.02.2026"],
];

/// The title `Rechnung` at x 40 beside `n` field lines of `6 k` pt at x 360 (label, bold when
/// `bold_labels`) and x 360 + 80 k (value), 8 k apart from the baseline 703 down, in the fonts
/// `fonts` (plain, bold), the title standing out as `stand`. Its baseline lies 1.5 k under that
/// of field line `at`, so it shares that line's row of text but not its baseline.
fn title_beside_fields(
    (k, fonts): (f32, (&str, &str)),
    stand: Stand,
    bold_labels: bool,
    n: usize,
    at: usize,
) -> String {
    let fs = 6.0 * k;
    let base = |i: usize| 703.0 - 8.0 * k * i as f32;
    let (font, size) = match stand {
        Stand::BoldLarger => (fonts.1, 1.5 * fs),
        Stand::Bold => (fonts.1, fs),
        Stand::Larger => (fonts.0, 1.5 * fs),
    };
    let mut t = show(font, size, 40.0, base(at) - 1.5 * k, "Rechnung");
    for (i, [label, value]) in TITLE_FIELDS.iter().take(n).enumerate() {
        let label_font = if bold_labels { fonts.1 } else { fonts.0 };
        t += &show(label_font, fs, 360.0, base(i), label);
        t += &show(fonts.0, fs, 360.0 + 80.0 * k, base(i), value);
    }
    t
}

/// The text of each paragraph, heading, list item and table row of the body.
fn texts_of(out: &ReadOutcome) -> Vec<String> {
    let mut found = Vec::new();
    for b in blocks(out) {
        match b {
            Block::Table { rows, .. } => found.extend(rows.iter().map(|r| {
                r.iter()
                    .map(|c| plain_text(&c.inlines))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })),
            _ => {
                let mut s = String::new();
                common::block_text(b, &mut s);
                found.push(s);
            }
        }
    }
    found
}

/// The words of a text of [`texts_of`]: cut at white space and cell bounds.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| c.is_whitespace() || c == '|')
}

/// Finding B2: a larger or bold title beside a block of small field lines, its baseline a
/// little off theirs, stands apart: no block or row holds the title and a field line, the
/// fields are one table of label and value, and no line is lost; in every size, font, look
/// of the title, with 3 to 6 field lines and the title on the first, a middle and the last
/// line's row (ruling P6-Y).
#[test]
fn title_beside_field_list() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for (stand, labels) in [Stand::BoldLarger, Stand::Bold, Stand::Larger]
                .into_iter()
                .flat_map(|s| [(s, false), (s, true)])
            {
                for n in 3..=6 {
                    for at in [0, n / 2, n - 1] {
                        let t = title_beside_fields((k, fonts), stand, labels, n, at);
                        let out = common::read(&page_in(&t, Look::Plain));
                        reads += 1;
                        let name = format!(
                            "{k}x, {fonts:?}, {stand:?}, bold labels {labels}, {n} lines, on {at}"
                        );
                        let fields = &TITLE_FIELDS[..n];
                        let field_text = |s: &str| fields.iter().flatten().any(|f| s.contains(f));
                        let texts = texts_of(&out);
                        for s in &texts {
                            let titled = words(s).any(|w| w == "Rechnung");
                            if titled && field_text(s) {
                                wrong.push(format!("{name}: the title shares {s:?}"));
                            }
                        }
                        let want: Vec<Vec<String>> = fields
                            .iter()
                            .map(|f| f.iter().map(|c| (*c).to_string()).collect())
                            .collect();
                        if !tables(&out).iter().any(|t| t.1 == want) {
                            wrong.push(format!(
                                "{name}: no table of the fields: {:?}",
                                tables(&out)
                            ));
                        }
                        let all = common::all_text(&out);
                        for s in fields.iter().flatten() {
                            if !all.contains(s) {
                                wrong.push(format!("{name}: {s:?} is lost"));
                            }
                        }
                        if !texts.iter().any(|s| words(s).any(|w| w == "Rechnung")) {
                            wrong.push(format!("{name}: the title is lost"));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(reads, 432);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Rows of goods in [`grouped_rows`]: article, quantity and amount.
const GROUP_ROWS: [[&str; 3]; 3] = [
    ["Beratung", "2", "1.000,00"],
    ["Material", "1", "200,00"],
    ["Versand", "1", "34,56"],
];

/// Three rows of [`GROUP_ROWS`] without rules at `k` times 8 pt in the fonts `fonts` (plain,
/// bold), `10 k` apart, the amount flush right; the first column at x 40 holds only
/// `Gruppe A`, standing out as `stand`, on the baseline of the first row. When `titled`, the
/// bold titles `Gruppe`, `Artikel`, `Menge` and `Betrag` stand over the rows, the article at
/// x 40 + 100 k, the quantity at x 300 and the amount at x 540. Else there are no titles, and
/// the article stands at x 200, the quantity at x 300 and the amount at x 380, so that the
/// white space after `Gruppe A` is the widest of the block.
fn grouped_rows(k: f32, fonts: (&str, &str), stand: Stand, titled: bool) -> String {
    let size = 8.0 * k;
    let (font, group) = match stand {
        Stand::BoldLarger => (fonts.1, 1.5 * size),
        Stand::Bold => (fonts.1, size),
        Stand::Larger => (fonts.0, 1.5 * size),
    };
    let (ax, rx) = if titled {
        (40.0 + 100.0 * k, 540.0)
    } else {
        (200.0, 380.0)
    };
    let qx = 300.0;
    let flush_right = |font: &str, y: f32, t: &str| {
        show(font, size, rx - common::font_width(font, t, size), y, t)
    };
    let top = 600.0 + 10.0 * k;
    let mut t = String::new();
    if titled {
        t += &show(fonts.1, size, 40.0, top, "Gruppe");
        t += &show(fonts.1, size, ax, top, "Artikel");
        t += &show(fonts.1, size, qx, top, "Menge");
        t += &flush_right(fonts.1, top, "Betrag");
    }
    t += &show(font, group, 40.0, 600.0, "Gruppe A");
    for (r, row) in GROUP_ROWS.iter().enumerate() {
        let y = 600.0 - 10.0 * k * r as f32;
        t += &show(fonts.0, size, ax, y, row[0]);
        t += &show(fonts.0, size, qx, y, row[1]);
        t += &flush_right(fonts.0, y, row[2]);
    }
    t
}

/// A lone cell on the baseline of a row of a table without rules is no title beside a block,
/// in every size, font and look of the cell. With a title over its column: one table of the
/// titles and three rows, `Gruppe A` in the first row. Without titles (the reader keeps the
/// rows as text): `Gruppe A` is read right before the rest of its row.
#[test]
fn lone_cell_on_a_row_baseline_stays_in_its_row() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for stand in [Stand::BoldLarger, Stand::Bold, Stand::Larger] {
                let name = format!("{k}x, {fonts:?}, {stand:?}");
                let out = common::read(&page_in(&grouped_rows(k, fonts, stand, true), Look::Plain));
                let mut want: Vec<Vec<String>> =
                    vec![owned(&[&["Gruppe", "Artikel", "Menge", "Betrag"]]).remove(0)];
                want.extend(GROUP_ROWS.iter().enumerate().map(|(r, row)| {
                    let first = if r == 0 { "Gruppe A" } else { "" };
                    std::iter::once(first)
                        .chain(row.iter().copied())
                        .map(str::to_string)
                        .collect()
                }));
                let got = tables(&out);
                if got.len() != 1 || got[0].1 != want {
                    wrong.push(format!("{name}, titled: {got:?}"));
                }
                let out =
                    common::read(&page_in(&grouped_rows(k, fonts, stand, false), Look::Plain));
                let texts = texts_of(&out);
                if !texts
                    .iter()
                    .any(|t| t.contains("Gruppe A Beratung 2 1.000,00"))
                {
                    wrong.push(format!("{name}, untitled: {texts:?}"));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Where the group label of [`group_label_rows`] stands, and how it stands out.
#[derive(Debug, Clone, Copy)]
enum Group {
    /// Bold and one and a half times the size of the rows, centred on the three rows.
    BoldLarger,
    /// Plain and one and a half times the size of the rows, centred on the three rows.
    Larger,
    /// Bold in the size of the rows, centred between the first two rows.
    BoldBetween,
}

/// The three rows of [`GROUP_ROWS`] without rules at `k` times 8 pt in the fonts `fonts`
/// (plain, bold), `10 k` apart from the baseline 600 down, the article at x 200, the quantity
/// at x 300 and the amount flush right at x 420, with the bold titles `Artikel`, `Menge` and
/// `Betrag` over them when `titled`; the group label `Gruppe A` at x 40 placed as `group`,
/// so that the white space after it is the widest of the block.
fn group_label_rows(k: f32, fonts: (&str, &str), group: Group, titled: bool) -> String {
    let size = 8.0 * k;
    let row = |r: usize| 600.0 - 10.0 * k * r as f32;
    let flush_right = |font: &str, y: f32, t: &str| {
        show(font, size, 420.0 - common::font_width(font, t, size), y, t)
    };
    let mut t = String::new();
    if titled {
        let top = row(0) + 10.0 * k;
        t += &show(fonts.1, size, 200.0, top, "Artikel");
        t += &show(fonts.1, size, 300.0, top, "Menge");
        t += &flush_right(fonts.1, top, "Betrag");
    }
    // A label of size `l` centred on rows of size `size` around the baseline `mid`: the middle
    // of its letters (0.35 of its size over the baseline) on theirs.
    let centred = |mid: f32, l: f32| mid + 0.35 * (size - l);
    let (font, label, y) = match group {
        Group::BoldLarger => (fonts.1, 1.5 * size, centred(row(1), 1.5 * size)),
        Group::Larger => (fonts.0, 1.5 * size, centred(row(1), 1.5 * size)),
        Group::BoldBetween => (fonts.1, size, (row(0) + row(1)) / 2.0),
    };
    t += &show(font, label, 40.0, y, "Gruppe A");
    for (r, cells) in GROUP_ROWS.iter().enumerate() {
        t += &show(fonts.0, size, 200.0, row(r), cells[0]);
        t += &show(fonts.0, size, 300.0, row(r), cells[1]);
        t += &flush_right(fonts.0, row(r), cells[2]);
    }
    t
}

/// The finding M2 of the Task 12 review: a group label beside three rows of a table without
/// rules, larger or bold and centred on them, is read once and before its rows, never between
/// them or in a cell; the rows are one table of their cells (under their titles when there
/// are titles); in every size, font, look and place of the label, with and without titles.
#[test]
fn a_group_label_beside_rows_is_read_before_them() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for group in [Group::BoldLarger, Group::Larger, Group::BoldBetween] {
                for titled in [false, true] {
                    let name = format!("{k}x, {fonts:?}, {group:?}, titled {titled}");
                    let t = group_label_rows(k, fonts, group, titled);
                    let out = common::read(&page_in(&t, Look::Plain));
                    let texts = texts_of(&out);
                    let with_label: Vec<usize> = (0..texts.len())
                        .filter(|&i| texts[i].contains("Gruppe A"))
                        .collect();
                    let first_row = texts.iter().position(|s| s.contains("Beratung"));
                    let alone = with_label.len() == 1
                        && texts[with_label[0]].trim() == "Gruppe A"
                        && first_row.is_some_and(|r| with_label[0] < r);
                    if !alone {
                        wrong.push(format!("{name}: the label is not alone before: {texts:?}"));
                    }
                    let mut want: Vec<Vec<String>> = Vec::new();
                    if titled {
                        want.push(owned(&[&["Artikel", "Menge", "Betrag"]]).remove(0));
                    }
                    want.extend(
                        GROUP_ROWS
                            .iter()
                            .map(|r| r.iter().map(|c| c.to_string()).collect()),
                    );
                    if !tables(&out).iter().any(|t| t.1 == want) {
                        wrong.push(format!("{name}: no table of the rows: {:?}", tables(&out)));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The blocks of page `n` that are not its body text or its page marker: their role and text.
fn page_texts(out: &ReadOutcome, n: u32) -> Vec<(Option<String>, String)> {
    let want = rezzer_model::Anchor(format!("p{n}"));
    let page = out
        .document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section {
                kind: rezzer_model::SectionKind::Page,
                anchor: Some(a),
                blocks,
                ..
            } if *a == want => Some(blocks.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no page {n}"));
    let mut found = Vec::new();
    for b in page {
        let role = match b {
            Block::Paragraph { role, .. } => role.clone(),
            _ => None,
        };
        if role.as_deref() == Some("page") {
            continue;
        }
        let text = match b {
            Block::Table { rows, .. } => rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| plain_text(&c.inlines))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .collect::<Vec<_>>()
                .join(" / "),
            _ => {
                let mut s = String::new();
                common::block_text(b, &mut s);
                s.trim().to_string()
            }
        };
        if !text.starts_with("die Muster") {
            found.push((role, text));
        }
    }
    found
}

/// The three columns of the finding B5 at x 40, 230 and 420 on the baselines 60, 52 and 44.
const BLANK_COLUMNS: [(f32, [&str; 3]); 3] = [
    (40.0, ["Muster GmbH", "Musterweg 1", "12345 Musterstadt"]),
    (
        230.0,
        [
            "Musterbank",
            "IBAN DE00 0000 0000 0000 0000 00",
            "BIC MUSTDEXXXXX",
        ],
    ),
    (
        420.0,
        [
            "Amtsgericht Musterstadt",
            "HRB 00000",
            "Gesch\u{e4}ftsf\u{fc}hrung Erika Mustermann",
        ],
    ),
];

/// Finding B5: three columns of an address, a bank and a register entry in a 6 pt footer, each
/// row drawn over a string of 150 spaces from x 40 that reaches past the first gutter (as a
/// print run blanks its field first): each column is read as one paragraph of three lines, the
/// columns one after the other. The footer stands on both pages of a file of two, so it is a
/// running footer: by ruling P6-X5 address columns part only on furniture lines (on a file of
/// one page the footer is body text and reads as at b9c878e).
#[test]
fn blank_strings_do_not_block_columns() {
    for font in ["FL", "FT"] {
        let mut t = String::new();
        for (k, y) in [60.0, 52.0, 44.0].into_iter().enumerate() {
            t += &show(font, 6.0, 40.0, y, &" ".repeat(150));
            for (x, lines) in BLANK_COLUMNS {
                t += &show(font, 6.0, x, y, lines[k]);
            }
        }
        for k in [0.75, 1.0, 1.5] {
            for wide in [false, true] {
                blank_columns_read(&footed_pages(&t, 2, k, wide));
            }
        }
    }
}

/// The checks of [`blank_strings_do_not_block_columns`] on the file `bytes`.
fn blank_columns_read(bytes: &[u8]) {
    let out = common::read(bytes);
    let texts = page_texts(&out, 1);
    let joined = texts
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let order: Vec<&str> = BLANK_COLUMNS.iter().flat_map(|c| c.1).collect();
    common::in_order(&joined, &order).unwrap_or_else(|e| panic!("{e}"));
    for (_, lines) in BLANK_COLUMNS {
        let found = paragraph(&out, lines[0]).unwrap_or_else(|| panic!("{lines:?} in {texts:?}"));
        assert_eq!(plain_text(found), lines.join(" "), "{texts:?}");
        assert_eq!(line_breaks(found), 2, "{found:?}");
    }
}

/// Finding B5: the bank and the register column side by side on shared baselines, without
/// colons, are two addresses and no table of labels and values: no table, each line read once.
#[test]
fn two_address_columns_are_not_key_values() {
    let mut t = String::new();
    for (k, y) in [60.0, 52.0, 44.0].into_iter().enumerate() {
        t += &show("FL", 6.0, 40.0, y, BLANK_COLUMNS[1].1[k]);
        t += &show("FL", 6.0, 300.0, y, BLANK_COLUMNS[2].1[k]);
    }
    for wide in [false, true] {
        let out = common::read(&footed_pages(&t, 1, 1.0, wide));
        assert!(tables(&out).is_empty(), "wide {wide}: {:?}", tables(&out));
        let texts = page_texts(&out, 1);
        for line in BLANK_COLUMNS[1].1.iter().chain(&BLANK_COLUMNS[2].1) {
            let n: usize = texts.iter().map(|(_, t)| t.matches(line).count()).sum();
            assert_eq!(n, 1, "wide {wide}: {line:?} in {texts:?}");
        }
    }
}

/// Guard: labels without colons beside their values still read as a table of label and value.
#[test]
fn labels_without_colons_still_pair() {
    let fields = [
        ["Rechnungsnummer", "0000-0001"],
        ["Kundennummer", "00000"],
        ["Datum", "01.02.2026"],
    ];
    let mut t = String::new();
    for (k, [label, value]) in fields.iter().enumerate() {
        let y = 700.0 - 10.0 * k as f32;
        t += &show("FL", 8.0, 40.0, y, label);
        t += &show("FL", 8.0, 160.0, y, value);
    }
    let out = common::read(&page_in(&t, Look::Plain));
    let want: Vec<&[&str]> = fields.iter().map(|f| f.as_slice()).collect();
    assert_eq!(tables(&out), [(0, owned(&want))]);
}

/// Guard: the totals under line items, labels without colons beside amounts, stay a table.
#[test]
fn totals_still_pair() {
    let totals = owned(&[
        &["Nettobetrag", "1.234,56"],
        &["USt 19 %", "234,57"],
        &["Gesamtbetrag", "1.469,13"],
    ]);
    for (name, out) in invoice_reads(Rules::End) {
        let found = tables(&out);
        assert_eq!(found.len(), 2, "{name}: {found:?}");
        assert_eq!(found[1].1, totals, "{name}");
    }
}

/// The blocks of an invented invoice footer, each a column: the firm's address, its bank, its
/// register entry and its shop's contact.
const FOOTER: [&[&str]; 4] = [
    &[
        "Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt",
        "Deutschland",
    ],
    &[
        "Musterbank",
        "IBAN DE00 0000 0000 0000 0000 00",
        "BIC MUSTDEXXXXX",
    ],
    &[
        "Amtsgericht Musterstadt",
        "HRB 00000",
        "USt-IdNr. DE000000000",
        "Gesch\u{e4}ftsf\u{fc}hrung",
        "Erika Mustermann",
        "Max Mustermann",
    ],
    &[
        "Muster Shop",
        "Tel. 0000 000000",
        "E-Mail info@muster.example",
        "Web www.muster.example",
    ],
];

/// How the columns of a generated footer stand.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stack {
    /// Every column starts on the first baseline.
    Top,
    /// A shorter column is centred on the height of the longest (half a line off when the
    /// counts differ by an odd number).
    Centred,
}

/// A footer of the `columns` (their lines) at 7 pt in `fonts` (plain, bold), the line `bold`
/// (column, line) set bold, the first baseline at 70 and 8.4 points apart, each column three
/// sizes right of the widest line of the one before it.
fn footer(
    fonts: (&str, &str),
    columns: &[Vec<&str>],
    bold: Option<(usize, usize)>,
    stack: Stack,
) -> String {
    columns_at(fonts, columns, bold, stack, 70.0)
}

/// The columns of [`footer`] with the first baseline at `top`.
fn columns_at(
    fonts: (&str, &str),
    columns: &[Vec<&str>],
    bold: Option<(usize, usize)>,
    stack: Stack,
    top: f32,
) -> String {
    let size = 7.0;
    let rows = columns.iter().map(Vec::len).max().unwrap_or(0);
    let mut t = String::new();
    let mut x = 40.0;
    for (c, lines) in columns.iter().enumerate() {
        let off = match stack {
            Stack::Top => 0.0,
            Stack::Centred => (rows - lines.len()) as f32 / 2.0,
        };
        let mut widest: f32 = 0.0;
        for (k, line) in lines.iter().enumerate() {
            let font = if bold == Some((c, k)) {
                fonts.1
            } else {
                fonts.0
            };
            t += &show(font, size, x, top - 8.4 * (k as f32 + off), line);
            widest = widest.max(common::font_width(font, line, size));
        }
        x += widest + 3.0 * size;
    }
    t
}

/// What is wrong with the footer `columns` on page `n` of `out`: a block that holds lines of
/// two columns, a line not read or read twice, a column that is not one block or not in
/// column order, or a column whose role is not `role`.
fn footer_broken(
    out: &ReadOutcome,
    n: u32,
    columns: &[Vec<&str>],
    role: Option<&str>,
) -> Vec<String> {
    let texts = page_texts(out, n);
    let mut wrong = Vec::new();
    // The columns whose lines each block holds.
    let holds = |s: &str| -> Vec<usize> {
        (0..columns.len())
            .filter(|&c| columns[c].iter().any(|l| s.contains(l)))
            .collect()
    };
    for (_, s) in &texts {
        if holds(s).len() > 1 {
            wrong.push(format!("page {n}: {s:?} holds lines of two columns"));
        }
    }
    for line in columns.iter().flatten() {
        let k: usize = texts.iter().map(|(_, s)| s.matches(line).count()).sum();
        if k != 1 {
            wrong.push(format!("page {n}: {line:?} is read {k} times"));
        }
    }
    let mut last = None;
    for (c, lines) in columns.iter().enumerate() {
        let at: Vec<usize> = (0..texts.len())
            .filter(|&b| lines.iter().any(|l| texts[b].1.contains(l)))
            .collect();
        match at.as_slice() {
            [b] => {
                if texts[*b].1 != lines.join(" ") {
                    wrong.push(format!("page {n}: column {c} reads {:?}", texts[*b].1));
                }
                if texts[*b].0.as_deref() != role {
                    wrong.push(format!(
                        "page {n}: column {c} has the role {:?}",
                        texts[*b].0
                    ));
                }
                if last.is_some_and(|l| l >= *b) {
                    wrong.push(format!(
                        "page {n}: column {c} is read before the one before it"
                    ));
                }
                last = Some(*b);
            }
            _ => wrong.push(format!("page {n}: column {c} is in blocks {at:?}")),
        }
    }
    if !wrong.is_empty() {
        wrong.push(format!("page {n}: {texts:?}"));
    }
    wrong
}

/// A line of body text that runs over the width of the page, as the lines of an item table or
/// a letter do: no gutter of the footer runs through it.
const WIDE_LINE: &str = "die Muster GmbH liefert die bestellte Ware gem\u{e4}\u{df} den Bedingungen und \
                         berechnet die Leistungen wie vereinbart, zahlbar ohne Abzug bis zum Ende des Monats";

/// Lines of plain `size` pt body text from `top` down, at least 600 ink characters, each
/// [`WIDE_LINE`] when `wide`, else [`BODY_LINE`].
fn body_of(size: f32, top: f32, wide: bool) -> String {
    if !wide {
        return body(size, top);
    }
    let mut out = String::new();
    let mut ink = 0;
    let mut y = top;
    while ink < 600 {
        out.push_str(&show("FL", size, 40.0, y, WIDE_LINE));
        ink += WIDE_LINE.chars().filter(|c| !c.is_whitespace()).count();
        y -= size * 1.3;
    }
    out
}

/// What is wrong with the footer `columns` on page `n` of `out` where it is body text: a line
/// not read or read twice, or a line in a running header or footer.
fn footer_lines_kept(out: &ReadOutcome, n: u32, columns: &[Vec<&str>]) -> Vec<String> {
    let texts = page_texts(out, n);
    let mut wrong = Vec::new();
    for line in columns.iter().flatten() {
        let holders: Vec<&(Option<String>, String)> =
            texts.iter().filter(|(_, t)| t.contains(line)).collect();
        let k: usize = holders.iter().map(|(_, t)| t.matches(line).count()).sum();
        if k != 1 || holders.iter().any(|(r, _)| r.is_some()) {
            wrong.push(format!("page {n}: {line:?} in {holders:?}"));
        }
    }
    wrong
}

/// A file of `count` pages, each the 7 pt body (wide or not) with `extra` under it, drawn `k`
/// times as large.
fn footed_pages(extra: &str, count: usize, k: f32, wide: bool) -> Vec<u8> {
    let page = FormPage {
        template: format!("{}{extra}", body_of(7.0, 500.0, wide)),
        scale: k,
        ..FormPage::default()
    };
    print_form(&vec![page; count], "", "")
}

/// Carried from Task 10 (an invoice of two pages, its shape only): a footer of 7 pt text in
/// four columns of uneven length, the address, the bank, the register entry and the contact
/// under a bold shop name, at the foot of both pages. Each column is one block, in column
/// order, a running footer on both pages, and the file has no loss.
#[test]
fn invoice_footer_columns_are_furniture_on_two_pages() {
    let columns: Vec<Vec<&str>> = FOOTER.iter().map(|c| c.to_vec()).collect();
    let t = footer(("FL", "FB"), &columns, Some((3, 0)), Stack::Top);
    let mut wrong = Vec::new();
    for wide in [false, true] {
        let out = common::read(&footed_pages(&t, 2, 1.0, wide));
        for n in 1..=2 {
            wrong.extend(footer_broken(&out, n, &columns, Some("page-footer")));
        }
        if !out.document.losses.is_empty() {
            wrong.push(format!("wide {wide}: {:?}", out.document.losses));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The column sets of the generated footers: two to four blocks of [`FOOTER`] next to each
/// other, with their uneven line counts, and each cut to three lines.
fn footer_sets() -> Vec<(String, Vec<Vec<&'static str>>)> {
    let mut out = Vec::new();
    for n in 2..=4 {
        for start in 0..=FOOTER.len() - n {
            let blocks = &FOOTER[start..start + n];
            out.push((
                format!("{n} columns from {start}"),
                blocks.iter().map(|b| b.to_vec()).collect(),
            ));
            out.push((
                format!("{n} columns from {start}, three lines"),
                blocks.iter().map(|b| b[..3].to_vec()).collect(),
            ));
        }
    }
    out
}

/// Finding B5 under rulings P6-Y and P6-X5: a footer of two to four address-shaped columns, in
/// every size and font, set flush to the top or centred, with no bold line, a bold first line
/// of its last column or a bold second line of its first, on one, two or three pages. On files
/// of two and three pages, where it is a running footer: no block holds lines of two columns,
/// every line is read once, each column is one block in column order, and no loss. On a file
/// of one it is body text and reads as at b9c878e (address columns part only on furniture
/// lines): every line is read once, as body text.
#[test]
fn footer_columns_read_apart_in_every_size_and_font() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for (set, columns) in footer_sets() {
        let n = columns.len();
        for k in [0.75, 1.0, 1.5] {
            for fonts in FONTS {
                for stack in [Stack::Top, Stack::Centred] {
                    for bold in [None, Some((n - 1, 0)), Some((0, 1))] {
                        let t = footer(fonts, &columns, bold, stack);
                        for (count, wide) in (1..=3usize).flat_map(|c| [(c, false), (c, true)]) {
                            let out = common::read(&footed_pages(&t, count, k, wide));
                            reads += 1;
                            let name = format!(
                                "{set}, {k}x, {fonts:?}, {stack:?}, bold {bold:?}, {count} pages, \
                                 wide {wide}"
                            );
                            if count == 1 {
                                for w in footer_lines_kept(&out, 1, &columns) {
                                    wrong.push(format!("{name}: {w}"));
                                }
                                continue;
                            }
                            for p in 1..=count as u32 {
                                for w in footer_broken(&out, p, &columns, Some("page-footer")) {
                                    wrong.push(format!("{name}: {w}"));
                                }
                            }
                            if !out.document.losses.is_empty() {
                                wrong.push(format!("{name}: {:?}", out.document.losses));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong in {reads} reads:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The sender block, the bold title under it and the address beside them of the carried
/// finding C-r4-b, at `size` pt (8 in the finding) and its distances in that ratio: the
/// sender `Muster GmbH` and `Musterweg 2` at x 40 from the baseline 722 down, 10 apart, the
/// title `Rechnung` bold in `title` pt 12 under them, and the address of four lines at x 360
/// from 1.5 under the title's baseline, 10 apart.
fn sender_title_address(fonts: (&str, &str), size: f32, title: f32) -> String {
    let k = size / 8.0;
    let mut t = show(fonts.0, size, 40.0, 722.0, "Muster GmbH")
        + &show(fonts.0, size, 40.0, 722.0 - 10.0 * k, "Musterweg 2")
        + &show(fonts.1, title, 40.0, 722.0 - 22.0 * k, "Rechnung");
    for (n, line) in ADDRESS.iter().enumerate() {
        let y = 722.0 - 23.5 * k - 10.0 * k * n as f32;
        t += &show(fonts.0, size, 360.0, y, line);
    }
    t
}

/// The address of [`sender_title_address`].
const ADDRESS: [&str; 4] = [
    "Erika Mustermann",
    "Musterstrasse 1",
    "12345 Musterstadt",
    "Deutschland",
];

/// Carried finding C-r4-b: a bold title under the sender lines and beside the address of a
/// letter stays apart from both, in every page size, font, text size and title size, also in
/// the size of the sender: no block holds the title and a line of the sender or the address,
/// the sender lines are one block and the address another, each line read once.
#[test]
fn a_bold_title_under_the_sender_stays_apart() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for (size, ratio) in [6.0, 8.0, 10.0]
                .into_iter()
                .flat_map(|s| [0.875, 1.0, 1.125, 1.25, 1.5].map(|r| (s, r)))
            {
                let title = size * ratio;
                let name = format!("{k}x, {fonts:?}, {size} pt, title {title} pt");
                let t = sender_title_address(fonts, size, title);
                let out = common::read(&footed_pages(&t, 1, k, false));
                let texts = page_texts(&out, 1);
                let sender = ["Muster GmbH", "Musterweg 2"];
                for (_, s) in &texts {
                    let titled = words(s).any(|w| w == "Rechnung");
                    let other = sender.iter().chain(&ADDRESS).any(|l| s.contains(l));
                    if titled && other {
                        wrong.push(format!("{name}: the title shares {s:?}"));
                    }
                }
                for block in [&sender[..], &ADDRESS[..]] {
                    if !texts.iter().any(|(_, s)| *s == block.join(" ")) {
                        wrong.push(format!("{name}: {block:?} is not one block: {texts:?}"));
                    }
                }
                for line in sender.iter().chain(&ADDRESS).chain(&["Rechnung"]) {
                    let n: usize = texts.iter().map(|(_, s)| s.matches(line).count()).sum();
                    if n != 1 {
                        wrong.push(format!("{name}: {line:?} read {n} times: {texts:?}"));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Finding B5: an address block with one line wider than 24 times its size (a long name of
/// the managing director) keeps a line break after each line: the width of a block of short
/// lines in the shape of an address does not make it prose.
#[test]
fn a_wide_address_line_keeps_its_line_breaks() {
    let lines = [
        "Muster GmbH",
        "Vertretungsberechtigte Gesch\u{e4}ftsf\u{fc}hrerin Erika Mustermann-Musterfrau",
        "Musterweg 1",
        "12345 Musterstadt",
    ];
    for font in ["FL", "FT"] {
        let mut t = String::new();
        for (k, line) in lines.iter().enumerate() {
            t += &show(font, 8.0, 40.0, 700.0 - 10.0 * k as f32, line);
        }
        for k in [0.75, 1.0, 1.5] {
            let out = common::read(&footed_pages(&t, 1, k, false));
            let found = paragraph(&out, "Muster GmbH Vertretungs")
                .unwrap_or_else(|| panic!("{font}, {k}x: no address paragraph"));
            assert_eq!(plain_text(found), lines.join(" "), "{font}, {k}x");
            assert_eq!(line_breaks(found), 3, "{font}, {k}x: {found:?}");
        }
    }
}

/// Guards of finding B5 under rulings P6-Y and P6-X5: the footer columns of
/// [`footer_columns_read_apart_in_every_size_and_font`] on pages 1 and 2 of a file of three
/// pages, and the same columns at the top of both pages of a file of two, are content: no
/// running footer or header, every line read once as body text, in every size, font and column
/// set. Being no furniture, they read as at b9c878e.
#[test]
fn footer_columns_on_too_few_pages_or_at_the_top_stay_content() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for (set, columns) in footer_sets() {
        for k in [0.75, 1.0, 1.5] {
            for fonts in FONTS {
                for stack in [Stack::Top, Stack::Centred] {
                    for wide in [false, true] {
                        let name = format!("{set}, {k}x, {fonts:?}, {stack:?}, wide {wide}");
                        let page = |extra: String| FormPage {
                            template: format!("{}{extra}", body_of(7.0, 500.0, wide)),
                            scale: k,
                            ..FormPage::default()
                        };
                        let foot = footer(fonts, &columns, None, stack);
                        let two_of_three = [page(foot.clone()), page(foot), page(String::new())];
                        let out = common::read(&print_form(&two_of_three, "", ""));
                        reads += 1;
                        for n in 1..=2 {
                            for w in footer_lines_kept(&out, n, &columns) {
                                wrong.push(format!("{name}, on 2 of 3 pages: {w}"));
                            }
                        }
                        let head = columns_at(fonts, &columns, None, stack, 790.0);
                        let out =
                            common::read(&print_form(&[page(head.clone()), page(head)], "", ""));
                        reads += 1;
                        for n in 1..=2 {
                            for w in footer_lines_kept(&out, n, &columns) {
                                wrong.push(format!("{name}, at the top: {w}"));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong in {reads} reads:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The labels and values of a block of a notice: (label, value), an empty value for none.
const NOTICE_FIELDS: [(&str, &str); 12] = [
    ("Steuernummer", "000/000/00000"),
    ("Aktenzeichen", "00 000 00000"),
    ("Veranlagungsjahr", "2025"),
    ("Bescheid vom", "01.02.2026"),
    ("Zu versteuerndes Einkommen", "12.345"),
    ("Festgesetzte Steuer", "1.234,56"),
    ("Solidarit\u{e4}tszuschlag", ""),
    ("Kirchensteuer", "123,45"),
    ("Anzurechnende Steuer", "1.000,00"),
    ("Verbleibender Betrag", ""),
    ("Bereits gezahlt", "234,56"),
    ("Noch zu zahlen", "123,45"),
];

/// Ruling P6-X3: a block of `n` label and value lines of a notice, the labels plain at x 40
/// and the values bold flush right at x 400 on the same baselines, some values empty, in every
/// size and font: each value is read after its own label and before the next label, never as
/// a column of its own after a column of labels.
#[test]
fn labels_beside_bold_values_with_empty_values_stay_pairs() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for n in [6, 9, 12] {
                let mut t = String::new();
                for (r, (label, value)) in NOTICE_FIELDS.iter().take(n).enumerate() {
                    let y = 700.0 - 11.0 * r as f32;
                    t += &show(fonts.0, 8.0, 40.0, y, label);
                    if !value.is_empty() {
                        let x = 400.0 - common::font_width(fonts.1, value, 8.0);
                        t += &show(fonts.1, 8.0, x, y, value);
                    }
                }
                let out = common::read(&footed_pages(&t, 1, k, false));
                let all = texts_of(&out).join("\n");
                let fields = &NOTICE_FIELDS[..n];
                for (r, (label, value)) in fields.iter().enumerate() {
                    if value.is_empty() {
                        continue;
                    }
                    let next = fields.get(r + 1).map(|f| f.0);
                    let order: Vec<&str> = [*label, *value].into_iter().chain(next).collect();
                    if let Err(e) = common::in_order(&all, &order) {
                        wrong.push(format!("{k}x, {fonts:?}, {n} rows: {e}"));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The columns of an invoice footer whose contact lines start with a web and a mail address in
/// small letters, as real footers set them.
fn contact_columns() -> Vec<Vec<&'static str>> {
    vec![
        FOOTER[0][..3].to_vec(),
        FOOTER[1].to_vec(),
        FOOTER[2].to_vec(),
        vec![
            "Muster Shop",
            "Tel. 0000 000000",
            "info@muster.example",
            "www.muster.example",
        ],
    ]
}

/// Fix round 2, item 1: the columns of a footer of two pages, a running footer on both, keep a
/// line break between each two of their lines, also where lines start with a web or mail
/// address in small letters, in every size and font: a column of a footer is lines, not prose.
#[test]
fn footer_columns_keep_a_line_break_between_their_lines() {
    let mut wrong = Vec::new();
    let columns = contact_columns();
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for wide in [false, true] {
                let t = footer(fonts, &columns, Some((3, 0)), Stack::Top);
                let out = common::read(&footed_pages(&t, 2, k, wide));
                for b in blocks(&out) {
                    let Block::Paragraph { inlines, role, .. } = b else {
                        continue;
                    };
                    let text = plain_text(inlines);
                    let Some(c) = columns.iter().find(|c| text.starts_with(c[0])) else {
                        continue;
                    };
                    if role.as_deref() != Some("page-footer") || line_breaks(inlines) != c.len() - 1
                    {
                        wrong.push(format!(
                            "{k}x, {fonts:?}, wide {wide}: {role:?} {} breaks in {text:?}",
                            line_breaks(inlines)
                        ));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// A page of an invented invoice of the shape of fix round 2, item 3: the sender line, the bold
/// recipient address, the bold title beside a field list, the body and the footer of
/// [`contact_columns`] with a bold shop name. The sender and the recipient hold none of the
/// footer's lines.
fn invoice_page(fonts: (&str, &str)) -> String {
    let mut t = show(
        fonts.0,
        7.0,
        40.0,
        770.0,
        "Beispiel AG, Beispielweg 2, 54321 Beispielstadt",
    );
    for (k, l) in ["Max Beispiel", "Beispielstrasse 3", "54321 Beispielstadt"]
        .iter()
        .enumerate()
    {
        t += &show(fonts.1, 9.0, 40.0, 750.0 - 11.0 * k as f32, l);
    }
    t += &show(fonts.1, 14.0, 40.0, 690.0, "Rechnung");
    for (k, [label, value]) in TITLE_FIELDS.iter().take(4).enumerate() {
        let y = 703.0 - 8.0 * k as f32;
        t += &show(fonts.0, 6.0, 360.0, y, label);
        t += &show(fonts.0, 6.0, 440.0, y, value);
    }
    t += &body_of(7.0, 600.0, true);
    t + &footer(fonts, &contact_columns(), Some((3, 0)), Stack::Top)
}

/// Fix round 2, item 3: on both pages of an invoice of two pages, every line of the footer
/// columns is a running footer, each column one block, in every size and font; the title and
/// the field list stay content.
#[test]
fn a_two_page_invoice_footer_is_furniture_on_both_pages() {
    let mut wrong = Vec::new();
    let columns = contact_columns();
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            let pages: Vec<FormPage> = (1..=2)
                .map(|_| FormPage {
                    template: invoice_page(fonts),
                    scale: k,
                    ..FormPage::default()
                })
                .collect();
            let out = common::read(&print_form(&pages, "", ""));
            for n in 1..=2 {
                for w in footer_broken(&out, n, &columns, Some("page-footer")) {
                    wrong.push(format!("{k}x, {fonts:?}: {w}"));
                }
                let texts = page_texts(&out, n);
                if texts.iter().any(|(r, t)| {
                    r.is_some() && (t.contains("Rechnung") || t.contains("Kundennummer"))
                }) {
                    wrong.push(format!("{k}x, {fonts:?}: page {n} letterhead {texts:?}"));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// A measured line: x, baseline, size, bold and its invented text.
type Measured = (f32, f32, f32, bool, &'static str);

/// The footer columns of a reduced model of a real invoice footer shape, with invented text and
/// rounded, moved and varied places: (x, lines), each line its baseline, size, bold and text. The second column ends in two lines of white space only, a little left of
/// its text (a print run blanks them), and the fourth is a bold 11 pt shop name over a bold
/// web address.
fn measured_footer() -> Vec<Vec<Measured>> {
    let rows = [69.0, 61.0, 52.0, 44.0, 35.0, 27.0];
    let column = |x: f32, lines: &[&'static str]| -> Vec<Measured> {
        lines
            .iter()
            .zip(rows)
            .map(|(t, y)| (x, y, 7.0, false, *t))
            .collect()
    };
    vec![
        column(
            129.0,
            &[
                "Muster Shop GmbH",
                "Inhaberin Erika Mustermann",
                "Musterweg 12a",
                "Telefon 000 000 00 00",
                "E-Mail info@muster.example",
                "www.muster.example",
            ],
        ),
        column(
            236.0,
            &[
                "Bankverbindung",
                "Musterbank Nord",
                "IBAN DE00 0000 0000 0000 0000 00",
                "SWIFT-BIC MUSTDEXX000",
            ],
        ),
        column(
            367.0,
            &[
                "Gesch\u{e4}ftsf\u{fc}hrer Max",
                "Mustermann, Erika Musterfrau",
                "Sitz Musterstadt,",
                "Register: Amtsgericht Musterstadt",
                "HRB 00000 USt-IdNr. DE000000000",
                "Steuernummer 000/000/000",
            ],
        ),
        vec![
            (492.0, 53.0, 11.0, true, "Shop Muster"),
            (492.0, 42.0, 7.0, true, "shop.muster.example"),
        ],
    ]
}

/// Page `n` of the reduced invoice model of fix round 3: a letter with a sender line, a
/// recipient, a bold title beside a field list, line items and closing lines on page 1, one
/// line on page 2, the page number `Seite n von 2` over the footer, and the footer of
/// [`measured_footer`] with its two blank lines.
fn measured_invoice_page(n: u32) -> String {
    let mut t = String::new();
    if n == 1 {
        t += &show(
            "FL",
            6.0,
            77.0,
            713.0,
            "Beispiel Handel KG - Beispielweg 4b - 54321 Beispielort - Telefon 111 111 11 11 - post@beispiel.example",
        );
        for (y, l) in [
            (692.0, "Herrn Max B."),
            (678.0, "Beispielweg 3a"),
            (664.0, "54321 Beispielort"),
        ] {
            t += &show("FL", 9.0, 77.0, y, l);
        }
        t += &show("FB", 9.0, 77.0, 578.0, "Rechnungsnr. 0000001");
        for (k, (label, value)) in [
            ("Datum", "01.02.2026"),
            ("Kundennummer", "000000000"),
            ("Bestellnummer", "0000000"),
            ("Bestelldatum", "31.01.2026"),
            ("Lieferdatum", "01.02.2026"),
        ]
        .iter()
        .enumerate()
        {
            let y = 610.0 - 11.0 * k as f32;
            t += &show("FL", 6.0, 366.0, y, label);
            t += &show("FL", 6.0, 443.0, y, value);
        }
        t += &show(
            "FL",
            9.0,
            77.0,
            529.0,
            "Sehr geehrter Herr Beispiel, vielen Dank f\u{fc}r Ihre Bestellung, die wir wie folgt berechnen.",
        );
        t += &show(
            "FL",
            9.0,
            77.0,
            518.0,
            "Bitte zahlen Sie den Betrag ohne Abzug bis zum genannten Datum.",
        );
        // The line items: a bold header of six titles, three items of seven cells (two with
        // a wrapped item text under them) and the totals of label and amount.
        let header = [
            (80.0, "Pos."),
            (103.0, "Artikel"),
            (162.0, "Bezeichnung"),
            (383.0, "Menge"),
            (443.0, "Einzel"),
            (503.0, "Gesamt EUR"),
        ];
        for (x, text) in header {
            t += &show("FB", 8.0, x, 457.0, text);
        }
        for (y, cells) in [
            (
                439.0,
                [
                    "1",
                    "00001",
                    "Musterartikel mit Zubeh\u{f6}r",
                    "1",
                    "St\u{fc}ck",
                    "100,00",
                    "100,00",
                ],
            ),
            (
                400.0,
                [
                    "2",
                    "00002",
                    "Beispielartikel",
                    "2",
                    "St\u{fc}ck",
                    "50,00",
                    "100,00",
                ],
            ),
            (
                371.0,
                ["3", "00003", "Versand", "1", "Pauschal", "4,90", "4,90"],
            ),
        ] {
            for (x, c) in [87.0, 103.0, 162.0, 383.0, 403.0, 453.0, 513.0]
                .iter()
                .zip(cells)
            {
                t += &show("FL", 7.0, *x, y, c);
            }
        }
        for (y, text) in [
            (430.0, "Farbe Blau, Gr\u{f6}\u{df}e M"),
            (421.0, "mit Tasche und Anleitung, Modell 2026"),
            (392.0, "Farbe Rot"),
        ] {
            t += &show("FL", 7.0, 162.0, y, text);
        }
        for (y, size, bold, label, value) in [
            (345.0, 9.0, false, "Summe netto", "204,90 EUR"),
            (334.0, 7.0, false, "zzgl. 19 % USt", "38,93 EUR"),
            (325.0, 7.0, false, "zzgl. Versand netto", "0,00 EUR"),
            (308.0, 9.0, true, "Gesamtbetrag", "243,83 EUR"),
        ] {
            let font = if bold { "FB" } else { "FL" };
            t += &show(font, size, 349.0, y, label);
            t += &right(bold, size, 542.0, y, value);
        }
        for (k, y) in [253.0, 243.0, 222.0, 212.0, 170.0, 160.0, 139.0, 118.0]
            .into_iter()
            .enumerate()
        {
            let line = [
                "Zahlungsart",
                "Vorkasse per \u{dc}berweisung",
                "Versandart Paket",
                "Lieferung an die Adresse in der Bestellung",
                "Es gelten die allgemeinen Gesch\u{e4}ftsbedingungen des Verk\u{e4}ufers in der Fassung vom Bestelltag.",
                "Mit freundlichen Gr\u{fc}\u{df}en",
                "Ihr Team vom Shop",
                "Ihr Kundenservice",
            ][k];
            t += &show(if k == 0 { "FB" } else { "FL" }, 8.0, 77.0, y, line);
        }
    } else {
        t += &show("FL", 8.0, 77.0, 674.0, "Fortsetzung der Seite");
    }
    t += &show("FL", 8.0, 489.0, 102.0, &format!("Seite {n} von 2"));
    for column in measured_footer() {
        for (x, y, size, bold, text) in column {
            t += &show(if bold { "FB" } else { "FL" }, size, x, y, text);
        }
    }
    for y in [35.0, 27.0] {
        t += &show("FL", 7.0, 233.0, y, &" ".repeat(20));
    }
    t
}

/// Fix round 3: the reduced model of an invoice of two pages (a real footer shape, no source
/// named). On both pages every line
/// of the footer and the page number are running footers, the three text columns are one
/// block each with a line break between their lines, and the 11 pt shop name of the footer is
/// no heading: the bold title of the page is the only heading, at level 1. Two lines of white
/// space only under the second column are no body text that the footer must stand below. In
/// Helvetica and Times, at page scales 0.75, 1 and 1.5 (ruling P6-Y).
#[test]
fn the_measured_invoice_footer_is_furniture_with_its_line_breaks() {
    let mut wrong = Vec::new();
    for k in [0.75, 1.0, 1.5] {
        for times in [false, true] {
            for w in measured_invoice_broken(k, times) {
                wrong.push(format!("{k}x, Times {times}: {w}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// What [`the_measured_invoice_footer_is_furniture_with_its_line_breaks`] finds wrong on the
/// invoice drawn `k` times as large, in Times when `times`.
fn measured_invoice_broken(k: f32, times: bool) -> Vec<String> {
    let pages: Vec<FormPage> = (1..=2)
        .map(|n| {
            let mut template = measured_invoice_page(n);
            if times {
                template = template.replace("/FL ", "/FT ").replace("/FB ", "/FTB ");
            }
            FormPage {
                template,
                scale: k,
                ..FormPage::default()
            }
        })
        .collect();
    let out = common::read(&print_form(&pages, "", ""));
    let mut wrong = Vec::new();
    let footer = measured_footer();
    for n in 1..=2u32 {
        let texts = page_texts(&out, n);
        let number = format!("Seite {n} von 2");
        let lines = footer
            .iter()
            .flatten()
            .map(|l| l.4)
            .chain([number.as_str()]);
        for line in lines {
            let holders: Vec<&(Option<String>, String)> =
                texts.iter().filter(|(_, t)| t.contains(line)).collect();
            if holders.len() != 1 || holders[0].0.as_deref() != Some("page-footer") {
                wrong.push(format!("page {n}: {line:?} in {holders:?}"));
            }
        }
    }
    for b in blocks(&out) {
        let Block::Paragraph { inlines, .. } = b else {
            continue;
        };
        let text = plain_text(inlines);
        for column in &footer[..3] {
            if text.starts_with(column[0].4) && line_breaks(inlines) != column.len() - 1 {
                wrong.push(format!("{} breaks in {text:?}", line_breaks(inlines)));
            }
        }
    }
    let heads = headings(&out);
    if heads != [(1, "Rechnungsnr. 0000001".to_string())] {
        wrong.push(format!("headings {heads:?}"));
    }
    wrong
}

/// The two-column tables of the Task 13 review (finding I-1), each a header and its rows.
const BODY_TABLES: [[[&str; 2]; 4]; 3] = [
    [
        ["Artikel", "Menge"],
        ["Schraube M4", "200 Stk"],
        ["Mutter M5", "150 Stk"],
        ["Scheibe M6", "300 Stk"],
    ],
    [
        ["Strasse", "Ort"],
        ["Musterweg 1", "12345 Musterstadt"],
        ["Musterweg 2", "23456 Musterdorf"],
        ["Musterweg 3", "34567 Musterhausen"],
    ],
    [
        ["Leistung", "Tarif"],
        ["Wartung Q1", "Tarif A2"],
        ["Pflege Q2", "Tarif B3"],
        ["Reinigung Q3", "Tarif C4"],
    ],
];

/// The looks of the review's probes: page scales and fonts (ruling P6-Y).
fn probe_looks() -> Vec<(f32, (&'static str, &'static str))> {
    [0.75, 1.0, 1.5]
        .into_iter()
        .flat_map(|k| FONTS.map(|f| (k, f)))
        .collect()
}

/// The looks of the review's probes with the first baseline of the block: in the body (700),
/// wholly in the top band (780) and wholly in the bottom band (140) of the page.
fn probe_places() -> Vec<(f32, (&'static str, &'static str), f32)> {
    probe_looks()
        .into_iter()
        .flat_map(|(k, f)| [700.0, 780.0, 140.0].map(|top| (k, f, top)))
        .collect()
}

/// Rulings P6-X4 and P6-X5 (review findings I-1 and M-6): a two-column table in the body or
/// wholly in a page band that is no running header or footer, with a header row and
/// cells of text and numbers stays a table, its values set flush left at x 200 or flush right
/// at x 260, in every size and font. Address-shaped columns are a matter of the page bands
/// only.
#[test]
fn two_column_body_tables_with_a_header_stay_tables() {
    let mut wrong = Vec::new();
    for (k, fonts, top) in probe_places() {
        for table in BODY_TABLES {
            for flush_right in [false, true] {
                let mut t = String::new();
                for (r, [a, b]) in table.iter().enumerate() {
                    let y = top - 10.0 * r as f32;
                    let font = if r == 0 { fonts.1 } else { fonts.0 };
                    t += &show(font, 8.0, 40.0, y, a);
                    let x = if flush_right {
                        260.0 - common::font_width(font, b, 8.0)
                    } else {
                        200.0
                    };
                    t += &show(font, 8.0, x, y, b);
                }
                let out = common::read(&page_in(&t, Look::Scaled(k)));
                let want: Vec<Vec<String>> = table
                    .iter()
                    .map(|r| r.iter().map(|c| (*c).to_string()).collect())
                    .collect();
                if !tables(&out).iter().any(|t| t.1 == want) {
                    wrong.push(format!(
                        "{k}x, {fonts:?}, top {top}, {}, flush right {flush_right}: {:?} {:?}",
                        table[0][0],
                        tables(&out),
                        texts_of(&out)
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The headerless notice block of the Task 13 review: plain labels, some with a digit, beside
/// values with letters, one value empty.
const NOTICE_PAIRS: [(&str, &str); 6] = [
    ("Name", "Erika Mustermann"),
    ("Zimmer 2", "Raum 101"),
    ("Aktenzeichen", "Az 000 B"),
    ("Steuerjahr 2025", "Einzelveranlagung"),
    ("Veranlagung", ""),
    ("Bearbeitung", "Max Mustermann"),
];

/// The address, bank, register and contact blocks of carries C-5 and C-3, of 3 to 5 lines.
const SIDE_BLOCKS: [&[&str]; 4] = [
    &[
        "Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt",
        "Deutschland",
    ],
    &[
        "Musterbank",
        "IBAN DE00 0000 0000 0000 0000 00",
        "BIC MUSTDEXXXXX",
    ],
    &[
        "Amtsgericht Musterstadt",
        "HRB 00000",
        "USt-IdNr. DE000000000",
        "Geschaeftsfuehrer Max Muster",
        "Erika Mustermann",
    ],
    &[
        "Tel. 0000 000000",
        "E-Mail info@muster.example",
        "Web www.muster.example",
    ],
];

/// Carries C-5 and C-3 (ruling P6-X: a block that mixes two columns is a wrong reading order):
/// blocks of an address, a bank, a register entry and a contact side by side on shared
/// baselines, top aligned, three sizes apart, in the body at the top of a page or at its foot
/// on a file of one page (no running footer), the first 2, 3 or 4 of them, cut to their first
/// 3 lines or whole: no block of the read holds lines of two of them, and each line is read.
/// Sizes 6 to 14, both fonts.
#[test]
fn blocks_side_by_side_in_the_body_are_read_block_by_block() {
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for fonts in FONTS {
            for (top, whole, n) in [700.0, 90.0]
                .into_iter()
                .flat_map(|top| [false, true].map(|whole| (top, whole)))
                .flat_map(|(top, whole)| [2, 3, 4].map(|n| (top, whole, n)))
            {
                let blocks: Vec<&[&str]> = SIDE_BLOCKS[..n]
                    .iter()
                    .map(|b| if whole { *b } else { &b[..3] })
                    .collect();
                let mut t = String::new();
                let mut x = 40.0;
                for lines in &blocks {
                    let mut widest: f32 = 0.0;
                    for (n, line) in lines.iter().enumerate() {
                        t += &show(fonts.0, size, x, top - 1.2 * size * n as f32, line);
                        widest = widest.max(common::font_width(fonts.0, line, size));
                    }
                    x += widest + 3.0 * size;
                }
                let out = common::read(&page_in(&t, Look::Plain));
                let texts = texts_of(&out);
                let at = format!("{size} pt, {fonts:?}, top {top}, whole {whole}, {n} blocks");
                for (c, lines) in blocks.iter().enumerate() {
                    for line in lines.iter() {
                        if !texts.iter().any(|t| t.contains(line)) {
                            wrong.push(format!("{at}: {line:?} not read: {texts:?}"));
                        }
                    }
                    for other in &blocks[c + 1..] {
                        let mixed = texts.iter().find(|t| {
                            lines.iter().any(|l| t.contains(l))
                                && other.iter().any(|l| t.contains(l))
                        });
                        if let Some(m) = mixed {
                            wrong.push(format!("{at}: two blocks in {m:?}"));
                        }
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Lines of `rows` (cells left to right, `""` for none) at `size`, columns 15 sizes apart, the
/// first row bold when `bold`.
fn small_rows(rows: &[&[&str]], size: f32, fonts: (&str, &str), bold: bool) -> String {
    let k = size / 8.0;
    let mut t = String::new();
    for (r, row) in rows.iter().enumerate() {
        let f = if bold && r == 0 { fonts.1 } else { fonts.0 };
        for (c, cell) in row.iter().enumerate() {
            if !cell.is_empty() {
                t += &show(
                    f,
                    size,
                    40.0 + 120.0 * k * c as f32,
                    700.0 - 1.2 * size * r as f32,
                    cell,
                );
            }
        }
    }
    t
}

/// Task 15b part 2c review (Critical 1, ruling P6-X): small tables whose columns list records
/// of one kind, with a plain header, a bold one or none, and line items with amounts, are
/// read as tables, row by row, as at f13f512, never block by block. Sizes 6 to 14, both fonts.
#[test]
fn small_tables_keep_their_rows() {
    let items: &[&[&str]] = &[
        &["Artikel", "Lager", "Menge"],
        &["Schraube M4", "Halle 2", "12 Stueck"],
        &["Mutter M5", "Regal 3B", "8 Stueck"],
        &["Scheibe", "Halle 1", "20 Stueck"],
        &["Duebel 6", "Halle 3", "5 Stueck"],
    ];
    let shipments: &[&[&str]] = &[
        &["Muster GmbH", "Lieferung 12", "3 Kartons"],
        &["Beispiel AG", "Lieferung 13", "12 Paletten"],
        &["Muster KG 2", "Ruecksendung", "1 Karton"],
    ];
    let lines: &[&[&str]] = &[
        &["Beratung 2 Std", "1.000,00 EUR"],
        &["Material", "200,00 EUR"],
        &["Versand 3", "34,56 EUR"],
    ];
    let cases: [(&str, &[&[&str]], bool); 5] = [
        ("plain header, 3 rows", &items[..4], false),
        ("plain header, 4 rows", items, false),
        ("bold header", &items[..4], true),
        ("headerless", shipments, false),
        ("line items", lines, false),
    ];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for fonts in FONTS {
            for (name, rows, bold) in cases {
                let out = common::read(&page_in(&small_rows(rows, size, fonts, bold), Look::Plain));
                let want: Vec<Vec<String>> = rows
                    .iter()
                    .map(|r| r.iter().map(|c| (*c).to_string()).collect())
                    .collect();
                if !tables(&out).iter().any(|t| t.1 == want) {
                    wrong.push(format!("{name}, {size}, {fonts:?}: {:?}", texts_of(&out)));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Task 15b part 2c review (Important 1, ruling P6-X): labels beside values whose columns are
/// of uneven height (a last label without a value, a value of two address lines) or full keep
/// each value with its label, or a loss says that labels beside values are read apart; never
/// silently apart. Sizes 6 to 14, both fonts.
#[test]
fn label_blocks_keep_their_pairs_or_say_so() {
    let cases: [(&str, &[&[&str]]); 3] = [
        (
            "a last label without a value",
            &[
                &["Zimmer 2", "Raum 101"],
                &["Steuerjahr 2025", "Einzelveranlagung"],
                &["Bearbeitung", "Max Mustermann"],
                &["Aktenzeichen", ""],
            ],
        ),
        (
            "a value of two address lines",
            &[
                &["Name", "Erika Mustermann"],
                &["Steuerjahr 2025", "Raum 101"],
                &["Anschrift", "Musterweg 1"],
                &["", "12345 Musterstadt"],
            ],
        ),
        (
            "full",
            &[
                &["Zimmer 2", "Raum 101"],
                &["Steuerjahr 2025", "Einzelveranlagung"],
                &["Bearbeitung", "Max Mustermann 3"],
            ],
        ),
    ];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for fonts in FONTS {
            for (name, rows) in cases {
                let out =
                    common::read(&page_in(&small_rows(rows, size, fonts, false), Look::Plain));
                let said = out
                    .document
                    .losses
                    .iter()
                    .any(|l| l.what.contains("labels beside values"));
                let found = tables(&out);
                let paired = rows
                    .iter()
                    .filter(|r| !r[1].is_empty() && !r[0].is_empty())
                    .all(|r| {
                        found.iter().any(|t| {
                            t.1.iter().any(|c| {
                                c.first().map(String::as_str) == Some(r[0])
                                    && c.get(1).is_some_and(|v| v.starts_with(r[1]))
                            })
                        })
                    });
                if !said && !paired {
                    wrong.push(format!(
                        "{name}, {size}, {fonts:?}: {found:?} {:?}",
                        texts_of(&out)
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Task 15b part 2c review (Strengths, Minor 5): blocks side by side one line lower, bottom
/// aligned, with a PO box, a c/o line, or under bold headings are read block by block, or a
/// loss says that a table there may be blocks; every line is read. Sizes 6 to 14, both fonts.
#[test]
fn side_blocks_offset_or_headed_are_apart_or_said() {
    let addr: &[&str] = &[
        "Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt",
        "Deutschland",
    ];
    let bank: &[&str] = &[
        "Musterbank",
        "IBAN DE00 0000 0000 0000 0000 00",
        "BIC MUSTDEXXXXX",
    ];
    let reg: &[&str] = &[
        "Amtsgericht Musterstadt",
        "HRB 00000",
        "USt-IdNr. DE000000000",
    ];
    let pobox: &[&str] = &["Muster GmbH", "Postfach 12 34", "12345 Musterstadt"];
    let co: &[&str] = &[
        "Erika Mustermann",
        "c/o Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt",
    ];
    let h_addr: &[&str] = &[
        "Anschrift",
        "Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt",
    ];
    let h_bank: &[&str] = &[
        "Bankverbindung",
        "Musterbank",
        "IBAN DE00 0000 0000 0000 0000 00",
        "BIC MUSTDEXXXXX",
    ];
    let h_reg: &[&str] = &["Register", "Amtsgericht Musterstadt", "HRB 00000"];
    type Blocks<'a> = Vec<(&'a [&'a str], usize, bool)>;
    let cases: Vec<(&str, Blocks<'_>)> = vec![
        ("top 4/3", vec![(addr, 0, false), (bank, 0, false)]),
        (
            "bank one lower",
            vec![(addr, 0, false), (bank, 1, false), (reg, 0, false)],
        ),
        (
            "bottom aligned",
            vec![(addr, 0, false), (bank, 1, false), (reg, 1, false)],
        ),
        (
            "headed",
            vec![(h_addr, 0, true), (h_bank, 0, true), (h_reg, 0, true)],
        ),
        ("po box", vec![(pobox, 0, false), (bank, 0, false)]),
        (
            "c/o",
            vec![(co, 0, false), (bank, 0, false), (reg, 0, false)],
        ),
    ];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for fonts in FONTS {
            for (name, blocks) in &cases {
                let mut t = String::new();
                let mut x = 40.0;
                for (lines, off, bold) in blocks {
                    let mut widest: f32 = 0.0;
                    for (n, line) in lines.iter().enumerate() {
                        let f = if *bold && n == 0 { fonts.1 } else { fonts.0 };
                        t += &show(f, size, x, 700.0 - 1.2 * size * (n + off) as f32, line);
                        widest = widest.max(common::font_width(f, line, size));
                    }
                    x += widest + 3.0 * size;
                }
                let out = common::read(&page_in(&t, Look::Plain));
                let texts = texts_of(&out);
                let said = out
                    .document
                    .losses
                    .iter()
                    .any(|l| l.what.contains("side by side"));
                let at = format!("{name}, {size}, {fonts:?}");
                for (c, (lines, ..)) in blocks.iter().enumerate() {
                    for line in lines.iter() {
                        if !texts.iter().any(|t| t.contains(line)) {
                            wrong.push(format!("{at}: {line:?} not read"));
                        }
                    }
                    for (other, ..) in &blocks[c + 1..] {
                        let mixed = texts.iter().find(|t| {
                            lines.iter().any(|l| t.contains(l))
                                && other.iter().any(|l| t.contains(l))
                        });
                        if let (Some(m), false) = (mixed, said) {
                            wrong.push(format!("{at}: two blocks in {m:?}, no loss"));
                        }
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Rulings P6-X4 and P6-X5 (review findings I-1 and M-6): the notice block in the body or wholly
/// in a page band that is no running header or footer, the labels plain at x 40
/// and the values bold flush right at x 300 or plain at x 160, on shared baselines: each
/// value is read with its label, before the next label, in every size and font.
#[test]
fn a_headerless_notice_block_in_the_body_keeps_its_pairs() {
    let mut wrong = Vec::new();
    for (k, fonts, top) in probe_places() {
        for bold_right in [true, false] {
            let mut t = String::new();
            for (r, (label, value)) in NOTICE_PAIRS.iter().enumerate() {
                let y = top - 10.0 * r as f32;
                t += &show(fonts.0, 8.0, 40.0, y, label);
                if value.is_empty() {
                    continue;
                }
                if bold_right {
                    let x = 300.0 - common::font_width(fonts.1, value, 8.0);
                    t += &show(fonts.1, 8.0, x, y, value);
                } else {
                    t += &show(fonts.0, 8.0, 160.0, y, value);
                }
            }
            let out = common::read(&page_in(&t, Look::Scaled(k)));
            let all = texts_of(&out).join("\n");
            for (r, (label, value)) in NOTICE_PAIRS.iter().enumerate() {
                if value.is_empty() {
                    continue;
                }
                let next = NOTICE_PAIRS.get(r + 1).map(|p| p.0);
                let order: Vec<&str> = [*label, *value].into_iter().chain(next).collect();
                if let Err(e) = common::in_order(&all, &order) {
                    wrong.push(format!(
                        "{k}x, {fonts:?}, top {top}, bold right {bold_right}: {e}"
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Review finding M-1: a sentence of a footer that wraps at one edge reads as one line, and a
/// line that starts a new item keeps its line break. The two lines of `lines` at x 40, 8 points
/// apart, as the footer of both pages of a file of two, in every size and font; the result is
/// the number of line breaks in their `page-footer` paragraph.
fn footer_wrap_breaks(lines: [&str; 2], k: f32, fonts: (&str, &str)) -> Option<usize> {
    let t = show(fonts.0, 7.0, 40.0, 50.0, lines[0]) + &show(fonts.0, 7.0, 40.0, 42.0, lines[1]);
    let out = common::read(&footed_pages(&t, 2, k, false));
    let mut found = None;
    for b in blocks(&out) {
        if let Block::Paragraph { inlines, role, .. } = b
            && role.as_deref() == Some("page-footer")
            && plain_text(inlines) == lines.join(" ")
        {
            found = Some(line_breaks(inlines));
        }
    }
    found
}

#[test]
fn a_wrapped_footer_sentence_joins_and_footer_items_keep_their_breaks() {
    let cases: [([&str; 2], usize); 4] = [
        (
            [
                "Es gelten unsere Allgemeinen Gesch\u{e4}ftsbedingungen in der jeweils",
                "aktuellen Fassung, die wir Ihnen auf Wunsch gern zusenden.",
            ],
            0,
        ),
        (
            [
                "Es gelten unsere Allgemeinen Gesch\u{e4}ftsbedingungen.",
                "die Muster GmbH sendet sie Ihnen auf Wunsch gern zu.",
            ],
            1,
        ),
        (
            [
                "Es gelten unsere Allgemeinen Gesch\u{e4}ftsbedingungen in der jeweils",
                "Fassung vom Tag der Bestellung, die wir Ihnen gern zusenden.",
            ],
            1,
        ),
        (["Tel. 0000 000000", "info@muster.example"], 1),
    ];
    let mut wrong = Vec::new();
    for (k, fonts) in probe_looks() {
        for (lines, want) in cases {
            let got = footer_wrap_breaks(lines, k, fonts);
            if got != Some(want) {
                wrong.push(format!(
                    "{k}x, {fonts:?}, {lines:?}: {got:?} breaks, want {want}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Review carry C-6: the footer of [`FOOTER`] at 7 pt in columns at x 40, 180, 330 and 460 from
/// the baseline 100 down, 8.4 apart, and the page number `Seite n von N` centred at x 290,
/// 20 points over the first row (`above`) or 40 points under the last row.
fn footer_with_page_number(fonts: (&str, &str), n: usize, pages: usize, above: bool) -> String {
    let mut t = String::new();
    for (c, lines) in FOOTER.iter().enumerate() {
        let x = [40.0, 180.0, 330.0, 460.0][c];
        for (k, line) in lines.iter().enumerate() {
            t += &show(fonts.0, 7.0, x, 100.0 - 8.4 * k as f32, line);
        }
    }
    let rows = FOOTER.iter().map(|c| c.len()).max().unwrap_or(0);
    let y = if above {
        120.0
    } else {
        100.0 - 8.4 * (rows - 1) as f32 - 40.0
    };
    t + &show(fonts.0, 7.0, 290.0, y, &format!("Seite {n} von {pages}"))
}

/// Review carry C-6 (fix round 6): a page number centred over or under a running footer of
/// four columns takes no part in finding its columns. Each column is one block in column
/// order, no block holds lines of two columns, and the page number is a running footer of its
/// own, in every size and font, on files of two and three pages.
#[test]
fn a_centred_page_number_does_not_bridge_footer_columns() {
    let columns: Vec<Vec<&str>> = FOOTER.iter().map(|c| c.to_vec()).collect();
    let mut wrong = Vec::new();
    for (k, fonts) in probe_looks() {
        for pages in [2, 3] {
            for above in [true, false] {
                let file: Vec<FormPage> = (1..=pages)
                    .map(|n| FormPage {
                        template: format!(
                            "{}{}",
                            body_of(7.0, 500.0, true),
                            footer_with_page_number(fonts, n, pages, above)
                        ),
                        scale: k,
                        ..FormPage::default()
                    })
                    .collect();
                let out = common::read(&print_form(&file, "", ""));
                let name = format!("{k}x, {fonts:?}, {pages} pages, above {above}");
                for n in 1..=pages as u32 {
                    for w in footer_broken(&out, n, &columns, Some("page-footer")) {
                        wrong.push(format!("{name}: {w}"));
                    }
                    let number = format!("Seite {n} von {pages}");
                    let texts = page_texts(&out, n);
                    if !texts
                        .iter()
                        .any(|(r, t)| r.as_deref() == Some("page-footer") && *t == number)
                    {
                        wrong.push(format!("{name}: page {n} number: {texts:?}"));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The logos of an invoice template: 100 by 50 points top left, 100 by 40 at the right just
/// below the top 15 % of the page, 60 by 30 bottom left. Each covers less than 2 % of the
/// page, together more.
fn invoice_logos() -> String {
    [
        logo(40.0, 770.0, 100.0, 50.0),
        logo(455.0, 692.0, 100.0, 40.0),
        logo(40.0, 30.0, 60.0, 30.0),
    ]
    .concat()
}

/// The alt texts of the pictures on page `n`, in the order the page holds them.
fn media_alts(out: &ReadOutcome, n: u32) -> Vec<String> {
    fn walk(blocks: &[Block], found: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Section { blocks, .. } => walk(blocks, found),
                Block::Media { alt, .. } => found.push(alt.clone().unwrap_or_default()),
                _ => {}
            }
        }
    }
    let want = rezzer_model::Anchor(format!("p{n}"));
    let page = out
        .document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section {
                kind: rezzer_model::SectionKind::Page,
                anchor: Some(a),
                blocks,
                ..
            } if *a == want => Some(blocks.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no page {n}"));
    let mut found = Vec::new();
    walk(page, &mut found);
    found
}

#[test]
fn template_logos_need_no_vision() {
    let out = pages_logos(2);
    assert!(needing(&out).is_empty(), "{:#?}", out.document.assets);
    let md = markdown(&out);
    assert!(!md.contains("Needs vision"), "{md}");
    for n in 1..=2 {
        let alts = media_alts(&out, n);
        assert_eq!(alts.len(), 3, "page {n}: {alts:?}");
        assert!(
            alts.iter()
                .any(|a| a.starts_with("logo or small picture") && a.contains("box 455, 692")),
            "page {n}: {alts:?}"
        );
    }
}

#[test]
fn logos_on_one_page_need_no_vision() {
    let out = pages_logos(1);
    assert!(needing(&out).is_empty(), "{:#?}", out.document.assets);
    let alts = media_alts(&out, 1);
    assert_eq!(alts.len(), 3, "{alts:?}");
    assert!(
        alts.iter().all(|a| a.starts_with("logo or small picture")),
        "{alts:?}"
    );
}

/// `count` pages of the invoice template with its logos and the body.
fn pages_logos(count: usize) -> ReadOutcome {
    common::read(&pages(&vec![invoice_logos(); count]))
}

/// The places of template logos, as shares of the page width and height (x0, y0, x1, y1):
/// the four corners, in the corner but outside the top and bottom 18 % of the page, and the
/// middle of the top and the bottom band. Each logo covers less than 2 % of the page.
const LOGO_PLACES: [(&str, [f32; 4]); 6] = [
    ("top left", [0.04, 0.805, 0.16, 0.848]),
    ("top right", [0.84, 0.805, 0.96, 0.848]),
    ("bottom left", [0.04, 0.152, 0.16, 0.195]),
    ("bottom right", [0.84, 0.152, 0.96, 0.195]),
    ("top band", [0.4, 0.88, 0.6, 0.93]),
    ("bottom band", [0.4, 0.03, 0.6, 0.08]),
];

/// A paper by its name and its size in points.
type Paper = (&'static str, (f32, f32));

/// What is wrong with the read of `count` pages of `paper` printed at `scale`, with logos
/// at `places` and, with `content`, a large content picture of each page's own.
fn template_logo_variant(
    paper: Paper,
    scale: f32,
    places: &[&(&str, [f32; 4])],
    count: usize,
    content: bool,
) -> Vec<String> {
    let (paper, (w, h)) = paper;
    let template: String = places
        .iter()
        .map(|(_, [x0, y0, x1, y1])| logo(x0 * w, y0 * h, (x1 - x0) * w, (y1 - y0) * h))
        .collect();
    let file: Vec<FormPage> = (0..count)
        .map(|_| FormPage {
            template: format!("{template}{}", show("FL", 7.0, 40.0, 500.0, BODY_LINE)),
            scale,
            paper: (w, h),
            photo: content.then_some([0.25 * w, 0.22 * h, 0.5 * w, 0.17 * h]),
            ..FormPage::default()
        })
        .collect();
    let out = common::read(&print_form(&file, "", ""));
    let names: Vec<&str> = places.iter().map(|(n, _)| *n).collect();
    let name = format!("{paper} {scale}x, {count} pages, logos {names:?}, content {content}");
    let mut wrong = Vec::new();
    let needing = needing(&out);
    for n in 1..=count {
        // A large content picture always needs vision: its page is rendered whole, logos and
        // all.
        let page = format!("page-{n}");
        if content != needing.contains(&page.as_str()) {
            wrong.push(format!("{name}: page {n} needing {needing:?}"));
        }
        // Its alt text counts the logos with the content picture; from three pages on they
        // are page furniture, which it does not count.
        if content {
            let k = if count >= 3 { 1 } else { places.len() + 1 };
            let want = format!("{k} {}", if k == 1 { "picture" } else { "pictures" });
            let alt = out
                .document
                .assets
                .iter()
                .find(|a| a.id == page)
                .and_then(|a| a.alt.clone());
            if alt.as_deref() != Some(want.as_str()) {
                wrong.push(format!("{name}: page {n} alt {alt:?}"));
            }
        }
        // Every logo of a page that is not rendered whole is listed as a logo. From three
        // pages on, logos at one place are page furniture: each is listed once, at its
        // place on the first page, with its pages.
        let alts = media_alts(&out, n as u32);
        let furniture = format!(", on pages 1 to {count}");
        let listed = alts
            .iter()
            .filter(|a| {
                a.starts_with("logo or small picture")
                    && (count < 3 || a.ends_with(furniture.as_str()))
            })
            .count();
        let want = if content || (count >= 3 && n > 1) {
            0
        } else {
            places.len()
        };
        if listed != want || (!content && alts.len() != want) {
            wrong.push(format!("{name}: page {n} lists {alts:?}"));
        }
    }
    // Template logos never need vision.
    if needing.iter().any(|id| id.contains("-pic-")) {
        wrong.push(format!("{name}: needing {needing:?}"));
    }
    wrong
}

#[test]
fn template_logos_never_need_vision_and_content_always_does() {
    // A4 and Letter, printed at 0.75, 1 and 1.5 times their size; one to four logos at the
    // places of `LOGO_PLACES` (each run of places in turn); files of one, two and three
    // pages; without and, for two runs of places, with a large content picture (8.5 % of the
    // page, its own on every page) under a line of text. Each paper and scale is read on a
    // thread of its own.
    let setups: Vec<(Paper, f32)> = [("A4", (595.0, 842.0)), ("Letter", (612.0, 792.0))]
        .into_iter()
        .flat_map(|paper| [0.75, 1.0, 1.5].map(|scale| (paper, scale)))
        .collect();
    let runs: Vec<(usize, Vec<String>)> = std::thread::scope(|scope| {
        let threads: Vec<_> = setups
            .iter()
            .map(|&(paper, scale)| {
                scope.spawn(move || {
                    let mut reads = 0;
                    let mut wrong = Vec::new();
                    for logos in 1..=4 {
                        for first in 0..LOGO_PLACES.len() {
                            let places: Vec<&(&str, [f32; 4])> = (0..logos)
                                .map(|i| &LOGO_PLACES[(first + i) % LOGO_PLACES.len()])
                                .collect();
                            for count in 1..=3 {
                                // A page that needs vision is rendered to be read for codes,
                                // which is slow: the content picture is tried with two runs
                                // of places only.
                                let contents: &[bool] = if first % 3 == 0 {
                                    &[false, true]
                                } else {
                                    &[false]
                                };
                                for &content in contents {
                                    reads += 1;
                                    wrong.extend(template_logo_variant(
                                        paper, scale, &places, count, content,
                                    ));
                                }
                            }
                        }
                    }
                    (reads, wrong)
                })
            })
            .collect();
        threads.into_iter().map(|t| t.join().unwrap()).collect()
    });
    let reads: usize = runs.iter().map(|(n, _)| n).sum();
    let wrong: Vec<String> = runs.into_iter().flat_map(|(_, w)| w).collect();
    assert_eq!(reads, 6 * 4 * (6 + 2) * 3);
    assert!(
        wrong.is_empty(),
        "{} of {reads} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_figure_of_small_pictures_across_the_band_edge_needs_vision() {
    // Four columns of three small pictures (40 by 40 points, 3 points apart), the top two
    // rows in the top band, the lowest row under it: one figure, together over 2 % of the
    // page. A4 and Letter, printed at 0.75, 1 and 1.5 times their size.
    for (w, h) in [(595.0f32, 842.0f32), (612.0, 792.0)] {
        let mut template = show("FL", 7.0, 40.0, 500.0, BODY_LINE);
        for column in 0..4 {
            for row in 0..3 {
                let x = 0.28 * w + column as f32 * 70.0;
                let y = 0.88 * h - row as f32 * 43.0;
                template.push_str(&logo(x, y, 40.0, 40.0));
            }
        }
        for scale in [0.75, 1.0, 1.5] {
            let page = FormPage {
                template: template.clone(),
                scale,
                paper: (w, h),
                ..FormPage::default()
            };
            let out = common::read(&print_form(&[page], "", ""));
            assert_eq!(
                needing(&out),
                ["page-1"],
                "{w} by {h} at {scale}x: {:#?}",
                out.document.assets
            );
        }
    }
}

/// Content drawn as tiles or strips in a logo place, as (x, y, width, height) shares of an A4
/// page: a photo of 14 strips in the top band (11 % of the page), a chart of 2 by 2 tiles in
/// the bottom band (7.2 %), 2 by 2 tiles in the top left corner (3.5 %) and 30 tiles each
/// under 1 cm2 in the bottom band (3.7 %). Each tile alone covers less than 2 %.
fn tiled_contents() -> Vec<(&'static str, Vec<[f32; 4]>)> {
    let (w, h) = (595.0, 842.0);
    let share = |x: f32, y: f32, dx: f32, dy: f32| [x / w, y / h, dx / w, dy / h];
    let strips = (0..14).map(|i| share(97.0, 700.0 + i as f32 * 10.0, 400.0, 10.0));
    let chart = (0..4).map(|i| {
        share(
            147.0 + (i % 2) as f32 * 150.0,
            30.0 + (i / 2) as f32 * 60.0,
            150.0,
            60.0,
        )
    });
    let corner = (0..4).map(|i| {
        share(
            8.0 + (i % 2) as f32 * 55.0,
            674.0 + (i / 2) as f32 * 80.0,
            55.0,
            80.0,
        )
    });
    let tiny = (0..30).map(|i| {
        share(
            110.0 + (i % 15) as f32 * 25.0,
            40.0 + (i / 15) as f32 * 25.0,
            25.0,
            25.0,
        )
    });
    vec![
        ("strips in the top band", strips.collect()),
        ("tiles in the bottom band", chart.collect()),
        ("tiles in the top left corner", corner.collect()),
        ("tiny tiles in the bottom band", tiny.collect()),
    ]
}

#[test]
fn content_drawn_as_tiles_in_a_logo_place_needs_vision() {
    // On A4 and Letter, printed at 0.75, 1 and 1.5 times the size, alone on its page or on
    // the first of two pages.
    let mut wrong = Vec::new();
    for (paper, (w, h)) in [("A4", (595.0f32, 842.0f32)), ("Letter", (612.0, 792.0))] {
        for (name, tiles) in tiled_contents() {
            let drawn: String = tiles
                .iter()
                .map(|[x, y, dx, dy]| logo(x * w, y * h, dx * w, dy * h))
                .collect();
            let text = show("FL", 7.0, 40.0, 500.0, BODY_LINE);
            for scale in [0.75f32, 1.0, 1.5] {
                for count in 1..=2 {
                    let file: Vec<FormPage> = (0..count)
                        .map(|n| FormPage {
                            template: if n == 0 {
                                format!("{drawn}{text}")
                            } else {
                                text.clone()
                            },
                            scale,
                            paper: (w, h),
                            ..FormPage::default()
                        })
                        .collect();
                    let out = common::read(&print_form(&file, "", ""));
                    if needing(&out) != ["page-1"] {
                        wrong.push(format!(
                            "{paper} {scale}x, {count} pages, {name}: {:?}",
                            needing(&out)
                        ));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn furniture_on_a_page_rendered_whole_is_named_with_every_page() {
    // A logo at one place on three pages; the first page also has a large photo, so it is
    // rendered whole. The logo is listed on page 2, named with all three pages.
    let file: Vec<FormPage> = (0..3)
        .map(|n| FormPage {
            template: format!(
                "{}{}",
                logo(40.0, 30.0, 60.0, 30.0),
                show("FL", 7.0, 40.0, 500.0, BODY_LINE)
            ),
            photo: (n == 0).then_some([150.0, 200.0, 300.0, 200.0]),
            ..FormPage::default()
        })
        .collect();
    let out = common::read(&print_form(&file, "", ""));
    assert_eq!(needing(&out), ["page-1"], "{:#?}", out.document.assets);
    let alt = "logo or small picture, box 40, 30 to 100, 60, on pages 1 to 3";
    let a = out
        .document
        .assets
        .iter()
        .find(|a| a.id == "page-2-pic-1")
        .unwrap_or_else(|| panic!("{:#?}", out.document.assets));
    assert_eq!(a.alt.as_deref(), Some(alt));
    assert_eq!(media_alts(&out, 2), [alt]);
    assert!(media_alts(&out, 3).is_empty());
}

/// A file of one page with `head` over the body in `body` pt, drawn `scale` times as large.
fn page_with(head: &str, body_size: f32, scale: f32) -> Vec<u8> {
    let page = FormPage {
        template: format!("{head}{}", body(body_size, 500.0)),
        scale,
        ..FormPage::default()
    };
    print_form(&[page], "", "")
}

#[test]
fn recipient_name_is_not_the_title() {
    let head = show("FB", 10.0, 40.0, 720.0, "Erika Mustermann")
        + &show("FL", 8.0, 40.0, 709.0, "Musterweg 1")
        + &show("FL", 8.0, 40.0, 699.0, "12345 Musterstadt")
        + &show("FB", 9.0, 40.0, 640.0, "Rechnung");
    let out = common::read(&page_with(&head, 7.0, 1.0));
    assert_eq!(out.document.properties.title.as_deref(), Some("Rechnung"));
}

#[test]
fn recipient_name_is_not_the_title_in_any_size_and_font() {
    // The same shape in other sizes, fonts and page sizes: every size relative to the body.
    for b in [6.0f32, 7.0, 9.0] {
        for (bold, plain) in [("FB", "FL"), ("FTB", "FT")] {
            for scale in [0.8f32, 1.0, 1.25] {
                let step = b * 8.0 / 7.0 * 1.3;
                let head = show(bold, b * 10.0 / 7.0, 40.0, 720.0, "Erika Mustermann")
                    + &show(plain, b * 8.0 / 7.0, 40.0, 720.0 - step, "Musterweg 1")
                    + &show(
                        plain,
                        b * 8.0 / 7.0,
                        40.0,
                        720.0 - 2.0 * step,
                        "12345 Musterstadt",
                    )
                    + &show(bold, b * 9.0 / 7.0, 40.0, 640.0, "Rechnung");
                let out = common::read(&page_with(&head, b, scale));
                assert_eq!(
                    out.document.properties.title.as_deref(),
                    Some("Rechnung"),
                    "body {b}, fonts {bold} {plain}, scale {scale}"
                );
            }
        }
    }
}

#[test]
fn footer_heading_does_not_hide_the_title() {
    for (b, scale) in [(7.0f32, 0.8f32), (7.0, 1.0), (9.0, 1.25)] {
        let head = show("FB", b * 10.0 / 7.0, 40.0, 720.0, "Rechnung")
            + &show("FB", b * 12.0 / 7.0, 40.0, 40.0, "Muster GmbH");
        let out = common::read(&page_with(&head, b, scale));
        assert_eq!(
            out.document.properties.title.as_deref(),
            Some("Rechnung"),
            "body {b}, scale {scale}"
        );
    }
}

#[test]
fn recipient_name_over_a_longer_line_is_not_the_title() {
    // A line that is no address line right under the town: the name still opens an address.
    for b in [6.0f32, 7.0, 9.0] {
        for scale in [0.8f32, 1.0, 1.25] {
            let small = b * 8.0 / 7.0;
            let step = small * 1.3;
            let head = show("FB", b * 10.0 / 7.0, 40.0, 720.0, "Erika Mustermann")
                + &show("FL", small, 40.0, 720.0 - step, "Musterweg 1")
                + &show("FL", small, 40.0, 720.0 - 2.0 * step, "12345 Musterstadt")
                + &show(
                    "FL",
                    small,
                    40.0,
                    720.0 - 3.0 * step,
                    "Kundennummer 4711 bitte bei jeder Zahlung angeben",
                )
                + &show("FB", b * 9.0 / 7.0, 40.0, 640.0, "Rechnung");
            let out = common::read(&page_with(&head, b, scale));
            assert_eq!(
                out.document.properties.title.as_deref(),
                Some("Rechnung"),
                "body {b}, scale {scale}"
            );
        }
    }
}
