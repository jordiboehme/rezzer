// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shapes of a real pay slip (measured by structure only; all text and values invented),
//! drawn in a geometry that can be shifted, scaled and varied: F1 to F3 (a strip of seven
//! fields with a rate over its divider, a row under a longer rule, an underlined title), rows
//! of fields in any direction and alignment, the header field grid (P6-H), the column of totals
//! beside a section (P6-O), boxed fields, and the block of labels with their values beside them
//! over the bank row. Every height is relative to the data size and every x to the data font's
//! character pitch.

use crate::common::{DataLine, FormPage, label};
use crate::draw::{Stroke, bar_h, bold, courier, line_h, mark_b, plain, rule_h, rule_v};

/// The font of the data layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DataFont {
    /// One line from the data margin in Courier, padded into the fields.
    Courier,
    /// The same in Courier-Bold.
    CourierBold,
    /// Each value placed at its field in Helvetica (a proportional font).
    Helvetica,
}

/// The geometry of a generated form: a shift, the size of the data font (which sets the
/// character pitch, the row height and every height relative to it), the label size, the
/// data font, the thickness of the thin rules and of the bar under a title, whether the bar
/// is a stroked line, whether drawn decimal marks are on the page, how the thin rules are
/// drawn, whether dividers rise from the lower rule, and the scale and turn of the page.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Geo {
    pub dx: f32,
    pub dy: f32,
    pub data: f32,
    pub label: f32,
    pub font: DataFont,
    pub rule: f32,
    pub bar: f32,
    pub stroked_bar: bool,
    pub marks: bool,
    /// Data columns added to the second field of the strip: every later field moves right.
    pub wide: usize,
    /// The strip's last two fields are one.
    pub fewer: bool,
    pub stroke: Stroke,
    /// Dividers rise from the rule under the row instead of hanging from the rule over it.
    pub rising: bool,
    /// The whole page drawn this many times as large.
    pub scale: f32,
    /// The whole page drawn a quarter turn across.
    pub turn: bool,
}

/// The geometry of the real form.
pub(crate) const REAL: Geo = Geo {
    dx: 0.0,
    dy: 0.0,
    data: 9.5,
    label: 6.6,
    font: DataFont::CourierBold,
    rule: 0.24,
    bar: 0.72,
    stroked_bar: false,
    marks: true,
    wide: 0,
    fewer: false,
    stroke: Stroke::Bar,
    rising: false,
    scale: 1.0,
    turn: false,
};

/// A field to expect: its label and the value it must pair with (`None`: empty), or `Err`
/// when the geometry leaves the field unclear (it must not pair, and a loss must name it).
pub(crate) type Expect = (&'static str, Result<Option<&'static str>, ()>);

/// A page being built: its template, its data lines, the values to find once each, the fields
/// to expect, the columns a section must hold (its title and its values from the top), and the
/// raised markers labels carry.
pub(crate) struct Built {
    pub page: FormPage,
    pub values: Vec<String>,
    pub fields: Vec<Expect>,
    pub columns: Vec<(&'static str, Vec<&'static str>)>,
    pub markers: Vec<&'static str>,
}

impl Built {
    /// An empty page in geometry `g`.
    pub(crate) fn new(g: &Geo) -> Built {
        Built {
            page: FormPage {
                left: 42.52 + g.dx,
                size: g.data,
                bold_data: g.font == DataFont::CourierBold,
                scale: g.scale,
                turn: g.turn,
                ..FormPage::default()
            },
            values: Vec::new(),
            fields: Vec::new(),
            columns: Vec::new(),
            markers: Vec::new(),
        }
    }

    /// Text in the data font elsewhere, so it is a text size of the page.
    pub(crate) fn address(&mut self, g: &Geo) {
        for (k, l) in ["Muster GmbH", "Musterweg 2", "54321 Musterdorf"]
            .iter()
            .enumerate()
        {
            g.values(self, 120.0 + g.dy - 1.2 * g.data * k as f32, &[(0, l)]);
        }
    }

    /// A value to find once, and its field.
    fn expect(&mut self, label: &'static str, value: &'static str) {
        if value.is_empty() {
            self.fields.push((label, Ok(None)));
        } else {
            self.values.push(value.to_string());
            self.fields.push((label, Ok(Some(value))));
        }
    }
}

impl Geo {
    /// The pitch of a character of the data font.
    pub(crate) fn pitch(&self) -> f32 {
        0.6 * self.data
    }

    /// The x of data column `k`.
    pub(crate) fn col(&self, k: usize) -> f32 {
        42.52 + self.dx + k as f32 * self.pitch()
    }

    /// Data column `k` of the base layout in this one: columns from the third field of the
    /// strip on move right by `wide`.
    pub(crate) fn k(&self, k: usize) -> usize {
        if k >= 20 { k + self.wide } else { k }
    }

    /// The advance of a digit of the data font.
    pub(crate) fn digit(&self) -> f32 {
        match self.font {
            DataFont::Helvetica => 0.556 * self.data,
            _ => self.pitch(),
        }
    }

    /// A thin horizontal rule, its top at `y`.
    pub(crate) fn hrule(&self, x0: f32, x1: f32, y: f32) -> String {
        rule_h(self.stroke, x0, x1, y, self.rule, self.data)
    }

    /// A thin vertical rule.
    pub(crate) fn vrule(&self, x: f32, y0: f32, y1: f32) -> String {
        rule_v(self.stroke, x, y0, y1, self.rule, self.data)
    }

    /// A divider at `x` of the row between the rules `top` and `bottom`: hanging from `top`
    /// over half the row, or rising from `bottom` as far.
    pub(crate) fn divider(&self, x: f32, top: f32, bottom: f32) -> String {
        let h = 1.21 * self.data;
        if self.rising {
            self.vrule(x, bottom, bottom + h)
        } else {
            self.vrule(x, top - h, top)
        }
    }

