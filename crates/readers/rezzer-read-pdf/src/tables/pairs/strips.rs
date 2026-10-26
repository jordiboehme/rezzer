// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rows of fields between drawn rules: frames of a free rule and the rule under it parted by
//! dividers, the cuts of value lines at their field edges, and the fields of each row with
//! their labels over, beside or under their values.

use std::collections::{HashMap, HashSet};

use super::*;
use crate::regions::{Seg, grids};

/// The note before the first table of fields between rules of a page.
const STRIP_FOUND_BY: &str = "Found by: label and value pairs from rows of fields \
     between drawn rules; each label is the smaller text and pairs with the value beside it or \
     under it.";

/// Lines a strip of fields between two rules may hold; a taller one is text, not fields.
const STRIP_LINES: usize = 40;

/// A divider of a strip is at least this share of the strip's height long: it parts the
/// fields over much of the row, where a drawn separator mark is a small share of it.
const DIVIDER: f32 = 0.25;

/// Steps the search for fields between rules may take on one page.
const STRIP_WORK: usize = 1_000_000;

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
/// with rules inside its frame (see [`crate::regions::inner_ruled`]; the frame of a figure is
/// none), or a frame of fields between rules with a divider (see [`frames`]). An underlined
/// title is looked for only there: on a page of prose a heading over a rule is a heading of
/// the prose.
pub(crate) fn form_drawn(rules: &[Rule], boxes: &[[f32; 4]]) -> bool {
    if !boxes.is_empty() {
        return true;
    }
    let Ok(g) = grids(rules) else {
        return false;
    };
    g.groups
        .iter()
        .any(|group| crate::regions::inner_ruled(&g, group))
        || frames(&g.hs, &taken_by_grids(g.hs.len(), &g.groups), &g.vs)
            .is_some_and(|f| f.iter().any(|f| !f.dividers.is_empty()))
}

/// Which of `n` horizontal lines the grids of `groups` take.
pub(in crate::tables) fn taken_by_grids(
    n: usize,
    groups: &[(Vec<usize>, Vec<usize>)],
) -> Vec<bool> {
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
pub(in crate::tables) fn frames(hs: &[Seg], taken: &[bool], vs: &[Seg]) -> Option<Vec<Frame>> {
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
pub(in crate::tables) fn field_cuts(flow: &Flow, frames: &[Frame]) -> Vec<(usize, Vec<usize>)> {
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
pub(in crate::tables) fn field_strips(
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
/// field's column (its label within a character of the same place), none of them of the
/// piece's kind and all of them shorter than the piece, while the piece is of the kind of the
/// value before it, the piece is that value's overflow: it goes back to its field, and the
/// next field is empty. A short piece (a letter where the column holds numbers) may be that
/// field's own odd value, so it stays where the cut put it.
fn overflow_back(flow: &Flow, strips: &mut [Strip], g: &[usize]) {
    let column = |f: &Field| f.label.first().map(|&i| &flow.runs[i]);
    // The kinds of the values of the group by the place and size of their labels.
    let mut seen: Vec<(f32, usize, Kind, usize)> = Vec::new();
    for &k in g {
        for f in &strips[k].fields {
            if let (Some(l), false) = (column(f), f.value.is_empty()) {
                let text = words(flow, &f.value);
                seen.push((l.x, k, kind_of(&text), text.chars().count()));
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
            let length = rb.text.trim().chars().count();
            let others: Vec<(Kind, usize)> = seen
                .iter()
                .filter(|s| s.1 != k && (s.0 - l.x).abs() <= CHAR * l.size.max(1.0))
                .map(|s| (s.2, s.3))
                .collect();
            if touching
                && !others.is_empty()
                && others.iter().all(|&(kind, n)| kind != piece && n < length)
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
