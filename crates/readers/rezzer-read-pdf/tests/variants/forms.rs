// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shapes of print-run forms (Tasks 4, 5, 6 and 9): the pay slip shapes of
//! `form_fields/shapes.rs` in every variant, and separator marks, section grids and a band
//! table with empty cells, each in the geometry of the variant (every height relative to the
//! data size, every x to the data pitch). All text and values are invented.

use crate::axes::{ALL, Axes, Data, Extent, Look, Next, Shape, Sheet, Variant};
use crate::common::label;
use crate::draw::{bold, courier, mark_b};
use crate::shapes::{
    BLOCK_A, Built, DataFont, Geo, ROWS, Rows, THREE_ROWS, boxed_row, grids_side_by_side,
    header_grid, label_block, lone_field, overflow_rows, page, rows_page, short_value_rows,
    title_grid_page, total_column,
};

/// Task 15b part 2c review (Important 2 and 3): a row of three boxed fields whose middle box
/// holds an address, a street in the label font over its postcode and town in the data font:
/// it is left out of the row and a loss says so; the fields beside it pair.
fn address_box_row(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let h = 2.4 * g.data;
    let mut parts = Vec::new();
    for (k, l, v) in [
        (0usize, "Personalnummer", "00000"),
        (20, "Musterweg 1", "12345 Musterstadt"),
        (40, "Eintritt", "01.01.2020"),
    ] {
        let (x0, x1) = (g.col(k) - 0.12, g.col(k + 20) - 0.12);
        b.page.template += &g.hrule(x0, x1, top);
        b.page.template += &g.hrule(x0, x1, top - h);
        b.page.template += &g.vrule(x0, top - h, top);
        b.page.template += &g.vrule(x1, top - h, top);
        b.page.template += &label(
            g.col(k) + 0.3 * g.label,
            top - 0.9 * g.label - 0.3 * g.data,
            g.label,
            l,
        );
        parts.push((k + 1, v));
        b.values.push(v.to_string());
        if k == 20 {
            // Not clear as a field: it must not pair, and a loss names it.
            b.values.push(l.to_string());
            b.fields.push((l, Err(())));
        } else {
            b.fields.push((l, Ok(Some(v))));
        }
    }
    g.values(b, top - h + 0.3 * g.data, &parts);
    top - h
}

/// Task 15b part 2c concern 3 ruling: a row of three boxed fields whose middle box holds a
/// company name, in another size than the labels beside it and without a colon, over a street
/// and a postcode with its town in the data font: nothing shows the name to be a label, so it
/// pairs with nothing and a loss names it; the fields beside it pair.
fn company_box_row(g: &Geo, b: &mut Built, top: f32) -> f32 {
    company_row(g, b, top, |g, x, y, l| label(x, y, 0.8 * g.data, l))
}

/// Task 15b part 2d item 4: [`company_box_row`] with the company name at the labels' size in
/// Courier, another font family than the labels' (Helvetica or Times): its style is not the
/// labels', so it pairs with nothing and a loss names it.
fn company_in_another_font_row(g: &Geo, b: &mut Built, top: f32) -> f32 {
    company_row(g, b, top, |g, x, y, l| courier(x, y, g.label, l))
}

