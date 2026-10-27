// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The axes a shape is varied along, the sample of their combinations each shape is read in,
//! and the pages a variant draws.

use crate::draw::{Stroke, bold};
use crate::shapes::{Built, DataFont, Geo, REAL};

/// The page scale: the whole page printed this many times as large. 0.7 and 1.5 are the
/// scale looks of Task 9, 0.75 and 1.5 the scales of ruling P6-Y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Scale {
    S070,
    S075,
    S100,
    S150,
}

impl Scale {
    pub(crate) fn factor(self) -> f32 {
        match self {
            Scale::S070 => 0.7,
            Scale::S075 => 0.75,
            Scale::S100 => 1.0,
            Scale::S150 => 1.5,
        }
    }
}

/// The text sizes on the page, before the scale: the real ones, or all about a tenth larger
/// or smaller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Size {
    Real,
    Larger,
    Smaller,
}

/// The font of the data layer, or of the body text of a business page: monospaced
/// (Courier-Bold, Courier) or proportional (Helvetica on a form, Times on a business page).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Data {
    CourierBold,
    Courier,
    Proportional,
}

/// The font of the template labels: Helvetica at 90 % as templates set them, or Times.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Labels {
    Helvetica,
    Times,
}

/// How the rules are drawn (with the scales, the six looks of Task 9): filled bars as on the
/// real form, dashed, dotted, dividers rising from the rule under a row, or thick bars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Look {
    Real,
    Dashed,
    Dotted,
    Rising,
    Thick,
}

/// How the page is turned on a landscape page box: not, a quarter turn counterclockwise
/// (90 degrees) or clockwise (270 degrees).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Turn {
    None,
    Left,
    Right,
}

/// What stands next to the shape: nothing, a bold title over it, a data line over it, a row
/// of column titles with two lines of a table under it, or a second block of fields beside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Next {
    None,
    TitleOver,
    LineOver,
    RowUnder,
    Beside,
}

/// The file has one page, or two pages of the same shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Pages {
    One,
    Two,
}

/// One combination of the axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Variant {
    pub scale: Scale,
    pub size: Size,
    pub data: Data,
    pub labels: Labels,
    pub look: Look,
    pub turn: Turn,
    pub next: Next,
    pub pages: Pages,
}

/// The variant every shape is drawn in first: the real form.
pub(crate) const BASE: Variant = Variant {
    scale: Scale::S100,
    size: Size::Real,
    data: Data::CourierBold,
    labels: Labels::Helvetica,
    look: Look::Real,
    turn: Turn::None,
    next: Next::None,
    pages: Pages::One,
};

impl Variant {
    /// The name of the variant: the axes where it is not [`BASE`], or `base`.
    pub(crate) fn name(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.scale != BASE.scale {
            parts.push(format!("scale {}", self.scale.factor()));
        }
        match self.size {
            Size::Real => {}
            Size::Larger => parts.push("larger text".into()),
            Size::Smaller => parts.push("smaller text".into()),
        }
        match self.data {
            Data::CourierBold => {}
            Data::Courier => parts.push("courier".into()),
            Data::Proportional => parts.push("proportional".into()),
        }
        if self.labels == Labels::Times {
            parts.push("times labels".into());
        }
        match self.look {
            Look::Real => {}
            Look::Dashed => parts.push("dashed".into()),
            Look::Dotted => parts.push("dotted".into()),
            Look::Rising => parts.push("rising".into()),
            Look::Thick => parts.push("thick".into()),
        }
        match self.turn {
            Turn::None => {}
            Turn::Left => parts.push("turned 90".into()),
            Turn::Right => parts.push("turned 270".into()),
        }
        match self.next {
            Next::None => {}
            Next::TitleOver => parts.push("title over".into()),
            Next::LineOver => parts.push("line over".into()),
            Next::RowUnder => parts.push("row under".into()),
            Next::Beside => parts.push("beside".into()),
        }
        if self.pages == Pages::Two {
            parts.push("two pages".into());
        }
        if parts.is_empty() {
            "base".into()
        } else {
            parts.join(", ")
        }
    }

