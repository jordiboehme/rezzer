// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shapes of business documents (Tasks 7, 8 and 10 to 13): a letter head with a title, a
//! date, an address and a footnote, line items with an end rule and with a rule under every
//! row, close column titles, a title beside a field list, and footer columns. Each page has a
//! body of at least 600 ink characters of plain text at 7 points times the variant's size
//! factor; every other size and every height is relative to it. All text is invented.

use crate::axes::{ALL, Axes, Data, Look, Next, Pages, Shape, Sheet, Variant};
use crate::common::{FormPage, font_width, show};
use crate::shapes::Built;

/// One line of the body text: small words, no word of a value of the shapes.
const BODY_LINE: &str = "die Firma liefert die bestellte Ware nach den vereinbarten Bedingungen";

/// The axes of a business page: no data layer, no label font, no neighbour (its shapes run
/// over the width of the page), and rules only where the shape draws them.
const DOC: Axes = Axes {
    data: &[Data::CourierBold, Data::Proportional],
    labels: &[crate::axes::Labels::Helvetica],
    look: &[Look::Real],
    next: &[Next::None],
    ..ALL
};

/// The looks of the rules of a business page: stroked lines, dashed, dotted, and bars of a
/// point.
const RULED: Axes = Axes {
    look: &[Look::Real, Look::Dashed, Look::Dotted, Look::Thick],
    ..DOC
};

/// A page of `template` over the body, in the variant's fonts.
fn doc_page(v: &Variant, template: String, body_top: f32) -> Built {
    let (plain, _) = v.faces();
    let size = 7.0 * v.k();
    let mut t = template;
    let mut ink = 0;
    let mut y = body_top;
    while ink < 600 {
        t += &show(plain, size, 40.0, y, BODY_LINE);
        ink += BODY_LINE.chars().filter(|c| !c.is_whitespace()).count();
        y -= size * 1.3;
    }
    let mut b = Built::new(&v.geo());
    b.page = FormPage {
        template: t,
        ..FormPage::default()
    };
    b
}

/// `text` in `font` at `size`, its right end at `x`.
fn right(font: &str, size: f32, x: f32, y: f32, text: &str) -> String {
    show(font, size, x - font_width(font, text, size), y, text)
}

/// A horizontal rule from `x0` to `x1` at `y` as the variant draws it; `size` sets the dashes.
fn rule(v: &Variant, x0: f32, x1: f32, y: f32, size: f32) -> String {
    match v.look {
        Look::Thick => format!("{x0} {} {} 1 re f\n", y - 0.5, x1 - x0),
        Look::Dashed => {
            let (dash, step) = (0.2 * size, 0.3 * size);
            let mut out = String::new();
            let mut x = x0;
            while x < x1 {
                out += &format!("{x} {} {} 0.5 re f\n", y - 0.25, dash.min(x1 - x));
                x += step;
            }
            out
        }
        Look::Dotted => format!(
            "q 0.5 w 1 J [0 {}] 0 d {x0} {y} m {x1} {y} l S Q\n",
            0.15 * size
        ),
        _ => crate::common::hrule(x0, x1, y),
    }
}

fn owned(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|c| c.to_string()).collect())
        .collect()
}

/// The line items of [`items`].
const ITEMS: [[&str; 5]; 3] = [
    ["1", "Beratung", "2", "500,00", "1.000,00"],
    ["2", "Material", "1", "200,00", "200,00"],
    ["3", "Versand", "1", "34,56", "34,56"],
];

