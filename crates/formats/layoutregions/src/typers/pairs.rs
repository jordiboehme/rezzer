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
use crate::extract::pairs::strips::{
    FieldFrame, Kind as ValueKind, cells_of, field_cuts, field_edges, kind_of, strip_cells,
    table_body, table_rows,
};
use crate::extract::pairs::{
    Field, Parsed, Reading, Style, WEIGHT_ONLY, parse, same_style, small_size, style_of,
    touch_reach, touching, words,
};
use crate::extract::table::grid::{Shape, titles_over_values};
use crate::extract::table::ruled::edges;
use crate::input::TextRun;
use crate::line::{EDGE_SIZES, LineRun, same_size};
use crate::region::Kind;
use crate::segment::cut::SIZE_STEP;
use crate::segment::{Segment, Source};
use crate::typers::table::group_at;
use crate::typers::{Lined, PageView, Typed, lined, seg_runs, signal, text_alone};

/// `label_value.fields`: the pair rules read the segment as fields.
pub const LABEL_FIELDS: u16 = 450;
/// `label_value.field_cells`: every cell with text of a grid (drawn boxes, or fields between
/// rules with dividers) reads as a field: the cells are fields, not a table's values. It leads
/// the most a paragraph reading of their text reaches (750) by the margin.
pub const LABEL_FIELD_CELLS: u16 = 900;
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
/// `label_value.boxes_touch`: the boxes of the fields touch in a row or a column. With the
/// field and its label style (550) it reaches 900, the margin over a paragraph (750).
pub const LABEL_BOXES_TOUCH: u16 = 350;
/// `label_value.dividers`: the fields stand between rules with dividers.
pub const LABEL_DIVIDERS: u16 = 100;
/// `label_value.known_style`: a pair of one size, its label in a style that pairs with
/// independent evidence show. With the field and its label style (550) it reaches 950: the
/// margin over the most a paragraph reading of the box's two lines reaches (750).
pub const LABEL_KNOWN_STYLE: u16 = 400;
/// `label_value.line_pairs`: every line is a short label and its value, each pair with
/// evidence of its own (see [`line_pair`]). With the colon or the value shape it carries it
/// reaches 1000, which leads the most a paragraph reading of such lines reaches (text size,
/// joins and sentences: 750) and a large bold heading that keeps its lines (850) by the
/// margin.
pub const LABEL_LINE_PAIRS: u16 = 850;

/// Whether a text run of `seg` stands on a running header or footer line, in the page's
/// footnote legend, or on the lines of the page title: lines the band, legend and title kinds
/// claim, which the pair rules of lines (line pairs, strip fields) do not read.
fn claimed_lines(view: &PageView<'_>, seg: &Segment) -> bool {
    let title: &[u32] = view.title().map_or(&[], |t| t.lines.as_slice());
    seg.prims.iter().any(|p| match *p {
        crate::input::PrimId::Text(i) => view.page.texts.get(i as usize).is_some_and(|r| {
            view.band(r.line).is_some() || view.in_legend(i) || title.contains(&r.line)
        }),
        _ => false,
    })
}

/// Whether every pair of a block of labels and values has evidence of its own: a label that
/// ends with a colon, a value shaped as a number, a date or a code, or a label in another style
/// than its value (set apart by size, weight or face). Two names side by side in one style
/// (an address beside an address) are no pair. A label without a value (an empty field) needs
/// none; at least one pair has a value.
fn pairs_evidenced(runs: &[&TextRun], rows: &[crate::extract::pairs::blocks::KeyValue]) -> bool {
    let mut valued = 0;
    for row in rows {
        let Some((s, e)) = row.value else { continue };
        valued += 1;
        let label: Vec<usize> = (row.label.0..row.label.1).collect();
        let value: Vec<usize> = (s..e).collect();
        let colon = words(runs, &label).ends_with(':');
        let shaped = value_shape(&words(runs, &value));
        let apart = match (label.first(), value.first()) {
            (Some(&l), Some(&v)) => !same_style(runs[l], runs[v]),
            _ => false,
        };
        if !(colon || shaped || apart) {
            return false;
        }
    }
    valued > 0
}

/// `label_value.strip_fields`: the lines stand in strips between drawn rules that span them,
/// and each strip reads as fields, a label (the smaller text) over, beside or under its value
/// (today's rule of fields between rules, [`strip_cells`], one copy). It leads the most a
/// table reading (800), a paragraph reading (750) and a heading that keeps its lines (850)
/// by the margin.
pub const LABEL_STRIP_FIELDS: u16 = 1000;

/// One strip between two rules of the page: a rule piece and the nearest piece above it that
/// overlaps it, where both reach (strips side by side on the same rules are strips of their
/// own), the runs with ink whose middle it holds (each run in the first such strip, top
/// to bottom, then left to right), and what it reads as.
#[derive(Debug, Clone)]
pub(crate) struct StripFrame {
    pub bottom: f32,
    pub top: f32,
    pub x0: f32,
    pub x1: f32,
    pub runs: Vec<u32>,
    read: FrameRead,
}

/// A key that orders `x` as [`f32::total_cmp`] does.
fn x_key(x: f32) -> i64 {
    let b = i64::from(x.to_bits() as i32);
    b ^ (((b >> 63) as u64) >> 33) as i64
}

