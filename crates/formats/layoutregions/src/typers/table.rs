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
//! under a table are no table's. Lines the walk of the text reads as an address or as
//! sentences are no table's either: they say what the lines are.
//!
//! The totals typer looks for the table right above a segment among the segments beside the
//! segment's path to the root, and asks whether every line of the segment holds an amount
//! under a money column of that table.

use crate::extract::pairs::blocks::{
    Columns, KeyValues, address_block, address_columns, blocks_of, key_values, postal,
};
use crate::extract::table::area::{MANY_COLUMNS, MAX_COLUMNS, ON_PICTURE, PAGE_FRAME, TINY_TEXT};
use crate::extract::table::banded::{
    BAND_WORK, BandCells, BodyBelow, band_cells, band_head_refused, band_titles, body_below,
    long_or_prose,
};
use crate::extract::table::caption::{FIGURE_NEAR, caption, captions_near};
use crate::extract::table::grid::{Piece, Shape, amount, grid_cells, set_full, titles_over_values};
use crate::extract::table::ruled::{
    RuledIn, boxed_field, edges, hanging_rows, ruled_cells, sentence_across,
};
use crate::extract::table::sections::sections;
use crate::extract::table::{
    CONTENTS, CROSSES_LINES, Cells, EMPTY_CELLS, LONG_TEXT, MAX_SQUARES, NO_ROWS_AND_COLUMNS,
    ROWS_ALIKE, RUNS_ON, RunRange, SHORT_CELL, TableOut, header_of, middle,
};
use crate::input::{PrimId, TextRun};
use crate::line::LineRun;
use crate::region::{Bounds, Kind, Signal};
use crate::segment::cut::SIZE_STEP;
use crate::segment::drawn::{Grids, Line, SNAP, grid_frame};
use crate::segment::{Segment, Source};
use crate::typers::{Lined, PageView, Typed, lined, seg_runs, signal, text_alone};

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
/// Why lines of a heading size that read as an address are no table.
const ADDRESS_TEXT: &str = "its lines read as an address";
/// Why lines a little larger than the body that read as sentences are no table.
const SENTENCE_TEXT: &str = "its lines read as sentences";
/// Why a grid whose cells each hold a label and its value is no table.
const FIELD_CELLS: &str = "its cells each hold a label and its value";
/// Why lines of labels and values are no table.
const KEY_VALUE_LINES: &str = "its lines are labels and values";
/// Why two lines of titles over their values are no table.
const TITLE_LINES: &str = "its lines are titles over their values";
/// A refusal: the lines are fields in strips between rules (see
/// [`super::pairs::strip_fields`]).
const FIELD_STRIPS: &str = "its lines are fields in strips between rules";
/// A refusal: the lines alternate between two styles (see [`alternating`]).
const ALTERNATING: &str = "its lines alternate between labels and values";
/// A refusal: the lines hold a strip between rules that reads no table of its own.
const STRIP_INSIDE: &str = "its lines take in a strip between rules that reads as something else";
/// A refusal: the sizes of the lines interleave (see [`interleaved`]).
const INTERLEAVED: &str = "its lines interleave two sizes, labels between values";
/// A refusal: the lines are blocks side by side (see [`address_columns`]).
const SIDE_BLOCKS: &str = "its columns are blocks side by side, each a block of its own";
/// Why lines read as totals under a table are no table of their own.
const TOTALS_LINES: &str = "its lines are totals under the table above";

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

/// A refused reading: the kind its reason speaks for, as a named signal. A refusal for prose
/// carries the paragraph's weight ([`PARAGRAPH_PROSE_CELLS`]); every other refusal carries
/// none: a figure's and the spatial fallback's for those kinds to weigh, and those for labels
/// and values, totals, or an address or sentences the walk reads, because those kinds have
/// signals of their own for the same evidence.
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
        let given = matches!(seg.source, Source::Given(_));
        if !given
            && !matches!(seg.source, Source::Drawing)
            && let Some(why) = content_against(view, seg)
        {
            return refused_for(why);
        }
        match seg.source {
            Source::Grid { rows, cols } if rows >= 2 && cols >= 2 => {
                if super::pairs::cell_fields(view, seg).is_some() {
                    return refused(Speaks::LabelValue, FIELD_CELLS);
                }
                ruled(view, seg)
            }
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

/// The kinds a refusal of the table rules speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Speaks {
    Paragraph,
    Figure,
    Spatial,
    LabelValue,
    Totals,
}

