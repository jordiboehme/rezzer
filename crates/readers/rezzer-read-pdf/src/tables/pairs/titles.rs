// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Titles over a drawn bar: a line of bold text alone over a horizontal rule no grid takes,
//! with the lines under the rule as its text.

use super::*;
use crate::regions::{Seg, grids};
use crate::tables::LABEL_STEP;

/// Lines under an underlined title follow each other at most this many line heights
/// apart; past it the title's text ends.
const TITLE_TEXT_PITCH: f32 = 1.6;

/// Lines of text one underlined title may have.
const TITLE_TEXT_LINES: usize = 20;

/// Steps the search for underlined titles may take on one page (runs looked at near a bar,
/// bars looked at for a rule between lines); past them it stops and says so.
const TITLE_WORK: usize = 4_000_000;

/// Titles over a drawn bar: a line of bold text alone over a horizontal rule that no grid
/// takes, the rule under its baseline by at most the title's size, longer than the title and
/// starting within two of its sizes of the title's start; and under the rule, within its
/// stretch, the lines of one style larger than the title (by half a point, any weight) that
/// follow each other, the first within two line heights of the rule and no rule between
/// them. Each title is a heading where it stands and its lines its text; their runs leave
/// `flow`. `rules` are the page's drawn rules. The search stops at `stop` or past
/// [`TITLE_WORK`] steps, with the titles found before it and why it stopped.
pub(crate) fn underlined_titles(
    flow: &mut Flow,
    rules: &[Rule],
    stop: &dyn Fn() -> bool,
) -> (Vec<BoxTitle>, Option<&'static str>) {
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
        return (Vec::new(), None);
    }
    let hs = crate::regions::merged(segs);
    // The bars by their height (a bar at no height is at none), to find those near a height
    // by a search.
    let mut by_at: Vec<usize> = (0..hs.len()).filter(|&k| !hs[k].at.is_nan()).collect();
    by_at.sort_by(|&a, &b| hs[a].at.total_cmp(&hs[b].at));
    let ats: Vec<f32> = by_at.iter().map(|&k| hs[k].at).collect();
    // The bars whose height lies within `lo..hi` at most (a superset: the caller tests each).
    let near = |lo: f32, hi: f32| {
        let a = ats.partition_point(|v| *v < lo);
        let z = ats.partition_point(|v| *v <= hi);
        &by_at[a..z.max(a)]
    };
    let mut work = 0usize;
    let too_much = "the page has too many bars and too much text over them";
    // The rules of grids are a table's or a section's, not a title's.
    let taken: Vec<bool> = match grids(rules) {
        Ok(g) if !g.hs.is_empty() => {
            let mut t = vec![false; hs.len()];
            for (gh, _) in &g.groups {
                for &j in gh {
                    let s = g.hs[j];
                    let close = near(s.at - 2.0 * SNAP, s.at + 2.0 * SNAP);
                    work += close.len() + 1;
                    if work > TITLE_WORK {
                        return (Vec::new(), Some(too_much));
                    }
                    for &k in close {
                        let h = hs[k];
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
    let mut halted: Option<&'static str> = None;
    // Runs between two heights, charged to the work before they are copied.
    let between = |lo: f32, hi: f32, work: &mut usize| -> Option<Vec<usize>> {
        *work += by_y.count_between(lo, hi) + 1;
        (*work <= TITLE_WORK).then(|| by_y.runs_between(lo, hi))
    };
    for (k, bar) in hs.iter().enumerate() {
        if taken[k] {
            continue;
        }
        if stop() {
            halted = Some("the time limit was reached");
            break;
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
        let Some(close) = between(bar.at, bar.at + 3.0 * MAX_CAPTION_SIZE, &mut work) else {
            halted = Some(too_much);
            break;
        };
        let over: Vec<usize> = close
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
        let Some(beside) = between(line_y - size, line_y + size, &mut work) else {
            halted = Some(too_much);
            break;
        };
        let alone = beside
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
        let Some(under) = between(
            bar.at - 2.0 * TITLE_TEXT_LINES as f32 * size * TITLE_TEXT_PITCH,
            bar.at,
            &mut work,
        ) else {
            halted = Some(too_much);
            break;
        };
        let below: Vec<usize> = under
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
            let a = ats.partition_point(|v| *v <= y);
            let z = ats.partition_point(|v| *v < last - SNAP);
            let lying = &by_at[a..z.max(a)];
            work += lying.len();
            let ruled = lying.iter().any(|&j| {
                let s = hs[j];
                s.at < last - SNAP && s.at > y && s.from < bar.to && s.to > bar.from
            });
            let styles: Vec<Style> = l.iter().map(|&i| style_of(&flow.runs[i])).collect();
            let one = styles.iter().all(|s| *s == styles[0]);
            if gap > limit || ruled || !one || style.is_some_and(|s| s != styles[0]) {
                break;
            }
            if (styles[0].0 as f32 / 2.0) < LABEL_STEP * size {
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
        return (Vec::new(), halted);
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
    (out, halted)
}