/// Task 10: line items at 8 points times the size factor, the bold header `Pos`, `Artikel`,
/// `Menge` at x 40, 80 and 300 and `Preis` and `Betrag` flush right at x 420 and 540, a rule
/// under the header, and with `end` a rule under the last item over three totals (labels at x
/// 340, values flush right at 540), else a rule under every item but the last. The items are
/// one table, and the totals another.
fn items(v: &Variant, end: bool) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let y = |dy: f32| 600.0 - dy * k;
    let mut t = show(bold, size, 40.0, y(0.0), "Pos")
        + &show(bold, size, 80.0, y(0.0), "Artikel")
        + &show(bold, size, 300.0, y(0.0), "Menge")
        + &right(bold, size, 420.0, y(0.0), "Preis")
        + &right(bold, size, 540.0, y(0.0), "Betrag");
    t += &rule(v, 40.0, 540.0, y(4.0), size);
    for (n, item) in ITEMS.iter().enumerate() {
        let at = y(10.0 * (n as f32 + 1.0));
        t += &show(plain, size, 40.0, at, item[0]);
        t += &show(plain, size, 80.0, at, item[1]);
        t += &show(plain, size, 300.0, at, item[2]);
        t += &right(plain, size, 420.0, at, item[3]);
        t += &right(plain, size, 540.0, at, item[4]);
        if !end && n < 2 {
            t += &rule(v, 40.0, 540.0, y(10.0 * (n as f32 + 1.0) + 4.0), size);
        }
    }
    let totals = [
        ("Nettobetrag", "1.234,56"),
        ("USt 19 %", "234,57"),
        ("Gesamtbetrag", "1.469,13"),
    ];
    if end {
        t += &rule(v, 40.0, 540.0, y(36.0), size);
        for (n, (label, value)) in totals.iter().enumerate() {
            let at = y(44.0 + 10.0 * n as f32);
            let font = if n == 2 { bold } else { plain };
            t += &show(font, size, 340.0, at, label);
            t += &right(font, size, 540.0, at, value);
        }
    }
    let mut s = Sheet::new(doc_page(v, t, 420.0));
    let mut table = vec![vec![
        "Pos".to_string(),
        "Artikel".into(),
        "Menge".into(),
        "Preis".into(),
        "Betrag".into(),
    ]];
    table.extend(
        ITEMS
            .iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect()),
    );
    s.want.tables.push(table);
    if end {
        s.want.tables.push(
            totals
                .iter()
                .map(|(l, v)| vec![l.to_string(), v.to_string()])
                .collect(),
        );
    }
    s
}

/// Task 11: line items whose two close titles `Menge Einheit` are one show over the
/// quantities and the units: the title is cut at the gutter of its two columns.
fn close_titles(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let mut t = show(bold, size, 40.0, 600.0, "Pos")
        + &show(bold, size, 80.0, 600.0, "Artikel")
        + &show(bold, size, 300.0, 600.0, "Menge Einheit")
        + &right(bold, size, 540.0, 600.0, "Betrag");
    let rows = [
        ["1", "Beratung", "2", "Std", "1.000,00"],
        ["2", "Material", "1", "Stk", "200,00"],
        ["3", "Versand", "1", "Stk", "34,56"],
    ];
    // The units stand where `Einheit` starts in the title.
    let unit_x = 300.0 + font_width(bold, "Menge ", size);
    for (r, row) in rows.iter().enumerate() {
        let y = 600.0 - 10.0 * k * (r as f32 + 1.0);
        t += &show(plain, size, 40.0, y, row[0]);
        t += &show(plain, size, 80.0, y, row[1]);
        t += &show(plain, size, 300.0, y, row[2]);
        t += &show(plain, size, unit_x, y, row[3]);
        t += &right(plain, size, 540.0, y, row[4]);
    }
    let mut s = Sheet::new(doc_page(v, t, 420.0));
    let mut table = owned(&[&["Pos", "Artikel", "Menge", "Einheit", "Betrag"]]);
    table.extend(owned(&rows.iter().map(|r| &r[..]).collect::<Vec<_>>()));
    s.want.tables.push(table);
    s
}

/// The field lines of [`title_beside_fields`].
const TITLE_FIELDS: [[&str; 2]; 4] = [
    ["Rechnungsnummer", "0000-0001"],
    ["Kundennummer", "00000"],
    ["Datum", "01.02.2026"],
    ["Seite", "1 von 1"],
];

