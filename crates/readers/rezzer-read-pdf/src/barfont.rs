// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Barcodes printed with a barcode font: the bars are glyphs of a font, and the text of those
//! glyphs (`Í%"$"*%!)Î`) is no text of the page. A run is taken for such a code only on
//! positive evidence: the font's name says it is a barcode font, or the glyphs of the run are
//! all upright bars of one height whose widths and spaces are whole modules. Odd characters
//! alone are never evidence. The bars' boxes become regions of the page that are rendered
//! again and read like codes drawn as paths ([`crate::codes::VectorCode`]).

use std::hash::{DefaultHasher, Hash, Hasher};

use kurbo::{Affine, BezPath, PathEl, Point, Shape as _};

use crate::codes::VectorCode;

/// Parts of a font's base name (lowercase, without a subset prefix, spaces, hyphens and
/// underscores) that name a barcode font: `Code128`, `Code 128`, `Code39`, `3of9`, `EAN13`,
/// `Barcode`, `IDAutomation`, `Libre Barcode` and the like.
const NAMES: [&str; 8] = [
    "code128",
    "code39",
    "code93",
    "3of9",
    "2of5",
    "ean13",
    "barcode",
    "idautomation",
];

/// Short names of a symbology that name a barcode font only as a whole word of the name
/// (`BC C128 HD Wide`), as they are short enough to stand inside other words.
const SHORT: [&str; 9] = [
    "c128", "c39", "c93", "i2of5", "i25", "ean13", "ean8", "upca", "upce",
];

/// Whether a font's base name (lowercase, subset prefix removed) names a barcode font: one of
/// [`NAMES`] anywhere in it, or one of [`SHORT`] as a word (words are cut at anything but a
/// letter or digit; two words in a row count too, so `UPC-A` is `upca`).
pub(crate) fn barcode_name(name: &str) -> bool {
    let name = name.replace("#20", " ");
    let squeezed: String = name
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .collect();
    if NAMES.iter().any(|n| squeezed.contains(n)) {
        return true;
    }
    let words: Vec<&str> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let short = |w: &str| SHORT.contains(&w);
    words.iter().any(|w| short(w))
        || words
            .windows(2)
            .any(|p| short(&format!("{}{}", p[0], p[1])))
}

/// The widest bar or space of a code, in modules: Code 128 and EAN have bars of up to 4.
const MOST_MODULES: f64 = 4.0;

/// How far a width may lie from a whole number of modules, in modules.
const WHOLE: f64 = 0.2;

/// The share of a bar's height by which the bottoms and tops of the bars of one font may
/// differ.
const SAME_HEIGHT: f64 = 0.02;

/// Bars a run shows at least (bars of glyphs that touch are one), different glyphs among them,
/// and different widths of its bars and spaces, to be a code by its outlines: the shortest
/// Code 128 (a start, one symbol, the check and the stop) has 13 bars and widths of 1 to 4
/// modules.
const MIN_BARS: usize = 12;
const MIN_KINDS: usize = 2;
const MIN_WIDTHS: usize = 3;

/// Bars of glyphs side by side whose gap is under this share of the run's narrowest bar are
/// one bar (a font that draws a bar across two glyphs, widths rounded to font units).
const TOUCH: f64 = 0.25;

/// The bars of a run may stand this many modules apart (each way) and still be one code.
const JOIN_MODULES: f32 = 3.0;

/// Runs of a page joined into codes by comparing each pair, at most; past them each run is a
/// region of its own.
const MAX_JOINED: usize = 2000;

/// Glyphs of barcode fonts a page keeps the places of, at most: past them a note says so.
pub(crate) const MAX_GLYPHS: usize = 50_000;

/// What the outline of a glyph is, in font units (1000 to the em).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outline {
    /// Nothing is drawn (a space).
    Empty,
    /// One or more upright bars side by side from `y0` to `y1`, each from its left to its
    /// right, left to right, each at least twice as high as the widest. A font may draw one
    /// bar a glyph, so one bar is a bar here; whether a run of them is a code is the run's
    /// question ([`bars_fit`]).
    Bars {
        y0: f64,
        y1: f64,
        bars: Vec<(f64, f64)>,
    },
    /// Anything else: curves, slanted lines, bars of different heights, squares.
    Other,
}