    /// Writes `parts` (column, text) as the data line at baseline `y`: one padded line in
    /// Courier, or each part at its column in Helvetica.
    pub(crate) fn values(&self, b: &mut Built, y: f32, parts: &[(usize, &str)]) {
        match self.font {
            DataFont::Helvetica => {
                for (k, t) in parts {
                    b.page.template += &plain(self.col(*k), y, self.data, t);
                }
            }
            _ => {
                let mut text = String::new();
                for (k, t) in parts {
                    let len = text.chars().count();
                    text += &" ".repeat(k.saturating_sub(len));
                    text += t;
                }
                b.page.lines.push(DataLine { y, text });
            }
        }
    }
}

/// About the width of a template label (see [`label`]): Helvetica at 90 %, half a size a
/// character on the average.
fn label_width(text: &str, size: f32) -> f32 {
    0.9 * 0.55 * size * text.chars().count() as f32
}

/// F1: a strip of seven fields (`left` to `right` in data columns) between two rules, all
/// labels on one baseline, all values on one data line, dividers over half of the row. The
/// rate field holds `rate` (its first two digits and the next ones parted by a drawn decimal
/// mark when `marks`), which fills it and runs on into the next field when `overflow` (no
/// space before the next value), as on the real form; the last two labels are one run when
/// `one_run`. Returns the rule's height at the bottom.
pub(crate) fn strip_f1(g: &Geo, b: &mut Built, top: f32, overflow: bool, one_run: bool) -> f32 {
    let h = 2.4 * g.data;
    let bottom = top - h;
    b.page.template += &g.hrule(g.col(0) - 0.12, g.col(g.k(61)), top);
    b.page.template += &g.hrule(g.col(0) - 0.12, g.col(g.k(61)), bottom);
    // Field starts in data columns.
    let mut starts = vec![0usize, 13, 43, 47, 51, 56, 58];
    if g.fewer {
        starts.pop();
    }
    for &k in &starts[1..] {
        b.page.template += &g.divider(g.col(g.k(k)) - 0.06, top, bottom);
    }
    let base = top - 0.86 * g.data;
    let lab = |k: usize| g.col(k) + 0.18 * g.label;
    for (k, l) in [
        (0, "Feld A"),
        (13, "Feld B"),
        (g.k(43), "Satz %"),
        (g.k(47), "Gr C"),
        (g.k(51), "Gr D"),
    ] {
        b.page.template += &label(lab(k), base, g.label, l);
    }
    // The raised footnote marker of the rate's label, right after it and inside its field, as
    // a template sets it.
    let marker_w = 0.556 * 0.71 * g.label;
    b.page.template += &plain(
        (lab(g.k(43)) + label_width("Satz %", g.label)).min(g.col(g.k(47)) - 0.3 - marker_w),
        base + 0.29 * g.label,
        0.71 * g.label,
        "8",
    );
    b.markers.push("8");
    if g.fewer {
        b.page.template += &label(lab(g.k(56)), base, g.label, "Tage");
    } else if one_run {
        b.page.template += &label(lab(g.k(56)), base, g.label, "Um. Tage");
    } else {
        b.page.template += &label(lab(g.k(56)), base, g.label, "Um.");
        b.page.template += &label(lab(g.k(58)), base, g.label, "Tage");
    }
    let y = top - 2.23 * g.data;
    // The rate: four digits in its field, then three of the next field, touching them when
    // the line overflows.
    let next = g.k(if overflow { 47 } else { 48 });
    // Without overflow the group is a digit shorter, so a space parts it from the next value.
    let (rate, group) = ("1234", if overflow { "567" } else { "56" });
    let mut parts = vec![(0, "12345678X012"), (13, "Muster Kasse"), (g.k(43), rate)];
    parts.push((next, group));
    if g.fewer {
        parts.extend([(g.k(51), "8901"), (g.k(56), "30")]);
    } else {
        parts.extend([(g.k(51), "8901"), (g.k(56), "7"), (g.k(58), "30")]);
    }
    g.values(b, y, &parts);
    let rate_text = if g.marks {
        let x = g.col(g.k(43)) + 2.0 * g.digit();
        b.page.template += &mark_b(x, y - 0.087 * g.data, 0.2 * g.data);
        "12,34"
    } else {
        "1234"
    };
    // In Helvetica the rate runs over its field even without overflow: four digits are
    // wider than four columns of the pitch only when the font is wider, so it fits.
    let clear = !overflow || g.marks;
    b.values.extend(
        ["12345678X012", "Muster Kasse", "8901", "30"]
            .iter()
            .map(|s| s.to_string()),
    );
    if !g.fewer {
        b.values.push("7".into());
        b.fields.push(("Um.", Ok(Some("7"))));
    }
    if clear {
        b.values.push(rate_text.into());
        b.values.push(group.into());
    }
    b.fields.extend([
        ("Feld A", Ok(Some("12345678X012"))),
        ("Feld B", Ok(Some("Muster Kasse"))),
        ("Gr D", Ok(Some("8901"))),
        ("Tage", Ok(Some("30"))),
    ]);
    if clear {
        b.fields
            .push(("Satz %", Ok(Some(if g.marks { "12,34" } else { "1234" }))));
        b.fields
            .push(("Gr C", Ok(Some(if overflow { "567" } else { "56" }))));
    } else {
        b.fields.push(("Satz %", Err(())));
        b.fields.push(("Gr C", Err(())));
    }
    bottom
}

