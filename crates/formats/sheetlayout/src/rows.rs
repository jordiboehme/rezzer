// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Step 6c of the detection order, after the header band of a table: group rows (rule R2),
//! trailing notes, label columns (rule L), the row hierarchy (rule R3) and subtotal and total
//! rows (spec 8.6). Totals come only from facts and formulas, never from words.

use std::collections::BTreeMap;

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
/// a lower level. Group rows take part with the level of their own source, so the rows under
/// a group row have it as parent, but they get no level of their own. Gives (row, level,
/// parent, words) for every row that is not a group row, or nothing when no row has a
/// parent. `None` when stopped.
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
    let mut found: Vec<(u32, Source, bool)> = Vec::new();
    for l in &lines.lines[range] {
        if tick.step() {
            return None;
        }
        let group = groups.binary_search(&l.row).is_ok();
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
        found.push((l.row, source, group));
    }
    let mut leads: Vec<usize> = found
        .iter()
        .filter_map(|(_, s, _)| match s {
            Source::Lead(n) => Some(*n),
            _ => None,
        })
        .collect();
    leads.sort_unstable();
    leads.dedup();
    let any = found.iter().any(|(_, s, _)| !matches!(s, Source::None));
    if !any {
        return Some(Vec::new());
    }
    let mut out: Vec<(u32, u8, Option<u32>, String)> = Vec::with_capacity(found.len());
    // The rows above with their levels, each lower than the one after it.
    let mut stack: Vec<(u8, u32)> = Vec::new();
    let mut parents = false;
    for (row, source, group) in found {
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
        stack.push((level, row));
        if !group {
            parents |= parent.is_some();
            out.push((row, level, parent, why));
        }
    }
    Some(if parents { out } else { Vec::new() })
}

/// A formula cell that may mark its row as a subtotal or total (spec 8.6).
struct Sum {
    row: u32,
    col: u32,
    /// SUBTOTAL or AGGREGATE: always a subtotal.
    sub: bool,
    /// Its reads cover the rows from the first data row (itself or through the totals it
    /// reads).
    first: bool,
    why: String,
}

/// Row ranges of one column a total cell reads: first row, last row.
type Spans = Vec<(u32, u32)>;

/// The total cells of one column so far, in row order.
#[derive(Default)]
struct Column {
    /// The rows of the total cells that cover from the first data row.
    first: Vec<u32>,
    /// The same-column ranges they read.
    reads: Vec<(u32, u32)>,
}

fn any_in(rows: &[u32], top: u32, bottom: u32) -> bool {
    let k = rows.partition_point(|&r| r < top);
    rows.get(k).is_some_and(|&r| r <= bottom)
}

/// The total cells of a line. `block` is the first row of the block the line closes: the
/// first data row, or the row after the last group row or total row above.
#[allow(clippy::too_many_arguments)]
fn sums_of<'c>(
    cells: impl Iterator<Item = &'c CellIn>,
    row: u32,
    above: Option<u32>,
    data: Rect,
    first_data: u32,
    block: u32,
    columns: &BTreeMap<u32, Column>,
    tick: &Tick,
) -> Option<Vec<(Sum, Spans)>> {
    let mut out = Vec::new();
    for c in cells {
        let Some(f) = &c.formula else {
            continue;
        };
        let col = c.pos.col;
        let at = Rect::cell(c.pos).a1();
        let call = |name: &str| f.functions.iter().any(|n| n.eq_ignore_ascii_case(name));
        let none = Column::default();
        let column = columns.get(&col).unwrap_or(&none);
        // Same-column ranges above the row, inside the table.
        let mut own: Vec<Rect> = Vec::new();
        let mut in_row = false;
        for r in &f.reads {
            if tick.step() {
                return None;
            }
            let r = r.normalized();
            in_row |= r.top <= row && row <= r.bottom;
            if r.left == col && r.right == col && r.top >= data.top && r.bottom < row {
                own.push(r);
            }
        }
        let sub = ["SUBTOTAL", "AGGREGATE"].into_iter().find(|n| call(n));
        if let Some(name) = sub {
            let first = own
                .iter()
                .any(|r| r.top <= first_data || any_in(&column.first, r.top, r.bottom));
            let reads = own.iter().map(|r| (r.top, r.bottom)).collect();
            out.push((
                Sum {
                    row,
                    col,
                    sub: true,
                    first,
                    why: format!("{name} formula in {at}"),
                },
                reads,
            ));
            continue;
        }
        // A SUM that reads its own row is a row total or a running balance, never a total
        // row.
        if !call("SUM") || in_row {
            continue;
        }
        // A read counts when it spans 2 rows or more or starts at or above the top of the
        // block (the cells of earlier totals lie there).
        own.retain(|r| r.height() >= 2 || r.top <= block);
        let reaches = own
            .iter()
            .map(|r| r.bottom)
            .max()
            .is_some_and(|b| above.is_some_and(|a| b >= a));
        if !reaches {
            continue;
        }
        let first = own
            .iter()
            .any(|r| r.top <= first_data || any_in(&column.first, r.top, r.bottom));
        let words: Vec<String> = own.iter().map(|r| r.a1()).collect();
        let reads = own.iter().map(|r| (r.top, r.bottom)).collect();
        out.push((
            Sum {
                row,
                col,
                sub: false,
                first,
                why: format!("SUM over {} in {at}", words.join(", ")),
            },
            reads,
        ));
    }
    Some(out)
}

