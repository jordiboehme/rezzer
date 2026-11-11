// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The generators of truth on hand-built documents: the facts they write, that the read they
//! were made from holds them all, and that a worse read of the same file does not. All text
//! is invented.

mod common;

use common::*;
use rezzer_model::Document;
use scorecard::check::check;
use scorecard::doc::View;
use scorecard::facts::{Fact, parse};
use scorecard::generate::{fields::fields, invoice::invoice, tagged::tagged};
use scorecard::strip::{Stripped, strip_tags};

fn lines(facts: &[Fact]) -> Vec<String> {
    facts.iter().map(Fact::to_string).collect()
}

/// How many of `facts` hold in `doc`, after a round through their written form.
fn passed(doc: &Document, facts: &[Fact]) -> usize {
    let view = View::of(doc);
    let written = lines(facts).join("\n");
    parse(&written)
        .unwrap()
        .iter()
        .filter(|w| check(&view, &w.fact).pass)
        .count()
}

const STATED: &str = "given.file_states";

/// A tagged read: a heading, two paragraphs, a table with a header row and a paragraph the
/// reader found by layout.
fn tagged_read() -> Document {
    document(
        vec![
            heading("p1:b1", 1, "Entgeltabrechnung Oktober"),
            paragraph("p1:b2", "Muster GmbH zahlt das Entgelt zum Monatsende aus"),
            table(
                "p1:t1",
                1,
                &[
                    &["Lohnart", "Betrag"],
                    &["Grundgehalt", "3.000,00"],
                    &["Fahrgeld", "50,00"],
                    &["", "nur eine Zelle"],
                ],
            ),
            paragraph("p1:b3", "Der Betrag wird auf das Konto überwiesen"),
            paragraph("p1:b4", "Vom Leser gefunden und nicht vom Tag"),
            table("p1:t2", 0, &[&["nicht", "getaggt"]]),
            serde_json::json!({ "type": "paragraph", "anchor": "p1:b5", "inlines": [
                { "type": "link", "target": "https://example.org/hilfe", "internal": false,
                  "inlines": [{ "type": "text", "text": "Hilfe" }] }
            ] }),
        ],
        vec![
            region("p1:b1", "heading", "exact", STATED),
            region("p1:b2", "paragraph", "exact", STATED),
            region("p1:t1", "table", "exact", STATED),
            region("p1:b3", "paragraph", "exact", STATED),
            region("p1:b4", "paragraph", "rule", "paragraph.text_size"),
            region("p1:t2", "table", "rule", "pdf.ruled"),
        ],
        Vec::new(),
    )
}

#[test]
fn tags_give_rows_headings_order_and_text() {
    let doc = tagged_read();
    let facts = tagged(&doc);
    assert_eq!(
        lines(&facts),
        [
            "row: Lohnart | Betrag [table]",
            "row: Grundgehalt | 3.000,00 [table]",
            "row: Fahrgeld | 50,00 [table]",
            "not_row: Lohnart | 3.000,00 [table]",
            "not_row: Grundgehalt | 50,00 [table]",
            "cell_under: Lohnart => Grundgehalt [table]",
            "cell_under: Betrag => 3.000,00 [table]",
            "cell_under: Lohnart => Fahrgeld [table]",
            "cell_under: Betrag => 50,00 [table]",
            "cell_under: Betrag => nur eine Zelle [table]",
            "heading: Entgeltabrechnung Oktober [heading]",
            "order: ~Entgeltabrechnung Oktober << ~Muster GmbH zahlt das Entgelt zum [order]",
            "order: ~Muster GmbH zahlt das Entgelt zum << ~Der Betrag wird auf das Konto [order]",
            "text: ~Muster GmbH zahlt das Entgelt zum Monatsende aus [text]",
            "text: ~Der Betrag wird auf das Konto überwiesen [text]",
            "link: https://example.org/hilfe [link]",
        ]
    );
    // The read the facts were made from holds every one of them.
    assert_eq!(passed(&doc, &facts), facts.len());
}

#[test]
fn a_file_without_tags_and_blocks_found_by_layout_give_no_facts() {
    let mut doc = tagged_read();
    doc.properties.facts.tagged = Some(false);
    assert!(tagged(&doc).is_empty());
    doc.properties.facts.tagged = None;
    assert!(tagged(&doc).is_empty());
    // Nothing of the untagged table or the paragraph found by layout.
    let all = lines(&tagged(&tagged_read())).join("\n");
    assert!(
        !all.contains("getaggt") && !all.contains("Vom Leser"),
        "{all}"
    );
}

