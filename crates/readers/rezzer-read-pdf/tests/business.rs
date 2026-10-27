// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Business documents (invoices, offers, orders, delivery notes), built in code with invented
//! values only. Unless a test says otherwise, a page has a body of at least 600 ink characters
//! of plain 7 pt text, so that 7 pt is the body size.

mod common;

use common::{FormPage, print_form, show};
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
                Close::Rate => c.replace(" %", "%"),
                _ => c.to_string(),
            })
        })
        .collect();
    let (second, a, b) = match close {
        Close::OneShow => ("Einheit", 2, 3),
        Close::FlushRight | Close::Apart => ("USt-Satz", 2, 4),
        Close::Rate => ("MwSt.", 2, 4),
    };
    let apart = if matches!(close, Close::Apart) {
        0.9
    } else {
        0.4
    };
    let mut t = show(tf, size, 40.0, 600.0, "Pos") + &show(tf, size, 80.0, 600.0, "Artikel");
    // Where the second title starts, and where the values of each title stand.
    let x2 = match close {
        Close::OneShow => {
            t += &show(tf, size, 300.0, 600.0, "Menge Einheit");
            300.0 + common::font_width(tf, "Menge ", size)
        }
        Close::FlushRight | Close::Apart | Close::Rate => {
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
            Close::FlushRight | Close::Apart | Close::Rate => {
                let end = 300.0 + common::font_width(tf, "Menge", size);
                t += &flush_right(vf, end, y, &row[a]);
                t += &flush_right(vf, x2 + common::font_width(tf, second, size), y, &row[b]);
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
    for close in [Close::OneShow, Close::FlushRight, Close::Apart, Close::Rate] {
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
    assert_eq!(reads, 192);
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
    assert_eq!(reads, 480);
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

/// Guard: lines right under line items without rules that are no wrap of their last row do
/// not join it, in every size, font and weight and with 2 to 5 rows: a note across the gutter
/// between `Pos` and `Artikel`, a line under `Artikel` wider than its column, and a word under
/// `Pos` wider than its numbers. They stay text out of the table, with no loss. A short word under `Artikel` that
/// starts half a size into the column stays out of the table too, and a loss says that it may
/// go on in the last row.
#[test]
fn lines_under_items_without_rules_that_are_no_wrap_stay_paragraphs() {
    let mut wrong = Vec::new();
    let mut reads = 0;
    for k in [0.75, 1.0, 1.5] {
        for fonts in FONTS {
            for bold in [false, true] {
                for (text, x, loss) in [
                    ("Alle Preise in EUR", 40.0, false),
                    ("Zahlbar bis zum 01.03.2026 ohne Abzug", 80.0, false),
                    ("Hinweis", 40.0, false),
                    ("inkl.", 80.0 + 4.0 * k, true),
                ] {
                    for n in 2..=5 {
                        let (t, columns) = unruled_items(k, fonts, bold, n, None, Some((text, x)));
                        let out = common::read(&page_in(&t, Look::Plain));
                        reads += 1;
                        let name = format!("{k}x, {fonts:?}, bold {bold}, {n} rows, {text:?}");
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
                        let losses: Vec<&str> = out
                            .document
                            .losses
                            .iter()
                            .map(|l| l.what.as_str())
                            .collect();
                        let said = losses == [WRAP_LOSS];
                        if said != loss {
                            b.push(format!("losses {losses:?}"));
                        }
                        wrong.extend(b.into_iter().map(|b| format!("{name}: {b}")));
                    }
                }
            }
        }
    }
    assert_eq!(reads, 192);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
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
    for close in [Close::OneShow, Close::FlushRight, Close::Apart, Close::Rate] {
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
    assert_eq!(reads, 1152);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