    /// The same variant at scale 1 in the real sizes and fonts: the read every scale and
    /// font of it must equal.
    pub(crate) fn reference(&self) -> Variant {
        Variant {
            scale: Scale::S100,
            size: Size::Real,
            data: Data::CourierBold,
            labels: Labels::Helvetica,
            ..*self
        }
    }

    /// The geometry of a form drawn in this variant (its scale and turn are set on the pages
    /// afterwards). A turned page keeps its drawn marks: its digits read in the direction of
    /// the page's text.
    pub(crate) fn geo(&self) -> Geo {
        let (data, label) = match self.size {
            Size::Real => (9.5, 6.6),
            Size::Larger => (10.5, 7.3),
            Size::Smaller => (8.5, 5.9),
        };
        let font = match self.data {
            Data::CourierBold => DataFont::CourierBold,
            Data::Courier => DataFont::Courier,
            Data::Proportional => DataFont::Helvetica,
        };
        let (stroke, rising, rule, bar) = match self.look {
            Look::Real => (Stroke::Bar, false, REAL.rule, REAL.bar),
            Look::Dashed => (Stroke::Dashed, false, REAL.rule, REAL.bar),
            Look::Dotted => (Stroke::Dotted, false, REAL.rule, REAL.bar),
            Look::Rising => (Stroke::Bar, true, REAL.rule, REAL.bar),
            Look::Thick => (Stroke::Bar, false, 0.6, 1.2),
        };
        Geo {
            data,
            label,
            font,
            stroke,
            rising,
            rule,
            bar,
            times_labels: self.labels == Labels::Times,
            ..REAL
        }
    }

    /// The size factor of the text of a business page (its body is 7 points at 1).
    pub(crate) fn k(&self) -> f32 {
        match self.size {
            Size::Real => 1.0,
            Size::Larger => 1.1,
            Size::Smaller => 0.9,
        }
    }

    /// The plain and bold fonts of a business page: Helvetica, or Times when the data axis is
    /// proportional.
    pub(crate) fn faces(&self) -> (&'static str, &'static str) {
        match self.data {
            Data::Proportional => ("FT", "FTB"),
            _ => ("FL", "FB"),
        }
    }
}

/// The values of each axis a shape is drawn in; the first value of each is the base one.
#[derive(Debug, Clone)]
pub(crate) struct Axes {
    pub scale: &'static [Scale],
    pub size: &'static [Size],
    pub data: &'static [Data],
    pub labels: &'static [Labels],
    pub look: &'static [Look],
    pub turn: &'static [Turn],
    pub next: &'static [Next],
    pub pages: &'static [Pages],
}

/// Every axis in every value.
pub(crate) const ALL: Axes = Axes {
    scale: &[Scale::S100, Scale::S070, Scale::S075, Scale::S150],
    size: &[Size::Real, Size::Larger, Size::Smaller],
    data: &[Data::CourierBold, Data::Courier, Data::Proportional],
    labels: &[Labels::Helvetica, Labels::Times],
    look: &[
        Look::Real,
        Look::Dashed,
        Look::Dotted,
        Look::Rising,
        Look::Thick,
    ],
    turn: &[Turn::None, Turn::Left, Turn::Right],
    next: &[
        Next::None,
        Next::TitleOver,
        Next::LineOver,
        Next::RowUnder,
        Next::Beside,
    ],
    pages: &[Pages::One, Pages::Two],
};

impl Axes {
    fn sizes(&self) -> [usize; 8] {
        [
            self.scale.len(),
            self.size.len(),
            self.data.len(),
            self.labels.len(),
            self.look.len(),
            self.turn.len(),
            self.next.len(),
            self.pages.len(),
        ]
    }

    fn variant(&self, at: [usize; 8]) -> Variant {
        Variant {
            scale: self.scale[at[0]],
            size: self.size[at[1]],
            data: self.data[at[2]],
            labels: self.labels[at[3]],
            look: self.look[at[4]],
            turn: self.turn[at[5]],
            next: self.next[at[6]],
            pages: self.pages[at[7]],
        }
    }

