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

/// A label is at most this many characters and words, a value of a field between rules at
/// most this many characters.
const LABEL_CHARS: usize = 40;
const LABEL_WORDS: usize = 6;
const VALUE_CHARS: usize = 80;

/// A box's label is at most this many lines.
const LABEL_LINES: usize = 2;

/// Lines a strip of fields between two rules may hold; a taller one is text, not fields.
const STRIP_LINES: usize = 40;

/// A divider of a strip is at least this long: shorter strokes are marks.
const DIVIDER: f32 = 3.0 * SNAP;

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
struct Field {
    label: Vec<usize>,
    value: Vec<usize>,
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
struct Reading {
    /// A box: its text is one field, its label at most [`LABEL_LINES`] lines (lines of the
    /// label style one under the other are one label).
    one_field: bool,
    /// The label must be smaller than the value by half a point; else another weight or
    /// slant also sets it apart.
    smaller: bool,
    /// Text of one style under this size (half points) is labels only: empty fields.
    small: i32,
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
    // Each run's role: a marker goes with the run before it.
    let mut marker: HashSet<usize> = HashSet::new();
    for line in &lines {
        for w in line.windows(2) {
            if raised(&flow.runs[w[1]], &flow.runs[w[0]]) {
                marker.insert(w[1]);
            }
        }
    }
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
            // Bold text alone in a box is its title, not the label of an empty field: the
            // text after it is its own.
            if styles[0].0 >= how.small || (how.one_field && styles[0].1) {
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
/// its top. An empty value is a grey `empty` status.
fn pairs_table(flow: &Flow, fields: &[&Field], top: f32, note: &'static str) -> Option<Planned> {
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

/// Sends the runs of the fields of each table to its cells.
fn claim(target: &mut [Target], t: usize, fields: &[&Field]) {
    let t = u32::try_from(t).unwrap_or(u32::MAX);
    for (r, f) in fields.iter().enumerate() {
        for &i in &f.label {
            target[i] = Some((t, r as u32, 0));
        }
        for &i in &f.value {
            target[i] = Some((t, r as u32, 1));
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
    let how = Reading {
        one_field: true,
        smaller: false,
        small: small_size(text_sizes).saturating_sub(1),
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
    let near = |a: [f32; 4], b: [f32; 4]| {
        let row = (a[3] - b[3]).abs() <= SNAP
            && ((a[2] - b[0]).abs() <= SNAP || (b[2] - a[0]).abs() <= SNAP);
        let column = (a[0] - b[0]).abs() <= SNAP
            && ((a[1] - b[3]).abs() <= SNAP || (b[1] - a[3]).abs() <= SNAP);
        row || column
    };
    // Rows first: boxes left to right, each going on the row whose last box it touches.
    let mut order: Vec<usize> = (0..boxes.len()).filter(|&b| fields[b].is_some()).collect();
    order.sort_by(|&a, &b| boxes[a][0].total_cmp(&boxes[b][0]));
    let chains = |order: &[usize], next: &dyn Fn([f32; 4], [f32; 4]) -> bool| {
        let mut out: Vec<Vec<usize>> = Vec::new();
        for &b in order {
            match out
                .iter_mut()
                .find(|c| next(boxes[*c.last().expect("a box")], boxes[b]))
            {
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
    for c in chains(&order, &|a, b| {
        (a[3] - b[3]).abs() <= SNAP && (a[2] - b[0]).abs() <= SNAP
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
    for c in chains(&rest, &|a, b| {
        (a[0] - b[0]).abs() <= SNAP && (a[1] - b[3]).abs() <= SNAP
    }) {
        if c.len() >= 2 && has_value(&c) {
            groups.push(c);
        }
    }
    // A box that is no field beside the boxes of fields says why.
    let paired: Vec<usize> = groups.iter().flatten().copied().collect();
    let reasons: Vec<&str> = unsure
        .iter()
        .filter(|(b, _)| paired.iter().any(|&p| near(boxes[*b], boxes[p])))
        .map(|(_, why)| *why)
        .collect();
    let mut said: Vec<String> = Vec::new();
    for why in [THREE_STYLES, LABEL_BELOW] {
        let n = reasons.iter().filter(|w| **w == why).count();
        if n > 0 {
            let (s, verb) = if n == 1 { ("", "is") } else { ("es", "are") };
            said.push(format!(
                "{n} drawn box{s} beside label and value fields on page {page} {verb} not read \
                 as a label and its value ({why}): the text is read as it stands"
            ));
        }
    }
    if groups.is_empty() && framed.is_empty() {
        return (Vec::new(), Vec::new(), said);
    }
    let mut planned: Vec<Planned> = Vec::new();
    let mut target: Vec<Target> = vec![None; flow.runs.len()];
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
        if fields[b].is_some() || !bold || !paired.iter().any(|&p| near(boxes[b], boxes[p])) {
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
            (Some((t, r, c)), _) => planned[t as usize].rows[r as usize][c as usize]
                .runs
                .push(run),
            (None, Some((k, false))) => titles[k].runs.push(run),
            (None, Some((k, true))) => titles[k].text.push(run),
            (None, None) => {}
        }
    }
    (planned, titles, said)
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

/// One strip of fields between two rules: its rules' heights, its left and right ends, and
/// its fields by cell, left to right.
struct Strip {
    top: f32,
    bottom: f32,
    x0: f32,
    fields: Vec<Field>,
}

/// Where fields between rules may stand: a rule no grid takes and the nearest rule below that
/// covers it bound the strip, and dividers in it that reach its top or its bottom part it.
#[derive(Debug, Clone)]
pub(super) struct Frame {
    pub x0: f32,
    pub x1: f32,
    pub bottom: f32,
    pub top: f32,
    /// The places of its dividers, left to right.
    pub dividers: Vec<f32>,
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
        let Some(low) = below.filter(|s| s.from <= up.from + SNAP && s.to >= up.to - SNAP) else {
            continue;
        };
        let (x0, x1, bottom, top) = (up.from, up.to, low.at, up.at);
        let lo = vs.partition_point(|v| v.at <= x0 + SNAP);
        let mut dividers = Vec::new();
        for v in &vs[lo..] {
            work += 1;
            if work > STRIP_WORK {
                return None;
            }
            if v.at >= x1 - SNAP {
                break;
            }
            let reaches = (v.to - top).abs() <= SNAP || (v.from - bottom).abs() <= SNAP;
            if reaches && v.to - v.from >= DIVIDER && v.from >= bottom - SNAP && v.to <= top + SNAP
            {
                dividers.push(v.at);
            }
        }
        out.push(Frame {
            x0,
            x1,
            bottom,
            top,
            dividers,
        });
    }
    Some(out)
}

/// Why a row of fields that crosses its field edges is not paired.
const ACROSS: &str = "a value stands across two fields";

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

/// Fields between drawn rules, in the `frames` of the page: when the text of each cell of a
/// frame is fields (see [`parse`]; the label smaller than its value by half a point) and
/// short, the strip is a row of fields. A cell is cut further at the start of each label of
/// its first line when that line holds labels apart from each other over lines of values.
/// A run that still stands across a field edge (a value line the dividers could not cut at
/// a word gap) leaves its row as text, with a loss in `unsure` that names the row. Strips
/// at the same heights are one row; rows one under the other, each starting at the rule the
/// one above ends at, make one table, when it has two fields or more and one of them a
/// value. Runs `busy` names are not looked at. `small` (half points) is the size under which
/// text of one style is labels of empty fields. The runs of each table found go to
/// `claimed` under its place in `tables`.
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
    };
    let mut strips: Vec<Strip> = Vec::new();
    for fr in frames {
        let ids = frame_runs(flow, by_y, fr, busy);
        if ids.is_empty() || lines_of(flow, &ids).len() > STRIP_LINES {
            continue;
        }
        let edges = field_edges(flow, fr, &ids);
        let cells = cells_of(flow, &edges, &ids);
        let mut fields: Vec<Field> = Vec::new();
        let mut fits = true;
        for cell in cells.iter().filter(|c| !c.is_empty()) {
            match parse(flow, cell, how) {
                Parsed::Fields(f) if f.iter().all(|f| short(flow, f)) => fields.extend(f),
                _ => {
                    fits = false;
                    break;
                }
            }
        }
        if !fits || fields.is_empty() {
            continue;
        }
        // Never a wrong pair: a run over a field edge leaves the row as text.
        let inner = &edges[1..edges.len() - 1];
        if ids
            .iter()
            .any(|&i| inner.iter().any(|&e| across(&flow.runs[i], e)))
        {
            let labels: Vec<String> = fields.iter().map(|f| words(flow, &f.label)).collect();
            unsure.push(format!(
                "the row of fields {} on page {page} is not read as labels and values ({ACROSS}): \
                 its text is read as it stands",
                labels.join(", ")
            ));
            continue;
        }
        strips.push(Strip {
            top: fr.top,
            bottom: fr.bottom,
            x0: fr.x0,
            fields,
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
    for g in groups {
        let list: Vec<&Field> = g.iter().flat_map(|&k| &strips[k].fields).collect();
        if list.len() < 2 || list.iter().all(|f| f.value.is_empty()) {
            continue;
        }
        let top = strips[g[0]].top;
        let Some(p) = pairs_table(flow, &list, top, STRIP_FOUND_BY) else {
            continue;
        };
        let t = tables.len();
        for (r, f) in list.iter().enumerate() {
            for &i in &f.label {
                claimed[i] = Some((t, r, 0));
            }
            for &i in &f.value {
                claimed[i] = Some((t, r, 1));
            }
        }
        tables.push(p);
    }
}

/// The starts of the second and later labels of the first line of a cell: runs of the first
/// line, in its smallest style, apart from the run before by more than a cell gap, over lines
/// of a larger size below. None when the first line is not such labels.
fn label_starts(flow: &Flow, cell: &[usize]) -> Vec<f32> {
    let ids: Vec<usize> = cell
        .iter()
        .copied()
        .filter(|&i| inked(&flow.runs[i]))
        .collect();
    let lines = lines_of(flow, &ids);
    let (Some(first), true) = (lines.first(), lines.len() >= 2) else {
        return Vec::new();
    };
    let size = |i: usize| flow.runs[i].size;
    let small = first.iter().map(|&i| size(i)).fold(f32::MAX, f32::min);
    let below = lines[1..]
        .iter()
        .flatten()
        .map(|&i| size(i))
        .fold(f32::MIN, f32::max);
    if below - small < 0.5
        || first.iter().any(|&i| {
            size(i) - small >= 0.5
                && !{
                    // A raised marker goes with the label before it.
                    let k = first.iter().position(|&j| j == i).unwrap_or(0);
                    k > 0 && raised(&flow.runs[i], &flow.runs[first[k - 1]])
                }
        })
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    for w in first.windows(2) {
        let (a, b) = (&flow.runs[w[0]], &flow.runs[w[1]]);
        if raised(b, a) {
            continue;
        }
        if b.x - (a.x + a.w) > CELL_GAP * a.size.max(1.0) {
            out.push(b.x - SNAP.min(0.5 * (b.x - a.x - a.w)));
        }
    }
    out
}