/// F2: a two-field row under the long lower rule of the strip above, with a short lower rule
/// of its own that covers only its stretch (data columns 47 to 61), a divider from the long
/// rule; a date in the first field (printed without its points and drawn as two marks when
/// `marks`) and the second field empty with two marks only. A row of the same stretch under
/// it. Returns the bottom of that row.
pub(crate) fn row_f2(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let h = 2.4 * g.data;
    let (x0, x1) = (g.col(g.k(47)) - 0.18, g.col(g.k(61)));
    let mut bottom = top;
    for (n, (labels, split)) in [
        (["Eintritt", "Austritt"], 54usize),
        (["Steuer-ID", "Kz"], 59),
    ]
    .into_iter()
    .enumerate()
    {
        let row_top = top - h * n as f32;
        bottom = row_top - h;
        b.page.template += &g.hrule(x0, x1, bottom);
        let split = g.k(split);
        b.page.template += &g.divider(g.col(split) - 0.18, row_top, bottom);
        let base = row_top - 0.86 * g.data;
        b.page.template += &label(x0 + 0.18 * g.label, base, g.label, labels[0]);
        b.page.template += &label(g.col(split) + 0.18 * g.label, base, g.label, labels[1]);
        let y = row_top - 2.23 * g.data;
        if n == 0 {
            if g.marks {
                g.values(b, y, &[(g.k(47), "010120")]);
                for k in [2.0, 4.0] {
                    let x = g.col(g.k(47)) + k * g.digit();
                    b.page.template += &mark_b(x, y - 0.087 * g.data, 0.2 * g.data);
                }
                // The empty date's marks.
                for k in [56, 58] {
                    b.page.template += &mark_b(g.col(g.k(k)), y - 0.087 * g.data, 0.2 * g.data);
                }
            } else {
                // Printed with its points, the date is as short as the field.
                g.values(b, y, &[(g.k(47), "1.1.20")]);
            }
            let date = if g.marks { "01.01.20" } else { "1.1.20" };
            b.values.push(date.into());
            b.fields.push(("Eintritt", Ok(Some(date))));
            b.fields.push(("Austritt", Ok(None)));
        } else {
            g.values(b, y, &[(g.k(47), "12345678901")]);
            b.values.push("12345678901".into());
            b.fields.push(("Steuer-ID", Ok(Some("12345678901"))));
            b.fields.push(("Kz", Ok(None)));
        }
    }
    bottom
}

/// F3: a bold title under the thin rule `above`, a bar under it a little under its baseline
/// and longer than it, no verticals, two lines of the data font under the bar and no rule
/// under them; left of it an address in bold, and a full-width ruled grid below with its own
/// title, so the reading order has a table after the titled text.
pub(crate) fn titled_f3(g: &Geo, b: &mut Built, above: f32) {
    let x0 = g.col(g.k(47)) - 0.18;
    let title_y = above - 2.11 * g.data;
    let title_size = g.label * 1.01;
    b.page.template += &bold(x0, title_y, title_size, "Hinweise Muster");
    let bar_top = title_y - 0.4 * title_size;
    let bar_end = g.col(g.k(87));
    b.page.template += &if g.stroked_bar {
        line_h(x0, bar_end, bar_top, g.bar)
    } else {
        bar_h(x0, bar_end, bar_top, g.bar)
    };
    let first = bar_top - 1.08 * g.data;
    let lines = [
        "Muster vom 01.02.21 Kst.           4321",
        "- Musterhinweis 2 Muster < 99 J.: 9.99%",
    ];
    for (k, l) in lines.iter().enumerate() {
        g.values(b, first - 1.2 * g.data * k as f32, &[(g.k(47), l)]);
    }
    // The address on the left, in bold, and the grid under all of it.
    let addr = first - 3.0 * g.data;
    for (k, l) in ["Erika Mustermann", "Musterweg 1", "12345 Musterstadt"]
        .iter()
        .enumerate()
    {
        b.page.template += &bold(
            59.67 + g.dx,
            addr - 1.2 * g.data * k as f32,
            1.2 * g.data,
            l,
        );
    }
    let grid_top = addr - 8.0 * g.data;
    let (gx0, gmid, gx1) = (42.40 + g.dx, 287.0 + g.dx, 458.80 + g.dx);
    b.page.template += &bold(
        gx0,
        grid_top + 0.33 * g.data,
        0.7 * g.data,
        "Muster-Bezuege",
    );
    for y in [grid_top, grid_top - 1.4 * g.data, grid_top - 3.0 * g.data] {
        b.page.template += &g.hrule(gx0, gx1, y);
    }
    for x in [gx0, gmid, gx1] {
        b.page.template += &g.vrule(x, grid_top - 3.0 * g.data, grid_top);
    }
    let head = grid_top - 0.88 * g.data;
    b.page.template += &plain(gx0 + 2.0, head, g.label, "Art");
    b.page.template += &plain(gmid + 2.0, head, g.label, "Betrag");
    let row = grid_top - 2.4 * g.data;
    b.page.template += &plain(gx0 + 2.0, row, g.data, "Gehalt");
    b.page.template += &plain(gmid + 2.0, row, g.data, "1.234,56");
}

/// The page of the three shapes F1 to F3 in geometry `g`: F1 at the top (its rate
/// overflowing when `overflow`), F2 under it, F3 under that.
pub(crate) fn page(g: &Geo, overflow: bool, one_run: bool) -> Built {
    let mut b = Built::new(g);
    let top = 760.0 + g.dy;
    let below_f1 = strip_f1(g, &mut b, top, overflow, one_run);
    let below_f2 = row_f2(g, &mut b, below_f1);
    titled_f3(g, &mut b, below_f2);
    b.address(g);
    b
}

/// Where a text stands in its field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
}

/// A block of rows of fields between drawn rules, set in Courier so that every width is
/// known: labels of the geometry's label size, values of its data size.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rows {
    /// The labels stand under the rule and the values on it (a signature line), not the
    /// labels at the top of the space between two rules and the values at its bottom.
    pub under: bool,
    pub labels: Align,
    pub values: Align,
    /// Dividers part the fields over the labels; without them a vertical rule stands far
    /// from the fields, so the page has rules of both directions.
    pub dividers: bool,
}

pub(crate) const ROWS: Rows = Rows {
    under: false,
    labels: Align::Left,
    values: Align::Left,
    dividers: true,
};

/// A field of [`rows_page`]: its first data column, its width in columns, its label and its
/// value ("" for none).
pub(crate) type FieldSpec = (usize, usize, &'static str, &'static str);

