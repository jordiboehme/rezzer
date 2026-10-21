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

/// Grid cells of the search, in points.
const GRID: f32 = 12.0;

/// Rectangles a grid cell compares with, at most.
const PER_CELL: usize = 64;

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// The regions among `cells` (filled axis-aligned rectangles: x0, y0, x1, y1) that look like
/// a code: at least [`MIN_CELLS`] small rectangles close to each other, that cover between a
/// quarter and three quarters of their box, the box at least 8 modules on its short side for
/// a matrix code, or bars of one height for a linear code. At most [`MAX_REGIONS`], the ones
/// with the most rectangles first.
pub(crate) fn vector_codes(cells: &[[f32; 4]]) -> Vec<VectorCode> {
    let cells: Vec<[f32; 4]> = cells
        .iter()
        .copied()
        .filter(|c| {
            let (w, h) = (c[2] - c[0], c[3] - c[1]);
            w.min(h) >= MIN_MODULE * 0.9 && w.min(h) <= MAX_MODULE && w.max(h) <= MAX_BAR
        })
        .collect();
    if cells.len() < MIN_CELLS {
        return Vec::new();
    }
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    let key = |x: f32, y: f32| ((x / GRID).floor() as i32, (y / GRID).floor() as i32);
    for (i, c) in cells.iter().enumerate() {
        let (a, b) = (key(c[0], c[1]), key(c[2], c[3]));
        for gx in a.0..=b.0.min(a.0 + 30) {
            for gy in a.1..=b.1.min(a.1 + 30) {
                grid.entry((gx, gy)).or_default().push(i);
            }
        }
    }
    let mut parent: Vec<usize> = (0..cells.len()).collect();
    // Modules of a matrix code touch or nearly touch. Bars of a linear code stand up to four
    // modules apart (its widest space), next to bars of the same height.
    let bar = |c: &[f32; 4]| c[3] - c[1] >= 4.0 * (c[2] - c[0]);
    let reach_of = |c: &[f32; 4]| {
        let short = (c[2] - c[0]).min(c[3] - c[1]);
        short * if bar(c) { 4.5 } else { 1.5 } + 0.01
    };
    for (i, c) in cells.iter().enumerate() {
        let reach = reach_of(c);
        let (a, b) = (
            key(c[0] - reach, c[1] - reach),
            key(c[2] + reach, c[3] + reach),
        );
        for gx in a.0..=b.0.min(a.0 + 32) {
            for gy in a.1..=b.1.min(a.1 + 32) {
                let Some(list) = grid.get(&(gx, gy)) else {
                    continue;
                };
                for &j in list.iter().take(PER_CELL) {
                    if j <= i {
                        continue;
                    }
                    let d = cells[j];
                    let gap_x = (d[0] - c[2]).max(c[0] - d[2]);
                    let gap_y = (d[1] - c[3]).max(c[1] - d[3]);
                    let joined = if bar(c) && bar(&d) {
                        // Bars side by side: the same rows, a space between.
                        let same = (c[1] - d[1]).abs() <= 0.1 * (c[3] - c[1])
                            && (c[3] - d[3]).abs() <= 0.1 * (c[3] - c[1]);
                        same && gap_x <= reach.max(reach_of(&d))
                    } else {
                        let near = reach.min(reach_of(&d));
                        gap_x <= near && gap_y <= near
                    };
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
    let mut found: Vec<(usize, VectorCode)> = Vec::new();
    for members in groups.into_values() {
        if members.len() < MIN_CELLS {
            continue;
        }
        if let Some(code) = region(&cells, &members) {
            found.push((members.len(), code));
        }
    }
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.b[3].total_cmp(&a.1.b[3]))
            .then(a.1.b[0].total_cmp(&b.1.b[0]))
    });
    found.truncate(MAX_REGIONS);
    found.into_iter().map(|(_, c)| c).collect()
}

/// The code region of `members` of `cells`, when it looks like one.
fn region(cells: &[[f32; 4]], members: &[usize]) -> Option<VectorCode> {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut area = 0.0f32;
    let mut module = f32::MAX;
    let mut bars = 0usize;
    for &i in members {
        let c = cells[i];
        b = [
            b[0].min(c[0]),
            b[1].min(c[1]),
            b[2].max(c[2]),
            b[3].max(c[3]),
        ];
        let (w, h) = (c[2] - c[0], c[3] - c[1]);
        area += w * h;
        module = module.min(w.min(h));
        if h >= 4.0 * w {
            bars += 1;
        }
    }
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    if w.max(h) > MAX_SIDE || module < MIN_MODULE {
        return None;
    }
    let cover = area / (w * h).max(f32::MIN_POSITIVE);
    if !(0.25..=0.75).contains(&cover) {
        return None;
    }
    // A linear code: most rectangles are bars, and the bars are about as high as the region.
    let linear = bars * 10 >= members.len() * 8;
    if linear {
        let tall = members
            .iter()
            .filter(|&&i| cells[i][3] - cells[i][1] >= 0.6 * h)
            .count();
        if tall * 10 < members.len() * 8 {
            return None;
        }
    } else if w.min(h) < 8.0 * module {
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
        let found = vector_codes(&cells);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].b, [100.0, 200.0, 142.0, 242.0]);
        assert_eq!(found[0].module, 2.0);
        // The same code elsewhere is the same code.
        let moved = vector_codes(&matrix(300.0, 50.0, 2.0));
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
        let found = vector_codes(&cells);
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
        assert!(vector_codes(&rows).is_empty());
        // Ten check boxes.
        let boxes: Vec<[f32; 4]> = (0..10)
            .map(|k| [50.0 + k as f32 * 20.0, 100.0, 58.0 + k as f32 * 20.0, 108.0])
            .collect();
        assert!(vector_codes(&boxes).is_empty());
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
        assert!(vector_codes(&solid).is_empty());
    }
}
