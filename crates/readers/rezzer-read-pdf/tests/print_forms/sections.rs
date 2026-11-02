// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Form sections of ruled grids, with and without marks, and boxed amounts beside them.

use super::*;

/// The edges of `n` columns of equal width across the form, from 40 to 555.
pub(crate) fn form_columns(n: usize) -> Vec<f32> {
    (0..=n)
        .map(|k| 40.0 + 515.0 * k as f32 / n as f32)
        .collect()
}

/// How the values of a form grid are printed.
#[derive(Clone, Copy)]
pub(crate) struct Print {
    /// Each value row as one data line from the data margin, padded into the cells; else
    /// each value as a data line of its own at its cell.
    pub one_line: bool,
    /// The baseline of the values above the bottom of their row.
    pub rise: f32,
}

/// Values one to a cell, 5 points above the row's bottom.
pub(crate) const BY_CELL: Print = Print {
    one_line: false,
    rise: 5.0,
};

/// One section of a form grid: rows of 16 points from `top` down between the column edges
/// `xs`, every rule drawn; the first row holds the labels `titles`, the other rows the
/// values of `rows`, printed as `print` says, each 3 points into its cell.
pub(crate) fn form_section(
    xs: &[f32],
    top: f32,
    titles: &[&str],
    rows: &[&[&str]],
    print: Print,
) -> String {
    let ys: Vec<f32> = (0..=rows.len() + 1)
        .map(|k| top - 16.0 * k as f32)
        .collect();
    let mut c = grid(xs, &ys);
    for (k, t) in titles.iter().enumerate() {
        c += &label(xs[k] + 3.0, top - 11.0, 7.0, t);
    }
    for (r, row) in rows.iter().enumerate() {
        let y = top - 16.0 * (r + 2) as f32 + print.rise;
        if print.one_line {
            let mut line = String::new();
            for (k, v) in row.iter().enumerate() {
                let at = pad_to(xs[k] + 3.0);
                line += &" ".repeat(at.saturating_sub(line.chars().count()));
                line += v;
            }
            c += &data_line(40.0, y, 9.5, &line);
        } else {
            for (k, v) in row.iter().enumerate() {
                c += &data_line(xs[k] + 3.0, y, 9.5, v);
            }
        }
    }
    c
}

/// The upper section of the form: 8 columns of personal data, a label row over two value
/// rows.
pub(crate) fn personal_section(print: Print) -> String {
    form_section(
        &form_columns(8),
        760.0,
        &[
            "Pers.-Nr.",
            "Name",
            "Eintritt",
            "Klasse",
            "Kinder",
            "Konf.",
            "Kasse",
            "Abt.",
        ],
        &[
            &[
                "0001", "Muster", "01.01.20", "1", "0,5", "x", "Kasse A", "100",
            ],
            &[
                "0002", "Beispiel", "15.03.21", "3", "1,0", "y", "Kasse B", "200",
            ],
        ],
        print,
    )
}

/// The earnings section: 9 columns, a label row over three rows of amounts, the first of
/// them drawn by `brutto`.
pub(crate) fn earnings_section(top: f32, brutto: &str, print: Print) -> String {
    form_section(
        &form_columns(9),
        top,
        &[
            "Nr.", "Art", "Tage", "Std.", "Satz", "Basis", "Brutto", "Steuer", "Netto",
        ],
        &[
            &[
                "1001", "Gehalt", "20", "160", "12,34", "1.974,40", brutto, "123,45", "876,54",
            ],
            &[
                "1002", "Zulage", "20", "8", "5,00", "40,00", "40,00", "4,00", "36,00",
            ],
            &[
                "1003", "Bonus", "1", "0", "0,00", "100,00", "100,00", "10,00", "90,00",
            ],
        ],
        print,
    )
}

/// The captions of the tables of the read, in order.
pub(crate) fn read_captions(out: &ReadOutcome) -> Vec<Option<String>> {
    let mut found = Vec::new();
    for b in &out.document.body {
        if let Block::Section { blocks, .. } = b {
            for inner in blocks {
                if let Block::Table { caption, .. } = inner {
                    found.push(caption.clone());
                }
            }
        }
    }
    found
}

