// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The decision procedure per region (spec 6.7 step 6): header band, header paths, the rows
//! of a table (step 6c) and the report rules when no band is found (steps 6d to 6h), then
//! the second passes of steps 7 and 8. Each region is read through its own rows: the cells it
//! holds, grouped by row, and for each row the number of its merges that reach the row.
//! Nothing is per position of the used area.

use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use crate::header::{self, Band};
use crate::input::{CellIn, FactKind, Input, Limits, Rect};
use crate::occupancy::Prep;
use crate::paths::{self, PathStop};
use crate::report;
use crate::rows::{self, FactRows, Outline};
use crate::{
    Certainty, Evidence, Issue, IssueKind, Kind, Layout, Origin, Region, Work, segment_work,
};

/// Steps of work between two questions to the stop callback.
const STOP_EVERY: u32 = 4096;

/// The stop callback, asked every few thousand steps. Once it says stop, it stays stopped.
pub(crate) struct Tick<'a> {
    stop: &'a dyn Fn() -> bool,
    steps: Cell<u32>,
    hit: Cell<bool>,
}

impl<'a> Tick<'a> {
    fn new(stop: &'a dyn Fn() -> bool) -> Self {
        Self {
            stop,
            steps: Cell::new(0),
            hit: Cell::new(false),
        }
    }

    /// One step of work. Gives true when the work must end.
    pub fn step(&self) -> bool {
        if self.hit.get() {
            return true;
        }
        let n = self.steps.get() + 1;
        if n >= STOP_EVERY {
            self.steps.set(0);
            return self.ask();
        }
        self.steps.set(n);
        false
    }

    /// Asks the callback now.
    pub fn ask(&self) -> bool {
        if !self.hit.get() && (self.stop)() {
            self.hit.set(true);
        }
        self.hit.get()
    }

    pub fn hit(&self) -> bool {
        self.hit.get()
    }
}

/// One row of a region that holds at least one occupied cell.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Line {
    pub row: u32,
    /// The row's cells: a range of `Lines::cells`.
    pub from: u32,
    pub to: u32,
    /// The merges of the region that reach this row.
    pub merges: u32,
    /// The sum of their indices, so the one merge is known when there is one.
    merge_sum: u64,
}

/// The rows of one region.
pub(crate) struct Lines<'p, 'a> {
    pub prep: &'p Prep<'a>,
    /// The table body: the rows after the captions and the table's own columns.
    pub rect: Rect,
    /// Indices into `prep.cells` of the region's cells, sorted by position.
    pub cells: Vec<u32>,
    pub lines: Vec<Line>,
    /// The region's merges that reach its rectangle, sorted by top row.
    pub merges: Vec<u32>,
}

impl<'p, 'a> Lines<'p, 'a> {
    /// Groups the cells inside `rect` by row and counts the merges per row by a sweep over
    /// their tops and bottoms. Gives `None` when the stop callback ends the work.
    pub fn new(
        prep: &'p Prep<'a>,
        rect: Rect,
        cells: Vec<u32>,
        merges: &[u32],
        tick: &Tick,
    ) -> Option<Self> {
        let mut merges: Vec<u32> = merges
            .iter()
            .copied()
            .filter(|&m| prep.merges[m as usize].intersects(&rect))
            .collect();
        merges.sort_unstable();
        merges.dedup();
        let mut cells = cells;
        cells.retain(|&k| rect.contains(prep.cell(k as usize).pos));
        let mut lines = Vec::new();
        let mut open: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        let mut next = 0usize;
        let mut count = 0u32;
        let mut sum = 0u64;
        let mut i = 0usize;
        while i < cells.len() {
            if tick.step() {
                return None;
            }
            let row = prep.cell(cells[i] as usize).pos.row;
            let from = i;
            while i < cells.len() && prep.cell(cells[i] as usize).pos.row == row {
                i += 1;
            }
            while next < merges.len() && prep.merges[merges[next] as usize].top <= row {
                let m = merges[next];
                open.push(Reverse((prep.merges[m as usize].bottom, m)));
                count += 1;
                sum += u64::from(m);
                next += 1;
            }
            while let Some(&Reverse((bottom, m))) = open.peek() {
                if bottom >= row {
                    break;
                }
                open.pop();
                count -= 1;
                sum -= u64::from(m);
            }
            lines.push(Line {
                row,
                from: from as u32,
                to: i as u32,
                merges: count,
                merge_sum: sum,
            });
        }
        Some(Self {
            prep,
            rect,
            cells,
            lines,
            merges,
        })
    }

