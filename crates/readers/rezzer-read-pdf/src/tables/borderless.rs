// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables without rules, from the white space between columns.

use std::collections::HashMap;

use super::*;
use crate::layout::{ColumnLine, Columns, Gaps, Slot, address_columns};
use crate::regions::{BASELINE_NEAR, Part, offset_baselines};

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
    // The frame of column titles over items that holds a run, if one does.
    let bodies = std::mem::take(&mut found.bodies);
    let body_of = |i: usize| bodies.get(i).copied().flatten();
    let off: Vec<bool> = lines
        .iter()
        .map(|l| flow.runs[l.start..l.end].iter().any(|r| r.off_axis) || (l.start..l.end).any(held))
        .collect();
    // Regions: lines that follow each other down the page, most of them in two pieces or
    // more; at most two lines of one piece in a row (cell text that wraps). Each region also
    // has the lines right under it that may go on in its last row (see [`Tail`]).
    let mut regions: Vec<(usize, usize, Tail)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if off[i] || counts[i] < 2 {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        let mut last_multi = i;
        // The widest step between two lines of the region so far.
        let mut pitch = 0.0f32;
        while end < lines.len() && !off[end] {
            let (prev, cur) = (&lines[end - 1], &lines[end]);
            let gap = prev.y - cur.y;
            if gap <= 0.0 || gap > 3.0 * prev.size.max(cur.size) {
                break;
            }
            // A gap of more than twice the steps of the lines on both sides of it parts two
            // blocks (a table under a field list or under totals, with no rule between) when
            // the line after it stands in other columns, or is the header of a table of its own;
            // a group of rows of the same table after a blank row stands in its columns.
            let next = lines
                .get(end + 1)
                .filter(|_| !off[end + 1])
                .map(|n| cur.y - n.y)
                .filter(|&g| g > 0.0 && g <= 3.0 * cur.size.max(lines[end + 1].size));
            if let Some(next) = next
                && pitch > 0.0
                && gap > REGION_STEPS * pitch.max(next)
                && (!in_columns(flow, &lines[i..end], cur)
                    || header_line(flow, &lines[i..end], cur, &lines[end + 1]))
            {
                break;
            }
            pitch = pitch.max(gap);
            // A header band's rule between the lines parts them.
            let cut = found.cuts.partition_point(|y| *y <= cur.y);
            if found.cuts.get(cut).is_some_and(|y| *y < prev.y) {
                break;
            }
            // A frame of column titles over items starts a region of its own: the lines over
            // its top rule (a title line of two pieces, a reference and a date) are not part of
            // its header. The first line of a column title of two lines with the rule drawn
            // between them stays out too, as a paragraph before the table: no bound of size,
            // height or alignment tells it from a short line set over the rule, and a wrong
            // header cell is worse than the lost link.
            let starts_body = (cur.start..cur.end)
                .filter_map(body_of)
                .any(|b| !(prev.start..prev.end).any(|r| body_of(r) == Some(b)));
            if starts_body {
                break;
            }
            if counts[end] >= 2 {
                last_multi = end;
            } else if end - last_multi > 2 {
                break;
            }
            end += 1;
        }
        // A rule across the lines may end their table: the lines below it are a region of
        // their own (totals under line items).
        let z = ended(flow, &lines, (i, last_multi + 1), &found.ends).unwrap_or(last_multi + 1);
        let mut tail = Tail {
            end: z,
            marked: false,
        };
        // Lines under the region that the frame of column titles over items it holds holds
        // too (a wrapped last item) may be its own; else the lines of one piece the region
        // ends with.
        if z == last_multi + 1 {
            let mut z = z;
            let held_bodies: Vec<u32> = (lines[i].start..lines[z - 1].end)
                .filter_map(body_of)
                .collect();
            while z < lines.len() && !off[z] {
                let (prev, cur) = (&lines[z - 1], &lines[z]);
                let gap = prev.y - cur.y;
                let own = (cur.start..cur.end)
                    .all(|r| body_of(r).is_some_and(|b| held_bodies.contains(&b)));
                if gap <= 0.0 || gap > 3.0 * prev.size.max(cur.size) || !own {
                    break;
                }
                z += 1;
            }
            tail = if z > last_multi + 1 {
                Tail {
                    end: z,
                    marked: true,
                }
            } else {
                Tail { end, marked: false }
            };
        }
        regions.push((i, z, tail));
        i = tail.end.max(z);
    }
    // Accepted regions, and for each run the table, row and cell it goes to.
    let mut claim: Vec<Planned> = Vec::new();
    let mut target: Vec<Target> = vec![None; flow.runs.len()];
    // The caption lines that gated a note already: one note for each caption.
    let mut noted: Vec<usize> = Vec::new();
    // Tables of labels and values, and their rows whose one run holds label and value.
    let mut kv: Vec<usize> = Vec::new();
    let mut splits: Vec<(usize, usize)> = Vec::new();
    // Tables whose first row is the line of titles of a frame of column titles over items.
    let mut titled: Vec<usize> = Vec::new();
    // Tables with close titles to cut, and where (see [`Grid`]).
    let mut title_cuts: Vec<(usize, Vec<TitleCut>)> = Vec::new();
    // Blocks side by side read one after the other: where their runs start in the flow, and
    // the runs in the order to read them.
    let mut apart: Vec<(usize, Vec<usize>)> = Vec::new();
    for (g, (a, mut z, tail)) in regions.into_iter().enumerate() {
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
        let mut pieces: Vec<Vec<Piece>> = lines[a..z].iter().map(|l| pieces_of(flow, l)).collect();
        // Blocks of an address, a bank or a register side by side are no table: their runs are
        // read block by block. Rows were possible too, so that is said. Under a header row of
        // their headings, they are read as the table they make, with the doubt said.
        let mut headed = false;
        if let Some((s, order, verdict)) =
            side_blocks(flow, &lines, (a, z, tail.end.max(z)), &pieces, &body_of)
        {
            if verdict == Columns::Headed {
                headed = true;
            } else {
                found.uncertain.push(format!(
                    "blocks of short lines side by side on page {page} are read one block after \
                     the other, as blocks of their own: if their lines are rows of a table or \
                     labels beside values, those rows are not kept"
                ));
                apart.push((s, order));
                continue;
            }
        }
        // A block of labels and values is a table of two columns.
        match key_values(flow, &lines, a, z, &pieces, &off) {
            KeyValues::No => {}
            KeyValues::TooMany(n) => {
                found.skipped.push(format!(
                    "a block of {n} label and value lines on page {page} is not read as a \
                     table: it has more than {MAX_SQUARES} cells; its text stays paragraphs"
                ));
                continue;
            }
            KeyValues::Rows(rows) => {
                let t = u32::try_from(claim.len()).unwrap_or(u32::MAX);
                let first = rows
                    .iter()
                    .flat_map(|r| [r.label.0, r.value.map_or(r.label.0, |v| v.0)])
                    .min()
                    .unwrap_or(0);
                let top = lines[rows[0].line];
                for (k, row) in rows.iter().enumerate() {
                    for slot in &mut target[row.label.0..row.label.1] {
                        *slot = Some((t, k as u32, 0));
                    }
                    match row.value {
                        Some((s, e)) => {
                            for slot in &mut target[s..e] {
                                *slot = Some((t, k as u32, 1));
                            }
                        }
                        None => splits.push((claim.len(), k)),
                    }
                }
                kv.push(claim.len());
                claim.push(Planned {
                    at: flow.at[first],
                    top: top.y + top.size,
                    rows: rows
                        .iter()
                        .map(|_| {
                            (0..2)
                                .map(|_| PlannedCell {
                                    runs: Vec::new(),
                                    row_span: 1,
                                    col_span: 1,
                                    status: None,
                                })
                                .collect()
                        })
                        .collect(),
                    header_rows: 0,
                    trs: Vec::new(),
                    additions: Vec::new(),
                    tagged: None,
                    continues: false,
                    caption: Vec::new(),
                    found_by: None,
                });
                continue;
            }
        }
        // The lines under the region that go on in its last row join it.
        let (joined, doubt, guessed) = joined_tail(flow, &lines[a..tail.end], &pieces, tail.marked);
        pieces.extend(lines[z..z + joined].iter().map(|l| pieces_of(flow, l)));
        z += joined;
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
        let mut refused: Option<(&'static str, f32, usize)> = None;
        // Why a region without a caption is no table, when its refusal is a loss.
        let mut lost: Option<&'static str> = None;
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
            let (caption_line, figure_caption) =
                captions_near(flow, &lines, &by_y, b, &line_sizes, &in_region);
            let table_caption = caption_line.is_some();
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
                                        status: None,
                                    })
                                    .collect()
                            })
                            .collect(),
                        header_rows: 0,
                        trs: Vec::new(),
                        additions: Vec::new(),
                        tagged: None,
                        continues: false,
                        caption: Vec::new(),
                        found_by: None,
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
                    // A line the grid leaves out is said, unless the loss of its frame says it.
                    let unframed = grid
                        .left
                        .iter()
                        .any(|&(s, e)| (s..e).any(|i| body_of(i).is_none()));
                    if unframed {
                        found.uncertain.push(format!(
                            "a line between the rows of a table on page {page} runs over its \
                             columns: it is read as a paragraph after the table"
                        ));
                    }
                    if goes_on_beside(flow, &lines[a..z], &by_y, b, &in_region) {
                        found.uncertain.push(format!(
                            "a table on page {page} may go on in a column beside it, on the \
                             baselines of its rows: that column is read apart from the table"
                        ));
                    }
                    if frame_titles(flow, &grid, &body_of, &found.body_labels) {
                        titled.push(claim.len());
                    }
                    if !grid.cuts.is_empty() {
                        title_cuts.push((claim.len(), grid.cuts));
                    }
                    claim.push(planned);
                }
                Err(why) => {
                    if let (Some(line), None) = (caption_line, refused) {
                        let why = if split { SIDE_BY_SIDE } else { why };
                        refused = Some((why, b[3], line));
                    }
                    // Next to a figure caption or on a picture, the region is a chart's.
                    if caption_line.is_none() && !split && !figure_caption && !area.on_picture(b) {
                        lost = Some(why);
                    }
                }
            }
        }
        if taken && headed {
            found.uncertain.push(format!(
                "a table on page {page} may be blocks of short lines side by side under their \
                 headings: its rows may not belong together"
            ));
        }
        if taken && guessed {
            found.uncertain.push(format!(
                "an amount alone under a table on page {page} is read as one more row of it, as \
                 the amounts alone over it are: it may be a total under the table"
            ));
        }
        if taken && doubt {
            found.uncertain.push(format!(
                "a line right under a table on page {page} may go on in its last row: it is \
                 read as a paragraph after the table"
            ));
        }
        // A grid of rules over the same text may have said so already.
        let (lo, hi) = (flow.at[lines[a].start], flow.at[lines[z - 1].end - 1]);
        let said = found.unsure.iter().any(|u| (lo..=hi).contains(&u.at));
        if let (false, Some(why)) = (taken, lost) {
            // A block of three lines or more in three columns or more looks like a table: its
            // refusal is a loss, unless a grid or band refused without a note holds its text
            // and its loss speaks for it.
            let columned = (a..z).filter(|&k| counts[k] >= 3).count();
            let quietly = (lines[a].start..lines[z - 1].end)
                .any(|i| found.quietly.get(i).copied().unwrap_or(false) || body_of(i).is_some());
            if z - a >= 3 && columned >= 3 && !said && !quietly {
                found.quiet.push((lo, why, Refused::Region));
            }
        }
        if let (false, Some((why, top, line))) = (taken, refused) {
            // A caption gates one note: a region it gated before said so.
            if !said && !noted.contains(&line) {
                noted.push(line);
                found.unsure.push(Unsure {
                    at: flow.at[lines[a].start],
                    top,
                    reason: why.to_string(),
                    text: None,
                });
            }
        }
    }
    // A frame of column titles over items whose every line no table took is said, by its
    // titles.
    let mut left: Vec<bool> = vec![false; found.body_labels.len()];
    for (i, b) in bodies.iter().enumerate() {
        if let Some(b) = b
            && target.get(i).is_none_or(Option::is_none)
            && !flow.runs[i].text.trim().is_empty()
        {
            left[*b as usize] = true;
        }
    }
    for (b, names) in found.body_labels.iter().enumerate() {
        if left[b] {
            found.uncertain.push(format!(
                "the lines under the column titles {} on page {page} are not all read as a \
                 table: their text is read as it stands",
                names.join(", ")
            ));
        }
    }
    // The runs of blocks side by side go in the flow block by block; their places stay.
    for (start, order) in apart {
        let runs: Vec<Run> = order.iter().map(|&i| flow.runs[i].clone()).collect();
        let slots: Vec<Slot> = order.iter().map(|&i| flow.slots[i]).collect();
        for (k, (r, sl)) in runs.into_iter().zip(slots).enumerate() {
            flow.runs[start + k] = r;
            flow.slots[start + k] = sl;
        }
    }
    if claim.is_empty() {
        return;
    }
    // The runs of all accepted regions leave the flow at once; each goes to its cell.
    let mut planned = claim;
    fill(&mut planned, flow, &target);
    for (t, k) in splits {
        split_label(&mut planned[t].rows[k]);
    }
    for (t, cuts) in title_cuts {
        for (r, cell, run, byte) in cuts {
            split_title(&mut planned[t].rows[r], cell, run, byte);
        }
    }
    for (t, mut p) in planned.into_iter().enumerate() {
        if titled.contains(&t) {
            p.header_rows = 1;
        } else if !kv.contains(&t) {
            p.header_rows = header_of(&p.rows);
        }
        found.tables.push(p);
    }
    found.tables.sort_by_key(|t| t.at);
}

