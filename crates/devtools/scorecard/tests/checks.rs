// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Every kind of fact on small hand-built documents: where it holds, where it does not, and
//! the edges (lines in a cell, spans, one text in two cells). All text is invented.

mod common;

use common::*;
use rezzer_model::Document;
use scorecard::check::{Place, check};
use scorecard::doc::View;
use scorecard::facts::parse;
use serde_json::json;

/// Whether the one fact of `line` holds in `doc`.
fn holds(doc: &Document, line: &str) -> bool {
    outcome(doc, line).0
}

fn outcome(doc: &Document, line: &str) -> (bool, Place) {
    let facts = parse(line).unwrap_or_else(|e| panic!("{line}: {e}"));
    assert_eq!(facts.len(), 1, "{line}");
    let outcome = check(&View::of(doc), &facts[0].fact);
    (outcome.pass, outcome.place)
}

/// Each line with whether it holds.
fn expect(doc: &Document, lines: &[(&str, bool)]) {
    for (line, want) in lines {
        assert_eq!(holds(doc, line), *want, "{line}");
    }
}

fn items() -> Document {
    doc(vec![
        table(
            "p1:t1",
            1,
            &[
                &["Pos", "Artikel", "Menge", "Preis", "Summe"],
                &["1", "Muster A\nblau", "2", "10,00", "20,00"],
                &["2", "Muster B", "2", "1.234,56", "2.469,12"],
                &["", "Summe", "", "", "2.489,12"],
            ],
        ),
        table("p1:t2", 0, &[&["Muster C", "5,00"]]),
    ])
}

#[test]
fn row_texts_stand_in_one_row_in_column_order() {
    expect(
        &items(),
        &[
            ("row: 1 | 2 | 10,00 | 20,00 [table]", true),
            // Other cells may lie between.
            ("row: Pos | Summe [table]", true),
            ("row: Muster B | 2.469,12 [table]", true),
            // One text alone, as a cell.
            ("row: Muster B [table]", true),
            // The column order counts.
            ("row: 20,00 | 10,00 [table]", false),
            ("row: Muster B | 2 | 2 [table]", false),
            // Never over two rows, never over two tables.
            ("row: Muster A blau | 2.469,12 [table]", false),
            ("row: Muster B | 5,00 [table]", false),
            // A text is the whole text of its cell.
            ("row: Muster | 2.469,12 [table]", false),
            ("row: ~Muster | 2.469,12 [table]", true),
            // An amount is compared by value.
            ("row: Muster B | #1234.56 | #2469.12 [table]", true),
            ("row: Muster B | #1234.5 [table]", false),
            // A text that is no cell at all.
            ("row: Fehlt | 2 [table]", false),
        ],
    );
}

#[test]
fn a_cell_keeps_its_lines() {
    let doc = doc(vec![table_of(
        "p1:t1",
        0,
        vec![vec![
            cell("Muster A\nblau"),
            // A break written as text and an empty line.
            json!({ "inlines": [{ "type": "text", "text": "oben<br><br>unten" }],
                    "row_span": 1, "col_span": 1 }),
        ]],
    )]);
    expect(
        &doc,
        &[
            // The whole cell, its lines joined by white space.
            ("row: Muster A blau | oben unten [table]", true),
            ("row: Muster A<br>blau [table]", true),
            // One line of the cell is not the cell.
            ("row: Muster A [table]", false),
            ("row: Muster A@1 | unten@3 [table]", true),
            ("row: blau@2 [table]", true),
            ("row: blau@1 [table]", false),
            ("row: Muster A blau@1 [table]", false),
            ("row: unten@2 [table]", false),
            ("row: unten@9 [table]", false),
            ("row: ~Muster@1 | ~unt@3 [table]", false),
            ("row: ~Muster@1 | ~unten@3 [table]", true),
        ],
    );
}

#[test]
fn one_text_in_two_cells_needs_two_cells() {
    let doc = doc(vec![table(
        "p1:t1",
        0,
        &[&["A", "10,00", "10,00"], &["B", "10,00", "5,00"]],
    )]);
    expect(
        &doc,
        &[
            ("row: A | 10,00 | 10,00 [table]", true),
            ("row: B | 10,00 | 10,00 [table]", false),
            ("same_row: #10 | #10 | A [table]", true),
            ("same_row: #10 | #10 | B [table]", false),
            ("same_row: #5 | B | #10 [table]", true),
        ],
    );
}

