// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Barcodes printed with a barcode font: the bars are glyphs of a font, and the text of those
//! glyphs (`Í%"$"*%!)Î`) is no text of the page. A run is taken for such a code only on
//! positive evidence of its shape: its glyphs, as the page shows them, are upright bars of one
//! height whose bars and spaces fit a code. A font's name never takes a run on its own; a name
//! that says the font is a barcode font only lowers what the shape must show. Odd characters
//! alone are never evidence. The bars' boxes become regions of the page that are rendered
//! again and read like codes drawn as paths ([`crate::codes::VectorCode`]).

use std::hash::{DefaultHasher, Hash, Hasher};

use kurbo::{Affine, BezPath, PathEl, Point};

use crate::codes::VectorCode;

/// Words of a font's base name that say it is a barcode font: a symbology, long or short.
/// They count only as whole words (or two words in a row, so `Code 128` and `UPC-A` count),
/// never inside a longer word (`Ocean13`, `BarcodeSans`). A vendor's name is no evidence: the
/// same vendors sell OCR and MICR text fonts.
const WORDS: [&str; 17] = [
    "barcode",
    "code128",
    "code39",
    "code93",
    "3of9",
    "2of5",
    "i2of5",
    "interleaved2of5",
    "itf",
    "ean13",
    "ean8",
    "upca",
    "upce",
    "c128",
    "c39",
    "c93",
    "i25",
];

/// Whether a font's base name (lowercase, subset prefix removed) says it is a barcode font
/// (see [`WORDS`]). Words are cut at anything but a letter or digit.
pub(crate) fn barcode_name(name: &str) -> bool {
    let name = name.replace("#20", " ");
    let words: Vec<&str> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let known = |w: &str| WORDS.contains(&w);
    words.iter().any(|w| known(w))
        || words
            .windows(2)
            .any(|p| known(&format!("{}{}", p[0], p[1])))
}

/// The widest bar or space of a code of whole modules, in modules: Code 128 and EAN have bars
/// of up to 4.
const MOST_MODULES: f64 = 4.0;

/// How far a width may lie from a whole number of modules, in modules.
const WHOLE: f64 = 0.2;

/// The share of a bar's height by which the bottoms and tops of the bars of one font may
/// differ.
const SAME_HEIGHT: f64 = 0.02;

/// Bars a run shows at least (bars of glyphs that touch are one) and different glyphs among
/// them, to be a code by its outlines: the shortest Code 128 (a start, one symbol, the check
/// and the stop) has 13 bars. A font whose name says it is a barcode font needs fewer.
const MIN_BARS: usize = 12;
const MIN_KINDS: usize = 2;
const NAMED_MIN_BARS: usize = 4;

/// Different widths of bars and spaces a code of whole modules shows at least: Code 128, EAN
/// and UPC have 1 to 4 modules. Codes of two widths (Code 39, ITF) must fit their structure.
const MIN_WIDTHS: usize = 3;

/// A code of two widths: the wide one is this many times the narrow one, from to.
const WIDE_FROM: f64 = 1.8;
const WIDE_TO: f64 = 3.4;

/// How far a narrow or wide element may lie from its class's mean, as a share of it.
const CLASS_SPREAD: f64 = 0.25;

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
    /// right, left to right, each at least twice as high as the widest. Other ink may stand
    /// only under the bars (the digits of a human-readable barcode font); the bars are at
    /// least half the glyph's height. A font may draw one bar a glyph, so one bar is a bar
    /// here; whether a run of them is a code is the run's question ([`bars_fit`]).
    Bars {
        y0: f64,
        y1: f64,
        bars: Vec<(f64, f64)>,
    },
    /// Anything else: letters, curves, slanted lines, bars of different heights, squares.
    Other,
}

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

/// One subpath of a glyph's outline: its points when every side is straight, and its box
/// (control points too).
struct Sub {
    points: Vec<Point>,
    curved: bool,
    lo: Point,
    hi: Point,
}

