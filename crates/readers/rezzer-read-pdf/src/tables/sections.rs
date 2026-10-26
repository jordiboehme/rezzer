// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sections of a connected grid of drawn rules. A form draws one grid over several
//! sections with column sets of their own (a field grid over an earnings table), so the
//! grid's column edges are those of all its sections. Each section is judged as a grid of
//! its own.

use super::SNAP;
use crate::regions::{Seg, clustered};

/// Whether every value of `inner` lies within [`SNAP`] of one of `outer`. Both are sorted.
fn holds(outer: &[f32], inner: &[f32]) -> bool {
    inner.iter().all(|x| {
        let k = outer.partition_point(|o| *o < x - SNAP);
        outer.get(k).is_some_and(|o| (o - x).abs() <= SNAP)
    })
}

/// The sections of the grid of horizontal segments `h` and vertical segments `v`, top
/// first. The grid is cut at a horizontal rule that crosses its whole width (within
/// [`SNAP`]) where neither the column edges of the section above nor those of the band right
/// below hold the other: a header whose cells span the body's columns stays with its body.
/// Bands are the strips between two such rules; the section above has the edges of all its
/// bands, so a row with cells joined differently from its neighbours stays too. At a cut,
/// the last bands of the section above that are one cell across the whole width (a title
/// band) go down with it, as a title stands above its table; a band with column edges of
/// its own (a total row) stays with the section above. The horizontal rule of a cut belongs to both sections,
/// vertical segments are clipped to their section. A grid without a cut is returned whole.
pub(super) fn sections(h: &[Seg], v: &[Seg]) -> Vec<(Vec<Seg>, Vec<Seg>)> {
    let whole = || vec![(h.to_vec(), v.to_vec())];
    let (Some(left), Some(right)) = (
        h.iter().map(|s| s.from).reduce(f32::min),
        h.iter().map(|s| s.to).reduce(f32::max),
    ) else {
        return whole();
    };
    let bottom = h.iter().map(|s| s.at).chain(v.iter().map(|s| s.from));
    let top = h.iter().map(|s| s.at).chain(v.iter().map(|s| s.to));
    let (bottom, top) = (
        bottom.fold(f32::MAX, f32::min),
        top.fold(f32::MIN, f32::max),
    );
    // The band edges from the bottom: the grid's ends and the rules across its width.
    let mut ys: Vec<f32> = h
        .iter()
        .filter(|s| s.from <= left + SNAP && s.to >= right - SNAP)
        .map(|s| s.at)
        .filter(|y| *y > bottom + SNAP && *y < top - SNAP)
        .collect();
    ys.push(bottom);
    ys.push(top);
    let ys = clustered(ys);
    let bands = ys.len() - 1;
    if bands < 2 {
        return whole();
    }
    // The bands each vertical segment reaches into, from the bottom, and the column edges of
    // each band. A segment reaches into no more bands than the rules it crossed when the
    // grid was found, so this is bounded by that search.
    let reach = |s: &Seg| {
        let first = ys
            .partition_point(|y| *y <= s.from + SNAP)
            .saturating_sub(1);
        first..ys.partition_point(|y| *y < s.to - SNAP).min(bands)
    };
    let mut edges: Vec<Vec<f32>> = vec![Vec::new(); bands];
    for s in v {
        for k in reach(s) {
            edges[k].push(s.at);
        }
    }
    let edges: Vec<Vec<f32>> = edges.into_iter().map(clustered).collect();
    // The section of each band, counted from the top: the edges of the section above (when
    // one set holds the other, the larger is their union) against those of the band below.
    let mut section = vec![0usize; bands];
    let mut start = bands - 1;
    let mut above = &edges[start];
    let mut n = 0;
    for k in (0..bands - 1).rev() {
        let below = &edges[k];
        if holds(above, below) {
            section[k] = n;
            continue;
        }
        if holds(below, above) {
            above = below;
            section[k] = n;
            continue;
        }
        // The section's last bands that are a title across the width go down with it.
        let mut cut = k + 1;
        while cut < start && edges[cut].len() <= 2 && holds(below, &edges[cut]) {
            cut += 1;
        }
        n += 1;
        section[k..cut].fill(n);
        above = below;
        start = cut - 1;
    }
    if n == 0 {
        return whole();
    }
    // Each section's extent, and its segments: a horizontal one goes to the sections whose
    // extent holds it within SNAP, a vertical one is clipped to each section it reaches into.
    let mut extent = vec![(f32::MAX, f32::MIN); n + 1];
    for (k, &m) in section.iter().enumerate() {
        extent[m] = (extent[m].0.min(ys[k]), extent[m].1.max(ys[k + 1]));
    }
    let mut out: Vec<(Vec<Seg>, Vec<Seg>)> = vec![(Vec::new(), Vec::new()); n + 1];
    for s in h {
        let k = ys.partition_point(|y| *y <= s.at).clamp(1, bands) - 1;
        let mut last = None;
        for b in [k.checked_sub(1), Some(k), Some(k + 1)]
            .into_iter()
            .flatten()
        {
            let Some(&m) = section.get(b) else { continue };
            let (lo, hi) = extent[m];
            if last != Some(m) && s.at >= lo - SNAP && s.at <= hi + SNAP {
                out[m].0.push(*s);
                last = Some(m);
            }
        }
    }
    for s in v {
        let mut last = None;
        for k in reach(s) {
            let m = section[k];
            if last != Some(m) {
                let (lo, hi) = extent[m];
                out[m].1.push(Seg {
                    at: s.at,
                    from: s.from.max(lo),
                    to: s.to.min(hi),
                });
                last = Some(m);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid section: horizontal rules across `xs` at each of `ys`, vertical rules at each
    /// of `xs` from the lowest to the highest of `ys`.
    fn section(xs: &[f32], ys: &[f32]) -> (Vec<Seg>, Vec<Seg>) {
        let (x0, x1) = (xs[0], xs[xs.len() - 1]);
        let (y0, y1) = (ys[0], ys[ys.len() - 1]);
        let h = ys
            .iter()
            .map(|&at| Seg {
                at,
                from: x0,
                to: x1,
            })
            .collect();
        let v = xs
            .iter()
            .map(|&at| Seg {
                at,
                from: y0,
                to: y1,
            })
            .collect();
        (h, v)
    }

    fn stacked(parts: &[(Vec<f32>, Vec<f32>)]) -> (Vec<Seg>, Vec<Seg>) {
        let (mut h, mut v) = (Vec::new(), Vec::new());
        for (xs, ys) in parts {
            let (a, b) = section(xs, ys);
            h.extend(a);
            v.extend(b);
        }
        (h, v)
    }

    /// The (bottom, top) of each section and its column edges.
    fn shape(parts: &[(Vec<Seg>, Vec<Seg>)]) -> Vec<((f32, f32), Vec<f32>)> {
        parts
            .iter()
            .map(|(h, v)| {
                let lo = h.iter().map(|s| s.at).fold(f32::MAX, f32::min);
                let hi = h.iter().map(|s| s.at).fold(f32::MIN, f32::max);
                for s in v {
                    assert!(s.from >= lo && s.to <= hi, "{s:?} outside {lo}..{hi}");
                }
                ((lo, hi), clustered(v.iter().map(|s| s.at).collect()))
            })
            .collect()
    }

    #[test]
    fn different_column_sets_are_cut() {
        let (h, v) = stacked(&[
            (vec![0.0, 100.0, 200.0], vec![80.0, 90.0, 100.0]),
            (vec![0.0, 50.0, 150.0, 200.0], vec![50.0, 60.0, 70.0, 80.0]),
        ]);
        assert_eq!(
            shape(&sections(&h, &v)),
            [
                ((80.0, 100.0), vec![0.0, 100.0, 200.0]),
                ((50.0, 80.0), vec![0.0, 50.0, 150.0, 200.0]),
            ]
        );
    }

    #[test]
    fn header_of_joined_cells_stays_with_its_body() {
        let (h, v) = stacked(&[
            (vec![0.0, 100.0, 200.0], vec![80.0, 90.0]),
            (
                vec![0.0, 50.0, 100.0, 150.0, 200.0],
                vec![50.0, 60.0, 70.0, 80.0],
            ),
        ]);
        assert_eq!(sections(&h, &v).len(), 1);
    }

    #[test]
    fn rows_joined_differently_stay_one_section() {
        // Every band has all outer edges; the inner ones differ from band to band, each
        // held by the edges of the bands above.
        let (h, v) = stacked(&[
            (vec![0.0, 50.0, 100.0, 150.0, 200.0], vec![90.0, 100.0]),
            (vec![0.0, 50.0, 150.0, 200.0], vec![80.0, 90.0]),
            (vec![0.0, 100.0, 200.0], vec![70.0, 80.0]),
        ]);
        assert_eq!(sections(&h, &v).len(), 1);
    }

    #[test]
    fn a_title_band_goes_down_with_its_table() {
        // A field section of 3 columns, a title band across the width, then a table of 4
        // columns: the title band heads the table.
        let (h, v) = stacked(&[
            (vec![0.0, 70.0, 130.0, 200.0], vec![80.0, 90.0, 100.0]),
            (vec![0.0, 200.0], vec![70.0, 80.0]),
            (
                vec![0.0, 50.0, 100.0, 150.0, 200.0],
                vec![40.0, 50.0, 60.0, 70.0],
            ),
        ]);
        assert_eq!(
            shape(&sections(&h, &v)),
            [
                ((80.0, 100.0), vec![0.0, 70.0, 130.0, 200.0]),
                ((40.0, 80.0), vec![0.0, 50.0, 100.0, 150.0, 200.0]),
            ]
        );
    }

    #[test]
    fn a_total_row_stays_with_its_table() {
        // A table of 3 columns whose last row joins its first two cells (a total), over a
        // table of 3 other columns that shares the amount column: the total row stays.
        let (h, v) = stacked(&[
            (vec![0.0, 100.0, 150.0, 200.0], vec![80.0, 90.0, 100.0]),
            (vec![0.0, 150.0, 200.0], vec![70.0, 80.0]),
            (vec![0.0, 50.0, 150.0, 200.0], vec![40.0, 50.0, 60.0, 70.0]),
        ]);
        let got = shape(&sections(&h, &v));
        let spans: Vec<(f32, f32)> = got.iter().map(|g| g.0).collect();
        assert_eq!(spans, [(70.0, 100.0), (40.0, 70.0)]);
    }

    #[test]
    fn three_sections() {
        let (h, v) = stacked(&[
            (vec![0.0, 100.0, 200.0], vec![80.0, 90.0, 100.0]),
            (vec![0.0, 30.0, 170.0, 200.0], vec![60.0, 70.0, 80.0]),
            (vec![0.0, 70.0, 130.0, 200.0], vec![40.0, 50.0, 60.0]),
        ]);
        let got = shape(&sections(&h, &v));
        let spans: Vec<(f32, f32)> = got.iter().map(|g| g.0).collect();
        assert_eq!(spans, [(80.0, 100.0), (60.0, 80.0), (40.0, 60.0)]);
    }

    #[test]
    fn a_grid_without_rules_across_is_whole() {
        let (h, v) = section(&[0.0, 100.0, 200.0], &[0.0, 10.0, 20.0]);
        assert_eq!(sections(&h, &v).len(), 1);
        assert!(sections(&[], &v).len() == 1);
    }
}