#[test]
fn same_row_takes_any_column_order() {
    expect(
        &items(),
        &[
            ("same_row: #20 | ~Muster A | #2 | #10 [table]", true),
            ("same_row: ~Muster B | #2469.12 [table]", true),
            ("same_row: ~Muster A | #2469.12 [table]", false),
            ("same_row: ~Muster C | #2 [table]", false),
        ],
    );
}

#[test]
fn spans_put_a_cell_in_every_row_and_column_it_covers() {
    // | Gruppe (2 rows) | a | 1 |
    // |                 | b | 2 |
    // | Kopf over two columns  | 3 |
    let doc = doc(vec![table_of(
        "p1:t1",
        0,
        vec![
            vec![spanned("Gruppe", 2, 1), cell("a"), cell("1")],
            vec![cell("b"), cell("2")],
            vec![spanned("Kopf", 1, 2), cell("3")],
        ],
    )]);
    expect(
        &doc,
        &[
            ("row: Gruppe | a | 1 [table]", true),
            // The cell that spans down stands in the second row too, left of its cells.
            ("row: Gruppe | b | 2 [table]", true),
            ("row: b | Gruppe [table]", false),
            ("row: Gruppe | 3 [table]", false),
            // `b` is in the second column, under `a`, not in the first.
            ("cell_under: a => b [table]", true),
            ("cell_under: Gruppe => b [table]", false),
            ("cell_under: 1 => 2 [table]", true),
            ("cell_under: 1 => 3 [table]", true),
            // A cell over two columns stands under neither of their headers.
            ("cell_under: a => Kopf [table]", false),
            ("cell_under: Gruppe => Kopf [table]", false),
        ],
    );
}

#[test]
fn not_row_fails_on_a_shared_row_and_on_a_glued_cell() {
    let glued = doc(vec![table("p1:t1", 0, &[&["x", "Muster A Muster B"]])]);
    expect(
        &items(),
        &[
            ("not_row: Muster A | Muster B [table]", true),
            ("not_row: 20,00 | 2.469,12 [table]", true),
            ("not_row: Muster B | 2.469,12 [table]", false),
            // A text that is lost is never a pass.
            ("not_row: Fehlt | Muster B [table]", false),
            ("not_row: Muster B | Fehlt [table]", false),
            // Texts of two tables share no row.
            ("not_row: Muster C | Muster B [table]", true),
        ],
    );
    assert!(!holds(&glued, "not_row: Muster A | Muster B [table]"));
}

#[test]
fn cell_under_goes_by_the_header_row_and_the_column() {
    let unmarked = doc(vec![table(
        "p1:t1",
        0,
        &[&["Name", "Ort"], &["Erika Mustermann", "Musterstadt"]],
    )]);
    let two_heads = doc(vec![table_of(
        "p1:t1",
        2,
        vec![
            vec![spanned("Betrag", 1, 2), cell("Datum")],
            vec![cell("netto"), cell("brutto"), cell("")],
            vec![cell("10,00"), cell("11,90"), cell("01.02.2026")],
        ],
    )]);
    expect(
        &items(),
        &[
            ("cell_under: Preis => 10,00 [table]", true),
            ("cell_under: Preis => #1234.56 [table]", true),
            ("cell_under: Summe => 10,00 [table]", false),
            ("cell_under: Artikel => Muster A blau [table]", true),
            // A cell of the body is no header.
            ("cell_under: Muster B => Summe [table]", false),
            // Not in another table.
            ("cell_under: Preis => 5,00 [table]", false),
        ],
    );
    expect(
        &unmarked,
        &[
            // No header row marked: the first row is one.
            ("cell_under: Ort => Musterstadt [table]", true),
            ("cell_under: Name => Musterstadt [table]", false),
        ],
    );
    expect(
        &two_heads,
        &[
            ("cell_under: Betrag => 10,00 [table]", true),
            ("cell_under: Betrag => 11,90 [table]", true),
            ("cell_under: Betrag => 01.02.2026 [table]", false),
            ("cell_under: brutto => 11,90 [table]", true),
            ("cell_under: netto => 11,90 [table]", false),
            ("cell_under: Datum => 01.02.2026 [table]", true),
            // A header cell of the second header row stands under the first.
            ("cell_under: Betrag => netto [table]", true),
        ],
    );
}