/// What a glyph's outline is, with the box of its ink in font units when it is not bars.
pub(crate) type Looked = (Outline, Option<[f64; 4]>);

/// The rectangle a subpath of `points` draws, when its sides lie along the axes and it has
/// four corners: points on a side between two corners (as a TrueType contour may have) and a
/// closing point on the first are left out.
fn upright_rect(points: &[Point]) -> Option<[f64; 4]> {
    let mut p: Vec<Point> = Vec::with_capacity(8);
    for &q in points {
        if p.last() != Some(&q) {
            p.push(q);
        }
    }
    while p.len() > 1 && p.first() == p.last() {
        p.pop();
    }
    // Every side along an axis.
    let n = p.len();
    if !(4..=16).contains(&n) {
        return None;
    }
    for k in 0..n {
        let (a, b) = (p[k], p[(k + 1) % n]);
        if a.x != b.x && a.y != b.y {
            return None;
        }
    }
    // The corners: points where the side turns.
    let corners = (0..n)
        .filter(|&k| {
            let (a, b, c) = (p[(k + n - 1) % n], p[k], p[(k + 1) % n]);
            (a.x == b.x) != (b.x == c.x)
        })
        .count();
    if corners != 4 {
        return None;
    }
    let x0 = p.iter().map(|q| q.x).fold(f64::MAX, f64::min);
    let x1 = p.iter().map(|q| q.x).fold(f64::MIN, f64::max);
    let y0 = p.iter().map(|q| q.y).fold(f64::MAX, f64::min);
    let y1 = p.iter().map(|q| q.y).fold(f64::MIN, f64::max);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

/// Whether a curve from `a` to `b` with control points `c` is a straight side along an axis:
/// every control point on that side.
fn straight(a: Point, b: Point, c: &[Point]) -> bool {
    (a.x == b.x
        && c.iter()
            .all(|q| q.x == a.x && q.y >= a.y.min(b.y) && q.y <= a.y.max(b.y)))
        || (a.y == b.y
            && c.iter()
                .all(|q| q.y == a.y && q.x >= a.x.min(b.x) && q.x <= a.x.max(b.x)))
}

/// What the outline `path` of a glyph is (see [`Outline`]). It is read the same for every
/// font program (TrueType, Type1, CFF): a curve whose control points lie on a straight side
/// along an axis is that side.
pub(crate) fn outline(path: &BezPath) -> Outline {
    let mut rects: Vec<[f64; 4]> = Vec::new();
    let mut points: Vec<Point> = Vec::new();
    let end = |points: &mut Vec<Point>, rects: &mut Vec<[f64; 4]>| -> bool {
        if points.is_empty() {
            return true;
        }
        let r = upright_rect(points);
        points.clear();
        match r {
            Some(r) => {
                rects.push(r);
                true
            }
            None => false,
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if !end(&mut points, &mut rects) {
                    return Outline::Other;
                }
                points.push(p);
            }
            PathEl::LineTo(p) => points.push(p),
            PathEl::QuadTo(c, p) => match points.last() {
                Some(&a) if straight(a, p, &[c]) => points.push(p),
                _ => return Outline::Other,
            },
            PathEl::CurveTo(c1, c2, p) => match points.last() {
                Some(&a) if straight(a, p, &[c1, c2]) => points.push(p),
                _ => return Outline::Other,
            },
            PathEl::ClosePath => {
                if !end(&mut points, &mut rects) {
                    return Outline::Other;
                }
            }
        }
        if rects.len() > 64 || points.len() > 17 {
            return Outline::Other;
        }
    }
    if !end(&mut points, &mut rects) {
        return Outline::Other;
    }
    if rects.is_empty() {
        return Outline::Empty;
    }
    rects.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let (y0, y1) = (rects[0][1], rects[0][3]);
    let height = y1 - y0;
    let widest = rects.iter().map(|r| r[2] - r[0]).fold(0.0, f64::max);
    let level = |a: f64, b: f64| (a - b).abs() <= SAME_HEIGHT * height;
    let upright = rects.iter().all(|r| level(r[1], y0) && level(r[3], y1))
        && rects.windows(2).all(|w| w[1][0] >= w[0][2])
        && height >= 2.0 * widest;
    if !upright {
        return Outline::Other;
    }
    Outline::Bars {
        y0,
        y1,
        bars: rects.iter().map(|r| (r[0], r[2])).collect(),
    }
}