impl Sub {
    fn new(p: Point) -> Sub {
        Sub {
            points: vec![p],
            curved: false,
            lo: p,
            hi: p,
        }
    }

    fn reach(&mut self, p: Point) {
        self.lo = Point::new(self.lo.x.min(p.x), self.lo.y.min(p.y));
        self.hi = Point::new(self.hi.x.max(p.x), self.hi.y.max(p.y));
    }
}

/// Subpaths of a glyph looked at, at most.
const MAX_SUBPATHS: usize = 256;

/// What the outline `path` of a glyph is (see [`Outline`]). It is read the same for every
/// font program (TrueType, Type1, CFF): a curve whose control points lie on a straight side
/// along an axis is that side.
pub(crate) fn outline(path: &BezPath) -> Outline {
    let mut subs: Vec<Sub> = Vec::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if subs.len() == MAX_SUBPATHS {
                    return Outline::Other;
                }
                subs.push(Sub::new(p));
            }
            PathEl::LineTo(p) => {
                let Some(s) = subs.last_mut() else {
                    return Outline::Other;
                };
                s.reach(p);
                if s.points.len() < 18 {
                    s.points.push(p);
                } else {
                    s.curved = true;
                }
            }
            PathEl::QuadTo(c, p) | PathEl::CurveTo(_, c, p) => {
                let Some(s) = subs.last_mut() else {
                    return Outline::Other;
                };
                let controls: Vec<Point> = match *el {
                    PathEl::CurveTo(c1, c2, _) => vec![c1, c2],
                    _ => vec![c],
                };
                for &q in &controls {
                    s.reach(q);
                }
                s.reach(p);
                match s.points.last() {
                    Some(&a) if straight(a, p, &controls) && s.points.len() < 18 => {
                        s.points.push(p)
                    }
                    _ => s.curved = true,
                }
            }
            PathEl::ClosePath => {}
        }
    }
    let subs: Vec<Sub> = subs.into_iter().filter(|s| s.hi != s.lo).collect();
    if subs.is_empty() {
        return Outline::Empty;
    }
    let rects: Vec<Option<[f64; 4]>> = subs
        .iter()
        .map(|s| {
            (!s.curved)
                .then(|| upright_rect(&s.points))
                .flatten()
                .filter(|r| r[3] - r[1] >= 2.0 * (r[2] - r[0]))
        })
        .collect();
    // The bars: the tallest upright rectangle and those level with it.
    let Some(tallest) = rects
        .iter()
        .flatten()
        .max_by(|a, b| (a[3] - a[1]).total_cmp(&(b[3] - b[1])))
        .copied()
    else {
        return Outline::Other;
    };
    let (y0, y1) = (tallest[1], tallest[3]);
    let height = y1 - y0;
    let level = |r: &[f64; 4]| {
        (r[1] - y0).abs() <= SAME_HEIGHT * height && (r[3] - y1).abs() <= SAME_HEIGHT * height
    };
    let mut bars: Vec<[f64; 4]> = rects
        .iter()
        .flatten()
        .filter(|r| level(r))
        .copied()
        .collect();
    // Anything else stands under the bars, and the bars are most of the glyph.
    let mut bottom = y0;
    for (s, r) in subs.iter().zip(&rects) {
        if r.is_some_and(|r| level(&r)) {
            continue;
        }
        if s.hi.y > y0 + SAME_HEIGHT * height {
            return Outline::Other;
        }
        bottom = bottom.min(s.lo.y);
    }
    if height < 0.5 * (y1 - bottom) {
        return Outline::Other;
    }
    bars.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let widest = bars.iter().map(|r| r[2] - r[0]).fold(0.0, f64::max);
    if !bars.windows(2).all(|w| w[1][0] >= w[0][2]) || height < 2.0 * widest {
        return Outline::Other;
    }
    Outline::Bars {
        y0,
        y1,
        bars: bars.iter().map(|r| (r[0], r[2])).collect(),
    }
}