#[test]
fn a_pair_is_a_two_cell_row_or_a_line_of_the_body() {
    let doc = document(
        vec![
            table("p1:t1", 0, &[&["Pers.-Nr.:", "00123"], &["Name", ""]]),
            // A row of four cells in a table read as pairs.
            table(
                "p1:t2",
                0,
                &[
                    &["Eintritt", "01.01.80", "Austritt", "31.12.99"],
                    &["Leer", "", "Kinder", "0,5"],
                ],
            ),
            // The same shape in a plain table is no pair.
            table("p1:t3", 0, &[&["Brutto", "100,00", "Netto", "80,00"]]),
            // Two amounts side by side.
            table("p1:t5", 0, &[&["100,00 EUR", "80,00 EUR"]]),
            // A totals row in a table: a label over two columns and its amount.
            table_of(
                "p1:t4",
                0,
                vec![
                    vec![cell("7%"), cell("160,00"), cell("11,20")],
                    vec![spanned("Summe USt:", 1, 2), cell("75,04 EUR")],
                ],
            ),
            paragraph(
                "p1:b1",
                "Muster GmbH\nUSt-IdNr.: DE000000000\nIBAN DE00 0000 0000",
            ),
            spatial(
                "p1:b2",
                &[&["Gesamt:", "1.234,56"], &["Zahlbar", "30 Tage"]],
                &[],
            ),
            paragraph("p1:b3", "Uhrzeit 10:30"),
        ],
        vec![
            region("p1:t1", "label_value", "rule", "pdf.strips"),
            region("p1:t2", "label_value", "rule", "pdf.strips"),
            region("p1:t3", "table", "rule", "pdf.ruled"),
        ],
        vec![
            field("form[0].Vorname[0]", None, "Erika"),
            field("f2", Some("Nachname:"), "Mustermann"),
        ],
    );
    expect(
        &doc,
        &[
            // A two-cell row; the colon at the end of a label does not count.
            ("pair: Pers.-Nr. == 00123 [pair]", true),
            ("pair: Pers.-Nr.: == 00123 [pair]", true),
            ("pair: 00123 == Pers.-Nr. [pair]", false),
            ("pair: Pers.-Nr. == 0012 [pair]", false),
            // A label with no value is no pair.
            ("pair: Name == 00123 [pair]", false),
            // Side by side in a table read as pairs, by place.
            ("pair: Eintritt == 01.01.80 [pair]", true),
            ("pair: Austritt == 31.12.99 [pair]", true),
            ("pair: 01.01.80 == Austritt [pair]", false),
            ("pair: Kinder == 0,5 [pair]", true),
            ("pair: Leer == Kinder [pair]", false),
            // Not in a table of another kind.
            ("pair: Brutto == 100,00 [pair]", false),
            // Two cells with text in a wider row.
            ("pair: Summe USt == 75,04 EUR [pair]", true),
            ("pair: * == #75.04 [pair]", true),
            ("pair: 7% == 160,00 [pair]", false),
            // Any label is text with a letter and no digit: an amount beside an amount is no
            // pair.
            ("pair: * == #11.2 [pair]", false),
            ("pair: * == #80 [pair]", false),
            // A line `Key: value`, also as one line of a block and in a block kept as laid
            // out.
            ("pair: USt-IdNr. == DE000000000 [pair]", true),
            ("pair: * == DE000000000 [pair]", true),
            ("pair: Gesamt == #1234.56 [pair]", true),
            // No colon, no pair.
            ("pair: IBAN == DE00 0000 0000 [pair]", false),
            ("pair: Zahlbar == 30 Tage [pair]", false),
            // The value is all that follows the colon.
            ("pair: USt-IdNr. == DE [pair]", false),
            ("pair: * == #30 [pair]", false),
            // A form field listed beside the body is no pair: the body must show it.
            ("pair: Vorname == Erika [pair]", false),
            ("pair: Nachname == Mustermann [pair]", false),
            ("pair: f2 == Mustermann [pair]", false),
        ],
    );
}

