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

/// A region larger than this on a side, in points, is not a code.
const MAX_SIDE: f32 = 500.0;

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

/// Groups of at least this many rectangles are looked at for merging, at most this many.
const MERGE_CELLS: usize = 8;
const MERGE_GROUPS: usize = 2_000;

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
/// searched: its work would pass [`MAX_ENTRIES`] or [`MAX_PAIRS`].
pub(crate) fn vector_codes(cells: &[[f32; 4]]) -> Result<Vec<VectorCode>, String> {
    let cells: Vec<[f32; 4]> = cells
        .iter()
        .copied()
        .filter(|c| {
            let (w, h) = (c[2] - c[0], c[3] - c[1]);
            w.min(h) >= MIN_MODULE * 0.9 && w.min(h) <= MAX_MODULE && w.max(h) <= MAX_BAR
        })
        .collect();
    if cells.len() < MIN_CELLS {
        return Ok(Vec::new());
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
    groups.truncate(MERGE_GROUPS);
    let groups = merge_bands(&cells, groups);
    let mut found: Vec<(usize, VectorCode)> = Vec::new();
    let look = |members: &[usize], found: &mut Vec<(usize, VectorCode)>| match region(
        &cells,
        members,
        &|b| quiet(&cells, &grid_of, members, b, &key),
    ) {
        Some(code) => {
            found.push((members.len(), code));
            true
        }
        None => false,
    };
    for members in groups {
        if members.len() < MIN_CELLS || look(&members, &mut found) {
            continue;
        }
        // Short bars of codes one over the other are close enough to join as modules: each
        // band of them may be a code of its own.
        for flat in [false, true] {
            let mut any = false;
            for part in band_parts(&cells, &members, flat) {
                if part.len() >= MIN_CELLS {
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
    Ok(found.into_iter().map(|(_, c)| c).collect())
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
    quiet: &dyn Fn([f32; 4]) -> bool,
) -> Option<VectorCode> {
    let (b, module) = bounds(cells, members);
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    if w.max(h) > MAX_SIDE || module < MIN_MODULE {
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
    let kind_ok = if bars(cells, members, false) {
        linear(cells, members, b, module, false, quiet)
    } else if bars(cells, members, true) && linear(cells, members, b, module, true, quiet) {
        true
    } else {
        lattice() && matrix(cells, members, b, module)
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
    })
}

/// Whether bars `members` (standing up, or lying down with `flat`) in box `b` are a linear
/// code: about as long as the region and at least two modules long, of at least two widths
/// and at most 4 modules wide, every bar and nearly every space a whole number of modules
/// wide, not all at one pitch, with 5 modules free at both ends.
fn linear(
    cells: &[[f32; 4]],
    members: &[usize],
    b: [f32; 4],
    module: f32,
    flat: bool,
    quiet: &dyn Fn([f32; 4]) -> bool,
) -> bool {
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
    // Bars at least two modules long; every bar and space a whole number of modules wide,
    // but for a few spaces (codes side by side closer than their quiet zone).
    if members
        .iter()
        .any(|&i| long(&cells[i]) < 2.0 * module - 0.01)
    {
        return false;
    }
    let whole = |v: f32| {
        let k = v / module;
        k.round() >= 1.0 && (k - k.round()).abs() <= 0.2
    };
    if runs.iter().any(|(a, z)| !whole(z - a)) {
        return false;
    }
    let spaces = runs.windows(2).filter(|w| !whole(w[1].0 - w[0].1)).count();
    if spaces * 10 > runs.len().saturating_sub(1) {
        return false;
    }
    // A code's bars are at most 4 modules wide and never at one pitch: bars that start at
    // one step after another (a ruler, a scale bar, the ticks of an axis, hatching) carry
    // nothing.
    if runs.iter().any(|(a, z)| (z - a) / module > 4.2) {
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
    let mut widths: Vec<i32> = runs
        .iter()
        .map(|(a, z)| ((z - a) / module).round() as i32)
        .collect();
    widths.sort_unstable();
    widths.dedup();
    if widths.len() < 2 {
        return false;
    }
    let q = 5.0 * module;
    if flat {
        quiet([b[0], b[1] - q, b[2], b[1] - 0.01]) && quiet([b[0], b[3] + 0.01, b[2], b[3] + q])
    } else {
        quiet([b[0] - q, b[1], b[0] - 0.01, b[3]]) && quiet([b[2] + 0.01, b[1], b[2] + q, b[3]])
    }
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
}