    /// The cells of a line.
    pub fn cells_of(&self, line: &Line) -> impl Iterator<Item = &'a CellIn> + '_ {
        self.cells[line.from as usize..line.to as usize]
            .iter()
            .map(|&k| self.prep.cell(k as usize))
    }

    /// The index of the first line at or below `row`.
    pub fn first_at(&self, row: u32) -> usize {
        self.lines.partition_point(|l| l.row < row)
    }

    /// The line of `row`, when the row holds a cell.
    pub fn at(&self, row: u32) -> Option<&Line> {
        let i = self.first_at(row);
        self.lines.get(i).filter(|l| l.row == row)
    }

    /// The text cell of a single-text row (spec 6.3): exactly one non-empty cell, a text, and
    /// at most one merge reaching the row, which holds that cell.
    pub fn single_text(&self, line: &Line) -> Option<&'a CellIn> {
        let mut found = None;
        for c in self.cells_of(line) {
            if c.text.is_empty() {
                continue;
            }
            if found.is_some() || c.class != crate::input::Class::Text {
                return None;
            }
            found = Some(c);
        }
        let cell = found?;
        match line.merges {
            0 => Some(cell),
            1 => {
                let m = &self.prep.merges[line.merge_sum as usize];
                m.contains(cell.pos).then_some(cell)
            }
            _ => None,
        }
    }

    /// The one merge that reaches a line, when exactly one does.
    pub fn merge_of(&self, line: &Line) -> Option<&'p Rect> {
        (line.merges == 1).then(|| &self.prep.merges[line.merge_sum as usize])
    }

    /// The cells, to keep them after the lines are no longer needed.
    pub fn into_cells(self) -> Vec<u32> {
        self.cells
    }

    /// The indices into `prep.cells` of the cells in the rows `top` to `bottom`.
    pub fn cells_in(&self, top: u32, bottom: u32) -> Vec<u32> {
        let lo = self.first_at(top);
        let hi = self.lines.partition_point(|l| l.row <= bottom).max(lo);
        match (
            self.lines.get(lo),
            hi.checked_sub(1).and_then(|k| self.lines.get(k)),
        ) {
            (Some(a), Some(b)) if lo < hi => self.cells[a.from as usize..b.to as usize].to_vec(),
            _ => Vec::new(),
        }
    }

    /// The occupied cell at a position of the region.
    pub fn cell_at(&self, row: u32, col: u32) -> Option<&'a CellIn> {
        let line = self.at(row)?;
        let slice = &self.cells[line.from as usize..line.to as usize];
        let k = slice.partition_point(|&k| self.prep.cell(k as usize).pos.col < col);
        slice
            .get(k)
            .map(|&k| self.prep.cell(k as usize))
            .filter(|c| c.pos.col == col)
    }
}

/// Gives each region the cells it holds, by a sweep over the rows with the regions that are
/// open in each row keyed by their left column. Regions do not overlap, so a cell has at most
/// one.
fn distribute(prep: &Prep, rects: &[Rect], tick: &Tick) -> Option<Vec<Vec<u32>>> {
    let mut out = vec![Vec::new(); rects.len()];
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by_key(|&i| rects[i].top);
    let mut next = 0usize;
    let mut open: BTreeMap<u32, usize> = BTreeMap::new();
    for k in 0..prep.cells.len() {
        if tick.step() {
            return None;
        }
        let p = prep.cell(k).pos;
        while next < order.len() && rects[order[next]].top <= p.row {
            let i = order[next];
            open.insert(rects[i].left, i);
            next += 1;
        }
        while let Some((&left, &i)) = open.range(..=p.col).next_back() {
            let r = &rects[i];
            if r.bottom < p.row {
                // Ended above: it can never hold a cell again.
                open.remove(&left);
                continue;
            }
            if p.col <= r.right {
                out[i].push(k as u32);
            }
            break;
        }
    }
    Some(out)
}

