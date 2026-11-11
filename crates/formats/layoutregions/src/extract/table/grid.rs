// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The cells of lines in columns without rules: pieces of lines apart by white space, columns
//! from the lines with the most common number of pieces, rows from the lines that fill half of
//! them, and the tests a grid of white space gutters must pass.

use std::collections::HashMap;

use super::banded::long_or_prose;
use super::{
    CAPTIONED_CELL, CELL_GAP, CONTENTS, CellOut, ROWS_ALIKE, RUNS_OVER_COLUMNS, RunRange,
    SHORT_CELL, TableOut, TitleCut,
};
use crate::extract::pairs::words;
use crate::extract::text::signed_amount;
use crate::file::CURRENCIES;
use crate::line::LineRun;

/// Share of the rows of a table without rules whose pieces stay inside the columns.
const CLEAN_ROWS: f32 = 0.8;

/// Each part of a cut title stands within this many sizes of its column's start or of its
/// column's end (a title is set flush left or flush right with its values).
const TITLE_ALIGN: f32 = 0.25;

/// A part of a cut title that covers its column ends at most this many sizes after the
/// column's end.
const COVER_END: f32 = 0.5;

/// A run of text of one line, apart from its neighbours by a gap: one cell's text.
#[derive(Debug, Clone)]
pub struct Piece {
    pub x0: f32,
    pub x1: f32,
    /// Range of the runs.
    pub start: usize,
    pub end: usize,
}

/// A line holds at least this many word gaps besides a wide one for its gaps to say whether
/// it is justified prose.
const PROSE_GAPS: usize = 4;

/// A gap after a sentence end in a justified line is at most this many times the line's median
/// word gap (a stretched line widens the space after a full stop more than its word spaces).
const SENTENCE_GAP: f32 = 2.5;

/// The median of the word gaps of the line of runs `start..end`: those inside its runs and those
/// between them. None for a line with fewer than [`PROSE_GAPS`] + 1 gaps. Its work is counted.
fn median_word_gap<R: LineRun>(runs: &[R], start: usize, end: usize) -> Option<f32> {
    let mut gaps: Vec<f32> = Vec::new();
    let mut last: Option<f32> = None;
    for r in &runs[start..end] {
        if let Some(e) = last {
            let g = r.x() - e;
            if g > 0.0 {
                gaps.push(g);
            }
        }
        gaps.extend(r.gaps().map(|g| g.to - g.from).filter(|g| *g > 0.0));
        last = Some(r.x() + r.w());
    }
    crate::work::add(gaps.len() as u64 + (end - start) as u64);
    if gaps.len() <= PROSE_GAPS {
        return None;
    }
    gaps.sort_by(f32::total_cmp);
    Some(gaps[gaps.len() / 2])
}

/// Whether the wide gap between `before` and `after` is the space after a sentence end in a
/// justified line, not a gap between cells: `before` ends a sentence (a word of two letters or
/// more and a full stop, question or exclamation mark), `after` starts with a capital letter,
/// and the gap is at most [`SENTENCE_GAP`] times the line's median word gap (`median`, worked
/// out once when first needed). Positive evidence only: without it the gap parts the line.
fn sentence_gap<R: LineRun>(
    before: &R,
    after: &R,
    gap: f32,
    median: &mut dyn FnMut() -> Option<f32>,
) -> bool {
    let b = before.text().trim_end();
    let Some(word) = b
        .strip_suffix(['.', '?', '!'])
        .and_then(|t| t.split_whitespace().last())
    else {
        return false;
    };
    if word.chars().count() < 2 || !word.chars().all(char::is_alphabetic) {
        return false;
    }
    if !after
        .text()
        .trim_start()
        .chars()
        .next()
        .is_some_and(char::is_uppercase)
    {
        return false;
    }
    median().is_some_and(|m| gap <= SENTENCE_GAP * m)
}

/// The median word gap of the line of runs `start..end`, worked out on the first call.
fn lazy_median<R: LineRun>(
    runs: &[R],
    start: usize,
    end: usize,
) -> impl FnMut() -> Option<f32> + '_ {
    let mut cache: Option<Option<f32>> = None;
    move || *cache.get_or_insert_with(|| median_word_gap(runs, start, end))
}

