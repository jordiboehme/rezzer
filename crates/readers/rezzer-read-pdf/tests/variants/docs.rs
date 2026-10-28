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

/// The axes of a business page: no data layer, no label font, no neighbour, and rules only
/// where the shape draws them.
const DOC: Axes = Axes {
    data: &[Data::CourierBold, Data::Proportional],
    labels: &[crate::axes::Labels::Helvetica],
    look: &[Look::Real],
    next: &[Next::None],
    ..ALL
};

/// The axes of a business page with its neighbours over and under it.
const DOC_NEXT: Axes = Axes {
    next: &[Next::None, Next::TitleOver, Next::LineOver, Next::RowUnder],
    ..DOC
};

/// The looks of the rules of a business page: stroked lines, dashed, dotted, and bars of a
/// point.
const RULED: Axes = Axes {
    look: &[Look::Real, Look::Dashed, Look::Dotted, Look::Thick],
    ..DOC_NEXT
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

/// Where a business shape stands, for its neighbours: the baselines of its first and its last
/// line.
#[derive(Debug, Clone, Copy)]
struct Span {
    top: f32,
    bottom: f32,
}

/// The page of a business shape: `template`, the variant's neighbour next to the shape at
/// `span` (a bold title or a line over it, a table of three columns under it; none beside, as
/// the shapes run over the width of the page), and the body from `body_top`. Every text shown
/// on the page is a value to find as many times as it is drawn.
fn sheet(v: &Variant, template: String, body_top: f32, span: Option<Span>) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let mut t = template;
    let mut want = crate::axes::Want::default();
    if let Some(at) = span {
        match v.next {
            Next::TitleOver => {
                let title = "Monatsabrechnung Muster";
                t += &show(bold, 14.0 * k, 40.0, at.top + 30.0 * k, title);
                want.may_title.push(title.into());
            }
            Next::LineOver => {
                t += &show(
                    plain,
                    8.0 * k,
                    40.0,
                    at.top + 16.0 * k,
                    "Abrechnung Oktober 2026",
                );
            }
            Next::RowUnder => {
                let rows = [
                    ["Kennung", "Vorgang", "Wert"],
                    ["7001", "Fahrgeld", "45,67"],
                    ["7002", "Essensgeld", "8,90"],
                ];
                for (n, row) in rows.iter().enumerate() {
                    let y = at.bottom - 24.0 * k - 10.0 * k * n as f32;
                    let font = if n == 0 { bold } else { plain };
                    for (x, c) in [40.0, 200.0, 400.0].iter().zip(row) {
                        t += &show(font, 8.0 * k, *x, y, c);
                    }
                }
                want.tables
                    .push(owned(&rows.iter().map(|r| &r[..]).collect::<Vec<_>>()));
            }
            Next::None | Next::Beside => {}
        }
    }
    let mut s = Sheet::new(doc_page(v, t, body_top));
    s.want = want;
    s.once = crate::axes::shown(&s.built.page.template)
        .into_iter()
        .filter(|x| !x.contains('*'))
        .collect();
    s
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
/// under the header (unless `header_rule` is false), and with `end` a rule under the last
/// item over three totals (labels at x 340, values flush right at 540), else a rule under
/// every item but the last. With `note` (Task 15b part 2d fix round 1, the private invoice) a
/// smaller note stands under the totals over a rule of its own, so the totals stand between two
/// rules in three styles. The items are one table, and the totals another.
fn items(v: &Variant, end: bool, header_rule: bool, note: bool) -> Sheet {
    items_sized(v, end, header_rule, note, (1.0, 1.0))
}

/// [`items`] with the first totals row and the last one `first` and `last` times the size of
/// the others (Task 15b part 2d fix round 3, C1: totals not of one size between rules).
fn items_sized(
    v: &Variant,
    end: bool,
    header_rule: bool,
    note: bool,
    (first, last): (f32, f32),
) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let y = |dy: f32| 600.0 - dy * k;
    let mut t = show(bold, size, 40.0, y(0.0), "Pos")
        + &show(bold, size, 80.0, y(0.0), "Artikel")
        + &show(bold, size, 300.0, y(0.0), "Menge")
        + &right(bold, size, 420.0, y(0.0), "Preis")
        + &right(bold, size, 540.0, y(0.0), "Betrag");
    if header_rule {
        t += &rule(v, 40.0, 540.0, y(4.0), size);
    }
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
            let fs = size
                * match n {
                    0 => first,
                    2 => last,
                    _ => 1.0,
                };
            t += &show(font, fs, 340.0, at, label);
            t += &right(font, fs, 540.0, at, value);
        }
    }
    if note {
        t += &show(
            plain,
            0.8 * size,
            40.0,
            y(72.0),
            "Zahlbar innerhalb von 14 Tagen ohne Abzug.",
        );
        t += &rule(v, 40.0, 540.0, y(80.0), size);
    }
    let bottom = if note {
        y(80.0)
    } else if end {
        y(64.0)
    } else {
        y(30.0)
    };
    let mut s = sheet(
        v,
        t,
        420.0,
        Some(Span {
            top: y(0.0),
            bottom,
        }),
    );
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

/// A row of totals of [`totals_shape`]: its cells (text, x, flush right), empty cells left out.
type TotalsRow = &'static [(&'static str, f32, bool)];

/// Task 15b part 2d fix round 5 (R1): line items over totals of another shape between the rule
/// under the items and a rule under a smaller note: `rows`, the last one (the total) bold and
/// `last` times the size. The totals are one table of their rows.
fn totals_shape(v: &Variant, rows: &[TotalsRow], last: f32) -> Sheet {
    totals_drawn(v, rows, last, Under::Note)
}

/// What stands under the totals of [`totals_drawn`] (part 2e-1, item 5).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Under {
    /// A smaller note over a rule of its own.
    Note,
    /// A rule right under the totals.
    Rule,
    /// A rule under the note, and the first row bold: titles over their amounts.
    Titles,
}

/// [`totals_shape`] with `under` the totals.
fn totals_drawn(v: &Variant, rows: &[TotalsRow], last: f32, under: Under) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let y = |dy: f32| 600.0 - dy * k;
    let mut t = show(bold, size, 40.0, y(0.0), "Pos")
        + &show(bold, size, 80.0, y(0.0), "Artikel")
        + &right(bold, size, 540.0, y(0.0), "Betrag");
    for (n, item) in ITEMS.iter().enumerate() {
        let at = y(10.0 * (n as f32 + 1.0));
        t += &show(plain, size, 40.0, at, item[0]);
        t += &show(plain, size, 80.0, at, item[1]);
        t += &right(plain, size, 540.0, at, item[4]);
    }
    t += &rule(v, 40.0, 540.0, y(36.0), size);
    let mut table: Vec<Vec<String>> = Vec::new();
    for (n, cells) in rows.iter().enumerate() {
        let total = n + 1 == rows.len();
        let heads = if under == Under::Titles {
            n == 0
        } else {
            total
        };
        let font = if heads { bold } else { plain };
        let fs = if total { last * size } else { size };
        let at = y(44.0 + 10.0 * n as f32);
        for &(text, x, flush) in cells.iter().filter(|c| !c.0.is_empty()) {
            t += &if flush {
                right(font, fs, x, at, text)
            } else {
                show(font, fs, x, at, text)
            };
        }
        table.push(cells.iter().map(|c| c.0.to_string()).collect());
    }
    let low = 44.0 + 10.0 * rows.len() as f32;
    if under == Under::Rule {
        t += &rule(v, 40.0, 540.0, y(low - 1.0), size);
    } else {
        t += &show(
            plain,
            0.8 * size,
            40.0,
            y(low + 6.0),
            "Zahlbar innerhalb von 14 Tagen ohne Abzug.",
        );
        t += &rule(v, 40.0, 540.0, y(low + 14.0), size);
    }
    let mut s = sheet(
        v,
        t,
        420.0,
        Some(Span {
            top: y(0.0),
            bottom: y(low + 14.0),
        }),
    );
    s.want.tables.push(table);
    s
}

