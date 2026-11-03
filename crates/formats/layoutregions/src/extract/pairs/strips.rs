// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rows of fields between drawn rules: where a value line is cut at its field edges
//! ([`field_cuts`]), the fields of the cells of one row ([`strip_cells`]), and the fields of a
//! group of rows whose labels stand under the rule their values stand on ([`pair_under`]).
//! The frames, the groups of rows and the text beyond a group are the reader's to find.

use std::collections::{HashMap, HashSet};

use super::*;
use crate::extract::table::{CELL_GAP, middle};
use crate::input::GapIn;
use crate::line::{LABEL_STEP, LineRun};
use crate::segment::drawn::SNAP;

/// Steps the search for fields between rules may take on one page.
pub const STRIP_WORK: usize = 1_000_000;

/// The edges of a frame of fields between rules: its left and right ends and its dividers,
/// left to right.
#[derive(Debug, Clone, Copy)]
pub struct FieldFrame<'a> {
    pub x0: f32,
    pub x1: f32,
    pub dividers: &'a [f32],
}

/// Where to cut the runs of the page's `frames` of fields (`ids_of` gives the runs of a frame
/// by its place in `frames`, asked once for each frame in order until the work bound is
/// spent): a run that stands across a field edge (see `field_edges`) is cut at its word gap
/// nearest the edge, when that gap lies at most `NEAR_EDGE` characters from it. Each run
/// with the indexes of its gaps (see [`LineRun::gaps`]), in the order of the runs.
pub fn field_cuts<R: LineRun>(
    runs: &[R],
    frames: &[FieldFrame<'_>],
    ids_of: &mut dyn FnMut(usize) -> Vec<usize>,
) -> Vec<(usize, Vec<usize>)> {
    let mut at: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut work = 0usize;
    for (f, fr) in frames.iter().enumerate() {
        let ids = ids_of(f);
        work += ids.len();
        if work > STRIP_WORK {
            break;
        }
        if ids.is_empty() {
            continue;
        }
        let edges = field_edges(runs, fr, &ids);
        let inner = &edges[1..edges.len() - 1];
        if !holds_fields(runs, fr, &ids, inner) {
            continue;
        }
        for &i in &ids {
            let r = &runs[i];
            for &e in inner.iter().filter(|&&e| across(r, e)) {
                let distance = |g: &GapIn| {
                    if e < g.from {
                        g.from - e
                    } else if e > g.to {
                        e - g.to
                    } else {
                        0.0
                    }
                };
                let near = r
                    .gaps()
                    .enumerate()
                    .min_by(|a, b| distance(&a.1).total_cmp(&distance(&b.1)));
                if let Some((k, g)) = near
                    && distance(&g) <= NEAR_EDGE * CHAR * r.size().max(1.0)
                {
                    at.entry(i).or_default().push(k);
                }
            }
        }
    }
    let mut out: Vec<(usize, Vec<usize>)> = at
        .into_iter()
        .map(|(i, mut ks)| {
            ks.sort_unstable();
            ks.dedup();
            (i, ks)
        })
        .collect();
    out.sort_by_key(|c| c.0);
    out
}

/// One strip of fields between two rules: its rules' heights, its left and right ends, its
/// dividers, and its fields by cell, left to right, read with each label over or left of its
/// value.
pub struct Strip {
    pub top: f32,
    pub bottom: f32,
    pub x0: f32,
    pub x1: f32,
    pub dividers: Vec<f32>,
    pub fields: Vec<Field>,
    /// Its text as one line of labels over one line of values (see [`two_lines`]).
    pub rows: Option<(Vec<usize>, Vec<usize>)>,
    /// A field of it was left as text.
    pub left_out: bool,
}

/// Why fields between rules are not paired: a value over a field edge, a value under none of
/// the labels of its part of the row or under two, labels that do not stand over or beside
/// their values, labels under the rule their values stand on whose values are not all clear,
/// and one field alone.
pub(crate) const ACROSS: &str = "a value stands across two fields";

pub(crate) const UNDER_NONE: &str =
    "a value stands under none of the labels of its field or under two";

pub const NOT_FIELDS: &str = "its labels do not all stand over or beside their values";

pub const UNDER_UNCLEAR: &str = "its labels stand under the rule their values stand on, and not \
     every value's label is clear";

/// Why two lines in columns between two rules are read line by line.
pub const LINE_BY_LINE: &str = "each line may be a row, or each column a cell of its own whose \
     lines go together";
pub const LONE: &str = "it is one field alone between its rules, which a caption over a line of \
     text between rules looks like";

/// The width of a character of a data font, as a share of its size: a value line padded
/// into its fields has one to three of them between two fields.
pub const CHAR: f32 = 0.6;

/// A word gap this many characters from a field edge or nearer is where the value line is
/// cut for it.
pub(crate) const NEAR_EDGE: f32 = 3.0;

/// What [`strip_cells`] reads in the cells of a row of fields.
#[derive(Debug, Default)]
pub struct StripRead {
    /// The fields of its cells read right, left to right.
    pub fields: Vec<Field>,
    /// The cells left out of the row, by why, with the names of their labels.
    pub left_out: Vec<(&'static str, Vec<String>)>,
    /// Every part of a cell is a field, a title, text of one style with no label, an address
    /// or not clear: else the row holds text that is no field.
    pub fits: bool,
    /// A cell is not clear for a reason of fields.
    pub unread: bool,
    /// The first reason of fields a cell is not clear for, when its text is as short as a
    /// field's (see `field_sized`), not for its many styles alone.
    pub field_why: Option<&'static str>,
    /// Two lines in columns that may be rows or cells (see `text_cells`).
    pub doubt: bool,
    /// The heads of a cell in three styles whose first line is heads over lines of their own,
    /// or of a cell of one face whose first line is a little smaller: the row is a row of
    /// fields not clear, which is said by them.
    pub styled_heads: Vec<String>,
}

/// The fields of the `cells` of a row of fields between rules, each read `how`. `edges` are
/// the row's left end, its dividers and its right end, left to right (two at least), and
/// `cells` the runs between each two of them, one list a cell (as [`cells_of`] parts them),
/// so there is one cell fewer than edges. A cell a run stands across (a value line the
/// dividers could not cut at a word gap) is left out with `ACROSS`, a cell whose values
/// stand under no label of its own with `UNDER_NONE`; a cell whose first line is two labels
/// or more over lines of values is parted (see [`parts_of`]), and each part is read as fields
/// (see [`parse`]) that are short. Never a wrong pair: the other cells of the row still pair.
pub fn strip_cells<R: LineRun>(
    runs: &[R],
    edges: &[f32],
    cells: &[Vec<usize>],
    how: Reading<'_>,
) -> StripRead {
    debug_assert!(
        edges.len() >= 2 && cells.len() == edges.len() - 1,
        "a cell between each two edges"
    );
    // Never a wrong pair: a run over a divider leaves the cells it stands in as text; the
    // other cells of the row still pair.
    let inner = &edges[1..edges.len() - 1];
    let cell_at = |x: f32| {
        edges
            .partition_point(|e| *e <= x)
            .saturating_sub(1)
            .min(cells.len() - 1)
    };
    let mut over = vec![false; cells.len()];
    for &i in cells.iter().flatten() {
        let r = &runs[i];
        if inner.iter().any(|&e| across(r, e)) {
            let c = CHAR * r.size().max(1.0);
            for flag in &mut over[cell_at(r.x() + c)..=cell_at(r.x() + r.w() - c)] {
                *flag = true;
            }
        }
    }
    let mut fields: Vec<Field> = Vec::new();
    let mut left_out: Vec<(&'static str, Vec<String>)> = Vec::new();
    // A cell not clear for a reason of fields (a label under its value, a weight only) whose
    // text is as short as a field's (see [`field_sized`]), not for its many styles alone.
    let (mut fits, mut unread, mut field_why) = (true, false, None);
    // Two lines in columns that may be rows or cells (see [`text_cells`]).
    let mut doubt = false;
    // The heads of a cell in three styles whose first line is heads over lines of their
    // own: the row is a row of fields not clear, which is said by them.
    let mut styled_heads: Vec<String> = Vec::new();
    for (c, cell) in cells.iter().enumerate().filter(|(_, c)| !c.is_empty()) {
        let why = if over[c] {
            Some(ACROSS)
        } else {
            match parts_of(runs, cell) {
                Some(parts) => {
                    let parts = match text_cells(runs, how, parts, cells.len()) {
                        TextCells::Whole(p) | TextCells::Cells(p) => p,
                        TextCells::Doubt(p) => {
                            doubt = true;
                            p
                        }
                    };
                    // Parts side by side in a cell, or cells parted by dividers.
                    let beside = parts.len() >= 2 || cells.len() >= 2;
                    for part in parts {
                        match parse(runs, &part, how) {
                            Parsed::Fields(f) if f.iter().all(|f| short(runs, f)) => {
                                fields.extend(f);
                            }
                            // Beside the other heads of its line, a part that is no field
                            // is left out of the row: a bold title is its own text, and
                            // text of one style with no label is said, by its first line.
                            Parsed::No if beside && title_part(runs, &part) => {}
                            Parsed::No if beside && value_sized(runs, &part) => {
                                let names: Vec<String> = lines_of(runs, &part)
                                    .first()
                                    .map(|l| words(runs, l))
                                    .into_iter()
                                    .collect();
                                match left_out.iter_mut().find(|l| l.0 == NO_LABEL) {
                                    Some(l) => l.1.extend(names),
                                    None => left_out.push((NO_LABEL, names)),
                                }
                            }
                            // An address is left out of the row, which is said;
                            // the other fields of the row stay.
                            Parsed::Unsure(why @ (ADDRESS | NAMED_ADDRESS)) => {
                                let names = label_names(runs, &part);
                                match left_out.iter_mut().find(|l| l.0 == why) {
                                    Some(l) => l.1.extend(names),
                                    None => left_out.push((why, names)),
                                }
                            }
                            // One face whose first line is smaller by less than a step:
                            // labels not set apart from their values. The row is not
                            // clear, which is said by its labels, and it is read cell
                            // by cell, never line by line across its cells.
                            Parsed::No if under_a_step(runs, &part) && field_sized(runs, &part) => {
                                unread = true;
                                field_why = field_why.or(Some(NO_LABEL));
                                styled_heads.extend(label_names(runs, &part));
                            }
                            Parsed::Unsure(why) => {
                                unread = true;
                                if why != THREE_STYLES && field_sized(runs, &part) {
                                    field_why = field_why.or(Some(why));
                                } else if why == THREE_STYLES
                                    && let Some(names) = heads_over_lines(runs, &part)
                                {
                                    field_why = field_why.or(Some(why));
                                    styled_heads.extend(names);
                                }
                            }
                            _ => fits = false,
                        }
                    }
                    None
                }
                None => Some(UNDER_NONE),
            }
        };
        if let Some(why) = why {
            match left_out.iter_mut().find(|l| l.0 == why) {
                Some(l) => l.1.extend(label_names(runs, cell)),
                None => left_out.push((why, label_names(runs, cell))),
            }
        }
    }
    StripRead {
        fields,
        left_out,
        fits,
        unread,
        field_why,
        doubt,
        styled_heads,
    }
}

/// The edges of the fields of a frame: its dividers, and in each cell between them the start
/// of each later label of its first line (see [`label_starts`]); with the frame's ends.
pub(crate) fn field_edges<R: LineRun>(runs: &[R], fr: &FieldFrame, ids: &[usize]) -> Vec<f32> {
    let mut xs = vec![fr.x0];
    xs.extend(fr.dividers);
    xs.push(fr.x1);
    let mut edges = xs.clone();
    for cell in cells_of(runs, &xs, ids) {
        edges.extend(label_starts(runs, &cell));
    }
    edges.sort_by(f32::total_cmp);
    edges
}

/// The runs `ids` by the cell between the edges `xs` their middle lies in.
pub fn cells_of<R: LineRun>(runs: &[R], xs: &[f32], ids: &[usize]) -> Vec<Vec<usize>> {
    let mut cells: Vec<Vec<usize>> = vec![Vec::new(); xs.len() - 1];
    for &i in ids {
        let (cx, _) = middle(&runs[i]);
        let c = xs
            .partition_point(|e| *e < cx)
            .saturating_sub(1)
            .min(cells.len() - 1);
        cells[c].push(i);
    }
    cells
}

/// Whether the run stands across the field edge `e`: more than a character of it on each
/// side.
pub fn across<R: LineRun>(r: &R, e: f32) -> bool {
    let c = CHAR * r.size().max(1.0);
    e - r.x() > c && r.x() + r.w() - e > c
}

/// Whether the frame `fr` with the runs `ids` holds fields, judged before its runs are cut at
/// its field edges `inner`: its cells between dividers, without the runs that stand across an
/// edge, read as fields (see [`parse`]; labels alone are fields here) that are short, and each
/// run that stands across an edge is in pieces between the edges no longer than a value may
/// be. A line of prose under a smaller caption is too long for values, so it is not cut.
pub(crate) fn holds_fields<R: LineRun>(
    runs: &[R],
    fr: &FieldFrame,
    ids: &[usize],
    inner: &[f32],
) -> bool {
    let how = Reading {
        one_field: false,
        smaller: true,
        small: i32::MAX,
        labels: &[],
    };
    let crossed = |r: &R| -> Vec<f32> { inner.iter().copied().filter(|&e| across(r, e)).collect() };
    let mut rest = Vec::new();
    for &i in ids {
        let r = &runs[i];
        let edges = crossed(r);
        if edges.is_empty() {
            rest.push(i);
            continue;
        }
        let chars = r.text().chars().count().max(1);
        let per = r.w().max(0.0) / chars as f32;
        let mut cuts = vec![r.x()];
        cuts.extend(edges);
        cuts.push(r.x() + r.w());
        if cuts
            .windows(2)
            .any(|w| per > 0.0 && (w[1] - w[0]) / per > VALUE_CHARS as f32)
        {
            return false;
        }
    }
    let mut xs = vec![fr.x0];
    xs.extend(fr.dividers);
    xs.push(fr.x1);
    let fields = |c: &[usize], beside: bool| match parse(runs, c, how) {
        Parsed::Fields(f) => f.iter().all(|f| short(runs, f)),
        // An address is a cell of the row, left out of it when the row is read.
        Parsed::Unsure(why) => why == ADDRESS || why == NAMED_ADDRESS,
        // So is a title or text with no label beside other heads.
        Parsed::No => beside && (title_part(runs, c) || value_sized(runs, c)),
    };
    cells_of(runs, &xs, &rest)
        .iter()
        .filter(|c| !c.is_empty())
        .all(|c| {
            fields(c, false)
                || parts_of(runs, c)
                    .filter(|p| p.len() >= 2)
                    .is_some_and(|p| p.iter().all(|p| fields(p, true)))
        })
}

/// How the one cell of a frame with no dividers is parted (see [`text_cells`]).
pub(crate) enum TextCells {
    /// As it was given.
    Whole(Vec<Vec<usize>>),
    /// Cells of text side by side, parted by their gutter.
    Cells(Vec<Vec<usize>>),
    /// Two lines of text in columns that may be rows or cells: left whole, which is said.
    Doubt(Vec<Vec<usize>>),
}

/// The parts of the one cell of a frame with no dividers (`cells` 1), when `parts` is that cell
/// whole, of no label, two lines whose text stands in columns parted by gutters (see
/// [`gutter_columns`]), each column of no label, as short as a value and of both lines. A
/// column that is a street over its postcode and town (see [`blocks::postal`]), beside
/// no column that can be its labels (a colon at a line's end, no digit on any line, or words
/// and a short index), shows that the lines of a column go together: the
/// columns are cells of text side by side between two rules, which the gutter parts as a
/// divider would ([`TextCells::Cells`]). Else the two lines
/// may be rows (a label beside its value on each) or cells (a name over its firm beside
/// another): they are left whole, and that is said ([`TextCells::Doubt`]). A column of numbers
/// is a table's (see [`numeric_piece`]) and titles over one row of values are a header over a
/// row (see [`titles_over_row`]); a frame of one line is a row (a table with a rule under every
/// row), and one of three lines or more is left to the table pass: these stay whole.
pub(crate) fn text_cells<R: LineRun>(
    runs: &[R],
    how: Reading<'_>,
    parts: Vec<Vec<usize>>,
    cells: usize,
) -> TextCells {
    let [cell] = &parts[..] else {
        return TextCells::Whole(parts);
    };
    if cells != 1
        || lines_of(runs, cell).len() != 2
        || !matches!(parse(runs, cell, how), Parsed::No)
    {
        return TextCells::Whole(parts);
    }
    let columns = gutter_columns(runs, cell);
    let text = |c: &Vec<usize>| {
        let lines = lines_of(runs, c);
        lines.len() == 2
            && !lines.iter().all(|l| numeric_piece(&words(runs, l)))
            && matches!(parse(runs, c, how), Parsed::No)
            && value_sized(runs, c)
    };
    if columns.len() < 2 || !columns.iter().all(text) || titles_over_row(runs, &columns) {
        return TextCells::Whole(parts);
    }
    let texts = |c: &Vec<usize>| -> Vec<String> {
        lines_of(runs, c).iter().map(|l| words(runs, l)).collect()
    };
    let postal = columns.iter().any(|c| {
        let lines = texts(c);
        blocks::postal(&lines.iter().map(String::as_str).collect::<Vec<_>>())
    });
    // A column that can be the labels of the column beside it: a colon at a line's end, no
    // digit on any line (whatever its words: `Strasse / Nr.`, `Str. u. Nr.`), or words and a
    // short index (`Feld 1`).
    let labels = columns.iter().any(|c| {
        let lines = texts(c);
        let digit = |t: &str| t.chars().any(|ch| ch.is_ascii_digit());
        let indexed = |t: &str| {
            let w: Vec<&str> = t.split_whitespace().collect();
            w.len() >= 2
                && !w[..w.len() - 1].iter().any(|x| digit(x))
                && w.last().is_some_and(|x| digit(x) && x.chars().count() <= 3)
        };
        lines.iter().any(|t| t.ends_with(':'))
            || lines.iter().all(|t| !digit(t))
            || lines.iter().all(|t| indexed(t))
    });
    let address = postal && !labels;
    if address {
        TextCells::Cells(columns)
    } else {
        TextCells::Doubt(parts)
    }
}

/// Whether the `columns` of a cell are titles over one row of values: two lines, the first
/// one word with no digit in each column, the second with a digit in one (`Name Ort Nr` over
/// `Beispiel Musterstadt 4711`). The table pass reads them as a header and a row.
pub(crate) fn titles_over_row<R: LineRun>(runs: &[R], columns: &[Vec<usize>]) -> bool {
    let digit = |l: &[usize]| words(runs, l).chars().any(|c| c.is_ascii_digit());
    let lines: Vec<Vec<Vec<usize>>> = columns.iter().map(|c| lines_of(runs, c)).collect();
    lines.iter().all(|l| {
        l.len() == 2 && !digit(&l[0]) && words(runs, &l[0]).split_whitespace().count() == 1
    }) && lines.iter().any(|l| digit(&l[1]))
}

/// Columns a frame's labels must head, each over this many value lines or more in the same
/// rows, for its text to be a table's body: column titles over line items.
pub(crate) const BODY_COLUMNS: usize = 3;
pub(crate) const BODY_LINES: usize = 2;

/// The names of the labels on the first line of a cell, when it is labels over values (see
/// `first_line_labels`).
pub fn first_labels<R: LineRun>(runs: &[R], cell: &[usize]) -> Vec<String> {
    first_line_labels(runs, &lines_of(runs, cell))
        .unwrap_or_default()
        .iter()
        .map(|l| words(runs, l))
        .collect()
}

/// Whether the cells of a frame are a table's body, not fields: labels on a first line, each
/// over the value lines under it (see [`parts_of`]) and none without one, and `BODY_COLUMNS`
/// of them or more over
/// `BODY_LINES` value lines or more each, those lines in the same rows across the columns
/// (their baselines within a third of their size). A field's value of two lines stands under
/// one label, not under three in the same rows.
pub fn table_body<R: LineRun>(runs: &[R], cells: &[Vec<usize>]) -> bool {
    let mut columns: Vec<Vec<(f32, f32)>> = Vec::new();
    for cell in cells.iter().filter(|c| !c.is_empty()) {
        let lines = lines_of(runs, cell);
        if first_line_labels(runs, &lines).is_none_or(|l| l.len() < 2) {
            continue;
        }
        let Some(parts) = parts_of(runs, cell) else {
            continue;
        };
        for part in parts {
            let lines = lines_of(runs, &part);
            // A title with no value under it is an empty field: a column of a table has its
            // values.
            if lines.len() < 2 {
                return false;
            }
            let rows: Vec<(f32, f32)> = lines[1..]
                .iter()
                .map(|l| {
                    let r = &runs[l[0]];
                    (r.y(), r.size().max(1.0))
                })
                .collect();
            if rows.len() >= BODY_LINES {
                columns.push(rows);
            }
        }
    }
    if columns.len() < BODY_COLUMNS {
        return false;
    }
    // The rows of the column with the most lines; every column's lines stand in them.
    let widest = columns
        .iter()
        .max_by_key(|c| c.len())
        .cloned()
        .unwrap_or_default();
    let in_rows = |c: &Vec<(f32, f32)>| {
        c.iter()
            .all(|&(y, size)| widest.iter().any(|&(w, _)| (w - y).abs() <= size / 3.0))
    };
    columns.iter().filter(|c| in_rows(c)).count() >= BODY_COLUMNS
}

/// The names of the heads of a cell's first line (runs apart by a cell gap, see
/// [`labels_in_line`]) when there are two or more, each of them a label over its value (all ink
/// of the lines under it that stands under it is a size step larger, see [`LABEL_STEP`]) or a
/// head with nothing under it, and one of them a label: the cell is set as fields side by side,
/// whatever their styles. Lines of one size side by side (a label beside its amount over more
/// such rows: totals) and a slogan over a body line are no such heads.
pub(crate) fn heads_over_lines<R: LineRun>(runs: &[R], cell: &[usize]) -> Option<Vec<String>> {
    let lines = lines_of(runs, cell);
    let heads = labels_in_line(runs, lines.first()?);
    if lines.len() < 2 || heads.len() < 2 {
        return None;
    }
    let pieces: Vec<(f32, f32, f32)> = lines[1..]
        .iter()
        .flatten()
        .filter(|&&i| inked(&runs[i]))
        .flat_map(|&i| {
            let size = runs[i].size();
            ink_pieces(&runs[i])
                .into_iter()
                .map(move |(a, b)| (a, b, size))
        })
        .collect();
    // For each head: whether ink stands under it, and whether all of it is a step larger.
    let kinds: Vec<(bool, bool)> = heads
        .iter()
        .map(|h| {
            let (x0, x1) = extent(runs, h);
            let size = h.iter().map(|&i| runs[i].size()).fold(0.0f32, f32::max);
            let under: Vec<f32> = pieces
                .iter()
                .filter(|&&(a, b, _)| a < x1 && b > x0)
                .map(|p| p.2)
                .collect();
            (
                !under.is_empty(),
                under.iter().all(|&v| v >= LABEL_STEP * size),
            )
        })
        .collect();
    let fields = kinds.iter().all(|&(ink, larger)| !ink || larger)
        && kinds.iter().any(|&(ink, larger)| ink && larger);
    fields.then(|| heads.iter().map(|h| words(runs, h)).collect())
}

/// Whether the runs `ids` of a frame are rows of one grid of a table, by their places alone: two
/// lines or more of two pieces or more (runs apart by a cell gap, see [`labels_in_line`]), their
/// first pieces in one column and their last pieces in one column, one of the two flush right
/// (ending in one place: amounts, with or without a currency or a percent column between), and
/// every other line one piece (a note under them). Such a frame with no field read is left to
/// the table pass; any other (address blocks side by side, a cell of two lines beside cells of
/// one) is read cell by cell.
pub fn totals_rows<R: LineRun>(runs: &[R], ids: &[usize]) -> bool {
    grid_sizes(runs, ids).is_some()
}

/// Whether the runs `ids` of a frame are rows of one grid (see [`totals_rows`]) whose first and
/// last pieces on each line are of one size, and two of whose lines are of one size too (each
/// within a size step, see [`LABEL_STEP`]): rows of a table, as a label stands a size step
/// under its value, over it or beside it. Totals under the rule of line items with a larger
/// total last are such rows; a row of labels over their values and labels beside their values
/// are not.
pub fn table_rows<R: LineRun>(runs: &[R], ids: &[usize]) -> bool {
    let one = |a: f32, b: f32| a.max(b) < LABEL_STEP * a.min(b);
    grid_sizes(runs, ids).is_some_and(|rows| {
        let mut sizes: Vec<f32> = Vec::new();
        for (first, last) in rows {
            if !one(first, last) {
                return false;
            }
            sizes.push(first.max(last));
        }
        sizes.sort_by(f32::total_cmp);
        sizes.windows(2).any(|w| one(w[0], w[1]))
    })
}

/// The sizes of the first and the last piece of each line of two pieces or more of the runs
/// `ids` of a frame, when they are rows of one grid (see [`totals_rows`]).
pub(crate) fn grid_sizes<R: LineRun>(runs: &[R], ids: &[usize]) -> Option<Vec<(f32, f32)>> {
    let ids: Vec<usize> = ids.iter().copied().filter(|&i| inked(&runs[i])).collect();
    // The first and the last piece of each line of two pieces or more: start, end, size.
    let mut rows: Vec<[(f32, f32, f32); 2]> = Vec::new();
    for line in lines_of(runs, &ids) {
        let pieces = labels_in_line(runs, &line);
        if pieces.len() < 2 {
            continue;
        }
        let at = |p: &Vec<usize>| {
            let (a, b) = extent(runs, p);
            let size = p.iter().map(|&i| runs[i].size()).fold(0.0f32, f32::max);
            (a, b, size)
        };
        rows.push([at(&pieces[0]), at(&pieces[pieces.len() - 1])]);
    }
    if rows.len() < 2 {
        return None;
    }
    let same = |k: usize, end: bool| {
        rows.iter().all(|r| {
            let (x, y) = if end {
                (r[k].1, rows[0][k].1)
            } else {
                (r[k].0, rows[0][k].0)
            };
            (x - y).abs() <= CHAR * r[k].2.max(rows[0][k].2)
        })
    };
    let column = |k: usize| same(k, false) || same(k, true);
    // A flush right column holds a number on every line but a first line of titles (an
    // amount, a rate, a count): lines that end in one place by chance (a street and its town
    // beside two lines of a name) are no rows of totals.
    let numbers = |k: usize| {
        lines_of(runs, &ids).iter().skip(1).all(|line| {
            let pieces = labels_in_line(runs, line);
            pieces.len() < 2
                || numeric_piece(&words(
                    runs,
                    &pieces[if k == 0 { 0 } else { pieces.len() - 1 }],
                ))
        })
    };
    let flush = |k: usize| same(k, true) && numbers(k);
    (column(0) && column(1) && (flush(0) || flush(1)))
        .then(|| rows.iter().map(|r| (r[0].2, r[1].2)).collect())
}

/// Whether `text` is a number of a row of totals: digits with separators, signs and spaces, and
/// at most a currency or a percent sign or code beside them (`1.234,56`, `19 %`, `EUR 12,00`,
/// `12,50-`).
pub(crate) fn numeric_piece(text: &str) -> bool {
    let rest: String = text
        .split_whitespace()
        .filter(|w| !matches!(*w, "EUR" | "USD" | "CHF" | "GBP" | "€" | "$" | "£" | "%"))
        .collect();
    let rest = rest.trim_matches(['€', '$', '£', '%']);
    rest.chars().any(|c| c.is_ascii_digit())
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || ".,'+-".contains(c))
}

/// The runs of `cell` in columns left to right, parted by gutters: white space of a cell gap
/// (see [`CELL_GAP`]) or more that no run of the cell crosses on any of its lines. A cell with
/// no such gutter is one column.
pub fn gutter_columns<R: LineRun>(runs: &[R], cell: &[usize]) -> Vec<Vec<usize>> {
    // The stretches of ink of the cell's runs, merged where less than a cell gap apart.
    let mut spans: Vec<(f32, f32, f32)> = cell
        .iter()
        .filter(|&&i| inked(&runs[i]))
        .map(|&i| {
            let r = &runs[i];
            (r.x(), r.x() + r.w().max(0.0), r.size().max(1.0))
        })
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32, f32)> = Vec::new();
    for (a, b, size) in spans {
        match merged.last_mut() {
            Some(m) if a - m.1 < CELL_GAP * size.max(m.2) => {
                m.1 = m.1.max(b);
                m.2 = m.2.max(size);
            }
            _ => merged.push((a, b, size)),
        }
    }
    let mut parts: Vec<Vec<usize>> = vec![Vec::new(); merged.len().max(1)];
    for &i in cell {
        let (cx, _) = middle(&runs[i]);
        let k = merged
            .iter()
            .position(|m| cx <= m.1)
            .unwrap_or(merged.len().saturating_sub(1));
        parts[k].push(i);
    }
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

/// The columns of a cell between dividers, `parts` left to right, with each column that holds
/// no label joined to the column left of it when every line of it stands beside a label there
/// (on its baseline): values beside their labels are one field per line, not two cells. A
/// label is a run within a size step of the cell's smallest text (see [`LABEL_STEP`]).
pub fn beside_labels<R: LineRun>(runs: &[R], parts: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    let ink = |i: &usize| inked(&runs[*i]);
    let small = parts
        .iter()
        .flatten()
        .filter(|i| ink(i))
        .map(|&i| runs[i].size())
        .fold(f32::MAX, f32::min);
    let label = |i: usize| inked(&runs[i]) && runs[i].size() < LABEL_STEP * small;
    let mut out: Vec<Vec<usize>> = Vec::new();
    // Whether each column of `out` took values beside its labels.
    let mut beside: Vec<bool> = Vec::new();
    for part in parts {
        let joins = out.last().is_some_and(|left: &Vec<usize>| {
            !part.iter().any(|&i| label(i))
                && part.iter().filter(|i| ink(i)).all(|&i| {
                    let r = &runs[i];
                    left.iter().any(|&j| {
                        label(j) && (runs[j].y() - r.y()).abs() <= 0.3 * r.size().max(1.0)
                    })
                })
        });
        match out.last_mut() {
            Some(left) if joins => {
                left.extend(part);
                if let Some(b) = beside.last_mut() {
                    *b = true;
                }
            }
            _ => {
                out.push(part);
                beside.push(false);
            }
        }
    }
    // A column of labels with their values beside them is a field on each line.
    out.into_iter()
        .zip(beside)
        .flat_map(|(c, b)| if b { lines_of(runs, &c) } else { vec![c] })
        .collect()
}

/// The runs `line` of one line (in runs order) parted where the style changes (see
/// [`style_of`]; white space goes with the runs before it): a label and a value line of
/// another field that stand on one line (a value printed over the label of the row above it)
/// are two texts, never one word.
pub fn by_style<R: LineRun>(runs: &[R], line: Vec<usize>) -> Vec<Vec<usize>> {
    let mut out: Vec<(Option<Style>, Vec<usize>)> = Vec::new();
    for i in line {
        let r = &runs[i];
        let style = inked(r).then(|| style_of(r));
        match out.last_mut() {
            Some((s, runs)) if style.is_none() || *s == style || s.is_none() => {
                if s.is_none() {
                    *s = style;
                }
                runs.push(i);
            }
            _ => out.push((style, vec![i])),
        }
    }
    out.into_iter().map(|(_, runs)| runs).collect()
}

/// Whether the text of a part of a cell is as short as a value.
pub(crate) fn value_sized<R: LineRun>(runs: &[R], part: &[usize]) -> bool {
    words(runs, part).chars().count() <= VALUE_CHARS
}

/// Whether a part of a cell is a title: bold text alone, of a line or two.
pub(crate) fn title_part<R: LineRun>(runs: &[R], part: &[usize]) -> bool {
    let ids: Vec<usize> = part.iter().copied().filter(|&i| inked(&runs[i])).collect();
    !ids.is_empty()
        && ids.iter().all(|&i| runs[i].bold())
        && lines_of(runs, &ids).len() <= LABEL_LINES
}

/// Whether a field's label stands beside its value: its last label run and its first value
/// run share a line.
pub fn beside_value<R: LineRun>(runs: &[R], f: &Field) -> bool {
    match (f.label.last(), f.value.first()) {
        (Some(&l), Some(&v)) => lines_of(runs, &[l, v]).len() == 1,
        _ => false,
    }
}

/// Whether the text of a part of a cell is as short as a field's: by style, the text of some
/// style as short as a label, and the text of every style as short as a value. A heading over
/// lines of prose between rules is no field whose label stands under its value.
pub(crate) fn field_sized<R: LineRun>(runs: &[R], part: &[usize]) -> bool {
    let mut by_style: Vec<(Style, Vec<usize>)> = Vec::new();
    for &i in part.iter().filter(|&&i| inked(&runs[i])) {
        let s = style_of(&runs[i]);
        match by_style.iter_mut().find(|b| b.0 == s) {
            Some(b) => b.1.push(i),
            None => by_style.push((s, vec![i])),
        }
    }
    let text: Vec<String> = by_style.iter().map(|b| words(runs, &b.1)).collect();
    text.iter().all(|t| t.chars().count() <= VALUE_CHARS)
        && text.iter().any(|t| {
            t.chars().count() <= LABEL_CHARS && t.split_whitespace().count() <= LABEL_WORDS
        })
}

/// What a value is, to tell the values of a column of fields apart: a number (digits with
/// separators), words (no digit), or a code of both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Number,
    Words,
    Code,
}

pub(crate) fn kind_of(text: &str) -> Kind {
    let t = text.trim();
    if !t.chars().any(|c| c.is_ascii_digit()) {
        Kind::Words
    } else if t
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '/' | ' ' | '%'))
    {
        Kind::Number
    } else {
        Kind::Code
    }
}

