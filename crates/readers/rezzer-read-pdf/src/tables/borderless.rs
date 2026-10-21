// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables without rules, from the white space between columns.

use std::collections::HashMap;

use super::*;
use crate::regions::offset_baselines;

/// A run of text of one line, apart from its neighbours by a gap: one cell's text.
#[derive(Debug, Clone)]
struct Piece {
    x0: f32,
    x1: f32,
    /// Range of runs in the flow.
    start: usize,
    end: usize,
}

/// How many pieces `pieces_of` finds in a line, without building them.
fn piece_count(flow: &Flow, l: &FlowLine) -> usize {
    let mut n = 0;
    let mut end = f32::MIN;
    for r in &flow.runs[l.start..l.end] {
        if n == 0 || r.x - end > CELL_GAP * r.size.max(1.0) {
            n += 1;
            end = r.x + r.w;
        } else {
            end = end.max(r.x + r.w);
        }
    }
    n
}

fn pieces_of(flow: &Flow, l: &FlowLine) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for i in l.start..l.end {
        let r = &flow.runs[i];
        let x0 = r.x;
        match out.last_mut() {
            Some(p) if x0 - p.x1 <= CELL_GAP * r.size.max(1.0) => {
                p.x1 = p.x1.max(r.x + r.w);
                p.end = i + 1;
            }
            _ => out.push(Piece {
                x0,
                x1: r.x + r.w,
                start: i,
                end: i + 1,
            }),
        }
    }
    out
}

/// The text of runs `start..end` of the flow, white space collapsed.
fn text_of(flow: &Flow, start: usize, end: usize) -> String {
    let mut s = String::new();
    for r in &flow.runs[start..end] {
        for w in r.text.split_whitespace() {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(w);
        }
    }
    s
}