/// How many pieces [`pieces_of`] finds in the line of runs `start..end`, without building
/// them.
pub fn piece_count<R: LineRun>(runs: &[R], start: usize, end: usize) -> usize {
    let mut n = 0;
    let mut last = f32::MIN;
    let mut median = lazy_median(runs, start, end);
    for (i, r) in runs.iter().enumerate().take(end).skip(start) {
        let gap = r.x() - last;
        if n == 0
            || (gap > CELL_GAP * r.size().max(1.0)
                && !sentence_gap(&runs[i - 1], r, gap, &mut median))
        {
            n += 1;
            last = r.x() + r.w();
        } else {
            last = last.max(r.x() + r.w());
        }
    }
    n
}

/// The pieces of the line of runs `start..end`: runs closer than [`CELL_GAP`] font sizes
/// are one piece.
pub fn pieces_of<R: LineRun>(runs: &[R], start: usize, end: usize) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut median = lazy_median(runs, start, end);
    for (i, r) in runs.iter().enumerate().take(end).skip(start) {
        let x0 = r.x();
        match out.last_mut() {
            Some(p)
                if x0 - p.x1 <= CELL_GAP * r.size().max(1.0)
                    || sentence_gap(&runs[i - 1], r, x0 - p.x1, &mut median) =>
            {
                p.x1 = p.x1.max(r.x() + r.w());
                p.end = i + 1;
            }
            _ => out.push(Piece {
                x0,
                x1: r.x() + r.w(),
                start: i,
                end: i + 1,
            }),
        }
    }
    out
}

/// The text of runs `start..end`, white space collapsed.
pub fn text_of<R: LineRun>(runs: &[R], start: usize, end: usize) -> String {
    let ids: Vec<usize> = (start..end).collect();
    words(runs, &ids)
}