/// A row of three boxed fields whose middle box holds a company name, drawn by `name`, over a
/// street and a postcode with its town in the data font: it must not pair, a loss names it,
/// and the fields beside it pair.
fn company_row(g: &Geo, b: &mut Built, top: f32, name: fn(&Geo, f32, f32, &str) -> String) -> f32 {
    let h = 3.6 * g.data;
    let mut parts = Vec::new();
    for (k, l, v) in [
        (0usize, "Personalnummer", "00000"),
        (20, "Muster GmbH", ""),
        (40, "Eintritt", "01.01.2020"),
    ] {
        let (x0, x1) = (g.col(k) - 0.12, g.col(k + 20) - 0.12);
        b.page.template += &g.hrule(x0, x1, top);
        b.page.template += &g.hrule(x0, x1, top - h);
        b.page.template += &g.vrule(x0, top - h, top);
        b.page.template += &g.vrule(x1, top - h, top);
        let (x, y) = (g.col(k) + 0.3 * g.label, top - 0.9 * g.label - 0.3 * g.data);
        b.page.template += &if k == 20 {
            name(g, x, y, l)
        } else {
            label(x, y, g.label, l)
        };
        if k == 20 {
            b.values.push(l.to_string());
            b.fields.push((l, Err(())));
            for (n, line) in ["Musterweg 1", "12345 Musterstadt"].iter().enumerate() {
                g.values(
                    b,
                    top - h + (1.5 - 1.2 * n as f32) * g.data,
                    &[(k + 1, line)],
                );
                b.values.push(line.to_string());
            }
        } else {
            parts.push((k + 1, v));
            b.values.push(v.to_string());
            b.fields.push((l, Ok(Some(v))));
        }
    }
    g.values(b, top - h + 0.3 * g.data, &parts);
    top - h
}

/// Task 15b part 2d item 1: a row of three boxed fields whose middle box holds a bold title of
/// two lines, at the data size: the title is a heading of its own, and the fields on both
/// sides of it pair.
fn title_box_row(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let h = 3.6 * g.data;
    let mut parts = Vec::new();
    for (k, l, v) in [
        (0usize, "Personalnummer", "00000"),
        (20, "", ""),
        (40, "Eintritt", "01.01.2020"),
    ] {
        let (x0, x1) = (g.col(k) - 0.12, g.col(k + 20) - 0.12);
        b.page.template += &g.hrule(x0, x1, top);
        b.page.template += &g.hrule(x0, x1, top - h);
        b.page.template += &g.vrule(x0, top - h, top);
        b.page.template += &g.vrule(x1, top - h, top);
        let x = g.col(k) + 0.3 * g.label;
        if k == 20 {
            for (n, line) in ["Angaben zum", "Betrieb"].iter().enumerate() {
                let y = top - 1.2 * g.data - 1.2 * g.data * n as f32;
                b.page.template += &bold(x, y, g.data, line);
                b.values.push(line.to_string());
            }
        } else {
            b.page.template += &label(x, top - 0.9 * g.label - 0.3 * g.data, g.label, l);
            parts.push((k + 1, v));
            b.values.push(v.to_string());
            b.fields.push((l, Ok(Some(v))));
        }
    }
    g.values(b, top - h + 0.3 * g.data, &parts);
    top - h
}

/// Task 15b part 2d item 2: a row of three boxed fields whose middle box holds a company name
/// at the data size in the data font, over its street and its postcode with the town: the box
/// has no label, so it pairs with nothing and a loss names it; the fields beside it pair.
fn value_box_row(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let h = 3.6 * g.data;
    let mut parts = Vec::new();
    for (k, l, v) in [
        (0usize, "Personalnummer", "00000"),
        (20, "Muster GmbH", ""),
        (40, "Eintritt", "01.01.2020"),
    ] {
        let (x0, x1) = (g.col(k) - 0.12, g.col(k + 20) - 0.12);
        b.page.template += &g.hrule(x0, x1, top);
        b.page.template += &g.hrule(x0, x1, top - h);
        b.page.template += &g.vrule(x0, top - h, top);
        b.page.template += &g.vrule(x1, top - h, top);
        if k == 20 {
            b.fields.push((l, Err(())));
            for (n, line) in [l, "Musterweg 1", "12345 Musterstadt"].iter().enumerate() {
                g.values(
                    b,
                    top - h + (2.7 - 1.2 * n as f32) * g.data,
                    &[(k + 1, line)],
                );
                b.values.push(line.to_string());
            }
        } else {
            b.page.template += &label(
                g.col(k) + 0.3 * g.label,
                top - 0.9 * g.label - 0.3 * g.data,
                g.label,
                l,
            );
            parts.push((k + 1, v));
            b.values.push(v.to_string());
            b.fields.push((l, Ok(Some(v))));
        }
    }
    g.values(b, top - h + 0.3 * g.data, &parts);
    top - h
}