    /// The sample of combinations, in a fixed order: the base, then each value of each axis
    /// alone (one axis at a time), then every scale with every text size and data font (the
    /// axes an absolute threshold is sensitive to), then a pairwise cover: rows chosen greedily
    /// until every pair of values of two axes stands in some row. A row starts from the first
    /// pair no row holds yet and takes, axis by axis, the value that adds the most such pairs
    /// (the first on a tie), so the sample is the same on every run. No variant is left out of
    /// the sample for what it reads.
    pub(crate) fn sample(&self) -> Vec<Variant> {
        let n = self.sizes();
        let mut rows: Vec<[usize; 8]> = vec![[0; 8]];
        for a in 0..8 {
            for v in 1..n[a] {
                let mut r = [0; 8];
                r[a] = v;
                rows.push(r);
            }
        }
        for scale in 0..n[0] {
            for size in 0..n[1] {
                for data in 0..n[2] {
                    let r = [scale, size, data, 0, 0, 0, 0, 0];
                    if !rows.contains(&r) {
                        rows.push(r);
                    }
                }
            }
        }
        // Whether some row holds value `va` of axis `a` and `vb` of axis `b`.
        let covered = |rows: &[[usize; 8]], (a, va): (usize, usize), (b, vb): (usize, usize)| {
            rows.iter().any(|r| r[a] == va && r[b] == vb)
        };
        loop {
            let mut first = None;
            'find: for a in 0..8 {
                for b in a + 1..8 {
                    for va in 0..n[a] {
                        for vb in 0..n[b] {
                            if !covered(&rows, (a, va), (b, vb)) {
                                first = Some((a, va, b, vb));
                                break 'find;
                            }
                        }
                    }
                }
            }
            let Some((a, va, b, vb)) = first else {
                break;
            };
            let mut row: [Option<usize>; 8] = [None; 8];
            row[a] = Some(va);
            row[b] = Some(vb);
            for c in 0..8 {
                if row[c].is_some() {
                    continue;
                }
                // The value that adds the most uncovered pairs with the values set so far.
                let mut best = (0usize, 0usize);
                for vc in 0..n[c] {
                    let gain = (0..8)
                        .filter_map(|d| row[d].map(|vd| (d, vd)))
                        .filter(|&set| !covered(&rows, (c, vc), set))
                        .count();
                    if gain > best.1 {
                        best = (vc, gain);
                    }
                }
                row[c] = Some(best.0);
            }
            rows.push(row.map(|v| v.unwrap_or(0)));
        }
        let mut out: Vec<Variant> = Vec::new();
        for r in rows {
            let v = self.variant(r);
            if !out.contains(&v) {
                out.push(v);
            }
        }
        out
    }
}

/// What a page must read as, besides the fields, values and columns of its [`Built`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Want {
    /// Tables to find on the page, each with exactly these rows of cell texts.
    pub tables: Vec<Vec<Vec<String>>>,
    /// Texts to find as headings.
    pub headings: Vec<String>,
    /// Texts no heading may be.
    pub no_headings: Vec<String>,
    /// Paragraphs of a role to find: the role and the text.
    pub roles: Vec<(String, String)>,
    /// Paragraphs to find, by their text (line breaks as spaces).
    pub paragraphs: Vec<String>,
    /// Texts the page's text holds in this order.
    pub order: Vec<String>,
    /// Texts no block may hold together: each pair stands in different blocks.
    pub apart: Vec<(String, String)>,
    /// The document title the read must find (on the first page only).
    pub title: Option<String>,
    /// Titles the read may find instead (a bold title over the shape).
    pub may_title: Vec<String>,
    /// The document date the read must find (yyyy-mm-dd), on the first page only.
    pub date: Option<String>,
    /// Codes to find once each: symbology and text.
    pub codes: Vec<(String, String)>,
    /// Whether some asset of the page needs vision.
    pub vision: Option<bool>,
    /// Parts of the text of losses the page must have.
    pub losses: Vec<String>,
}

/// One page of a variant: what is drawn and what it must read as.
pub(crate) struct Sheet {
    pub built: Built,
    pub want: Want,
    /// Texts the shape shows, each to be found as many times as it is drawn (see
    /// [`expect_drawn`]).
    pub once: Vec<String>,
}

impl Sheet {
    pub(crate) fn new(built: Built) -> Sheet {
        Sheet {
            built,
            want: Want::default(),
            once: Vec::new(),
        }
    }
}

