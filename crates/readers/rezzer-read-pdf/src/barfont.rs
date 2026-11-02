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
const NAMES: [&str; 7] = [
    "code128",
    "code39",
    "code93",
    "3of9",
    "ean13",
    "barcode",
    "idautomation",
];

/// Whether a font's base name (lowercase, subset prefix removed) names a barcode font.
pub(crate) fn barcode_name(name: &str) -> bool {
    let squeezed: String = name
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .collect();
    NAMES.iter().any(|n| squeezed.contains(n))
}

/// The widest bar or space of a code, in modules: Code 128 and EAN have bars of up to 4.
const MOST_MODULES: f64 = 4.0;

/// How far a width may lie from a whole number of modules, in modules.
const WHOLE: f64 = 0.2;

/// The share of a bar's height by which the bottoms and tops of the bars of one font may
/// differ.
const SAME_HEIGHT: f64 = 0.02;

/// Glyphs of bars a run has at least, and different glyphs among them, to be a code by its
/// outlines: a code has a start, symbols and a stop.
const MIN_GLYPHS: usize = 3;
const MIN_KINDS: usize = 2;

/// The bars of a run may stand this many modules apart (each way) and still be one code.
const JOIN_MODULES: f32 = 3.0;

/// Runs of a page joined into codes by comparing each pair, at most; past them each run is a
/// region of its own.
const MAX_JOINED: usize = 2000;

/// What the outline of a glyph is, in font units (1000 to the em).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outline {
    /// Nothing is drawn (a space).
    Empty,
    /// Two or more upright bars side by side from `y0` to `y1`, each from its left to its
    /// right, left to right.
    Bars {
        y0: f64,
        y1: f64,
        bars: Vec<(f64, f64)>,
    },
    /// Anything else: curves, slanted lines, a single bar, bars of different heights.
    Other,
}