/// Three rows of two fields each.
pub(crate) const THREE_ROWS: [&[FieldSpec]; 3] = [
    &[
        (0, 30, "Name", "Erika Mustermann"),
        (30, 20, "Ort", "Musterstadt"),
    ],
    &[(0, 30, "Strasse", "Musterweg 1"), (30, 20, "Hausnr", "12")],
    &[(0, 30, "PLZ", "12345"), (30, 20, "Land", "DE")],
];

/// A page of rows of fields in the shape `s` and geometry `g`, each row a list of fields,
/// with what to expect of each field: its value, or `Err` where `unclear` names it.
pub(crate) fn rows_page(g: &Geo, s: &Rows, rows: &[&[FieldSpec]], unclear: &[&str]) -> Built {
    let mut b = Built::new(g);
    let pitch = g.pitch();
    let col = |k: usize| g.col(k);
    let h = 2.4 * g.data;
    let end = rows
        .iter()
        .flat_map(|r| r.iter().map(|f| f.0 + f.1))
        .max()
        .unwrap_or(0);
    let (x0, x1) = (col(0) - 0.12, col(end));
    let top = 760.0 + g.dy;
    // The rules: one over each row and one under the last; with labels under, one per row.
    let n = if s.under { rows.len() } else { rows.len() + 1 };
    for k in 0..n {
        b.page.template += &g.hrule(x0, x1, top - h * k as f32);
    }
    if !s.dividers {
        b.page.template += &g.vrule(30.0 + g.dx, 200.0 + g.dy, 300.0 + g.dy);
    }
    let at = |align: Align, f: &FieldSpec, w: f32, pad: f32| match align {
        Align::Left => col(f.0) + pad,
        Align::Center => col(f.0) + (f.1 as f32 * pitch - w) / 2.0,
        Align::Right => col(f.0 + f.1) - w - pad,
    };
    for (k, row) in rows.iter().enumerate() {
        let rule = top - h * k as f32;
        let (label_y, value_y) = if s.under {
            (rule - 0.86 * g.data, rule + 0.17 * g.data)
        } else {
            (rule - 0.86 * g.data, rule - 2.23 * g.data)
        };
        for (n, f) in row.iter().enumerate() {
            if n > 0 && s.dividers {
                // With labels under, the divider parts the labels under the rule, or rises
                // over the rule between the values.
                b.page.template += &if s.under {
                    let x = col(f.0) - 0.06;
                    let span = 1.21 * g.data;
                    if g.rising {
                        g.vrule(x, rule, rule + span)
                    } else {
                        g.vrule(x, rule - span, rule)
                    }
                } else {
                    g.divider(col(f.0) - 0.06, rule, rule - h)
                };
            }
            let lw = 0.6 * g.label * f.2.chars().count() as f32;
            let lx = at(s.labels, f, lw, 0.18 * g.label);
            b.page.template += &courier(lx, label_y, g.label, f.2);
            if !f.3.is_empty() {
                let vw = pitch * f.3.chars().count() as f32;
                let vx = at(s.values, f, vw, 0.3 * pitch);
                b.page.template += &courier(vx, value_y, g.data, f.3);
                b.values.push(f.3.to_string());
            }
            let want = if unclear.contains(&f.2) {
                Err(())
            } else if f.3.is_empty() {
                Ok(None)
            } else {
                Ok(Some(f.3))
            };
            b.fields.push((f.2, want));
        }
    }
    for (k, l) in ["Muster GmbH", "Musterweg 2", "54321 Musterdorf"]
        .iter()
        .enumerate()
    {
        b.page.template += &courier(col(0), 120.0 + g.dy - 1.2 * g.data * k as f32, g.data, l);
    }
    b
}

/// The rows of the header field grid: each field's first data column, its label and its
/// value; the dividers stand at different places in each row, as on the real form.
const HEADER: [&[(usize, &str, &str)]; 3] = [
    &[
        (0, "Pers.-Nr.", "00123"),
        (8, "Eintritt", "01.01.80"),
        (18, "Stkl", "3"),
        (24, "Konfession", ""),
    ],
    &[
        (0, "Geburtsdatum", "01.01.70"),
        (13, "Austritt", ""),
        (23, "Kinder", "0,5"),
    ],
    &[
        (0, "Kostenst.", "4711"),
        (10, "Krankenkasse", "Muster BKK"),
        (30, "Abteilung", "Vertrieb"),
    ],
];

/// P6-H: the header field grid, three rows of label rows over value rows between rules over
/// the left part (data columns 0 to 40), a raised marker after one label, and a right part
/// (columns 44 to 60) on the same rules with a date in its first row only.
pub(crate) fn header_grid(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let h = 2.4 * g.data;
    let (left, right) = ((g.col(0) - 0.12, g.col(40)), (g.col(44) - 0.12, g.col(60)));
    for k in 0..=HEADER.len() {
        let y = top - h * k as f32;
        b.page.template += &g.hrule(left.0, left.1, y);
        b.page.template += &g.hrule(right.0, right.1, y);
    }
    let lab = |k: usize| g.col(k) + 0.18 * g.label;
    for (n, row) in HEADER.iter().enumerate() {
        let rule = top - h * n as f32;
        let base = rule - 0.86 * g.data;
        let mut parts = Vec::new();
        for (i, &(k, l, v)) in row.iter().enumerate() {
            if i > 0 {
                b.page.template += &g.divider(g.col(k) - 0.06, rule, rule - h);
            }
            b.page.template += &label(lab(k), base, g.label, l);
            if l == "Stkl" {
                b.page.template += &plain(
                    lab(k) + label_width(l, g.label),
                    base + 0.29 * g.label,
                    0.71 * g.label,
                    "1",
                );
            }
            if !v.is_empty() {
                parts.push((k, v));
            }
            b.expect(l, v);
        }
        if n == 0 {
            b.page.template += &label(lab(44), base, g.label, "Datum");
            parts.push((44, "01.10.26"));
            b.expect("Datum", "01.10.26");
        }
        g.values(b, rule - 2.23 * g.data, &parts);
    }
    b.markers.push("1");
    top - h * HEADER.len() as f32
}