/// Whether the first row of `grid` is the line of titles of one frame of column titles over
/// items (see [`Found::body_labels`]): all its runs are the frame's, and its text is the
/// frame's titles. Titles that wrap into a second line are no such row: the table keeps the
/// header that [`header_of`] finds, and where the second line has text in the columns of two
/// close titles, they stay one merged cell (see [`grid_of`]).
fn frame_titles(
    flow: &Flow,
    grid: &Grid,
    body_of: &dyn Fn(usize) -> Option<u32>,
    labels: &[Vec<String>],
) -> bool {
    let Some(first) = grid.rows.first() else {
        return false;
    };
    let mut ranges: Vec<(usize, usize)> = first.iter().flat_map(|c| c.ranges.clone()).collect();
    ranges.sort_unstable();
    let Some(&(s, _)) = ranges.first() else {
        return false;
    };
    let Some(frame) = body_of(s) else {
        return false;
    };
    if !ranges
        .iter()
        .all(|&(s, e)| (s..e).all(|i| body_of(i) == Some(frame)))
    {
        return false;
    }
    let text: Vec<String> = ranges.iter().map(|&(s, e)| text_of(flow, s, e)).collect();
    let titles = labels.get(frame as usize).map(|l| l.join(" "));
    titles.is_some_and(|t| {
        t.split_whitespace()
            .eq(text.iter().flat_map(|t| t.split_whitespace()))
    })
}

/// The lines right under a region that may go on in its last row: up to line `end`, either
/// lines of one piece, or (`marked`) lines whose every run the frame of column titles over
/// items the region holds holds too.
#[derive(Debug, Clone, Copy)]
struct Tail {
    end: usize,
    marked: bool,
}

/// A page number has at most this many words.
const PAGE_WORDS: usize = 4;

/// Whether `text` has the shape of a page number (`Seite 1`, `Seite 1 von 2`, `Page 2 of 3`,
/// `Seite 1/2`): a few words that start with a word of letters, each a word of letters or a
/// number (two numbers parted by a slash too), at least one a number.
fn page_number(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    let letters = |w: &str| {
        let w = w.trim_end_matches(['.', ':']);
        !w.is_empty() && w.chars().all(char::is_alphabetic)
    };
    let number = |w: &str| {
        w.split('/')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    };
    (2..=PAGE_WORDS).contains(&words.len())
        && letters(words[0])
        && words.iter().all(|w| letters(w) || number(w))
        && words.iter().any(|w| number(w))
}

