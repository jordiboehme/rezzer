// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables without rules, from the white space between columns.

use std::collections::HashSet;

use layoutregions::PrimId;
use layoutregions::segment::Segment;

use layoutregions::extract::table::grid::{self, Piece, Shape, amount, columns_of, grid_cells};
use layoutregions::extract::table::{Cells, TableOut};

use super::*;
use crate::layout::{ColumnLine, Columns, Gaps, Slot, address_columns};
use crate::regions::{BASELINE_NEAR, Part, offset_baselines};

/// Why blocks side by side are read one block after the other (see [`side_blocks`]).
const BLOCK_BY_BLOCK: &str = "blocks of short lines side by side are read one block after the \
     other: their lines may be rows of a table or labels beside values";

/// Why lines side by side read row by row may be no rows (see [`side_blocks`]).
const MAY_BE_BLOCKS: &str = "a column of the lines holds an address: they may be blocks side \
     by side, whose lines do not belong together";

/// Whether the lines of a region (their `pieces`) are a paragraph set full (justified): every
/// line starts where the others do, and every line of two pieces or more ends where they end,
/// within half a size; every line of three pieces or
/// more has gaps that are even (the word spacing of a line set full is the same between all its
/// words), and two lines or more are such lines; and their words are prose: a piece opens with
/// a small letter, or the gaps differ from line to line by more than half a size (each line set
/// full is spaced to fill the measure on its own). The rows of a table have gaps of the widths
/// their cells leave, which are not even on a row, and a grid of cells of one width whose gaps
/// are even (numbers flush right) has the same gaps on every row and no word in small letters.
/// A line of two pieces shows nothing: one gap is always even, and a gap at the edge of a cell
/// gap may join two words of a line set full into one piece.
fn set_full(flow: &Flow, pieces: &[Vec<Piece>], lines: &[FlowLine]) -> bool {
    let tol = 0.5
        * lines
            .iter()
            .map(|l| l.size)
            .fold(f32::MAX, f32::min)
            .max(1.0);
    let used: Vec<&Vec<Piece>> = pieces.iter().filter(|l| !l.is_empty()).collect();
    let left = used
        .iter()
        .flat_map(|l| l.first())
        .map(|p| p.x0)
        .fold(f32::MAX, f32::min);
    let right = used
        .iter()
        .flat_map(|l| l.last())
        .map(|p| p.x1)
        .fold(f32::MIN, f32::max);
    // A line of one piece (one word, which no spacing can fill out, or a line whose spaces are
    // too narrow to part it) starts at the left edge only.
    let edges = used.iter().all(|l| {
        l.first().is_some_and(|p| (p.x0 - left).abs() <= tol)
            && (l.len() < 2 || l.last().is_some_and(|p| (p.x1 - right).abs() <= tol))
    });
    if !edges {
        return false;
    }
    // The even gap of each line of three pieces or more.
    let mut spacing: Vec<f32> = Vec::new();
    for l in used.iter().filter(|l| l.len() >= 3) {
        let gaps: Vec<f32> = l.windows(2).map(|w| w[1].x0 - w[0].x1).collect();
        let (lo, hi) = gaps
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &g| (lo.min(g), hi.max(g)));
        if hi - lo > tol {
            return false;
        }
        spacing.push(0.5 * (lo + hi));
    }
    if spacing.len() < 2 {
        return false;
    }
    let (lo, hi) = spacing
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &g| (lo.min(g), hi.max(g)));
    let small = used.iter().flat_map(|l| l.iter()).any(|p| {
        text_of(flow, p.start, p.end)
            .trim_start()
            .chars()
            .next()
            .is_some_and(char::is_lowercase)
    });
    small || hi - lo > tol
}

/// How many pieces `pieces_of` finds in a line, without building them.
fn piece_count(flow: &Flow, l: &FlowLine) -> usize {
    grid::piece_count(&flow.runs, l.start, l.end)
}

