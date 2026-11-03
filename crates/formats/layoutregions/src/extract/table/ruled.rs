// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The cells of a section of a grid of drawn rules, and where a column rule cuts a run of
//! text in two.

use std::collections::{HashMap, HashSet};

use super::banded::{BAND_EDGE, BODY_GAP, row_starts};
use super::{
    CELL_GAP, CellOut, Cells, MAX_SQUARES, PROSE_LINE, PROSE_WORDS, RUNS_INTO_CELL, RunRange,
    TableOut, alone_on_line, middle, numeric, run_ranges, texted_per_line,
};
use crate::line::{LABEL_STEP, LineRun, is_placeholder};
use crate::segment::drawn::{Line, SNAP, clustered};

/// A grid whose cells are more often empty than this is a form to fill in.
const EMPTY_SHARE: f32 = 0.6;

/// Steps one page may take to cut its runs at column rules; past them the rest stay whole.
pub const COLUMN_CUT_WORK: usize = 1_000_000;

/// The column edges and the row edges (from the top) of a grid of horizontal segments `h`
/// and vertical segments `v`.
pub fn edges(h: &[Line], v: &[Line]) -> (Vec<f32>, Vec<f32>) {
    let mut xs: Vec<f32> = v.iter().map(|s| s.at).collect();
    xs.push(h.iter().map(|s| s.from).fold(f32::MAX, f32::min));
    xs.push(h.iter().map(|s| s.to).fold(f32::MIN, f32::max));
    let mut ys: Vec<f32> = h.iter().map(|s| s.at).collect();
    ys.push(v.iter().map(|s| s.from).fold(f32::MAX, f32::min));
    ys.push(v.iter().map(|s| s.to).fold(f32::MIN, f32::max));
    let mut ys = clustered(ys);
    ys.reverse();
    (clustered(xs), ys)
}

/// For the rows between the row edges `ys` (from the top) and the column edges `xs`, the
/// heights where the inner column rules of a row end when it has some, all of them hang
/// from its top and they end at one height clear of its bottom, each with that bottom. Each
/// rule is looked at in the rows it reaches into, no more.
pub fn hanging_feet(v: &[Line], xs: &[f32], ys: &[f32]) -> Vec<(f32, f32)> {
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
/// `within` of `runs` is boxed fields: columns of one width, narrow as a box for each digit,
/// and in each row at most one run of text, which is digits (with the separators read from
/// marks, see [`is_placeholder`]). Its text is read as it is, and its refusal is no loss.
pub fn boxed_field<R: LineRun>(runs: &[R], within: &[usize], xs: &[f32], ys: &[f32]) -> bool {
    let (Some(first), Some(size)) = (
        xs.windows(2).map(|w| w[1] - w[0]).next(),
        within.iter().map(|&i| runs[i].size()).reduce(f32::max),
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
            let r = &runs[i];
            let (_, cy) = middle(r);
            digits(r.text()) && rows.insert(ys.partition_point(|e| *e > cy))
        })
}

/// The open rows of a section of column edges `xs` and row edges `ys` (from the top), its
/// vertical segments `v`: a row whose inner column rules all hang from its top and end at one
/// height is a header over lines without rules: a rule across at their feet parts the two,
/// and the row below it (its lines) is open: one cell across that is no spanning box. Only in
/// a cut grid (`cut`) or a row alone, and only over lines of text it is for (`holds_text` of
/// the height range from the row's bottom to the foot). The feet are added to `ys` and as
/// rules to `h`; their heights are returned.
pub fn hanging_rows(
    v: &[Line],
    xs: &[f32],
    (ys, h): (&mut Vec<f32>, &mut Vec<Line>),
    cut: bool,
    holds_text: &dyn Fn(f32, f32) -> bool,
) -> Vec<f32> {
    let mut open: Vec<f32> = Vec::new();
    if xs.len() < 3 || !(cut || ys.len() == 2) {
        return open;
    }
    let feet: Vec<f32> = hanging_feet(v, xs, ys)
        .into_iter()
        .filter(|&(foot, bottom)| holds_text(bottom, foot))
        .map(|(foot, _)| foot)
        .collect();
    for foot in feet {
        let at = ys.partition_point(|e| *e > foot);
        ys.insert(at, foot);
        open.push(foot);
        h.push(Line {
            at: foot,
            from: xs[0],
            to: xs[xs.len() - 1],
        });
    }
    open
}

/// Whether the text of a run that stands over `columns` columns reads as a sentence: at
/// least [`PROSE_WORDS`] words and [`PROSE_LINE`] characters, more words than columns, and
/// most of them words with a small letter and no digit. Values, codes, amounts and flags do
/// not.
pub(crate) fn prose_like(text: &str, columns: usize) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    let worded = words
        .iter()
        .filter(|w| w.chars().any(char::is_lowercase) && !w.chars().any(|c| c.is_ascii_digit()))
        .count();
    words.len() >= PROSE_WORDS.max(columns + 1)
        && text.trim().chars().count() >= PROSE_LINE
        && 2 * worded > words.len()
}