/// P6-O: a ruled section of two columns (data columns 0 to 20 and 20 to 46) with a header row
/// and three value lines, and right of it, a gutter of about half a character away, a column
/// drawn with horizontal rules only at the section's top, two lines over its bottom and at its
/// bottom: its upper strip heads its amounts on the baseline of the section's column titles
/// (one more column of the section), its lower strip holds a total label over its amount (a
/// pair).
pub(crate) fn total_column(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let line = 1.2 * g.data;
    let head = 1.4 * g.data;
    let bottom = top - head - 5.0 * line;
    let (x0, mid, x1) = (g.col(0) - 0.12, g.col(20), g.col(46));
    for y in [top, top - head, bottom] {
        b.page.template += &g.hrule(x0, x1, y);
    }
    for x in [x0, mid, x1] {
        b.page.template += &g.vrule(x, bottom, top);
    }
    let base = top - 0.88 * g.data;
    b.page.template += &plain(x0 + 2.0, base, g.label, "Art");
    b.page.template += &plain(mid + 2.0, base, g.label, "Schluessel");
    // The column beside it.
    let (c0, c1) = (x1 + 0.58 * g.data, x1 + 0.58 * g.data + 12.0 * g.pitch());
    let split = bottom + 2.0 * line + 0.4 * g.data;
    for y in [top, split, bottom] {
        b.page.template += &g.hrule(c0, c1, y);
    }
    b.page.template += &bold(c0 + 2.0, base, 0.7 * g.data, "Summe");
    let k0 = ((c0 + 2.0 - g.col(0)) / g.pitch()).ceil() as usize;
    let rows = [
        ("Gehalt", "X Y", "1.234,56"),
        ("Zulage", "Y Z", "12,34"),
        ("Bonus", "Z X", "56,78"),
    ];
    for (n, (art, key, amount)) in rows.iter().enumerate() {
        let y = top - head - line * (n as f32 + 1.0) + 0.3 * g.data;
        g.values(b, y, &[(0, art), (21, key), (k0, amount)]);
        b.values
            .extend([art.to_string(), key.to_string(), amount.to_string()]);
    }
    b.columns
        .push(("Summe", vec!["1.234,56", "12,34", "56,78"]));
    b.page.template += &bold(c0 + 2.0, split - 0.9 * g.data, 0.7 * g.data, "Gesamt");
    g.values(b, bottom + 0.15 * g.data, &[(k0, "1.303,68")]);
    b.expect("Gesamt", "1.303,68");
    bottom
}

/// Boxed fields: three boxes in a row, touching, each with a label at its top and a value on
/// the data line at its bottom; the last one empty.
pub(crate) fn boxed_row(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let h = 2.4 * g.data;
    let mut parts = Vec::new();
    for (k, l, v) in [
        (0usize, "Personalnummer", "00000"),
        (20, "Eintritt", "01.01.2020"),
        (40, "Steuerklasse", ""),
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
        if !v.is_empty() {
            parts.push((k + 1, v));
        }
        b.expect(l, v);
    }
    g.values(b, top - h + 0.3 * g.data, &parts);
    top - h
}

/// The block of labels with their values beside them (two columns of four and three lines
/// parted by a divider over the whole block), and under its lower rule the bank row: two
/// lines of a label and its value beside it, down to a rule of its own.
pub(crate) fn label_block(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let line = 1.2 * g.data;
    let bottom = top - 4.0 * line - 0.6 * g.data;
    let (x0, mid, x1) = (g.col(0) - 0.12, g.col(24) - 0.12, g.col(48));
    b.page.template += &g.hrule(x0, x1, top);
    b.page.template += &g.hrule(x0, x1, bottom);
    b.page.template += &g.vrule(mid, bottom, top);
    let left = [
        ("Steuer", "123,45"),
        ("Kirche", "21,43"),
        ("Soli", "1,23"),
        ("Rente", "234,56"),
    ];
    let right = [("Kranken", "98,76"), ("Pflege", "8,76"), ("Arbeitslos", "")];
    for k in 0..4 {
        let y = top - 0.86 * g.data - line * k as f32;
        let mut parts = Vec::new();
        if let Some(&(l, v)) = left.get(k) {
            b.page.template += &label(g.col(0) + 0.18 * g.label, y, g.label, l);
            parts.push((14, v));
            b.expect(l, v);
        }
        if let Some(&(l, v)) = right.get(k) {
            b.page.template += &label(g.col(24) + 0.18 * g.label, y, g.label, l);
            if !v.is_empty() {
                parts.push((40, v));
            }
            b.expect(l, v);
        }
        g.values(b, y, &parts);
    }
    // The bank row under the block's lower rule.
    let bank = bottom - 2.4 * g.data;
    b.page.template += &g.hrule(x0, x1, bank);
    for (k, (l, v)) in [
        ("Bank", "Musterbank"),
        ("IBAN", "DE00 0000 0000 0000 0000 00"),
    ]
    .iter()
    .enumerate()
    {
        let y = bottom - 0.86 * g.data - line * k as f32;
        b.page.template += &label(g.col(0) + 0.18 * g.label, y, g.label, l);
        g.values(b, y, &[(10, v)]);
        b.expect(l, v);
    }
    bank
}

/// Rows of a narrow name field (six data columns) and a number field, a divider between them:
/// in the first row a name of two words runs on into the empty number field, its word gap at
/// the divider; in the other rows the number field holds numbers. The name is one value, and
/// the number field of the first row is empty.
pub(crate) fn overflow_rows(g: &Geo, b: &mut Built, top: f32) -> f32 {
    field_pair_rows(
        g,
        b,
        top,
        6,
        &[
            (
                "Name",
                "Erika Mustermann",
                "Nr",
                "",
                Some("Erika Mustermann"),
            ),
            ("Ruf", "Max", "Haus", "12", None),
            ("Kurz", "Udo", "Zi", "34", None),
        ],
    )
}