/// Task 15b part 2d item 3: a strip of three fields between two rules with no rule between
/// them, the values on padded data lines; the middle field is drawn by `middle` from the top
/// of its cell, and says whether an address goes under it. The fields on both sides pair.
fn open_strip(
    g: &Geo,
    b: &mut Built,
    top: f32,
    middle: fn(&Geo, &mut Built, f32, f32) -> bool,
) -> f32 {
    open_strip_at(g, b, top, middle, 20, 1.0)
}

/// [`open_strip`] with the middle field in data column `at` (0, 20 or 40) and the two fields
/// in the other two, the label of the first of them `first` times the labels' size.
fn open_strip_at(
    g: &Geo,
    b: &mut Built,
    top: f32,
    middle: fn(&Geo, &mut Built, f32, f32) -> bool,
    at: usize,
    first: f32,
) -> f32 {
    let h = 3.6 * g.data;
    let (x0, x1) = (g.col(0) - 0.12, g.col(60) - 0.12);
    b.page.template += &g.hrule(x0, x1, top);
    b.page.template += &g.hrule(x0, x1, top - h);
    let fields: Vec<usize> = [0usize, 20, 40].into_iter().filter(|&k| k != at).collect();
    let named = [("Personalnummer", "00000"), ("Eintritt", "01.01.2020")];
    for (n, (&k, (l, _))) in fields.iter().zip(named).enumerate() {
        let size = if n == 0 { first * g.label } else { g.label };
        b.page.template += &label(
            g.col(k) + 0.3 * g.label,
            top - 0.9 * g.label - 0.3 * g.data,
            size,
            l,
        );
    }
    let address = middle(g, b, g.col(at) + 0.3 * g.label, top);
    for (l, v) in named {
        b.values.push(v.to_string());
        b.fields.push((l, Ok(Some(v))));
    }
    let bottom = top - h + 0.3 * g.data;
    let mut low: Vec<(usize, &str)> = fields
        .iter()
        .zip(named)
        .map(|(&k, (_, v))| (k + 1, v))
        .collect();
    if address {
        g.values(b, bottom + 1.2 * g.data, &[(at + 1, "Musterweg 1")]);
        low.push((at + 1, "12345 Musterstadt"));
    }
    low.sort_by_key(|p| p.0);
    g.values(b, bottom, &low);
    top - h
}

/// A shape drawn by `draw` with the cells `cells` listed (see [`crate::axes::Want::cells`]).
fn with_cells(
    v: &Variant,
    draw: fn(&Geo, &mut Built, f32) -> f32,
    right: usize,
    cells: &[&[&str]],
) -> (Sheet, Option<Extent>) {
    let (mut s, e) = drawn(v, draw, right);
    for c in cells {
        s.want
            .cells
            .push(c.iter().map(|t| (*t).to_string()).collect());
    }
    (s, e)
}

/// Task 15b part 2d fix round 2 (I6, M1): a row of three boxed fields whose middle box is
/// `middle`: with no text (an empty box), or with a label alone (an empty field). The fields
/// beside it pair, and an empty field is one.
fn gap_box_row(g: &Geo, b: &mut Built, top: f32, middle: Option<&'static str>) -> f32 {
    let h = 2.4 * g.data;
    let mut parts = Vec::new();
    for (k, l, v) in [
        (0usize, Some("Personalnummer"), "00000"),
        (20, middle, ""),
        (40, Some("Eintritt"), "01.01.2020"),
    ] {
        let (x0, x1) = (g.col(k) - 0.12, g.col(k + 20) - 0.12);
        b.page.template += &g.hrule(x0, x1, top);
        b.page.template += &g.hrule(x0, x1, top - h);
        b.page.template += &g.vrule(x0, top - h, top);
        b.page.template += &g.vrule(x1, top - h, top);
        let Some(l) = l else {
            continue;
        };
        b.page.template += &label(
            g.col(k) + 0.3 * g.label,
            top - 0.9 * g.label - 0.3 * g.data,
            g.label,
            l,
        );
        if v.is_empty() {
            b.fields.push((l, Ok(None)));
        } else {
            parts.push((k + 1, v));
            b.values.push(v.to_string());
            b.fields.push((l, Ok(Some(v))));
        }
    }
    g.values(b, top - h + 0.3 * g.data, &parts);
    top - h
}