/// Spec 8.6 by formulas over the data lines (not group rows, not stated totals rows):
/// SUBTOTAL and AGGREGATE give `Subtotal`. A SUM over the rows above in its own column is a
/// `Subtotal` when a later SUM or SUBTOTAL of the column reads it, else a `Total` when it
/// covers from the first data row, else a `Subtotal`. In one row `Total` wins.
fn formula_kinds(
    lines: &Lines,
    range: std::ops::Range<usize>,
    groups: &[u32],
    data: Rect,
    total_from: u32,
    tick: &Tick,
) -> Option<Vec<(u32, RowKind, String)>> {
    let first_data = lines.lines[range.clone()]
        .iter()
        .map(|l| l.row)
        .find(|r| groups.binary_search(r).is_err())
        .unwrap_or(data.top);
    let mut columns: BTreeMap<u32, Column> = BTreeMap::new();
    let mut sums: Vec<Sum> = Vec::new();
    let mut above: Option<u32> = None;
    let mut block = first_data;
    for l in &lines.lines[range] {
        if tick.step() {
            return None;
        }
        let row = l.row;
        if row >= total_from {
            break;
        }
        if groups.binary_search(&row).is_ok() {
            block = row.saturating_add(1);
            above = Some(row);
            continue;
        }
        let found = sums_of(
            lines.cells_of(l),
            row,
            above,
            data,
            first_data,
            block,
            &columns,
            tick,
        )?;
        if !found.is_empty() {
            block = row.saturating_add(1);
        }
        for (sum, reads) in found {
            let column = columns.entry(sum.col).or_default();
            if sum.first {
                column.first.push(row);
            }
            column.reads.extend(reads);
            sums.push(sum);
        }
        above = Some(row);
    }
    // A total cell is read by a later one when its row lies in a range that another total
    // cell of its column reads: such ranges end above the cell that reads them.
    let mut read: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
    for (&col, column) in columns.iter_mut() {
        let mut spans = std::mem::take(&mut column.reads);
        spans.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(spans.len());
        for (a, b) in spans {
            if tick.step() {
                return None;
            }
            match merged.last_mut() {
                Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        read.insert(col, merged);
    }
    let mut out: Vec<(u32, RowKind, String)> = Vec::new();
    for sum in sums {
        let spans = read.get(&sum.col).map_or(&[][..], |v| v.as_slice());
        let k = spans.partition_point(|s| s.1 < sum.row);
        let is_read = spans.get(k).is_some_and(|s| s.0 <= sum.row);
        let kind = if !sum.sub && !is_read && sum.first {
            RowKind::Total
        } else {
            RowKind::Subtotal
        };
        match out.last_mut() {
            Some(last) if last.0 == sum.row => {
                if last.1 == RowKind::Subtotal && kind == RowKind::Total {
                    *last = (sum.row, kind, sum.why);
                }
            }
            _ => out.push((sum.row, kind, sum.why)),
        }
    }
    Some(out)
}

/// Step 6c on a table: group rows, trailing notes, label columns, hierarchy and subtotals.
/// `pivot` keeps the pivot's own label columns. Gives false when the stop callback ends the
/// work.
///
/// A row of one label in the first column (no merge across the table) in a table with
/// values is a data row with empty values: it is a group row only when a data row follows
/// it, and never a trailing note.
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
    let single = |l: &Line| lines.single_text(l).is_some();
    let mut values = false;
    for l in &lines.lines[range.clone()] {
        if tick.step() {
            return false;
        }
        if !single(l) {
            values = true;
            break;
        }
    }
    let label_only =
        |l: &Line| values && group_words(lines, l, body) == Some("one text in the first column");
    let mut end = range.end;
    // Trailing rows of one text are notes (rule N inside a region); a stated totals row
    // stays a total, and a label alone in the first column stays a data row.
    if totals.is_none() {
        while end > range.start {
            if tick.step() {
                return false;
            }
            let l = &lines.lines[end - 1];
            if !single(l) || label_only(l) {
                break;
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

    // Group rows (R2): a run of rows of one text in the first column or one merged area
    // across the table, followed by a data row. Runs give nested groups.
    let mut out: Vec<RowNote> = Vec::new();
    let mut groups: Vec<u32> = Vec::new();
    let mut run: Vec<(u32, &'static str)> = Vec::new();
    for l in &lines.lines[range.clone()] {
        if tick.step() {
            return false;
        }
        if l.row >= total_from {
            break;
        }
        if let Some(words) = group_words(lines, l, body) {
            run.push((l.row, words));
            continue;
        }
        if !single(l) {
            for (row, words) in run.drain(..) {
                groups.push(row);
                out.push(note(row, RowKind::Group, words.to_string()));
            }
        }
        run.clear();
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

    let Some(mut kinds) = formula_kinds(lines, range.clone(), &groups, data, total_from, tick)
    else {
        return false;
    };
    if let Some((from, why)) = totals {
        for l in &lines.lines[range] {
            if l.row >= from {
                kinds.push((l.row, RowKind::Total, why.to_string()));
            }
        }
    }
    let mut at = 0usize;
    let mut more: Vec<RowNote> = Vec::new();
    for (row, kind, why) in kinds {
        if tick.step() {
            return false;
        }
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
