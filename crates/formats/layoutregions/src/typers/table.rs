// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typers of tables and of the totals under them.
//!
//! The table typer reads a segment with the one rule that builds cells, as the text typers
//! read the walk of the text: [`ruled_cells`] for a grid of drawn rules, [`band_cells`] for a
//! row of drawn cells and the lines under it in its columns, [`grid_cells`] for lines in
//! columns without rules. A reading the rule accepts is the table's evidence. A refused reading
//! gives the table nothing, and no weight is taken away anywhere: the reason is evidence for
//! the kind it speaks for (see [`refusal`]): prose for a paragraph, a caption, a picture or
//! tiny text for a figure, a page frame or a form for the spatial fallback, labels and values
//! or totals for theirs. Where today's rules read a place one way first, that reading refuses
//! the other: lines [`key_values`] reads, two lines of titles over values (see
//! [`titles_over_values`]), a grid whose cells all read as fields and lines read as totals
//! under a table are no table's.
//!
//! The totals typer looks for the table right above a segment among the segments beside the
//! segment's path to the root, and asks whether every line of the segment holds an amount
//! under a money column of that table.

use crate::extract::pairs::blocks::{KeyValues, key_values};
use crate::extract::table::area::{MANY_COLUMNS, MAX_COLUMNS, ON_PICTURE, PAGE_FRAME, TINY_TEXT};
use crate::extract::table::banded::{
    BAND_WORK, BandCells, BodyBelow, band_cells, band_head_refused, band_titles, body_below,
    long_or_prose,
};
use crate::extract::table::caption::{FIGURE_NEAR, caption, captions_near};
use crate::extract::table::grid::{
    Piece, Shape, amount, grid_cells, pieces_of, set_full, titles_over_values,
};
use crate::extract::table::ruled::{
    RuledIn, boxed_field, edges, hanging_rows, ruled_cells, sentence_across,
};
use crate::extract::table::sections::sections;
use crate::extract::table::{
    CONTENTS, CROSSES_LINES, Cells, EMPTY_CELLS, LONG_TEXT, MAX_SQUARES, NO_ROWS_AND_COLUMNS,
    ROWS_ALIKE, RUNS_ON, RunRange, SHORT_CELL, TableOut, header_of, middle, run_ranges,
};
use crate::input::{PrimId, TextRun};
use crate::line::LineRun;
use crate::region::{Bounds, Kind, Signal};
use crate::segment::drawn::{Grids, Line, SNAP, grid_frame};
use crate::segment::{Segment, Source};
use crate::typers::{PageView, Typed, seg_runs, signal};

/// `table.drawn_cells`, `table.header_band` and `table.columns`: the rule that builds the
/// cells of a grid of drawn rules, of a header band over lines, or of lines in columns without
/// rules reads the segment as a table.
pub const TABLE_READING: u16 = 600;
/// `table.shared_columns`: three rows or more share the table's column edges.
pub const TABLE_SHARED: u16 = 100;
/// `table.header_row`: the table has a header row (drawn, in a style of its own, or of titles
/// over numbers).
pub const TABLE_HEADER: u16 = 100;
/// `paragraph.prose_cells`: the text the table rules refuse as prose (long text, text that
/// runs on like prose, a paragraph set full in pieces, a sentence across a ruled row).
pub const PARAGRAPH_PROSE_CELLS: u16 = 850;
/// `totals.under_money_columns`: every line holds an amount under a money column of the table
/// right above.
pub const TOTALS_UNDER: u16 = 900;
/// `totals.labels_in_description`: each of those lines has its label in the table's
/// description column.
pub const TOTALS_LABELS: u16 = 100;
/// `totals.end_rule`: a rule across the money columns stands between the table and them.
pub const TOTALS_RULE: u16 = 50;

/// Why a reading is refused because a paragraph set full (justified) stands in pieces.
const SET_FULL: &str = "its lines are a paragraph set full";
/// Why a grid of digit boxes is no table.
const COMB: &str = "its boxes hold one digit each, a field to fill in";

/// How far below a table its totals may stand: this many sizes of their text.
const TOTALS_GAP: f32 = 3.0;

/// The columns a table reading finds: its column edges left to right (one more than its
/// columns), the primitives of its header cells by column, its rows and header rows. Task 19
/// compares them with the cells a tagged table gives.
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumns {
    pub edges: Vec<f32>,
    pub header: Vec<Vec<PrimId>>,
    pub rows: usize,
    pub header_rows: usize,
}

/// Which rule read the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum How {
    Ruled,
    Band,
    Lines,
}