/// The pieces of a line: runs closer than [`CELL_GAP`] font sizes are one (see
/// `grid::pieces_of`).
fn pieces_of(flow: &Flow, l: &FlowLine) -> Vec<Piece> {
    grid::pieces_of(&flow.runs, l.start, l.end)
}

/// The text of runs `start..end` of the flow, white space collapsed.
fn text_of(flow: &Flow, start: usize, end: usize) -> String {
    grid::text_of(&flow.runs, start, end)
}

/// The area of `areas` each of `lines` is in: the first area that holds the ids of all its
/// runs (`Flow::ids`, the page's text primitives), or `None`.
fn line_areas(flow: &Flow, lines: &[FlowLine], areas: &[&Segment]) -> Vec<Option<usize>> {
    let n = flow.ids.iter().map(|&i| i as usize + 1).max().unwrap_or(0);
    let mut of: Vec<Option<usize>> = vec![None; n];
    for (k, a) in areas.iter().enumerate() {
        for p in &a.prims {
            if let PrimId::Text(t) = *p
                && let Some(slot) = of.get_mut(t as usize)
            {
                slot.get_or_insert(k);
            }
        }
    }
    lines
        .iter()
        .map(|l| {
            let mut ids = flow.ids[l.start..l.end].iter();
            let first = of.get(*ids.next()? as usize).copied().flatten();
            ids.all(|&i| of.get(i as usize).copied().flatten() == first)
                .then_some(first)
                .flatten()
        })
        .collect()
}