/// Columns: the x ranges the pieces of the lines with the most common number of pieces fill;
/// the first line may be a header that spans columns.
pub fn columns_of(pieces: &[Vec<Piece>]) -> Vec<(f32, f32)> {
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

/// `columns` (see [`columns_of`]) with a column of their own for the pieces that stand in a
/// gutter between two of them, in no column, on lines under the first with more pieces than
/// the columns: a column with text in a few rows only (a rate beside one of the totals). A
/// line of no more pieces than the columns with text in a gutter (a note between rows, a cell
/// over two columns) adds none. Such pieces that overlap are one column.
fn with_sparse(columns: Vec<(f32, f32)>, pieces: &[Vec<Piece>]) -> Vec<(f32, f32)> {
    let (Some(first), Some(last)) = (columns.first(), columns.last()) else {
        return columns;
    };
    let (left, right) = (first.1, last.0);
    let mut spans: Vec<(f32, f32)> = pieces
        .iter()
        .skip(1)
        .filter(|l| l.len() > columns.len())
        .flatten()
        .filter(|p| p.x0 > left && p.x1 < right)
        .filter(|p| !columns.iter().any(|c| p.x1 > c.0 && p.x0 < c.1))
        .map(|p| (p.x0, p.x1))
        .collect();
    if spans.is_empty() {
        return columns;
    }
    spans.extend(columns);
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f32, f32)> = Vec::new();
    for (s, e) in spans {
        match out.last_mut() {
            Some(c) if s <= c.1 => c.1 = c.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// Where a piece of a line goes in a grid: its column, its column span and its range of runs.
type Placement = (usize, u32, (usize, usize));

struct GridCell {
    ranges: Vec<(usize, usize)>,
    span: u32,
    pictures: Vec<u32>,
}

/// The cells of a region of lines without rules, each line its `pieces` of `runs`, or why it
/// is no table (`Err`). `rows_needed` is the fewest rows a table there must have; a
/// `table_caption` next to it lets its cells be longer. A line between two rows that runs over
/// the columns (a note) is [`RUNS_OVER_COLUMNS`] and in no cell; the reader keeps its text as
/// it stands.
pub fn grid_cells<R: LineRun>(
    runs: &[R],
    pieces: &[Vec<Piece>],
    rows_needed: usize,
    table_caption: bool,
) -> Result<TableOut, &'static str> {
    grid_cells_with(runs, pieces, Around::default(), rows_needed, table_caption)
}

/// What stands around the lines of a region besides their text (Task 29): the pictures of the
/// page near them, each with the caller's index for it and its box (x0, y0, x1, y1), and the
/// places where a rule across them breaks (column bounds the rule draws, see
/// `ruled::rule_breaks`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Around<'a> {
    pub pictures: &'a [(u32, [f32; 4])],
    pub breaks: &'a [f32],
}

/// The column of the pictures that a piece of the header line over no text column names: two
/// pictures or more under the piece (overlapping it in width, their tops under the header's
/// baseline), in no text column. A picture
/// column named by its header (`Bild` over a picture in each row).
fn picture_column(
    (first, baseline): (&[Piece], f32),
    columns: &[(f32, f32)],
    pictures: &[(u32, [f32; 4])],
) -> Option<(f32, f32)> {
    let free = |a: f32, b: f32| !columns.iter().any(|c| b > c.0 && a < c.1);
    first.iter().find_map(|q| {
        if !free(q.x0, q.x1) {
            return None;
        }
        let under: Vec<&[f32; 4]> = pictures
            .iter()
            .map(|(_, p)| p)
            .filter(|p| p[3] < baseline && p[2] > q.x0 && p[0] < q.x1 && free(p[0], p[2]))
            .collect();
        (under.len() >= 2).then(|| {
            under
                .iter()
                .fold((q.x0, q.x1), |(a, b), p| (a.min(p[0]), b.max(p[2])))
        })
    })
}

/// [`grid_cells`] with what stands `around` the lines. A picture stands in a cell (Task 29,
/// B1 and vision class C: a picture in each row of an item table) only when it lies inside one
/// column's bounds (the middle of the gutters on both sides, or a break of a rule across the
/// table in the gutter) and the baselines it holds are all lines of one row: a stamp or a logo
/// over several rows, or a picture across a column bound, is no cell's. Pictures never make
/// a column of text; a column of pictures is one only where a title of the header line names
/// it (see [`picture_column`]).
pub fn grid_cells_with<R: LineRun>(
    runs: &[R],
    pieces: &[Vec<Piece>],
    around: Around<'_>,
    rows_needed: usize,
    table_caption: bool,
) -> Result<TableOut, &'static str> {
    let lines = pieces.len();
    let mut columns = with_sparse(columns_of(pieces), pieces);
    let header = pieces.first().and_then(|first| {
        let p = first.iter().find(|p| p.end > p.start)?;
        Some((first.as_slice(), runs[p.start].y()))
    });
    if let Some(c) = header.and_then(|h| picture_column(h, &columns, around.pictures)) {
        let at = columns.partition_point(|x| x.0 < c.0);
        columns.insert(at, c);
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
    // The column of each piece, and its range of runs; a line whose piece crosses a gutter is
    // not clean.
    let mut clean = 0usize;
    let mut placed: Vec<Vec<Placement>> = Vec::with_capacity(lines);
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
    if (clean as f32) < CLEAN_ROWS * lines as f32 {
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
        let number = |&(_, span, (s, e)): &Placement| span == 1 && amount(&text_of(runs, s, e));
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
    let row_at = rows_of_lines(runs, pieces, &anchors);
    let row_of = |k: usize| row_at[k];
    // Close titles of the first line over two columns are cut at the gutter between them.
    // The cut that falls inside a run is made once the runs are in their cells.
    let mut inside: Vec<(usize, usize)> = Vec::new();
    let first = std::mem::take(&mut placed[0]);
    for (k, &(c, span, (start, end))) in first.iter().enumerate() {
        // Both columns hold text of their own in two lines below or more, and no other line
        // of the first row has text in them.
        let alone = |col: usize| {
            (1..lines)
                .filter(|&j| placed[j].iter().any(|x| x.0 == col && x.1 == 1))
                .count()
                >= 2
        };
        let shared = (1..lines).any(|j| {
            row_of(j) == row_of(0)
                && placed[j]
                    .iter()
                    .any(|x| x.0 <= c + 1 && x.0 + x.1 as usize > c)
        });
        let cut = if span == 2 && !shared && alone(c) && alone(c + 1) {
            // The shape of most values of a column, from its own text in the lines below.
            let shape_of = |col: usize| {
                let shapes: Vec<Option<Shape>> = (1..lines)
                    .flat_map(|j| placed[j].iter().filter(move |x| x.0 == col && x.1 == 1))
                    .map(|x| Shape::of(&text_of(runs, x.2.0, x.2.1)))
                    .collect();
                Shape::most(&shapes)
            };
            // A column with no shape allows no cut.
            let differ = matches!((shape_of(c), shape_of(c + 1)), (Some(a), Some(b)) if a != b);
            title_cut(runs, &pieces[0][k], (columns[c], columns[c + 1]), differ)
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
        let rs = &runs[s..e];
        let x0 = rs.iter().map(|r| r.x()).fold(f32::MAX, f32::min);
        let x1 = rs.iter().map(|r| r.x() + r.w()).fold(f32::MIN, f32::max);
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
                    pictures: Vec::new(),
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
    for (pic, col, row) in cell_pictures(runs, pieces, &columns, around, &row_of) {
        rows[row][col].pictures.push(pic);
    }
    // A cell that spans hides the cells it covers.
    for row in &mut rows {
        let mut c = 0;
        while c < row.len() {
            let span = row[c].span as usize;
            for k in c + 1..(c + span).min(row.len()) {
                let moved = std::mem::take(&mut row[k].ranges);
                row[c].ranges.extend(moved);
                let moved = std::mem::take(&mut row[k].pictures);
                row[c].pictures.extend(moved);
                row[k].span = 0;
            }
            c += span.max(1);
        }
        row.retain(|c| c.span > 0);
    }
    // Each cut inside a run: the cell that holds the run, and the run's place among the runs
    // of the cell, which go to it in their order.
    let r0 = row_of(0);
    let cuts: Vec<TitleCut> = inside
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
    let texts: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| {
                    let mut ranges = c.ranges.clone();
                    ranges.sort_unstable();
                    ranges
                        .iter()
                        .map(|&(s, e)| text_of(runs, s, e))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()
        })
        .collect();
    if texts.len() > 1 && texts.iter().all(|r| *r == texts[0]) {
        return Err(ROWS_ALIKE);
    }
    // Short cells, or mostly numbers; longer next to a table caption.
    let short = if table_caption {
        CAPTIONED_CELL
    } else {
        SHORT_CELL
    };
    if let Some(why) = long_or_prose(&texts, short) {
        return Err(why);
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
        return Err(CONTENTS);
    }
    let expand = |ranges: &[(usize, usize)]| -> Vec<usize> {
        ranges.iter().flat_map(|&(s, e)| s..e).collect()
    };
    Ok(TableOut {
        rows: rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|c| CellOut {
                        runs: expand(&c.ranges),
                        row_span: 1,
                        col_span: c.span,
                        status: None,
                    })
                    .collect()
            })
            .collect(),
        header_rows: 0,
        unplaced: expand(&left)
            .into_iter()
            .map(|i| (i, RUNS_OVER_COLUMNS))
            .collect(),
        icons: Vec::new(),
        cuts,
        pictures: rows
            .iter()
            .enumerate()
            .flat_map(|(r, cells)| {
                cells
                    .iter()
                    .enumerate()
                    .flat_map(move |(c, cell)| cell.pictures.iter().map(move |&pic| (r, c, pic)))
            })
            .collect(),
    })
}

