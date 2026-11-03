// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The cells of a header band (a row of drawn cells) and the lines without rules under it,
//! which stand in its columns.

use std::collections::BTreeMap;

use super::caption::ByY;
use super::{
    CELL_GAP, CellOut, LONG_TEXT, MAX_SQUARES, PROSE_LINE, PROSE_WORDS, RUNS_ON, RunRange,
    SHORT_CELL, TableOut, middle, numeric,
};
use crate::line::LineRun;
use crate::segment::drawn::{Line, SNAP};

/// The lines of a band's body follow each other at most this many line heights apart.
pub const BODY_GAP: f32 = 3.0;

/// A rule as wide as a band, within this share of its width, ends the band's body.
pub const BAND_EDGE: f32 = 0.05;

/// A header band is at most this many times its largest title text high.
pub const BAND_HEIGHT: f32 = 4.0;

/// Why a row of drawn cells `b` (x0, y0, x1, y1) with the runs `head` of `runs` and the
/// `titles` of its cells (see [`band_titles`]) is no header band, if it is not: column titles
/// are words, so fewer than three cells of words is a scale or a counter; and a band is a line
/// or two high, so one taller than [`BAND_HEIGHT`] times its largest text holds blocks of text.
pub fn band_head_refused<R: LineRun>(
    runs: &[R],
    head: &[usize],
    titles: &BTreeMap<usize, String>,
    b: [f32; 4],
) -> Option<&'static str> {
    let size = head.iter().map(|&i| runs[i].size()).fold(0.0f32, f32::max);
    let tall = b[3] - b[1] > BAND_HEIGHT * size;
    if tall {
        Some("it is too tall for a header over lines")
    } else if titles.values().filter(|t| !numeric(t)).count() < 3 {
        Some("its cells hold too few column titles")
    } else {
        None
    }
}

/// Where the body of a header band is looked for: the page's lines and their index by height,
/// its horizontal rules sorted by height, the height no body line may reach (`floor`), and the
/// work the search may take (`bound`).
pub struct BodyBelow<'a> {
    pub lines: &'a [RunRange],
    pub by_y: &'a ByY,
    pub hs: &'a [Line],
    pub floor: f32,
    pub bound: usize,
}

/// The lines below the band `b` with cell edges `xs` that are its body, from the top: lines
/// of runs `free` takes, above the floor, each less than [`BODY_GAP`] line heights below the
/// one before, with no rule as wide as the band between them, each piece of each standing in
/// one column. Also whether a line that does not stand in the columns ended it. `None` once
/// `work` passes the bound.
pub fn body_below<R: LineRun>(
    runs: &[R],
    at: &BodyBelow<'_>,
    b: [f32; 4],
    xs: &[f32],
    free: &dyn Fn(usize) -> bool,
    work: &mut usize,
) -> Option<(Vec<BodyLine>, bool)> {
    let lo = at.by_y.line_y.partition_point(|y| *y < b[1]);
    let mut body: Vec<BodyLine> = Vec::new();
    let mut last_y = b[1];
    let mut misfit = false;
    for &k in at.by_y.lines[..lo].iter().rev() {
        *work += 1;
        let l = &at.lines[k];
        if l.y <= at.floor {
            break;
        }
        if *work > at.bound {
            return None;
        }
        let ids: Vec<usize> = (l.start..l.end)
            .filter(|&i| {
                let r = &runs[i];
                let (cx, _) = middle(r);
                cx > b[0] - SNAP && cx < b[2] + SNAP && free(i) && !r.text().trim().is_empty()
            })
            .collect();
        *work += l.end - l.start;
        if ids.is_empty() {
            continue;
        }
        if last_y - l.y > BODY_GAP * l.size.max(1.0) {
            break;
        }
        // A rule as wide as the band between this line and the one above ends the table.
        let top = l.y + l.size;
        let from = at.hs.partition_point(|s| s.at < top);
        let to = at.hs.partition_point(|s| s.at < last_y);
        let tol = SNAP.max(BAND_EDGE * (b[2] - b[0]));
        *work += to.saturating_sub(from);
        if at.hs[from..to.max(from)]
            .iter()
            .any(|s| s.from <= b[0] + tol && s.to >= b[2] - tol)
        {
            break;
        }
        // Pieces, each in one column of the band (see `body_pieces`): a line that does not
        // fit ends the body.
        let Some(placed) = body_pieces(runs, &ids, xs) else {
            misfit = true;
            break;
        };
        body.push((l.y, l.size, placed));
        last_y = l.y;
    }
    (*work <= at.bound).then_some((body, misfit))
}

/// A line of a band's body: its baseline and size, and its pieces, each a column and its
/// runs.
pub type BodyLine = (f32, f32, Vec<(usize, Vec<usize>)>);

