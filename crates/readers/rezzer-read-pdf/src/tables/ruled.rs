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

/// Joined lines a group of segments keeps open for joining at once; past them the oldest is
/// closed. Only a page of many long overlapping lines at slightly different places has more.
const OPEN_LINES: usize = 32;

/// Collinear segments that touch become one. Segments are grouped by where they lie: a
/// segment joins the group when it lies within [`SNAP`] of the group's mean place, so lines
/// in a dense run (bars of a barcode, hatching) never chain far apart. Within a group, a
/// piece joins a line that it overlaps or touches within [`SNAP`] and that lies within
/// [`SNAP`] of it; the joined line lies at the mean of its pieces. Two lines a point apart
/// that never meet along their length stay apart.
fn merged(mut segs: Vec<Seg>) -> Vec<Seg> {
    segs.sort_by(|a, b| a.at.total_cmp(&b.at));
    let mut out: Vec<Seg> = Vec::with_capacity(segs.len());
    let mut start = 0;
    while start < segs.len() {
        let mut end = start + 1;
        let mut sum = f64::from(segs[start].at);
        while end < segs.len() {
            let mean = sum / (end - start) as f64;
            if f64::from(segs[end].at) - mean > f64::from(SNAP) {
                break;
            }
            sum += f64::from(segs[end].at);
            end += 1;
        }
        let group = &mut segs[start..end];
        group.sort_by(|a, b| a.from.total_cmp(&b.from));
        // Open lines: the joined segment, the sum of its pieces' places and their number.
        let mut open: Vec<(Seg, f64, u32)> = Vec::new();
        let close = |(mut s, sum, n): (Seg, f64, u32), out: &mut Vec<Seg>| {
            s.at = (sum / f64::from(n)) as f32;
            out.push(s);
        };
        for s in group.iter() {
            let mut k = 0;
            while k < open.len() {
                if open[k].0.to + SNAP < s.from {
                    close(open.remove(k), &mut out);
                } else {
                    k += 1;
                }
            }
            let near = open.iter_mut().find(|(_, sum, n)| {
                (f64::from(s.at) - sum / f64::from(*n)).abs() <= f64::from(SNAP)
            });
            match near {
                Some((line, sum, n)) => {
                    line.to = line.to.max(s.to);
                    *sum += f64::from(s.at);
                    *n += 1;
                }
                None => {
                    if open.len() == OPEN_LINES {
                        close(open.remove(0), &mut out);
                    }
                    open.push((*s, f64::from(s.at), 1));
                }
            }
        }
        for line in open {
            close(line, &mut out);
        }
        start = end;
    }
    // The grid search looks segments up by where they lie.
    out.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
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
    // Runs of grids refused with a note.
    let mut held: Vec<bool> = vec![false; flow.runs.len()];
    let mut tables: Vec<Planned> = Vec::new();
    'grids: for (g, (gh, gv)) in grids.into_iter().enumerate() {
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
        if tables.len() >= MAX_TABLES {
            found.skipped.push(too_many_tables(page));
            break;
        }
        let at = flow.at[within[0]];
        let sizes: Vec<f32> = within.iter().map(|&i| flow.runs[i].size).collect();
        let (table_caption, figure_caption) = captions_near(flow, &lines, &by_y, b, &sizes, &|i| {
            let (_, cy) = middle(&flow.runs[i]);
            inside(i) && cy > b[1] && cy < b[3]
        });
        // A refused grid gets a note when a caption names a table, or when it looks like one:
        // three columns or more with text in two rows or in six squares, and nothing about
        // the area (the page's frame, a picture, tiny text) that rules out a table. Its text
        // then stays paragraphs and no detector without rules takes it.
        let mut text_rows: HashSet<usize> = HashSet::new();
        let mut text_squares: HashSet<(usize, usize)> = HashSet::new();
        for &i in &within {
            let (cx, cy) = middle(&flow.runs[i]);
            let r = ys
                .partition_point(|e| *e > cy)
                .saturating_sub(1)
                .min(rows - 1);
            let c = xs
                .partition_point(|e| *e < cx)
                .saturating_sub(1)
                .min(cols - 1);
            text_rows.insert(r);
            text_squares.insert((r, c));
        }
        let noted = table_caption
            || (cols >= 3
                && (text_rows.len() >= 2 || text_squares.len() >= 6)
                && area.refuses(b, &sizes).is_none());
        let mut refuse = |why: &str, found: &mut Found| {
            if !noted {
                return;
            }
            found.unsure.push(Unsure {
                at,
                top: b[3],
                reason: why.to_string(),
            });
            for &i in &mine {
                held[i] = true;
            }
        };
        // A grid of many columns is refused before its cells are worked out.
        if cols > MAX_COLUMNS {
            refuse("it has more than 15 columns", found);
            continue;
        }
        // The segments on each boundary of the grid, found once.
        let nearest = |edges: &[f32], at: f32| -> Option<usize> {
            let k = edges.partition_point(|e| *e < at);
            [k.checked_sub(1), Some(k)]
                .into_iter()
                .flatten()
                .filter(|&i| i < edges.len() && (edges[i] - at).abs() <= SNAP)
                .min_by(|&a, &b| (edges[a] - at).abs().total_cmp(&(edges[b] - at).abs()))
        };
        let mut on_x: Vec<Vec<Seg>> = vec![Vec::new(); xs.len()];
        for s in &v {
            if let Some(k) = nearest(&xs, s.at) {
                on_x[k].push(*s);
            }
        }
        let ys_up: Vec<f32> = ys.iter().rev().copied().collect();
        let mut on_y: Vec<Vec<Seg>> = vec![Vec::new(); ys.len()];
        for s in &h {
            if let Some(k) = nearest(&ys_up, s.at) {
                on_y[ys.len() - 1 - k].push(*s);
            }
        }
        // Whether a boundary is drawn over at least half of a cell's side.
        let covered = |segs: &[Seg], lo: f32, hi: f32| {
            let len: f32 = segs
                .iter()
                .map(|s| (s.to.min(hi) - s.from.max(lo)).max(0.0))
                .sum();
            len >= 0.5 * (hi - lo)
        };
        // The cell each grid square belongs to: a missing line between two squares joins
        // them, across first, then down where whole cells line up.
        let mut owner: Vec<Vec<(usize, usize)>> = vec![vec![(0, 0); cols]; rows];
        for (r, row) in owner.iter_mut().enumerate() {
            if r % 1024 == 1023 && stop() {
                found.skipped.push(format!(
                    "ruled tables on page {page} are looked for only in part: the time limit \
                     was reached"
                ));
                break 'grids;
            }
            for c in 0..cols {
                row[c] = if c > 0 && !covered(&on_x[c], ys[r + 1], ys[r]) {
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
                if lined_up && !(c..c + span).any(|k| covered(&on_y[r], xs[k], xs[k + 1])) {
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
        let refused = if figure_caption || caption_inside {
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
            refuse(why, found);
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
            trs: Vec::new(),
            additions: Vec::new(),
            tagged: None,
            continues: false,
        });
    }
    if tables.is_empty() {
        if held.contains(&true) {
            found.held = held;
        }
        return;
    }
    // The runs of all grids leave the flow at once; the runs held stay where they are.
    if held.contains(&true) {
        found.held = held
            .iter()
            .zip(&claimed)
            .filter(|(_, c)| c.is_none())
            .map(|(h, _)| *h)
            .collect();
    }
    let target: Vec<Target> = claimed
        .into_iter()
        .map(|c| c.map(|(t, r, k)| (t as u32, r as u32, k as u32)))
        .collect();
    fill(&mut tables, flow, &target);
    for mut t in tables {
        t.header_rows = header_of(&t.rows);
        found.tables.push(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(at: f32, from: f32, to: f32) -> Seg {
        Seg { at, from, to }
    }

    #[test]
    fn merged_keeps_near_collinear_segments() {
        // Two sides of a box drawn twice a little apart, and a column rule 1.5 points to
        // the right of them in three pieces lower down: the rule's pieces stay apart from
        // the box sides and from each other.
        let out = merged(vec![
            seg(411.5, 689.5, 739.8),
            seg(412.2, 689.5, 739.8),
            seg(413.3, 106.2, 261.5),
            seg(413.3, 481.0, 557.5),
            seg(413.3, 621.2, 672.2),
        ]);
        let mut got: Vec<(f32, f32, f32)> = out.iter().map(|s| (s.at, s.from, s.to)).collect();
        got.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
        assert_eq!(got.len(), 4, "{got:?}");
        for (k, (from, to)) in [(106.2, 261.5), (481.0, 557.5), (621.2, 672.2)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(got[k], (413.3, from, to), "{got:?}");
        }
        assert!((got[3].0 - 411.85).abs() < 0.01, "{got:?}");
        assert_eq!((got[3].1, got[3].2), (689.5, 739.8), "{got:?}");
        // Pieces that touch within the snap distance join; pieces of one line far apart
        // stay apart.
        let out = merged(vec![
            seg(100.0, 0.0, 10.0),
            seg(100.5, 11.5, 20.0),
            seg(100.0, 30.0, 40.0),
        ]);
        assert_eq!(out.len(), 2, "{out:?}");
        // Bars every 1.8 points and two column rules 40 points apart among them, over other
        // heights: the rules stay apart.
        let mut segs: Vec<Seg> = (0..56)
            .map(|k| seg(400.0 + 1.8 * k as f32, 100.0, 140.0))
            .collect();
        segs.push(seg(410.0, 640.0, 700.0));
        segs.push(seg(450.0, 640.0, 700.0));
        let out = merged(segs);
        let rules: Vec<f32> = out
            .iter()
            .filter(|s| s.from == 640.0)
            .map(|s| s.at)
            .collect();
        assert_eq!(rules, [410.0, 450.0], "{out:?}");
    }
}