/// Where a shape stands on its page, for its neighbours: its top and bottom, and its right
/// end, in points of the template.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Extent {
    pub top: f32,
    pub bottom: f32,
    pub right: f32,
}

/// A shape: its name, the axes it is drawn along, and how it draws one page in a variant
/// (with where it stands, when neighbours may be drawn next to it).
pub(crate) struct Shape {
    pub name: &'static str,
    pub axes: Axes,
    pub draw: fn(&Variant) -> (Sheet, Option<Extent>),
}

/// One variant of a shape, as the brief's `Page`: its name, the variant and the pages of the
/// file (the second page, when there is one, holds the same shape).
pub(crate) struct Page {
    pub name: String,
    pub variant: Variant,
    pub sheets: Vec<Sheet>,
}

/// Every sampled variant of `shape` (see [`Axes::sample`]).
pub fn variants(shape: &Shape) -> impl Iterator<Item = Page> + '_ {
    shape.axes.sample().into_iter().map(|v| page_of(shape, v))
}

/// The page of `shape` in variant `v`.
pub(crate) fn page_of(shape: &Shape, v: Variant) -> Page {
    let count = if v.pages == Pages::Two { 2 } else { 1 };
    let sheets = (0..count)
        .map(|n| {
            let (mut sheet, extent) = (shape.draw)(&v);
            if let Some(e) = extent {
                neighbour(&v, &mut sheet, e);
            }
            finish(&v, &mut sheet);
            expect_drawn(&mut sheet, n + 1);
            sheet
        })
        .collect();
    Page {
        name: v.name(),
        variant: v,
        sheets,
    }
}

/// Sets the scale, the turn and the label font of a drawn page.
fn finish(v: &Variant, sheet: &mut Sheet) {
    let page = &mut sheet.built.page;
    page.scale = v.scale.factor();
    page.turn = v.turn != Turn::None;
    page.clockwise = v.turn == Turn::Right;
    if v.labels == Labels::Times {
        page.template = times_labels(&page.template);
    }
}

/// `template` with every template label (Helvetica at 90 %, see `common::label`) set in
/// Times-Roman instead.
fn times_labels(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    for (k, part) in template.split("BT /FL ").enumerate() {
        if k > 0 {
            let label = part
                .split_once("ET")
                .is_some_and(|(head, _)| head.contains(" Tf 90 Tz "));
            out.push_str(if label { "BT /FT " } else { "BT /FL " });
        }
        out.push_str(part);
    }
    out
}

/// Draws the neighbour of `v` next to the shape at `e`, its text in the variant's fonts;
/// every text it adds is to be found once, and the fields of a block beside are to pair.
fn neighbour(v: &Variant, sheet: &mut Sheet, e: Extent) {
    let g = v.geo();
    let d = g.data;
    let b = &mut sheet.built;
    match v.next {
        Next::None => {}
        Next::TitleOver => {
            let t = "Monatsabrechnung Muster";
            b.page.template += &bold(g.col(0), e.top + 2.6 * d, 1.4 * d, t);
            b.values.push(t.into());
            sheet.want.may_title.push(t.into());
        }
        Next::LineOver => {
            let t = "Abrechnung Oktober 2026";
            g.values(b, e.top + 1.6 * d, &[(0, t)]);
            b.values.push(t.into());
        }
        Next::RowUnder => {
            let base = e.bottom - 2.4 * d;
            for (k, t) in [(0, "Kennung"), (14, "Vorgang"), (40, "Wert")] {
                b.page.template +=
                    &crate::common::label(g.col(k) + 0.18 * g.label, base, g.label, t);
                b.values.push(t.into());
            }
            for (n, line) in [
                [(0, "7001"), (14, "Fahrgeld"), (40, "45,67")],
                [(0, "7002"), (14, "Essensgeld"), (40, "8,90")],
            ]
            .iter()
            .enumerate()
            {
                g.values(b, base - 1.2 * d * (n as f32 + 1.0), line);
                b.values.extend(line.iter().map(|p| p.1.to_string()));
            }
        }
        Next::Beside => {
            // A block of three fields, each a label over its value between rules, four data
            // columns right of the shape.
            let k0 = ((e.right - g.col(0)) / g.pitch()).ceil() as usize + 4;
            let (x0, x1) = (g.col(k0) - 0.12, g.col(k0 + 12));
            let h = 2.4 * d;
            for (n, (l, val)) in [("Folge", "4/9"), ("Vorg", "Q"), ("Zeichen", "770")]
                .iter()
                .enumerate()
            {
                let rule = e.top - h * n as f32;
                b.page.template += &g.hrule(x0, x1, rule);
                b.page.template +=
                    &crate::common::label(g.col(k0) + 0.18 * g.label, rule - 0.86 * d, g.label, l);
                g.values(b, rule - 2.23 * d, &[(k0, val)]);
                b.values.push(val.to_string());
                b.fields.push((l, Ok(Some(val))));
            }
            b.page.template += &g.hrule(x0, x1, e.top - 3.0 * h);
        }
    }
}