fn refused(speaks: Speaks, why: &'static str) -> Find {
    let (kind, id, weight) = match (speaks, why) {
        // The walk's demotion is already the paragraph's signal: the refusal names it with no
        // weight of its own.
        (Speaks::Paragraph, ADDRESS_TEXT | SENTENCE_TEXT) => {
            (Kind::Paragraph, "paragraph.content_refuses_table", 0)
        }
        (Speaks::Paragraph, _) => (
            Kind::Paragraph,
            "paragraph.prose_cells",
            PARAGRAPH_PROSE_CELLS,
        ),
        (Speaks::Figure, ON_PICTURE) => (Kind::Figure, "figure.on_picture", 0),
        (Speaks::Figure, TINY_TEXT) => (Kind::Figure, "figure.page_picture", 0),
        (Speaks::Figure, _) => (Kind::Figure, "figure.caption_near", 0),
        (Speaks::Spatial, PAGE_FRAME) => (Kind::Spatial, "spatial.page_frame", 0),
        (Speaks::Spatial, MANY_COLUMNS) => (Kind::Spatial, "spatial.many_columns", 0),
        (Speaks::Spatial, EMPTY_CELLS) => (Kind::Spatial, "spatial.empty_cells", 0),
        (Speaks::Spatial, COMB) => (Kind::Spatial, "spatial.comb_boxes", 0),
        (Speaks::Spatial, CONTENTS) => (Kind::Spatial, "spatial.contents", 0),
        (Speaks::Spatial, ROWS_ALIKE) => (Kind::Spatial, "spatial.rows_alike", 0),
        (Speaks::Spatial, CROSSES_LINES) => (Kind::Spatial, "spatial.crossed_lines", 0),
        (Speaks::Spatial, NO_ROWS_AND_COLUMNS) => (Kind::Spatial, "spatial.no_rows_and_columns", 0),
        (Speaks::Spatial, SIDE_BLOCKS) => (Kind::Spatial, "spatial.side_blocks", 0),
        (Speaks::Spatial, ALTERNATING) => (Kind::Spatial, "spatial.alternating", 0),
        (Speaks::Spatial, STRIP_INSIDE) => (Kind::Spatial, "spatial.strip_inside", 0),
        (Speaks::Spatial, INTERLEAVED) => (Kind::Spatial, "spatial.interleaved", 0),
        (Speaks::Spatial, _) => (Kind::Spatial, "spatial.refused_table", 0),
        (Speaks::LabelValue, _) => (Kind::LabelValue, "label_value.refuses_table", 0),
        (Speaks::Totals, _) => (Kind::Totals, "totals.refuses_table", 0),
    };
    Find::Refused(Refusal {
        kind,
        signal: signal(id, why.to_string(), weight),
    })
}

/// What a reason of a table rule speaks for; `None` for a reason that only says the evidence
/// of a table is missing (too few rows, no columns, columns not clean).
fn speaks_for(why: &'static str) -> Option<Speaks> {
    match why {
        LONG_TEXT | RUNS_ON | SET_FULL | ADDRESS_TEXT | SENTENCE_TEXT => Some(Speaks::Paragraph),
        ON_PICTURE | TINY_TEXT | FIGURE_NEAR => Some(Speaks::Figure),
        PAGE_FRAME | MANY_COLUMNS | EMPTY_CELLS | NO_ROWS_AND_COLUMNS | CROSSES_LINES | COMB
        | ROWS_ALIKE | CONTENTS | SIDE_BLOCKS | ALTERNATING | INTERLEAVED | STRIP_INSIDE => {
            Some(Speaks::Spatial)
        }
        _ => None,
    }
}

fn refused_for(why: &'static str) -> Find {
    match speaks_for(why) {
        Some(speaks) => refused(speaks, why),
        None => Find::Nothing,
    }
}