#[test]
fn a_value_ends_where_the_next_label_starts() {
    let doc = doc(vec![
        paragraph("p1:b1", "Netto: 100,00 Brutto: 119,00"),
        paragraph("p1:b2", "Rabatt: Skonto: 2,50"),
        paragraph("p1:b3", "Name: Erika Mustermann Ort: Musterstadt"),
        paragraph(
            "p1:b4",
            "Menge: 3 Stk Preis: 4,50 EUR Zeit 10:30 Summe: 13,50",
        ),
    ]);
    expect(
        &doc,
        &[
            ("pair: Netto == #100 [pair]", true),
            ("pair: Netto == 100,00 [pair]", true),
            ("pair: Brutto == #119 [pair]", true),
            ("pair: * == #119 [pair]", true),
            // Never the value of the next label.
            ("pair: Netto == #119 [pair]", false),
            ("pair: Netto == ~119,00 [pair]", false),
            ("pair: Netto == ~Brutto [pair]", false),
            ("pair: Netto == 100,00 Brutto: 119,00 [pair]", false),
            // A label with no value takes none from the label after it.
            ("pair: Rabatt == #2.5 [pair]", false),
            ("pair: Rabatt == Skonto [pair]", false),
            ("pair: Skonto == #2.5 [pair]", true),
            // Words alone before another label: where the value ends is not stated, so no
            // part of them is taken as the value or as the label.
            ("pair: Name == Erika [pair]", false),
            ("pair: Name == Erika Mustermann [pair]", false),
            ("pair: Ort == Musterstadt [pair]", false),
            ("pair: Mustermann Ort == Musterstadt [pair]", false),
            // The words after the last digit are the next label.
            ("pair: Menge == #3 [pair]", true),
            ("pair: Menge == #4.5 [pair]", false),
            ("pair: Stk Preis == #4.5 [pair]", true),
            ("pair: Preis == #13.5 [pair]", false),
            // A colon between digits parts nothing: the time is part of the value.
            ("pair: Stk Preis == 4,50 EUR Zeit 10:30 [pair]", true),
            // A contained text stands at the start of the value, an amount is its first.
            ("pair: Stk Preis == ~4,50 EUR [pair]", true),
            ("pair: Stk Preis == ~10:30 [pair]", false),
            ("pair: Stk Preis == #10 [pair]", false),
            ("pair: Summe == #13.5 [pair]", true),
            ("pair: Zeit 10 == #30 [pair]", false),
        ],
    );
}

#[test]
fn a_next_label_without_a_colon_gives_its_value_to_no_other_label() {
    let doc = doc(vec![
        paragraph("p1:b1", "Netto: 100,00 Brutto 119,00"),
        spatial("p1:b2", &[&["Summe:", "1 234,56 EUR", "Rest", "7,00"]], &[]),
    ]);
    expect(
        &doc,
        &[
            ("pair: Netto == #100 [pair]", true),
            ("pair: Netto == #119 [pair]", false),
            ("pair: Netto == ~Brutto [pair]", false),
            ("pair: Netto == ~100,00 [pair]", true),
            ("pair: Netto == ~100,00 Brutto [pair]", true),
            ("pair: Netto == ~119,00 [pair]", false),
            // The whole rest of the line is still the exact value.
            ("pair: Netto == 100,00 Brutto 119,00 [pair]", true),
            ("pair: Netto == 100,00 [pair]", false),
            // The first amount may be groups of three digits.
            ("pair: Summe == #1234.56 [pair]", true),
            ("pair: Summe == #7 [pair]", false),
            ("pair: Summe == ~Rest [pair]", false),
        ],
    );
}

