// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Step 6c of the detection order, after the header band of a table: group rows (rule R2),
//! trailing notes, label columns (rule L), the row hierarchy (rule R3) and subtotal and total
//! rows (spec 8.6). Totals come only from facts and formulas, never from words.

use crate::detect::{Line, Lines, Tick};
use crate::header;
use crate::input::{CellIn, FactKind, Input, Rect};
use crate::{Region, RowKind, RowNote};

/// Row outline levels as disjoint runs sorted by first row: first row, last row, level. A row
/// in several groups takes the deepest level. Built by a sweep over the groups' ends, never
/// per row.
#[derive(Default)]
pub(crate) struct Outline {
    runs: Vec<(u32, u32, u8)>,
}

impl Outline {
    /// Gives `None` when the stop callback ends the work.
    pub fn new(input: &Input, tick: &Tick) -> Option<Self> {
        // (row, start, level): at one row an end (false) comes before a start.
        let mut ends: Vec<(u64, bool, u8)> = Vec::with_capacity(input.outline.len() * 2);
        for &(a, b, level) in &input.outline {
            if level == 0 {
                continue;
            }
            let (a, b) = (a.min(b), a.max(b));
            ends.push((u64::from(a), true, level));
            ends.push((u64::from(b) + 1, false, level));
        }
        ends.sort_unstable();
        let mut counts = [0u32; 256];
        let mut set = [0u64; 4];
        let mut runs = Vec::new();
        let mut i = 0usize;
        while i < ends.len() {
            if tick.step() {
                return None;
            }
            let at = ends[i].0;
            while i < ends.len() && ends[i].0 == at {
                let (_, start, level) = ends[i];
                let l = usize::from(level);
                if start {
                    counts[l] += 1;
                } else {
                    counts[l] = counts[l].saturating_sub(1);
                }
                if counts[l] > 0 {
                    set[l / 64] |= 1 << (l % 64);
                } else {
                    set[l / 64] &= !(1 << (l % 64));
                }
                i += 1;
            }
            let deepest = (0..4)
                .rev()
                .find(|&k| set[k] != 0)
                .map(|k| (k * 64 + 63 - set[k].leading_zeros() as usize) as u8);
            if let (Some(level), Some(next)) = (deepest, ends.get(i).map(|e| e.0)) {
                // `at` and `next - 1` are rows of the sheet, so they fit in u32.
                runs.push((at as u32, (next - 1) as u32, level));
            }
        }
        Some(Self { runs })
    }

    /// The outline level of a row, when a group holds it.
    pub fn level(&self, row: u32) -> Option<u8> {
        let k = self.runs.partition_point(|r| r.0 <= row);
        (k > 0 && self.runs[k - 1].1 >= row).then(|| self.runs[k - 1].2)
    }

    /// Whether a group holds a row from `top` to `bottom`.
    pub fn any_in(&self, top: u32, bottom: u32) -> bool {
        let k = self.runs.partition_point(|r| r.1 < top);
        self.runs.get(k).is_some_and(|r| r.0 <= bottom)
    }
}

/// What a fact states about a table's rows.
pub(crate) struct FactRows<'f> {
    pub kind: &'f FactKind,
    pub rect: Rect,
}

/// The stated totals rows of a fact: the first such row and the words.
fn stated_totals(fact: Option<&FactRows>, data: Rect) -> Option<(u32, &'static str)> {
    let f = fact?;
    let bottom = f.rect.bottom;
    let (n, why) = match f.kind {
        FactKind::ListObject { totals_rows, .. } if *totals_rows > 0 => {
            (*totals_rows, "Excel table totals row")
        }
        FactKind::DatabaseRange { footer: true, .. } => (1, "database range footer"),
        FactKind::Pivot {
            total_row: true, ..
        } => (1, "pivot total row"),
        _ => return None,
    };
    let first = bottom.saturating_sub(n - 1).max(data.top);
    (first <= data.bottom).then_some((first, why))
}

/// The index range of the lines inside the rows `top` to `bottom`.
fn line_range(lines: &Lines, top: u32, bottom: u32) -> std::ops::Range<usize> {
    let lo = lines.first_at(top);
    let hi = lines.lines.partition_point(|l| l.row <= bottom);
    lo..hi.max(lo)
}

/// The group row words of a single-text line, when it is a group row: one text in the first
/// column, or one merged area across the table (at least 80 % of the width).
fn group_words(lines: &Lines, line: &Line, body: Rect) -> Option<&'static str> {
    let cell = lines.single_text(line)?;
    let wide = lines.merge_of(line).is_some_and(|m| {
        let left = m.left.max(body.left);
        let right = m.right.min(body.right);
        let width = u64::from(right) - u64::from(left) + 1;
        width * 5 >= body.width() * 4
    });
    if wide {
        Some("one merged area across the table")
    } else if cell.pos.col == body.left {
        Some("one text in the first column")
    } else {
        None
    }
}