/// Puts back whole a value that runs on into the empty next field of its row of the group `g`
/// of `strips`: a value cut at its word gap at a divider pairs its last piece with the next
/// field when the dividers part it there. When the other rows of the group hold values in that
/// field's column (its label within a character of the same place), none of them of the
/// piece's kind and all of them shorter than the piece, while the piece is of the kind of the
/// value before it, the piece is that value's overflow: it goes back to its field, and the
/// next field is empty. A short piece (a letter where the column holds numbers) may be that
/// field's own odd value, so it stays where the cut put it.
pub fn overflow_back<R: LineRun>(runs: &[R], strips: &mut [Strip], g: &[usize]) {
    let column = |f: &Field| f.label.first().map(|&i| &runs[i]);
    // The kinds of the values of the group by the place and size of their labels.
    let mut seen: Vec<(f32, usize, Kind, usize)> = Vec::new();
    for &k in g {
        for f in &strips[k].fields {
            if let (Some(l), false) = (column(f), f.value.is_empty()) {
                let text = words(runs, &f.value);
                seen.push((l.x(), k, kind_of(&text), text.chars().count()));
            }
        }
    }
    for &k in g {
        let dividers = strips[k].dividers.clone();
        let fields = &mut strips[k].fields;
        for i in 1..fields.len() {
            let (before, after) = fields.split_at_mut(i);
            let (a, b) = (&mut before[i - 1], &mut after[0]);
            let (Some(&va), [vb]) = (a.value.last(), b.value.as_slice()) else {
                continue;
            };
            let (ra, rb) = (&runs[va], &runs[*vb]);
            let ch = CHAR * ra.size().max(1.0);
            let end = ra.x() + ra.w();
            let touching = (ra.y() - rb.y()).abs() <= 0.3 * ra.size()
                && rb.x() - end <= 1.5 * ch
                && rb.x() - end >= -ch
                && dividers.iter().any(|&d| d > end - ch && d < rb.x() + ch);
            let Some(l) = column(b) else {
                continue;
            };
            let piece = kind_of(rb.text());
            let length = rb.text().trim().chars().count();
            let others: Vec<(Kind, usize)> = seen
                .iter()
                .filter(|s| s.1 != k && (s.0 - l.x()).abs() <= CHAR * l.size().max(1.0))
                .map(|s| (s.2, s.3))
                .collect();
            if touching
                && !others.is_empty()
                && others.iter().all(|&(kind, n)| kind != piece && n < length)
                && kind_of(ra.text()) == piece
            {
                a.value.push(*vb);
                b.value.clear();
            }
        }
    }
}

