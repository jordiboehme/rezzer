// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Facts from the invoice data a PDF carries as XML (ZUGFeRD and Factur-X, the Cross
//! Industry Invoice). The XML states each item line with its name, quantity, price and line
//! total, the totals of the invoice and its parties. It does not state how the page prints
//! them, so:
//!
//! - an item line is a `same_row` fact (the order of the printed columns is not known) of
//!   the name and the amounts, each amount compared by value;
//! - a total is a `text` fact of its amount: the amount is printed. The XML states no printed
//!   label, so no fact says which label the amount stands with;
//! - the names of the seller and the buyer and the invoice number are `text` facts: the page
//!   prints them in an address block or a sentence as often as beside a label.
//!
//! So invoice truth checks that the data is present and that an item line is one row. It
//! does not check the pairing of a total with its label.
//!
//! Only what the page prints at all becomes a fact: a name or an amount that stands nowhere
//! in the text of the read is left out, so that no fact asks for data the page does not show.

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use rezzer_model::Document;

use super::{CAP, distinct, sample};
use crate::doc::View;
use crate::facts::{Fact, Term};
use crate::text::{Number, has_number, has_words, norm};

/// The names of the attachment that holds the invoice, in lower case.
const NAMES: [&str; 3] = ["factur-x.xml", "zugferd-invoice.xml", "xrechnung.xml"];

/// Bounds of the XML that is read.
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 200_000;

/// The invoice XML attached to the PDF `bytes`, when it has one.
pub fn invoice_xml(bytes: &[u8]) -> Result<Option<String>, String> {
    let pdf = pdfparts::Pdf::open(bytes, None, &pdfparts::Limits::default(), &|| false)
        .map_err(|e| format!("it does not open: {e}"))?;
    let Some(index) = pdf
        .embedded_files()
        .iter()
        .position(|f| NAMES.contains(&f.name.to_lowercase().as_str()))
    else {
        return Ok(None);
    };
    let data = pdf
        .embedded_file_bytes(index)
        .map_err(|issue| format!("its invoice XML is not read: {}", issue.what))?;
    String::from_utf8(data)
        .map(Some)
        .map_err(|_| "its invoice XML is not UTF-8".to_string())
}

/// One element: its local name, its own text and its child elements.
#[derive(Debug, Default)]
struct Node {
    name: String,
    text: String,
    children: Vec<Node>,
}

impl Node {
    /// The first element named `name` below this one, in document order.
    fn find(&self, name: &str) -> Option<&Node> {
        self.children.iter().find_map(|c| {
            if c.name == name {
                Some(c)
            } else {
                c.find(name)
            }
        })
    }

    /// The element reached by finding each of `names` below the one before.
    fn path(&self, names: &[&str]) -> Option<&Node> {
        names.iter().try_fold(self, |node, name| node.find(name))
    }

    /// Every element named `name` below this one.
    fn all<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            } else {
                c.all(name, out);
            }
        }
    }

    /// The normalised text of the element at `names`, when it has text.
    fn text_at(&self, names: &[&str]) -> Option<String> {
        self.path(names)
            .map(|n| norm(&n.text))
            .filter(|t| !t.is_empty())
    }
}

/// The predefined entities of XML.
fn predefined(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

/// The elements of `xml` as a tree under an unnamed root. A DOCTYPE, a tree too deep or too
/// large and XML that is not well formed are errors.
fn tree(xml: &str) -> Result<Node, String> {
    let mut reader = Reader::from_str(xml);
    let mut stack = vec![Node::default()];
    let mut nodes = 0usize;
    let close = |stack: &mut Vec<Node>| {
        if let Some(done) = stack.pop()
            && let Some(parent) = stack.last_mut()
        {
            parent.children.push(done);
        }
    };
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("the XML is not read: {e}"))?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                nodes += 1;
                if stack.len() > MAX_DEPTH || nodes > MAX_NODES {
                    return Err("the XML is too deep or too large".into());
                }
                stack.push(Node {
                    name: start.local_name().as_ref().to_string(),
                    ..Node::default()
                });
                if matches!(event, Event::Empty(_)) {
                    close(&mut stack);
                }
            }
            Event::End(_) => {
                if stack.len() < 2 {
                    return Err("the XML closes an element it did not open".into());
                }
                close(&mut stack);
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.xml_content(XmlVersion::Implicit1_0));
                }
            }
            Event::CData(c) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&c);
                }
            }
            Event::GeneralRef(r) => {
                let c = if r.is_char_ref() {
                    r.resolve_char_ref().ok().flatten()
                } else {
                    predefined(&r)
                };
                if let (Some(c), Some(top)) = (c, stack.last_mut()) {
                    top.text.push(c);
                }
            }
            Event::DocType(_) => return Err("the XML has a DOCTYPE".into()),
            Event::Eof => break,
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
    }
    if stack.len() != 1 {
        return Err("the XML ends inside an element".into());
    }
    stack.pop().ok_or_else(|| "the XML is empty".to_string())
}