/// Whether the outlines of a run's glyphs (each with its glyph id and its transform, font
/// units to the page) are a code, on the run's bars as the page shows them: no glyph is
/// anything but bars or a space; all bars of one height; bars of glyphs that touch joined
/// into one; at least [`MIN_BARS`] bars, from [`MIN_KINDS`] different glyphs; and either every
/// bar and every space between two bars a whole number of modules (the narrowest of them), at
/// most [`MOST_MODULES`], with at least [`MIN_WIDTHS`] different widths (Code 128, EAN, UPC),
/// or two widths in the structure of Code 39 or ITF ([`two_widths`]). A row of equal bars (a
/// ruler, `||||`) or of one glyph is no code; nor are letters, which are not bars.
///
/// With `named` (the font's name says it is a barcode font) the shape needs less: at least
/// [`NAMED_MIN_BARS`] bars, of one glyph or more, and whole modules of any number of widths,
/// or two widths in any order. The glyphs must still be bars.
#[cfg(test)]
pub(crate) fn bars_fit(glyphs: &[(u32, Affine, &Outline)], named: bool) -> bool {
    !code_parts(glyphs, named).is_empty()
}

/// A part of a run of bars that is a code (see [`bars_fit`]): from the left of its first bar
/// to the right of its last, in points along the baseline of the run's first glyph (as
/// [`glyph_spans`] gives them), and the Code 128 value its bars print, when they print one.
pub(crate) struct Part {
    pub from: f64,
    pub to: f64,
    pub text: Option<String>,
}

/// The parts of a run of bars that are codes. A run may hold a quiet zone of white glyphs
/// and more bars past it: each part between wide spaces ([`segments`]) is looked at on its
/// own.
pub(crate) fn code_parts(glyphs: &[(u32, Affine, &Outline)], named: bool) -> Vec<Part> {
    let Some((joined, kinds)) = run_bars(glyphs) else {
        return Vec::new();
    };
    let (min_bars, min_kinds) = if named {
        (NAMED_MIN_BARS, 1)
    } else {
        (MIN_BARS, MIN_KINDS)
    };
    if kinds < min_kinds {
        return Vec::new();
    }
    let fits = |part: &[(f64, f64)]| {
        if part.len() < min_bars {
            return false;
        }
        let widths = widths_of(part);
        if let Some(modules) = whole_modules(&widths) {
            let mut seen: Vec<u8> = modules.clone();
            seen.sort_unstable();
            seen.dedup();
            if seen.len() >= MIN_WIDTHS || (named && seen.len() >= 2) {
                return true;
            }
        }
        match two_widths(&widths) {
            Some(wide) => named || code39(&wide) || itf(&wide),
            None => false,
        }
    };
    segments(&joined)
        .into_iter()
        .filter(|part| fits(part))
        .map(|part| Part {
            from: part[0].0,
            to: part[part.len() - 1].1,
            text: code128_of(&widths_of(part)),
        })
        .collect()
}

/// Where the bars of each glyph of a run stand, in points along the baseline of its first
/// glyph: from the left of its first bar to the right of its last, `None` for a space.
pub(crate) fn glyph_spans(glyphs: &[(u32, Affine, &Outline)]) -> Vec<Option<(f64, f64)>> {
    let Some(along) = glyphs.first().and_then(|g| baseline(g.1)) else {
        return vec![None; glyphs.len()];
    };
    glyphs
        .iter()
        .map(|&(_, t, o)| {
            let Outline::Bars { y0, bars, .. } = o else {
                return None;
            };
            let (x0, x1) = (bars.first()?.0, bars.last()?.1);
            let (a, z) = (
                along(t * Point::new(x0, *y0)),
                along(t * Point::new(x1, *y0)),
            );
            Some((a.min(z), a.max(z)))
        })
        .collect()
}