/// A refused reading: the kind its reason speaks for, as a named signal. A refusal for a
/// paragraph carries the paragraph's weight; for a figure or the spatial fallback it carries
/// none (those kinds weigh it themselves).
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub kind: Kind,
    pub signal: Signal,
}

/// A table reading of a segment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TableRead {
    pub how: How,
    pub columns: TableColumns,
    /// The cell texts of its body rows (after the header), by column.
    pub body: Vec<Vec<String>>,
    /// Its box (x0, y0, x1, y1): its grid's frame, or the box of its cells' runs.
    pub bounds: [f32; 4],
    /// Lines of it that read as a sentence across its columns (see [`sentence_across`]).
    pub sentences: usize,
}

/// What the table rules read in a segment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Find {
    Table(Box<TableRead>),
    Refused(Refusal),
    Nothing,
}

impl Find {
    /// The reading of `seg` by the rule its source asks for (see the module doc), before
    /// totals are looked at.
    pub(crate) fn of(view: &PageView<'_>, seg: &Segment) -> Find {
        match seg.source {
            Source::Grid { rows, cols } if rows >= 2 && cols >= 2 => ruled(view, seg),
            Source::Grid { .. } | Source::Drawing => Find::Nothing,
            Source::Given(_) => lines(view, seg, true),
            _ => {
                let drawn = |c: &Segment| matches!(c.source, Source::Grid { .. } | Source::Box);
                let kids: Vec<&Segment> = seg
                    .children
                    .iter()
                    .filter_map(|c| view.segs.all.get(c.0 as usize))
                    .collect();
                if let Some(band) = kids
                    .iter()
                    .find(|c| matches!(c.source, Source::Grid { rows: 1, cols } if cols >= 2))
                {
                    return banded(view, seg, band);
                }
                if kids.iter().any(|c| drawn(c)) {
                    return Find::Nothing;
                }
                lines(view, seg, false)
            }
        }
    }
}

fn refused(kind: Kind, why: &'static str) -> Find {
    let (id, weight) = match (kind, why) {
        (Kind::Paragraph, _) => ("paragraph.prose_cells", PARAGRAPH_PROSE_CELLS),
        (Kind::Figure, ON_PICTURE) => ("figure.on_picture", 0),
        (Kind::Figure, TINY_TEXT) => ("figure.page_picture", 0),
        (Kind::Figure, _) => ("figure.caption", 0),
        (Kind::Spatial, PAGE_FRAME) => ("spatial.page_frame", 0),
        (Kind::Spatial, MANY_COLUMNS) => ("spatial.many_columns", 0),
        (Kind::Spatial, EMPTY_CELLS) => ("spatial.empty_cells", 0),
        (Kind::Spatial, COMB) => ("spatial.comb_boxes", 0),
        (Kind::Spatial, CONTENTS) => ("spatial.contents", 0),
        (Kind::Spatial, ROWS_ALIKE) => ("spatial.rows_alike", 0),
        (Kind::Spatial, _) => ("spatial.no_rows_and_columns", 0),
        (Kind::LabelValue, _) => ("label_value.refuses_table", 0),
        (Kind::Totals, _) => ("totals.refuses_table", 0),
        _ => ("spatial.no_rows_and_columns", 0),
    };
    Find::Refused(Refusal {
        kind,
        signal: signal(id, why.to_string(), weight),
    })
}

/// The kind a reason of a table rule speaks for; `None` for a reason that only says the
/// evidence of a table is missing (too few rows, no columns, columns not clean).
fn speaks_for(why: &'static str) -> Option<Kind> {
    match why {
        LONG_TEXT | RUNS_ON | SET_FULL => Some(Kind::Paragraph),
        ON_PICTURE | TINY_TEXT | FIGURE_NEAR => Some(Kind::Figure),
        PAGE_FRAME | MANY_COLUMNS | EMPTY_CELLS | NO_ROWS_AND_COLUMNS | CROSSES_LINES | COMB
        | ROWS_ALIKE | CONTENTS => Some(Kind::Spatial),
        _ => None,
    }
}

fn refused_for(why: &'static str) -> Find {
    match speaks_for(why) {
        Some(kind) => refused(kind, why),
        None => Find::Nothing,
    }
}

/// The group of `g` whose frame is `b` (within [`SNAP`]).
fn group_at<'g>(g: &'g Grids, b: &Bounds) -> Option<&'g (Vec<usize>, Vec<usize>)> {
    let near = |a: f32, c: f32| (a - c).abs() <= SNAP;
    g.groups.iter().find(|grp| {
        grid_frame(g, grp).is_some_and(|(f, _, _)| {
            near(f.x0, b.x0) && near(f.x1, b.x1) && near(f.y0, b.y0) && near(f.y1, b.y1)
        })
    })
}

