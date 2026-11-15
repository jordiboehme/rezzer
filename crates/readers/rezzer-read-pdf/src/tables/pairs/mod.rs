// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Labels and values of a form, read as tables of two columns: drawn boxes whose text is a
//! small label at the top or left and a value in another style (see [`boxes`]), rows of fields
//! between drawn rules (a header field grid of label rows over value rows, a block of labels
//! with their values beside them; see [`strips`]), and titles over a drawn bar with their lines
//! (see [`titles`]). This module finds them and builds their tables; the engine reads the
//! text of a box or a cell as fields (see [`layoutregions::extract::pairs`]).

mod boxes;
mod strips;
mod titles;

use layoutregions::extract::pairs::strips::{
    BottomEnd, CHAR, FieldFrame, LINE_BY_LINE, LONE, NOT_FIELDS, STRIP_WORK, Strip, StripRead,
    TopEnd, UNDER_UNCLEAR, Way, across, beside_labels, beside_value, by_style, cells_of,
    first_labels, gutter_columns, in_role, label_names, labels_in_line, overflow_back, pair_under,
    parts_of, strip_cells, table_body, table_rows, totals_rows, two_lines, way_of,
};
use layoutregions::extract::pairs::{
    ADDRESS, Field, LABEL_BELOW, LABEL_CHARS, LABEL_LINES, LABEL_WORDS, NAMED_ADDRESS, NO_LABEL,
    Parsed, Reading, Style, THREE_STYLES, WEIGHT_ONLY, box_fields, inked, lines_of, markers, parse,
    same_style, short, small_size, style_of, words,
};

use super::*;

pub(crate) use boxes::box_pairs;
pub(crate) use strips::{Frame, field_frames, form_drawn};
pub(super) use strips::{
    Top, field_cuts, field_strips, frame_tops, frames, holds_values, taken_by_grids,
};
pub(crate) use titles::underlined_titles;

/// The title of a drawn box, read as a heading where the box stands: bold text alone in a box
/// beside boxed fields, or the bold line at the top of a framed box over lines of text, which
/// follow it as its text.
#[derive(Debug)]
pub(crate) struct BoxTitle {
    pub at: usize,
    pub top: f32,
    pub runs: Vec<Run>,
    /// The lines in the frame under the title.
    pub text: Vec<Run>,
    /// The ids of the runs of `runs` and `text` (see [`Flow::ids`]), in the order taken.
    pub ids: Vec<u32>,
    /// The ids of the runs of `text`, in the order taken.
    pub text_ids: Vec<u32>,
}

/// A table of two columns, a label and its value in each row, for `fields` in order; `top` is
/// its top. An empty value is a grey `empty` status. Every table of labels and values is built
/// here: drawn boxes, rows of fields between rules and the totals of a column beside a section,
/// each found `by` its detector.
pub(super) fn pairs_table(
    flow: &Flow,
    fields: &[&Field],
    top: f32,
    (note, by): (&'static str, Detector),
) -> Option<Planned> {
    let first = fields
        .iter()
        .flat_map(|f| f.label.iter().chain(&f.value))
        .min()?;
    let cell = |status: Option<&'static str>| PlannedCell {
        runs: Vec::new(),
        row_span: 1,
        col_span: 1,
        status,
    };
    Some(Planned {
        at: flow.at[*first],
        top,
        rows: fields
            .iter()
            .map(|f| vec![cell(None), cell(f.value.is_empty().then_some("empty"))])
            .collect(),
        trs: Vec::new(),
        additions: Vec::new(),
        header_rows: 0,
        tagged: None,
        continues: false,
        caption: Vec::new(),
        found_by: Some(note),
        kind: layoutregions::Kind::LabelValue,
        ids: Vec::new(),
        by,
        drawn: None,
        pictures: Vec::new(),
    })
}

/// The box around `boxes` (x0, y0, x1, y1); `None` for none.
pub(super) fn union(boxes: impl Iterator<Item = [f32; 4]>) -> Option<[f32; 4]> {
    boxes.reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}

/// Sends the runs of `fields`, the rows of the table `t`, to their cells in `claimed`.
pub(super) fn claim(claimed: &mut [Option<(usize, usize, usize)>], t: usize, fields: &[&Field]) {
    for (r, f) in fields.iter().enumerate() {
        for &i in &f.label {
            claimed[i] = Some((t, r, 0));
        }
        for &i in &f.value {
            claimed[i] = Some((t, r, 1));
        }
    }
}