/// A strip's text as one line of labels over one line of values, each line of one style (a
/// raised marker aside) and the upper one a size step smaller: the runs of each line. A
/// strip of one line of one style has no values. None when its text is not that.
pub fn two_lines<R: LineRun>(runs: &[R], ids: &[usize]) -> Option<(Vec<usize>, Vec<usize>)> {
    let lines = lines_of(runs, ids);
    let marker = markers(runs, &lines);
    let style = |l: &[usize]| -> Option<Style> {
        let mut s = l
            .iter()
            .filter(|i| !marker.contains(i))
            .map(|&i| style_of(&runs[i]));
        let first = s.next()?;
        s.all(|o| o == first).then_some(first)
    };
    match lines.as_slice() {
        [l] => style(l).map(|_| (l.clone(), Vec::new())),
        [a, b] => {
            let (sa, sb) = (style(a)?, style(b)?);
            // The exact sizes decide the step: two sizes on both sides of a half-point edge
            // of the style key by float noise are one size (see [`LABEL_STEP`]).
            let size = |l: &[usize]| mean_size(runs, l.iter().copied(), &marker);
            (sb.0 - sa.0 >= 1 && size(b) >= LABEL_STEP * size(a)).then(|| (a.clone(), b.clone()))
        }
        _ => None,
    }
}

