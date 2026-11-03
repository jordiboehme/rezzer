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
use crate::extract::table::grid::{Shape, titles_over_values};
use crate::extract::table::ruled::edges;
use crate::input::TextRun;
use crate::region::Kind;
use crate::segment::{Segment, Source};
use crate::typers::table::group_at;
use crate::typers::{Lined, PageView, Typed, lined, seg_runs, signal, text_alone};

/// `label_value.fields`: the pair rules read the segment as fields.
pub const LABEL_FIELDS: u16 = 450;
/// `label_value.field_cells`: every cell with text of a grid (drawn boxes, or fields between
/// rules with dividers) reads as a field: the cells are fields, not a table's values.
pub const LABEL_FIELD_CELLS: u16 = 850;
/// `label_value.key_values` and `label_value.labels_over_values`: the lines are a block of
/// labels and values, or two lines of titles over their values (today's first readings). Such
/// lines stand in two pieces or more, so a heading's reading of them is demoted as cells (at
/// most 650), and a list's reading reaches 750 at most.
pub const LABEL_KEY_VALUES: u16 = 900;
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
            _ if text_alone(view, seg) => {
                let Lined {
                    runs,
                    ranges,
                    pieces,
                    ..
                } = lined(&refs);
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
    view.pairs_of(seg)
}

/// [`type_label_value`] worked out (the view keeps it per segment).
pub(crate) fn label_value(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
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
            if view.touches(seg.id.0 as usize) {
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
        _ if text_alone(view, seg) => {
            let Lined {
                runs,
                ranges,
                pieces,
                ..
            } = lined(&refs);
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

/// For each segment of the view's tree, whether it is a box with text that touches another
/// box with text of its parent in a row or a column (see [`touching`], each box within its own
/// reach, see [`touch_reach`]), or a box in a grid of one row or one column. The boxes are put
/// in a grid of cells by their left and top edges once, so each box looks only at the boxes
/// near the two points where a touching box would stand.
pub(crate) fn touching_boxes(view: &PageView<'_>) -> Vec<bool> {
    let all = &view.segs.all;
    let mut out = vec![false; all.len()];
    // Each box with text: its index, frame and reach.
    let mut boxes: Vec<(usize, [f32; 4], f32)> = Vec::new();
    for (i, seg) in all.iter().enumerate() {
        if seg.source != Source::Box {
            continue;
        }
        let (refs, _) = seg_runs(view, seg);
        crate::work::add(refs.len() as u64 + 1);
        if refs.iter().all(|r| r.text.trim().is_empty()) {
            continue;
        }
        let in_line = seg.parent.and_then(|p| all.get(p.0 as usize)).is_some_and(
            |p| matches!(p.source, Source::Grid { rows, cols } if rows == 1 || cols == 1),
        );
        out[i] = in_line;
        let ids: Vec<usize> = (0..refs.len()).collect();
        boxes.push((i, frame(seg), touch_reach(&refs, &ids)));
    }
    let reach = boxes.iter().map(|b| b.2).fold(0.0f32, f32::max);
    // The boxes by the cell of a grid as fine as the reach their left and top edges stand in:
    // a box that touches another in a row has its left edge at that box's right edge and its
    // top at that box's top; in a column its left edge at that box's left edge and its top at
    // that box's bottom. Each look is at the nine cells around a point, so each box looks only
    // at boxes near it in both coordinates.
    // At least a point, so a page of text without size (a reach of 0) keeps finite keys.
    let step = (2.0 * reach).max(1.0);
    let key = |x: f32, y: f32| ((x / step).floor() as i64, (y / step).floor() as i64);
    let mut cells: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for (b, bx) in boxes.iter().enumerate() {
        cells.entry(key(bx.1[0], bx.1[3])).or_default().push(b);
    }
    for a in 0..boxes.len() {
        let (ia, fa, ra) = boxes[a];
        let parent = all[ia].parent;
        // Right of it (a row) and under it (a column).
        for (x, y) in [(fa[2], fa[3]), (fa[0], fa[1])] {
            let (kx, ky) = key(x, y);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(near) = cells.get(&(kx.saturating_add(dx), ky.saturating_add(dy)))
                    else {
                        continue;
                    };
                    for &b in near {
                        crate::work::add(1);
                        let (ib, fb, rb) = boxes[b];
                        if ib != ia && all[ib].parent == parent && touching(fa, fb, ra.max(rb)) {
                            out[ia] = true;
                            out[ib] = true;
                        }
                    }
                }
            }
        }
    }
    out
}

/// The cells of a grid read as fields: the runs of the segment and the fields, which name
/// runs of them.
pub(crate) struct CellFields<'a> {
    pub runs: Vec<&'a TextRun>,
    pub fields: Vec<Field>,
}