/// Whether `run` is a note across the columns of `xs`: a sentence over at least one inner
/// column edge (see [`prose_like`]). The caller checks that it is the only run on its line.
fn note_across<R: LineRun>(run: &R, xs: &[f32]) -> bool {
    let across = xs[1..xs.len() - 1]
        .iter()
        .filter(|&&e| e > run.x() + SNAP && e < run.x() + run.w() - SNAP)
        .count();
    across > 0 && prose_like(run.text(), across + 1)
}

/// Whether `run` goes on with the line of a note `note` above it: in its size and weight,
/// one line pitch below, and a sentence itself (see [`prose_like`]), so a value alone on
/// the line below a note stays a row of its own. The caller checks that it is alone on its
/// line.
fn goes_on_note<R: LineRun>(note: &R, run: &R) -> bool {
    let pitch = note.y() - run.y();
    (run.size() - note.size()).abs() <= 0.05 * note.size().max(1.0)
        && run.bold() == note.bold()
        && pitch > 0.3 * note.size()
        && pitch <= 1.6 * note.size().max(1.0)
        && prose_like(run.text(), 1)
}

/// A section of a grid of drawn rules, as the run cuts read it: its horizontal and vertical
/// segments, and whether its grid was cut in sections.
#[derive(Debug, Clone, Copy)]
pub struct GridPart<'a> {
    pub h: &'a [Line],
    pub v: &'a [Line],
    pub cut: bool,
}

/// A rule that may cut runs: its place and the heights it cuts at, from and to.
type Span = (f32, f32, f32);

/// A place of rules that may cut runs and the heights they cut at, as sorted ranges apart
/// from each other.
type Place = (f32, Vec<(f32, f32)>);

/// The rules that may cut runs by place (within a tenth of a point).
fn places_of(mut spans: Vec<Span>) -> Vec<Place> {
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut places: Vec<Place> = Vec::new();
    for (x, from, to) in spans {
        match places.last_mut() {
            Some((px, list)) if x - *px <= 0.1 => list.push((from, to)),
            _ => places.push((x, vec![(from, to)])),
        }
    }
    for (_, list) in &mut places {
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f32, f32)> = Vec::with_capacity(list.len());
        for &(from, to) in list.iter() {
            match merged.last_mut() {
                Some(last) if from <= last.1 => last.1 = last.1.max(to),
                _ => merged.push((from, to)),
            }
        }
        *list = merged;
    }
    places
}