/// Whether the run `r` is in one of the roles `set` (styles with their font family): of its
/// size, weight and slant, and of its family where both families are known.
pub fn in_role<R: LineRun>(set: &[Style], r: &R) -> bool {
    let s = style_of(r);
    set.iter()
        .any(|t| (t.0, t.1, t.2) == (s.0, s.1, s.2) && (t.3 == 0 || s.3 == 0 || t.3 == s.3))
}

/// What stands over the top rule of a group of strips, as the reader finds it in the text
/// beyond the group: nothing in the values' role, values of some of its fields only (or runs
/// at no field start), or a value for every field of its top strips.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TopEnd {
    None,
    Partial,
    Complete,
}

/// What stands under the bottom rule of a group of strips, as the reader finds it in the text
/// beyond the group: nothing in the labels' role, labels of some of its lowest values only (or
/// at no value), a label for every
/// lowest value beyond the group, or a row of labels only of the group itself (its empty last
/// row, or labels over a rule drawn under them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BottomEnd {
    None,
    Partial,
    Free,
    Own,
}

/// The way a group of strips reads by its ends (ruling P6-Z2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Way {
    /// Labels over (or beside) their values.
    Over,
    /// Labels under the rule their values stand on.
    Under,
    /// Neither is clear: a loss names the fields.
    Unclear,
}

