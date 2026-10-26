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

/// A rule across a grid may leave gaps where another group of rules (a nested block) has
/// its own rule at the same height; together they cross the grid when the gaps are at most
/// this share of its width.
const CUT_GAPS: f32 = 0.05;

/// Pieces of the page's horizontal rules the sections of one page may look at to complete a
/// rule across a grid; past them a rule with gaps no longer cuts.
const CUT_WORK: usize = 1_000_000;

/// Whether the rules at height `y` cross `left..right`: the pieces of `page` (sorted by
/// height) that lie within [`SNAP`] of `y`, together reaching both ends and leaving gaps of
/// at most [`CUT_GAPS`] of the width.
fn crosses(page: &[Seg], y: f32, left: f32, right: f32, work: &mut usize) -> bool {
    let lo = page.partition_point(|s| s.at < y - SNAP);
    let mut pieces: Vec<(f32, f32)> = Vec::new();
    for s in &page[lo..] {
        if s.at > y + SNAP {
            break;
        }
        *work += 1;
        if *work > CUT_WORK {
            return false;
        }
        if s.to > left && s.from < right {
            pieces.push((s.from.max(left), s.to.min(right)));
        }
    }
    pieces.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut reach, mut gaps) = (left, 0.0f32);
    for (from, to) in pieces {
        if from > reach + SNAP {
            gaps += from - reach;
        }
        reach = reach.max(to);
    }
    reach >= right - SNAP && gaps <= CUT_GAPS * (right - left)
}