/// The level source of a label cell before ranking.
enum Source {
    Outline(u8),
    Indent(u8),
    /// The length of a leading run of spaces or dots.
    Lead(usize),
    None,
}

fn lead_run(text: &str) -> usize {
    text.chars().take_while(|&c| c == ' ' || c == '.').count()
}

/// Rule R3 on the data lines: the level of each row from the outline, the indent, then the
/// ranked length of a leading run of spaces or dots; the parent is the nearest row above with
/// a lower level. Gives (row, level, parent, words) for every row, or nothing when no row has
/// a parent. `None` when stopped.
#[allow(clippy::type_complexity)]
fn hierarchy(
    lines: &Lines,
    range: std::ops::Range<usize>,
    groups: &[u32],
    data: Rect,
    outline: &Outline,
    tick: &Tick,
) -> Option<Vec<(u32, u8, Option<u32>, String)>> {
    let body = lines.rect;
    let by_outline = outline.any_in(data.top, data.bottom);
    let mut found: Vec<(u32, Source)> = Vec::new();
    for l in &lines.lines[range] {
        if tick.step() {
            return None;
        }
        if groups.binary_search(&l.row).is_ok() {
            continue;
        }
        let label = lines.cell_at(l.row, body.left);
        let source = if by_outline {
            Source::Outline(outline.level(l.row).unwrap_or(0))
        } else {
            match label {
                Some(c) if c.style.indent.is_some_and(|n| n > 0) => {
                    Source::Indent(c.style.indent.unwrap_or(0))
                }
                Some(c) if lead_run(&c.text) > 0 => Source::Lead(lead_run(&c.text)),
                _ => Source::None,
            }
        };
        found.push((l.row, source));
    }
    let mut leads: Vec<usize> = found
        .iter()
        .filter_map(|(_, s)| match s {
            Source::Lead(n) => Some(*n),
            _ => None,
        })
        .collect();
    leads.sort_unstable();
    leads.dedup();
    let any = found.iter().any(|(_, s)| !matches!(s, Source::None));
    if !any {
        return Some(Vec::new());
    }
    let mut out: Vec<(u32, u8, Option<u32>, String)> = Vec::with_capacity(found.len());
    // The rows above with their levels, each lower than the one after it.
    let mut stack: Vec<(u8, u32)> = Vec::new();
    let mut parents = false;
    for (row, source) in found {
        if tick.step() {
            return None;
        }
        let (level, why) = match source {
            Source::Outline(n) => (n, format!("outline level {n}")),
            Source::Indent(n) => (n, format!("indent {n}")),
            Source::Lead(n) => {
                let rank = leads.partition_point(|&x| x < n) + 1;
                let rank = rank.min(usize::from(u8::MAX)) as u8;
                (rank, format!("leading dots or spaces, rank {rank}"))
            }
            Source::None => (0, "base level".to_string()),
        };
        while stack.last().is_some_and(|&(l, _)| l >= level) {
            stack.pop();
        }
        let parent = stack.last().map(|&(_, r)| r);
        parents |= parent.is_some();
        stack.push((level, row));
        out.push((row, level, parent, why));
    }
    Some(if parents { out } else { Vec::new() })
}

/// Rule 8.6 on one row: a formula that calls SUBTOTAL or AGGREGATE gives `Subtotal`; a SUM
/// over the rows above in its own column, inside the table, gives `Subtotal`, or `Total` when
/// it starts at the first data row. `Total` wins over `Subtotal`.
fn formula_kind<'c>(
    cells: impl Iterator<Item = &'c CellIn>,
    row: u32,
    above: Option<u32>,
    data: Rect,
    first_data: u32,
    tick: &Tick,
) -> Option<(RowKind, String)> {
    let mut best: Option<(RowKind, String)> = None;
    for c in cells {
        let Some(f) = &c.formula else {
            continue;
        };
        let at = Rect::cell(c.pos).a1();
        let call = |name: &str| f.functions.iter().any(|n| n.eq_ignore_ascii_case(name));
        let mut kind = None;
        for name in ["SUBTOTAL", "AGGREGATE"] {
            if call(name) {
                kind = Some((RowKind::Subtotal, format!("{name} formula in {at}")));
                break;
            }
        }
        if kind.is_none() && call("SUM") {
            for r in &f.reads {
                if tick.step() {
                    return None;
                }
                let r = r.normalized();
                let column = r.left == c.pos.col && r.right == c.pos.col;
                let reaches = r.bottom < row && above.is_some_and(|a| r.bottom >= a);
                if column && reaches && r.top >= data.top {
                    let k = if r.top <= first_data {
                        RowKind::Total
                    } else {
                        RowKind::Subtotal
                    };
                    let words = format!("SUM over {} in {at}", r.a1());
                    if kind.as_ref().is_none_or(|(old, _)| *old != RowKind::Total) {
                        kind = Some((k, words));
                    }
                }
            }
        }
        if let Some(k) = kind {
            let better = match &best {
                None => true,
                Some((old, _)) => *old == RowKind::Subtotal && k.0 == RowKind::Total,
            };
            if better {
                best = Some(k);
            }
        }
    }
    best
}

