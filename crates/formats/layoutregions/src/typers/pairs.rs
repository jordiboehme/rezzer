// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typer of label/value groups.
//!
//! It reads a segment with the pair rules, one copy each: [`parse`] for the text of a drawn
//! box (a label set apart at its top or left, by size or by another style, over or before its
//! value), [`strip_cells`] for a row of fields between rules with dividers, [`key_values`] for
//! lines of labels and values, [`titles_over_values`] for two lines of titles over their
//! values. A label style learned from pairs seeds further pairs (a box whose label and value
//! differ in weight only) only when the seeding pairs had independent evidence: a label that
//! ends with a colon, or a value shaped as a number, a date or a code (see [`seeds`]). Lines
//! read as totals under a table are the totals' (see [`super::table::type_totals`]).

use crate::extract::pairs::blocks::{KeyValues, key_values};
use crate::extract::pairs::strips::{Kind as ValueKind, cells_of, kind_of, strip_cells};
use crate::extract::pairs::{
    Field, Parsed, Reading, Style, WEIGHT_ONLY, parse, small_size, style_of, touch_reach, touching,
    words,
};
use crate::extract::table::grid::{Piece, Shape, pieces_of, titles_over_values};
use crate::extract::table::ruled::edges;
use crate::extract::table::{RunRange, run_ranges};
use crate::input::TextRun;
use crate::region::Kind;
use crate::segment::{Segment, Source};
use crate::typers::{PageView, Typed, seg_runs, signal};

/// `label_value.fields`: the pair rules read the segment as fields.
pub const LABEL_FIELDS: u16 = 450;
/// `label_value.field_cells`: every cell with text of a grid (drawn boxes, or fields between
/// rules with dividers) reads as a field: the cells are fields, not a table's values.
pub const LABEL_FIELD_CELLS: u16 = 850;
/// `label_value.key_values` and `label_value.labels_over_values`: the lines are a block of
/// labels and values, or two lines of titles over their values (today's first readings).
pub const LABEL_KEY_VALUES: u16 = 850;
/// `label_value.label_style`: the label is set apart by a size step or another style at the
/// top or left of its value.
pub const LABEL_STYLE: u16 = 100;
/// `label_value.colon`: the labels end with a colon.
pub const LABEL_COLON: u16 = 150;
/// `label_value.value_shape`: the values are numbers, dates or codes.
pub const LABEL_VALUE_SHAPE: u16 = 150;
/// `label_value.boxes_touch`: the boxes of the fields touch in a row or a column.
pub const LABEL_BOXES_TOUCH: u16 = 300;
/// `label_value.dividers`: the fields stand between rules with dividers.
pub const LABEL_DIVIDERS: u16 = 100;
/// `label_value.known_style`: a pair of one size, its label in a style that pairs with
/// independent evidence show.
pub const LABEL_KNOWN_STYLE: u16 = 100;

/// Whether `text` is a value of a shape of its own: a number, a percent, a date, or a code
/// (digits and letters, every word with a digit: `RE-2026-0001`, not `Musterweg 1`).
pub(crate) fn value_shape(text: &str) -> bool {
    matches!(
        Shape::of(text),
        Some(Shape::Number | Shape::Percent | Shape::Date)
    ) || (kind_of(text) == ValueKind::Code
        && text
            .split_whitespace()
            .all(|w| w.chars().any(|c| c.is_ascii_digit())))
}