/// Whether the outlines of a run's glyphs (each with its glyph id and its transform, font
/// units to the page) are a code, on the run's bars as the page shows them: no glyph is
/// anything but bars or a space; all bars of one height; bars of glyphs that touch joined
/// into one; at least [`MIN_BARS`] bars, from [`MIN_KINDS`] different glyphs; every bar and
/// every space between two bars a whole number of modules (the narrowest of them), at most
/// [`MOST_MODULES`], and at least [`MIN_WIDTHS`] different widths among them. A row of equal
/// bars (a ruler, `||||`) or of one glyph is no code; nor are letters, which are not bars.
pub(crate) fn bars_fit(glyphs: &[(u32, Affine, &Outline)]) -> bool {
    let Some((joined, kinds)) = run_bars(glyphs) else {
        return false;
    };
    if kinds < MIN_KINDS || joined.len() < MIN_BARS {
        return false;
    }
    let widths = widths_of(&joined);
    let module = widths.iter().copied().fold(f64::MAX, f64::min);
    if module <= 0.0 || !module.is_finite() {
        return false;
    }
    let mut seen: Vec<i64> = Vec::new();
    for &w in &widths {
        let n = w / module;
        let r = n.round();
        if r < 1.0 || r > MOST_MODULES || (n - r).abs() > WHOLE {
            return false;
        }
        if !seen.contains(&(r as i64)) {
            seen.push(r as i64);
        }
    }
    seen.len() >= MIN_WIDTHS
}

/// The bars of a run as the page shows them, in points along the baseline of its first glyph
/// (bars of glyphs that touch joined into one), and how many different glyphs drew them.
/// `None` when a glyph is not bars or a space, or the bars differ in height.
fn run_bars(glyphs: &[(u32, Affine, &Outline)]) -> Option<(Vec<(f64, f64)>, usize)> {
    if glyphs.iter().any(|(_, _, o)| matches!(o, Outline::Other)) {
        return None;
    }
    let &(_, first, _) = glyphs.first()?;
    let c = first.as_coeffs();
    let len = c[0].hypot(c[1]);
    if len <= 0.0 {
        return None;
    }
    let (dx, dy) = (c[0] / len, c[1] / len);
    let origin = Point::new(c[4], c[5]);
    let along = |p: Point| (p.x - origin.x) * dx + (p.y - origin.y) * dy;
    let mut height: Option<(f64, f64)> = None;
    let mut bars: Vec<(f64, f64)> = Vec::new();
    let mut kinds: Vec<u32> = Vec::new();
    for &(id, t, o) in glyphs {
        let Outline::Bars { y0, y1, bars: b } = o else {
            continue;
        };
        match height {
            None => height = Some((*y0, *y1)),
            Some((h0, h1)) => {
                let h = h1 - h0;
                if (y0 - h0).abs() > SAME_HEIGHT * h || (y1 - h1).abs() > SAME_HEIGHT * h {
                    return None;
                }
            }
        }
        kinds.push(id);
        for &(x0, x1) in b {
            let (a, z) = (
                along(t * Point::new(x0, *y0)),
                along(t * Point::new(x1, *y0)),
            );
            bars.push((a.min(z), a.max(z)));
        }
    }
    kinds.sort_unstable();
    kinds.dedup();
    bars.sort_by(|a, b| a.0.total_cmp(&b.0));
    let narrowest = bars.iter().map(|b| b.1 - b.0).fold(f64::MAX, f64::min);
    let mut joined: Vec<(f64, f64)> = Vec::with_capacity(bars.len());
    for b in bars {
        match joined.last_mut() {
            Some(last) if b.0 - last.1 < TOUCH * narrowest => {
                last.1 = last.1.max(b.1);
            }
            _ => joined.push(b),
        }
    }
    Some((joined, kinds.len()))
}