#[test]
fn a_worse_read_of_the_same_file_fails_facts_of_its_tags() {
    let facts = tagged(&tagged_read());
    // The read without tags: the heading is a paragraph, the table lost a row to the row
    // above, and the two paragraphs changed places.
    let worse = doc(vec![
        paragraph("p1:b1", "Entgeltabrechnung Oktober"),
        paragraph("p1:b2", "Der Betrag wird auf das Konto überwiesen"),
        table(
            "p1:t1",
            0,
            &[
                &["Lohnart", "Betrag"],
                &["Grundgehalt\nFahrgeld", "3.000,00\n50,00"],
            ],
        ),
        paragraph("p1:b3", "Muster GmbH zahlt das Entgelt zum Monatsende aus"),
    ]);
    let view = View::of(&worse);
    let failing: Vec<String> = facts
        .iter()
        .filter(|f| !check(&view, f).pass)
        .map(Fact::to_string)
        .collect();
    assert_eq!(
        failing,
        [
            "row: Grundgehalt | 3.000,00 [table]",
            "row: Fahrgeld | 50,00 [table]",
            "not_row: Grundgehalt | 50,00 [table]",
            "cell_under: Lohnart => Grundgehalt [table]",
            "cell_under: Betrag => 3.000,00 [table]",
            "cell_under: Lohnart => Fahrgeld [table]",
            "cell_under: Betrag => 50,00 [table]",
            "cell_under: Betrag => nur eine Zelle [table]",
            "heading: Entgeltabrechnung Oktober [heading]",
            "order: ~Muster GmbH zahlt das Entgelt zum << ~Der Betrag wird auf das Konto [order]",
            "link: https://example.org/hilfe [link]",
        ]
    );
}

#[test]
fn a_cell_that_spans_rows_gives_no_fact_about_itself() {
    // A header cell that spans down into the body, and a cell of the body that spans two rows.
    let doc = document(
        vec![table_of(
            "p1:t1",
            1,
            vec![
                vec![spanned("Hinweis", 3, 1), cell("Lohnart"), cell("Betrag")],
                vec![cell("Grundgehalt"), spanned("3.000,00", 2, 1)],
                vec![cell("Fahrgeld")],
            ],
        )],
        vec![region("p1:t1", "table", "exact", STATED)],
        Vec::new(),
    );
    let facts = tagged(&doc);
    let all = lines(&facts);
    assert!(
        !all.iter().any(|l| l.contains("Hinweis => Hinweis")),
        "{all:#?}"
    );
    assert!(
        !all.iter().any(|l| l.starts_with("not_row: Hinweis")),
        "{all:#?}"
    );
    assert!(
        all.contains(&"cell_under: Betrag => 3.000,00 [table]".to_string()),
        "{all:#?}"
    );
    assert!(
        all.contains(&"row: Hinweis | Fahrgeld | 3.000,00 [table]".to_string()),
        "{all:#?}"
    );
    assert_eq!(passed(&doc, &facts), facts.len());
}

#[test]
fn a_long_file_gives_at_most_the_cap_of_each_kind() {
    let rows: Vec<Vec<String>> = (0..500)
        .map(|n| vec![format!("Zeile {n}"), format!("Wert {n}")])
        .collect();
    let rows: Vec<Vec<&str>> = rows
        .iter()
        .map(|r| r.iter().map(String::as_str).collect())
        .collect();
    let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
    let doc = document(
        vec![table("p1:t1", 0, &rows)],
        vec![region("p1:t1", "table", "exact", STATED)],
        Vec::new(),
    );
    let facts = tagged(&doc);
    let of = |kind: &str| {
        facts
            .iter()
            .filter(|f| f.to_string().starts_with(kind))
            .count()
    };
    assert_eq!(of("row:"), 200);
    assert_eq!(of("not_row:"), 200);
    assert_eq!(passed(&doc, &facts), facts.len());
}

