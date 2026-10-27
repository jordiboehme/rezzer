// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shapes of print-run forms (Tasks 4, 5, 6 and 9): the pay slip shapes of
//! `form_fields/shapes.rs` in every variant, and separator marks, section grids and a band
//! table with empty cells, each in the geometry of the variant (every height relative to the
//! data size, every x to the data pitch). All text and values are invented.

use crate::axes::{ALL, Axes, Data, Extent, Look, Next, Shape, Sheet, Variant};
use crate::common::label;
use crate::draw::mark_b;
use crate::shapes::{
    BLOCK_A, Built, DataFont, Geo, ROWS, Rows, THREE_ROWS, boxed_row, grids_side_by_side,
    header_grid, label_block, lone_field, overflow_rows, page, rows_page, short_value_rows,
    title_grid_page, total_column,
};

/// The top of a shape on its page.
fn top(g: &Geo) -> f32 {
    760.0 + g.dy
}

/// The axes of a shape that has no room or no place for a neighbour.
const ALONE: Axes = Axes {
    next: &[Next::None],
    ..ALL
};

/// A shape drawn by `draw` from the top of its page, with the address under it, ending at
/// data column `right`.
fn drawn(
    v: &Variant,
    draw: fn(&Geo, &mut Built, f32) -> f32,
    right: usize,
) -> (Sheet, Option<Extent>) {
    let g = v.geo();
    let mut b = Built::new(&g);
    let t = top(&g);
    let bottom = draw(&g, &mut b, t);
    b.address(&g);
    let e = Extent {
        top: t,
        bottom,
        right: g.col(right),
    };
    (Sheet::new(b), Some(e))
}

/// F1 to F3 of the pay slip: a strip of seven fields with a rate over its divider and drawn
/// decimal marks, a row under a longer rule, and an underlined title over two lines, as on
/// the real form (the rate overflows, the last two labels are one run).
fn f1_to_f3(v: &Variant) -> (Sheet, Option<Extent>) {
    let g = v.geo();
    let mut s = Sheet::new(page(&g, true, true));
    s.want.headings.push("Hinweise Muster".into());
    (s, None)
}

/// Rows of fields between rules, labels over values.
fn rows(v: &Variant) -> (Sheet, Option<Extent>) {
    let g = v.geo();
    let t = top(&g);
    let b = rows_page(&g, &ROWS, &THREE_ROWS, &[]);
    let e = Extent {
        top: t,
        bottom: t - 3.0 * 2.4 * g.data,
        right: g.col(50),
    };
    (Sheet::new(b), Some(e))
}

/// Rows of fields with the labels under the rule and the values on it.
fn labels_under(v: &Variant) -> (Sheet, Option<Extent>) {
    let g = v.geo();
    let t = top(&g);
    let s = Rows {
        under: true,
        ..ROWS
    };
    let b = rows_page(&g, &s, &THREE_ROWS, &[]);
    let e = Extent {
        top: t,
        bottom: t - 2.0 * 2.4 * g.data - 1.2 * g.data,
        right: g.col(50),
    };
    (Sheet::new(b), Some(e))
}

/// The digits of a boxed field: one box a digit of the data pitch from data column `k`, its
/// data line at `y`, and a drawn mark before each of `before`.
fn boxed_digits(g: &Geo, b: &mut Built, k: usize, y: f32, digits: &str, before: &[usize]) {
    for i in 0..digits.len() {
        let x = g.col(k + i);
        b.page.template += &format!(
            "{x} {} {} {} re S\n",
            y - 0.21 * g.data,
            g.pitch(),
            1.16 * g.data
        );
    }
    for &i in before {
        b.page.template += &mark_b(g.col(k + i), y - 0.087 * g.data, 0.2 * g.data);
    }
}

/// Task 4: an amount and a date in boxes of their digits with drawn decimal and date marks,
/// and amounts and a date written as text elsewhere, which set the style. On a turned page
/// the digits still read in the direction of the page's text, so the marks are read.
fn separators(v: &Variant) -> (Sheet, Option<Extent>) {
    let g = v.geo();
    let mut b = Built::new(&g);
    let t = top(&g);
    let d = g.data;
    b.page.template += &label(g.col(0) + 0.18 * g.label, t - 0.86 * d, g.label, "Betrag");
    b.page.template += &label(g.col(20) + 0.18 * g.label, t - 0.86 * d, g.label, "Datum");
    let y = t - 2.23 * d;
    boxed_digits(&g, &mut b, 0, y, "123456", &[4]);
    boxed_digits(&g, &mut b, 20, y, "01022026", &[2, 4]);
    if g.font == DataFont::Helvetica {
        // A proportional digit is set in the middle of its box.
        for (k, digits) in [(0, "123456"), (20, "01022026")] {
            for (i, c) in digits.chars().enumerate() {
                let x = g.col(k + i) + (g.pitch() - g.digit()) / 2.0;
                b.page.template += &crate::draw::plain(x, y, g.data, &c.to_string());
            }
        }
    } else {
        g.values(&mut b, y, &[(0, "123456"), (20, "01022026")]);
    }
    let lines = ["Summe 9.876,54", "Netto 1.234,56", "Stand 15.03.2026"];
    for (k, l) in lines.iter().enumerate() {
        g.values(&mut b, t - 6.0 * d - 1.4 * d * k as f32, &[(0, l)]);
        b.values.push(l.to_string());
    }
    b.values.extend(["1234,56".into(), "01.02.2026".into()]);
    let s = Sheet::new(b);
    let e = Extent {
        top: t,
        bottom: t - 6.0 * d - 2.8 * d - 0.3 * d,
        right: g.col(28),
    };
    (s, Some(e))
}