/// Task 15b part 2d fix round 2: a strip of two cells between two rules with no rule between
/// them, `Personalnummer` over its value beside a value alone on the same data line: the field
/// is one field alone between its rules, refused, and the strip is read cell by cell.
fn lone_beside_a_value(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let h = 3.6 * g.data;
    let (x0, x1) = (g.col(0) - 0.12, g.col(40) - 0.12);
    b.page.template += &g.hrule(x0, x1, top);
    b.page.template += &g.hrule(x0, x1, top - h);
    let y = top - 0.9 * g.label - 0.3 * g.data;
    b.page.template += &label(g.col(0) + 0.3 * g.label, y, g.label, "Personalnummer");
    b.fields.push(("Personalnummer", Err(())));
    for v in ["00000", "Beispielwert"] {
        b.values.push(v.to_string());
    }
    g.values(
        b,
        top - h + 0.3 * g.data,
        &[(1, "00000"), (21, "Beispielwert")],
    );
    top - h
}

/// Task 15b part 2d fix round 1 (rulings on concerns 1 and 3): a strip of two cells between two
/// rules with no rule between them, `Personalnummer` over its value beside a company name a
/// little smaller than it over its address. The name is left out, so the field is one field
/// alone between its rules with no sign of a label: it pairs with nothing and a loss names it;
/// the strip is read cell by cell.
fn lone_strip(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let h = 3.6 * g.data;
    let (x0, x1) = (g.col(0) - 0.12, g.col(40) - 0.12);
    b.page.template += &g.hrule(x0, x1, top);
    b.page.template += &g.hrule(x0, x1, top - h);
    let y = top - 0.9 * g.label - 0.3 * g.data;
    b.page.template += &label(g.col(0) + 0.3 * g.label, y, g.label, "Personalnummer");
    b.fields.push(("Personalnummer", Err(())));
    smaller_name(g, b, g.col(20) + 0.3 * g.label, top);
    b.values.push("00000".to_string());
    let bottom = top - h + 0.3 * g.data;
    g.values(b, bottom + 1.2 * g.data, &[(21, "Musterweg 1")]);
    g.values(b, bottom, &[(1, "00000"), (21, "12345 Musterstadt")]);
    top - h
}

/// [`open_strip`] with `middle` as a shape: no block holds a text of the middle field with a
/// label or a value of the fields beside it.
fn open_strip_shape(
    v: &Variant,
    draw: fn(&Geo, &mut Built, f32) -> f32,
    theirs: &[&str],
) -> (Sheet, Option<Extent>) {
    let (mut s, e) = drawn(v, draw, 60);
    s.want
        .cells
        .push(theirs.iter().map(|t| (*t).to_string()).collect());
    for o in ["Personalnummer", "00000", "Eintritt", "01.01.2020"] {
        for t in theirs {
            s.want.apart.push((o.to_string(), (*t).to_string()));
        }
    }
    (s, e)
}

/// The middle of [`open_strip`]: a company name a little smaller than the labels beside it,
/// over a street and a postcode with its town: it pairs with nothing and a loss names it.
fn smaller_name(g: &Geo, b: &mut Built, x: f32, top: f32) -> bool {
    b.page.template += &label(
        x,
        top - 0.9 * g.label - 0.3 * g.data,
        0.9 * g.label,
        "Beispiel AG",
    );
    b.fields.push(("Beispiel AG", Err(())));
    for t in SMALLER_NAME {
        b.values.push(t.to_string());
    }
    true
}