#[test]
fn an_amount_keeps_its_sign_and_reads_as_its_document_reads() {
    let german = doc(vec![table(
        "p1:t1",
        0,
        &[
            &["Gutschrift", "-12,50"],
            &["Menge", "1.234"],
            &["Rest", "5,00-"],
        ],
    )]);
    expect(
        &german,
        &[
            ("pair: Gutschrift == #-12.5 [pair]", true),
            ("pair: Gutschrift == #12.5 [pair]", false),
            ("pair: Rest == #-5 [pair]", true),
            // The document parts decimals with a comma: the point parts thousands.
            ("pair: Menge == #1234 [pair]", true),
            ("pair: Menge == #1.234 [pair]", false),
        ],
    );
    let english = doc(vec![table(
        "p1:t1",
        0,
        &[&["Price", "12.50"], &["Weight", "1.234"]],
    )]);
    expect(
        &english,
        &[
            ("pair: Weight == #1.234 [pair]", true),
            ("pair: Weight == #1234 [pair]", false),
        ],
    );
}

#[test]
fn a_heading_is_a_heading_block_with_this_text() {
    let doc = doc(vec![
        heading("p1:b1", 1, "Entgeltabrechnung"),
        heading("p1:b2", 2, "Bezüge  und\nAbzüge"),
        paragraph("p1:b3", "Kein Titel"),
    ]);
    expect(
        &doc,
        &[
            ("heading: Entgeltabrechnung [heading]", true),
            ("heading: Entgeltabrechnung level 1 [heading]", true),
            ("heading: Entgeltabrechnung level 2 [heading]", false),
            ("heading: Bezüge und Abzüge level 2 [heading]", true),
            ("heading: Bezüge [heading]", false),
            ("heading: Kein Titel [heading]", false),
            ("heading: Fehlt [heading]", false),
        ],
    );
}

#[test]
fn order_goes_by_the_text_of_the_whole_document() {
    let doc = doc(vec![
        paragraph("p1:b1", "Erster Absatz mit Text"),
        table("p1:t1", 0, &[&["Zelle eins", "Zelle zwei"]]),
        paragraph("p1:b2", "Zweiter Absatz, Summe 12,50"),
        paragraph("p1:b3", "Erster Absatz mit Text"),
    ]);
    expect(
        &doc,
        &[
            ("order: Zelle eins << Zelle zwei [order]", true),
            ("order: Zelle zwei << Zelle eins [order]", false),
            ("order: Erster Absatz << Zelle eins [order]", true),
            ("order: Zelle eins << Zweiter Absatz [order]", true),
            // A text that goes on over the end of a block is still text in order.
            ("order: mit Text Zelle << Zweiter [order]", true),
            // A text that stands twice counts by its first place.
            ("order: Zweiter Absatz << Erster Absatz [order]", false),
            ("order: Erster Absatz << Zweiter Absatz [order]", true),
            ("order: Zelle eins << #12.5 [order]", true),
            ("order: #12.5 << Zelle eins [order]", false),
            ("order: Fehlt << Zelle eins [order]", false),
            ("order: Zelle eins << Fehlt [order]", false),
            // Only at word bounds.
            ("order: Zell << Zweiter [order]", false),
        ],
    );
}

#[test]
fn a_text_is_in_one_block_and_rezzers_own_words_are_no_text() {
    let doc = doc(vec![
        json!({ "type": "paragraph", "role": "page",
                "inlines": [{ "type": "status", "text": "Page 1", "color": "grey" }] }),
        json!({ "type": "paragraph", "role": "found-by", "anchor": "p1:b1",
                "inlines": [{ "type": "text", "text": "Found by: Regel" }] }),
        paragraph("p1:b2", "Muster GmbH liefert"),
        paragraph("p1:b3", "die bestellte Ware"),
        table_of(
            "p1:t1",
            0,
            vec![vec![
                cell("in der Zelle"),
                json!({ "inlines": [{ "type": "status", "text": "empty", "color": "grey" }],
                        "row_span": 1, "col_span": 1 }),
            ]],
        ),
        json!({ "type": "list", "ordered": false, "anchor": "p1:b4",
                "items": [[paragraph("p1:b5", "erster Punkt")]] }),
        spatial("p1:b6", &[&["links", "rechts"], &["unten"]], &[]),
    ]);
    expect(
        &doc,
        &[
            ("text: Muster GmbH liefert [text]", true),
            ("text: GmbH [text]", true),
            ("text: in der Zelle [text]", true),
            ("text: erster Punkt [text]", true),
            // Lines of a block kept as laid out are one block.
            ("text: rechts unten [text]", true),
            // Split over two blocks.
            ("text: liefert die bestellte [text]", false),
            // Words Rezzer adds are not text of the file.
            ("text: Page 1 [text]", false),
            ("text: Found by [text]", false),
            ("text: empty [text]", false),
            ("row: in der Zelle | empty [table]", false),
        ],
    );
}

