// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Data lines, address blocks and padding that are no headings or bullets.

use super::*;

/// Template labels of 6 pt, at least `ink` ink characters, from `top` down at 40.
pub(crate) fn label_mass(ink: usize, top: f32) -> String {
    const LABEL: &str = "Angaben zur Person und zum Arbeitgeber laut Vertrag";
    let mut out = String::new();
    let (mut n, mut y) = (0, top);
    while n < ink {
        out.push_str(&label(40.0, y, 6.0, LABEL));
        n += LABEL.chars().filter(|c| !c.is_whitespace()).count();
        y -= 8.0;
    }
    out
}

/// Data lines of the print layer, each padded into its place, about 300 ink characters from
/// `top` down. No two start at one x, so they form no columns.
pub(crate) fn padded_data_lines(top: f32) -> Vec<DataLine> {
    (0..7)
        .map(|k| DataLine {
            y: top - 12.0 * k as f32,
            text: format!(
                "{}Muster{k} Kostenstelle Abteilung Musterort Nord",
                " ".repeat(4 * k)
            ),
        })
        .collect()
}

pub(crate) fn headings_of(out: &ReadOutcome) -> Vec<(u8, String)> {
    fn walk(blocks: &[Block], found: &mut Vec<(u8, String)>) {
        for b in blocks {
            match b {
                Block::Heading { level, inlines, .. } => {
                    found.push((*level, plain_text(inlines)));
                }
                Block::Section { blocks, .. } => walk(blocks, found),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    walk(&out.document.body, &mut found);
    found
}

/// The top-level blocks of the first page section.
pub(crate) fn page_blocks(out: &ReadOutcome) -> &[Block] {
    out.document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section { blocks, .. } => Some(blocks.as_slice()),
            _ => None,
        })
        .expect("a page section")
}

#[test]
fn data_lines_are_not_headings() {
    let page = FormPage {
        template: label_mass(800, 400.0),
        lines: padded_data_lines(700.0),
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(headings_of(&out).is_empty(), "{:?}", headings_of(&out));
    assert!(all_text(&out).contains("Muster5"), "{}", all_text(&out));
}

#[test]
fn address_block_keeps_its_lines() {
    let mut lines = vec![
        DataLine {
            y: 760.0,
            text: "Erika Mustermann".into(),
        },
        DataLine {
            y: 748.0,
            text: "Musterweg 1".into(),
        },
        DataLine {
            y: 736.0,
            text: "12345 Musterstadt".into(),
        },
        DataLine {
            y: 724.0,
            text: "Deutschland".into(),
        },
    ];
    lines.extend(padded_data_lines(640.0));
    let page = FormPage {
        template: label_mass(800, 400.0),
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(headings_of(&out).is_empty(), "{:?}", headings_of(&out));
    let address = page_blocks(&out)
        .iter()
        .find_map(|b| match b {
            Block::Paragraph { inlines, .. }
                if plain_text(inlines).starts_with("Erika Mustermann") =>
            {
                Some(inlines)
            }
            _ => None,
        })
        .expect("the address paragraph");
    assert_eq!(
        plain_text(address),
        "Erika Mustermann Musterweg 1 12345 Musterstadt Deutschland"
    );
    let breaks = address
        .iter()
        .filter(|i| matches!(i, rezzer_model::Inline::LineBreak))
        .count();
    assert_eq!(breaks, 3, "{address:?}");
}

#[test]
fn star_with_padding_is_no_bullet() {
    let page = FormPage {
        template: label(40.0, 760.0, 6.0, "Muster GmbH Abrechnung"),
        lines: vec![
            DataLine {
                y: 700.0,
                text: "*     Betrag".into(),
            },
            DataLine {
                y: 640.0,
                text: "die Ware wird wie vereinbart an die Muster GmbH geliefert".into(),
            },
        ],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let lists = page_blocks(&out)
        .iter()
        .filter(|b| matches!(b, Block::List { .. }))
        .count();
    assert_eq!(lists, 0, "{:#?}", page_blocks(&out));
    assert!(all_text(&out).contains("Betrag"), "{}", all_text(&out));
}