/// The widths of bars `joined` and of the spaces between them, in order: bar, space, bar.
fn widths_of(joined: &[(f64, f64)]) -> Vec<f64> {
    let mut widths: Vec<f64> = Vec::with_capacity(2 * joined.len());
    for (k, b) in joined.iter().enumerate() {
        widths.push(b.1 - b.0);
        if let Some(next) = joined.get(k + 1) {
            widths.push(next.0 - b.1);
        }
    }
    widths
}

/// The bar and space widths of each Code 128 symbol in modules, bar first; the stop (106)
/// has 7. From the Code 128 symbology (ISO/IEC 15417), a table of facts.
const CODE128: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

/// The symbol a group of element widths (in points) is: the widths in modules (the group's
/// width over `modules`), each 1 to 4 within [`WHOLE`], looked up in [`CODE128`].
fn symbol(w: &[f64], modules: f64) -> Option<u8> {
    let module = w.iter().sum::<f64>() / modules;
    let mut key = String::with_capacity(w.len());
    for &x in w {
        let n = x / module;
        let r = n.round();
        if !(1.0..=4.0).contains(&r) || (n - r).abs() > WHOLE {
            return None;
        }
        key.push(char::from(b'0' + r as u8));
    }
    CODE128
        .iter()
        .position(|p| *p == key)
        .and_then(|v| u8::try_from(v).ok())
}

/// The text of the Code 128 the bars of a run print, read from the glyphs' geometry (no
/// render): the symbols from the start to the stop, the check symbol right, and only the
/// characters of code sets A, B and C with their switches and shifts. `None` when the bars
/// are no such Code 128 (another symbology, a function character, a damaged run): then the
/// read from the render is checked as a picture's is.
pub(crate) fn code128_text(glyphs: &[(u32, Affine, &Outline)]) -> Option<String> {
    let (joined, _) = run_bars(glyphs)?;
    let w = widths_of(&joined);
    // A start, at least one symbol and the check (6 each), and the stop (7).
    if w.len() < 6 * 3 + 7 || !(w.len() - 7).is_multiple_of(6) {
        return None;
    }
    let k = (w.len() - 7) / 6;
    let mut values: Vec<u8> = Vec::with_capacity(k);
    for i in 0..k {
        values.push(symbol(&w[6 * i..6 * i + 6], 11.0)?);
    }
    if symbol(&w[6 * k..], 13.0)? != 106 {
        return None;
    }
    let (start, rest) = values.split_first()?;
    let (check, data) = rest.split_last()?;
    let sum = data
        .iter()
        .enumerate()
        .fold(u32::from(*start), |s, (i, &v)| {
            s + (i as u32 + 1) * u32::from(v)
        });
    if sum % 103 != u32::from(*check) {
        return None;
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Set {
        A,
        B,
        C,
    }
    let mut set = match start {
        103 => Set::A,
        104 => Set::B,
        105 => Set::C,
        _ => return None,
    };
    let mut text = String::new();
    let mut shift = false;
    for &v in data {
        let now = if shift {
            shift = false;
            if set == Set::A { Set::B } else { Set::A }
        } else {
            set
        };
        match (now, v) {
            (Set::C, 0..=99) => text.push_str(&format!("{v:02}")),
            (Set::C, 100) => set = Set::B,
            (Set::C, 101) => set = Set::A,
            (Set::A | Set::B, 0..=63) => text.push(char::from(v + 32)),
            (Set::A, 64..=95) => text.push(char::from(v - 64)),
            (Set::B, 64..=95) => text.push(char::from(v + 32)),
            (Set::A | Set::B, 98) => shift = true,
            (Set::A | Set::B, 99) => set = Set::C,
            (Set::A, 100) | (Set::B, 101) => set = if set == Set::A { Set::B } else { Set::A },
            _ => return None,
        }
    }
    Some(text)
}

/// One glyph of a barcode font as the page draws it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Placed {
    /// The box of its ink (or of its advance when its outline is not known), in the frame
    /// of the page's pictures.
    pub b: [f32; 4],
    /// Its narrowest bar in points, 0 when its outline is not bars.
    pub module: f32,
    /// The glyph id and the font's cache key, for the code's identity.
    pub glyph: u32,
    pub font: u128,
    /// The draw call it came in: glyphs of one call are one run.
    pub run: u32,
}