/// The pictures of `around` that stand in a cell of the grid of `columns` (see
/// [`grid_cells_with`]): each picture's place among them, its column and its row (`row_of`
/// gives the row of a line). A picture stands in a row when the baselines it holds are all
/// that row's and it lies inside the row's band (see `band` below): a picture that reaches
/// over a gap into another block is no row's.
fn cell_pictures<R: LineRun>(
    runs: &[R],
    pieces: &[Vec<Piece>],
    columns: &[(f32, f32)],
    around: Around<'_>,
    row_of: &dyn Fn(usize) -> usize,
) -> Vec<(u32, usize, usize)> {
    let cols = columns.len();
    // The baseline of each line.
    let base: Vec<Option<f32>> = pieces
        .iter()
        .map(|l| {
            l.iter()
                .find(|p| p.end > p.start)
                .map(|p| runs[p.start].y())
        })
        .collect();
    // The bound between columns `c` and `c + 1`: a break of a rule in their gutter, else its
    // middle.
    let bound = |c: usize| {
        let (a, b) = (columns[c].1, columns[c + 1].0);
        around
            .breaks
            .iter()
            .copied()
            .find(|x| *x >= a && *x <= b)
            .unwrap_or((a + b) / 2.0)
    };
    // Each row's band: from the last baseline of the row over it (over the first row, the
    // table's tallest row or a line and a half over its first baseline) down to the first baseline of the row under it (under
    // the last row, the table's tallest row under its first baseline, or three lines).
    let rows = (0..pieces.len()).map(row_of).max().map_or(0, |r| r + 1);
    let mut firsts: Vec<Option<(f32, f32)>> = vec![None; rows];
    let mut lasts: Vec<Option<f32>> = vec![None; rows];
    for (l, y) in base.iter().enumerate() {
        if let Some(y) = *y {
            let r = row_of(l);
            let size = pieces[l]
                .iter()
                .find(|p| p.end > p.start)
                .map_or(1.0, |p| runs[p.start].size());
            firsts[r].get_or_insert((y, size));
            lasts[r] = Some(y);
        }
    }
    let tallest = firsts
        .windows(2)
        .filter_map(|w| Some(w[0]?.0 - w[1]?.0))
        .fold(0.0f32, f32::max);
    let band = |r: usize| -> Option<(f32, f32)> {
        let (first, size) = firsts[r]?;
        let top = match r.checked_sub(1).and_then(|o| lasts[o]) {
            Some(y) => y,
            None => first + (1.5 * size).max(tallest),
        };
        let bottom = match firsts.get(r + 1).copied().flatten() {
            Some((y, _)) => y,
            None if tallest > 0.0 => first - tallest,
            None => first - 3.0 * size,
        };
        Some((bottom, top))
    };
    let mut out = Vec::new();
    for &(pic, ref p) in around.pictures {
        let held: Vec<usize> = (0..pieces.len())
            .filter(|&l| base[l].is_some_and(|y| y >= p[1] && y <= p[3]))
            .collect();
        let Some(&first) = held.first() else {
            continue;
        };
        if held.iter().any(|&l| row_of(l) != row_of(first)) {
            continue;
        }
        let inside = band(row_of(first)).is_some_and(|(bottom, top)| p[1] >= bottom && p[3] <= top);
        if !inside {
            continue;
        }
        let col = (0..cols).find(|&c| {
            let left = if c == 0 { f32::MIN } else { bound(c - 1) };
            let right = if c + 1 == cols { f32::MAX } else { bound(c) };
            p[0] >= left && p[2] <= right
        });
        if let Some(col) = col {
            out.push((pic, col, row_of(first)));
        }
    }
    out
}