/// Whether text beside the table of the region `lines` (its box `b`) is a column of it that
/// the reading order put apart: on each line of the region (two or more), text right of the
/// table, or each left of it, on the line's baseline (within [`BASELINE_NEAR`] of its size),
/// the texts of one side starting or ending together (within [`WRAP_ALIGN`] of the size), and
/// no other text of that column within one and a half steps of the region over its first line
/// or under its last: a column of the table's own rows, not prose that goes on beside it.
fn goes_on_beside(
    flow: &Flow,
    lines: &[FlowLine],
    by_y: &ByY,
    b: [f32; 4],
    in_region: &dyn Fn(usize) -> bool,
) -> bool {
    if lines.len() < 2 {
        return false;
    }
    let pitch = lines
        .windows(2)
        .map(|w| w[0].y - w[1].y)
        .fold(0.0f32, f32::max);
    let inked = |i: usize| !in_region(i) && !flow.runs[i].text.trim().is_empty();
    // The stretch of the text beside each line on one side, if every line has some.
    let side = |right: bool| -> Option<Vec<(f32, f32)>> {
        lines
            .iter()
            .map(|l| {
                let near = BASELINE_NEAR * l.size;
                let (mut x0, mut x1) = (f32::MAX, f32::MIN);
                for i in by_y.runs_between(l.y - l.size, l.y + l.size) {
                    let r = &flow.runs[i];
                    let beside = if right {
                        r.x >= b[2]
                    } else {
                        r.x + r.w <= b[0]
                    };
                    if inked(i) && (r.y - l.y).abs() <= near && beside && !r.off_axis {
                        x0 = x0.min(r.x);
                        x1 = x1.max(r.x + r.w);
                    }
                }
                (x0 <= x1).then_some((x0, x1))
            })
            .collect()
    };
    let size = lines.iter().map(|l| l.size).fold(0.0f32, f32::max);
    let tol = WRAP_ALIGN * size;
    let column = |spans: &[(f32, f32)]| {
        let together = |k: fn(&(f32, f32)) -> f32| {
            let lo = spans.iter().map(k).fold(f32::MAX, f32::min);
            let hi = spans.iter().map(k).fold(f32::MIN, f32::max);
            hi - lo <= tol
        };
        if !(together(|s| s.0) || together(|s| s.1)) {
            return false;
        }
        let (x0, x1) = spans
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, z), s| (a.min(s.0), z.max(s.1)));
        let (top, bottom) = (lines[0].y, lines[lines.len() - 1].y);
        let reach = 1.5 * pitch;
        // Other text of the column right over or under the rows.
        let more = |lo: f32, hi: f32| {
            by_y.runs_between(lo, hi).into_iter().any(|i| {
                let r = &flow.runs[i];
                inked(i) && r.x < x1 && r.x + r.w > x0 && (r.y > top || r.y < bottom)
            })
        };
        !more(top + 0.5 * size, top + reach) && !more(bottom - reach, bottom - 0.5 * size)
    };
    [true, false]
        .into_iter()
        .any(|right| side(right).is_some_and(|spans| column(&spans)))
}

/// Two lines of a region without rules stand at most this many times the widest step of the
/// region so far, or of the next two lines, apart: a wider gap parts two blocks, unless the
/// line after it stands in the region's columns (see [`in_columns`]).
const REGION_STEPS: f32 = 2.0;

/// A piece stands in a column of a region when it starts or ends within this many sizes of
/// where a piece of the region starts or ends.
const COLUMN_ALIGN: f32 = 0.25;

/// Whether every piece of `line` starts or ends where a piece of `region` starts or ends (see
/// [`COLUMN_ALIGN`]): its text stands in the region's columns, set flush left or flush right
/// as theirs.
fn in_columns(flow: &Flow, region: &[FlowLine], line: &FlowLine) -> bool {
    let tol = COLUMN_ALIGN * line.size;
    let edges: Vec<(f32, f32)> = region
        .iter()
        .flat_map(|l| pieces_of(flow, l))
        .map(|p| (p.x0, p.x1))
        .collect();
    let own = pieces_of(flow, line);
    !own.is_empty()
        && own.iter().all(|p| {
            edges
                .iter()
                .any(|e| (p.x0 - e.0).abs() <= tol || (p.x1 - e.1).abs() <= tol)
        })
}

/// Whether `line` has the shape of the header of a table of its own under the lines `region`,
/// the line `next` under it: two pieces or more, each words or a whole number of at most
/// [`HEADER_DIGITS`] digits (a year, `2025`), one with a letter, none an amount or a date; and
/// it stands apart from the region's rows by its style (all bold over rows that mostly are
/// not, or a size step larger than most of them, see [`crate::order::SIZE_STEP`]; a bold
/// total row among them does not count) or by its shape (no digit, over a line with a digit,
/// under rows that all hold one after the region's first line).
fn header_line(flow: &Flow, region: &[FlowLine], line: &FlowLine, next: &FlowLine) -> bool {
    if region.is_empty() {
        return false;
    }
    let own = pieces_of(flow, line);
    let texts: Vec<String> = own.iter().map(|p| text_of(flow, p.start, p.end)).collect();
    let year = |t: &str| t.len() <= HEADER_DIGITS && t.chars().all(|c| c.is_ascii_digit());
    let cells = own.len() >= 2
        && texts.iter().any(|t| t.chars().any(char::is_alphabetic))
        && texts.iter().all(|t| year(t) || !numeric(t));
    if !cells {
        return false;
    }
    let inked = |l: &FlowLine| {
        flow.runs[l.start..l.end]
            .iter()
            .filter(|r| !r.text.trim().is_empty())
            .collect::<Vec<_>>()
    };
    let bold = |l: &FlowLine| {
        let rs = inked(l);
        !rs.is_empty() && rs.iter().all(|r| r.bold)
    };
    let size = |l: &FlowLine| inked(l).iter().map(|r| r.size).fold(0.0f32, f32::max);
    // The rows' style: what most of them have.
    let body_bold = 2 * region.iter().filter(|l| bold(l)).count() > region.len();
    let mut sizes: Vec<f32> = region.iter().map(size).collect();
    sizes.sort_by(f32::total_cmp);
    let body_size = sizes[sizes.len() / 2];
    let styled = (bold(line) && !body_bold) || size(line) >= crate::order::SIZE_STEP * body_size;
    let digit = |l: &FlowLine| {
        flow.runs[l.start..l.end]
            .iter()
            .any(|r| r.text.chars().any(|c| c.is_ascii_digit()))
    };
    let shaped = region.len() > 1 && !digit(line) && digit(next) && region[1..].iter().all(digit);
    styled || shaped
}

/// A header cell may be a whole number of at most this many digits (a year).
const HEADER_DIGITS: usize = 4;

/// Whether `text` is an amount: a number (see [`Shape::Number`]) with a decimal part of two
/// digits (`20,00`, `1.000,00`, `12,50 EUR`). Whole numbers, rates and dates are none.
fn amount(text: &str) -> bool {
    if Shape::of(text) != Some(Shape::Number) {
        return false;
    }
    let mut t = text.trim();
    for c in CURRENCIES {
        t = t.trim_start_matches(c).trim_end_matches(c).trim();
    }
    t.rfind(['.', ','])
        .is_some_and(|at| t.len() - at == 3 && t[at + 1..].bytes().all(|c| c.is_ascii_digit()))
}

/// A line of one piece under a table may go on in its last row when it starts within this
/// many sizes of the start of its column; its font size may differ from the column's by this
/// share.
const WRAP_ALIGN: f32 = 0.1;

