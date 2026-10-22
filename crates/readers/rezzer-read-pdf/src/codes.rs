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

/// The most code regions of one page.
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
/// start bars). A linear code has bars of one height and at least two widths, standing up or
/// lying down, with a quiet zone of 5 modules at both ends. Groups in one row band a few
/// modules apart are one region (a PDF417 has light columns through all its rows). At most
/// [`MAX_REGIONS`], the ones with the most rectangles first. `Err` says why the page was not
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
                        (Shape::Up, Shape::Up) => {
                            let hh = c[3] - c[1];
                            let same =
                                (c[1] - d[1]).abs() <= 0.1 * hh && (c[3] - d[3]).abs() <= 0.1 * hh;
                            joined |= same && gap_x <= far;
                        }
                        (Shape::Flat, Shape::Flat) => {
                            let ww = c[2] - c[0];
                            let same =
                                (c[0] - d[0]).abs() <= 0.1 * ww && (c[2] - d[2]).abs() <= 0.1 * ww;
                            joined |= same && gap_y <= far;
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
    for members in groups {
        if members.len() < MIN_CELLS {
            continue;
        }
        if let Some(code) = region(&cells, &members, &|b| {
            quiet(&cells, &grid_of, &members, b, &key)
        }) {
            found.push((members.len(), code));
        }
    }
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.b[3].total_cmp(&a.1.b[3]))
            .then(a.1.b[0].total_cmp(&b.1.b[0]))
    });
    found.truncate(MAX_REGIONS);
    Ok(found.into_iter().map(|(_, c)| c).collect())
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
/// one above the other, merged.
fn merge_bands(cells: &[[f32; 4]], groups: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..groups.len()).collect();
    let info: Vec<([f32; 4], f32)> = groups.iter().map(|g| bounds(cells, g)).collect();
    for i in 0..groups.len() {
        for j in i + 1..groups.len() {
            let ((a, ma), (b, mb)) = (info[i], info[j]);
            let m = ma.min(mb);
            let rows = (a[1] - b[1]).abs() <= m && (a[3] - b[3]).abs() <= m;
            let cols = (a[0] - b[0]).abs() <= m && (a[2] - b[2]).abs() <= m;
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
    // Every edge on one lattice of the module size, from the region's corner.
    let on = |v: f32, o: f32| {
        let k = (v - o) / module;
        (k - k.round()).abs() <= 0.2
    };
    let lattice = members
        .iter()
        .filter(|&&i| {
            let c = cells[i];
            on(c[0], b[0]) && on(c[2], b[0]) && on(c[1], b[1]) && on(c[3], b[1])
        })
        .count();
    if lattice * 10 < members.len() * 9 {
        return None;
    }
    let up = members
        .iter()
        .filter(|&&i| shape(&cells[i]) == Shape::Up)
        .count();
    let flat = members
        .iter()
        .filter(|&&i| shape(&cells[i]) == Shape::Flat)
        .count();
    let kind_ok = if up * 10 >= members.len() * 8 {
        linear(cells, members, b, module, false, quiet)
    } else if flat * 10 >= members.len() * 8 && linear(cells, members, b, module, true, quiet) {
        true
    } else {
        matrix(cells, members, b, module)
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
/// code: about as long as the region, of at least two widths, with 5 modules free at both
/// ends.
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
