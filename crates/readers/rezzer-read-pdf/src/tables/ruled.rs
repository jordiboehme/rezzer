// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables from grids of drawn rules.

use std::collections::{HashMap, HashSet};

use super::*;

/// A straight horizontal (`at` is y, `from`..`to` along x) or vertical segment.
#[derive(Debug, Clone, Copy)]
struct Seg {
    at: f32,
    from: f32,
    to: f32,
}

/// Collinear segments that touch become one.
fn merged(mut segs: Vec<Seg>) -> Vec<Seg> {
    segs.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
    let mut out: Vec<Seg> = Vec::with_capacity(segs.len());
    let mut group_start = 0;
    for s in segs {
        let joined = out[group_start..]
            .iter_mut()
            .rev()
            .find(|o| (o.at - s.at).abs() <= SNAP && s.from <= o.to + SNAP);
        match joined {
            Some(o) => o.to = o.to.max(s.to),
            None => {
                if out.last().is_none_or(|o| (o.at - s.at).abs() > SNAP) {
                    group_start = out.len();
                }
                out.push(s);
            }
        }
    }
    out
}

/// Values closer than [`SNAP`] are one, sorted.
fn clustered(mut values: Vec<f32>) -> Vec<f32> {
    values.sort_by(f32::total_cmp);
    let mut out: Vec<f32> = Vec::new();
    for v in values {
        match out.last() {
            Some(l) if v - l <= SNAP => {}
            _ => out.push(v),
        }
    }
    out
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Grids of drawn rules: horizontal and vertical lines that meet, with text in their cells.
pub(super) fn ruled(
    page: u32,
    flow: &mut Flow,
    rules: &[Rule],
    area: &Area<'_>,
    stop: &dyn Fn() -> bool,
    found: &mut Found,
) {
    let mut hs = Vec::new();
    let mut vs = Vec::new();
    for r in rules {
        let (x0, x1) = (r.x0.min(r.x1), r.x0.max(r.x1));
        let (y0, y1) = (r.y0.min(r.y1), r.y0.max(r.y1));
        match r.kind {
            RuleKind::Line if y1 - y0 <= SNAP / 2.0 => hs.push(Seg {
                at: (y0 + y1) / 2.0,
                from: x0,
                to: x1,
            }),
            RuleKind::Line => vs.push(Seg {
                at: (x0 + x1) / 2.0,
                from: y0,
                to: y1,
            }),
            RuleKind::Rect => {
                hs.push(Seg {
                    at: y0,
                    from: x0,
                    to: x1,
                });
                hs.push(Seg {
                    at: y1,
                    from: x0,
                    to: x1,
                });
                vs.push(Seg {
                    at: x0,
                    from: y0,
                    to: y1,
                });
                vs.push(Seg {
                    at: x1,
                    from: y0,
                    to: y1,
                });
            }
        }
    }
    if hs.is_empty() || vs.is_empty() {
        return;
    }
    if hs.len() + vs.len() > MAX_SEGMENTS {
        found.skipped.push(format!(
            "ruled tables on page {page} are not looked for: the page draws more than \
             {MAX_SEGMENTS} straight lines"
        ));
        return;
    }
    let hs = merged(hs);
    let vs = merged(vs);
    // Segments that meet are one grid. Horizontal ones are sorted by y.
    let n = hs.len();
    let mut parent: Vec<usize> = (0..n + vs.len()).collect();
    let mut work = 0usize;
    for (k, v) in vs.iter().enumerate() {
        let lo = hs.partition_point(|h| h.at < v.from - SNAP);
        for (j, h) in hs.iter().enumerate().skip(lo) {
            if h.at > v.to + SNAP {
                break;
            }
            work += 1;
            if work > GRID_WORK {
                found.skipped.push(format!(
                    "ruled tables on page {page} are not looked for: its lines meet too often"
                ));
                return;
            }
            if v.at >= h.from - SNAP && v.at <= h.to + SNAP {
                let (a, b) = (find(&mut parent, j), find(&mut parent, n + k));
                parent[a] = b;
            }
        }
    }
    let mut groups: HashMap<usize, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for j in 0..n {
        let root = find(&mut parent, j);
        groups.entry(root).or_default().0.push(j);
    }
    for k in 0..vs.len() {
        let root = find(&mut parent, n + k);
        groups.entry(root).or_default().1.push(k);
    }
    let mut grids: Vec<(Vec<usize>, Vec<usize>)> = groups
        .into_values()
        .filter(|(h, v)| h.len() >= 2 && v.len() >= 2)
        .collect();
    // Top to bottom.
    grids.sort_by(|a, b| {
        let top =
            |g: &(Vec<usize>, Vec<usize>)| g.0.iter().map(|&j| hs[j].at).fold(f32::MIN, f32::max);
        top(b).total_cmp(&top(a))
    });
    let lines = flow_lines(flow);
    let by_y = ByY::new(flow, &lines);
    // Runs a grid took, and for each the table and the row and cell it goes to.
    let mut claimed: Vec<Option<(usize, usize, usize)>> = vec![None; flow.runs.len()];
    let mut tables: Vec<Planned> = Vec::new();
    for (g, (gh, gv)) in grids.into_iter().enumerate() {
        if g % CHECK_EVERY == CHECK_EVERY - 1 && stop() {
            found.skipped.push(format!(
                "ruled tables on page {page} are looked for only in part: the time limit was \
                 reached"
            ));
            break;
        }
        let h: Vec<Seg> = gh.iter().map(|&j| hs[j]).collect();
        let v: Vec<Seg> = gv.iter().map(|&k| vs[k]).collect();
        let mut xs: Vec<f32> = v.iter().map(|s| s.at).collect();
        xs.push(h.iter().map(|s| s.from).fold(f32::MAX, f32::min));
        xs.push(h.iter().map(|s| s.to).fold(f32::MIN, f32::max));
        let mut ys: Vec<f32> = h.iter().map(|s| s.at).collect();
        ys.push(v.iter().map(|s| s.from).fold(f32::MAX, f32::min));
        ys.push(v.iter().map(|s| s.to).fold(f32::MIN, f32::max));
        let xs = clustered(xs);
        let mut ys = clustered(ys);
        // Rows from the top.
        ys.reverse();
        let (cols, rows) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
        if cols < 2 || rows < 2 {
            continue;
        }
        if rows.saturating_mul(cols) > MAX_SQUARES {
            found.skipped.push(format!(
                "a grid of {rows} by {cols} drawn cells on page {page} is not read as a table: \
                 it has more than {MAX_SQUARES} cells"
            ));
            continue;
        }
        let b = [xs[0], ys[rows], xs[cols], ys[0]];
        let inside = |i: usize| {
            let r = &flow.runs[i];
            let (cx, _) = middle(r);
            !r.off_axis && claimed[i].is_none() && cx > b[0] && cx < b[2]
        };
        let mine: Vec<usize> = by_y
            .runs_between(b[1], b[3])
            .into_iter()
            .filter(|&i| inside(i))
            .collect();
        let within: Vec<usize> = mine
            .iter()
            .copied()
            .filter(|&i| !flow.runs[i].text.trim().is_empty())
            .collect();
        if within.is_empty() {
            continue;
        }
        let at = flow.at[within[0]];
        let (table_caption, figure_caption) = captions_near(flow, &lines, &by_y, b, &|i| {
            let (_, cy) = middle(&flow.runs[i]);
            inside(i) && cy > b[1] && cy < b[3]
        });
        let sizes: Vec<f32> = within.iter().map(|&i| flow.runs[i].size).collect();
        // Whether a boundary is drawn over at least half of a cell's side.
        let covered = |segs: &[Seg], at: f32, lo: f32, hi: f32| {
            let mut len = 0.0;
            for s in segs.iter().filter(|s| (s.at - at).abs() <= SNAP) {
                len += (s.to.min(hi) - s.from.max(lo)).max(0.0);
            }
            len >= 0.5 * (hi - lo)
        };
        // The cell each grid square belongs to: a missing line between two squares joins
        // them, across first, then down where whole cells line up.
        let mut owner: Vec<Vec<(usize, usize)>> = vec![vec![(0, 0); cols]; rows];
        for (r, row) in owner.iter_mut().enumerate() {
            for c in 0..cols {
                row[c] = if c > 0 && !covered(&v, xs[c], ys[r + 1], ys[r]) {
                    row[c - 1]
                } else {
                    (r, c)
                };
            }
        }
        let span_of =
            |row: &[(usize, usize)], c: usize| row.iter().filter(|o| **o == row[c]).count();
        for r in 1..rows {
            let mut c = 0;
            while c < cols {
                let span = span_of(&owner[r], c);
                let above = owner[r - 1][c];
                let lined_up =
                    owner[r][c] == (r, c) && above.1 == c && span_of(&owner[r - 1], c) == span;
                if lined_up && !(c..c + span).any(|k| covered(&h, ys[r], xs[k], xs[k + 1])) {
                    owner[r][c..c + span].fill(above);
                }
                c += span;
            }
        }
        // Each cell's extent in squares: rows down and columns across.
        let mut extent: HashMap<(usize, usize), (u32, u32)> = HashMap::new();
        for (r, row) in owner.iter().enumerate() {
            for (c, o) in row.iter().enumerate() {
                let e = extent.entry(*o).or_insert((0, 0));
                if c == o.1 {
                    e.0 += 1;
                }
                if r == o.0 {
                    e.1 += 1;
                }
            }
        }
        let cell_at = |x: f32, y: f32| -> (usize, usize) {
            let r = ys
                .partition_point(|e| *e > y)
                .saturating_sub(1)
                .min(rows - 1);
            let c = xs
                .partition_point(|e| *e < x)
                .saturating_sub(1)
                .min(cols - 1);
            owner[r][c]
        };
        let mut with_text: HashSet<(usize, usize)> = HashSet::new();
        let mut crossing = 0usize;
        let mut caption_inside = false;
        for &i in &within {
            let r = &flow.runs[i];
            let (cx, cy) = middle(r);
            let o = cell_at(cx, cy);
            with_text.insert(o);
            // A run that reaches over its cell's side crosses a drawn line.
            let (x0, x1) = (xs[o.1], xs[(o.1 + extent[&o].1 as usize).min(cols)]);
            if r.x < x0 - SNAP || r.x + r.w > x1 + SNAP {
                crossing += 1;
            }
            if caption(&r.text) == Some(false) {
                caption_inside = true;
            }
        }
        let spanning = extent.values().filter(|(rs, cs)| rs * cs > 1).count();
        let empty = extent.len() - with_text.len();
        let refused = if cols > MAX_COLUMNS {
            Some("it has more than 15 columns")
        } else if figure_caption || caption_inside {
            Some("a figure caption stands next to it")
        } else if empty as f32 > EMPTY_SHARE * extent.len() as f32 {
            Some("most of its cells are empty")
        } else if 10 * spanning > 3 * extent.len() {
            Some("its boxes do not form rows and columns")
        } else if 5 * crossing > within.len() {
            Some("its text crosses its lines")
        } else {
            area.refuses(b, &sizes)
        };
        if let Some(why) = refused {
            if table_caption {
                found.unsure.push(Unsure {
                    at,
                    top: b[3],
                    reason: why.to_string(),
                });
            }
            continue;
        }
        // The place of each cell in the planned rows.
        let mut place: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
        let mut planned_rows: Vec<Vec<PlannedCell>> = Vec::with_capacity(rows);
        for (r, row) in owner.iter().enumerate() {
            let mut cells = Vec::new();
            for (c, o) in row.iter().enumerate() {
                if *o != (r, c) {
                    continue;
                }
                let (row_span, col_span) = extent[o];
                place.insert(*o, (r, cells.len()));
                cells.push(PlannedCell {
                    runs: Vec::new(),
                    row_span,
                    col_span,
                });
            }
            planned_rows.push(cells);
        }
        let t = tables.len();
        for &i in &mine {
            let (cx, cy) = middle(&flow.runs[i]);
            let (r, c) = place[&cell_at(cx, cy)];
            claimed[i] = Some((t, r, c));
        }
        tables.push(Planned {
            at,
            top: b[3],
            rows: planned_rows,
            header_rows: 0,
            tagged: None,
            continues: false,
        });
    }
    if tables.is_empty() {
        return;
    }
    // The runs of all grids leave the flow at once.
    let mut k = 0;
    let taken = flow.take(|_, _| {
        k += 1;
        claimed[k - 1].is_some()
    });
    for ((r, _, _), (t, row, cell)) in taken.into_iter().zip(claimed.into_iter().flatten()) {
        tables[t].rows[row][cell].runs.push(r);
    }
    for mut t in tables {
        t.header_rows = header_of(&t.rows);
        found.tables.push(t);
    }
}