/// The same rows with a name of one word that fills its field (seven columns) but one, and a
/// short value of letters at the next field's start, one space after it: the next field's own
/// value, though the other rows hold numbers there. It pairs with its own field.
pub(crate) fn short_value_rows(g: &Geo, b: &mut Built, top: f32) -> f32 {
    field_pair_rows(
        g,
        b,
        top,
        7,
        &[
            ("Name", "Peters", "Kz", "V", None),
            ("Ruf", "Max", "Haus", "12", None),
            ("Kurz", "Udo", "Zi", "34", None),
        ],
    )
}

/// Rows of two fields parted by a divider at data column `split`: each row its two labels,
/// their values, and the whole first value when the data line writes it over both fields.
fn field_pair_rows(
    g: &Geo,
    b: &mut Built,
    top: f32,
    split: usize,
    rows: &[(
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
    )],
) -> f32 {
    let h = 2.4 * g.data;
    for k in 0..=rows.len() {
        b.page.template += &g.hrule(g.col(0) - 0.12, g.col(30), top - h * k as f32);
    }
    let lab = |k: usize| g.col(k) + 0.18 * g.label;
    for (n, (l1, v1, l2, v2, _)) in rows.iter().enumerate() {
        let rule = top - h * n as f32;
        b.page.template += &g.divider(g.col(split) - 0.06, rule, rule - h);
        let base = rule - 0.86 * g.data;
        b.page.template += &label(lab(0), base, g.label, l1);
        b.page.template += &label(lab(split), base, g.label, l2);
        let mut parts = vec![(0, *v1)];
        if !v2.is_empty() {
            parts.push((split, *v2));
        }
        g.values(b, rule - 2.23 * g.data, &parts);
        b.expect(l1, v1);
        b.expect(l2, v2);
    }
    top - h * rows.len() as f32
}

/// The rows of [`beside_grid`]: a label and its value, and a second field of each.
const BESIDE: [(&str, &str, &str, &str); 4] = [
    ("Zeitraum", "10.2026", "Blatt", "9"),
    ("Steuer-Nr", "00000000000", "Kz", "A"),
    ("SV-Nr", "00000000X000", "Gruppe", "1111"),
    ("Bank", "Musterbank", "Konto", "4321"),
];

/// A grid of fields beside the header grid on its top rule (data columns 62 to 96), one row of
/// [`BESIDE`] for each of `heights` (in data sizes), each a label at the top of its row over a
/// value on its lower rule, with the second field from column 82 behind a divider when `two`:
/// rows of another pitch than the grid beside it, so the rules of the two grids interleave.
/// Returns its bottom.
pub(crate) fn beside_grid(g: &Geo, b: &mut Built, top: f32, heights: &[f32], two: bool) -> f32 {
    let (x0, x1) = (g.col(62) - 0.12, g.col(if two { 96 } else { 82 }));
    let lab = |k: usize| g.col(k) + 0.18 * g.label;
    let mut rule = top;
    b.page.template += &g.hrule(x0, x1, rule);
    for (&(l, v, l2, v2), &h) in BESIDE.iter().zip(heights) {
        let h = h * g.data;
        b.page.template += &label(lab(62), rule - 0.86 * g.data, g.label, l);
        let mut parts = vec![(62, v)];
        b.expect(l, v);
        if two {
            b.page.template += &label(lab(82), rule - 0.86 * g.data, g.label, l2);
            b.page.template += &g.divider(g.col(82) - 0.06, rule, rule - h);
            parts.push((82, v2));
            b.expect(l2, v2);
        }
        g.values(b, rule - h + 0.17 * g.data, &parts);
        rule -= h;
        b.page.template += &g.hrule(x0, x1, rule);
    }
    rule
}

/// The header grid with a grid beside it on the same top rule whose rows are of another
/// pitch, of one field or of two fields a row.
pub(crate) fn grids_side_by_side(g: &Geo, two: bool) -> Built {
    let mut b = Built::new(g);
    let top = 760.0 + g.dy;
    header_grid(g, &mut b, top);
    beside_grid(g, &mut b, top, &[3.2, 2.0, 3.2, 2.0], two);
    b.address(g);
    b
}

/// One field alone between two rules: a label over its value, nothing beside it or under it;
/// a vertical rule far from it, so the page has rules of both directions.
pub(crate) fn lone_field(g: &Geo, b: &mut Built, top: f32) -> f32 {
    let h = 2.4 * g.data;
    b.page.template += &g.vrule(30.0 + g.dx, 200.0 + g.dy, 300.0 + g.dy);
    b.page.template += &g.hrule(g.col(0) - 0.12, g.col(20), top);
    b.page.template += &g.hrule(g.col(0) - 0.12, g.col(20), top - h);
    b.page.template += &label(
        g.col(0) + 0.18 * g.label,
        top - 0.86 * g.data,
        g.label,
        "Kundennr",
    );
    g.values(b, top - 2.23 * g.data, &[(0, "98765")]);
    b.values.push("98765".into());
    b.fields.push(("Kundennr", Err(())));
    top - h
}

/// What stands over the top rule of [`title_grid`] (the shape file's variants a to d): the
/// document title's baseline over the rule (in data sizes), whether the period line, the date
/// and the page number stand there, the label `Blatt:`'s baseline over it, and whether the
/// code line stands over all of them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TitleBlock {
    pub title: Option<f32>,
    pub period: bool,
    pub date: bool,
    pub page_no: bool,
    pub blatt: Option<f32>,
    pub code: bool,
}

/// Variant a: everything as on the page.
pub(crate) const BLOCK_A: TitleBlock = TitleBlock {
    title: Some(0.33),
    period: true,
    date: true,
    page_no: true,
    blatt: Some(0.30),
    code: true,
};

