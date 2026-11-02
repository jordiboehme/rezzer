// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Barcodes and QR codes drawn as vector paths: regions of many small filled rectangles
//! (the modules of a matrix code, the bars of a linear one). Such a region is rendered again
//! on its own, one module on a whole number of pixels, and read like a picture.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// A region of the page that may be a code drawn as paths.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorCode {
    /// Its box, in the frame of the page's pictures (`PageItems::images`).
    pub b: [f32; 4],
    /// The size of one module in points: the narrowest side of its rectangles.
    pub module: f32,
    /// What it is: its rectangles relative to its corner, so the same code drawn on many
    /// pages is one.
    pub id: u64,
    /// For a part printed in a picture that shows QR finder patterns (see [`printed_codes`]):
    /// how many, 0 otherwise.
    pub finders: usize,
    /// It is printed with a barcode font (see [`crate::barfont`]): its bars are glyphs, and
    /// the read is checked against them as a picture's is.
    pub font: bool,
    /// For a code printed with a barcode font: the Code 128 values its glyphs print, read
    /// from their geometry, when each of its runs is one (empty otherwise). A read from it
    /// must be one of them.
    pub texts: Vec<String>,
}

/// The most filled rectangles a page keeps for finding codes.
pub(crate) const MAX_CELLS: usize = 200_000;

/// The most code regions of one page that are rendered again and read; the regions past
/// them are said (`render::vision`).
pub(crate) const MAX_REGIONS: usize = 8;

/// Rectangles in a region of a code, at least: a QR code of version 1 has well over this many
/// dark modules, a linear code this many bars.
const MIN_CELLS: usize = 20;

/// The narrow side of a module or bar, in points, at most; a bar's long side at most.
const MAX_MODULE: f32 = 12.0;
const MAX_BAR: f32 = 300.0;

/// The narrowest module that is looked at, in points.
const MIN_MODULE: f32 = 0.15;

/// Grid cells of the search: four times the median narrow side of the rectangles, within
/// these bounds in points.
const GRID_MIN: f32 = 0.5;
const GRID_MAX: f32 = 12.0;

/// Grid cells one rectangle is entered in or looks at, at most, along each axis.
const SPAN: i32 = 32;

/// The work of one page's search, at most: entries in the grid and pairs compared. Past
/// either, the page is not searched (`Err` with the reason).
const MAX_ENTRIES: usize = 2_000_000;
const MAX_PAIRS: u64 = 40_000_000;

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// About the bytes [`vector_codes`] holds for `cells`: its copy of them, the grid entries and
/// the grid's buckets, at most for [`MAX_ENTRIES`].
pub(crate) fn search_bytes(cells: &[[f32; 4]]) -> u64 {
    let n = cells.len() as u64;
    // Each cell lies in a few grid cells; the grid is bounded by MAX_ENTRIES.
    let entries = (n * 4).min(MAX_ENTRIES as u64);
    n * (16 + 8 + 8) + entries * (8 + 48)
}

/// How a rectangle may belong to a code: a bar of a linear code standing up, a bar lying
/// down (a linear code turned a quarter), or a module (or a run of modules) of a matrix code.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Up,
    Flat,
    Module,
}

fn shape(c: &[f32; 4]) -> Shape {
    let (w, h) = (c[2] - c[0], c[3] - c[1]);
    if h >= 4.0 * w {
        Shape::Up
    } else if w >= 4.0 * h {
        Shape::Flat
    } else {
        Shape::Module
    }
}

/// About the bytes one rectangle of [`pixel_cells`] takes while it is searched: itself in
/// pixels and in points, and its share of [`search_bytes`].
pub(crate) const PIXEL_CELL_BYTES: u64 = 16 + 16 + 32 + 4 * 56;

/// Dark parts of a render smaller than this many pixels both ways are left out by
/// [`pixel_cells`]: no bar or module is read at that size.
const PIXEL_MIN: u32 = 3;

/// The gray levels by which the dark and the light tone of a render differ at least for it to
/// be searched by [`pixel_cells`].
const PIXEL_CONTRAST: u32 = 64;

/// The dark parts of a render (`rgba`, `w` by `h` opaque pixels, row by row from the top
/// left) as filled rectangles in pixels (left, top, right, bottom): each run of dark pixels
/// in a row, joined with the runs at the same place (within [`PIXEL_SHIFT`]) in the rows
/// under it, so a bar or a
/// module of a code printed in a picture is one rectangle, as one drawn as paths is (for
/// [`vector_codes`]). Dark is darker than the render's Otsu threshold of gray; a render
/// without two tones [`PIXEL_CONTRAST`] apart has none. A part smaller than [`PIXEL_MIN`]
/// pixels both ways is left out. `None` when there are more than `max`.
pub(crate) fn pixel_cells(rgba: &[u8], w: usize, h: usize, max: usize) -> Option<Vec<[u32; 4]>> {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return Some(Vec::new());
    }
    let gray = |i: usize| gray_at(rgba, i);
    let mut hist = [0u64; 256];
    for i in 0..w * h {
        hist[usize::from(gray(i))] += 1;
    }
    let Some(t) = otsu(&hist) else {
        return Some(Vec::new());
    };
    let mut cells: Vec<[u32; 4]> = Vec::new();
    // The runs still open from the rows above, by the left of their first row: that row's
    // left and right, their widest left and right, and their first row.
    let mut open: Vec<Open> = Vec::new();
    let mut next: Vec<Open> = Vec::new();
    let close = |run: Open, y: u32, cells: &mut Vec<[u32; 4]>| {
        let (x0, x1, top) = (run.lo, run.hi, run.top);
        if x1 - x0 >= PIXEL_MIN || y - top >= PIXEL_MIN {
            cells.push([x0, top, x1, y]);
        }
        cells.len() <= max
    };
    for y in 0..=h {
        next.clear();
        let mut k = 0;
        let mut x = 0;
        while y < h && x < w {
            if gray(y * w + x) > t {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < w && gray(y * w + x) <= t {
                x += 1;
            }
            let (x0, x1) = (x0 as u32, x as u32);
            // The open runs that start before this one end here.
            while k < open.len() && open[k].first.0 + PIXEL_SHIFT < x0 {
                if !close(open[k], y as u32, &mut cells) {
                    return None;
                }
                k += 1;
            }
            // A run within a pixel of an open one's first row goes on with it: the soft ends
            // of blurred bars are a pixel narrower.
            match open.get(k) {
                Some(o)
                    if o.first.0.abs_diff(x0) <= PIXEL_SHIFT
                        && o.first.1.abs_diff(x1) <= PIXEL_SHIFT =>
                {
                    next.push(Open {
                        lo: o.lo.min(x0),
                        hi: o.hi.max(x1),
                        ..*o
                    });
                    k += 1;
                }
                _ => next.push(Open {
                    first: (x0, x1),
                    lo: x0,
                    hi: x1,
                    top: y as u32,
                }),
            }
        }
        for &run in &open[k..] {
            if !close(run, y as u32, &mut cells) {
                return None;
            }
        }
        std::mem::swap(&mut open, &mut next);
    }
    Some(cells)
}

/// A run of dark pixels of [`pixel_cells`] open from the rows above: the left and right of
/// its first row, its widest left and right, and its first row.
#[derive(Clone, Copy)]
struct Open {
    first: (u32, u32),
    lo: u32,
    hi: u32,
    top: u32,
}

/// How many pixels the ends of a run may stand off those of the first row of the run above
/// it for [`pixel_cells`] to join them.
const PIXEL_SHIFT: u32 = 1;

/// The gray of pixel `i` of `rgba`.
fn gray_at(rgba: &[u8], i: usize) -> u8 {
    let p = &rgba[i * 4..i * 4 + 3];
    ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000) as u8
}

/// How much lower than the lines on both sides the contrast of a line across the bars of a
/// linear code is at most for [`crossed`]: a line of paper between two codes, blurred.
const CROSSED_DIP: f32 = 0.8;

/// The lines of bars on each side of such a line, at least.
const CROSSED_PART: usize = 3;

/// Whether the bars of a linear code in box `b` (pixels: left, top, right, bottom) of the
/// render `rgba` (`size` in pixels) are crossed by a lighter line: along its bars, between two
/// parts of at least [`CROSSED_PART`] lines of bars each, the contrast across them (from the
/// darkest to the lightest tenth of the line) dips to [`CROSSED_DIP`] of the lower part's.
/// The soft ends of blurred bars only fade out. The bars stand the way the lines across
/// them show more contrast.
pub(crate) fn crossed(rgba: &[u8], (w, h): (usize, usize), b: [u32; 4]) -> bool {
    let (x0, x1) = (b[0] as usize, (b[2] as usize).min(w));
    let (y0, y1) = (b[1] as usize, (b[3] as usize).min(h));
    if x1 <= x0 || y1 <= y0 || rgba.len() < w * h * 4 {
        return false;
    }
    let contrast = |pixels: &mut dyn Iterator<Item = usize>| {
        let mut v: Vec<u8> = pixels.map(|i| gray_at(rgba, i)).collect();
        v.sort_unstable();
        let at = |share: usize| f32::from(v[(v.len() - 1) * share / 10]);
        at(9) - at(1)
    };
    let rows: Vec<f32> = (y0..y1)
        .map(|y| contrast(&mut (x0..x1).map(|x| y * w + x)))
        .collect();
    let cols: Vec<f32> = (x0..x1)
        .map(|x| contrast(&mut (y0..y1).map(|y| y * w + x)))
        .collect();
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let lines = if mean(&rows) >= mean(&cols) {
        rows
    } else {
        cols
    };
    (1..lines.len().saturating_sub(1)).any(|i| {
        let (before, after) = (&lines[..i], &lines[i + 1..]);
        let top = |v: &[f32]| v.iter().copied().fold(0.0, f32::max);
        let low = top(before).min(top(after));
        let strong = |v: &[f32]| v.iter().filter(|&&c| c >= 0.9 * low).count() >= CROSSED_PART;
        low > 0.0 && lines[i] <= CROSSED_DIP * low && strong(before) && strong(after)
    })
}

/// How two reads of one linear code are one code: its bars go on between them, or a dark
/// line runs across them there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Joined {
    Bars,
    Dark,
}

/// Whether boxes `a` and `b` (pixels: left, top, right, bottom) of the render `rgba` (`size`
/// in pixels), where one linear code was read twice, are bars of one code: they stand one
/// after the other along the bars, across them nearly the same ([`SPLIT_SAME`] of the
/// narrower), at most [`SPLIT_GAP`] of the shorter apart, and between them (or at their
/// meeting) a line is dark across [`SPLIT_DARK`] of their common part ([`Joined::Dark`]: it
/// may also be two codes with a dark line between them), or every line shows the bars as
/// strongly as the first box does, give or take [`CROSSED_DIP`] ([`Joined::Bars`]). Paper or a
/// lighter line between them is two codes.
pub(crate) fn joined(
    rgba: &[u8],
    (w, h): (usize, usize),
    a: [u32; 4],
    b: [u32; 4],
) -> Option<Joined> {
    if rgba.len() < w * h * 4 {
        return None;
    }
    let gray = |x: u32, y: u32| gray_at(rgba, y as usize * w + x as usize);
    [false, true].into_iter().find_map(|lying| {
        // Along the bars and across them: for bars standing up, y and x.
        let parts = |r: [u32; 4]| -> ((u32, u32), (u32, u32)) {
            if lying {
                ((r[0], r[2]), (r[1], r[3]))
            } else {
                ((r[1], r[3]), (r[0], r[2]))
            }
        };
        let ((a0, a1), (x0, x1)) = parts(a);
        let ((b0, b1), (y0, y1)) = parts(b);
        let (lo, hi) = (x0.max(y0), x1.min(y1));
        let narrow = (x1 - x0).min(y1 - y0);
        if hi <= lo || ((hi - lo) as f32) < SPLIT_SAME * narrow as f32 {
            return None;
        }
        // The first one along, then the gap to the other.
        let ((f0, f1), (s0, s1)) = if a0 <= b0 {
            ((a0, a1), (b0, b1))
        } else {
            ((b0, b1), (a0, a1))
        };
        let short = (f1 - f0).min(s1 - s0);
        if s0 + 1 < f1 || (s0.saturating_sub(f1) as f32) > SPLIT_GAP * short as f32 {
            return None;
        }
        let at = |t: u32, c: u32| if lying { gray(t, c) } else { gray(c, t) };
        // The tones of the bars: the darkest and the lightest tenth across the first one's middle.
        let mid = (f0 + f1) / 2;
        let mut row: Vec<u8> = (lo..hi).map(|c| at(mid, c)).collect();
        row.sort_unstable();
        let (dark, light) = (row[row.len() / 10], row[(row.len() - 1) * 9 / 10]);
        if light.saturating_sub(dark) < PIXEL_CONTRAST as u8 {
            return None;
        }
        let t = (u32::from(dark) + u32::from(light)) / 2;
        let limit = if lying { w as u32 } else { h as u32 };
        let gap = f1.saturating_sub(1)..(s0 + 1).min(limit);
        if gap.clone().any(|l| {
            let n = (lo..hi).filter(|&c| u32::from(at(l, c)) < t).count();
            n as f32 >= SPLIT_DARK * (hi - lo) as f32
        }) {
            return Some(Joined::Dark);
        }
        // The contrast across a line: from its darkest to its lightest tenth.
        let contrast = |l: u32| {
            let mut v: Vec<u8> = (lo..hi).map(|c| at(l, c)).collect();
            v.sort_unstable();
            f32::from(v[(v.len() - 1) * 9 / 10]) - f32::from(v[v.len() / 10])
        };
        let strong = f32::from(light - dark);
        gap.clone()
            .all(|l| contrast(l) >= CROSSED_DIP * strong)
            .then_some(Joined::Bars)
    })
}

