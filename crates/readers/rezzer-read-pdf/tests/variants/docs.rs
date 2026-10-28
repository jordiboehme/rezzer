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
        let (font, fs) = if total {
            (bold, last * size)
        } else {
            (plain, size)
        };
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
    t += &show(
        plain,
        0.8 * size,
        40.0,
        y(low + 6.0),
        "Zahlbar innerhalb von 14 Tagen ohne Abzug.",
    );
    t += &rule(v, 40.0, 540.0, y(low + 14.0), size);
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
        Second::Plain => [
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
        let all: Vec<&[&str]> = first.iter().chain(&second).map(|r| &r[..]).collect();
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
