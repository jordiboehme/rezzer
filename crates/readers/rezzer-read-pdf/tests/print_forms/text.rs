// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Text shown in odd ways: placed twice, filled and stroked, fake bold, umlauts.

use super::*;

#[test]
fn cm_placed_identical_lines_are_kept() {
    let page = FormPage {
        template: label(40.0, 760.0, 6.0, "Muster GmbH Abrechnung"),
        lines: vec![
            DataLine {
                y: 700.0,
                text: "      0,00".into(),
            },
            DataLine {
                y: 680.0,
                text: "      0,00".into(),
            },
        ],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert_eq!(text.matches("0,00").count(), 2, "{text}");
    assert_no_loss(&out);
}

/// Guard: a fill-then-stroke show (render mode 2) inside a `cm` block is still one text.
#[test]
fn fill_and_stroke_show_reads_once() {
    let page = FormPage {
        template: "q 1 0 0 1 40 700 cm BT /FB 12 Tf 2 Tr 0 0 Td (Erika Mustermann) Tj ET Q\n"
            .into(),
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert_eq!(text.matches("Erika Mustermann").count(), 1, "{text}");
    assert_no_loss(&out);
}

/// Guard: one data line drawn twice 0.3 points apart (a fake bold) is read once.
#[test]
fn fake_bold_line_reads_once() {
    let line = "Auszahlung      1.234,56";
    let page = FormPage {
        template: format!(
            "{}{}",
            data_line(40.0, 700.0, 9.5, line),
            data_line(40.3, 700.0, 9.5, line)
        ),
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert_eq!(text.matches("1.234,56").count(), 1, "{text}");
    assert_eq!(text.matches("Auszahlung").count(), 1, "{text}");
    assert_no_loss(&out);
}

/// One data line drawn twice 0.3 points apart (a fake bold) whose copies have their middles
/// on the two sides of a box's edge is read once, in the box of the first copy (Task 4
/// carry): the box ends between the middles of the two copies.
#[test]
fn fake_bold_across_a_box_edge_reads_once() {
    let line = "Auszahlung";
    // The middles of the two copies lie about 65.8 points from the left of the page.
    let mut failed = Vec::new();
    for edge in [65.6f32, 65.7, 65.8, 65.9, 66.0] {
        let page = FormPage {
            template: format!(
                "{}{}{}{}",
                boxed(30.0, 690.0, edge - 30.0, 30.0),
                label(32.0, 712.0, 6.0, "Betrag"),
                data_line(40.0, 700.0, 9.5, line),
                data_line(40.3, 700.0, 9.5, line)
            ),
            ..FormPage::default()
        };
        let out = read(&print_form(&[page], "", ""));
        let text = all_text(&out);
        if text.matches(line).count() != 1 {
            failed.push(format!("edge {edge}: {text:?}"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The builder writes text in WinAnsi, so a German label reads back as written.
#[test]
fn label_with_umlauts_reads_back() {
    let words = "Bez\u{fc}ge \u{c4}rger \u{d6}l \u{dc}bung Gr\u{f6}\u{df}e Stra\u{df}e";
    let page = FormPage {
        template: label(40.0, 760.0, 8.0, words),
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(text.contains(words), "{text}");
    assert_no_loss(&out);
}