/// The fields of the cells of a Grid segment of the view: each square of its grid (its column
/// and row edges, see [`edges`]) that holds text, read as one field (see [`parse`], with the
/// page's seeds), when every one of them reads as a field, two or more and one with a value.
pub(crate) fn cell_fields<'a>(view: &PageView<'a>, seg: &Segment) -> Option<CellFields<'a>> {
    let Ok(g) = &view.drawn.grids else {
        return None;
    };
    let grp = group_at(g, &seg.bounds)?;
    let h: Vec<_> = grp.0.iter().map(|&j| g.hs[j]).collect();
    let v: Vec<_> = grp.1.iter().map(|&k| g.vs[k]).collect();
    let (xs, ys) = edges(&h, &v);
    if xs.len() < 2 || ys.len() < 2 {
        return None;
    }
    let (refs, _) = seg_runs(view, seg);
    crate::work::add(refs.len() as u64);
    let mut squares: std::collections::BTreeMap<(usize, usize), Vec<usize>> =
        std::collections::BTreeMap::new();
    for (k, r) in refs.iter().enumerate() {
        if r.text.trim().is_empty() || r.off_axis {
            continue;
        }
        let (cx, cy) = crate::extract::table::middle(r);
        let c = xs
            .partition_point(|e| *e < cx)
            .saturating_sub(1)
            .min(xs.len() - 2);
        let row = ys
            .partition_point(|e| *e > cy)
            .saturating_sub(1)
            .min(ys.len() - 2);
        squares.entry((row, c)).or_default().push(k);
    }
    if squares.len() < 2 {
        return None;
    }
    let mut fields = Vec::with_capacity(squares.len());
    for ids in squares.values() {
        let (f, _) = box_field(&refs, ids, view, Some(view.seeds()))?;
        fields.push(f);
    }
    fields
        .iter()
        .any(|f| !f.value.is_empty())
        .then_some(CellFields { runs: refs, fields })
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
    let line = rows == 1 || cols == 1;
    if let Some(cells) = cell_fields(view, seg) {
        signals.push(signal(
            "label_value.field_cells",
            format!(
                "{} cells each hold a label and its value",
                cells.fields.len()
            ),
            LABEL_FIELD_CELLS,
        ));
        if line {
            signals.push(signal(
                "label_value.boxes_touch",
                "the boxes of the fields touch in a row or a column".into(),
                LABEL_BOXES_TOUCH,
            ));
        }
        field_signals(&cells.runs, &cells.fields, signals);
        return Some(());
    }
    let (refs, _) = seg_runs(view, seg);
    // A row of fields between rules with dividers.
    if rows != 1 {
        return None;
    }
    let Ok(g) = &view.drawn.grids else {
        return None;
    };
    let grp = group_at(g, &seg.bounds)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::FileFacts;
    use crate::input::{FontKind, PageIn, RuleIn, Shift};
    use crate::limits::Limits;
    use crate::segment::drawn::drawn;
    use crate::segment::segment_page;
    use crate::typers::FileIndex;

    /// Boxes whose runs have no size have a reach of 0: the touch search keeps finite cell
    /// keys and does not overflow them (a panic in a debug build).
    #[test]
    fn boxes_of_text_without_size_do_not_overflow_the_touch_search() {
        let side = |x0: f32, y0: f32, x1: f32, y1: f32| RuleIn {
            x0,
            y0,
            x1,
            y1,
            thick: 0.5,
            dashed: false,
            rect: false,
        };
        let mut texts = Vec::new();
        let mut rules = Vec::new();
        for (k, x) in [72.0f32, 222.0].into_iter().enumerate() {
            texts.push(TextRun {
                text: "Feld".into(),
                x: x + 3.0,
                y: 683.0,
                w: 20.0,
                h: 0.0,
                size: 0.0,
                font: 1,
                font_kind: FontKind::Proportional,
                bold: false,
                italic: false,
                color: None,
                shift: Shift::Base,
                line: k as u32,
                off_axis: false,
                gaps: Vec::new(),
            });
            rules.extend([
                side(x, 660.0, x + 150.0, 660.0),
                side(x, 692.0, x + 150.0, 692.0),
                side(x, 660.0, x, 692.0),
                side(x + 150.0, 660.0, x + 150.0, 692.0),
            ]);
        }
        let page = PageIn {
            page: 1,
            size: [595.0, 842.0],
            turn: 0,
            texts,
            rules,
            boxes: Vec::new(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given: Vec::new(),
        };
        let facts = FileFacts::default();
        let index = FileIndex::new(&facts);
        let structure = drawn(&page, &|_| true);
        let segs = segment_page(&page, &Limits::default(), &|| false);
        let none = |_: &str| false;
        let view = PageView::new(&page, &structure, &segs, &facts, &index, &none);
        let touching = touching_boxes(&view);
        assert_eq!(touching.len(), segs.all.len());
    }
}