/// The way a group reads by its ends, `single` when it has one field a row:
///
/// | fields a row | top | bottom | reading |
/// | --- | --- | --- | --- |
/// | two or more | complete | free, complete | labels under values |
/// | two or more | complete | none, partial, own | not clear |
/// | two or more | none, partial | free, complete | not clear |
/// | two or more | none, partial | none, partial, own | labels over values |
/// | one | complete | any | not clear |
/// | one | any | free, complete | not clear |
/// | one | none, partial | none, partial, own | labels over values |
///
/// Labels under values need both ends complete, field for field, beyond the group. One
/// complete end is also what a grid of the other way shows (an empty first or last row, the
/// column titles of a next table, a line of data over it), so it is not clear. An own row of
/// labels is the group's own empty last row as much as labels over a rule under the last
/// labels: no end. One field a row is too little evidence for labels under values. Found with
/// the runs other groups hold, evidence in part at both ends is not clear either when runs
/// joined to the group by its rules make it at each end: of a field group (a grid parted by
/// double rules) or of a ruled table that shares the group's end rule; from other text it
/// keeps labels over values (the reader finds the ends in the text beyond the group).
pub fn way_of(single: bool, top: TopEnd, bottom: BottomEnd) -> Way {
    match (single, top, bottom) {
        (true, TopEnd::Complete, _) | (true, _, BottomEnd::Free) => Way::Unclear,
        (true, _, _) => Way::Over,
        (false, TopEnd::Complete, BottomEnd::Free) => Way::Under,
        (false, TopEnd::Complete, _) | (false, _, BottomEnd::Free) => Way::Unclear,
        (false, _, _) => Way::Over,
    }
}