/// One ruled section of `rows` rows of the data size between the column edges `edges` (data
/// columns) from `top`: the first row of labels, the others of values. Returns its bottom.
fn section(g: &Geo, b: &mut Built, top: f32, edges: &[usize], rows: &[&[&str]]) -> f32 {
    let h = 1.8 * g.data;
    let bottom = top - h * rows.len() as f32;
    let (x0, x1) = (g.col(edges[0]) - 0.12, g.col(*edges.last().unwrap_or(&0)));
    for k in 0..=rows.len() {
        b.page.template += &g.hrule(x0, x1, top - h * k as f32);
    }
    for &e in edges {
        b.page.template += &g.vrule(g.col(e) - 0.06, bottom, top);
    }
    for (n, row) in rows.iter().enumerate() {
        let row_top = top - h * n as f32;
        if n == 0 {
            for (&e, t) in edges.iter().zip(row.iter()) {
                b.page.template += &label(
                    g.col(e) + 0.25 * g.pitch(),
                    row_top - 1.2 * g.label,
                    g.label,
                    t,
                );
            }
        } else {
            let parts: Vec<(usize, &str)> = edges
                .iter()
                .map(|e| e + 1)
                .zip(row.iter().copied())
                .collect();
            g.values(b, row_top - h + 0.35 * g.data, &parts);
        }
    }
    bottom
}

/// The rows of the two sections of [`sections`].
const SECTION_A: [&[&str]; 3] = [
    &["Pers.-Nr.", "Name", "Eintritt", "Klasse"],
    &["0001", "Muster", "01.01.20", "1"],
    &["0002", "Beispiel", "15.03.21", "3"],
];
const SECTION_B: [&[&str]; 3] = [
    &["Nr.", "Art", "Tage", "Std.", "Satz", "Brutto"],
    &["1001", "Gehalt", "20", "160", "12,34", "1.974,40"],
    &["1002", "Zulage", "20", "8", "5,00", "40,00"],
];

fn owned(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|c| c.to_string()).collect())
        .collect()
}

/// Task 5: one drawn grid of two sections, 4 columns over 6 that share only some edges:
/// each section is a table of its own.
fn sections(v: &Variant) -> (Sheet, Option<Extent>) {
    let g = v.geo();
    let mut b = Built::new(&g);
    let t = top(&g);
    let mid = section(&g, &mut b, t, &[0, 15, 30, 45, 60], &SECTION_A);
    let bottom = section(&g, &mut b, mid, &[0, 10, 20, 30, 40, 50, 60], &SECTION_B);
    b.address(&g);
    let cells: Vec<String> = SECTION_A
        .iter()
        .chain(SECTION_B.iter())
        .flat_map(|r| r.iter().map(|c| c.to_string()))
        .collect();
    let mut s = Sheet::new(b);
    s.once = cells;
    s.want.tables = vec![owned(&SECTION_A), owned(&SECTION_B)];
    let e = Extent {
        top: t,
        bottom,
        right: g.col(60),
    };
    (s, Some(e))
}

/// The lines of [`band_table`]: each a list of (column, text).
const BAND_LINES: [&[(usize, &str)]; 4] = [
    &[(0, "Steuer A"), (1, "1.000,00"), (2, "10 %"), (3, "100,00")],
    &[(3, "20,00")],
    &[(0, "Steuer C"), (1, "500,00"), (2, "5 %"), (3, "25,00")],
    &[(3, "5,00")],
];