/// The frames between horizontal `rules` (bottom, top, x0, x1), top to bottom, then left to
/// right: each stretch of a rule piece with the nearest piece above it that overlaps that
/// stretch in x. A rule of another column between the heights of two rules does not part them:
/// it frames only the stretch it lies over, and the rest of the piece goes on up.
///
/// One sweep from the top down, level by level (pieces within [`SNAP`] of the level's top):
/// the skyline holds, for each stretch of x, the nearest piece above the levels done, as
/// stretches that do not overlap, sorted by their left end. Each piece of a level reads the
/// stretches it lies under, then the level's pieces replace them. Every stretch is read once
/// before it is cut or replaced, so the work is the number of pieces times the log of the
/// skyline, plus the frames.
///
/// A level reaches [`SNAP`] down from its top piece, not from piece to piece: in a chain of
/// pieces each within `SNAP` of the next but more than `SNAP` from the top, the first piece past
/// the top's reach opens the next level, and the frame between it and the level above may be
/// thinner than `SNAP` (it holds no line of text; the scan of the tests never sees such a
/// chain, as its pieces stand whole points apart).
///
/// [`SNAP`]: crate::segment::drawn::SNAP
fn rule_frames(rules: &[crate::segment::drawn::Line]) -> Vec<(f32, f32, f32, f32)> {
    use std::collections::BTreeMap;
    let snap = crate::segment::drawn::SNAP;
    let mut sorted: Vec<&crate::segment::drawn::Line> = rules.iter().collect();
    sorted.sort_by(|a, b| b.at.total_cmp(&a.at).then(a.from.total_cmp(&b.from)));
    // The skyline: by the key of its left end, a stretch (x0, x1) and the height of its piece.
    let mut sky: BTreeMap<i64, (f32, f32, f32)> = BTreeMap::new();
    // The key of the first stretch that may reach past `x`: the last one that starts at or
    // left of it, else the first.
    let first = |sky: &BTreeMap<i64, (f32, f32, f32)>, x: f32| {
        sky.range(..=x_key(x))
            .next_back()
            .map_or(i64::MIN, |(k, _)| *k)
    };
    let mut frames: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut k = 0;
    while k < sorted.len() {
        let top = sorted[k].at;
        let end = k + sorted[k..].partition_point(|l| top - l.at <= snap);
        let level = &sorted[k..end];
        for low in level {
            for (_, &(a, b, at)) in sky.range(first(&sky, low.from)..) {
                crate::work::add(1);
                if a >= low.to {
                    break;
                }
                let (x0, x1) = (a.max(low.from), b.min(low.to));
                if x1 - x0 > snap {
                    frames.push((low.at, at, x0, x1));
                }
            }
        }
        for low in level {
            let cut: Vec<(i64, (f32, f32, f32))> = sky
                .range(first(&sky, low.from)..)
                .take_while(|(_, s)| s.0 < low.to)
                .filter(|(_, s)| s.1 > low.from)
                .map(|(k, s)| (*k, *s))
                .collect();
            crate::work::add(1 + cut.len() as u64);
            for (key, (a, b, at)) in cut {
                sky.remove(&key);
                if a < low.from {
                    sky.insert(x_key(a), (a, low.from, at));
                }
                if b > low.to {
                    sky.insert(x_key(low.to), (low.to, b, at));
                }
            }
            sky.insert(x_key(low.from), (low.from, low.to, low.at));
        }
        k = end;
    }
    frames.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.total_cmp(&b.2)));
    frames
}

/// The strips of the page of `view` (see [`StripFrame`]), found once: the frames of the page's
/// horizontal rules (see [`rule_frames`]), and the runs by one pass over them
/// sorted by the height of their middle.
pub(crate) fn strip_frames(view: &PageView<'_>) -> Vec<StripFrame> {
    let frames = rule_frames(view.hs());
    if frames.is_empty() {
        return Vec::new();
    }
    let texts = &view.page.texts;
    let mut by_y: Vec<(f32, u32)> = texts
        .iter()
        .enumerate()
        .filter(|(_, r)| crate::line::ink(&r.text) > 0 && !r.off_axis)
        .map(|(i, r)| (r.y + 0.5 * r.size, i as u32))
        .collect();
    by_y.sort_by(|a, b| a.0.total_cmp(&b.0));
    // The same runs by the middle of their width: a tall, narrow strip finds its runs by it.
    let mut by_x: Vec<(f32, u32)> = by_y
        .iter()
        .map(|&(_, i)| {
            let r = &texts[i as usize];
            (r.x + 0.5 * r.w, i)
        })
        .collect();
    by_x.sort_by(|a, b| a.0.total_cmp(&b.0));
    crate::work::add(2 * by_y.len() as u64);
    let middle = |i: u32| {
        let r = &texts[i as usize];
        (r.x + 0.5 * r.w, r.y + 0.5 * r.size)
    };
    let mut taken = vec![false; texts.len()];
    let how = Reading {
        one_field: false,
        smaller: true,
        small: small_size(view.text_sizes()).saturating_sub(1),
        labels: &[],
    };
    let mut out = Vec::new();
    for (bottom, top, x0, x1) in frames {
        // The runs whose middle lies in the strip, from the shorter of the two lists: those
        // in its height or those in its width.
        let lo = by_y.partition_point(|r| r.0 <= bottom);
        let hi = by_y.partition_point(|r| r.0 < top);
        let left = by_x.partition_point(|r| r.0 <= x0);
        let right = by_x.partition_point(|r| r.0 < x1);
        let near = if right.saturating_sub(left) < hi.saturating_sub(lo) {
            &by_x[left..right.max(left)]
        } else {
            &by_y[lo..hi.max(lo)]
        };
        crate::work::add(near.len() as u64);
        let mut runs = Vec::new();
        for &(_, i) in near {
            let (mx, my) = middle(i);
            if !taken[i as usize] && mx > x0 && mx < x1 && my > bottom && my < top {
                taken[i as usize] = true;
                runs.push(i);
            }
        }
        if runs.is_empty() {
            continue;
        }
        runs.sort_unstable();
        let own: Vec<&TextRun> = runs.iter().map(|&i| &texts[i as usize]).collect();
        let read = read_frame(&own, x0, x1, how);
        out.push(StripFrame {
            bottom,
            top,
            x0,
            x1,
            runs,
            read,
        });
    }
    out
}