/// The words of a text, cut at white space.
fn words_of(s: &str) -> Vec<&str> {
    s.split_whitespace().collect()
}

/// The texts drawn on `page`: each text of the template shown with `Tj`, each template label
/// shown with `TJ` (its parts joined), and each data line.
fn drawn(page: &crate::common::FormPage) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in page.template.split("BT ").skip(1) {
        let Some((head, _)) = chunk.split_once("ET") else {
            continue;
        };
        if head.contains(") Tj") || head.contains("] TJ") {
            out.push(literals(head));
        }
    }
    out.extend(page.lines.iter().map(|l| l.text.clone()));
    out
}

/// The strings of the literals `(...)` in `ops`, joined, escapes undone.
fn literals(ops: &str) -> String {
    let mut out = String::new();
    let (mut inside, mut escaped) = (false, false);
    for c in ops.chars() {
        match (inside, escaped, c) {
            (true, true, _) => {
                out.push(c);
                escaped = false;
            }
            (true, false, '\\') => escaped = true,
            (true, false, ')') => inside = false,
            (true, false, _) => out.push(c),
            (false, _, '(') => inside = true,
            _ => {}
        }
    }
    out
}

/// Holds every value of `sheet`, page `number` of its file, to be found as many times as it
/// is drawn: each value of the page and each text of `once` (the texts a shape shows) is
/// listed in the values once for every place its words stand, one after the other, in a drawn
/// text (or, for a text no drawn text holds whole, across drawn texts in the order they are
/// drawn) or in the page's marker (`Page 1`, which the read writes as a paragraph). A text of
/// `once` that is drawn nowhere as it stands (a run cut by the read) is left out.
pub(crate) fn expect_drawn(sheet: &mut Sheet, number: usize) {
    let mut drawn = drawn(&sheet.built.page);
    drawn.push(format!("Page {number}"));
    let all: Vec<Vec<&str>> = drawn.iter().map(|t| words_of(t)).collect();
    let joined: Vec<&str> = all.iter().flatten().copied().collect();
    let b = &mut sheet.built;
    let mut texts: Vec<String> = Vec::new();
    for t in b.values.iter().chain(sheet.once.iter()) {
        if !texts.contains(t) {
            texts.push(t.clone());
        }
    }
    for t in texts {
        let want = words_of(&t);
        if want.is_empty() {
            continue;
        }
        let n: usize = all
            .iter()
            .map(|w| {
                w.windows(want.len())
                    .filter(|x| *x == want.as_slice())
                    .count()
            })
            .sum();
        // A text of several words that no drawn text holds whole may stand across two drawn
        // texts one after the other (a value drawn in parts): it is counted as the read joins
        // them, in the order they are drawn.
        let n = if n == 0 && want.len() > 1 {
            joined
                .windows(want.len())
                .filter(|x| *x == want.as_slice())
                .count()
        } else {
            n
        };
        let have = b.values.iter().filter(|x| **x == t).count();
        for _ in have..n {
            b.values.push(t.clone());
        }
    }
}

/// The texts `template` shows with `Tj` (as [`crate::common::show`] writes them), in order.
pub(crate) fn shown(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find(" Td (") {
        rest = &rest[i + 5..];
        match rest.find(") Tj") {
            Some(j) => {
                out.push(rest[..j].to_string());
                rest = &rest[j..];
            }
            None => break,
        }
    }
    out
}
