// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rows of fields between drawn rules: frames of a free rule and the rule under it parted by
//! dividers, the cuts of value lines at their field edges, and the fields of each row with
//! their labels over, beside or under their values.

use std::collections::{HashMap, HashSet};

use layoutregions::segment::drawn::{Grids, Line, Unsearched};

use super::*;

pub(super) use crate::found::STRIP_FOUND_BY;

/// Lines a strip of fields between two rules may hold; a taller one is text, not fields.
const STRIP_LINES: usize = 40;

/// A divider of a strip is at least this share of the strip's height long: it parts the
/// fields over much of the row, where a drawn separator mark is a small share of it.
const DIVIDER: f32 = 0.25;

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
    /// Its top rule is the lowest rule of a grid (see [`frame_tops`]).
    pub under_grid: bool,
}

/// The frames of fields between rules among the page's `rules` (see [`frames`]), under rules
/// no grid takes; none on a page without horizontal rules or with too many of them. (A frame
/// under a grid's lowest rule is a row of fields only by its text, see [`holds_values`].)
///
/// It searches the grids of `rules` on its own: the separators drawn as marks are read (see
/// `marks::cut_at_dividers`) before the page's drawn structure is found, as that structure's
/// boxes need the map of the page's text with them, and on a page turned a quarter in the
/// frame its text reads in.
pub(crate) fn field_frames(rules: &[Rule]) -> Vec<Frame> {
    let Ok(g) = crate::regions::grids(rules) else {
        return Vec::new();
    };
    frames(&g.hs, &frame_tops(&g.hs, &g.vs, &g.groups, false), &g.vs).unwrap_or_default()
}

/// Whether the page's drawing shows a form: a drawn box its text is read in (`boxes`), a grid
/// of the page's `grids` with rules inside its frame (see
/// [`layoutregions::segment::drawn::inner_ruled`]; the frame of a figure is none), or a frame
/// of fields between rules with a divider (see [`frames`]). An underlined title is looked for
/// only there: on a page of prose a heading over a rule is a heading of the prose.
pub(crate) fn form_drawn(grids: Result<&Grids, Unsearched>, boxes: &[[f32; 4]]) -> bool {
    if !boxes.is_empty() {
        return true;
    }
    let Ok(g) = grids else {
        return false;
    };
    g.groups
        .iter()
        .any(|group| layoutregions::segment::drawn::inner_ruled(g, group))
        || frames(&g.hs, &frame_tops(&g.hs, &g.vs, &g.groups, false), &g.vs)
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

/// What a horizontal line may be to a frame of fields under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::tables) enum Top {
    /// No top: a grid takes it.
    No,
    /// A rule no grid takes.
    Free,
    /// The lowest rule of a grid: a box drawn over a row of fields (a section's title cells)
    /// takes the row's top rule into its grid, and the row under it is still a row of fields
    /// when its text holds fields with a value (see [`holds_values`]).
    Grid,
}

/// What each of the horizontal lines `hs` may be to a frame of fields under it (see [`Top`]):
/// with `lowest`, the lowest line of each grid of `groups` may top one too. A grid of two
/// rules and dividers between them (`vs`), none at their ends and none running on over or
/// under them (a row of fields open at both ends), is no grid to its top rule: the frame under
/// it is a row of fields with dividers.
pub(in crate::tables) fn frame_tops(
    hs: &[Line],
    vs: &[Line],
    groups: &[(Vec<usize>, Vec<usize>)],
    lowest: bool,
) -> Vec<Top> {
    let mut tops: Vec<Top> = taken_by_grids(hs.len(), groups)
        .into_iter()
        .map(|t| if t { Top::No } else { Top::Free })
        .collect();
    for (gh, gv) in groups {
        let [a, b] = gh[..] else {
            continue;
        };
        let (x0, x1) = (hs[a].from.min(hs[b].from), hs[a].to.max(hs[b].to));
        let (y0, y1) = (hs[a].at.min(hs[b].at), hs[a].at.max(hs[b].at));
        let inner = !gv.is_empty()
            && gv.iter().all(|&k| {
                let v = &vs[k];
                v.at > x0 + SNAP && v.at < x1 - SNAP && v.from >= y0 - SNAP && v.to <= y1 + SNAP
            });
        if inner {
            let top = if hs[a].at > hs[b].at { a } else { b };
            tops[top] = Top::Free;
        }
    }
    if lowest {
        for (gh, _) in groups {
            if let Some(&low) = gh.iter().min_by(|&&a, &&b| hs[a].at.total_cmp(&hs[b].at)) {
                tops[low] = Top::Grid;
            }
        }
    }
    tops
}