/// Where to cut each run of `runs` that an inner column rule of a section of `parts` crosses:
/// in the white space between two of its words, so each part stands in its own column (a
/// value line padded into its fields often has only one or two spaces between two fields).
/// A rule cuts at the heights it is drawn at; the inner rules of a header of hanging rules
/// also cut the lines below them in their section (see [`hanging_feet`]), but not a sentence
/// alone on its line that runs over them as words (a note across the section, see
/// [`prose_like`]). So do the inner column edges of a row of three drawn cells or more (a
/// header band) the lines under it, down to the first of the page's horizontal rules `hs`
/// (sorted by height) as wide as the row or to a gap of [`BODY_GAP`] line heights: its
/// table's lines without rules. A rule within a tenth of the text size of a gap cuts there; a
/// rule inside a word cuts nothing.
///
/// Gives each run to cut (by index, in order) with the gaps to cut it at (by their place
/// among its gaps, sorted), and false when more than `budget` steps were needed: the runs past
/// them stay whole.
pub fn column_cuts<R: LineRun>(
    runs: &[R],
    parts: &[GridPart<'_>],
    hs: &[Line],
    budget: usize,
) -> (Vec<(usize, Vec<usize>)>, bool) {
    // The rules as drawn, and the inner column edges of hanging headers below their feet.
    let (mut drawn, mut hanging): (Vec<Span>, Vec<Span>) = (Vec::new(), Vec::new());
    // The page's lines from the top, for the bodies of rows of drawn cells, once needed.
    let mut lines: Option<Vec<RunRange>> = None;
    for p in parts {
        let (xs, ys) = edges(p.h, p.v);
        if xs.len() < 3 || ys.len() < 2 {
            continue;
        }
        let inner = &xs[1..xs.len() - 1];
        for s in p.v {
            if s.at > xs[0] + SNAP && s.at < xs[xs.len() - 1] - SNAP {
                drawn.push((s.at, s.from, s.to));
            }
        }
        if p.cut || ys.len() == 2 {
            for (foot, bottom) in hanging_feet(p.v, &xs, &ys) {
                hanging.extend(inner.iter().map(|&x| (x, bottom, foot)));
            }
        }
        if ys.len() == 2 && xs.len() >= 4 {
            let (left, right, bottom) = (xs[0], xs[xs.len() - 1], ys[1]);
            let tol = SNAP.max(BAND_EDGE * (right - left));
            let rule = hs[..hs.partition_point(|s| s.at < bottom - SNAP)]
                .iter()
                .rev()
                .find(|s| s.from <= left + tol && s.to >= right - tol)
                .map_or(f32::MIN, |s| s.at);
            // As far down as the band's body reaches: its lines follow each other at most
            // BODY_GAP line heights apart.
            let mut floor = bottom;
            let mut last = bottom;
            let below = lines.get_or_insert_with(|| {
                let mut all = run_ranges(runs);
                all.sort_by(|a, b| b.y.total_cmp(&a.y));
                all
            });
            for l in below.iter().filter(|l| l.y < bottom && l.y > rule) {
                let inside = runs[l.start..l.end].iter().any(|r| {
                    let (cx, _) = middle(r);
                    cx > left - SNAP && cx < right + SNAP && !r.text().trim().is_empty()
                });
                if !inside {
                    continue;
                }
                if last - l.y > BODY_GAP * l.size.max(1.0) {
                    break;
                }
                last = l.y;
                floor = l.y - 0.25 * l.size;
            }
            hanging.extend(inner.iter().map(|&x| (x, floor.max(rule), bottom)));
        }
    }
    if drawn.is_empty() && hanging.is_empty() {
        return (Vec::new(), true);
    }
    let (drawn, hanging) = (places_of(drawn), places_of(hanging));
    let mut work = 0usize;
    let mut done = true;
    // The runs to cut, each with the gaps it is cut at.
    let mut cuts: Vec<(usize, Vec<usize>)> = Vec::new();
    // The last line of a note, which the next line may go on: such a line is not cut either.
    let mut note: Option<&R> = None;
    let per_line = texted_per_line(runs);
    'runs: for (i, r) in runs.iter().enumerate() {
        if r.text().trim().is_empty() {
            continue;
        }
        if let Some(n) = note.filter(|n| n.line() != r.line()) {
            if goes_on_note(n, r) && alone_on_line(&per_line, r) {
                note = Some(r);
                continue;
            }
            note = None;
        }
        if r.off_axis() || r.gaps().next().is_none() {
            continue;
        }
        let (_, cy) = middle(r);
        let tol = 0.1 * r.size().max(1.0);
        let mut at: Vec<usize> = Vec::new();
        // At the hanging edges: the gaps found, the edges crossed and whether one of them
        // falls inside a word.
        let (mut below, mut crossed, mut in_word) = (Vec::new(), 0usize, false);
        for (list, places) in [&drawn, &hanging].into_iter().enumerate() {
            let first = places.partition_point(|p| p.0 <= r.x() + tol);
            // The places go right, so the first gap that does not end before a place only
            // moves forward: one pass over the run's gaps per list.
            let mut gaps = r.gaps().enumerate().peekable();
            for (x, heights) in &places[first..] {
                if *x >= r.x() + r.w() - tol {
                    break;
                }
                work += 1;
                if work > budget {
                    done = false;
                    break 'runs;
                }
                let h = heights.partition_point(|s| s.0 <= cy);
                if h == 0 || heights[h - 1].1 < cy {
                    continue;
                }
                // The first gap that does not end before the rule, and whether the rule lies
                // in it (the gaps are in order).
                while gaps.next_if(|(_, g)| g.to + tol < *x).is_some() {}
                let gap = gaps
                    .peek()
                    .filter(|(_, g)| g.from - tol <= *x)
                    .map(|(k, _)| *k);
                if list == 0 {
                    at.extend(gap);
                    continue;
                }
                crossed += 1;
                match gap {
                    Some(k) => below.push(k),
                    None => in_word = true,
                }
            }
        }
        // A sentence alone on its line that runs over the hanging edges as words (a note)
        // is not cut there.
        if in_word && prose_like(r.text(), crossed + 1) && alone_on_line(&per_line, r) {
            note = Some(r);
        } else {
            at.extend(below);
        }
        at.sort_unstable();
        at.dedup();
        if !at.is_empty() {
            cuts.push((i, at));
        }
    }
    (cuts, done)
}

/// A section of a grid ready to be read as a table: its segments, its column and row edges
/// (from the top), the feet of its open rows (rows of lines without rules under a header of
/// hanging rules), its runs (`mine`) and those of them with text (`within`), both sorted, and
/// how many runs with text each line of the page has (see [`texted_per_line`], built once per
/// page).
pub struct RuledIn<'a> {
    pub h: &'a [Line],
    pub v: &'a [Line],
    pub xs: &'a [f32],
    pub ys: &'a [f32],
    pub open: &'a [f32],
    pub mine: &'a [usize],
    pub within: &'a [usize],
    pub per_line: &'a HashMap<u32, u32>,
}