/// The totals of [`totals_shape`]: a currency column, the label right of its amount, amounts
/// without decimals, and a percent column in every row.
const CURRENCY_TOTALS: [TotalsRow; 3] = [
    &[
        ("Nettobetrag", 300.0, false),
        ("EUR", 450.0, false),
        ("1.234,56", 540.0, true),
    ],
    &[
        ("zzgl. USt. 19 %", 300.0, false),
        ("EUR", 450.0, false),
        ("234,57", 540.0, true),
    ],
    &[
        ("Gesamt", 300.0, false),
        ("EUR", 450.0, false),
        ("1.469,13", 540.0, true),
    ],
];
const LABEL_RIGHT_TOTALS: [TotalsRow; 3] = [
    &[("1.234,56 EUR", 400.0, true), ("Nettobetrag", 420.0, false)],
    &[
        ("234,57 EUR", 400.0, true),
        ("zzgl. USt. 19 %", 420.0, false),
    ],
    &[("1.469,13 EUR", 400.0, true), ("Gesamt", 420.0, false)],
];
const WHOLE_TOTALS: [TotalsRow; 3] = [
    &[("Nettobetrag", 300.0, false), ("1.200 EUR", 540.0, true)],
    &[("zzgl. USt. 19 %", 300.0, false), ("228 EUR", 540.0, true)],
    &[("Gesamt", 300.0, false), ("1.428 EUR", 540.0, true)],
];
const PERCENT_TOTALS: [TotalsRow; 3] = [
    &[
        ("USt.", 300.0, false),
        ("7 %", 450.0, true),
        ("1,00", 540.0, true),
    ],
    &[
        ("USt.", 300.0, false),
        ("19 %", 450.0, true),
        ("19,00", 540.0, true),
    ],
    &[
        ("Gesamt", 300.0, false),
        ("", 450.0, true),
        ("120,00", 540.0, true),
    ],
];

/// Part 2e-1 (item 5): plain totals of a label and an amount with its currency.
const PLAIN_TOTALS: [TotalsRow; 3] = [
    &[("Nettobetrag", 300.0, false), ("1.234,56 EUR", 540.0, true)],
    &[
        ("zzgl. USt. 19 %", 300.0, false),
        ("234,57 EUR", 540.0, true),
    ],
    &[("Gesamt", 300.0, false), ("1.469,13 EUR", 540.0, true)],
];
/// Part 2e-1 (item 5): a percent column in one row only.
const PERCENT_ONE_ROW_TOTALS: [TotalsRow; 3] = [
    &[
        ("Nettobetrag", 300.0, false),
        ("", 450.0, true),
        ("1.234,56", 540.0, true),
    ],
    &[
        ("USt.", 300.0, false),
        ("19 %", 450.0, true),
        ("234,57", 540.0, true),
    ],
    &[
        ("Gesamt", 300.0, false),
        ("", 450.0, true),
        ("1.469,13", 540.0, true),
    ],
];
/// Part 2e-1 (item 5): the net, tax and gross titles over their amounts.
const NET_TAX_GROSS_TOTALS: [TotalsRow; 2] = [
    &[
        ("Netto", 300.0, false),
        ("USt.", 450.0, true),
        ("Brutto", 540.0, true),
    ],
    &[
        ("1.234,56", 300.0, false),
        ("234,57", 450.0, true),
        ("1.469,13", 540.0, true),
    ],
];

/// Part 2e-1 fix round 1 (the private notice): a block of five lines at one left edge beside
/// a letter address, each a label at 8 points times the size factor and its bold value after
/// it, with a box around the lines `boxed` (none: no box). No line of the read holds the value
/// of one line and the label of the next: each line is a field of its own.
fn field_lines(v: &Variant, boxed: Option<(usize, usize)>) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let pitch = 1.2 * size;
    let (x0, top) = (320.0, 640.0);
    let lines = [
        ("Ihr Zeichen vom", "12 Monate"),
        ("Unser Zeichen vom", "Muster"),
        ("Ort der", "Sitz"),
        ("Ab", "Datum"),
        ("Az.-Nr. Kd/Ref", "12345678 Muster GmbH"),
    ];
    let mut t = String::new();
    for (n, (l, val)) in lines.iter().enumerate() {
        let y = top - pitch * n as f32;
        t += &show(plain, size, x0, y, l);
        t += &show(
            bold,
            size,
            x0 + font_width(plain, l, size) + 0.28 * size,
            y,
            val,
        );
    }
    if let Some((a, b)) = boxed {
        let y0 = top - pitch * (b as f32 - 1.0) - 0.4 * size;
        let y1 = top - pitch * a as f32 + size;
        t += &format!("0 0 0 RG 0.5 w {} {y0} 240 {} re S\n", x0 - 4.0, y1 - y0);
    }
    for (n, l) in ["Erika Mustermann", "Beispielweg 2", "54321 Beispielstadt"]
        .iter()
        .enumerate()
    {
        t += &show(plain, size, 40.0, top - pitch * n as f32, l);
    }
    let bottom = top - pitch * 5.0;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    for w in lines.windows(2) {
        s.want
            .lines_apart
            .push((w[0].1.to_string(), w[1].0.to_string()));
    }
    s
}

/// Part 2e-1 fix round 2 (review R1 and R2): a column of five short lines of one face at one
/// left edge beside a letter address, at `size` points times the size factor in the bold face
/// (`bold`) or the plain one, with a box around the lines `boxed` (none: no box). No line of
/// the read, and no heading, holds two of them.
fn one_face_column(v: &Variant, size: f32, bold: bool, boxed: Option<(usize, usize)>) -> Sheet {
    let lines = [
        "Kundennummer 12345",
        "Rechnungsnummer 67890",
        "Datum 01.10.2026",
        "Seite 1 von 2",
        "Bearbeiter Muster",
    ];
    let mut s = column_of(v, &lines, size, bold, boxed);
    // `Datum` labels the date of the document.
    s.want.date = Some("2026-10-01".into());
    s
}

/// Part 2e-2 (item 4, O2): field lines of one face of five to seven words each.
const LONG_FIELD_LINES: [&str; 5] = [
    "Zahlbar bis zum 31.10.2026 ohne Abzug",
    "Bearbeiter Frau Erika Beispiel",
    "Ihre Kundennummer bei uns 12345",
    "Unser Zeichen AB 12 vom 01.10.2026",
    "Seite 1 von 2",
];

/// Part 2e-2 (item 4, O1): slogan lines, each a sentence of 8 to 10 words ending in an
/// exclamation mark.
const SLOGAN_LINES: [&str; 3] = [
    "Jetzt umsteigen und bei jeder Fahrt bares Geld sparen!",
    "Einfach online buchen - ganz ohne Papier und Wartezeit!",
    "Wir sind auch am Wochenende gerne für Sie da!",
];

/// Part 2e-2 (item 5, O3): lines of starred items.
const STARRED_LINES: [&str; 3] = ["* A123456B * x/y", "*Abc.-Nr. 12345* 12a", "*Abc.-Nr. ab*"];

/// The lines `lines` of one face as a column at one left edge beside a letter address (see
/// [`one_face_column`]), each line's text in one show: no line of the read, and no heading,
/// holds two of them.
fn column_of(
    v: &Variant,
    lines: &[&str],
    size: f32,
    bold: bool,
    boxed: Option<(usize, usize)>,
) -> Sheet {
    let k = v.k();
    let (plain, heavy) = v.faces();
    let font = if bold { heavy } else { plain };
    let size = size * k;
    let pitch = 1.2 * size;
    let (x0, top) = (320.0, 640.0);
    let mut t = String::new();
    for (n, l) in lines.iter().enumerate() {
        t += &show(font, size, x0, top - pitch * n as f32, l);
    }
    if let Some((a, b)) = boxed {
        let y0 = top - pitch * (b as f32 - 1.0) - 0.4 * size;
        let y1 = top - pitch * a as f32 + size;
        t += &format!("0 0 0 RG 0.5 w {} {y0} 240 {} re S\n", x0 - 4.0, y1 - y0);
    }
    for (n, l) in ["Erika Mustermann", "Beispielweg 2", "54321 Beispielstadt"]
        .iter()
        .enumerate()
    {
        t += &show(plain, 7.0 * k, 40.0, top - 8.4 * k * n as f32, l);
    }
    let bottom = top - pitch * lines.len() as f32;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    for w in lines.windows(2) {
        s.want
            .lines_apart
            .push((w[0].to_string(), w[1].to_string()));
    }
    s
}