/// The lines of the group `grp` of `g`: horizontal and vertical.
fn group_lines(g: &Grids, grp: &(Vec<usize>, Vec<usize>)) -> (Vec<Line>, Vec<Line>) {
    (
        grp.0.iter().map(|&j| g.hs[j]).collect(),
        grp.1.iter().map(|&k| g.vs[k]).collect(),
    )
}

/// The median of `sizes`, 0 for none.
fn median(mut sizes: Vec<f32>) -> f32 {
    sizes.sort_by(f32::total_cmp);
    sizes.get(sizes.len() / 2).copied().unwrap_or(0.0)
}

/// A figure caption next to the box `b` (lines of the page whose runs `skip` names left
/// out) or in the runs `within`: its refusal.
fn figure_near<R: LineRun>(
    view: &PageView<'_>,
    b: [f32; 4],
    sizes: &[f32],
    skip: &dyn Fn(usize) -> bool,
    within: impl Iterator<Item = R>,
) -> (bool, bool) {
    let (table, figure) =
        captions_near(&view.page.texts, view.lines(), view.by_y(), b, sizes, skip);
    let inside = within.into_iter().any(|r| caption(r.text()) == Some(false));
    (table.is_some(), figure || inside)
}

/// The reading of a Grid segment: each section of its grid (see [`sections`]) of two rows
/// and two columns or more is read by [`ruled_cells`] after what the area says (digit boxes,
/// a figure caption, more than [`MAX_COLUMNS`] columns, the page frame, a picture, tiny text);
/// the section that reads as a table with most rows is the reading, else the first refusal.
fn ruled(view: &PageView<'_>, seg: &Segment) -> Find {
    let Ok(g) = &view.drawn.grids else {
        return Find::Nothing;
    };
    let Some(grp) = group_at(g, &seg.bounds) else {
        return Find::Nothing;
    };
    let (h, v) = group_lines(g, grp);
    let (refs, ids) = seg_runs(view, seg);
    let mut work = 0usize;
    let parts = sections(&h, &v, &g.hs, &mut work);
    let cut = parts.len() > 1;
    let mut best: Option<TableRead> = None;
    let mut first_refusal: Option<Find> = None;
    let area = view.area();
    for (mut h, v) in parts {
        let (xs, mut ys) = edges(&h, &v);
        if xs.len() < 2 || ys.len() < 2 {
            continue;
        }
        let holds_text = |bottom: f32, foot: f32| {
            refs.iter().any(|r| {
                let (cx, cy) = middle(r);
                !r.off_axis
                    && cy > bottom
                    && cy < foot
                    && cx > xs[0]
                    && cx < xs[xs.len() - 1]
                    && !r.text.trim().is_empty()
            })
        };
        let open = hanging_rows(&v, &xs, (&mut ys, &mut h), cut, &holds_text);
        let (cols, rows) = (xs.len() - 1, ys.len() - 1);
        if cols < 2 || rows < 2 || rows.saturating_mul(cols) > MAX_SQUARES {
            continue;
        }
        let b = [xs[0], ys[rows], xs[cols], ys[0]];
        let mine: Vec<usize> = (0..refs.len())
            .filter(|&k| {
                let r = refs[k];
                let (cx, cy) = middle(r);
                !r.off_axis && cx > b[0] && cx < b[2] && cy > b[1] && cy < b[3]
            })
            .collect();
        let within: Vec<usize> = mine
            .iter()
            .copied()
            .filter(|&k| !refs[k].text.trim().is_empty())
            .collect();
        crate::work::add((mine.len() + rows * cols) as u64);
        if within.is_empty() {
            continue;
        }
        let mut refuse = |f: Find| {
            first_refusal.get_or_insert(f);
        };
        if boxed_field(&refs, &within, &xs, &ys) {
            refuse(refused(Kind::Spatial, COMB));
            continue;
        }
        let sizes: Vec<f32> = within.iter().map(|&k| refs[k].size).collect();
        let inside = |i: usize| {
            let (cx, cy) = middle(&view.page.texts[i]);
            cx > b[0] && cx < b[2] && cy > b[1] && cy < b[3]
        };
        let (_, figure) = figure_near(view, b, &sizes, &inside, within.iter().map(|&k| refs[k]));
        if figure {
            refuse(refused(Kind::Figure, FIGURE_NEAR));
            continue;
        }
        if cols > MAX_COLUMNS {
            refuse(refused(Kind::Spatial, MANY_COLUMNS));
            continue;
        }
        let cells = ruled_cells(
            &refs,
            &RuledIn {
                h: &h,
                v: &v,
                xs: &xs,
                ys: &ys,
                open: &open,
                mine: &mine,
                within: &within,
                per_line: view.per_line(),
            },
            &|| false,
            &|| area.refuses(b, &sizes),
            &mut |_| false,
        );
        match cells {
            Cells::Table(t) => {
                let read = read_of(&refs, &|k| ids[k], &t, xs.clone(), How::Ruled);
                let texts = cell_texts(&refs, &t);
                if let Some(why) = long_or_prose(&texts, SHORT_CELL) {
                    refuse(refused(Kind::Paragraph, why));
                    continue;
                }
                let sentences = sentence_lines(&refs, &within, &xs);
                // A grid's own frame bounds it.
                let read = TableRead {
                    sentences,
                    bounds: b,
                    ..read
                };
                if best
                    .as_ref()
                    .is_none_or(|r| r.columns.rows < read.columns.rows)
                {
                    best = Some(read);
                }
            }
            Cells::Refused(why) => refuse(refused_for(why)),
            Cells::TooLarge { .. } | Cells::Stopped => {}
        }
    }
    match (best, first_refusal) {
        (Some(t), _) => Find::Table(Box::new(t)),
        (None, Some(r)) => r,
        (None, None) => Find::Nothing,
    }
}

