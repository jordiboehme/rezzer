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
pub(crate) const DOC: Axes = Axes {
    data: &[Data::CourierBold, Data::Proportional],
    labels: &[crate::axes::Labels::Helvetica],
    look: &[Look::Real],
    next: &[Next::None],
    ..ALL
};

/// The axes of a business page with its neighbours over and under it.
pub(crate) const DOC_NEXT: Axes = Axes {
    next: &[Next::None, Next::TitleOver, Next::LineOver, Next::RowUnder],
    ..DOC
};

/// The looks of the rules of a business page: stroked lines, dashed, dotted, and bars of a
/// point.
pub(crate) const RULED: Axes = Axes {
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
pub(crate) struct Span {
    pub top: f32,
    pub bottom: f32,
}

/// The page of a business shape: `template`, the variant's neighbour next to the shape at
/// `span` (a bold title or a line over it, a table of three columns under it; none beside, as
/// the shapes run over the width of the page), and the body from `body_top`. Every text shown
/// on the page is a value to find as many times as it is drawn.
pub(crate) fn sheet(v: &Variant, template: String, body_top: f32, span: Option<Span>) -> Sheet {
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
                want.table(
                    owned(&rows.iter().map(|r| &r[..]).collect::<Vec<_>>()),
                    "table",
                );
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
pub(crate) fn right(font: &str, size: f32, x: f32, y: f32, text: &str) -> String {
    show(font, size, x - font_width(font, text, size), y, text)
}

/// A horizontal rule from `x0` to `x1` at `y` as the variant draws it; `size` sets the dashes.
pub(crate) fn rule(v: &Variant, x0: f32, x1: f32, y: f32, size: f32) -> String {
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

pub(crate) fn owned(rows: &[&[&str]]) -> Vec<Vec<String>> {
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
    s.want.table(table, "table");
    if end {
        s.want.table(
            totals
                .iter()
                .map(|(l, v)| vec![l.to_string(), v.to_string()])
                .collect(),
            "totals",
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
    s.want.table(table, "totals");
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
    s.want.table(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect(),
        "label_value",
    );
    s
}

/// Part 2e-4 (item 7): a smaller label and its bold value set against it with no space,
/// the value starting a fifth of a size inside the label's last letter: two texts, never one
/// word.
fn touching(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 9.0 * k;
    let top = 640.0;
    let label = 0.7 * size;
    let end = 40.0 + font_width(plain, "Name", label);
    let mut t = show(plain, label, 40.0, top, "Name");
    t += &show(bold, size, end - 0.2 * size, top, "Hinterhaus");
    let mut s = sheet(
        v,
        t,
        420.0,
        Some(Span {
            top,
            bottom: top - size,
        }),
    );
    // One line of the read, its two words apart (Task 19: the engine keeps this one line as
    // laid out, no region type having clear evidence for it; a right read, Task 36 makes it a
    // paragraph again).
    s.want
        .lines_joined
        .push(("Name".into(), "Hinterhaus".into()));
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
    // One line of the read, its two words apart (Task 19: the engine keeps this one line as
    // laid out, no region type having clear evidence for it; a right read, Task 36 makes it a
    // paragraph again).
    s.want
        .lines_joined
        .push(("Name".into(), "Hinterhaus".into()));
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

/// Part 2e-4 (item 9): labels with a short index (`Kundennr. 1`) beside the lines of an
/// address: a table of labels and values, with no doubt that blocks stand side by side.
fn numbered_labels(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 640.0;
    let rows = [
        ["Kundennr. 1", "Muster GmbH"],
        ["Vertrag 7", "Musterweg 1"],
        ["Tarif A2", "12345 Musterstadt"],
    ];
    let mut t = String::new();
    for (n, row) in rows.iter().enumerate() {
        for (x, c) in [40.0f32, 220.0].iter().zip(row) {
            t += &show(plain, size, *x, top - 1.2 * size * n as f32, c);
        }
    }
    let bottom = top - 2.4 * size;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want.table(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect(),
        "label_value",
    );
    s.want
        .no_losses
        .push("though a column of them holds an address".into());
    s
}

/// `text` in `font` at `size` wrapped greedily to `measure` points: its lines.
fn wrapped(text: &str, font: &str, size: f32, measure: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for w in text.split_whitespace() {
        let cand = if cur.is_empty() {
            w.to_string()
        } else {
            format!("{cur} {w}")
        };
        if cur.is_empty() || font_width(font, &cand, size) <= measure {
            cur = cand;
        } else {
            lines.push(std::mem::replace(&mut cur, w.to_string()));
        }
    }
    lines.push(cur);
    lines
}

/// Part 2e-4 (item 8): prose of two sentences right over, and right under, a column of field
/// lines in its face, at its pitch and left edge (`line_breaks` takes evidence from the lines
/// over and under a block): the prose goes on line by line, the field lines keep their breaks,
/// and no field line goes on the prose.
fn prose_by_fields(v: &Variant, under: bool) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 640.0;
    let fields = [
        "Kunde Muster GmbH",
        "Projekt 4711 Musterstadt",
        "Lieferdatum 01.10.2026",
    ];
    let text = "Die Muster GmbH liefert die bestellte Ware innerhalb von zehn Werktagen an die \
                genannte Anschrift. Bitte pruefen Sie die Lieferung bei Erhalt und melden Sie \
                Schaeden sofort.";
    let prose = wrapped(text, plain, size, 30.0 * size);
    let mut all: Vec<String> = Vec::new();
    if under {
        all.extend(fields.iter().map(|f| f.to_string()));
        all.extend(prose.iter().cloned());
    } else {
        all.extend(prose.iter().cloned());
        all.extend(fields.iter().map(|f| f.to_string()));
    }
    let mut t = String::new();
    for (n, l) in all.iter().enumerate() {
        t += &show(plain, size, 40.0, top - 1.2 * size * n as f32, l);
    }
    let bottom = top - 1.2 * size * (all.len() as f32 - 1.0);
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    for w in all.windows(2) {
        let field = fields.contains(&w[0].as_str()) || fields.contains(&w[1].as_str());
        let a = w[0].rsplit(' ').next().unwrap_or("").to_string();
        let b = w[1].split(' ').next().unwrap_or("").to_string();
        if field {
            s.want.lines_apart.push((w[0].clone(), w[1].clone()));
        } else {
            s.want.lines_joined.push((a, b));
        }
    }
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

/// Task 31a fix round 1 (I2): three paragraphs of three lines, the last line short, and two
/// short rules far apart, one over the first paragraph and one under the last (the edges of
/// a mark in the margin, of a bar behind a title). The two rules bound a tall narrow frame
/// that the full lines run through; only the short last lines have their middles in it. The
/// frame is no strip of fields: each short line is the last line of its paragraph.
fn paragraphs_across_a_frame(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let top = 640.0;
    let pitch = 1.2 * size;
    let step = 3.0 * pitch + size;
    let ends = ["of the long room", "of the cold hall", "of the wide yard"];
    let names = ["first", "second", "third"];
    let mut t = String::new();
    let mut joined = Vec::new();
    let mut right = 0.0f32;
    for (p, (end, name)) in ends.iter().zip(names).enumerate() {
        let y = top - step * p as f32;
        let one = format!("the {name} green lamp stands beside a wooden shelf and the small desk");
        let two =
            format!("under the {name} window keeps the old books of a quiet reader near the door");
        t += &show(plain, size, 40.0, y, &one);
        t += &show(plain, size, 40.0, y - pitch, &two);
        t += &show(plain, size, 40.0, y - 2.0 * pitch, end);
        right = right.max(40.0 + font_width(plain, &two, size));
        joined.push(("desk".to_string(), "under".to_string()));
        joined.push(("door".to_string(), "of".to_string()));
    }
    // The frame: around the middle of the short lines.
    let short = font_width(plain, ends[0], size);
    let (x0, x1) = (40.0 + 0.3 * short, 40.0 + 0.7 * short);
    let bottom = top - 2.0 * step - 2.0 * pitch;
    t += &rule(v, x0, x1, top + 1.5 * size, size);
    t += &rule(v, x0, x1, bottom - 0.8 * size, size);
    let mut s = sheet(
        v,
        t,
        right,
        Some(Span {
            top: top + 1.5 * size,
            bottom: bottom - 0.8 * size,
        }),
    );
    s.want.lines_joined.extend(joined);
    s
}

/// Task 31a fix round 2 (N1): rows of a label and its value at one white space from row to
/// row, the last row (a total) set larger. The larger row is a row, no row of values under a
/// row of labels: each label reads with its own value. `rows` are the label and the value of
/// each row; the last one is the larger.
fn rows_over_a_larger_row(v: &Variant, rows: &[[&str; 2]]) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let (size, large) = (10.0 * k, 11.5 * k);
    let top = 640.0;
    let mut t = String::new();
    let mut order = Vec::new();
    let mut y = top;
    for (n, row) in rows.iter().enumerate() {
        let last = n + 1 == rows.len();
        let (face, s) = if last { (bold, large) } else { (plain, size) };
        // The white space over a row is the same for every row: a larger row stands lower
        // by what it is higher.
        if n > 0 {
            y -= 1.0 * size + s;
        }
        t += &show(face, s, 40.0, y, row[0]);
        t += &show(face, s, 200.0, y, row[1]);
        order.extend(row.iter().map(|c| c.to_string()));
    }
    let mut s = sheet(v, t, 320.0, Some(Span { top, bottom: y }));
    s.want.order = order;
    s
}

/// Task 31a fix round 2 (ruling on I3): the lines of a table of contents at one pitch with
/// white space between the lines: a name and its page (a number or a range) at one edge, the
/// lines under a title set in, a title line without a page. They read as one table of a name
/// and a page per row. A guard: the contents page of a public payroll sample that reads as
/// one-row tables per line (Task 26) has more on it (a second column, a small picture beside
/// each title) and is not built here.
fn contents_lines(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let top = 640.0;
    // A line without a page is the title of the lines under it.
    let entries = [
        ["Opening notes", "3"],
        ["The green lamp", "7"],
        ["Lists for the house", ""],
        ["A wooden shelf (part one)", "12"],
        ["The small desk", "14-15"],
        ["Books of a quiet reader", "21"],
        ["Other lists (samples)", ""],
        ["The long room", "30"],
        ["Closing notes", "44"],
    ];
    let mut t = String::new();
    for (n, e) in entries.iter().enumerate() {
        let y = top - 1.8 * size * n as f32;
        // The lines under a title are set in by two ems; the pages start at one edge.
        let x = if e[1].is_empty() || n < 2 {
            40.0
        } else {
            40.0 + 2.0 * size
        };
        t += &show(plain, size, x, y, e[0]);
        if !e[1].is_empty() {
            t += &show(plain, size, 300.0, y, e[1]);
        }
    }
    let mut s = sheet(
        v,
        t,
        400.0,
        Some(Span {
            top,
            bottom: top - 1.8 * size * (entries.len() - 1) as f32,
        }),
    );
    // One table: a title line is a row with an empty page cell.
    s.want.tables = vec![
        entries
            .iter()
            .map(|e| e.iter().map(|c| c.to_string()).collect())
            .collect(),
    ];
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
    s.want.table(table, "table");
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
    let mut s = two_cells(v, divided, &cells);
    for lines in cells {
        s.want
            .cells
            .push(lines.iter().map(|l| l.to_string()).collect());
    }
    s
}

/// Part 2e-4 (item 1): a row between rules with no divider whose `cells` of two lines show no
/// sign that their lines go together (a name over a firm beside a firm over its street; fix
/// round 1, C1: labels beside a street and its town, two rows of items): rows or cells, so the
/// two lines read line by line and a loss says the doubt.
fn cells_or_rows(v: &Variant, cells: &[[&str; 2]]) -> Sheet {
    let mut s = two_cells(v, false, cells);
    for n in [0, 1] {
        for w in cells.windows(2) {
            s.want
                .lines_joined
                .push((w[0][n].to_string(), w[1][n].to_string()));
        }
    }
    s.want
        .losses
        .push("two lines of text in columns between two rules".into());
    s
}

/// Fix round 1 (I1): words drawn in two weights, the second part touching the first and opening
/// with a small letter (`Rechnungs` bold and `nummer 4711`, `Muster` and `mann GmbH` bold): one
/// word each.
fn two_weights(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 9.0 * k;
    let top = 640.0;
    let mut t = String::new();
    let words = [
        ("Rechnungs", bold, "nummer 4711", plain),
        ("Muster", plain, "mann GmbH", bold),
    ];
    for (n, (a, fa, b, fb)) in words.iter().enumerate() {
        let y = top - 2.0 * size * n as f32;
        t += &show(fa, size, 40.0, y, a);
        t += &show(fb, size, 40.0 + font_width(fa, a, size), y, b);
    }
    let mut s = sheet(
        v,
        t,
        420.0,
        Some(Span {
            top,
            bottom: top - 2.0 * size,
        }),
    );
    s.once.retain(|o| {
        !matches!(
            o.trim(),
            "Rechnungs" | "nummer 4711" | "Muster" | "mann GmbH"
        )
    });
    for (a, b) in [("Rechnungsnummer", "4711"), ("Mustermann", "GmbH")] {
        s.once.push(format!("{a} {b}"));
        s.want.lines_joined.push((a.into(), b.into()));
    }
    // The first line in bold at the top of the page may be its title.
    s.want.may_title.push("Rechnungsnummer 4711".into());
    s
}

/// Fix round 1 (I2): prose whose line ends in the first half of a range (`von 08.00-` over
/// `18.00 Uhr ...`): the range goes on with no space inside it.
fn wrapped_range(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 640.0;
    let lines = [
        "Unser Kundenservice ist montags bis freitags von 08.00-",
        "18.00 Uhr fuer Sie erreichbar und beantwortet gerne Ihre Fragen.",
    ];
    let mut t = String::new();
    for (n, l) in lines.iter().enumerate() {
        t += &show(plain, size, 40.0, top - 1.2 * size * n as f32, l);
    }
    let mut s = sheet(
        v,
        t,
        420.0,
        Some(Span {
            top,
            bottom: top - 1.2 * size,
        }),
    );
    s.once.retain(|o| !lines.contains(&o.trim()));
    s.once.push("08.00-18.00".into());
    s.want
        .lines_joined
        .push(("von".into(), "08.00-18.00 Uhr".into()));
    s
}

/// `cells` of two lines between two rules, at x 44, 224 and on (180 points apart), with a
/// divider between the first two when `divided`.
fn two_cells(v: &Variant, divided: bool, cells: &[[&str; 2]]) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 640.0;
    let h = 4.0 * size;
    let x1 = 40.0 + 180.0 * cells.len() as f32;
    let mut t = rule(v, 40.0, x1, top, size) + &rule(v, 40.0, x1, top - h, size);
    if divided {
        t += &format!("0 0 0 RG 0.5 w 220 {} m 220 {top} l S\n", top - h);
    }
    for (c, lines) in cells.iter().enumerate() {
        let x = 44.0 + 180.0 * c as f32;
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
    s.want.table(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect(),
        "label_value",
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
    s.want.table(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect(),
        "label_value",
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
        s.want.table(table, "table");
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
    s.want.table(table, "table");
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
    s.want.table(
        TITLE_FIELDS
            .iter()
            .map(|f| f.iter().map(|c| c.to_string()).collect())
            .collect(),
        "label_value",
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

/// Task 36a fix round 1 (review I2): the label of an address in bold at 10 points over a
/// complete address at 8 (a name, a street, a town), at the lines' own pitch, as an invoice
/// sets `Delivery address`. The address has its own name line, so the bold line is a
/// heading over it and no first line of it. Under it a bold title (which the read may take
/// as the document's) and a line of an amount.
fn heading_over_a_named_address(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let s8 = 8.0 * k;
    let step = s8 * 1.3;
    let mut t = show(bold, 10.0 * k, 40.0, 720.0, "Delivery address");
    for (n, line) in ["Sample Trading", "Mill Lane 4", "20481 Oldtown"]
        .iter()
        .enumerate()
    {
        t += &show(plain, s8, 40.0, 720.0 - step * (n + 1) as f32, line);
    }
    t += &show(bold, 9.0 * k, 40.0, 640.0, "Rechnung");
    t += &show(plain, 9.0 * k, 40.0, 580.0, "Betrag 1.234,56");
    let mut s = sheet(v, t, 500.0, None);
    s.want.headings.push("Delivery address".into());
    s.want.may_title.push("Rechnung".into());
    s.want.order = vec![
        "Delivery address".into(),
        "Sample Trading".into(),
        "Mill Lane 4".into(),
        "20481 Oldtown".into(),
    ];
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

/// The rows of [`foot_table_with_a_wrapped_row`]: titles, three items each with its
/// description and its price, and the wrapped end of the last description.
const FOOT_TABLE: [[&str; 3]; 5] = [
    ["Item", "Description", "Price"],
    [
        "Bolt",
        "steel bolt for the frame, zinc plated, size M8",
        "1.20",
    ],
    ["Nut", "nut for the bolt, zinc plated, size M8, hex", "0.40"],
    [
        "Washer",
        "washer for the nut and the bolt, flat, set of",
        "0.10",
    ],
    ["", "twelve pieces", ""],
];

/// The rows of the table with a column of words whose last cell is empty: the last item has
/// no source, so the other columns end on different rows (Task 36a fix round 2, re-review
/// N1, probe 10).
const FOOT_TABLE_EMPTY_CELL: [[&str; 3]; 5] = [
    ["Item", "Description", "Source"],
    [
        "Bolt",
        "steel bolt for the frame, zinc plated, size M8",
        "Mill",
    ],
    [
        "Nut",
        "nut for the bolt, zinc plated, size M8, hex",
        "Forge",
    ],
    [
        "Washer",
        "washer for the nut and the bolt, flat, set of",
        "",
    ],
    ["", "twelve pieces", ""],
];

/// The same with the last description wrapped over two more lines (probe 16).
const FOOT_TABLE_TWO_LINES: [[&str; 3]; 6] = [
    ["Item", "Description", "Source"],
    [
        "Bolt",
        "steel bolt for the frame, zinc plated, size M8",
        "Mill",
    ],
    [
        "Nut",
        "nut for the bolt, zinc plated, size M8, hex",
        "Forge",
    ],
    [
        "Washer",
        "washer for the nut and the bolt, flat, set of",
        "",
    ],
    ["", "twelve pieces, zinc plated, packed in a paper", ""],
    ["", "bag", ""],
];

/// Task 36a fix round 1 (review I1): a table of three columns without rules at the foot of
/// the page, at 8 points times the size factor, its columns left aligned at an even pitch of
/// 161 points from x 40. The descriptions open with a small letter and nearly fill their
/// column, and the last one wraps: all a wrapped paragraph in a footer of columns shows. But
/// each row is an item of its own: its description and its price stay with it, and no line
/// of the read holds the descriptions of two items. On two pages the table is a running
/// footer.
///
/// Fix round 2 (re-review N1): the same holds with a cell left empty at the end of a column
/// of words, and with the last description over two more lines: no other column reaches
/// the last row of the description, so nothing shows blocks side by side. `rows` are the
/// rows of the table, the first its titles, a row with an empty first cell the wrapped end
/// of the description over it. With a column of words the rows read as one paragraph in row
/// order, as they did before the columns of a footer were read (each item with its
/// description and its source, in order): `apart` says whether the descriptions of two
/// items are asked to stand on lines of their own, as they do with a column of prices.
fn foot_table_with_a_wrapped_row(v: &Variant, rows: &[[&str; 3]], apart: bool) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let mut t = String::new();
    for (n, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate().filter(|(_, c)| !c.is_empty()) {
            let (x, y) = (40.0 + 161.0 * k * c as f32, 78.0 - 1.2 * size * n as f32);
            t += &show(plain, size, x, y, cell);
        }
    }
    let mut s = sheet(v, t, 500.0, None);
    let items: Vec<&[&str; 3]> = rows[1..].iter().filter(|r| !r[0].is_empty()).collect();
    for row in &items {
        let cells = row.iter().filter(|c| !c.is_empty());
        s.want.together.push(cells.map(|c| c.to_string()).collect());
    }
    for (a, upper) in items.iter().enumerate() {
        for lower in items[a + 1..].iter().filter(|_| apart) {
            s.want
                .lines_apart
                .push((upper[1].to_string(), lower[1].to_string()));
        }
    }
    s
}

/// The rows of [`table_whose_item_wraps`]: the titles, three items, and under the last item
/// the second line of its name (a norm) beside the wrapped end of its description. Only the
/// first item has a source.
const ITEM_WRAPS: [[&str; 3]; 5] = [
    ["Item", "Description", "Source"],
    [
        "Bolt",
        "steel bolt for the frame, zinc plated, size M8",
        "Mill",
    ],
    ["Nut", "nut for the bolt, zinc plated, size M8, hex", ""],
    [
        "Washer",
        "washer for the nut and the bolt, flat, set of",
        "",
    ],
    ["DIN 125", "twelve pieces", ""],
];

/// Task 36a fix round 3 (re-review 2, I2, probe 2): a table of three columns without rules,
/// its first baseline at `top` (in the body, or at the foot of the page, where it is a
/// running footer on two pages), at 8 points times the size factor, left aligned at a pitch
/// of 161 points from x 40. The name of the last item has a second line beside the wrapped end of its description,
/// so the item column stands beside the descriptions down to their last row, and the column
/// of sources ends after its first row: all that blocks side by side with a wrapped paragraph
/// in the middle one show. Nothing in the lines tells the two apart, so the read may take
/// the columns for blocks. What it may never do is join the descriptions of two items into
/// one text: each line of a column stays a line of its own (the ruling of fix round 3: a
/// wrong read as blocks loses the order of the rows, never where an item ends).
fn table_whose_item_wraps(v: &Variant, top: f32) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let mut t = String::new();
    for (n, row) in ITEM_WRAPS.iter().enumerate() {
        for (c, cell) in row.iter().enumerate().filter(|(_, c)| !c.is_empty()) {
            let (x, y) = (40.0 + 161.0 * k * c as f32, top - 1.2 * size * n as f32);
            t += &show(plain, size, x, y, cell);
        }
    }
    t += &show(bold, 9.0 * k, 40.0, 640.0, "Receipt");
    t += &show(plain, 9.0 * k, 40.0, 580.0, "Sum 1.234,56");
    let mut s = sheet(v, t, 500.0, None);
    s.want.title = Some("Receipt".into());
    // The first line of the description of each item.
    let texts: Vec<&str> = ITEM_WRAPS[1..4].iter().map(|r| r[1]).collect();
    for (a, upper) in texts.iter().enumerate() {
        for lower in &texts[a + 1..] {
            s.want
                .lines_apart
                .push((upper.to_string(), lower.to_string()));
        }
    }
    s
}

/// Task 36a fix round 3 (re-review 2, M1, probes 21 to 23 and probe 12): a page title in bold
/// at 14 points times the size factor under a small kicker line, and at the right on the
/// same two baselines a small label over a form number in the title's size and face. The
/// title stands like a value under its label; it is the largest text of the top of the
/// page, and it is the page's title however few words it has.
fn title_under_a_kicker(v: &Variant, title: &str) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let t = show(plain, 7.0 * k, 40.0, 760.0, "Department of Samples")
        + &show(plain, 7.0 * k, 400.0, 760.0, "Form")
        + &show(bold, 14.0 * k, 40.0, 744.0, title)
        + &show(bold, 14.0 * k, 400.0, 744.0, "12")
        + &show(bold, 9.0 * k, 40.0, 640.0, "Receipt")
        + &show(plain, 9.0 * k, 40.0, 580.0, "Sum 1.234,56");
    let mut s = sheet(v, t, 500.0, None);
    s.want.title = Some(title.into());
    s
}

/// The columns of [`footer_with_a_wrapped_column`]: the company, its managers under a title
/// (one paragraph of names, each with its office in brackets, wrapped over three lines), the
/// chair of its board under a title.
const WRAPPED_FOOTER: [&[&str]; 3] = [
    &[
        "Oakfield Rail Ltd",
        "Head office in Oldtown",
        "Registered at Oldtown HRB 12 345",
        "Tax-No. DE123456789",
    ],
    &[
        "Managers",
        "Dr. Robin Sample (Chair), Ann Specimen",
        "(Sales), Alex Other (Staff), Max Sampleton",
        "(Works)",
    ],
    &["Chair of the supervisory board", "Morgan Example"],
];

/// Task 36a part 1 (class D): a footer of three columns without rules at 7 points times the
/// size factor, top aligned at the baseline 64.85 and 1.2 sizes apart, the columns at an even
/// pitch from x 85. The middle column is a title over one paragraph of names wrapped over
/// three lines. The pitch is the width of its second line of names less a fifth of a point:
/// that line reaches 0.2 points past the left edge of the third column, where that column
/// is empty, and the first line of names ends 4.5 points (under a cell gap) before the name
/// of the third column, which starts at the left edge of its title. A page number stands one
/// line under the footer at the right. Each column reads top to bottom as one block, line by
/// line; the order of the columns is free.
fn footer_with_a_wrapped_column(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 7.0 * k;
    let step = 1.2 * size;
    // The width with the brackets as the fonts set them (333 units in both; `font_width`
    // counts them as a space).
    let width = |text: &str| {
        let space = font_width(plain, " ", 1000.0);
        let brackets = text.chars().filter(|c| matches!(c, '(' | ')')).count() as f32;
        font_width(plain, text, size) + brackets * (333.0 - space) * size / 1000.0
    };
    let pitch = width(WRAPPED_FOOTER[1][2]) - 0.2 * k;
    let mut t = String::new();
    for (c, lines) in WRAPPED_FOOTER.iter().enumerate() {
        for (n, line) in lines.iter().enumerate() {
            let x = 85.0 + pitch * c as f32;
            t += &show(plain, size, x, 64.85 - step * n as f32, line);
        }
    }
    let number = format!("Sheet {} / 2", crate::axes::sheet_no() + 1);
    t += &show(plain, size, 521.3, 64.85 - step * 4.0, &number);
    let mut s = sheet(v, t, 500.0, None);
    for lines in WRAPPED_FOOTER {
        if v.pages == Pages::Two {
            s.want.roles.push(("page-footer".into(), lines.join(" ")));
        }
        let chain: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        s.want.orders.push(chain.clone());
        s.want.together.push(chain);
        // Each line of a column stays a line of its own, the wrapped lines of the names
        // too (the ruling of fix round 3): what shows the columns the rows of a table can
        // show as well, and lines kept apart never join the texts of two items.
        for pair in lines.windows(2) {
            s.want
                .lines_apart
                .push((pair[0].to_string(), pair[1].to_string()));
        }
    }
    for (a, left) in WRAPPED_FOOTER.iter().enumerate() {
        for right in &WRAPPED_FOOTER[a + 1..] {
            for l in left.iter() {
                for r in right.iter() {
                    s.want.lines_apart.push((l.to_string(), r.to_string()));
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
        Rows::Table | Rows::Records => s.want.table(owned(rows), "table"),
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
        s.want.table(owned(&all), "table");
        // The note row may head a table of its own (Task 15b part 2d fix round 1): said.
        s.want.losses.push("may start a table of its own".into());
    } else {
        s.want.table(
            owned(&first.iter().map(|r| &r[..]).collect::<Vec<_>>()),
            "table",
        );
        // A header and one row are read as labels over their values (Task 18b: titles over
        // one table row read as pairs).
        let kind = if second_kind == Second::PlainOne {
            "label_value"
        } else {
            "table"
        };
        s.want.table(
            owned(&second.iter().map(|r| &r[..]).collect::<Vec<_>>()),
            kind,
        );
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
    s.want.heading("Ergebnisse der Messung", "title_block");
    s.want.title = Some("Ergebnisse der Messung".into());
    s.want.paragraphs.push(PROSE[..3].join(" "));
    s.want.paragraphs.push(PROSE[3..].join(" "));
    if beside == Beside::Table {
        s.want.table(
            owned(&rows.iter().map(|r| &r[..]).collect::<Vec<_>>()),
            "table",
        );
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

/// The lines of the head block of [`head_block_beside_a_logo`], set flush right.
const HEAD_BLOCK: [&str; 5] = [
    "Northwind Supply Ltd.",
    "Office register Fairhaven 4521",
    "Partners Alex Sample and Robin Example",
    "Ref. No.: AB123456789",
    "Member ID:  CD12EFG34567890123",
];

/// The paragraphs of the letter of [`head_block_beside_a_logo`], line by line.
const LETTER: [&[&str]; 3] = [
    &[
        "Thank you for the order that reached us on Monday morning.",
        "All parts are packed and leave our store by the end of the week.",
    ],
    &[
        "The carrier brings the boxes to your door and takes the old",
        "frames back with him at no further cost to you.",
    ],
    &[
        "Please look at every part when it arrives and write to us",
        "within ten days when something is missing or broken.",
    ],
];

/// Task 31b, class G: the head of a letter. A logo at the top left; right of it a block of
/// five lines at 9 points set flush right at the pitch of their size, the last two of them a
/// label with a colon and its value; under the logo a sender line at 9 points, lines of one
/// space each, the recipient's address of three lines set in, a date and a subject line with
/// a small stroked box right of them (a label over it, a label and its value inside), then
/// the letter in paragraphs with a blank line between them. The head block is read whole at
/// the page head, before the address and the letter, each of its lines a line of its own;
/// no line of it stands between two paragraphs of the letter. With `two` the block is its
/// first two lines only: two lines set flush right, the shorter one first, are two lines and
/// no line that wraps.
fn head_block_beside_a_logo(v: &Variant, two: bool) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let (small, size) = (9.0 * k, 11.0 * k);
    let mut t = crate::common::logo(30.0, 767.0, 88.0, 50.0);
    let head = if two {
        &HEAD_BLOCK[..2]
    } else {
        &HEAD_BLOCK[..]
    };
    for (n, l) in head.iter().enumerate() {
        t += &right(plain, small, 565.0, 808.0 - small * n as f32, l);
    }
    t += &show(
        plain,
        small,
        30.0,
        750.0,
        "Northwind Supply Ltd. - 12 Harbor Road - 12345 Fairhaven",
    );
    for y in [738.0, 726.0, 714.0, 657.0, 645.0, 633.0] {
        t += &show(plain, size, 30.0, y, " ");
    }
    let address = ["Erika Example", "34 Mill Lane", "67890 Oakfield"];
    for (n, l) in address.iter().enumerate() {
        t += &show(plain, size, 56.91, 699.0 - 13.0 * n as f32, l);
    }
    t += &show(plain, size, 32.0, 622.0, "12 May 2026");
    t += &show(bold, size, 30.0, 609.0, "Your order of last week");
    t += &right(plain, small, 565.0, 622.0, "Quote in every reply");
    t += "q 0.5 w 458 607 107 13 re S Q\n";
    t += &right(plain, small, 562.0, 609.5, "Ref: 123456789");
    let mut y = 583.0;
    for para in LETTER {
        for l in para {
            t += &show(plain, size, 30.0, y, l);
            y -= 12.0 * k;
        }
        t += &show(plain, size, 30.0, y, " ");
        y -= 12.0 * k;
    }
    let mut s = sheet(v, t, y - 60.0, None);
    // The block is whole and in its order; where it stands on the page is free.
    s.want.order = vec![HEAD_BLOCK[0].into(), HEAD_BLOCK[1].into()];
    if !two {
        s.want.order.extend([
            "Partners".to_string(),
            "AB123456789".into(),
            "CD12EFG34567890123".into(),
        ]);
        // Its two lines of a label and a value keep each label with its value on a line of
        // its own.
        s.want
            .lines_apart
            .push(("AB123456789".into(), "Member ID".into()));
        s.want
            .lines_joined
            .push(("Ref. No.:".into(), "AB123456789".into()));
        s.want
            .lines_joined
            .push(("Member ID:".into(), "CD12EFG34567890123".into()));
    }
    // The letter reads in its order, wherever the block stands: the address, then its
    // paragraphs one after the other. A chain of its own, as the place of the block against
    // the letter is free.
    let letter = [address[0], LETTER[0][0], LETTER[1][0], LETTER[2][0]];
    s.want.orders.push(letter.map(String::from).to_vec());
    for w in letter.windows(2) {
        s.want.lines_apart.push((w[0].into(), w[1].into()));
    }
    s.want
        .paragraphs
        .extend(LETTER.iter().map(|para| para.join(" ")));
    s.want
        .lines_apart
        .push((HEAD_BLOCK[0].into(), HEAD_BLOCK[1].into()));
    // No line of the block stands in a paragraph of the letter.
    for l in head {
        for para in LETTER {
            s.want.apart.push(((*l).into(), para[0].into()));
        }
    }
    s
}

/// The rows of [`framed_pairs`]: a label and its value.
const FRAMED_PAIRS: [(&str, &str); 4] = [
    ("Holder", "Alex Sample Ltd"),
    ("Account", "AB12 3456 7890 1234 5678 90 / CDEFGHIJ123"),
    ("Amount", "45,67"),
    ("Reference", "123456789-987654321"),
];

/// Task 31b, class B (a guard: the cut at equal bands of Task 31a reads it): a stroked frame
/// around four rows of a label and its value in two columns, at 11 points times the size
/// factor, the rows `pitch` sizes apart (15 points at size 11 is 1.36, 17 points is 1.55).
/// Over the frame stands a paragraph whose last line is one space, on the frame's top edge;
/// the amount is followed by a space and a currency sign in a run of its own. With `beside`
/// a picture stands right of the frame. Each label reads with its value at any row pitch a
/// writer uses in a frame.
fn framed_pairs(v: &Variant, pitch: f32, beside: bool) -> Sheet {
    framed_pairs_in(v, pitch, beside, 401.25)
}

/// The axes of [`a_payment_box_of_two_widths`]: a file of two pages in every variant.
const TWO_PAGES: Axes = Axes {
    pages: &[crate::axes::Pages::Two],
    ..DOC
};

/// The widths of the frame of [`a_payment_box_of_two_widths`] on its first and second page.
const PAYMENT_BOX_WIDTHS: [f32; 2] = [401.25, 505.0];

/// Task 26, class B6: a payment box whose width differs between pages. The frame of
/// [`framed_pairs`] around its four rows of a label and its value, on two pages: the text of
/// the rows stands at the same places on both, the frame is a quarter wider on the second
/// (as a box that is drawn to the width of the block around it). The four labels read with
/// their values on each page alike: the width of the frame is no evidence of anything.
fn a_payment_box_of_two_widths(v: &Variant) -> Sheet {
    let width = PAYMENT_BOX_WIDTHS[crate::axes::sheet_no().min(1)];
    let mut s = framed_pairs_in(v, 15.0 / 11.0, false, width);
    for (label, _) in FRAMED_PAIRS {
        s.want.kinds.push((label.into(), "label_value"));
    }
    s
}

/// [`framed_pairs`] with the frame `width` points wide.
fn framed_pairs_in(v: &Variant, pitch: f32, beside: bool, width: f32) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 11.0 * k;
    let step = pitch * size;
    let top = 640.0;
    let para = [
        "Please send the sum to the account named in the frame under these lines",
        "and quote the reference, so that we can match it with your order at once.",
    ];
    let mut t = String::new();
    for (n, l) in para.iter().enumerate() {
        t += &show(plain, size, 30.0, top - 12.0 * k * n as f32, l);
    }
    // The frame: its top edge on the baseline of the line of one space.
    let edge = top - 12.0 * k * 2.0 - 3.0 * k;
    t += &show(plain, size, 30.0, edge, " ");
    let first = edge - 15.0 * k;
    let last = first - step * 3.0;
    let bottom = last - 8.0 * k;
    t += &format!(
        "q 1 0 0 RG 0.5 w 30 {bottom} {width} {} re S Q\n",
        edge - bottom
    );
    let x_value = 34.0 + 11.8 * size;
    for (n, (label, value)) in FRAMED_PAIRS.iter().enumerate() {
        let y = first - step * n as f32;
        t += &show(plain, size, 34.0, y, label);
        if *label == "Amount" {
            let amount = format!("{value} ");
            t += &show(plain, size, x_value, y, &amount);
            t += &show(
                plain,
                size,
                x_value + font_width(plain, &amount, size),
                y,
                "$",
            );
        } else {
            t += &show(plain, size, x_value, y, value);
        }
    }
    if beside {
        t += &crate::common::logo(481.5, bottom, 55.0 * k, 55.0 * k);
    }
    let mut s = sheet(v, t, bottom - 60.0, None);
    for (label, value) in FRAMED_PAIRS {
        let value = if label == "Amount" { "45,67 $" } else { value };
        s.built.fields.push((label, Ok(Some(value))));
    }
    s.want.paragraphs.push(para.join(" "));
    s.want.order = vec![para[0].into(), "Holder".into(), "Reference".into()];
    s
}

/// Task 31b, class C1: a notes block in the page head. A small bold label over a thin rule
/// with two lines under it, and a paragraph lower on the page. The label names its two
/// lines: it is no heading over the rest of the page, and the paragraph is no part of its
/// block.
fn labelled_block(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 9.0 * k;
    let top = 760.0;
    let lines = ["Bring the crates to the side door", "Ring twice before ten"];
    let mut t = show(bold, 7.0 * k, 40.0, top, "Notes");
    t += &format!(
        "q 0.5 w 40 {} m 240 {} l S Q\n",
        top - 3.0 * k,
        top - 3.0 * k
    );
    for (n, l) in lines.iter().enumerate() {
        t += &show(plain, size, 40.0, top - 13.0 * k - 11.0 * k * n as f32, l);
    }
    let para = [
        "The driver of the green van takes the long road through the valley",
        "and stops at the mill before he turns back to the yard at noon.",
    ];
    let y = top - 80.0;
    for (n, l) in para.iter().enumerate() {
        t += &show(plain, size, 40.0, y - 11.0 * k * n as f32, l);
    }
    let mut s = sheet(v, t, y - 80.0, None);
    s.want.no_headings.push("Notes".into());
    s.want.order = vec![
        "Notes".into(),
        lines[0].into(),
        lines[1].into(),
        para[0].into(),
    ];
    s.want.paragraphs.push(para.join(" "));
    s.want.apart.push(("Notes".into(), para[0].into()));
    s.want.apart.push((lines[1].into(), para[0].into()));
    s
}

/// Task 31b, class C2: two stroked boxes side by side, each with a small bold label over a
/// rule and two lines under it. Each label reads with its own lines, the left box first.
fn labelled_boxes(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 9.0 * k;
    let top = 760.0;
    let boxes = [
        (
            40.0,
            "Sender",
            ["Harbor store, gate four", "Open from eight to five"],
        ),
        (
            300.0,
            "Carrier",
            ["Green van of the valley line", "Leaves the yard at noon"],
        ),
    ];
    let mut t = String::new();
    for (x, label, lines) in boxes {
        t += &format!("q 0.5 w {x} {} 240 {} re S Q\n", top - 44.0 * k, 44.0 * k);
        t += &show(bold, 7.0 * k, x + 4.0, top - 9.0 * k, label);
        t += &format!(
            "q 0.5 w {x} {} m {} {} l S Q\n",
            top - 12.0 * k,
            x + 240.0,
            top - 12.0 * k
        );
        for (n, l) in lines.iter().enumerate() {
            t += &show(
                plain,
                size,
                x + 4.0,
                top - 23.0 * k - 11.0 * k * n as f32,
                l,
            );
        }
    }
    let mut s = sheet(v, t, top - 120.0, None);
    for (_, label, lines) in boxes {
        s.want.no_headings.push(label.into());
        s.want
            .order
            .extend([label.to_string(), lines[0].into(), lines[1].into()]);
    }
    s.want.apart.push(("Sender".into(), boxes[1].2[0].into()));
    s.want.apart.push(("Carrier".into(), boxes[0].2[0].into()));
    s.want
        .apart
        .push((boxes[0].2[1].into(), boxes[1].2[0].into()));
    s
}

/// Task 31b fix round 2 (I8): a row of four labels sitting on underlines that touch at the
/// bounds of the fields (a rule in four pieces, no upright lines), two rows of values under
/// them and a closing rule, with a block of lines at the left over it and one at the right.
/// By the evidence of a ruled block (a rule in pieces whose joints stand between the cells,
/// rows of one geometry, titles over the rule) the labels are the header of a table and each
/// row of values reads whole under them.
fn fields_on_touching_underlines(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let mut t = String::new();
    for (n, l) in ["Harbor store", "Gate four", "Fairhaven"]
        .iter()
        .enumerate()
    {
        t += &show(plain, size, 40.0, 740.0 - 12.0 * k * n as f32, l);
    }
    for (n, l) in ["Sheet 3", "Clerk Alex Sample", "Week 19", "Shift two"]
        .iter()
        .enumerate()
    {
        t += &right(plain, 8.0 * k, 540.0, 746.0 - 10.0 * k * n as f32, l);
    }
    let xs = [40.0, 170.0, 300.0, 420.0, 540.0];
    let top = 660.0;
    let rows = [
        ["Name", "Town", "Code", "Year"],
        ["Alex Sample", "Oakfield", "AB12", "2026"],
        ["Robin Example", "Fairhaven", "CD34", "2025"],
    ];
    for (r, row) in rows.iter().enumerate() {
        let y = if r == 0 {
            top
        } else {
            top - 3.0 * k - 13.0 * k * r as f32
        };
        for (c, cell) in row.iter().enumerate() {
            t += &show(plain, size, xs[c] + 2.0, y, cell);
        }
    }
    for w in xs.windows(2) {
        t += &format!(
            "q 0.5 w {} {} m {} {} l S Q\n",
            w[0],
            top - 2.0 * k,
            w[1],
            top - 2.0 * k
        );
    }
    let close = top - 3.0 * k - 26.0 * k - 5.0 * k;
    t += &format!("q 0.5 w 40 {close} m 540 {close} l S Q\n");
    let mut s = sheet(v, t, close - 60.0, None);
    s.want.table(
        owned(&rows.iter().map(|r| &r[..]).collect::<Vec<_>>()),
        "table",
    );
    s
}

/// The axes of a form whose edges are measured against each other (Task 31b fix round 7):
/// one face, so that a line set flush right starts where the shape wants it.
const TRACED: Axes = Axes {
    data: &[Data::CourierBold],
    ..DOC
};

/// One text of a form in the geometry of a traced page: where it starts (flush right:
/// where it ends), its baseline, its size, bold, flush right, and the text.
type Placed = (f32, f32, f32, bool, bool, &'static str);

/// Plain text from `x` on the baseline `y`, of 6.7 points.
const fn at(x: f32, y: f32, text: &'static str) -> Placed {
    (x, y, 6.7, false, false, text)
}

/// A bold value from `x` on the baseline `y`, of 7.6 points.
const fn value(x: f32, y: f32, text: &'static str) -> Placed {
    (x, y, 7.6, true, false, text)
}

/// Text of `size` that ends at `x` on the baseline `y`.
const fn ends(x: f32, y: f32, size: f32, bold: bool, text: &'static str) -> Placed {
    (x, y, size, bold, true, text)
}

/// The texts `items` drawn with the baseline `top` at 740: every place and size times the
/// variant's size factor, from the left margin and from that baseline, so the page keeps
/// its proportions.
fn placed(v: &Variant, top: f32, items: &[Placed]) -> String {
    let k = v.k();
    let (plain, bold) = v.faces();
    let mut t = String::new();
    for &(x, y, size, heavy, flush, text) in items {
        let (x, y) = (40.0 + k * (x - 40.0), 740.0 - k * (top - y));
        let font = if heavy { bold } else { plain };
        t += &if flush {
            right(font, k * size, x, y, text)
        } else {
            show(font, k * size, x, y, text)
        };
    }
    t
}

/// The lines of the note of [`note_beside_field_rows`].
const NOTE: [&str; 11] = [
    "This is a record of",
    "fees and is also",
    "sent on to the",
    "main office. This",
    "sheet is to be",
    "used to fill in",
    "Form 8863 and to",
    "claim a study",
    "credit. Hand it to the",
    "adviser of your hall or",
    "use it for the return.",
];

/// The rows of fields of [`note_beside_field_rows`], left of its note.
const NOTE_FIELDS: [Placed; 67] = [
    (
        141.8,
        369.1,
        9.0,
        true,
        false,
        "RECORDS OFFICE AND REGISTER USE ONLY SAMPLE",
    ),
    (262.9, 358.0, 8.0, true, false, "REVISED COPY"),
    at(66.0, 348.0, "Maker's name"),
    at(300.4, 348.0, "1 Fees taken in for"),
    ends(460.2, 347.9, 6.7, false, "Ref No. 1545-1574"),
    at(306.1, 341.3, "tested lessons and added"),
    value(66.0, 336.0, "NORTHWIND HALL"),
    at(306.1, 334.5, "costs"),
    at(66.0, 324.9, "Street address"),
    at(239.0, 324.9, "Room no."),
    ends(391.5, 323.6, 7.6, true, "3,600"),
    at(299.1, 323.1, "$"),
    at(401.5, 308.5, "For the year 2026"),
    at(300.4, 313.4, "2"),
    value(66.0, 313.0, "10 HARBOR AVENUE"),
    at(66.0, 301.9, "Town"),
    at(149.1, 301.9, "Region"),
    at(204.4, 301.9, "Land"),
    at(239.0, 301.9, "Postal code"),
    value(66.0, 289.9, "OAKFIELD"),
    value(167.8, 289.9, "OW"),
    value(260.3, 289.9, "ZIP"),
    value(404.6, 289.7, "Form 77-T"),
    at(300.4, 278.8, "3"),
    at(66.0, 277.8, "Phone number:"),
    at(66.0, 267.3, "Maker's register number"),
    at(183.7, 267.3, "Member's number"),
    at(300.4, 255.7, "4 Changes made for a"),
    at(398.2, 255.7, "5 Grants or awards"),
    value(99.2, 255.3, "37-700XXXX"),
    value(215.1, 255.3, "605-00-XXXX"),
    at(306.1, 247.7, "past year"),
    at(66.0, 244.2, "Member's name"),
    value(66.0, 232.2, "DANA REED"),
    at(299.1, 230.8, "$"),
    at(396.0, 230.8, "$"),
    at(66.0, 221.1, "Street address"),
    at(239.0, 221.1, "Flat no."),
    at(300.4, 221.1, "6 Changes to"),
    at(398.2, 221.1, "7 Ticked if the sum"),
    at(404.0, 213.9, "in box 1 holds"),
    at(306.1, 213.0, "grants or awards"),
    value(66.0, 209.2, "160 COLLEGE DRIVE"),
    at(404.0, 206.7, "sums for a"),
    at(306.1, 205.0, "of an older term"),
    at(404.0, 199.5, "study term"),
    at(66.0, 198.1, "Town"),
    at(149.1, 198.1, "Region"),
    at(204.4, 198.1, "Land"),
    at(239.0, 198.1, "Postal code"),
    at(404.0, 192.3, "starting January to"),
    value(66.0, 186.1, "OAKFIELD"),
    value(167.8, 186.1, "OW"),
    value(260.3, 186.1, "ZIP"),
    at(404.0, 185.1, "March 2027"),
    at(299.1, 184.7, "$"),
    at(66.0, 175.0, "Account or file number (see notes)"),
    at(217.3, 175.0, "8 Ticked if at"),
    at(300.4, 175.0, "9 Ticked if a senior"),
    ends(484.6, 175.0, 6.7, false, "10 Cover claims paid or refunded"),
    at(383.9, 165.0, "4"),
    at(223.1, 164.4, "least half time"),
    at(306.1, 164.4, "member"),
    at(396.0, 161.6, "$"),
    at(62.2, 149.4, "Form 77-T"),
    at(173.5, 149.5, "(keep for your files)"),
    at(312.4, 149.5, "www.example.org/Form77T"),
];

/// Task 31b fix round 7 (the path of a public form, irs-f6744 untagged page 108): rows of
/// fields with a narrow note column at their right, set flush right: two title lines, two
/// lines that name the copy, and a note of eleven lines at a pitch of its own. A foot line
/// under the fields ends where the note ends. Eight of the sixteen lines of the note column
/// pass a row of the fields by chance, four of them lines of the note itself, whose other
/// seven lines pass none. The note is one block: its lines follow each other, and no row
/// of the fields holds one of them.
fn note_beside_field_rows(v: &Variant) -> Sheet {
    let mut t = placed(v, 369.1, &NOTE_FIELDS);
    let mut right_column = vec![
        ends(567.4, 323.9, 10.0, true, "Lesson"),
        ends(567.4, 312.4, 10.0, true, "Record"),
        ends(567.4, 277.2, 8.0, true, "Copy B"),
        ends(567.4, 265.4, 8.0, true, "For Member"),
        ends(
            567.4,
            149.5,
            6.7,
            false,
            "Office of the Registrar - Member Records Service",
        ),
    ];
    for (n, l) in NOTE.iter().enumerate() {
        right_column.push(ends(567.4, 248.5 - 7.7 * n as f32, 6.7, false, l));
    }
    t += &placed(v, 369.1, &right_column);
    let mut s = sheet(v, t, 740.0 - v.k() * 290.0, None);
    s.want.may_title.push(NOTE_FIELDS[0].5.into());
    s.want
        .orders
        .push(NOTE.iter().map(|l| l.to_string()).collect());
    for field in [
        "DANA REED",
        "6 Changes to",
        "160 COLLEGE DRIVE",
        "Postal code",
        "March 2027",
    ] {
        for l in NOTE {
            s.want.apart.push((l.into(), field.into()));
        }
    }
    s
}

/// The lines of the note of [`amount_before_a_note_column`], under its title lines.
const AMOUNT_NOTE: [&str; 11] = [
    "This is a record of",
    "costs and is also",
    "sent on to the main",
    "office. If you must",
    "file a return, a",
    "charge or another",
    "step may follow",
    "for you if the",
    "office finds that a",
    "sum was left out",
    "since you stated a",
];

/// Task 31b fix round 7 (the path of a public form, irs-f6744 untagged page 108, its first
/// form): the right half of a form, boxes of fields with a note column at their right, set
/// flush right. The amount of box 1 is set flush right in its box: it starts a little after
/// the end of the longest line of the boxes and ends a gutter's width before the note
/// column, with its currency sign at the left of its row and a line of the note column at
/// the right. The amount is the value of the label over it: it reads after its label,
/// before the lines of the note column, and on no line of that column.
fn amount_before_a_note_column(v: &Variant) -> Sheet {
    let label = "1 Loan interest taken in by the lender";
    let mut items = vec![
        at(398.1, 725.7, "Ref No. 1545-1576"),
        ends(566.3, 710.2, 11.4, true, "Member"),
        (401.9, 694.5, 21.0, true, false, "2026"),
        ends(566.3, 696.4, 11.4, true, "Loan Costs"),
        ends(566.3, 682.7, 11.4, true, "Summary"),
        (404.8, 668.4, 9.5, true, false, "Form 88-E"),
        at(302.5, 657.1, label),
        ends(566.3, 656.0, 9.5, true, "Copy B"),
        (302.0, 644.5, 9.5, false, false, "$"),
        ends(488.0, 644.7, 7.6, true, "2,250"),
        ends(566.3, 644.3, 8.6, true, "For Holder"),
        at(
            302.5,
            554.6,
            "2 If ticked, box 1 does not hold sign-up fees",
        ),
        ends(566.3, 553.4, 6.7, false, "sum for the cost of"),
        ends(
            467.2,
            547.9,
            6.7,
            false,
            "or interest added to loans made before the first",
        ),
        at(493.7, 546.3, "a loan in the year."),
        at(308.3, 541.3, "of June 2004"),
        at(310.0, 530.3, "www.example.org/Form88E"),
        ends(
            566.3,
            530.3,
            6.7,
            false,
            "Office of the Registrar - Member Records Service",
        ),
    ];
    // Two lines of the note start where the note column starts, as its lines under them.
    for (n, l) in AMOUNT_NOTE.iter().enumerate() {
        let y = 632.0 - 7.15 * n as f32;
        items.push(match n {
            1 | 3 => at(494.1, y, l),
            _ => ends(566.3, y, 6.7, false, l),
        });
    }
    // The leader dots after the last line of box 2, each a piece of its own.
    for n in 0..8 {
        items.push(at(383.4 + 11.43 * n as f32, 541.3, "."));
    }
    let t = placed(v, 725.7, &items);
    let mut s = sheet(v, t, 740.0 - v.k() * 260.0, None);
    s.want.may_title.push("2026".into());
    s.want
        .orders
        .push(vec![label.into(), "2,250".into(), "Copy B".into()]);
    s.want
        .lines_apart
        .push(("2,250".into(), "For Holder".into()));
    s
}

/// Task 31b fix round 7 (the path of a public form, irs-f6744 untagged pages 174 and 175):
/// three labels of boxes on one row, set so close that they are one piece, from the left
/// edge of a block of fields over the white space beside it into the first two of the four
/// rows of the block at its right, with a value at the far right of their row. Under them,
/// at the left, the second line of the first label and the values of the first two boxes.
/// The row of labels reads before the second line of its first label and before its
/// values.
///
/// With `boxes` (fix round 8, review R3) the boxes of the fields are drawn as on the form,
/// in rule pieces that form no rows and columns: the reader's grid search gives the text
/// up, and it is read as lines of paragraphs. The value at the far right of the row of
/// labels is the value of the third box; the line that follows it in reading order, far
/// left and a little lower, is the second line of the label of the first box. They are
/// two lines of two boxes, never one line of one paragraph.
fn label_row_over_two_blocks(v: &Variant, boxes: bool) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let labels = "1 Income 2 Gross income 3 Chapter mark. Enter the digit 3 or the digit 4";
    // The row of labels runs from 40.3 to 203.5, whatever the widths of the face.
    let size = 163.2 / font_width(plain, labels, 1.0);
    let small = |x: f32, y: f32, text: &'static str| -> Placed { (x, y, 5.4, false, false, text) };
    // In drawn boxes each label stands in its own box, as on the form: from its place to
    // its end whatever the widths of the face, with a space after its last word, so close
    // to the next that the three are one piece of their row still.
    let boxed: [(f32, f32, f32, &'static str); 3] = [
        (40.3, 62.7, 390.7, "1 Income "),
        (68.1, 107.6, 390.7, "2 Gross income "),
        (112.4, 203.5, 389.2, "3 Chapter mark. Enter digit 3 or 4"),
    ];
    let mut items: Vec<Placed> = if boxes {
        boxed
            .iter()
            .map(|&(x0, x1, y, text)| {
                let size = (x1 - x0) / font_width(plain, text.trim_end(), 1.0);
                (x0, y, size, false, false, text)
            })
            .collect()
    } else {
        vec![(40.3, 390.7, size, false, false, labels)]
    };
    items.extend([
        small(218.6, 389.0, "3"),
        small(45.0, 384.6, "code"),
        small(112.4, 379.9, "3a Waiver code"),
        small(166.7, 380.7, "04"),
        small(179.0, 379.9, "4a Waiver code"),
        small(47.9, 373.7, "20"),
        small(80.9, 374.6, "2000"),
        small(112.4, 370.7, "3b Tax rate"),
        small(157.2, 370.2, "0.00"),
        small(179.0, 370.7, "4b Tax rate"),
        small(40.3, 361.5, "5 Holding allowance"),
        small(40.3, 352.2, "6 Net income"),
        small(205.0, 352.6, "1800"),
        small(40.3, 343.0, "7a State tax held"),
    ]);
    if boxes {
        items.push(small(40.3, 333.8, "7b State tax paid"));
    }
    let mut t = placed(v, 390.7, &items);
    if boxes {
        // The rule pieces of the form around these fields, from one end to the other.
        const RULES: [[f32; 4]; 34] = [
            [37.1, 396.1, 104.0, 396.1],
            [103.4, 396.1, 237.1, 396.1],
            [36.9, 368.4, 65.2, 368.4],
            [37.3, 368.2, 37.3, 396.3],
            [64.8, 368.4, 109.5, 368.4],
            [65.0, 368.2, 65.0, 396.3],
            [109.2, 386.9, 237.1, 386.9],
            [236.9, 386.7, 236.9, 396.3],
            [109.4, 386.7, 109.4, 396.3],
            [109.2, 377.6, 176.1, 377.6],
            [109.4, 377.4, 109.4, 387.1],
            [109.2, 368.4, 159.5, 368.4],
            [109.4, 368.2, 109.4, 377.8],
            [159.1, 368.4, 176.1, 368.4],
            [175.7, 377.6, 237.1, 377.6],
            [236.9, 377.4, 236.9, 387.1],
            [175.9, 377.4, 175.9, 387.1],
            [175.7, 368.4, 220.5, 368.4],
            [175.9, 368.2, 175.9, 377.8],
            [220.1, 368.4, 237.1, 368.4],
            [236.9, 368.2, 236.9, 377.8],
            [36.9, 359.1, 237.1, 359.1],
            [236.9, 359.0, 236.9, 368.6],
            [37.3, 359.0, 37.3, 368.6],
            [36.9, 349.9, 237.1, 349.9],
            [236.9, 349.7, 236.9, 359.3],
            [37.3, 349.7, 37.3, 359.3],
            [36.9, 340.7, 237.1, 340.7],
            [236.9, 340.5, 236.9, 350.1],
            [37.3, 340.5, 37.3, 350.1],
            [236.9, 377.4, 236.9, 396.3],
            [36.9, 331.5, 237.1, 331.5],
            [236.9, 331.3, 236.9, 340.9],
            [37.3, 331.3, 37.3, 340.9],
        ];
        let at = |x: f32, y: f32| (40.0 + k * (x - 40.0), 740.0 - k * (390.7 - y));
        for [x0, y0, x1, y1] in RULES {
            let ((x0, y0), (x1, y1)) = (at(x0, y0), at(x1, y1));
            t += &format!("q 0.5 w {x0} {y0} m {x1} {y1} l S Q\n");
        }
    }
    let mut s = sheet(v, t, 740.0 - k * 110.0, None);
    let row: Vec<String> = if boxes {
        boxed.iter().map(|l| l.3.trim_end().to_string()).collect()
    } else {
        vec![labels.into()]
    };
    s.want.orders.push(
        row.into_iter()
            .chain(["code".into(), "20".into()])
            .collect(),
    );
    s.want.lines_apart.push(("3".into(), "code".into()));
    s
}

/// Class K of Task 31 (masked geometry of an owner's page): a ruled table of three bold header
/// cells over five empty rows, every cell side a stroke of its own, and right of it, outside
/// the table's frame, two fields: a label on an underline with its value under it. The first
/// label stands on the baseline of the header cells. The header row stands apart from the
/// fields, and each label stands with its value.
///
/// `third` draws the underline of a third field farther right, at a height between the two
/// underlines. The strips between rules are searched level by level, so with it the two
/// underlines bound no strip, and the shape runs through the path of the owner's page (which
/// holds more rules): the header cells a paragraph of a grid read as no table, each label a
/// paragraph with its value, and the first label on the line of the header cells in the text
/// pass. Without it the two underlines bound a strip that holds the first value and the
/// second label.
fn side_fields_beside_an_empty_table(v: &Variant, third: bool) -> Sheet {
    side_fields_beside_a_table(v, third, false, false, 2)
}

/// Task 26, class A5: an underlined label with its value under it beside an empty ruled
/// grid, the label's baseline inside the grid's header band. The table of
/// [`side_fields_beside_an_empty_table`] with the first of its fields only, its underline as
/// long as the label's words: one underline bounds no strip, so the label and its value are
/// two lines of text in one size, and the underline of the label is what says which of them
/// is the label. The label reads as a pair with its value; the header cells of the grid
/// stand apart from it.
fn an_underlined_label_beside_an_empty_grid(v: &Variant) -> Sheet {
    side_fields_beside_a_table(v, false, false, false, 1)
}

/// The rows of the table of [`side_fields_beside_a_table`] when it is filled.
const FRAME_ROWS: [[&str; 3]; 5] = [
    ["Left rail", "2", "sound"],
    ["Right rail", "2", "sound"],
    ["Cross bar", "4", "one bent"],
    ["Corner plate", "8", "sound"],
    ["Foot", "4", "two worn"],
];

/// [`side_fields_beside_an_empty_table`] in its other layouts (review 1 of Task 31c, M3).
/// With `mirror` the two fields stand left of the table, so the first label comes first on
/// the line of the header cells. With `filled` the five rows of the table hold text
/// ([`FRAME_ROWS`]), so the rows under the header row are no empty lines.
/// `count` is how many of the two fields are drawn; a field alone is wanted as a pair of
/// its label and its value.
fn side_fields_beside_a_table(
    v: &Variant,
    third: bool,
    mirror: bool,
    filled: bool,
    count: usize,
) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let top = 707.5;
    // The fields start 6 points after the table, or the table 6 points after the longer
    // underline.
    let (xs, at) = if mirror {
        ([117.0, 312.0, 383.0, 454.0], 31.0)
    } else {
        ([31.0, 226.0, 297.0, 368.0], 374.0)
    };
    let row = 10.0 * k;
    let mut t = String::new();
    // Each cell's four sides, every one a stroke of its own.
    for r in 0..6 {
        let (y1, y0) = (top - row * r as f32, top - row * (r + 1) as f32);
        for c in xs.windows(2) {
            let (x0, x1) = (c[0], c[1]);
            t += &format!(
                "q 1 w {} {y1} m {} {y1} l S {} {y0} m {} {y0} l S Q\n",
                x0 - 0.5,
                x1 + 0.5,
                x0 - 0.5,
                x1 + 0.5
            );
            t += &format!(
                "q 1.25 w {x0} {} m {x0} {} l S {x1} {} m {x1} {} l S Q\n",
                y0 - 0.63,
                y1 + 0.63,
                y0 - 0.63,
                y1 + 0.63
            );
        }
    }
    let titles = ["Part of the frame", "Count", "Note"];
    for (c, title) in titles.iter().enumerate() {
        t += &show(bold, 8.0 * k, xs[c] + 3.0, top - 7.5 * k, title);
    }
    // The underlines are as long as on the owner's page (47 and 79 points), not as the labels.
    let fields = [
        ("Checked by", "A/B", 0.0, 47.45),
        ("Passed on to", "C/D", 24.14, 79.33),
    ];
    let fields = &fields[..count];
    if filled {
        for (r, cells) in FRAME_ROWS.iter().enumerate() {
            for (c, cell) in cells.iter().enumerate() {
                let y = top - row * (r + 1) as f32 - 7.5 * k;
                t += &show(plain, 8.0 * k, xs[c] + 3.0, y, cell);
            }
        }
    }
    for &(label, value, down, rule) in fields {
        // A field alone has the underline of its label: as long as the label's words.
        let rule = if count == 1 {
            font_width(plain, label, 7.0 * k)
        } else {
            rule
        };
        let y = top - 6.57 * k - down * k;
        t += &show(plain, 7.0 * k, at, y, label);
        t += &format!(
            "q 0.5 w {at} {} m {} {} l S Q\n",
            y - 0.58,
            at + rule,
            y - 0.58
        );
        t += &show(plain, 7.0 * k, at, y - 8.05 * k, value);
    }
    if third {
        t += &format!(
            "q 0.5 w 480 {} m 540 {} l S Q\n",
            top - 20.0 * k,
            top - 20.0 * k
        );
    }
    // The page holds the table and the fields only: no body text makes a column beside them.
    let mut built = Built::new(&v.geo());
    built.page = FormPage {
        template: t,
        ..FormPage::default()
    };
    let mut s = Sheet::new(built);
    s.once = crate::axes::shown(&s.built.page.template);
    s.want
        .orders
        .push(titles.iter().map(|t| t.to_string()).collect());
    for &(label, value, ..) in fields {
        // The label stands with its value, apart from the header row.
        s.want
            .orders
            .push(vec![label.to_string(), value.to_string()]);
        for title in titles {
            s.want.lines_apart.push((title.into(), label.into()));
            s.want.lines_apart.push((title.into(), value.into()));
        }
    }
    if let [first, second] = fields {
        s.want.lines_apart.push((first.1.into(), second.0.into()));
    } else {
        for &(label, value, ..) in fields {
            s.built.fields.push((label, Ok(Some(value))));
            s.want.kinds.push((label.into(), "label_value"));
        }
    }
    if filled {
        // The table with its rows, apart from the fields.
        let rows = std::iter::once(titles).chain(FRAME_ROWS);
        s.want
            .tables
            .push(rows.map(|r| r.map(String::from).to_vec()).collect());
    }
    s
}

/// The paragraph of [`letter_past_a_box`]: a short line, a longer one, a line of the full
/// width and a short last line, each filled from these words up to its measure.
const PAST_A_BOX: [(&str, f32); 4] = [
    ("The parts you asked for are here,", 140.0),
    (
        "and the carrier brings all six boxes to your door on the first day of the week,",
        339.0,
    ),
    (
        "where he also takes the old frames back with him at no further cost to you, so please keep them near the gate until he comes",
        535.0,
    ),
    ("and sign the list he hands you.", 201.0),
];

/// Task 31c, class F (masked geometry of an owner's page): the head of a letter as in
/// [`head_block_beside_a_logo`], and a paragraph whose first lines are short and stand left
/// of the box of the head, while its third line runs the full width under the box. Under it
/// a label on the right and a line on the left, then a paragraph of full width. The
/// paragraph beside the box reads in one piece: the column flows past the block on its
/// right, and the head block and the box read before or after it.
fn letter_past_a_box(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let (small, size) = (9.0 * k, 11.0 * k);
    let mut t = crate::common::logo(30.0, 767.0, 88.0, 50.0);
    for (n, l) in HEAD_BLOCK.iter().enumerate() {
        t += &right(plain, small, 565.0, 808.0 - small * n as f32, l);
    }
    t += &show(
        plain,
        small,
        30.0,
        750.0,
        "Northwind Supply Ltd. - 12 Harbor Road - 12345 Fairhaven",
    );
    for y in [738.0, 726.0, 714.0, 657.0, 645.0, 633.0] {
        t += &show(plain, size, 30.0, y, " ");
    }
    let address = ["Erika Example", "34 Mill Lane", "67890 Oakfield"];
    for (n, l) in address.iter().enumerate() {
        t += &show(plain, size, 56.91, 699.0 - 13.0 * n as f32, l);
    }
    t += &show(plain, size, 32.0, 622.0, "12 May 2026");
    t += &show(bold, size, 32.0, 609.0, "Your order of last week");
    t += &right(plain, size, 563.0, 622.0, "Quote in every reply");
    t += "q 0.5 w 458 607 107 13 re S Q\n";
    t += &right(plain, size, 563.0, 609.5, "Ref: 123456789");
    let step = 12.0 * k;
    let mut y = 583.0;
    t += &show(plain, size, 30.0, y, "Dear Ms Example,");
    t += &show(plain, size, 30.0, y - step, " ");
    y -= 2.0 * step;
    // Each line holds the words that fit its measure: the same places in every face.
    let lines: Vec<String> = PAST_A_BOX
        .iter()
        .map(|(words, measure)| wrapped(words, plain, size, *measure).swap_remove(0))
        .collect();
    for l in &lines {
        t += &show(plain, size, 30.0, y, l);
        y -= step;
    }
    // A label on the right and a line on the left under the paragraph, then full lines.
    t += &right(plain, size, 554.0, y - 5.0 * k, "Scan to pay");
    t += &show(
        plain,
        size,
        32.0,
        y - 15.0 * k,
        "The account for the sum stands on the next sheet.",
    );
    y -= 15.0 * k + 74.0;
    let close = wrapped(
        "Please look at every part when it arrives and write to us within ten days when something is missing or broken, so that we can send a new part with the next van that leaves our yard for your town and the towns around it.",
        plain,
        size,
        530.0,
    );
    for l in &close {
        t += &show(plain, size, 30.0, y, l);
        y -= step;
    }
    let mut s = sheet(v, t, y - 60.0, None);
    let head = [
        HEAD_BLOCK[0],
        HEAD_BLOCK[1],
        "Partners",
        "AB123456789",
        "CD12EFG34567890123",
    ];
    s.want.orders.push(head.map(String::from).to_vec());
    // The paragraph beside and under the box is one block, in its order.
    s.want.orders.push(lines.clone());
    s.want.together.push(lines.clone());
    s.want.paragraphs.push(close.join(" "));
    for l in &lines {
        for other in ["Quote in every reply", "Ref: 123456789", HEAD_BLOCK[0]] {
            s.want.apart.push((l.clone(), other.into()));
        }
    }
    s
}

/// The words of the two columns of [`full_width_under_two_columns`].
const COLUMN_WORDS: &str = "The yard opens at seven each morning and the first van leaves soon after with the parts that were packed the evening before, so an order that reaches us by noon is on its way the next day. A second van follows at ten and takes the larger frames, which two people must lift, to the towns along the river and back. Whoever waits for a part can ask the driver for the list of the day, where every box stands with its number, its weight and the name of the one who packed it, and can sign for it at the door when it comes. A box that nobody takes in goes back to the yard with the last van and waits there on the shelf by the gate for three more days before we write to ask what shall be done with it.";

/// Review 1 of Task 31c, M6: two columns of prose on the same baselines, the right one a
/// line shorter, and right under them, at their pitch, left edge and size, a paragraph of
/// the full width that ends in a short line. The text flows from the left column into the
/// right one; the paragraph follows both. Its full lines stand across the gutter between
/// the last line of the left column and its own short last line, which looks like a column
/// that flows past a block on its right (class F), but the lines over them stand beside the
/// right column: the paragraph is not read between the two columns.
fn full_width_under_two_columns(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 10.0 * k;
    let step = 12.0 * k;
    let lines = wrapped(COLUMN_WORDS, plain, size, 235.0);
    assert!(lines.len() >= 11, "{}", lines.len());
    let (left, right) = (&lines[..6], &lines[6..11]);
    let mut t = String::new();
    for (n, l) in left.iter().enumerate() {
        t += &show(plain, size, 40.0, 700.0 - step * n as f32, l);
    }
    for (n, l) in right.iter().enumerate() {
        t += &show(plain, size, 315.0, 700.0 - step * n as f32, l);
    }
    let mut full = wrapped(
        "Please look at every part when it arrives and write to us within ten days when something is missing or broken, so that we can send a new part with the next van that leaves our yard for your town and the towns around it, at no cost to you and with no further form to fill in for the driver or for the office of the yard",
        plain,
        size,
        510.0,
    );
    full.truncate(2);
    full.push("and close the gate behind you.".into());
    let mut y = 700.0 - step * 6.0;
    for l in &full {
        t += &show(plain, size, 40.0, y, l);
        y -= step;
    }
    let mut s = sheet(v, t, y - 60.0, None);
    // The columns in their order, the right one right after the left one; then the paragraph.
    let columns: Vec<String> = left.iter().chain(right).cloned().collect();
    s.want.orders.push(columns.clone());
    s.want.together.push(columns);
    s.want.orders.push(
        [right[4].clone()]
            .into_iter()
            .chain(full.iter().cloned())
            .collect(),
    );
    s.want.together.push(full);
    s
}

/// The shapes of business documents.
pub(crate) fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            name: "labels on touching underlines over two rows of fields",
            axes: DOC,
            draw: |v| (fields_on_touching_underlines(v), None),
        },
        Shape {
            name: "a small bold label over a rule with two lines",
            axes: DOC,
            draw: |v| (labelled_block(v), None),
        },
        Shape {
            name: "two boxes side by side, each with its label",
            axes: DOC,
            draw: |v| (labelled_boxes(v), None),
        },
        Shape {
            name: "two fields of a label over a rule beside a ruled table with empty rows",
            axes: DOC,
            draw: |v| (side_fields_beside_an_empty_table(v, false), None),
        },
        Shape {
            name: "two fields of a label over a rule beside a ruled table with empty rows and the rule of a third field",
            axes: DOC,
            draw: |v| (side_fields_beside_an_empty_table(v, true), None),
        },
        Shape {
            name: "a paragraph of full width under two columns of prose, the right one a line shorter",
            axes: DOC,
            draw: |v| (full_width_under_two_columns(v), None),
        },
        Shape {
            name: "two fields of a label over a rule left of a ruled table with empty rows and the rule of a third field",
            axes: DOC,
            draw: |v| (side_fields_beside_a_table(v, true, true, false, 2), None),
        },
        Shape {
            name: "two fields of a label over a rule beside a ruled table with filled rows and the rule of a third field",
            axes: DOC,
            draw: |v| (side_fields_beside_a_table(v, true, false, true, 2), None),
        },
        Shape {
            name: "an underlined label beside an empty ruled grid",
            axes: DOC,
            draw: |v| (an_underlined_label_beside_an_empty_grid(v), None),
        },
        Shape {
            name: "a framed label column and value column, rows 1.36 sizes apart beside a picture",
            axes: DOC,
            draw: |v| (framed_pairs(v, 15.0 / 11.0, true), None),
        },
        Shape {
            name: "a framed label column and value column, rows 1.55 sizes apart",
            axes: DOC,
            draw: |v| (framed_pairs(v, 17.0 / 11.0, false), None),
        },
        Shape {
            name: "a payment box whose width differs between pages",
            axes: TWO_PAGES,
            draw: |v| (a_payment_box_of_two_widths(v), None),
        },
        Shape {
            name: "a head block beside a logo over a letter",
            axes: DOC,
            draw: |v| (head_block_beside_a_logo(v, false), None),
        },
        Shape {
            name: "a letter paragraph that runs on under a box on its right",
            axes: DOC,
            draw: |v| (letter_past_a_box(v), None),
        },
        Shape {
            name: "a head block of two lines beside a logo over a letter",
            axes: DOC,
            draw: |v| (head_block_beside_a_logo(v, true), None),
        },
        Shape {
            name: "letter head",
            axes: DOC_NEXT,
            draw: |v| (letter_head(v, false, false), None),
        },
        Shape {
            name: "a heading over an address with its own name",
            axes: DOC,
            draw: |v| (heading_over_a_named_address(v), None),
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
                s.want.order = STARRED_LINES.iter().map(|l| l.to_string()).collect();
                (s, None)
            },
        },
        Shape {
            name: "plain lines of starred items",
            axes: DOC,
            draw: |v| {
                let mut s = column_of(v, &STARRED_LINES, 8.0, false, None);
                s.want.order = STARRED_LINES.iter().map(|l| l.to_string()).collect();
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
            name: "a value that touches its label",
            axes: DOC,
            draw: |v| (touching(v), None),
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
            name: "numbered labels beside an address",
            axes: DOC,
            draw: |v| (numbered_labels(v), None),
        },
        Shape {
            name: "prose right under a column of field lines",
            axes: DOC,
            draw: |v| (prose_by_fields(v, true), None),
        },
        Shape {
            name: "a paragraph without a stop set full in a narrow column",
            axes: DOC,
            draw: |v| (justified_paragraph(v), None),
        },
        Shape {
            name: "rows of a label with a colon and its value over a larger total row",
            axes: DOC,
            draw: |v| {
                let rows = [
                    ["Date:", "12 May"],
                    ["Order:", "po_123"],
                    ["Balance Due:", "127.50"],
                ];
                (rows_over_a_larger_row(v, &rows), None)
            },
        },
        Shape {
            name: "a subtotal over a larger total without colons",
            axes: DOC,
            draw: |v| {
                let rows = [["Subtotal", "120.00"], ["Total", "127.50"]];
                (rows_over_a_larger_row(v, &rows), None)
            },
        },
        Shape {
            name: "rows of a name and a word over a larger last row",
            axes: DOC,
            draw: |v| {
                let rows = [["Colour", "green"], ["Shape", "round"], ["Grade", "prime"]];
                (rows_over_a_larger_row(v, &rows), None)
            },
        },
        Shape {
            name: "lines of a table of contents with their pages at one edge",
            axes: DOC,
            draw: |v| (contents_lines(v), None),
        },
        Shape {
            name: "paragraphs with a short last line across a frame of two rules",
            axes: DOC,
            draw: |v| (paragraphs_across_a_frame(v), None),
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
            draw: |v| {
                let cells = [
                    ["Erika Mustermann", "Muster GmbH"],
                    ["Beispiel AG", "Probeweg 12"],
                ];
                (cells_or_rows(v, &cells), None)
            },
        },
        Shape {
            name: "labels beside a street and its town between rules with no divider",
            axes: RULED,
            draw: |v| {
                let cells = [["Strasse", "Ort"], ["Musterweg 1", "12345 Musterstadt"]];
                (cells_or_rows(v, &cells), None)
            },
        },
        Shape {
            name: "abbreviated labels beside a street and its town between rules with no divider",
            axes: RULED,
            draw: |v| {
                let cells = [
                    ["Str. u. Nr.", "PLZ u. Ort"],
                    ["Musterweg 1", "12345 Musterstadt"],
                ];
                (cells_or_rows(v, &cells), None)
            },
        },
        Shape {
            name: "two rows of items between rules with no divider",
            axes: RULED,
            draw: |v| {
                let cells = [
                    ["Pos 1", "Pos 2"],
                    ["Schrauben M4", "Muttern 8 mm"],
                    ["Lager A", "Lager B"],
                ];
                (cells_or_rows(v, &cells), None)
            },
        },
        Shape {
            name: "words in two weights",
            axes: DOC,
            draw: |v| (two_weights(v), None),
        },
        Shape {
            name: "a range that wraps after its hyphen in prose",
            axes: DOC,
            draw: |v| (wrapped_range(v), None),
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
        Shape {
            name: "a footer of three columns with a wrapped paragraph in the middle one",
            axes: DOC,
            draw: |v| (footer_with_a_wrapped_column(v), None),
        },
        Shape {
            name: "a table without rules at the foot whose last row wraps",
            axes: DOC,
            draw: |v| (foot_table_with_a_wrapped_row(v, &FOOT_TABLE, true), None),
        },
        Shape {
            name: "a table without rules at the foot with an empty last cell whose last row wraps",
            axes: DOC,
            draw: |v| {
                (
                    foot_table_with_a_wrapped_row(v, &FOOT_TABLE_EMPTY_CELL, false),
                    None,
                )
            },
        },
        Shape {
            name: "a table without rules at the foot with an empty last cell whose last row wraps twice",
            axes: DOC,
            draw: |v| {
                (
                    foot_table_with_a_wrapped_row(v, &FOOT_TABLE_TWO_LINES, false),
                    None,
                )
            },
        },
        Shape {
            name: "a table without rules whose item wraps on the last row",
            axes: DOC,
            draw: |v| (table_whose_item_wraps(v, 720.0), None),
        },
        Shape {
            name: "a table without rules at the foot whose item wraps on the last row",
            axes: DOC,
            draw: |v| (table_whose_item_wraps(v, 78.0), None),
        },
        Shape {
            name: "a page title under a kicker beside a form number: Sample Leave Request",
            axes: DOC,
            draw: |v| (title_under_a_kicker(v, "Sample Leave Request"), None),
        },
        Shape {
            name: "a page title under a kicker beside a form number: Annual Report",
            axes: DOC,
            draw: |v| (title_under_a_kicker(v, "Annual Report"), None),
        },
        Shape {
            name: "a page title under a kicker beside a form number: Request for Sample Leave",
            axes: DOC,
            draw: |v| (title_under_a_kicker(v, "Request for Sample Leave"), None),
        },
        Shape {
            name: "a page title under a kicker beside a form number: Request for Sample Leave 2026",
            axes: DOC,
            draw: |v| {
                (
                    title_under_a_kicker(v, "Request for Sample Leave 2026"),
                    None,
                )
            },
        },
        Shape {
            name: "a note column beside rows of fields",
            axes: TRACED,
            draw: |v| (note_beside_field_rows(v), None),
        },
        Shape {
            name: "an amount in its box before a note column",
            axes: TRACED,
            draw: |v| (amount_before_a_note_column(v), None),
        },
        Shape {
            name: "a row of labels over two blocks of fields",
            axes: TRACED,
            draw: |v| (label_row_over_two_blocks(v, false), None),
        },
        Shape {
            name: "a row of labels over two blocks of fields in drawn boxes",
            axes: TRACED,
            draw: |v| (label_row_over_two_blocks(v, true), None),
        },
    ]
}