/// The tables of the read: the text of each cell.
pub(crate) fn read_tables(out: &ReadOutcome) -> Vec<Vec<Vec<String>>> {
    let mut found = Vec::new();
    for b in &out.document.body {
        if let Block::Section { blocks, .. } = b {
            for inner in blocks {
                if let Block::Table { rows, .. } = inner {
                    found.push(
                        rows.iter()
                            .map(|r| r.iter().map(|c| plain_text(&c.inlines)).collect())
                            .collect(),
                    );
                }
            }
        }
    }
    found
}

/// The form's grid of two sections without marks: 8 columns over 9 that share only the
/// outer edges, 17 column edges in all. Each section is read as its own table.
#[test]
fn form_sections_without_marks() {
    let template = personal_section(BY_CELL) + &earnings_section(712.0, "1.234,56", BY_CELL);
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 2, "{found:?} {:?}", out.document.losses);
    assert_eq!(found[0].len(), 3, "{found:?}");
    assert!(found[0].iter().all(|r| r.len() == 8), "{found:?}");
    assert_eq!(found[0][1][1], "Muster", "{found:?}");
    assert_eq!(found[1].len(), 4, "{found:?}");
    assert!(found[1].iter().all(|r| r.len() == 9), "{found:?}");
    assert_eq!(found[1][1][6], "1.234,56", "{found:?}");
    assert_no_loss(&out);
}

/// The earnings section alone, its amounts printed as digits with drawn decimal marks on a
/// baseline 2 points above the row rules, filled (`re f`) or filled and stroked (`B*`) as
/// `stroked` says. The marks are no rules, so the grid keeps its 9 columns and 4 rows and is
/// read as one table with its separators.
pub(crate) fn marks_without_sections(stroked: bool) {
    let xs = form_columns(9);
    let top = 712.0;
    let amounts: [[&str; 4]; 3] = [
        ["197440", "123456", "12345", "87654"],
        ["4000", "4000", "400", "3600"],
        ["10000", "10000", "1000", "9000"],
    ];
    let rows: Vec<Vec<&str>> = ["1001", "1002", "1003"]
        .iter()
        .zip(["Gehalt", "Zulage", "Bonus"])
        .zip(&amounts)
        .map(|((nr, art), a)| {
            let mut row = vec![*nr, art, "20", "8", "5,00"];
            row.extend(a.iter().copied());
            row
        })
        .collect();
    let refs: Vec<&[&str]> = rows.iter().map(|r| r.as_slice()).collect();
    let titles = [
        "Nr.", "Art", "Tage", "Std.", "Satz", "Basis", "Brutto", "Steuer", "Netto",
    ];
    let print = Print {
        one_line: false,
        rise: 2.0,
    };
    let mut template = form_section(&xs, top, &titles, &refs, print);
    for (r, row) in amounts.iter().enumerate() {
        let y = top - 16.0 * (r + 2) as f32 + print.rise;
        for (k, v) in row.iter().enumerate() {
            let x = xs[5 + k] + 3.0 + (v.len() - 2) as f32 * PITCH;
            template += &if stroked {
                mark_b(x, y - 0.5, 2.0)
            } else {
                mark(x - 0.12, y - 0.5)
            };
        }
    }
    let page = FormPage {
        template,
        lines: vec![
            DataLine {
                y: 600.0,
                text: "Summe 9.876,54".into(),
            },
            DataLine {
                y: 586.0,
                text: "Netto 1.234,56".into(),
            },
        ],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:?} {:?}", out.document.losses);
    assert_eq!(found[0].len(), 4, "{found:?}");
    assert!(found[0].iter().all(|r| r.len() == 9), "{found:?}");
    assert_eq!(
        found[0][1][5..],
        ["1974,40", "1234,56", "123,45", "876,54"],
        "{found:?}"
    );
    assert_eq!(found[0][3][5..], ["100,00", "100,00", "10,00", "90,00"]);
    assert_no_loss(&out);
}