/// The rectangle a subpath of `points` draws, when it has four corners and sides along the
/// axes (a closing point on the first is left out).
fn upright_rect(points: &[Point]) -> Option<[f64; 4]> {
    let mut p: Vec<Point> = Vec::with_capacity(5);
    for &q in points {
        if p.last() != Some(&q) {
            p.push(q);
        }
    }
    if p.len() == 5 && p[0] == p[4] {
        p.pop();
    }
    if p.len() != 4 {
        return None;
    }
    for k in 0..4 {
        let (a, b) = (p[k], p[(k + 1) % 4]);
        if a.x != b.x && a.y != b.y {
            return None;
        }
    }
    let x0 = p.iter().map(|q| q.x).fold(f64::MAX, f64::min);
    let x1 = p.iter().map(|q| q.x).fold(f64::MIN, f64::max);
    let y0 = p.iter().map(|q| q.y).fold(f64::MAX, f64::min);
    let y1 = p.iter().map(|q| q.y).fold(f64::MIN, f64::max);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

/// What the outline `path` of a glyph is (see [`Outline`]).
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
            PathEl::ClosePath => {
                if !end(&mut points, &mut rects) {
                    return Outline::Other;
                }
            }
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => return Outline::Other,
        }
        if rects.len() > 64 || points.len() > 6 {
            return Outline::Other;
        }
    }
    if !end(&mut points, &mut rects) {
        return Outline::Other;
    }
    if rects.is_empty() {
        return Outline::Empty;
    }
    if rects.len() < 2 {
        return Outline::Other;
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

/// Whether the outlines of a run's glyphs (each with its glyph id and the scale of its
/// transform, points to a font unit) are a code: at least [`MIN_GLYPHS`] glyphs of bars,
/// [`MIN_KINDS`] of them different, the bars of one height, and every bar and every space
/// between the bars of a glyph a whole number of modules (the narrowest bar), at most
/// [`MOST_MODULES`].
pub(crate) fn bars_fit(glyphs: &[(u32, f64, &Outline)]) -> bool {
    // Each glyph of bars: its id, its scale, its bottom and top, and its bars.
    struct Barred<'a> {
        id: u32,
        scale: f64,
        y0: f64,
        y1: f64,
        bars: &'a [(f64, f64)],
    }
    let barred: Vec<Barred<'_>> = glyphs
        .iter()
        .filter_map(|&(id, scale, o)| match o {
            Outline::Bars { y0, y1, bars } => Some(Barred {
                id,
                scale,
                y0: *y0,
                y1: *y1,
                bars,
            }),
            _ => None,
        })
        .collect();
    // Spaces may stand among them; anything else drawn may not.
    if barred.len() < MIN_GLYPHS || glyphs.iter().any(|(_, _, o)| matches!(o, Outline::Other)) {
        return false;
    }
    let mut kinds: Vec<u32> = barred.iter().map(|b| b.id).collect();
    kinds.sort_unstable();
    kinds.dedup();
    if kinds.len() < MIN_KINDS {
        return false;
    }
    let (y0, y1) = (barred[0].y0, barred[0].y1);
    let height = y1 - y0;
    if barred.iter().any(|b| {
        (b.y0 - y0).abs() > SAME_HEIGHT * height || (b.y1 - y1).abs() > SAME_HEIGHT * height
    }) {
        return false;
    }
    let mut widths: Vec<f64> = Vec::new();
    let mut module = f64::MAX;
    for b in &barred {
        for (k, &(x0, x1)) in b.bars.iter().enumerate() {
            widths.push((x1 - x0) * b.scale);
            module = module.min((x1 - x0) * b.scale);
            if let Some(&(next, _)) = b.bars.get(k + 1) {
                widths.push((next - x1) * b.scale);
            }
        }
    }
    if module <= 0.0 || !module.is_finite() {
        return false;
    }
    widths.iter().all(|&w| {
        let n = w / module;
        n.round() >= 1.0 && n.round() <= MOST_MODULES && (n - n.round()).abs() <= WHOLE
    })
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

/// Where a glyph of a barcode font draws, with transform `t` (font units to the page) and
/// outline `o` (`path` when it is not bars), its advance `advance` in font units when known:
/// its box and its narrowest bar in points. `None` when it draws nothing.
pub(crate) fn place(
    t: Affine,
    o: &Outline,
    path: Option<&BezPath>,
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
            let ink = path.map(|p| p.bounding_box()).filter(|r| r.area() > 0.0);
            let b = match ink {
                Some(r) => [r.x0, r.y0, r.x1, r.y1],
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
pub(crate) fn codes(placed: &[Placed]) -> Vec<VectorCode> {
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
            VectorCode {
                b,
                module,
                id: h.finish(),
                finders: 0,
                font: true,
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
        ] {
            assert!(barcode_name(name), "{name}");
        }
        for name in [
            "helvetica",
            "arial-boldmt",
            "symbol",
            "times-roman",
            "codesans",
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
        // One bar (an `l`, an `I`, a `|`) is not bars.
        assert_eq!(outline(&rects(&[[0.0, 0.0, 80.0, 700.0]])), Outline::Other);
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
        // A curve is not.
        let mut c = pattern(&[2, 1, 2]);
        c.curve_to((0.0, 0.0), (10.0, 10.0), (20.0, 0.0));
        assert_eq!(outline(&c), Outline::Other);
    }

    #[test]
    fn a_run_is_a_code_only_with_glyphs_of_whole_modules() {
        let a = outline(&pattern(&[2, 1, 1, 2, 1, 4]));
        let b = outline(&pattern(&[2, 1, 2, 2, 2, 2]));
        let c = outline(&pattern(&[2, 3, 3, 1, 1, 1, 2]));
        let s = 0.012;
        assert!(bars_fit(&[(1, s, &a), (2, s, &b), (3, s, &c)]));
        // Too few glyphs, or one glyph three times.
        assert!(!bars_fit(&[(1, s, &a), (2, s, &b)]));
        assert!(!bars_fit(&[(1, s, &a), (1, s, &a), (1, s, &a)]));
        // Widths that are no whole modules (a double bar sign of 1 : 2.5).
        let odd = outline(&rects(&[
            [0.0, 0.0, 60.0, 900.0],
            [210.0, 0.0, 270.0, 900.0],
        ]));
        let odd2 = outline(&rects(&[
            [0.0, 0.0, 60.0, 900.0],
            [150.0, 0.0, 300.0, 900.0],
        ]));
        assert!(!bars_fit(&[(1, s, &odd), (2, s, &odd2), (1, s, &odd)]));
        // A glyph that is not bars among them.
        assert!(!bars_fit(&[
            (1, s, &a),
            (2, s, &b),
            (3, s, &Outline::Other)
        ]));
    }
}