/// The texts of [`smaller_name`]: a name the address under the shape does not hold, so that
/// each of its texts is drawn once.
const SMALLER_NAME: [&str; 3] = ["Beispiel AG", "Musterweg 1", "12345 Musterstadt"];

/// The texts of the company boxes.
const NAME_AND_ADDRESS: [&str; 3] = ["Muster GmbH", "Musterweg 1", "12345 Musterstadt"];

/// The middle of [`open_strip`]: a bold title at the data size, its own text in no pair.
fn strip_title(g: &Geo, b: &mut Built, x: f32, top: f32) -> bool {
    b.page.template += &bold(x, top - 1.2 * g.data, g.data, "Angaben");
    b.values.push("Angaben".to_string());
    false
}

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
            name: "boxed fields beside a company over its address",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| with_cells(v, company_box_row, 60, &[&NAME_AND_ADDRESS]),
        },
        Shape {
            name: "boxed fields beside a bold title",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| {
                let (mut s, e) = with_cells(v, title_box_row, 60, &[&["Angaben zum", "Betrieb"]]);
                s.want.headings.push("Angaben zum Betrieb".into());
                (s, e)
            },
        },
        Shape {
            name: "boxed fields beside a box at the value size",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| with_cells(v, value_box_row, 60, &[&NAME_AND_ADDRESS]),
        },
        Shape {
            name: "a strip without dividers beside a smaller name over its address",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| {
                open_strip_shape(
                    v,
                    |g, b, t| open_strip(g, b, t, smaller_name),
                    &SMALLER_NAME,
                )
            },
        },
        Shape {
            name: "a lone field in a strip beside a smaller name over its address",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| {
                let (mut s, e) = with_cells(
                    v,
                    lone_strip,
                    40,
                    &[&SMALLER_NAME, &["Personalnummer", "00000"]],
                );
                for o in ["Personalnummer", "00000"] {
                    for t in SMALLER_NAME {
                        s.want.apart.push((o.to_string(), t.to_string()));
                    }
                }
                (s, e)
            },
        },
        Shape {
            name: "a strip without dividers after a smaller name over its address",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| {
                open_strip_shape(
                    v,
                    |g, b, t| open_strip_at(g, b, t, smaller_name, 0, 1.0),
                    &SMALLER_NAME,
                )
            },
        },
        Shape {
            name: "a strip of a smaller label beside a smaller name over its address",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| {
                open_strip_shape(
                    v,
                    |g, b, t| open_strip_at(g, b, t, smaller_name, 20, 0.9),
                    &SMALLER_NAME,
                )
            },
        },
        Shape {
            name: "a lone field in a strip beside a value",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| {
                with_cells(
                    v,
                    lone_beside_a_value,
                    40,
                    &[&["Personalnummer", "00000"], &["Beispielwert"]],
                )
            },
        },
        Shape {
            name: "a strip without dividers beside a bold title",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| open_strip_shape(v, |g, b, t| open_strip(g, b, t, strip_title), &["Angaben"]),
        },
        Shape {
            name: "boxed fields beside an empty box",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| drawn(v, |g, b, t| gap_box_row(g, b, t, None), 60),
        },
        Shape {
            name: "boxed fields beside an empty field",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| drawn(v, |g, b, t| gap_box_row(g, b, t, Some("Steuerklasse")), 60),
        },
        Shape {
            name: "boxed fields beside a company in another font",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| with_cells(v, company_in_another_font_row, 60, &[&NAME_AND_ADDRESS]),
        },
        Shape {
            name: "boxed fields beside an address",
            axes: Axes {
                look: NO_RISING,
                ..ALL
            },
            draw: |v| {
                with_cells(
                    v,
                    address_box_row,
                    60,
                    &[&["Musterweg 1", "12345 Musterstadt"]],
                )
            },
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
                    // The value runs on into the next field: the two fields read as one cell
                    // (Task 15b part 2d fix round 2), their labels over the value.
                    s.want
                        .cells
                        .push(vec!["Name".into(), "Nr".into(), "Erika Mustermann".into()]);
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