/// The fields of `seg` read strip by strip (see [`strip_frames`]): every run with ink of the
/// segment stands in a strip, the segment holds every run of each of those strips, every one
/// of them reads clear (see [`read_frame`]), no strip stacked on one of them (sharing a rule,
/// side by side with it in part) holds fields that are not clear (labels under the rule their
/// values stand on), and there are two fields at least. `None` otherwise.
pub(crate) fn strip_fields(view: &PageView<'_>, seg: &Segment) -> Option<StripFields> {
    let frames = view.strip_frames();
    if frames.is_empty() {
        return None;
    }
    let (_, ids) = seg_runs(view, seg);
    let inked: Vec<u32> = ids
        .iter()
        .copied()
        .filter(|&i| {
            let r = &view.page.texts[i as usize];
            crate::line::ink(&r.text) > 0 && !r.off_axis
        })
        .collect();
    if inked.len() < 2 {
        return None;
    }
    crate::work::add(inked.len() as u64);
    let at = view.strip_of();
    let mut mine: Vec<usize> = Vec::new();
    for i in &inked {
        let f = *at.get(i)?;
        if !mine.contains(&f) {
            mine.push(f);
        }
    }
    let snap = crate::segment::drawn::SNAP;
    let (mut count, mut colon, mut shape) = (0usize, true, true);
    let mut label_styles: Vec<&TextRun> = Vec::new();
    let mut empty_labels: Vec<&TextRun> = Vec::new();
    for &f in &mine {
        let frame = &frames[f];
        if frame.runs.iter().any(|r| ids.binary_search(r).is_err()) {
            return None;
        }
        let FrameRead::Clear(read) = &frame.read else {
            return None;
        };
        count += read.fields;
        colon &= read.colon;
        shape &= read.shape;
        label_styles.extend(read.labels.iter());
        empty_labels.extend(read.empty.iter());
        for (k, g) in frames.iter().enumerate() {
            crate::work::add(1);
            let shares =
                (g.top - frame.bottom).abs() <= snap || (g.bottom - frame.top).abs() <= snap;
            if k != f
                && shares
                && g.x0 < frame.x1
                && frame.x0 < g.x1
                && g.read == FrameRead::Unclear
            {
                return None;
            }
        }
    }
    // The direction of the stack the strips stand in: labels over values only with positive
    // evidence at both of its ends (see [`stack_reads_down`]).
    if !stack_reads_down(view, frames, &mine, &label_styles) {
        return None;
    }
    // A field is never alone (a caption over a line between rules looks like one). An empty
    // field's label is in the style of the labels of fields with a value: text in another
    // style alone in a field is a value without its label, not an empty field.
    let empty_ok = empty_labels
        .iter()
        .all(|e| label_styles.iter().any(|l| same_style(*l, *e)));
    (count >= 2 && !label_styles.is_empty() && empty_ok).then_some(StripFields {
        fields: count,
        colon,
        shape,
    })
}

/// The band beyond each edge rule of a stack of strips that tells its direction is this many
/// sizes of the stack's largest text deep.
const STACK_EDGE: f32 = 1.5;

/// Whether the stack of strips the frames `mine` stand in reads as labels over values. The
/// stack is every strip that shares a rule with one of it, side by side with it in part. On a
/// form whose labels stand under their values, the lines between two rules are one row's
/// labels and the next row's values, so a strip of it looks like labels over values; its edges
/// tell: the values of its first row stand over its top rule, outside every strip, and the
/// labels of its last row under its bottom rule. So a stack reads down (labels over values)
/// only with positive evidence at both of its ends, in the band [`STACK_EDGE`] sizes of its
/// largest text deep beyond its edge rule, across its width:
/// - over its top rule, no rule and no box edge, and every run that starts where a label of the
///   stack starts (within its size) is in the size of its labels or smaller: no value stands
///   there;
/// - under its bottom rule, no rule and no box edge, and every such run is a label step larger
///   than its labels: no label stands there.
///
/// A band with nothing in it is such evidence: the edge row a stack of the other direction
/// needs is looked for where it must stand and is not there. Everything else (a run in the
/// stack's columns that may be a value over it or a label under it, a rule or a box beyond
/// which the edge row may stand) leaves the direction not clear, and the strip rule reads no
/// fields: the stack is spatial until Task 26 reads stacks (today's reader of labels under
/// their values runs before, as a given region).
fn stack_reads_down(
    view: &PageView<'_>,
    frames: &[StripFrame],
    mine: &[usize],
    labels: &[&TextRun],
) -> bool {
    let snap = crate::segment::drawn::SNAP;
    let Some(label) = labels.iter().map(|l| l.size).reduce(f32::min) else {
        return false;
    };
    // The stack: the strips that share a rule with one in it, side by side in part.
    let mut stack: Vec<usize> = mine.to_vec();
    let mut in_stack = vec![false; frames.len()];
    for &f in mine {
        if let Some(s) = in_stack.get_mut(f) {
            *s = true;
        }
    }
    let mut k = 0;
    while k < stack.len() {
        let f = &frames[stack[k]];
        for (g, h) in frames.iter().enumerate() {
            crate::work::add(1);
            let shares = (h.top - f.bottom).abs() <= snap || (h.bottom - f.top).abs() <= snap;
            if shares && h.x0 < f.x1 && f.x0 < h.x1 && !in_stack[g] {
                in_stack[g] = true;
                stack.push(g);
            }
        }
        k += 1;
    }
    let texts = &view.page.texts;
    let top = stack
        .iter()
        .map(|&f| frames[f].top)
        .fold(f32::MIN, f32::max);
    let bottom = stack
        .iter()
        .map(|&f| frames[f].bottom)
        .fold(f32::MAX, f32::min);
    let x0 = stack.iter().map(|&f| frames[f].x0).fold(f32::MAX, f32::min);
    let x1 = stack.iter().map(|&f| frames[f].x1).fold(f32::MIN, f32::max);
    let size = stack
        .iter()
        .flat_map(|&f| frames[f].runs.iter())
        .map(|&i| texts[i as usize].size)
        .fold(label, f32::max);
    crate::work::add(stack.len() as u64);
    let reach = STACK_EDGE * size;
    let over = |y: f32| y > top + snap && y - top <= reach;
    let under = |y: f32| y < bottom - snap && bottom - y <= reach;
    let across = |a: f32, b: f32| a.min(b) < x1 && x0 < a.max(b);
    // A rule or a box edge in a band: the edge row may stand beyond it.
    for h in view.hs() {
        crate::work::add(1);
        if across(h.from, h.to) && (over(h.at) || under(h.at)) {
            return false;
        }
    }
    for b in &view.page.boxes {
        crate::work::add(1);
        let (y0, y1) = (b.bounds.y0.min(b.bounds.y1), b.bounds.y0.max(b.bounds.y1));
        if across(b.bounds.x0, b.bounds.x1) && [y0, y1].into_iter().any(|y| over(y) || under(y)) {
            return false;
        }
    }
    let larger = label * crate::line::LABEL_STEP;
    let in_column = |r: &TextRun| labels.iter().any(|l| (r.x - l.x).abs() <= r.size);
    for r in texts {
        crate::work::add(1);
        if crate::line::ink(&r.text) == 0 || r.off_axis {
            continue;
        }
        let mx = r.x + 0.5 * r.w;
        let my = r.y + 0.5 * r.size;
        if mx <= x0 || mx >= x1 || !in_column(r) {
            continue;
        }
        // A run over the top rule that may be a value, or one under the bottom rule that may
        // be a label.
        if (over(my) && r.size >= larger) || (under(my) && r.size < larger) {
            return false;
        }
    }
    true
}