/// How a box is read as one field, as the boxes of a page are (see
/// [`crate::extract::pairs::box_fields`]): no known label styles yet.
fn box_reading(view: &PageView<'_>) -> Reading<'static> {
    Reading {
        one_field: true,
        smaller: false,
        small: small_size(view.text_sizes()).saturating_sub(1),
        labels: &[],
    }
}

/// The field of the text `ids` of `runs` in a box, read without known label styles; with
/// `seeds` too when it is a pair of one size whose label differs in weight only.
fn box_field<'v>(
    runs: &[&TextRun],
    ids: &[usize],
    view: &'v PageView<'_>,
    seeds: Option<&'v [Style]>,
) -> Option<(Field, bool)> {
    let how = box_reading(view);
    match parse(runs, ids, how) {
        Parsed::Fields(mut f) if f.len() == 1 => f.pop().map(|f| (f, false)),
        Parsed::Unsure(WEIGHT_ONLY) => {
            let seeds = seeds?;
            if seeds.is_empty() {
                return None;
            }
            let how = Reading {
                labels: seeds,
                ..how
            };
            match parse(runs, ids, how) {
                Parsed::Fields(mut f) if f.len() == 1 => f.pop().map(|f| (f, true)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The lines and their pieces of the runs with ink of `runs`.
fn lined<'r>(runs: &[&'r TextRun]) -> (Vec<&'r TextRun>, Vec<RunRange>, Vec<Vec<Piece>>) {
    let inked: Vec<&TextRun> = runs
        .iter()
        .copied()
        .filter(|r| !r.text.trim().is_empty() && !r.off_axis)
        .collect();
    let ranges = run_ranges(&inked);
    let pieces = ranges
        .iter()
        .map(|l| pieces_of(&inked, l.start, l.end))
        .collect();
    (inked, ranges, pieces)
}

/// The label styles that pairs with independent evidence show on the page: the label of a
/// box read as a field with a value, or of a line of a block of labels and values, whose label
/// ends with a colon or whose value is shaped as a number, a date or a code. A pair decided by
/// size alone, and a label alone in an empty field, seed nothing.
pub(crate) fn seeds(view: &PageView<'_>) -> Vec<Style> {
    let mut out: Vec<Style> = Vec::new();
    let mut seed = |runs: &[&TextRun], label: &[usize], value: &[usize]| {
        if label.is_empty() || value.is_empty() {
            return;
        }
        let colon = words(runs, label).ends_with(':');
        if colon || value_shape(&words(runs, value)) {
            let s = style_of(&runs[label[0]]);
            if !out.contains(&s) {
                out.push(s);
            }
        }
    };
    for seg in &view.segs.all {
        let (refs, _) = seg_runs(view, seg);
        if refs.is_empty() {
            continue;
        }
        crate::work::add(refs.len() as u64);
        match seg.source {
            Source::Box => {
                let ids: Vec<usize> = (0..refs.len()).collect();
                if let Some((f, false)) = box_field(&refs, &ids, view, None) {
                    seed(&refs, &f.label, &f.value);
                }
            }
            Source::Gutter | Source::Band | Source::Rule | Source::Style | Source::Rest
                if seg.children.is_empty() =>
            {
                let (runs, ranges, pieces) = lined(&refs);
                let off = vec![false; ranges.len()];
                if let KeyValues::Rows(rows) =
                    key_values(&runs, &ranges, 0, ranges.len(), &pieces, &off)
                {
                    for row in rows {
                        if let Some((s, e)) = row.value {
                            let label: Vec<usize> = (row.label.0..row.label.1).collect();
                            let value: Vec<usize> = (s..e).collect();
                            seed(&runs, &label, &value);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The signals a field adds: a label that ends with a colon, a value of its own shape.
fn field_signals(runs: &[&TextRun], fields: &[Field], signals: &mut Vec<crate::region::Signal>) {
    let filled: Vec<&Field> = fields.iter().filter(|f| !f.value.is_empty()).collect();
    if !fields.is_empty() && fields.iter().all(|f| words(runs, &f.label).ends_with(':')) {
        signals.push(signal(
            "label_value.colon",
            "the labels end with a colon".into(),
            LABEL_COLON,
        ));
    }
    if !filled.is_empty() && filled.iter().all(|f| value_shape(&words(runs, &f.value))) {
        signals.push(signal(
            "label_value.value_shape",
            "the values are numbers, dates or codes".into(),
            LABEL_VALUE_SHAPE,
        ));
    }
}

/// The box (x0, y0, x1, y1) of a segment.
fn frame(s: &Segment) -> [f32; 4] {
    [s.bounds.x0, s.bounds.y0, s.bounds.x1, s.bounds.y1]
}

/// Label/value: a box read as a field (`label_value.fields`, its label set apart:
/// `label_value.label_style`), touching another box in a row or a column
/// (`label_value.boxes_touch`); every cell of a grid read as a field
/// (`label_value.field_cells`), between rules with dividers (`label_value.dividers`); lines
/// read as a block of labels and values or as titles over their values
/// (`label_value.key_values`, `label_value.labels_over_values`); labels with a colon
/// (`label_value.colon`), values of their own shape (`label_value.value_shape`), a weight-only
/// pair read by a seeded style (`label_value.known_style`). Lines read as totals under a table
/// are none.
pub fn type_label_value(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    if view.totals_of(seg, |t| t.is_some()) {
        return None;
    }
    let (refs, _) = seg_runs(view, seg);
    if refs.iter().all(|r| r.text.trim().is_empty()) {
        return None;
    }
    let mut signals = Vec::new();
    match seg.source {
        Source::Box => {
            let ids: Vec<usize> = (0..refs.len()).collect();
            let (field, known) = box_field(&refs, &ids, view, Some(view.seeds()))?;
            if field.value.is_empty() {
                return None;
            }
            signals.push(signal(
                "label_value.fields",
                "the box holds a label and its value".into(),
                LABEL_FIELDS,
            ));
            signals.push(signal(
                "label_value.label_style",
                "its label is set apart at the top or left of its value".into(),
                LABEL_STYLE,
            ));
            field_signals(&refs, std::slice::from_ref(&field), &mut signals);
            if box_touches(view, seg, &refs) {
                signals.push(signal(
                    "label_value.boxes_touch",
                    "its box touches another box of a field in a row or a column".into(),
                    LABEL_BOXES_TOUCH,
                ));
            }
            if known {
                signals.push(signal(
                    "label_value.known_style",
                    "its label is in a style that pairs with a colon or a shaped value show".into(),
                    LABEL_KNOWN_STYLE,
                ));
            }
        }
        Source::Grid { rows, cols } => grid_fields(view, seg, (rows, cols), &mut signals)?,
        Source::Gutter | Source::Band | Source::Rule | Source::Style | Source::Rest
            if seg.children.is_empty() =>
        {
            let (runs, ranges, pieces) = lined(&refs);
            let off = vec![false; ranges.len()];
            if let KeyValues::Rows(rows) =
                key_values(&runs, &ranges, 0, ranges.len(), &pieces, &off)
            {
                signals.push(signal(
                    "label_value.key_values",
                    format!("{} lines of a label and its value", rows.len()),
                    LABEL_KEY_VALUES,
                ));
            } else if titles_over_values(&runs, &ranges, &pieces) {
                signals.push(signal(
                    "label_value.labels_over_values",
                    "a line of titles over a line of their values".into(),
                    LABEL_KEY_VALUES,
                ));
            } else {
                return None;
            }
        }
        _ => return None,
    }
    Typed::of(Kind::LabelValue, signals)
}

/// Whether the box `seg` touches a sibling box with text in a row or a column (see
/// [`touching`]), or stands in a grid of one row or one column.
fn box_touches(view: &PageView<'_>, seg: &Segment, refs: &[&TextRun]) -> bool {
    let Some(parent) = seg.parent.and_then(|p| view.segs.all.get(p.0 as usize)) else {
        return false;
    };
    if matches!(parent.source, Source::Grid { rows, cols } if rows == 1 || cols == 1) {
        return true;
    }
    let ids: Vec<usize> = (0..refs.len()).collect();
    let reach = touch_reach(refs, &ids);
    parent.children.iter().any(|c| {
        let Some(other) = view.segs.all.get(c.0 as usize) else {
            return false;
        };
        if other.id == seg.id || other.source != Source::Box {
            return false;
        }
        let (theirs, _) = seg_runs(view, other);
        let them: Vec<usize> = (0..theirs.len()).collect();
        let t = reach.max(touch_reach(&theirs, &them));
        !theirs.is_empty() && touching(frame(seg), frame(other), t)
    })
}

/// The fields of a Grid segment: each cell box with text read as a field, at least one with a
/// value; else, for a row of one, its cells between the grid's dividers read as fields (see
/// [`strip_cells`]).
fn grid_fields(
    view: &PageView<'_>,
    seg: &Segment,
    (rows, cols): (u32, u32),
    signals: &mut Vec<crate::region::Signal>,
) -> Option<()> {
    let (refs, _) = seg_runs(view, seg);
    let line = rows == 1 || cols == 1;
    // The cells of the grid that hold text, each read as a field.
    let mut fields: Vec<(Vec<&TextRun>, Field)> = Vec::new();
    let mut all = true;
    for c in &seg.children {
        let Some(cell) = view.segs.all.get(c.0 as usize) else {
            continue;
        };
        if cell.source != Source::Box {
            continue;
        }
        let (runs, _) = seg_runs(view, cell);
        if runs.iter().all(|r| r.text.trim().is_empty()) {
            continue;
        }
        let ids: Vec<usize> = (0..runs.len()).collect();
        match box_field(&runs, &ids, view, Some(view.seeds())) {
            Some((f, _)) => fields.push((runs, f)),
            None => all = false,
        }
    }
    if all && fields.len() >= 2 && fields.iter().any(|(_, f)| !f.value.is_empty()) {
        signals.push(signal(
            "label_value.field_cells",
            format!("{} cells each hold a label and its value", fields.len()),
            LABEL_FIELD_CELLS,
        ));
        if line {
            signals.push(signal(
                "label_value.boxes_touch",
                "the boxes of the fields touch in a row or a column".into(),
                LABEL_BOXES_TOUCH,
            ));
        }
        let colon = fields
            .iter()
            .all(|(r, f)| words(r, &f.label).ends_with(':'));
        let filled: Vec<&(Vec<&TextRun>, Field)> =
            fields.iter().filter(|(_, f)| !f.value.is_empty()).collect();
        if colon {
            signals.push(signal(
                "label_value.colon",
                "the labels end with a colon".into(),
                LABEL_COLON,
            ));
        }
        if filled.iter().all(|(r, f)| value_shape(&words(r, &f.value))) {
            signals.push(signal(
                "label_value.value_shape",
                "the values are numbers, dates or codes".into(),
                LABEL_VALUE_SHAPE,
            ));
        }
        return Some(());
    }
    // A row of fields between rules with dividers.
    if rows != 1 {
        return None;
    }
    let Ok(g) = &view.drawn.grids else {
        return None;
    };
    let near = |a: f32, b: f32| (a - b).abs() <= crate::segment::drawn::SNAP;
    let grp = g.groups.iter().find(|grp| {
        crate::segment::drawn::grid_frame(g, grp).is_some_and(|(f, _, _)| {
            near(f.x0, seg.bounds.x0)
                && near(f.x1, seg.bounds.x1)
                && near(f.y0, seg.bounds.y0)
                && near(f.y1, seg.bounds.y1)
        })
    })?;
    let h: Vec<_> = grp.0.iter().map(|&j| g.hs[j]).collect();
    let v: Vec<_> = grp.1.iter().map(|&k| g.vs[k]).collect();
    let (xs, _) = edges(&h, &v);
    if xs.len() < 3 {
        return None;
    }
    let ids: Vec<usize> = (0..refs.len())
        .filter(|&k| !refs[k].text.trim().is_empty() && !refs[k].off_axis)
        .collect();
    let cells = cells_of(&refs, &xs, &ids);
    let how = Reading {
        one_field: false,
        smaller: true,
        small: i32::MAX,
        labels: &[],
    };
    let read = strip_cells(&refs, &xs, &cells, how);
    if read.fields.is_empty() || read.fields.iter().all(|f| f.value.is_empty()) {
        return None;
    }
    let whole = read.left_out.is_empty() && read.fits;
    if whole {
        signals.push(signal(
            "label_value.field_cells",
            format!("{} fields fill the cells of the row", read.fields.len()),
            LABEL_FIELD_CELLS,
        ));
    } else {
        signals.push(signal(
            "label_value.fields",
            format!("{} fields in the cells of the row", read.fields.len()),
            LABEL_FIELDS,
        ));
    }
    signals.push(signal(
        "label_value.dividers",
        "the fields stand between rules with dividers".into(),
        LABEL_DIVIDERS,
    ));
    field_signals(&refs, &read.fields, signals);
    Some(())
}