/// Words for a stop during detection, added to the one `Stopped` issue.
pub(crate) fn note_stop(issues: &mut Vec<Issue>, words: &str) {
    match issues.iter_mut().find(|i| i.kind == IssueKind::Stopped) {
        Some(i) => {
            i.what.push_str("; ");
            i.what.push_str(words);
        }
        None => issues.push(Issue {
            what: format!("time limit reached: {words}"),
            kind: IssueKind::Stopped,
            rect: None,
        }),
    }
}

/// The table body of a region: a fact's own range, else the data area. A title that T2 put
/// above it can make the region wider than its table.
fn body(input: &Input, w: &Work) -> Rect {
    match w.fact {
        Some(f) => input.facts[f].rect.normalized(),
        None => w.region.data.unwrap_or(w.region.rect),
    }
}

/// A region the stop callback left undecided: a grid of its cells.
fn undecided(w: &mut Work, body: Rect) {
    let r = &mut w.region;
    r.kind = Kind::Grid;
    r.certainty = Certainty::Unsure;
    r.above_header.clear();
    r.header_rows.clear();
    r.body = None;
    r.header_paths.clear();
    r.label_cols = 0;
    r.hidden_header = false;
    if w.origin == Origin::Fact {
        r.data = Some(body);
    }
    r.unsure = vec!["time limit reached before the header was found".to_string()];
}

/// Why the decision of a region ended early.
pub(crate) enum Halt {
    /// The header paths would pass the path budget.
    Over,
    Stopped,
}

/// What the decision procedure carries from region to region.
pub(crate) struct Ctx<'t> {
    pub tick: &'t Tick<'t>,
    pub outline: &'t Outline,
    pub limits: &'t Limits,
    /// Bytes of header path text left.
    pub budget: usize,
    /// Regions that section headings may still add.
    pub room: usize,
    /// Issues of the report rules.
    pub issues: Vec<Issue>,
}

/// The cells of `cells` that lie in `rect`.
pub(crate) fn keep(prep: &Prep, mut cells: Vec<u32>, rect: Rect) -> Vec<u32> {
    cells.retain(|&k| rect.contains(prep.cell(k as usize).pos));
    cells
}

