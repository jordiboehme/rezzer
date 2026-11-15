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
    table_body, table_rows, undecided,
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
/// labels and values, or two lines of titles over their values (the first readings of the pdf reader). Such
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

/// `label_value.label_over_value`: the segment is a row of labels over the row of their
/// values, as the whitespace cut found them ([`Source::Cell`]: two labels or more side by
/// side, each set a size step smaller than the value under it, each value under one label
/// only; or a row set like such a row of its region), and each value stands under one label
/// ([`crate::extract::pairs::blocks::labels_with_values`]). A label with nothing under it has
/// an empty value. It leads the most a paragraph reading of its two lines (750) and a heading
/// that keeps its lines (850) reach.
pub const LABEL_OVER_VALUE: u16 = 900;

/// The id of the signal of [`LABEL_OVER_VALUE`]: the readers that extract such a cell read
/// the same pair by it.
pub const LABEL_OVER_VALUE_ID: &str = "label_value.label_over_value";

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
/// none; at least one pair has a value. A row cut inside a run (`KeyValue::cut`) has a value
/// and its evidence already: the cut is made only at a colon or before a value of such a shape
/// (see `value_cut`), and its label range holds the whole cut run, the start of the value
/// with it, so the ranges are not asked again.
fn pairs_evidenced(runs: &[&TextRun], rows: &[crate::extract::pairs::blocks::KeyValue]) -> bool {
    let mut valued = 0;
    for row in rows {
        if row.cut.is_some() {
            valued += 1;
            continue;
        }
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
/// (the current rule of fields between rules, [`strip_cells`], one copy). It leads the most a
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
/// stretch in x, with the place in `rules` of that piece above it, its top. A rule of another column between the heights of two rules does not part them:
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
fn rule_frames_under(rules: &[crate::segment::drawn::Line]) -> Vec<(Place, usize)> {
    use std::collections::BTreeMap;
    let snap = crate::segment::drawn::SNAP;
    let mut sorted: Vec<(usize, &crate::segment::drawn::Line)> = rules.iter().enumerate().collect();
    sorted.sort_by(|(_, a), (_, b)| b.at.total_cmp(&a.at).then(a.from.total_cmp(&b.from)));
    // The skyline: by the key of its left end, a stretch (x0, x1), the height of its piece
    // and the place of that piece in `rules`.
    type Stretch = (f32, f32, f32, usize);
    let mut sky: BTreeMap<i64, Stretch> = BTreeMap::new();
    // The key of the first stretch that may reach past `x`: the last one that starts at or
    // left of it, else the first.
    let first = |sky: &BTreeMap<i64, Stretch>, x: f32| {
        sky.range(..=x_key(x))
            .next_back()
            .map_or(i64::MIN, |(k, _)| *k)
    };
    let mut frames: Vec<(Place, usize)> = Vec::new();
    let mut k = 0;
    while k < sorted.len() {
        let top = sorted[k].1.at;
        let end = k + sorted[k..].partition_point(|l| top - l.1.at <= snap);
        let level = &sorted[k..end];
        for (_, low) in level {
            for (_, &(a, b, at, piece)) in sky.range(first(&sky, low.from)..) {
                crate::work::add(1);
                if a >= low.to {
                    break;
                }
                let (x0, x1) = (a.max(low.from), b.min(low.to));
                if x1 - x0 > snap {
                    frames.push(((low.at, at, x0, x1), piece));
                }
            }
        }
        for &(piece, low) in level {
            let cut: Vec<(i64, Stretch)> = sky
                .range(first(&sky, low.from)..)
                .take_while(|(_, s)| s.0 < low.to)
                .filter(|(_, s)| s.1 > low.from)
                .map(|(k, s)| (*k, *s))
                .collect();
            crate::work::add(1 + cut.len() as u64);
            for (key, (a, b, at, over)) in cut {
                sky.remove(&key);
                if a < low.from {
                    sky.insert(x_key(a), (a, low.from, at, over));
                }
                if b > low.to {
                    sky.insert(x_key(low.to), (low.to, b, at, over));
                }
            }
            sky.insert(x_key(low.from), (low.from, low.to, low.at, piece));
        }
        k = end;
    }
    frames.sort_by(|(a, _), (b, _)| b.1.total_cmp(&a.1).then(a.2.total_cmp(&b.2)));
    frames
}

/// The strips of a page (see [`strip_frames`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct Strips {
    pub frames: Vec<StripFrame>,
    /// The search for the dividers of the strips went past
    /// [`STRIP_WORK`](crate::extract::pairs::strips::STRIP_WORK) steps: every strip is read
    /// without dividers, which the page's issues say (see [`DIVIDERS_UNSEARCHED`]).
    pub unparted: bool,
}

/// What the issue of a page says whose strips are read without their dividers.
pub(crate) const DIVIDERS_UNSEARCHED: &str = "the dividers of the rows of fields between rules \
     were not looked for: the page has too many lines";

/// Where a strip stands: the heights of its bottom and top rules and its left and right ends.
type Place = (f32, f32, f32, f32);

/// A place that holds nothing: no line stands at its rules between its ends.
const EMPTY_PLACE: Place = (0.0, 0.0, 1.0, 0.0);

/// For each of `frames` (bottom, top, x0, x1), the lines of `vs` (vertical, sorted by place)
/// that stand at its top or at its bottom rule between its ends, by their places in `vs`, in
/// order: every divider of the frame is one of them (a divider reaches one of the two rules or
/// runs on through it, see [`crate::extract::pairs::strips::dividers_of`]), and a line at
/// another height is none of them, however many of those the page draws.
///
/// One sweep up the page: a line is at hand from [`SNAP`](crate::segment::drawn::SNAP) under
/// its low end to as much over its high end, sorted by its place, and each frame reads the
/// lines at hand between its ends at the height of each of its two rules. `work` counts a step
/// for each line, each frame and each line read; past
/// [`STRIP_WORK`](crate::extract::pairs::strips::STRIP_WORK) the sweep stops with `None`.
fn lines_at_rules(
    frames: &[Place],
    vs: &[crate::segment::drawn::Line],
    work: &mut usize,
) -> Option<Vec<Vec<u32>>> {
    use crate::extract::pairs::strips::STRIP_WORK;
    use std::collections::BTreeSet;
    let snap = crate::segment::drawn::SNAP;
    // At one height a line comes to hand before a frame reads there, and goes after it.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum What {
        Come,
        Read,
        Go,
    }
    let mut events: Vec<(f32, What, u32)> = Vec::with_capacity(2 * (vs.len() + frames.len()));
    for (k, v) in vs.iter().enumerate() {
        events.push((v.from - snap, What::Come, k as u32));
        events.push((v.to + snap, What::Go, k as u32));
    }
    for (f, &(bottom, top, _, _)) in frames.iter().enumerate() {
        events.push((bottom, What::Read, f as u32));
        events.push((top, What::Read, f as u32));
    }
    *work += events.len();
    crate::work::add(events.len() as u64);
    if *work > STRIP_WORK {
        return None;
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut at_hand: BTreeSet<(i64, u32)> = BTreeSet::new();
    let mut out: Vec<Vec<u32>> = vec![Vec::new(); frames.len()];
    for (_, what, k) in events {
        match what {
            What::Come => {
                at_hand.insert((x_key(vs[k as usize].at), k));
            }
            What::Go => {
                at_hand.remove(&(x_key(vs[k as usize].at), k));
            }
            What::Read => {
                let (_, _, x0, x1) = frames[k as usize];
                if x0 > x1 {
                    continue;
                }
                for &(_, v) in at_hand.range((x_key(x0), 0)..=(x_key(x1), u32::MAX)) {
                    *work += 1;
                    crate::work::add(1);
                    if *work > STRIP_WORK {
                        return None;
                    }
                    out[k as usize].push(v);
                }
            }
        }
    }
    // A line at both rules of a frame was read twice; `vs` is sorted by place, so are these.
    for lines in &mut out {
        lines.sort_unstable();
        lines.dedup();
    }
    Some(out)
}

/// The strips of the page of `view` (see [`StripFrame`]), found once: the frames of the page's
/// horizontal rules (see [`rule_frames_under`]), and the runs by one pass over them
/// sorted by the height of their middle. A strip inside a grid (its top rule is a rule of the
/// grid, see [`crate::extract::pairs::strips::frame_tops`]) is a row of the grid and reads as
/// no fields: it is the table typer's, as the reader leaves it, and the column rules of a
/// table never part its row into fields. Each other strip is read with the dividers the page
/// draws
/// in it (see [`crate::extract::pairs::strips::dividers_of`]), looked for among the lines
/// that stand at its rules (see [`lines_at_rules`]) within
/// [`STRIP_WORK`](crate::extract::pairs::strips::STRIP_WORK) steps for the page, as the
/// reader's search for fields between rules; past them no strip has dividers
/// ([`Strips::unparted`]).
pub(crate) fn strip_frames(view: &PageView<'_>) -> Strips {
    use crate::extract::pairs::strips::{STRIP_WORK, Top, dividers_of, frame_tops};
    let Ok(grids) = view.drawn.grids.as_ref() else {
        return Strips::default();
    };
    let frames = rule_frames_under(&grids.hs);
    if frames.is_empty() {
        return Strips::default();
    }
    // A rule a grid takes tops a row of the grid, which is the table typer's to read; the
    // lowest rule of a grid, and the upper rule of a row of fields open at both ends, top a
    // strip (see [`frame_tops`]).
    let tops = frame_tops(&grids.hs, &grids.vs, &grids.groups, true);
    crate::work::add(grids.hs.len() as u64);
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
    // The frames that hold runs, each with its runs and whether it stands in a grid.
    let mut held: Vec<(Place, Vec<u32>)> = Vec::new();
    let mut in_grid: Vec<bool> = Vec::new();
    for ((bottom, top, x0, x1), piece) in frames {
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
        held.push(((bottom, top, x0, x1), runs));
        in_grid.push(tops[piece] == Top::No);
    }
    // The dividers the page draws in each strip part its fields, as its labels do. A row of
    // a grid is not read, so its column rules are not looked for.
    let mut vs: Vec<crate::segment::drawn::Line> = grids.vs.clone();
    vs.sort_by(|a, b| a.at.total_cmp(&b.at));
    crate::work::add(vs.len() as u64);
    let places: Vec<Place> = held
        .iter()
        .zip(&in_grid)
        .map(|(h, &inside)| if inside { EMPTY_PLACE } else { h.0 })
        .collect();
    let mut work = 0usize;
    let dividers: Option<Vec<Vec<f32>>> = lines_at_rules(&places, &vs, &mut work).and_then(|at| {
        let mut all = Vec::with_capacity(at.len());
        for (lines, &(bottom, top, x0, x1)) in at.iter().zip(&places) {
            let near: Vec<crate::segment::drawn::Line> =
                lines.iter().map(|&k| vs[k as usize]).collect();
            let found = dividers_of(&near, (bottom, top), (x0, x1), &mut || {
                work += 1;
                crate::work::add(1);
                work <= STRIP_WORK
            })?;
            all.push(found.into_iter().map(|d| d.0).collect());
        }
        Some(all)
    });
    let unparted = dividers.is_none();
    let mut out = Vec::with_capacity(held.len());
    for (k, ((bottom, top, x0, x1), runs)) in held.into_iter().enumerate() {
        let own: Vec<&TextRun> = runs.iter().map(|&i| &texts[i as usize]).collect();
        let parted: &[f32] = dividers.as_ref().map_or(&[], |d| &d[k]);
        let read = if in_grid[k] {
            FrameRead::No
        } else {
            read_frame(&own, x0, x1, parted, how)
        };
        out.push(StripFrame {
            bottom,
            top,
            x0,
            x1,
            runs,
            read,
        });
    }
    Strips {
        frames: out,
        unparted,
    }
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
/// fields: the stack is spatial until the engine reads stacks (the older reader of labels under
/// their values runs before, as a given region; its move into the engine comes after 0.1.0).
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
    /// a value of two lines or pieces, a line that mixes labels and another field's value, or
    /// a field edge in a number in digit groups (see `strips::undecided`).
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

/// The runs `own` of a strip between rules from `x0` to `x1`, parted by its drawn `dividers`
/// (see [`crate::extract::pairs::strips::dividers_of`]), read as fields (the older rule of
/// fields between rules, one copy): the runs are cut at field edges first, as the older reader
/// cuts them (see [`field_cuts`]), on this reading's own copies of the pieces.
fn read_frame(own: &[&TextRun], x0: f32, x1: f32, dividers: &[f32], how: Reading<'_>) -> FrameRead {
    let frame = FieldFrame { x0, x1, dividers };
    let all: Vec<usize> = (0..own.len()).collect();
    // A field edge in a number in digit groups: the fields are not decided.
    let undecided = undecided(own, &frame, &all);
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
    if undecided {
        return FrameRead::Unclear;
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
pub const ITEM_PITCH: f32 = 2.0;

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

/// Whether the one line `range` of `runs` stands right under a row of smaller text: the
/// nearest row of the page over it that overlaps it along the line stands within
/// [`ITEM_PITCH`] sizes, is set a size step ([`SIZE_STEP`]) smaller and holds no run that
/// reads as an amount or a date (the cut's reading, [`reads_as_value`], which knows an
/// amount with the word of its currency too: the engine's own, so every host gets the same
/// answer). The row is found by its place (the page's lines by their
/// baselines, every line on the baseline of the nearest one, within [`BASELINE_NEAR`]), not
/// by the order the reader numbers its lines in: labels that are each a line of their own, or
/// that the reader lists after their values, are that row too. Small labels over the row of
/// their values look so (`Bank   Account   Paid` over `Oakfield Bank   30.10.2026`): the
/// row of values is no label and its value, though its first piece is words and its second
/// a date. When the cut pairs such rows they are a region of their own; when it cannot (a
/// value under two labels), the values stay a line.
///
/// [`BASELINE_NEAR`]: crate::segment::cut::BASELINE_NEAR
/// [`reads_as_value`]: crate::segment::cut::reads_as_value
fn under_smaller_line(
    view: &PageView<'_>,
    runs: &[&TextRun],
    range: &crate::extract::table::RunRange,
) -> bool {
    let (Some(first), Some(last)) = (runs.get(range.start), runs.get(range.end.wrapping_sub(1)))
    else {
        return false;
    };
    let near = crate::segment::cut::BASELINE_NEAR * range.size;
    let (by, lines, texts) = (view.by_y(), view.lines(), &view.page.texts);
    let runs_of = |k: usize| {
        let l = &lines[k];
        &texts[l.start.min(texts.len())..l.end.min(texts.len())]
    };
    let (x0, x1) = (first.x, last.x + last.w);
    // The lines over this one, the nearest first: past its own baseline (a line beside it
    // on a baseline a hair higher is its own row) up to the pitch of a block.
    let from = by.line_y.partition_point(|y| *y <= range.y + near);
    let to = by
        .line_y
        .partition_point(|y| *y <= range.y + ITEM_PITCH * range.size);
    let window = from..to.max(from);
    crate::work::add(1 + window.len() as u64);
    let overlaps = |k: usize| {
        let over = runs_of(k);
        crate::work::add(over.len() as u64);
        over.iter().any(|r| r.x < x1 && r.x + r.w > x0)
    };
    let Some(at) = window.clone().find(|&i| overlaps(by.lines[i])) else {
        return false;
    };
    // Every line of that row: the lines of the window on its baseline.
    let row: Vec<usize> = window
        .filter(|&i| (by.line_y[i] - by.line_y[at]).abs() <= near)
        .map(|i| by.lines[i])
        .collect();
    // Labels hold no amount and no date: a smaller row with one (`Subtotal 120.00` over a
    // larger `Total 127.50`) is a row of its own, and the line under it is one too.
    // An amount with its currency in letters is one too (`1.200 EUR`).
    let labels = !row
        .iter()
        .flat_map(|&k| runs_of(k))
        .any(|r| crate::segment::cut::reads_as_value(&r.text));
    let size = row.iter().map(|&k| lines[k].size).fold(0.0f32, f32::max);
    labels && SIZE_STEP * size <= range.size
}

/// `label_value.underlined_label`: two lines alone, the upper one a short label with an
/// underline of its own and the lower one its value (see [`underlined_label`]). The readers
/// that extract such a region read the pair as they read a row of labels over their values
/// ([`crate::extract::pairs::blocks::labels_with_values`]); it weighs what such a row does
/// ([`LABEL_OVER_VALUE`]).
pub const LABEL_UNDERLINED_ID: &str = "label_value.underlined_label";

/// The underline of a line of text lies at most this share of its text size under its
/// baseline. The faces set their underline about a tenth of the size under the baseline
/// (0.075 to 0.1 in the standard faces), and a line drawn by a writer stays over the end of
/// the descenders, a fifth to a quarter of the size. A rule farther down is a rule between
/// two rows (half way to the next baseline at any pitch of a block, 0.5 sizes or more).
const UNDERLINE_DROP: f32 = 0.25;

/// Each end of the underline of a line of text stands within this share of the text size of
/// the end of the text: about the width of a letter, so an underline drawn with the blank
/// before or after its words is one. A rule that goes on farther is as long as a field or a
/// column, not as the words over it: a line to fill in, on which a value may stand as well
/// as a label, so it says nothing about which of two lines is the label.
const UNDERLINE_ENDS: f32 = 0.5;

/// The share of the words of the line under an underlined label that hold a digit, a slash
/// or an at sign, from which the line reads as a value. A share of the line's own words, no
/// count: one such word makes `Room 12` or `N/A` a value, but not a line of plain words
/// around one number (`Budget 2026 review`), which is text under a heading. Half, because a
/// value of two words is as often a word with its number as two marked words; below half
/// the plain words are the larger part and the line reads as text.
const VALUE_WORDS_SHARE: f32 = 0.5;

/// Whether the two lines `ranges` of `runs` (one piece each, `pieces`) are a label with an
/// underline of its own over its value:
///
/// - a rule of the page lies under the baseline of the upper line, at most
///   [`UNDERLINE_DROP`] of its size, and each of its ends stands within [`UNDERLINE_ENDS`]
///   of the size of an end of the line's ink: the rule is as long as the words, their
///   underline. The underline is what sets the upper line apart as the label; a longer rule
///   is a line of a field, which a value may stand on with its label under it, and gives no
///   pair;
/// - the lower line has no such underline (two underlined lines are a list of links or of
///   titles) and starts at the left edge of the upper one (within [`EDGE_SIZES`]);
/// - the upper line is a short label (at most [`PAIR_LABEL_WORDS`] words) of plain words:
///   letters, a hyphen or an apostrophe inside a word, a dot or a colon only at its end. A
///   net address, a number or a code with an underline is a link or a value, no label;
/// - the lower line is short (at most [`PAIR_VALUE_WORDS`] words), no sentence, no label
///   itself (no word of it ends with a colon), and not set a size step ([`SIZE_STEP`])
///   smaller than the upper one (a small line under an underlined larger one is a note
///   under a title);
/// - the lower line reads as a value: a number, a date or a code ([`value_shape`]), or at
///   least [`VALUE_WORDS_SHARE`] of its words hold a digit, a slash or an at sign (`N/A`,
///   `A/B`, `Room 12`, a net address). The underline alone shows that the upper line is
///   set apart, not that it is a label: a writer underlines a filled value over its label
///   (`Alex Sample` over `Signature`), a name over its role, a link and a heading over a
///   line of text just so. With plain words under it, or words with one number among them
///   (`Budget 2026 review` under a heading), the two lines stay lines.
///
/// Anything else is no pair: the two lines stay lines.
fn underlined_label(
    view: &PageView<'_>,
    runs: &[&TextRun],
    ranges: &[crate::extract::table::RunRange],
    pieces: &[Vec<crate::extract::table::grid::Piece>],
) -> bool {
    let ([a, b], [pa, pb]) = (ranges, pieces) else {
        return false;
    };
    if pa.len() != 1 || pb.len() != 1 {
        return false;
    }
    // The upper line first, by place.
    let (up, low) = if a.y >= b.y { (a, b) } else { (b, a) };
    if up.y - low.y <= crate::segment::cut::BASELINE_NEAR * up.size {
        return false;
    }
    // The extent of the ink of a line: without the blanks its last run ends with (their
    // share of the run's width by its characters).
    let ink = |l: &crate::extract::table::RunRange| {
        let (first, last) = (runs[l.start], runs[l.end - 1]);
        let chars = last.text.chars().count().max(1) as f32;
        let kept = last.text.trim_end().chars().count() as f32;
        (first.x, last.x + last.w * kept / chars)
    };
    let underlined = |l: &crate::extract::table::RunRange| {
        let (x0, x1) = ink(l);
        let hs = view.hs();
        let from = hs.partition_point(|h| h.at < l.y - UNDERLINE_DROP * l.size);
        let to = hs.partition_point(|h| h.at < l.y);
        crate::work::add(1 + to.saturating_sub(from) as u64);
        let ends = UNDERLINE_ENDS * l.size;
        hs[from..to.max(from)]
            .iter()
            .any(|h| (h.from - x0).abs() <= ends && (h.to - x1).abs() <= ends)
    };
    if !underlined(up) || underlined(low) {
        return false;
    }
    if (ink(up).0 - ink(low).0).abs() > EDGE_SIZES * up.size.max(low.size) {
        return false;
    }
    if up.size >= SIZE_STEP * low.size {
        return false;
    }
    let text =
        |l: &crate::extract::table::RunRange| words(runs, &(l.start..l.end).collect::<Vec<_>>());
    let (label, value) = (text(up), text(low));
    let plain = |w: &str| {
        let w = w.trim_end_matches(['.', ':']);
        w.chars().any(char::is_alphabetic)
            && w.chars()
                .all(|c| c.is_alphabetic() || matches!(c, '-' | '\''))
    };
    let label_words = label.split_whitespace().count();
    let value_words = value.split_whitespace().count();
    (1..=PAIR_LABEL_WORDS).contains(&label_words)
        && label.split_whitespace().all(plain)
        && (1..=PAIR_VALUE_WORDS).contains(&value_words)
        && !value.trim_end().ends_with(['.', '!', '?'])
        && !value.contains(". ")
        && !value.split_whitespace().any(|w| w.ends_with(':'))
        && (value_shape(&value) || {
            let marked = value
                .split_whitespace()
                .filter(|w| {
                    w.chars()
                        .any(|c| c.is_ascii_digit() || matches!(c, '/' | '@'))
                })
                .count();
            marked as f32 >= VALUE_WORDS_SHARE * value_words as f32
        })
}

/// The colons of `run` that end a word or are a word of their own (white space or the end of
/// the text after them), each with its byte in the text and its place along the line: the
/// share of the run's width that the characters before it have of the text without the
/// blanks at its end. That is the place of the colon in a fixed-width face, whose listings
/// have columns of colons; in another face it is the place only of a colon that opens the
/// run.
fn colons<R: LineRun>(run: &R) -> impl Iterator<Item = (usize, f32)> + '_ {
    let text = run.text().trim_end();
    let chars = text.chars().count().max(1) as f32;
    text.char_indices()
        .enumerate()
        .filter(move |&(_, (at, c))| {
            c == ':'
                && text[at + 1..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map(move |(n, (at, _))| (at, run.x() + run.w() * n as f32 / chars))
}

/// Whether another line of the block of the line `range` has a colon at `x` along the line,
/// within [`EDGE_SIZES`] of the text size: a line of the page whose baseline is at most
/// [`ITEM_PITCH`] sizes over or under this one (the pitch of the lines of a block of labels
/// and values; a line on this baseline is the line itself), with a colon that ends a word
/// or is a word of its own there (see [`colons`]). The lines are found by their place on
/// the page, in whatever segments the cut put them: a listing whose lines the cut parted
/// has its column of colons all the same.
///
/// [`EDGE_SIZES`]: crate::line::EDGE_SIZES
fn colon_column(view: &PageView<'_>, range: &crate::extract::table::RunRange, x: f32) -> bool {
    let near = crate::segment::cut::BASELINE_NEAR * range.size;
    let reach = ITEM_PITCH * range.size;
    let (by, lines, texts) = (view.by_y(), view.lines(), &view.page.texts);
    let from = by.line_y.partition_point(|y| *y < range.y - reach);
    let to = by.line_y.partition_point(|y| *y <= range.y + reach);
    crate::work::add(1 + to.saturating_sub(from) as u64);
    (from..to)
        .filter(|&i| (by.line_y[i] - range.y).abs() > near)
        .any(|i| {
            let l = &lines[by.lines[i]];
            let runs = &texts[l.start.min(texts.len())..l.end.min(texts.len())];
            crate::work::add(runs.iter().map(|r| r.text.len() as u64).sum());
            runs.iter()
                .filter(|r| !r.off_axis)
                .flat_map(colons)
                .any(|c| (c.1 - x).abs() <= crate::line::EDGE_SIZES * range.size)
        })
}

/// How a line stands in its group, to the rule of line pairs ([`line_pair`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stand {
    /// The only line of its group.
    Lone,
    /// One of several lines of its group.
    Among,
    /// A line whose two pieces are cells whatever they read as: it stands in a drawn grid
    /// of two columns or more and its first piece ends with no colon, or right under a row
    /// of smaller labels (the row of their values, see [`under_smaller_line`]). Two pieces
    /// of it are never a label and its value.
    Cells,
}

/// The label and the value of one line of `runs` (its pieces `pieces`), each pair with
/// evidence of its own, whether the label ends with a colon, and where the line parts:
///
/// - one piece that holds a colon after a short label (at most [`PAIR_LABEL_WORDS`] words with
///   a letter): the value after it is shaped as a number, date or code, or short (at most
///   [`PAIR_VALUE_WORDS`] words) and no sentence; the line parts after the colon;
/// - as the only line of its group ([`Stand::Lone`]), two pieces: a short label with a letter and a
///   value shaped as a number, a date or a code (`Rechnungs-Nr.   2026004711`), or, after a
///   label that ends with a colon, a short value that is no sentence and no label; the value
///   not set a size step ([`SIZE_STEP`]) smaller than its label: a small code beside a larger
///   name (`United States Treasury` and a `000` in two thirds of its size) is a mark printed
///   beside it, not its value; after a label that ends with a colon a smaller value is its
///   value; the line parts between the pieces;
/// - two pieces, the first of which is a short label that ends with a colon (`Sheet:` and
///   `03.04.2026`, in the head block of a letter), alone or among other lines
///   ([`Stand::Among`]): the second piece is its value by the rule of the lone line. The
///   colon says the first piece is a label, which among other lines a value shape alone
///   does not (a name and an amount are a row of a table). The line parts between the
///   pieces;
/// - two pieces, the second of which opens with a colon as a word of its own (`Currency`
///   and `: EUR`, a listing in a fixed-width face that pads its labels up to a column of
///   colons): the line parts after the colon. The colon is the label's, and the line is
///   read as the one before, only when `column` says for the place of the colon along the
///   line that another line of the block has a colon there (see [`colon_column`]): two
///   lines that share the column are a listing, one colon before a cell is chance. Without
///   that the colon says nothing, and the line is a pair only as a lone line with a value
///   shaped as a number, a date or a code;
/// - two pieces, the first of which holds a colon with words after it (`Phone: 12` and
///   `Fax 34`, or `Ref: AB/CD` and a number): no pair. The first piece is a label and its
///   value already, and the gap says the second piece is another cell: it never joins that
///   value, whatever it reads as.
///
/// Two pieces of a line that stands as cells ([`Stand::Cells`]) are no pair by any of these.
///
/// A line that opens as a caption ([`crate::extract::table::caption::caption`]) is none.
/// Anything else is no pair; more pieces are a row, which the table rules read.
fn line_pair<R: LineRun>(
    runs: &[R],
    pieces: &[crate::extract::table::grid::Piece],
    stand: Stand,
    column: &dyn Fn(f32) -> bool,
) -> Option<LinePair> {
    let lone = stand == Stand::Lone;
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
        [a, b] => {
            let (mut label, mut value) = (text(a), text(b));
            // A colon that opens the second piece as a word of its own, after a label
            // without one: the colon of the label, set at a column of its own (a listing
            // in a fixed-width face pads its labels up to the colon), and the line parts
            // after it. It says the first piece is a label as a colon at its end does when
            // another line of the block has a colon in that column; one such colon alone
            // may stand before a cell by chance and says nothing.
            let mut cut = PairCut {
                run: b.start,
                byte: 0,
            };
            let mut chance = false;
            if !label.ends_with(':')
                && let Some(rest) = value.strip_prefix(':')
                && rest.starts_with(char::is_whitespace)
                && let Some(at) = colons(&runs[b.start]).next()
            {
                label.push_str(" :");
                value = rest.trim().to_string();
                cut.byte = at.0 + 1;
                chance = !column(at.1);
            }
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
            let colon = label.ends_with(':') && !chance;
            let no_label = |v: &str| !v.split_whitespace().any(|w| w.ends_with(':'));
            let value_ok =
                value_shape(&value) || (colon && no_sentence(&value) && no_label(&value));
            // The first piece holds a colon with words after it: it is a label and its value
            // already, and the piece after the gap is another cell, never more of that value
            // (`Phone: 12   Fax 34`, `Date: 03.04.2026   Page 1 of 2`). The line stays a line.
            if label.contains(": ") {
                return None;
            }
            // Alone, or among other lines after a label that ends with a colon: the two
            // pieces are the label and its value, the whole line.
            // The words of the label are counted without its colon, as in one piece: a
            // colon set apart from a padded label (`Name of the seller :`) is no word.
            let pair = (lone || (colon && stand == Stand::Among))
                && short_label(label.trim_end_matches(':'))
                && value_ok
                && (!smaller || colon);
            pair.then_some(LinePair { value, colon, cut })
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
    let stand = if lone { Stand::Lone } else { Stand::Among };
    // The readers ask for a line the typer read as a pair: its colon has its column.
    let cut = line_pair(&inked, &pieces, stand, &|_| true)?.cut;
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
/// (`label_value.key_values`, `label_value.labels_over_values`); a label with an underline of
/// its own over its value (`label_value.underlined_label`); labels with a colon
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
        // A small colon label set into a data line, kept with the text after it as one
        // plain line, and the head of that line before the label: the cut parted the line
        // and claimed no pair, and no pair rule reads one into a part of it (the text after
        // the label may be a cell of a column, or the value may stand under the label; a
        // date after a title in the head is no value of the title).
        Source::Line => return None,
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
        // A row of labels over their values between two rules is a strip of fields: the
        // strip rule above reads it with the strips around it. Any other such row is read
        // by the pairs the cut found.
        Source::Cell if text_alone(view, seg) => {
            let under = crate::extract::pairs::blocks::labels_with_values(&refs)?;
            let filled = under.iter().filter(|u| !u.value.is_empty()).count();
            signals.push(signal(
                LABEL_OVER_VALUE_ID,
                format!(
                    "{} labels side by side, {filled} with a value under it",
                    under.len()
                ),
                LABEL_OVER_VALUE,
            ));
            if under.iter().all(|u| words(&refs, &u.label).ends_with(':')) {
                signals.push(signal(
                    "label_value.colon",
                    "the labels end with a colon".into(),
                    LABEL_COLON,
                ));
            }
            if filled > 0
                && under
                    .iter()
                    .filter(|u| !u.value.is_empty())
                    .all(|u| value_shape(&words(&refs, &u.value)))
            {
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
            } else if underlined_label(view, &runs, &ranges, &pieces) {
                signals.push(signal(
                    LABEL_UNDERLINED_ID,
                    "a label with an underline of its own over its value".into(),
                    LABEL_OVER_VALUE,
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
                // A first piece that ends with a colon says it is a label, in a grid too
                // (`Total:` and an amount in the rest of a ruled sheet): the grid makes
                // cells only of a line without one, whose pair would rest on the shape of
                // its value alone.
                let colon_label = |k: usize| {
                    let first = &pieces[k][0];
                    let ids: Vec<usize> = (first.start..first.end).collect();
                    words(&runs, &ids).ends_with(':')
                };
                // Nor is a line right under a line of smaller text a label and its value on
                // its own: it is the row of the values of those labels (see
                // [`under_smaller_line`]), whatever its first piece reads as. Both hold for
                // the only line of a group and for each line among others; a line of one
                // piece is read by its colon wherever it stands, so only lines of two
                // pieces are asked.
                let cells = |k: usize| {
                    pieces[k].len() == 2
                        && ((in_grid && !colon_label(k))
                            || under_smaller_line(view, &runs, &ranges[k]))
                };
                // An item of a list (see [`list_item`]) is no pair on its own.
                if ranges.len() == 1 && list_item(view, &runs, &ranges[0]) {
                    return None;
                }
                let pairs: Option<Vec<LinePair>> = pieces
                    .iter()
                    .enumerate()
                    .map(|(k, p)| {
                        let stand = match ranges.len() {
                            _ if cells(k) => Stand::Cells,
                            1 => Stand::Lone,
                            _ => Stand::Among,
                        };
                        line_pair(&runs, p, stand, &|x| colon_column(view, &ranges[k], x))
                    })
                    .collect();
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
    let grp = group_at(view, g, &seg.bounds)?;
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
    let grp = group_at(view, g, &seg.bounds)?;
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

    /// The frames of [`rule_frames_under`] without their top pieces.
    fn rule_frames(rules: &[crate::segment::drawn::Line]) -> Vec<Place> {
        rule_frames_under(rules).into_iter().map(|f| f.0).collect()
    }

    /// The frames by a scan over every higher piece (each open stretch of a piece with the
    /// nearest piece above it that overlaps it): what [`rule_frames_under`] finds by its sweep.
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

    /// The sweep finds the frames a scan over every higher piece finds, on
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

    /// A chain of rules each 1.5 points under the one
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

    /// Pieces in two columns, none over another, cost the sweep about one
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

    /// The dividers of a strip are looked for among the lines that
    /// stand at its rules, not among every line between its ends. Many strips over many
    /// short lines at other heights cost about a step for each strip and each line, and the
    /// lines found hold every divider a scan of all lines finds.
    #[test]
    fn the_dividers_of_many_strips_are_looked_for_at_their_rules_in_linear_work() {
        use crate::extract::pairs::strips::dividers_of;
        use crate::segment::drawn::Line;
        let page = |n: usize| {
            // Strips 4 points high, one on another, each with a divider of its own in its
            // middle; and as many short lines far over them, between their ends.
            let frames: Vec<(f32, f32, f32, f32)> = (0..n)
                .map(|k| (4.0 * k as f32, 4.0 * k as f32 + 4.0, 0.0, 200.0))
                .collect();
            let mut vs: Vec<Line> = (0..n)
                .map(|k| Line {
                    at: 100.0,
                    from: 4.0 * k as f32,
                    to: 4.0 * k as f32 + 4.0,
                })
                .collect();
            vs.extend((0..n).map(|k| Line {
                at: 1.0 + 198.0 * k as f32 / n as f32,
                from: 4.0 * n as f32 + 100.0,
                to: 4.0 * n as f32 + 103.0,
            }));
            vs.sort_by(|a, b| a.at.total_cmp(&b.at));
            (frames, vs)
        };
        let steps = |n: usize| {
            let (frames, vs) = page(n);
            let mut work = 0usize;
            let at = lines_at_rules(&frames, &vs, &mut work).expect("within the bound");
            (frames, vs, at, work)
        };
        let (frames, vs, at, work) = steps(1000);
        for (lines, &(bottom, top, x0, x1)) in at.iter().zip(&frames) {
            // The strip's own divider and those of the strips that share a rule with it.
            assert!(lines.len() <= 3, "{lines:?}");
            let near: Vec<Line> = lines.iter().map(|&k| vs[k as usize]).collect();
            let found = dividers_of(&near, (bottom, top), (x0, x1), &mut || true);
            let scanned = dividers_of(&vs, (bottom, top), (x0, x1), &mut || true);
            assert_eq!(found, scanned);
            assert_eq!(found.map(|d| d.len()), Some(1));
        }
        // 1,000 strips against 2,000 lines: a scan of the lines between the ends of each
        // strip is two million steps.
        assert!(work < 12 * 1000, "{work} steps");
        let (.., double) = steps(2000);
        assert!(double < 3 * work, "{double} steps after {work}");
    }

    /// The search for dividers takes the bound of the reader's
    /// search for fields between rules. Lines that stand at the rules of many strips cost a
    /// step for each strip they stand in; past `STRIP_WORK` steps the search gives up.
    #[test]
    fn the_search_for_dividers_stops_at_the_bound_of_the_strip_search() {
        use crate::extract::pairs::strips::STRIP_WORK;
        use crate::segment::drawn::Line;
        let sweep = |strips: usize| {
            let frames: Vec<(f32, f32, f32, f32)> = (0..strips)
                .map(|k| (4.0 * k as f32, 4.0 * k as f32 + 4.0, 0.0, 2000.0))
                .collect();
            // A thousand lines down all the strips.
            let vs: Vec<Line> = (0..1000)
                .map(|k| Line {
                    at: 1.0 + 1.5 * k as f32,
                    from: 0.0,
                    to: 4.0 * strips as f32,
                })
                .collect();
            let mut work = 0usize;
            let found = lines_at_rules(&frames, &vs, &mut work);
            (found.map(|f| f.len()), work)
        };
        // Each strip reads the thousand lines at its two rules.
        let (found, work) = sweep(STRIP_WORK / 2000 - 2);
        assert_eq!(found, Some(STRIP_WORK / 2000 - 2));
        assert!(work <= STRIP_WORK);
        let (found, work) = sweep(STRIP_WORK / 2000 + 1);
        assert_eq!(found, None);
        assert_eq!(work, STRIP_WORK + 1);
    }

    /// A strip inside a grid is left to the table typer, as the
    /// reader leaves it: a row of a table of rates between its row rules, with its column
    /// rules, is no row of fields (its small values would read as the labels of empty
    /// fields). The same row between two rules of its own, with dividers that hang from the
    /// upper one, stands in no grid and is read; so is the strip under the lowest rule of a
    /// grid.
    #[test]
    fn a_table_row_between_the_rules_of_a_grid_is_no_strip_of_fields() {
        let line = |x0: f32, y0: f32, x1: f32, y1: f32| RuleIn {
            x0,
            y0,
            x1,
            y1,
            thick: 0.5,
            dashed: false,
            rect: false,
        };
        let sized = |text: &str, x: f32, y: f32, size: f32, line: u32| TextRun {
            y,
            w: 0.5 * size * text.chars().count() as f32,
            h: size,
            size,
            line,
            ..on_line(text, x)
        };
        let xs = [72.0, 222.0, 372.0, 522.0];
        let cells = |row: [&str; 3], y: f32, line: u32| -> Vec<TextRun> {
            row.iter()
                .zip(xs)
                .map(|(t, x)| sized(t, x + 4.0, y, 7.0, line))
                .collect()
        };
        // The reads of the strips of a page of `rules` that hold the row of rates at 664,
        // and of the strip of small labels under 640, with text in a larger size elsewhere.
        let reads = |rules: Vec<RuleIn>| {
            let mut texts: Vec<TextRun> = (0..3)
                .map(|k| {
                    let y = 780.0 - 14.0 * k as f32;
                    sized(
                        "Tall crates stand in the depot by the gate",
                        72.0,
                        y,
                        10.0,
                        k,
                    )
                })
                .collect();
            texts.extend(cells(["Rate", "Net", "Tax"], 684.0, 3));
            texts.extend(cells(["7%", "160.00", "11.20"], 664.0, 4));
            texts.extend(cells(["19%", "200.00", "38.00"], 644.0, 5));
            texts.extend(cells(["Sheet", "Depot", "Days"], 630.0, 6));
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
            let strips = strip_frames(&view);
            let read_at = |y: f32| {
                strips
                    .frames
                    .iter()
                    .find(|f| f.bottom < y && y < f.top)
                    .map(|f| f.read.clone())
            };
            (read_at(666.0), read_at(632.0))
        };
        let clear = |r: &Option<FrameRead>| matches!(r, Some(FrameRead::Clear(_)));
        let ys = [700.0, 680.0, 660.0, 640.0, 620.0];
        // A grid of three rows with its column rules, and a rule under it.
        let mut grid: Vec<RuleIn> = ys.iter().map(|&y| line(72.0, y, 522.0, y)).collect();
        grid.extend(xs.iter().map(|&x| line(x, 640.0, x, 700.0)));
        let (row, under) = reads(grid);
        assert_eq!(row, Some(FrameRead::No));
        assert!(clear(&under), "{under:?}");
        // Rules alone, with dividers that hang from each rule over half of the row under it.
        let mut open: Vec<RuleIn> = ys.iter().map(|&y| line(72.0, y, 522.0, y)).collect();
        for &y in &ys[..4] {
            open.extend(xs[1..3].iter().map(|&x| line(x, y - 10.0, x, y)));
        }
        let (row, under) = reads(open);
        assert!(clear(&row), "{row:?}");
        assert!(clear(&under), "{under:?}");
    }

    /// A page whose strips were read without their dividers says so
    /// in its issues, as a limit; another page says nothing.
    #[test]
    fn strips_read_without_their_dividers_are_an_issue_of_the_page() {
        let page = PageIn {
            page: 1,
            size: [595.0, 842.0],
            turn: 0,
            texts: vec![on_line("Depot", 72.0)],
            rules: Vec::new(),
            boxes: Vec::new(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given: Vec::new(),
        };
        let mut segs = segment_page(&page, &Limits::default(), &|| false);
        crate::typers::say_unparted(&mut segs, false);
        assert!(segs.issues.is_empty());
        crate::typers::say_unparted(&mut segs, true);
        assert_eq!(
            segs.issues,
            [crate::region::Issue {
                what: DIVIDERS_UNSEARCHED.to_string(),
                kind: crate::region::IssueKind::Limit,
            }]
        );
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

    /// The row over a line is found by its place, not by the
    /// order of the reader's lines. The values `Oakfield Bank   30.10.2026` come first in
    /// the reader's order and each small label over them is a line of its own after them:
    /// the values still stand under a row of smaller text. A row with an amount over them, a
    /// row in their own size, a row too far over them or beside them is no such row.
    #[test]
    fn the_row_over_a_line_is_found_by_place_in_any_line_order() {
        let run = |text: &str, x: f32, y: f32, size: f32, line: u32| TextRun {
            y,
            size,
            h: 0.7 * size,
            w: 0.5 * size * text.chars().count() as f32,
            line,
            ..on_line(text, x)
        };
        let under = |labels: &[(&str, f32, f32, f32)]| {
            let mut texts = vec![
                run("Oakfield Bank", 43.0, 700.0, 10.0, 0),
                run("30.10.2026", 203.0, 700.0, 10.0, 0),
            ];
            for (n, &(text, x, y, size)) in labels.iter().enumerate() {
                texts.push(run(text, x, y, size, 1 + n as u32));
            }
            let page = PageIn {
                page: 1,
                size: [595.0, 842.0],
                turn: 0,
                texts,
                rules: Vec::new(),
                boxes: Vec::new(),
                pictures: Vec::new(),
                codes: Vec::new(),
                given: Vec::new(),
            };
            let facts = FileFacts::default();
            let index = FileIndex::new(&facts);
            let structure = drawn(&page, &|_| true);
            let segs = segment_page(&page, &Limits::default(), &|| false);
            // No reading of amounts by the host: the row is read by the engine alone.
            let amount = |_: &str| false;
            let view = PageView::new(&page, &structure, &segs, &facts, &index, &amount);
            let runs: Vec<&TextRun> = page.texts.iter().collect();
            let range = crate::extract::table::run_ranges(&page.texts)[0];
            under_smaller_line(&view, &runs, &range)
        };
        let labels = [
            ("Bank", 40.0, 712.0, 8.0),
            ("Account", 80.0, 712.0, 8.0),
            ("Paid", 200.0, 712.0, 8.0),
        ];
        assert!(under(&labels));
        // One label alone over the line, and a baseline a hair off the others.
        assert!(under(&labels[..1]));
        let mut jitter = labels;
        jitter[1].2 = 712.5;
        assert!(under(&jitter));
        // An amount among them: a row of its own values, no labels.
        let mut amount = labels;
        amount[2].0 = "120.00";
        assert!(!under(&amount));
        // So is a whole number with its currency in letters; the number
        // alone may be a label (a year), and a word that ends like a currency is none.
        amount[2].0 = "120 EUR";
        assert!(!under(&amount));
        amount[2].0 = "2026";
        assert!(!under(&amount));
        amount[2].0 = "120 TEUR";
        assert!(under(&amount));
        // The size step moved both ways: 8 under 10 is a step, 9.5 is none.
        assert!(!under(&labels.map(|l| (l.0, l.1, l.2, 9.5))));
        // The pitch moved both ways: two sizes over the line, and past them.
        assert!(under(&labels.map(|l| (l.0, l.1, 720.0, l.3))));
        assert!(!under(&labels.map(|l| (l.0, l.1, 720.5, l.3))));
        // A small line beside the values, not over them, and no line at all.
        assert!(!under(&[("Bank", 400.0, 712.0, 8.0)]));
        assert!(!under(&[]));
        // A line beside the values on their own row is not over them.
        assert!(!under(&[("Paid", 400.0, 701.0, 8.0)]));
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

    /// A strip whose value line ends with a value of one character
    /// right after a drawn divider reads that character as the value of its own field, not
    /// glued to the value before it: the strip is clear, three fields with a value and the
    /// last one empty.
    #[test]
    fn a_one_character_value_after_a_divider_is_its_own_field() {
        let ch = 5.7;
        let sized = |text: &str, x: f32, y: f32, size: f32, line: u32| TextRun {
            y,
            w: 0.6 * size * text.chars().count() as f32,
            h: size,
            size,
            line,
            font_kind: FontKind::Monospace,
            ..on_line(text, x)
        };
        // The dividers stand a hair inside the box of the character after them.
        let dividers = [42.0 + 6.0 * ch + 0.46, 42.0 + 15.0 * ch + 0.46, 170.0];
        let mut value = sized("00123 01.02.70 3", 42.0, 679.0, 9.5, 1);
        value.gaps = [(1, 5), (2, 14)]
            .iter()
            .map(|&(words, k)| crate::input::GapIn {
                words,
                from: 42.0 + ch * k as f32,
                to: 42.0 + ch * (k + 1) as f32,
            })
            .collect();
        let runs = [
            sized("Number", 44.0, 692.0, 6.6, 0),
            sized("Born", dividers[0] + 2.0, 692.0, 6.6, 0),
            sized("Class", dividers[1] + 2.0, 692.0, 6.6, 0),
            sized("Factor", dividers[2] + 2.0, 692.0, 6.6, 0),
            value,
        ];
        let own: Vec<&TextRun> = runs.iter().collect();
        let how = Reading {
            one_field: false,
            smaller: true,
            small: small_size(&[6.6, 9.5]).saturating_sub(1),
            labels: &[],
        };
        let texts =
            |runs: &[TextRun]| -> Vec<String> { runs.iter().map(|r| r.text.clone()).collect() };
        match read_frame(&own, 40.0, 220.0, &dividers, how) {
            FrameRead::Clear(f) => {
                assert_eq!(texts(&f.labels), ["Number", "Born", "Class"]);
                assert_eq!(texts(&f.empty), ["Factor"]);
            }
            other => panic!("{other:?}"),
        }
    }

    /// The gap between the digit groups of a number
    /// is no field edge, and a drawn divider in it (or within its reach of it) or inside the
    /// number is no evidence of one number either: the strip reads as fields that are not
    /// clear, neither cut at the gap nor with the number in one field. So does `123 456`
    /// with a divider in its gap or in a group. With the divider less than a character into
    /// the leading digit, out of reach of the gap, the amount does not stand across it: it
    /// stays whole in its own field and the field before it keeps its own value or stays
    /// empty.
    #[test]
    fn a_divider_in_the_group_gap_of_an_amount_reads_unclear() {
        let sized = |text: &str, x: f32, y: f32, size: f32, line: u32| TextRun {
            y,
            w: 0.6 * size * text.chars().count() as f32,
            h: size,
            size,
            line,
            font_kind: FontKind::Monospace,
            ..on_line(text, x)
        };
        // The gap after the first group of `text`, which starts at 130.
        let read = |text: &str, divider: f32, count: Option<&str>| {
            let first = text.chars().take_while(char::is_ascii_digit).count() as f32;
            let mut amount = sized(text, 130.0, 679.0, 9.5, 1);
            amount.w -= 4.1;
            amount.gaps = vec![crate::input::GapIn {
                words: 1,
                from: 130.0 + 5.7 * first,
                to: 130.0 + 5.7 * first + 1.6,
            }];
            let mut runs = vec![
                sized("Count", 44.0, 692.0, 6.6, 0),
                sized("Amount", divider + 12.0, 692.0, 6.6, 0),
            ];
            runs.extend(count.map(|c| sized(c, 44.0, 679.0, 9.5, 1)));
            runs.push(amount);
            let own: Vec<&TextRun> = runs.iter().collect();
            let how = Reading {
                one_field: false,
                smaller: true,
                small: small_size(&[6.6, 9.5]).saturating_sub(1),
                labels: &[],
            };
            let texts =
                |runs: &[TextRun]| -> Vec<String> { runs.iter().map(|r| r.text.clone()).collect() };
            match read_frame(&own, 40.0, 220.0, &[divider], how) {
                FrameRead::Clear(f) => Some((texts(&f.labels), texts(&f.empty))),
                FrameRead::Unclear => None,
                FrameRead::No => panic!("divider at {divider}: no fields"),
            }
        };
        let amount = "1\u{2009}234,56";
        let none: [&str; 0] = [];
        // In the leading digit, out of reach of the gap: clear, the amount whole.
        let (labels, empty) = read(amount, 133.0, None).expect("clear");
        assert_eq!(
            (labels, empty),
            (vec!["Amount".into()], vec!["Count".into()])
        );
        let (labels, empty) = read(amount, 133.0, Some("7")).expect("clear");
        assert_eq!(labels, ["Count", "Amount"]);
        assert_eq!(empty, none);
        // A hair before the group gap, in it and a hair past it: not clear.
        for divider in [135.0, 136.0, 137.5] {
            assert_eq!(read(amount, divider, None), None, "divider at {divider}");
            assert_eq!(
                read(amount, divider, Some("7")),
                None,
                "divider at {divider}"
            );
        }
        // Inside the second group, more than a character from the number's ends: not clear.
        assert_eq!(read(amount, 140.0, None), None);
        assert_eq!(read(amount, 140.0, Some("7")), None);
        // Two groups of three: two values or one number, at the gap and in a group.
        for count in [None, Some("7")] {
            assert_eq!(read("123\u{2009}456", 148.0, count), None);
            assert_eq!(read("123\u{2009}456", 144.0, count), None);
        }
    }

    /// A number led by a sign (`-`, `+`, U+2212), an opening
    /// bracket, a currency sign or a currency code, with a drawn divider in its digit group
    /// gap, reads as fields that are not clear: the sign and the first group are not the
    /// value of the field before the divider. The other way: letters before the first digit
    /// that are no currency code make a code, cut at the divider as before.
    #[test]
    fn a_divider_in_the_group_gap_of_a_number_with_a_sign_reads_unclear() {
        let sized = |text: &str, x: f32, y: f32, size: f32, line: u32| TextRun {
            y,
            w: 0.6 * size * text.chars().count() as f32,
            h: size,
            size,
            line,
            font_kind: FontKind::Monospace,
            ..on_line(text, x)
        };
        // The value run starts at 130 with one gap of 1.6 after its first `chars`
        // characters, the divider 0.8 into that gap; `Count` has no value of its own.
        let read = |text: &str, chars: usize| {
            let from = 130.0 + 5.7 * chars as f32;
            let before: String = text.chars().take(chars).collect();
            let mut value = sized(text, 130.0, 679.0, 9.5, 1);
            value.w -= 4.1;
            value.gaps = vec![crate::input::GapIn {
                words: before.split_whitespace().count() as u32,
                from,
                to: from + 1.6,
            }];
            let divider = from + 0.8;
            let runs = [
                sized("Count", 44.0, 692.0, 6.6, 0),
                sized("Amount", divider + 12.0, 692.0, 6.6, 0),
                value,
            ];
            let own: Vec<&TextRun> = runs.iter().collect();
            let how = Reading {
                one_field: false,
                smaller: true,
                small: small_size(&[6.6, 9.5]).saturating_sub(1),
                labels: &[],
            };
            let texts =
                |runs: &[TextRun]| -> Vec<String> { runs.iter().map(|r| r.text.clone()).collect() };
            match read_frame(&own, 40.0, 220.0, &[divider], how) {
                FrameRead::Clear(f) => Some((texts(&f.labels), texts(&f.empty))),
                FrameRead::Unclear => None,
                FrameRead::No => panic!("{text}: no fields"),
            }
        };
        for (text, chars) in [
            ("\u{2212}1 234,56", 2),
            ("(1 234,56)", 2),
            ("[1 234]", 2),
            ("\u{20ac}1 234,56", 2),
            ("$12 345.00", 3),
            ("EUR1 234,56", 4),
            ("-\u{20ac}1 234,56", 3),
            ("\u{20ac} 1 234,56", 3),
            ("EUR 1 234,56", 5),
            ("AB 1 234", 4),
        ] {
            assert_eq!(read(text, chars), None, "{text}");
        }
        let none: [&str; 0] = [];
        for (text, chars) in [("AB1 234,56", 3), ("Nr1 234", 3), ("#1 234,56", 2)] {
            let (labels, empty) = read(text, chars).expect("clear");
            assert_eq!(labels, ["Count", "Amount"], "{text}");
            assert_eq!(empty, none, "{text}");
        }
    }

    /// A field edge taken from the start of a label (no drawn
    /// divider) that falls in the digit group gap of a number, or inside the number, leaves
    /// the strip not clear as a drawn divider does: the digits say one number and the labels
    /// say two fields. The number is never read whole into one field with the other empty.
    /// The other way: a value line whose gap is no group gap is cut at the label's start.
    #[test]
    fn a_label_start_in_a_number_in_digit_groups_reads_unclear() {
        let sized = |text: &str, x: f32, y: f32, size: f32, line: u32| TextRun {
            y,
            w: 0.6 * size * text.chars().count() as f32,
            h: size,
            size,
            line,
            font_kind: FontKind::Monospace,
            ..on_line(text, x)
        };
        // Two small labels on one line, the second at `second`, and no divider; the value
        // line is one run from 44 with one gap `gap` wide after its first word, so that its
        // second word starts under the second label.
        let read = |labels: [&str; 2], second: f32, text: &str, gap: f32| {
            let first = text.chars().take_while(|c| *c != ' ').count() as f32;
            let mut value = sized(text, 44.0, 679.0, 9.5, 1);
            value.w += gap - 5.7;
            value.gaps = vec![crate::input::GapIn {
                words: 1,
                from: 44.0 + 5.7 * first,
                to: 44.0 + 5.7 * first + gap,
            }];
            let runs = [
                sized(labels[0], 44.0, 692.0, 6.6, 0),
                sized(labels[1], second, 692.0, 6.6, 0),
                value,
            ];
            let own: Vec<&TextRun> = runs.iter().collect();
            let how = Reading {
                one_field: false,
                smaller: true,
                small: small_size(&[6.6, 9.5]).saturating_sub(1),
                labels: &[],
            };
            let texts =
                |runs: &[TextRun]| -> Vec<String> { runs.iter().map(|r| r.text.clone()).collect() };
            match read_frame(&own, 40.0, 220.0, &[], how) {
                FrameRead::Clear(f) => Some((texts(&f.labels), texts(&f.empty))),
                FrameRead::Unclear => None,
                FrameRead::No => panic!("{text}: no fields"),
            }
        };
        for (labels, second, text, gap) in [
            (["A", "B"], 58.0, "12 345", 2.6),
            (["A", "B"], 56.0, "7 500", 6.3),
            (["A", "B"], 56.0, "7 500,00", 6.3),
            (["No", "Qty"], 60.0, "12 345", 4.6),
            (["No", "Qty"], 62.0, "12 345", 6.6),
            // The label start over the middle of "345", away from the group gap.
            (["A", "B"], 68.0, "12 345", 2.6),
            (["No", "Qty"], 68.0, "12 345", 4.6),
        ] {
            assert_eq!(
                read(labels, second, text, gap),
                None,
                "{text} under {second}"
            );
        }
        // No group after the gap: two values, each under its label.
        let none: [&str; 0] = [];
        for (labels, second, text, gap) in [
            (["A", "B"], 58.0, "12 3456", 2.6),
            (["No", "Qty"], 60.0, "12 34,5", 4.6),
        ] {
            let (found, empty) = read(labels, second, text, gap).expect("clear");
            assert_eq!(found, labels, "{text}");
            assert_eq!(empty, none, "{text}");
        }
    }

    /// The line pair rule says where a line parts: in a run after the colon
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
        // After a label that ends with a colon, a lone line's value may be any
        // short text that is no sentence; without the colon it needs a shape of its own.
        let coded = [
            on_line("Member Code:", 40.0),
            on_line("MC 4417 2093 88", 200.0),
        ];
        assert_eq!(
            line_pair_cut(&coded, true),
            Some(PairCut { run: 1, byte: 0 })
        );
        // The colon says so among other lines too. The line parts at
        // the gap after the colon, as a line of one piece parts after it.
        assert_eq!(
            line_pair_cut(&coded, false),
            Some(PairCut { run: 1, byte: 0 })
        );
        // A colon that opens the second piece as a word of its own is the
        // colon of the label (a listing in a fixed-width face pads its labels up to a
        // column of colons). The line parts after the colon, alone and among other lines,
        // whatever short text the value is.
        for value in [
            ": EUR",
            ": 549910",
            ": basic-2017",
            " : EUR",
            ":  two words",
        ] {
            let line = [on_line("Currency ", 40.0), on_line(value, 200.0)];
            let byte = value.find(':').map(|at| at + 1).unwrap_or(0);
            for lone in [true, false] {
                assert_eq!(
                    line_pair_cut(&line, lone),
                    Some(PairCut { run: 1, byte }),
                    "{value}"
                );
            }
        }
        // The colon of a padded label is no word of it. A label
        // of four words (`PAIR_LABEL_WORDS`) pairs as it does in one run, one of five does
        // not, in one run or in two.
        for (label, pairs) in [
            ("Name of the seller", true),
            ("Name of the first seller", false),
        ] {
            let padded = [
                on_line(&format!("{label} "), 40.0),
                on_line(": Rowan", 300.0),
            ];
            let glued = [on_line(&format!("{label}: Rowan"), 40.0)];
            assert_eq!(line_pair_cut(&padded, false).is_some(), pairs, "{label}");
            assert_eq!(line_pair_cut(&glued, false).is_some(), pairs, "{label}");
        }
        // The colon in a run of its own: the line parts after that run.
        let apart = [
            on_line("Currency ", 40.0),
            on_line(":", 200.0),
            on_line(" EUR", 206.0),
        ];
        assert_eq!(
            line_pair_cut(&apart, false),
            Some(PairCut { run: 2, byte: 0 })
        );
        // No colon of the label: a colon glued to what follows it (a time, a mark), a colon
        // alone with no value, a sentence after it, a label that has its colon already
        // (the second piece is another cell) and a first piece that is no short label.
        for (label, value) in [
            ("Currency ", ":EUR"),
            ("Opens ", ":30 past nine"),
            ("Currency ", ":"),
            (
                "Note ",
                ": this is a sentence that says much more than a value.",
            ),
            ("Currency: EUR ", ": USD"),
            ("12 ", ": 34"),
            ("one two three four five six seven ", ": EUR"),
        ] {
            let line = [on_line(label, 40.0), on_line(value, 300.0)];
            assert_eq!(line_pair_cut(&line, false), None, "{label}{value}");
        }
        // A first piece that holds a label, its colon and a value
        // is a pair of its own, and the piece after the gap is another cell. It never joins
        // the value, whatever it reads as: a number (`Ref: AB/CD` is not `20481/55012` too),
        // words, or a word and a number. The line stays a line, alone or among other lines.
        for (first, second) in [
            ("Ref: AB/CD", "20481/55012"),
            ("Phone: 030 123456", "Fax 030 123457"),
            ("Date: 03.04.2026", "Page 1 of 2"),
            ("Name: Smith", "Apples"),
            ("Order: 4711", "03.04.2026"),
        ] {
            let line = [on_line(first, 40.0), on_line(second, 200.0)];
            for lone in [true, false] {
                assert_eq!(line_pair_cut(&line, lone), None, "{first} | {second}");
            }
            // The first piece alone is the pair it says it is.
            let alone = [on_line(first, 40.0)];
            assert!(line_pair_cut(&alone, false).is_some(), "{first}");
        }
        // Two labels with their values on one line are no pair, nor is a second piece among
        // other lines after a first piece without a colon (a row of a table).
        let two = [on_line("Phone: 12", 40.0), on_line("Fax: 34", 200.0)];
        let row = [on_line("Parcels", 40.0), on_line("12", 200.0)];
        for lone in [true, false] {
            assert_eq!(line_pair_cut(&two, lone), None);
        }
        assert_eq!(line_pair_cut(&row, false), None);
        let bare = [
            on_line("Member Code", 40.0),
            on_line("MC 4417 2093 88", 200.0),
        ];
        assert_eq!(line_pair_cut(&bare, true), None);
        let labels = [on_line("Bill To:", 40.0), on_line("Ship To:", 200.0)];
        assert_eq!(line_pair_cut(&labels, true), None);
        // A value a size step smaller than its label is a mark beside a name,
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

    /// A row cut inside a run carries the evidence of
    /// its cut. Its label range holds the whole cut run, so read by its ranges the label
    /// `Customer reference: 20481` ends with no colon and the run after it (`north`) has no
    /// shape: the block was refused. The same ranges without the cut are still refused.
    #[test]
    fn a_row_cut_inside_a_run_is_evidenced_by_its_cut() {
        use crate::extract::pairs::blocks::KeyValue;
        let texts = [
            on_line("Order:", 40.0),
            on_line("4711", 140.0),
            on_line("Customer reference: 20481", 40.0),
            on_line("north", 190.0),
        ];
        let runs: Vec<&TextRun> = texts.iter().collect();
        let rows = |cut: Option<(usize, usize)>| {
            vec![
                KeyValue {
                    line: 0,
                    label: (0, 1),
                    value: Some((1, 2)),
                    cut: None,
                },
                KeyValue {
                    line: 1,
                    label: (2, 3),
                    value: Some((3, 4)),
                    cut,
                },
            ]
        };
        assert!(pairs_evidenced(&runs, &rows(Some((2, 20)))));
        assert!(!pairs_evidenced(&runs, &rows(None)));
        // A cut row alone is a pair with a value.
        let lone = vec![KeyValue {
            line: 1,
            label: (2, 3),
            value: None,
            cut: Some((2, 20)),
        }];
        assert!(pairs_evidenced(&runs, &lone));
    }
}