#[test]
fn a_link_has_exactly_this_target() {
    let doc = doc(vec![
        json!({ "type": "paragraph", "anchor": "p1:b1", "inlines": [
            { "type": "text", "text": "siehe " },
            { "type": "link", "target": "https://example.org/a", "internal": false,
              "inlines": [{ "type": "text", "text": "hier" }] }
        ] }),
        spatial("p1:b2", &[&["Kontakt"]], &["mailto:info@example.org"]),
    ]);
    expect(
        &doc,
        &[
            ("link: https://example.org/a [link]", true),
            ("link: mailto:info@example.org [link]", true),
            ("link: https://example.org/ [link]", false),
            ("link: https://example.org/a/b [link]", false),
            // The text of a link is text of its block.
            ("text: siehe hier [text]", true),
        ],
    );
}

#[test]
fn a_fact_stands_in_the_region_of_its_first_text_the_document_holds() {
    let doc = document(
        vec![
            table("p1:t1", 0, &[&["Brutto", "100,00"]]),
            json!({ "type": "list", "ordered": false, "anchor": "p1:b1",
                    "items": [[paragraph("p1:b2", "erster Punkt")]] }),
            paragraph("p1:b3", "ohne Region"),
            spatial("p1:b4", &[&["wie", "gesetzt"]], &["https://example.org/"]),
        ],
        vec![
            region("p1:t1", "label_value", "rule", "pdf.strips"),
            // The list has the region, not the paragraph of its item.
            region("p1:b1", "list", "exact", "given.file_states"),
            region("p1:b4", "spatial", "unsure", "pdf.spatial"),
        ],
        Vec::new(),
    );
    let place = |line: &str| outcome(&doc, line).1.name();
    assert_eq!(place("pair: Brutto == 100,00 [pair]"), "label_value");
    // The first text is nowhere: the second decides.
    assert_eq!(place("row: Fehlt | 100,00 [table]"), "label_value");
    assert_eq!(place("text: erster Punkt [text]"), "list");
    assert_eq!(place("text: ohne Region [text]"), "(no region)");
    assert_eq!(place("text: wie gesetzt [text]"), "spatial");
    assert_eq!(place("link: https://example.org/ [link]"), "spatial");
    assert_eq!(
        place("link: https://example.org/fehlt [link]"),
        "(not found)"
    );
    assert_eq!(place("heading: Fehlt [heading]"), "(not found)");
    assert_eq!(place("pair: * == Fehlt [pair]"), "(not found)");
}

#[test]
fn a_fact_written_twice_counts_once() {
    use scorecard::facts::Class;
    use scorecard::report::score_file;
    use scorecard::split::Half;
    let doc = doc(vec![table("p1:t1", 0, &[&["Brutto", "100,00"]])]);
    // The same fact three times (once written another way), and one that fails twice.
    let facts = parse(
        "pair: Brutto == 100,00 [pair]\npair: Brutto == 100,00 [pair]\n\
         pair:  Brutto  ==  100,00 [pair]\ntext: Fehlt [text]\ntext: Fehlt [text]\n",
    )
    .unwrap();
    let score = score_file("muster.pdf.facts", 1, Half::Dev, &facts, &doc);
    assert_eq!(score.classes[&Class::Pair].total, 1);
    assert_eq!(score.classes[&Class::Pair].passed, 1);
    assert_eq!(score.classes[&Class::Text].total, 1);
    assert_eq!(score.failing.len(), 1);
}