/// How many of the lines under a region (`lines`, the region's first, its `pieces` the
/// region's own) go on in its last row, whether the first that does not may (a loss says
/// so), and whether a line that joins as a row of its own may be a total (a loss says so).
/// - A marked line (see [`Tail`]) does when each of its pieces stands over one column of the
///   region (it crosses no gutter): the frame holds it.
/// - A line of one piece never does: a wrap of the last row and a note under the table look
///   the same. It may, whatever its first letter, when it is set as a wrap of a cell's text:
///   it stands over one column (it crosses no gutter), starts at the column's start (see
///   [`WRAP_ALIGN`]) and ends before the middle of the gutter after it, is set in the font
///   family and size of the column's text,
///   and stands no farther under the line over it than the lines of the region stand apart.
///   A column of numbers mostly holds no wrap, and a line of a number or an amount only, or
///   a page number by its shape (see [`page_number`]), is none.
fn joined_tail(
    flow: &Flow,
    lines: &[FlowLine],
    pieces: &[Vec<Piece>],
    marked: bool,
) -> (usize, bool, bool) {
    let core = pieces.len();
    if lines.len() <= core || core < 2 {
        return (0, false, false);
    }
    let columns = columns_of(pieces);
    let over = |p: &Piece| -> Vec<usize> {
        (0..columns.len())
            .filter(|&c| p.x1 > columns[c].0 && p.x0 < columns[c].1)
            .collect()
    };
    if !marked {
        let line = &lines[core];
        let own = pieces_of(flow, line);
        let [p] = own.as_slice() else {
            return (0, false, false);
        };
        let [c] = over(p)[..] else {
            return (0, false, false);
        };
        let pitch = (1..core)
            .map(|k| lines[k - 1].y - lines[k].y)
            .fold(0.0f32, f32::max);
        let size = line.size.max(1.0);
        let close = lines[core - 1].y - line.y <= pitch + WRAP_ALIGN * size;
        // It starts at its column's start and ends before the middle of the gutter after it,
        // as a line of a cell between rows must (see [`grid_of`]).
        let within = (p.x0 - columns[c].0).abs() <= WRAP_ALIGN * size
            && columns
                .get(c + 1)
                .is_none_or(|n| p.x1 <= (columns[c].1 + n.0) / 2.0);
        let text = text_of(flow, p.start, p.end);
        // The pieces of the column under its title, and their text.
        let cells: Vec<&Piece> = pieces[1..core]
            .iter()
            .flatten()
            .filter(|q| over(q) == [c])
            .collect();
        let texts: Vec<String> = cells
            .iter()
            .map(|q| text_of(flow, q.start, q.end))
            .collect();
        let numbers = 2 * texts.iter().filter(|t| numeric(t)).count() > texts.len();
        // Set as the column's text: each inked run of the line in a family and a size of it.
        let inked = |q: &Piece| {
            flow.runs[q.start..q.end]
                .iter()
                .filter(|r| !r.text.trim().is_empty())
                .collect::<Vec<_>>()
        };
        let faces: Vec<(u32, f32)> = cells
            .iter()
            .flat_map(|q| inked(q))
            .map(|r| (r.family, r.size))
            .collect();
        let same_face = inked(p).iter().all(|r| {
            faces
                .iter()
                .any(|f| f.0 == r.family && (f.1 - r.size).abs() <= WRAP_ALIGN * f.1)
        });
        let wrap =
            close && within && same_face && !numbers && !numeric(&text) && !page_number(&text);
        // An amount alone at the pitch of the rows, in the column of numbers it stands in (flush
        // with its start or its end), under an amount of the last line, is one more row of its
        // own (see [`grid_of`]) when the region has such rows already: amounts alone in the same
        // column, each under an amount (the body of a header band). It may still be a total
        // under the table, so a loss says so. Any other amount alone under a table may be a
        // total, and it stays out.
        let amount_in = |q: &Piece| amount(&text_of(flow, q.start, q.end));
        let lone_under = |k: usize, line: &[Piece]| {
            let [q] = line else { return false };
            over(q) == [c]
                && amount_in(q)
                && pieces[k - 1].iter().any(|a| over(a) == [c] && amount_in(a))
        };
        let stacked = (2..core).any(|k| lone_under(k, &pieces[k]));
        let flush =
            within || ((p.x1 - columns[c].1).abs() <= WRAP_ALIGN * size && p.x0 >= columns[c].0);
        if stacked && close && flush && same_face && lone_under(core, &own) {
            return (1, false, true);
        }
        return (0, wrap, false);
    }
    let mut joined = 0;
    for line in &lines[core..] {
        // A line that crosses a gutter is no text of the frame's cells: it stays out, and the
        // frame's loss says so.
        if !pieces_of(flow, line).iter().all(|p| over(p).len() == 1) {
            break;
        }
        joined += 1;
    }
    (joined, false, false)
}

/// A rule that ends a region without rules is at most its median text size divided by this
/// high and at least this many times that size wide, and spans this share of the ink of the
/// lines above it.
const THIN_RULE: f32 = 8.0;
const END_RULE: f32 = 4.5;
const END_SHARE: f32 = 0.9;

/// The glyphs of a line reach this share of its size over its baseline (its x-height) and
/// this share under it: a rule there crosses them (a strike-through stands at a quarter to a
/// third of the size).
const GLYPH_TOP: f32 = 0.5;
const GLYPH_FOOT: f32 = 0.2;

/// Where a rule of `ends` ends the region of lines `a..z`: the line right under the one rule
/// in the gap between two of its lines (see [`GLYPH_TOP`]) that is thin and long for its text (see [`THIN_RULE`]) and spans
/// [`END_SHARE`] of the ink of the lines above it. A rule under the first line underlines a
/// header, and rules between two pairs of lines or more part rows: neither ends the region.
fn ended(flow: &Flow, lines: &[FlowLine], (a, z): (usize, usize), ends: &[HRule]) -> Option<usize> {
    if ends.is_empty() || z - a < 3 {
        return None;
    }
    let mut sizes: Vec<f32> = lines[a..z].iter().map(|l| l.size).collect();
    sizes.sort_by(f32::total_cmp);
    let size = sizes[sizes.len() / 2];
    // The ink of the lines above the pair looked at, from the smallest run x to the largest
    // run end.
    let (mut x0, mut x1) = (f32::MAX, f32::MIN);
    let mut end = None;
    for k in a + 1..z {
        for r in &flow.runs[lines[k - 1].start..lines[k - 1].end] {
            x0 = x0.min(r.x);
            x1 = x1.max(r.x + r.w);
        }
        if k == a + 1 {
            continue;
        }
        // In the gap between the two lines, not through the glyphs of either (a rule through
        // an item strikes it out).
        let (lo, hi) = (
            lines[k].y + GLYPH_TOP * lines[k].size,
            lines[k - 1].y - GLYPH_FOOT * lines[k - 1].size,
        );
        let ruled = ends[ends.partition_point(|e| e.y <= lo)..]
            .iter()
            .take_while(|e| e.y < hi)
            .any(|e| {
                e.h <= size / THIN_RULE
                    && e.x1 - e.x0 >= END_RULE * size
                    && e.x1.min(x1) - e.x0.max(x0) >= END_SHARE * (x1 - x0)
            });
        if ruled {
            if end.is_some() {
                return None;
            }
            end = Some(k);
        }
    }
    end
}

/// Lines of labels and values one table may have at most.
const MAX_KEY_VALUES: usize = MAX_SQUARES / 2;

/// Labels and values line up within this many points.
const KV_ALIGN: f32 = 1.0;

/// A label is at most this many characters and words; a value at most this many characters.
const KV_LABEL: usize = 40;
const KV_LABEL_WORDS: usize = 6;
const KV_VALUE: usize = 80;

/// One row of a block of labels and values: its line, and the flow ranges of its label and
/// value; a row without a value range has both in one run, cut after the label's colon.
struct KeyValue {
    line: usize,
    label: (usize, usize),
    value: Option<(usize, usize)>,
}

enum KeyValues {
    No,
    /// A block of this many lines: more than one table may have.
    TooMany(usize),
    Rows(Vec<KeyValue>),
}