/// The column of `xs` that holds the stretch `x0..x1`, when one does.
fn column_of(xs: &[f32], x0: f32, x1: f32) -> Option<usize> {
    let c = xs
        .partition_point(|e| *e <= (x0 + x1) / 2.0)
        .checked_sub(1)?;
    (c + 1 < xs.len() && x0 >= xs[c] - SNAP && x1 <= xs[c + 1] + SNAP).then_some(c)
}

/// The pieces of a line of a band's body (the runs `line` of `runs`, left to right) in the
/// band's columns of edges `xs`: runs apart by more than a cell gap, or by a column edge of
/// the band in the white space between them (a value line padded into the band's columns),
/// each with its column. `None` when a piece stands in no one column: the line does not fit
/// the band.
pub fn body_pieces<R: LineRun>(
    runs: &[R],
    line: &[usize],
    xs: &[f32],
) -> Option<Vec<(usize, Vec<usize>)>> {
    let inner = &xs[1..xs.len() - 1];
    let mut pieces: Vec<(f32, f32, Vec<usize>)> = Vec::new();
    for &i in line {
        let r = &runs[i];
        let tol = 0.1 * r.size().max(1.0);
        match pieces.last_mut() {
            Some(p)
                if r.x() - p.1 <= CELL_GAP * r.size().max(1.0)
                    && !inner.iter().any(|&e| e >= p.1 - tol && e <= r.x() + tol) =>
            {
                p.1 = p.1.max(r.x() + r.w());
                p.2.push(i);
            }
            _ => pieces.push((r.x(), r.x() + r.w(), vec![i])),
        }
    }
    pieces
        .into_iter()
        .map(|(x0, x1, ids)| column_of(xs, x0, x1).map(|c| (c, ids)))
        .collect()
}

/// The column of `xs` (the band's cell edges) whose cell holds the middle of `r`.
fn head_column<R: LineRun>(xs: &[f32], r: &R) -> usize {
    let (cx, _) = middle(r);
    xs.partition_point(|e| *e < cx)
        .saturating_sub(1)
        .min(xs.len() - 2)
}

/// The titles of a band's cells that hold text, by column: the text of its runs `head` of
/// `runs` in each cell of the edges `xs`.
pub fn band_titles<R: LineRun>(runs: &[R], head: &[usize], xs: &[f32]) -> BTreeMap<usize, String> {
    let mut titles: BTreeMap<usize, String> = BTreeMap::new();
    for &i in head {
        let r = &runs[i];
        titles
            .entry(head_column(xs, r))
            .or_default()
            .push_str(r.text().trim());
    }
    titles.retain(|_, t| !t.is_empty());
    titles
}