/// Finds the regions of one sheet and decides each one, in the order of spec 6.7: steps 1 to
/// 5 as [`segment`](crate::segment) does, then per region the header band (rules H1, H2 and
/// H3, print titles and frozen panes) and the header paths.
///
/// - A region with a band is a `Table`. Step 6c then finds its group rows (R2), trailing
///   notes, label columns (L), row hierarchy (R3) and subtotal and total rows (from facts and
///   formulas only). Fact regions keep their exact header and go to step 6c too.
/// - Rules that disagree give a `Grid` with certainty `Unsure` and the reason in `unsure`.
/// - A region with no band is cut at section heading rows (R1, up to
///   `Limits::max_section_depth` deep), each piece decided again. Else it is a `KeyValue`
///   block (K), a `Text` block (X), a header-less `Table` when it holds a number, date or
///   bool (evidence `H0`, "no header found"), or else a `Grid` with the reason
///   `no header found`.
/// - Then a region with no band right below a table, in exactly its columns and at most one
///   empty row down, joins that table (C), and text regions at most 2 empty rows below a
///   table, inside its columns, become its notes (N). Text beside a table stays (S).
///
/// Rest regions are not decided. A header path is empty when no band cell names its column;
/// the caller writes the column letter. Path text counts against `Limits::max_path_bytes`
/// per sheet: a region whose paths would pass it stays a rest grid with a `Limit` issue.
/// Sections that would pass the region limit are not split, with a `Limit` issue without a
/// range. When `stop` says stop, the regions not yet decided stay grids and the one `Stopped`
/// issue says how many.
pub fn detect(input: &Input, limits: &Limits, stop: &dyn Fn() -> bool) -> Layout {
    let (prep, mut works, mut issues) = segment_work(input, limits, stop);
    let tick = Tick::new(stop);
    let outline = if input.outline.is_empty() || tick.ask() {
        Outline::default()
    } else {
        Outline::new(input, &tick).unwrap_or_default()
    };
    let rects: Vec<Rect> = works.iter().map(|w| w.region.rect).collect();
    let mut handed = !tick.hit();
    if handed {
        match distribute(&prep, &rects, &tick) {
            Some(all) => {
                for (w, cells) in works.iter_mut().zip(all) {
                    w.cells = cells;
                }
            }
            None => handed = false,
        }
    }
    let mut ctx = Ctx {
        tick: &tick,
        outline: &outline,
        limits,
        budget: limits.max_path_bytes,
        room: limits.max_regions.max(1).saturating_sub(works.len()),
        issues: Vec::new(),
    };
    let mut extra: Vec<Work> = Vec::new();
    let mut left = 0usize;
    for w in works.iter_mut() {
        if w.origin == Origin::Rest || w.region.kind == Kind::Title {
            continue;
        }
        let body = body(input, w);
        if !handed || tick.ask() {
            undecided(w, body);
            left += 1;
            continue;
        }
        let saved = w.region.clone();
        let room = ctx.room;
        let said = ctx.issues.len();
        match decide(&prep, w, body, &mut ctx) {
            Ok(pieces) => extra.extend(pieces),
            Err(halt) => {
                // The region goes back to what segmenting left; its pieces are dropped.
                w.region = saved;
                w.dead = false;
                ctx.room = room;
                ctx.issues.truncate(said);
                match halt {
                    Halt::Over => over_budget(w, body, &mut issues),
                    Halt::Stopped => {
                        undecided(w, body);
                        left += 1;
                    }
                }
            }
        }
    }
    issues.append(&mut ctx.issues);
    works.extend(extra);
    works.retain(|w| !w.dead);
    works.sort_by_key(|w| (w.region.rect.top, w.region.rect.left));
    if left > 0 {
        let words = if left == 1 {
            "1 region was not decided".to_string()
        } else {
            format!("{left} regions were not decided")
        };
        note_stop(&mut issues, &words);
    } else if !tick.hit() {
        let done = report::continuation(&prep, &mut works, &mut ctx).is_ok()
            && report::attach_notes(&prep, &mut works, &tick).is_ok();
        if !done {
            note_stop(
                &mut issues,
                "the continuation and note rules were not applied to every table",
            );
        }
        works.retain(|w| !w.dead);
    }
    Layout {
        regions: works.into_iter().map(|w| w.region).collect(),
        issues,
    }
}

/// Step 6 for one region: the fact path to step 6c, or the whole procedure for a rule region.
/// Gives the new regions that section headings cut off.
fn decide(prep: &Prep, w: &mut Work, body: Rect, ctx: &mut Ctx) -> Result<Vec<Work>, Halt> {
    let cells = std::mem::take(&mut w.cells);
    let Some(lines) = Lines::new(prep, body, cells, &w.merges, ctx.tick) else {
        return Err(Halt::Stopped);
    };
    let out = if w.origin == Origin::Fact {
        finish_table(&lines, w, ctx).map(|()| Vec::new())
    } else {
        rule(&lines, w, 1, ctx)
    };
    let rect = w.region.rect;
    w.cells = keep(prep, lines.into_cells(), rect);
    out
}

/// Steps 6a to 6h for a rule region (or a piece of one) read through `lines`, the rows of its
/// data area. `depth` is the section depth of the region (1 at the top).
pub(crate) fn rule(
    lines: &Lines,
    w: &mut Work,
    depth: u32,
    ctx: &mut Ctx,
) -> Result<Vec<Work>, Halt> {
    let input = lines.prep.input;
    let tick = ctx.tick;
    // Print titles stay evidence only when they set the band.
    let had_print = w.region.evidence.contains(&Evidence::PrintTitles);
    w.region.evidence.retain(|e| *e != Evidence::PrintTitles);
    let before = w.region.data;
    let band = match before {
        Some(data) => header::band(lines, data, had_print, tick),
        None => Band::None,
    };
    if tick.hit() {
        return Err(Halt::Stopped);
    }
    match band {
        Band::None => report::no_band(lines, w, depth, ctx),
        Band::Unsure(why, evidence) => {
            let region = &mut w.region;
            region.kind = Kind::Grid;
            region.certainty = Certainty::Unsure;
            region.unsure = vec![why];
            region.evidence.extend(evidence);
            retain_panes(input, lines.rect, region);
            Ok(Vec::new())
        }
        Band::Rows {
            first,
            end,
            certainty,
            evidence,
        } => {
            let region = &mut w.region;
            let data = before.unwrap_or(region.rect);
            region.kind = Kind::Table;
            region.certainty = certainty;
            region.header_rows = (first..=end).collect();
            region.data = (end < data.bottom).then(|| Rect {
                top: end + 1,
                ..data
            });
            region.unsure.clear();
            region.evidence.extend(evidence);
            finish_table(lines, w, ctx)?;
            Ok(Vec::new())
        }
    }
}