/// The block of labels and values the lines `a..z` of a region are, when they are one: at
/// least three lines in a row of exactly two pieces, the labels flush left, the values flush
/// left or right or the labels ending in a colon, no label or value wrapping, short pieces,
/// and no bold first line over plain ones (a header). A line right after the block whose one
/// piece starts with a short label and a colon joins it, cut after the colon.
fn key_values(
    flow: &Flow,
    lines: &[FlowLine],
    a: usize,
    z: usize,
    pieces: &[Vec<Piece>],
    off: &[bool],
) -> KeyValues {
    if pieces.iter().any(|p| p.len() != 2) || z - a < 2 {
        return KeyValues::No;
    }
    if z - a > MAX_KEY_VALUES {
        return KeyValues::TooMany(z - a);
    }
    let label = |p: &Piece| text_of(flow, p.start, p.end);
    let labels: Vec<String> = pieces.iter().map(|p| label(&p[0])).collect();
    let colons = labels.iter().all(|l| l.ends_with(':'));
    // Lines after the region whose one run starts with a short label and a colon.
    let mut extra: Vec<usize> = Vec::new();
    let mut end = z;
    while colons && end < lines.len() && !off[end] && extra.len() < MAX_KEY_VALUES {
        let (prev, cur) = (&lines[end - 1], &lines[end]);
        let gap = prev.y - cur.y;
        if gap <= 0.0 || gap > 2.0 * prev.size.max(cur.size) || cur.end - cur.start != 1 {
            break;
        }
        let text = flow.runs[cur.start].text.trim();
        let Some(cut) = text.find(": ") else { break };
        let head = &text[..=cut];
        if head.chars().count() > KV_LABEL || head.split_whitespace().count() > KV_LABEL_WORDS {
            break;
        }
        if (flow.runs[cur.start].x - pieces[0][0].x0).abs() > KV_ALIGN {
            break;
        }
        extra.push(end);
        end += 1;
    }
    let n = z - a + extra.len();
    if n < 3 {
        return KeyValues::No;
    }
    let x0 = pieces[0][0].x0;
    let lefts = pieces.iter().all(|p| (p[0].x0 - x0).abs() <= KV_ALIGN);
    let (v0, v1) = (pieces[0][1].x0, pieces[0][1].x1);
    let flush_left = pieces.iter().all(|p| (p[1].x0 - v0).abs() <= KV_ALIGN);
    let flush_right = pieces.iter().all(|p| (p[1].x1 - v1).abs() <= KV_ALIGN);
    if !lefts || !(flush_left || flush_right || colons) {
        return KeyValues::No;
    }
    let short = labels
        .iter()
        .all(|l| l.chars().count() <= KV_LABEL && l.split_whitespace().count() <= KV_LABEL_WORDS)
        && pieces
            .iter()
            .all(|p| text_of(flow, p[1].start, p[1].end).chars().count() <= KV_VALUE);
    if !short {
        return KeyValues::No;
    }
    // Lines of the block follow each other at most two line heights apart.
    let spaced = (a + 1..z).all(|k| {
        let gap = lines[k - 1].y - lines[k].y;
        gap > 0.0 && gap <= 2.0 * lines[k - 1].size.max(lines[k].size)
    });
    // A line right after the block that starts under the values goes on a value.
    let wraps = lines.get(end).is_some_and(|next| {
        let last = &lines[end - 1];
        let gap = last.y - next.y;
        gap > 0.0
            && gap <= 1.6 * last.size.max(next.size)
            && (flow.runs[next.start].x - v0).abs() <= KV_ALIGN
    });
    let bold = |l: &FlowLine| flow.runs[l.start..l.end].iter().all(|r| r.bold);
    let header = bold(&lines[a]) && (a + 1..z).any(|k| !bold(&lines[k]));
    if !spaced || wraps || header {
        return KeyValues::No;
    }
    let mut rows: Vec<KeyValue> = pieces
        .iter()
        .enumerate()
        .map(|(k, p)| KeyValue {
            line: a + k,
            label: (p[0].start, p[0].end),
            value: Some((p[1].start, p[1].end)),
        })
        .collect();
    rows.extend(extra.into_iter().map(|k| KeyValue {
        line: k,
        label: (lines[k].start, lines[k].end),
        value: None,
    }));
    KeyValues::Rows(rows)
}

/// Cuts a row of two cells whose first cell holds label and value in one run after the
/// label's colon: the rest goes to the second cell.
fn split_label(row: &mut [PlannedCell]) {
    let [label, value] = row else { return };
    let Some(run) = label.runs.first_mut() else {
        return;
    };
    let lead = run.text.len() - run.text.trim_start().len();
    let Some(cut) = run.text[lead..].find(": ").map(|c| lead + c + 1) else {
        return;
    };
    let rest = cut_run(run, cut);
    value.runs.push(rest);
    value.runs.extend(label.runs.drain(1..));
}

/// Cuts `run` at byte `at` of its text: it keeps the text before, and the text after, white
/// space at the cut trimmed, comes back as a run of its own. The cut is at the gap between
/// the words where the glyphs say where it is; else the run's width is shared out by its
/// characters.
fn cut_run(run: &mut Run, at: usize) -> Run {
    let words = run.text[..at].split_whitespace().count() as u32;
    let cut_at_gap = run
        .gaps
        .iter()
        .position(|g| g.words == words)
        .and_then(|k| run.cut_at(k));
    if let Some((before, mut rest)) = cut_at_gap {
        rest.text = rest.text.trim_start().to_string();
        *run = before;
        return rest;
    }
    let mut rest = run.clone();
    let chars = run.text.chars().count().max(1) as f32;
    let before = run.text[..at].chars().count() as f32;
    rest.text = run.text[at..].trim_start().to_string();
    rest.x = run.x + run.w * before / chars;
    rest.w = run.w - (rest.x - run.x);
    rest.gaps = Gaps::default();
    run.text.truncate(at);
    run.text.truncate(run.text.trim_end().len());
    run.w -= rest.w;
    run.gaps = Gaps::default();
    rest
}

/// Cuts the cell `cell` of `row` (a header row) at byte `byte` of its run `run`: the text
/// after the cut goes first into the next cell, the cell of the next column.
fn split_title(row: &mut [PlannedCell], cell: usize, run: usize, byte: usize) {
    let Some(r) = row.get_mut(cell).and_then(|c| c.runs.get_mut(run)) else {
        return;
    };
    if byte == 0 || byte >= r.text.len() {
        return;
    }
    let rest = cut_run(r, byte);
    // The cut cell spans one column, so the next column has a cell of its own.
    debug_assert!(row.len() > cell + 1, "a cut title has no next cell");
    if let Some(next) = row.get_mut(cell + 1) {
        next.runs.insert(0, rest);
    }
}

/// A grid rebuilt from white space gutters: rows of cells, each cell its runs as flow ranges;
/// the runs of close titles to cut; and the flow ranges of the lines it leaves out (see
/// [`grid_of`]).
struct Grid {
    rows: Vec<Vec<GridCell>>,
    cuts: Vec<TitleCut>,
    left: Vec<(usize, usize)>,
}

/// A run of close titles to cut, once the runs are in their cells: its row, its cell, its
/// place among the runs of the cell and the byte of its text the next title starts at (see
/// [`title_cut`]).
type TitleCut = (usize, usize, usize, usize);

/// Where a piece of a line goes in a grid: its column, its column span and its flow range.
type Placement = (usize, u32, (usize, usize));

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
/// either the region begins or ends with two lines or more on one side only, or the baselines
/// of its sides do not line up.
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
    // A list that starts earlier or ends later than the one beside it: the region begins or
    // ends with two lines or more that hold text on the same side only. (A table's group
    // heading is one line on one side, between rows that fill both.)
    let side: Vec<u8> = pieces
        .iter()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let left = l.iter().any(|p| p.x1 <= mid);
            let right = l.iter().any(|p| p.x0 >= mid);
            u8::from(left) | (u8::from(right) << 1)
        })
        .collect();
    let run = |it: &mut dyn Iterator<Item = &u8>| -> usize {
        let mut it = it.peekable();
        let Some(&&first) = it.peek() else { return 0 };
        if first == 3 {
            return 0;
        }
        it.take_while(|s| **s == first).count()
    };
    if run(&mut side.iter()) >= 2 || run(&mut side.iter().rev()) >= 2 {
        return Some(mid);
    }
    // The baselines and sizes of a side, line by line: those of its runs.
    let side = |left: bool| -> Vec<Vec<Part>> {
        pieces
            .iter()
            .map(|l| {
                l.iter()
                    .filter(|p| if left { p.x1 <= mid } else { p.x0 >= mid })
                    .flat_map(|p| {
                        flow.runs[p.start..p.end]
                            .iter()
                            .map(|r| (r.y, r.size, !r.text.trim().is_empty()))
                    })
                    .collect::<Vec<Part>>()
            })
            .filter(|l| !l.is_empty())
            .collect()
    };
    offset_baselines(&side(true), &side(false), size).then_some(mid)
}