/// What [`band_cells`] makes of a band and its body.
#[derive(Debug, Clone, PartialEq)]
pub enum BandCells {
    Table(TableOut),
    /// No table, and why.
    Refused(&'static str),
    /// More cells than [`MAX_SQUARES`]: `rows` by `cols`.
    TooLarge {
        rows: usize,
        cols: usize,
    },
}

/// The table of a band of cell edges `xs` with its runs `head` and `titles` (see
/// [`band_titles`]) over the lines `body` from the top (`misfit` when a line that does not
/// stand in its columns ended them). Rows start by [`row_starts`]; only the columns with text
/// in the band or the body are kept. Refused when no line or fewer than three rows follow it,
/// or by [`long_or_prose`]. Its header is its first row.
pub fn band_cells<R: LineRun>(
    runs: &[R],
    head: &[usize],
    titles: &BTreeMap<usize, String>,
    xs: &[f32],
    (body, misfit): (&[BodyLine], bool),
) -> BandCells {
    let mut used: Vec<usize> = body.iter().flat_map(|l| l.2.iter().map(|p| p.0)).collect();
    used.sort_unstable();
    used.dedup();
    if used.is_empty() {
        return BandCells::Refused(if misfit {
            "the text below its header does not stand in its columns"
        } else {
            "too few rows follow its header"
        });
    }
    let mut row_of: Vec<usize> = Vec::with_capacity(body.len());
    let mut rows = 0usize;
    let pieces: Vec<Vec<(usize, bool)>> = body
        .iter()
        .map(|(_, _, placed)| {
            placed
                .iter()
                .map(|(c, ids)| {
                    let text: String = ids.iter().map(|&i| runs[i].text()).collect();
                    (*c, numeric(&text))
                })
                .collect()
        })
        .collect();
    for starts in row_starts(&pieces, used.len()) {
        rows += usize::from(starts);
        row_of.push(rows.saturating_sub(1));
    }
    if rows < 3 {
        return BandCells::Refused("too few rows follow its header");
    }
    // Columns with text in the band or the body; the others are dropped.
    let mut kept: Vec<usize> = titles.keys().copied().chain(used.iter().copied()).collect();
    kept.sort_unstable();
    kept.dedup();
    if (rows + 1).saturating_mul(kept.len()) > MAX_SQUARES {
        return BandCells::TooLarge {
            rows: rows + 1,
            cols: kept.len(),
        };
    }
    // The text of each body cell, for the tests of a table without rules: only the kept
    // columns, so the table is never as wide as all of the band's cells.
    let slot = |c: usize| kept.binary_search(&c).ok();
    let mut texts: Vec<Vec<String>> = vec![vec![String::new(); kept.len()]; rows];
    for ((_, _, placed), &r) in body.iter().zip(&row_of) {
        for (c, ids) in placed {
            let Some(k) = slot(*c) else { continue };
            let t = &mut texts[r][k];
            for &i in ids {
                for w in runs[i].text().split_whitespace() {
                    if !t.is_empty() {
                        t.push(' ');
                    }
                    t.push_str(w);
                }
            }
        }
    }
    if let Some(why) = long_or_prose(&texts, SHORT_CELL) {
        return BandCells::Refused(why);
    }
    let cell = || CellOut {
        runs: Vec::new(),
        row_span: 1,
        col_span: 1,
        status: None,
    };
    let mut out: Vec<Vec<CellOut>> = (0..=rows)
        .map(|_| kept.iter().map(|_| cell()).collect())
        .collect();
    for &i in head {
        if let Some(k) = slot(head_column(xs, &runs[i])) {
            out[0][k].runs.push(i);
        }
    }
    for ((_, _, placed), &r) in body.iter().zip(&row_of) {
        for (c, ids) in placed {
            if let Some(k) = slot(*c) {
                out[r + 1][k].runs.extend(ids);
            }
        }
    }
    BandCells::Table(TableOut {
        rows: out,
        header_rows: 1,
        ..TableOut::default()
    })
}

/// For the lines of a body without rules from the top, each its pieces as (column, whether
/// the piece is a number), whether the line starts a row. A line that fills at least half of
/// the `used` columns starts one, also with an empty first cell (a date printed once a day).
/// A line that fills less goes on the row above only when each of its pieces goes on text in
/// its column there and none is a number (a cell that wraps); any other line is a row of its
/// own, with empty cells where it has no piece. Lines of words above the first line that
/// fills half of the columns, each in a column that line fills, are the first lines of its
/// cells (a cell that wraps upward over values set at the middle of the row): they start no
/// row and go on the first row. Any other first line starts a row.
pub(crate) fn row_starts(lines: &[Vec<(usize, bool)>], used: usize) -> Vec<bool> {
    let columns = |pieces: &[(usize, bool)]| {
        let mut cols: Vec<usize> = pieces.iter().map(|p| p.0).collect();
        cols.sort_unstable();
        cols.dedup();
        cols
    };
    let full = |pieces: &[(usize, bool)]| 2 * columns(pieces).len() >= used;
    // The lines above the first full line that wrap upward into its cells.
    let lead = lines
        .iter()
        .position(|l| full(l))
        .filter(|&f| {
            let below = columns(&lines[f]);
            lines[..f]
                .iter()
                .flatten()
                .all(|(c, number)| !number && below.contains(c))
        })
        .unwrap_or(0);
    let mut filled: Vec<usize> = lines[..lead].iter().flat_map(|l| columns(l)).collect();
    let mut out = vec![false; lead];
    for (k, pieces) in lines.iter().enumerate().skip(lead) {
        let cols = columns(pieces);
        let joins = k > lead
            && 2 * cols.len() < used
            && pieces
                .iter()
                .all(|(c, number)| !number && filled.contains(c));
        if !joins && k > lead {
            filled.clear();
        }
        filled.extend(cols);
        out.push(!joins);
    }
    out
}

/// Why the cells `texts` (rows of cells) of a table without rules are not a table's: their
/// text is long (the median cell has `short` characters or more, and fewer than half of the
/// cells are numbers), or runs on from a cell into the cell below like prose (a cell of a few
/// words that ends without a stop over a cell that goes on in a small letter). One rule for
/// the cells under a header band and for lines without rules (see [`super::grid::grid_cells`]),
/// and the evidence the typers read as prose.
pub fn long_or_prose(texts: &[Vec<String>], short: usize) -> Option<&'static str> {
    let mut lens: Vec<usize> = texts
        .iter()
        .flatten()
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().count())
        .collect();
    lens.sort_unstable();
    let numbers = texts.iter().flatten().filter(|t| numeric(t)).count();
    if lens.is_empty() || (lens[lens.len() / 2] >= short && 2 * numbers < lens.len()) {
        return Some(LONG_TEXT);
    }
    for r in 1..texts.len() {
        for (above, below) in texts[r - 1].iter().zip(&texts[r]) {
            let words = above.split_whitespace().count();
            let ends = above.ends_with(['.', ':', ';', '!', '?']);
            let line = words >= PROSE_WORDS && above.chars().count() >= PROSE_LINE;
            if line && !ends && below.chars().next().is_some_and(char::is_lowercase) {
                return Some(RUNS_ON);
            }
        }
    }
    None
}