/// A field of [`title_grid`]: its first data column, its label and its value ("" for none).
type GridField = (usize, &'static str, &'static str);

/// The rows of [`title_grid`], each its fields left of column 61 and right of column 63; the
/// dividers stand at the starts of the fields after the first of each part.
const TITLE_GRID: [(&[GridField], &[GridField]); 4] = [
    (
        &[
            (0, "Pers.-Nr.", "00000"),
            (6, "Eintritt", "1.1.80"),
            (13, "Kl", "3"),
            (15, "Konf.", "rk"),
            (20, "Ki", "0,5"),
            (24, "Tage", "30"),
            (30, "Kasse", "BKK7"),
            (39, "Abt.", "Verkauf"),
            (47, "Kst", "471"),
            (51, "Datum", "1.2.26"),
            (58, "Kz", "B"),
        ],
        &[
            (63, "Nr", "123"),
            (69, "Lfd", "001"),
            (75, "Seite", ""),
            (81, "Ref", "77"),
        ],
    ),
    (
        &[
            (0, "Steuer-ID", "12345678A12"),
            (13, "IBAN", "DE00 0000 0000 0000 0000 00"),
            (43, "Gr", "12"),
            (47, "Bz", "5"),
            (51, "Std", "160"),
            (56, "K", ""),
            (58, "St", "7"),
        ],
        &[(63, "Kranken", "Muster BKK"), (75, "Mitglied", "99999")],
    ),
    (
        &[(47, "Stunden", "160,00"), (54, "Satz", "12,34")],
        &[(63, "Brutto", "1.234,56"), (75, "Netto", "2.345,67")],
    ),
    (
        &[(47, "Steuer", "123,45")],
        &[(63, "Auszahlung", "9.876,54")],
    ),
];

/// The labels of [`title_grid`].
pub(crate) fn title_grid_labels() -> Vec<&'static str> {
    TITLE_GRID
        .iter()
        .flat_map(|(l, r)| l.iter().chain(r.iter()))
        .map(|f| f.1)
        .collect()
}

/// The shape file's header grid with a title block over its top rule: rows of labels over
/// values between rules, a left part (data columns 0 to 61) that ends after the second row, a
/// right part (63 to 87) for all four rows, the lower two rows of the left part only from
/// column 47 on, dividers at other places in each row; over its top rule what `over` says (a
/// bold proportional title of the data size, the period line, the date and the page number in
/// the data font at the values' depth, the label `Blatt:`, a code line higher up), and under
/// its bottom rule, left of the lower rows, two lines of an address window in bold. Every text
/// over the rule is to be found once; the grid's fields all pair.
pub(crate) fn title_grid(g: &Geo, b: &mut Built, top: f32, over: TitleBlock) -> f32 {
    let h = 2.4 * g.data;
    let d = g.data;
    let lab = |k: usize| g.col(k) + 0.18 * g.label;
    for k in 0..=TITLE_GRID.len() {
        let y = top - h * k as f32;
        let left = if k <= 2 { 0 } else { 47 };
        let x0 = if left == 0 {
            g.col(0) - 0.12
        } else {
            g.col(47) - 0.12
        };
        b.page.template += &g.hrule(x0, g.col(61), y);
        b.page.template += &g.hrule(g.col(63) - 0.12, g.col(87), y);
    }
    for (n, (left, right)) in TITLE_GRID.iter().enumerate() {
        let rule = top - h * n as f32;
        let mut parts = Vec::new();
        for part in [left, right] {
            for (i, &(k, l, v)) in part.iter().enumerate() {
                if i > 0 {
                    b.page.template += &g.divider(g.col(k) - 0.06, rule, rule - h);
                }
                b.page.template += &label(lab(k), rule - 0.86 * d, g.label, l);
                if !v.is_empty() {
                    parts.push((k, v));
                }
                b.expect(l, v);
            }
        }
        g.values(b, rule - 2.23 * d, &parts);
    }
    let find = |b: &mut Built, t: &str| b.values.push(t.to_string());
    if let Some(at) = over.title {
        let t = "Abrechnung der Muster/Muster-Bezuege";
        b.page.template += &bold(g.col(0), top + at * d, 1.003 * d, t);
        find(b, t);
    }
    let mut line = Vec::new();
    for (on, k, t) in [
        (over.period, 30, "fuer Muster 2026"),
        (over.date, 69, "12.34.56"),
        (over.page_no, 85, "1"),
    ] {
        if on {
            line.push((k, t));
            // The page number is the number of the page's own marker too.
            if t != "1" {
                find(b, t);
            }
        }
    }
    if !line.is_empty() {
        g.values(b, top + 0.17 * d, &line);
    }
    if let Some(at) = over.blatt {
        b.page.template += &label(
            g.col(81) + 0.25 * g.pitch(),
            top + at * d,
            g.label,
            "Blatt:",
        );
    }
    if over.code {
        let t = "00A/00B  00000/00000/000";
        g.values(b, top + 1.37 * d, &[(62, t)]);
        find(b, t);
    }
    let bottom = top - h * TITLE_GRID.len() as f32;
    for (at, t) in [(1.56, "Muster GmbH"), (2.11, "Erika Mustermann")] {
        b.page.template += &bold(g.col(0), bottom - at * d, 0.8 * d, t);
    }
    bottom
}

/// A page of the header grid with the title block `over` over it.
pub(crate) fn title_grid_page(g: &Geo, over: TitleBlock) -> Built {
    let mut b = Built::new(g);
    title_grid(g, &mut b, 760.0 + g.dy, over);
    b.address(g);
    b
}

/// The document title of the shape file's variant a, over the top rule of rows of fields drawn
/// by [`rows_page`].
pub(crate) fn title_over(g: &Geo, b: &mut Built) {
    let t = "Abrechnung der Muster/Muster-Bezuege";
    b.page.template += &bold(g.col(0), 760.0 + g.dy + 0.33 * g.data, 1.003 * g.data, t);
    b.values.push(t.to_string());
}