/// The cells of a section of two rows and two columns or more of `runs`: the cell each square
/// belongs to (see `cell_owners`), its header (see `header_rows_of`), the rows its bands
/// are cut in, and the tests a table must pass (see `grid_refused`, then `refuses`, the
/// reader's tests of the area). Each run goes to the cell of its middle; a run that reaches
/// over its cell's side into white space too, but one that reaches onto the text of another
/// cell on its line is in no cell ([`RUNS_INTO_CELL`]). A first-row cell without text that
/// holds drawn strokes (`strokes` of the cell's box) is an icon. `stop` is the time limit.
pub fn ruled_cells<R: LineRun>(
    runs: &[R],
    s: &RuledIn<'_>,
    stop: &dyn Fn() -> bool,
    refuses: &dyn Fn() -> Option<&'static str>,
    strokes: &mut dyn FnMut([f32; 4]) -> bool,
) -> Cells {
    let (xs, ys, within) = (s.xs, s.ys, s.within);
    debug_assert!(
        within.is_sorted() && s.mine.is_sorted(),
        "the runs come sorted"
    );
    let (cols, rows) = (xs.len() - 1, ys.len() - 1);
    let borders = Borders::new(s.h, s.v, xs, ys);
    let Some(mut owner) = cell_owners(&borders, xs, ys, stop) else {
        return Cells::Stopped;
    };
    let (head, styled) = header_rows_of(runs, within, &borders, xs, ys);
    // An open row (the lines under a header of hanging rules) has a cell for each of the
    // header's columns: each value goes into the column it stands under, and a field
    // left empty is an empty cell.
    let is_open: Vec<bool> = (0..rows)
        .map(|r| s.open.iter().any(|f| (ys[r] - f).abs() <= SNAP))
        .collect();
    for (r, row) in owner.iter_mut().enumerate() {
        if is_open[r] {
            *row = (0..cols).map(|c| (r, c)).collect();
        }
    }
    // A band of the body that no column rule parts, under a header that has them, is
    // read in the header's columns when its lines hold text in two columns or more.
    if head >= 1 && cols >= 3 {
        open_bands(runs, within, head, xs, ys, &mut owner);
    }
    let cells = Squares::new(xs, ys, owner, is_open);
    let row_at = |y: f32| cells.row_at(y);
    let cell_at = |x: f32, y: f32| cells.cell_at(x, y);
    // Rows of the body that only column rules part: the lines of a band that fill at
    // least half of its columns with text, the first of them included, each start a row.
    let mut anchors = anchor_rows(runs, within, head, &cells, &row_at, &cell_at);
    // The lines under a header of hanging rules are rows by the header's columns.
    open_anchors(runs, within, (xs, ys), s.open, &row_at, &mut anchors);
    let bands = Bands { anchors };
    // A line of prose across an open row (a note over the section) is one cell across
    // its row of the table; the value lines around it keep their columns.
    let mut spanned: HashSet<(usize, usize)> = HashSet::new();
    for &i in within {
        let run = &runs[i];
        let r = cells.row_at(middle(run).1);
        if !cells.is_open[r] {
            continue;
        }
        if alone_on_line(s.per_line, run) && note_across(run, xs) {
            spanned.insert((r, bands.sub_of(r, run)));
        }
    }
    let planned_count: usize = (0..rows).map(|r| bands.subs(r)).sum();
    if planned_count.saturating_mul(cols) > MAX_SQUARES {
        return Cells::TooLarge {
            rows: planned_count,
            cols,
        };
    }
    let mut with_text: HashSet<((usize, usize), usize)> = HashSet::new();
    for &i in within {
        let r = &runs[i];
        let (cx, cy) = middle(r);
        let o = cells.cell_at(cx, cy);
        with_text.insert((o, bands.sub_of(o.0, r)));
    }
    if let Some(why) = grid_refused(runs, within, &cells, &bands, &with_text, head).or_else(refuses)
    {
        return Cells::Refused(why);
    }
    let (mut planned_rows, place) = plan_grid(&cells, &bands, &spanned, planned_count);
    let unplaced = onto_other_cells(runs, within, &cells);
    // The refusals above count an unplaced run as text of the cell of its middle, as they
    // did before the guard; the icon test below looks only at the text a cell keeps.
    let kept_text: HashSet<((usize, usize), usize)> = if unplaced.is_empty() {
        with_text
    } else {
        within
            .iter()
            .filter(|i| unplaced.binary_search(i).is_err())
            .map(|&i| {
                let r = &runs[i];
                let (cx, cy) = middle(r);
                let o = cells.cell_at(cx, cy);
                (o, bands.sub_of(o.0, r))
            })
            .collect()
    };
    for &i in s.mine {
        if unplaced.binary_search(&i).is_ok() {
            continue;
        }
        let run = &runs[i];
        let (cx, cy) = middle(run);
        let o = cells.cell_at(cx, cy);
        let (r, c) = place[&(o, bands.sub_of(o.0, run))];
        planned_rows[r][c].runs.push(i);
    }
    // A cell of the first row without text that holds drawn strokes is a title drawn as
    // an icon.
    let mut icons = Vec::new();
    for (c, o) in cells.owner[0].iter().enumerate() {
        if *o != (0, c) || kept_text.contains(&(*o, 0)) {
            continue;
        }
        let (row_span, col_span) = cells.extent[o];
        let cell = [
            xs[c],
            ys[row_span as usize],
            xs[c + col_span as usize],
            ys[0],
        ];
        if strokes(cell) {
            let (r, k) = place[&(*o, 0)];
            icons.push((r, k, cell));
        }
    }
    Cells::Table(TableOut {
        rows: planned_rows,
        // A header of two rows or more in a style of its own is the grid's, as are the
        // rows over the first open row; otherwise the first row is judged by its text.
        header_rows: if styled {
            head
        } else {
            (0..cells.is_open.iter().position(|o| *o).unwrap_or(0))
                .map(|r| bands.subs(r))
                .sum()
        },
        unplaced: unplaced.into_iter().map(|i| (i, RUNS_INTO_CELL)).collect(),
        icons,
        cuts: Vec::new(),
    })
}