/// The fields of the group `g` of `strips` read with each label under the rule its value
/// stands on: the labels at the top of each strip pair with the values over its top rule (at
/// the bottom of the strip over it, or among `above` for a strip at the top), and the values
/// at the bottom of a strip with no strip under it pair with the labels under its bottom rule
/// (among `below`), each row of a label line and a value line read by `pair_row`. None when
/// a strip is no line of labels over a line of values, a field of it was left out, a row is
/// not clear, or a value is left without its label.
pub fn pair_under<R: LineRun>(
    runs: &[R],
    strips: &[Strip],
    g: &[usize],
    above: &[usize],
    below: &[usize],
) -> Option<Vec<Field>> {
    let near = |a: f32, b: f32| (a - b).abs() <= SNAP;
    let overlaps = |s: &Strip, t: &Strip| s.x0 < t.x1 && t.x0 < s.x1;
    let inside = |i: usize, s: &Strip| {
        let (cx, _) = middle(&runs[i]);
        cx > s.x0 && cx < s.x1
    };
    let mut fields = Vec::new();
    let mut used: HashSet<usize> = HashSet::new();
    for &k in g {
        let s = &strips[k];
        if s.left_out {
            return None;
        }
        let (labels, values) = s.rows.as_ref()?;
        let over: Vec<&Strip> = g
            .iter()
            .map(|&o| &strips[o])
            .filter(|t| near(t.bottom, s.top) && overlaps(s, t))
            .collect();
        let mut dividers = s.dividers.clone();
        let over_values: Vec<usize> = if over.is_empty() {
            above.iter().copied().filter(|&i| inside(i, s)).collect()
        } else {
            for t in &over {
                dividers.extend(t.dividers.iter().filter(|&&d| d > s.x0 && d < s.x1));
            }
            over.iter()
                .filter_map(|t| t.rows.as_ref())
                .flat_map(|r| r.1.iter().copied())
                .filter(|&i| inside(i, s))
                .collect()
        };
        dividers.sort_by(f32::total_cmp);
        dividers.dedup();
        fields.extend(pair_row(runs, labels, &over_values, &dividers, s.x0, s.x1)?);
        used.extend(&over_values);
        let last = !g
            .iter()
            .any(|&o| near(strips[o].top, s.bottom) && overlaps(s, &strips[o]));
        if last && !values.is_empty() {
            let labels: Vec<usize> = below.iter().copied().filter(|&i| inside(i, s)).collect();
            fields.extend(pair_row(runs, &labels, values, &s.dividers, s.x0, s.x1)?);
            used.extend(values);
        }
    }
    let all_used = g
        .iter()
        .filter_map(|&k| strips[k].rows.as_ref())
        .flat_map(|r| &r.1)
        .chain(above)
        .all(|i| used.contains(i));
    all_used.then_some(fields)
}