/// The sections of the grid of horizontal segments `h` and vertical segments `v`, top
/// first. `page` holds every horizontal rule of the page, sorted by height, and `work`
/// counts the pieces of it looked at (see [`CUT_WORK`]).
///
/// The grid is cut at a horizontal rule that crosses its whole width (within [`SNAP`]; a
/// rule with gaps where a nested block draws its own rule at the same height counts too)
/// where neither the column edges of the section above nor those of the band right below
/// hold the other: a header whose cells span the body's columns stays with its body. Bands
/// are the strips between two such rules; the section above has the edges of all its bands,
/// so a row with cells joined differently from its neighbours stays too. A band whose inner
/// column rules all hang from its top and end at one height (a header row over lines
/// without rules) always starts a section. A band of one cell across the whole width (a note
/// or a title band) stays with the section above. The horizontal rule of a cut belongs to
/// both sections, vertical segments are clipped to their section. A grid without a cut is
/// returned whole.
pub(super) fn sections(
    h: &[Seg],
    v: &[Seg],
    page: &[Seg],
    work: &mut usize,
) -> Vec<(Vec<Seg>, Vec<Seg>)> {
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
    let inner: Vec<f32> = clustered(
        h.iter()
            .map(|s| s.at)
            .filter(|y| *y > bottom + SNAP && *y < top - SNAP)
            .collect(),
    );
    let mut ys: Vec<f32> = inner
        .into_iter()
        .filter(|&y| {
            h.iter()
                .any(|s| (s.at - y).abs() <= SNAP && s.from <= left + SNAP && s.to >= right - SNAP)
                || crosses(page, y, left, right, work)
        })
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
    // For each band, the feet of its inner column rules when all of them hang from its top,
    // and whether one does not.
    let mut feet: Vec<Vec<f32>> = vec![Vec::new(); bands];
    let mut standing = vec![false; bands];
    for s in v {
        let outer = s.at <= left + SNAP || s.at >= right - SNAP;
        for k in reach(s) {
            edges[k].push(s.at);
            if outer {
                continue;
            }
            if s.to >= ys[k + 1] - SNAP && s.from > ys[k] + SNAP {
                feet[k].push(s.from);
            } else {
                standing[k] = true;
            }
        }
    }
    let edges: Vec<Vec<f32>> = edges.into_iter().map(clustered).collect();
    let hangs = |k: usize| !standing[k] && clustered(feet[k].clone()).len() == 1;
    // The section of each band, counted from the top: the edges of the section above (when
    // one set holds the other, the larger is their union) against those of the band below.
    let mut section = vec![0usize; bands];
    let mut above = &edges[bands - 1];
    let mut n = 0;
    for k in (0..bands - 1).rev() {
        let below = &edges[k];
        if !hangs(k) {
            if holds(above, below) {
                section[k] = n;
                continue;
            }
            if holds(below, above) {
                above = below;
                section[k] = n;
                continue;
            }
        }
        n += 1;
        section[k] = n;
        above = below;
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

    /// The sections of `h` and `v` with no other rules on the page.
    fn cut(h: &[Seg], v: &[Seg]) -> Vec<(Vec<Seg>, Vec<Seg>)> {
        let mut page = h.to_vec();
        page.sort_by(|a, b| a.at.total_cmp(&b.at));
        sections(h, v, &page, &mut 0)
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
            shape(&cut(&h, &v)),
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
        assert_eq!(cut(&h, &v).len(), 1);
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
        assert_eq!(cut(&h, &v).len(), 1);
    }

    #[test]
    fn a_note_row_stays_with_the_section_above() {
        // A field section of 3 columns, a band of one cell across the width, then a table of
        // 4 columns: the band is the last row of the section above.
        let (h, v) = stacked(&[
            (vec![0.0, 70.0, 130.0, 200.0], vec![80.0, 90.0, 100.0]),
            (vec![0.0, 200.0], vec![70.0, 80.0]),
            (
                vec![0.0, 50.0, 100.0, 150.0, 200.0],
                vec![40.0, 50.0, 60.0, 70.0],
            ),
        ]);
        assert_eq!(
            shape(&cut(&h, &v)),
            [
                ((70.0, 100.0), vec![0.0, 70.0, 130.0, 200.0]),
                ((40.0, 70.0), vec![0.0, 50.0, 100.0, 150.0, 200.0]),
            ]
        );
    }

    #[test]
    fn a_header_of_hanging_rules_starts_a_section() {
        // A table of 3 columns over a band whose one inner rule hangs from its top, at an
        // edge of the table above: the band starts a section all the same.
        let (mut h, mut v) = section(&[0.0, 100.0, 150.0, 200.0], &[80.0, 90.0, 100.0]);
        h.push(Seg {
            at: 40.0,
            from: 0.0,
            to: 200.0,
        });
        for at in [0.0, 200.0] {
            v.push(Seg {
                at,
                from: 40.0,
                to: 80.0,
            });
        }
        v.push(Seg {
            at: 100.0,
            from: 69.0,
            to: 80.0,
        });
        let spans: Vec<(f32, f32)> = shape(&cut(&h, &v)).iter().map(|g| g.0).collect();
        assert_eq!(spans, [(80.0, 100.0), (40.0, 80.0)]);
    }

    #[test]
    fn a_rule_completed_by_a_nested_block_cuts() {
        // The rule between two sections leaves the left 40 % to a nested block, which draws
        // its own rule at that height with a small gap: the rule cuts. Without the nested
        // rule it does not.
        let (mut h, v) = stacked(&[
            (vec![0.0, 100.0, 200.0], vec![80.0, 90.0, 100.0]),
            (vec![0.0, 50.0, 150.0, 200.0], vec![50.0, 60.0, 70.0, 80.0]),
        ]);
        h.retain(|s| s.at != 80.0);
        h.push(Seg {
            at: 80.0,
            from: 85.0,
            to: 200.0,
        });
        let mut page = h.clone();
        assert_eq!(cut(&h, &v).len(), 1);
        page.push(Seg {
            at: 80.2,
            from: 0.0,
            to: 80.0,
        });
        page.sort_by(|a, b| a.at.total_cmp(&b.at));
        assert_eq!(sections(&h, &v, &page, &mut 0).len(), 2);
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
        let got = shape(&cut(&h, &v));
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
        let got = shape(&cut(&h, &v));
        let spans: Vec<(f32, f32)> = got.iter().map(|g| g.0).collect();
        assert_eq!(spans, [(80.0, 100.0), (60.0, 80.0), (40.0, 60.0)]);
    }

    #[test]
    fn a_grid_without_rules_across_is_whole() {
        let (h, v) = section(&[0.0, 100.0, 200.0], &[0.0, 10.0, 20.0]);
        assert_eq!(cut(&h, &v).len(), 1);
        assert!(cut(&[], &v).len() == 1);
    }
}