/// The runs of `within` (sorted) that reach over a side of their cell (by more than
/// [`SNAP`], in a row that is not open) onto the ink of a run of another cell on their line:
/// the middle would guess their cell. A run that reaches into white space goes by its middle.
fn onto_other_cells<R: LineRun>(runs: &[R], within: &[usize], cells: &Squares<'_>) -> Vec<usize> {
    // Each run that crosses: its cell and the stretches of it outside the cell.
    type Crossing = (usize, (usize, usize), [(f32, f32); 2]);
    let crossing: Vec<Crossing> = within
        .iter()
        .filter_map(|&i| {
            let r = &runs[i];
            let (o, x0, x1) = crosses(r, cells)?;
            Some((i, o, [(r.x(), x0), (x1, r.x() + r.w())]))
        })
        .collect();
    if crossing.is_empty() {
        return Vec::new();
    }
    let mut by_line: HashMap<u32, Vec<usize>> = HashMap::new();
    for &j in within {
        by_line.entry(runs[j].line()).or_default().push(j);
    }
    crossing
        .into_iter()
        .filter(|&(i, o, outside)| {
            by_line[&runs[i].line()].iter().any(|&j| {
                let other = &runs[j];
                let (cx, cy) = middle(other);
                let (a, b) = (other.x(), other.x() + other.w());
                j != i
                    && cells.cell_at(cx, cy) != o
                    && outside
                        .iter()
                        .any(|&(from, to)| b.min(to) - a.max(from) > 0.0)
            })
        })
        .map(|(i, _, _)| i)
        .collect()
}

/// The cell of `r` (by its middle) and the cell's left and right edges, when `r` reaches over
/// a side of it by more than [`SNAP`]: it crosses a drawn line. In an open row no line is
/// drawn there: a value that runs on into the next field goes by its middle, so it never
/// crosses.
fn crosses<R: LineRun>(r: &R, cells: &Squares<'_>) -> Option<((usize, usize), f32, f32)> {
    let (xs, cols) = (cells.xs, cells.cols());
    let (cx, cy) = middle(r);
    let o = cells.cell_at(cx, cy);
    let (x0, x1) = (xs[o.1], xs[(o.1 + cells.extent[&o].1 as usize).min(cols)]);
    (!cells.is_open[o.0] && (r.x() < x0 - SNAP || r.x() + r.w() > x1 + SNAP)).then_some((o, x0, x1))
}

/// The segments on each boundary of a section of column edges `xs` and row edges `ys` (from
/// the top), found once: its vertical segments by column edge, its horizontal ones by row
/// edge.
struct Borders {
    on_x: Vec<Vec<Line>>,
    on_y: Vec<Vec<Line>>,
}

impl Borders {
    fn new(h: &[Line], v: &[Line], xs: &[f32], ys: &[f32]) -> Borders {
        let nearest = |edges: &[f32], at: f32| -> Option<usize> {
            let k = edges.partition_point(|e| *e < at);
            [k.checked_sub(1), Some(k)]
                .into_iter()
                .flatten()
                .filter(|&i| i < edges.len() && (edges[i] - at).abs() <= SNAP)
                .min_by(|&a, &b| (edges[a] - at).abs().total_cmp(&(edges[b] - at).abs()))
        };
        let mut on_x: Vec<Vec<Line>> = vec![Vec::new(); xs.len()];
        for s in v {
            if let Some(k) = nearest(xs, s.at) {
                on_x[k].push(*s);
            }
        }
        let ys_up: Vec<f32> = ys.iter().rev().copied().collect();
        let mut on_y: Vec<Vec<Line>> = vec![Vec::new(); ys.len()];
        for s in h {
            if let Some(k) = nearest(&ys_up, s.at) {
                on_y[ys.len() - 1 - k].push(*s);
            }
        }
        Borders { on_x, on_y }
    }

    /// Whether a boundary is drawn over at least half of a cell's side.
    fn covered(segs: &[Line], lo: f32, hi: f32) -> bool {
        let len: f32 = segs
            .iter()
            .map(|s| (s.to.min(hi) - s.from.max(lo)).max(0.0))
            .sum();
        len >= 0.5 * (hi - lo)
    }
}