/// The frames under the page's horizontal lines `hs` (sorted by height) that `tops` names
/// (see [`frame_tops`]), with the dividers among the vertical lines `vs` (sorted by place);
/// `None` past [`STRIP_WORK`] steps.
pub(in crate::tables) fn frames(hs: &[Line], tops: &[Top], vs: &[Line]) -> Option<Vec<Frame>> {
    let mut work = 0usize;
    let mut out = Vec::new();
    for (j, up) in hs.iter().enumerate() {
        if tops[j] == Top::No {
            continue;
        }
        // The nearest rule below that reaches under it, and it must cover it.
        let mut below: Option<&Line> = None;
        for s in hs[..j].iter().rev() {
            work += 1;
            crate::work::add(1);
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
            crate::work::add(1);
            if work > STRIP_WORK {
                return None;
            }
            if v.at >= x1 - SNAP {
                break;
            }
            // A column rule of a grid over the frame that runs on through its top rule (the
            // grid's lowest, see [`frame_tops`]) and ends in the frame hangs from its top; one
            // of a grid under it that runs on through its bottom rule rises from its bottom.
            let through = |at: f32| v.from < at - SNAP && v.to > at + SNAP;
            let to = if through(top) { top } else { v.to };
            let from = if through(bottom) { bottom } else { v.from };
            let reaches = (to - top).abs() <= SNAP || (from - bottom).abs() <= SNAP;
            if reaches
                && to - from >= DIVIDER * (top - bottom)
                && from >= bottom - SNAP
                && to <= top + SNAP
            {
                dividers.push(v.at);
                spans.push((from, to));
            }
        }
        out.push(Frame {
            x0,
            x1,
            bottom,
            top,
            dividers,
            spans,
            under_grid: tops[j] == Top::Grid,
        });
    }
    Some(out)
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

/// Where to cut the runs of the page's `frames` of fields (see
/// [`layoutregions::extract::pairs::strips::field_cuts`]): each run with its gaps, in the order
/// of the flow.
pub(in crate::tables) fn field_cuts(flow: &Flow, frames: &[Frame]) -> Vec<(usize, Vec<usize>)> {
    let lines = flow_lines(flow);
    let by_y = ByY::new(&flow.runs, &lines);
    let edges: Vec<FieldFrame<'_>> = frames
        .iter()
        .map(|fr| FieldFrame {
            x0: fr.x0,
            x1: fr.x1,
            dividers: &fr.dividers,
        })
        .collect();
    layoutregions::extract::pairs::strips::field_cuts(&flow.runs, &edges, &mut |f| {
        frame_runs(flow, &by_y, &frames[f], &|_| false)
    })
}

/// Whether the frame `fr` holds a row of fields with a value: its cells between dividers read
/// as fields (see [`parse`]) that are short, one of them with a value. A row of column titles
/// or a line of text under a rule holds none.
pub(in crate::tables) fn holds_values(flow: &Flow, by_y: &ByY, fr: &Frame) -> bool {
    let how = Reading {
        one_field: false,
        smaller: true,
        small: i32::MAX,
        labels: &[],
    };
    let ids = frame_runs(flow, by_y, fr, &|_| false);
    if ids.is_empty() {
        return false;
    }
    let mut xs = vec![fr.x0];
    xs.extend(&fr.dividers);
    xs.push(fr.x1);
    let mut value = false;
    for cell in cells_of(&flow.runs, &xs, &ids)
        .iter()
        .filter(|c| !c.is_empty())
    {
        let Some(parts) = parts_of(&flow.runs, cell) else {
            return false;
        };
        for part in parts {
            match parse(&flow.runs, &part, how) {
                Parsed::Fields(f) if f.iter().all(|f| short(&flow.runs, f)) => {
                    value |= f.iter().any(|f| !f.value.is_empty());
                }
                // An address is a cell of the row, no value of a field.
                Parsed::Unsure(ADDRESS | NAMED_ADDRESS) => {}
                _ => return false,
            }
        }
    }
    value
}

/// Fields between drawn rules, in the `frames` of the page. A frame's dividers part it into
/// cells; a cell whose first line is two labels or more over lines of values is parted further,
/// each value going to the label it stands under (see [`parts_of`]). When the text of each
/// part is fields (see [`parse`]; the label a size step smaller than its value) and short,
/// the strip is a row of fields. A cell a run stands across (a value line the dividers could
/// not cut at a word gap) or a value under no label of its own leaves its fields as text, and
/// a strip whose labels do not all stand over or beside their values leaves all of them as
/// text; a loss in `unsure` names them. Strips joined by their rules (see [`connected`]) make
/// one table, when it has two fields or more and one of them a value; one field alone with a
/// value is named in a loss. Text just beyond such a group, at both its ends, can show that
/// its labels stand under the rule their values stand on (see [`ends`]; only text no row of
/// fields holds counts, every group read the normal way first): its fields are then read that
/// way (see [`pair_under`]), or left as text with a loss when that is not clear (`tabled`
/// names the runs of ruled tables among them). Runs `busy` names are
/// not looked at. The runs of each table found go to `claimed` under its place in `tables`;
/// the runs of a row or a group left as text with a loss are marked in `kept`, so that no later
/// table takes them: a table of the text of fields reads as pairs too.
#[allow(clippy::too_many_arguments)]
pub(in crate::tables) fn field_strips(
    page: u32,
    flow: &Flow,
    by_y: &ByY,
    frames: &[Frame],
    hs: &[Line],
    table_rules: &[Line],
    busy: &dyn Fn(usize) -> bool,
    tabled: &dyn Fn(usize) -> bool,
    text_sizes: &[f32],
    claimed: &mut [Option<(usize, usize, usize)>],
    kept: &mut [bool],
    tables: &mut Vec<Planned>,
    unsure: &mut Vec<String>,
    doubts: &mut Doubts,
    bodies: &mut Vec<(Vec<usize>, Vec<String>)>,
    apart: &mut Vec<Vec<Vec<usize>>>,
) {
    let how = Reading {
        one_field: false,
        smaller: true,
        small: small_size(text_sizes).saturating_sub(1),
        labels: &[],
    };
    // Each loss also gives its reason to the runs it keeps as text (see `Found::doubts`).
    let say = |unsure: &mut Vec<String>,
               doubts: &mut Doubts,
               names: &[String],
               why: &'static str,
               runs: &[usize]| {
        doubts.push((runs.iter().map(|&i| flow.ids[i]).collect(), why));
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
        if ids.is_empty() || lines_of(&flow.runs, &ids).len() > STRIP_LINES {
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
        let cells = cells_of(&flow.runs, &edges, &ids);
        // Column titles over rows of items are a table's body, left to the table pass; its
        // runs and the names of its titles go to `bodies`, so that a line the table pass
        // leaves out is said.
        if table_body(&flow.runs, &cells) {
            let names = cells
                .iter()
                .flat_map(|c| first_labels(&flow.runs, c))
                .collect();
            bodies.push((ids, names));
            continue;
        }
        let StripRead {
            fields,
            left_out,
            fits,
            unread,
            field_why,
            doubt,
            styled_heads,
        } = strip_cells(&flow.runs, &edges, &cells, how);
        // A cell left out as an address under a name is no row of a table or of totals,
        // though its lines may line up with the lines beside it.
        let address = left_out.iter().any(|l| l.0 == NAMED_ADDRESS);
        // Rows of a table between rules (totals under line items, a larger total last) are no
        // row of fields, whatever pairs their sizes make: their text is free for the table pass
        // as it stands.
        if !address && table_rows(&flow.runs, &ids) {
            continue;
        }
        // Text that is not fields makes the strip no row of fields; labels and values in the
        // wrong places beside fields read right make it one that is not clear, and that is
        // said. A row whose every field is not clear for a reason of fields is said too,
        // without names: which of its text is a label is what is not clear. Text in three
        // styles alone is no sign of a row of fields (a caption, prose and a note between
        // rules), so it is not said.
        if !fits {
            if doubt {
                doubts.push((ids.iter().map(|&i| flow.ids[i]).collect(), LINE_BY_LINE));
                unsure.push(format!(
                    "two lines of text in columns between two rules on page {page} are read \
                     line by line: each line may be a row, or each column a cell of its own \
                     whose lines go together"
                ));
            }
            continue;
        }
        if unread {
            if !fields.is_empty() {
                let names: Vec<String> = cells
                    .iter()
                    .flat_map(|c| label_names(&flow.runs, c))
                    .collect();
                say(unsure, doubts, &names, NOT_FIELDS, &ids);
            } else if let Some(why) = field_why {
                say(unsure, doubts, &styled_heads, why, &ids);
            } else {
                continue;
            }
            for &i in &ids {
                kept[i] = true;
            }
            apart.extend(cell_order(
                flow,
                by_y,
                (fr.top, fr.bottom, fr.x0, fr.x1),
                &fr.dividers,
                busy,
            ));
            continue;
        }
        // Every cell left out: no row of fields is read, and it is read cell by cell, but for
        // totals between rules (rows of a label and an amount), whose text is free for the
        // table pass as it stands: they are no row of fields, and no loss names them as one
        // (nor the note under them).
        let to_tables = fields.is_empty() && !address && totals_rows(&flow.runs, &ids);
        if !to_tables {
            for (why, names) in &left_out {
                say(unsure, doubts, names, why, &ids);
            }
        }
        if fields.is_empty() {
            if !left_out.is_empty() && !to_tables {
                apart.extend(cell_order(
                    flow,
                    by_y,
                    (fr.top, fr.bottom, fr.x0, fr.x1),
                    &fr.dividers,
                    busy,
                ));
            }
            continue;
        }
        strips.push(Strip {
            top: fr.top,
            bottom: fr.bottom,
            x0: fr.x0,
            x1: fr.x1,
            dividers: fr.dividers.clone(),
            fields,
            rows: two_lines(&flow.runs, &ids),
            left_out: !left_out.is_empty(),
        });
    }
    rows_in_order(&mut strips);
    let groups = connected(&strips);
    for g in &groups {
        overflow_back(&flow.runs, &mut strips, g);
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
    // The strip that holds each run of a field.
    let holder: HashMap<usize, usize> = strips
        .iter()
        .enumerate()
        .flat_map(|(k, s)| {
            s.fields
                .iter()
                .flat_map(|f| f.label.iter().chain(&f.value))
                .map(move |&i| (i, k))
        })
        .collect();
    // Blocks of a group side by side on the same rules (no rule of one reaches the other)
    // may set their labels in different ways: when the text beyond them reads one block with
    // its labels under its values and another with its labels over them, each block is a group
    // of its own, so the labels of one block never pair by the way of the other.
    let groups: Vec<Vec<usize>> = groups
        .into_iter()
        .flat_map(|g| {
            let blocks = side_blocks(&strips, &g);
            if blocks.len() < 2 {
                return vec![g];
            }
            let free = |i: usize| !busy(i) && !held.contains(&i);
            let ways: Vec<Ends> = blocks
                .iter()
                .map(|b| {
                    ends(
                        flow,
                        by_y,
                        &free,
                        &free,
                        None,
                        &|_| false,
                        frames,
                        &strips,
                        b,
                    )
                })
                .collect();
            let under = ways.iter().any(|w| matches!(w, Ends::Under(..)));
            let over = ways.iter().any(|w| matches!(w, Ends::Over));
            // A block whose rules end beyond the rules of the others (rows of its own past their
            // first or last rule) and that stands on no rule inside another block is a grid of
            // its own beside them: its ends are its own, so when they read its labels over its
            // values while another block is not clear, the two are read apart. A cell beside
            // the first row of a grid (its bottom rule a rule inside the grid) is a part of it.
            let span = |b: &[usize]| {
                b.iter().fold((f32::MIN, f32::MAX), |(t, l), &k| {
                    (t.max(strips[k].top), l.min(strips[k].bottom))
                })
            };
            let spans: Vec<(f32, f32)> = blocks.iter().map(|b| span(b)).collect();
            let inside = |y: f32, o: (f32, f32)| y < o.0 - SNAP && y > o.1 + SNAP;
            let own = |k: usize| {
                let (t, l) = spans[k];
                let others = || spans.iter().enumerate().filter(move |&(o, _)| o != k);
                others().all(|(_, &o)| !inside(t, o) && !inside(l, o))
                    && others().any(|(_, &o)| t > o.0 + SNAP || l < o.1 - SNAP)
            };
            let apart = (0..blocks.len()).any(|k| matches!(ways[k], Ends::Over) && own(k))
                && ways.iter().any(|w| !matches!(w, Ends::Over));
            if (under && over) || apart {
                blocks
            } else {
                vec![g]
            }
        })
        .collect();
    // Runs beyond a group that a group read with its labels under their values took.
    let mut outside: HashSet<usize> = HashSet::new();
    // The strips of the groups that are read (they hold a value): each says of its own fields,
    // in a pair or in a loss.
    let read: HashSet<usize> = groups
        .iter()
        .filter(|g| {
            g.iter()
                .any(|&k| strips[k].fields.iter().any(|f| !f.value.is_empty()))
        })
        .flatten()
        .copied()
        .collect();
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
        // A group with a strip of another shape takes any evidence as a loss. When every label
        // with a value stands beside it (a block of labels and values) no reading puts them
        // under the rule their values stand on, so the runs of ruled tables are no evidence for
        // it (a group that mixes such a block with labels over values has that reading):
        // the row of a section over such a block is that table's text. Labels over a value of
        // two lines may stand under the rule of the values over them: for them, as for every
        // other group, the runs of ruled tables count. Held and free runs always count.
        let shaped = g.iter().all(|&k| strips[k].rows.is_some());
        let fields = || g.iter().flat_map(|&k| &strips[k].fields);
        let beside = fields().any(|f| beside_value(&flow.runs, f))
            && fields().all(|f| f.value.is_empty() || beside_value(&flow.runs, f));
        let with_held = |i: usize| {
            !own.contains(&i) && !outside.contains(&i) && (shaped || !beside || !tabled(i))
        };
        // A run joined to the group by its rules: one of a strip of another group over or
        // under a strip of the group, across a stretch both share, its rule within one row
        // pitch of the group's (the two rules of a double rule); or one of a ruled table over
        // the group's top rule or under its bottom rule whose grid has that rule as its own
        // (`table_rules`).
        let (top, bottom) = g.iter().fold((f32::MIN, f32::MAX), |(t, b), &k| {
            (t.max(strips[k].top), b.min(strips[k].bottom))
        });
        let at_rule = |y: f32, r: &Run| {
            table_rules
                .iter()
                .any(|s| (s.at - y).abs() <= SNAP && s.from < r.x + r.w && r.x < s.to)
        };
        let in_table = |i: usize| {
            let r = &flow.runs[i];
            tabled(i) && ((r.y > top && at_rule(top, r)) || (r.y < bottom && at_rule(bottom, r)))
        };
        let neighbour = |i: usize| {
            in_table(i)
                || holder.get(&i).is_some_and(|&k| {
                    let t = &strips[k];
                    !g.contains(&k)
                        && g.iter().any(|&o| {
                            let s = &strips[o];
                            let pitch = s.top - s.bottom;
                            s.x0 < t.x1
                                && t.x0 < s.x1
                                && ((t.bottom - s.top).abs() <= pitch
                                    || (s.bottom - t.top).abs() <= pitch)
                        })
                })
        };
        // One field alone between its rules reads like a caption over a line: it is not
        // paired, and that is said, with the labels under its rule that may be of its value.
        // Beside a cell of its row left out, it is a field of the row only when a colon ends
        // its label: a style is no evidence (it may come from a pair that size alone decided).
        let row = g.len() == 1
            && strips[g[0]].left_out
            && list
                .iter()
                .all(|f| words(&flow.runs, &f.label).ends_with(':'));
        if list.len() < 2 && !row {
            let mut names: Vec<String> = list.iter().map(|f| words(&flow.runs, &f.label)).collect();
            if let Ends::Under(_, below) | Ends::Unclear(below) = ends(
                flow,
                by_y,
                &with_held,
                &free,
                Some(hs),
                &neighbour,
                frames,
                &strips,
                &g,
            ) {
                for line in lines_of(&flow.runs, &below) {
                    for l in labels_in_line(&flow.runs, &line) {
                        let name = words(&flow.runs, &l);
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
            }
            say(unsure, doubts, &names, LONE, &runs);
            for &i in &runs {
                kept[i] = true;
            }
            for &k in &g {
                read_cells(flow, by_y, &strips[k], busy, apart);
            }
            continue;
        }
        let without = ends(
            flow,
            by_y,
            &free,
            &free,
            None,
            &|_| false,
            frames,
            &strips,
            &g,
        );
        let with = ends(
            flow,
            by_y,
            &with_held,
            &free,
            Some(hs),
            &neighbour,
            frames,
            &strips,
            &g,
        );
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
        let stack = matches!(found, Ends::Under(..));
        let list: Vec<&Field> = match found {
            Ends::Over => list,
            Ends::Under(above, below) => {
                match pair_under(&flow.runs, &strips, &g, &above, &below) {
                    Some(f) if f.len() >= 2 && f.iter().all(|f| short(&flow.runs, f)) => {
                        under = f;
                        outside.extend(above.iter().chain(&below));
                        under.iter().collect()
                    }
                    _ => {
                        let names: Vec<String> =
                            list.iter().map(|f| words(&flow.runs, &f.label)).collect();
                        let kept_runs: Vec<usize> = runs.iter().chain(&below).copied().collect();
                        say(unsure, doubts, &names, UNDER_UNCLEAR, &kept_runs);
                        for &i in runs.iter().chain(&below) {
                            kept[i] = true;
                        }
                        read_group_cells(flow, by_y, &strips, &g, busy, apart);
                        continue;
                    }
                }
            }
            Ends::Unclear(below) => {
                let mut names: Vec<String> =
                    list.iter().map(|f| words(&flow.runs, &f.label)).collect();
                // A label another group that is read holds is named by that group: a field is
                // named in one loss only.
                let theirs = |i: &usize| {
                    holder
                        .get(i)
                        .is_some_and(|k| !g.contains(k) && read.contains(k))
                };
                let named: Vec<usize> = below.iter().copied().filter(|i| !theirs(i)).collect();
                for line in lines_of(&flow.runs, &named) {
                    for l in labels_in_line(&flow.runs, &line) {
                        let name = words(&flow.runs, &l);
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
                let kept_runs: Vec<usize> = runs
                    .iter()
                    .chain(below.iter().filter(|i| !held.contains(i)))
                    .copied()
                    .collect();
                say(unsure, doubts, &names, UNDER_UNCLEAR, &kept_runs);
                for &i in runs
                    .iter()
                    .chain(below.iter().filter(|i| !held.contains(i)))
                {
                    kept[i] = true;
                }
                read_group_cells(flow, by_y, &strips, &g, busy, apart);
                continue;
            }
        };
        // A cell left out of the pairs reads as a whole, its lines together.
        for &k in g.iter().filter(|&&k| strips[k].left_out) {
            read_cells(flow, by_y, &strips[k], busy, apart);
        }
        let top = strips[g[0]].top;
        // A group read with its labels under their values is a stack: a pre-pass of its own
        // (see `layout::today`).
        let by = if stack {
            Detector::StripStack
        } else {
            Detector::Strips
        };
        let Some(mut p) = pairs_table(flow, &list, top, (STRIP_FOUND_BY, by)) else {
            continue;
        };
        p.drawn = union(g.iter().map(|&k| {
            let s = &strips[k];
            [s.x0, s.bottom, s.x1, s.top]
        }));
        claim(claimed, tables.len(), &list);
        tables.push(p);
    }
}

/// The cells of a row of fields between the rules `top` and `bottom` from `x0` to `x1`, with
/// their runs (none that `busy` names, none in another direction) in the order to read them:
/// cell by cell, left to right, each cell's lines top down. Cells are parted by the `dividers`
/// that no run stands across and, inside them, by gutters: white space of a cell gap (see
/// [`layoutregions::extract::table::CELL_GAP`]) or more that no run of the cell crosses on any
/// of its lines (a line that runs on into the next cell joins the two), and values beside
/// their labels are a field per line
/// (see [`beside_labels`]). One cell is read whole too: its lines together, before the text
/// beside it on them. None when there is no run. Every row of fields that is refused, or that
/// has a cell left out of its pairs, is read this way (see [`read_cells`]).
fn cell_order(
    flow: &Flow,
    by_y: &ByY,
    (top, bottom, x0, x1): (f32, f32, f32, f32),
    dividers: &[f32],
    busy: &dyn Fn(usize) -> bool,
) -> Option<Vec<Vec<usize>>> {
    let ids: Vec<usize> = by_y
        .runs_between(bottom, top)
        .into_iter()
        .filter(|&i| {
            let r = &flow.runs[i];
            let (cx, _) = middle(r);
            !busy(i) && !r.off_axis && cx > x0 && cx < x1
        })
        .collect();
    // A divider a run stands across parts no cells: the run is one value over both.
    let mut edges = vec![x0];
    edges.extend(dividers.iter().copied().filter(|&e| {
        !ids.iter()
            .any(|&i| inked(&flow.runs[i]) && across(&flow.runs[i], e))
    }));
    edges.push(x1);
    let mut columns: Vec<Vec<usize>> = Vec::new();
    for cell in cells_of(&flow.runs, &edges, &ids) {
        columns.extend(beside_labels(&flow.runs, gutter_columns(&flow.runs, &cell)));
    }
    if columns.is_empty() {
        return None;
    }
    // Each cell's lines top down, the runs of a line in the order of the flow (their spaces
    // between words go with them).
    Some(
        columns
            .iter()
            .map(|c| {
                lines_of(&flow.runs, c)
                    .into_iter()
                    .flat_map(|mut l| {
                        l.sort_unstable();
                        l
                    })
                    .collect()
            })
            .collect(),
    )
}

/// Reads the strips `g` of a group whose labels may stand under their values cell by cell,
/// each strip between its own rules, and so the band of one strip's height over its top rule
/// (the first values, when the labels stand under them) and under its bottom rule (the last
/// labels), each with its cells by gutters, each line of a cell a block of its own, and on
/// a line each stretch of runs of one style (see [`by_style`]).
fn read_group_cells(
    flow: &Flow,
    by_y: &ByY,
    strips: &[Strip],
    g: &[usize],
    busy: &dyn Fn(usize) -> bool,
    apart: &mut Vec<Vec<Vec<usize>>>,
) {
    // Each line of a cell is a block of its own: a label under the rule and the value of the
    // next row under it stand in one column, and which goes with which is what is not clear.
    let by_line = |cells: Vec<Vec<usize>>| -> Vec<Vec<usize>> {
        cells
            .into_iter()
            .flat_map(|c| {
                lines_of(&flow.runs, &c).into_iter().flat_map(|mut l| {
                    l.sort_unstable();
                    by_style(&flow.runs, l)
                })
            })
            .collect()
    };
    for &k in g {
        let s = &strips[k];
        apart.extend(
            cell_order(flow, by_y, (s.top, s.bottom, s.x0, s.x1), &s.dividers, busy).map(by_line),
        );
    }
    let (Some(&first), Some(&last)) = (g.first(), g.last()) else {
        return;
    };
    let (top, bottom) = g.iter().fold((f32::MIN, f32::MAX), |(t, b), &k| {
        (t.max(strips[k].top), b.min(strips[k].bottom))
    });
    let (x0, x1) = g.iter().fold((f32::MAX, f32::MIN), |(a, b), &k| {
        (a.min(strips[k].x0), b.max(strips[k].x1))
    });
    let over = strips[first].top - strips[first].bottom;
    let under = strips[last].top - strips[last].bottom;
    for band in [(top + over, top, x0, x1), (bottom, bottom - under, x0, x1)] {
        apart.extend(cell_order(flow, by_y, band, &[], busy).map(by_line));
    }
}

/// Reads the strip `s` of fields cell by cell (see [`cell_order`]): its cells go to `apart`.
fn read_cells(
    flow: &Flow,
    by_y: &ByY,
    s: &Strip,
    busy: &dyn Fn(usize) -> bool,
    apart: &mut Vec<Vec<Vec<usize>>>,
) {
    apart.extend(cell_order(
        flow,
        by_y,
        (s.top, s.bottom, s.x0, s.x1),
        &s.dividers,
        busy,
    ));
}

/// Sorts `strips` top first and each row left to right: strips whose tops lie within
/// [`SNAP`] of the row's first one stand in one row, as [`connected`] joins them. A rule drawn
/// in dashes lies at the mean of its dashes, so two parts of one row differ in their tops by
/// float noise, which must not decide their order.
fn rows_in_order(strips: &mut [Strip]) {
    strips.sort_by(|a, b| b.top.total_cmp(&a.top).then(a.x0.total_cmp(&b.x0)));
    let mut i = 0;
    while i < strips.len() {
        let top = strips[i].top;
        let n = strips[i..]
            .iter()
            .take_while(|s| s.top >= top - SNAP)
            .count();
        strips[i..i + n].sort_by(|a, b| a.x0.total_cmp(&b.x0));
        i += n;
    }
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

/// The blocks of the group `g` of `strips` side by side: strips whose stretches overlap or
/// touch (within [`SNAP`]) are one block, each in the order of `g`.
fn side_blocks(strips: &[Strip], g: &[usize]) -> Vec<Vec<usize>> {
    let mut blocks: Vec<(f32, f32, Vec<usize>)> = Vec::new();
    for &k in g {
        let s = &strips[k];
        let (mut x0, mut x1, mut members) = (s.x0, s.x1, vec![k]);
        // Every block this strip meets joins it.
        let mut i = 0;
        while i < blocks.len() {
            let b = &blocks[i];
            if b.0 <= x1 + SNAP && x0 <= b.1 + SNAP {
                let (bx0, bx1, bm) = blocks.swap_remove(i);
                x0 = x0.min(bx0);
                x1 = x1.max(bx1);
                members.extend(bm);
                i = 0;
            } else {
                i += 1;
            }
        }
        blocks.push((x0, x1, members));
    }
    let mut out: Vec<Vec<usize>> = blocks
        .into_iter()
        .map(|(_, _, mut m)| {
            m.sort_by_key(|k| g.iter().position(|x| x == k));
            m
        })
        .collect();
    out.sort_by_key(|b| b.first().and_then(|k| g.iter().position(|x| x == k)));
    out
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
    open: Option<&[Line]>,
    neighbour: &dyn Fn(usize) -> bool,
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
        &flow.runs,
        &rows.iter().map(|(_, r)| r.0.clone()).collect::<Vec<_>>(),
    );
    let role = style_of;
    let mut ls: Vec<Style> = Vec::new();
    let mut vs: Vec<Style> = Vec::new();
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
        let over = |h: &&Line| h.from < r.x + r.w && r.x < h.to;
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
        let fields = labels_in_line(&flow.runs, &by_x(values.clone()));
        let mut headed = vec![false; fields.len()];
        for label in labels_in_line(&flow.runs, &by_x(found.clone())) {
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
        let heads = labels_in_line(&flow.runs, labels);
        let mut taken = vec![false; heads.len()];
        for value in labels_in_line(&flow.runs, &by_x(found.clone())) {
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
        labels_in_line(&flow.runs, labels).len() == 1
            && !rows.iter().any(|(t, _)| {
                !std::ptr::eq(*s, *t) && near(t.top, s.top) && near(t.bottom, s.bottom)
            })
    });
    let way = match way_of(single, top, bottom) {
        // A strip of another shape: labels over values only without evidence at either end.
        Way::Under if !shaped => Way::Unclear,
        Way::Over if !shaped && (top != TopEnd::None || bottom != BottomEnd::None) => Way::Unclear,
        // With the runs other groups hold (`open`), evidence in part at both ends that runs
        // joined to the group by its rules (`neighbour`) make at each end is not clear: a grid
        // parted by double rules with fields left empty at its inner ends, or a grid whose
        // ends lie in the cells of ruled tables that share its top and bottom rule (rulings
        // P6-Z3, P6-Z4). Evidence in part from other text (a loose line, column titles under
        // a rule of their own) keeps labels over values.
        Way::Over
            if open.is_some()
                && top == TopEnd::Partial
                && bottom == BottomEnd::Partial
                && above.iter().any(|&i| neighbour(i))
                && below.iter().any(|&i| neighbour(i)) =>
        {
            Way::Unclear
        }
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
                BottomEnd::Free | BottomEnd::Partial => below,
                BottomEnd::Own => further,
                BottomEnd::None => Vec::new(),
            };
            named.extend(over_labels);
            Ends::Unclear(named)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two parts of one row of fields whose top rules, each the mean of its dashes, differ
    /// by float noise read left to right, as the row under them; a row a rule pitch lower
    /// comes after.
    #[test]
    fn strips_of_one_row_read_left_to_right() {
        let strip = |top: f32, x0: f32| Strip {
            top,
            bottom: top - 22.8,
            x0,
            x1: x0 + 90.0,
            dividers: Vec::new(),
            fields: Vec::new(),
            rows: None,
            left_out: false,
        };
        let mut strips = vec![
            strip(737.08, 42.4),
            strip(759.879_76, 42.4),
            strip(759.880_25, 293.2),
        ];
        rows_in_order(&mut strips);
        let got: Vec<(f32, f32)> = strips.iter().map(|s| (s.top, s.x0)).collect();
        assert_eq!(
            got,
            [(759.879_76, 42.4), (759.880_25, 293.2), (737.08, 42.4)]
        );
    }
}