/// The place of a point along the baseline of a glyph drawn with `first`, in points from its
/// origin. `None` for a transform that draws nothing.
fn baseline(first: Affine) -> Option<impl Fn(Point) -> f64> {
    let c = first.as_coeffs();
    let len = c[0].hypot(c[1]);
    if len <= 0.0 {
        return None;
    }
    let (dx, dy) = (c[0] / len, c[1] / len);
    let origin = Point::new(c[4], c[5]);
    Some(move |p: Point| (p.x - origin.x) * dx + (p.y - origin.y) * dy)
}

/// A space of a code is at most [`MOST_MODULES`]: a space wider than this many times the
/// run's narrowest element (a quiet zone) ends a part of the run ([`segments`]).
const QUIET: f64 = 6.0;

/// The parts of bars `joined` between spaces wider than [`QUIET`] times the narrowest bar or
/// space of the run.
fn segments(joined: &[(f64, f64)]) -> Vec<&[(f64, f64)]> {
    let narrowest = widths_of(joined).into_iter().fold(f64::MAX, f64::min);
    let mut out = Vec::new();
    let mut from = 0;
    for k in 1..joined.len() {
        if joined[k].0 - joined[k - 1].1 > QUIET * narrowest {
            out.push(&joined[from..k]);
            from = k;
        }
    }
    if from < joined.len() {
        out.push(&joined[from..]);
    }
    out
}

/// How far the sum of a bar and the space after it may lie from a whole number of modules.
const PAIR_WHOLE: f64 = 0.15;

/// The widths `w` (bar, space, ..., bar) in whole modules of 1 to [`MOST_MODULES`], when they
/// are: the module from the sums of each bar and the space after it (whole numbers of modules
/// whatever the spread of the ink, which makes bars wider and spaces narrower by the same),
/// the largest that fits them all; then the spread, and each bar less it and each space with
/// it within [`WHOLE`] of a whole module. `None` otherwise.
fn whole_modules(w: &[f64]) -> Option<Vec<u8>> {
    let sums: Vec<f64> = w
        .chunks(2)
        .filter(|p| p.len() == 2)
        .map(|p| p[0] + p[1])
        .collect();
    let least = sums.iter().copied().fold(f64::MAX, f64::min);
    if sums.is_empty() || least <= 0.0 || !least.is_finite() {
        return None;
    }
    // The least sum is 2 to 8 modules.
    let module = (2..=8).map(|k| least / f64::from(k)).find(|&m| {
        sums.iter().all(|&x| {
            let n = x / m;
            (n - n.round()).abs() <= PAIR_WHOLE
        })
    })?;
    // The spread: what the bars hold past whole modules, on average.
    let bars: Vec<f64> = w.iter().step_by(2).copied().collect();
    let spread = bars
        .iter()
        .map(|&b| b - (b / module).round().max(1.0) * module)
        .sum::<f64>()
        / bars.len() as f64;
    w.iter()
        .enumerate()
        .map(|(k, &x)| {
            let x = if k % 2 == 0 { x - spread } else { x + spread };
            let n = x / module;
            let r = n.round();
            ((1.0..=MOST_MODULES).contains(&r) && (n - r).abs() <= WHOLE).then_some(r as u8)
        })
        .collect()
}

/// The widths `w` as narrow (`false`) and wide (`true`) when they are of two widths only: the
/// wide [`WIDE_FROM`] to [`WIDE_TO`] times the narrow, each within [`CLASS_SPREAD`] of its
/// class's mean. `None` otherwise (one width, three or more).
fn two_widths(w: &[f64]) -> Option<Vec<bool>> {
    let lo = w.iter().copied().fold(f64::MAX, f64::min);
    let hi = w.iter().copied().fold(f64::MIN, f64::max);
    if lo <= 0.0 || !lo.is_finite() || hi < WIDE_FROM * lo * (1.0 - CLASS_SPREAD) {
        return None;
    }
    let cut = (lo + hi) / 2.0;
    let wide: Vec<bool> = w.iter().map(|&x| x > cut).collect();
    let mean = |want: bool| {
        let v: Vec<f64> = w
            .iter()
            .zip(&wide)
            .filter(|(_, k)| **k == want)
            .map(|(x, _)| *x)
            .collect();
        (v.iter().sum::<f64>() / v.len() as f64, v)
    };
    let (narrow, nv) = mean(false);
    let (broad, bv) = mean(true);
    let near = |v: &[f64], m: f64| v.iter().all(|x| (x - m).abs() <= CLASS_SPREAD * m);
    let ratio = broad / narrow;
    (near(&nv, narrow) && near(&bv, broad) && (WIDE_FROM..=WIDE_TO).contains(&ratio))
        .then_some(wide)
}

