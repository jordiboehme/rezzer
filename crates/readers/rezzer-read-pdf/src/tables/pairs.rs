// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Labels and values of a form, read as tables of two columns: drawn boxes whose text is a
//! small label at the top or left and a value in another style, and rows of fields between
//! drawn rules (a header field grid of label rows over value rows, a block of labels with
//! their values beside them).

use std::collections::HashSet;

use super::banded::heads_lines;
use super::*;
use crate::regions::{Seg, grids, regions_of};

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
            if styles[0].0 >= how.small {
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
/// label stands below its value.
pub(crate) fn box_pairs(
    page: u32,
    flow: &mut Flow,
    boxes: &[[f32; 4]],
    rules: &[Rule],
    text_sizes: &[f32],
) -> (Vec<Planned>, Vec<String>) {
    if boxes.is_empty() || flow.runs.is_empty() {
        return (Vec::new(), Vec::new());
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
        return (Vec::new(), Vec::new());
    };
    let mut in_box: Vec<Vec<usize>> = vec![Vec::new(); boxes.len()];
    for (i, r) in regions.iter().enumerate() {
        if let Some(b) = r {
            in_box[*b as usize].push(i);
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
        if ids.is_empty() {
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
        let hs = hs.get_or_insert_with(|| grids(rules).map(|g| g.hs).unwrap_or_default());
        if heads_lines(flow, hs, b, &xs) {
            for &k in &c {
                used[k] = true;
            }
            continue;
        }
        for &k in &c {
            used[k] = true;
        }
        groups.push(c);
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
    let mut reasons: Vec<&str> = unsure
        .iter()
        .filter(|(b, _)| paired.iter().any(|&p| near(boxes[*b], boxes[p])))
        .map(|(_, why)| *why)
        .collect();
    reasons.sort_unstable();
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
    if groups.is_empty() {
        return (Vec::new(), said);
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
    fill(&mut planned, flow, &target);
    (planned, said)
}

/// One strip of fields between two rules: its rules' heights, its left and right ends, and
/// its fields by cell, left to right.
struct Strip {
    top: f32,
    bottom: f32,
    x0: f32,
    fields: Vec<Field>,
}

/// Fields between drawn rules: a rule no grid takes, with the nearest rule below it that
/// covers it, bounds a strip; dividers that hang from its top or rise from its bottom part it
/// in cells. When the text of each cell is fields (see [`parse`]; the label smaller than its
/// value by half a point) and short, the strip is a row of fields. Strips at the same heights
/// are one row; rows one under the other, each starting at the rule the one above ends at,
/// make one table, when it has two fields or more and one of them a value. `hs` are the
/// page's horizontal lines sorted by height, `taken` those a grid takes, `vs` the vertical
/// ones sorted by place; runs `busy` names are not looked at. `small` (half points) is the
/// size under which text of one style is labels of empty fields. The runs of each table
/// found go to `claimed` under its place in `tables`.
#[allow(clippy::too_many_arguments)]
pub(super) fn field_strips(
    flow: &Flow,
    by_y: &ByY,
    hs: &[Seg],
    taken: &[bool],
    vs: &[Seg],
    busy: &dyn Fn(usize) -> bool,
    text_sizes: &[f32],
    claimed: &mut [Option<(usize, usize, usize)>],
    tables: &mut Vec<Planned>,
) -> bool {
    let how = Reading {
        one_field: false,
        smaller: true,
        small: small_size(text_sizes).saturating_sub(1),
    };
    let mut work = 0usize;
    let mut strips: Vec<Strip> = Vec::new();
    for (j, up) in hs.iter().enumerate() {
        if taken[j] {
            continue;
        }
        // The nearest rule below that reaches under it, and it must cover it.
        let mut below: Option<&Seg> = None;
        for s in hs[..j].iter().rev() {
            work += 1;
            if work > STRIP_WORK {
                return false;
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
        let ids: Vec<usize> = by_y
            .runs_between(bottom, top)
            .into_iter()
            .filter(|&i| {
                let r = &flow.runs[i];
                let (cx, _) = middle(r);
                !busy(i) && !r.off_axis && inked(r) && cx > x0 && cx < x1
            })
            .collect();
        work += ids.len();
        if ids.is_empty() || lines_of(flow, &ids).len() > STRIP_LINES {
            continue;
        }
        // Cell edges: dividers in the strip that reach its top or its bottom.
        let lo = vs.partition_point(|v| v.at <= x0 + SNAP);
        let mut xs = vec![x0];
        for v in &vs[lo..] {
            work += 1;
            if v.at >= x1 - SNAP || work > STRIP_WORK {
                break;
            }
            let reaches = (v.to - top).abs() <= SNAP || (v.from - bottom).abs() <= SNAP;
            if reaches && v.to - v.from >= DIVIDER && v.from >= bottom - SNAP && v.to <= top + SNAP
            {
                xs.push(v.at);
            }
        }
        xs.push(x1);
        let mut cells: Vec<Vec<usize>> = vec![Vec::new(); xs.len() - 1];
        for &i in &ids {
            let (cx, _) = middle(&flow.runs[i]);
            let c = xs
                .partition_point(|e| *e < cx)
                .saturating_sub(1)
                .min(cells.len() - 1);
            cells[c].push(i);
        }
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
        if fits && !fields.is_empty() {
            strips.push(Strip {
                top,
                bottom,
                x0,
                fields,
            });
        }
    }
    if work > STRIP_WORK {
        return false;
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
    true
}
