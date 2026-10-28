// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Padded values of a data layer in their boxes and header field grids.

use super::*;

/// `n` spaces of the data font, then `text`: a value the print run pads into its place.
pub(crate) fn padded(n: usize, text: &str) -> String {
    format!("{}{text}", " ".repeat(n))
}

/// Two boxes side by side with a label each; the data lines start at the left edge of the
/// page and are padded into the boxes. Each value is read in its own box, after its label,
/// left box first.
#[test]
fn padded_values_stand_in_their_box() {
    let mut template = boxed(40.0, 620.0, 250.0, 90.0) + &boxed(300.0, 620.0, 255.0, 90.0);
    template += &label(45.0, 698.0, 6.0, "Arbeitgeber");
    template += &label(305.0, 698.0, 6.0, "Arbeitnehmer");
    // Data lines from x 40 in Courier 9.5 (5.7 points a character): 47 spaces reach x 307.9,
    // inside the right box.
    let lines = vec![
        DataLine {
            y: 680.0,
            text: format!("  Muster GmbH{}", padded(34, "Erika Mustermann")),
        },
        DataLine {
            y: 665.0,
            text: padded(2, "Musterweg 1"),
        },
        DataLine {
            y: 650.0,
            text: padded(47, "Beispielstr. 2"),
        },
        DataLine {
            y: 635.0,
            text: padded(47, "12345 Musterstadt"),
        },
    ];
    let page = FormPage {
        template,
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    let at = |t: &str| text.find(t).unwrap_or_else(|| panic!("no {t:?} in {text}"));
    let order = [
        "Arbeitgeber",
        "Muster GmbH",
        "Musterweg 1",
        "Arbeitnehmer",
        "Erika Mustermann",
        "Beispielstr. 2",
        "12345 Musterstadt",
    ];
    for w in order.windows(2) {
        assert!(
            at(w[0]) < at(w[1]),
            "{:?} before {:?} in {text}",
            w[0],
            w[1]
        );
    }
    assert!(!text.contains("  "), "{text:?}");
}

/// A template label and a data line padded onto its baseline read as one line, label first.
/// One row only, so the column search does not run: the rows of a label column are
/// `padded_values_beside_long_labels_read_row_by_row`.
#[test]
fn label_then_value_on_one_row() {
    let page = FormPage {
        template: label(60.0, 600.0, 8.0, "Auszahlungsbetrag"),
        lines: vec![DataLine {
            y: 600.0,
            text: padded(30, "1.234,56"),
        }],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(
        text.lines().any(|l| l == "Auszahlungsbetrag 1.234,56"),
        "{text:?}"
    );
}

/// A column of long labels with their values on the same baselines, the values padded from
/// the left margin far to the right (as a tax notice prints them): read row by row, each
/// label with its value. Padding is no text, so the values do not read as a column of prose.
#[test]
fn padded_values_beside_long_labels_read_row_by_row() {
    let rows = [
        ("Zu versteuerndes Einkommen", "12.345"),
        ("Festgesetzte Einkommensteuer", "1.234,56"),
        ("Solidaritaetszuschlag", "0,00"),
        ("Kirchensteuer evangelisch", "123,45"),
        ("Anzurechnende Lohnsteuer", "1.000,00"),
    ];
    let order: Vec<&str> = rows.iter().flat_map(|(l, v)| [*l, *v]).collect();
    // Labels left of the data margin and right of it, and values padded at the end to a fixed
    // record width too.
    let mut failed = Vec::new();
    for (x, width) in [(30.0, 0), (45.0, 0), (30.0, 90)] {
        let mut template = String::new();
        let mut lines = Vec::new();
        for (k, (l, v)) in rows.iter().enumerate() {
            let y = 600.0 - 14.0 * k as f32;
            template += &label(x, y, 8.0, l);
            lines.push(DataLine {
                y,
                text: format!("{:<width$}", padded(50, v)),
            });
        }
        let page = FormPage {
            template,
            lines,
            ..FormPage::default()
        };
        let out = read(&print_form(&[page], "", ""));
        if let Err(e) = in_order(&all_text(&out), &order) {
            failed.push(format!("labels at x {x}, width {width}: {e}"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The fields of a header grid in the right half of the page: (x, label) for each field of
/// each grid row, with its value.
pub(crate) const HEADER_FIELDS: [[(f32, &str, &str); 3]; 2] = [
    [
        (320.0, "Personalnr", "00001"),
        (400.0, "Eintritt", "01.04.2020"),
        (480.0, "Kostenstelle", "4711"),
    ],
    [
        (320.0, "Geburtsdatum", "01.01.1970"),
        (400.0, "Austritt", "31.12.2030"),
        (480.0, "Abteilung", "Muster"),
    ],
];

/// The spaces of the data font from the data margin to `x`.
pub(crate) fn pad_to(x: f32) -> usize {
    ((x - 40.0) / 5.7).round() as usize
}

/// A header grid read row by row: the labels of a grid row, then its values, left to right;
/// `one_line` prints the values of a grid row as one data line, else one line per field.
pub(crate) fn header_grid(one_line: bool) {
    let mut template = String::new();
    let mut lines = Vec::new();
    for (row, y) in HEADER_FIELDS.iter().zip([700.0f32, 676.0]) {
        let mut line = String::new();
        for &(x, l, v) in row {
            template += &label(x, y + 4.0, 6.0, l);
            if one_line {
                line = format!("{line:<0$}{v}", pad_to(x));
            } else {
                lines.push(DataLine {
                    y: y - 6.0,
                    text: padded(pad_to(x), v),
                });
            }
        }
        if one_line {
            lines.push(DataLine {
                y: y - 6.0,
                text: line,
            });
        }
    }
    let page = FormPage {
        template,
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let order: Vec<&str> = HEADER_FIELDS
        .iter()
        .flat_map(|row| {
            row.iter()
                .map(|f| f.1)
                .chain(row.iter().map(|f| f.2))
                .collect::<Vec<_>>()
        })
        .collect();
    if let Err(e) = in_order(&all_text(&out), &order) {
        panic!("{e}");
    }
}

/// A header field grid whose values are padded from the left margin, one data line per field:
/// read grid row by grid row, not in column bands.
#[test]
fn header_field_grid_reads_row_by_row() {
    header_grid(false);
}

/// Guard: the same grid with the values of a grid row in one data line.
#[test]
fn header_field_grid_of_one_line_a_row_reads_row_by_row() {
    header_grid(true);
}

// ---------------------------------------------------------------------------------------
// Separators drawn as marks