/// Part 2e-2 (item 6, the docling report's glues): rows of a table read as text, a row label
/// cut by a hyphen (`Page-`) over the cells of the next row, which start right of the cut, and
/// a row whose last cell is a number range over a line of prose, all in the body's size: no
/// line of the read holds the cut label and the next row, or the range and the prose.
fn cut_rows(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 7.0 * k;
    let pitch = 1.2 * size;
    let top = 640.0;
    let w = font_width(plain, "Page-", size);
    let mut t = show(plain, size, 40.0, top, "Page-");
    t += &show(
        plain,
        size,
        40.0 + w + 2.0 * size,
        top - pitch,
        "70878 6.51 5.58 6.00",
    );
    t += &show(
        plain,
        size,
        40.0,
        top - 4.0 * pitch,
        "Text 510377 45.82 49.28 70-85",
    );
    t += &show(
        plain,
        size,
        40.0,
        top - 5.0 * pitch,
        "Preparation work included uploading the files",
    );
    let bottom = top - 6.0 * pitch;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want
        .lines_apart
        .push(("Page-".into(), "70878 6.51 5.58 6.00".into()));
    s.want.lines_apart.push((
        "Text 510377 45.82 49.28 70-85".into(),
        "Preparation work included uploading the files".into(),
    ));
    s
}

/// Part 2e-2 (item 2): two lines in three columns alone in the body, titles over one row of
/// values, at 8 points times the size factor: a table of a header and one row.
fn two_lines_in_columns(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 640.0;
    let rows = [["Name", "Ort", "Nr"], ["Beispiel", "Musterstadt", "4711"]];
    let mut t = String::new();
    for (n, row) in rows.iter().enumerate() {
        for (x, c) in [40.0f32, 200.0, 400.0].iter().zip(row) {
            t += &show(plain, size, *x, top - 1.3 * size * n as f32, c);
        }
    }
    let bottom = top - 2.6 * size;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want.tables.push(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect(),
    );
    s
}

/// Part 2e-2 (item 3): a value printed over the label of its line, at 9 points times the size
/// factor, the label at 0.7 of it: two texts, never one word.
fn overprinted(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 9.0 * k;
    let top = 640.0;
    let mut t = show(plain, 0.7 * size, 40.0, top, "Name");
    t += &show(
        bold,
        size,
        40.0 + 0.3 * size,
        top + 0.3 * size,
        "Hinterhaus",
    );
    let mut s = sheet(
        v,
        t,
        420.0,
        Some(Span {
            top,
            bottom: top - size,
        }),
    );
    s.want.paragraphs.push("Name Hinterhaus".into());
    s
}

/// Part 2e-2 fix round 1 (I1): prose of sentences at 8 points times the size factor whose
/// lines end in a number over a noun (`mit 2` over `Prozent Skonto`), and a two-line paragraph
/// with no stop at its end: every line goes on in the next.
fn number_prose(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 640.0;
    let lines = [
        "Die Rechnung ist zahlbar innerhalb von 14 Tagen ohne Abzug oder mit 2",
        "Prozent Skonto innerhalb von 8 Tagen. Nach dem 31.10.2026 berechnen wir 5",
        "Prozentpunkte Zinsen ueber dem Basiszinssatz, wie es der Vertrag vorsieht.",
    ];
    let pair = [
        "Weitere Informationen zu unseren Angeboten erhalten Sie in jeder Filiale oder im",
        "Kundenportal der Muster GmbH",
    ];
    let mut t = String::new();
    for (n, l) in lines.iter().enumerate() {
        t += &show(plain, size, 40.0, top - 1.2 * size * n as f32, l);
    }
    let low = top - 5.0 * size;
    for (n, l) in pair.iter().enumerate() {
        t += &show(plain, size, 40.0, low - 1.2 * size * n as f32, l);
    }
    let mut s = sheet(
        v,
        t,
        420.0,
        Some(Span {
            top,
            bottom: low - 2.4 * size,
        }),
    );
    s.want
        .lines_joined
        .push(("mit 2".into(), "Prozent Skonto".into()));
    s.want
        .lines_joined
        .push(("wir 5".into(), "Prozentpunkte".into()));
    s.want
        .lines_joined
        .push(("oder im".into(), "Kundenportal".into()));
    s
}

/// Part 2e-2 fix round 1 (C2): two lines of three words set full in a narrow column (12 ems),
/// the second with a digit: prose, no header over a row.
fn justified_pair(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let top = 640.0;
    let m = 14.0 * size;
    let mut t = String::new();
    for (n, l) in ["Unsere Lieferung erfolgt", "in 2 Teilen."]
        .iter()
        .enumerate()
    {
        let words: Vec<&str> = l.split(' ').collect();
        let total: f32 = words.iter().map(|w| font_width(plain, w, size)).sum();
        let gap = (m - total) / (words.len() as f32 - 1.0);
        let mut x = 40.0;
        for w in &words {
            t += &show(plain, size, x, top - 1.2 * size * n as f32, w);
            x += font_width(plain, w, size) + gap;
        }
    }
    let mut s = sheet(
        v,
        t,
        420.0,
        Some(Span {
            top,
            bottom: top - 2.4 * size,
        }),
    );
    s.want.lines_joined.push(("erfolgt".into(), "in 2".into()));
    s
}

/// Part 2e-4 (item 2): a paragraph without a stop set full (justified) in a column of 14 ems,
/// three words or so a line, its word gaps wide enough to part the words into pieces: prose, no
/// table of three columns; each line goes on in the next.
fn justified_paragraph(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let top = 640.0;
    let m = 14.0 * size;
    let text = "Weitere Informationen zu unseren Angeboten und Leistungen erhalten Sie jederzeit \
                in unseren Filialen oder im Kundenportal der Muster GmbH";
    let mut lines: Vec<Vec<&str>> = vec![Vec::new()];
    for w in text.split_whitespace() {
        let mut cand = lines.last().cloned().unwrap_or_default();
        cand.push(w);
        if lines.last().is_some_and(Vec::is_empty) || font_width(plain, &cand.join(" "), size) <= m
        {
            if let Some(l) = lines.last_mut() {
                l.push(w);
            }
        } else {
            lines.push(vec![w]);
        }
    }
    let mut t = String::new();
    let n = lines.len();
    for (row, words) in lines.iter().enumerate() {
        let total: f32 = words.iter().map(|w| font_width(plain, w, size)).sum();
        let space = font_width(plain, " ", size);
        let gap = if row + 1 < n && words.len() > 1 {
            (m - total) / (words.len() as f32 - 1.0)
        } else {
            space
        };
        let mut x = 40.0;
        for w in words {
            t += &show(plain, size, x, top - 1.2 * size * row as f32, w);
            x += font_width(plain, w, size) + gap;
        }
    }
    let mut s = sheet(
        v,
        t,
        420.0,
        Some(Span {
            top,
            bottom: top - 1.2 * size * n as f32,
        }),
    );
    // Each word is drawn on its own: the text is found as the words of the paragraph.
    s.once
        .retain(|o| !text.split_whitespace().any(|w| w == o.trim()));
    s.once.push(text.into());
    for w in lines.windows(2) {
        if let (Some(a), Some(b)) = (w[0].last(), w[1].first()) {
            s.want.lines_joined.push((a.to_string(), b.to_string()));
        }
    }
    s
}

/// Part 2e-2 fix round 1 (C1): line items with amounts flush left, the last a credit with a
/// trailing minus (`10,00-`), over a total under the amounts at the row pitch: the total is no
/// end of the credit's cell.
fn credit_over_total(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let pitch = 1.25 * size;
    let top = 600.0;
    let mut y = top;
    let mut t = String::new();
    for (x, h) in [
        (40.0, "Pos"),
        (80.0, "Artikel"),
        (300.0, "Menge"),
        (480.0, "Betrag"),
    ] {
        t += &show(bold, size, x, y, h);
    }
    let rows = [
        ["1", "Schrauben", "10", "50,00"],
        ["2", "Muttern", "20", "40,00"],
        ["3", "Rabatt", "1", "10,00-"],
    ];
    let mut table = vec![vec![
        "Pos".to_string(),
        "Artikel".into(),
        "Menge".into(),
        "Betrag".into(),
    ]];
    for r in rows {
        y -= pitch;
        for (x, c) in [40.0, 80.0, 300.0, 480.0].iter().zip(r) {
            t += &show(plain, size, *x, y, c);
        }
        table.push(r.iter().map(|c| c.to_string()).collect());
    }
    y -= pitch;
    t += &show(plain, size, 480.0, y, "80,00");
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom: y }));
    s.want.tables.push(table);
    s
}

