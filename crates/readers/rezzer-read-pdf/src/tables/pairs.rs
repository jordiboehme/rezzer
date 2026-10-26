// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Labels and values of a form, read as tables of two columns: drawn boxes whose text is a
//! small label at the top or left and a value in another style, and rows of fields between
//! drawn rules (a header field grid of label rows over value rows, a block of labels with
//! their values beside them).

use std::collections::{HashMap, HashSet};

use super::banded::heads_lines;
use super::*;
use crate::regions::{Index, Seg, grids, regions_of};

/// The note before the first table of drawn boxes of a page.
pub(crate) const BOX_FOUND_BY: &str = "Found by: label and value pairs from drawn boxes; the label \
     is the smaller text at the top of each box.";

/// The note before the first table of fields between rules of a page.
pub(super) const STRIP_FOUND_BY: &str = "Found by: label and value pairs from rows of fields \
     between drawn rules; each label is the smaller text and pairs with the value beside it or \
     under it.";

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

/// Lines a strip of fields between two rules may hold; a taller one is text, not fields.
const STRIP_LINES: usize = 40;

/// A divider of a strip is at least this share of the strip's height long: it parts the
/// fields over much of the row, where a drawn separator mark is a small share of it.
const DIVIDER: f32 = 0.25;

/// Boxes of one row or column of fields touch within this share of the size of their
/// smallest text: farther apart, they are boxes on their own. Relative to the text, so a form
/// printed larger or smaller chains the same.
const TOUCH: f32 = 0.5;

/// Steps the search for fields between rules may take on one page.
const STRIP_WORK: usize = 1_000_000;

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