/// The fields of one row of labels on one line and values on another, between `x0` and `x1`
/// and parted by `dividers` (sorted): the label of each part with the values in it, or in a
/// part of several labels each value with the label it stands under (see [`under_labels`]);
/// a part's labels without values are empty fields. None when a run stands across a divider
/// or a value has no certain label.
pub(crate) fn pair_row<R: LineRun>(
    runs: &[R],
    labels: &[usize],
    values: &[usize],
    dividers: &[f32],
    x0: f32,
    x1: f32,
) -> Option<Vec<Field>> {
    let mut edges = vec![x0];
    edges.extend(dividers);
    edges.push(x1);
    let inner = &edges[1..edges.len() - 1];
    if labels
        .iter()
        .chain(values)
        .any(|&i| inner.iter().any(|&e| across(&runs[i], e)))
    {
        return None;
    }
    let by_x = |mut v: Vec<usize>| {
        v.sort_by(|&a, &b| runs[a].x().total_cmp(&runs[b].x()));
        v
    };
    let mut out = Vec::new();
    for (ls, vs) in cells_of(runs, &edges, labels)
        .into_iter()
        .zip(cells_of(runs, &edges, values))
    {
        let labels = labels_in_line(runs, &ls);
        if labels.is_empty() {
            if !vs.is_empty() {
                return None;
            }
            continue;
        }
        let under = if labels.len() == 1 {
            vec![vs]
        } else {
            under_labels(runs, &labels, &vs)?
        };
        out.extend(labels.into_iter().zip(under).map(|(label, value)| Field {
            label,
            value: by_x(value),
        }));
    }
    Some(out)
}

/// The labels of one line of labels, left to right: runs apart by more than a cell gap start a
/// new label, and a raised marker goes with the label before it.
pub fn labels_in_line<R: LineRun>(runs: &[R], line: &[usize]) -> Vec<Vec<usize>> {
    let mut sorted = line.to_vec();
    sorted.sort_by(|&a, &b| runs[a].x().total_cmp(&runs[b].x()));
    let mut out: Vec<Vec<usize>> = Vec::new();
    for i in sorted {
        let r = &runs[i];
        if let Some(label) = out.last_mut() {
            let prev = &runs[*label.last().expect("a run")];
            if raised(r, prev) || r.x() - (prev.x() + prev.w()) <= CELL_GAP * prev.size().max(1.0) {
                label.push(i);
                continue;
            }
        }
        out.push(vec![i]);
    }
    out
}

/// The left and right ends of the ink of runs.
pub(crate) fn extent<R: LineRun>(runs: &[R], ids: &[usize]) -> (f32, f32) {
    ids.iter().fold((f32::MAX, f32::MIN), |(a, b), &i| {
        let r = &runs[i];
        (a.min(r.x()), b.max(r.x() + r.w().max(0.0)))
    })
}

/// Each of `values` given to the one label of `labels` (left to right, on one line) it stands
/// under: its ink overlaps that label's and no other's, it reaches no more than a character
/// past the middle of the room to the label before, and the same to the label after unless it
/// starts where its label starts (a value set from its label's start may run on up to the next
/// label). A label set in the middle or at the right end of its field over values at the left
/// end has no value under it, so its field is not clear. None when a value stands under no
/// label or under two.
pub(crate) fn under_labels<R: LineRun>(
    runs: &[R],
    labels: &[Vec<usize>],
    values: &[usize],
) -> Option<Vec<Vec<usize>>> {
    let ends: Vec<(f32, f32)> = labels.iter().map(|l| extent(runs, l)).collect();
    let mut out = vec![Vec::new(); labels.len()];
    for &v in values {
        let r = &runs[v];
        let (a, b) = (r.x(), r.x() + r.w().max(0.0));
        let mut under = ends.iter().enumerate().filter(|(_, e)| a < e.1 && b > e.0);
        let (k, _) = under.next()?;
        if under.next().is_some() {
            return None;
        }
        let lo = (k > 0).then(|| (ends[k - 1].1 + ends[k].0) / 2.0);
        let hi = (k + 1 < ends.len()).then(|| (ends[k].1 + ends[k + 1].0) / 2.0);
        let aligned = (a - ends[k].0).abs() <= CHAR * r.size().max(1.0);
        if lo.is_some_and(|e| across(r, e)) || (!aligned && hi.is_some_and(|e| across(r, e))) {
            return None;
        }
        out[k].push(v);
    }
    Some(out)
}