/// What [`strip_fields`] read: how many fields, whether every label ends with a colon, and
/// whether every value is shaped as a number, a date or a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StripFields {
    pub fields: usize,
    pub colon: bool,
    pub shape: bool,
}

/// What one strip between rules reads as.
#[derive(Debug, Clone, PartialEq)]
enum FrameRead {
    /// Fields read right, never a wrong pair.
    Clear(FrameFields),
    /// Fields, but not clear: a cell left out or not clear, a piece that is no label or value,
    /// a value of two lines or pieces, or a line that mixes labels and another field's value.
    Unclear,
    /// No fields: text that is no field, a table's body or rows.
    No,
}

/// The fields of a clear strip.
#[derive(Debug, Clone, PartialEq)]
struct FrameFields {
    fields: usize,
    colon: bool,
    shape: bool,
    /// The first run of the label of each field with a value, and of each empty field.
    labels: Vec<TextRun>,
    empty: Vec<TextRun>,
}

/// The runs `own` of a strip between rules from `x0` to `x1` read as fields (today's rule of
/// fields between rules, one copy): the runs are cut at field edges first, as today's reader
/// cuts them (see [`field_cuts`]), on this reading's own copies of the pieces.
fn read_frame(own: &[&TextRun], x0: f32, x1: f32, how: Reading<'_>) -> FrameRead {
    let frame = FieldFrame {
        x0,
        x1,
        dividers: &[],
    };
    let all: Vec<usize> = (0..own.len()).collect();
    let cuts = field_cuts(own, &[frame], &mut |_| all.clone());
    let mut pieces: Vec<TextRun> = Vec::with_capacity(own.len());
    for (k, r) in own.iter().enumerate() {
        match cuts.iter().find(|c| c.0 == k) {
            Some((_, at)) => pieces.extend(cut_at_gaps(r, at)),
            None => pieces.push((*r).clone()),
        }
    }
    crate::work::add(pieces.len() as u64);
    let runs: Vec<&TextRun> = pieces.iter().collect();
    let ids: Vec<usize> = (0..runs.len()).collect();
    let edges = field_edges(&runs, &frame, &ids);
    let cells = cells_of(&runs, &edges, &ids);
    if table_body(&runs, &cells) || table_rows(&runs, &ids) {
        return FrameRead::No;
    }
    let read = strip_cells(&runs, &edges, &cells, how);
    if !read.fits || read.fields.is_empty() {
        return FrameRead::No;
    }
    if read.unread || !read.left_out.is_empty() {
        return FrameRead::Unclear;
    }
    // Never a wrong pair: every piece of the strip is a label or a value, each field has a
    // label (an empty field has no value), and a value stands on one line in one piece (a
    // value of two lines, or of two pieces a cell gap apart, may be a field and the text of a
    // neighbour).
    let paired: usize = read
        .fields
        .iter()
        .map(|f| f.label.len() + f.value.len())
        .sum();
    let whole = read.fields.iter().all(|f| {
        !f.label.is_empty()
            && f.value
                .iter()
                .all(|&v| runs[v].line == runs[f.value[0]].line)
            && one_piece(&runs, &f.value)
    });
    // A line that holds labels and the value of a field whose label stands on another line
    // mixes two fields' roles (a second value line set on a label line): not clear.
    let label_lines: Vec<u32> = read
        .fields
        .iter()
        .flat_map(|f| f.label.iter().map(|&i| runs[i].line))
        .collect();
    let mixed = read.fields.iter().any(|f| {
        let own = f.label.first().map(|&i| runs[i].line);
        f.value
            .iter()
            .any(|&v| Some(runs[v].line) != own && label_lines.contains(&runs[v].line))
    });
    if paired != ids.len() || !whole || mixed {
        return FrameRead::Unclear;
    }
    let mut out = FrameFields {
        fields: read.fields.len(),
        colon: true,
        shape: true,
        labels: Vec::new(),
        empty: Vec::new(),
    };
    for f in &read.fields {
        out.colon &= words(&runs, &f.label).ends_with(':');
        if f.value.is_empty() {
            out.empty.push(runs[f.label[0]].clone());
        } else {
            out.shape &= value_shape(&words(&runs, &f.value));
            out.labels.push(runs[f.label[0]].clone());
        }
    }
    FrameRead::Clear(out)
}