/// How many lines of the runs `within` of `runs` read as a sentence across the columns `xs`.
fn sentence_lines(runs: &[&TextRun], within: &[usize], xs: &[f32]) -> usize {
    let mut by_line: Vec<(u32, f32, usize)> = within
        .iter()
        .map(|&k| (runs[k].line, runs[k].x, k))
        .collect();
    by_line.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    by_line
        .chunk_by(|a, b| a.0 == b.0)
        .filter(|l| {
            let line: Vec<usize> = l.iter().map(|x| x.2).collect();
            line.len() > 1 && sentence_across(runs, &line, xs)
        })
        .count()
}

/// The text of each cell of `t`, row by row, its runs in their order.
fn cell_texts(runs: &[&TextRun], t: &TableOut) -> Vec<Vec<String>> {
    t.rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    let mut ids = c.runs.clone();
                    ids.sort_unstable();
                    crate::extract::pairs::words(runs, &ids)
                })
                .collect()
        })
        .collect()
}

/// The reading of the table `t` of `runs` (`prim` gives a run's primitive id), its column
/// edges `edges` (none: from its cells' runs).
fn read_of<R: LineRun>(
    runs: &[R],
    prim: &dyn Fn(usize) -> u32,
    t: &TableOut,
    edges: Vec<f32>,
    how: How,
) -> TableRead {
    // The column of each cell: the spans of the cells before it in its row.
    let placed: Vec<Vec<(usize, &crate::extract::table::CellOut)>> = t
        .rows
        .iter()
        .map(|row| {
            let mut c = 0usize;
            row.iter()
                .map(|cell| {
                    let at = c;
                    c += cell.col_span.max(1) as usize;
                    (at, cell)
                })
                .collect()
        })
        .collect();
    let cols = if edges.len() >= 2 {
        edges.len() - 1
    } else {
        placed
            .iter()
            .map(|r| r.last().map_or(0, |(c, x)| c + x.col_span.max(1) as usize))
            .max()
            .unwrap_or(0)
    };
    let edges = if edges.len() >= 2 {
        edges
    } else {
        cell_edges(runs, &placed, cols)
    };
    let header_rows = if t.header_rows > 0 {
        t.header_rows
    } else {
        let rows: Vec<Vec<Vec<&R>>> = t
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|c| {
                        let mut ids = c.runs.clone();
                        ids.sort_unstable();
                        ids.iter().map(|&i| &runs[i]).collect()
                    })
                    .collect()
            })
            .collect();
        let slices: Vec<Vec<&[&R]>> = rows
            .iter()
            .map(|r| r.iter().map(Vec::as_slice).collect())
            .collect();
        header_of(&slices)
    };
    let mut header: Vec<Vec<PrimId>> = vec![Vec::new(); cols];
    for row in placed.iter().take(header_rows) {
        for (c, cell) in row {
            if let Some(h) = header.get_mut(*c) {
                let mut ids: Vec<usize> = cell.runs.clone();
                ids.sort_unstable();
                h.extend(ids.iter().map(|&i| PrimId::Text(prim(i))));
            }
        }
    }
    let mut body: Vec<Vec<String>> = Vec::new();
    for row in placed.iter().skip(header_rows) {
        let mut texts = vec![String::new(); cols];
        for (c, cell) in row {
            if let Some(t) = texts.get_mut(*c) {
                let mut ids = cell.runs.clone();
                ids.sort_unstable();
                *t = crate::extract::pairs::words(runs, &ids);
            }
        }
        body.push(texts);
    }
    let all = t.rows.iter().flatten().flat_map(|c| c.runs.iter().copied());
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for i in all {
        let r = &runs[i];
        bounds[0] = bounds[0].min(r.x());
        bounds[1] = bounds[1].min(r.y() - 0.25 * r.size());
        bounds[2] = bounds[2].max(r.x() + r.w());
        bounds[3] = bounds[3].max(r.y() + r.size());
    }
    TableRead {
        how,
        columns: TableColumns {
            edges,
            header,
            rows: t.rows.len(),
            header_rows,
        },
        body,
        bounds,
        sentences: 0,
    }
}