/// Task 6: a header band of four grey cells over lines without rules, two of them with an
/// amount only: each line is a row, its empty cells stay empty. The cells touch, or with
/// `spaced` stand a data pitch apart.
fn band_table(v: &Variant, spaced: bool) -> (Sheet, Option<Extent>) {
    let g = v.geo();
    let mut b = Built::new(&g);
    let t = top(&g);
    let d = g.data;
    let starts = [0usize, 21, 42, 63];
    b.page.template += "0.9 g\n";
    for &k in &starts {
        b.page.template += &format!(
            "{} {} {} {} re f\n",
            g.col(k) - 0.4 * d,
            t - 1.5 * d,
            if spaced { 20.0 } else { 21.0 } * g.pitch(),
            1.5 * d
        );
    }
    b.page.template += "0 g\n";
    for (&k, title) in starts.iter().zip(["Art", "Basis", "Satz", "Betrag"]) {
        b.page.template += &crate::draw::bold(g.col(k), t - 1.1 * d, 0.85 * d, title);
    }
    for (n, line) in BAND_LINES.iter().enumerate() {
        let parts: Vec<(usize, &str)> = line.iter().map(|&(c, t)| (starts[c], t)).collect();
        g.values(&mut b, t - 3.0 * d - 1.5 * d * n as f32, &parts);
        b.values.extend(line.iter().map(|p| p.1.to_string()));
    }
    b.address(&g);
    let mut s = Sheet::new(b);
    s.want.tables = vec![owned(&[
        &["Art", "Basis", "Satz", "Betrag"],
        &["Steuer A", "1.000,00", "10 %", "100,00"],
        &["", "", "", "20,00"],
        &["Steuer C", "500,00", "5 %", "25,00"],
        &["", "", "", "5,00"],
    ])];
    let e = Extent {
        top: t,
        bottom: t - 3.0 * d - 4.5 * d,
        right: g.col(83),
    };
    (s, Some(e))
}

/// The looks without rising dividers, for shapes that have no dividers.
const NO_RISING: &[Look] = &[Look::Real, Look::Dashed, Look::Dotted, Look::Thick];

/// The shapes of print-run forms.
pub(crate) fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            name: "F1 to F3",
            axes: ALONE,
            draw: f1_to_f3,
        },
        Shape {
            name: "header grid",
            axes: ALL,
            draw: |v| drawn(v, header_grid, 60),
        },
        Shape {
            name: "column of totals",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| drawn(v, total_column, 61),
        },
        Shape {
            name: "boxed fields",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| drawn(v, boxed_row, 60),
        },
        Shape {
            name: "label block and bank row",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| drawn(v, label_block, 48),
        },
        Shape {
            name: "overflow into an empty field",
            axes: ALL,
            draw: |v| {
                let (mut s, e) = drawn(v, overflow_rows, 30);
                // In a proportional font the name's word gap is not at the divider: its second
                // word stands across it, so the first row is unclear.
                if v.data == Data::Proportional {
                    for f in &mut s.built.fields {
                        if f.0 == "Name" || f.0 == "Nr" {
                            f.1 = Err(());
                        }
                    }
                }
                (s, e)
            },
        },
        Shape {
            name: "a short value after a full field",
            axes: ALL,
            draw: |v| drawn(v, short_value_rows, 30),
        },
        Shape {
            name: "rows of fields",
            axes: ALL,
            draw: rows,
        },
        Shape {
            name: "labels under values",
            axes: ALL,
            draw: labels_under,
        },
        Shape {
            name: "title block over a header grid",
            axes: ALONE,
            draw: |v| (Sheet::new(title_grid_page(&v.geo(), BLOCK_A)), None),
        },
        Shape {
            name: "grids side by side",
            axes: ALONE,
            draw: |v| (Sheet::new(grids_side_by_side(&v.geo(), true)), None),
        },
        Shape {
            name: "a lone field",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| {
                let (mut s, e) = drawn(v, lone_field, 20);
                // Beside a block of fields of its own style it is a field among fields.
                if v.next == Next::Beside {
                    for f in &mut s.built.fields {
                        if f.0 == "Kundennr" {
                            f.1 = Ok(Some("98765"));
                        }
                    }
                }
                (s, e)
            },
        },
        Shape {
            name: "separator marks",
            axes: Axes {
                look: &[Look::Real],
                ..ALL
            },
            draw: separators,
        },
        Shape {
            name: "section grids",
            axes: Axes {
                look: NO_RISING,
                next: &[Next::None, Next::TitleOver, Next::LineOver, Next::RowUnder],
                ..ALL
            },
            draw: sections,
        },
        Shape {
            name: "band table with empty cells",
            axes: Axes {
                look: &[Look::Real],
                next: &[Next::None, Next::TitleOver, Next::LineOver, Next::RowUnder],
                ..ALL
            },
            draw: |v| band_table(v, false),
        },
        Shape {
            name: "band table with spaced header cells",
            axes: Axes {
                look: &[Look::Real],
                next: &[Next::None, Next::TitleOver, Next::LineOver, Next::RowUnder],
                ..ALL
            },
            draw: |v| band_table(v, true),
        },
    ]
}