/// For [`joined`]: the share of the narrower box the two have in common across the
/// bars, the gap between them as a share of the shorter one's bars, and the share of a line
/// between them that is dark.
const SPLIT_SAME: f32 = 0.75;
const SPLIT_GAP: f32 = 0.25;
const SPLIT_DARK: f32 = 0.9;

/// The Otsu threshold of gray histogram `hist`: the level that splits it into its dark and
/// light tones best; `None` when they are not [`PIXEL_CONTRAST`] apart.
fn otsu(hist: &[u64; 256]) -> Option<u8> {
    let total: u64 = hist.iter().sum();
    let sum: f64 = hist
        .iter()
        .enumerate()
        .map(|(v, &n)| v as f64 * n as f64)
        .sum();
    let (mut below, mut below_sum) = (0u64, 0f64);
    let mut best = (0f64, 0u8, 0f64, 0f64);
    for (v, &n) in hist.iter().enumerate() {
        below += n;
        below_sum += v as f64 * n as f64;
        let above = total - below;
        if below == 0 || above == 0 {
            continue;
        }
        let dark = below_sum / below as f64;
        let light = (sum - below_sum) / above as f64;
        let between = below as f64 * above as f64 * (dark - light).powi(2);
        if between > best.0 {
            best = (between, v as u8, dark, light);
        }
    }
    (best.0 > 0.0 && best.3 - best.2 >= f64::from(PIXEL_CONTRAST)).then_some(best.1)
}

/// Groups of at least this many rectangles are looked at for merging, at most this many: past
/// them the search says how many it left.
const MERGE_CELLS: usize = 8;
const MERGE_GROUPS: usize = 2_000;

/// Module places a search for codes printed in a picture may test for QR finder patterns
/// (each place of each region looked at once); past them it stops and says so.
const FINDER_WORK: usize = 10_000_000;

/// What a search for codes found, with why it looked at only part of the page when it did.
#[derive(Debug)]
pub(crate) struct Searched {
    pub codes: Vec<VectorCode>,
    pub partly: Option<String>,
}

/// The regions among `cells` (filled axis-aligned rectangles: x0, y0, x1, y1) that look like
/// a code: at least [`MIN_CELLS`] small rectangles on one module lattice, close to each
/// other, that make the pattern of a code. A matrix code shows its finder (QR Code and Micro
/// QR Code: the corner squares, Data Matrix: the solid L, Aztec: the bullseye, PDF417: the
/// start bars). A linear code has bars of one height (of any length, at least two modules)
/// and at least two widths, its bars and spaces whole modules wide, standing up or lying
/// down, with a quiet zone of 5 modules at both ends; bars of one band join across their
/// spaces whatever their shape, bars one after the other along their length do not. Groups
/// in one row band a few modules apart are one region (a PDF417 has light columns through
/// all its rows). All regions found, the ones with the most rectangles first: the caller
/// reads the first [`MAX_REGIONS`] and says the others. `Err` says why the page was not
/// searched: its work would pass [`MAX_ENTRIES`] or [`MAX_PAIRS`]. Groups of rectangles past
/// [`MERGE_GROUPS`] are not looked at, and [`Searched::partly`] says so.
pub(crate) fn vector_search(cells: &[[f32; 4]]) -> Result<Searched, String> {
    codes_among(cells, None, &|| false)
}

/// The codes [`vector_search`] finds.
#[cfg(test)]
pub(crate) fn vector_codes(cells: &[[f32; 4]]) -> Result<Vec<VectorCode>, String> {
    vector_search(cells).map(|s| s.codes)
}

/// The bars of a linear code drawn as paths are at least this many modules long.
const MIN_BAR: f32 = 2.0;

/// The fewest bars of a printed linear code cut by the edge of the render.
const MIN_CUT_BARS: usize = 8;

/// The bars of a linear code printed in a picture are at least this many modules long: a
/// printed code's bars are long beside its module, while a screen of dots cut by the edge of
/// a picture stands in bars of about 2.
const PRINTED_BAR: f32 = 3.0;

/// The share of the bars of a printed linear code that are of one width, below: its bars have
/// several widths, the dots or lines of a screen one.
const SAME_WIDTH: f32 = 0.9;

/// The paper around a matrix code printed in a picture, in modules, at least, within the
/// render: a screen of dots cut by its edge has none there.
const PRINTED_MATRIX_QUIET: f32 = 2.0;

/// What a search for codes printed in a picture holds to: the narrowest module, in points,
/// the box of the render, in points, and the dark parts cut by its edge (left out of the
/// search).
#[derive(Clone, Copy)]
struct Printed<'a> {
    min_module: f32,
    page: [f32; 4],
    cut: &'a [[f32; 4]],
    /// The module places tested for QR finder patterns so far (see [`FINDER_WORK`]).
    finder_work: &'a std::cell::Cell<usize>,
}

/// The regions among `cells`, the dark parts of a render in points (see [`pixel_cells`]),
/// that look like a code printed there: as [`vector_codes`], with modules at least
/// [`PRINTED_MODULE`] of the render's pixels (`pixel` points) wide, and positive evidence of a
/// code, not of a screen: a linear code has bars [`PRINTED_BAR`] modules long at least, of
/// several widths ([`SAME_WIDTH`]), whole modules give or take the spread of the ink,
/// with paper past their ends (see [`linear`]); a matrix code has paper around it within the
/// render (`page`, its box in points). The search stops when `late` says the time for
/// reading codes is over, or past [`FINDER_WORK`]: [`Searched::partly`] says so.
pub(crate) fn printed_codes(
    cells: &[[f32; 4]],
    pixel: f32,
    page: [f32; 4],
    cut: &[[f32; 4]],
    late: &dyn Fn() -> bool,
) -> Result<Searched, String> {
    let finder_work = std::cell::Cell::new(0);
    codes_among(
        cells,
        Some(Printed {
            min_module: PRINTED_MODULE * pixel,
            page,
            cut,
            finder_work: &finder_work,
        }),
        late,
    )
}

/// The modules of a code printed in a picture are at least this many pixels of its render
/// wide: on a lattice of one pixel every part of a render lies, letters too.
const PRINTED_MODULE: f32 = 2.0;

/// [`vector_search`], or with `printed` [`printed_codes`].
fn codes_among(
    cells: &[[f32; 4]],
    printed: Option<Printed<'_>>,
    late: &dyn Fn() -> bool,
) -> Result<Searched, String> {
    let cells: Vec<[f32; 4]> = cells
        .iter()
        .copied()
        .filter(|c| {
            let (w, h) = (c[2] - c[0], c[3] - c[1]);
            w.min(h) >= MIN_MODULE * 0.9 && w.min(h) <= MAX_MODULE && w.max(h) <= MAX_BAR
        })
        .collect();
    // A printed code cut by the edge of the render keeps fewer bars.
    let floor = if printed.is_some() {
        MIN_CUT_BARS
    } else {
        MIN_CELLS
    };
    if cells.len() < floor {
        return Ok(Searched {
            codes: Vec::new(),
            partly: None,
        });
    }
    let short = |c: &[f32; 4]| (c[2] - c[0]).min(c[3] - c[1]);
    let mut shorts: Vec<f32> = cells.iter().map(short).collect();
    let mid = shorts.len() / 2;
    let (_, median, _) = shorts.select_nth_unstable_by(mid, f32::total_cmp);
    let grid = (*median * 4.0).clamp(GRID_MIN, GRID_MAX);
    let key = |x: f32, y: f32| ((x / grid).floor() as i32, (y / grid).floor() as i32);
    let too_much = || {
        format!(
            "the page draws more small filled rectangles than are searched ({MAX_ENTRIES} grid \
             entries or {MAX_PAIRS} pairs)"
        )
    };
    let mut grid_of: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    let mut entries = 0usize;
    for (i, c) in cells.iter().enumerate() {
        let (a, b) = (key(c[0], c[1]), key(c[2], c[3]));
        for gx in a.0..=b.0.min(a.0 + SPAN) {
            for gy in a.1..=b.1.min(a.1 + SPAN) {
                grid_of.entry((gx, gy)).or_default().push(i);
                entries += 1;
            }
        }
        if entries > MAX_ENTRIES {
            return Err(too_much());
        }
    }
    let mut parent: Vec<usize> = (0..cells.len()).collect();
    // Modules of a matrix code touch or nearly touch. Bars of a linear code stand up to four
    // modules apart (its widest space), next to bars of the same rows (or columns).
    let reach_of =
        |c: &[f32; 4]| short(c) * if shape(c) == Shape::Module { 1.5 } else { 4.5 } + 0.01;
    let mut pairs = 0u64;
    for (i, c) in cells.iter().enumerate() {
        let reach = reach_of(c);
        let (a, b) = (
            key(c[0] - reach, c[1] - reach),
            key(c[2] + reach, c[3] + reach),
        );
        for gx in a.0..=b.0.min(a.0 + SPAN + 2) {
            for gy in a.1..=b.1.min(a.1 + SPAN + 2) {
                let Some(list) = grid_of.get(&(gx, gy)) else {
                    continue;
                };
                pairs += list.len() as u64;
                if pairs > MAX_PAIRS {
                    return Err(too_much());
                }
                for &j in list {
                    if j <= i {
                        continue;
                    }
                    let d = cells[j];
                    let gap_x = (d[0] - c[2]).max(c[0] - d[2]);
                    let gap_y = (d[1] - c[3]).max(c[1] - d[3]);
                    let near = (short(c).min(short(&d))) * 1.5 + 0.01;
                    let mut joined = gap_x <= near && gap_y <= near;
                    let far = reach.max(reach_of(&d));
                    match (shape(c), shape(&d)) {
                        // Bars one after the other along their length, apart, are two codes
                        // stacked; touching, one bar drawn in pieces.
                        (Shape::Up, Shape::Up) => {
                            let hh = c[3] - c[1];
                            let same =
                                (c[1] - d[1]).abs() <= 0.1 * hh && (c[3] - d[3]).abs() <= 0.1 * hh;
                            joined = (joined && gap_y <= 0.01) || (same && gap_x <= far);
                        }
                        (Shape::Flat, Shape::Flat) => {
                            let ww = c[2] - c[0];
                            let same =
                                (c[0] - d[0]).abs() <= 0.1 * ww && (c[2] - d[2]).abs() <= 0.1 * ww;
                            joined = (joined && gap_x <= 0.01) || (same && gap_y <= far);
                        }
                        _ => {}
                    }
                    if joined {
                        let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                        if ri != rj {
                            parent[ri.max(rj)] = ri.min(rj);
                        }
                    }
                }
            }
        }
    }
    join_bands(&cells, &mut parent, false);
    join_bands(&cells, &mut parent, true);
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..cells.len() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = groups
        .into_values()
        .filter(|g| g.len() >= MERGE_CELLS)
        .collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let what = if printed.is_some() {
        "dark parts"
    } else {
        "small filled rectangles"
    };
    let mut partly = (groups.len() > MERGE_GROUPS).then(|| {
        format!(
            "only the {MERGE_GROUPS} largest of its {} groups of {what} are looked at",
            groups.len()
        )
    });
    groups.truncate(MERGE_GROUPS);
    let groups = merge_bands(&cells, groups);
    let mut found: Vec<(usize, VectorCode)> = Vec::new();
    let look = |members: &[usize], found: &mut Vec<(usize, VectorCode)>| match region(
        &cells,
        members,
        printed,
        &|b| quiet(&cells, &grid_of, members, b, &key),
    ) {
        Some(code) => {
            found.push((members.len(), code));
            true
        }
        None => false,
    };
    for members in groups {
        if late() {
            partly = Some("the time for reading codes ran out".into());
            break;
        }
        if printed.is_some_and(|p| p.finder_work.get() > FINDER_WORK) {
            partly = Some(format!(
                "its groups of {what} take too many steps to look at for QR finder patterns"
            ));
            break;
        }
        if members.len() < floor || look(&members, &mut found) {
            continue;
        }
        // Short bars of codes one over the other are close enough to join as modules: each
        // band of them may be a code of its own.
        let mut any = false;
        for flat in [false, true] {
            for part in band_parts(&cells, &members, flat) {
                if part.len() >= floor {
                    any |= look(&part, &mut found);
                }
            }
            if any {
                break;
            }
        }
    }
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.b[3].total_cmp(&a.1.b[3]))
            .then(a.1.b[0].total_cmp(&b.1.b[0]))
    });
    Ok(Searched {
        codes: found.into_iter().map(|(_, c)| c).collect(),
        partly,
    })
}

/// The ends of `c` along its bars and its extent across them: for bars standing up its
/// bottom and top and its left and right, for bars lying down (`flat`) the other way round.
fn along(c: &[f32; 4], flat: bool) -> ((f32, f32), (f32, f32)) {
    if flat {
        ((c[0], c[2]), (c[1], c[3]))
    } else {
        ((c[1], c[3]), (c[0], c[2]))
    }
}