/// Column edges from the runs of the cells of one column each: the left end of the first, the
/// middle of each gutter, the right end of the last.
fn cell_edges<R: LineRun>(
    runs: &[R],
    placed: &[Vec<(usize, &crate::extract::table::CellOut)>],
    cols: usize,
) -> Vec<f32> {
    let mut spans = vec![(f32::MAX, f32::MIN); cols];
    for (c, cell) in placed.iter().flatten() {
        if cell.col_span > 1 {
            continue;
        }
        if let Some(s) = spans.get_mut(*c) {
            for &i in &cell.runs {
                s.0 = s.0.min(runs[i].x());
                s.1 = s.1.max(runs[i].x() + runs[i].w());
            }
        }
    }
    let mut out = Vec::with_capacity(cols + 1);
    for c in 0..cols {
        if c == 0 {
            out.push(spans[0].0);
        } else {
            out.push((spans[c - 1].1 + spans[c].0) / 2.0);
        }
    }
    if let Some(last) = spans.last() {
        out.push(last.1);
    }
    out
}

/// The reading of a segment that holds a row of drawn cells `band` with lines under it in its
/// columns (see [`band_cells`]).
fn banded(view: &PageView<'_>, seg: &Segment, band: &Segment) -> Find {
    let Ok(g) = &view.drawn.grids else {
        return Find::Nothing;
    };
    let Some(grp) = group_at(g, &band.bounds) else {
        return Find::Nothing;
    };
    let (h, v) = group_lines(g, grp);
    let (xs, ys) = edges(&h, &v);
    if xs.len() < 3 || ys.len() < 2 {
        return Find::Nothing;
    }
    let b = [xs[0], ys[ys.len() - 1], xs[xs.len() - 1], ys[0]];
    let texts = &view.page.texts;
    let mut own: Vec<usize> = seg
        .prims
        .iter()
        .filter_map(|p| match p {
            PrimId::Text(i) => Some(*i as usize),
            _ => None,
        })
        .collect();
    own.sort_unstable();
    let mine = |i: usize| own.binary_search(&i).is_ok();
    let head: Vec<usize> = own
        .iter()
        .copied()
        .filter(|&i| {
            let r = &texts[i];
            let (cx, cy) = middle(r);
            !r.off_axis && cx > b[0] && cx < b[2] && cy > b[1] && cy < b[3]
        })
        .collect();
    let titles = band_titles(texts, &head, &xs);
    if band_head_refused(texts, &head, &titles, b).is_some() {
        return Find::Nothing;
    }
    let below = BodyBelow {
        lines: view.lines(),
        by_y: view.by_y(),
        hs: view.hs(),
        floor: seg.bounds.y0 - SNAP,
        bound: BAND_WORK,
    };
    let free = |i: usize| mine(i) && !texts[i].off_axis;
    let mut work = 0usize;
    let Some((body, misfit)) = body_below(texts, &below, b, &xs, &free, &mut work) else {
        return Find::Nothing;
    };
    crate::work::add(work as u64);
    match band_cells(texts, &head, &titles, &xs, (&body, misfit)) {
        BandCells::Table(t) => {
            let bottom = body.last().map_or(b[1], |l| l.0 - 0.25 * l.1);
            let sizes: Vec<f32> = body.iter().map(|l| l.1).collect();
            if let Some(why) = view.area().refuses([b[0], bottom, b[2], b[3]], &sizes) {
                return refused_for(why);
            }
            Find::Table(Box::new(read_of(
                texts,
                &|i| i as u32,
                &t,
                Vec::new(),
                How::Band,
            )))
        }
        BandCells::Refused(why) => refused_for(why),
        BandCells::TooLarge { .. } => Find::Nothing,
    }
}

