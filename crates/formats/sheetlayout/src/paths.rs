// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Header paths (spec 6.6): per column, the band cells above it, top to bottom.

use crate::detect::{Lines, Tick};
use crate::input::{Pos, Rect};

/// Why no paths were built.
pub(crate) enum PathStop {
    /// The paths would pass the path budget.
    Over,
    Stopped,
}

/// The bytes a path costs: its `String` and its text.
const STRING: u64 = std::mem::size_of::<String>() as u64;

fn charge(budget: &mut usize, bytes: u64) -> Result<(), PathStop> {
    if bytes > *budget as u64 {
        return Err(PathStop::Over);
    }
    *budget -= bytes as usize;
    Ok(())
}

/// Paths from an Excel table's column names, when its header row is hidden. Columns without a
/// name get an empty path; names past the width are not used.
pub(crate) fn from_names(
    names: &[String],
    rect: Rect,
    budget: &mut usize,
) -> Result<Vec<String>, PathStop> {
    let width = rect.width();
    let used = &names[..names.len().min(width.min(usize::MAX as u64) as usize)];
    let text: u64 = used.iter().map(|n| n.len() as u64).sum();
    charge(budget, width.saturating_mul(STRING).saturating_add(text))?;
    let mut out = used.to_vec();
    out.resize(width as usize, String::new());
    Ok(out)
}

/// The texts that one band row gives its columns.
struct RowParts<'a> {
    /// The row's own cells with text, by column.
    own: Vec<(u32, &'a str)>,
    /// Texts spread over columns: horizontal merges anchored in this row and center across
    /// selection runs. By first column.
    spread: Vec<(u32, u32, &'a str)>,
    /// Per spread, the one with the widest right edge among it and those before it, so
    /// overlapping spreads (a damaged file) give the outer one.
    widest: Vec<(u32, &'a str)>,
}

impl<'a> RowParts<'a> {
    fn part(&self, col: u32) -> Option<&'a str> {
        let k = self.own.partition_point(|&(c, _)| c < col);
        if let Some(&(c, t)) = self.own.get(k)
            && c == col
        {
            return Some(t);
        }
        let k = self.spread.partition_point(|&(l, _, _)| l <= col);
        let &(right, t) = self.widest[..k].last()?;
        (col <= right).then_some(t)
    }
}

fn row_parts<'a>(lines: &Lines<'_, 'a>, row: u32, tick: &Tick) -> Result<RowParts<'a>, PathStop> {
    let rect = lines.rect;
    let mut own = Vec::new();
    let mut spread = Vec::new();
    // The merges that cover this row, by left column with the widest right edge so far, and
    // the columns of the merges anchored in it. A cell covered by a merge anchored elsewhere
    // holds a stale text: the merge's spread wins there.
    let hi = lines
        .merges
        .partition_point(|&m| lines.prep.merges[m as usize].top <= row);
    let mut covers: Vec<(u32, u32)> = Vec::new();
    let mut anchors: Vec<u32> = Vec::new();
    for &m in &lines.merges[..hi] {
        if tick.step() {
            return Err(PathStop::Stopped);
        }
        let r = lines.prep.merges[m as usize];
        if r.bottom >= row {
            covers.push((r.left, r.right));
            if r.top == row {
                anchors.push(r.left);
            }
        }
    }
    covers.sort_unstable();
    let mut reach = 0u32;
    for c in covers.iter_mut() {
        reach = reach.max(c.1);
        c.1 = reach;
    }
    anchors.sort_unstable();
    let stale = |col: u32| {
        let k = covers.partition_point(|&(l, _)| l <= col);
        k > 0 && covers[k - 1].1 >= col && anchors.binary_search(&col).is_err()
    };
    if let Some(line) = lines.at(row) {
        for c in lines.cells_of(line) {
            if c.text.is_empty() || stale(c.pos.col) {
                continue;
            }
            own.push((c.pos.col, c.text.as_str()));
        }
        // Center across selection: the text spreads over the empty cells to its right that
        // have the same alignment.
        for (i, c) in lines.cells_of(line).enumerate() {
            if c.text.is_empty() || c.style.center_across != Some(true) {
                continue;
            }
            let next_own = lines.cells[line.from as usize + i + 1..line.to as usize]
                .first()
                .map(|&k| lines.prep.cell(k as usize).pos.col);
            let mut end = c.pos.col;
            while end < rect.right && next_own.is_none_or(|n| end + 1 < n) {
                if tick.step() {
                    return Err(PathStop::Stopped);
                }
                let p = Pos { row, col: end + 1 };
                if lines.prep.across.binary_search(&p).is_err() {
                    break;
                }
                end += 1;
            }
            if end > c.pos.col {
                spread.push((c.pos.col + 1, end, c.text.as_str()));
            }
        }
    }
    // A merge anchored in this row spreads its text over its columns. A merge that starts
    // above this row was counted in its own first row.
    let lo = lines
        .merges
        .partition_point(|&m| lines.prep.merges[m as usize].top < row);
    for &m in &lines.merges[lo..] {
        if tick.step() {
            return Err(PathStop::Stopped);
        }
        let r = lines.prep.merges[m as usize];
        if r.top != row {
            break;
        }
        // Only an anchor cell of this region counts.
        let Some(anchor) = lines.cell_at(r.top, r.left) else {
            continue;
        };
        if anchor.text.is_empty() {
            continue;
        }
        spread.push((
            r.left.max(rect.left),
            r.right.min(rect.right),
            anchor.text.as_str(),
        ));
    }
    spread.sort_by_key(|&(l, r, _)| (l, r));
    let mut widest: Vec<(u32, &str)> = Vec::with_capacity(spread.len());
    for &(_, r, t) in &spread {
        match widest.last() {
            Some(&(w, wt)) if w >= r => widest.push((w, wt)),
            _ => widest.push((r, t)),
        }
    }
    Ok(RowParts {
        own,
        spread,
        widest,
    })
}

/// The parts of one column, top to bottom, equal neighbours once.
fn fill<'a>(rows: &[RowParts<'a>], col: u32, parts: &mut Vec<&'a str>) {
    parts.clear();
    for r in rows {
        if let Some(t) = r.part(col)
            && parts.last() != Some(&t)
        {
            parts.push(t);
        }
    }
}