/// Tasks 8 and 12: the bold title `Rechnung` at 9 points times the size factor at x 40,
/// beside four field lines of 6 points at x 360 (labels) and 80 points further (values), its
/// baseline 1.5 points under that of the first: the title stands apart, the fields are one
/// table, the title is the document's and the `Datum` field gives its date.
fn title_beside_fields(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let fs = 6.0 * k;
    let base = |i: usize| 703.0 - 8.0 * k * i as f32;
    let mut t = show(bold, 1.5 * fs, 40.0, base(0) - 1.5 * k, "Rechnung");
    for (i, [label, value]) in TITLE_FIELDS.iter().enumerate() {
        t += &show(plain, fs, 360.0, base(i), label);
        t += &show(plain, fs, 360.0 + 80.0 * k, base(i), value);
    }
    let mut s = Sheet::new(doc_page(v, t, 500.0));
    s.want.tables.push(
        TITLE_FIELDS
            .iter()
            .map(|f| f.iter().map(|c| c.to_string()).collect())
            .collect(),
    );
    s.want
        .apart
        .push(("Rechnung".into(), "Rechnungsnummer".into()));
    s.want.title = Some("Rechnung".into());
    s.want.date = Some("2026-02-01".into());
    s
}

/// Tasks 7 and 8: a letter head. The recipient's address in bold at 9 points, three lines
/// (one paragraph, no heading, no title), or with `larger_name` the name at 10 points over
/// two plain lines at 8; the date line `Datum 01.02.2026` at the right over the name, or with
/// `date_beside` on the name's baseline; the bold title `Rechnung` at 9 (the document's
/// title), a bold amount line at 9 (no heading), a line whose word carries a star, and at the
/// foot the star's legend at 6 (a footnote). Every size is the variant's size factor times
/// these.
fn letter_head(v: &Variant, larger_name: bool, date_beside: bool) -> Sheet {
    let k = v.k();
    let date_y = if date_beside { 720.0 } else { 760.0 };
    let (plain, bold) = v.faces();
    let s8 = 8.0 * k;
    let (name, lines, font) = if larger_name {
        (10.0 * k, s8, plain)
    } else {
        (9.0 * k, 9.0 * k, bold)
    };
    let step = lines * 1.3;
    let t = show(bold, name, 40.0, 720.0, "Erika Mustermann")
        + &show(font, lines, 40.0, 720.0 - step, "Musterweg 1")
        + &show(font, lines, 40.0, 720.0 - 2.0 * step, "12345 Musterstadt")
        + &show(plain, s8, 400.0, date_y, "Datum")
        + &show(plain, s8, 400.0 + 40.0 * k, date_y, "01.02.2026")
        + &show(bold, 9.0 * k, 40.0, 640.0, "Rechnung")
        + &show(bold, 9.0 * k, 40.0, 600.0, "Gesamtbetrag 2.345,67 EUR")
        + &show(plain, 9.0 * k, 40.0, 580.0, "Betrag* 1.234,56")
        + &show(plain, 6.0 * k, 40.0, 60.0, "* steuerfrei");
    let mut s = Sheet::new(doc_page(v, t, 500.0));
    s.want.title = Some("Rechnung".into());
    s.want.date = Some("2026-02-01".into());
    s.want.no_headings = vec!["Erika Mustermann".into(), "Gesamtbetrag".into()];
    s.want.roles.push(("footnote".into(), "steuerfrei".into()));
    s.want
        .paragraphs
        .push("Erika Mustermann Musterweg 1 12345 Musterstadt".into());
    s
}

/// The columns of [`footer`]: address, bank, register and contact under a bold shop name.
const FOOTER: [&[&str]; 4] = [
    &[
        "Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt",
        "Deutschland",
    ],
    &[
        "Musterbank",
        "IBAN DE00 0000 0000 0000 0000 00",
        "BIC MUSTDEXXXXX",
    ],
    &[
        "Amtsgericht Musterstadt",
        "HRB 00000",
        "USt-IdNr. DE000000000",
        "Erika Mustermann",
    ],
    &[
        "Muster Shop",
        "Tel. 0000 000000",
        "E-Mail info@muster.example",
    ],
];