/// The reading of the lines of a segment without drawn parts (see [`grid_cells`]), after the
/// rules that read them otherwise first: a paragraph set full, labels and values, titles over
/// one row of values; then a figure caption next to them and what the area says. `given`: a
/// region the file states, read by geometry alone (none of those rules).
fn lines(view: &PageView<'_>, seg: &Segment, given: bool) -> Find {
    let (refs, ids) = seg_runs(view, seg);
    let inked: Vec<usize> = (0..refs.len())
        .filter(|&k| !refs[k].text.trim().is_empty() && !refs[k].off_axis)
        .collect();
    if inked.len() < 4 {
        return Find::Nothing;
    }
    let runs: Vec<&TextRun> = inked.iter().map(|&k| refs[k]).collect();
    let ranges: Vec<RunRange> = run_ranges(&runs);
    let pieces: Vec<Vec<Piece>> = ranges
        .iter()
        .map(|l| pieces_of(&runs, l.start, l.end))
        .collect();
    crate::work::add((runs.len() + ranges.len()) as u64);
    if pieces.iter().filter(|p| p.len() >= 2).count() < 2 {
        return Find::Nothing;
    }
    let prim = |k: usize| ids[inked[k]];
    if !given {
        if set_full(&runs, &pieces, &ranges) {
            return refused(Kind::Paragraph, SET_FULL);
        }
        let off = vec![false; ranges.len()];
        if let KeyValues::Rows(_) = key_values(&runs, &ranges, 0, ranges.len(), &pieces, &off) {
            return refused(Kind::LabelValue, "its lines are labels and values");
        }
        if titles_over_values(&runs, &ranges, &pieces) {
            return refused(Kind::LabelValue, "its lines are titles over their values");
        }
    }
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for r in &runs {
        b[0] = b[0].min(r.x);
        b[1] = b[1].min(r.y - 0.25 * r.size);
        b[2] = b[2].max(r.x + r.w);
        b[3] = b[3].max(r.y + r.size);
    }
    let sizes: Vec<f32> = ranges.iter().map(|l| l.size).collect();
    let (table_caption, figure) = if given {
        (false, false)
    } else {
        let mut own: Vec<u32> = ids.clone();
        own.sort_unstable();
        let skip = |i: usize| own.binary_search(&(i as u32)).is_ok();
        figure_near(view, b, &sizes, &skip, runs.iter().copied())
    };
    if figure {
        return refused(Kind::Figure, FIGURE_NEAR);
    }
    let rows_needed = if table_caption { 2 } else { 3 };
    match grid_cells(&runs, &pieces, rows_needed, table_caption) {
        Ok(t) => {
            if !given && let Some(why) = view.area().refuses(b, &sizes) {
                return refused_for(why);
            }
            Find::Table(Box::new(read_of(&runs, &prim, &t, Vec::new(), How::Lines)))
        }
        Err(why) if given => {
            let _ = why;
            Find::Nothing
        }
        Err(why) => refused_for(why),
    }
}

/// The reading of `seg` with totals looked at: lines that read as totals under a table above
/// are no table of their own.
fn find<U>(view: &PageView<'_>, seg: &Segment, f: impl FnOnce(&Find) -> U) -> U {
    let totals = view.totals_of(seg, |t| t.is_some());
    if totals {
        return f(&refused(
            Kind::Totals,
            "its lines are totals under the table above",
        ));
    }
    view.reading(seg, f)
}

/// Why the table rules refuse `seg`, as evidence for the kind the reason speaks for: a
/// paragraph's carries its weight (see [`PARAGRAPH_PROSE_CELLS`]); a figure's (`figure.*`) and
/// the spatial fallback's (`spatial.*`) carry none, for those kinds to weigh; labels and values
/// and totals that refuse a table are evidence their own typers weigh.
pub fn refusal(view: &PageView<'_>, seg: &Segment) -> Option<Refusal> {
    find(view, seg, |f| match f {
        Find::Refused(r) => Some(r.clone()),
        _ => None,
    })
}

/// The columns the table rules read in `seg`, by geometry alone for a region the file states
/// (a tagged table), so its cells can be checked against them.
pub fn table_columns(view: &PageView<'_>, seg: &Segment) -> Option<TableColumns> {
    view.reading(seg, |f| match f {
        Find::Table(t) => Some(t.columns.clone()),
        _ => None,
    })
}