/// The box `b` (x0, y0, x1, y1 in font units) mapped by `t`.
fn mapped(t: Affine, b: [f64; 4]) -> [f32; 4] {
    let corners = [
        t * Point::new(b[0], b[1]),
        t * Point::new(b[2], b[1]),
        t * Point::new(b[0], b[3]),
        t * Point::new(b[2], b[3]),
    ];
    let x0 = corners.iter().map(|p| p.x).fold(f64::MAX, f64::min);
    let x1 = corners.iter().map(|p| p.x).fold(f64::MIN, f64::max);
    let y0 = corners.iter().map(|p| p.y).fold(f64::MAX, f64::min);
    let y1 = corners.iter().map(|p| p.y).fold(f64::MIN, f64::max);
    [x0 as f32, y0 as f32, x1 as f32, y1 as f32]
}

/// The box of the ink of `path` in font units, `None` when it has none.
pub(crate) fn ink(path: &BezPath) -> Option<[f64; 4]> {
    let r = path.bounding_box();
    (r.area() > 0.0).then_some([r.x0, r.y0, r.x1, r.y1])
}

/// Where a glyph of a barcode font draws, with transform `t` (font units to the page) and
/// outline `o` (the box of its ink `ink` when it is not bars), its advance `advance` in font
/// units when known: its box and its narrowest bar in points. `None` when it draws nothing.
pub(crate) fn place(
    t: Affine,
    o: &Outline,
    ink: Option<[f64; 4]>,
    advance: Option<f32>,
) -> Option<([f32; 4], f32)> {
    let scale = {
        let c = t.as_coeffs();
        c[0].hypot(c[1])
    };
    match o {
        Outline::Bars { y0, y1, bars } => {
            let x0 = bars.first()?.0;
            let x1 = bars.last()?.1;
            let module = bars.iter().map(|&(a, b)| b - a).fold(f64::MAX, f64::min) * scale;
            Some((mapped(t, [x0, *y0, x1, *y1]), module as f32))
        }
        Outline::Empty => None,
        Outline::Other => {
            let b = match ink {
                Some(b) => b,
                None => [0.0, 0.0, f64::from(advance.filter(|a| *a > 0.0)?), 1000.0],
            };
            Some((mapped(t, b), 0.0))
        }
    }
}