/// Part 2e-4 (item 3): line items with the amounts flush right at x 540 and the minus of the
/// last one, a credit, hanging after them (`10,00` and `-`), over a total flush right under
/// them: the total stays apart from the credit, on a line of its own, never glued
/// (`10,00-80,00`).
fn hanging_credit(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let pitch = 1.25 * size;
    let top = 600.0;
    let mut y = top;
    let mut t = String::new();
    for (x, h) in [
        (40.0, "Pos"),
        (80.0, "Artikel"),
        (300.0, "Menge"),
        (480.0, "Betrag"),
    ] {
        t += &show(bold, size, x, y, h);
    }
    for r in [
        ["1", "Schrauben", "10", "50,00"],
        ["2", "Muttern", "20", "40,00"],
        ["3", "Rabatt", "1", "10,00"],
    ] {
        y -= pitch;
        for (x, c) in [40.0, 80.0, 300.0].iter().zip(r) {
            t += &show(plain, size, *x, y, c);
        }
        t += &right(plain, size, 540.0, y, r[3]);
    }
    t += &show(plain, size, 540.0, y, "-");
    y -= pitch;
    t += &right(plain, size, 540.0, y, "80,00");
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom: y }));
    s.once.retain(|o| !matches!(o.trim(), "10,00" | "-"));
    s.once.push("10,00-".into());
    s.want.cells.push(vec!["10,00-".into()]);
    s.want.cells.push(vec!["80,00".into()]);
    s.want.lines_apart.push(("10,00-".into(), "80,00".into()));
    s
}

/// Part 2e-4 (item 3, list line 9): line items with the amounts flush right, a deduction with a
/// minus after it (`12,50-`) on a line of its own under the first amount: a row of its own,
/// never one cell with the amount over it.
fn stacked_deduction(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let pitch = 1.3 * size;
    let top = 600.0;
    let mut y = top;
    let mut t = String::new();
    for (x, h) in [
        (40.0, "Pos"),
        (80.0, "Bezeichnung"),
        (300.0, "Menge"),
        (500.0, "Betrag"),
    ] {
        t += &show(bold, size, x, y, h);
    }
    for r in [
        ["1", "Grundgehalt", "1", "2.500,00"],
        ["", "", "", "12,50-"],
        ["2", "Zulage", "2", "100,00"],
        ["3", "Fahrgeld", "1", "45,67"],
    ] {
        y -= pitch;
        for (x, c) in [40.0, 80.0, 300.0].iter().zip(r) {
            t += &show(plain, size, *x, y, c);
        }
        t += &right(plain, size, 540.0, y, r[3]);
    }
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom: y }));
    s.want
        .lines_apart
        .push(("2.500,00".into(), "12,50-".into()));
    s
}

/// Part 2e-2 fix round 1 (I3): an accent drawn as a run of its own over the first letter of
/// its word, after the word (`Uber`, the dieresis, ` den Vertrag`): it stays in its word.
fn lone_accent(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let top = 640.0;
    let u = font_width(plain, "Uber", size);
    let mut t = show(plain, size, 40.0, top, "Uber");
    t += &show(plain, size, 40.0 + 0.15 * size, top, "\u{a8}");
    t += &show(plain, size, 40.0 + u, top, " den Vertrag");
    let bottom = top - size;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    // The three runs read as one text, the mark in its word.
    s.once
        .retain(|o| !matches!(o.trim(), "Uber" | "\u{a8}" | "den Vertrag"));
    s.once.push("Uber\u{a8} den Vertrag".into());
    s.want
        .lines_joined
        .push(("Uber\u{a8}".into(), "den Vertrag".into()));
    s
}

/// Part 2e-2 fix round 1 (M4, item 3): a row between rules with dividers whose cells hold two
/// lines with no label each (a street over its town beside a firm over its street), lines
/// that line up by chance: no rows of totals; each cell is read whole. Part 2e-4 (item 1): with
/// `divided` false the row has no divider, and the cells are still read whole.
fn cells_by_chance(v: &Variant, divided: bool) -> Sheet {
    let cells = [
        ["Musterweg 1", "12345 Musterstadt"],
        ["Beispiel AG", "Probeweg 12"],
    ];
    let mut s = two_cells(v, divided, cells);
    for lines in cells {
        s.want
            .cells
            .push(lines.iter().map(|l| l.to_string()).collect());
    }
    s
}

/// Part 2e-4 (item 1): a row between rules with no divider whose two cells of two lines show
/// no sign that their lines go together (a name over a firm beside a firm over its street):
/// rows or cells, so the two lines read line by line and a loss says the doubt.
fn cells_or_rows(v: &Variant) -> Sheet {
    let cells = [
        ["Erika Mustermann", "Muster GmbH"],
        ["Beispiel AG", "Probeweg 12"],
    ];
    let mut s = two_cells(v, false, cells);
    for n in 0..2 {
        s.want
            .lines_joined
            .push((cells[0][n].to_string(), cells[1][n].to_string()));
    }
    s.want
        .losses
        .push("two lines of text in columns between two rules".into());
    s
}

/// Two `cells` of two lines between two rules at x 44 and 224, with a divider between them
/// when `divided`.
fn two_cells(v: &Variant, divided: bool, cells: [[&str; 2]; 2]) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 640.0;
    let h = 4.0 * size;
    let mut t = rule(v, 40.0, 400.0, top, size) + &rule(v, 40.0, 400.0, top - h, size);
    if divided {
        t += &format!("0 0 0 RG 0.5 w 220 {} m 220 {top} l S\n", top - h);
    }
    for (x, lines) in [44.0, 224.0].into_iter().zip(cells) {
        for (n, l) in lines.iter().enumerate() {
            t += &show(plain, size, x, top - 1.5 * size - 1.2 * size * n as f32, l);
        }
    }
    sheet(
        v,
        t,
        420.0,
        Some(Span {
            top,
            bottom: top - h,
        }),
    )
}

/// Part 2e-4 (item 4): titles over one row whose last cell ends with the stop of an
/// abbreviation (`Stk.`), alone in the body: a table of a header and one row.
fn titles_over_a_stop(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 640.0;
    let rows = [["Artikel", "Menge", "Einheit"], ["Schrauben", "12", "Stk."]];
    let mut t = String::new();
    for (n, row) in rows.iter().enumerate() {
        for (x, c) in [40.0f32, 200.0, 400.0].iter().zip(row) {
            t += &show(plain, size, *x, top - 1.3 * size * n as f32, c);
        }
    }
    let bottom = top - 1.3 * size;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want.tables.push(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect(),
    );
    s
}

/// Part 2e-2 fix round 1 (M4, item 2): two lines in three columns of one style between two
/// rules with no dividers, titles over one row of values: a table of a header and one row.
fn two_lines_in_a_strip(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 640.0;
    let rows = [["Name", "Ort", "Nr"], ["Beispiel", "Musterstadt", "4711"]];
    let mut t = rule(v, 36.0, 520.0, top + 1.2 * size, size);
    for (n, row) in rows.iter().enumerate() {
        for (x, c) in [40.0f32, 200.0, 400.0].iter().zip(row) {
            t += &show(plain, size, *x, top - 1.3 * size * n as f32, c);
        }
    }
    let bottom = top - 1.3 * size - 0.5 * size;
    t += &rule(v, 36.0, 520.0, bottom, size);
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want.tables.push(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect(),
    );
    s
}

/// Part 2e-1 (items 6 and 7): line items without rules at 8 points times the size factor, a
/// bold header `Pos`, `Zeitraum`, `Basis` and `Betrag`; with `range` the first two periods
/// wrap after their dash (`01.10.2026 -` over `31.10.2026`) and the cell keeps the space after
/// the dash; else the last row is followed by an amount alone under the base and one alone
/// under the amount, on two lines, which are never one block.
fn period_items(v: &Variant, range: bool) -> Sheet {
    period_items_of(v, range, false)
}

