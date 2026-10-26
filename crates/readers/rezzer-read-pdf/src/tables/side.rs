// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A column of totals beside the sections of a form: a narrow column right of a grid, drawn
//! with horizontal rules only, whose rules line up with those of the sections beside it.
//! Where a strip of the column heads its amounts on the baseline of a section's column
//! titles, the strip is one more column of that section's table; where it holds a total
//! label alone on its line over an amount, the two are a pair.

use super::*;
use crate::regions::Seg;

/// A column stands beside a grid when it starts at most this many times the size of its text
/// right of the grid's right edge (and more than [`SNAP`], or the two would be one rule): a
/// gutter of about a character parts a column of totals from its section; it is also at least
/// this wide. Relative to the text, so a form printed larger or smaller reads the same.
pub(super) const SIDE_GAP: f32 = 1.0;

/// The note before the first table on a page the column of totals adds to.
pub(super) const SIDE_FOUND_BY: &str = "Found by: a column of amounts beside a table, drawn with \
     rules that line up with the table's rules; its amounts are read as one more column of the \
     table, and each total label in it pairs with the amount under it.";

/// Rules one page may look at for columns of totals; past them none is looked for.
pub(super) const SIDE_WORK: usize = 1_000_000;

/// A column of totals: its left and right ends, the widest gap it may stand from a section
/// beside it (see [`SIDE_GAP`]), and its strips.
#[derive(Debug)]
pub(super) struct Side {
    pub x0: f32,
    pub x1: f32,
    pub gap: f32,
    pub strips: Vec<Strip>,
}

/// The stretch of a column between two of its rules: its top, and whether it is a column of
/// the table beside it or holds totals.
#[derive(Debug)]
pub(super) struct Strip {
    pub top: f32,
    pub kind: StripKind,
}

#[derive(Debug)]
pub(super) enum StripKind {
    /// Its first line stands on the baseline of the column titles beside it: it is one more
    /// column of that table.
    Joins,
    /// Total labels each alone on its line, each over its amount: the runs of each label and
    /// of its amount.
    Totals(Vec<(Vec<usize>, Vec<usize>)>),
    /// Neither: its text is left where it is.
    Other,
}

/// A grid section beside which a column may stand: its left and right edges and its row
/// edges from the top.
pub(super) struct Beside<'a> {
    pub left: f32,
    pub right: f32,
    pub ys: &'a [f32],
}