/// Whether two lengths along bars, `a` and `b`, are one: both ends within a tenth of the
/// shorter.
fn same_ends(a: (f32, f32), b: (f32, f32)) -> bool {
    let tol = 0.1 * (a.1 - a.0).min(b.1 - b.0);
    (a.0 - b.0).abs() <= tol && (a.1 - b.1).abs() <= tol
}

/// The rectangles before one in its band that [`join_bands`] compares it with, at most.
const BAND_LOOK: usize = 8;

/// Joins the rectangles of one band (one bottom and one top, or with `flat` one left and one
/// right side) that stand at most four and a half of the narrower one's width apart across
/// the band: the bars of a linear code, of any shape but the other way's. Short bars are
/// wider than a quarter of their length, so they are no `Shape::Up`, and their spaces are
/// wider than the reach of a module.
fn join_bands(cells: &[[f32; 4]], parent: &mut [usize], flat: bool) {
    // A bar of the other way is no bar of this one: two codes stacked stay apart.
    let other = if flat { Shape::Up } else { Shape::Flat };
    let mut order: Vec<usize> = (0..cells.len())
        .filter(|&i| shape(&cells[i]) != other)
        .collect();
    order.sort_by(|&i, &j| {
        let (a, b) = (along(&cells[i], flat).0, along(&cells[j], flat).0);
        a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
    });
    // Runs of rectangles whose first end is within the tolerance of the run's first.
    let mut start = 0;
    while start < order.len() {
        let first = along(&cells[order[start]], flat).0;
        let mut end = start + 1;
        while end < order.len() {
            let a = along(&cells[order[end]], flat).0;
            if (a.0 - first.0).abs() > 0.1 * (first.1 - first.0).min(a.1 - a.0) {
                break;
            }
            end += 1;
        }
        let mut band: Vec<usize> = order[start..end].to_vec();
        band.sort_by(|&i, &j| {
            along(&cells[i], flat)
                .1
                .0
                .total_cmp(&along(&cells[j], flat).1.0)
        });
        // Each rectangle joins the nearest one before it in the band with the same ends.
        let mut last: Vec<usize> = Vec::new();
        for &k in &band {
            let (ends, across) = along(&cells[k], flat);
            // The last few before it are enough: the bars of a code follow each other.
            if last.len() > BAND_LOOK {
                last.remove(0);
            }
            if let Some(&p) = last
                .iter()
                .rev()
                .find(|&&p| same_ends(along(&cells[p], flat).0, ends))
            {
                let (_, q) = along(&cells[p], flat);
                let narrow = (q.1 - q.0).min(across.1 - across.0);
                if across.0 - q.1 <= 4.5 * narrow + 0.01 {
                    let (ri, rj) = (find(parent, p), find(parent, k));
                    if ri != rj {
                        parent[ri.max(rj)] = ri.min(rj);
                    }
                }
            }
            last.push(k);
        }
        start = end;
    }
}

/// `members` of `cells` parted by band: the rectangles with the same ends along their bars
/// (standing up, or lying down with `flat`), each band once.
fn band_parts(cells: &[[f32; 4]], members: &[usize], flat: bool) -> Vec<Vec<usize>> {
    let hundredths = |v: f32| (v * 100.0).round() as i64;
    let mut order = members.to_vec();
    order.sort_by_key(|&i| {
        let e = along(&cells[i], flat).0;
        (hundredths(e.0), hundredths(e.1))
    });
    let mut parts: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let ends = along(&cells[i], flat).0;
        match parts.last_mut() {
            Some(p) if same_ends(along(&cells[p[0]], flat).0, ends) => p.push(i),
            _ => parts.push(vec![i]),
        }
    }
    parts
}

/// Whether the rectangles `members` of `cells` share one band: one bottom and one top (or
/// with `flat` one left and one right side), as the bars of a linear code without guard bars.
fn one_band(cells: &[[f32; 4]], members: &[usize], flat: bool) -> bool {
    let Some(&first) = members.first() else {
        return false;
    };
    let ends = along(&cells[first], flat).0;
    members
        .iter()
        .all(|&i| same_ends(along(&cells[i], flat).0, ends))
}

/// Whether most of `members` are bars standing up (or lying down, with `flat`): of that
/// shape, or all of one band.
fn bars(cells: &[[f32; 4]], members: &[usize], flat: bool) -> bool {
    let want = if flat { Shape::Flat } else { Shape::Up };
    let shaped = members
        .iter()
        .filter(|&&i| shape(&cells[i]) == want)
        .count();
    shaped * 10 >= members.len() * 8 || one_band(cells, members, flat)
}

/// The box and the narrowest side of `members` of `cells`.
fn bounds(cells: &[[f32; 4]], members: &[usize]) -> ([f32; 4], f32) {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut m = f32::MAX;
    for &i in members {
        let c = cells[i];
        b = [
            b[0].min(c[0]),
            b[1].min(c[1]),
            b[2].max(c[2]),
            b[3].max(c[3]),
        ];
        m = m.min((c[2] - c[0]).min(c[3] - c[1]));
    }
    (b, m)
}

/// Groups that share their row band (bottom and top within a module) and stand at most 5
/// modules apart side by side, or share their column band and stand at most 5 modules apart
/// one above the other, merged; but not two groups of bars one after the other along their
/// bars (stacked linear codes).
fn merge_bands(cells: &[[f32; 4]], groups: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..groups.len()).collect();
    let info: Vec<([f32; 4], f32)> = groups.iter().map(|g| bounds(cells, g)).collect();
    let standing: Vec<bool> = groups.iter().map(|g| bars(cells, g, false)).collect();
    let lying: Vec<bool> = groups.iter().map(|g| bars(cells, g, true)).collect();
    for i in 0..groups.len() {
        for j in i + 1..groups.len() {
            let ((a, ma), (b, mb)) = (info[i], info[j]);
            let m = ma.min(mb);
            // Two linear codes one after the other along their bars are two codes.
            let rows =
                (a[1] - b[1]).abs() <= m && (a[3] - b[3]).abs() <= m && !(lying[i] && lying[j]);
            let cols = (a[0] - b[0]).abs() <= m
                && (a[2] - b[2]).abs() <= m
                && !(standing[i] && standing[j]);
            let gap_x = (b[0] - a[2]).max(a[0] - b[2]);
            let gap_y = (b[1] - a[3]).max(a[1] - b[3]);
            if (rows && gap_x <= 5.0 * m) || (cols && gap_y <= 5.0 * m) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri.max(rj)] = ri.min(rj);
                }
            }
        }
    }
    let mut merged: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, g) in groups.into_iter().enumerate() {
        let r = find(&mut parent, i);
        merged.entry(r).or_default().extend(g);
    }
    merged.into_values().collect()
}

/// Whether no rectangle but `members` lies in box `b`: the quiet zone of a linear code.
fn quiet(
    cells: &[[f32; 4]],
    grid_of: &HashMap<(i32, i32), Vec<usize>>,
    members: &[usize],
    b: [f32; 4],
    key: &dyn Fn(f32, f32) -> (i32, i32),
) -> bool {
    let (a, z) = (key(b[0], b[1]), key(b[2], b[3]));
    let inside = |c: &[f32; 4]| c[0] < b[2] && c[2] > b[0] && c[1] < b[3] && c[3] > b[1];
    for gx in a.0..=z.0.min(a.0 + 4 * SPAN) {
        for gy in a.1..=z.1.min(a.1 + 4 * SPAN) {
            if let Some(list) = grid_of.get(&(gx, gy))
                && list
                    .iter()
                    .any(|&k| inside(&cells[k]) && !members.contains(&k))
            {
                return false;
            }
        }
    }
    true
}

/// The most modules a side of a matrix code has.
const MAX_MODULES: usize = 200;