/// Whether the lines `a..z` of a region (their `pieces`) are blocks side by side (see
/// [`address_columns`]), with the lines of one piece under them up to `y` that end a longer
/// block, or without those: where their runs start in the flow, the runs in the order to read
/// them, block by block, and the verdict. Lines a frame of column titles over items holds are
/// no such blocks.
fn side_blocks(
    flow: &Flow,
    lines: &[FlowLine],
    (a, z, y): (usize, usize, usize),
    pieces: &[Vec<Piece>],
    body_of: &dyn Fn(usize) -> Option<u32>,
) -> Option<(usize, Vec<usize>, Columns)> {
    let ends = if y > z { vec![y, z] } else { vec![z] };
    for end in ends {
        let (s, e) = (lines[a].start, lines[end - 1].end);
        if (s..e).any(|i| body_of(i).is_some()) {
            return None;
        }
        let mut all = pieces.to_vec();
        all.extend(lines[z..end].iter().map(|l| pieces_of(flow, l)));
        let (columns, order) = blocks_of(flow, &all);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        if !sorted.iter().copied().eq(s..e) {
            continue;
        }
        match address_columns(&columns) {
            Columns::Rows => {}
            verdict => return Some((s, order, verdict)),
        }
    }
    None
}

/// The lines of a region by column (see [`columns_of`]), each column's line on each row, and
/// the region's runs column by column, each column top down. A piece over two columns, two
/// pieces of a row in one column or a piece in no column gives no columns.
fn blocks_of(flow: &Flow, pieces: &[Vec<Piece>]) -> (Vec<Vec<Option<ColumnLine>>>, Vec<usize>) {
    let columns = columns_of(pieces);
    let none = || (Vec::new(), Vec::new());
    let mut lines: Vec<Vec<Option<ColumnLine>>> = vec![vec![None; pieces.len()]; columns.len()];
    let mut runs: Vec<Vec<usize>> = vec![Vec::new(); columns.len()];
    for (row, line) in pieces.iter().enumerate() {
        for p in line {
            let hit: Vec<usize> = (0..columns.len())
                .filter(|&c| p.x1 > columns[c].0 && p.x0 < columns[c].1)
                .collect();
            let [c] = hit[..] else {
                return none();
            };
            if lines[c][row].is_some() {
                return none();
            }
            let inked: Vec<&Run> = flow.runs[p.start..p.end]
                .iter()
                .filter(|r| !r.text.trim().is_empty())
                .collect();
            if inked.is_empty() {
                return none();
            }
            lines[c][row] = Some(ColumnLine {
                text: text_of(flow, p.start, p.end),
                bold: inked.iter().all(|r| r.bold),
                size: inked.iter().map(|r| r.size).fold(0.0f32, f32::max),
            });
            runs[c].extend(p.start..p.end);
        }
    }
    (lines, runs.into_iter().flatten().collect())
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
    // The column of each piece, and its flow range; a line whose piece crosses a gutter is
    // not clean.
    let mut clean = 0usize;
    let mut placed: Vec<Vec<Placement>> = Vec::with_capacity(lines.len());
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
            row.push((col, span, (p.start, p.end)));
        }
        if ok {
            clean += 1;
        }
        placed.push(row);
    }
    if (clean as f32) < CLEAN_ROWS * lines.len() as f32 {
        return Err("its columns are not clean");
    }
    // Rows: lines that fill at least half the columns; other lines join the row of the
    // nearest one above them (lines above the first join the first).
    let filled = |row: &Vec<Placement>| {
        let mut c: Vec<usize> = row.iter().map(|x| x.0).collect();
        c.dedup();
        c.len()
    };
    // A line of amounts only (see [`amount`]), each under an amount of the row above in its
    // column, is a row of its own too: two amounts in one cell are never one cell's (the rule of
    // a header band's body, see `banded::row_starts`). A date or a range under a date, a whole
    // number under a whole number (two article numbers) and a rate under a rate go on in their
    // cell.
    let mut anchors: Vec<usize> = Vec::new();
    // The columns of the row being built that hold an amount.
    let mut above: Vec<usize> = Vec::new();
    for (k, row) in placed.iter().enumerate() {
        let number = |&(_, span, (s, e)): &Placement| span == 1 && amount(&text_of(flow, s, e));
        let stacked = !anchors.is_empty()
            && !row.is_empty()
            && row.iter().all(|x| number(x) && above.contains(&x.0));
        if 2 * filled(row) >= cols || stacked {
            anchors.push(k);
            above.clear();
        }
        above.extend(row.iter().filter(|x| number(x)).map(|x| x.0));
    }
    if anchors.len() < rows_needed.max(1) {
        return Err("too few rows");
    }
    // The row each line goes to: its own when it is an anchor, else the one before it.
    let row_of = |k: usize| {
        let after = anchors.partition_point(|&a| a < k);
        if anchors.get(after) == Some(&k) {
            after
        } else {
            after.saturating_sub(1)
        }
    };
    // Close titles of the first line over two columns are cut at the gutter between them.
    // The cut that falls inside a run is made once the runs are in their cells.
    let mut inside: Vec<(usize, usize)> = Vec::new();
    let first = std::mem::take(&mut placed[0]);
    for (k, &(c, span, (start, end))) in first.iter().enumerate() {
        // Both columns hold text of their own in two lines below or more, and no other line
        // of the first row has text in them.
        let alone = |col: usize| {
            (1..lines.len())
                .filter(|&j| placed[j].iter().any(|x| x.0 == col && x.1 == 1))
                .count()
                >= 2
        };
        let shared = (1..lines.len()).any(|j| {
            row_of(j) == row_of(0)
                && placed[j]
                    .iter()
                    .any(|x| x.0 <= c + 1 && x.0 + x.1 as usize > c)
        });
        let cut = if span == 2 && !shared && alone(c) && alone(c + 1) {
            // The shape of most values of a column, from its own text in the lines below.
            let shape_of = |col: usize| {
                let shapes: Vec<Option<Shape>> = (1..lines.len())
                    .flat_map(|j| placed[j].iter().filter(move |x| x.0 == col && x.1 == 1))
                    .map(|x| Shape::of(&text_of(flow, x.2.0, x.2.1)))
                    .collect();
                Shape::most(&shapes)
            };
            // A column with no shape allows no cut.
            let differ = matches!((shape_of(c), shape_of(c + 1)), (Some(a), Some(b)) if a != b);
            title_cut(flow, &pieces[0][k], (columns[c], columns[c + 1]), differ)
        } else {
            None
        };
        let row = &mut placed[0];
        match cut {
            Some((run, 0)) => {
                row.push((c, 1, (start, run)));
                row.push((c + 1, 1, (run, end)));
            }
            Some((run, byte)) => {
                row.push((c, 1, (start, run + 1)));
                if run + 1 < end {
                    row.push((c + 1, 1, (run + 1, end)));
                }
                inside.push((run, byte));
            }
            None => row.push((c, span, (start, end))),
        }
    }
    // A line under the first that would join the row of another and has a piece over a gutter,
    // or one that runs past the middle of a gutter (a note between rows), is no text of the
    // cells: it would make one cell of two columns, or put the note in a cell. It stays out of
    // the grid, its text read as it stands.
    let runs_over = |&(col, span, (s, e)): &Placement| {
        let rs = &flow.runs[s..e];
        let x0 = rs.iter().map(|r| r.x).fold(f32::MAX, f32::min);
        let x1 = rs.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max);
        let before = col
            .checked_sub(1)
            .is_some_and(|c| x0 < (columns[c].1 + columns[col].0) / 2.0);
        let after = columns
            .get(col + 1)
            .is_some_and(|n| x1 > (columns[col].1 + n.0) / 2.0);
        span > 1 || before || after
    };
    let mut left: Vec<(usize, usize)> = Vec::new();
    for (k, row) in placed.iter_mut().enumerate().skip(1) {
        if anchors.binary_search(&k).is_err() && row.iter().any(runs_over) {
            left.extend(row.drain(..).map(|x| x.2));
        }
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
        for &(col, span, range) in row {
            let cell = &mut rows[row_of(k)][col];
            cell.ranges.push(range);
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
    // Each cut inside a run: the cell that holds the run, and the run's place among the runs
    // of the cell, which go to it in flow order.
    let r0 = row_of(0);
    let cuts = inside
        .into_iter()
        .filter_map(|(run, byte)| {
            let cell = rows[r0]
                .iter()
                .position(|c| c.ranges.iter().any(|&(s, e)| (s..e).contains(&run)))?;
            let before = rows[r0][cell]
                .ranges
                .iter()
                .map(|&(s, e)| run.clamp(s, e) - s)
                .sum();
            Some((r0, cell, before, byte))
        })
        .collect();
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
    Ok(Grid { rows, cuts, left })
}

/// Each part of a cut title stands within this many sizes of its column's start or of its
/// column's end (a title is set flush left or flush right with its values).
const TITLE_ALIGN: f32 = 0.25;

/// A part of a cut title that covers its column ends at most this many sizes after the
/// column's end.
const COVER_END: f32 = 0.5;

/// The shape of a value: what a column of values holds. A shape read wrong may only keep a
/// title merged, never cut one, so the shapes are few and broad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// A whole number, a decimal or an amount: digits with thousands or decimal marks, a sign
    /// before them and a currency sign or code before or after them (`1.200`, `-6,98`,
    /// `12,50 EUR`, `\u{20ac} 3,00`).
    Number,
    /// A number with a percent sign (`19%`, `19 %`).
    Percent,
    /// A day, a month and a year in range, parted by dots, slashes or hyphens (`01.01.2026`,
    /// `2026-01-01`, `1/31/2026`, `31.01.26`).
    Date,
    /// Letters and no digits (`Std`, `Stk`).
    Text,
}