/// The heads of the first of the `lines` of a cell when that line is labels over values, each
/// with the value lines under it (see [`parts_of`]): labels in the cell's smallest size (a
/// raised marker aside), over lines a size step larger (see [`LABEL_STEP`]). Labels of one
/// size side by side may have a smaller one between them (a name a little smaller than the
/// labels beside it, see [`heads_of`]).
pub(crate) fn first_line_labels<R: LineRun>(
    runs: &[R],
    lines: &[Vec<usize>],
) -> Option<Vec<Vec<usize>>> {
    let (Some(first), true) = (lines.first(), lines.len() >= 2) else {
        return None;
    };
    let size = |i: usize| runs[i].size();
    let small = first.iter().map(|&i| size(i)).fold(f32::MAX, f32::min);
    let below = lines[1..]
        .iter()
        .flatten()
        .map(|&i| size(i))
        .fold(f32::MIN, f32::max);
    let marker = markers(runs, &lines[..1]);
    if below < LABEL_STEP * small
        || first
            .iter()
            .any(|&i| size(i) >= LABEL_STEP * small && !marker.contains(&i))
    {
        return heads_of(runs, lines, &marker);
    }
    Some(labels_in_line(runs, first))
}

/// The ink of a run in pieces between the gaps of its words: a value line padded into its
/// fields is one run over them.
pub(crate) fn ink_pieces<R: LineRun>(r: &R) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let mut from = r.x();
    for g in r.gaps() {
        if g.from > from {
            out.push((from, g.from));
        }
        from = from.max(g.to);
    }
    let end = r.x() + r.w().max(0.0);
    if end > from {
        out.push((from, end));
    }
    out
}

/// The heads of a first line whose text is not of one size: its labels (runs apart by a cell
/// gap, see [`labels_in_line`]), each a size step under the ink under it (see [`ink_pieces`]; a
/// head with nothing under it is the label of an empty field). A head may be smaller than the
/// labels beside it (a value is never smaller than its label), but a head a size step larger
/// than the head left of it may be that head's value beside it: it is a head only when another
/// head of the line has its size (within a size step), or the head left of it has a value of
/// its own under it (ink under it alone, apart from other ink of its line by a cell gap). A
/// head that is no label heads its own lines when the ink under it has its size (a value or a
/// name at the values' size). Each piece of ink stands under one head at most, and one head at
/// least is a label. None when the line is not such heads.
pub(crate) fn heads_of<R: LineRun>(
    runs: &[R],
    lines: &[Vec<usize>],
    marker: &HashSet<usize>,
) -> Option<Vec<Vec<usize>>> {
    let heads = labels_in_line(runs, &lines[0]);
    if heads.len() < 2 {
        return None;
    }
    let size_of = |h: &Vec<usize>| {
        h.iter()
            .filter(|i| !marker.contains(i))
            .map(|&i| runs[i].size())
            .fold(0.0f32, f32::max)
    };
    let sizes: Vec<f32> = heads.iter().map(size_of).collect();
    let ends: Vec<(f32, f32)> = heads.iter().map(|h| extent(runs, h)).collect();
    // The sizes of the ink under each head, and whether a value stands under it alone.
    let mut under: Vec<Vec<f32>> = vec![Vec::new(); heads.len()];
    let mut own = vec![false; heads.len()];
    for &i in lines[1..].iter().flatten() {
        let r = &runs[i];
        let pieces = ink_pieces(r);
        // The pieces of the run in chunks parted by a cell gap.
        let mut chunk: Vec<usize> = Vec::new();
        for (n, &(a, b)) in pieces.iter().enumerate() {
            let mut over = ends.iter().enumerate().filter(|(_, e)| a < e.1 && b > e.0);
            if let Some((k, _)) = over.next() {
                if over.next().is_some() {
                    return None;
                }
                under[k].push(r.size());
                if !chunk.contains(&k) {
                    chunk.push(k);
                }
            }
            let parted = pieces
                .get(n + 1)
                .is_none_or(|next| next.0 - b > CELL_GAP * r.size().max(1.0));
            if parted {
                if let [k] = chunk[..] {
                    own[k] = true;
                }
                chunk.clear();
            }
        }
    }
    let label: Vec<bool> = (0..heads.len())
        .map(|k| under[k].iter().all(|&v| v >= LABEL_STEP * sizes[k]))
        .collect();
    let shared = |k: usize| {
        (0..heads.len())
            .any(|j| j != k && sizes[j] < LABEL_STEP * sizes[k] && sizes[k] < LABEL_STEP * sizes[j])
    };
    for k in 0..heads.len() {
        if k > 0 && sizes[k] >= LABEL_STEP * sizes[k - 1] && !shared(k) && !own[k - 1] {
            return None;
        }
        // No label: the ink under it is of its own size.
        if !label[k] && under[k].iter().any(|&v| v >= LABEL_STEP * sizes[k]) {
            return None;
        }
    }
    label.contains(&true).then_some(heads)
}

/// The parts of a cell between dividers: when its first line is two labels or more over lines
/// of values, each label with the runs under it (see `under_labels`); else the cell whole.
/// None when a run under the labels has no certain label.
pub fn parts_of<R: LineRun>(runs: &[R], cell: &[usize]) -> Option<Vec<Vec<usize>>> {
    let lines = lines_of(runs, cell);
    let Some(labels) = first_line_labels(runs, &lines).filter(|l| l.len() >= 2) else {
        return Some(vec![cell.to_vec()]);
    };
    let rest: Vec<usize> = lines[1..].iter().flatten().copied().collect();
    let under = under_labels(runs, &labels, &rest)?;
    Some(
        labels
            .into_iter()
            .zip(under)
            .map(|(mut l, v)| {
                l.extend(v);
                l
            })
            .collect(),
    )
}

/// The labels of a cell by their text, for a loss: on each line the runs of the cell's
/// smallest size (raised markers aside), each label apart.
pub fn label_names<R: LineRun>(runs: &[R], cell: &[usize]) -> Vec<String> {
    let lines = lines_of(runs, cell);
    let marker = markers(runs, &lines);
    let small = cell
        .iter()
        .filter(|i| !marker.contains(i))
        .map(|&i| runs[i].size())
        .fold(f32::MAX, f32::min);
    lines
        .iter()
        .flat_map(|l| {
            let labels: Vec<usize> = l
                .iter()
                .copied()
                .filter(|&i| !marker.contains(&i) && runs[i].size() < LABEL_STEP * small)
                .collect();
            labels_in_line(runs, &labels)
        })
        .map(|l| words(runs, &l))
        .filter(|w| !w.is_empty())
        .collect()
}

/// The starts of the second and later labels of the first line of a cell (see
/// [`first_line_labels`]), each a little left of its first run: where a value line that runs
/// on into the next label's field is cut. None when the first line is not such labels.
pub(crate) fn label_starts<R: LineRun>(runs: &[R], cell: &[usize]) -> Vec<f32> {
    let ids: Vec<usize> = cell.iter().copied().filter(|&i| inked(&runs[i])).collect();
    let lines = lines_of(runs, &ids);
    let Some(labels) = first_line_labels(runs, &lines) else {
        return Vec::new();
    };
    labels
        .windows(2)
        .map(|w| {
            let (_, end) = extent(runs, &w[0]);
            let r = &runs[w[1][0]];
            // A character of the label left of it, or half the room to the label before.
            r.x() - (CHAR * r.size()).min(0.5 * (r.x() - end))
        })
        .collect()
}