/// [`period_items`], with `last` the last period a range that wraps too (`01.12.2026 -` over
/// `31.12.2026`, part 2e-2 item 3): its second line, under the table, ends its cell.
fn period_items_of(v: &Variant, range: bool, last: bool) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let xs = [40.0, 80.0, 380.0 * k.max(0.9), 540.0];
    let mut y = 600.0;
    let mut t = show(bold, size, xs[0], y, "Pos")
        + &show(bold, size, xs[1], y, "Zeitraum")
        + &right(bold, size, xs[2], y, "Basis")
        + &right(bold, size, xs[3], y, "Betrag");
    let rows: [[&str; 5]; 3] = [
        ["1", "01.10.2026 -", "31.10.2026", "1.000,00", "100,00"],
        ["2", "01.11.2026 -", "30.11.2026", "500,00", "50,00"],
        if last {
            ["3", "01.12.2026 -", "31.12.2026", "200,00", "20,00"]
        } else {
            ["3", "Dezember", "", "200,00", "20,00"]
        },
    ];
    let mut table = vec![vec![
        "Pos".to_string(),
        "Zeitraum".into(),
        "Basis".into(),
        "Betrag".into(),
    ]];
    for (n, r) in rows.iter().enumerate() {
        y -= 10.0 * k;
        let (from, to) = if range || n == 2 {
            (r[1], r[2])
        } else {
            ("Oktober", "")
        };
        t += &show(plain, size, xs[0], y, r[0]);
        t += &show(plain, size, xs[1], y, from);
        t += &right(plain, size, xs[2], y, r[3]);
        t += &right(plain, size, xs[3], y, r[4]);
        if !to.is_empty() {
            y -= 10.0 * k;
            t += &show(plain, size, xs[1], y, to);
        }
        let period = if to.is_empty() {
            from.to_string()
        } else {
            format!("{from} {to}")
        };
        table.push(vec![r[0].into(), period, r[3].into(), r[4].into()]);
    }
    let top = 600.0;
    if !range {
        y -= 10.0 * k;
        t += &right(plain, size, xs[2], y, "5,00");
        y -= 10.0 * k;
        t += &right(plain, size, xs[3], y, "125,00");
    }
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom: y }));
    if range {
        s.want.tables.push(table);
    } else {
        s.want.apart.push(("5,00".into(), "125,00".into()));
    }
    s
}

/// Task 15b part 2d fix round 3: a notice band between two rules, a bold slogan 1.25 times
/// the size beside a reference at the size, over one long line of text 1.1 times the size (three
/// styles). It holds no field: no loss may call it a row of fields.
fn notice_band(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 9.0 * k;
    // Under the top third of the page: the slogan is no title of the page.
    let top = 520.0;
    let mut t = rule(v, 40.0, 540.0, top, size);
    t += &show(
        bold,
        1.25 * size,
        44.0,
        top - 1.5 * size,
        "Neu - jetzt online!",
    );
    t += &show(plain, size, 440.0, top - 1.5 * size, "Az. 12-345");
    t += &show(
        plain,
        1.1 * size,
        44.0,
        top - 3.0 * size,
        "Ihre Unterlagen finden Sie ab sofort bequem und sicher im Kundenportal unter Service.",
    );
    t += &rule(v, 40.0, 540.0, top - 4.0 * size, size);
    let mut s = sheet(
        v,
        t,
        380.0,
        Some(Span {
            top,
            bottom: top - 4.0 * size,
        }),
    );
    s.want.no_losses.push("row of fields".into());
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
    let span = Span {
        top: 600.0,
        bottom: 600.0 - 30.0 * k,
    };
    let mut s = sheet(v, t, 420.0, Some(span));
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
    let span = Span {
        top: base(0),
        bottom: base(3),
    };
    let mut s = sheet(v, t, 500.0, Some(span));
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
    // The third column of the table under it stands under the field list: when the reading
    // order puts it apart from the table, a loss says so.
    if v.next == Next::RowUnder {
        s.want.losses.push("may go on in a column beside it".into());
    }
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
    let span = Span {
        top: date_y.max(720.0),
        bottom: 580.0,
    };
    let mut s = sheet(v, t, 500.0, Some(span));
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
    let mut s = sheet(v, t, 500.0, None);
    for lines in FOOTER {
        s.built.values.extend(lines.iter().map(|l| l.to_string()));
        if v.pages == Pages::Two {
            s.want.roles.push(("page-footer".into(), lines.join(" ")));
        }
    }
    for (a, left) in FOOTER.iter().enumerate() {
        for right in &FOOTER[a + 1..] {
            for l in left.iter() {
                for r in right.iter() {
                    s.want.apart.push((l.to_string(), r.to_string()));
                }
            }
        }
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
/// baseline 650 and 1.2 sizes apart: no block holds lines of two columns (any line of one
/// with any line of another).
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
    let span = Span {
        top: 650.0,
        bottom: 650.0 - 2.4 * size,
    };
    let mut s = sheet(v, t, 500.0, Some(span));
    for lines in COLUMNS {
        s.built.values.extend(lines.iter().map(|l| l.to_string()));
    }
    for (a, left) in COLUMNS.iter().enumerate() {
        for right in &COLUMNS[a + 1..] {
            for l in left.iter() {
                for r in right.iter() {
                    s.want.apart.push((l.to_string(), r.to_string()));
                }
            }
        }
    }
    s
}

/// The rows of [`rows_shape`]: a small table whose columns list records under a plain
/// header, a headerless table of records, labels beside values with a last label alone, and
/// blocks side by side under a bold row of their headings.
const SMALL_TABLE: [&[&str]; 4] = [
    &["Artikel", "Lager", "Menge"],
    &["Schraube M4", "Halle 2", "12 Stueck"],
    &["Mutter M5", "Regal 3B", "8 Stueck"],
    &["Scheibe", "Halle 1", "20 Stueck"],
];
const RECORDS: [&[&str]; 3] = [
    &["Muster GmbH", "Lieferung 12", "3 Kartons"],
    &["Beispiel AG", "Lieferung 13", "12 Paletten"],
    &["Muster KG 2", "Ruecksendung", "1 Karton"],
];
const LABELS: [&[&str]; 4] = [
    &["Zimmer 2", "Raum 101"],
    &["Steuerjahr 2025", "Einzelveranlagung"],
    &["Bearbeitung", "Max Mustermann"],
    &["Aktenzeichen", ""],
];
const HEADED: [&[&str]; 4] = [
    &["Anschrift", "Bankverbindung", "Register"],
    &["Muster GmbH", "Musterbank", "Amtsgericht Musterstadt"],
    &[
        "Musterweg 1",
        "IBAN DE00 0000 0000 0000 0000 00",
        "HRB 00000",
    ],
    &[
        "12345 Musterstadt",
        "BIC MUSTDEXXXXX",
        "USt-IdNr. DE000000000",
    ],
];

/// What [`rows_shape`] draws.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Rows {
    /// [`SMALL_TABLE`]: one table, row by row.
    Table,
    /// [`RECORDS`]: one table, row by row.
    Records,
    /// [`LABELS`]: each value with its label, or a loss that says labels beside values are
    /// read apart.
    Labels,
    /// [`HEADED`], the first row bold: read block by block, or a table with a loss that says it
    /// may be blocks under their headings.
    Headed,
}

/// Task 15b part 2c review (Critical 1, Important 1, Minor 5): rows of 7 points times the size
/// factor at x 40, 180 and 330, 1.2 sizes apart from the baseline 650, as `what` says.
fn rows_shape(v: &Variant, what: Rows) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 7.0 * k;
    let rows: &[&[&str]] = match what {
        Rows::Table => &SMALL_TABLE,
        Rows::Records => &RECORDS,
        Rows::Labels => &LABELS,
        Rows::Headed => &HEADED,
    };
    let mut t = String::new();
    for (r, row) in rows.iter().enumerate() {
        let font = if what == Rows::Headed && r == 0 {
            bold
        } else {
            plain
        };
        for (cell, x) in row.iter().zip([40.0, 180.0, 330.0]) {
            if !cell.is_empty() {
                t += &show(font, size, x, 650.0 - 1.2 * size * r as f32, cell);
            }
        }
    }
    let span = Span {
        top: 650.0,
        bottom: 650.0 - 1.2 * size * (rows.len() - 1) as f32,
    };
    let mut s = sheet(v, t, 500.0, Some(span));
    for row in rows {
        s.built
            .values
            .extend(row.iter().filter(|c| !c.is_empty()).map(|c| c.to_string()));
    }
    match what {
        Rows::Table | Rows::Records => s.want.tables.push(owned(rows)),
        Rows::Labels => s.want.losses.push("labels beside values".into()),
        Rows::Headed => s.want.losses.push("under their headings".into()),
    }
    s
}

/// Task 15b part 2c review (Important 4, Minor 1): line items over a bold total, a blank row,
/// then a second table whose middle column ends where the first one's middle title ends. With
/// `plain` the second header is plain, else bold with years. At 8 points times the size
/// factor, the columns at x 40, 112 and flush right at 400 times the factor. Two tables.
fn two_tables(v: &Variant, plain: bool) -> Sheet {
    two_tables_or_note(v, if plain { Second::Plain } else { Second::Years }, 1.0)
}