/// The cell each grid square of a section of column edges `xs` and row edges `ys` (from the
/// top) belongs to: a missing line between two squares joins them, across first, then down
/// where whole cells line up. None when the time limit (`stop`) was reached.
fn cell_owners(
    borders: &Borders,
    xs: &[f32],
    ys: &[f32],
    stop: &dyn Fn() -> bool,
) -> Option<Vec<Vec<(usize, usize)>>> {
    let (cols, rows) = (xs.len() - 1, ys.len() - 1);
    let mut owner: Vec<Vec<(usize, usize)>> = vec![vec![(0, 0); cols]; rows];
    for (r, row) in owner.iter_mut().enumerate() {
        if r % 1024 == 1023 && stop() {
            return None;
        }
        for c in 0..cols {
            row[c] = if c > 0 && !Borders::covered(&borders.on_x[c], ys[r + 1], ys[r]) {
                row[c - 1]
            } else {
                (r, c)
            };
        }
    }
    let span_of = |row: &[(usize, usize)], c: usize| row.iter().filter(|o| **o == row[c]).count();
    for r in 1..rows {
        let mut c = 0;
        while c < cols {
            let span = span_of(&owner[r], c);
            let above = owner[r - 1][c];
            let lined_up =
                owner[r][c] == (r, c) && above.1 == c && span_of(&owner[r - 1], c) == span;
            if lined_up
                && !(c..c + span).any(|k| Borders::covered(&borders.on_y[r], xs[k], xs[k + 1]))
            {
                owner[r][c..c + span].fill(above);
            }
            c += span;
        }
    }
    Some(owner)
}

/// The header of a section of column edges `xs` and row edges `ys` (from the top) with the
/// runs `within`: the grid rows above the first rule under the top border that crosses
/// every column, when rows stand below it; and whether they look it: a style of their own
/// (bold over plain, or another size) in two rows or more. A cell over several rows alone
/// makes no header.
fn header_rows_of<R: LineRun>(
    runs: &[R],
    within: &[usize],
    borders: &Borders,
    xs: &[f32],
    ys: &[f32],
) -> (usize, bool) {
    let (cols, rows) = (xs.len() - 1, ys.len() - 1);
    let head = (1..rows)
        .find(|&k| (0..cols).all(|c| Borders::covered(&borders.on_y[k], xs[c], xs[c + 1])))
        .unwrap_or(0);
    let styled = head >= 2 && {
        let (mut top, mut below): (Vec<&R>, Vec<&R>) = (Vec::new(), Vec::new());
        for &i in within {
            let r = &runs[i];
            let (_, cy) = middle(r);
            let row = ys.partition_point(|e| *e > cy).saturating_sub(1);
            if row < head {
                top.push(r)
            } else {
                below.push(r)
            }
        }
        let median = |runs: &mut Vec<&R>| {
            runs.sort_by(|a, b| a.size().total_cmp(&b.size()));
            runs.get(runs.len() / 2).map_or(0.0, |r| r.size())
        };
        let bold =
            !top.is_empty() && top.iter().all(|r| r.bold()) && below.iter().any(|r| !r.bold());
        let (a, b) = (median(&mut top), median(&mut below));
        !below.is_empty() && (bold || a.max(b) >= LABEL_STEP * a.min(b))
    };
    (head, styled)
}

/// The squares of a section read as a table: its column and row edges (from the top), the
/// cell each grid square belongs to, each cell's extent in squares (rows down and columns
/// across) and its open rows.
struct Squares<'a> {
    xs: &'a [f32],
    ys: &'a [f32],
    owner: Vec<Vec<(usize, usize)>>,
    extent: HashMap<(usize, usize), (u32, u32)>,
    is_open: Vec<bool>,
}

impl<'a> Squares<'a> {
    fn new(
        xs: &'a [f32],
        ys: &'a [f32],
        owner: Vec<Vec<(usize, usize)>>,
        is_open: Vec<bool>,
    ) -> Squares<'a> {
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
        Squares {
            xs,
            ys,
            owner,
            extent,
            is_open,
        }
    }

    fn cols(&self) -> usize {
        self.xs.len() - 1
    }

    /// The grid row at the height `y`.
    fn row_at(&self, y: f32) -> usize {
        self.ys
            .partition_point(|e| *e > y)
            .saturating_sub(1)
            .min(self.ys.len() - 2)
    }

    /// The cell at `(x, y)`.
    fn cell_at(&self, x: f32, y: f32) -> (usize, usize) {
        let c = self
            .xs
            .partition_point(|e| *e < x)
            .saturating_sub(1)
            .min(self.cols() - 1);
        self.owner[self.row_at(y)][c]
    }
}

/// The rows each grid row of a table is cut in: the baselines of their anchor lines, from the
/// top.
struct Bands {
    anchors: Vec<Vec<f32>>,
}

impl Bands {
    /// The rows grid row `r` is cut in.
    fn subs(&self, r: usize) -> usize {
        self.anchors[r].len().max(1)
    }

    /// The row of a band a run goes to: that of the nearest anchor line at or above it.
    fn sub_of<R: LineRun>(&self, r: usize, run: &R) -> usize {
        // The baselines go up: those at or above the run are the last ones.
        let a = &self.anchors[r];
        let tol = 0.3 * run.size().max(1.0);
        let above = a.len() - a.partition_point(|y| *y < run.y() - tol);
        above.saturating_sub(1)
    }
}