/// Tables without rules: regions of lines whose pieces stand in columns with white space
/// between them, accepted only on strong evidence. `areas` are the segments of the page it
/// reads (see `tables::table_areas`), each as the union of its runs: a region holds the
/// lines of one area, and a line with runs of none or of two areas is in no region.
pub(super) fn borderless(
    page: u32,
    flow: &mut Flow,
    areas: &[&Segment],
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
    let in_area = line_areas(flow, &lines, areas);
    let off: Vec<bool> = lines
        .iter()
        .zip(&in_area)
        .map(|(l, a)| {
            a.is_none()
                || flow.runs[l.start..l.end].iter().any(|r| r.off_axis)
                || (l.start..l.end).any(held)
        })
        .collect();
    // Whether line `j` is in the area of line `i`.
    let same_area = |i: usize, j: usize| in_area[i] == in_area[j];
    // Regions: lines that follow each other down the page, most of them in two pieces or
    // more; at most two lines of one piece in a row (cell text that wraps). Each region also
    // has the lines right under it that may go on in its last row (see [`Tail`]).
    let mut regions: Vec<(usize, usize, Tail)> = Vec::new();
    // The lines where a region was parted from the one over it: by a blank row before a header
    // of its own or other columns, or by a rule that ends a table.
    let mut parted: HashSet<usize> = HashSet::new();
    // Of them, the lines where a rule that ends a table parted it: totals under the table.
    let mut ruled_off: HashSet<usize> = HashSet::new();
    // The first runs of lines of text after a blank row over rows of their region's shape.
    let mut maybe_heads: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if off[i] || counts[i] < 2 {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        let mut last_multi = i;
        // The steps between two lines of the region so far, blank rows aside, sorted; the
        // pitch of its rows is their common step, the middle one (the lower of two): a wider
        // step inside a table (a row set a little apart) does not raise it, so a blank row
        // after it still parts a table under it.
        let mut steps: Vec<f32> = Vec::new();
        let mut pitch = 0.0f32;
        while end < lines.len() && !off[end] && same_area(i, end) {
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
                .filter(|_| !off[end + 1] && same_area(i, end + 1))
                .map(|n| cur.y - n.y)
                .filter(|&g| g > 0.0 && g <= 3.0 * cur.size.max(lines[end + 1].size));
            let blank =
                next.is_some_and(|next| pitch > 0.0 && gap > REGION_STEPS * pitch.max(next));
            if blank {
                match header_line(flow, &lines[i..end], cur, &lines[end + 1]) {
                    Head::Header => {
                        parted.insert(end);
                        break;
                    }
                    _ if !in_columns(flow, &lines[i..end], cur) => {
                        parted.insert(end);
                        break;
                    }
                    // A line of text over rows of the region's own shape goes on as a row of
                    // its table, but it may head a table of its own: that is said.
                    Head::Doubt => maybe_heads.push(cur.start),
                    Head::No => {}
                }
            }
            // A blank row the region goes on over is no step of its rows: a gap after it is
            // judged by the rows' own steps.
            if !blank {
                steps.insert(steps.partition_point(|s| *s < gap), gap);
                pitch = steps[(steps.len() - 1) / 2];
            }
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
        let rule_end = ended(flow, &lines, (i, last_multi + 1), &found.ends);
        parted.extend(rule_end);
        ruled_off.extend(rule_end);
        let z = rule_end.unwrap_or(last_multi + 1);
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
            while z < lines.len() && !off[z] && same_area(i, z) {
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
    let mut title_cuts: Vec<(usize, Vec<table::TitleCut>)> = Vec::new();
    // Blocks side by side read one after the other: where their runs start in the flow, and
    // the runs in the order to read them.
    let mut apart: Vec<(usize, Vec<usize>)> = Vec::new();
    // Whether the region before was taken as a table.
    let mut table_over = false;
    for (g, (a, mut z, tail)) in regions.into_iter().enumerate() {
        // A region parted from a table right over it (a second table after a blank row, the
        // titles of totals over their amounts under a table's end rule) is a table of a header
        // and one row too, or its refusal is said.
        let after_table = std::mem::take(&mut table_over) && parted.contains(&a);
        // Totals under a table: the lines the rule that ends it parts from it.
        let totals_under = after_table && ruled_off.contains(&a);
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
        // A paragraph set full (justified) in a narrow column, whose word gaps are wide enough
        // to part its words into pieces, is prose: no columns.
        if set_full(flow, &pieces, &lines[a..z]) {
            continue;
        }
        // Two lines alone whose pieces stand in the same three columns or more, piece for piece,
        // the first of titles (one word each, no digit) over a row that holds a digit (`Pos
        // Datum Betrag` over `1 15.01.2026 9,99`), are a table of a header and one row, or its
        // refusal is said. Blocks side by side that open with a name of two words or more (a
        // firm over its street beside a bank over its account) are no titles over values.
        let digit = |p: &Piece| {
            flow.runs[p.start..p.end]
                .iter()
                .any(|r| r.text.chars().any(|c| c.is_ascii_digit()))
        };
        let title =
            |p: &Piece| !digit(p) && text_of(flow, p.start, p.end).split_whitespace().count() == 1;
        // Each title stands in one column with the value under it: their starts or their ends
        // meet (within half a size), as columns set flush left or flush right do. Words of two
        // lines set full (justified) share the lines' outer edges, and a middle word stands about
        // in the middle, but no word between them starts or ends where the one under it does:
        // middles that meet are no evidence.
        let tol = 0.5
            * lines[a..z]
                .iter()
                .map(|l| l.size)
                .fold(f32::MAX, f32::min)
                .max(1.0);
        let meet = |p: &Piece, q: &Piece| (p.x0 - q.x0).abs() <= tol || (p.x1 - q.x1).abs() <= tol;
        // Two lines whose words stand equal gaps apart and that share both outer edges are set
        // full (justified): prose, whatever words of theirs meet by chance.
        let even = |line: &[Piece]| {
            let gaps: Vec<f32> = line.windows(2).map(|w| w[1].x0 - w[0].x1).collect();
            let (lo, hi) = gaps
                .iter()
                .fold((f32::MAX, f32::MIN), |(l, h), &g| (l.min(g), h.max(g)));
            hi - lo <= tol
        };
        // Two lines that share both outer edges, as lines set to one measure do.
        let outer = |a: &[Piece], b: &[Piece]| {
            a.first()
                .zip(b.first())
                .is_some_and(|(p, q)| (p.x0 - q.x0).abs() <= tol)
                && a.last()
                    .zip(b.last())
                    .is_some_and(|(p, q)| (p.x1 - q.x1).abs() <= tol)
        };
        let aligned = z - a == 2
            && pieces[0].len() >= 3
            && pieces[0].len() == pieces[1].len()
            && pieces[0]
                .iter()
                .zip(&pieces[1])
                .all(|(p, q)| p.x0 < q.x1 && q.x0 < p.x1 && meet(p, q))
            && !(even(&pieces[0]) && even(&pieces[1]) && outer(&pieces[0], &pieces[1]))
            && pieces[0].iter().all(title)
            && pieces[1].iter().any(digit)
            && !prose_row(
                &text_of(flow, lines[a + 1].start, lines[a + 1].end),
                even(&pieces[1]) && outer(&pieces[0], &pieces[1]),
            );
        // Blocks of an address, a bank or a register side by side are no table: their runs are
        // read block by block. Rows were possible too, so that is said. Under a header row of
        // their headings, they are read as the table they make, with the doubt said.
        let mut headed = false;
        if let Some((s, order, verdict)) =
            side_blocks(flow, &lines, (a, z, tail.end.max(z)), &pieces, &body_of)
        {
            if verdict == Columns::Headed {
                headed = true;
            } else if verdict == Columns::Doubt {
                found.doubts.push((
                    flow.ids[lines[a].start..lines[z - 1].end].to_vec(),
                    MAY_BE_BLOCKS,
                ));
                found.uncertain.push(format!(
                    "lines side by side on page {page} are read row by row, though a column of \
                     them holds an address: they may be blocks side by side, whose lines do not \
                     belong together"
                ));
            } else {
                found
                    .doubts
                    .push((order.iter().map(|&i| flow.ids[i]).collect(), BLOCK_BY_BLOCK));
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
                    kind: layoutregions::Kind::Table,
                    ids: Vec::new(),
                    by: if totals_under {
                        Detector::TotalsUnder
                    } else {
                        Detector::KeyValues
                    },
                    drawn: None,
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
            let rows_needed = if table_caption || after_table || aligned {
                2
            } else {
                3
            };
            let cells = match grid_cells(&flow.runs, pieces, rows_needed, table_caption) {
                Cells::Table(grid) => Ok(grid),
                Cells::Refused(why) => Err(why),
                // Lines without rules neither count squares nor look at the clock.
                Cells::TooLarge { .. } | Cells::Stopped => {
                    unreachable!("grid_cells builds or refuses")
                }
            };
            let verdict = cells.and_then(|grid| {
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
                        // Filled in below, once the runs are taken out.
                        rows: planned_rows(&grid),
                        header_rows: 0,
                        trs: Vec::new(),
                        additions: Vec::new(),
                        tagged: None,
                        continues: false,
                        caption: Vec::new(),
                        found_by: None,
                        kind: layoutregions::Kind::Table,
                        ids: Vec::new(),
                        // The class of the region, in this order where several hold.
                        by: if totals_under {
                            Detector::TotalsUnder
                        } else if headed {
                            Detector::SideBlocks
                        } else if aligned {
                            Detector::LabelsOverValues
                        } else {
                            Detector::Grid
                        },
                        drawn: None,
                    };
                    let t = u32::try_from(claim.len()).unwrap_or(u32::MAX);
                    for (r, cells) in grid.rows.iter().enumerate() {
                        for (c, g) in cells.iter().enumerate() {
                            for &i in &g.runs {
                                target[i] = Some((t, r as u32, c as u32));
                            }
                        }
                    }
                    // A line the grid leaves out is said, unless the loss of its frame says it.
                    let unframed = grid.unplaced.iter().any(|&(i, _)| body_of(i).is_none());
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
                        title_cuts.push((claim.len(), grid.cuts.clone()));
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
            let looks = if after_table {
                z - a >= 2 && columned >= 2
            } else {
                (z - a >= 3 && columned >= 3) || aligned
            };
            if looks && !said && !quietly {
                found.quiet.push((lo, why, Refused::Region));
                found
                    .doubts
                    .push((flow.ids[lines[a].start..lines[z - 1].end].to_vec(), why));
            }
        }
        if let (false, Some((why, top, line))) = (taken, refused) {
            // A caption gates one note: a region it gated before said so.
            if !said && !noted.contains(&line) {
                noted.push(line);
                found
                    .doubts
                    .push((flow.ids[lines[a].start..lines[z - 1].end].to_vec(), why));
                found.unsure.push(Unsure {
                    at: flow.at[lines[a].start],
                    top,
                    reason: why.to_string(),
                    text: None,
                });
            }
        }
        table_over = taken;
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
    // A line of text read as a row of a table, though it may head a table of its own.
    let doubted = maybe_heads
        .iter()
        .filter(|&&r| target.get(r).is_some_and(Option::is_some))
        .count();
    if doubted > 0 {
        let (s, them) = if doubted == 1 {
            ("", "it")
        } else {
            ("s", "they")
        };
        found.uncertain.push(format!(
            "{doubted} line{s} of text after a gap in a table without rules on page {page} {} \
             read as a row of the table, as the rows under {them} have the table's shape: {them} \
             may start a table of {} own",
            if doubted == 1 { "is" } else { "are" },
            if doubted == 1 { "its" } else { "their" },
        ));
    }
    // The runs of blocks side by side go in the flow block by block; their places stay.
    for (start, order) in apart {
        let runs: Vec<Run> = order.iter().map(|&i| flow.runs[i].clone()).collect();
        let slots: Vec<Slot> = order.iter().map(|&i| flow.slots[i]).collect();
        let ids: Vec<u32> = order.iter().map(|&i| flow.ids[i]).collect();
        for (k, ((r, sl), id)) in runs.into_iter().zip(slots).zip(ids).enumerate() {
            flow.runs[start + k] = r;
            flow.slots[start + k] = sl;
            flow.ids[start + k] = id;
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
/// close titles, they stay one merged cell (see [`grid_cells`]).
fn frame_titles(
    flow: &Flow,
    grid: &TableOut,
    body_of: &dyn Fn(usize) -> Option<u32>,
    labels: &[Vec<String>],
) -> bool {
    let Some(first) = grid.rows.first() else {
        return false;
    };
    let mut runs: Vec<usize> = first.iter().flat_map(|c| c.runs.iter().copied()).collect();
    runs.sort_unstable();
    let Some(&s) = runs.first() else {
        return false;
    };
    let Some(frame) = body_of(s) else {
        return false;
    };
    if !runs.iter().all(|&i| body_of(i) == Some(frame)) {
        return false;
    }
    let titles = labels.get(frame as usize).map(|l| l.join(" "));
    titles.is_some_and(|t| {
        t.split_whitespace().eq(runs
            .iter()
            .flat_map(|&i| flow.runs[i].text.split_whitespace()))
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

/// Two lines of a region without rules stand at most this many times the common step of the
/// region so far, or the step of the next two lines, apart: a wider gap parts two blocks,
/// unless the line after it stands in the region's columns (see [`in_columns`]).
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
/// total row among them does not count) or by its shape (no digit, over a line with a digit
/// whose cells have another shape than every row of the region, see [`Shape`], under rows that
/// all hold one after the region's first line). Over a line of a row's shape it may be either
/// ([`Head::Doubt`]).
fn header_line(flow: &Flow, region: &[FlowLine], line: &FlowLine, next: &FlowLine) -> Head {
    if region.is_empty() {
        return Head::No;
    }
    let own = pieces_of(flow, line);
    let texts: Vec<String> = own.iter().map(|p| text_of(flow, p.start, p.end)).collect();
    let year = |t: &str| t.len() <= HEADER_DIGITS && t.chars().all(|c| c.is_ascii_digit());
    let cells = own.len() >= 2
        && texts.iter().any(|t| t.chars().any(char::is_alphabetic))
        && texts.iter().all(|t| year(t) || !numeric(t));
    if !cells {
        return Head::No;
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
    // The shapes of a line's cells, left to right: a row under the line that has the shape of
    // the region's own rows goes on the region's table, and the line over it is one of its
    // rows too (a note row between groups of rows), not the header of another table.
    let shapes = |l: &FlowLine| -> Vec<Option<Shape>> {
        pieces_of(flow, l)
            .iter()
            .map(|p| Shape::of(&text_of(flow, p.start, p.end)))
            .collect()
    };
    let own_rows = region[1..].iter().any(|l| shapes(l) == shapes(next));
    let shaped = region.len() > 1 && !digit(line) && digit(next) && region[1..].iter().all(digit);
    if styled || (shaped && !own_rows) {
        Head::Header
    } else if shaped {
        Head::Doubt
    } else {
        Head::No
    }
}

/// What [`header_line`] finds a line after a blank row to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Head {
    /// The header of a table of its own.
    Header,
    /// A line of text over a row of the region's own shape: a row of the region's table (a
    /// note row), or the header of a table of its own.
    Doubt,
    /// No header.
    No,
}

/// A header cell may be a whole number of at most this many digits (a year).
const HEADER_DIGITS: usize = 4;

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
        // as a line of a cell between rows must (see [`grid_cells`]).
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
        // The cell of the column on the last line is open: a date, then white space and a dash
        // (`01.12.2026 -`), over a date set as its wrap (`31.12.2026`): its range goes on. A
        // trailing minus of an amount (`10,00-`) or a price (`50,-`) opens nothing, and an
        // amount under the table is a total, never the end of a range.
        let cut = pieces[core - 1].iter().filter(|q| over(q) == [c]).any(|q| {
            let cell = text_of(flow, q.start, q.end);
            cell.trim_end()
                .strip_suffix('-')
                .filter(|rest| rest.ends_with(char::is_whitespace))
                .and_then(|rest| rest.split_whitespace().next_back())
                .is_some_and(|from| Shape::of(from) == Some(Shape::Date))
        }) && Shape::of(text.trim()) == Some(Shape::Date);
        if close && within && same_face && cut {
            return (1, false, false);
        }
        let wrap =
            close && within && same_face && !numbers && !numeric(&text) && !page_number(&text);
        // An amount alone at the pitch of the rows, in the column of numbers it stands in (flush
        // with its start or its end), under an amount of the last line, is one more row of its
        // own (see [`grid_cells`]) when the region has such rows already: amounts alone in the same
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

/// Why a region of two blocks side by side is no table.
const SIDE_BY_SIDE: &str = "two blocks stand side by side";

/// A gutter this many font sizes wide or more, and this many times the median gutter, that
/// no line crosses may part two blocks side by side.
const BLOCK_GUTTER: f32 = 8.0;
const BLOCK_GUTTER_RATIO: f32 = 3.0;

/// Whether the second of two lines reads as the end of a sentence that goes on from the line
/// over it: it opens with a small letter, or it ends with a full stop and is `set_full` to the
/// measure of the line over it (its words even gaps apart, both its edges at that line's
/// edges), as a line of prose set full is (`Die 3 Hinweise.`). A row of values does not open
/// with a small letter, and its cells stand where their columns start or end: its last cell
/// may end with the stop of an abbreviation (`12 Stk.`, `Muster Co. KG.`).
fn prose_row(text: &str, set_full: bool) -> bool {
    let t = text.trim();
    t.starts_with(char::is_lowercase) || (t.ends_with('.') && set_full)
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
    let mut doubt: Option<(usize, Vec<usize>)> = None;
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
            Columns::Doubt => doubt = doubt.or(Some((s, order))),
            verdict => return Some((s, order, verdict)),
        }
    }
    doubt.map(|(s, order)| (s, order, Columns::Doubt))
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

#[cfg(test)]
mod tests {
    use layoutregions::PrimId;
    use layoutregions::segment::{SegId, Source};

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
        Flow::new(runs, vec![Slot::Free; n])
    }

    /// A title cut inside a run (see `grid::title_cut`): the text after the cut goes first
    /// into the next cell, as a run of its own where the run's width shared out by its
    /// characters puts it.
    #[test]
    fn a_cut_title_goes_to_the_next_cell() {
        let one = flow_of(vec![run("Menge Einheit", 300.0, 600.0, 0)]);
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

    /// A segment of the page that holds the runs `ids`.
    fn area_of(id: u32, ids: impl Iterator<Item = u32>) -> Segment {
        Segment {
            id: SegId(id),
            bounds: layoutregions::Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: 595.0,
                y1: 842.0,
            },
            prims: ids.map(PrimId::Text).collect(),
            source: Source::Rest,
            parent: None,
            children: Vec::new(),
            alternatives: Vec::new(),
        }
    }

    /// The words of each table found, in order.
    fn table_words(found: &Found) -> Vec<Vec<String>> {
        found
            .tables
            .iter()
            .map(|t| {
                t.rows
                    .iter()
                    .flatten()
                    .flat_map(|c| c.runs.iter().map(|r| r.text.clone()))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn borderless_uses_the_page_segments() {
        // Eight rows of one table of items: name, quantity and amount.
        let mut runs = Vec::new();
        for k in 0..8u32 {
            let y = 700.0 - 14.0 * k as f32;
            runs.push(run(&format!("Muster {k}"), 40.0, y, k));
            runs.push(run(&format!("{}", k + 2), 200.0, y, k));
            runs.push(run(&format!("1.234,{k}6"), 300.0, y, k));
        }
        let area = Area {
            width: 595.0,
            height: 842.0,
            images: &[],
            body_size: 8.0,
        };
        let read = |areas: &[&Segment]| {
            let mut flow = flow_of(runs.clone());
            let mut found = Found::default();
            borderless(1, &mut flow, areas, &area, &|| false, &mut found);
            (table_words(&found), flow.runs.len())
        };
        // One area that holds every run: one table of all rows, as the page's union reads.
        let all = area_of(0, 0..24);
        let (tables, left) = read(&[&all]);
        assert_eq!(tables.len(), 1, "{tables:?}");
        assert_eq!(tables[0].len(), 24);
        assert_eq!(left, 0);
        // Two areas: no table reaches across them.
        let (top, bottom) = (area_of(1, 0..12), area_of(2, 12..24));
        let (tables, _) = read(&[&top, &bottom]);
        assert_eq!(tables.len(), 2, "{tables:?}");
        let names = |t: &[String]| -> Vec<String> {
            t.iter()
                .filter(|w| w.starts_with("Muster"))
                .cloned()
                .collect()
        };
        assert_eq!(
            names(&tables[0]),
            ["Muster 0", "Muster 1", "Muster 2", "Muster 3"]
        );
        assert_eq!(
            names(&tables[1]),
            ["Muster 4", "Muster 5", "Muster 6", "Muster 7"]
        );
        // An area of the top half only: the bottom rows are no table's and stay in the flow.
        let (tables, left) = read(&[&top]);
        assert_eq!(tables.len(), 1, "{tables:?}");
        assert_eq!(tables[0].len(), 12);
        assert_eq!(left, 12);
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
        let mut flow = Flow::new(runs, vec![Slot::Free; n]);
        let area = Area {
            width: 595.0,
            height: 842.0,
            images: &[],
            body_size: 8.0,
        };
        let mut found = Found::default();
        let every = area_of(0, 0..n as u32);
        borderless(1, &mut flow, &[&every], &area, &|| false, &mut found);
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