/// Tables without rules: regions of lines whose pieces stand in columns with white space
/// between them, accepted only on strong evidence.
pub(super) fn borderless(
    page: u32,
    flow: &mut Flow,
    area: &Area<'_>,
    stop: &dyn Fn() -> bool,
    found: &mut Found,
) {
    let lines = flow_lines(flow);
    let by_y = ByY::new(flow, &lines);
    // The number of pieces of each line; the pieces themselves are built for a region only.
    let counts: Vec<usize> = lines.iter().map(|l| piece_count(flow, l)).collect();
    // Lines of other directions, and lines a refused ruled grid holds, are in no region.
    let held = |i: usize| found.held.get(i).copied().unwrap_or(false);
    let off: Vec<bool> = lines
        .iter()
        .map(|l| flow.runs[l.start..l.end].iter().any(|r| r.off_axis) || (l.start..l.end).any(held))
        .collect();
    // Regions: lines that follow each other down the page, most of them in two pieces or
    // more; at most two lines of one piece in a row (cell text that wraps).
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if off[i] || counts[i] < 2 {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        let mut last_multi = i;
        while end < lines.len() && !off[end] {
            let (prev, cur) = (&lines[end - 1], &lines[end]);
            let gap = prev.y - cur.y;
            if gap <= 0.0 || gap > 3.0 * prev.size.max(cur.size) {
                break;
            }
            if counts[end] >= 2 {
                last_multi = end;
            } else if end - last_multi > 2 {
                break;
            }
            end += 1;
        }
        regions.push((i, last_multi + 1));
        i = last_multi + 1;
    }
    // Accepted regions, and for each run the table, row and cell it goes to.
    let mut claim: Vec<Planned> = Vec::new();
    let mut target: Vec<Target> = vec![None; flow.runs.len()];
    // A region without rules was noted on this page already.
    let mut noted = false;
    for (g, (a, z)) in regions.into_iter().enumerate() {
        if g % CHECK_EVERY == CHECK_EVERY - 1 && stop() {
            found.skipped.push(format!(
                "tables without rules on page {page} are looked for only in part: the time \
                 limit was reached"
            ));
            break;
        }
        if found.tables.len() + claim.len() >= MAX_TABLES {
            found.skipped.push(too_many_tables(page));
            break;
        }
        let multi = (a..z).filter(|&k| counts[k] >= 2).count();
        if multi < 2 {
            continue;
        }
        let pieces: Vec<Vec<Piece>> = lines[a..z].iter().map(|l| pieces_of(flow, l)).collect();
        // Two blocks side by side are judged each alone.
        let sets: Vec<Vec<Vec<Piece>>> = match side_by_side(flow, &lines[a..z], &pieces) {
            Some(x) => vec![
                pieces
                    .iter()
                    .map(|l| l.iter().filter(|p| p.x1 <= x).cloned().collect())
                    .collect(),
                pieces
                    .iter()
                    .map(|l| l.iter().filter(|p| p.x0 >= x).cloned().collect())
                    .collect(),
            ],
            None => vec![pieces],
        };
        let split = sets.len() > 1;
        let mut refused: Option<(&'static str, f32)> = None;
        let mut taken = false;
        for pieces in &sets {
            let runs: Vec<usize> = pieces
                .iter()
                .flatten()
                .flat_map(|p| p.start..p.end)
                .collect();
            let Some(&first) = runs.iter().min() else {
                continue;
            };
            let used: Vec<usize> = (0..pieces.len())
                .filter(|&k| !pieces[k].is_empty())
                .collect();
            let (Some(&top), Some(&bottom)) = (used.first(), used.last()) else {
                continue;
            };
            let x0 = pieces
                .iter()
                .flatten()
                .map(|p| p.x0)
                .fold(f32::MAX, f32::min);
            let x1 = pieces
                .iter()
                .flatten()
                .map(|p| p.x1)
                .fold(f32::MIN, f32::max);
            let (lt, lb) = (&lines[a + top], &lines[a + bottom]);
            let b = [x0, lb.y - 0.25 * lb.size, x1, lt.y + lt.size];
            let in_region = |i: usize| (lines[a].start..lines[z - 1].end).contains(&i);
            let line_sizes: Vec<f32> = lines[a..z].iter().map(|l| l.size).collect();
            let (table_caption, figure_caption) =
                captions_near(flow, &lines, &by_y, b, &line_sizes, &in_region);
            let rows_needed = if table_caption { 2 } else { 3 };
            let verdict =
                grid_of(flow, &lines[a..z], pieces, rows_needed, table_caption).and_then(|grid| {
                    if figure_caption {
                        return Err("a figure caption stands next to it");
                    }
                    let sizes: Vec<f32> = runs.iter().map(|&i| flow.runs[i].size).collect();
                    match area.refuses(b, &sizes) {
                        Some(why) => Err(why),
                        None => Ok(grid),
                    }
                });
            match verdict {
                Ok(grid) => {
                    taken = true;
                    let planned = Planned {
                        at: flow.at[first],
                        top: b[3],
                        rows: grid
                            .rows
                            .iter()
                            .map(|r| {
                                r.iter()
                                    .map(|c| PlannedCell {
                                        // Filled in below, once the runs are taken out.
                                        runs: Vec::new(),
                                        row_span: 1,
                                        col_span: c.span,
                                    })
                                    .collect()
                            })
                            .collect(),
                        header_rows: 0,
                        trs: Vec::new(),
                        additions: Vec::new(),
                        tagged: None,
                        continues: false,
                    };
                    let t = u32::try_from(claim.len()).unwrap_or(u32::MAX);
                    for (r, cells) in grid.rows.iter().enumerate() {
                        for (c, g) in cells.iter().enumerate() {
                            for &(s, e) in &g.ranges {
                                for slot in &mut target[s..e] {
                                    *slot = Some((t, r as u32, c as u32));
                                }
                            }
                        }
                    }
                    claim.push(planned);
                }
                Err(why) => {
                    if table_caption && refused.is_none() {
                        let why = if split { SIDE_BY_SIDE } else { why };
                        refused = Some((why, b[3]));
                    }
                }
            }
        }
        if let (false, Some((why, top))) = (taken, refused) {
            // A grid of rules over the same text may have said so already, and a page gets
            // one note for its regions without rules.
            let (lo, hi) = (flow.at[lines[a].start], flow.at[lines[z - 1].end - 1]);
            let said = noted || found.unsure.iter().any(|u| (lo..=hi).contains(&u.at));
            if !said {
                noted = true;
                found.unsure.push(Unsure {
                    at: flow.at[lines[a].start],
                    top,
                    reason: why.to_string(),
                    text: None,
                });
            }
        }
    }
    if claim.is_empty() {
        return;
    }
    // The runs of all accepted regions leave the flow at once; each goes to its cell.
    let mut planned = claim;
    fill(&mut planned, flow, &target);
    for mut p in planned {
        p.header_rows = header_of(&p.rows);
        found.tables.push(p);
    }
    found.tables.sort_by_key(|t| t.at);
}

/// A grid rebuilt from white space gutters: rows of cells, each cell its runs as flow ranges.
struct Grid {
    rows: Vec<Vec<GridCell>>,
}

struct GridCell {
    ranges: Vec<(usize, usize)>,
    span: u32,
}

/// Why a region of two blocks side by side is no table.
const SIDE_BY_SIDE: &str = "two blocks stand side by side";

/// A gutter this many font sizes wide or more, and this many times the median gutter, that
/// no line crosses may part two blocks side by side.
const BLOCK_GUTTER: f32 = 8.0;
const BLOCK_GUTTER_RATIO: f32 = 3.0;

/// Columns: the x ranges the pieces of the lines with the most common number of pieces fill;
/// the first line may be a header that spans columns.
fn columns_of(pieces: &[Vec<Piece>]) -> Vec<(f32, f32)> {
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for p in pieces.iter().filter(|p| p.len() >= 2) {
        *counts.entry(p.len()).or_default() += 1;
    }
    let mode = counts
        .iter()
        .max_by_key(|(n, k)| (**k, **n))
        .map(|(n, _)| *n)
        .unwrap_or(0);
    let mut spans: Vec<(f32, f32)> = pieces
        .iter()
        .enumerate()
        .filter(|(k, p)| p.len() == mode && (*k > 0 || pieces.len() == 1))
        .flat_map(|(_, p)| p.iter().map(|p| (p.x0, p.x1)))
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut columns: Vec<(f32, f32)> = Vec::new();
    for (s, e) in spans {
        match columns.last_mut() {
            Some(c) if s <= c.1 => c.1 = c.1.max(e),
            _ => columns.push((s, e)),
        }
    }
    columns
}

/// Where two blocks that stand side by side part, when they do: the widest gutter between
/// columns, when no line crosses it, the white space all lines leave there is at least
/// [`BLOCK_GUTTER`] font sizes wide and [`BLOCK_GUTTER_RATIO`] times the median gutter, and
/// the baselines of its sides do not line up.
fn side_by_side(flow: &Flow, lines: &[FlowLine], pieces: &[Vec<Piece>]) -> Option<f32> {
    let columns = columns_of(pieces);
    if columns.len() < 3 {
        return None;
    }
    let gutters: Vec<(f32, f32)> = columns.windows(2).map(|w| (w[0].1, w[1].0)).collect();
    let mut widths: Vec<f32> = gutters.iter().map(|g| g.1 - g.0).collect();
    widths.sort_by(f32::total_cmp);
    let median = widths[(widths.len() - 1) / 2];
    let (g0, g1) = gutters
        .into_iter()
        .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))?;
    // The white space every line leaves around the middle of that gutter.
    let mid = (g0 + g1) / 2.0;
    if pieces.iter().flatten().any(|p| p.x0 < mid && p.x1 > mid) {
        return None;
    }
    let all = || pieces.iter().flatten();
    let g0 = all()
        .filter(|p| p.x1 <= mid)
        .map(|p| p.x1)
        .fold(f32::MIN, f32::max);
    let g1 = all()
        .filter(|p| p.x0 >= mid)
        .map(|p| p.x0)
        .fold(f32::MAX, f32::min);
    let mut sizes: Vec<f32> = lines.iter().map(|l| l.size).collect();
    sizes.sort_by(f32::total_cmp);
    let size = sizes[sizes.len() / 2];
    if g1 - g0 < BLOCK_GUTTER * size || g1 - g0 < BLOCK_GUTTER_RATIO * median {
        return None;
    }
    // The baselines of a side, line by line: those of its runs.
    let side = |left: bool| -> Vec<Vec<f32>> {
        pieces
            .iter()
            .map(|l| {
                l.iter()
                    .filter(|p| if left { p.x1 <= mid } else { p.x0 >= mid })
                    .flat_map(|p| flow.runs[p.start..p.end].iter().map(|r| r.y))
                    .collect::<Vec<f32>>()
            })
            .filter(|l| !l.is_empty())
            .collect()
    };
    offset_baselines(&side(true), &side(false), size).then_some(mid)
}

