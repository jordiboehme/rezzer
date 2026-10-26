// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rows of fields between drawn rules as print runs fill them, in the shapes of a real pay
//! slip (measured by structure only; all text and values invented), and generated variants
//! of each shape: shifted, scaled, other label and data sizes, other data fonts, other rule
//! thicknesses, with and without drawn decimal marks. Each variant is held to the same
//! invariants: no wrong pair ever, every value once, a pair where the geometry is clear, a
//! named loss where it is not.

mod common;

use common::{DataLine, FormPage, all_text, label, print_form};
use rezzer_model::{Block, LossReason, ReadOutcome, Severity, plain_text};

#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

fn read(bytes: &[u8]) -> ReadOutcome {
    common::read(bytes)
}

/// A rule drawn as a filled bar of thickness `t` from `x0` to `x1`, its top at `y`.
fn bar_h(x0: f32, x1: f32, y: f32, t: f32) -> String {
    format!("{x0} {} {} {t} re f\n", y - t, x1 - x0)
}

/// The same rule stroked as a line of width `t`, its middle at `y - t / 2`.
fn line_h(x0: f32, x1: f32, y: f32, t: f32) -> String {
    let m = y - t / 2.0;
    format!("q {t} w {x0} {m} m {x1} {m} l S Q\n")
}

/// A rule drawn as a filled bar of thickness `t` from `y0` to `y1`, centred on `x`.
fn bar_v(x: f32, y0: f32, y1: f32, t: f32) -> String {
    format!("{} {y0} {t} {} re f\n", x - t / 2.0, y1 - y0)
}

/// A separator mark of `h` points from `y0`, centred on `x`, filled and stroked with a
/// hairline as print runs draw it.
fn mark_b(x: f32, y0: f32, h: f32) -> String {
    let (l, r, top) = (x - 0.12, x + 0.12, y0 + h);
    format!("q 0 w {r} {top} m {l} {top} l {l} {y0} l {r} {y0} l h B* Q\n")
}