/// Currency signs and codes a number may have before or after it.
const CURRENCIES: [&str; 7] = ["\u{20ac}", "$", "\u{a3}", "EUR", "USD", "GBP", "CHF"];

impl Shape {
    /// The shape of `text`; none for a value with digits in a form none of the shapes knows
    /// (`12,-`, `6,98-`, `(6,98)`, `8:00`, `50 ct`, `1.2.24`) and for one with neither digits
    /// nor letters.
    fn of(text: &str) -> Option<Shape> {
        let t = text.trim();
        if let Some(n) = t.strip_suffix('%')
            && number(n.trim_end())
        {
            return Some(Shape::Percent);
        }
        if date(t) {
            return Some(Shape::Date);
        }
        let mut n = t;
        for c in CURRENCIES {
            n = n.strip_prefix(c).unwrap_or(n).trim();
            n = n.strip_suffix(c).unwrap_or(n).trim();
        }
        if number(n) {
            return Some(Shape::Number);
        }
        let letters = t.chars().any(char::is_alphabetic);
        let digits = t.chars().any(|c| c.is_ascii_digit());
        (letters && !digits).then_some(Shape::Text)
    }

    /// The shape more than half of `shapes` have; none on a tie, without a majority, when
    /// most values have no shape, or without values.
    fn most(shapes: &[Option<Shape>]) -> Option<Shape> {
        shapes
            .iter()
            .find(|s| s.is_some() && 2 * shapes.iter().filter(|x| x == s).count() > shapes.len())
            .copied()
            .flatten()
    }
}

/// Whether `text` is a number: a sign before it, digits, or groups of three digits after
/// the first one to three parted by one thousands mark (a dot, a comma or a space), and a
/// decimal part after the other mark (`1.234,56`, `1,200`, `1 200`, `-6,98`, `3.50`).
fn number(text: &str) -> bool {
    let t = text.strip_prefix(['-', '+']).unwrap_or(text);
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    // The decimal part: after the last dot or comma, unless that mark is the one between
    // groups of three (then the number has none).
    let whole = |s: &str, mark: Option<char>| -> bool {
        let Some(mark) = mark else { return digits(s) };
        let groups: Vec<&str> = s.split(mark).collect();
        (1..=3).contains(&groups[0].len())
            && groups.iter().all(|g| digits(g))
            && groups[1..].iter().all(|g| g.len() == 3)
    };
    let marks = |s: &str| s.chars().find(|c| matches!(c, '.' | ',' | ' '));
    if whole(t, marks(t)) || digits(t) {
        return true;
    }
    // A decimal part after the last dot or comma, the whole part before it with the other
    // mark (or a space) between groups of three, if any.
    let Some(at) = t.rfind(['.', ',']) else {
        return false;
    };
    let (int, dec) = (&t[..at], &t[at + 1..]);
    let decimal = t[at..].chars().next();
    let thousands = marks(int);
    digits(dec) && thousands != decimal && whole(int, thousands)
}

/// Whether `text` is a date: three groups of digits parted by one kind of mark, a year of
/// four digits first (`2026-01-01`) or last, or of two digits last after a day and a month of
/// two digits each (`31.01.26`), and a day and a month in range (day first, or month first
/// with slashes: `1/31/2026`).
fn date(text: &str) -> bool {
    let Some(mark) = text.chars().find(|c| matches!(c, '.' | '/' | '-')) else {
        return false;
    };
    let parts: Vec<&str> = text.split(mark).collect();
    let [a, b, c] = parts[..] else { return false };
    if [a, b, c]
        .iter()
        .any(|p| p.is_empty() || !p.chars().all(|ch| ch.is_ascii_digit()))
    {
        return false;
    }
    let value = |p: &str| p.parse::<u32>().unwrap_or(0);
    let day = |d: &str| (1..=31).contains(&value(d)) && d.len() <= 2;
    let month = |m: &str| (1..=12).contains(&value(m)) && m.len() <= 2;
    if a.len() == 4 {
        return month(b) && day(c);
    }
    // A year of two digits only after a day and a month of two digits each.
    let year = c.len() == 4 || (c.len() == 2 && a.len() == 2 && b.len() == 2);
    year && ((day(a) && month(b)) || (mark == '/' && month(a) && day(b)))
}

