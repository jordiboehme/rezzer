// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables from grids of drawn rules.

use std::collections::{HashMap, HashSet};

use super::banded::{Band, Shared, Taken, banded};
use super::*;
use crate::regions::{Grids, Seg, Unsearched, clustered, grids};

/// Why a grid with a figure caption next to it is not a table.
const FIGURE_NEAR: &str = "a figure caption stands next to it";

/// The one note of a page for its grids refused with a note: a grid alone says why; several
/// say how many and why, by reason, the reasons in the order they first came. The note stands
/// where the first of them does.
fn page_note(page: u32, refusals: &[(usize, f32, &'static str)]) -> Option<Unsure> {
    let first = refusals.iter().min_by_key(|r| r.0)?;
    let mut text = None;
    if refusals.len() > 1 {
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for (_, _, why) in refusals {
            match counts.iter_mut().find(|(w, _)| w == why) {
                Some((_, n)) => *n += 1,
                None => counts.push((why, 1)),
            }
        }
        let parts: Vec<String> = counts
            .iter()
            .map(|&(why, n)| {
                let why = if n == 1 { why } else { plural(why) };
                format!("{n} ({why})")
            })
            .collect();
        text = Some(format!(
            "Possible tables on page {page} not rebuilt: {}",
            parts.join(", ")
        ));
    }
    Some(Unsure {
        at: first.0,
        top: first.1,
        reason: first.2.to_string(),
        text,
    })
}

/// A reason a ruled grid is refused, said of several grids.
fn plural(why: &str) -> &str {
    match why {
        "it has more than 15 columns" => "they have more than 15 columns",
        "most of its cells are empty" => "most of their cells are empty",
        "its boxes do not form rows and columns" => "their boxes do not form rows and columns",
        "its text crosses its lines" => "their text crosses their lines",
        "it covers most of the page" => "they cover most of the page",
        "it lies on a picture" => "they lie on a picture",
        "its text is much smaller than the body text, as in a picture of a page" => {
            "their text is much smaller than the body text, as in pictures of pages"
        }
        "too few rows follow its header" => "too few rows follow their headers",
        "the text below its header does not stand in its columns" => {
            "the text below their headers does not stand in their columns"
        }
        "its cells hold long text" => "their cells hold long text",
        "its text runs on from row to row like prose" => {
            "their text runs on from row to row like prose"
        }
        other => other,
    }
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
    let Grids { hs, vs, groups } = match grids(rules) {
        Ok(g) => g,
        Err(Unsearched::TooMany) => {
            found.skipped.push(format!(
                "ruled tables on page {page} are not looked for: the page draws more than \
                 {MAX_SEGMENTS} straight lines"
            ));
            return;
        }
        Err(Unsearched::MeetTooOften) => {
            found.skipped.push(format!(
                "ruled tables on page {page} are not looked for: its lines meet too often"
            ));
            return;
        }
    };
    let mut grids = groups;
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
    // Runs of grids refused with a note, and those grids: where each stands and why.
    let mut held: Vec<bool> = vec![false; flow.runs.len()];
    let mut refusals: Vec<(usize, f32, &'static str)> = Vec::new();
    let mut tables: Vec<Planned> = Vec::new();
    let mut bands: Vec<Band> = Vec::new();
    // Cells drawn as icons, the page's rule boxes sorted by their bottom once needed, and
    // the steps spent looking.
    let mut icons: Vec<(usize, usize, usize, [f32; 4])> = Vec::new();
    let mut strokes: Option<Vec<[f32; 4]>> = None;
    let mut icon_work = 0usize;
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
        if rows == 1 && cols >= 2 {
            // A row of drawn cells may head a table whose lines have no rules.
            bands.push(Band {
                b: [xs[0], ys[1], xs[cols], ys[0]],
                xs,
            });
            continue;
        }
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
        let (caption_line, figure_caption) = captions_near(flow, &lines, &by_y, b, &sizes, &|i| {
            let (_, cy) = middle(&flow.runs[i]);
            inside(i) && cy > b[1] && cy < b[3]
        });
        let table_caption = caption_line.is_some();
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
        // A figure caption speaks against a table: no note for it.
        let mut refuse = |why: &'static str| {
            if !noted || why == FIGURE_NEAR {
                return;
            }
            refusals.push((at, b[3], why));
            for &i in &mine {
                held[i] = true;
            }
        };
        // A grid of many columns is refused before its cells are worked out.
        if cols > MAX_COLUMNS {
            refuse("it has more than 15 columns");
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
        // The header: the grid rows above the first rule under the top border that crosses
        // every column, when rows stand below it.
        let head = (1..rows)
            .find(|&k| (0..cols).all(|c| covered(&on_y[k], xs[c], xs[c + 1])))
            .unwrap_or(0);
        // A band of the body that no column rule parts, under a header that has them, is
        // read in the header's columns when its lines hold text in two columns or more.
        if head >= 1 && cols >= 3 {
            open_bands(flow, &within, head, &xs, &ys, &mut owner);
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
        let row_at = |y: f32| -> usize {
            ys.partition_point(|e| *e > y)
                .saturating_sub(1)
                .min(rows - 1)
        };
        let cell_at = |x: f32, y: f32| -> (usize, usize) {
            let c = xs
                .partition_point(|e| *e < x)
                .saturating_sub(1)
                .min(cols - 1);
            owner[row_at(y)][c]
        };
        // Rows of the body that only column rules part: the lines of a band that fill at
        // least half of its columns with text, the first of them included, each start a row.
        let anchors = anchor_rows(
            flow, &within, head, rows, &owner, &extent, &row_at, &cell_at,
        );
        let subs = |r: usize| anchors[r].len().max(1);
        // The row of a band a run goes to: that of the nearest anchor line at or above it.
        let sub_of = |r: usize, run: &Run| -> usize {
            // The baselines go up: those at or above the run are the last ones.
            let a = &anchors[r];
            let tol = 0.3 * run.size.max(1.0);
            let above = a.len() - a.partition_point(|y| *y < run.y - tol);
            above.saturating_sub(1)
        };
        let planned_count: usize = (0..rows).map(subs).sum();
        if planned_count.saturating_mul(cols) > MAX_SQUARES {
            found.skipped.push(format!(
                "a grid of {planned_count} by {cols} cells on page {page} is not read as a \
                 table: it has more than {MAX_SQUARES} cells; its text stays paragraphs"
            ));
            // No detector without rules takes its text either.
            for &i in &mine {
                held[i] = true;
            }
            continue;
        }
        let mut with_text: HashSet<((usize, usize), usize)> = HashSet::new();
        let mut crossing = 0usize;
        let mut caption_inside = false;
        for &i in &within {
            let r = &flow.runs[i];
            let (cx, cy) = middle(r);
            let o = cell_at(cx, cy);
            with_text.insert((o, sub_of(o.0, r)));
            // A run that reaches over its cell's side crosses a drawn line.
            let (x0, x1) = (xs[o.1], xs[(o.1 + extent[&o].1 as usize).min(cols)]);
            if r.x < x0 - SNAP || r.x + r.w > x1 + SNAP {
                crossing += 1;
            }
            if caption(&r.text) == Some(false) {
                caption_inside = true;
            }
        }
        // Cells counted with the rows each band was cut in; only cells of the body count as
        // spanning, as a header often spans.
        let all: usize = extent.keys().map(|o| subs(o.0)).sum();
        let spanning: usize = extent
            .iter()
            .filter(|(o, (rs, cs))| o.0 >= head && rs * cs > 1)
            .map(|(o, _)| subs(o.0))
            .sum();
        let empty = all - with_text.len();
        let refused = if figure_caption || caption_inside {
            Some(FIGURE_NEAR)
        } else if empty as f32 > EMPTY_SHARE * all as f32 {
            Some("most of its cells are empty")
        } else if 10 * spanning > 3 * all {
            Some("its boxes do not form rows and columns")
        } else if 5 * crossing > within.len() {
            Some("its text crosses its lines")
        } else {
            area.refuses(b, &sizes)
        };
        if let Some(why) = refused {
            refuse(why);
            continue;
        }
        // The place of each cell, and each row a band was cut in, in the planned rows.
        let mut place: HashMap<((usize, usize), usize), (usize, usize)> = HashMap::new();
        let mut planned_rows: Vec<Vec<PlannedCell>> = Vec::with_capacity(planned_count);
        for (r, row) in owner.iter().enumerate() {
            for sub in 0..subs(r) {
                let mut cells = Vec::new();
                for (c, o) in row.iter().enumerate() {
                    if *o != (r, c) {
                        continue;
                    }
                    let (row_span, col_span) = extent[o];
                    place.insert((*o, sub), (planned_rows.len(), cells.len()));
                    cells.push(PlannedCell {
                        runs: Vec::new(),
                        row_span,
                        col_span,
                    });
                }
                planned_rows.push(cells);
            }
        }
        let t = tables.len();
        for &i in &mine {
            let run = &flow.runs[i];
            let (cx, cy) = middle(run);
            let o = cell_at(cx, cy);
            let (r, c) = place[&(o, sub_of(o.0, run))];
            claimed[i] = Some((t, r, c));
        }
        // A cell of the first row without text that holds drawn strokes is a title drawn as
        // an icon.
        for (c, o) in owner[0].iter().enumerate() {
            if *o != (0, c) || with_text.contains(&(*o, 0)) {
                continue;
            }
            let (row_span, col_span) = extent[o];
            let cell = [
                xs[c],
                ys[row_span as usize],
                xs[c + col_span as usize],
                ys[0],
            ];
            if drawn_inside(&mut strokes, rules, cell, &mut icon_work) {
                let (r, k) = place[&(*o, 0)];
                icons.push((t, r, k, cell));
            }
        }
        tables.push(Planned {
            at,
            top: b[3],
            rows: planned_rows,
            // A header of two rows or more is the grid's; one row is judged by its text.
            header_rows: if head >= 2 { head } else { 0 },
            trs: Vec::new(),
            additions: Vec::new(),
            tagged: None,
            continues: false,
        });
    }
    // Regions without rules never run across a band.
    found.cuts = bands.iter().flat_map(|b| [b.b[1], b.b[3]]).collect();
    found.cuts.sort_by(f32::total_cmp);
    banded(
        &Shared {
            page,
            flow,
            lines: &lines,
            by_y: &by_y,
            area,
            hs: &hs,
        },
        bands,
        &mut Taken {
            claimed: &mut claimed,
            held: &mut held,
            refusals: &mut refusals,
            tables: &mut tables,
            skipped: &mut found.skipped,
        },
    );
    if let Some(u) = page_note(page, &refusals) {
        found.unsure.push(u);
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
    if let Some(template) = tables
        .iter()
        .flat_map(|t| &t.rows)
        .flatten()
        .find_map(|c| c.runs.first())
        .cloned()
    {
        for (t, r, k, cell) in icons {
            let size = template.size;
            tables[t].rows[r][k].runs.push(Run {
                text: ICON.into(),
                x: cell[0] + SNAP,
                y: (cell[1] + cell[3]) / 2.0 - size / 3.0,
                w: cell[2] - cell[0] - 2.0 * SNAP,
                bold: false,
                italic: false,
                mcid: None,
                uri: None,
                ..template.clone()
            });
        }
    }
    for mut t in tables {
        if t.header_rows == 0 {
            t.header_rows = header_of(&t.rows);
        }
        found.tables.push(t);
    }
}

/// What a title drawn as an icon is written as.
const ICON: &str = "(icon)";

/// Steps one page may take to look for icons in the cells of its tables.
const ICON_WORK: usize = 1_000_000;

/// Whether drawn strokes lie inside `cell` (x0, y0, x1, y1), off its sides. `strokes` are the
/// boxes of `rules` sorted by their bottom, built at the first look.
fn drawn_inside(
    strokes: &mut Option<Vec<[f32; 4]>>,
    rules: &[Rule],
    cell: [f32; 4],
    work: &mut usize,
) -> bool {
    let boxes = strokes.get_or_insert_with(|| {
        let mut b: Vec<[f32; 4]> = rules
            .iter()
            .map(|r| {
                [
                    r.x0.min(r.x1),
                    r.y0.min(r.y1),
                    r.x0.max(r.x1),
                    r.y0.max(r.y1),
                ]
            })
            .collect();
        b.sort_by(|a, b| a[1].total_cmp(&b[1]));
        b
    });
    let lo = boxes.partition_point(|b| b[1] < cell[1] + SNAP);
    for b in &boxes[lo..] {
        *work += 1;
        if b[1] > cell[3] - SNAP || *work > ICON_WORK {
            return false;
        }
        if b[0] > cell[0] + SNAP && b[2] < cell[2] - SNAP && b[3] < cell[3] - SNAP {
            return true;
        }
    }
    false
}

/// Opens the bands of the body below the header that one cell fills from side to side: when
/// at least two of a band's lines hold text in two of the header's columns or more, each of
/// its squares is a cell of its own.
fn open_bands(
    flow: &Flow,
    within: &[usize],
    head: usize,
    xs: &[f32],
    ys: &[f32],
    owner: &mut [Vec<(usize, usize)>],
) {
    let (rows, cols) = (ys.len() - 1, xs.len() - 1);
    let whole: Vec<bool> = (0..rows)
        .map(|r| r >= head && owner[r].iter().all(|o| *o == (r, 0)))
        .collect();
    if !whole.contains(&true) {
        return;
    }
    // The runs of each such band: baseline, size and column.
    let mut bands: Vec<Vec<(f32, f32, usize)>> = vec![Vec::new(); rows];
    for &i in within {
        let run = &flow.runs[i];
        let (cx, cy) = middle(run);
        let r = ys
            .partition_point(|e| *e > cy)
            .saturating_sub(1)
            .min(rows - 1);
        if whole[r] {
            let c = xs
                .partition_point(|e| *e < cx)
                .saturating_sub(1)
                .min(cols - 1);
            bands[r].push((run.y, run.size.max(1.0), c));
        }
    }
    for (r, band) in bands.iter_mut().enumerate() {
        if band.len() < 4 {
            continue;
        }
        band.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.2.cmp(&b.2)));
        let mut spread = 0usize;
        let mut k = 0;
        while k < band.len() {
            let (y, size, first) = band[k];
            let mut e = k + 1;
            let mut columns = 1;
            let mut last = first;
            while e < band.len() && y - band[e].0 <= 0.3 * size {
                if band[e].2 != last {
                    columns += 1;
                    last = band[e].2;
                }
                e += 1;
            }
            if columns >= 2 {
                spread += 1;
            }
            k = e;
        }
        if spread >= 2 {
            owner[r] = (0..cols).map(|c| (r, c)).collect();
        }
    }
}

/// For each grid row, the baselines of the lines that start a row in it, going up: empty
/// when the band is one row. A band of the body whose cells are each one square and of
/// which at least two hold text is cut when at least two of its lines are anchors: lines
/// with text in at least half of the cells that hold text in the band, the first of those
/// cells included.
#[allow(clippy::too_many_arguments)]
fn anchor_rows(
    flow: &Flow,
    within: &[usize],
    head: usize,
    rows: usize,
    owner: &[Vec<(usize, usize)>],
    extent: &HashMap<(usize, usize), (u32, u32)>,
    row_at: &dyn Fn(f32) -> usize,
    cell_at: &dyn Fn(f32, f32) -> (usize, usize),
) -> Vec<Vec<f32>> {
    let mut out: Vec<Vec<f32>> = vec![Vec::new(); rows];
    // The runs of each band, by baseline from the top.
    let mut bands: Vec<Vec<(f32, f32, usize)>> = vec![Vec::new(); rows];
    for &i in within {
        let r = &flow.runs[i];
        let (cx, cy) = middle(r);
        let row = row_at(cy);
        let o = cell_at(cx, cy);
        if row >= head && o.0 == row {
            bands[row].push((r.y, r.size.max(1.0), o.1));
        }
    }
    for (r, band) in bands.iter_mut().enumerate() {
        let one_high = owner[r]
            .iter()
            .all(|o| o.0 == r && extent.get(o) == Some(&(1, 1)));
        if band.len() < 2 || !one_high {
            continue;
        }
        band.sort_by(|a, b| b.0.total_cmp(&a.0));
        // Lines: runs whose baselines lie within a third of their size of the line's first.
        let mut lines: Vec<(f32, Vec<usize>)> = Vec::new();
        for &(y, size, c) in band.iter() {
            match lines.last_mut() {
                Some((ly, cells)) if *ly - y <= 0.3 * size => cells.push(c),
                _ => lines.push((y, vec![c])),
            }
        }
        let mut used: Vec<usize> = band.iter().map(|b| b.2).collect();
        used.sort_unstable();
        used.dedup();
        // One cell of text is a block of lines, not records.
        if used.len() < 2 {
            continue;
        }
        let first = used[0];
        let anchors: Vec<f32> = lines
            .iter_mut()
            .filter_map(|(y, cells)| {
                cells.sort_unstable();
                cells.dedup();
                (2 * cells.len() >= used.len() && cells.first() == Some(&first)).then_some(*y)
            })
            .collect();
        if anchors.len() >= 2 {
            // Kept going up, for a search by baseline.
            out[r] = anchors.into_iter().rev().collect();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regions::merged;

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