/// `run` cut at its word gaps `at` (indexes into its gaps): each piece keeps its words, from
/// the end of the ink before the gap to the start of the ink after it.
fn cut_at_gaps(run: &TextRun, at: &[usize]) -> Vec<TextRun> {
    let words: Vec<&str> = run.text.split_whitespace().collect();
    let mut out = Vec::new();
    let (mut from_word, mut x) = (0usize, run.x);
    for &k in at {
        let Some(g) = run.gaps.get(k) else { continue };
        let w = (g.words as usize).min(words.len());
        if w <= from_word {
            continue;
        }
        out.push(TextRun {
            text: words[from_word..w].join(" "),
            x,
            w: (g.from - x).max(0.0),
            gaps: Vec::new(),
            ..run.clone()
        });
        from_word = w;
        x = g.to;
    }
    out.push(TextRun {
        text: words[from_word.min(words.len())..].join(" "),
        x,
        w: (run.x + run.w - x).max(0.0),
        gaps: Vec::new(),
        ..run.clone()
    });
    out
}

/// Whether the runs `ids` of `runs` stand in one piece: no gap between them, left to right,
/// of a cell gap (see [`crate::extract::table::CELL_GAP`]) of their size or more.
fn one_piece(runs: &[&TextRun], ids: &[usize]) -> bool {
    let mut by_x: Vec<&TextRun> = ids.iter().map(|&i| runs[i]).collect();
    by_x.sort_by(|a, b| a.x.total_cmp(&b.x));
    by_x.windows(2)
        .all(|w| w[1].x - (w[0].x + w[0].w) < crate::extract::table::CELL_GAP * w[1].size.max(1.0))
}

/// A label of a line pair holds at most this many words: a label names, it does not say.
pub(crate) const PAIR_LABEL_WORDS: usize = 4;
/// A value of a line pair that has no shape of its own holds at most this many words and no
/// sentence: longer text after a colon is prose (`Note: this is a sentence ...`).
pub(crate) const PAIR_VALUE_WORDS: usize = 6;

/// Where a line parts into its label and its value by the line pair rule (see
/// [`line_pair_cut`]): the label is the line's runs before run `run` and the first `byte`
/// bytes of run `run`; the value is the rest. With `byte` 0 run `run` opens the value whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairCut {
    pub run: usize,
    pub byte: usize,
}

/// A line read as a pair by [`line_pair`]: its value as text, whether the label ends with a
/// colon, and where it parts, in the runs the rule read (`cut.run` indexes them).
struct LinePair {
    value: String,
    colon: bool,
    cut: PairCut,
}

/// The items of a list follow each other at most this many text sizes apart, baseline to
/// baseline (the pitch [`key_values`] allows the lines of a block of labels and values).
const ITEM_PITCH: f32 = 2.0;

/// Whether the one line `range` of `runs` (a segment of its own) is an item of a list: it
/// opens with a list marker, with an enumerator in brackets (`(a)`, `(12)`, `(iv)`), or with
/// a word of one character that the page's line right over
/// or under it opens with too, at its left edge, in its size and within [`ITEM_PITCH`] sizes
/// (a bullet the marker rule does not know, set as a letter or a sign of its own: `o`, `>`).
/// An item that reads `label: value` is an item of its list, not a table of one row between
/// the other items. A line that stands alone, and lines of labels and values under each
/// other, open with words of their own and are not held back.
fn list_item(
    view: &PageView<'_>,
    runs: &[&TextRun],
    range: &crate::extract::table::RunRange,
) -> bool {
    let Some(first) = runs.get(range.start) else {
        return false;
    };
    let text: String = runs[range.start..range.end]
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if crate::extract::text::list_marker(text.trim_start()).is_some() {
        return true;
    }
    // An enumerator in brackets with text after it: digits (`(12)`), one letter (`(a)`) or
    // a small roman numeral (`(iv)`), up to four characters. A short name in brackets
    // (`(EIN)`, `(GLN)`) is none.
    let mut words = text.split_whitespace();
    let bracketed = words.next().is_some_and(|w| {
        w.strip_prefix('(')
            .and_then(|w| w.strip_suffix(')'))
            .is_some_and(|w| {
                let n = w.chars().count();
                (1..=4).contains(&n)
                    && (w.chars().all(|c| c.is_ascii_digit())
                        || (n == 1 && w.chars().all(char::is_alphabetic))
                        || w.chars().all(|c| matches!(c, 'i' | 'v' | 'x')))
            })
    });
    if bracketed && words.next().is_some() {
        return true;
    }
    // The word a line opens with, when it is one character and more text follows it.
    let mark = |t: &str| -> Option<char> {
        let mut words = t.split_whitespace();
        let mut w = words.next()?.chars();
        let c = w.next()?;
        (w.next().is_none() && words.next().is_some()).then_some(c)
    };
    let (Some(own), Some(&k)) = (mark(&text), view.line_at.get(&first.line)) else {
        return false;
    };
    let lines = view.lines();
    let same_mark = |j: usize| {
        let o = &lines[j];
        let size = o.size.max(range.size);
        let Some(r) = view.page.texts.get(o.start) else {
            return false;
        };
        let other: String = view.page.texts[o.start..o.end.min(view.page.texts.len())]
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        same_size(o.size, range.size)
            && (r.x - first.x).abs() <= EDGE_SIZES * size
            && (o.y - range.y).abs() <= ITEM_PITCH * size
            && mark(&other) == Some(own)
    };
    crate::work::add(2);
    (k > 0 && same_mark(k - 1)) || (k + 1 < lines.len() && same_mark(k + 1))
}

