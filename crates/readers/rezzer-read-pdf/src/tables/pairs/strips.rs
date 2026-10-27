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
    frames(&g.hs, &frame_tops(&g.hs, &g.groups), &g.vs).unwrap_or_default()
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
        || frames(&g.hs, &frame_tops(&g.hs, &g.groups), &g.vs)
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

/// Which of the horizontal lines `hs` may be the top rule of a frame of fields: those no grid
/// of `groups` takes, and the lowest line of each grid. A box drawn over a row of fields (a
/// section's title cells) takes the row's top rule into its grid; the row under it is still a
/// row of fields.
pub(in crate::tables) fn frame_tops(hs: &[Seg], groups: &[(Vec<usize>, Vec<usize>)]) -> Vec<bool> {
    let mut tops: Vec<bool> = taken_by_grids(hs.len(), groups)
        .into_iter()
        .map(|t| !t)
        .collect();
    for (gh, _) in groups {
        if let Some(&low) = gh.iter().min_by(|&&a, &&b| hs[a].at.total_cmp(&hs[b].at)) {
            tops[low] = true;
        }
    }
    tops
}

/// The frames under the page's horizontal lines `hs` (sorted by height) that `tops` names
/// (see [`frame_tops`]), with the dividers among the vertical lines `vs` (sorted by place);
/// `None` past [`STRIP_WORK`] steps.
pub(in crate::tables) fn frames(hs: &[Seg], tops: &[bool], vs: &[Seg]) -> Option<Vec<Frame>> {
    let mut work = 0usize;
    let mut out = Vec::new();
    for (j, up) in hs.iter().enumerate() {
        if !tops[j] {
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
            // A column rule of a grid over the frame that runs on through its top rule (the
            // grid's lowest, see [`frame_tops`]) and ends in the frame hangs from its top.
            let to = if v.from < top - SNAP && v.to > top + SNAP {
                top
            } else {
                v.to
            };
            let reaches = (to - top).abs() <= SNAP || (v.from - bottom).abs() <= SNAP;
            if reaches
                && to - v.from >= DIVIDER * (top - bottom)
                && v.from >= bottom - SNAP
                && to <= top + SNAP
            {
                dividers.push(v.at);
                spans.push((v.from, to));
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
/// their values, labels under the rule their values stand on whose values are not all clear,
/// and one field alone.
const ACROSS: &str = "a value stands across two fields";

const UNDER_NONE: &str = "a value stands under none of the labels of its field or under two";

const NOT_FIELDS: &str = "its labels do not all stand over or beside their values";

const UNDER_UNCLEAR: &str = "its labels stand under the rule their values stand on, and not \
     every value's label is clear";

const LONE: &str = "it is one field alone between its rules, which a caption over a line of \
     text between rules looks like";

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
/// text; a loss in `unsure` names them. Strips joined by their rules (see [`connected`]) make
/// one table, when it has two fields or more and one of them a value; one field alone with a
/// value is named in a loss. Text just beyond such a group, at both its ends, can show that
/// its labels stand under the rule their values stand on (see [`ends`]; only text no row of
/// fields holds counts, every group read the normal way first): its fields are then read that way (see
/// [`pair_under`]), or left as text with a loss when that is not clear (`tabled` names the
/// runs of ruled tables among them). Runs `busy` names are
/// not looked at. The runs of each table found go to `claimed` under its place in `tables`;
/// the runs of a row or a group left as text with a loss are marked in `kept`, so that no later
/// table takes them: a table of the text of fields reads as pairs too.
#[allow(clippy::too_many_arguments)]
pub(in crate::tables) fn field_strips(
    page: u32,
    flow: &Flow,
    by_y: &ByY,
    frames: &[Frame],
    hs: &[Seg],
    busy: &dyn Fn(usize) -> bool,
    tabled: &dyn Fn(usize) -> bool,
    text_sizes: &[f32],
    claimed: &mut [Option<(usize, usize, usize)>],
    kept: &mut [bool],
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
        // A cell not clear for a reason of fields (a label under its value, a weight only) whose
        // text is as short as a field's (see [`field_sized`]), not for its many styles alone.
        let (mut fits, mut unread, mut field_why) = (true, false, None);
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
                                Parsed::Unsure(why) => {
                                    unread = true;
                                    if why != THREE_STYLES && field_sized(flow, &part) {
                                        field_why = field_why.or(Some(why));
                                    }
                                }
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
        // said. A row whose every field is not clear for a reason of fields is said too,
        // without names: which of its text is a label is what is not clear. Text in three
        // styles alone is no sign of a row of fields (a caption, prose and a note between
        // rules), so it is not said.
        if !fits {
            continue;
        }
        if unread {
            if !fields.is_empty() {
                let names: Vec<String> = cells.iter().flat_map(|c| label_names(flow, c)).collect();
                say(unsure, &names, NOT_FIELDS);
            } else if let Some(why) = field_why {
                say(unsure, &[], why);
            } else {
                continue;
            }
            for &i in &ids {
                kept[i] = true;
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
    strips.sort_by(|a, b| b.top.total_cmp(&a.top).then(a.x0.total_cmp(&b.x0)));
    let groups = connected(&strips);
    for g in &groups {
        overflow_back(flow, &mut strips, g);
    }
    // Text beyond a group is only what no row of fields holds: every group is read the
    // normal way first. Text between two rules that hold no fields still counts: the values of
    // a first row under a section's rule, or the last labels over a rule drawn under them.
    let held: HashSet<usize> = strips
        .iter()
        .flat_map(|s| &s.fields)
        .flat_map(|f| f.label.iter().chain(&f.value))
        .copied()
        .collect();
    // Runs beyond a group that a group read with its labels under their values took.
    let mut outside: HashSet<usize> = HashSet::new();
    for g in groups {
        let list: Vec<&Field> = g.iter().flat_map(|&k| &strips[k].fields).collect();
        if list.iter().all(|f| f.value.is_empty()) {
            continue;
        }
        // The runs of the group, kept as text when it is not paired.
        let runs: Vec<usize> = list
            .iter()
            .flat_map(|f| f.label.iter().chain(&f.value))
            .copied()
            .collect();
        // Only runs a group holds are left out; the runs of rows refused with a loss count.
        // Under the table of ends (see [`way_of`]) more text beyond a group can only make an
        // end complete: a loss, or labels under values with both ends complete. Text left out
        // can drop a complete end to none and read the group labels over values shifted.
        let free = |i: usize| !busy(i) && !held.contains(&i) && !outside.contains(&i);
        // The runs other groups or tables hold count toward a loss only: a double rule parts
        // one grid into two groups whose ends lie in each other, and a boxed frame over a grid
        // is a table that holds the first row's values. The ends are found without them and
        // with them; when the two ways differ the group is not clear, and when they agree it
        // is read the way found without them, so a run is never taken twice. The second way
        // leaves out only the group's own runs and the runs a group read labels under values
        // took, and it rules runs out (a label over a value) only by the runs the first way
        // counts and not by a row between two drawn rules: every other run it counts can only
        // add evidence, which under the table of ends gives a loss or the reading both ways
        // agree on.
        let own: HashSet<usize> = runs.iter().copied().collect();
        // A group with a strip of another shape (a block of labels beside their values) takes
        // any evidence as a loss, so the runs of ruled tables are no evidence for it: the row
        // of a section over such a block is that table's text. Held and free runs still count.
        let shaped = g.iter().all(|&k| strips[k].rows.is_some());
        let with_held =
            |i: usize| !own.contains(&i) && !outside.contains(&i) && (shaped || !tabled(i));
        // One field alone between its rules reads like a caption over a line: it is not
        // paired, and that is said, with the labels under its rule that may be of its value.
        if list.len() < 2 {
            let mut names: Vec<String> = list.iter().map(|f| words(flow, &f.label)).collect();
            if let Ends::Under(_, below) | Ends::Unclear(below) =
                ends(flow, by_y, &with_held, &free, Some(hs), frames, &strips, &g)
            {
                for line in lines_of(flow, &below) {
                    for l in labels_in_line(flow, &line) {
                        let name = words(flow, &l);
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
            }
            say(unsure, &names, LONE);
            for &i in &runs {
                kept[i] = true;
            }
            continue;
        }
        let without = ends(flow, by_y, &free, &free, None, frames, &strips, &g);
        let with = ends(flow, by_y, &with_held, &free, Some(hs), frames, &strips, &g);
        let found = match (without, with) {
            (Ends::Over, Ends::Over) => Ends::Over,
            (Ends::Under(above, below), Ends::Under(..)) => Ends::Under(above, below),
            // Not clear either way, or the two ways differ: the labels under the bottom rule
            // either way are named.
            (a, b) => {
                let below = |e: Ends| match e {
                    Ends::Under(_, b) | Ends::Unclear(b) => b,
                    Ends::Over => Vec::new(),
                };
                let mut named = below(a);
                for i in below(b) {
                    if !named.contains(&i) {
                        named.push(i);
                    }
                }
                Ends::Unclear(named)
            }
        };
        let under: Vec<Field>;
        let list: Vec<&Field> = match found {
            Ends::Over => list,
            Ends::Under(above, below) => match pair_under(flow, &strips, &g, &above, &below) {
                Some(f) if f.len() >= 2 && f.iter().all(|f| short(flow, f)) => {
                    under = f;
                    outside.extend(above.iter().chain(&below));
                    under.iter().collect()
                }
                _ => {
                    let names: Vec<String> = list.iter().map(|f| words(flow, &f.label)).collect();
                    say(unsure, &names, UNDER_UNCLEAR);
                    for &i in runs.iter().chain(&below) {
                        kept[i] = true;
                    }
                    continue;
                }
            },
            Ends::Unclear(below) => {
                let mut names: Vec<String> = list.iter().map(|f| words(flow, &f.label)).collect();
                for line in lines_of(flow, &below) {
                    for l in labels_in_line(flow, &line) {
                        let name = words(flow, &l);
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
                say(unsure, &names, UNDER_UNCLEAR);
                for &i in runs
                    .iter()
                    .chain(below.iter().filter(|i| !held.contains(i)))
                {
                    kept[i] = true;
                }
                continue;
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

/// Whether the text of a part of a cell is as short as a field's: by style, the text of some
/// style as short as a label, and the text of every style as short as a value. A heading over
/// lines of prose between rules is no field whose label stands under its value.
fn field_sized(flow: &Flow, part: &[usize]) -> bool {
    let mut by_style: Vec<(Style, Vec<usize>)> = Vec::new();
    for &i in part.iter().filter(|&&i| inked(&flow.runs[i])) {
        let s = style_of(&flow.runs[i]);
        match by_style.iter_mut().find(|b| b.0 == s) {
            Some(b) => b.1.push(i),
            None => by_style.push((s, vec![i])),
        }
    }
    let text: Vec<String> = by_style.iter().map(|b| words(flow, &b.1)).collect();
    text.iter().all(|t| t.chars().count() <= VALUE_CHARS)
        && text.iter().any(|t| {
            t.chars().count() <= LABEL_CHARS && t.split_whitespace().count() <= LABEL_WORDS
        })
}

/// The groups of `strips` (sorted top first, then left to right) that make one table each, in
/// that order: strips at the same heights are one row, and a strip whose top rule is the
/// bottom rule of another over a stretch both share goes on under it. Strips are joined by
/// their rules, not by their order on the page, so a grid beside another one with rows of
/// another pitch, whose strips interleave with its own, splits neither.
fn connected(strips: &[Strip]) -> Vec<Vec<usize>> {
    let mut root: Vec<usize> = (0..strips.len()).collect();
    fn find(root: &mut [usize], mut k: usize) -> usize {
        while root[k] != k {
            root[k] = root[root[k]];
            k = root[k];
        }
        k
    }
    for (a, s) in strips.iter().enumerate() {
        // The strips after it in the order start no higher; past its bottom rule none meets it.
        for (b, t) in strips.iter().enumerate().skip(a + 1) {
            if t.top < s.bottom - SNAP {
                break;
            }
            let same_row = (s.top - t.top).abs() <= SNAP && (s.bottom - t.bottom).abs() <= SNAP;
            let under = (s.bottom - t.top).abs() <= SNAP && s.x0 < t.x1 && t.x0 < s.x1;
            if same_row || under {
                let (ra, rb) = (find(&mut root, a), find(&mut root, b));
                root[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut at: HashMap<usize, usize> = HashMap::new();
    for k in 0..strips.len() {
        let r = find(&mut root, k);
        match at.get(&r) {
            Some(&g) => groups[g].push(k),
            None => {
                at.insert(r, groups.len());
                groups.push(vec![k]);
            }
        }
    }
    groups
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

/// What the text just beyond a group of strips shows of where its labels stand.
#[derive(Debug)]
enum Ends {
    /// Nothing that says the labels stand anywhere but over or beside their values.
    Over,
    /// The labels stand under the rule their values stand on: the values over the top rule
    /// and the labels under the bottom rule, field for field.
    Under(Vec<usize>, Vec<usize>),
    /// Neither reading is clear (see [`way_of`]). The labels under the bottom rule beyond the
    /// group, to name them.
    Unclear(Vec<usize>),
}

/// Whether the run `r` is in one of the roles `set` (a style and a font family): of its style,
/// and of its family where both families are known.
fn in_role(set: &[(Style, u32)], r: &Run) -> bool {
    let s = style_of(r);
    set.iter()
        .any(|&(t, f)| t == s && (f == 0 || r.family == 0 || f == r.family))
}

/// The text just beyond the group `g` of `strips` that shows its labels stand under the rule
/// their values stand on (a signature line). Only a group whose every strip is one line of
/// labels over one line of values can show it, among the runs `free` allows (the runs of no
/// row of fields). Each end is complete when, field for field, every field of its strips has
/// one run:
/// - under the bottom rule of every strip with values and no values under it, runs in the
///   role (style and font family) of the group's labels, as far under the rule as its labels
///   stand under theirs (half a size of play): the labels of a strip of labels only under it
///   (a rule drawn under the last labels), or else free runs with no run in the values' role
///   under them within one row pitch or up to the next rule of a frame; each label heads a
///   field of that strip that holds one of its values, at the offset from label to value the
///   group shows inside its strips (a character of play), crosses no divider, and each value
///   has one;
/// - over the top rule of every strip with no strip over it, runs in the values' role as far
///   over the rule as its values stand over theirs, with no label over them within the same
///   reach; each value starts within a character of the start of a field of that strip (where
///   one of its labels starts, or the divider or left end before it), crosses no divider, and
///   every field has one.
///
/// The ends give the way the group reads by the table of [`way_of`].
#[allow(clippy::too_many_arguments)]
fn ends(
    flow: &Flow,
    by_y: &ByY,
    free: &dyn Fn(usize) -> bool,
    veto: &dyn Fn(usize) -> bool,
    open: Option<&[Seg]>,
    frames: &[Frame],
    strips: &[Strip],
    g: &[usize],
) -> Ends {
    // Each strip's line of labels and line of values; for a strip of another shape (a value of
    // two lines) the labels and values of its fields. Such a strip cannot be read with its
    // labels under its values: its ends can only make the group not clear.
    let shaped = g.iter().all(|&k| strips[k].rows.is_some());
    let rows: Vec<_> = g
        .iter()
        .map(|&k| {
            let s = &strips[k];
            let lines = s.rows.clone().unwrap_or_else(|| {
                let labels = s.fields.iter().flat_map(|f| f.label.iter().copied());
                let values = s.fields.iter().flat_map(|f| f.value.iter().copied());
                (labels.collect(), values.collect())
            });
            (s, lines)
        })
        .collect();
    let marker = markers(
        flow,
        &rows.iter().map(|(_, r)| r.0.clone()).collect::<Vec<_>>(),
    );
    let role = |r: &Run| (style_of(r), r.family);
    let mut ls: Vec<(Style, u32)> = Vec::new();
    let mut vs: Vec<(Style, u32)> = Vec::new();
    // How far the labels stand under the rule over them, and the values over the rule under
    // them.
    let (mut a, mut b): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
    for (s, (labels, values)) in &rows {
        for &i in labels.iter().filter(|i| !marker.contains(i)) {
            ls.push(role(&flow.runs[i]));
            a.push(s.top - flow.runs[i].y);
        }
        for &i in values {
            vs.push(role(&flow.runs[i]));
            b.push(flow.runs[i].y - s.bottom);
        }
    }
    // How far right of its label a value starts inside the strips.
    let mut offset: Vec<f32> = rows
        .iter()
        .flat_map(|(s, _)| &s.fields)
        .filter_map(|f| Some(flow.runs[*f.value.first()?].x - flow.runs[*f.label.first()?].x))
        .collect();
    // No field with a value: the caller reads no such group, and no pair comes of it.
    if vs.is_empty() || ls.is_empty() || offset.is_empty() {
        return Ends::Over;
    }
    let median = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let (a, b, offset) = (median(&mut a), median(&mut b), median(&mut offset));
    let near = |p: f32, q: f32| (p - q).abs() <= SNAP;
    let overlaps = |s: &Strip, t: &Strip| s.x0 < t.x1 && t.x0 < s.x1;
    let inside = |i: usize, s: &Strip| {
        let (cx, _) = middle(&flow.runs[i]);
        cx > s.x0 && cx < s.x1
    };
    let usable = |i: usize| {
        let r = &flow.runs[i];
        free(i) && inked(r) && !r.off_axis
    };
    // The runs that can rule a run out as evidence (a label over a value, a value under a
    // label): those `veto` allows; with `open` (the page's horizontal rules), none that stands
    // between two of them a row or two apart, as a row of fields does whose rules a grid took
    // (its reading is what is open).
    let pitch_max = rows
        .iter()
        .map(|(s, _)| s.top - s.bottom)
        .fold(0.0f32, f32::max);
    let enclosed = |r: &Run| {
        let Some(hs) = open else {
            return false;
        };
        let over = |h: &&Seg| h.from < r.x + r.w && r.x < h.to;
        let up = hs
            .iter()
            .filter(over)
            .filter(|h| h.at > r.y)
            .fold(f32::MAX, |m, h| m.min(h.at));
        let down = hs
            .iter()
            .filter(over)
            .filter(|h| h.at < r.y)
            .fold(f32::MIN, |m, h| m.max(h.at));
        up - down <= 2.0 * pitch_max
    };
    let vetoes = |i: usize| {
        let r = &flow.runs[i];
        veto(i) && inked(r) && !r.off_axis && !enclosed(r)
    };
    let side_by_side = |p: &Run, q: &Run| p.x < q.x + q.w && q.x < p.x + p.w;
    // How far beyond the rule `at` of a strip of row pitch `pitch` text over (or under) the
    // run `r` is looked at: one row pitch, up to the next rule of a frame over it. A rule
    // between the rule and the run (a double rule) leaves nothing to look at.
    let reach = |r: &Run, at: f32, pitch: f32, up: bool| {
        let rules = frames
            .iter()
            .filter(|f| f.x0 < r.x + r.w && r.x < f.x1)
            .flat_map(|f| [f.top, f.bottom]);
        if up {
            rules.filter(|&y| y > at + SNAP).fold(at + pitch, f32::min)
        } else {
            rules.filter(|&y| y < at - SNAP).fold(at - pitch, f32::max)
        }
    };
    let by_x = |mut v: Vec<usize>| {
        v.sort_by(|&p, &q| flow.runs[p].x.total_cmp(&flow.runs[q].x));
        v
    };
    // Under the bottom rule: labels under every strip with values and no values under it.
    // A strip of labels only under it (a rule drawn under the last labels) holds them.
    let (mut below, mut bottom, mut own) = (Vec::new(), true, false);
    // Labels beyond a row of labels only of the group, to name them.
    let mut further: Vec<usize> = Vec::new();
    for (s, (_, values)) in &rows {
        let under: Vec<_> = rows
            .iter()
            .filter(|(t, _)| near(t.top, s.bottom) && overlaps(s, t))
            .collect();
        if values.is_empty() || under.iter().any(|(_, r)| !r.1.is_empty()) {
            continue;
        }
        let pitch = s.top - s.bottom;
        let at_depth = |i: usize| {
            let r = &flow.runs[i];
            r.y < s.bottom
                && inside(i, s)
                && in_role(&ls, r)
                && ((s.bottom - r.y) - a).abs() <= 0.5 * r.size
        };
        let found: Vec<usize> = if under.is_empty() {
            by_y.runs_between(s.bottom - pitch, s.bottom)
                .into_iter()
                .filter(|&i| {
                    let r = &flow.runs[i];
                    usable(i)
                        && at_depth(i)
                        && !by_y
                            .runs_between(reach(r, s.bottom, pitch, false).min(r.y), r.y)
                            .into_iter()
                            .any(|j| {
                                let o = &flow.runs[j];
                                vetoes(j) && o.y < r.y && in_role(&vs, o) && side_by_side(o, r)
                            })
                })
                .collect()
        } else {
            own = true;
            for (t, _) in &under {
                let pitch = t.top - t.bottom;
                further.extend(
                    by_y.runs_between(t.bottom - pitch, t.bottom)
                        .into_iter()
                        .filter(|&i| {
                            let r = &flow.runs[i];
                            usable(i)
                                && r.y < t.bottom
                                && inside(i, t)
                                && in_role(&ls, r)
                                && ((t.bottom - r.y) - a).abs() <= 0.5 * r.size
                        }),
                );
            }
            under
                .iter()
                .flat_map(|(_, r)| r.0.iter().copied())
                .filter(|&i| at_depth(i))
                .collect()
        };
        // Each label heads a field of the strip with one of its values in it, and each value
        // of the strip has its label.
        let fields = labels_in_line(flow, &by_x(values.clone()));
        let mut headed = vec![false; fields.len()];
        for label in labels_in_line(flow, &by_x(found.clone())) {
            let l = &flow.runs[label[0]];
            let value = fields.iter().position(|f| {
                let r = &flow.runs[f[0]];
                ((r.x - l.x) - offset).abs() <= CHAR * r.size.max(1.0)
            });
            let crossed = label
                .iter()
                .any(|&i| s.dividers.iter().any(|&d| across(&flow.runs[i], d)));
            match value {
                Some(v) if !crossed && !headed[v] => headed[v] = true,
                _ => bottom = false,
            }
        }
        bottom &= headed.iter().all(|h| *h);
        below.extend(found);
    }
    bottom &= !below.is_empty();
    // Over the top rule: values over every strip with none over it, field for field.
    let mut above = Vec::new();
    let mut over_labels: Vec<usize> = Vec::new();
    let mut clear = true;
    for (s, (labels, _)) in &rows {
        if rows
            .iter()
            .any(|(t, _)| near(t.bottom, s.top) && overlaps(s, t))
        {
            continue;
        }
        let pitch = s.top - s.bottom;
        let found: Vec<usize> = by_y
            .runs_between(s.top, s.top + pitch)
            .into_iter()
            .filter(|&i| {
                let r = &flow.runs[i];
                usable(i)
                    && r.y > s.top
                    && inside(i, s)
                    && in_role(&vs, r)
                    && ((r.y - s.top) - b).abs() <= 0.5 * r.size
                    && !by_y
                        .runs_between(r.y, reach(r, s.top, pitch, true).max(r.y))
                        .into_iter()
                        .any(|j| {
                            let o = &flow.runs[j];
                            vetoes(j) && o.y > r.y && in_role(&ls, o) && side_by_side(o, r)
                        })
            })
            .collect();
        // Each label of the strip heads a field from its divider (or the strip's left end)
        // or from its own start; each value over the rule starts at one of them, and each
        // field has one value.
        let heads = labels_in_line(flow, labels);
        let mut taken = vec![false; heads.len()];
        for value in labels_in_line(flow, &by_x(found.clone())) {
            let r = &flow.runs[value[0]];
            let ch = CHAR * r.size.max(1.0);
            let field = heads
                .iter()
                .enumerate()
                .filter_map(|(k, l)| {
                    let lx = flow.runs[l[0]].x;
                    let start = std::iter::once(s.x0)
                        .chain(s.dividers.iter().copied())
                        .filter(|&d| d <= lx + ch)
                        .fold(s.x0, f32::max);
                    let off = (r.x - lx).abs().min((r.x - start).abs());
                    (off <= ch).then_some((k, off))
                })
                .min_by(|p, q| p.1.total_cmp(&q.1))
                .map(|(k, _)| k);
            let crossed = value
                .iter()
                .any(|&i| s.dividers.iter().any(|&d| across(&flow.runs[i], d)));
            match field {
                Some(k) if !crossed && !taken[k] => taken[k] = true,
                _ => clear = false,
            }
        }
        clear &= taken.iter().all(|t| *t);
        // Labels between two drawn rules over these values, which rule nothing out here: a
        // row of fields whose rules a grid took, to name in a loss.
        for &i in &found {
            let r = &flow.runs[i];
            for j in by_y.runs_between(r.y, reach(r, s.top, pitch, true).max(r.y)) {
                let o = &flow.runs[j];
                if veto(j)
                    && o.y > r.y
                    && in_role(&ls, o)
                    && side_by_side(o, r)
                    && enclosed(o)
                    && !over_labels.contains(&j)
                {
                    over_labels.push(j);
                }
            }
        }
        above.extend(found);
    }
    let top = match (above.is_empty(), clear) {
        (true, _) => TopEnd::None,
        (false, false) => TopEnd::Partial,
        (false, true) => TopEnd::Complete,
    };
    let bottom = match (bottom, own, below.is_empty()) {
        (true, false, _) => BottomEnd::Free,
        (true, true, _) => BottomEnd::Own,
        (false, _, false) => BottomEnd::Partial,
        (false, _, true) => BottomEnd::None,
    };
    let single = rows.iter().all(|(s, (labels, _))| {
        labels_in_line(flow, labels).len() == 1
            && !rows.iter().any(|(t, _)| {
                !std::ptr::eq(*s, *t) && near(t.top, s.top) && near(t.bottom, s.bottom)
            })
    });
    let way = match way_of(single, top, bottom) {
        // A strip of another shape: labels over values only without evidence at either end.
        Way::Under if !shaped => Way::Unclear,
        Way::Over if !shaped && (top != TopEnd::None || bottom != BottomEnd::None) => Way::Unclear,
        way => way,
    };
    match way {
        // The labels of a row between two rules over its top rule go with them, to be named
        // when the other way differs (`open` only; without it there are none).
        Way::Under => {
            let mut below = below;
            below.extend(over_labels);
            Ends::Under(above, below)
        }
        Way::Over => Ends::Over,
        // The labels beyond the group are named; values over it are not those of a field.
        // So are the labels of a row between two rules over its top rule.
        Way::Unclear => {
            let mut named = match bottom {
                BottomEnd::Free => below,
                BottomEnd::Own => further,
                _ => Vec::new(),
            };
            named.extend(over_labels);
            Ends::Unclear(named)
        }
    }
}

/// What stands over the top rule of a group of strips (see [`ends`]): nothing in the values'
/// role, values of some of its fields only (or runs at no field start), or a value for every
/// field of its top strips.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TopEnd {
    None,
    Partial,
    Complete,
}

/// What stands under the bottom rule of a group of strips (see [`ends`]): nothing in the
/// labels' role, labels of some of its lowest values only (or at no value), a label for every
/// lowest value beyond the group, or a row of labels only of the group itself (its empty last
/// row, or labels over a rule drawn under them).
#[derive(Debug, Clone, Copy, PartialEq)]
enum BottomEnd {
    None,
    Partial,
    Free,
    Own,
}

/// The way a group of strips reads by its ends (ruling P6-Z2).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Way {
    /// Labels over (or beside) their values.
    Over,
    /// Labels under the rule their values stand on.
    Under,
    /// Neither is clear: a loss names the fields.
    Unclear,
}

/// The way a group reads by its ends, `single` when it has one field a row:
///
/// | fields a row | top | bottom | reading |
/// | --- | --- | --- | --- |
/// | two or more | complete | free, complete | labels under values |
/// | two or more | complete | none, partial, own | not clear |
/// | two or more | none, partial | free, complete | not clear |
/// | two or more | none, partial | none, partial, own | labels over values |
/// | one | complete | any | not clear |
/// | one | any | free, complete | not clear |
/// | one | none, partial | none, partial, own | labels over values |
///
/// Labels under values need both ends complete, field for field, beyond the group. One
/// complete end is also what a grid of the other way shows (an empty first or last row, the
/// column titles of a next table, a line of data over it), so it is not clear. An own row of
/// labels is the group's own empty last row as much as labels over a rule under the last
/// labels: no end. One field a row is too little evidence for labels under values.
fn way_of(single: bool, top: TopEnd, bottom: BottomEnd) -> Way {
    match (single, top, bottom) {
        (true, TopEnd::Complete, _) | (true, _, BottomEnd::Free) => Way::Unclear,
        (true, _, _) => Way::Over,
        (false, TopEnd::Complete, BottomEnd::Free) => Way::Under,
        (false, TopEnd::Complete, _) | (false, _, BottomEnd::Free) => Way::Unclear,
        (false, _, _) => Way::Over,
    }
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
            family: 0,
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