/// Text or a grid printed next to a grid of fields on its page, as forms have them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Neighbour {
    /// A data line at the values' depth over the top rule, across the dividers under it.
    DataLine,
    /// A bold proportional title of the data size at the values' depth over the top rule.
    Title,
    /// The title, a period line and the page number of the shape file's variant a.
    Cluster,
    /// The column titles of the next table in the labels' style, as far under the last rule
    /// of labels as labels stand under their rules and at the field starts of the last row,
    /// no rule under them; with lines of that table in the data font under them, or alone.
    ColumnTitles {
        body: bool,
    },
    /// A grid of two fields a row beside it on the same top rule, with rows of another pitch.
    Beside,
    /// A rule across the page a row pitch and a half over the grid's top rule (a section's
    /// rule), or as far under its last rule, its two rules framing nothing of the grid.
    RuleOver,
    RuleUnder,
    /// A short data line at the values' depth over the top rule from the first field's start,
    /// and the column titles alone under the last rule. When they could head every lowest
    /// field with a value, the same page is a grid of labels under their values whose first row
    /// has one value: neither reading is clear, so every field of the grid is to stay text with
    /// a loss. Else the grid pairs, labels over values.
    LineAndTitles,
}

/// A grid of fields to draw neighbours beside: its top rule, its last rule, the first data
/// columns of the fields of its last row, whether its labels stand under their values,
/// whether its labels and values are set in Courier (see [`rows_page`]) rather than in the
/// label font and the data layer, and whether its last rule runs under every field with a
/// value and no field under it (the header grid's right part ends after its first row), so
/// that column titles under it could head them all.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GridAt {
    pub top: f32,
    pub bottom: f32,
    pub starts: &'static [usize],
    pub under: bool,
    pub courier: bool,
    pub whole_bottom: bool,
}

impl GridAt {
    /// Writes `parts` (column, text) at baseline `y` in the grid's data font and size.
    fn data(&self, g: &Geo, b: &mut Built, y: f32, parts: &[(usize, &str)]) {
        if self.courier {
            for (k, t) in parts {
                b.page.template += &courier(g.col(*k), y, g.data, t);
            }
        } else {
            g.values(b, y, parts);
        }
    }

    /// Writes `text` at `x` and baseline `y` in the grid's label font and size.
    fn label(&self, g: &Geo, x: f32, y: f32, text: &str) -> String {
        if self.courier {
            courier(x, y, g.label, text)
        } else {
            label(x, y, g.label, text)
        }
    }
}

/// Draws `n` next to the grid `at` on the page `b`, its text in the grid's own fonts. Every
/// text it adds that a field must not take is to be found once; the fields of a grid beside
/// are to pair.
pub(crate) fn neighbour(g: &Geo, b: &mut Built, at: GridAt, n: Neighbour) {
    let d = g.data;
    let title = "Abrechnung der Muster/Muster-Bezuege";
    match n {
        Neighbour::DataLine => {
            let t = "Abrechnung Oktober 2026";
            at.data(g, b, at.top + 0.17 * d, &[(0, t)]);
            b.values.push(t.to_string());
        }
        Neighbour::Title => {
            b.page.template += &bold(g.col(0), at.top + 0.17 * d, 1.003 * d, title);
            b.values.push(title.to_string());
        }
        Neighbour::Cluster => {
            b.page.template += &bold(g.col(0), at.top + 0.33 * d, 1.003 * d, title);
            b.values.push(title.to_string());
            let t = "fuer Muster 2026";
            // Right of the title's end, so the two do not run into one word.
            at.data(g, b, at.top + 0.17 * d, &[(36, t), (85, "1")]);
            b.values.push(t.to_string());
        }
        Neighbour::ColumnTitles { body } => {
            // Under labels that stand under the last rule, a row pitch further down.
            let base = at.bottom - 0.86 * d - if at.under { 2.4 * d } else { 0.0 };
            let titles = ["Lohnart", "Bezeichnung", "Betrag"];
            for (&k, t) in at.starts.iter().zip(titles) {
                b.page.template += &at.label(g, g.col(k) + 0.18 * g.label, base, t);
            }
            if body {
                for (n, line) in [["1000", "Gehalt", "1.111,11"], ["2000", "Zulage", "22,22"]]
                    .iter()
                    .enumerate()
                {
                    let parts: Vec<(usize, &str)> = at
                        .starts
                        .iter()
                        .copied()
                        .zip(line.iter().copied())
                        .collect();
                    at.data(g, b, base - 1.2 * d * (n as f32 + 1.0), &parts);
                    b.values.extend(parts.iter().map(|p| p.1.to_string()));
                }
            }
        }
        Neighbour::Beside => {
            beside_grid(g, b, at.top, &[3.2, 2.0, 3.2, 2.0], true);
        }
        Neighbour::RuleOver | Neighbour::RuleUnder => {
            let h = 2.4 * d;
            let y = if n == Neighbour::RuleOver {
                at.top + 1.5 * h
            } else {
                at.bottom - 1.5 * h
            };
            b.page.template += &g.hrule(g.col(0) - 0.6 * d, g.col(70), y);
        }
        Neighbour::LineAndTitles => {
            let t = "Abrechnung";
            at.data(g, b, at.top + 0.17 * d, &[(0, t)]);
            b.values.push(t.to_string());
            neighbour(g, b, at, Neighbour::ColumnTitles { body: false });
            if at.whole_bottom {
                for f in &mut b.fields {
                    f.1 = Err(());
                }
            }
        }
    }
}

/// A page of one shape drawn by `draw` from the top, with the address under it.
pub(crate) fn shape_page(g: &Geo, draw: fn(&Geo, &mut Built, f32) -> f32) -> Built {
    let mut b = Built::new(g);
    draw(g, &mut b, 760.0 + g.dy);
    b.address(g);
    b
}