/// Why the cells of a table of drawn rules with the runs `within` (in the cells `with_text`
/// of `cells`, cut in the rows of `bands`) are no table, if they are not: most of them are
/// empty, too many of its body span, or too much of its text crosses its lines. `head` is the
/// number of its header rows.
fn grid_refused<R: LineRun>(
    runs: &[R],
    within: &[usize],
    cells: &Squares<'_>,
    bands: &Bands,
    with_text: &HashSet<((usize, usize), usize)>,
    head: usize,
) -> Option<&'static str> {
    let crossing = within
        .iter()
        .filter(|&&i| crosses(&runs[i], cells).is_some())
        .count();
    // Cells counted with the rows each band was cut in; only cells of the body count as
    // spanning, as a header often spans.
    let all: usize = cells.extent.keys().map(|o| bands.subs(o.0)).sum();
    let spanning: usize = cells
        .extent
        .iter()
        .filter(|(o, (rs, cs))| o.0 >= head && rs * cs > 1 && !cells.is_open[o.0])
        .map(|(o, _)| bands.subs(o.0))
        .sum();
    // The fields of an open row are often left empty: its cells do not count here.
    let counted: usize = cells
        .extent
        .keys()
        .filter(|o| !cells.is_open[o.0])
        .map(|o| bands.subs(o.0))
        .sum();
    let empty = counted
        - with_text
            .iter()
            .filter(|(o, _)| !cells.is_open[o.0])
            .count();
    if empty as f32 > EMPTY_SHARE * counted as f32 {
        Some("most of its cells are empty")
    } else if 10 * spanning > 3 * all {
        Some("its boxes do not form rows and columns")
    } else if 5 * crossing > within.len() {
        Some("its text crosses its lines")
    } else {
        None
    }
}

/// The place in the planned rows of each cell of a grid (its owner square) and each row a band
/// was cut in: the row and the cell.
type Places = HashMap<((usize, usize), usize), (usize, usize)>;

/// The rows of a table of `cells` cut in the rows of `bands` (`count` of them), the rows
/// `spanned` one cell across, and the place of each cell and each row a band was cut in
/// among them.
fn plan_grid(
    cells: &Squares<'_>,
    bands: &Bands,
    spanned: &HashSet<(usize, usize)>,
    count: usize,
) -> (Vec<Vec<CellOut>>, Places) {
    let mut place: Places = HashMap::new();
    let mut planned_rows: Vec<Vec<CellOut>> = Vec::with_capacity(count);
    for (r, row) in cells.owner.iter().enumerate() {
        for sub in 0..bands.subs(r) {
            let mut planned = Vec::new();
            if spanned.contains(&(r, sub)) {
                for o in row {
                    place.insert((*o, sub), (planned_rows.len(), 0));
                }
                planned.push(CellOut {
                    runs: Vec::new(),
                    row_span: 1,
                    col_span: cells.cols() as u32,
                    status: None,
                });
                planned_rows.push(planned);
                continue;
            }
            for (c, o) in row.iter().enumerate() {
                if *o != (r, c) {
                    continue;
                }
                let (row_span, col_span) = cells.extent[o];
                place.insert((*o, sub), (planned_rows.len(), planned.len()));
                planned.push(CellOut {
                    runs: Vec::new(),
                    row_span,
                    col_span,
                    status: None,
                });
            }
            planned_rows.push(planned);
        }
    }
    (planned_rows, place)
}

/// For each open row (its top one of the feet `open`), the lines of `within` in it that
/// start a row, as `anchors`; a row with fewer than two such lines is left as it is.
fn open_anchors<R: LineRun>(
    runs: &[R],
    within: &[usize],
    (xs, ys): (&[f32], &[f32]),
    open: &[f32],
    row_at: &dyn Fn(f32) -> usize,
    anchors: &mut [Vec<f32>],
) {
    if open.is_empty() {
        return;
    }
    let mut by_row: HashMap<usize, Vec<usize>> = HashMap::new();
    for &i in within {
        let r = row_at(middle(&runs[i]).1);
        if open.iter().any(|f| (ys[r] - f).abs() <= SNAP) {
            by_row.entry(r).or_default().push(i);
        }
    }
    for (r, idx) in by_row {
        let lines = open_rows(runs, &idx, xs);
        if lines.len() >= 2 {
            anchors[r] = lines;
        }
    }
}