/// Fields of drawn boxes (reading regions, in the frame of the runs): a box whose text is a
/// label at its top or left in one style (smaller by half a point or in another weight or
/// slant) and a value in another is a field, and a box with only a label smaller than the
/// page's largest text size an empty one. Fields of boxes next to each other in one row
/// (tops within [`SNAP`]) or one column (left edges within [`SNAP`]) make one table of two
/// columns, when one of them holds a value; a row of boxes that heads lines in its columns
/// below is a header band, not fields. Their runs leave `flow`. Also says, for boxes beside
/// fields that are not read as fields, why: a box of three styles or more, or one whose
/// label stands below its value. A box beside boxed fields whose text is bold alone is a
/// title: its runs leave `flow` as a [`BoxTitle`].
pub(crate) fn box_pairs(
    page: u32,
    flow: &mut Flow,
    boxes: &[[f32; 4]],
    rules: &[Rule],
    text_sizes: &[f32],
) -> (Vec<Planned>, Vec<BoxTitle>, Vec<String>) {
    if boxes.is_empty() || flow.runs.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let points: Vec<(f32, f32)> = flow
        .runs
        .iter()
        .map(|r| {
            if r.off_axis {
                (f32::NAN, f32::NAN)
            } else {
                (r.x + r.w.max(0.0) / 2.0, r.y + 0.25 * r.size)
            }
        })
        .collect();
    let Ok(regions) = regions_of(&points, boxes) else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let mut in_box: Vec<Vec<usize>> = vec![Vec::new(); boxes.len()];
    for (i, r) in regions.iter().enumerate() {
        if let Some(b) = r {
            in_box[*b as usize].push(i);
        }
    }
    // Framed boxes with a title over their lines, found by the frame: the frame's text is
    // what lies in it outside any box inside it with text.
    let framed = framed_titles(flow, boxes, &points, &regions, &in_box);
    let mut consumed = vec![false; boxes.len()];
    for f in &framed {
        for &b in &f.boxes {
            consumed[b] = true;
        }
    }
    let mut how = Reading {
        one_field: true,
        smaller: false,
        small: small_size(text_sizes).saturating_sub(1),
        labels: &[],
    };
    let mut fields: Vec<Option<Field>> = (0..boxes.len()).map(|_| None).collect();
    let mut unsure: Vec<(usize, &'static str)> = Vec::new();
    for (b, ids) in in_box.iter().enumerate() {
        if ids.is_empty() || consumed[b] {
            continue;
        }
        match parse(flow, ids, how) {
            Parsed::Fields(mut f) if f.len() == 1 => fields[b] = f.pop(),
            Parsed::Unsure(why) => unsure.push((b, why)),
            _ => {}
        }
    }
    // The style of the text alone in an empty boxed field is a label's: boxes whose label and
    // value differ in weight or slant only are read again knowing it.
    let mut known: Vec<Style> = fields
        .iter()
        .flatten()
        .filter(|f| f.value.is_empty())
        .filter_map(|f| f.label.first().map(|&i| style_of(&flow.runs[i])))
        .collect();
    known.sort_unstable();
    known.dedup();
    if !known.is_empty() && unsure.iter().any(|u| u.1 == WEIGHT_ONLY) {
        how.labels = &known;
        for (b, why) in &mut unsure {
            if *why != WEIGHT_ONLY {
                continue;
            }
            match parse(flow, &in_box[*b], how) {
                Parsed::Fields(mut f) if f.len() == 1 => {
                    fields[*b] = f.pop();
                    *why = "";
                }
                Parsed::Unsure(again) => *why = again,
                _ => *why = "",
            }
        }
        unsure.retain(|u| !u.1.is_empty());
    }
    // How near two boxes touch: by the size of their smallest text (see [`TOUCH`]).
    let touch: Vec<f32> = in_box
        .iter()
        .map(|ids| {
            let small = ids
                .iter()
                .filter(|&&i| inked(&flow.runs[i]))
                .map(|&i| flow.runs[i].size)
                .fold(f32::INFINITY, f32::min);
            if small.is_finite() {
                TOUCH * small
            } else {
                0.0
            }
        })
        .collect();
    let near = |p: usize, q: usize| {
        let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
        let row =
            (a[3] - b[3]).abs() <= t && ((a[2] - b[0]).abs() <= t || (b[2] - a[0]).abs() <= t);
        let column =
            (a[0] - b[0]).abs() <= t && ((a[1] - b[3]).abs() <= t || (b[1] - a[3]).abs() <= t);
        row || column
    };
    // Rows first: boxes left to right, each going on the row whose last box it touches.
    let mut order: Vec<usize> = (0..boxes.len()).filter(|&b| fields[b].is_some()).collect();
    order.sort_by(|&a, &b| boxes[a][0].total_cmp(&boxes[b][0]));
    let chains = |order: &[usize], next: &dyn Fn(usize, usize) -> bool| {
        let mut out: Vec<Vec<usize>> = Vec::new();
        for &b in order {
            match out.iter_mut().find(|c| next(*c.last().expect("a box"), b)) {
                Some(c) => c.push(b),
                None => out.push(vec![b]),
            }
        }
        out
    };
    let has_value = |c: &[usize]| {
        c.iter()
            .any(|&b| fields[b].as_ref().is_some_and(|f| !f.value.is_empty()))
    };
    let mut hs: Option<Vec<Seg>> = None;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut used = vec![false; boxes.len()];
    for c in chains(&order, &|p, q| {
        let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
        (a[3] - b[3]).abs() <= t && (a[2] - b[0]).abs() <= t
    }) {
        if c.len() < 2 || !has_value(&c) {
            continue;
        }
        // A row of boxes over lines in its columns heads a table.
        let mut xs: Vec<f32> = c.iter().map(|&b| boxes[b][0]).collect();
        xs.push(boxes[c[c.len() - 1]][2]);
        let b = [
            xs[0],
            c.iter().map(|&b| boxes[b][1]).fold(f32::MAX, f32::min),
            xs[xs.len() - 1],
            c.iter().map(|&b| boxes[b][3]).fold(f32::MIN, f32::max),
        ];
        for &k in &c {
            used[k] = true;
        }
        let hs = hs.get_or_insert_with(|| grids(rules).map(|g| g.hs).unwrap_or_default());
        if !heads_lines(flow, hs, b, &xs) {
            groups.push(c);
        }
    }
    let mut rest: Vec<usize> = order.into_iter().filter(|&b| !used[b]).collect();
    rest.sort_by(|&a, &b| boxes[b][3].total_cmp(&boxes[a][3]));
    for c in chains(&rest, &|p, q| {
        let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
        (a[0] - b[0]).abs() <= t && (a[1] - b[3]).abs() <= t
    }) {
        if c.len() >= 2 && has_value(&c) {
            groups.push(c);
        }
    }
    // A box that is no field beside the boxes of fields says why; so do boxes next to each
    // other whose label and value differ in weight only, which are fields but for that.
    let paired: Vec<usize> = groups.iter().flatten().copied().collect();
    let reasons: Vec<&str> = unsure
        .iter()
        .filter(|(b, why)| {
            paired.iter().any(|&p| near(*b, p))
                || (*why == WEIGHT_ONLY
                    && unsure
                        .iter()
                        .any(|(o, w)| o != b && *w == WEIGHT_ONLY && near(*b, *o)))
        })
        .map(|(_, why)| *why)
        .collect();
    let mut said: Vec<String> = Vec::new();
    for why in [THREE_STYLES, LABEL_BELOW, WEIGHT_ONLY] {
        let n = reasons.iter().filter(|w| **w == why).count();
        if n > 0 {
            let (s, verb) = if n == 1 { ("", "is") } else { ("es", "are") };
            let place = if why == WEIGHT_ONLY {
                "of label and value fields"
            } else {
                "beside label and value fields"
            };
            said.push(format!(
                "{n} drawn box{s} {place} on page {page} {verb} not read as a label and its \
                 value ({why}): the text is read as it stands"
            ));
        }
    }
    if groups.is_empty() && framed.is_empty() {
        return (Vec::new(), Vec::new(), said);
    }
    let mut planned: Vec<Planned> = Vec::new();
    let mut target: Vec<Option<(usize, usize, usize)>> = vec![None; flow.runs.len()];
    for c in &groups {
        let list: Vec<&Field> = c.iter().filter_map(|&b| fields[b].as_ref()).collect();
        let top = c.iter().map(|&b| boxes[b][3]).fold(f32::MIN, f32::max);
        if let Some(p) = pairs_table(flow, &list, top, BOX_FOUND_BY) {
            claim(&mut target, planned.len(), &list);
            planned.push(p);
        }
    }
    // Boxes beside the fields that hold bold text alone are titles, and so are the titles of
    // framed boxes, with their text.
    let mut titles: Vec<BoxTitle> = Vec::new();
    let mut title_of: Vec<Option<(usize, bool)>> = vec![None; flow.runs.len()];
    for f in &framed {
        for &i in &f.title {
            title_of[i] = Some((titles.len(), false));
        }
        for &i in &f.text {
            title_of[i] = Some((titles.len(), true));
        }
        let first = f.title.iter().chain(&f.text).min().copied().unwrap_or(0);
        titles.push(BoxTitle {
            at: flow.at[first],
            top: f.top,
            runs: Vec::new(),
            text: Vec::new(),
        });
    }
    for (b, ids) in in_box.iter().enumerate() {
        if consumed[b] {
            continue;
        }
        let inked_ids: Vec<usize> = ids
            .iter()
            .copied()
            .filter(|&i| inked(&flow.runs[i]))
            .collect();
        let bold = !inked_ids.is_empty()
            && inked_ids
                .iter()
                .all(|&i| flow.runs[i].bold && !flow.runs[i].off_axis)
            && lines_of(flow, &inked_ids).len() <= LABEL_LINES;
        if fields[b].is_some() || !bold || !paired.iter().any(|&p| near(b, p)) {
            continue;
        }
        for &i in ids {
            title_of[i] = Some((titles.len(), false));
        }
        titles.push(BoxTitle {
            at: flow.at[ids[0]],
            top: boxes[b][3],
            runs: Vec::new(),
            text: Vec::new(),
        });
    }
    // The runs of the tables and the titles leave the flow at once.
    let picked: Vec<usize> = (0..flow.runs.len())
        .filter(|&i| target[i].is_some() || title_of[i].is_some())
        .collect();
    let taken = flow.take(|i, _| target[i].is_some() || title_of[i].is_some());
    for ((run, _, _), i) in taken.into_iter().zip(picked) {
        match (target[i], title_of[i]) {
            (Some((t, r, c)), _) => planned[t].rows[r][c].runs.push(run),
            (None, Some((k, false))) => titles[k].runs.push(run),
            (None, Some((k, true))) => titles[k].text.push(run),
            (None, None) => {}
        }
    }
    (planned, titles, said)
}

/// Lines under an underlined title follow each other at most this many line heights
/// apart; past it the title's text ends.
const TITLE_TEXT_PITCH: f32 = 1.6;

/// Lines of text one underlined title may have.
const TITLE_TEXT_LINES: usize = 20;

/// Titles over a drawn bar: a line of bold text alone over a horizontal rule that no grid
/// takes, the rule under its baseline by at most the title's size, longer than the title and
/// starting within two of its sizes of the title's start; and under the rule, within its
/// stretch, the lines of one style larger than the title (by half a point, any weight) that
/// follow each other, the first within two line heights of the rule and no rule between
/// them. Each title is a heading where it stands and its lines its text; their runs leave
/// `flow`. `rules` are the page's drawn rules.
pub(crate) fn underlined_titles(flow: &mut Flow, rules: &[Rule]) -> Vec<BoxTitle> {
    // A line rule runs along one axis by how it is made: it is flat when it is wider than high.
    let flat = |r: &Rule| r.kind == RuleKind::Line && (r.y0 - r.y1).abs() < (r.x0 - r.x1).abs();
    let segs: Vec<Seg> = rules
        .iter()
        .filter(|r| flat(r))
        .map(|r| Seg {
            at: (r.y0 + r.y1) / 2.0,
            from: r.x0.min(r.x1),
            to: r.x0.max(r.x1),
        })
        .collect();
    if segs.is_empty() || flow.runs.is_empty() {
        return Vec::new();
    }
    let hs = crate::regions::merged(segs);
    // The rules of grids are a table's or a section's, not a title's.
    let taken: Vec<bool> = match grids(rules) {
        Ok(g) if !g.hs.is_empty() => {
            let mut t = vec![false; hs.len()];
            for (gh, _) in &g.groups {
                for &j in gh {
                    let s = g.hs[j];
                    for (k, h) in hs.iter().enumerate() {
                        if (h.at - s.at).abs() <= SNAP && h.from < s.to && h.to > s.from {
                            t[k] = true;
                        }
                    }
                }
            }
            t
        }
        _ => vec![false; hs.len()],
    };
    let lines = flow_lines(flow);
    let by_y = ByY::new(flow, &lines);
    let mut used = vec![false; flow.runs.len()];
    let mut found: Vec<(Vec<usize>, Vec<usize>, f32)> = Vec::new();
    for (k, bar) in hs.iter().enumerate() {
        if taken[k] {
            continue;
        }
        let in_range = |i: usize| {
            let (cx, _) = middle(&flow.runs[i]);
            cx > bar.from - SNAP && cx < bar.to + SNAP
        };
        let usable = |i: usize, used: &[bool]| {
            let r = &flow.runs[i];
            !used[i] && !r.off_axis && inked(r)
        };
        // The title: the lowest line over the bar within its size.
        let near = by_y.runs_between(bar.at, bar.at + 3.0 * MAX_CAPTION_SIZE);
        let over: Vec<usize> = near
            .into_iter()
            .filter(|&i| {
                let r = &flow.runs[i];
                usable(i, &used) && in_range(i) && r.y > bar.at && r.y - bar.at <= r.size
            })
            .collect();
        let Some(&low) = over
            .iter()
            .min_by(|&&a, &&b| flow.runs[a].y.total_cmp(&flow.runs[b].y))
        else {
            continue;
        };
        let title: Vec<usize> = lines_of(flow, &over)
            .into_iter()
            .find(|l| l.contains(&low))
            .unwrap_or_default();
        let size = title
            .iter()
            .map(|&i| flow.runs[i].size)
            .fold(0.0f32, f32::max);
        let x0 = title
            .iter()
            .map(|&i| flow.runs[i].x)
            .fold(f32::MAX, f32::min);
        let x1 = title
            .iter()
            .map(|&i| flow.runs[i].x + flow.runs[i].w)
            .fold(f32::MIN, f32::max);
        // Nothing else stands on the title's line within the bar's stretch.
        let line_y = flow.runs[low].y;
        let alone = by_y
            .runs_between(line_y - size, line_y + size)
            .into_iter()
            .filter(|&i| in_range(i) && inked(&flow.runs[i]))
            .all(|i| title.contains(&i) || (flow.runs[i].y - line_y).abs() > 0.3 * size);
        if title.is_empty()
            || !alone
            || !title.iter().all(|&i| flow.runs[i].bold)
            || x1 - x0 >= bar.to - bar.from
            || (x0 - bar.from).abs() > 2.0 * size
        {
            continue;
        }
        // Its text: the lines under the bar within its stretch.
        let below: Vec<usize> = by_y
            .runs_between(
                bar.at - 2.0 * TITLE_TEXT_LINES as f32 * size * TITLE_TEXT_PITCH,
                bar.at,
            )
            .into_iter()
            .filter(|&i| usable(i, &used) && in_range(i))
            .collect();
        let text_lines = lines_of(flow, &below);
        let mut text: Vec<usize> = Vec::new();
        let mut last = bar.at;
        let mut style: Option<Style> = None;
        for l in text_lines.iter().take(TITLE_TEXT_LINES) {
            let y = flow.runs[l[0]].y;
            let h = l.iter().map(|&i| flow.runs[i].size).fold(0.0f32, f32::max);
            let gap = last - y;
            let first = text.is_empty();
            let limit = if first { 2.0 * h } else { TITLE_TEXT_PITCH * h };
            // A rule between the lines ends the text.
            let ruled = hs
                .iter()
                .any(|s| s.at < last - SNAP && s.at > y && s.from < bar.to && s.to > bar.from);
            let styles: Vec<Style> = l.iter().map(|&i| style_of(&flow.runs[i])).collect();
            let one = styles.iter().all(|s| *s == styles[0]);
            if gap > limit || ruled || !one || style.is_some_and(|s| s != styles[0]) {
                break;
            }
            if styles[0].0 as f32 / 2.0 - size < 0.5 {
                break;
            }
            style = Some(styles[0]);
            text.extend(l.iter().copied());
            last = y;
        }
        if text.is_empty() {
            continue;
        }
        for &i in title.iter().chain(&text) {
            used[i] = true;
        }
        let top = title
            .iter()
            .map(|&i| flow.runs[i].y + flow.runs[i].size)
            .fold(f32::MIN, f32::max);
        found.push((title, text, top));
    }
    if found.is_empty() {
        return Vec::new();
    }
    let mut owner: Vec<Option<(usize, bool)>> = vec![None; flow.runs.len()];
    let mut out: Vec<BoxTitle> = Vec::with_capacity(found.len());
    for (k, (title, text, top)) in found.iter().enumerate() {
        for &i in title {
            owner[i] = Some((k, false));
        }
        for &i in text {
            owner[i] = Some((k, true));
        }
        let first = title.iter().chain(text).min().copied().unwrap_or(0);
        out.push(BoxTitle {
            at: flow.at[first],
            top: *top,
            runs: Vec::new(),
            text: Vec::new(),
        });
    }
    let picked: Vec<usize> = (0..flow.runs.len())
        .filter(|&i| owner[i].is_some())
        .collect();
    let taken_runs = flow.take(|i, _| owner[i].is_some());
    for ((run, _, _), i) in taken_runs.into_iter().zip(picked) {
        match owner[i] {
            Some((k, false)) => out[k].runs.push(run),
            Some((k, true)) => out[k].text.push(run),
            None => {}
        }
    }
    out
}

/// A framed box read as a title over its lines: the boxes it takes (the frame and a title
/// box in it), the runs of its title and of its text, and the frame's top.
struct Framed {
    boxes: Vec<usize>,
    title: Vec<usize>,
    text: Vec<usize>,
    top: f32,
}

/// The framed boxes of a page that hold a title over lines of text: a bold line or two at the
/// top of the frame (or alone in a box in the frame above its lines), smaller than the lines
/// under it, which are in one style, none of them bold, two lines or more. The frame's own
/// text is what lies in it outside its boxes with text: runs `regions` gives no box (the
/// frame holds a box with text, so it is no region of its own) whose smallest holder it is,
/// and runs of the frame itself. `points` are the runs' ink middles.
fn framed_titles(
    flow: &Flow,
    boxes: &[[f32; 4]],
    points: &[(f32, f32)],
    regions: &[Option<u32>],
    in_box: &[Vec<usize>],
) -> Vec<Framed> {
    let Some(mut index) = Index::new(boxes) else {
        return Vec::new();
    };
    let area = |b: [f32; 4]| (b[2] - b[0]) * (b[3] - b[1]);
    let mut own: Vec<Vec<usize>> = in_box.to_vec();
    for (i, r) in regions.iter().enumerate() {
        if r.is_some() || !inked(&flow.runs[i]) {
            continue;
        }
        let (x, y) = points[i];
        let mut best: Option<u32> = None;
        index.holders(x, y, |b| {
            if best.is_none_or(|o| area(boxes[b as usize]) < area(boxes[o as usize])) {
                best = Some(b);
            }
        });
        if index.work > crate::regions::BOX_WORK {
            return Vec::new();
        }
        if let Some(b) = best {
            own[b as usize].push(i);
        }
    }
    let bold_lines = |ids: &[usize]| -> Option<Vec<Vec<usize>>> {
        let ids: Vec<usize> = ids
            .iter()
            .copied()
            .filter(|&i| inked(&flow.runs[i]))
            .collect();
        let lines = lines_of(flow, &ids);
        (!lines.is_empty()
            && lines.len() <= LABEL_LINES
            && lines.iter().flatten().all(|&i| flow.runs[i].bold))
        .then_some(lines)
    };
    // The text under a title: lines of one style, none bold, larger than the title.
    let text_of = |lines: &[Vec<usize>], title_size: f32| {
        let mut styles: Vec<Style> = lines
            .iter()
            .flatten()
            .map(|&i| style_of(&flow.runs[i]))
            .collect();
        styles.sort_unstable();
        styles.dedup();
        lines.len() >= 2
            && styles.len() == 1
            && !styles[0].1
            && styles[0].0 as f32 / 2.0 - title_size >= 0.5
    };
    let mut out = Vec::new();
    for (b, ids) in own.iter().enumerate() {
        let mut ids = ids.clone();
        ids.retain(|&i| inked(&flow.runs[i]) && !flow.runs[i].off_axis);
        if ids.is_empty() {
            continue;
        }
        let fb = boxes[b];
        let lines = lines_of(flow, &ids);
        let size = |l: &Vec<usize>| l.iter().map(|&i| flow.runs[i].size).fold(0.0f32, f32::max);
        // The title at the top of the frame itself.
        let lead = lines
            .iter()
            .take_while(|l| l.iter().all(|&i| flow.runs[i].bold))
            .count();
        if (1..=LABEL_LINES).contains(&lead) {
            let title_size = lines[..lead].iter().map(size).fold(0.0f32, f32::max);
            if text_of(&lines[lead..], title_size) {
                out.push(Framed {
                    boxes: vec![b],
                    title: lines[..lead].iter().flatten().copied().collect(),
                    text: lines[lead..].iter().flatten().copied().collect(),
                    top: fb[3],
                });
                continue;
            }
        }
        // A title in a box of its own at the top of the frame.
        if lead > 0 {
            continue;
        }
        // The top of the frame's own lines: the title box stands above them.
        let lines_top = ids
            .iter()
            .map(|&i| flow.runs[i].y + 0.75 * flow.runs[i].size)
            .fold(f32::MIN, f32::max);
        let inner = (0..boxes.len()).find(|&t| {
            let tb = boxes[t];
            t != b
                && tb[0] >= fb[0] - SNAP
                && tb[2] <= fb[2] + SNAP
                && tb[3] <= fb[3] + SNAP
                && tb[1] >= lines_top - SNAP
                && bold_lines(&own[t]).is_some()
        });
        let Some(t) = inner else {
            continue;
        };
        let title = bold_lines(&own[t]).unwrap_or_default();
        let title_size = title.iter().map(size).fold(0.0f32, f32::max);
        if text_of(&lines, title_size) {
            out.push(Framed {
                boxes: vec![b, t],
                title: title.into_iter().flatten().collect(),
                text: ids,
                top: fb[3],
            });
        }
    }
    out
}

/// One strip of fields between two rules: its rules' heights, its left and right ends, its
/// dividers, and its fields by cell, left to right, read with each label over or left of its
/// value.
struct Strip {
    top: f32,
    bottom: f32,
    x0: f32,
    x1: f32,
    dividers: Vec<f32>,
    fields: Vec<Field>,
    /// Its text as one line of labels over one line of values (see [`two_lines`]).
    rows: Option<(Vec<usize>, Vec<usize>)>,
    /// A field of it was left as text.
    left_out: bool,
}

/// Where fields between rules may stand: a rule no grid takes and the nearest rule below it
/// bound the strip, over all of the upper rule when the lower one covers it, over the lower
/// one's stretch when it lies within the upper one; dividers in it that reach its top or its
/// bottom part it.
#[derive(Debug, Clone)]
pub(crate) struct Frame {
    pub x0: f32,
    pub x1: f32,
    pub bottom: f32,
    pub top: f32,
    /// The places of its dividers, left to right.
    pub dividers: Vec<f32>,
    /// The low and high end of each of its dividers.
    pub spans: Vec<(f32, f32)>,
}

/// The frames of fields between rules among the page's `rules` (see [`frames`]); none on a
/// page without rules of both directions or with too many of them.
pub(crate) fn field_frames(rules: &[Rule]) -> Vec<Frame> {
    let Ok(g) = grids(rules) else {
        return Vec::new();
    };
    frames(&g.hs, &taken_by_grids(g.hs.len(), &g.groups), &g.vs).unwrap_or_default()
}

/// Whether the page's drawing shows a form: a drawn box its text is read in (`boxes`), a grid
/// of rules, or a frame of fields between rules with a divider (see [`frames`]). An underlined
/// title is looked for only there: on a page of prose a heading over a rule is a heading of
/// the prose.
pub(crate) fn form_drawn(rules: &[Rule], boxes: &[[f32; 4]]) -> bool {
    if !boxes.is_empty() {
        return true;
    }
    let Ok(g) = grids(rules) else {
        return false;
    };
    !g.groups.is_empty()
        || frames(&g.hs, &taken_by_grids(g.hs.len(), &g.groups), &g.vs)
            .is_some_and(|f| f.iter().any(|f| !f.dividers.is_empty()))
}

/// Which of `n` horizontal lines the grids of `groups` take.
pub(super) fn taken_by_grids(n: usize, groups: &[(Vec<usize>, Vec<usize>)]) -> Vec<bool> {
    let mut taken = vec![false; n];
    for (gh, _) in groups {
        for &j in gh {
            taken[j] = true;
        }
    }
    taken
}

/// The frames of the page's horizontal lines `hs` (sorted by height) that no grid takes
/// (`taken`), with the dividers among the vertical lines `vs` (sorted by place); `None` past
/// [`STRIP_WORK`] steps.
pub(super) fn frames(hs: &[Seg], taken: &[bool], vs: &[Seg]) -> Option<Vec<Frame>> {
    let mut work = 0usize;
    let mut out = Vec::new();
    for (j, up) in hs.iter().enumerate() {
        if taken[j] {
            continue;
        }
        // The nearest rule below that reaches under it, and it must cover it.
        let mut below: Option<&Seg> = None;
        for s in hs[..j].iter().rev() {
            work += 1;
            if work > STRIP_WORK {
                return None;
            }
            if s.at < up.at - SNAP && s.to > up.from + SNAP && s.from < up.to - SNAP {
                below = Some(s);
                break;
            }
        }
        let Some(low) = below else {
            continue;
        };
        // A lower rule that covers the upper one bounds all of it; one that lies within it
        // (a row under part of a longer rule) bounds the stretch both share.
        let (x0, x1) = if low.from <= up.from + SNAP && low.to >= up.to - SNAP {
            (up.from, up.to)
        } else if low.from >= up.from - SNAP && low.to <= up.to + SNAP {
            (low.from, low.to)
        } else {
            continue;
        };
        let (bottom, top) = (low.at, up.at);
        let lo = vs.partition_point(|v| v.at <= x0 + SNAP);
        let (mut dividers, mut spans) = (Vec::new(), Vec::new());
        for v in &vs[lo..] {
            work += 1;
            if work > STRIP_WORK {
                return None;
            }
            if v.at >= x1 - SNAP {
                break;
            }
            let reaches = (v.to - top).abs() <= SNAP || (v.from - bottom).abs() <= SNAP;
            if reaches
                && v.to - v.from >= DIVIDER * (top - bottom)
                && v.from >= bottom - SNAP
                && v.to <= top + SNAP
            {
                dividers.push(v.at);
                spans.push((v.from, v.to));
            }
        }
        out.push(Frame {
            x0,
            x1,
            bottom,
            top,
            dividers,
            spans,
        });
    }
    Some(out)
}

/// Why fields between rules are not paired: a value over a field edge, a value under none of
/// the labels of its part of the row or under two, labels that do not stand over or beside
/// their values, and labels under the rule their values stand on whose values are not all
/// clear.
const ACROSS: &str = "a value stands across two fields";
const UNDER_NONE: &str = "a value stands under none of the labels of its field or under two";
const NOT_FIELDS: &str = "its labels do not all stand over or beside their values";
const UNDER_UNCLEAR: &str = "its labels stand under the rule their values stand on, and not \
     every value's label is clear";

/// The width of a character of a data font, as a share of its size: a value line padded
/// into its fields has one to three of them between two fields.
const CHAR: f32 = 0.6;

/// A word gap this many characters from a field edge or nearer is where the value line is
/// cut for it.
const NEAR_EDGE: f32 = 3.0;

/// The edges of the fields of a frame: its dividers, and in each cell between them the start
/// of each later label of its first line (see [`label_starts`]); with the frame's ends.
fn field_edges(flow: &Flow, fr: &Frame, ids: &[usize]) -> Vec<f32> {
    let mut xs = vec![fr.x0];
    xs.extend(&fr.dividers);
    xs.push(fr.x1);
    let mut edges = xs.clone();
    for cell in cells_of(flow, &xs, ids) {
        edges.extend(label_starts(flow, &cell));
    }
    edges.sort_by(f32::total_cmp);
    edges
}

/// The runs `ids` by the cell between the edges `xs` their middle lies in.
fn cells_of(flow: &Flow, xs: &[f32], ids: &[usize]) -> Vec<Vec<usize>> {
    let mut cells: Vec<Vec<usize>> = vec![Vec::new(); xs.len() - 1];
    for &i in ids {
        let (cx, _) = middle(&flow.runs[i]);
        let c = xs
            .partition_point(|e| *e < cx)
            .saturating_sub(1)
            .min(cells.len() - 1);
        cells[c].push(i);
    }
    cells
}

/// The runs of a frame: their middle inside it, with ink, in the page's direction.
fn frame_runs(flow: &Flow, by_y: &ByY, fr: &Frame, busy: &dyn Fn(usize) -> bool) -> Vec<usize> {
    by_y.runs_between(fr.bottom, fr.top)
        .into_iter()
        .filter(|&i| {
            let r = &flow.runs[i];
            let (cx, _) = middle(r);
            !busy(i) && !r.off_axis && inked(r) && cx > fr.x0 && cx < fr.x1
        })
        .collect()
}

/// Whether the run stands across the field edge `e`: more than a character of it on each
/// side.
fn across(r: &Run, e: f32) -> bool {
    let c = CHAR * r.size.max(1.0);
    e - r.x > c && r.x + r.w - e > c
}

/// Where to cut the runs of the page's frames of fields: a run that stands across a field
/// edge (see [`field_edges`]) is cut at its word gap nearest the edge, when that gap lies at
/// most [`NEAR_EDGE`] characters from it. Each run with its gaps, in the order of the flow.
pub(super) fn field_cuts(flow: &Flow, frames: &[Frame]) -> Vec<(usize, Vec<usize>)> {
    let lines = flow_lines(flow);
    let by_y = ByY::new(flow, &lines);
    let mut at: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut work = 0usize;
    for fr in frames {
        let ids = frame_runs(flow, &by_y, fr, &|_| false);
        work += ids.len();
        if work > STRIP_WORK {
            break;
        }
        if ids.is_empty() {
            continue;
        }
        let edges = field_edges(flow, fr, &ids);
        let inner = &edges[1..edges.len() - 1];
        if !holds_fields(flow, fr, &ids, inner) {
            continue;
        }
        for &i in &ids {
            let r = &flow.runs[i];
            for &e in inner.iter().filter(|&&e| across(r, e)) {
                let distance = |g: &crate::layout::Gap| {
                    if e < g.from {
                        g.from - e
                    } else if e > g.to {
                        e - g.to
                    } else {
                        0.0
                    }
                };
                let near = r
                    .gaps
                    .iter()
                    .enumerate()
                    .min_by(|a, b| distance(a.1).total_cmp(&distance(b.1)));
                if let Some((k, g)) = near
                    && distance(g) <= NEAR_EDGE * CHAR * r.size.max(1.0)
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

/// Whether the frame `fr` with the runs `ids` holds fields, judged before its runs are cut at
/// its field edges `inner`: its cells between dividers, without the runs that stand across an
/// edge, read as fields (see [`parse`]; labels alone are fields here) that are short, and each
/// run that stands across an edge is in pieces between the edges no longer than a value may
/// be. A line of prose under a smaller caption is too long for values, so it is not cut.
fn holds_fields(flow: &Flow, fr: &Frame, ids: &[usize], inner: &[f32]) -> bool {
    let how = Reading {
        one_field: false,
        smaller: true,
        small: i32::MAX,
        labels: &[],
    };
    let crossed =
        |r: &Run| -> Vec<f32> { inner.iter().copied().filter(|&e| across(r, e)).collect() };
    let mut rest = Vec::new();
    for &i in ids {
        let r = &flow.runs[i];
        let edges = crossed(r);
        if edges.is_empty() {
            rest.push(i);
            continue;
        }
        let chars = r.text.chars().count().max(1);
        let per = r.w.max(0.0) / chars as f32;
        let mut cuts = vec![r.x];
        cuts.extend(edges);
        cuts.push(r.x + r.w);
        if cuts
            .windows(2)
            .any(|w| per > 0.0 && (w[1] - w[0]) / per > VALUE_CHARS as f32)
        {
            return false;
        }
    }
    let mut xs = vec![fr.x0];
    xs.extend(&fr.dividers);
    xs.push(fr.x1);
    cells_of(flow, &xs, &rest)
        .iter()
        .filter(|c| !c.is_empty())
        .all(|c| matches!(parse(flow, c, how), Parsed::Fields(f) if f.iter().all(|f| short(flow, f))))
}

/// Fields between drawn rules, in the `frames` of the page. A frame's dividers part it into
/// cells; a cell whose first line is two labels or more over lines of values is parted further,
/// each value going to the label it stands under (see [`parts_of`]). When the text of each
/// part is fields (see [`parse`]; the label smaller than its value by half a point) and short,
/// the strip is a row of fields. A cell a run stands across (a value line the dividers could
/// not cut at a word gap) or a value under no label of its own leaves its fields as text, and
/// a strip whose labels do not all stand over or beside their values leaves all of them as
/// text; a loss in `unsure` names them. Strips at the same heights are one row; rows one under
/// the other, each starting at the rule the one above ends at, make one table, when it has two
/// fields or more and one of them a value. Text just beyond such a group can show that its
/// labels stand under the rule their values stand on (see [`beyond`]): its fields are then
/// read that way (see [`pair_under`]), or left as text with a loss when that is not clear.
/// Runs `busy` names are not looked at. The runs of each table found go to `claimed` under
/// its place in `tables`.
#[allow(clippy::too_many_arguments)]
pub(super) fn field_strips(
    page: u32,
    flow: &Flow,
    by_y: &ByY,
    frames: &[Frame],
    busy: &dyn Fn(usize) -> bool,
    text_sizes: &[f32],
    claimed: &mut [Option<(usize, usize, usize)>],
    tables: &mut Vec<Planned>,
    unsure: &mut Vec<String>,
) {
    let how = Reading {
        one_field: false,
        smaller: true,
        small: small_size(text_sizes).saturating_sub(1),
        labels: &[],
    };
    let say = |unsure: &mut Vec<String>, names: &[String], why: &str| {
        let names = if names.is_empty() {
            String::new()
        } else {
            format!(" {}", names.join(", "))
        };
        unsure.push(format!(
            "the fields{names} of a row of fields on page {page} are not read as labels and \
             values ({why}): their text is read as it stands"
        ));
    };
    let mut strips: Vec<Strip> = Vec::new();
    for fr in frames {
        let ids = frame_runs(flow, by_y, fr, busy);
        if ids.is_empty() || lines_of(flow, &ids).len() > STRIP_LINES {
            continue;
        }
        // No text between the rules may run over the frame's ends: the frame would cut it.
        let over_end = by_y.runs_between(fr.bottom, fr.top).into_iter().any(|i| {
            let r = &flow.runs[i];
            inked(r) && !r.off_axis && (across(r, fr.x0) || across(r, fr.x1))
        });
        if over_end {
            continue;
        }
        let mut edges = vec![fr.x0];
        edges.extend(&fr.dividers);
        edges.push(fr.x1);
        let cells = cells_of(flow, &edges, &ids);
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
        for &i in &ids {
            let r = &flow.runs[i];
            if inner.iter().any(|&e| across(r, e)) {
                let c = CHAR * r.size.max(1.0);
                for flag in &mut over[cell_at(r.x + c)..=cell_at(r.x + r.w - c)] {
                    *flag = true;
                }
            }
        }
        let mut fields: Vec<Field> = Vec::new();
        let mut left_out: Vec<(&'static str, Vec<String>)> = Vec::new();
        let (mut fits, mut unread) = (true, false);
        for (c, cell) in cells.iter().enumerate().filter(|(_, c)| !c.is_empty()) {
            let why = if over[c] {
                Some(ACROSS)
            } else {
                match parts_of(flow, cell) {
                    Some(parts) => {
                        for part in parts {
                            match parse(flow, &part, how) {
                                Parsed::Fields(f) if f.iter().all(|f| short(flow, f)) => {
                                    fields.extend(f);
                                }
                                Parsed::Unsure(_) => unread = true,
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
                    Some(l) => l.1.extend(label_names(flow, cell)),
                    None => left_out.push((why, label_names(flow, cell))),
                }
            }
        }
        // Text that is not fields makes the strip no row of fields; labels and values in the
        // wrong places beside fields read right make it one that is not clear, and that is
        // said.
        if !fits {
            continue;
        }
        if unread {
            if !fields.is_empty() {
                let names: Vec<String> = cells.iter().flat_map(|c| label_names(flow, c)).collect();
                say(unsure, &names, NOT_FIELDS);
            }
            continue;
        }
        for (why, names) in &left_out {
            say(unsure, names, why);
        }
        if fields.is_empty() {
            continue;
        }
        strips.push(Strip {
            top: fr.top,
            bottom: fr.bottom,
            x0: fr.x0,
            x1: fr.x1,
            dividers: fr.dividers.clone(),
            fields,
            rows: two_lines(flow, &ids),
            left_out: !left_out.is_empty(),
        });
    }
    // Rows of strips at the same heights, top first, each left to right; rows one under the
    // other make a table.
    strips.sort_by(|a, b| b.top.total_cmp(&a.top).then(a.x0.total_cmp(&b.x0)));
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut last: Option<(f32, f32)> = None;
    for (k, s) in strips.iter().enumerate() {
        let same_row =
            last.is_some_and(|(t, b)| (t - s.top).abs() <= SNAP && (b - s.bottom).abs() <= SNAP);
        let below = last.is_some_and(|(_, b)| (b - s.top).abs() <= SNAP);
        match groups.last_mut() {
            Some(g) if same_row || below => g.push(k),
            _ => groups.push(vec![k]),
        }
        last = Some((s.top, s.bottom));
    }
    for g in &groups {
        overflow_back(flow, &mut strips, g);
    }
    // Runs beyond a group that a group took.
    let mut outside: HashSet<usize> = HashSet::new();
    for g in groups {
        let list: Vec<&Field> = g.iter().flat_map(|&k| &strips[k].fields).collect();
        if list.len() < 2 || list.iter().all(|f| f.value.is_empty()) {
            continue;
        }
        let free = |i: usize| !busy(i) && !outside.contains(&i);
        let (above, below) = beyond(flow, by_y, &free, &strips, &g);
        let under: Vec<Field>;
        let list: Vec<&Field> = if above.is_empty() && below.is_empty() {
            list
        } else {
            match pair_under(flow, &strips, &g, &above, &below) {
                Some(f) if f.len() >= 2 && f.iter().all(|f| short(flow, f)) => {
                    under = f;
                    outside.extend(above.iter().chain(&below));
                    under.iter().collect()
                }
                _ => {
                    let names: Vec<String> = list.iter().map(|f| words(flow, &f.label)).collect();
                    say(unsure, &names, UNDER_UNCLEAR);
                    continue;
                }
            }
        };
        let top = strips[g[0]].top;
        let Some(p) = pairs_table(flow, &list, top, STRIP_FOUND_BY) else {
            continue;
        };
        claim(claimed, tables.len(), &list);
        tables.push(p);
    }
}

/// What a value is, to tell the values of a column of fields apart: a number (digits with
/// separators), words (no digit), or a code of both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
    Words,
    Code,
}

fn kind_of(text: &str) -> Kind {
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
/// field's column (its label within a character of the same place) and none of them is of the
/// piece's kind, while the piece is of the kind of the value before it, the piece is that
/// value's overflow: it goes back to its field, and the next field is empty.
fn overflow_back(flow: &Flow, strips: &mut [Strip], g: &[usize]) {
    let column = |f: &Field| f.label.first().map(|&i| &flow.runs[i]);
    // The kinds of the values of the group by the place and size of their labels.
    let mut seen: Vec<(f32, f32, usize, Kind)> = Vec::new();
    for &k in g {
        for f in &strips[k].fields {
            if let (Some(l), false) = (column(f), f.value.is_empty()) {
                seen.push((l.x, l.size, k, kind_of(&words(flow, &f.value))));
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
            let (ra, rb) = (&flow.runs[va], &flow.runs[*vb]);
            let ch = CHAR * ra.size.max(1.0);
            let end = ra.x + ra.w;
            let touching = (ra.y - rb.y).abs() <= 0.3 * ra.size
                && rb.x - end <= 1.5 * ch
                && rb.x - end >= -ch
                && dividers.iter().any(|&d| d > end - ch && d < rb.x + ch);
            let Some(l) = column(b) else {
                continue;
            };
            let piece = kind_of(&rb.text);
            let others: Vec<Kind> = seen
                .iter()
                .filter(|s| s.2 != k && (s.0 - l.x).abs() <= CHAR * l.size.max(1.0))
                .map(|s| s.3)
                .collect();
            if touching
                && !others.is_empty()
                && !others.contains(&piece)
                && kind_of(&ra.text) == piece
            {
                a.value.push(*vb);
                b.value.clear();
            }
        }
    }
}

/// A strip's text as one line of labels over one line of values, each line of one style (a
/// raised marker aside) and the upper one smaller by half a point: the runs of each line. A
/// strip of one line of one style has no values. None when its text is not that.
fn two_lines(flow: &Flow, ids: &[usize]) -> Option<(Vec<usize>, Vec<usize>)> {
    let lines = lines_of(flow, ids);
    let marker = markers(flow, &lines);
    let style = |l: &[usize]| -> Option<Style> {
        let mut s = l
            .iter()
            .filter(|i| !marker.contains(i))
            .map(|&i| style_of(&flow.runs[i]));
        let first = s.next()?;
        s.all(|o| o == first).then_some(first)
    };
    match lines.as_slice() {
        [l] => style(l).map(|_| (l.clone(), Vec::new())),
        [a, b] => {
            let (sa, sb) = (style(a)?, style(b)?);
            (sb.0 - sa.0 >= 1).then(|| (a.clone(), b.clone()))
        }
        _ => None,
    }
}

/// Text just beyond the group `g` of `strips` that shows its labels stand under the rule
/// their values stand on (a signature line): runs in the style of the group's values just over
/// its top rule, as far over it as its values stand over their lower rules, with no run in the
/// style of its labels over them; and runs in the style of its labels just under its bottom
/// rule, as far under it as its labels stand under their upper rules, with no value under
/// them. Each within a strip at that rule, half a size of play, among the runs `free` allows.
/// Only a group whose every strip is one line of labels over one line of values has such
/// text; for the others both are empty.
fn beyond(
    flow: &Flow,
    by_y: &ByY,
    free: &dyn Fn(usize) -> bool,
    strips: &[Strip],
    g: &[usize],
) -> (Vec<usize>, Vec<usize>) {
    let none = (Vec::new(), Vec::new());
    let Some(rows) = g
        .iter()
        .map(|&k| strips[k].rows.as_ref().map(|r| (&strips[k], r)))
        .collect::<Option<Vec<_>>>()
    else {
        return none;
    };
    let lines = rows.iter().map(|(_, r)| (&r.0, &r.1)).collect::<Vec<_>>();
    let marker = markers(
        flow,
        &lines.iter().map(|(l, _)| (*l).clone()).collect::<Vec<_>>(),
    );
    let mut ls: Vec<Style> = Vec::new();
    let mut vs: Vec<Style> = Vec::new();
    // How far the labels stand under the rule over them, and the values over the rule under
    // them.
    let (mut a, mut b): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
    for (s, (labels, values)) in &rows {
        for &i in labels.iter().filter(|i| !marker.contains(i)) {
            ls.push(style_of(&flow.runs[i]));
            a.push(s.top - flow.runs[i].y);
        }
        for &i in values {
            vs.push(style_of(&flow.runs[i]));
            b.push(flow.runs[i].y - s.bottom);
        }
    }
    if vs.is_empty() || ls.is_empty() {
        return none;
    }
    let median = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let (a, b) = (median(&mut a), median(&mut b));
    let top = rows.iter().map(|(s, _)| s.top).fold(f32::MIN, f32::max);
    let bottom = rows.iter().map(|(s, _)| s.bottom).fold(f32::MAX, f32::min);
    let height = rows
        .iter()
        .map(|(s, _)| s.top - s.bottom)
        .fold(0.0f32, f32::max);
    let within = |i: usize, at: f32, top_rule: bool| {
        let (cx, _) = middle(&flow.runs[i]);
        rows.iter().any(|(s, _)| {
            let rule = if top_rule { s.top } else { s.bottom };
            (rule - at).abs() <= SNAP && cx > s.x0 && cx < s.x1
        })
    };
    let usable = |i: usize| {
        let r = &flow.runs[i];
        free(i) && inked(r) && !r.off_axis
    };
    let band = |lo: f32, hi: f32| -> Vec<usize> {
        by_y.runs_between(lo, hi)
            .into_iter()
            .filter(|&i| usable(i) && flow.runs[i].y > lo && flow.runs[i].y < hi)
            .collect()
    };
    let over = band(top, top + height);
    let above: Vec<usize> = over
        .iter()
        .copied()
        .filter(|&i| {
            let r = &flow.runs[i];
            vs.contains(&style_of(r))
                && within(i, top, true)
                && ((r.y - top) - b).abs() <= 0.5 * r.size
                && !over.iter().any(|&j| {
                    let o = &flow.runs[j];
                    o.y > r.y && ls.contains(&style_of(o)) && within(j, top, true)
                })
        })
        .collect();
    let under = band(bottom - height, bottom);
    let below: Vec<usize> = under
        .iter()
        .copied()
        .filter(|&i| {
            let r = &flow.runs[i];
            ls.contains(&style_of(r))
                && within(i, bottom, false)
                && ((bottom - r.y) - a).abs() <= 0.5 * r.size
                && !under.iter().any(|&j| {
                    let o = &flow.runs[j];
                    o.y < r.y && vs.contains(&style_of(o)) && within(j, bottom, false)
                })
        })
        .collect();
    (above, below)
}

/// The fields of the group `g` of `strips` read with each label under the rule its value
/// stands on: the labels at the top of each strip pair with the values over its top rule (at
/// the bottom of the strip over it, or among `above` for a strip at the top), and the values
/// at the bottom of a strip with no strip under it pair with the labels under its bottom rule
/// (among `below`), each row of a label line and a value line read by [`pair_row`]. None when
/// a strip is no line of labels over a line of values, a field of it was left out, a row is
/// not clear, or a value is left without its label.
fn pair_under(
    flow: &Flow,
    strips: &[Strip],
    g: &[usize],
    above: &[usize],
    below: &[usize],
) -> Option<Vec<Field>> {
    let near = |a: f32, b: f32| (a - b).abs() <= SNAP;
    let overlaps = |s: &Strip, t: &Strip| s.x0 < t.x1 && t.x0 < s.x1;
    let inside = |i: usize, s: &Strip| {
        let (cx, _) = middle(&flow.runs[i]);
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
        fields.extend(pair_row(flow, labels, &over_values, &dividers, s.x0, s.x1)?);
        used.extend(&over_values);
        let last = !g
            .iter()
            .any(|&o| near(strips[o].top, s.bottom) && overlaps(s, &strips[o]));
        if last && !values.is_empty() {
            let labels: Vec<usize> = below.iter().copied().filter(|&i| inside(i, s)).collect();
            fields.extend(pair_row(flow, &labels, values, &s.dividers, s.x0, s.x1)?);
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
fn pair_row(
    flow: &Flow,
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
        .any(|&i| inner.iter().any(|&e| across(&flow.runs[i], e)))
    {
        return None;
    }
    let by_x = |mut v: Vec<usize>| {
        v.sort_by(|&a, &b| flow.runs[a].x.total_cmp(&flow.runs[b].x));
        v
    };
    let mut out = Vec::new();
    for (ls, vs) in cells_of(flow, &edges, labels)
        .into_iter()
        .zip(cells_of(flow, &edges, values))
    {
        let labels = labels_in_line(flow, &ls);
        if labels.is_empty() {
            if !vs.is_empty() {
                return None;
            }
            continue;
        }
        let under = if labels.len() == 1 {
            vec![vs]
        } else {
            under_labels(flow, &labels, &vs)?
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
fn labels_in_line(flow: &Flow, line: &[usize]) -> Vec<Vec<usize>> {
    let mut sorted = line.to_vec();
    sorted.sort_by(|&a, &b| flow.runs[a].x.total_cmp(&flow.runs[b].x));
    let mut out: Vec<Vec<usize>> = Vec::new();
    for i in sorted {
        let r = &flow.runs[i];
        if let Some(label) = out.last_mut() {
            let prev = &flow.runs[*label.last().expect("a run")];
            if raised(r, prev) || r.x - (prev.x + prev.w) <= CELL_GAP * prev.size.max(1.0) {
                label.push(i);
                continue;
            }
        }
        out.push(vec![i]);
    }
    out
}

/// The left and right ends of the ink of runs.
fn extent(flow: &Flow, ids: &[usize]) -> (f32, f32) {
    ids.iter().fold((f32::MAX, f32::MIN), |(a, b), &i| {
        let r = &flow.runs[i];
        (a.min(r.x), b.max(r.x + r.w.max(0.0)))
    })
}

/// Each of `values` given to the one label of `labels` (left to right, on one line) it stands
/// under: its ink overlaps that label's and no other's, it reaches no more than a character
/// past the middle of the room to the label before, and the same to the label after unless it
/// starts where its label starts (a value set from its label's start may run on up to the next
/// label). A label set in the middle or at the right end of its field over values at the left
/// end has no value under it, so its field is not clear. None when a value stands under no
/// label or under two.
fn under_labels(flow: &Flow, labels: &[Vec<usize>], values: &[usize]) -> Option<Vec<Vec<usize>>> {
    let ends: Vec<(f32, f32)> = labels.iter().map(|l| extent(flow, l)).collect();
    let mut out = vec![Vec::new(); labels.len()];
    for &v in values {
        let r = &flow.runs[v];
        let (a, b) = (r.x, r.x + r.w.max(0.0));
        let mut under = ends.iter().enumerate().filter(|(_, e)| a < e.1 && b > e.0);
        let (k, _) = under.next()?;
        if under.next().is_some() {
            return None;
        }
        let lo = (k > 0).then(|| (ends[k - 1].1 + ends[k].0) / 2.0);
        let hi = (k + 1 < ends.len()).then(|| (ends[k].1 + ends[k + 1].0) / 2.0);
        let aligned = (a - ends[k].0).abs() <= CHAR * r.size.max(1.0);
        if lo.is_some_and(|e| across(r, e)) || (!aligned && hi.is_some_and(|e| across(r, e))) {
            return None;
        }
        out[k].push(v);
    }
    Some(out)
}

/// The labels of the first of the `lines` of a cell when that line is labels over values: in
/// the cell's smallest size (a raised marker aside), over lines larger by half a point.
fn first_line_labels(flow: &Flow, lines: &[Vec<usize>]) -> Option<Vec<Vec<usize>>> {
    let (Some(first), true) = (lines.first(), lines.len() >= 2) else {
        return None;
    };
    let size = |i: usize| flow.runs[i].size;
    let small = first.iter().map(|&i| size(i)).fold(f32::MAX, f32::min);
    let below = lines[1..]
        .iter()
        .flatten()
        .map(|&i| size(i))
        .fold(f32::MIN, f32::max);
    let marker = markers(flow, &lines[..1]);
    if below - small < 0.5
        || first
            .iter()
            .any(|&i| size(i) - small >= 0.5 && !marker.contains(&i))
    {
        return None;
    }
    Some(labels_in_line(flow, first))
}

/// The parts of a cell between dividers: when its first line is two labels or more over lines
/// of values, each label with the runs under it (see [`under_labels`]); else the cell whole.
/// None when a run under the labels has no certain label.
fn parts_of(flow: &Flow, cell: &[usize]) -> Option<Vec<Vec<usize>>> {
    let lines = lines_of(flow, cell);
    let Some(labels) = first_line_labels(flow, &lines).filter(|l| l.len() >= 2) else {
        return Some(vec![cell.to_vec()]);
    };
    let rest: Vec<usize> = lines[1..].iter().flatten().copied().collect();
    let under = under_labels(flow, &labels, &rest)?;
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
fn label_names(flow: &Flow, cell: &[usize]) -> Vec<String> {
    let lines = lines_of(flow, cell);
    let marker = markers(flow, &lines);
    let small = cell
        .iter()
        .filter(|i| !marker.contains(i))
        .map(|&i| flow.runs[i].size)
        .fold(f32::MAX, f32::min);
    lines
        .iter()
        .flat_map(|l| {
            let labels: Vec<usize> = l
                .iter()
                .copied()
                .filter(|&i| !marker.contains(&i) && flow.runs[i].size - small < 0.5)
                .collect();
            labels_in_line(flow, &labels)
        })
        .map(|l| words(flow, &l))
        .filter(|w| !w.is_empty())
        .collect()
}

/// The starts of the second and later labels of the first line of a cell (see
/// [`first_line_labels`]), each a little left of its first run: where a value line that runs
/// on into the next label's field is cut. None when the first line is not such labels.
fn label_starts(flow: &Flow, cell: &[usize]) -> Vec<f32> {
    let ids: Vec<usize> = cell
        .iter()
        .copied()
        .filter(|&i| inked(&flow.runs[i]))
        .collect();
    let lines = lines_of(flow, &ids);
    let Some(labels) = first_line_labels(flow, &lines) else {
        return Vec::new();
    };
    labels
        .windows(2)
        .map(|w| {
            let (_, end) = extent(flow, &w[0]);
            let r = &flow.runs[w[1][0]];
            // A character of the label left of it, or half the room to the label before.
            r.x - (CHAR * r.size).min(0.5 * (r.x - end))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run of `text` at `x` on the baseline `y` in `size`, 0.6 of the size a character,
    /// with a gap at each space.
    fn run(text: &str, x: f32, y: f32, size: f32) -> Run {
        let ch = 0.6 * size;
        let mut gaps = Vec::new();
        let mut words = 0u32;
        for (k, c) in text.char_indices() {
            if c == ' ' {
                words += 1;
                gaps.push(crate::layout::Gap {
                    words,
                    from: x + ch * k as f32,
                    to: x + ch * (k + 1) as f32,
                });
            }
        }
        Run {
            page: 1,
            text: text.into(),
            x,
            y,
            w: ch * text.chars().count() as f32,
            h: size,
            size,
            bold: false,
            italic: false,
            mcid: None,
            uri: None,
            line: 0,
            off_axis: false,
            note: false,
            gaps: crate::layout::Gaps::new(gaps),
        }
    }

    fn flow_of(runs: Vec<Run>) -> Flow {
        let n = runs.len();
        Flow {
            runs,
            slots: vec![crate::layout::Slot::Free; n],
            at: (0..n).collect(),
        }
    }

    fn frame() -> Frame {
        Frame {
            x0: 0.0,
            x1: 700.0,
            bottom: 80.0,
            top: 130.0,
            dividers: Vec::new(),
            spans: Vec::new(),
        }
    }

    /// A prose line between two rules under a smaller caption line of two parts is no row of
    /// fields: it is too long for values, so it is not cut where the caption's second part
    /// starts.
    #[test]
    fn a_prose_line_under_a_caption_is_not_cut() {
        let mut caption = run("Abb. 1", 10.0, 120.0, 6.0);
        caption.line = 0;
        let mut source = run("Quelle: Muster", 200.0, 120.0, 6.0);
        source.line = 0;
        let mut prose = run(
            "Die Werte steigen im Lauf des Jahres stark an und fallen erst im Herbst wieder auf \
             den Stand des Vorjahres zurueck",
            10.0,
            100.0,
            10.0,
        );
        prose.line = 1;
        let flow = flow_of(vec![caption, source, prose]);
        assert!(field_cuts(&flow, &[frame()]).is_empty());
    }

    /// Guard: a value line of two fields under their labels is cut where the second label
    /// starts.
    #[test]
    fn a_value_line_under_two_labels_is_cut() {
        let flow = flow_of(vec![
            run("Name", 10.0, 120.0, 6.0),
            run("Ort", 100.0, 120.0, 6.0),
            {
                let mut r = run("Erika Mustermann Musterstadt", 10.0, 100.0, 10.0);
                r.line = 1;
                r
            },
        ]);
        assert_eq!(field_cuts(&flow, &[frame()]), [(2, vec![1])]);
    }
}