/// Text in Helvetica-Bold (`/FB`).
fn bold(x: f32, y: f32, size: f32, text: &str) -> String {
    format!("BT /FB {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// Text in Helvetica (`/FL`) without the scaling of a template label.
fn plain(x: f32, y: f32, size: f32, text: &str) -> String {
    format!("BT /FL {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// The font of the data layer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DataFont {
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
/// is a stroked line, and whether drawn decimal marks are on the page.
#[derive(Debug, Clone, Copy)]
struct Geo {
    dx: f32,
    dy: f32,
    data: f32,
    label: f32,
    font: DataFont,
    rule: f32,
    bar: f32,
    stroked_bar: bool,
    marks: bool,
    /// Data columns added to the second field of the strip: every later field moves right.
    wide: usize,
    /// The strip's last two fields are one.
    fewer: bool,
}

/// The geometry of the real form.
const REAL: Geo = Geo {
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
};

/// A field to expect: its label and the value it must pair with (`None`: empty), or `Err`
/// when the geometry leaves the field unclear (it must not pair, and a loss must name it).
type Expect = (&'static str, Result<Option<&'static str>, ()>);

/// A page being built: its template, its data lines, the values to find once each, and the
/// fields to expect.
struct Built {
    page: FormPage,
    values: Vec<String>,
    fields: Vec<Expect>,
}

impl Geo {
    /// The pitch of a character of the data font.
    fn pitch(&self) -> f32 {
        0.6 * self.data
    }

    /// The x of data column `k`.
    fn col(&self, k: usize) -> f32 {
        42.52 + self.dx + k as f32 * self.pitch()
    }

    /// Data column `k` of the base layout in this one: columns from the third field of the
    /// strip on move right by `wide`.
    fn k(&self, k: usize) -> usize {
        if k >= 20 { k + self.wide } else { k }
    }

    /// The advance of a digit of the data font.
    fn digit(&self) -> f32 {
        match self.font {
            DataFont::Helvetica => 0.556 * self.data,
            _ => self.pitch(),
        }
    }

    /// Writes `parts` (column, text) as the data line at baseline `y`: one padded line in
    /// Courier, or each part at its column in Helvetica.
    fn values(&self, b: &mut Built, y: f32, parts: &[(usize, &str)]) {
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

/// F1: a strip of seven fields (`left` to `right` in data columns) between two rules, all
/// labels on one baseline, all values on one data line, dividers that hang from the upper
/// rule over a third of the row. The rate field holds `rate` (its first two digits and the
/// next ones parted by a drawn decimal mark when `marks`), which fills it and runs on into the
/// next field when `overflow` (no space before the next value), as on the real form; the
/// last two labels are one run when `one_run`. Returns the rule's height at the bottom.
fn strip_f1(g: &Geo, b: &mut Built, top: f32, overflow: bool, one_run: bool) -> f32 {
    let h = 2.4 * g.data;
    let bottom = top - h;
    b.page.template += &bar_h(g.col(0) - 0.12, g.col(g.k(61)), top, g.rule);
    b.page.template += &bar_h(g.col(0) - 0.12, g.col(g.k(61)), bottom, g.rule);
    // Field starts in data columns.
    let mut starts = vec![0usize, 13, 43, 47, 51, 56, 58];
    if g.fewer {
        starts.pop();
    }
    for &k in &starts[1..] {
        b.page.template += &bar_v(g.col(g.k(k)) - 0.06, top - 1.21 * g.data, top, g.rule);
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
    // The raised footnote marker of the rate's label.
    b.page.template += &plain(
        lab(g.k(43)) + 0.6 * g.label * 3.0,
        base + 0.29 * g.label,
        0.71 * g.label,
        "8",
    );
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
    match g.font {
        DataFont::Helvetica => g.values(b, y, &parts),
        _ => {
            // One line: the rate and the group touch when they overflow.
            let mut text = String::new();
            for (k, t) in &parts {
                let len = text.chars().count();
                text += &" ".repeat(k - len);
                text += t;
            }
            b.page.lines.push(DataLine { y, text });
        }
    }
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
/// of its own that covers only its stretch (data columns 47 to 61), a divider hanging from
/// the long rule; a date in the first field (printed without its points and drawn as two
/// marks when `marks`) and the second field empty with two marks only. A row of the same
/// stretch under it. Returns the bottom of that row.
fn row_f2(g: &Geo, b: &mut Built, top: f32) -> f32 {
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
        b.page.template += &bar_h(x0, x1, bottom, g.rule);
        let split = g.k(split);
        b.page.template += &bar_v(
            g.col(split) - 0.18,
            row_top - 1.21 * g.data,
            row_top,
            g.rule,
        );
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
fn titled_f3(g: &Geo, b: &mut Built, above: f32) {
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
        b.page.template += &bar_h(gx0, gx1, y, g.rule);
    }
    for x in [gx0, gmid, gx1] {
        b.page.template += &bar_v(x, grid_top - 3.0 * g.data, grid_top, g.rule);
    }
    let head = grid_top - 0.88 * g.data;
    b.page.template += &plain(gx0 + 2.0, head, g.label, "Art");
    b.page.template += &plain(gmid + 2.0, head, g.label, "Betrag");
    let row = grid_top - 2.4 * g.data;
    b.page.template += &plain(gx0 + 2.0, row, g.data, "Gehalt");
    b.page.template += &plain(gmid + 2.0, row, g.data, "1.234,56");
}

/// The page of all three shapes in geometry `g`: F1 at the top (its rate overflowing when
/// `overflow`), F2 under it, F3 under that.
fn page(g: &Geo, overflow: bool, one_run: bool) -> Built {
    let mut b = Built {
        page: FormPage {
            left: 42.52 + g.dx,
            size: g.data,
            bold_data: g.font == DataFont::CourierBold,
            ..FormPage::default()
        },
        values: Vec::new(),
        fields: Vec::new(),
    };
    let top = 760.0 + g.dy;
    let below_f1 = strip_f1(g, &mut b, top, overflow, one_run);
    let below_f2 = row_f2(g, &mut b, below_f1);
    titled_f3(g, &mut b, below_f2);
    // Text in the data font elsewhere, so it is a text size of the page.
    for (k, l) in ["Muster GmbH", "Musterweg 2", "54321 Musterdorf"]
        .iter()
        .enumerate()
    {
        g.values(&mut b, 120.0 + g.dy - 1.2 * g.data * k as f32, &[(0, l)]);
    }
    b
}

/// The label and value pairs of the tables of two columns without a header row.
fn pairs_of(out: &ReadOutcome) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for b in &out.document.body {
        let Block::Section { blocks, .. } = b else {
            continue;
        };
        for inner in blocks {
            if let Block::Table {
                rows,
                header_rows: 0,
                ..
            } = inner
            {
                for r in rows {
                    if let [l, v] = r.as_slice() {
                        found.push((plain_text(&l.inlines), plain_text(&v.inlines)));
                    }
                }
            }
        }
    }
    found
}

/// The blocks of the first page.
fn blocks(out: &ReadOutcome) -> &[Block] {
    out.document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section { blocks, .. } => Some(blocks.as_slice()),
            _ => None,
        })
        .expect("a page")
}

/// The fields' invariants on a read: every pair is a right pair, each clear field pairs
/// (an empty one with `empty`), each unclear field does not pair and a loss names it, every
/// value stands once in the text. Each broken one is listed.
fn check_fields(out: &ReadOutcome, b: &Built) -> Vec<String> {
    let mut broken = Vec::new();
    let pairs = pairs_of(out);
    for (l, v) in &pairs {
        let label = l.trim_end_matches(" 8").trim_end_matches('8').trim();
        match b.fields.iter().find(|f| f.0 == label) {
            Some((_, Ok(Some(want)))) if v == want => {}
            Some((_, Ok(None))) if v == "empty" => {}
            Some((_, want)) => broken.push(format!("wrong pair {l:?}: {v:?}, want {want:?}")),
            // Pairs of other text (the title is checked on its own).
            None => {
                if l.contains("Hinweise") {
                    broken.push(format!("the title is a pair: {l:?}: {v:?}"));
                }
            }
        }
    }
    let uncertain: Vec<&str> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Uncertain && l.severity == Severity::Info)
        .map(|l| l.what.as_str())
        .collect();
    for (name, want) in &b.fields {
        let paired = pairs
            .iter()
            .any(|(l, _)| l.trim_end_matches(" 8").trim_end_matches('8').trim() == *name);
        match want {
            Ok(_) if !paired => broken.push(format!("{name} does not pair")),
            Err(()) if paired => broken.push(format!("{name} pairs though it is unclear")),
            Err(()) if !uncertain.iter().any(|w| w.contains(name)) => {
                broken.push(format!("no loss names {name}: {uncertain:?}"))
            }
            _ => {}
        }
    }
    let text = all_text(out);
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == '|')
        .filter(|w| !w.is_empty())
        .collect();
    for v in &b.values {
        let parts: Vec<&str> = v.split_whitespace().collect();
        let n = words
            .windows(parts.len())
            .filter(|w| *w == parts.as_slice())
            .count();
        if n != 1 {
            broken.push(format!("{v:?} stands {n} times"));
        }
    }
    broken
}

/// The title's invariants: no table row holds it, it is a heading followed directly by its
/// two lines, and it stands before the grid's table.
fn check_title(out: &ReadOutcome) -> Vec<String> {
    let mut broken = Vec::new();
    let bl = blocks(out);
    let heading = bl.iter().position(
        |b| matches!(b, Block::Heading { inlines, .. } if plain_text(inlines) == "Hinweise Muster"),
    );
    let Some(h) = heading else {
        broken.push("the title is no heading".into());
        return broken;
    };
    let next: String = bl[h + 1..]
        .iter()
        .take(2)
        .map(|b| match b {
            Block::Paragraph { inlines, .. } => plain_text(inlines),
            Block::List { .. } => "(a list)".into(),
            _ => "(other)".into(),
        })
        .collect::<Vec<_>>()
        .join(" | ");
    if !(next.contains("Muster vom 01.02.21 Kst.") && next.contains("- Musterhinweis 2 Muster")) {
        broken.push(format!("the title's lines do not follow it: {next:?}"));
    }
    let grid = bl.iter().position(|b| {
        matches!(b, Block::Table { rows, .. }
            if rows.iter().flatten().any(|c| plain_text(&c.inlines) == "Gehalt"))
    });
    if grid.is_some_and(|t| t < h) {
        broken.push("the title stands after the grid's table".into());
    }
    broken
}

#[test]
fn rate_over_its_divider_pairs_by_its_mark() {
    let b = page(&REAL, true, true);
    let out = read(&print_form(std::slice::from_ref(&b.page), "", ""));
    let broken = check_fields(&out, &b);
    assert!(broken.is_empty(), "{broken:#?}\n{:#?}", blocks(&out));
}

#[test]
fn rate_over_its_divider_without_a_mark_names_only_its_fields() {
    let g = Geo {
        marks: false,
        ..REAL
    };
    let b = page(&g, true, true);
    let out = read(&print_form(std::slice::from_ref(&b.page), "", ""));
    let broken = check_fields(&out, &b);
    assert!(broken.is_empty(), "{broken:#?}\n{:#?}", blocks(&out));
    let named: Vec<&str> = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.contains("row of fields") || l.what.contains("fields "))
        .map(|l| l.what.as_str())
        .collect();
    assert!(
        named
            .iter()
            .all(|w| !w.contains("Feld A") && !w.contains("Gr D")),
        "{named:?}"
    );
}

#[test]
fn row_under_a_longer_rule_pairs() {
    let b = page(&REAL, true, true);
    let out = read(&print_form(std::slice::from_ref(&b.page), "", ""));
    let pairs = pairs_of(&out);
    for want in [("Eintritt", "01.01.20"), ("Austritt", "empty")] {
        assert!(
            pairs.iter().any(|(l, v)| l == want.0 && v == want.1),
            "{want:?} in {pairs:?}"
        );
    }
}

#[test]
fn title_over_a_bar_heads_its_lines() {
    let b = page(&REAL, true, true);
    let out = read(&print_form(std::slice::from_ref(&b.page), "", ""));
    let broken = check_title(&out);
    assert!(broken.is_empty(), "{broken:#?}\n{:#?}", blocks(&out));
}

/// The generated variants of the three shapes, by name, each with whether its rate
/// overflows and whether its last two labels are one run.
fn variants() -> Vec<(String, Geo, bool, bool)> {
    let mut out = Vec::new();
    for (name, g) in [
        ("real", REAL),
        (
            "shifted",
            Geo {
                dx: -17.3,
                dy: -41.0,
                ..REAL
            },
        ),
        ("larger data", Geo { data: 10.5, ..REAL }),
        ("smaller data", Geo { data: 8.5, ..REAL }),
        ("larger labels", Geo { label: 7.5, ..REAL }),
        ("smaller labels", Geo { label: 5.6, ..REAL }),
        (
            "courier",
            Geo {
                font: DataFont::Courier,
                ..REAL
            },
        ),
        (
            "helvetica",
            Geo {
                font: DataFont::Helvetica,
                ..REAL
            },
        ),
        (
            "thick rules",
            Geo {
                rule: 0.6,
                bar: 1.2,
                ..REAL
            },
        ),
        (
            "hairlines",
            Geo {
                rule: 0.1,
                bar: 0.3,
                ..REAL
            },
        ),
        (
            "stroked bar",
            Geo {
                stroked_bar: true,
                ..REAL
            },
        ),
        (
            "no marks",
            Geo {
                marks: false,
                ..REAL
            },
        ),
        ("wider field", Geo { wide: 5, ..REAL }),
        (
            "one field less",
            Geo {
                fewer: true,
                ..REAL
            },
        ),
        (
            "wider and fewer, courier",
            Geo {
                wide: 3,
                fewer: true,
                font: DataFont::Courier,
                ..REAL
            },
        ),
        (
            "all changed",
            Geo {
                dx: 11.0,
                dy: 23.0,
                data: 10.0,
                label: 7.0,
                font: DataFont::Courier,
                rule: 0.4,
                bar: 1.0,
                stroked_bar: true,
                marks: true,
                wide: 0,
                fewer: false,
            },
        ),
    ] {
        for overflow in [true, false] {
            let label = format!("{name}{}", if overflow { ", overflow" } else { "" });
            out.push((label, g, overflow, true));
        }
    }
    // The two short labels as two text objects, each in its field, close together.
    for (name, g) in [
        ("separate labels", REAL),
        ("separate labels, larger data", Geo { data: 10.5, ..REAL }),
        (
            "separate labels, smaller labels",
            Geo { label: 5.6, ..REAL },
        ),
    ] {
        out.push((name.to_string(), g, true, false));
    }
    out
}

/// Variants that break an invariant today, with why (reported, not tuned away): the test
/// fails when one of them starts to hold or another one breaks.
const KNOWN: &[(&str, &str)] = &[(
    "separate labels",
    "the two short labels are two text objects less than half a label size apart, so the \
     text pass reads them as one word without a space; that word stands across the divider \
     between them, so both fields stay text with a loss that names them (no wrong pair)",
)];

#[test]
fn generated_variants_hold_the_invariants() {
    let mut report = Vec::new();
    let mut unexpected = Vec::new();
    for (name, g, overflow, one_run) in variants() {
        let b = page(&g, overflow, one_run);
        let out = read(&print_form(std::slice::from_ref(&b.page), "", ""));
        let mut broken = check_fields(&out, &b);
        broken.extend(check_title(&out));
        let known = KNOWN.iter().any(|k| k.0 == name);
        if !broken.is_empty() {
            report.push(format!("{name}: {broken:?}"));
        }
        if broken.is_empty() == known {
            unexpected.push(name);
        }
    }
    assert!(
        unexpected.is_empty(),
        "unexpected: {unexpected:?}\n{}",
        report.join("\n")
    );
}