const INVOICE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rsm:CrossIndustryInvoice xmlns:rsm="urn:a" xmlns:ram="urn:b">
  <rsm:ExchangedDocument><ram:ID>RE-2026-0001</ram:ID></rsm:ExchangedDocument>
  <rsm:SupplyChainTradeTransaction>
    <ram:IncludedSupplyChainTradeLineItem>
      <ram:SpecifiedTradeProduct><ram:Name>Beratung &amp; Planung</ram:Name></ram:SpecifiedTradeProduct>
      <ram:SpecifiedLineTradeAgreement>
        <ram:NetPriceProductTradePrice><ram:ChargeAmount>80.0000</ram:ChargeAmount></ram:NetPriceProductTradePrice>
      </ram:SpecifiedLineTradeAgreement>
      <ram:SpecifiedLineTradeDelivery><ram:BilledQuantity unitCode="HUR">12.5000</ram:BilledQuantity></ram:SpecifiedLineTradeDelivery>
      <ram:SpecifiedLineTradeSettlement>
        <ram:SpecifiedTradeSettlementLineMonetarySummation><ram:LineTotalAmount>1000.00</ram:LineTotalAmount></ram:SpecifiedTradeSettlementLineMonetarySummation>
      </ram:SpecifiedLineTradeSettlement>
    </ram:IncludedSupplyChainTradeLineItem>
    <ram:IncludedSupplyChainTradeLineItem>
      <ram:SpecifiedTradeProduct><ram:Name>Nicht gedruckter Name</ram:Name></ram:SpecifiedTradeProduct>
      <ram:SpecifiedLineTradeAgreement>
        <ram:NetPriceProductTradePrice><ram:ChargeAmount>234.56</ram:ChargeAmount></ram:NetPriceProductTradePrice>
      </ram:SpecifiedLineTradeAgreement>
      <ram:SpecifiedLineTradeDelivery><ram:BilledQuantity unitCode="C62">1</ram:BilledQuantity></ram:SpecifiedLineTradeDelivery>
      <ram:SpecifiedLineTradeSettlement>
        <ram:SpecifiedTradeSettlementLineMonetarySummation><ram:LineTotalAmount>234.56</ram:LineTotalAmount></ram:SpecifiedTradeSettlementLineMonetarySummation>
      </ram:SpecifiedLineTradeSettlement>
    </ram:IncludedSupplyChainTradeLineItem>
    <ram:ApplicableHeaderTradeAgreement>
      <ram:SellerTradeParty><ram:Name>Muster GmbH</ram:Name></ram:SellerTradeParty>
      <ram:BuyerTradeParty><ram:Name>Erika Mustermann</ram:Name></ram:BuyerTradeParty>
    </ram:ApplicableHeaderTradeAgreement>
    <ram:ApplicableHeaderTradeSettlement>
      <ram:SpecifiedTradeSettlementHeaderMonetarySummation>
        <ram:LineTotalAmount>1234.56</ram:LineTotalAmount>
        <ram:ChargeTotalAmount>0.00</ram:ChargeTotalAmount>
        <ram:TaxBasisTotalAmount>1234.56</ram:TaxBasisTotalAmount>
        <ram:TaxTotalAmount currencyID="EUR">234.57</ram:TaxTotalAmount>
        <ram:GrandTotalAmount>1469.13</ram:GrandTotalAmount>
        <ram:TotalPrepaidAmount>0.00</ram:TotalPrepaidAmount>
        <ram:DuePayableAmount>1469.13</ram:DuePayableAmount>
      </ram:SpecifiedTradeSettlementHeaderMonetarySummation>
    </ram:ApplicableHeaderTradeSettlement>
  </rsm:SupplyChainTradeTransaction>
</rsm:CrossIndustryInvoice>"#;

/// The page of [`INVOICE`]: quantity before the name, German amounts, totals as pairs. The
/// tax total is not printed.
fn invoice_read() -> Document {
    doc(vec![
        paragraph("p1:b1", "Muster GmbH\nMusterweg 1\n12345 Musterstadt"),
        paragraph("p1:b2", "Erika Mustermann"),
        paragraph("p1:b3", "Rechnung RE-2026-0001 vom 01.02.2026"),
        table(
            "p1:t1",
            1,
            &[
                &["Menge", "Leistung", "Einzelpreis", "Gesamt"],
                &[
                    "12,5 Std",
                    "Beratung & Planung\nvor Ort",
                    "80,00 €",
                    "1.000,00 €",
                ],
                &["1", "Sonstiges", "234,56 €", "234,56 €"],
            ],
        ),
        table("p1:t2", 0, &[&["Summe netto", "1.234,56 €"]]),
        paragraph("p1:b4", "Gesamtbetrag: 1.469,13 €"),
    ])
}