/// The label and the value of one line of `runs` (its pieces `pieces`), each pair with
/// evidence of its own, whether the label ends with a colon, and where the line parts:
///
/// - one piece that holds a colon after a short label (at most [`PAIR_LABEL_WORDS`] words with
///   a letter): the value after it is shaped as a number, date or code, or short (at most
///   [`PAIR_VALUE_WORDS`] words) and no sentence; the line parts after the colon;
/// - with `lone` (the only line of its group), two pieces: a short label with a letter and a
///   value shaped as a number, a date or a code (`Rechnungs-Nr.   2026004711`), or, after a
///   label that ends with a colon, a short value that is no sentence and no label; the value
///   not set a size step ([`SIZE_STEP`]) smaller than its label: a small code beside a larger
///   name (`United States Treasury` and a `000` in two thirds of its size) is a mark printed
///   beside it, not its value; after a label that ends with a colon a smaller value is its
///   value; the line parts between the pieces.
///
/// A line that opens as a caption ([`crate::extract::table::caption::caption`]) is none.
/// Anything else is no pair; more pieces are a row, which the table rules read.
fn line_pair<R: LineRun>(
    runs: &[R],
    pieces: &[crate::extract::table::grid::Piece],
    lone: bool,
) -> Option<LinePair> {
    let text =
        |p: &crate::extract::table::grid::Piece| words(runs, &(p.start..p.end).collect::<Vec<_>>());
    let short_label = |l: &str| {
        let n = l.split_whitespace().count();
        (1..=PAIR_LABEL_WORDS).contains(&n) && l.chars().any(char::is_alphabetic)
    };
    let no_sentence = |v: &str| {
        let n = v.split_whitespace().count();
        n <= PAIR_VALUE_WORDS && !v.trim_end().ends_with(['.', '!', '?']) && !v.contains(". ")
    };
    // A line that opens as a caption ("Figure 1: ...", "Abb. 3: ...") names a figure or
    // a table: it is no label and its value.
    let opens = pieces.first().map(text)?;
    if crate::extract::table::caption::caption(&opens).is_some() {
        return None;
    }
    match pieces {
        [one] => {
            let t = text(one);
            let at = t.find(": ")?;
            let (label, value) = (t[..=at].trim(), t[at + 1..].trim());
            let label_words = label.trim_end_matches(':');
            if value.is_empty() || !short_label(label_words) {
                return None;
            }
            if !(value_shape(value) || no_sentence(value)) {
                return None;
            }
            Some(LinePair {
                value: value.to_string(),
                colon: true,
                cut: colon_cut(runs, one.start..one.end)?,
            })
        }
        [a, b] if lone => {
            let (label, value) = (text(a), text(b));
            let size = |p: &crate::extract::table::grid::Piece| {
                runs[p.start..p.end]
                    .iter()
                    .map(LineRun::size)
                    .fold(0.0f32, f32::max)
            };
            let smaller = size(b) > 0.0 && size(a) >= SIZE_STEP * size(b);
            // A label that ends with a colon says it is one, as in one piece: its value may be
            // any short text that is no sentence and no label (no word of it ends with a
            // colon: `Bill To:   Ship To:` is two labels).
            let colon = label.ends_with(':');
            let no_label = !value.split_whitespace().any(|w| w.ends_with(':'));
            let value_ok = value_shape(&value) || (colon && no_sentence(&value) && no_label);
            (short_label(&label) && value_ok && (!smaller || colon)).then_some(LinePair {
                value,
                colon,
                cut: PairCut {
                    run: b.start,
                    byte: 0,
                },
            })
        }
        _ => None,
    }
}

/// Where the runs `range` of `runs` part after their first colon that ends a word with a word
/// after it (the first `": "` of their words joined by spaces): in the run of that word, just
/// after the colon.
fn colon_cut<R: LineRun>(runs: &[R], range: std::ops::Range<usize>) -> Option<PairCut> {
    let mut found: Option<PairCut> = None;
    for k in range {
        let t = runs[k].text();
        for w in t.split_whitespace() {
            // A word after the colon: the colon ends the label.
            if found.is_some() {
                return found;
            }
            if w.ends_with(':') {
                let end = w.as_ptr() as usize - t.as_ptr() as usize + w.len();
                found = Some(PairCut { run: k, byte: end });
            }
        }
    }
    None
}