/// The baselines, going up, of the lines that start a row in an open row (lines without
/// rules under a header of hanging rules) that holds the runs `idx`, in lines by baseline, by
/// the rule of a body under a header band ([`row_starts`]) in the columns of `xs`. A note
/// across the columns (see [`note_across`]) is a row of its own; a line that goes on with it
/// (the only run on its line, in its style, one line pitch below) joins it, and the line after
/// the note starts a row.
fn open_rows<R: LineRun>(runs: &[R], idx: &[usize], xs: &[f32]) -> Vec<f32> {
    let cols = xs.len() - 1;
    let mut placed: Vec<(usize, usize)> = idx
        .iter()
        .map(|&i| {
            let (cx, _) = middle(&runs[i]);
            let c = xs
                .partition_point(|e| *e < cx)
                .saturating_sub(1)
                .min(cols - 1);
            (i, c)
        })
        .collect();
    placed.sort_by(|a, b| runs[b.0].y().total_cmp(&runs[a.0].y()));
    // Lines from the top: each its baseline, the text of its columns and its runs.
    struct Line {
        y: f32,
        cells: Vec<(usize, String)>,
        runs: Vec<usize>,
    }
    let mut lines: Vec<Line> = Vec::new();
    for (i, c) in placed {
        let r = &runs[i];
        if lines
            .last()
            .is_none_or(|l| l.y - r.y() > 0.3 * r.size().max(1.0))
        {
            lines.push(Line {
                y: r.y(),
                cells: Vec::new(),
                runs: Vec::new(),
            });
        }
        let Some(line) = lines.last_mut() else {
            continue;
        };
        line.runs.push(i);
        match line.cells.iter_mut().find(|p| p.0 == c) {
            Some(p) => p.1.push_str(r.text()),
            None => line.cells.push((c, r.text().to_string())),
        }
    }
    let mut used: Vec<usize> = lines
        .iter()
        .flat_map(|l| l.cells.iter().map(|p| p.0))
        .collect();
    used.sort_unstable();
    used.dedup();
    let pieces: Vec<Vec<(usize, bool)>> = lines
        .iter()
        .map(|l| l.cells.iter().map(|(c, t)| (*c, numeric(t))).collect())
        .collect();
    let mut starting = row_starts(&pieces, used.len());
    // The run of a line that is its only run.
    let only = |l: &Line| match l.runs.as_slice() {
        [i] => Some(&runs[*i]),
        _ => None,
    };
    let mut in_note: Option<&R> = None;
    for (k, line) in lines.iter().enumerate() {
        let run = only(line);
        let goes_on = match (in_note, run) {
            (Some(note), Some(r)) => goes_on_note(note, r),
            _ => false,
        };
        if goes_on {
            starting[k] = false;
            in_note = run;
        } else if let Some(r) = run.filter(|r| note_across(*r, xs)) {
            starting[k] = true;
            in_note = Some(r);
        } else {
            if in_note.is_some() {
                starting[k] = true;
            }
            in_note = None;
        }
    }
    // Kept going up, for a search by baseline.
    let mut starts: Vec<f32> = lines
        .iter()
        .zip(starting)
        .filter(|(_, starts)| *starts)
        .map(|(l, _)| l.y)
        .collect();
    starts.reverse();
    starts
}

/// Opens the bands of the body below the header that one cell fills from side to side: when
/// at least two of a band's lines hold text in two of the header's columns or more, and at
/// most a quarter of its pieces of text reach over a column's side, each of its squares is a
/// cell of its own.
fn open_bands<R: LineRun>(
    runs: &[R],
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
        let run = &runs[i];
        let (_, cy) = middle(run);
        let r = ys
            .partition_point(|e| *e > cy)
            .saturating_sub(1)
            .min(rows - 1);
        if whole[r] {
            bands[r].push((run.y(), run.size().max(1.0), run.x(), run.x() + run.w()));
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

/// For each grid row of `cells` (its `is_open` says which are rows under a header of hanging
/// rules), the baselines of the lines that start a row in it, going up: empty when the band
/// is one row. A band of the body whose cells are each one square and of which at least two
/// hold text is cut when at least two of its lines are anchors: lines with text in at least
/// half of the cells that hold text in the band (an empty first cell included). No band is
/// cut when the rules part most bands into one record each.
fn anchor_rows<R: LineRun>(
    runs: &[R],
    within: &[usize],
    head: usize,
    cells: &Squares<'_>,
    row_at: &dyn Fn(f32) -> usize,
    cell_at: &dyn Fn(f32, f32) -> (usize, usize),
) -> Vec<Vec<f32>> {
    let (is_open, owner, extent) = (&cells.is_open, &cells.owner, &cells.extent);
    let rows = is_open.len();
    let mut out: Vec<Vec<f32>> = vec![Vec::new(); rows];
    // Bands of the body with text in two cells or more, and of those the ones with two lines
    // or more that fill the band and start in its first cell.
    let (mut texted, mut records) = (0usize, 0usize);
    // The runs of each band, by baseline from the top.
    let mut bands: Vec<Vec<(f32, f32, usize)>> = vec![Vec::new(); rows];
    for &i in within {
        let r = &runs[i];
        let (cx, cy) = middle(r);
        let row = row_at(cy);
        let o = cell_at(cx, cy);
        if row >= head && o.0 == row {
            bands[row].push((r.y(), r.size().max(1.0), o.1));
        }
    }
    for (r, band) in bands.iter_mut().enumerate() {
        let one_high = owner[r]
            .iter()
            .all(|o| o.0 == r && extent.get(o) == Some(&(1, 1)));
        // An open row is cut by its own lines (see [`open_anchors`]).
        if band.len() < 2 || !one_high || is_open[r] {
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