#[test]
fn invoice_data_gives_item_rows_totals_and_names() {
    let doc = invoice_read();
    let facts = invoice(INVOICE, &doc).unwrap();
    assert_eq!(
        lines(&facts),
        [
            "same_row: ~Beratung & Planung | #12.5 | #80 | #1000 [table]",
            // The name is not printed: the amounts alone, held together by the line total.
            "same_row: #1 | #234.56 | #234.56 [table]",
            // A total is a text fact: the amount is printed. Nothing says with which label.
            "text: #1234.56 [text]",
            "text: #1469.13 [text]",
            "text: ~Muster GmbH [text]",
            "text: ~Erika Mustermann [text]",
            "text: ~RE-2026-0001 [text]",
        ]
    );
    assert_eq!(passed(&doc, &facts), facts.len());
}

#[test]
fn an_item_read_over_two_rows_fails_its_fact() {
    let facts = invoice(INVOICE, &invoice_read()).unwrap();
    // The name in a row of its own, and the totals as loose text.
    let worse = doc(vec![
        paragraph("p1:b1", "Muster GmbH Erika Mustermann RE-2026-0001"),
        table(
            "p1:t1",
            0,
            &[
                &["", "Beratung & Planung", "", ""],
                &["12,5 Std", "vor Ort", "80,00 €", "1.000,00 €"],
                &["1", "Sonstiges", "234,56 €", "234,56 €"],
            ],
        ),
        spatial(
            "p1:b2",
            &[
                &["Summe netto", "1.234,56 €"],
                &["Gesamtbetrag", "1.469,13 €"],
            ],
            &[],
        ),
    ]);
    let view = View::of(&worse);
    let failing: Vec<String> = facts
        .iter()
        .filter(|f| !check(&view, f).pass)
        .map(Fact::to_string)
        .collect();
    // The totals are still printed, so their text facts hold: invoice truth does not see
    // that they are no pairs here.
    assert_eq!(
        failing,
        ["same_row: ~Beratung & Planung | #12.5 | #80 | #1000 [table]"]
    );
}

#[test]
fn the_older_invoice_format_gives_its_totals_too() {
    let xml = r#"<rsm:CrossIndustryDocument xmlns:rsm="urn:a" xmlns:ram="urn:b">
      <rsm:HeaderExchangedDocument><ram:ID>R-7</ram:ID></rsm:HeaderExchangedDocument>
      <rsm:SpecifiedSupplyChainTradeTransaction>
        <ram:ApplicableSupplyChainTradeSettlement>
          <ram:SpecifiedTradeSettlementMonetarySummation>
            <ram:GrandTotalAmount currencyID="EUR">119.00</ram:GrandTotalAmount>
          </ram:SpecifiedTradeSettlementMonetarySummation>
        </ram:ApplicableSupplyChainTradeSettlement>
        <ram:IncludedSupplyChainTradeLineItem>
          <ram:SpecifiedSupplyChainTradeAgreement>
            <ram:NetPriceProductTradePrice><ram:ChargeAmount>50.00</ram:ChargeAmount></ram:NetPriceProductTradePrice>
          </ram:SpecifiedSupplyChainTradeAgreement>
          <ram:SpecifiedSupplyChainTradeDelivery><ram:BilledQuantity>2.0000</ram:BilledQuantity></ram:SpecifiedSupplyChainTradeDelivery>
          <ram:SpecifiedSupplyChainTradeSettlement>
            <ram:SpecifiedTradeSettlementMonetarySummation><ram:LineTotalAmount>100.00</ram:LineTotalAmount></ram:SpecifiedTradeSettlementMonetarySummation>
          </ram:SpecifiedSupplyChainTradeSettlement>
          <ram:SpecifiedTradeProduct><ram:Name>Muster</ram:Name></ram:SpecifiedTradeProduct>
        </ram:IncludedSupplyChainTradeLineItem>
      </rsm:SpecifiedSupplyChainTradeTransaction>
    </rsm:CrossIndustryDocument>"#;
    let doc = doc(vec![
        paragraph("p1:b1", "Rechnung R-7"),
        table("p1:t1", 0, &[&["Muster", "2", "50,00", "100,00"]]),
        paragraph("p1:b2", "Summe: 119,00"),
    ]);
    assert_eq!(
        lines(&invoice(xml, &doc).unwrap()),
        [
            "same_row: ~Muster | #2 | #50 | #100 [table]",
            "text: #119 [text]",
            "text: ~R-7 [text]",
        ]
    );
}

