// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The scorecard end to end on one tagged corpus file: its tags give facts, the read with
//! tags holds every one of them, and the same file with its tags taken out is read by layout
//! alone and scored against the same facts.

mod common;

use std::path::PathBuf;

use scorecard::check::check;
use scorecard::doc::View;
use scorecard::facts::{Class, parse};
use scorecard::generate::tagged::tagged;
use scorecard::strip::{Stripped, strip_tags};

/// A tagged invoice of the corpus (Apache-2.0, see `tests/corpus/MANIFEST.md`).
const FILE: &str = "mustang-gnuaccounting-invoice-508.pdf";

fn corpus(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn a_tagged_file_gives_its_facts_and_its_tagged_read_holds_them_all() {
    let bytes = corpus(FILE);
    let with_tags = common::read(&bytes).document;
    assert_eq!(with_tags.properties.facts.tagged, Some(true));
    let facts = tagged(&with_tags);
    // Facts of the tables, of the order and of the text.
    for class in [Class::Table, Class::Order, Class::Text] {
        let n = facts.iter().filter(|f| f.class == class).count();
        assert!(n >= 5, "{n} facts of {}", class.name());
    }
    let lines: Vec<String> = facts.iter().map(|f| f.to_string()).collect();
    assert!(
        lines.contains(&"row: 7% | 160.00 € | 11.20 € [table]".to_string()),
        "{lines:#?}"
    );
    // Through the written form, as the scorecard reads an assertion file.
    let written = parse(&lines.join("\n")).unwrap();
    assert_eq!(written.len(), facts.len());
    let view = View::of(&with_tags);
    let failing: Vec<&str> = written
        .iter()
        .filter(|w| !check(&view, &w.fact).pass)
        .map(|w| w.line.as_str())
        .collect();
    assert!(failing.is_empty(), "{failing:#?}");

    // The same pages without tags: read by layout, scored against the same facts.
    let Stripped::Done(stripped) = strip_tags(&bytes).unwrap() else {
        panic!("{FILE} has tags to take out");
    };
    let without_tags = common::read(&stripped).document;
    assert_eq!(without_tags.properties.facts.tagged, Some(false));
    let view = View::of(&without_tags);
    let passed = written
        .iter()
        .filter(|w| check(&view, &w.fact).pass)
        .count();
    // The text of the page is still read: most facts of its text hold. How many of the table
    // facts hold is the score of the layout engine, which this test does not pin.
    let texts: Vec<_> = written
        .iter()
        .filter(|w| w.fact.class == Class::Text)
        .collect();
    let held = texts.iter().filter(|w| check(&view, &w.fact).pass).count();
    assert!(held * 2 > texts.len(), "{held} of {}", texts.len());
    assert!(passed <= written.len());
}