/// Where the piece `p` of a header line, over the columns `left` and `right` side by side, is
/// cut into two titles: the last word space whose words before it end before `right` and
/// whose words after it start after `left`, so that each part stands over one column only.
/// Gives the run whose text the cut starts in, and its byte; byte 0 when the cut lies between
/// two runs. A word space inside a run is where the glyphs say, else where the run's width
/// shared out by its characters puts it.
///
/// One title over both columns has word spaces over the gutter too, so a cut also needs
/// (ruling P6-X2 for the first):
/// - the two columns hold values of different shapes (see [`Shape`]): two close titles head
///   different kinds of values (a quantity and a tax rate), a title over both stands over
///   values of one kind (`Gesamt Preis` over two columns of numbers, `Date Range` over two
///   columns of dates);
/// - each part lines up with its own column, at its start or at its end, or covers the
///   column's whole width and ends at most [`COVER_END`] after it (see [`TITLE_ALIGN`];
///   values set inside a wider title, as a digit under `Menge`); a title centered over two
///   columns wider than its words does neither (the words before the space never reach into
///   the other column, see above). Measured: the parts of close titles over values set inside
///   them end 0.29 to 0.38 sizes after their columns, the words of titles centered over two
///   narrow columns 1.2 sizes and more;
/// - the words on both sides of the space do not start with a small letter: a title starts
///   with a capital, a digit or a sign, and a word in small letters goes on the words before
///   it (`Gueltig von bis`, `Zeitraum der Leistung`).
///
/// The small letter is what tells the two apart; the line-up alone cannot. A title of many
/// words set from the first column's start lines up with both columns as two close titles
/// do: measured over sizes, fonts, weights and row counts, its last word starts 0.07 to 0.09
/// sizes from the second column, while close titles stand up to 0.159 sizes off. The small
/// letter is a signal of the text of German and English titles, not a rule fitted to
/// samples. Where these cannot tell, the title stays one cell over both columns: a merged
/// cell is never a wrong cell.
fn title_cut(
    flow: &Flow,
    p: &Piece,
    (left, right): ((f32, f32), (f32, f32)),
    differ: bool,
) -> Option<(usize, usize)> {
    let size = flow.runs[p.start..p.end]
        .iter()
        .map(|r| r.size)
        .fold(1.0f32, f32::max);
    let near = |a: f32, b: f32| (a - b).abs() <= TITLE_ALIGN * size;
    // A part from `x0` to `x1` over the whole width of `column`, ending near its end, over
    // values of another shape than the other column's.
    let covers = |x0: f32, x1: f32, column: (f32, f32)| {
        x0 <= column.0 + TITLE_ALIGN * size
            && x1 >= column.1 - TITLE_ALIGN * size
            && x1 <= column.1 + COVER_END * size
    };
    // A space from `from` to `to` that lets both parts stand over their own column, each lined
    // up with it, between two words that start with no small letter.
    let fits = |from: f32, to: f32, before: &str, after: &str| {
        let small = |w: &str| w.chars().next().is_some_and(char::is_lowercase);
        let last = before.split_whitespace().next_back().unwrap_or("");
        differ
            && from > left.0
            && from <= right.0
            && to >= left.1
            && to < right.1
            && (near(p.x0, left.0) || near(from, left.1) || covers(p.x0, from, left))
            && (near(to, right.0) || near(p.x1, right.1) || covers(to, p.x1, right))
            && !small(last)
            && !small(after.trim_start())
    };
    let mut cut = None;
    for i in p.start..p.end {
        let r = &flow.runs[i];
        if i > p.start {
            let prev = &flow.runs[i - 1];
            if !r.text.trim().is_empty() && fits(prev.x + prev.w, r.x, &prev.text, &r.text) {
                cut = Some((i, 0));
            }
        }
        // The word starts of the run's text after its first word, by bytes and characters.
        let mut starts: Vec<(usize, usize, usize)> = Vec::new();
        let (mut seen, mut space) = (false, None);
        for (n, (b, ch)) in r.text.char_indices().enumerate() {
            if ch.is_whitespace() {
                if seen && space.is_none() {
                    space = Some(n);
                }
            } else {
                if let Some(from) = space.take() {
                    starts.push((b, from, n));
                }
                seen = true;
            }
        }
        let chars = r.text.chars().count().max(1) as f32;
        for (w, &(byte, from, to)) in starts.iter().enumerate() {
            let words = w as u32 + 1;
            let (x0, x1) = match r.gaps.iter().find(|g| g.words == words) {
                Some(g) => (g.from, g.to),
                None if r.gaps.is_empty() => (
                    r.x + r.w * from as f32 / chars,
                    r.x + r.w * to as f32 / chars,
                ),
                None => continue,
            };
            if fits(x0, x1, &r.text[..byte], &r.text[byte..]) {
                cut = Some((i, byte));
            }
        }
    }
    cut
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
            family: 0,
            mcid: None,
            uri: None,
            line,
            off_axis: false,
            note: false,
            gaps: crate::layout::Gaps::default(),
        }
    }

    /// A flow of `runs` on one line.
    fn flow_of(runs: Vec<Run>) -> Flow {
        let n = runs.len();
        Flow {
            runs,
            slots: vec![Slot::Free; n],
            at: (0..n).collect(),
        }
    }

    #[test]
    fn close_titles_are_cut_between_runs_and_by_characters() {
        let (left, right) = ((300.0, 305.0), (330.0, 345.0));
        let piece = |flow: &Flow| Piece {
            x0: flow.runs[0].x,
            x1: flow.runs.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max),
            start: 0,
            end: flow.runs.len(),
        };
        // Two runs: between them, from the end of the first to the start of the second.
        let two = flow_of(vec![
            run("Menge", 300.0, 600.0, 0),
            run("Einheit", 328.0, 600.0, 0),
        ]);
        assert_eq!(
            title_cut(&two, &piece(&two), (left, right), true),
            Some((1, 0))
        );
        // One run without glyph gaps: its 13 characters share its width of 65, so the space
        // lies from 325 to 330, over the gutter.
        let one = flow_of(vec![run("Menge Einheit", 300.0, 600.0, 0)]);
        assert_eq!(
            title_cut(&one, &piece(&one), (left, right), true),
            Some((0, 6))
        );
        // A space that lies over the left column (the words after it start in it) is none.
        assert_eq!(
            title_cut(&one, &piece(&one), ((300.0, 331.0), (335.0, 345.0)), true),
            None
        );
        let mut row = vec![
            PlannedCell {
                runs: one.runs.clone(),
                row_span: 1,
                col_span: 1,
                status: None,
            },
            PlannedCell {
                runs: Vec::new(),
                row_span: 1,
                col_span: 1,
                status: None,
            },
        ];
        split_title(&mut row, 0, 0, 6);
        let texts: Vec<&str> = row.iter().map(|c| c.runs[0].text.as_str()).collect();
        assert_eq!(texts, ["Menge", "Einheit"]);
        assert_eq!((row[1].runs[0].x, row[1].runs[0].w), (330.0, 35.0));
    }

    #[test]
    fn values_have_shapes() {
        use Shape::{Date, Number, Percent, Text};
        for (text, shape) in [
            // Numbers: whole, decimal, amounts, thousands marks, signs and currencies.
            ("2", Some(Number)),
            ("-12", Some(Number)),
            ("+5", Some(Number)),
            ("-6,98", Some(Number)),
            ("3.50", Some(Number)),
            ("1.234,56", Some(Number)),
            ("1,234.56", Some(Number)),
            ("1.200", Some(Number)),
            ("1,200", Some(Number)),
            ("1 200", Some(Number)),
            ("1.234.567", Some(Number)),
            ("12,50 EUR", Some(Number)),
            ("12,50 \u{20ac}", Some(Number)),
            ("\u{20ac} 12,50", Some(Number)),
            ("EUR 12,50", Some(Number)),
            ("$3.50", Some(Number)),
            // Percentages.
            ("19%", Some(Percent)),
            ("19 %", Some(Percent)),
            // Dates: a year of four digits, or of two after a day and a month of two.
            ("01.01.2026", Some(Date)),
            ("31.01.26", Some(Date)),
            ("2026-01-01", Some(Date)),
            ("1/31/2026", Some(Date)),
            ("31/12/2026", Some(Date)),
            // Text: letters and no digits.
            ("Stk", Some(Text)),
            ("Std", Some(Text)),
            // No shape: digits in a form no shape knows, or neither digits nor letters.
            ("12,-", None),
            ("6,98-", None),
            ("(6,98)", None),
            ("8:00", None),
            ("50 ct", None),
            ("1.2.24", None),
            ("32.01.2026", None),
            ("01.13.2026", None),
            ("1..2", None),
            ("12.34.56", None),
            ("Jan 26", None),
            ("-", None),
            ("", None),
        ] {
            assert_eq!(Shape::of(text), shape, "{text:?}");
        }
        let (n, t) = (Some(Number), Some(Text));
        assert_eq!(Shape::most(&[n, t, n]), Some(Number));
        assert_eq!(Shape::most(&[n, t, n, t]), None, "a tie");
        assert_eq!(Shape::most(&[n, t, Some(Percent)]), None, "no majority");
        assert_eq!(Shape::most(&[None, None, n]), None, "most have no shape");
        assert_eq!(Shape::most(&[]), None);
    }

    #[test]
    fn page_numbers_are_told_by_their_shape() {
        for page in [
            "Seite 1",
            "Seite 1 von 2",
            "Page 2 of 3",
            "Seite 1/2",
            "S. 3",
        ] {
            assert!(page_number(page), "{page}");
        }
        for other in [
            "Danke",
            "Frei Haus",
            "1.469,13",
            "Gesamt 1.469,13",
            "1 2",
            "Seite",
        ] {
            assert!(!page_number(other), "{other}");
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