/// The gap between two boxes along each axis (0 when they overlap there).
fn gaps(a: &[f32; 4], b: &[f32; 4]) -> (f32, f32) {
    (
        (a[0] - b[2]).max(b[0] - a[2]).max(0.0),
        (a[1] - b[3]).max(b[1] - a[3]).max(0.0),
    )
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// The codes printed by the glyphs `placed`: the glyphs of one draw call are one run, and
/// runs whose boxes stand within [`JOIN_MODULES`] modules of each other (a point when the
/// module is not known) are one code. Each is a region read like a code drawn as paths.
pub(crate) fn codes(placed: &[Placed], texts: &[(u32, String)]) -> Vec<VectorCode> {
    // The runs: their box, module and members.
    let mut runs: Vec<([f32; 4], f32, Vec<usize>)> = Vec::new();
    for (k, p) in placed.iter().enumerate() {
        match runs.last_mut() {
            Some((b, m, members)) if placed[members[0]].run == p.run => {
                *b = [
                    b[0].min(p.b[0]),
                    b[1].min(p.b[1]),
                    b[2].max(p.b[2]),
                    b[3].max(p.b[3]),
                ];
                if p.module > 0.0 && (*m == 0.0 || p.module < *m) {
                    *m = p.module;
                }
                members.push(k);
            }
            _ => runs.push((p.b, p.module, vec![k])),
        }
    }
    let n = runs.len();
    let mut parent: Vec<usize> = (0..n).collect();
    if n <= MAX_JOINED {
        for i in 0..n {
            for j in i + 1..n {
                let m = match (runs[i].1, runs[j].1) {
                    (0.0, 0.0) => 0.0,
                    (a, 0.0) | (0.0, a) => a,
                    (a, b) => a.min(b),
                };
                let reach = if m > 0.0 { JOIN_MODULES * m } else { 1.0 };
                let (dx, dy) = gaps(&runs[i].0, &runs[j].0);
                if dx <= reach && dy <= reach {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                }
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut root_of: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        let r = find(&mut parent, i);
        match root_of[r] {
            Some(g) => groups[g].push(i),
            None => {
                root_of[r] = Some(groups.len());
                groups.push(vec![i]);
            }
        }
    }
    groups
        .into_iter()
        .map(|g| {
            let mut b = runs[g[0]].0;
            let mut module = 0.0f32;
            for &i in &g {
                let r = &runs[i];
                b = [
                    b[0].min(r.0[0]),
                    b[1].min(r.0[1]),
                    b[2].max(r.0[2]),
                    b[3].max(r.0[3]),
                ];
                if r.1 > 0.0 && (module == 0.0 || r.1 < module) {
                    module = r.1;
                }
            }
            // What it is: its glyphs and their places relative to its corner, so the same
            // code printed on many pages is one.
            let mut h = DefaultHasher::new();
            "barcode font".hash(&mut h);
            for &i in &g {
                for &k in &runs[i].2 {
                    let p = &placed[k];
                    (p.font, p.glyph).hash(&mut h);
                    for (v, at) in p.b.iter().zip([b[0], b[1], b[0], b[1]]) {
                        ((v - at) * 10.0).round().to_bits().hash(&mut h);
                    }
                }
            }
            // The values its glyphs print, when each of its runs is a Code 128 read from them.
            let mut values: Vec<String> = Vec::new();
            let all = g.iter().all(|&i| {
                let run = placed[runs[i].2[0]].run;
                match texts.iter().find(|(r, _)| *r == run) {
                    Some((_, t)) => {
                        values.push(t.clone());
                        true
                    }
                    None => false,
                }
            });
            if !all {
                values.clear();
            }
            VectorCode {
                b,
                module,
                id: h.finish(),
                finders: 0,
                font: true,
                texts: values,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(r: &[[f64; 4]]) -> BezPath {
        let mut p = BezPath::new();
        for &[x0, y0, x1, y1] in r {
            p.move_to((x0, y0));
            p.line_to((x0, y1));
            p.line_to((x1, y1));
            p.line_to((x1, y0));
            p.close_path();
        }
        p
    }

    /// The bars of widths `w` (bar, space, bar, ...) in units of 60, 1600 high.
    fn pattern(w: &[u8]) -> BezPath {
        let mut x = 0.0;
        let mut r = Vec::new();
        for (k, &n) in w.iter().enumerate() {
            let x1 = x + f64::from(n) * 60.0;
            if k % 2 == 0 {
                r.push([x, 0.0, x1, 1600.0]);
            }
            x = x1;
        }
        rects(&r)
    }

    #[test]
    fn barcode_font_names_are_known_in_their_spellings() {
        for name in [
            "code128",
            "code 128",
            "libre barcode 128",
            "librebarcode39-regular",
            "idautomationhc39m",
            "free3of9",
            "ean-13",
            "code_39",
            "barcodefont",
            "bc c128 hd wide",
            "bc#20c128#20hd#20wide",
            "my-c93",
            "i2of5 std",
            "font i25",
            "ean8",
            "upc-a",
            "upc a",
            "shop upce",
            "interleaved 2of5",
        ] {
            assert!(barcode_name(name), "{name}");
        }
        for name in [
            "helvetica",
            "arial-boldmt",
            "symbol",
            "times-roman",
            "codesans",
            "arc128",
            "c1280",
            "upcase sans",
            "cupcake",
            "sec39ure",
        ] {
            assert!(!barcode_name(name), "{name}");
        }
    }

    #[test]
    fn a_glyph_of_upright_bars_is_bars_and_letters_are_not() {
        assert!(matches!(
            outline(&pattern(&[2, 1, 2, 2, 2, 2])),
            Outline::Bars { .. }
        ));
        assert_eq!(outline(&BezPath::new()), Outline::Empty);
        // One bar is a bar: a font may draw one bar a glyph (whether a run is a code is the
        // run's question).
        assert!(matches!(
            outline(&rects(&[[0.0, 0.0, 80.0, 700.0]])),
            Outline::Bars { .. }
        ));
        // A bar whose contour has a point in the middle of a side, or straight curves.
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((0.0, 300.0));
        p.line_to((0.0, 700.0));
        p.quad_to((40.0, 700.0), (80.0, 700.0));
        p.line_to((80.0, 0.0));
        p.close_path();
        assert!(
            matches!(outline(&p), Outline::Bars { .. }),
            "{:?}",
            outline(&p)
        );
        // Two bars of different heights, or lying bars (an `=`), are not.
        assert_eq!(
            outline(&rects(&[
                [0.0, 0.0, 80.0, 700.0],
                [200.0, 0.0, 280.0, 500.0]
            ])),
            Outline::Other
        );
        assert_eq!(
            outline(&rects(&[
                [0.0, 200.0, 500.0, 260.0],
                [0.0, 400.0, 500.0, 460.0]
            ])),
            Outline::Other
        );
        // A curve is not, nor an L.
        let mut c = pattern(&[2, 1, 2]);
        c.curve_to((0.0, 0.0), (10.0, 10.0), (20.0, 0.0));
        assert_eq!(outline(&c), Outline::Other);
        let mut l = BezPath::new();
        l.move_to((0.0, 0.0));
        l.line_to((0.0, 700.0));
        l.line_to((80.0, 700.0));
        l.line_to((80.0, 80.0));
        l.line_to((300.0, 80.0));
        l.line_to((300.0, 0.0));
        l.close_path();
        assert_eq!(outline(&l), Outline::Other);
    }

    /// The glyphs of the bar and space widths `w` (in modules of 60 units), one bar and its
    /// space a glyph, each with its id and transform at 12 points, set one after the other.
    fn one_bar_glyphs(w: &[(u8, u8)]) -> Vec<(u32, Affine, Outline)> {
        let mut x = 0.0;
        w.iter()
            .map(|&(b, s)| {
                let o = outline(&rects(&[[0.0, 0.0, f64::from(b) * 60.0, 1600.0]]));
                let t = Affine::new([0.012, 0.0, 0.0, 0.012, 100.0 + x, 500.0]);
                x += f64::from(b + s) * 60.0 * 0.012;
                (u32::from(b * 4 + s), t, o)
            })
            .collect()
    }

    #[test]
    fn a_run_is_a_code_only_with_bars_of_whole_modules() {
        let fit = |g: &[(u32, Affine, Outline)]| {
            let r: Vec<(u32, Affine, &Outline)> = g.iter().map(|(i, t, o)| (*i, *t, o)).collect();
            bars_fit(&r)
        };
        // Code 128 `A1` as one bar and its space a glyph: start B, A, 1, check, stop.
        let pairs = [
            (2, 1),
            (1, 2),
            (1, 4),
            (1, 1),
            (1, 3),
            (2, 3),
            (1, 2),
            (3, 3),
            (2, 1),
            (1, 3),
            (2, 2),
            (1, 1),
            (2, 3),
            (3, 1),
            (1, 1),
            (2, 0),
        ];
        assert!(fit(&one_bar_glyphs(&pairs)));
        // Bars that touch across glyphs are one bar.
        let mut touching = one_bar_glyphs(&pairs);
        let split = touching.remove(3);
        let half = outline(&rects(&[[0.0, 0.0, 30.0, 1600.0]]));
        let at = split.1;
        touching.insert(3, (99, at, half.clone()));
        touching.insert(4, (98, at * Affine::translate((30.0, 0.0)), half));
        assert!(fit(&touching));
        // Too few bars, one glyph only, equal bars (a ruler), widths that are no whole
        // modules, or a glyph that is not bars among them.
        assert!(!fit(&one_bar_glyphs(&pairs[..8])));
        assert!(!fit(&one_bar_glyphs(&[(1, 1); 20])));
        assert!(!fit(&one_bar_glyphs(&[(1, 1), (2, 2)].repeat(10))));
        let mut odd = one_bar_glyphs(&pairs);
        odd[5].2 = outline(&rects(&[[0.0, 0.0, 150.0, 1600.0]]));
        assert!(!fit(&odd));
        let mut other = one_bar_glyphs(&pairs);
        other[2].2 = Outline::Other;
        assert!(!fit(&other));
        // The 11-module glyphs of a Code 128 font.
        let a = outline(&pattern(&[2, 1, 1, 2, 1, 4]));
        let b = outline(&pattern(&[2, 1, 2, 2, 2, 2]));
        let c = outline(&pattern(&[2, 3, 3, 1, 1, 1, 2]));
        let t = |k: f64| Affine::new([0.012, 0.0, 0.0, 0.012, 100.0 + k * 660.0 * 0.012, 500.0]);
        assert!(bars_fit(&[
            (1, t(0.0), &a),
            (2, t(1.0), &b),
            (2, t(2.0), &b),
            (3, t(3.0), &c)
        ]));
        assert!(!bars_fit(&[
            (1, t(0.0), &a),
            (1, t(1.0), &a),
            (1, t(2.0), &a)
        ]));
    }

    /// The glyphs of a Code 128 of symbol values `values`, one bar and its space a glyph
    /// (bars 60 units a module), set one after the other at 12 points.
    fn code_glyphs(values: &[u8]) -> Vec<(u32, Affine, Outline)> {
        let mut pairs = Vec::new();
        for &v in values {
            let w: Vec<u8> = CODE128[usize::from(v)].bytes().map(|b| b - b'0').collect();
            for p in w.chunks(2) {
                pairs.push((p[0], p.get(1).copied().unwrap_or(0)));
            }
        }
        one_bar_glyphs(&pairs)
    }

    #[test]
    fn the_value_a_run_of_bars_prints_is_read_from_its_geometry() {
        let read = |g: &[(u32, Affine, Outline)]| {
            let r: Vec<(u32, Affine, &Outline)> = g.iter().map(|(i, t, o)| (*i, *t, o)).collect();
            code128_text(&r)
        };
        // `A1` in code set B: start B, 33, 17, the check (104 + 33 + 2 * 17) % 103 = 68, stop.
        assert_eq!(
            read(&code_glyphs(&[104, 33, 17, 68, 106])).as_deref(),
            Some("A1")
        );
        // `2026004711` in code set C.
        let c = [105u8, 20, 26, 0, 47, 11];
        let check = (u32::from(c[0])
            + c[1..]
                .iter()
                .enumerate()
                .map(|(i, &v)| (i as u32 + 1) * u32::from(v))
                .sum::<u32>())
            % 103;
        let mut v = c.to_vec();
        v.extend([check as u8, 106]);
        assert_eq!(read(&code_glyphs(&v)).as_deref(), Some("2026004711"));
        // A wrong check symbol, a missing stop, or a function character give none.
        assert_eq!(read(&code_glyphs(&[104, 33, 17, 69, 106])), None);
        assert_eq!(read(&code_glyphs(&[104, 33, 17, 68])), None);
        assert_eq!(read(&code_glyphs(&[104, 102, 17, 34, 106])), None);
    }
}