/// Rebuilds the grid of a region and tests it, or says why it is no table.
fn grid_of(
    flow: &Flow,
    lines: &[FlowLine],
    pieces: &[Vec<Piece>],
    rows_needed: usize,
    table_caption: bool,
) -> Result<Grid, &'static str> {
    let columns = columns_of(pieces);
    let cols = columns.len();
    if cols < 2 {
        return Err("no columns");
    }
    let over = |p: &Piece| -> Vec<usize> {
        (0..cols)
            .filter(|&c| p.x1 > columns[c].0 && p.x0 < columns[c].1)
            .collect()
    };
    // The column of each piece; a line whose piece crosses a gutter is not clean.
    let mut clean = 0usize;
    let mut placed: Vec<Vec<(usize, u32, &Piece)>> = Vec::with_capacity(lines.len());
    for (k, line) in pieces.iter().enumerate() {
        let mut row = Vec::new();
        let mut ok = true;
        for p in line {
            let hit = over(p);
            let (col, span) = match hit.as_slice() {
                // Text in a gutter belongs to no column: the grid does not hold.
                [] => return Err("some of its text stands between its columns"),
                [c] => (*c, 1),
                many => {
                    if k > 0 {
                        ok = false;
                    }
                    (many[0], (many[many.len() - 1] - many[0] + 1) as u32)
                }
            };
            row.push((col, span, p));
        }
        if ok {
            clean += 1;
        }
        placed.push(row);
    }
    if (clean as f32) < CLEAN_ROWS * lines.len() as f32 {
        return Err("its columns are not clean");
    }
    // Rows: lines that fill at least half the columns; other lines join the nearest one.
    let filled = |row: &Vec<(usize, u32, &Piece)>| {
        let mut c: Vec<usize> = row.iter().map(|x| x.0).collect();
        c.dedup();
        c.len()
    };
    let anchors: Vec<usize> = (0..lines.len())
        .filter(|&k| 2 * filled(&placed[k]) >= cols)
        .collect();
    if anchors.len() < rows_needed.max(1) {
        return Err("too few rows");
    }
    let mut rows: Vec<Vec<GridCell>> = anchors
        .iter()
        .map(|_| {
            (0..cols)
                .map(|_| GridCell {
                    ranges: Vec::new(),
                    span: 1,
                })
                .collect()
        })
        .collect();
    for (k, row) in placed.iter().enumerate() {
        // The anchor rows are in order: the nearest is the one before or the one after.
        let after = anchors.partition_point(|&a| a < k);
        let target = match (after.checked_sub(1), anchors.get(after)) {
            (Some(b), Some(&a)) => {
                let d = |x: usize| (lines[x].y - lines[k].y).abs();
                if d(a) < d(anchors[b]) { after } else { b }
            }
            (Some(b), None) => b,
            (None, _) => after,
        };
        for &(col, span, p) in row {
            let cell = &mut rows[target][col];
            cell.ranges.push((p.start, p.end));
            cell.span = cell.span.max(span);
        }
    }
    // A cell that spans hides the cells it covers.
    for row in &mut rows {
        let mut c = 0;
        while c < row.len() {
            let span = row[c].span as usize;
            for k in c + 1..(c + span).min(row.len()) {
                let moved = std::mem::take(&mut row[k].ranges);
                row[c].ranges.extend(moved);
                row[k].span = 0;
            }
            c += span.max(1);
        }
        row.retain(|c| c.span > 0);
    }
    // Short cells, or mostly numbers.
    let texts: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| {
                    let mut ranges = c.ranges.clone();
                    ranges.sort_unstable();
                    ranges
                        .iter()
                        .map(|&(s, e)| text_of(flow, s, e))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()
        })
        .collect();
    let mut lens: Vec<usize> = texts
        .iter()
        .flatten()
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().count())
        .collect();
    lens.sort_unstable();
    if texts.len() > 1 && texts.iter().all(|r| *r == texts[0]) {
        return Err("all its rows read the same");
    }
    let numbers = texts.iter().flatten().filter(|t| numeric(t)).count();
    let short = if table_caption { 60 } else { SHORT_CELL };
    if lens.is_empty() || (lens[lens.len() / 2] >= short && 2 * numbers < lens.len()) {
        return Err("its cells hold long text");
    }
    // Text that runs on from a cell into the cell below like prose is two columns of text.
    for c in 0..cols {
        for r in 1..texts.len() {
            let (Some(above), Some(below)) = (texts[r - 1].get(c), texts[r].get(c)) else {
                continue;
            };
            // A line of prose: a few words that end without a stop, and the next line goes on
            // in lower case.
            let words = above.split_whitespace().count();
            let ends = above.ends_with(['.', ':', ';', '!', '?']);
            let line = words >= PROSE_WORDS && above.chars().count() >= PROSE_LINE;
            if line && !ends && below.chars().next().is_some_and(char::is_lowercase) {
                return Err("its text runs on from row to row like prose");
            }
        }
    }
    // Numbered entries with page numbers in the last column, going up, are a table of
    // contents.
    let pages: Vec<Option<u32>> = texts
        .iter()
        .map(|r| r.last().and_then(|t| t.parse::<u32>().ok()))
        .collect();
    let numbered = texts.iter().all(|r| {
        r.first()
            .and_then(|t| t.split_whitespace().next())
            .is_some_and(|w| {
                w.trim_end_matches('.')
                    .split('.')
                    .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            })
    });
    if texts.len() >= 3
        && numbered
        && pages.iter().all(Option::is_some)
        && pages.windows(2).all(|w| w[0] <= w[1])
    {
        return Err("it reads like a table of contents");
    }
    Ok(Grid { rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Slot;

    fn run(text: &str, x: f32, y: f32, line: u32) -> Run {
        Run {
            page: 1,
            text: text.into(),
            x,
            y,
            w: 5.0 * text.chars().count() as f32,
            h: 8.0,
            size: 8.0,
            bold: false,
            italic: false,
            mcid: None,
            uri: None,
            line,
            off_axis: false,
        }
    }

    #[test]
    fn side_by_side_blocks_are_refused() {
        // Lines read row by row across two lists that stand side by side: labels and values
        // on the left, labels and values 270 points to the right, 4 points lower. No table
        // may mix the two lists.
        let rows = [
            ("Guest", "Sample", "Folio", "1000"),
            ("Room", "101", "Copy", "Original"),
            ("Arrival", "2030", "Cashier", "77"),
            ("Nights", "3", "Desk", "Front"),
        ];
        let mut runs = Vec::new();
        for (k, (a, b, c, d)) in rows.iter().enumerate() {
            let y = 650.0 - 16.0 * k as f32;
            let line = k as u32;
            runs.push(run(a, 18.0, y, line));
            runs.push(run(b, 70.0, y, line));
            runs.push(run(c, 400.0, y - 4.0, line));
            runs.push(run(d, 450.0, y - 4.0, line));
        }
        let n = runs.len();
        let mut flow = Flow {
            runs,
            slots: vec![Slot::Free; n],
            at: (0..n).collect(),
        };
        let area = Area {
            width: 595.0,
            height: 842.0,
            images: &[],
            body_size: 8.0,
        };
        let mut found = Found::default();
        borderless(1, &mut flow, &area, &|| false, &mut found);
        let left = [
            "Guest", "Sample", "Room", "101", "Arrival", "2030", "Nights", "3",
        ];
        for t in &found.tables {
            let words: Vec<&str> = t
                .rows
                .iter()
                .flatten()
                .flat_map(|c| c.runs.iter().map(|r| r.text.as_str()))
                .collect();
            let l = words.iter().filter(|w| left.contains(w)).count();
            assert!(
                l == 0 || l == words.len(),
                "a table mixes both lists: {words:?}"
            );
        }
    }
}