/// Guard: filled marks in one section; the form of two sections is refused only for its
/// sections (`form_sections_without_marks`).
#[test]
fn filled_marks_without_sections() {
    marks_without_sections(false);
}

/// Marks filled and stroked with a hairline: the 2 point sides of the stroke would meet the
/// row rules 1.5 points below and add 12 column edges; they are no rules either.
#[test]
fn stroked_marks_without_sections() {
    marks_without_sections(true);
}

/// Guard: a boxed amount of one row of digit boxes alone is a header band candidate that
/// holds no words: no table and no loss (this was so before Task 5 too).
#[test]
fn boxed_amount_alone_gives_no_loss() {
    let page = boxed_page("123456", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    let out = read(&print_form(&[page], "", ""));
    assert!(read_tables(&out).is_empty(), "{:?}", read_tables(&out));
    assert_no_loss(&out);
}

/// A boxed amount whose digit boxes stand under a box for its label: a boxed field, not a
/// table, and no loss.
#[test]
fn boxed_amount_under_a_label_box_gives_no_loss() {
    let x = 40.0 + 20.0 * PITCH;
    let mut page = boxed_page("123456", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    page.template += &boxed(x, 698.0, 6.0 * PITCH, 22.0);
    let out = read(&print_form(&[page], "", ""));
    assert!(all_text(&out).contains("1234,56"), "{}", all_text(&out));
    assert!(read_tables(&out).is_empty(), "{:?}", read_tables(&out));
    assert_no_loss(&out);
}

/// Two boxed fields, one right on top of the other: boxed fields, not a table, and no loss.
#[test]
fn stacked_boxed_fields_give_no_loss() {
    let x = 40.0 + 20.0 * PITCH;
    let mut page = boxed_page("123456", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    page.template += &boxed_field(x, 711.0, 6, &[4]);
    page.lines.push(DataLine {
        y: 711.0,
        text: padded(20, "654321"),
    });
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(
        text.contains("1234,56") && text.contains("6543,21"),
        "{text}"
    );
    assert!(read_tables(&out).is_empty(), "{:?}", read_tables(&out));
    assert_no_loss(&out);
}

/// Guard: a calendar of 31 narrow equal day columns with one digit run in each row is no
/// boxed field when a row holds the days apart: it stays a refused grid with its loss.
#[test]
fn day_columns_are_not_a_boxed_field() {
    let xs: Vec<f32> = (0..=31).map(|k| 40.0 + 12.0 * k as f32).collect();
    let mut template = grid(&xs, &[660.0, 676.0, 692.0]);
    for (k, x) in xs[..31].iter().enumerate() {
        template += &label(x + 1.0, 681.0, 7.0, &format!("{}", k + 1));
    }
    template += &data_line(41.0, 665.0, 9.5, "1");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let what = one_uncertain_loss(&out);
    assert!(what.contains("more than 15 columns"), "{what}");
}

/// A chart of twenty columns with labels in two rows and a figure caption: the caption is
/// looked at before the columns are counted, so no note says it may be a table, and no loss.
#[test]
fn wide_chart_with_two_label_rows_gives_no_note() {
    let xs: Vec<f32> = (0..=20).map(|k| 40.0 + 25.0 * k as f32).collect();
    let mut template = grid(&xs, &[640.0, 660.0, 680.0, 700.0]);
    for k in 0..5 {
        template += &label(43.0 + 100.0 * k as f32, 646.0, 7.0, &format!("{}0", k + 1));
        template += &label(43.0 + 100.0 * k as f32, 686.0, 7.0, &format!("{}5", k + 1));
    }
    template += &label(40.0, 625.0, 9.0, "Figure 3: Stunden je Monat");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(
        !all_text(&out).contains("Possible table"),
        "{}",
        all_text(&out)
    );
    assert_no_loss(&out);
}

// ---------------------------------------------------------------------------------------
// A pay slip form: the geometry of a real print-run form, all text and values invented
