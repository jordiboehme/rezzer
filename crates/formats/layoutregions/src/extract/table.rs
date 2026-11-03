// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The cells of tables: which run of a table goes to which cell, how far each cell spans, and
//! why a set of lines or a grid of rules does not hold as a table.
//!
//! Finding a table stays with the reader: where a grid's sections end, which runs a section
//! holds, titles and captions, the limits of a page and the refusals that read the page around
//! a table. Building its cells is here, with the refusals that read only the cells: a grid
//! whose cells are mostly empty, columns that are not clean, text that runs on like prose.
//! Three extractors build them: [`ruled::ruled_cells`] for a section of a grid of drawn rules,
//! [`banded::band_cells`] for a header band of drawn cells and the lines under it, and
//! [`grid::grid_cells`] for lines in columns without rules. Each answers with the table or
//! why there is none: [`Cells`] for a ruled section, which may also be too large or stopped,
//! [`banded::BandCells`] for a band, which may be too large, and a `Result` for lines without
//! rules.
//!
//! Every function works on the reader's own runs (see [`LineRun`]), borrowed, and takes and
//! returns indexes into the slice it is given: the reader cuts runs in pieces that share one
//! primitive, so an index names a piece where a primitive id cannot.

use std::collections::HashMap;

use crate::line::LineRun;
use crate::segment::drawn::Line;

pub mod area;
pub mod banded;
pub mod caption;
pub mod grid;
pub mod ruled;
pub mod sections;

/// A gap wider than this many font sizes between two runs of a line separates two cells.
pub const CELL_GAP: f32 = 1.0;

/// Squares one grid of drawn rules may have to be read as a table.
pub const MAX_SQUARES: usize = 100_000;

/// Cells of a table without rules are short: their median length is under this many
/// characters, unless most of them are numbers.
pub(crate) const SHORT_CELL: usize = 40;

/// Cells of a table without rules next to a table caption may be longer: their median length
/// is under this many characters.
pub(crate) const CAPTIONED_CELL: usize = 60;

/// A cell of at least this many words and characters that goes on in lower case in the cell
/// below reads as a line of prose.
pub(crate) const PROSE_WORDS: usize = 3;
pub(crate) const PROSE_LINE: usize = 20;

/// Lines a title over a grid may have.
const TITLE_LINES: usize = 2;

/// Steps the title search of one page may take over the page's column rules; past them no
/// line over a cut grid is taken as a title.
pub const TITLE_WORK: usize = 1_000_000;

/// Why a run of a ruled section is in no cell: it reaches over the side of its cell onto the
/// text of another cell on its line.
pub(crate) const RUNS_INTO_CELL: &str =
    "it runs over the side of its cell into the text of another cell";

/// Why a line of a table without rules is in no cell: it stands between two rows and runs over
/// the gutter between two columns (a note between rows).
pub(crate) const RUNS_OVER_COLUMNS: &str = "it stands between two rows and runs over their columns";

/// One cell: the runs in it (indexes into the runs the extractor was given, in no particular
/// order), how many rows and columns it spans, and what it says when it has no runs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellOut {
    pub runs: Vec<usize>,
    pub row_span: u32,
    pub col_span: u32,
    pub status: Option<&'static str>,
}

/// Where a run of close titles of a table without rules is cut in two once the runs are in
/// their cells: its row, its cell, its place among the runs of the cell (in the order of the
/// runs) and the byte of its text the next title starts at (see [`grid::title_cut`]).
pub type TitleCut = (usize, usize, usize, usize);

/// The cells of a table.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableOut {
    pub rows: Vec<Vec<CellOut>>,
    /// The leading rows that are its header by drawn rules or style; 0 leaves the header to
    /// the text of its first row (see [`header_of`]), once the cells are filled.
    pub header_rows: usize,
    /// Runs in no cell, each with why: the reader keeps their text where it stands and says
    /// so. No run is guessed into a cell.
    pub unplaced: Vec<(usize, &'static str)>,
    /// Cells drawn as icons (a ruled section's first-row cell without text that holds drawn
    /// strokes): row, cell and the cell's box (x0, y0, x1, y1).
    pub icons: Vec<(usize, usize, [f32; 4])>,
    /// Runs of close titles to cut (tables without rules).
    pub cuts: Vec<TitleCut>,
}