/// The code region of `members` of `cells`, when they make the pattern of a code (see
/// [`vector_codes`]). `quiet` says whether a box holds no other rectangle.
fn region(
    cells: &[[f32; 4]],
    members: &[usize],
    printed: Option<Printed<'_>>,
    quiet: &dyn Fn([f32; 4]) -> bool,
) -> Option<VectorCode> {
    let min_module = printed.map_or(MIN_MODULE, |p| p.min_module);
    let (b, module) = bounds(cells, members);
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    // No size in points bounds a region: two codes side by side past their quiet zone may
    // join into one wider than any code, and are still looked at (and named when the codes
    // read do not fill them).
    if module < min_module.max(MIN_MODULE) {
        return None;
    }
    let area: f32 = members
        .iter()
        .map(|&i| (cells[i][2] - cells[i][0]) * (cells[i][3] - cells[i][1]))
        .sum();
    let cover = area / (w * h).max(f32::MIN_POSITIVE);
    if !(0.15..=0.85).contains(&cover) {
        return None;
    }
    // A matrix code has every edge on one lattice of the module size, from the region's
    // corner; a linear code its bars and spaces across (see [`linear`]).
    let on = |v: f32, o: f32| {
        let k = (v - o) / module;
        (k - k.round()).abs() <= 0.2
    };
    let lattice = || {
        let on_it = members
            .iter()
            .filter(|&&i| {
                let c = cells[i];
                on(c[0], b[0]) && on(c[2], b[0]) && on(c[1], b[1]) && on(c[3], b[1])
            })
            .count();
        on_it * 10 >= members.len() * 9
    };
    let edges = printed.map(|p| p.cut);
    let mut finders = 0;
    // A printed matrix code has paper around it, inside the render.
    let paper_around = || {
        let Some(p) = printed else { return true };
        let q = PRINTED_MATRIX_QUIET * module;
        b[0] - q >= p.page[0]
            && b[1] - q >= p.page[1]
            && b[2] + q <= p.page[2]
            && b[3] + q <= p.page[3]
            && quiet([b[0] - q, b[1] - q, b[0] - 0.01, b[3] + q])
            && quiet([b[2] + 0.01, b[1] - q, b[2] + q, b[3] + q])
            && quiet([b[0], b[1] - q, b[2], b[1] - 0.01])
            && quiet([b[0], b[3] + 0.01, b[2], b[3] + q])
    };
    let kind_ok = if bars(cells, members, false) {
        linear(cells, members, (b, module, edges), false, quiet)
    } else if bars(cells, members, true) && linear(cells, members, (b, module, edges), true, quiet)
    {
        true
    } else {
        members.len() >= MIN_CELLS
            && lattice()
            && ((matrix(cells, members, b, module) && paper_around())
                || printed.is_some_and(|p| {
                    finders = qr_finders(cells, members, b, module, p.finder_work);
                    finders >= 2
                }))
    };
    if !kind_ok {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    let mut rel: Vec<[i32; 4]> = members
        .iter()
        .map(|&i| {
            let c = cells[i];
            let r = |v: f32, o: f32| ((v - o) * 100.0).round() as i32;
            [r(c[0], b[0]), r(c[1], b[1]), r(c[2], b[0]), r(c[3], b[1])]
        })
        .collect();
    rel.sort_unstable();
    rel.hash(&mut hasher);
    // Apart from picture ids (hashes of picture data), by a marker.
    "vector code".hash(&mut hasher);
    Some(VectorCode {
        b,
        module,
        id: hasher.finish(),
        finders,
        font: false,
        texts: Vec::new(),
    })
}

/// Whether bars `members` (standing up, or lying down with `flat`) in box `b` of `module`
/// are a linear code: about as long as the region and at least [`MIN_BAR`] modules long, of
/// at least two widths and at most 4 modules wide, every bar and nearly every space a whole
/// number of modules wide, not all at one pitch, with 5 modules free at both ends. A code
/// `printed` in a picture has its bars and spaces whole modules give or take the ink's
/// spread, and paper past the ends of its bars too.
fn linear(
    cells: &[[f32; 4]],
    members: &[usize],
    (b, module, cut): ([f32; 4], f32, Option<&[[f32; 4]]>),
    flat: bool,
    quiet: &dyn Fn([f32; 4]) -> bool,
) -> bool {
    let printed = cut.is_some();
    let sides = |c: &[f32; 4]| {
        if flat {
            (c[2] - c[0], c[3] - c[1])
        } else {
            (c[3] - c[1], c[2] - c[0])
        }
    };
    let long = |c: &[f32; 4]| sides(c).0;
    let across = |c: &[f32; 4]| sides(c).1;
    let span = if flat { b[2] - b[0] } else { b[3] - b[1] };
    let tall = members
        .iter()
        .filter(|&&i| long(&cells[i]) >= 0.6 * span)
        .count();
    if tall * 10 < members.len() * 8 {
        return false;
    }
    // Bars drawn a module at a time are joined first: a bar is a run of touching ones.
    let start = |c: &[f32; 4]| if flat { c[1] } else { c[0] };
    let mut spans: Vec<(f32, f32)> = members
        .iter()
        .map(|&i| (start(&cells[i]), start(&cells[i]) + across(&cells[i])))
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut runs: Vec<(f32, f32)> = Vec::new();
    for (a, z) in spans {
        match runs.last_mut() {
            Some(last) if a <= last.1 + 0.2 * module => last.1 = last.1.max(z),
            _ => runs.push((a, z)),
        }
    }
    // Ink spreads in print: its bars grow and its spaces shrink by as much. The module of a
    // printed code is the mean of its narrowest bar and space, the spread what that bar has
    // more.
    let (module, spread) = match runs.windows(2).map(|w| w[1].0 - w[0].1).reduce(f32::min) {
        Some(space) if printed => {
            let bar = runs.iter().map(|(a, z)| z - a).fold(f32::MAX, f32::min);
            ((bar + space) / 2.0, (bar - space) / 2.0)
        }
        _ => (module, 0.0),
    };
    // Bars at least two modules long; every bar and space a whole number of modules wide,
    // but for a few spaces (codes side by side closer than their quiet zone).
    let min_bar = if printed { PRINTED_BAR } else { MIN_BAR };
    if members
        .iter()
        .any(|&i| long(&cells[i]) < min_bar * module - 0.01)
    {
        return false;
    }
    let whole = |v: f32| {
        let k = v / module;
        k.round() >= 1.0 && (k - k.round()).abs() <= 0.2
    };
    if runs.iter().any(|(a, z)| !whole(z - a - spread)) {
        return false;
    }
    let spaces = runs
        .windows(2)
        .filter(|w| !whole(w[1].0 - w[0].1 + spread))
        .count();
    if spaces * 10 > runs.len().saturating_sub(1) {
        return false;
    }
    // A code's bars are at most 4 modules wide and never at one pitch: bars that start at
    // one step after another (a ruler, a scale bar, the ticks of an axis, hatching) carry
    // nothing.
    if runs.iter().any(|(a, z)| (z - a - spread) / module > 4.2) {
        return false;
    }
    // At one pitch by their starts, their ends or their middles: ticks may be set by any.
    let one_pitch = |at: &dyn Fn(&(f32, f32)) -> f32| {
        let mut steps: Vec<i32> = runs
            .windows(2)
            .map(|w| ((at(&w[1]) - at(&w[0])) / module * 4.0).round() as i32)
            .collect();
        steps.sort_unstable();
        let most = steps
            .chunk_by(|a, b| a == b)
            .map(<[i32]>::len)
            .max()
            .unwrap_or(0);
        most * 10 >= steps.len() * 9
    };
    if one_pitch(&|r| r.0) || one_pitch(&|r| r.1) || one_pitch(&|r| (r.0 + r.1) / 2.0) {
        return false;
    }
    if printed && one_width(&runs, module, spread) {
        return false;
    }
    let mut widths: Vec<i32> = runs
        .iter()
        .map(|(a, z)| ((z - a - spread) / module).round() as i32)
        .collect();
    widths.sort_unstable();
    widths.dedup();
    if widths.len() < 2 {
        return false;
    }
    let q = 5.0 * module;
    // Paper at both ends; a printed code may run off the edge of the render at one of them
    // (its bar cut there is left out), when the other end shows its quiet zone.
    let (first, last) = if flat {
        (
            quiet([b[0], b[1] - q, b[2], b[1] - 0.01]),
            quiet([b[0], b[3] + 0.01, b[2], b[3] + q]),
        )
    } else {
        (
            quiet([b[0] - q, b[1], b[0] - 0.01, b[3]]),
            quiet([b[2] + 0.01, b[1], b[2] + q, b[3]]),
        )
    };
    // A bar cut by the edge of the render past an end, in the code's band, a space at most
    // after its last bar: the code runs on there.
    let cut = cut.map(|cut| {
        let (ends, (lo, hi)) = along(&b, flat);
        let runs_on = |past: &dyn Fn(f32, f32) -> f32| {
            cut.iter().any(|c| {
                let (e, (c0, c1)) = along(c, flat);
                let gap = past(c0, c1);
                same_ends(ends, e) && (-0.5 * module..=4.5 * module).contains(&gap)
            })
        };
        (runs_on(&|_, c1| lo - c1), runs_on(&|c0, _| c0 - hi))
    });
    let ends = match cut {
        Some((cut_first, cut_last)) => {
            (first && last) || (first && cut_last) || (cut_first && last)
        }
        None => first && last,
    };
    // Fewer bars than a code has at least only where it runs off the render.
    if runs.len() < MIN_CELLS && !cut.is_some_and(|(a, z)| a || z) {
        return false;
    }
    // A printed code stands on paper: past the ends of its bars too, from half a module (the
    // soft ends of blurred bars) to one and a half, where a screen of dots goes on.
    let (e0, e1) = (0.5 * module, 1.5 * module);
    ends && (!printed
        || if flat {
            quiet([b[0] - e1, b[1], b[0] - e0, b[3]]) && quiet([b[2] + e0, b[1], b[2] + e1, b[3]])
        } else {
            quiet([b[0], b[1] - e1, b[2], b[1] - e0]) && quiet([b[0], b[3] + e0, b[2], b[3] + e1])
        })
}

/// Whether nine in ten of bars `runs` (start and end across) of `module`, the ink spread by
/// `spread`, are as wide as each other ([`SAME_WIDTH`]): the dots or lines of a screen are, the
/// bars of a linear code have several widths.
fn one_width(runs: &[(f32, f32)], module: f32, spread: f32) -> bool {
    let mut widths: Vec<i32> = runs
        .iter()
        .map(|(a, z)| ((z - a - spread) / module * 4.0).round() as i32)
        .collect();
    widths.sort_unstable();
    let most = widths
        .chunk_by(|a, b| a == b)
        .map(<[i32]>::len)
        .max()
        .unwrap_or(0);
    most as f32 >= SAME_WIDTH * runs.len() as f32
}

/// The QR Code finder patterns among `members` in box `b` on a lattice of `module`, each
/// whole (7 by 7 modules: a dark ring, a light ring and a dark core of 3, nine in ten of its
/// modules so) with its light separator wherever that lies in the box, when at least two of
/// them stand in one row or one column of modules at least 14 modules apart (the finders of
/// one QR code, or of several side by side, or of a QR code cut by the edge of the page). How
/// many there are then, else 0: positive evidence of QR codes where the pattern of a whole one
/// is not there.
/// Each module place tested is added to `work`.
fn qr_finders(
    cells: &[[f32; 4]],
    members: &[usize],
    b: [f32; 4],
    module: f32,
    work: &std::cell::Cell<usize>,
) -> usize {
    let n = ((b[2] - b[0]) / module).round() as usize;
    let m = ((b[3] - b[1]) / module).round() as usize;
    if n < 7 || m < 7 || n > 2 * MAX_MODULES || m > 2 * MAX_MODULES {
        return 0;
    }
    work.set(work.get() + n * m);
    let mut dark = vec![false; n * m];
    for &i in members {
        let c = cells[i];
        let col = |v: f32| (((v - b[0]) / module).round().max(0.0) as usize).min(n);
        let row = |v: f32| (((b[3] - v) / module).round().max(0.0) as usize).min(m);
        for r in row(c[3])..row(c[1]) {
            for k in col(c[0])..col(c[2]) {
                dark[r * n + k] = true;
            }
        }
    }
    let at = |r: isize, k: isize| -> Option<bool> {
        (r >= 0 && k >= 0 && (r as usize) < m && (k as usize) < n)
            .then(|| dark[r as usize * n + k as usize])
    };
    let mut found: Vec<(usize, usize)> = Vec::new();
    // The places within 6 modules of a finder found, each way: no finder starts there.
    let mut blocked = vec![false; n * m];
    for r0 in 0..=m - 7 {
        for k0 in 0..=n - 7 {
            if blocked[r0 * n + k0] {
                continue;
            }
            // The separator (ring -1) is light; then a dark ring, a light ring and a dark
            // core. Only the separator may lie past the box.
            let (mut hits, mut all) = (0usize, 0usize);
            let mut whole = true;
            for r in -1..=7isize {
                for k in -1..=7isize {
                    let ring = r.min(k).min(6 - r).min(6 - k);
                    let want = ring >= 0 && ring != 1;
                    match at(r0 as isize + r, k0 as isize + k) {
                        Some(v) => {
                            all += 1;
                            hits += usize::from(v == want);
                        }
                        None => whole &= ring < 0,
                    }
                }
            }
            if whole && hits * 10 >= all * 9 {
                found.push((r0, k0));
                for r in r0.saturating_sub(6)..(r0 + 7).min(m) {
                    blocked[r * n + k0.saturating_sub(6)..r * n + (k0 + 7).min(n)].fill(true);
                }
            }
        }
    }
    // Two finders in one row (or column) at least 14 modules apart: the first and the last
    // of each row and column.
    let mut rows: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut columns: HashMap<usize, (usize, usize)> = HashMap::new();
    for &(r, k) in &found {
        let row = rows.entry(r).or_insert((k, k));
        *row = (row.0.min(k), row.1.max(k));
        let column = columns.entry(k).or_insert((r, r));
        *column = (column.0.min(r), column.1.max(r));
    }
    let lined = rows
        .values()
        .chain(columns.values())
        .any(|&(lo, hi)| hi - lo >= 14);
    if lined { found.len() } else { 0 }
}

/// Whether `members` in box `b` on a lattice of `module` make a matrix code's finder: QR Code
/// (three corner squares), Micro QR Code (one), Data Matrix (the solid L of its left column
/// and bottom row), Aztec (the bullseye) or PDF417 (8 dark modules starting every row). Also
/// for white modules on dark (the pattern inverted).
fn matrix(cells: &[[f32; 4]], members: &[usize], b: [f32; 4], module: f32) -> bool {
    let n = ((b[2] - b[0]) / module).round() as usize;
    let m = ((b[3] - b[1]) / module).round() as usize;
    if n < 8 || m < 3 || n > MAX_MODULES || m > MAX_MODULES {
        return false;
    }
    // Row 0 is the top row.
    let mut dark = vec![false; n * m];
    for &i in members {
        let c = cells[i];
        let col = |v: f32| (((v - b[0]) / module).round().max(0.0) as usize).min(n);
        let row = |v: f32| (((b[3] - v) / module).round().max(0.0) as usize).min(m);
        for r in row(c[3])..row(c[1]) {
            for k in col(c[0])..col(c[2]) {
                dark[r * n + k] = true;
            }
        }
    }
    for invert in [false, true] {
        let at = |r: usize, k: usize| dark[r * n + k] != invert;
        let share = |hits: usize, all: usize| hits * 100 >= all * 90;
        let finder = |r0: usize, k0: usize| {
            let mut hits = 0;
            for r in 0..7 {
                for k in 0..7 {
                    let ring = r.min(k).min(6 - r).min(6 - k);
                    hits += usize::from(at(r0 + r, k0 + k) == (ring != 1));
                }
            }
            share(hits, 49)
        };
        if n == m {
            if (21..=177).contains(&n) && (n - 21).is_multiple_of(4) {
                let corners = [finder(0, 0), finder(0, n - 7), finder(n - 7, 0)];
                if corners.iter().filter(|&&f| f).count() >= 3 {
                    return true;
                }
            }
            if (11..=17).contains(&n) && !n.is_multiple_of(2) && finder(0, 0) {
                return true;
            }
            if !n.is_multiple_of(2) && n >= 15 {
                let c = n / 2;
                let mut hits = 0;
                for r in c - 4..=c + 4 {
                    for k in c - 4..=c + 4 {
                        let ring = r.abs_diff(c).max(k.abs_diff(c));
                        hits += usize::from(at(r, k) == ring.is_multiple_of(2));
                    }
                }
                if share(hits, 81) {
                    return true;
                }
            }
        }
        // Data Matrix: a solid left column and bottom row.
        if n >= 8 && m >= 8 && n.is_multiple_of(2) && m.is_multiple_of(2) {
            let left = (0..m).filter(|&r| at(r, 0)).count();
            let bottom = (0..n).filter(|&k| at(m - 1, k)).count();
            if left * 100 >= m * 95 && bottom * 100 >= n * 95 {
                return true;
            }
        }
        // PDF417: every row starts with 8 dark modules and a light one.
        if n >= 17 {
            let rows = (0..m)
                .filter(|&r| (0..8).all(|k| at(r, k)) && !at(r, 8))
                .count();
            if rows * 100 >= m * 95 {
                return true;
            }
        }
    }
    false
}

/// A linear code the decoder read, with what it read, for [`bars_fit`].
#[derive(Clone, Copy, Debug)]
pub(crate) enum Linear<'a> {
    /// EAN-13, or UPC-A (an EAN-13 with a leading 0): its digits.
    Ean(&'a str),
    Code128,
    /// Code 39: its characters, without the start and stop.
    Code39(&'a str),
    /// ITF: its digits.
    Itf(&'a str),
}

/// The widths in modules of the digits of EAN-13 in the L set (space, bar, space, bar); the
/// R set has the same widths starting with a bar, the G set the L widths turned round.
const EAN_L: [[u8; 4]; 10] = [
    [3, 2, 1, 1],
    [2, 2, 2, 1],
    [2, 1, 2, 2],
    [1, 4, 1, 1],
    [1, 1, 3, 2],
    [1, 2, 3, 1],
    [1, 1, 1, 4],
    [1, 3, 1, 2],
    [1, 2, 1, 3],
    [3, 1, 1, 2],
];

/// Which of the six left digits of an EAN-13 are in the G set, by its first digit (bit 5 the
/// first of them).
const EAN_PARITY: [u8; 10] = [
    0b000000, 0b001011, 0b001101, 0b001110, 0b010011, 0b011001, 0b011100, 0b010101, 0b010110,
    0b011010,
];

/// The widths in modules of the bars and spaces of the EAN-13 of `digits` (13 digits, or 12
/// for a UPC-A), bar first: the guards, six digits each side and the middle guard (59).
fn ean_widths(digits: &str) -> Option<Vec<u8>> {
    let d: Vec<usize> = digits
        .chars()
        .map(|c| c.to_digit(10).map(|v| v as usize))
        .collect::<Option<_>>()?;
    let d = match d.len() {
        13 => d,
        12 => [vec![0], d].concat(),
        _ => return None,
    };
    let mut w = vec![1, 1, 1];
    for (k, &v) in d[1..7].iter().enumerate() {
        let mut digit = EAN_L[v];
        if EAN_PARITY[d[0]] >> (5 - k) & 1 == 1 {
            digit.reverse();
        }
        w.extend(digit);
    }
    w.extend([1, 1, 1, 1, 1]);
    for &v in &d[7..13] {
        w.extend(EAN_L[v]);
    }
    w.extend([1, 1, 1]);
    Some(w)
}

/// The characters of Code 39 and their bars and spaces (9 bits, the first element the
/// highest; a set bit is wide).
const CODE39: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";
const CODE39_WIDE: [u16; 43] = [
    0x034, 0x121, 0x061, 0x160, 0x031, 0x130, 0x070, 0x025, 0x124, 0x064, 0x109, 0x049, 0x148,
    0x019, 0x118, 0x058, 0x00D, 0x10C, 0x04C, 0x01C, 0x103, 0x043, 0x142, 0x013, 0x112, 0x052,
    0x007, 0x106, 0x046, 0x016, 0x181, 0x0C1, 0x1C0, 0x091, 0x190, 0x0D0, 0x085, 0x184, 0x0C4,
    0x0A8, 0x0A2, 0x08A, 0x02A,
];
const CODE39_STAR: u16 = 0x094;

/// The narrow (`Some(false)`) and wide (`Some(true)`) bars and spaces of the Code 39 of
/// `text`, bar first, start and stop included; `None` for the space between two characters,
/// which may have any width from narrow up.
fn code39_wide(text: &str) -> Option<Vec<Option<bool>>> {
    let mut chars = vec![CODE39_STAR];
    for c in text.chars() {
        chars.push(CODE39_WIDE[CODE39.find(c)?]);
    }
    chars.push(CODE39_STAR);
    let mut out = Vec::new();
    for (k, bits) in chars.iter().enumerate() {
        if k > 0 {
            out.push(None);
        }
        out.extend((0..9).rev().map(|b| Some(bits >> b & 1 == 1)));
    }
    Some(out)
}

/// The wide bars and spaces of the digits of ITF (5 elements each, 2 wide).
const ITF_WIDE: [u8; 10] = [
    0b00110, 0b10001, 0b01001, 0b11000, 0b00101, 0b10100, 0b01100, 0b00011, 0b10010, 0b01010,
];

/// The narrow and wide bars and spaces of the ITF of `digits` (an even count), bar first:
/// the start (four narrow), the digits two by two (the first one's bars between the second
/// one's spaces) and the stop (wide bar, narrow space, narrow bar).
fn itf_wide(digits: &str) -> Option<Vec<Option<bool>>> {
    let d: Vec<usize> = digits
        .chars()
        .map(|c| c.to_digit(10).map(|v| v as usize))
        .collect::<Option<_>>()?;
    if d.is_empty() || !d.len().is_multiple_of(2) {
        return None;
    }
    let mut out = vec![Some(false); 4];
    for pair in d.chunks(2) {
        let (a, b) = (ITF_WIDE[pair[0]], ITF_WIDE[pair[1]]);
        for k in (0..5).rev() {
            out.push(Some(a >> k & 1 == 1));
            out.push(Some(b >> k & 1 == 1));
        }
    }
    out.extend([Some(true), Some(false), Some(false)]);
    Some(out)
}

/// The start patterns of Code 128 (A, B and C) and its stop, in modules.
const CODE128_STARTS: [[u8; 6]; 3] = [[2, 1, 1, 4, 1, 2], [2, 1, 1, 2, 1, 4], [2, 1, 1, 2, 3, 2]];
const CODE128_STOP: [u8; 7] = [2, 3, 3, 1, 1, 1, 2];

/// How far a bar or space of a linear code read may be off its whole modules (or its narrow
/// or wide width), as a share of a module, after the ink's spread, for [`bars_fit`].
const FIT_TOLERANCE: f32 = 0.4;

/// [`FIT_TOLERANCE`] for a module of `module` pixels: up to half a module where that is
/// under a pixel or two, as each edge falls on a whole pixel.
fn tolerance(module: f32) -> f32 {
    FIT_TOLERANCE.max((1.0 / module).min(0.5))
}

/// The paper at both ends of a linear code read, in modules (narrow widths) at least, for
/// [`bars_fit`].
const FIT_QUIET: f32 = 3.0;

/// The length of the bars of a linear code read, in modules at least, for [`bars_fit`]: the
/// lines across them that show the code one after the other.
const FIT_BAR: f32 = 4.0;

/// One line across the bars of a code in a render: the edges of its dark runs, in pixels
/// along the line, each where the gray crosses the line's threshold between two pixels (a run
/// cut by an end of the line ends there). `None` when the line has too little contrast to
/// tell bars from paper.
fn line_edges(gray: &[u8], contrast: u8) -> Option<Vec<f32>> {
    let (lo, hi) = gray
        .iter()
        .fold((255u8, 0u8), |(a, b), &v| (a.min(v), b.max(v)));
    if hi.saturating_sub(lo) < contrast {
        return None;
    }
    let mut hist = [0u64; 256];
    for &v in gray {
        hist[usize::from(v)] += 1;
    }
    let t = otsu_middle(&hist);
    let dark = |v: u8| f32::from(v) < t;
    let mut edges = Vec::new();
    // A line that starts dark starts with a run cut by the line's end: a bar from there.
    if gray.first().is_some_and(|&v| dark(v)) {
        edges.push(0.0);
    }
    for k in 1..gray.len() {
        let (a, b) = (gray[k - 1], gray[k]);
        if dark(a) != dark(b) {
            let (fa, fb) = (f32::from(a), f32::from(b));
            // The pixel centres are at k - 0.5 and k + 0.5.
            edges.push(k as f32 - 0.5 + (t - fa) / (fb - fa));
        }
    }
    if gray.last().is_some_and(|&v| dark(v)) {
        edges.push(gray.len() as f32);
    }
    Some(edges)
}

/// The gray halfway between the dark and the light tone of a histogram, split at its Otsu
/// level: where a sharp edge between them lies.
fn otsu_middle(hist: &[u64; 256]) -> f32 {
    let total: u64 = hist.iter().sum();
    let sum: f64 = hist
        .iter()
        .enumerate()
        .map(|(v, &n)| v as f64 * n as f64)
        .sum();
    let (mut below, mut below_sum) = (0u64, 0f64);
    let mut best = (0f64, 0f64);
    for (v, &n) in hist.iter().enumerate() {
        below += n;
        below_sum += v as f64 * n as f64;
        let above = total - below;
        if below == 0 || above == 0 {
            continue;
        }
        let dark = below_sum / below as f64;
        let light = (sum - below_sum) / above as f64;
        let between = below as f64 * above as f64 * (dark - light).powi(2);
        if between > best.0 {
            best = (between, (dark + light) / 2.0);
        }
    }
    best.1 as f32
}

/// A window of bars and spaces on one line that fits a code: where it starts and ends along
/// the line, in pixels, and its module (narrow width).
#[derive(Clone, Copy, Debug)]
struct Fit {
    start: f32,
    end: f32,
    module: f32,
}

/// The ink's spread of bars `w` (bar first) beside the widths `want` (in modules, `module`
/// each): half of what the bars have more than the spaces, on average.
fn spread_of(w: &[f32], want: &[f32], module: f32) -> f32 {
    let (mut bars, mut spaces, mut nb, mut ns) = (0.0, 0.0, 0.0, 0.0);
    for (k, (&got, &e)) in w.iter().zip(want).enumerate() {
        if k.is_multiple_of(2) {
            bars += got - e * module;
            nb += 1.0;
        } else {
            spaces += got - e * module;
            ns += 1.0;
        }
    }
    (bars / f32::max(nb, 1.0) - spaces / f32::max(ns, 1.0)) / 2.0
}

/// Whether widths `w` (bar first) are `want` modules each, give or take
/// [`FIT_TOLERANCE`] after the ink's spread; the module when they are.
fn fits_modules(w: &[f32], want: &[f32]) -> Option<f32> {
    let total: f32 = want.iter().sum();
    let module = w.iter().sum::<f32>() / total;
    let spread = spread_of(w, want, module);
    let tol = tolerance(module);
    let ok = w.iter().zip(want).enumerate().all(|(k, (&got, &e))| {
        let s = if k.is_multiple_of(2) { spread } else { -spread };
        ((got - s) / module - e).abs() <= tol
    });
    (ok && spread.abs() <= 0.5 * module).then_some(module)
}

/// Whether widths `w` (bar first) are narrow and wide as `wide` says (`None`: a space between
/// characters, narrow or wider), the wide ones 2 to 3.5 times the narrow, give or take
/// [`FIT_TOLERANCE`] of the narrow width after the ink's spread; the narrow width when they are.
fn fits_wide(w: &[f32], wide: &[Option<bool>]) -> Option<f32> {
    let mean = |pick: &dyn Fn(usize, bool) -> bool| {
        let v: Vec<f32> = w
            .iter()
            .zip(wide)
            .enumerate()
            .filter(|(k, (_, f))| f.is_some_and(|f| pick(*k, f)))
            .map(|(_, (&x, _))| x)
            .collect();
        (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
    };
    let narrow_bar = mean(&|k, f| k % 2 == 0 && !f)?;
    let narrow_space = mean(&|k, f| k % 2 == 1 && !f)?;
    let spread = (narrow_bar - narrow_space) / 2.0;
    let narrow = (narrow_bar + narrow_space) / 2.0;
    let fixed = |k: usize, x: f32| {
        if k.is_multiple_of(2) {
            x - spread
        } else {
            x + spread
        }
    };
    let wides: Vec<f32> = w
        .iter()
        .zip(wide)
        .enumerate()
        .filter(|(_, (_, f))| **f == Some(true))
        .map(|(k, (&x, _))| fixed(k, x))
        .collect();
    let wide_w = wides.iter().sum::<f32>() / f32::max(wides.len() as f32, 1.0);
    let ratio = wide_w / narrow;
    if wides.is_empty() || !(2.0 - FIT_TOLERANCE..=3.5 + FIT_TOLERANCE).contains(&ratio) {
        return None;
    }
    let tol = tolerance(narrow);
    let ok = w.iter().zip(wide).enumerate().all(|(k, (&x, f))| {
        let x = fixed(k, x);
        match f {
            Some(true) => (x - wide_w).abs() <= tol * narrow,
            Some(false) => (x - narrow).abs() <= tol * narrow,
            None => x >= (1.0 - tol) * narrow,
        }
    });
    (ok && spread.abs() <= 0.5 * narrow).then_some(narrow)
}

/// Whether the bars and spaces `w` (bar first) from the first are a Code 128, with `first`
/// for the width of the first bar: a start pattern, symbols of 11 modules (6 bars and spaces
/// of 1 to 4 modules), and the stop (13 modules); the number of bars and spaces and the module
/// when they are. The widths looked at are added to `work`.
fn fits_code128(w: &[f32], first: f32, work: &mut usize) -> Option<(usize, f32)> {
    if w.len() < 6 * 3 + 7 {
        return None;
    }
    let width = |k: usize| if k == 0 { first } else { w[k] };
    let start: [f32; 6] = std::array::from_fn(width);
    *work += start.len();
    let module = start.iter().sum::<f32>() / 11.0;
    let as_f = |p: &[u8]| p.iter().map(|&v| f32::from(v)).collect::<Vec<f32>>();
    let close = |got: &[f32], want: &[f32], m: f32| {
        got.iter()
            .zip(want)
            .all(|(&g, &e)| (g / m - e).abs() <= FIT_TOLERANCE + 0.25)
    };
    if !CODE128_STARTS
        .iter()
        .any(|s| close(&start, &as_f(s), module))
    {
        return None;
    }
    // The sum of the widths before `at`, added in order (as a sum over them from the first).
    let mut before: f32 = start.iter().sum();
    let mut at = 6;
    while at + 7 <= w.len() {
        *work += 13;
        let stop = &w[at..at + 7];
        let m = stop.iter().fold(before, |a, &v| a + v) / ((at / 6) * 11 + 13) as f32;
        if at >= 18 && close(stop, &as_f(&CODE128_STOP), m) {
            // The whole: every bar and space whole modules after the ink's spread, symbols of
            // 11.
            let n = at + 7;
            *work += n;
            let w: Vec<f32> = (0..n).map(width).collect();
            let whole = |spread: f32| {
                let mut want: Vec<f32> = w[..at]
                    .iter()
                    .enumerate()
                    .map(|(k, &v)| {
                        let s = if k.is_multiple_of(2) { spread } else { -spread };
                        ((v - s) / m).round().clamp(1.0, 4.0)
                    })
                    .collect();
                want.extend(as_f(&CODE128_STOP));
                want
            };
            let want = whole(spread_of(&w, &whole(0.0), m));
            let sums = want[..at]
                .chunks(6)
                .all(|s| (s.iter().sum::<f32>() - 11.0).abs() < 0.5);
            return (sums && fits_modules(&w, &want).is_some()).then_some((n, m));
        }
        let symbol = &w[at..at + 6];
        let through = symbol.iter().fold(before, |a, &v| a + v);
        let m = through / ((at / 6 + 1) * 11) as f32;
        if (symbol.iter().sum::<f32>() / m - 11.0).abs() > 1.0 {
            return None;
        }
        before = through;
        at += 6;
    }
    None
}

/// Why [`bars_fit`] stopped before it decided: its work passed [`FIT_WORK`] (the code is not
/// taken), or the time for reading codes is over.
enum Halt {
    Work,
    Late,
}

/// The windows on one line (the edges of its dark runs, `edges`) that fit `kind`, with paper
/// for [`FIT_QUIET`] modules at both ends. The render's edges lie at `inside` along the line:
/// an end bar of a code cut there is as wide as it is meant to be, or less.
fn line_fits(
    edges: &[f32],
    len: f32,
    kind: Linear<'_>,
    want: &Want,
    inside: (f32, f32),
    (work, late): (&mut usize, &dyn Fn() -> bool),
) -> Result<Vec<Fit>, Halt> {
    // Bars and spaces from each dark run's start: edge 2k starts bar k.
    let mut out = Vec::new();
    let bars = edges.len() / 2;
    let at_edge = |x: f32, edge: f32| (x - edge).abs() < 0.75;
    // Code 128: the widths of the whole line once, the last bar of a code cut at the end of
    // the render as wide as a stop's (2 modules) at least.
    let line: Vec<f32> = match kind {
        Linear::Code128 => {
            let mut all: Vec<f32> = edges.windows(2).map(|e| e[1] - e[0]).collect();
            if all.len() > 7 && edges.last().is_some_and(|&e| at_edge(e, inside.1)) {
                let k = all.len() - 1;
                all[k] = all[k].max(2.0 * all[k - 6..k].iter().sum::<f32>() / 11.0);
            }
            all
        }
        _ => Vec::new(),
    };
    for first in 0..bars {
        if first % FIT_WINDOW == FIT_WINDOW - 1 && late() {
            return Err(Halt::Late);
        }
        // Each window is charged what it looks at past the [`FIT_WINDOW`] charged for it
        // beforehand.
        let mut looked = 0usize;
        let from = 2 * first;
        let widths = |n: usize| -> Option<Vec<f32>> {
            (from + n < edges.len()).then(|| {
                let mut w: Vec<f32> = (from..from + n).map(|k| edges[k + 1] - edges[k]).collect();
                // The end bars of EAN-13, Code 39 and ITF are one module (narrow).
                let narrow = |w: &[f32], k: usize| -> f32 {
                    match want {
                        Want::Modules(m) => {
                            let rest: f32 = m.iter().sum::<f32>() - m[k];
                            (w.iter().sum::<f32>() - w[k]) / rest
                        }
                        Want::Wide(f) => {
                            let v: Vec<f32> = (0..w.len())
                                .filter(|&j| j != k && j.is_multiple_of(2) && f[j] == Some(false))
                                .map(|j| w[j])
                                .collect();
                            v.iter().sum::<f32>() / f32::max(v.len() as f32, 1.0)
                        }
                    }
                };
                if at_edge(edges[from], inside.0) {
                    w[0] = w[0].max(narrow(&w, 0));
                }
                if at_edge(edges[from + n], inside.1) {
                    let k = n - 1;
                    w[k] = w[k].max(narrow(&w, k));
                }
                w
            })
        };
        let found = match (kind, want) {
            (Linear::Code128, _) => {
                // The end bars of Code 128 are 2 modules: the start's first, the stop's last
                // (above, for the whole line).
                let all = line.get(from..).unwrap_or_default();
                let mut first = all.first().copied().unwrap_or_default();
                if all.len() > 7 && at_edge(edges[from], inside.0) {
                    first = first.max(2.0 * all[1..6].iter().sum::<f32>() / 9.0);
                }
                fits_code128(all, first, &mut looked)
            }
            (_, Want::Modules(m)) => {
                looked += m.len();
                widths(m.len())
                    .and_then(|w| fits_modules(&w, m))
                    .map(|module| (m.len(), module))
            }
            (_, Want::Wide(f)) => {
                looked += f.len();
                widths(f.len())
                    .and_then(|w| fits_wide(&w, f))
                    .map(|module| (f.len(), module))
            }
        };
        *work += looked.saturating_sub(FIT_WINDOW);
        if *work > FIT_WORK {
            return Err(Halt::Work);
        }
        let Some((n, module)) = found else {
            continue;
        };
        let (start, end) = (edges[from], edges[from + n]);
        let before = if from == 0 {
            start
        } else {
            start - edges[from - 1]
        };
        let after = if from + n + 1 >= edges.len() {
            len - end
        } else {
            edges[from + n + 1] - end
        };
        if before >= FIT_QUIET * module && after >= FIT_QUIET * module {
            out.push(Fit { start, end, module });
        }
    }
    Ok(out)
}

/// What a code's bars and spaces must be: whole modules each, or narrow and wide.
enum Want {
    Modules(Vec<f32>),
    Wide(Vec<Option<bool>>),
}

/// Whether the linear code `kind`, read by the decoder in box `b` (pixels: left, top, right,
/// bottom) of the render `rgba` (`size` in pixels), stands there in bars that fit it: on lines
/// along the code, one after the other across its bars for [`FIT_BAR`] modules at least, the
/// bars and spaces are those of the code (EAN-13 and UPC-A: the widths of the digits read;
/// Code 39 and ITF: the narrow and wide of the characters read; Code 128: a start, symbols of
/// 11 modules and the stop), whole modules give or take [`FIT_TOLERANCE`] after the ink's
/// spread, with paper for [`FIT_QUIET`] modules at both ends. The bars may stand up or lie
/// down. A screen, letters or noise the decoder read as a code do not. `None` when `late`
/// says the time for reading codes is over before the check decided.
pub(crate) fn bars_fit(
    rgba: &[u8],
    (w, h): (usize, usize),
    b: [u32; 4],
    kind: Linear<'_>,
    late: &dyn Fn() -> bool,
) -> Option<bool> {
    if rgba.len() < w * h * 4 || w == 0 || h == 0 {
        return Some(false);
    }
    let want = match kind {
        Linear::Ean(d) => match ean_widths(d) {
            Some(m) => Want::Modules(m.into_iter().map(f32::from).collect()),
            None => return Some(false),
        },
        Linear::Code39(t) => match code39_wide(t) {
            Some(f) => Want::Wide(f),
            None => return Some(false),
        },
        Linear::Itf(d) => match itf_wide(d) {
            Some(f) => Want::Wide(f),
            None => return Some(false),
        },
        Linear::Code128 => Want::Modules(Vec::new()),
    };
    for along_y in [false, true] {
        if upright_fit(rgba, (w, h), b, (kind, &want), along_y, late)? {
            return Some(true);
        }
    }
    // Read along a slanted line, the box is that line, a little widened across (by an eighth
    // of its width, two pixels at least): the line runs corner to corner of the rest.
    let (bw, bh) = (
        b[2].saturating_sub(b[0]) as f32,
        b[3].saturating_sub(b[1]) as f32,
    );
    let rise = bh - 2.0 * (bw / 8.0).max(2.0);
    if bw < 8.0 || rise < 1.0 {
        return Some(false);
    }
    for sign in [1.0f32, -1.0] {
        if slanted_fit(rgba, (w, h), b, (kind, &want), (bw, sign * rise), late)? {
            return Some(true);
        }
    }
    Some(false)
}

/// [`bars_fit`] along one way: lines along x (bars standing up), or with `along_y` along y,
/// each line a row (or a column) of pixels.
fn upright_fit(
    rgba: &[u8],
    (w, h): (usize, usize),
    b: [u32; 4],
    (kind, want): (Linear<'_>, &Want),
    along_y: bool,
    late: &dyn Fn() -> bool,
) -> Option<bool> {
    let (len, wide) = if along_y { (h, w) } else { (w, h) };
    let (a0, a1, s0, s1) = if along_y {
        (b[1] as usize, b[3] as usize, b[0] as usize, b[2] as usize)
    } else {
        (b[0] as usize, b[2] as usize, b[1] as usize, b[3] as usize)
    };
    let span = a1.saturating_sub(a0).max(1);
    // Room along the line for the paper at both ends (past the edge of the render, paper, as
    // the decoder takes it); across, the middle of the box: its bars.
    let reach = span / 2 + 8;
    let (c0, c1) = middle((s0, s1.min(wide)));
    if c1 <= c0 || a1 > len {
        return Some(false);
    }
    let gray = |t: usize, s: usize| {
        let Some(t) = (a0 + t).checked_sub(reach).filter(|&t| t < len) else {
            return 255;
        };
        if along_y {
            gray_at(rgba, t * w + s)
        } else {
            gray_at(rgba, s * w + t)
        }
    };
    // Where the render begins and ends along the line.
    let inside = (reach as f32 - a0 as f32, (len + reach) as f32 - a0 as f32);
    lines_fit(
        &gray,
        (span + 2 * reach, inside),
        (c0, c1),
        wide,
        (kind, want),
        late,
    )
}

/// The bars and spaces one window of [`bars_fit`] is counted as, and the most such counts one
/// check takes: a picture of many dark runs along a long line read as a code is not checked
/// to the end (and the code not taken).
const FIT_WINDOW: usize = 64;
const FIT_WORK: usize = 200_000_000;

/// The most lines across the bars of a code read that [`bars_fit`] looks at, the middle ones:
/// enough for [`FIT_BAR`] modules of 32 pixels.
const FIT_LINES: usize = 128;

/// The middle [`FIT_LINES`] of lines `c0..c1`.
fn middle((c0, c1): (usize, usize)) -> (usize, usize) {
    let n = c1.saturating_sub(c0);
    if n <= FIT_LINES {
        return (c0, c1);
    }
    let from = c0 + (n - FIT_LINES) / 2;
    (from, from + FIT_LINES)
}

/// [`bars_fit`] along lines in the way `(dx, dy)` (pixels, y down) through the middle of box
/// `b`, the gray between pixels taken from the four around.
fn slanted_fit(
    rgba: &[u8],
    (w, h): (usize, usize),
    b: [u32; 4],
    (kind, want): (Linear<'_>, &Want),
    (dx, dy): (f32, f32),
    late: &dyn Fn() -> bool,
) -> Option<bool> {
    let n = dx.hypot(dy);
    let (ux, uy) = (dx / n, dy / n);
    let (nx, ny) = (-uy, ux);
    let (cx, cy) = ((b[0] + b[2]) as f32 / 2.0, (b[1] + b[3]) as f32 / 2.0);
    let (bw, bh) = ((b[2] - b[0]) as f32, (b[3] - b[1]) as f32);
    // Along: the line read and half its length past both ends; across: the box's breadth.
    let half = n / 2.0 + n / 2.0 + 8.0;
    let breadth = (((bw * nx).abs() + (bh * ny).abs()) / 2.0 + 2.0).min(FIT_LINES as f32 / 2.0);
    let (len, lines) = ((2.0 * half) as usize, (2.0 * breadth) as usize + 1);
    if lines * len > 4_000_000 {
        return Some(false);
    }
    // Past the edge of the render, paper.
    let pixel = |x: f32, y: f32| {
        if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
            return 255.0;
        }
        f32::from(gray_at(rgba, y as usize * w + x as usize))
    };
    let mut grid = vec![0u8; lines * len];
    for s in 0..lines {
        let off = s as f32 - breadth;
        for t in 0..len {
            let along = t as f32 - half;
            // Pixel centres are at whole numbers plus a half.
            let (x, y) = (
                cx + ux * along + nx * off - 0.5,
                cy + uy * along + ny * off - 0.5,
            );
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            let v = pixel(x0, y0) * (1.0 - fx) * (1.0 - fy)
                + pixel(x0 + 1.0, y0) * fx * (1.0 - fy)
                + pixel(x0, y0 + 1.0) * (1.0 - fx) * fy
                + pixel(x0 + 1.0, y0 + 1.0) * fx * fy;
            grid[s * len + t] = v.round() as u8;
        }
    }
    let gray = |t: usize, s: usize| grid[s * len + t];
    lines_fit(
        &gray,
        (len, (f32::NEG_INFINITY, f32::INFINITY)),
        (0, lines),
        lines,
        (kind, want),
        late,
    )
}

/// Whether lines `across.0..across.1` of `wide`, each `len` grays along (`gray(t, s)`), show
/// `kind` as [`bars_fit`] wants: fits on lines one after the other for [`FIT_BAR`] modules.
/// `None` when `late` says the time is over first.
fn lines_fit(
    gray: &dyn Fn(usize, usize) -> u8,
    (len, inside): (usize, (f32, f32)),
    (c0, c1): (usize, usize),
    wide: usize,
    (kind, want): (Linear<'_>, &Want),
    late: &dyn Fn() -> bool,
) -> Option<bool> {
    // Runs of lines one after the other whose fits go on from line to line.
    let mut work = 0usize;
    let mut prev: Vec<(Fit, usize)> = Vec::new();
    for s in c0..c1 {
        if late() {
            return None;
        }
        // The line alone, and averaged with one or two more on each side: the stair steps of
        // slanted bars in pixels even out.
        let mut fits: Vec<Fit> = Vec::new();
        for reach in 0..=2 {
            let near: Vec<usize> = (s.saturating_sub(reach)..(s + reach + 1).min(wide)).collect();
            let line: Vec<u8> = (0..len)
                .map(|t| {
                    let sum: u32 = near.iter().map(|&q| u32::from(gray(t, q))).sum();
                    (sum / near.len() as u32) as u8
                })
                .collect();
            let Some(e) = line_edges(&line, PIXEL_CONTRAST as u8) else {
                continue;
            };
            // Each dark run starts a window of a code's bars, both ways along the line: past
            // the work a code's check may take, the code is not taken.
            work += e.len() * 2 * FIT_WINDOW;
            if work > FIT_WORK {
                return Some(false);
            }
            // The code may run either way along the line.
            let n = line.len() as f32;
            let back: Vec<f32> = e.iter().rev().map(|x| n - x).collect();
            let back_inside = (n - inside.1, n - inside.0);
            let both = line_fits(&e, n, kind, want, inside, (&mut work, late)).and_then(|ahead| {
                line_fits(&back, n, kind, want, back_inside, (&mut work, late))
                    .map(|back| (ahead, back))
            });
            let (ahead, back) = match both {
                Ok(both) => both,
                Err(Halt::Work) => return Some(false),
                Err(Halt::Late) => return None,
            };
            fits.extend(ahead);
            fits.extend(back.into_iter().map(|f| Fit {
                start: n - f.end,
                end: n - f.start,
                module: f.module,
            }));
        }
        let next: Vec<(Fit, usize)> = fits
            .into_iter()
            .map(|f| {
                let shift = f32::max(1.5, 0.5 * f.module);
                let run = prev
                    .iter()
                    .filter(|(p, _)| {
                        (p.start - f.start).abs() <= shift && (p.end - f.end).abs() <= shift
                    })
                    .map(|(_, n)| n + 1)
                    .max()
                    .unwrap_or(1);
                (f, run)
            })
            .collect();
        // Lines are whole pixels: half a line short of the bar length still is it.
        if next
            .iter()
            .any(|(f, n)| (*n as f32 + 0.5) / f.module >= FIT_BAR)
        {
            return Some(true);
        }
        prev = next;
    }
    Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The modules of a made-up 21 by 21 matrix (a checker-like pattern with finder-like
    /// corners), each `m` points, its lower left corner at (`x`, `y`).
    fn matrix(x: f32, y: f32, m: f32) -> Vec<[f32; 4]> {
        let mut out = Vec::new();
        for row in 0..21 {
            for col in 0..21 {
                let finder = !(7..=13).contains(&row) && !(7..=13).contains(&col);
                let dark = if finder {
                    let (r, c) = (row % 14, col % 14);
                    let ring = r.min(c).min(6 - r.min(6)).min(6 - c.min(6));
                    ring != 1
                } else {
                    (row * 7 + col * 3) % 5 < 2
                };
                if dark {
                    let (x0, y0) = (x + col as f32 * m, y + row as f32 * m);
                    out.push([x0, y0, x0 + m, y0 + m]);
                }
            }
        }
        out
    }

    #[test]
    fn a_matrix_of_modules_is_a_region_with_its_module_size() {
        let cells = matrix(100.0, 200.0, 2.0);
        let found = vector_codes(&cells).expect("searched");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].b, [100.0, 200.0, 142.0, 242.0]);
        assert_eq!(found[0].module, 2.0);
        // The same code elsewhere is the same code.
        let moved = vector_codes(&matrix(300.0, 50.0, 2.0)).expect("searched");
        assert_eq!(moved[0].id, found[0].id);
    }

    #[test]
    fn a_matrix_drawn_as_runs_of_modules_is_one_region() {
        // Generators often join the dark modules of a row (or a column) into one rectangle.
        let modules = matrix(100.0, 200.0, 2.0);
        let runs = |by_row: bool| {
            let mut sorted = modules.clone();
            sorted.sort_by(|a, b| {
                let key = |c: &[f32; 4]| if by_row { (c[1], c[0]) } else { (c[0], c[1]) };
                let (ka, kb) = (key(a), key(b));
                ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
            });
            let mut out: Vec<[f32; 4]> = Vec::new();
            for c in sorted {
                match out.last_mut() {
                    Some(l) if by_row && l[1] == c[1] && l[2] == c[0] => l[2] = c[2],
                    Some(l) if !by_row && l[0] == c[0] && l[3] == c[1] => l[3] = c[3],
                    _ => out.push(c),
                }
            }
            out
        };
        for by_row in [true, false] {
            let cells = runs(by_row);
            assert!(cells.len() < modules.len(), "{by_row}");
            let found = vector_codes(&cells).expect("searched");
            assert_eq!(found.len(), 1, "runs by row {by_row}: {found:?}");
            assert_eq!(
                found[0].b,
                [100.0, 200.0, 142.0, 242.0],
                "runs by row {by_row}"
            );
        }
    }

    #[test]
    fn bars_of_one_height_are_a_linear_region() {
        let mut cells = Vec::new();
        let mut x = 50.0;
        for k in 0..30 {
            let w = if k % 3 == 0 { 1.0 } else { 0.5 };
            cells.push([x, 100.0, x + w, 130.0]);
            x += w + 0.5 + (k % 2) as f32 * 0.5;
        }
        let found = vector_codes(&cells).expect("searched");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].module, 0.5);
    }

    #[test]
    fn table_shading_and_few_boxes_are_not_codes() {
        // Large shaded rows of a table.
        let rows: Vec<[f32; 4]> = (0..40)
            .map(|k| {
                [
                    50.0,
                    100.0 + k as f32 * 20.0,
                    550.0,
                    110.0 + k as f32 * 20.0,
                ]
            })
            .collect();
        assert!(vector_codes(&rows).expect("searched").is_empty());
        // Ten check boxes.
        let boxes: Vec<[f32; 4]> = (0..10)
            .map(|k| [50.0 + k as f32 * 20.0, 100.0, 58.0 + k as f32 * 20.0, 108.0])
            .collect();
        assert!(vector_codes(&boxes).expect("searched").is_empty());
        // A solid block of touching squares covers its whole box.
        let mut solid = Vec::new();
        for r in 0..10 {
            for c in 0..10 {
                solid.push([
                    c as f32 * 2.0,
                    r as f32 * 2.0,
                    c as f32 * 2.0 + 2.0,
                    r as f32 * 2.0 + 2.0,
                ]);
            }
        }
        assert!(vector_codes(&solid).expect("searched").is_empty());
    }

    #[test]
    fn drawings_that_are_not_codes_are_not_regions() {
        let none = |name: &str, cells: Vec<[f32; 4]>| {
            assert!(vector_codes(&cells).expect("searched").is_empty(), "{name}");
        };
        // A checkerboard of 12 by 12 cells of 10 points.
        let mut checker = Vec::new();
        for r in 0..12 {
            for c in 0..12 {
                if (r + c) % 2 == 0 {
                    let (x, y) = (50.0 + c as f32 * 10.0, 50.0 + r as f32 * 10.0);
                    checker.push([x, y, x + 10.0, y + 10.0]);
                }
            }
        }
        none("checkerboard", checker);
        // A heatmap of 20 by 20 cells of 8 points, about half filled.
        let mut heat = Vec::new();
        let mut state = 7u32;
        for r in 0..20 {
            for c in 0..20 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                if state.is_multiple_of(2) {
                    let (x, y) = (50.0 + c as f32 * 8.0, 50.0 + r as f32 * 8.0);
                    heat.push([x, y, x + 8.0, y + 8.0]);
                }
            }
        }
        none("heatmap", heat);
        // 40 bars of one width and one height.
        none(
            "equal bars",
            (0..40)
                .map(|k| [50.0 + k as f32 * 2.0, 100.0, 51.0 + k as f32 * 2.0, 140.0])
                .collect(),
        );
        // The rules of a table: 10 columns and 30 rows of thin lines.
        let mut rules: Vec<[f32; 4]> = (0..11)
            .map(|k| [50.0 + k as f32 * 25.0, 100.0, 50.5 + k as f32 * 25.0, 250.0])
            .collect();
        rules
            .extend((0..31).map(|k| [50.0, 100.0 + k as f32 * 5.0, 300.0, 100.5 + k as f32 * 5.0]));
        none("ruled table", rules);
        // A bar chart: bars of one width, of many heights.
        none(
            "bar chart",
            (0..30)
                .map(|k| {
                    let x = 50.0 + k as f32 * 6.0;
                    [x, 100.0, x + 4.0, 110.0 + (k * 37 % 50) as f32]
                })
                .collect(),
        );
    }

    /// The bars of a Code 128 of `text`, `module` points a module and `bar` points long, its
    /// first module at `at`: standing up, or lying down (`flat`, the first module at the top).
    /// With `one_each`, one rectangle for each dark module column, else one for each bar.
    fn code128(
        text: &str,
        at: (f32, f32),
        module: f32,
        bar: f32,
        flat: bool,
        one_each: bool,
    ) -> Vec<[f32; 4]> {
        let m = barcodes::encode::text(
            barcodes::Symbology::Code128,
            text,
            &barcodes::encode::Options::default(),
        )
        .expect("encode");
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for col in 0..m.width {
            if !m.dark[col as usize] {
                continue;
            }
            match runs.last_mut() {
                Some(r) if !one_each && r.1 == col => r.1 = col + 1,
                _ => runs.push((col, col + 1)),
            }
        }
        runs.into_iter()
            .map(|(a, z)| {
                let (a, z) = (a as f32 * module, z as f32 * module);
                if flat {
                    [at.0, at.1 - z, at.0 + bar, at.1 - a]
                } else {
                    [at.0 + a, at.1, at.0 + z, at.1 + bar]
                }
            })
            .collect()
    }

    /// The box of `cells`.
    fn box_of(cells: &[[f32; 4]]) -> [f32; 4] {
        bounds(cells, &(0..cells.len()).collect::<Vec<_>>()).0
    }

    #[test]
    fn linear_codes_of_any_bar_length_are_regions() {
        // A linear code's bars need not be a whole number of modules long, and short bars are
        // wider than a quarter of their length: each is one region, boxed by its bars.
        for module in [0.5, 0.8, 1.2] {
            for bar in [3.0, 5.0, 8.0, 20.0, 30.0, 40.0, 80.0, 150.0] {
                for flat in [false, true] {
                    for one_each in [false, true] {
                        let cells =
                            code128("0000000001", (350.0, 600.0), module, bar, flat, one_each);
                        let case = format!(
                            "module {module}, bars {bar} long, flat {flat}, one rectangle a module \
                             {one_each}"
                        );
                        let found = vector_codes(&cells).expect("searched");
                        assert_eq!(found.len(), 1, "{case}: {found:?}");
                        let want = box_of(&cells);
                        assert!(
                            found[0]
                                .b
                                .iter()
                                .zip(want)
                                .all(|(a, b)| (a - b).abs() < 0.01),
                            "{case}: {:?} for {want:?}",
                            found[0].b
                        );
                        assert!((found[0].module - module).abs() < 0.01, "{case}: {found:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn rows_of_dashes_dots_and_dividers_are_not_codes() {
        let none = |name: &str, cells: Vec<[f32; 4]>| {
            assert!(vector_codes(&cells).expect("searched").is_empty(), "{name}");
        };
        // A dashed rule of dashes of two lengths, 0.5 points thick.
        let mut x = 50.0;
        let mut dashes = Vec::new();
        for k in 0..40 {
            let w = if k % 3 == 0 { 6.0 } else { 3.0 };
            dashes.push([x, 100.0, x + w, 100.5]);
            x += w + 1.5;
        }
        none("dashed rule", dashes.clone());
        // The same rule standing up.
        none(
            "dashed rule standing",
            dashes.iter().map(|d| [d[1], d[0], d[3], d[2]]).collect(),
        );
        // A dotted leader of squares of 0.5 points.
        none(
            "dotted leader",
            (0..60)
                .map(|k| {
                    let x = 50.0 + k as f32 * 2.0;
                    [x, 100.0, x + 0.5, 100.5]
                })
                .collect(),
        );
        // A leader of thick dots, every fifth one wider.
        none(
            "thick dots",
            (0..60)
                .map(|k| {
                    let x = 50.0 + k as f32 * 3.0;
                    let w = if k % 5 == 0 { 2.0 } else { 1.0 };
                    [x, 100.0, x + w, 101.0]
                })
                .collect(),
        );
        // The dividers of a comb of boxes: thin bars of two widths, 12 points apart.
        none(
            "comb dividers",
            (0..30)
                .map(|k| {
                    let x = 50.0 + k as f32 * 12.0;
                    let w = if k % 4 == 0 { 1.0 } else { 0.5 };
                    [x, 100.0, x + w, 112.0]
                })
                .collect(),
        );
        // Squares of a rating scale, 4 points, 1 apart.
        none(
            "rating squares",
            (0..30)
                .map(|k| {
                    let x = 50.0 + k as f32 * 5.0;
                    [x, 100.0, x + 4.0, 104.0]
                })
                .collect(),
        );
    }

    #[test]
    fn ticks_of_a_ruler_are_no_code() {
        // A ruler, a scale bar or the ticks of a chart axis: short bars of two widths at one
        // pitch, standing up or lying down, with free space at both ends. Bars at one pitch
        // carry nothing; a code's bars never do.
        for (minor, major, long, pitch) in [
            (0.5, 1.0, 3.0, 2.0),
            (0.5, 1.0, 6.0, 2.5),
            (0.3, 0.6, 3.0, 1.2),
        ] {
            let ticks: Vec<[f32; 4]> = (0..100)
                .map(|k| {
                    let x = 60.0 + k as f32 * pitch;
                    let w = if k % 5 == 0 { major } else { minor };
                    // Every other ruler sets its ticks by their right edge.
                    let x = if pitch > 2.0 { x - w } else { x };
                    [x, 700.0, x + w, 700.0 + long]
                })
                .collect();
            let case = format!("ticks {minor} and {major} wide, {long} long, {pitch} apart");
            assert!(vector_codes(&ticks).expect("searched").is_empty(), "{case}");
            let lying: Vec<[f32; 4]> = ticks
                .iter()
                .map(|t| [t[1] - 600.0, t[0], t[3] - 600.0, t[2]])
                .collect();
            assert!(
                vector_codes(&lying).expect("searched").is_empty(),
                "{case}, lying"
            );
        }
    }

    #[test]
    fn twin_codes_a_quiet_zone_apart_are_two_regions() {
        // 5 modules apart (the quiet zone), side by side: each code is its own region.
        let left = code128("0000000001", (100.0, 500.0), 0.8, 20.0, false, true);
        let w = box_of(&left)[2] - 100.0;
        let right = code128(
            "0000000001",
            (100.0 + w + 4.0, 500.0),
            0.8,
            20.0,
            false,
            true,
        );
        let mut both = left.clone();
        both.extend(right.iter().copied());
        let mut found: Vec<[f32; 4]> = vector_codes(&both)
            .expect("searched")
            .iter()
            .map(|c| c.b)
            .collect();
        found.sort_by(|a, b| a[0].total_cmp(&b[0]));
        assert_eq!(found, [box_of(&left), box_of(&right)]);
    }

    #[test]
    fn twin_codes_side_by_side_are_one_region_and_stacked_ones_two() {
        // Two equal codes closer than their quiet zone: side by side they are one drawing that
        // looks like a code (it may not decode, and that is said); one over the other each
        // keeps its own region.
        for gap in [1.0, 2.0, 3.0] {
            let mut side = code128("0000000001", (100.0, 500.0), 0.8, 20.0, false, true);
            let w = box_of(&side)[2] - 100.0;
            side.extend(code128(
                "0000000001",
                (100.0 + w + gap, 500.0),
                0.8,
                20.0,
                false,
                true,
            ));
            let found = vector_codes(&side).expect("searched");
            assert_eq!(found.len(), 1, "beside, {gap} apart: {found:?}");
            assert_eq!(found[0].b, box_of(&side), "beside, {gap} apart");
            for bar in [3.0, 5.0, 20.0] {
                let low = code128("0000000001", (100.0, 500.0), 0.8, bar, false, true);
                let high = code128(
                    "0000000001",
                    (100.0, 500.0 + bar + gap),
                    0.8,
                    bar,
                    false,
                    true,
                );
                let mut both = low.clone();
                both.extend(high.iter().copied());
                let mut found: Vec<[f32; 4]> = vector_codes(&both)
                    .expect("searched")
                    .iter()
                    .map(|c| c.b)
                    .collect();
                found.sort_by(|a, b| a[1].total_cmp(&b[1]));
                assert_eq!(
                    found,
                    [box_of(&low), box_of(&high)],
                    "over, bars {bar} long, {gap} apart"
                );
            }
        }
    }

    #[test]
    fn bars_lying_down_are_a_linear_region() {
        let mut cells = Vec::new();
        let mut y = 50.0;
        for k in 0..30 {
            let h = if k % 3 == 0 { 1.0 } else { 0.5 };
            cells.push([100.0, y, 130.0, y + h]);
            y += h + 0.5 + (k % 2) as f32 * 0.5;
        }
        let found = vector_codes(&cells).expect("searched");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].module, 0.5);
    }

    #[test]
    fn dark_parts_of_a_render_are_rectangles_joined_down() {
        // 12 by 8 pixels of paper (230) with a bar 2 wide and 6 high, a square of 2 that is
        // too small, and a run that changes width from row to row.
        let (w, h) = (12usize, 8usize);
        let mut gray = vec![230u8; w * h];
        for y in 1..7 {
            gray[y * w + 1] = 10;
            gray[y * w + 2] = 10;
        }
        for y in 1..3 {
            gray[y * w + 5] = 10;
            gray[y * w + 6] = 10;
        }
        for y in 4..7 {
            for x in 8..8 + y - 3 {
                gray[y * w + x] = 10;
            }
        }
        let rgba: Vec<u8> = gray.iter().flat_map(|&g| [g, g, g, 255]).collect();
        let mut cells = pixel_cells(&rgba, w, h, 100).expect("within");
        cells.sort_unstable();
        assert_eq!(cells, [[1, 1, 3, 7], [8, 6, 11, 7]]);
        assert_eq!(pixel_cells(&rgba, w, h, 1), None);
        // One tone only: nothing is dark.
        let paper: Vec<u8> = vec![230; w * h * 4];
        assert_eq!(pixel_cells(&paper, w, h, 100), Some(Vec::new()));
    }

    /// A long train of Code 128 start patterns, `symbols` of them at 2 pixels a module, bars
    /// 60 pixels long, with paper of 20 modules on each side and no stop: every start opens a
    /// window that runs on to the end of the line. The render, its size and the code's box.
    fn starts_without_a_stop(symbols: usize) -> (Vec<u8>, (usize, usize), [u32; 4]) {
        let quiet = 40;
        let w = 2 * 11 * symbols + 2 * quiet;
        let h = 80;
        let mut gray = vec![235u8; w * h];
        let mut x = quiet;
        for _ in 0..symbols {
            for (k, &m) in CODE128_STARTS[1].iter().enumerate() {
                let x1 = x + 2 * usize::from(m);
                if k.is_multiple_of(2) {
                    for y in 10..70 {
                        gray[y * w + x..y * w + x1].fill(0);
                    }
                }
                x = x1;
            }
        }
        let rgba: Vec<u8> = gray.iter().flat_map(|&g| [g, g, g, 255]).collect();
        (rgba, (w, h), [quiet as u32, 10, (w - quiet) as u32, 70])
    }

    #[test]
    fn groups_past_the_merge_bound_are_said() {
        // 2,100 small groups of 8 squares each, apart: the largest 2,000 are looked at and the
        // search says it left the others.
        let mut cells = Vec::new();
        for g in 0..2_100 {
            let (x, y) = ((g % 50) as f32 * 20.0, (g / 50) as f32 * 20.0);
            for k in 0..8 {
                let x0 = x + k as f32 * 1.5;
                cells.push([x0, y, x0 + 1.0, y + 1.0]);
            }
        }
        let searched = vector_search(&cells).expect("searched");
        let why = searched.partly.expect("said");
        assert!(why.contains("2000 largest of its 2100 groups"), "{why}");
        // Fewer groups are all looked at.
        cells.truncate(8 * 1_900);
        assert_eq!(vector_search(&cells).expect("searched").partly, None);
    }

    #[test]
    fn a_printed_search_stops_at_the_time_for_reading_codes() {
        let cells = matrix(100.0, 200.0, 2.0);
        let page = [0.0, 0.0, 600.0, 800.0];
        let found = printed_codes(&cells, 0.5, page, &[], &|| false).expect("searched");
        assert_eq!(found.partly, None);
        let late = printed_codes(&cells, 0.5, page, &[], &|| true).expect("searched");
        assert!(late.codes.is_empty());
        assert_eq!(late.partly.as_deref(), Some("the time for reading codes ran out"));
    }

    #[test]
    fn qr_finders_tiled_over_a_large_part_are_counted_in_one_pass() {
        // A part of 392 by 392 modules tiled with finder patterns 8 modules apart: each is
        // found once, and the places tested are charged.
        let (n, pitch) = (392usize, 8usize);
        let mut cells = Vec::new();
        for r0 in (0..n - 7).step_by(pitch) {
            for k0 in (0..n - 7).step_by(pitch) {
                for r in 0..7 {
                    for k in 0..7 {
                        let ring = r.min(k).min(6 - r).min(6 - k);
                        if ring != 1 {
                            let (x, y) = ((k0 + k) as f32, (n - 1 - (r0 + r)) as f32);
                            cells.push([x, y, x + 1.0, y + 1.0]);
                        }
                    }
                }
            }
        }
        let members: Vec<usize> = (0..cells.len()).collect();
        let work = std::cell::Cell::new(0);
        let start = std::time::Instant::now();
        let found = qr_finders(&cells, &members, [0.0, 0.0, n as f32, n as f32], 1.0, &work);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(found, 49 * 49);
        assert_eq!(work.get(), n * n);
    }

    /// [`bars_fit`] with no time limit.
    fn fits(rgba: &[u8], size: (usize, usize), b: [u32; 4], kind: Linear<'_>) -> bool {
        bars_fit(rgba, size, b, kind, &|| false).expect("no time limit")
    }

    #[test]
    fn a_long_train_of_code128_starts_is_checked_within_its_work() {
        // Each window is charged what it looks at: the check ends quickly, and takes no code.
        let (rgba, size, b) = starts_without_a_stop(500);
        let start = std::time::Instant::now();
        assert!(!fits(&rgba, size, b, Linear::Code128));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        // Past the time for reading codes the check stops and says so: it decided nothing.
        let calls = std::cell::Cell::new(0u32);
        let late = || {
            calls.set(calls.get() + 1);
            calls.get() > 3
        };
        assert_eq!(bars_fit(&rgba, size, b, Linear::Code128, &late), None);
        assert!(calls.get() <= 5, "{}", calls.get());
    }

    #[test]
    fn bars_that_fit_the_value_read_are_evidence_of_a_linear_code() {
        // An EAN-13 of 2 pixels a module, bars 20 pixels long, on paper: its bars fit its
        // digits; not another value's or kind's, not when the bars are too short, and not with
        // ink where its paper should be.
        let digits = "4006381333931";
        let widths = ean_widths(digits).expect("digits");
        let render = |bar: usize, quiet: usize, ink: bool| {
            let w = 2 * 95 + 2 * quiet;
            let h = bar + 20;
            let mut gray = vec![235u8; w * h];
            // A dark stroke 2 pixels before the code, where its paper should be.
            if ink {
                for y in 0..h {
                    gray[y * w + quiet - 6..y * w + quiet - 2].fill(0);
                }
            }
            let mut x = quiet;
            for (k, &e) in widths.iter().enumerate() {
                let x1 = x + 2 * usize::from(e);
                if k.is_multiple_of(2) {
                    for y in 10..10 + bar {
                        gray[y * w + x..y * w + x1].fill(0);
                    }
                }
                x = x1;
            }
            let rgba: Vec<u8> = gray.iter().flat_map(|&g| [g, g, g, 255]).collect();
            (
                rgba,
                (w, h),
                [quiet as u32, 10, (quiet + 190) as u32, (10 + bar) as u32],
            )
        };
        let (rgba, size, b) = render(20, 20, false);
        assert!(fits(&rgba, size, b, Linear::Ean(digits)));
        assert!(!fits(&rgba, size, b, Linear::Ean("5901234123457")));
        assert!(!fits(&rgba, size, b, Linear::Itf("12345678")));
        let (rgba, size, b) = render(6, 20, false);
        assert!(!fits(&rgba, size, b, Linear::Ean(digits)), "short bars");
        let (rgba, size, b) = render(20, 20, true);
        assert!(!fits(&rgba, size, b, Linear::Ean(digits)), "no paper");
        // Past the edge of the render is paper, as the decoder takes it.
        let (rgba, size, b) = render(20, 1, false);
        assert!(fits(&rgba, size, b, Linear::Ean(digits)), "at the edge");
        // The tables give the patterns of the standards.
        assert_eq!(widths.len(), 59);
        assert_eq!(widths.iter().map(|&v| u32::from(v)).sum::<u32>(), 95);
        let star = code39_wide("").expect("start and stop");
        assert_eq!(star.len(), 19);
        assert_eq!(star.iter().filter(|f| **f == Some(true)).count(), 6);
        let itf = itf_wide("12").expect("digits");
        assert_eq!(itf.len(), 4 + 10 + 3);
        assert_eq!(itf.iter().filter(|f| **f == Some(true)).count(), 5);
    }

    #[test]
    fn a_lighter_line_across_bars_is_seen() {
        // Bars of 4 pixels with spaces of 4, 40 rows high, softened at their ends, with a
        // lighter row in the middle or none.
        let (w, h) = (64usize, 44usize);
        let render = |dip: Option<u8>| {
            let mut gray = vec![235u8; w * h];
            for y in 2..42 {
                let ink = match y {
                    2 | 41 => 160,
                    3 | 40 => 60,
                    21 => dip.unwrap_or(0),
                    _ => 0,
                };
                for x in (0..w).filter(|x| x / 4 % 2 == 0) {
                    gray[y * w + x] = ink;
                }
            }
            gray.iter()
                .flat_map(|&g| [g, g, g, 255])
                .collect::<Vec<u8>>()
        };
        let b = [0, 2, 64, 42];
        assert!(!crossed(&render(None), (w, h), b));
        assert!(crossed(&render(Some(80)), (w, h), b));
        // Too faint a line to tell.
        assert!(!crossed(&render(Some(20)), (w, h), b));
        // The same bars lying down.
        let turned = |rgba: Vec<u8>| {
            let mut out = vec![0u8; rgba.len()];
            for y in 0..h {
                for x in 0..w {
                    let (s, d) = ((y * w + x) * 4, (x * h + y) * 4);
                    out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
                }
            }
            out
        };
        assert!(crossed(&turned(render(Some(80))), (h, w), [2, 0, 42, 64]));
        assert!(!crossed(&turned(render(None)), (h, w), [2, 0, 42, 64]));
    }
}