/// The facts the invoice `xml` gives for the read `doc` of its PDF.
pub fn invoice(xml: &str, doc: &Document) -> Result<Vec<Fact>, String> {
    let root = tree(xml)?;
    let view = View::of(doc);
    let printed = |text: &str| has_words(&view.stream, text);
    let amount = |node: Option<&Node>| {
        node.and_then(|n| Number::parse(&n.text))
            .filter(|n| has_number(&view.stream, n, view.decimal))
    };

    let mut items = Vec::new();
    root.all("IncludedSupplyChainTradeLineItem", &mut items);
    let mut rows = Vec::new();
    for item in items {
        let name = item
            .text_at(&["SpecifiedTradeProduct", "Name"])
            .filter(|n| printed(n));
        let price = item
            .path(&["NetPriceProductTradePrice", "ChargeAmount"])
            .or_else(|| item.path(&["GrossPriceProductTradePrice", "ChargeAmount"]));
        let total = amount(item.find("LineTotalAmount"));
        let mut terms: Vec<Term> = name.iter().map(|n| Term::contains(n)).collect();
        let anchored = name.is_some() || total.is_some();
        for n in [amount(item.find("BilledQuantity")), amount(price), total]
            .into_iter()
            .flatten()
        {
            terms.push(Term::number(n));
        }
        if anchored && terms.len() >= 2 {
            rows.push(Fact::same_row(terms));
        }
    }

    // The totals of the invoice: Factur-X and ZUGFeRD 2 name the element one way, ZUGFeRD 1
    // another, where the same name also stands in each item line.
    let sums = root
        .find("SpecifiedTradeSettlementHeaderMonetarySummation")
        .or_else(|| {
            root.path(&[
                "ApplicableSupplyChainTradeSettlement",
                "SpecifiedTradeSettlementMonetarySummation",
            ])
        });
    let mut totals: Vec<Number> = Vec::new();
    if let Some(sums) = sums {
        for name in [
            "LineTotalAmount",
            "TaxBasisTotalAmount",
            "TaxTotalAmount",
            "GrandTotalAmount",
            "DuePayableAmount",
        ] {
            let child = sums.children.iter().find(|c| c.name == name);
            if let Some(n) = amount(child)
                && !n.is_zero()
                && !totals.contains(&n)
            {
                totals.push(n);
            }
        }
    }

    let mut names: Vec<String> = Vec::new();
    for path in [
        &["SellerTradeParty", "Name"][..],
        &["BuyerTradeParty", "Name"],
        &["ExchangedDocument", "ID"],
        &["HeaderExchangedDocument", "ID"],
    ] {
        if let Some(text) = root.text_at(path)
            && printed(&text)
            && !names.contains(&text)
        {
            names.push(text);
        }
    }

    let mut out = sample(rows, CAP);
    out.extend(totals.into_iter().map(|n| Fact::text(Term::number(n))));
    out.extend(names.iter().map(|n| Fact::text(Term::contains(n))));
    Ok(distinct(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_keeps_names_text_and_entities() {
        let root = tree(
            "<?xml version='1.0'?><!-- c --><a:r xmlns:a='x'><a:n>A &amp; B&#32;C</a:n>\
             <e/><n><![CDATA[<d>]]></n></a:r>",
        )
        .unwrap();
        let r = &root.children[0];
        assert_eq!(r.name, "r");
        assert_eq!(r.find("n").unwrap().text, "A & B C");
        let mut all = Vec::new();
        r.all("n", &mut all);
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].text, "<d>");
        assert!(r.find("e").unwrap().children.is_empty());
    }

    #[test]
    fn xml_that_is_not_sound_is_refused() {
        assert!(tree("<!DOCTYPE a><a/>").unwrap_err().contains("DOCTYPE"));
        assert!(tree("<a><b></a>").is_err());
        assert!(tree("<a>").unwrap_err().contains("ends inside"));
        let deep = "<a>".repeat(MAX_DEPTH + 2);
        assert!(tree(&deep).unwrap_err().contains("too deep"));
    }
}