#[test]
fn a_filled_field_gives_a_pair_only_when_the_body_shows_it() {
    let mut check_box = field("Zustimmung", None, "Yes");
    check_box["kind"] = serde_json::json!("checkbox");
    let doc = document(
        vec![
            paragraph("p1:b1", "Vorname: Erika"),
            table("p1:t1", 0, &[&["Geburtsort", "Musterstadt"]]),
            // The label is printed, the value stands only in the list of fields.
            paragraph("p1:b2", "Nachname"),
            // Label and value are printed, but not as a pair.
            paragraph("p1:b3", "Strasse"),
            paragraph("p1:b4", "Musterweg 1"),
            paragraph("p1:b5", "Zustimmung: Yes"),
        ],
        Vec::new(),
        vec![
            field("form[0].Vorname[0]", None, "Erika"),
            field("f_02", Some("Geburtsort:"), "Musterstadt"),
            field("f_03", Some("Nachname"), "Mustermann"),
            field("Strasse", None, "Musterweg 1"),
            // Empty, a check box, and the same pair again.
            field("Vorname", None, ""),
            check_box,
            field("kopie.Vorname", None, "Erika"),
        ],
    );
    let facts = fields(&doc);
    assert_eq!(
        lines(&facts),
        [
            "pair: Vorname == Erika [pair]",
            "pair: Geburtsort == Musterstadt [pair]"
        ]
    );
    assert_eq!(passed(&doc, &facts), 2);
}

#[test]
fn a_fact_is_written_once_per_file() {
    // The same row twice, and one value under one header in every row.
    let doc = document(
        vec![table(
            "p1:t1",
            1,
            &[
                &["Datum", "Art"],
                &["01.02.2026", "Muster"],
                &["01.02.2026", "Muster"],
                &["02.02.2026", "Muster"],
            ],
        )],
        vec![region("p1:t1", "table", "exact", STATED)],
        Vec::new(),
    );
    let all = lines(&tagged(&doc));
    let mut once = all.clone();
    once.sort();
    once.dedup();
    assert_eq!(once.len(), all.len(), "{all:#?}");
    assert_eq!(
        all.iter()
            .filter(|l| *l == "row: 01.02.2026 | Muster [table]")
            .count(),
        1
    );
    // A value that repeats in its table gives no `cell_under` fact: it would hold under a
    // wrong header of a shifted read too. The date that stands once gives one.
    assert!(
        !all.iter().any(|l| l.ends_with("=> Muster [table]")),
        "{all:#?}"
    );
    assert!(
        !all.iter().any(|l| l.ends_with("=> 01.02.2026 [table]")),
        "{all:#?}"
    );
    assert!(
        all.contains(&"cell_under: Datum => 02.02.2026 [table]".to_string()),
        "{all:#?}"
    );
}

#[test]
fn a_tagged_pdf_loses_its_tags_and_nothing_else() {
    use lopdf::{Document as Pdf, Object, dictionary};
    let mut pdf = Pdf::with_version("1.7");
    let pages = pdf.new_object_id();
    let page = pdf.add_object(dictionary! { "Type" => "Page", "Parent" => pages });
    pdf.objects.insert(
        pages,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
        }),
    );
    let tree = pdf.add_object(dictionary! { "Type" => "StructTreeRoot" });
    let catalog = pdf.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages,
        "StructTreeRoot" => tree,
        "MarkInfo" => dictionary! { "Marked" => true },
        "Lang" => Object::string_literal("de-DE"),
    });
    pdf.trailer.set("Root", catalog);
    let mut tagged = Vec::new();
    pdf.save_to(&mut tagged).unwrap();

    let Stripped::Done(bytes) = strip_tags(&tagged).unwrap() else {
        panic!("the file is tagged");
    };
    let read = Pdf::load_mem(&bytes).unwrap();
    let catalog = read.catalog().unwrap();
    assert!(!catalog.has(b"StructTreeRoot") && !catalog.has(b"MarkInfo"));
    assert!(catalog.has(b"Lang") && catalog.has(b"Pages"));
    assert_eq!(read.get_pages().len(), 1);
    // Nothing left to take out.
    assert_eq!(strip_tags(&bytes).unwrap(), Stripped::NotTagged);
    assert!(strip_tags(b"no pdf").is_err());
}