/// Where one line of text parts into its label and its value by the rule the label and value
/// typer reads line pairs by ([`line_pair`]), or `None` when the line is no pair. `line` holds
/// the runs of one line in order, blank runs among them; `lone` says the line is the only one
/// of its group. A colon inside a run cuts that run; a blank run between the label and the
/// value goes to the side of the widest gap between the two, as the line stands. The typer
/// and the readers that extract a line pairs region read the same pairs by it.
pub fn line_pair_cut<R: LineRun>(line: &[R], lone: bool) -> Option<PairCut> {
    let at: Vec<usize> = (0..line.len())
        .filter(|&k| !line[k].text().trim().is_empty() && !line[k].off_axis())
        .collect();
    let inked: Vec<&R> = at.iter().map(|&k| &line[k]).collect();
    let pieces = crate::extract::table::grid::pieces_of(&inked, 0, inked.len());
    crate::work::add(line.len() as u64);
    let cut = line_pair(&inked, &pieces, lone)?.cut;
    // A cut between two inked runs: at the widest gap between them, blank runs included.
    let between = |before: usize| -> PairCut {
        let (from, to) = (at[before] + 1, at[before + 1]);
        let gap = |j: usize| line[j].x() - (line[j - 1].x() + line[j - 1].w());
        let run = (from..=to)
            .max_by(|&a, &b| gap(a).total_cmp(&gap(b)))
            .unwrap_or(to);
        PairCut { run, byte: 0 }
    };
    if cut.byte == 0 {
        return Some(between(cut.run.checked_sub(1)?));
    }
    if cut.byte >= inked[cut.run].text().len() {
        return (cut.run + 1 < inked.len()).then(|| between(cut.run));
    }
    Some(PairCut {
        run: at[cut.run],
        byte: cut.byte,
    })
}

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
        // Running lines, footnote legends and the page title have kinds of their own: the
        // pair rules of lines leave them alone.
        _ if text_alone(view, seg) && claimed_lines(view, seg) => return None,
        _ if text_alone(view, seg)
            && let Some(read) = view.strip_fields_of(seg) =>
        {
            signals.push(signal(
                "label_value.strip_fields",
                format!("{} fields in strips between rules", read.fields),
                LABEL_STRIP_FIELDS,
            ));
            if read.colon {
                signals.push(signal(
                    "label_value.colon",
                    "the labels end with a colon".into(),
                    LABEL_COLON,
                ));
            }
            if read.shape {
                signals.push(signal(
                    "label_value.value_shape",
                    "the values are numbers, dates or codes".into(),
                    LABEL_VALUE_SHAPE,
                ));
            }
        }
        // Lines between two rules are read as fields by the strip rule alone, which needs the
        // labels set apart: no other pair rule reads them.
        Source::Strip => return None,
        _ if text_alone(view, seg) => {
            let Lined {
                runs,
                ranges,
                pieces,
                ..
            } = lined(&refs);
            let off = vec![false; ranges.len()];
            let read = key_values(&runs, &ranges, 0, ranges.len(), &pieces, &off);
            if let KeyValues::Rows(rows) = read
                && pairs_evidenced(&runs, &rows)
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
                // Pieces in the cells of a drawn grid are cells, not a label and its value.
                let in_grid = {
                    let mut up = seg.parent;
                    let mut found = false;
                    while let Some(p) = up.and_then(|p| view.segs.all.get(p.0 as usize)) {
                        if matches!(p.source, Source::Grid { cols, .. } if cols >= 2) {
                            found = true;
                            break;
                        }
                        up = p.parent;
                    }
                    found
                };
                let lone = ranges.len() == 1 && !in_grid;
                // An item of a list (see [`list_item`]) is no pair on its own.
                if ranges.len() == 1 && list_item(view, &runs, &ranges[0]) {
                    return None;
                }
                let pairs: Option<Vec<LinePair>> =
                    pieces.iter().map(|p| line_pair(&runs, p, lone)).collect();
                let pairs = pairs.filter(|p| !p.is_empty())?;
                signals.push(signal(
                    "label_value.line_pairs",
                    format!("{} lines of a short label and its value", pairs.len()),
                    LABEL_LINE_PAIRS,
                ));
                if pairs.iter().all(|p| p.colon) {
                    signals.push(signal(
                        "label_value.colon",
                        "the labels end with a colon".into(),
                        LABEL_COLON,
                    ));
                }
                if pairs.iter().all(|p| value_shape(&p.value)) {
                    signals.push(signal(
                        "label_value.value_shape",
                        "the values are numbers, dates or codes".into(),
                        LABEL_VALUE_SHAPE,
                    ));
                }
                // Neither a colon nor a shape on every line: the pairs are not evidenced
                // as one group.
                if signals.len() == 1 {
                    return None;
                }
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

    /// The frames by a scan over every higher piece (each open stretch of a piece with the
    /// nearest piece above it that overlaps it): what [`rule_frames`] finds by its sweep.
    fn frames_by_scan(rules: &[crate::segment::drawn::Line]) -> Vec<(f32, f32, f32, f32)> {
        let snap = crate::segment::drawn::SNAP;
        let mut sorted: Vec<&crate::segment::drawn::Line> = rules.iter().collect();
        sorted.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
        let mut frames = Vec::new();
        for (i, low) in sorted.iter().enumerate() {
            let mut open = vec![(low.from, low.to)];
            for up in &sorted[i + 1..] {
                if up.at - low.at <= snap {
                    continue;
                }
                let mut left = Vec::new();
                for &(a, b) in &open {
                    let (x0, x1) = (a.max(up.from), b.min(up.to));
                    if x1 - x0 > snap {
                        frames.push((low.at, up.at, x0, x1));
                        if x0 > a {
                            left.push((a, x0));
                        }
                        if b > x1 {
                            left.push((x1, b));
                        }
                    } else {
                        left.push((a, b));
                    }
                }
                open = left;
            }
        }
        frames.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then(a.2.total_cmp(&b.2))
                .then(a.0.total_cmp(&b.0))
        });
        frames
    }

    /// Fix round 3 (N10): the sweep finds the frames a scan over every higher piece finds, on
    /// rules of many lengths and places, in columns and across them (pieces at one height
    /// apart, as merged drawn lines are).
    #[test]
    fn the_rule_sweep_finds_the_frames_of_a_scan() {
        let mut seed = 7u32;
        let mut next = |m: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) % m
        };
        for _ in 0..200 {
            let n = 2 + next(30) as usize;
            // Pieces at one height do not overlap: the drawn lines are merged.
            let mut rules: Vec<crate::segment::drawn::Line> = Vec::new();
            for _ in 0..n {
                let from = 10.0 * next(50) as f32;
                let l = crate::segment::drawn::Line {
                    at: 10.0 * next(40) as f32,
                    from,
                    to: from + 10.0 * (1 + next(30)) as f32,
                };
                if !rules
                    .iter()
                    .any(|r| r.at == l.at && r.from < l.to && l.from < r.to)
                {
                    rules.push(l);
                }
            }
            let mut swept = rule_frames(&rules);
            swept.sort_by(|a, b| {
                b.1.total_cmp(&a.1)
                    .then(a.2.total_cmp(&b.2))
                    .then(a.0.total_cmp(&b.0))
            });
            assert_eq!(swept, frames_by_scan(&rules), "{rules:?}");
        }
    }

    /// Boy scout N11 (Task 18b re-review 3): a chain of rules each 1.5 points under the one
    /// above (within `SNAP` of the next, not of the top) is one level from its top: the piece
    /// past the top's reach opens the next level, with a frame thinner than `SNAP` over it,
    /// and the rule far below frames up to it.
    #[test]
    fn a_chain_of_close_rules_is_one_level_from_its_top() {
        let line = |at: f32| crate::segment::drawn::Line {
            at,
            from: 0.0,
            to: 100.0,
        };
        let frames = rule_frames(&[line(104.0), line(102.5), line(101.0), line(50.0)]);
        assert_eq!(
            frames,
            [(101.0, 102.5, 0.0, 100.0), (50.0, 101.0, 0.0, 100.0)]
        );
    }

    /// Fix round 3 (N10): pieces in two columns, none over another, cost the sweep about one
    /// step each, not one for every higher piece.
    #[test]
    fn rules_in_two_columns_are_swept_in_linear_work() {
        let rules: Vec<crate::segment::drawn::Line> = (0..2000)
            .map(|k| {
                let left = k % 2 == 0;
                crate::segment::drawn::Line {
                    at: 5.0 * k as f32,
                    from: if left { 0.0 } else { 300.0 },
                    to: if left { 200.0 } else { 500.0 },
                }
            })
            .collect();
        let before = crate::work::work_steps();
        let frames = rule_frames(&rules);
        let after = crate::work::work_steps();
        assert_eq!(frames.len(), 1998);
        if let (Some(b), Some(a)) = (before, after) {
            assert!(a - b < 10 * rules.len() as u64, "{} steps", a - b);
        }
    }

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
        let boxes = segs
            .all
            .iter()
            .filter(|s| s.source == crate::segment::Source::Box)
            .count();
        assert_eq!(boxes, 2, "both boxes are segments: {:#?}", segs.all);
        let none = |_: &str| false;
        let view = PageView::new(&page, &structure, &segs, &facts, &index, &none);
        let touching = touching_boxes(&view);
        assert_eq!(touching.len(), segs.all.len());
    }

    /// A run of one line at `x`, 10 pt, its advance by the length of its text.
    fn on_line(text: &str, x: f32) -> TextRun {
        TextRun {
            text: text.into(),
            x,
            y: 700.0,
            w: 5.0 * text.chars().count() as f32,
            h: 7.0,
            size: 10.0,
            font: 1,
            font_kind: FontKind::Proportional,
            bold: false,
            italic: false,
            color: None,
            shift: Shift::Base,
            line: 0,
            off_axis: false,
            gaps: Vec::new(),
        }
    }

    /// Fix round 1b (I3): the line pair rule says where a line parts: in a run after the colon
    /// that ends its label, after a run that ends with it, or between two pieces of a lone line,
    /// a blank run between them on the side of the widest gap; a line it refuses parts nowhere.
    #[test]
    fn a_line_pair_parts_where_the_rule_reads_its_label_and_value() {
        let one = [on_line("Invoice no: 4711", 40.0)];
        assert_eq!(
            line_pair_cut(&one, false),
            Some(PairCut { run: 0, byte: 11 })
        );
        let two = [on_line("Date:", 40.0), on_line("2026-05-01", 67.0)];
        assert_eq!(
            line_pair_cut(&two, false),
            Some(PairCut { run: 1, byte: 0 })
        );
        // Two pieces apart: a pair only as the lone line of its group; the blank run right
        // after the label stays with it.
        let apart = [
            on_line("Order", 40.0),
            on_line(" ", 65.0),
            on_line("2026004711", 200.0),
        ];
        assert_eq!(line_pair_cut(&apart, false), None);
        assert_eq!(
            line_pair_cut(&apart, true),
            Some(PairCut { run: 2, byte: 0 })
        );
        // Prose after a colon, a caption, and a row of three pieces are no pair.
        let prose = [on_line(
            "Note: this is a sentence that says much more than a value.",
            40.0,
        )];
        assert_eq!(line_pair_cut(&prose, true), None);
        let caption = [on_line("Figure 1: 2026", 40.0)];
        assert_eq!(line_pair_cut(&caption, true), None);
        let row = [
            on_line("Item", 40.0),
            on_line("12", 200.0),
            on_line("3,50", 400.0),
        ];
        assert_eq!(line_pair_cut(&row, true), None);
        // Fix round 2: after a label that ends with a colon, a lone line's value may be any
        // short text that is no sentence; without the colon it needs a shape of its own.
        let coded = [
            on_line("Member Code:", 40.0),
            on_line("MC 4417 2093 88", 200.0),
        ];
        assert_eq!(
            line_pair_cut(&coded, true),
            Some(PairCut { run: 1, byte: 0 })
        );
        assert_eq!(line_pair_cut(&coded, false), None);
        let bare = [
            on_line("Member Code", 40.0),
            on_line("MC 4417 2093 88", 200.0),
        ];
        assert_eq!(line_pair_cut(&bare, true), None);
        let labels = [on_line("Bill To:", 40.0), on_line("Ship To:", 200.0)];
        assert_eq!(line_pair_cut(&labels, true), None);
        // Fix round 3: a value a size step smaller than its label is a mark beside a name,
        // unless the label ends with a colon, which says the value follows.
        let small = |label: &str| {
            let mut value = on_line("104857312", 200.0);
            value.size = 8.0;
            [on_line(label, 40.0), value]
        };
        assert_eq!(
            line_pair_cut(&small("Member Code:"), true),
            Some(PairCut { run: 1, byte: 0 })
        );
        assert_eq!(line_pair_cut(&small("Member Code"), true), None);
    }
}
