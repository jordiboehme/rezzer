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