/// Whether elements `wide` (bar, space, ...; narrow or wide) have the structure of Code 39:
/// characters of 9 elements with exactly 3 wide, a narrow gap between characters, at least 3
/// characters (the start and stop and one more).
fn code39(wide: &[bool]) -> bool {
    let n = wide.len() + 1;
    if !n.is_multiple_of(10) || n / 10 < 3 {
        return false;
    }
    wide.chunks(10)
        .all(|c| c[..9].iter().filter(|w| **w).count() == 3 && c.get(9).is_none_or(|g| !g))
}

/// Whether elements `wide` have the structure of ITF: a start of four narrow elements, pairs
/// of characters of 10 elements each (5 bars and 5 spaces, two of each wide), and a stop of a
/// wide bar, a narrow space and a narrow bar.
fn itf(wide: &[bool]) -> bool {
    let n = wide.len();
    if n < 17 || !(n - 7).is_multiple_of(10) {
        return false;
    }
    let start = wide[..4].iter().all(|w| !w);
    let stop = wide[n - 3..] == [true, false, false];
    let pairs = wide[4..n - 3].chunks(10).all(|c| {
        c.iter().step_by(2).filter(|w| **w).count() == 2
            && c.iter().skip(1).step_by(2).filter(|w| **w).count() == 2
    });
    start && stop && pairs
}

/// The bars of a run as the page shows them, in points along the baseline of its first glyph
/// (bars of glyphs that touch joined into one), and how many different glyphs drew them.
/// `None` when a glyph is not bars or a space, or the bars differ in height.
fn run_bars(glyphs: &[(u32, Affine, &Outline)]) -> Option<(Vec<(f64, f64)>, usize)> {
    if glyphs.iter().any(|(_, _, o)| matches!(o, Outline::Other)) {
        return None;
    }
    let &(_, first, _) = glyphs.first()?;
    let along = baseline(first)?;
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
    // The part of the run between quiet zones that is a Code 128, if one is.
    segments(&joined)
        .into_iter()
        .find_map(|part| code128_of(&widths_of(part)))
}

