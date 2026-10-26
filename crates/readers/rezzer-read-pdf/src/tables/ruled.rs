// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables from grids of drawn rules.

use std::collections::{HashMap, HashSet};

use super::banded::{Band, Shared, Taken, banded};
use super::sections::{CUT_WORK, sections};
use super::*;
use crate::marks::is_placeholder;
use crate::regions::{Grids, Seg, Unsearched, clustered, grids};

/// Why a grid with a figure caption next to it is not a table.
const FIGURE_NEAR: &str = "a figure caption stands next to it";

/// A section of a grid: its segments, whether its grid was cut, whether a cut is its bottom
/// or its top, and its grid's place among the page's grids.
struct Part {
    h: Vec<Seg>,
    v: Vec<Seg>,
    cut: bool,
    cut_below: bool,
    cut_above: bool,
    id: usize,
}

/// The one note of a page for its grids refused with a note: a grid alone says why; several
/// say how many and why, by reason, the reasons in the order they first came. The note stands
/// where the first of them does.
fn page_note(page: u32, refusals: &[(usize, f32, &'static str)]) -> Option<Unsure> {
    let first = refusals.iter().min_by_key(|r| r.0)?;
    let text = (refusals.len() > 1).then(|| {
        format!(
            "Possible tables on page {page} not rebuilt: {}",
            counted(refusals.iter().map(|r| r.2))
        )
    });
    Some(Unsure {
        at: first.0,
        top: first.1,
        reason: first.2.to_string(),
        text,
    })
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
    // Each connected grid in its sections, each judged as a grid of its own, top to bottom;
    // a section of a grid that was cut is no grid's to drop without a word.
    let mut cut_work = 0usize;
    let mut grids: Vec<Part> = Vec::new();
    for (id, (gh, gv)) in groups.iter().enumerate() {
        let h: Vec<Seg> = gh.iter().map(|&j| hs[j]).collect();
        let v: Vec<Seg> = gv.iter().map(|&k| vs[k]).collect();
        let parts = sections(&h, &v, &hs, &mut cut_work);
        let (cut, last) = (parts.len() > 1, parts.len().saturating_sub(1));
        grids.extend(parts.into_iter().enumerate().map(|(k, (h, v))| Part {
            h,
            v,
            cut,
            cut_below: k < last,
            cut_above: k > 0,
            id,
        }));
    }
    if cut_work > CUT_WORK {
        found.skipped.push(format!(
            "ruled grids on page {page} are cut in sections only in part: the page has too many \
             pieces of lines at the heights of their rules"
        ));
    }
    grids.sort_by(|a, b| {
        let top = |g: &Part| g.h.iter().map(|s| s.at).fold(f32::MIN, f32::max);
        top(b).total_cmp(&top(a))
    });
    let lines = flow_lines(flow);
    let by_y = ByY::new(flow, &lines);
    // Runs a grid took, and for each the table and the row and cell it goes to.
    let mut claimed: Vec<Option<(usize, usize, usize)>> = vec![None; flow.runs.len()];
    // Runs of grids refused with a note, and those grids: where each stands and why; and
    // the grids refused without a note.
    let mut held: Vec<bool> = vec![false; flow.runs.len()];
    // Runs that title the section below a cut: no section above takes them, and the section
    // below, by its grid, has them as its caption once it is a table.
    let mut titles: Vec<bool> = vec![false; flow.runs.len()];
    let mut pending: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut title_work = 0usize;
    let mut refusals: Vec<(usize, f32, &'static str)> = Vec::new();
    let mut quiet: Vec<(usize, &'static str, Refused)> = Vec::new();
    let mut tables: Vec<Planned> = Vec::new();
    let mut bands: Vec<Band> = Vec::new();
    // Cells drawn as icons, the page's rule boxes sorted by their bottom once needed, and
    // the steps spent looking.
    let mut icons: Vec<(usize, usize, usize, [f32; 4])> = Vec::new();
    let mut strokes: Option<Vec<[f32; 4]>> = None;
    let mut icon_work = 0usize;
    'grids: for (
        g,
        Part {
            mut h,
            v,
            cut,
            cut_below,
            cut_above,
            id,
        },
    ) in grids.into_iter().enumerate()
    {
        if g % CHECK_EVERY == CHECK_EVERY - 1 && stop() {
            found.skipped.push(format!(
                "ruled tables on page {page} are looked for only in part: the time limit was \
                 reached"
            ));
            break;
        }
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
        let mut title_above = pending.remove(&id).unwrap_or_default();
        if xs.len() >= 2 && ys.len() >= 2 {
            let mut place = TitlePlace {
                flow,
                by_y: &by_y,
                claimed: &claimed,
                titles: &mut titles,
                xs: &xs,
                work: &mut title_work,
            };
            // The first section of a cut grid has the line right over the grid as its title,
            // as each section after it has the line right over its cut.
            if cut && !cut_above {
                title_above.extend(place.over_grid(&vs, ys[0]));
            }
            // The line right above a cut is the title of the section below: it leaves this
            // section, and a row of one cell that held only it goes too.
            if cut_below {
                let title = place.above_cut(&v, &mut ys, &mut h);
                pending.insert(id, title);
            }
        }
        // A row whose inner column rules all hang from its top and end at one height is a
        // header over lines without rules: a rule across at their feet parts the two, and
        // the row below it (its lines) is open: one cell across that is no spanning box.
        // Only in a cut grid or a row alone, and only over lines of text it is for.
        let mut open: Vec<f32> = Vec::new();
        if xs.len() >= 3 && (cut || ys.len() == 2) {
            let feet: Vec<f32> = hanging_feet(&v, &xs, &ys)
                .into_iter()
                .filter(|&(foot, bottom)| {
                    by_y.runs_between(bottom, foot).into_iter().any(|i| {
                        let r = &flow.runs[i];
                        let (cx, cy) = middle(r);
                        !r.off_axis
                            && claimed[i].is_none()
                            && !titles[i]
                            && cy < foot
                            && cx > xs[0]
                            && cx < xs[xs.len() - 1]
                            && !r.text.trim().is_empty()
                    })
                })
                .map(|(foot, _)| foot)
                .collect();
            for foot in feet {
                let at = ys.partition_point(|e| *e > foot);
                ys.insert(at, foot);
                open.push(foot);
                h.push(Seg {
                    at: foot,
                    from: xs[0],
                    to: xs[xs.len() - 1],
                });
            }
        }
        let (cols, rows) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
        if rows == 1 && cols >= 2 {
            // A row of drawn cells may head a table whose lines have no rules.
            bands.push(Band {
                b: [xs[0], ys[1], xs[cols], ys[0]],
                xs,
                cut,
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
            !r.off_axis && claimed[i].is_none() && !titles[i] && cx > b[0] && cx < b[2]
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
        if within.is_empty() || boxed_field(flow, &within, &xs, &ys) {
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
        // A grid over most of the page is its frame, unless text fills at least half of its
        // squares in three rows and three columns or more: a table as large as the page.
        let dense = rows >= 3 && 2 * text_squares.len() >= rows * cols;
        let noted = table_caption
            || (cols >= 3
                && (text_rows.len() >= 2 || text_squares.len() >= 6)
                && match area.refuses(b, &sizes) {
                    None => true,
                    Some(why) => why == PAGE_FRAME && dense,
                });
        // A figure caption speaks against a table: no note and no loss for it. A grid that
        // looks like a table gets a note and holds its text as paragraphs. Any other grid is
        // a loss only, unless it is a chart's (a figure caption next to it or a picture under
        // it), whatever reason refuses it: its text stays free for the detector without
        // rules, as such a grid often frames a table of its own.
        let chart = figure_caption || area.on_picture(b);
        let mut refuse = |why: &'static str| {
            if why == FIGURE_NEAR {
                return;
            }
            if !noted {
                if !chart {
                    quiet.push((at, why, Refused::Grid));
                }
                return;
            }
            refusals.push((at, b[3], why));
            for &i in &mine {
                held[i] = true;
            }
        };
        // A figure caption next to the grid or inside it is looked at first, so a wide chart
        // gets no note; a grid of many columns is refused before its cells are worked out.
        if figure_caption
            || within
                .iter()
                .any(|&i| caption(&flow.runs[i].text) == Some(false))
        {
            refuse(FIGURE_NEAR);
            continue;
        }
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
        // Rows above that rule are the table's header only when they look it: a style of
        // their own (bold over plain, or another size). A cell over several rows alone makes
        // no header.
        let styled = head >= 2 && {
            let (mut top, mut below): (Vec<&Run>, Vec<&Run>) = (Vec::new(), Vec::new());
            for &i in &within {
                let r = &flow.runs[i];
                let (_, cy) = middle(r);
                let row = ys.partition_point(|e| *e > cy).saturating_sub(1);
                if row < head {
                    top.push(r)
                } else {
                    below.push(r)
                }
            }
            let median = |runs: &mut Vec<&Run>| {
                runs.sort_by(|a, b| a.size.total_cmp(&b.size));
                runs.get(runs.len() / 2).map_or(0.0, |r| r.size)
            };
            let bold =
                !top.is_empty() && top.iter().all(|r| r.bold) && below.iter().any(|r| !r.bold);
            !below.is_empty() && (bold || (median(&mut top) - median(&mut below)).abs() > 0.5)
        };
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
        let mut anchors = anchor_rows(
            flow, &within, head, rows, &owner, &extent, &row_at, &cell_at,
        );
        // The lines under a header of hanging rules are rows by the header's columns.
        open_anchors(flow, &within, &xs, &ys, &open, &row_at, &mut anchors);
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
        }
        // Cells counted with the rows each band was cut in; only cells of the body count as
        // spanning, as a header often spans.
        let all: usize = extent.keys().map(|o| subs(o.0)).sum();
        let spanning: usize = extent
            .iter()
            .filter(|(o, (rs, cs))| {
                o.0 >= head && rs * cs > 1 && !open.iter().any(|f| (ys[o.0] - f).abs() <= SNAP)
            })
            .map(|(o, _)| subs(o.0))
            .sum();
        let empty = all - with_text.len();
        let refused = if empty as f32 > EMPTY_SHARE * all as f32 {
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
        for &i in &title_above {
            claimed[i] = Some((t, CAPTION_ROW as usize, 0));
        }
        let at = title_above.iter().map(|&i| flow.at[i]).fold(at, usize::min);
        tables.push(Planned {
            at,
            top: b[3],
            rows: planned_rows,
            // A header of two rows or more in a style of its own is the grid's; otherwise the
            // first row is judged by its text.
            header_rows: if styled { head } else { 0 },
            trs: Vec::new(),
            additions: Vec::new(),
            tagged: None,
            continues: false,
            caption: Vec::new(),
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
            stop,
        },
        bands,
        &mut Taken {
            claimed: &mut claimed,
            held: &mut held,
            refusals: &mut refusals,
            quiet: &mut quiet,
            tables: &mut tables,
            skipped: &mut found.skipped,
        },
    );
    found.quiet = quiet;
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

/// Lines a title may have.
const TITLE_LINES: usize = 2;

/// Steps the title search of one page may take over the page's column rules; past them no
/// line over a cut grid is taken as a title.
const TITLE_WORK: usize = 1_000_000;

/// One line of text: its baseline, its largest size, whether all of it is bold, and its
/// runs.
struct TextLine {
    y: f32,
    size: f32,
    bold: bool,
    runs: Vec<usize>,
}

/// The lines of `runs` from the bottom: runs whose baselines lie within a third of their
/// size of the line's first.
fn lines_up(flow: &Flow, runs: &[usize]) -> Vec<TextLine> {
    let mut sorted = runs.to_vec();
    sorted.sort_by(|&a, &b| flow.runs[a].y.total_cmp(&flow.runs[b].y));
    let mut out: Vec<TextLine> = Vec::new();
    for i in sorted {
        let r = &flow.runs[i];
        match out.last_mut() {
            Some(l) if r.y - l.y <= 0.3 * r.size.max(1.0) => {
                l.size = l.size.max(r.size);
                l.bold &= r.bold;
                l.runs.push(i);
            }
            _ => out.push(TextLine {
                y: r.y,
                size: r.size,
                bold: r.bold,
                runs: vec![i],
            }),
        }
    }
    out
}

/// The title among `runs` that stands right over the rule at height `base`: the lowest line
/// over it and, for a title of two lines, the line one line pitch above it in the same
/// style, none crossed by a column rule (`crossed` of a height range). The title is at most
/// twice its size over the rule and closer to it than to the next line or to `ceiling` above
/// (the row edge over it), or bold and at most its size over the rule.
fn title_over(
    flow: &Flow,
    runs: &[usize],
    base: f32,
    ceiling: f32,
    crossed: &mut dyn FnMut(f32, f32) -> bool,
) -> Vec<usize> {
    let above: Vec<usize> = runs
        .iter()
        .copied()
        .filter(|&i| flow.runs[i].y > base)
        .collect();
    let lines = lines_up(flow, &above);
    let Some(first) = lines.first() else {
        return Vec::new();
    };
    if crossed(first.y, first.y + first.size) {
        return Vec::new();
    }
    let mut taken = 1;
    while taken < TITLE_LINES.min(lines.len()) {
        let (prev, next) = (&lines[taken - 1], &lines[taken]);
        if next.y - prev.y > 1.6 * prev.size.max(next.size)
            || next.bold != first.bold
            || next.y + next.size > ceiling
            || crossed(next.y, next.y + next.size)
        {
            break;
        }
        taken += 1;
    }
    let block = &lines[..taken];
    let size = block.iter().map(|l| l.size).fold(0.0f32, f32::max);
    let head = block.iter().map(|l| l.y + l.size).fold(f32::MIN, f32::max);
    let next = lines.get(taken).map_or(f32::MAX, |l| l.y);
    let (below, gap) = (first.y - base, next.min(ceiling) - head);
    let bold = block.iter().all(|l| l.bold);
    if below <= 2.0 * size && (below < gap || (bold && below <= size)) {
        block.iter().flat_map(|l| l.runs.iter().copied()).collect()
    } else {
        Vec::new()
    }
}

/// What the title search of a section looks at and marks.
struct TitlePlace<'a> {
    flow: &'a Flow,
    by_y: &'a ByY,
    claimed: &'a [Option<(usize, usize, usize)>],
    titles: &'a mut Vec<bool>,
    /// The section's column edges.
    xs: &'a [f32],
    work: &'a mut usize,
}

impl TitlePlace<'_> {
    /// The runs between the heights `lo` and `hi` over the section that no table and no
    /// title has taken.
    fn free(&self, lo: f32, hi: f32) -> Vec<usize> {
        let (left, right) = (self.xs[0], self.xs[self.xs.len() - 1]);
        self.by_y
            .runs_between(lo, hi)
            .into_iter()
            .filter(|&i| {
                let r = &self.flow.runs[i];
                let (cx, _) = middle(r);
                !r.off_axis
                    && self.claimed[i].is_none()
                    && !self.titles[i]
                    && cx > left
                    && cx < right
                    && !r.text.trim().is_empty()
            })
            .collect()
    }

    fn take(&mut self, title: &[usize]) {
        for &i in title {
            self.titles[i] = true;
        }
    }

    /// The title right over the grid's top rule at `top`: no column rule of the page
    /// (`vs`, sorted by place) may cross it.
    fn over_grid(&mut self, vs: &[Seg], top: f32) -> Vec<usize> {
        let runs = self.free(top, top + 2.0 * MAX_CAPTION_SIZE);
        let (left, right) = (self.xs[0] + SNAP, self.xs[self.xs.len() - 1] - SNAP);
        let lo = vs.partition_point(|s| s.at <= left);
        let work = &mut *self.work;
        let mut crossed = |a: f32, b: f32| {
            for s in &vs[lo..] {
                *work += 1;
                if *work > TITLE_WORK {
                    return true;
                }
                if s.at >= right {
                    break;
                }
                if s.from < b && s.to > a {
                    return true;
                }
            }
            false
        };
        let title = title_over(self.flow, &runs, top, f32::MAX, &mut crossed);
        self.take(&title);
        title
    }

    /// The title right over the cut at the section's bottom (`ys` from the top): no inner
    /// column rule of the section (`v`) may cross it. Its runs leave the section, and a
    /// lowest row of one cell that held only them goes too, with its rules of `h`.
    fn above_cut(&mut self, v: &[Seg], ys: &mut Vec<f32>, h: &mut Vec<Seg>) -> Vec<usize> {
        let n = ys.len();
        let runs = self.free(ys[n - 1], ys[0]);
        let (left, right) = (self.xs[0] + SNAP, self.xs[self.xs.len() - 1] - SNAP);
        let inner = |s: &&Seg| s.at > left && s.at < right;
        let low = runs
            .iter()
            .map(|&i| self.flow.runs[i].y)
            .fold(f32::MAX, f32::min);
        let ceiling = ys
            .iter()
            .copied()
            .filter(|e| *e > low)
            .fold(f32::MAX, f32::min);
        let mut crossed = |a: f32, b: f32| v.iter().filter(inner).any(|s| s.from < b && s.to > a);
        let title = title_over(self.flow, &runs, ys[n - 1], ceiling, &mut crossed);
        self.take(&title);
        if !title.is_empty() && n >= 3 {
            let (lo, hi) = (ys[n - 1], ys[n - 2]);
            let empty = runs.iter().all(|&i| {
                self.titles[i] || {
                    let (_, cy) = middle(&self.flow.runs[i]);
                    cy > hi || cy < lo
                }
            });
            let parted = v
                .iter()
                .filter(inner)
                .any(|s| s.from < hi - SNAP && s.to > lo + SNAP);
            if empty && !parted {
                ys.pop();
                h.retain(|s| s.at >= hi - SNAP);
            }
        }
        title
    }
}

/// For each open row (its top one of the feet `open`), the lines of `within` in it that
/// start a row, as `anchors`; a row with fewer than two such lines is left as it is.
fn open_anchors(
    flow: &Flow,
    within: &[usize],
    xs: &[f32],
    ys: &[f32],
    open: &[f32],
    row_at: &dyn Fn(f32) -> usize,
    anchors: &mut [Vec<f32>],
) {
    if open.is_empty() {
        return;
    }
    let mut by_row: HashMap<usize, Vec<usize>> = HashMap::new();
    for &i in within {
        let r = row_at(middle(&flow.runs[i]).1);
        if open.iter().any(|f| (ys[r] - f).abs() <= SNAP) {
            by_row.entry(r).or_default().push(i);
        }
    }
    for (r, runs) in by_row {
        let lines = open_rows(flow, &runs, xs);
        if lines.len() >= 2 {
            anchors[r] = lines;
        }
    }
}

/// The baselines, going up, of the lines that start a row in an open row (lines without
/// rules under a header of hanging rules) that holds `runs`, in lines by baseline: a line
/// starts a row when it has text in at least half of the columns of `xs` that the lines
/// use, the others join the row above.
fn open_rows(flow: &Flow, runs: &[usize], xs: &[f32]) -> Vec<f32> {
    let cols = xs.len() - 1;
    let mut runs: Vec<(f32, f32, usize)> = runs
        .iter()
        .map(|&i| {
            let r = &flow.runs[i];
            let (cx, _) = middle(r);
            let c = xs
                .partition_point(|e| *e < cx)
                .saturating_sub(1)
                .min(cols - 1);
            (r.y, r.size.max(1.0), c)
        })
        .collect();
    runs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut lines: Vec<(f32, Vec<usize>)> = Vec::new();
    for (y, size, c) in runs {
        match lines.last_mut() {
            Some((ly, cs)) if *ly - y <= 0.3 * size => cs.push(c),
            _ => lines.push((y, vec![c])),
        }
    }
    let mut used: Vec<usize> = lines.iter().flat_map(|l| l.1.iter().copied()).collect();
    used.sort_unstable();
    used.dedup();
    let mut starts: Vec<f32> = lines
        .iter_mut()
        .filter_map(|(y, cs)| {
            cs.sort_unstable();
            cs.dedup();
            (2 * cs.len() >= used.len()).then_some(*y)
        })
        .collect();
    // Kept going up, for a search by baseline; the first line always starts a row.
    if let Some(first) = lines.first()
        && starts.first() != Some(&first.0)
    {
        starts.insert(0, first.0);
    }
    starts.reverse();
    starts
}

/// For the rows between the row edges `ys` (from the top) and the column edges `xs`, the
/// heights where the inner column rules of a row end when it has some, all of them hang
/// from its top and they end at one height clear of its bottom, each with that bottom. Each
/// rule is looked at in the rows it reaches into, no more.
fn hanging_feet(v: &[Seg], xs: &[f32], ys: &[f32]) -> Vec<(f32, f32)> {
    let (left, right) = (xs[0], xs[xs.len() - 1]);
    let rows = ys.len() - 1;
    let mut feet: Vec<Vec<f32>> = vec![Vec::new(); rows];
    let mut standing = vec![false; rows];
    for s in v {
        if s.at <= left + SNAP || s.at >= right - SNAP {
            continue;
        }
        // Rows whose top is above the rule's foot and whose bottom is below its head.
        let first = ys.partition_point(|e| *e >= s.to - SNAP).saturating_sub(1);
        for r in first..rows {
            if ys[r] <= s.from + SNAP {
                break;
            }
            if ys[r + 1] >= s.to - SNAP {
                continue;
            }
            if s.to >= ys[r] - SNAP && s.from > ys[r + 1] + SNAP {
                feet[r].push(s.from);
            } else {
                standing[r] = true;
            }
        }
    }
    feet.into_iter()
        .zip(standing)
        .enumerate()
        .filter_map(|(r, (f, standing))| {
            let f = clustered(f);
            (!standing && f.len() == 1).then(|| (f[0], ys[r + 1]))
        })
        .collect()
}

/// A box of a boxed field is at most this many times as wide as the size of its digits.
const DIGIT_BOX: f32 = 1.5;

/// Whether the grid of column edges `xs` and row edges `ys` (from the top) with the runs
/// `within` is boxed fields: columns of one width, narrow as a box for each digit, and in
/// each row at most one run of text, which is digits (with the separators read from marks).
/// Its text is read as it is, and its refusal is no loss.
fn boxed_field(flow: &Flow, within: &[usize], xs: &[f32], ys: &[f32]) -> bool {
    let (Some(first), Some(size)) = (
        xs.windows(2).map(|w| w[1] - w[0]).next(),
        within.iter().map(|&i| flow.runs[i].size).reduce(f32::max),
    ) else {
        return false;
    };
    let digits = |t: &str| {
        let t = t.trim();
        t.starts_with(|c: char| c.is_ascii_digit())
            && t.chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',') || is_placeholder(c))
    };
    let mut rows: HashSet<usize> = HashSet::new();
    xs.len() >= 3
        && first <= DIGIT_BOX * size
        && xs.windows(2).all(|w| (w[1] - w[0] - first).abs() <= SNAP)
        && within.iter().all(|&i| {
            let r = &flow.runs[i];
            let (_, cy) = middle(r);
            digits(&r.text) && rows.insert(ys.partition_point(|e| *e > cy))
        })
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
/// at least two of a band's lines hold text in two of the header's columns or more, and at
/// most a quarter of its pieces of text reach over a column's side, each of its squares is a
/// cell of its own.
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
    // The runs of each such band: baseline, size and extent.
    let mut bands: Vec<Vec<(f32, f32, f32, f32)>> = vec![Vec::new(); rows];
    for &i in within {
        let run = &flow.runs[i];
        let (_, cy) = middle(run);
        let r = ys
            .partition_point(|e| *e > cy)
            .saturating_sub(1)
            .min(rows - 1);
        if whole[r] {
            bands[r].push((run.y, run.size.max(1.0), run.x, run.x + run.w));
        }
    }
    let column = |x: f32| {
        xs.partition_point(|e| *e < x)
            .saturating_sub(1)
            .min(cols - 1)
    };
    for (r, band) in bands.iter_mut().enumerate() {
        if band.len() < 4 {
            continue;
        }
        band.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.2.total_cmp(&b.2)));
        // Lines of pieces (runs closer than a cell gap): those in two columns or more, and
        // pieces that reach over a column's side.
        let (mut spread, mut pieces, mut crossing) = (0usize, 0usize, 0usize);
        let mut k = 0;
        while k < band.len() {
            let (y, size, _, _) = band[k];
            let mut e = k + 1;
            while e < band.len() && y - band[e].0 <= 0.3 * size {
                e += 1;
            }
            let line = &mut band[k..e];
            line.sort_by(|a, b| a.2.total_cmp(&b.2));
            let mut spans: Vec<(f32, f32)> = Vec::new();
            for &(_, size, x0, x1) in line.iter() {
                match spans.last_mut() {
                    Some(p) if x0 - p.1 <= CELL_GAP * size => p.1 = p.1.max(x1),
                    _ => spans.push((x0, x1)),
                }
            }
            let mut used: Vec<usize> = spans.iter().map(|p| column((p.0 + p.1) / 2.0)).collect();
            used.dedup();
            if used.len() >= 2 {
                spread += 1;
            }
            pieces += spans.len();
            crossing += spans
                .iter()
                .filter(|p| column(p.0 + SNAP) != column(p.1 - SNAP))
                .count();
            k = e;
        }
        // Text that runs over the columns (a note across the table) keeps the band whole.
        if spread >= 2 && 4 * crossing <= pieces {
            owner[r] = (0..cols).map(|c| (r, c)).collect();
        }
    }
}

/// For each grid row, the baselines of the lines that start a row in it, going up: empty
/// when the band is one row. A band of the body whose cells are each one square and of
/// which at least two hold text is cut when at least two of its lines are anchors: lines
/// with text in at least half of the cells that hold text in the band (an empty first cell
/// included). No band is cut when the rules part most bands into one record each.
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
    // Bands of the body with text in two cells or more, and of those the ones with two lines
    // or more that fill the band and start in its first cell.
    let (mut texted, mut records) = (0usize, 0usize);
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
        let (mut anchors, mut keyed) = (Vec::new(), 0usize);
        for (y, cells) in lines.iter_mut() {
            cells.sort_unstable();
            cells.dedup();
            if 2 * cells.len() >= used.len() {
                anchors.push(*y);
                if cells.first() == Some(&first) {
                    keyed += 1;
                }
            }
        }
        texted += 1;
        if keyed >= 2 {
            records += 1;
        }
        if anchors.len() >= 2 {
            // Kept going up, for a search by baseline.
            out[r] = anchors.into_iter().rev().collect();
        }
    }
    // When rules part the body in bands and most of them hold one record (one line that
    // fills the band and starts in its first cell), the rules part the records: a band with
    // more such lines is a record whose cells wrap, and is not cut.
    if texted >= 2 && 2 * records < texted {
        out.iter_mut().for_each(Vec::clear);
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