/// Two sizes of text within this share of each other are one size.
const SAME_SIZE: f32 = 0.1;

/// A blank between two lines of a row is wider than the steps around it by at least this many
/// sizes of their text: the white space that parts two records.
const BLANK_STEP: f32 = 0.25;

/// The row of each line of a region (its `pieces` of `runs`), the rows starting at the lines
/// `anchors` (sorted): an anchor starts its own row, a line before the first anchor joins the
/// first row. The lines between two anchors go with the row over them, as the wrapped text of
/// its cells, unless the evidence of their place puts some with the row under them (Task 29,
/// B2: the other cells of a row stand on the second line of a description that wraps, so the
/// description's first line stands over the row's anchor):
/// - the widest step of baselines from the anchor over them, through them, to the anchor under
///   them is a blank (see [`BLANK_STEP`]): the lines under it go with the row under it, the
///   others with the row over it;
/// - with no blank, lines in the plain face under a first row all in bold (a header in its own
///   weight; an item name in bold beside plain values is none), where the first row is the
///   anchor over them, are no text of the first row: they go with the row under them;
/// - either way only when the lines that go down go on into the row under them: each stands in
///   a column of the anchor under it, in its size (a detail line set apart under its own row
///   stays with it).
fn rows_of_lines<R: LineRun>(runs: &[R], pieces: &[Vec<Piece>], anchors: &[usize]) -> Vec<usize> {
    let lines = pieces.len();
    // The baseline, the size and the weight of each line, from its first piece's first run.
    let of = |k: usize| {
        pieces[k].iter().find(|p| p.end > p.start).map(|p| {
            (
                runs[p.start].y(),
                runs[p.start].size(),
                runs[p.start].bold(),
            )
        })
    };
    // Whether every inked run of line `k` is bold: the line is a header in its own weight.
    let all_bold = |k: usize| {
        let mut inked = pieces[k]
            .iter()
            .flat_map(|p| &runs[p.start..p.end])
            .filter(|r| !r.text().trim().is_empty())
            .peekable();
        inked.peek().is_some() && inked.all(|r| r.bold())
    };
    let mut out: Vec<usize> = (0..lines)
        .map(|k| {
            let after = anchors.partition_point(|&a| a < k);
            if anchors.get(after) == Some(&k) {
                after
            } else {
                after.saturating_sub(1)
            }
        })
        .collect();
    for w in 0..anchors.len().saturating_sub(1) {
        let (a, b) = (anchors[w], anchors[w + 1]);
        if b <= a + 1 {
            continue;
        }
        // The steps from the anchor over the lines, between them, and to the anchor under them.
        let mut steps: Vec<(f32, f32)> = Vec::with_capacity(b - a);
        let mut whole = true;
        for k in a..b {
            match (of(k), of(k + 1)) {
                (Some((y0, s0, _)), Some((y1, s1, _))) => steps.push((y0 - y1, s0.max(s1))),
                _ => whole = false,
            }
        }
        if !whole {
            continue;
        }
        let widest = (0..steps.len())
            .max_by(|&i, &j| steps[i].0.total_cmp(&steps[j].0))
            .unwrap_or(0);
        let blank = (0..steps.len())
            .filter(|&i| i != widest)
            .all(|i| steps[widest].0 - steps[i].0 >= BLANK_STEP * steps[widest].1);
        let first_from = if blank {
            // Step `widest` lies between line `a + widest` and the line under it.
            Some(a + widest + 1)
        } else if w == 0 && all_bold(a) {
            (a + 1..b).find(|&k| of(k).is_some_and(|l| !l.2))
        } else {
            None
        };
        // The lines that go down go on into the row under them: each stands in a column of the
        // anchor under it (its text starts where a piece of that line starts), in its font, slant
        // and size (a detail line in a face of its own stays with its row).
        let goes_on = |from: usize| {
            let Some((_, size, _)) = of(b) else {
                return false;
            };
            (from..b).all(|k| {
                pieces[k].iter().filter(|p| p.end > p.start).all(|p| {
                    pieces[b].iter().filter(|q| q.end > q.start).any(|q| {
                        let (a, c) = (&runs[p.start], &runs[q.start]);
                        (q.x0 - p.x0).abs() <= TITLE_ALIGN * size
                            && a.font() == c.font()
                            && a.italic() == c.italic()
                    })
                }) && of(k).is_some_and(|l| (l.1 - size).abs() <= SAME_SIZE * size)
            })
        };
        if let Some(from) = first_from.filter(|&f| goes_on(f)) {
            for slot in out.iter_mut().take(b).skip(from) {
                *slot = w + 1;
            }
        }
    }
    out
}

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
pub fn set_full<R: LineRun>(runs: &[R], pieces: &[Vec<Piece>], lines: &[RunRange]) -> bool {
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
        text_of(runs, p.start, p.end)
            .trim_start()
            .chars()
            .next()
            .is_some_and(char::is_lowercase)
    });
    small || hi - lo > tol
}