/// Whether every line with ink of `seg` reads as a sentence across the columns of a ruled
/// table of the page that holds it (see [`sentence_across`]).
pub(crate) fn sentences_across(view: &PageView<'_>, seg: &Segment) -> bool {
    if matches!(seg.source, Source::Grid { .. }) {
        return false;
    }
    let (refs, _) = seg_runs(view, seg);
    let mut runs: Vec<(u32, f32, usize)> = (0..refs.len())
        .filter(|&k| !refs[k].text.trim().is_empty())
        .map(|k| (refs[k].line, refs[k].x, k))
        .collect();
    if runs.is_empty() {
        return false;
    }
    runs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let grids: Vec<&Segment> = view
        .segs
        .all
        .iter()
        .filter(|s| matches!(s.source, Source::Grid { rows, cols } if rows >= 2 && cols >= 2))
        .collect();
    runs.chunk_by(|a, b| a.0 == b.0).all(|l| {
        let line: Vec<usize> = l.iter().map(|x| x.2).collect();
        let r = refs[line[0]];
        let (_, cy) = middle(r);
        let (x0, x1) = (
            r.x,
            refs[line[line.len() - 1]].x + refs[line[line.len() - 1]].w,
        );
        grids.iter().any(|g| {
            let b = &g.bounds;
            cy > b.y0
                && cy < b.y1
                && x0 < b.x1
                && x1 > b.x0
                && view.reading(g, |f| match f {
                    Find::Table(t) if t.how == How::Ruled => {
                        sentence_across(&refs, &line, &t.columns.edges)
                    }
                    _ => false,
                })
        })
    })
}

/// Table: the rule for its source reads the segment as a table (`table.drawn_cells`,
/// `table.header_band` or `table.columns`); three rows or more share its column edges
/// (`table.shared_columns`); it has a header row (`table.header_row`). A region the file
/// states is decided as given: no candidate.
pub fn type_table(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    if matches!(seg.source, Source::Given(_)) {
        return None;
    }
    find(view, seg, |f| {
        let Find::Table(t) = f else { return None };
        let cols = t.columns.edges.len().saturating_sub(1);
        let id = match t.how {
            How::Ruled => "table.drawn_cells",
            How::Band => "table.header_band",
            How::Lines => "table.columns",
        };
        let mut detail = format!("{} rows by {cols} columns", t.columns.rows);
        if t.sentences > 0 {
            detail.push_str(&format!(
                "; {} lines read as a sentence across its columns",
                t.sentences
            ));
        }
        let mut signals = vec![signal(id, detail, TABLE_READING)];
        if t.columns.rows >= 3 {
            signals.push(signal(
                "table.shared_columns",
                format!("{} rows share its column edges", t.columns.rows),
                TABLE_SHARED,
            ));
        }
        if t.columns.header_rows > 0 {
            signals.push(signal(
                "table.header_row",
                format!("{} header rows", t.columns.header_rows),
                TABLE_HEADER,
            ));
        }
        Typed::of(crate::region::Kind::Table, signals)
    })
}

/// Lines read as totals under a table: their count, whether each label stands in the table's
/// description column, and whether a rule across its money columns parts them from it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TotalsRead {
    lines: usize,
    labels: bool,
    rule: bool,
}

