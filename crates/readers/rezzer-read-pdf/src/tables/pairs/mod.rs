// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Labels and values of a form, read as tables of two columns: drawn boxes whose text is a
//! small label at the top or left and a value in another style (see [`boxes`]), rows of fields
//! between drawn rules (a header field grid of label rows over value rows, a block of labels
//! with their values beside them; see [`strips`]), and titles over a drawn bar with their lines
//! (see [`titles`]). This module reads the text of a box or a cell as fields and builds their
//! tables.

mod boxes;
mod strips;
mod titles;

use std::collections::HashSet;

use super::*;

pub(crate) use boxes::box_pairs;
pub(crate) use strips::{Frame, field_frames, form_drawn};
pub(super) use strips::{field_cuts, field_strips, frames, taken_by_grids};
pub(crate) use titles::underlined_titles;

/// Why a box beside fields is not read as a field.
const THREE_STYLES: &str = "it holds text in three styles or more";

const LABEL_BELOW: &str = "its label stands below its value";

const WEIGHT_ONLY: &str = "its label and its value differ in weight or slant only, so which is \
     the label is not clear";

/// A label is at most this many characters and words, a value of a field between rules at
/// most this many characters.
const LABEL_CHARS: usize = 40;

const LABEL_WORDS: usize = 6;

const VALUE_CHARS: usize = 80;

/// A box's label is at most this many lines.
const LABEL_LINES: usize = 2;

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
}

/// One field: the runs of its label and of its value (none: the field is empty).
#[derive(Debug, Default)]
pub(super) struct Field {
    pub label: Vec<usize>,
    pub value: Vec<usize>,
}

/// What the text of a box or of a cell between rules is.
#[derive(Debug)]
enum Parsed {
    Fields(Vec<Field>),
    /// No label and value: the text stays as it is.
    No,
    /// Not paired, for this reason.
    Unsure(&'static str),
}

/// A run's style: its size in half points, bold and italic.
type Style = (i32, bool, bool);

fn style_of(r: &Run) -> Style {
    ((r.size * 2.0).round() as i32, r.bold, r.italic)
}

fn inked(r: &Run) -> bool {
    r.text.chars().any(|c| !c.is_whitespace())
}

/// Whether `r` is a raised marker of the run `left` before it on its line (a footnote
/// reference of one or two characters, set smaller and higher): it goes with that run and
/// counts for no style of its own.
fn raised(r: &Run, left: &Run) -> bool {
    let t = r.text.trim();
    r.note
        || (!t.is_empty()
            && t.chars().count() <= 2
            && t.chars().all(|c| c.is_alphanumeric() || c == '*')
            && r.size <= 0.8 * left.size
            && r.y - left.y >= 0.2 * left.size)
}

/// The raised markers of `lines` (see [`raised`]): each goes with the run before it.
fn markers(flow: &Flow, lines: &[Vec<usize>]) -> HashSet<usize> {
    let mut marker: HashSet<usize> = HashSet::new();
    for line in lines {
        for w in line.windows(2) {
            if raised(&flow.runs[w[1]], &flow.runs[w[0]]) {
                marker.insert(w[1]);
            }
        }
    }
    marker
}

/// The runs `ids` in lines from the top, each line left to right: a run joins a line its
/// height overlaps by at least half of the smaller of the two.
fn lines_of(flow: &Flow, ids: &[usize]) -> Vec<Vec<usize>> {
    let extent = |r: &Run| (r.y - 0.25 * r.size, r.y + 0.75 * r.size);
    let mut order = ids.to_vec();
    order.sort_by(|&a, &b| flow.runs[b].y.total_cmp(&flow.runs[a].y));
    let mut lines: Vec<(Vec<usize>, (f32, f32))> = Vec::new();
    for i in order {
        let (lo, hi) = extent(&flow.runs[i]);
        if let Some((line, core)) = lines.last_mut() {
            let overlap = hi.min(core.1) - lo.max(core.0);
            let smaller = (hi - lo).min(core.1 - core.0).max(0.01);
            if overlap / smaller >= 0.5 {
                line.push(i);
                continue;
            }
        }
        lines.push((vec![i], (lo, hi)));
    }
    lines
        .into_iter()
        .map(|(mut l, _)| {
            l.sort_by(|&a, &b| flow.runs[a].x.total_cmp(&flow.runs[b].x));
            l
        })
        .collect()
}

/// How a box or a cell is read.
#[derive(Clone, Copy)]
struct Reading<'a> {
    /// A box: its text is one field, its label at most [`LABEL_LINES`] lines (lines of the
    /// label style one under the other are one label).
    one_field: bool,
    /// The label must be smaller than the value by half a point; else another weight or
    /// slant also sets it apart.
    smaller: bool,
    /// Text of one style under this size (half points) is labels only: empty fields.
    small: i32,
    /// Styles known as labels' on the page (the text alone in an empty boxed field): a label
    /// and a value of one size that differ in weight or slant only pair when the label's
    /// style is one of them; else which one is the label is not clear.
    labels: &'a [Style],
}

