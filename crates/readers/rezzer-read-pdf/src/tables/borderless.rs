// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables without rules, from the white space between columns.

use std::collections::HashMap;

use super::*;

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
    let off: Vec<bool> = lines
        .iter()
        .map(|l| flow.runs[l.start..l.end].iter().any(|r| r.off_axis))
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
        let x0 = (a..z)
            .flat_map(|k| pieces[k - a].iter().map(|p| p.x0))
            .fold(f32::MAX, f32::min);
        let x1 = (a..z)
            .flat_map(|k| pieces[k - a].iter().map(|p| p.x1))
            .fold(f32::MIN, f32::max);
        let b = [
            x0,
            lines[z - 1].y - 0.25 * lines[z - 1].size,
            x1,
            lines[a].y + lines[a].size,
        ];
        let in_region = |i: usize| (lines[a].start..lines[z - 1].end).contains(&i);
        let line_sizes: Vec<f32> = lines[a..z].iter().map(|l| l.size).collect();
        let (table_caption, figure_caption) =
            captions_near(flow, &lines, &by_y, b, &line_sizes, &in_region);
        let rows_needed = if table_caption { 2 } else { 3 };
        let verdict =
            grid_of(flow, &lines[a..z], &pieces, rows_needed, table_caption).and_then(|grid| {
                if figure_caption {
                    return Err("a figure caption stands next to it");
                }
                let sizes: Vec<f32> = flow.runs[lines[a].start..lines[z - 1].end]
                    .iter()
                    .map(|r| r.size)
                    .collect();
                match area.refuses(b, &sizes) {
                    Some(why) => Err(why),
                    None => Ok(grid),
                }
            });
        match verdict {
            Ok(grid) => {
                let planned = Planned {
                    at: flow.at[lines[a].start],
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
                // A grid of rules over the same text may have said so already.
                let (lo, hi) = (flow.at[lines[a].start], flow.at[lines[z - 1].end - 1]);
                let said = found.unsure.iter().any(|u| (lo..=hi).contains(&u.at));
                if table_caption && !said {
                    found.unsure.push(Unsure {
                        at: flow.at[lines[a].start],
                        top: b[3],
                        reason: why.to_string(),
                    });
                }
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

/// Rebuilds the grid of a region and tests it, or says why it is no table.
fn grid_of(
    flow: &Flow,
    lines: &[FlowLine],
    pieces: &[Vec<Piece>],
    rows_needed: usize,
    table_caption: bool,
) -> Result<Grid, &'static str> {
    // Columns: the x ranges the pieces of the lines with the most common number of pieces
    // fill; the first line may be a header that spans columns.
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