impl TotalsRead {
    /// The totals reading of `seg`: lines without drawn parts, each with an amount, under the
    /// nearest table above among the segments beside its path to the root (at most
    /// [`TOTALS_GAP`] sizes of their text below it), every amount under a money column of it (a
    /// column most of whose body cells are amounts).
    pub(crate) fn of(view: &PageView<'_>, seg: &Segment) -> Option<TotalsRead> {
        if !matches!(
            seg.source,
            Source::Gutter | Source::Band | Source::Rule | Source::Style | Source::Rest
        ) || !seg.children.is_empty()
        {
            return None;
        }
        let (refs, _) = seg_runs(view, seg);
        let runs: Vec<&TextRun> = refs
            .into_iter()
            .filter(|r| !r.text.trim().is_empty() && !r.off_axis)
            .collect();
        if runs.is_empty() {
            return None;
        }
        let ranges = run_ranges(&runs);
        let pieces: Vec<Vec<Piece>> = ranges
            .iter()
            .map(|l| pieces_of(&runs, l.start, l.end))
            .collect();
        let text = |p: &Piece| crate::extract::table::grid::text_of(&runs, p.start, p.end);
        let size = median(ranges.iter().map(|l| l.size).collect());
        let top = runs.iter().map(|r| r.y + r.size).fold(f32::MIN, f32::max);
        let left = runs.iter().map(|r| r.x).fold(f32::MAX, f32::min);
        let right = runs.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max);
        // Every line holds an amount.
        if !pieces.iter().all(|l| l.iter().any(|p| amount(&text(p)))) {
            return None;
        }
        crate::work::add((runs.len() + ranges.len()) as u64);
        // The nearest table above, among the segments beside the path to the root.
        let mut best: Option<(f32, Box<TableRead>)> = None;
        let (mut child, mut parent) = (seg.id, seg.parent);
        let own = view
            .segs
            .all
            .get(seg.id.0 as usize)
            .is_some_and(|s| std::ptr::eq(s, seg));
        while own && let Some(p) = parent {
            let Some(ps) = view.segs.all.get(p.0 as usize) else {
                break;
            };
            for c in &ps.children {
                if *c == child {
                    continue;
                }
                let Some(cs) = view.segs.all.get(c.0 as usize) else {
                    continue;
                };
                let t = view.reading(cs, |f| match f {
                    Find::Table(t) => Some(t.clone()),
                    _ => None,
                });
                let Some(t) = t else { continue };
                let gap = t.bounds[1] - top;
                let across = t.bounds[0] < right && t.bounds[2] > left;
                if across
                    && gap >= -0.3 * size
                    && gap <= TOTALS_GAP * size
                    && best.as_ref().is_none_or(|(g, _)| gap < *g)
                {
                    best = Some((gap, t));
                }
            }
            child = p;
            parent = ps.parent;
        }
        let (_, table) = best?;
        let edges = &table.columns.edges;
        let cols = edges.len().saturating_sub(1);
        let column = |c: usize| {
            table
                .body
                .iter()
                .filter_map(move |r| r.get(c))
                .filter(|t| !t.is_empty())
        };
        let money: Vec<bool> = (0..cols)
            .map(|c| {
                let n = column(c).count();
                let a = column(c).filter(|t| amount(t)).count();
                a > 0 && 2 * a >= n
            })
            .collect();
        if !money.iter().any(|m| *m) {
            return None;
        }
        let description = (0..cols).find(|&c| {
            let n = column(c).count();
            let words = column(c)
                .filter(|t| {
                    t.chars().any(char::is_alphabetic)
                        && matches!(Shape::of(t), Some(Shape::Text) | None)
                })
                .count();
            n > 0 && 2 * words > n
        });
        let slack = 0.5 * size;
        let under = |p: &Piece| {
            (0..cols).any(|c| {
                let mid = (p.x0 + p.x1) / 2.0;
                money[c]
                    && ((mid >= edges[c] && mid <= edges[c + 1])
                        || (p.x1 - edges[c + 1]).abs() <= slack)
            })
        };
        let mut labels = description.is_some();
        for line in &pieces {
            let first = line.iter().position(|p| amount(&text(p)))?;
            if !line[first..].iter().filter(|p| amount(&text(p))).all(under) {
                return None;
            }
            labels &= description.is_some_and(|d| {
                line[..first]
                    .first()
                    .is_some_and(|p| p.x0 >= edges[d] - slack && p.x0 <= edges[d + 1])
            });
        }
        let (lo, hi) = (0..cols)
            .filter(|&c| money[c])
            .fold((f32::MAX, f32::MIN), |(lo, hi), c| {
                (lo.min(edges[c]), hi.max(edges[c + 1]))
            });
        let from = view.hs().partition_point(|s| s.at < top - SNAP);
        // A rule of the table's own frame is no rule between it and its totals.
        let to = view.hs().partition_point(|s| s.at < table.bounds[1] - SNAP);
        let rule = view.hs()[from..to.max(from)]
            .iter()
            .any(|s| s.from <= lo + SNAP && s.to >= hi - SNAP);
        Some(TotalsRead {
            lines: pieces.len(),
            labels,
            rule,
        })
    }
}

/// Totals: every line of the segment holds an amount under a money column of the table right
/// above it (`totals.under_money_columns`), its labels stand in the table's description
/// column (`totals.labels_in_description`), and a rule across the money columns parts them
/// from the table (`totals.end_rule`).
pub fn type_totals(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    view.totals_of(seg, |t| {
        let t = t?;
        let mut signals = vec![signal(
            "totals.under_money_columns",
            format!(
                "{} lines hold amounts under the money columns of the table above",
                t.lines
            ),
            TOTALS_UNDER,
        )];
        if t.labels {
            signals.push(signal(
                "totals.labels_in_description",
                "their labels stand in the table's description column".into(),
                TOTALS_LABELS,
            ));
        }
        if t.rule {
            signals.push(signal(
                "totals.end_rule",
                "a rule across the money columns parts them from the table".into(),
                TOTALS_RULE,
            ));
        }
        Typed::of(Kind::Totals, signals)
    })
}