/// The columns of totals of a page: runs of horizontal rules (`hs`, the page's joined
/// segments, sorted by height) that no grid takes (`taken`), that no vertical rule of `vs`
/// touches, of one width and start, starting just right of a section of `parts` and with two
/// of their rules at that section's row edges. `work` counts the rules looked at.
pub(super) fn side_columns(
    flow: &Flow,
    by_y: &ByY,
    hs: &[Seg],
    vs: &[Seg],
    taken: &[bool],
    parts: &[Beside<'_>],
    work: &mut usize,
) -> Vec<Side> {
    let mut free: Vec<Seg> = hs
        .iter()
        .zip(taken)
        .filter(|(_, t)| !**t)
        .map(|(s, _)| *s)
        .collect();
    if free.len() < 2 || parts.is_empty() {
        return Vec::new();
    }
    free.sort_by(|a, b| a.from.total_cmp(&b.from).then(a.to.total_cmp(&b.to)));
    // Rules of one start and end, top first.
    let mut columns: Vec<(f32, f32, Vec<f32>)> = Vec::new();
    for s in free {
        *work += 1;
        if *work > SIDE_WORK {
            return Vec::new();
        }
        match columns
            .iter_mut()
            .rev()
            .take_while(|c| s.from - c.0 <= SNAP)
            .find(|c| (s.to - c.1).abs() <= SNAP)
        {
            Some(c) => c.2.push(s.at),
            None => columns.push((s.from, s.to, vec![s.at])),
        }
    }
    let mut out = Vec::new();
    for (x0, x1, mut ys) in columns {
        if ys.len() < 2 {
            continue;
        }
        ys.sort_by(|a, b| b.total_cmp(a));
        ys.dedup_by(|b, a| *a - *b <= SNAP);
        // The size of the column's text sets how far it may stand from a section, and how
        // narrow it may be.
        let size = by_y
            .runs_between(ys[ys.len() - 1], ys[0])
            .into_iter()
            .filter(|&i| {
                let r = &flow.runs[i];
                let (cx, _) = middle(r);
                !r.off_axis && cx > x0 && cx < x1 && !r.text.trim().is_empty()
            })
            .map(|i| flow.runs[i].size)
            .fold(0.0f32, f32::max);
        let gap_most = SIDE_GAP * size;
        if x1 - x0 <= gap_most {
            continue;
        }
        let on = |edges: &[f32], y: f32| edges.iter().any(|e| (e - y).abs() <= SNAP);
        let beside: Vec<&Beside<'_>> = parts
            .iter()
            .filter(|p| {
                let gap = x0 - p.right;
                gap > SNAP && gap <= gap_most && ys.iter().filter(|y| on(p.ys, **y)).count() >= 2
            })
            .collect();
        if beside.is_empty() {
            continue;
        }
        // Horizontal rules only: a vertical rule that touches the column's rules makes it a
        // box of its own.
        let (top, bottom) = (ys[0], ys[ys.len() - 1]);
        let lo = vs.partition_point(|v| v.at < x0 - SNAP);
        let mut boxed = false;
        for v in &vs[lo..] {
            *work += 1;
            if v.at > x1 + SNAP || *work > SIDE_WORK {
                break;
            }
            boxed |= v.from <= top + SNAP && v.to >= bottom - SNAP;
        }
        if boxed || *work > SIDE_WORK {
            continue;
        }
        let left = beside.iter().map(|p| p.left).fold(f32::MAX, f32::min);
        let strips = ys
            .windows(2)
            .map(|w| Strip {
                top: w[0],
                kind: strip_kind(flow, by_y, (x0, x1), (w[1], w[0]), left),
            })
            .collect();
        out.push(Side {
            x0,
            x1,
            gap: gap_most,
            strips,
        });
    }
    out
}

/// What the strip of the column `x0..x1` between the heights `lo..hi` holds. `left` is the
/// left edge of the sections beside it: text between it and the column on a line's baseline
/// is that line's row.
fn strip_kind(
    flow: &Flow,
    by_y: &ByY,
    (x0, x1): (f32, f32),
    (lo, hi): (f32, f32),
    left: f32,
) -> StripKind {
    let runs: Vec<usize> = by_y
        .runs_between(lo, hi)
        .into_iter()
        .filter(|&i| {
            let r = &flow.runs[i];
            let (cx, _) = middle(r);
            !r.off_axis && cx > x0 && cx < x1 && !r.text.trim().is_empty()
        })
        .collect();
    let mut lines = super::ruled::lines_up(flow, &runs);
    lines.reverse();
    let Some(first) = lines.first() else {
        return StripKind::Other;
    };
    let text = |l: &super::ruled::TextLine| -> String {
        let mut runs = l.runs.clone();
        runs.sort_by(|&a, &b| flow.runs[a].x.total_cmp(&flow.runs[b].x));
        runs.iter().map(|&i| flow.runs[i].text.as_str()).collect()
    };
    // Text beside the column on the baseline of `l`, within the sections beside it.
    let beside = |l: &super::ruled::TextLine| {
        let tol = 0.3 * l.size.max(1.0);
        by_y.runs_between(l.y - l.size, l.y + 2.0 * l.size)
            .into_iter()
            .any(|i| {
                let r = &flow.runs[i];
                let (cx, _) = middle(r);
                !r.off_axis
                    && (r.y - l.y).abs() <= tol
                    && cx > left
                    && cx < x0
                    && !r.text.trim().is_empty()
            })
    };
    if !numeric(&text(first)) && beside(first) {
        return StripKind::Joins;
    }
    if !lines.len().is_multiple_of(2) {
        return StripKind::Other;
    }
    let mut totals = Vec::new();
    for pair in lines.chunks(2) {
        let (label, amount) = (&pair[0], &pair[1]);
        // The amount's line may share its baseline with a title beside the column.
        if numeric(&text(label)) || !numeric(&text(amount)) || beside(label) {
            return StripKind::Other;
        }
        totals.push((label.runs.clone(), amount.runs.clone()));
    }
    StripKind::Totals(totals)
}

/// The planned table of the totals of a strip: one row of a label and its amount each, in
/// the order of the page.
pub(super) fn totals_table(flow: &Flow, totals: &[(Vec<usize>, Vec<usize>)]) -> Option<Planned> {
    let first = totals.iter().flat_map(|(l, a)| l.iter().chain(a)).min()?;
    let top = totals
        .iter()
        .flat_map(|(l, _)| l)
        .map(|&i| flow.runs[i].y + flow.runs[i].size)
        .fold(f32::MIN, f32::max);
    Some(Planned {
        at: flow.at[*first],
        top,
        rows: totals
            .iter()
            .map(|_| {
                (0..2)
                    .map(|_| PlannedCell {
                        runs: Vec::new(),
                        row_span: 1,
                        col_span: 1,
                        status: None,
                    })
                    .collect()
            })
            .collect(),
        trs: Vec::new(),
        additions: Vec::new(),
        header_rows: 0,
        tagged: None,
        continues: false,
        caption: Vec::new(),
        found_by: Some(SIDE_FOUND_BY),
    })
}