/// Why the walk of the segment's lines speaks against a table, if it does: lines of a
/// heading size that read as an address, or as sentences (see
/// [`crate::extract::text::Demotion`]). These say what the lines are, as the reader's
/// address blocks and prose refuse a table; the other demotions (lines apart, unwrapped, in
/// cells) say how lines stand, which a table's rows do.
fn content_against(view: &PageView<'_>, seg: &Segment) -> Option<&'static str> {
    use crate::extract::text::Demotion;
    view.with_text(seg, |t| {
        t.demotions().find_map(|d| match d {
            Demotion::Address => Some(ADDRESS_TEXT),
            Demotion::Sentences => Some(SENTENCE_TEXT),
            _ => None,
        })
    })
}

/// The group of `g` whose frame is `b` (within [`SNAP`]).
pub(crate) fn group_at<'g>(g: &'g Grids, b: &Bounds) -> Option<&'g (Vec<usize>, Vec<usize>)> {
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
            refuse(refused(Speaks::Spatial, COMB));
            continue;
        }
        let sizes: Vec<f32> = within.iter().map(|&k| refs[k].size).collect();
        let inside = |i: usize| {
            let (cx, cy) = middle(&view.page.texts[i]);
            cx > b[0] && cx < b[2] && cy > b[1] && cy < b[3]
        };
        let (_, figure) = figure_near(view, b, &sizes, &inside, within.iter().map(|&k| refs[k]));
        if figure {
            refuse(refused(Speaks::Figure, FIGURE_NEAR));
            continue;
        }
        if cols > MAX_COLUMNS {
            refuse(refused(Speaks::Spatial, MANY_COLUMNS));
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
                if let Some(why) = long_or_prose(&cell_texts(&refs, &t), SHORT_CELL) {
                    refuse(refused(Speaks::Paragraph, why));
                    continue;
                }
                let read = read_of(&refs, &|k| ids[k], &t, xs.clone(), How::Ruled);
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

/// Lines a block of alternating labels and values holds at least.
const ALTERNATING_LINES: usize = 4;

/// The style of each line of `ranges` (see [`crate::line::line_style`]): its size and weight;
/// `None` when a line has no ink.
fn line_styles(runs: &[&TextRun], ranges: &[RunRange]) -> Option<Vec<(f32, bool)>> {
    crate::work::add(ranges.len() as u64);
    ranges
        .iter()
        .map(|l| {
            crate::line::line_style(runs[l.start..l.end].iter().copied()).map(|r| (r.size, r.bold))
        })
        .collect()
}

/// Whether two sizes are a size step apart (see [`SIZE_STEP`]).
fn size_step(a: f32, b: f32) -> bool {
    a.min(b) > 0.0 && a.max(b) >= SIZE_STEP * a.min(b)
}

/// Whether the lines `ranges` of `runs` alternate between two styles, each line in another
/// style than the line before it (a size step, or another weight), over [`ALTERNATING_LINES`]
/// lines or more: lines of labels between lines of their values (values over labels, labels
/// over values), which rows of a table never are (a table's body rows share one style; a
/// header or a total changes it).
fn alternating(runs: &[&TextRun], ranges: &[RunRange]) -> bool {
    if ranges.len() < ALTERNATING_LINES {
        return false;
    }
    let Some(styles) = line_styles(runs, ranges) else {
        return false;
    };
    let differ = |a: (f32, bool), b: (f32, bool)| a.1 != b.1 || size_step(a.0, b.0);
    styles.windows(2).all(|w| differ(w[0], w[1])) && styles.windows(3).all(|w| !differ(w[0], w[2]))
}

/// How often the size of the lines may step to the other size and back: a header over the
/// rows and a total under them in the header's size step twice (there and back); lines of
/// labels between lines of values (values over labels with a second value line) step three
/// times or more.
const SIZE_TURNS: usize = 2;

/// Whether lines of two sizes a size step apart interleave: the sizes of consecutive lines,
/// taken as runs of one size, step more than [`SIZE_TURNS`] times between two sizes (each run
/// a size step off the one before and in the size of the one before that). A table's rows
/// share one size; a header and a total in a larger size step there and back once. A section
/// row (a line of one piece in the header's size, between rows of one size, see `pieces`) is
/// no line of labels among values: it is left out of the count, so a table grouped by section
/// rows in its header's size keeps its reading. One pass.
fn interleaved(runs: &[&TextRun], ranges: &[RunRange], pieces: &[Vec<Piece>]) -> bool {
    let Some(styles) = line_styles(runs, ranges) else {
        return false;
    };
    // A section row: a line of one piece in the size of the first line (the header's), a
    // size step off rows of one size on both sides of it.
    let rows = |k: usize| pieces.get(k).is_some_and(|p| p.len() >= 2);
    let section = |k: usize| {
        k > 0
            && k + 1 < styles.len()
            && pieces.get(k).is_some_and(|p| p.len() == 1)
            && rows(k - 1)
            && rows(k + 1)
            && !size_step(styles[k - 1].0, styles[k + 1].0)
            && size_step(styles[k - 1].0, styles[k].0)
            && !size_step(styles[0].0, styles[k].0)
    };
    // The sizes of the runs of lines of one size, section rows left out.
    let mut sizes: Vec<f32> = Vec::new();
    for (k, &(size, _)) in styles.iter().enumerate() {
        if section(k) {
            continue;
        }
        match sizes.last() {
            Some(&last) if !size_step(last, size) => {}
            _ => sizes.push(size),
        }
    }
    // Steps between two sizes only: each run in the size of the run two before it.
    let mut turns = 0usize;
    for (k, w) in sizes.windows(2).enumerate() {
        let back = k == 0 || !size_step(sizes[k - 1], w[1]);
        if back && size_step(w[0], w[1]) {
            turns += 1;
        } else {
            turns = usize::from(size_step(w[0], w[1]));
        }
    }
    turns > SIZE_TURNS
}

/// The reading of the lines of a segment without drawn parts (see [`grid_cells`]), after the
/// rules that read them otherwise first: a paragraph set full, labels and values, titles over
/// one row of values; then a figure caption next to them and what the area says. `given`: a
/// region the file states, read by geometry alone (none of those rules).
fn lines(view: &PageView<'_>, seg: &Segment, given: bool) -> Find {
    let (refs, ids) = seg_runs(view, seg);
    let Lined {
        runs,
        at: inked,
        ranges,
        pieces,
    } = lined(&refs);
    if runs.len() < 4 {
        return Find::Nothing;
    }
    if pieces.iter().filter(|p| p.len() >= 2).count() < 2 {
        return Find::Nothing;
    }
    let prim = |k: usize| ids[inked[k]];
    if !given {
        if set_full(&runs, &pieces, &ranges) {
            return refused(Speaks::Paragraph, SET_FULL);
        }
        if view.strip_fields_of(seg).is_some() {
            return refused(Speaks::LabelValue, FIELD_STRIPS);
        }
        // The lines between two rules are kept together because they are one block: a wider
        // reading does not take in a strip that reads positively as something else (fields,
        // or a refusal of the table rules: blocks side by side, labels between values). A
        // strip that reads nothing of its own (a header of two lines, too few rows) may be
        // part of the table.
        let strip_no_table = seg.children.iter().any(|c| {
            view.segs.all.get(c.0 as usize).is_some_and(|k| {
                k.source == Source::Strip
                    && (view.strip_fields_of(k).is_some()
                        || view.reading(k, |f| matches!(f, Find::Refused(_))))
            })
        });
        if strip_no_table {
            return refused(Speaks::Spatial, STRIP_INSIDE);
        }
        if alternating(&runs, &ranges) {
            return refused(Speaks::Spatial, ALTERNATING);
        }
        if interleaved(&runs, &ranges, &pieces) {
            return refused(Speaks::Spatial, INTERLEAVED);
        }
        let off = vec![false; ranges.len()];
        if let KeyValues::Rows(_) = key_values(&runs, &ranges, 0, ranges.len(), &pieces, &off) {
            return refused(Speaks::LabelValue, KEY_VALUE_LINES);
        }
        if titles_over_values(&runs, &ranges, &pieces) {
            return refused(Speaks::LabelValue, TITLE_LINES);
        }
        // Blocks side by side on shared baselines, each a block of facts of its own (an
        // address beside a bank or a register): their lines share no column structure, so
        // they are no rows of one table (today's rule of the lines without rules, one copy).
        // So are columns of which one holds a postal address down its lines: a table of
        // addresses holds one address a row, never one down a column.
        let (columns, _) = blocks_of(&runs, &pieces);
        let address_down = columns.iter().any(|c| {
            let lines: Vec<&str> = c.iter().flatten().map(|l| l.text.as_str()).collect();
            address_block(&lines) && postal(&lines)
        });
        if address_down || matches!(address_columns(&columns), Columns::Blocks | Columns::Headed) {
            return refused(Speaks::Spatial, SIDE_BLOCKS);
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
        return refused(Speaks::Figure, FIGURE_NEAR);
    }
    let rows_needed = if table_caption { 2 } else { 3 };
    match grid_cells(&runs, &pieces, rows_needed, table_caption) {
        Ok(t) => {
            if !given && let Some(why) = view.area().refuses(b, &sizes) {
                return refused_for(why);
            }
            Find::Table(Box::new(read_of(&runs, &prim, &t, Vec::new(), How::Lines)))
        }
        Err(_) if given => Find::Nothing,
        Err(why) => refused_for(why),
    }
}

/// The reading of `seg` with totals looked at: lines that read as totals under a table above
/// are no table of their own.
fn find<U>(view: &PageView<'_>, seg: &Segment, f: impl FnOnce(&Find) -> U) -> U {
    let totals = view.totals_of(seg, |t| t.is_some());
    if totals {
        return f(&refused(Speaks::Totals, TOTALS_LINES));
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

/// Spatial: why the table rules refuse `seg` as a form, a frame or text that is no table
/// (`spatial.page_frame`, `spatial.many_columns` and the other `spatial.*` reasons of
/// [`refusal`]), each of weight 0. Spatial is the fallback of the arbitration, not a
/// candidate: a score of 0 never decides a segment, and these signals only say why a segment
/// the arbitration is unsure of read no table. A region the file states is decided as given.
pub fn type_spatial(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    if matches!(seg.source, Source::Given(_)) {
        return None;
    }
    let r = refusal(view, seg)?;
    (r.kind == Kind::Spatial).then(|| Typed::of(Kind::Spatial, vec![r.signal]))?
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
            crate::work::add(1);
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
    /// The totals reading of `seg`, text alone (see [`text_alone`]) of the view's tree whose every line holds an
    /// amount, under the nearest table above among the segments beside its path to the root
    /// (its parent stands on that path; at most [`TOTALS_GAP`] sizes of their text below it,
    /// across it): every amount under a money column of it (a column most of whose body cells
    /// are amounts), and every other piece of a line in its description column (a label, which
    /// may run on over the column next to it) or anywhere else but in the shape of a column its
    /// rows fill: a piece of the shape most cells of such a column have (a quantity under
    /// Menge, a number under Pos) makes the line a row of the table, not its totals. A table whose rows fill no column but its description and its
    /// money columns has rows of the same shape as totals: only a rule across its money columns
    /// between them tells them apart, so its totals need that rule.
    pub(crate) fn of(view: &PageView<'_>, seg: &Segment) -> Option<TotalsRead> {
        if !text_alone(view, seg) {
            return None;
        }
        // Only a segment of the view's tree has a path to the root.
        let own = view
            .segs
            .all
            .get(seg.id.0 as usize)
            .is_some_and(|s| std::ptr::eq(s, seg));
        if !own {
            return None;
        }
        let (refs, _) = seg_runs(view, seg);
        let Lined {
            runs,
            ranges,
            pieces,
            ..
        } = lined(&refs);
        if runs.is_empty() {
            return None;
        }
        let text = |p: &Piece| crate::extract::table::grid::text_of(&runs, p.start, p.end);
        // Every line holds an amount.
        if !pieces.iter().all(|l| l.iter().any(|p| amount(&text(p)))) {
            return None;
        }
        let size = median(ranges.iter().map(|l| l.size).collect());
        let top = runs.iter().map(|r| r.y + r.size).fold(f32::MIN, f32::max);
        let left = runs.iter().map(|r| r.x).fold(f32::MAX, f32::min);
        let right = runs.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max);
        // The segments on the path to the root.
        let mut path: Vec<usize> = vec![seg.id.0 as usize];
        let mut up = seg.parent;
        while let Some(p) = up {
            crate::work::add(1);
            path.push(p.0 as usize);
            up = view.segs.all.get(p.0 as usize).and_then(|s| s.parent);
        }
        // The nearest table above: the tables whose bottom stands in the window over the
        // lines, found by search in the page's tables sorted by bottom.
        let tables = view.tables();
        let lo = tables.partition_point(|t| t.0 < top - 0.3 * size);
        let hi = tables.partition_point(|t| t.0 <= top + TOTALS_GAP * size);
        let mut best: Option<(f32, &TableRead)> = None;
        for &(bottom, i) in &tables[lo..hi.max(lo)] {
            crate::work::add(1);
            let cand = &view.segs.all[i];
            let beside =
                cand.parent.is_some_and(|p| path.contains(&(p.0 as usize))) && !path.contains(&i);
            let Some(Find::Table(t)) = view.reading_at(i) else {
                continue;
            };
            let gap = bottom - top;
            let across = t.bounds[0] < right && t.bounds[2] > left;
            if beside && across && best.is_none_or(|(g, _)| gap < g) {
                best = Some((gap, t));
            }
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
        // The columns the table's rows fill (text in at least half of them) other than its
        // description and money columns.
        let rows = table.body.len();
        let filled: Vec<bool> = (0..cols)
            .map(|c| !money[c] && Some(c) != description && 2 * column(c).count() >= rows.max(1))
            .collect();
        // The shape most cells of each filled column have (a number under Pos or Menge).
        let shapes: Vec<Option<Shape>> = (0..cols)
            .map(|c| {
                if !filled[c] {
                    return None;
                }
                let of: Vec<Option<Shape>> = column(c).map(|t| Shape::of(t)).collect();
                Shape::most(&of)
            })
            .collect();
        let slack = 0.5 * size;
        let column_at = |x: f32| {
            edges
                .partition_point(|e| *e <= x)
                .saturating_sub(1)
                .min(cols.saturating_sub(1))
        };
        let in_description = |p: &Piece| {
            description.is_some_and(|d| p.x0 >= edges[d] - slack && p.x0 <= edges[d + 1])
        };
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
            for p in line {
                if amount(&text(p)) {
                    if !under(p) {
                        return None;
                    }
                } else if !in_description(p) {
                    // A piece of the shape of a column the rows fill (a quantity under Menge,
                    // a number under Pos): a row of the table. A label of words stands
                    // wherever the layout puts it, flush left or against its amount, also under
                    // a column of words (Einheit): words never make a row.
                    let c = column_at((p.x0 + p.x1) / 2.0);
                    let shape = Shape::of(&text(p));
                    if shape.is_some() && shape != Some(Shape::Text) && shape == shapes[c] {
                        return None;
                    }
                }
            }
            labels &= line[..first].first().is_some_and(in_description);
        }
        let (mlo, mhi) = (0..cols)
            .filter(|&c| money[c])
            .fold((f32::MAX, f32::MIN), |(lo, hi), c| {
                (lo.min(edges[c]), hi.max(edges[c + 1]))
            });
        let from = view.hs().partition_point(|s| s.at < top - SNAP);
        // A rule of the table's own frame is no rule between it and its totals.
        let to = view.hs().partition_point(|s| s.at < table.bounds[1] - SNAP);
        let rule = view.hs()[from..to.max(from)]
            .iter()
            .any(|s| s.from <= mlo + SNAP && s.to >= mhi - SNAP);
        crate::work::add((to.saturating_sub(from) + pieces.len()) as u64);
        // Rows of a table of a description and amounts only are told from its totals by a
        // rule alone.
        if !filled.iter().any(|f| *f) && !rule {
            return None;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An address or sentences that the walk reads refuse a table for the paragraph, with no
    /// weight of their own (the demotion is the paragraph's signal); the other prose reasons
    /// carry the paragraph's weight; every spatial reason has its own name.
    #[test]
    fn refusals_name_the_kind_they_speak_for() {
        for (why, id, weight) in [
            (ADDRESS_TEXT, "paragraph.content_refuses_table", 0),
            (SENTENCE_TEXT, "paragraph.content_refuses_table", 0),
            (LONG_TEXT, "paragraph.prose_cells", PARAGRAPH_PROSE_CELLS),
            (CROSSES_LINES, "spatial.crossed_lines", 0),
            (NO_ROWS_AND_COLUMNS, "spatial.no_rows_and_columns", 0),
            (FIGURE_NEAR, "figure.caption_near", 0),
        ] {
            let Find::Refused(r) = refused_for(why) else {
                panic!("{why} is a refusal");
            };
            assert_eq!((r.signal.id, r.signal.weight), (id, weight), "{why}");
        }
        assert_eq!(refused_for("too few rows"), Find::Nothing);
    }
}