/// The header paths of the columns of `lines.rect` from the band rows `first` to `end`.
/// Equal neighbours count once; parts are joined with ` / `; a column with no part has an
/// empty path.
pub(crate) fn build(
    lines: &Lines,
    first: u32,
    end: u32,
    budget: &mut usize,
    tick: &Tick,
) -> Result<Vec<String>, PathStop> {
    let rect = lines.rect;
    let width = rect.width();
    if width.saturating_mul(STRING) > *budget as u64 {
        return Err(PathStop::Over);
    }
    let mut rows = Vec::new();
    for row in first..=end {
        rows.push(row_parts(lines, row, tick)?);
    }
    // The text is counted before any path is built, so a path budget that is passed costs
    // no memory.
    let mut parts: Vec<&str> = Vec::with_capacity(rows.len());
    let mut text = 0u64;
    for col in rect.left..=rect.right {
        if tick.step() {
            return Err(PathStop::Stopped);
        }
        fill(&rows, col, &mut parts);
        text += parts.iter().map(|p| p.len() as u64).sum::<u64>()
            + 3 * (parts.len().saturating_sub(1) as u64);
    }
    // Width and text are charged together, so a build that fails charges nothing.
    charge(budget, width.saturating_mul(STRING).saturating_add(text))?;
    let mut out = Vec::with_capacity(width as usize);
    for col in rect.left..=rect.right {
        if tick.step() {
            return Err(PathStop::Stopped);
        }
        fill(&rows, col, &mut parts);
        out.push(parts.join(" / "));
    }
    Ok(out)
}