/// Task 13: a footer of four columns at 7 points times the size factor, from the baseline 70
/// down at 1.2 sizes, each column three sizes right of the widest line of the one before. On
/// two pages each column is one running footer block in column order; on one page they are
/// content, and no block holds lines of two columns.
fn footer(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 7.0 * k;
    let mut t = String::new();
    let mut x = 40.0;
    for (c, lines) in FOOTER.iter().enumerate() {
        let mut widest: f32 = 0.0;
        for (n, line) in lines.iter().enumerate() {
            let font = if c == 3 && n == 0 { bold } else { plain };
            t += &show(font, size, x, 70.0 - 1.2 * size * n as f32, line);
            widest = widest.max(font_width(font, line, size));
        }
        x += widest + 3.0 * size;
    }
    let mut s = Sheet::new(doc_page(v, t, 500.0));
    for lines in FOOTER {
        s.built.values.extend(lines.iter().map(|l| l.to_string()));
        if v.pages == Pages::Two {
            s.want.roles.push(("page-footer".into(), lines.join(" ")));
        }
    }
    for (a, b) in [(0, 1), (1, 2), (2, 3)] {
        s.want
            .apart
            .push((FOOTER[a][1].to_string(), FOOTER[b][1].to_string()));
    }
    s
}

/// The columns of [`address_columns`].
const COLUMNS: [&[&str]; 4] = [
    &["Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
    &[
        "Musterbank",
        "IBAN DE00 0000 0000 0000 0000 00",
        "BIC MUSTDEXXXXX",
    ],
    &[
        "Amtsgericht Musterstadt",
        "HRB 00000",
        "USt-IdNr. DE000000000",
    ],
    &[
        "Tel. 0000 000000",
        "E-Mail info@muster.example",
        "Web www.muster.example",
    ],
];

/// Task 13 (carry C-5): address, bank, register and contact columns of three lines at 7
/// points times the size factor in the body, top aligned at x 40, 180, 330 and 460, the first
/// baseline 650 and 1.2 sizes apart: no block holds lines of two columns.
fn address_columns(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 7.0 * k;
    let mut t = String::new();
    for (lines, x) in COLUMNS.iter().zip([40.0, 180.0, 330.0, 460.0]) {
        for (n, line) in lines.iter().enumerate() {
            t += &show(plain, size, x, 650.0 - 1.2 * size * n as f32, line);
        }
    }
    let mut s = Sheet::new(doc_page(v, t, 500.0));
    for lines in COLUMNS {
        s.built.values.extend(lines.iter().map(|l| l.to_string()));
    }
    for (a, b) in [(0, 1), (1, 2), (2, 3)] {
        for (l, r) in COLUMNS[a].iter().zip(COLUMNS[b]) {
            s.want.apart.push((l.to_string(), r.to_string()));
        }
    }
    s
}

/// The shapes of business documents.
pub(crate) fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            name: "letter head",
            axes: DOC,
            draw: |v| (letter_head(v, false, false), None),
        },
        Shape {
            name: "letter head with a larger name",
            axes: DOC,
            draw: |v| (letter_head(v, true, false), None),
        },
        Shape {
            name: "letter head with the date beside the name",
            axes: DOC,
            draw: |v| (letter_head(v, false, true), None),
        },
        Shape {
            name: "line items with an end rule",
            axes: RULED,
            draw: |v| (items(v, true), None),
        },
        Shape {
            name: "line items with row rules",
            axes: RULED,
            draw: |v| (items(v, false), None),
        },
        Shape {
            name: "close column titles",
            axes: DOC,
            draw: |v| (close_titles(v), None),
        },
        Shape {
            name: "title beside a field list",
            axes: DOC,
            draw: |v| (title_beside_fields(v), None),
        },
        Shape {
            name: "address columns in the body",
            axes: DOC,
            draw: |v| (address_columns(v), None),
        },
        Shape {
            name: "footer columns",
            axes: DOC,
            draw: |v| (footer(v), None),
        },
    ]
}