/// What [`two_tables_or_note`] draws after the blank row.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Second {
    /// A second table under a plain header.
    Plain,
    /// A second table of one row under a plain header (part 2e-1, item 1: the two lines are
    /// a table of a header and one row).
    PlainOne,
    /// A second table under a bold header of years.
    Years,
    /// A note row of text over more rows of the first table's shape: one table, the note row
    /// a body row of it.
    Note,
}

/// [`two_tables`], or with [`Second::Note`] a note row and two more rows of the first table;
/// the third row of the first table stands `inner` pitches under the row over it (Task 15b
/// part 2d item 5: a row set a little apart does not raise the pitch of the rows).
fn two_tables_or_note(v: &Variant, second_kind: Second, inner: f32) -> Sheet {
    let plain = second_kind != Second::Years;
    let k = v.k();
    let (plain_face, bold) = v.faces();
    let size = 8.0 * k;
    let pitch = 1.25 * size;
    let first: [[&str; 3]; 5] = [
        ["Lohnart", "Bezeichnung", "Betrag"],
        ["1000", "Gehalt", "3.000,00"],
        ["1010", "Zulage", "150,00"],
        ["1020", "Fahrgeld", "45,67"],
        ["Summe", "", "3.195,67"],
    ];
    let second = match second_kind {
        Second::Plain | Second::PlainOne => [
            ["Kennung", "Menge", "Wert"],
            ["7001", "12", "45,67"],
            ["7002", "3", "8,90"],
        ],
        Second::Years => [
            ["Vorgang", "2025", "2026"],
            ["7001", "12", "45,67"],
            ["7002", "3", "8,90"],
        ],
        Second::Note => [
            ["Hinweis", "siehe Anlage", "Seite"],
            ["3000", "Bonus", "500,00"],
            ["3010", "Praemie", "50,00"],
        ],
    };
    let note = second_kind == Second::Note;
    let second = if second_kind == Second::PlainOne {
        &second[..2]
    } else {
        &second[..]
    };
    let (x0, x1, x2) = (40.0, 40.0 + 72.0 * k, 40.0 + 360.0 * k);
    let mid = x1 + font_width(bold, "Bezeichnung", size);
    let mut t = String::new();
    let mut y = 650.0;
    for (n, row) in first.iter().enumerate() {
        if n == 2 {
            y -= (inner - 1.0) * pitch;
        }
        let font = if n == 0 || n == 4 { bold } else { plain_face };
        t += &show(font, size, x0, y, row[0]);
        if !row[1].is_empty() {
            t += &show(font, size, x1, y, row[1]);
        }
        t += &right(font, size, x2, y, row[2]);
        y -= pitch;
    }
    y -= 1.2 * pitch;
    for (n, row) in second.iter().enumerate() {
        let font = if n == 0 && !plain { bold } else { plain_face };
        t += &show(font, size, x0, y, row[0]);
        if note {
            t += &show(font, size, x1, y, row[1]);
        } else {
            t += &right(font, size, mid, y, row[1]);
        }
        t += &right(font, size, x2, y, row[2]);
        y -= pitch;
    }
    let span = Span {
        top: 650.0,
        bottom: y + pitch,
    };
    let mut s = sheet(v, t, 420.0, Some(span));
    if note {
        let all: Vec<&[&str]> = first.iter().chain(second).map(|r| &r[..]).collect();
        s.want.tables.push(owned(&all));
        // The note row may head a table of its own (Task 15b part 2d fix round 1): said.
        s.want.losses.push("may start a table of its own".into());
    } else {
        s.want
            .tables
            .push(owned(&first.iter().map(|r| &r[..]).collect::<Vec<_>>()));
        s.want
            .tables
            .push(owned(&second.iter().map(|r| &r[..]).collect::<Vec<_>>()));
    }
    s
}

/// The blocks of [`side_blocks`]: an address beside two banks; an address beside two phone
/// lines and a register; an address beside a bank under a colon heading.
const TWO_BANKS: [&[&str]; 2] = [
    &[
        "Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt",
        "Deutschland",
    ],
    &[
        "Musterbank",
        "IBAN DE00 0000 0000 0000 0000 00",
        "Beispielbank",
        "IBAN DE11 1111 1111 1111 1111 11",
    ],
];
const TWO_PHONES: [&[&str]; 3] = [
    &["Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
    &["Telefon 0000 1", "Telefon 0000 2", "Fax 0000 3"],
    &[
        "Amtsgericht Musterstadt",
        "HRB 00000",
        "USt-IdNr. DE000000000",
    ],
];
const COLON_HEADING: [&[&str]; 2] = [
    &[
        "Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt",
        "Deutschland",
    ],
    &[
        "Bankverbindung:",
        "Musterbank",
        "IBAN DE00 0000 0000 0000 0000 00",
    ],
];

/// Task 15b part 2c re-review r1 (Important 1): `blocks` side by side at 7 points times the
/// size factor from the baseline 650, 1.2 sizes apart, each three sizes right of the widest
/// line of the one before: no block holds lines of two of them, and the loss of blocks side
/// by side is said.
fn side_blocks(v: &Variant, blocks: &[&[&str]]) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 7.0 * k;
    let mut t = String::new();
    let mut x = 40.0;
    let mut height = 0;
    for lines in blocks {
        let mut widest: f32 = 0.0;
        for (n, line) in lines.iter().enumerate() {
            t += &show(plain, size, x, 650.0 - 1.2 * size * n as f32, line);
            widest = widest.max(font_width(plain, line, size));
        }
        height = height.max(lines.len());
        x += widest + 3.0 * size;
    }
    let span = Span {
        top: 650.0,
        bottom: 650.0 - 1.2 * size * (height - 1) as f32,
    };
    let mut s = sheet(v, t, 500.0, Some(span));
    for (a, left) in blocks.iter().enumerate() {
        s.built.values.extend(left.iter().map(|l| l.to_string()));
        for right in &blocks[a + 1..] {
            for l in left.iter() {
                for r in right.iter() {
                    s.want.apart.push((l.to_string(), r.to_string()));
                }
            }
        }
    }
    s.want.losses.push("side by side".into());
    s
}

/// The lines of the prose of [`prose_beside`].
const PROSE: [&str; 6] = [
    "Die Messung zeigt einen klaren Anstieg der Werte im",
    "zweiten Abschnitt, der sich bis zum Ende der Reihe",
    "fortsetzt und im letzten Monat am hoechsten ist.",
    "Im Vergleich mit dem Vorjahr liegt der Wert damit",
    "deutlich hoeher, obwohl die Zahl der Proben gleich",
    "geblieben ist und die Methode nicht geaendert wurde.",
];

/// Part 3 (`form_drawn`): a bold heading a size step larger over a rule and two paragraphs of
/// prose at 9 points times the size factor, and lower on the page `beside`: nothing (the
/// control), a ruled table of three columns (`Table`) or a boxed note of two lines (`Note`). Neither is a form: the heading stays a
/// heading followed by its two paragraphs, and the table is a table.
fn prose_beside(v: &Variant, beside: Beside) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 9.0 * k;
    let mut t = show(bold, 1.25 * size, 72.0, 760.0, "Ergebnisse der Messung");
    t += &format!(
        "q 0.5 w 72 {} m 400 {} l S Q\n",
        760.0 - 0.28 * size,
        760.0 - 0.28 * size
    );
    for (n, l) in PROSE.iter().enumerate() {
        let y = 760.0 - 1.9 * size - 1.2 * size * n as f32 - if n >= 3 { 0.9 * size } else { 0.0 };
        t += &show(plain, size, 72.0, y, l);
    }
    let rows = [
        ["Monat", "Wert", "Proben"],
        ["Januar", "12,5", "40"],
        ["Februar", "13,1", "42"],
    ];
    match beside {
        Beside::Table => {
            let (top, h) = (600.0, 1.6 * size);
            for r in 0..=3 {
                let y = top - h * r as f32;
                t += &format!("q 0.5 w 72 {y} m 372 {y} l S Q\n");
            }
            for x in [72.0, 172.0, 272.0, 372.0] {
                t += &format!("q 0.5 w {x} {} m {x} {top} l S Q\n", top - 3.0 * h);
            }
            for (r, row) in rows.iter().enumerate() {
                for (c, cell) in row.iter().enumerate() {
                    let font = if r == 0 { bold } else { plain };
                    let y = top - h * r as f32 - 1.15 * size;
                    t += &show(font, 0.9 * size, 76.0 + 100.0 * c as f32, y, cell);
                }
            }
        }
        Beside::Note => {
            t += &format!(
                "q 0.5 w 72 {} 300 {} re S Q\n",
                600.0 - 3.0 * size,
                3.0 * size
            );
            t += &show(
                plain,
                0.9 * size,
                76.0,
                600.0 - 1.1 * size,
                "Hinweis: Die Werte sind gerundet",
            );
            t += &show(
                plain,
                0.9 * size,
                76.0,
                600.0 - 2.2 * size,
                "und gelten fuer alle Proben.",
            );
        }
        Beside::Nothing => {}
    }
    let mut s = sheet(v, t, 450.0, None);
    s.want.headings.push("Ergebnisse der Messung".into());
    s.want.title = Some("Ergebnisse der Messung".into());
    s.want.paragraphs.push(PROSE[..3].join(" "));
    s.want.paragraphs.push(PROSE[3..].join(" "));
    if beside == Beside::Table {
        s.want
            .tables
            .push(owned(&rows.iter().map(|r| &r[..]).collect::<Vec<_>>()));
    }
    s
}