/// Whether two lines alone (`lines`, their `pieces`) are a header of titles over one row of
/// values: their pieces stand in the same three columns or more, piece for piece, the first
/// of titles (one word each, no digit) over a row that holds a digit (`Pos Datum Betrag` over
/// `1 15.01.2026 9,99`). Blocks side by side that open with a name of two words or more (a
/// firm over its street beside a bank over its account) are no titles over values.
pub fn titles_over_values<R: LineRun>(
    runs: &[R],
    lines: &[RunRange],
    pieces: &[Vec<Piece>],
) -> bool {
    if lines.len() != 2 || pieces.len() != 2 {
        return false;
    }
    let digit = |p: &Piece| {
        runs[p.start..p.end]
            .iter()
            .any(|r| r.text().chars().any(|c| c.is_ascii_digit()))
    };
    let title =
        |p: &Piece| !digit(p) && text_of(runs, p.start, p.end).split_whitespace().count() == 1;
    // Each title stands in one column with the value under it: their starts or their ends
    // meet (within half a size), as columns set flush left or flush right do. Words of two
    // lines set full (justified) share the lines' outer edges, and a middle word stands about
    // in the middle, but no word between them starts or ends where the one under it does:
    // middles that meet are no evidence.
    let tol = 0.5
        * lines
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
    pieces[0].len() >= 3
        && pieces[0].len() == pieces[1].len()
        && pieces[0]
            .iter()
            .zip(&pieces[1])
            .all(|(p, q)| p.x0 < q.x1 && q.x0 < p.x1 && meet(p, q))
        && !(even(&pieces[0]) && even(&pieces[1]) && outer(&pieces[0], &pieces[1]))
        && pieces[0].iter().all(title)
        && pieces[1].iter().any(digit)
        && !prose_row(
            &text_of(runs, lines[1].start, lines[1].end),
            even(&pieces[1]) && outer(&pieces[0], &pieces[1]),
        )
}