/// What an extractor makes of the lines or the grid it is given.
#[derive(Debug, Clone, PartialEq)]
pub enum Cells {
    Table(TableOut),
    /// No table, and why.
    Refused(&'static str),
    /// More cells than [`MAX_SQUARES`]: `rows` by `cols`.
    TooLarge {
        rows: usize,
        cols: usize,
    },
    /// The time limit was reached.
    Stopped,
}

/// The point of a run that decides which cell it is in.
pub fn middle<R: LineRun>(r: &R) -> (f32, f32) {
    (r.x() + r.w() / 2.0, r.y() + r.h() * 0.3)
}

/// Whether a cell's text is a number: most of its characters are digits or signs.
pub fn numeric(text: &str) -> bool {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let digits = chars.iter().filter(|c| c.is_ascii_digit()).count();
    let signs = chars
        .iter()
        .filter(|c| matches!(c, '.' | ',' | '%' | '-' | '+' | '$' | '(' | ')' | '/'))
        .count();
    digits > 0 && 2 * (digits + signs) >= chars.len()
}

/// 1 when the first row of the cells `rows` (each the runs it holds) reads as column titles:
/// every cell has text and either a column has a word on top of numbers, or the first row is
/// bold and the rest is not.
pub fn header_of<R: LineRun>(rows: &[Vec<&[R]>]) -> usize {
    let text = |c: &[R]| {
        c.iter()
            .flat_map(|r| r.text().split_whitespace())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let Some(first) = rows.first() else { return 0 };
    if rows.len() < 2 || first.iter().any(|c| text(c).is_empty()) {
        return 0;
    }
    let titles = (0..first.len()).any(|c| {
        let below: Vec<String> = rows[1..]
            .iter()
            .filter_map(|r| r.get(c))
            .map(|c| text(c))
            .collect();
        let numbers = below.iter().filter(|t| numeric(t)).count();
        !numeric(&text(first[c])) && numbers > 0 && 2 * numbers >= below.len()
    });
    let bold = |row: &[&[R]]| row.iter().flat_map(|c| c.iter()).all(|r| r.bold());
    let bold_titles = bold(first) && rows[1..].iter().any(|r| !bold(r));
    usize::from(titles || bold_titles)
}

/// For each line number of `runs`, how many of its runs hold text, wherever they stand.
pub fn texted_per_line<R: LineRun>(runs: &[R]) -> HashMap<u32, u32> {
    let mut out: HashMap<u32, u32> = HashMap::new();
    for r in runs.iter().filter(|r| !r.text().trim().is_empty()) {
        *out.entry(r.line()).or_default() += 1;
    }
    out
}

/// Whether `run`, which holds text, is the only run with text on its line (`per_line` from
/// [`texted_per_line`]).
pub(crate) fn alone_on_line<R: LineRun>(per_line: &HashMap<u32, u32>, run: &R) -> bool {
    per_line.get(&run.line()).is_some_and(|n| *n == 1)
}

/// One line of runs (by [`LineRun::line`], next to each other): the range of its runs, its
/// baseline (its first run's) and its largest size, at least 1.
#[derive(Debug, Clone, Copy)]
pub struct RunRange {
    pub start: usize,
    pub end: usize,
    pub y: f32,
    pub size: f32,
}

pub fn run_ranges<R: LineRun>(runs: &[R]) -> Vec<RunRange> {
    let mut out: Vec<RunRange> = Vec::new();
    for (i, r) in runs.iter().enumerate() {
        match out.last_mut() {
            Some(l) if runs[l.start].line() == r.line() => {
                l.end = i + 1;
                l.size = l.size.max(r.size());
            }
            _ => out.push(RunRange {
                start: i,
                end: i + 1,
                y: r.y(),
                size: r.size().max(1.0),
            }),
        }
    }
    out
}

/// One line of text: its baseline, its largest size, whether all of it is bold, and its
/// runs.
#[derive(Debug, Clone, PartialEq)]
pub struct RunLine {
    pub y: f32,
    pub size: f32,
    pub bold: bool,
    pub runs: Vec<usize>,
}

/// The lines of the runs `idx` of `runs` from the bottom: runs whose baselines lie within a
/// third of their size of the line's first.
pub fn lines_up<R: LineRun>(runs: &[R], idx: &[usize]) -> Vec<RunLine> {
    let mut sorted = idx.to_vec();
    sorted.sort_by(|&a, &b| runs[a].y().total_cmp(&runs[b].y()));
    let mut out: Vec<RunLine> = Vec::new();
    for i in sorted {
        let r = &runs[i];
        match out.last_mut() {
            Some(l) if r.y() - l.y <= 0.3 * r.size().max(1.0) => {
                l.size = l.size.max(r.size());
                l.bold &= r.bold();
                l.runs.push(i);
            }
            _ => out.push(RunLine {
                y: r.y(),
                size: r.size(),
                bold: r.bold(),
                runs: vec![i],
            }),
        }
    }
    out
}

/// The title among the runs `idx` of `runs` that stands right over the rule at height `base`:
/// the lowest line over it and, for a title of two lines, the line one line pitch above it in
/// the same style, none crossed by a column rule (`crossed` of a height range). The title is
/// at most twice its size over the rule and closer to it than to the next line or to
/// `ceiling` above (the row edge over it), or bold and at most its size over the rule.
pub fn title_over<R: LineRun>(
    runs: &[R],
    idx: &[usize],
    base: f32,
    ceiling: f32,
    crossed: &mut dyn FnMut(f32, f32) -> bool,
) -> Vec<usize> {
    let above: Vec<usize> = idx
        .iter()
        .copied()
        .filter(|&i| runs[i].y() > base)
        .collect();
    let lines = lines_up(runs, &above);
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

/// Whether a column rule of `vs` (sorted by place) strictly between `left` and `right`
/// crosses the heights `lo..hi`. Each rule looked at is counted in `work`; past
/// [`TITLE_WORK`] the answer is yes, so no title is taken without the look.
pub fn column_across(
    vs: &[Line],
    (left, right): (f32, f32),
    (lo, hi): (f32, f32),
    work: &mut usize,
) -> bool {
    let first = vs.partition_point(|s| s.at <= left);
    for s in &vs[first..] {
        *work += 1;
        if *work > TITLE_WORK {
            return true;
        }
        if s.at >= right {
            break;
        }
        if s.from < hi && s.to > lo {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests;
