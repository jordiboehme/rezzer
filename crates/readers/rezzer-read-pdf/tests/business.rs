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