/// The fields the runs `ids` are: text of two styles, the first one met (at the top left)
/// the label's, smaller than the value or in another weight or slant; each label starts a
/// field and the value beside it on its line or on the lines under it is its value. Text of
/// one small style is labels of empty fields. Three styles or more, or a label after a value
/// in a box, is unsure.
fn parse(flow: &Flow, ids: &[usize], how: Reading) -> Parsed {
    let ids: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|&i| inked(&flow.runs[i]) && !flow.runs[i].off_axis)
        .collect();
    let lines = lines_of(flow, &ids);
    let marker = markers(flow, &lines);
    let mut styles: Vec<Style> = Vec::new();
    for &i in lines.iter().flatten() {
        let s = style_of(&flow.runs[i]);
        if !marker.contains(&i) && !styles.contains(&s) {
            styles.push(s);
        }
    }
    match styles.len() {
        0 => return Parsed::No,
        1 => {
            // Bold text alone in a box or between rules is a title, not the label of an empty
            // field: the text after it is its own.
            if styles[0].0 >= how.small || styles[0].1 {
                return Parsed::No;
            }
            // Labels only: each line an empty field, or in a box one label of a line or two.
            if how.one_field {
                if lines.len() > LABEL_LINES {
                    return Parsed::No;
                }
                return Parsed::Fields(vec![Field {
                    label: lines.into_iter().flatten().collect(),
                    value: Vec::new(),
                }]);
            }
            return Parsed::Fields(
                lines
                    .into_iter()
                    .map(|label| Field {
                        label,
                        value: Vec::new(),
                    })
                    .collect(),
            );
        }
        2 => {}
        _ => return Parsed::Unsure(THREE_STYLES),
    }
    let apart = |label: Style, value: Style| {
        value.0 - label.0 >= 1
            || (!how.smaller && (label.1, label.2) != (value.1, value.2) && label.0 <= value.0)
    };
    let (label, value) = (styles[0], styles[1]);
    if !apart(label, value) {
        // The label's style first met under the value's: the label stands below it.
        return if apart(value, label) {
            Parsed::Unsure(LABEL_BELOW)
        } else {
            Parsed::No
        };
    }
    // Of one size, weight or slant alone does not say which line is the label (a bold value
    // over a regular label reads like a bold label over a regular value): only a style known
    // as a label's does.
    if label.0 == value.0 {
        if how.labels.contains(&value) {
            return Parsed::Unsure(LABEL_BELOW);
        }
        if !how.labels.contains(&label) {
            return Parsed::Unsure(WEIGHT_ONLY);
        }
    }
    let is_label = |i: usize, prev: Option<bool>| {
        if marker.contains(&i) {
            prev.unwrap_or(false)
        } else {
            style_of(&flow.runs[i]) == label
        }
    };
    let mut fields: Vec<Field> = Vec::new();
    // The field being read, and whether its value has begun.
    let mut open: Option<(usize, bool)> = None;
    let mut label_lines = 0;
    for line in &lines {
        let mut kinds: Vec<bool> = Vec::with_capacity(line.len());
        for &i in line {
            let prev = kinds.last().copied();
            kinds.push(is_label(i, prev));
        }
        // A line is labels, then values: a label right of a value is no field.
        let first_value = kinds.iter().position(|k| !k).unwrap_or(kinds.len());
        if kinds[first_value..].iter().any(|k| *k) {
            return Parsed::Unsure(LABEL_BELOW);
        }
        let (labels, values) = line.split_at(first_value);
        if !labels.is_empty() {
            match open {
                // In a box, a second label line under the first, before any value, goes on it.
                Some((f, false)) if how.one_field && label_lines < LABEL_LINES => {
                    fields[f].label.extend_from_slice(labels);
                    label_lines += 1;
                }
                Some(_) if how.one_field => return Parsed::Unsure(LABEL_BELOW),
                _ => {
                    fields.push(Field {
                        label: labels.to_vec(),
                        value: Vec::new(),
                    });
                    label_lines = 1;
                }
            }
            if how.one_field && label_lines > LABEL_LINES {
                return Parsed::No;
            }
            open = Some((fields.len() - 1, false));
        }
        if !values.is_empty() {
            let Some((f, _)) = open else {
                // A value before any label: a label under it is unsure, none is no field.
                let later = lines
                    .iter()
                    .flatten()
                    .any(|&i| !marker.contains(&i) && style_of(&flow.runs[i]) == label);
                return if later {
                    Parsed::Unsure(LABEL_BELOW)
                } else {
                    Parsed::No
                };
            };
            fields[f].value.extend_from_slice(values);
            open = Some((f, true));
        }
    }
    Parsed::Fields(fields)
}

/// The text of runs, words joined by one space.
fn words(flow: &Flow, ids: &[usize]) -> String {
    let mut out = String::new();
    for &i in ids {
        for w in flow.runs[i].text.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(w);
        }
    }
    out
}

/// Whether a field's label and value are as short as a form's.
fn short(flow: &Flow, f: &Field) -> bool {
    let label = words(flow, &f.label);
    label.chars().count() <= LABEL_CHARS
        && label.split_whitespace().count() <= LABEL_WORDS
        && words(flow, &f.value).chars().count() <= VALUE_CHARS
}

/// A table of two columns, a label and its value in each row, for `fields` in order; `top` is
/// its top. An empty value is a grey `empty` status. Every table of labels and values is built
/// here: drawn boxes, rows of fields between rules and the totals of a column beside a section.
pub(super) fn pairs_table(
    flow: &Flow,
    fields: &[&Field],
    top: f32,
    note: &'static str,
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

/// The size under which text of one style is labels only (half points): half a point under
/// the largest of the page's text sizes; none on a page without text sizes.
fn small_size(text_sizes: &[f32]) -> i32 {
    text_sizes
        .last()
        .map_or(i32::MIN, |s| (s * 2.0).round() as i32)
}