/// The text of the Code 128 whose bar and space widths are `w`, if they are one.
fn code128_of(w: &[f64]) -> Option<String> {
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

/// Where a glyph of bars draws, with transform `t` (font units to the page) and outline `o`:
/// the box of its bars and its narrowest bar in points. `None` when it draws no bars.
pub(crate) fn place(t: Affine, o: &Outline) -> Option<([f32; 4], f32)> {
    let c = t.as_coeffs();
    let scale = c[0].hypot(c[1]);
    let Outline::Bars { y0, y1, bars } = o else {
        return None;
    };
    let x0 = bars.first()?.0;
    let x1 = bars.last()?.1;
    let module = bars.iter().map(|&(a, b)| b - a).fold(f64::MAX, f64::min) * scale;
    Some((mapped(t, [x0, *y0, x1, *y1]), module as f32))
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
    fn barcode_font_names_are_known_by_whole_words() {
        for name in [
            "code128",
            "code 128",
            "libre barcode 128",
            "ean-13",
            "code_39",
            "bc c128 hd wide",
            "bc#20c128#20hd#20wide",
            "my-c93",
            "mono-c128",
            "i2of5 std",
            "font i25",
            "ean8",
            "upc-a",
            "upc a",
            "shop upce",
            "interleaved 2of5",
            "itf-14",
        ] {
            assert!(barcode_name(name), "{name}");
        }
        // Inside a longer word, a vendor, or a text font: no.
        for name in [
            "helvetica",
            "arial-boldmt",
            "symbol",
            "codesans",
            "arc128",
            "c1280",
            "upcase sans",
            "cupcake",
            "sec39ure",
            "ocean13",
            "bean 13",
            "barcodesans",
            "monoc128",
            "idautomationocrb",
            "idautomationmicr",
            "idautomationhc39m",
            "librebarcode39-regular",
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
        // A human-readable bar font: bars with digits under them (squares here) are bars.
        assert!(matches!(
            outline(&rects(&[
                [0.0, 200.0, 60.0, 1000.0],
                [120.0, 200.0, 240.0, 1000.0],
                [10.0, 0.0, 90.0, 150.0],
                [130.0, 0.0, 200.0, 150.0],
            ])),
            Outline::Bars { bars, .. } if bars.len() == 2
        ));
        // Ink over the bars or beside them at their height (an `i`, an OCR letter): no.
        assert_eq!(
            outline(&rects(&[
                [0.0, 0.0, 80.0, 500.0],
                [0.0, 600.0, 80.0, 700.0]
            ])),
            Outline::Other
        );
        assert_eq!(
            outline(&rects(&[
                [0.0, 0.0, 80.0, 700.0],
                [100.0, 300.0, 400.0, 400.0]
            ])),
            Outline::Other
        );
        // A letter as one block: no.
        assert_eq!(
            outline(&rects(&[[50.0, 0.0, 550.0, 700.0]])),
            Outline::Other
        );
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
            bars_fit(&r, false)
        };
        let named = |g: &[(u32, Affine, Outline)]| {
            let r: Vec<(u32, Affine, &Outline)> = g.iter().map(|(i, t, o)| (*i, *t, o)).collect();
            bars_fit(&r, true)
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
        // A name lowers the bar: a short run, or two widths in any order, but bars still.
        assert!(named(&one_bar_glyphs(&pairs[..5])));
        assert!(named(&one_bar_glyphs(&[(1, 1), (2, 2)].repeat(3))));
        assert!(!named(&one_bar_glyphs(&[(1, 1); 20])));
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
        assert!(bars_fit(
            &[
                (1, t(0.0), &a),
                (2, t(1.0), &b),
                (2, t(2.0), &b),
                (3, t(3.0), &c)
            ],
            false
        ));
        assert!(!bars_fit(
            &[(1, t(0.0), &a), (1, t(1.0), &a), (1, t(2.0), &a)],
            false
        ));
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

    /// The elements (narrow 1, wide `w`) of a Code 39 of characters `chars` (each a pattern
    /// of 9 with `W` wide), a narrow gap between them, as glyphs of one bar and its space.
    fn code39_glyphs(chars: &[&str], w: u8) -> Vec<(u32, Affine, Outline)> {
        let mut e: Vec<u8> = Vec::new();
        for (k, c) in chars.iter().enumerate() {
            if k > 0 {
                e.push(1);
            }
            e.extend(c.bytes().map(|b| if b == b'W' { w } else { 1 }));
        }
        e.push(0);
        one_bar_glyphs(&e.chunks(2).map(|p| (p[0], p[1])).collect::<Vec<_>>())
    }

    /// The elements of an ITF of `pairs` pairs of the characters 1 and 2, wide `w`.
    fn itf_glyphs(pairs: usize, w: u8) -> Vec<(u32, Affine, Outline)> {
        let mut e: Vec<u8> = vec![1, 1, 1, 1];
        let (b, sp) = ("WnnnW", "nWnnW");
        for _ in 0..pairs {
            for (x, y) in b.bytes().zip(sp.bytes()) {
                e.push(if x == b'W' { w } else { 1 });
                e.push(if y == b'W' { w } else { 1 });
            }
        }
        e.extend([w, 1, 1, 0]);
        one_bar_glyphs(&e.chunks(2).map(|p| (p[0], p[1])).collect::<Vec<_>>())
    }

    #[test]
    fn codes_of_two_widths_are_taken_by_their_structure() {
        let fit = |g: &[(u32, Affine, Outline)]| {
            let r: Vec<(u32, Affine, &Outline)> = g.iter().map(|(i, t, o)| (*i, *t, o)).collect();
            bars_fit(&r, false)
        };
        // `*A1*` in Code 39: * = nWnnWnWnn, A = WnnnnWnnW, 1 = WnnWnnnnW.
        let star = "nWnnWnWnn";
        let chars = [star, "WnnnnWnnW", "WnnWnnnnW", star];
        assert!(fit(&code39_glyphs(&chars, 2)));
        assert!(fit(&code39_glyphs(&chars, 3)));
        // Two widths out of the structure (four wide in a character): no.
        assert!(!fit(&code39_glyphs(
            &[star, "WWnnnWnnW", "WnnWnnnnW", star],
            2
        )));
        // ITF: one pair has 9 bars (too few for a run known only by its bars), two pairs 14.
        assert!(!fit(&itf_glyphs(1, 3)));
        assert!(fit(&itf_glyphs(2, 3)));
        assert!(fit(&itf_glyphs(2, 2)));
        // A ruler of two widths in a plain rhythm: no.
        assert!(!fit(&one_bar_glyphs(
            &[(1, 1), (1, 1), (1, 1), (1, 1), (2, 1)].repeat(5)
        )));
    }

    /// [`one_bar_glyphs`] with each bar `spread` units wider (its space as much narrower), and
    /// after the code a quiet zone of `quiet` modules and a lone bar.
    fn spread_glyphs(w: &[(u8, u8)], spread: f64, quiet: u8) -> Vec<(u32, Affine, Outline)> {
        let mut out = one_bar_glyphs(w);
        for g in &mut out {
            if let Outline::Bars { bars, .. } = &mut g.2 {
                bars[0].1 += spread;
            }
        }
        if quiet > 0 {
            let width: f64 = w.iter().map(|&(b, s)| f64::from(b + s) * 60.0).sum();
            let x = (width + f64::from(quiet) * 60.0) * 0.012;
            let o = outline(&rects(&[[0.0, 0.0, 120.0, 1600.0]]));
            out.push((
                77,
                Affine::new([0.012, 0.0, 0.0, 0.012, 100.0 + x, 500.0]),
                o,
            ));
        }
        out
    }

    #[test]
    fn ink_spread_and_a_quiet_zone_do_not_hide_a_code() {
        // Bars a tenth of a module wider than whole modules (spaces as much narrower), as a
        // real font draws them, and a lone bar past a quiet zone of 12 modules in the same
        // run: the module comes from bar and space together, and the part before the quiet
        // zone is the code.
        let fit = |g: &[(u32, Affine, Outline)]| {
            let r: Vec<(u32, Affine, &Outline)> = g.iter().map(|(i, t, o)| (*i, *t, o)).collect();
            bars_fit(&r, false)
        };
        let read = |g: &[(u32, Affine, Outline)]| {
            let r: Vec<(u32, Affine, &Outline)> = g.iter().map(|(i, t, o)| (*i, *t, o)).collect();
            code128_text(&r)
        };
        let mut pairs = Vec::new();
        for v in [104u8, 33, 17, 68, 106] {
            let w: Vec<u8> = CODE128[usize::from(v)].bytes().map(|b| b - b'0').collect();
            for p in w.chunks(2) {
                pairs.push((p[0], p.get(1).copied().unwrap_or(0)));
            }
        }
        for spread in [0.0, 6.0, -6.0] {
            for quiet in [0, 12] {
                let g = spread_glyphs(&pairs, spread, quiet);
                assert!(fit(&g), "spread {spread}, quiet {quiet}");
                assert_eq!(
                    read(&g).as_deref(),
                    Some("A1"),
                    "spread {spread}, quiet {quiet}"
                );
            }
        }
    }
}