/// What stands lower on the page of [`prose_beside`].
#[derive(Debug, Clone, Copy, PartialEq)]
enum Beside {
    Nothing,
    Table,
    Note,
}

/// Part 3 (weight-only boxes): three drawn boxes in a row, each a bold label at its top and a
/// plain value of the same size under it, and a fourth box with its bold label only. The page
/// shows one label style (the bold of the lone label), so every box pairs.
fn weight_only_boxes(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let (top, h, w) = (700.0, 2.6 * size, 110.0);
    let fields = [
        ("Kundennr", "00000"),
        ("Vertrag", "M-0001"),
        ("Tarif", "Muster A"),
        ("Zusatz", ""),
    ];
    let mut t = String::new();
    for (n, (label, value)) in fields.iter().enumerate() {
        let x = 40.0 + w * n as f32;
        t += &format!("q 0.5 w {x} {} {w} {h} re S Q\n", top - h);
        t += &show(bold, size, x + 3.0, top - 1.1 * size, label);
        if !value.is_empty() {
            t += &show(plain, size, x + 3.0, top - 2.2 * size, value);
        }
    }
    let mut s = sheet(v, t, 500.0, None);
    for (label, value) in fields {
        s.built
            .fields
            .push((label, Ok(if value.is_empty() { None } else { Some(value) })));
    }
    s
}

/// The lines of the two columns of [`flowing_column`], above and under the frame.
const FLOW: [[&[&str]; 2]; 2] = [
    [
        &[
            "Die Ware wird vor dem Versand",
            "an jedem Standort geprueft und",
            "danach mit der Liefe-",
        ],
        &[
            "rung an den Kunden gegeben, der",
            "sie bei Erhalt quittiert.",
        ],
    ],
    [
        &[
            "Fehlt ein Teil der Sendung, so",
            "meldet der Kunde dies binnen",
            "einer Woche schriftlich.",
        ],
        &["Ersatz wird dann ohne weitere", "Kosten nachgeliefert."],
    ],
];

/// Part 4 (band order at drawn rules): two columns of 9 point prose times the size factor
/// with a framed figure across both in the middle; the last line of the left column above the
/// frame ends in a hyphen and goes on under it. The page is read in column order: the left
/// column above and under the frame, then the right column.
fn flowing_column(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let step = 1.25 * size;
    // The frame starts under the third line above it and is 80 points high.
    let frame_top = 760.0 - 2.0 * step - 0.8 * size;
    let frame_bottom = frame_top - 80.0;
    let mut t = String::new();
    for (c, parts) in FLOW.iter().enumerate() {
        let x = if c == 0 { 40.0 } else { 310.0 };
        for (n, l) in parts[0].iter().enumerate() {
            t += &show(plain, size, x, 760.0 - step * n as f32, l);
        }
        for (n, l) in parts[1].iter().enumerate() {
            t += &show(plain, size, x, frame_bottom - step - step * n as f32, l);
        }
    }
    // The figure's frame across both columns, with a picture in it.
    t += &format!("q 0.5 w 40 {frame_bottom} 515 80 re S Q\n");
    t += &crate::common::logo(200.0, frame_bottom + 5.0, 200.0, 70.0);
    let mut s = sheet(v, t, 420.0, None);
    s.want.order = vec![
        FLOW[0][0][0].into(),
        FLOW[0][1][1].into(),
        FLOW[1][0][0].into(),
        FLOW[1][1][1].into(),
    ];
    s
}