/// Step 6c on a table whose header band is set: group rows, trailing notes, label columns,
/// hierarchy and subtotals. `pivot` keeps the pivot's own label columns. Gives false when the
/// stop callback ends the work.
pub(crate) fn table_rows(
    lines: &Lines,
    region: &mut Region,
    fact: Option<FactRows>,
    pivot: bool,
    outline: &Outline,
    tick: &Tick,
) -> bool {
    region.rows.clear();
    region.notes.clear();
    let Some(mut data) = region.data else {
        return true;
    };
    let body = lines.rect;
    let totals = stated_totals(fact.as_ref(), data);
    let range = line_range(lines, data.top, data.bottom);
    let mut end = range.end;
    // Trailing rows of one text are notes (rule N inside a region); a stated totals row
    // stays a total.
    if totals.is_none() {
        while end > range.start && lines.single_text(&lines.lines[end - 1]).is_some() {
            if tick.step() {
                return false;
            }
            end -= 1;
        }
    }
    let notes: Vec<u32> = lines.lines[end..range.end].iter().map(|l| l.row).collect();
    if !notes.is_empty() {
        if end == range.start {
            region.data = None;
            region.notes = notes;
            return true;
        }
        data.bottom = lines.lines[end - 1].row;
        region.data = Some(data);
        region.notes = notes;
    }
    let range = range.start..end;
    let total_from = totals.map_or(u32::MAX, |t| t.0);

    // Row notes are built in row order: sorted vectors, merged, never a map per row.
    let mut out: Vec<RowNote> = Vec::new();
    let mut groups: Vec<u32> = Vec::new();
    for l in &lines.lines[range.clone()] {
        if tick.step() {
            return false;
        }
        if l.row >= total_from {
            break;
        }
        if let Some(words) = group_words(lines, l, body) {
            groups.push(l.row);
            out.push(note(l.row, RowKind::Group, words.to_string()));
        }
    }

    if !pivot {
        let (n, words) = header::label_cols(lines, &region.header_rows, data, &groups, tick);
        if tick.hit() {
            return false;
        }
        region.label_cols = n;
        region
            .evidence
            .retain(|e| !matches!(e, crate::Evidence::Rule { id: "L", .. }));
        if let Some(words) = words {
            region.evidence.push(crate::Evidence::Rule {
                id: "L",
                detail: words,
            });
        }
    }

    if region.label_cols > 0 {
        let Some(levels) = hierarchy(lines, range.clone(), &groups, data, outline, tick) else {
            return false;
        };
        let levels = levels.into_iter().map(|(row, level, parent, why)| RowNote {
            row,
            kind: RowKind::Data,
            level: Some(level),
            parent,
            why,
        });
        out = merge(out, levels.collect());
    }

    let first_data = lines.lines[range.clone()]
        .iter()
        .map(|l| l.row)
        .find(|r| groups.binary_search(r).is_err())
        .unwrap_or(data.top);
    let mut above: Option<u32> = None;
    let mut at = 0usize;
    let mut more: Vec<RowNote> = Vec::new();
    for l in &lines.lines[range] {
        if tick.step() {
            return false;
        }
        let row = l.row;
        let kind = if row >= total_from {
            totals.map(|t| (RowKind::Total, t.1.to_string()))
        } else if groups.binary_search(&row).is_ok() {
            None
        } else {
            match formula_kind(lines.cells_of(l), row, above, data, first_data, tick) {
                Some(k) => Some(k),
                None if tick.hit() => return false,
                None => None,
            }
        };
        if let Some((kind, why)) = kind {
            while out.get(at).is_some_and(|n| n.row < row) {
                at += 1;
            }
            match out.get_mut(at).filter(|n| n.row == row) {
                Some(n) => {
                    n.kind = kind;
                    n.why = format!("{why}; {}", n.why);
                }
                None => more.push(note(row, kind, why)),
            }
        }
        above = Some(row);
    }
    region.rows = merge(out, more);
    true
}

/// Two lists of row notes sorted by row, with no row in both, as one sorted list.
fn merge(a: Vec<RowNote>, b: Vec<RowNote>) -> Vec<RowNote> {
    if b.is_empty() {
        return a;
    }
    if a.is_empty() {
        return b;
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    let mut b = b.into_iter().peekable();
    for n in a {
        while let Some(m) = b.next_if(|m| m.row < n.row) {
            out.push(m);
        }
        out.push(n);
    }
    out.extend(b);
    out
}

fn note(row: u32, kind: RowKind, why: String) -> RowNote {
    RowNote {
        row,
        kind,
        level: None,
        parent: None,
        why,
    }
}