/// Whether the second of two lines reads as the end of a sentence that goes on from the line
/// over it: it opens with a small letter, or it ends with a full stop and is `set_full` to the
/// measure of the line over it (its words even gaps apart, both its edges at that line's
/// edges), as a line of prose set full is (`Die 3 Hinweise.`). A row of values does not open
/// with a small letter, and its cells stand where their columns start or end: its last cell
/// may end with the stop of an abbreviation (`12 Stk.`, `Muster Co. KG.`).
pub(crate) fn prose_row(text: &str, set_full: bool) -> bool {
    let t = text.trim();
    t.starts_with(char::is_lowercase) || (t.ends_with('.') && set_full)
}

/// The shape of a value: what a column of values holds. A shape read wrong may only keep a
/// title merged, never cut one, so the shapes are few and broad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
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

impl Shape {
    /// The shape of `text`; none for a value with digits in a form none of the shapes knows
    /// (`12,-`, `6,98-`, `(6,98)`, `8:00`, `50 ct`, `1.2.24`) and for one with neither digits
    /// nor letters.
    pub fn of(text: &str) -> Option<Shape> {
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
    pub fn most(shapes: &[Option<Shape>]) -> Option<Shape> {
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

/// Whether `text` is an amount: a number (see [`Shape::Number`]) with a decimal part of two
/// digits (`20,00`, `1.000,00`, `12,50 EUR`), or such a number with a minus after it (`12,50-`,
/// a deduction or a credit). Whole numbers, rates and dates are none.
pub fn amount(text: &str) -> bool {
    let text = text.trim();
    let text = if signed_amount(text) {
        &text[..text.len() - 1]
    } else {
        text
    };
    if Shape::of(text) != Some(Shape::Number) {
        return false;
    }
    let mut t = text;
    for c in CURRENCIES {
        t = t.trim_start_matches(c).trim_end_matches(c).trim();
    }
    t.rfind(['.', ','])
        .is_some_and(|at| t.len() - at == 3 && t[at + 1..].bytes().all(|c| c.is_ascii_digit()))
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
pub(crate) fn title_cut<R: LineRun>(
    runs: &[R],
    p: &Piece,
    (left, right): ((f32, f32), (f32, f32)),
    differ: bool,
) -> Option<(usize, usize)> {
    let size = runs[p.start..p.end]
        .iter()
        .map(|r| r.size())
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
        let r = &runs[i];
        if i > p.start {
            let prev = &runs[i - 1];
            if !r.text().trim().is_empty()
                && fits(prev.x() + prev.w(), r.x(), prev.text(), r.text())
            {
                cut = Some((i, 0));
            }
        }
        // The word starts of the run's text after its first word, by bytes and characters.
        let text = r.text();
        let mut starts: Vec<(usize, usize, usize)> = Vec::new();
        let (mut seen, mut space) = (false, None);
        for (n, (b, ch)) in text.char_indices().enumerate() {
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
        let chars = text.chars().count().max(1) as f32;
        let no_gaps = r.gaps().next().is_none();
        for (w, &(byte, from, to)) in starts.iter().enumerate() {
            let words = w as u32 + 1;
            let (x0, x1) = match r.gaps().find(|g| g.words == words) {
                Some(g) => (g.from, g.to),
                None if no_gaps => (
                    r.x() + r.w() * from as f32 / chars,
                    r.x() + r.w() * to as f32 / chars,
                ),
                None => continue,
            };
            if fits(x0, x1, &text[..byte], &text[byte..]) {
                cut = Some((i, byte));
            }
        }
    }
    cut
}