/// The rest of a table once its header rows are set: header paths, then step 6c.
fn finish_table(lines: &Lines, w: &mut Work, ctx: &mut Ctx) -> Result<(), Halt> {
    let input = lines.prep.input;
    let body = lines.rect;
    let tick = ctx.tick;
    let fact = w.origin == Origin::Fact;
    let region = &mut w.region;
    if fact {
        // A fact states its own header; print titles stay evidence only when they name the
        // same rows.
        let same = input.print_titles.is_some_and(|(a, b)| {
            let (a, b) = (a.min(b), a.max(b));
            region.header_rows.first() == Some(&a) && region.header_rows.last() == Some(&b)
        });
        if !same {
            region.evidence.retain(|e| *e != Evidence::PrintTitles);
        }
    }
    let built = if region.hidden_header {
        paths::from_names(hidden_names(input, w.fact), body, &mut ctx.budget)
    } else if let (Some(&first), Some(&end)) =
        (region.header_rows.first(), region.header_rows.last())
    {
        paths::build(lines, first, end, &mut ctx.budget, tick)
    } else {
        Ok(Vec::new())
    };
    match built {
        Ok(p) => {
            // A table without header rows has no paths and so no body.
            if !p.is_empty() {
                region.body = Some(body);
            }
            region.header_paths = p;
        }
        Err(PathStop::Stopped) => return Err(Halt::Stopped),
        Err(PathStop::Over) => return Err(Halt::Over),
    }
    let pivot = fact
        && region
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::Pivot { .. }));
    let facts = w.fact.map(|f| FactRows {
        kind: &input.facts[f].kind,
        rect: input.facts[f].rect.normalized(),
    });
    if !rows::table_rows(lines, region, facts, pivot, ctx.outline, tick) {
        return Err(Halt::Stopped);
    }
    retain_panes(input, body, region);
    Ok(())
}

/// The column names of an Excel table whose header row is hidden.
fn hidden_names<'a>(input: &'a Input, fact: Option<usize>) -> &'a [String] {
    match fact.map(|f| &input.facts[f].kind) {
        Some(FactKind::ListObject { columns, .. }) => columns,
        _ => &[],
    }
}

/// Frozen panes stay evidence only when they confirm the band or the label columns.
pub(crate) fn retain_panes(input: &Input, body: Rect, region: &mut Region) {
    let Some((rows, cols)) = input.frozen else {
        return;
    };
    let table = region.kind == Kind::Table;
    let by_rows = table
        && rows > 0
        && region
            .header_rows
            .last()
            .is_some_and(|&end| u64::from(end) + 1 == u64::from(rows));
    let by_cols = table
        && cols > 0
        && region.label_cols > 0
        && u64::from(body.left) + u64::from(region.label_cols) == u64::from(cols);
    if !(by_rows || by_cols) {
        region.evidence.retain(|e| *e != Evidence::FrozenPanes);
    }
}

/// A region whose paths would pass the path budget: a rest grid with a `Limit` issue.
fn over_budget(w: &mut Work, body: Rect, issues: &mut Vec<Issue>) {
    let r = &mut w.region;
    let what = "header path limit reached".to_string();
    issues.push(Issue {
        what: format!("{what}: {} is one grid region", r.rect.a1()),
        kind: IssueKind::Limit,
        rect: Some(r.rect),
    });
    r.kind = Kind::Grid;
    r.certainty = Certainty::Unsure;
    if w.origin == Origin::Fact {
        r.data = Some(body);
    }
    r.evidence
        .retain(|e| !matches!(e, Evidence::PrintTitles | Evidence::FrozenPanes));
    r.above_header.clear();
    r.body = None;
    r.header_rows.clear();
    r.header_paths.clear();
    r.label_cols = 0;
    r.hidden_header = false;
    r.unsure = vec![what];
    r.rest = true;
}