/// The shapes of business documents.
pub(crate) fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            name: "letter head",
            axes: DOC_NEXT,
            draw: |v| (letter_head(v, false, false), None),
        },
        Shape {
            name: "letter head with a larger name",
            axes: DOC_NEXT,
            draw: |v| (letter_head(v, true, false), None),
        },
        Shape {
            name: "letter head with the date beside the name",
            axes: DOC_NEXT,
            draw: |v| (letter_head(v, false, true), None),
        },
        Shape {
            name: "line items with a larger total between rules over a note",
            axes: RULED,
            draw: |v| (items_sized(v, true, true, true, (1.0, 1.25)), None),
        },
        Shape {
            name: "line items with a smaller first total between rules over a note",
            axes: RULED,
            draw: |v| (items_sized(v, true, true, true, (0.85, 1.0)), None),
        },
        Shape {
            name: "totals with a currency column between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &CURRENCY_TOTALS, 1.0), None),
        },
        Shape {
            name: "totals with a currency column and a larger total between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &CURRENCY_TOTALS, 1.25), None),
        },
        Shape {
            name: "totals with the label right of its amount between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &LABEL_RIGHT_TOTALS, 1.0), None),
        },
        Shape {
            name: "totals with the label right of its amount and a larger total between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &LABEL_RIGHT_TOTALS, 1.25), None),
        },
        Shape {
            name: "totals with whole amounts between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &WHOLE_TOTALS, 1.0), None),
        },
        Shape {
            name: "totals with whole amounts and a larger total between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &WHOLE_TOTALS, 1.25), None),
        },
        Shape {
            name: "totals with a percent column between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &PERCENT_TOTALS, 1.0), None),
        },
        Shape {
            name: "totals with a percent column and a larger total between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &PERCENT_TOTALS, 1.25), None),
        },
        Shape {
            name: "totals with a percent column in one row between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &PERCENT_ONE_ROW_TOTALS, 1.0), None),
        },
        Shape {
            name: "totals with a percent column in one row and a larger total between rules over a note",
            axes: RULED,
            draw: |v| (totals_shape(v, &PERCENT_ONE_ROW_TOTALS, 1.25), None),
        },
        Shape {
            name: "totals with a larger total under a rule",
            axes: RULED,
            draw: |v| (totals_drawn(v, &PLAIN_TOTALS, 1.25, Under::Rule), None),
        },
        Shape {
            name: "totals with a percent column and a larger total under a rule",
            axes: RULED,
            draw: |v| (totals_drawn(v, &PERCENT_TOTALS, 1.25, Under::Rule), None),
        },
        Shape {
            name: "net, tax and gross titles over their amounts between rules over a note",
            axes: RULED,
            draw: |v| {
                (
                    totals_drawn(v, &NET_TAX_GROSS_TOTALS, 1.0, Under::Titles),
                    None,
                )
            },
        },
        Shape {
            name: "lines of a label and its bold value",
            axes: DOC_NEXT,
            draw: |v| (field_lines(v, None), None),
        },
        Shape {
            name: "lines of a label and its bold value, two of them boxed",
            axes: DOC_NEXT,
            draw: |v| (field_lines(v, Some((0, 2))), None),
        },
        Shape {
            name: "lines of a label and its bold value, the middle two boxed",
            axes: DOC_NEXT,
            draw: |v| (field_lines(v, Some((1, 3))), None),
        },
        Shape {
            name: "a column of bold lines at heading size with a box around two of them",
            axes: DOC,
            draw: |v| (one_face_column(v, 9.5, true, Some((3, 5))), None),
        },
        Shape {
            name: "a column of bold lines at heading size with a box around the middle two",
            axes: DOC,
            draw: |v| (one_face_column(v, 9.5, true, Some((1, 3))), None),
        },
        Shape {
            name: "a column of lines of one face",
            axes: DOC,
            draw: |v| (one_face_column(v, 8.0, false, None), None),
        },
        Shape {
            name: "a column of lines of one face with a box around two of them",
            axes: DOC,
            draw: |v| (one_face_column(v, 8.0, false, Some((2, 4))), None),
        },
        Shape {
            name: "a column of one-face field lines of 5 to 7 words",
            axes: DOC,
            draw: |v| (column_of(v, &LONG_FIELD_LINES, 8.0, false, None), None),
        },
        Shape {
            name: "a column of one-face field lines of 5 to 7 words with a box around two of them",
            axes: DOC,
            draw: |v| {
                (
                    column_of(v, &LONG_FIELD_LINES, 8.0, false, Some((2, 4))),
                    None,
                )
            },
        },
        Shape {
            name: "a column of bold field lines of 5 to 7 words at heading size, two boxed",
            axes: DOC,
            draw: |v| {
                (
                    column_of(v, &LONG_FIELD_LINES, 9.5, true, Some((3, 5))),
                    None,
                )
            },
        },
        Shape {
            name: "slogan lines, each a sentence ending in an exclamation mark",
            axes: DOC,
            draw: |v| (column_of(v, &SLOGAN_LINES, 8.0, false, None), None),
        },
        Shape {
            name: "bold slogan lines, each a sentence ending in an exclamation mark",
            axes: DOC,
            draw: |v| (column_of(v, &SLOGAN_LINES, 9.5, true, None), None),
        },
        Shape {
            name: "bold lines of starred items",
            axes: DOC,
            draw: |v| {
                let mut s = column_of(v, &STARRED_LINES, 9.0, true, None);
                s.want.order = STARRED_LINES[1..].iter().map(|l| l.to_string()).collect();
                (s, None)
            },
        },
        Shape {
            name: "a row label cut by a hyphen and a row of numbers over prose",
            axes: DOC,
            draw: |v| (cut_rows(v), None),
        },
        Shape {
            name: "two lines in three columns alone in the body",
            axes: DOC,
            draw: |v| (two_lines_in_columns(v), None),
        },
        Shape {
            name: "a value printed over its label",
            axes: DOC,
            draw: |v| (overprinted(v), None),
        },
        Shape {
            name: "prose with a number at a line end",
            axes: DOC,
            draw: |v| (number_prose(v), None),
        },
        Shape {
            name: "two lines of three words set full",
            axes: DOC,
            draw: |v| (justified_pair(v), None),
        },
        Shape {
            name: "a paragraph without a stop set full in a narrow column",
            axes: DOC,
            draw: |v| (justified_paragraph(v), None),
        },
        Shape {
            name: "line items with a hanging credit minus over a total",
            axes: DOC,
            draw: |v| (hanging_credit(v), None),
        },
        Shape {
            name: "line items with a deduction with a minus under an amount",
            axes: DOC,
            draw: |v| (stacked_deduction(v), None),
        },
        Shape {
            name: "line items with a credit over a total",
            axes: DOC,
            draw: |v| (credit_over_total(v), None),
        },
        Shape {
            name: "an accent drawn on its own over its word",
            axes: DOC,
            draw: |v| (lone_accent(v), None),
        },
        Shape {
            name: "cells of two lines lining up by chance between rules",
            axes: DOC,
            draw: |v| (cells_by_chance(v, true), None),
        },
        Shape {
            name: "cells of two lines lining up by chance between rules with no divider",
            axes: RULED,
            draw: |v| (cells_by_chance(v, false), None),
        },
        Shape {
            name: "cells or rows of two lines between rules with no divider",
            axes: RULED,
            draw: |v| (cells_or_rows(v), None),
        },
        Shape {
            name: "titles over a row that ends in a stop",
            axes: DOC,
            draw: |v| (titles_over_a_stop(v), None),
        },
        Shape {
            name: "two lines in three columns between two rules",
            axes: DOC,
            draw: |v| (two_lines_in_a_strip(v), None),
        },
        Shape {
            name: "a column of short field lines after a sentence",
            axes: DOC,
            draw: |v| {
                let lines = [
                    "Zahlbar sofort.",
                    "Lieferung frei Haus",
                    "Versand per Spedition Muster",
                    "Gewicht 12 kg",
                ];
                (column_of(v, &lines, 8.0, false, None), None)
            },
        },
        Shape {
            name: "line items with wrapped date ranges",
            axes: DOC_NEXT,
            draw: |v| (period_items(v, true), None),
        },
        Shape {
            name: "line items whose last period wraps under the table",
            axes: DOC_NEXT,
            draw: |v| (period_items_of(v, true, true), None),
        },
        Shape {
            name: "line items over two amounts alone in two columns",
            axes: DOC_NEXT,
            draw: |v| (period_items(v, false), None),
        },
        Shape {
            name: "a notice band between rules",
            axes: DOC_NEXT,
            draw: |v| (notice_band(v), None),
        },
        Shape {
            name: "line items with totals between rules over a note",
            axes: RULED,
            draw: |v| (items(v, true, true, true), None),
        },
        Shape {
            name: "line items with an end rule",
            axes: RULED,
            draw: |v| (items(v, true, true, false), None),
        },
        Shape {
            name: "line items with row rules",
            axes: RULED,
            draw: |v| (items(v, false, true, false), None),
        },
        Shape {
            name: "line items without rules over an end rule",
            axes: RULED,
            draw: |v| (items(v, true, false, false), None),
        },
        Shape {
            name: "close column titles",
            axes: DOC_NEXT,
            draw: |v| (close_titles(v), None),
        },
        Shape {
            name: "title beside a field list",
            axes: DOC_NEXT,
            draw: |v| (title_beside_fields(v), None),
        },
        Shape {
            name: "address columns in the body",
            axes: DOC_NEXT,
            draw: |v| (address_columns(v), None),
        },
        Shape {
            name: "a small table of records under a plain header",
            axes: DOC_NEXT,
            draw: |v| (rows_shape(v, Rows::Table), None),
        },
        Shape {
            name: "a headerless table of records",
            axes: DOC_NEXT,
            draw: |v| (rows_shape(v, Rows::Records), None),
        },
        Shape {
            name: "labels beside values with a last label alone",
            axes: DOC_NEXT,
            draw: |v| (rows_shape(v, Rows::Labels), None),
        },
        Shape {
            name: "blocks side by side under their headings",
            axes: DOC_NEXT,
            draw: |v| (rows_shape(v, Rows::Headed), None),
        },
        Shape {
            name: "a second table under a plain header after a bold total",
            axes: DOC_NEXT,
            draw: |v| (two_tables(v, true), None),
        },
        Shape {
            name: "a second table under a plain header after a row set apart",
            axes: DOC_NEXT,
            draw: |v| (two_tables_or_note(v, Second::Plain, 1.5), None),
        },
        Shape {
            name: "a second table of one row under a plain header after a bold total",
            axes: DOC_NEXT,
            draw: |v| (two_tables_or_note(v, Second::PlainOne, 1.0), None),
        },
        Shape {
            name: "a note row after a blank row in a table",
            axes: DOC_NEXT,
            draw: |v| (two_tables_or_note(v, Second::Note, 1.0), None),
        },
        Shape {
            name: "a second table under a header of years after a bold total",
            axes: DOC_NEXT,
            draw: |v| (two_tables(v, false), None),
        },
        Shape {
            name: "an address beside two banks",
            axes: DOC_NEXT,
            draw: |v| (side_blocks(v, &TWO_BANKS), None),
        },
        Shape {
            name: "an address beside two phone lines and a register",
            axes: DOC_NEXT,
            draw: |v| (side_blocks(v, &TWO_PHONES), None),
        },
        Shape {
            name: "an address beside a bank under a colon heading",
            axes: DOC_NEXT,
            draw: |v| (side_blocks(v, &COLON_HEADING), None),
        },
        Shape {
            name: "a heading over a rule on a prose page",
            axes: DOC,
            draw: |v| (prose_beside(v, Beside::Nothing), None),
        },
        Shape {
            name: "a heading over a rule beside a ruled table",
            axes: DOC,
            draw: |v| (prose_beside(v, Beside::Table), None),
        },
        Shape {
            name: "a heading over a rule beside a boxed note",
            axes: DOC,
            draw: |v| (prose_beside(v, Beside::Note), None),
        },
        Shape {
            name: "weight-only boxes",
            axes: DOC,
            draw: |v| (weight_only_boxes(v), None),
        },
        Shape {
            name: "a column flowing past a framed figure",
            axes: DOC,
            draw: |v| (flowing_column(v), None),
        },
        Shape {
            name: "footer columns",
            axes: DOC,
            draw: |v| (footer(v), None),
        },
    ]
}
