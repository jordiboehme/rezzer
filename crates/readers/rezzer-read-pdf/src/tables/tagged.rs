// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables from the tag tree of a tagged file.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pdfparts::StructTree;

use super::{Addition, Planned, PlannedCell};
use crate::layout::{Flow, Run, Slot};

/// The rows of the tagged tables met so far, each table read once for all its pages, and what
/// the first page of each table set.
#[derive(Default)]
pub(crate) struct Tables {
    rows: RefCell<HashMap<usize, Rc<TableRows>>>,
    /// Tagged table to the page it started on and the text of its header rows.
    started: RefCell<HashMap<usize, (u32, Vec<String>)>>,
    /// What was found broken in the tag tree, once per table, in words.
    issues: RefCell<Vec<String>>,
    /// The place in its table of each tag tree row built so far, and the rows each table
    /// has: a row is planned once, and later pages add to its cells.
    built: RefCell<HashMap<(usize, usize), usize>>,
    counts: RefCell<HashMap<usize, usize>>,
    /// Tagged tables kept as paragraphs because their overhead did not fit: their text on
    /// every later page stays paragraphs too.
    as_text: RefCell<HashSet<usize>>,
    /// The cuts of each tagged table whose cells hold the text of several columns (see
    /// [`column_cuts`]), by the place of the tagged column: worked out on the first page that
    /// holds the table and kept for all its pages, so a table is cut the same way on every page.
    cuts: RefCell<HashMap<usize, Cuts>>,
    /// What was found glued in the tag tree since the last call, in words.
    glued: RefCell<Vec<String>>,
}

/// The cuts of a tagged table: for each place of a tagged column (counted by the cells'
/// spans) whose cells hold the text of several columns, the x of each cut, left to right.
type Cuts = Vec<(usize, Vec<f32>)>;

/// A gap between the ink of two columns in one tagged cell is wider than this many text sizes.
///
/// Measured margins (fix round 2, G3): the narrowest column gap of the masked private item
/// table is 2.73 points between two bold header titles of about 9 points, 0.30 sizes (0.33
/// against the size the gap is measured with in `column_cuts`, the run's own). A word space is
/// about 0.25 to 0.28 sizes (Helvetica 0.278). So the margin is small on both sides, which is
/// why the rule also needs the gap to line up over two rows and over half of the rows with text:
/// the gaps between words of runs set word by word fall at other places from row to row and
/// are filled by the other rows' ink. A gap measured against the column's own word gaps (the
/// `gaps` inside its runs) gives no safer margin: in a face like Helvetica those word gaps are
/// about 0.28 sizes too, as close to the 0.30 column gap as the fixed share is, and a column
/// with no word gap inside its runs (dates, amounts) has none to measure against. Kept at 0.25.
const COLUMN_GAP: f32 = 0.25;

/// The rows of one tagged table: the `TD` and `TH` cells of each row, the row and column of
/// each cell and, per page, how many cells hold marked content there.
struct TableRows {
    rows: Vec<Vec<usize>>,
    cell_at: HashMap<usize, (usize, usize)>,
    content_cells: HashMap<u32, usize>,
}

/// Cells looked at between two looks at the clock.
const STOP_EVERY: usize = 4_096;

/// Elements below one cell looked at to find the pages it has content on; a deeper cell
/// counts only what was seen.
const CELL_WALK: usize = 4_096;

impl Tables {
    /// The rows of `table`, read once. `None` when `stop` says so before they are read; they
    /// are read again on a later page then.
    fn rows(
        &self,
        tree: &StructTree,
        table: usize,
        page: u32,
        stop: &dyn Fn() -> bool,
    ) -> Option<Rc<TableRows>> {
        if let Some(found) = self.rows.borrow().get(&table) {
            return Some(found.clone());
        }
        let elems = &tree.elems;
        let mut trs: Vec<usize> = Vec::new();
        for &c in &elems.get(table)?.children {
            match elems.get(c).map(|e| e.role.as_str()) {
                Some("TR") => trs.push(c),
                Some("THead" | "TBody" | "TFoot") => trs.extend(
                    elems[c]
                        .children
                        .iter()
                        .copied()
                        .filter(|&r| elems.get(r).is_some_and(|e| e.role == "TR")),
                ),
                _ => {}
            }
        }
        let mut rows = Vec::with_capacity(trs.len());
        let mut cell_at: HashMap<usize, (usize, usize)> = HashMap::new();
        let mut twice = 0usize;
        let mut repeated = 0usize;
        for (i, tr) in trs.into_iter().enumerate() {
            let mut cells: Vec<usize> = Vec::new();
            for &c in &elems[tr].children {
                if !elems
                    .get(c)
                    .is_some_and(|e| matches!(e.role.as_str(), "TD" | "TH"))
                {
                    continue;
                }
                // A cell that a broken tree puts in two rows stays in the first; one that a
                // row lists twice is kept once.
                match cell_at.get(&c) {
                    Some(&(r, _)) if r == i => {
                        repeated += 1;
                        continue;
                    }
                    Some(_) => {
                        twice += 1;
                        continue;
                    }
                    None => {}
                }
                cell_at.insert(c, (i, cells.len()));
                cells.push(c);
            }
            rows.push(cells);
        }
        let mut issues = Vec::new();
        if twice > 0 {
            let (cells, stand, kept) = if twice == 1 {
                ("1 cell", "stands", "it is")
            } else {
                (&*format!("{twice} cells"), "stand", "each is")
            };
            issues.push(format!(
                "{cells} of a tagged table on page {page} {stand} in more than one row of the \
                 tag tree; {kept} kept in its first row"
            ));
        }
        if repeated > 0 {
            let (cells, is) = if repeated == 1 {
                ("1 cell", "it is")
            } else {
                (&*format!("{repeated} cells"), "each is")
            };
            issues.push(format!(
                "{cells} of a tagged table on page {page} listed twice in the same row of the \
                 tag tree; {is} kept once"
            ));
        }
        let mut content_cells: HashMap<u32, usize> = HashMap::new();
        for (k, &c) in cell_at.keys().enumerate() {
            if k % STOP_EVERY == STOP_EVERY - 1 && stop() {
                return None;
            }
            for p in content_pages(tree, c) {
                *content_cells.entry(p).or_default() += 1;
            }
        }
        self.issues.borrow_mut().extend(issues);
        let found = Rc::new(TableRows {
            rows,
            cell_at,
            content_cells,
        });
        self.rows.borrow_mut().insert(table, found.clone());
        Some(found)
    }

    /// Notes that row `tr` of tagged table `table` was built: it is the next row of the table.
    pub(crate) fn row_built(&self, table: usize, tr: usize) {
        let mut counts = self.counts.borrow_mut();
        let n = counts.entry(table).or_default();
        self.built.borrow_mut().insert((table, tr), *n);
        *n += 1;
    }

    /// Notes that tagged table `table` is kept as paragraphs for the rest of the file.
    pub(crate) fn keep_as_text(&self, table: usize) {
        self.as_text.borrow_mut().insert(table);
    }

    fn is_text(&self, table: usize) -> bool {
        self.as_text.borrow().contains(&table)
    }

    fn row_at(&self, table: usize, tr: usize) -> Option<usize> {
        self.built.borrow().get(&(table, tr)).copied()
    }

    /// What was found broken in the tag tree since the last call, in words.
    pub(crate) fn take_issues(&self) -> Vec<String> {
        std::mem::take(&mut self.issues.borrow_mut())
    }

    /// Whether tagged table `table` has columns its tags glue (see [`column_cuts`]): its
    /// cells are cut, so the tag tree is no exact reading of it.
    pub(crate) fn is_cut(&self, table: usize) -> bool {
        self.cuts
            .borrow()
            .get(&table)
            .is_some_and(|c| !c.is_empty())
    }

    /// Says once per page that a tagged table on `page` was cut at `cuts`.
    fn note_glued(&self, page: u32, cuts: &Cuts) {
        let columns: usize = cuts.iter().map(|c| c.1.len()).sum();
        let mut glued = self.glued.borrow_mut();
        glued.push(format!(
            "a tagged table on page {page} holds the text of {} columns in {} of its tagged \
             columns; its cells are cut where that text lines up apart over its rows",
            columns + cuts.len(),
            cuts.len()
        ));
    }

    /// What was found glued in the tag tree since the last call, in words.
    pub(crate) fn take_glued(&self) -> Vec<String> {
        std::mem::take(&mut self.glued.borrow_mut())
    }
}

/// The ink of a run, if it has any: its left and right x and its size.
fn ink(r: &Run) -> Option<(f32, f32, f32)> {
    (!r.text.trim().is_empty()).then_some((r.x, r.x + r.w.max(0.0), r.size))
}

/// A raised mark is at most this share of the size of the text it stands beside.
pub(crate) const MARK_SIZE: f32 = 0.75;

/// A raised mark's baseline stands at least this share of the size of the text beside it over
/// that text's baseline.
pub(crate) const MARK_RISE: f32 = 0.15;

/// Which of `runs` (the runs of one tagged cell) are raised marks: a footnote reference the
/// layout knows (`Run::note`), or a run with ink set at most [`MARK_SIZE`] times as large as a
/// run with ink beside it on its line and raised over that run's baseline by [`MARK_RISE`]
/// times its size, within its height (a footnote or reference mark). A mark carries no ink of
/// a column of its own; it stands with the run nearest to it (see [`parted`]).
pub(crate) fn marks(runs: &[&Run]) -> Vec<bool> {
    let mut by_y: Vec<(f32, f32)> = runs
        .iter()
        .filter(|r| !r.text.trim().is_empty())
        .map(|r| (r.y, r.size))
        .collect();
    by_y.sort_by(|a, b| a.0.total_cmp(&b.0));
    let most = by_y.iter().map(|t| t.1).fold(0.0f32, f32::max);
    runs.iter()
        .map(|r| {
            if r.note {
                return true;
            }
            if r.text.trim().is_empty() {
                return false;
            }
            // Runs whose baseline is up to the largest size under this one's.
            let lo = by_y.partition_point(|t| t.0 < r.y - most);
            let hi = by_y.partition_point(|t| t.0 < r.y);
            by_y[lo..hi].iter().any(|&(y, size)| {
                r.size <= MARK_SIZE * size && r.y - y >= MARK_RISE * size && r.y - y <= size
            })
        })
        .collect()
}

/// The tagged columns whose cells hold the text of several columns (see [`column_cuts`]),
/// by the rows of a table's first page: each column's place (counted by the cells' spans)
/// and its cuts. Only cells one column wide are looked at.
fn glued_columns(rows: &[Vec<PlannedCell>]) -> Cuts {
    let mut by_place: std::collections::BTreeMap<usize, Vec<Vec<&Run>>> =
        std::collections::BTreeMap::new();
    for row in rows {
        let mut at = 0usize;
        for c in row {
            if c.col_span == 1 {
                by_place
                    .entry(at)
                    .or_default()
                    .push(c.runs.iter().collect());
            }
            at += c.col_span as usize;
        }
    }
    by_place
        .into_iter()
        .filter_map(|(at, cells)| {
            let cuts = column_cuts(&cells);
            (!cuts.is_empty()).then_some((at, cuts))
        })
        .collect()
}

/// Where the cells of one tagged column (`cells`, one per row, each its runs) are cut into the
/// columns their text stands in: the ink of all their runs, side by side, parts into stretches
/// apart by more than [`COLUMN_GAP`] text sizes; a stretch that holds text of one row only
/// joins the stretch left of it (the first one the stretch right of it). The cells are glued
/// when two stretches or more are left and the text of at least two rows, and of at least half
/// of the rows with text, stands in two of them or more: the column gaps line up over the rows
/// (a header with titles over values, item rows; lines of one cell that wrap stand in one
/// stretch and change nothing). The cuts stand in the middle of the gaps; none when the cells
/// are not glued.
///
/// Limits: a table of one row with text on its first page is not looked at (no rows to line
/// up); columns whose text comes closer than [`COLUMN_GAP`] sizes on some row stay one; a
/// column with text in one row only stays with its neighbour; cells wider than one tagged
/// column are not cut; the cuts of the first page hold for the table's later pages.
fn column_cuts(cells: &[Vec<&Run>]) -> Vec<f32> {
    let mut spans: Vec<(f32, f32, f32, usize)> = Vec::new();
    let mut rows = 0usize;
    // The runs of each cell that carry column ink: no raised mark.
    let inked: Vec<Vec<&Run>> = cells
        .iter()
        .map(|runs| {
            runs.iter()
                .zip(marks(runs))
                .filter(|(_, m)| !m)
                .map(|(r, _)| *r)
                .collect()
        })
        .collect();
    for (k, runs) in inked.iter().enumerate() {
        let before = spans.len();
        spans.extend(
            runs.iter()
                .filter_map(|r| ink(r))
                .map(|(x0, x1, size)| (x0, x1, size, k)),
        );
        rows += usize::from(spans.len() > before);
    }
    if rows < 2 {
        return Vec::new();
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Stretches: (x0, x1, rows with text in it).
    let mut stretches: Vec<(f32, f32, HashSet<usize>)> = Vec::new();
    for (x0, x1, size, k) in spans {
        match stretches.last_mut() {
            Some(s) if x0 <= s.1 + COLUMN_GAP * size.max(1.0) => {
                s.1 = s.1.max(x1);
                s.2.insert(k);
            }
            _ => stretches.push((x0, x1, HashSet::from([k]))),
        }
    }
    // One pass: a stretch of one row joins the one left of it; the first, the one right of it.
    let mut kept: Vec<(f32, f32, HashSet<usize>)> = Vec::with_capacity(stretches.len());
    for (x0, x1, ks) in stretches {
        match kept.last_mut() {
            Some(s) if ks.len() < 2 || s.2.len() < 2 => {
                s.0 = s.0.min(x0);
                s.1 = s.1.max(x1);
                s.2.extend(ks);
            }
            _ => kept.push((x0, x1, ks)),
        }
    }
    let stretches = kept;
    if stretches.len() < 2 {
        return Vec::new();
    }
    let cuts: Vec<f32> = stretches
        .windows(2)
        .map(|w| (w[0].1 + w[1].0) / 2.0)
        .collect();
    let apart = inked
        .iter()
        .filter(|runs| {
            let mut at: HashSet<usize> = HashSet::new();
            for (x0, x1, _) in runs.iter().filter_map(|r| ink(r)) {
                at.insert(cuts.partition_point(|c| *c < (x0 + x1) / 2.0));
            }
            at.len() >= 2
        })
        .count();
    if apart >= 2 && 2 * apart >= rows {
        cuts
    } else {
        Vec::new()
    }
}

/// `runs` parted by the cuts `xs`: one list for each column, by the middle of each run.
fn parted(runs: Vec<Run>, xs: &[f32]) -> Vec<Vec<Run>> {
    let refs: Vec<&Run> = runs.iter().collect();
    let mark = marks(&refs);
    let column = |r: &Run| xs.partition_point(|c| *c < r.x + r.w.max(0.0) / 2.0);
    // The column of each run: a raised mark's is that of the run nearest to it on its line
    // (by the gap between them), else its own.
    let at: Vec<usize> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| {
            if !mark[i] {
                return column(r);
            }
            runs.iter()
                .enumerate()
                .filter(|(j, o)| {
                    !mark[*j]
                        && !o.text.trim().is_empty()
                        && r.y >= o.y - 0.3 * o.size
                        && r.y <= o.y + o.size
                })
                .map(|(_, o)| {
                    let gap = (o.x - (r.x + r.w)).max(r.x - (o.x + o.w)).max(0.0);
                    (gap, column(o))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map_or_else(|| column(r), |(_, c)| c)
        })
        .collect();
    let mut out: Vec<Vec<Run>> = (0..=xs.len()).map(|_| Vec::new()).collect();
    for (r, c) in runs.into_iter().zip(at) {
        out[c].push(r);
    }
    out
}

/// The extra columns the cuts put before the tagged place `at`, and in the places `at` to
/// `at + span`.
fn extra(cuts: &Cuts, at: usize, span: usize) -> (usize, usize) {
    let before = cuts.iter().filter(|c| c.0 < at).map(|c| c.1.len()).sum();
    let within = cuts
        .iter()
        .filter(|c| c.0 >= at && c.0 < at + span)
        .map(|c| c.1.len())
        .sum();
    (before, within)
}

/// A row of a tagged table with its glued cells cut (see [`glued_columns`]): a cell one column
/// wide at a cut place becomes one cell for each of its columns; a wider cell over cut places
/// spans their columns too.
fn cut_row(row: Vec<PlannedCell>, cuts: &Cuts) -> Vec<PlannedCell> {
    let mut out = Vec::with_capacity(row.len());
    let mut at = 0usize;
    for c in row {
        let span = c.col_span as usize;
        let here = cuts.iter().find(|k| k.0 == at).filter(|_| span == 1);
        match here {
            Some((_, xs)) => {
                for runs in parted(c.runs, xs) {
                    out.push(PlannedCell {
                        runs,
                        row_span: c.row_span,
                        col_span: 1,
                        status: None,
                    });
                }
            }
            None => {
                let (_, within) = extra(cuts, at, span);
                out.push(PlannedCell {
                    col_span: c.col_span + u32::try_from(within).unwrap_or(0),
                    ..c
                });
            }
        }
        at += span;
    }
    out
}

/// An addition to a row an earlier page began, its cell cut as the row's cells were: its
/// cell's place among the cut cells of its row, one addition for each column it has text in.
fn cut_addition(
    a: Addition,
    tag_row: &[usize],
    elems: &[pdfparts::StructElem],
    cuts: &Cuts,
) -> Vec<Addition> {
    let span = |c: usize| elems.get(c).map_or(1, |e| e.col_span.max(1) as usize);
    let at: usize = tag_row.iter().take(a.cell).map(|&c| span(c)).sum();
    // The cells of the row before this one after the cut: one per cell, plus the extra cells
    // of the cut ones.
    let mut index = a.cell;
    let mut place = 0usize;
    for &c in tag_row.iter().take(a.cell) {
        if span(c) == 1
            && let Some((_, xs)) = cuts.iter().find(|k| k.0 == place)
        {
            index += xs.len();
        }
        place += span(c);
    }
    let one_wide = tag_row.get(a.cell).is_some_and(|&c| span(c) == 1);
    match cuts.iter().find(|k| k.0 == at).filter(|_| one_wide) {
        Some((_, xs)) => parted(a.runs, xs)
            .into_iter()
            .enumerate()
            .filter(|(_, runs)| !runs.is_empty())
            .map(|(k, runs)| Addition {
                row: a.row,
                cell: index + k,
                runs,
            })
            .collect(),
        None => vec![Addition { cell: index, ..a }],
    }
}

/// The pages a cell has marked content on, from its element and the elements below it.
fn content_pages(tree: &StructTree, cell: usize) -> HashSet<u32> {
    let mut pages: HashSet<u32> = HashSet::new();
    let mut stack = vec![cell];
    let mut seen = 0;
    while let Some(i) = stack.pop() {
        seen += 1;
        if seen > CELL_WALK {
            break;
        }
        let Some(e) = tree.elems.get(i) else { continue };
        if !e.mcids.is_empty()
            && let Some(p) = e.page
        {
            pages.insert(p);
        }
        stack.extend(e.children.iter().copied());
    }
    pages
}

/// The text of a row's cells, to tell a header row repeated on a later page.
fn row_text(cells: &[PlannedCell]) -> String {
    let mut out = String::new();
    for c in cells {
        for r in &c.runs {
            out.extend(r.text.split_whitespace().flat_map(|w| [w, " "]));
        }
        out.push('\u{1f}');
    }
    out
}

/// The tagged tables of a page, from the tag tree only: the runs in table cells are taken out
/// of `flow`, and each table they belong to is planned with
/// the rows that have text on this page. A table that started on an earlier page goes on
/// there, without the header rows it repeats. A table whose cells on this page mostly have no
/// text is not used: its runs stay as text, set free, and the reason is returned so the page
/// can be looked at for tables as if it were untagged. A tagged column whose cells hold the
/// text of several columns that line up over its rows is cut into those columns (see
/// [`column_cuts`]; the tags lose their certainty, [`Tables::glued`] says so). A table kept as
/// paragraphs on an earlier page leaves its runs in the flow, in their cells, so they are laid
/// out as text where they stand.
pub(crate) fn tagged(
    page: u32,
    flow: &mut Flow,
    tree: &StructTree,
    tables: &Tables,
    in_part: bool,
    stop: &dyn Fn() -> bool,
) -> (Vec<Planned>, Vec<String>) {
    let elems = &tree.elems;
    // The cells with text on this page, by table, in order of the tables' first run.
    let mut order: Vec<usize> = Vec::new();
    let mut with_text: HashMap<usize, HashSet<usize>> = HashMap::new();
    for s in &flow.slots {
        if let Slot::Cell { table, cell } = *s {
            let cells = with_text.entry(table).or_insert_with(|| {
                order.push(table);
                HashSet::new()
            });
            cells.insert(cell);
        }
    }
    if order.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut fallbacks = Vec::new();
    let mut used: HashMap<usize, Rc<TableRows>> = HashMap::new();
    for &t in &order {
        if tables.is_text(t) {
            continue;
        }
        let Some(index) = tables.rows(tree, t, page, stop) else {
            continue;
        };
        let resolved = with_text[&t].len();
        let content = index.content_cells.get(&page).copied().unwrap_or(0);
        // On a page a limit cut, cells without text may only be unread: no judgement.
        if !in_part && resolved * 2 < content {
            fallbacks.push(format!(
                "a tagged table on page {page} is not used: only {resolved} of its {content} \
                 cells with content on the page have text; the page is looked at as if it were \
                 not tagged"
            ));
            continue;
        }
        used.insert(t, index);
    }
    // Take the runs of the tables used out of the page, by cell.
    let mut cells: HashMap<usize, Vec<Run>> = HashMap::new();
    let mut first: HashMap<usize, (usize, f32)> = HashMap::new();
    // The ids of the runs taken, by table.
    let mut ids: HashMap<usize, Vec<u32>> = HashMap::new();
    let taken =
        flow.take(|_, s| matches!(s, Slot::Cell { table, .. } if used.contains_key(&table)));
    for t in taken {
        if let Slot::Cell { table, cell } = t.slot {
            let r = t.run;
            let top = r.y + r.h;
            let f = first.entry(table).or_insert((t.at, top));
            f.1 = f.1.max(top);
            cells.entry(cell).or_default().push(r);
            ids.entry(table).or_default().push(t.id);
        }
    }
    // A table not used leaves its text as text. The runs of a table kept as text stay in
    // their cells: they are laid out as free text, but are content, not running headers.
    for s in &mut flow.slots {
        if matches!(*s, Slot::Cell { table, .. } if !tables.is_text(table)) {
            *s = Slot::Free;
        }
    }
    let mut out = Vec::new();
    for t in order {
        let Some(index) = used.get(&t) else { continue };
        // The cells with text on this page, by row and column: only those are looked at.
        let mut here: Vec<(usize, usize, usize)> = with_text[&t]
            .iter()
            .filter_map(|c| index.cell_at.get(c).map(|&(r, k)| (r, k, *c)))
            .collect();
        here.sort_unstable();
        let mut touched: Vec<usize> = Vec::new();
        let mut additions: Vec<Addition> = Vec::new();
        // The tag tree row of each addition.
        let mut added_to: Vec<usize> = Vec::new();
        for (row, k, c) in here {
            // A row an earlier page began gets this page's runs in its own cells.
            match tables.row_at(t, row) {
                Some(at) => {
                    if let Some(runs) = cells.remove(&c) {
                        additions.push(Addition {
                            row: at,
                            cell: k,
                            runs,
                        });
                        added_to.push(row);
                    }
                }
                None if touched.last() != Some(&row) => touched.push(row),
                None => {}
            }
        }
        let mut rows: Vec<Vec<PlannedCell>> = Vec::with_capacity(touched.len());
        let mut trs: Vec<usize> = Vec::with_capacity(touched.len());
        let mut header_rows = 0;
        let mut in_header = true;
        for row in touched {
            let row_cells = &index.rows[row];
            let built: Vec<PlannedCell> = row_cells
                .iter()
                .map(|c| PlannedCell {
                    runs: cells
                        .remove(c)
                        .map(|mut runs| {
                            runs.shrink_to_fit();
                            runs
                        })
                        .unwrap_or_default(),
                    row_span: elems[*c].row_span.max(1),
                    col_span: elems[*c].col_span.max(1),
                    status: None,
                })
                .collect();
            if in_header && row_cells.iter().all(|c| elems[*c].role == "TH") {
                header_rows += 1;
            } else {
                in_header = false;
            }
            rows.push(built);
            trs.push(row);
        }
        // The columns the tags glue are cut where their text stands apart (decided on the
        // table's first page, kept for its later pages).
        let cuts = tables
            .cuts
            .borrow_mut()
            .entry(t)
            .or_insert_with(|| glued_columns(&rows))
            .clone();
        if !cuts.is_empty() {
            rows = rows.into_iter().map(|r| cut_row(r, &cuts)).collect();
            additions = additions
                .into_iter()
                .zip(added_to)
                .flat_map(|(a, tr)| cut_addition(a, &index.rows[tr], elems, &cuts))
                .collect();
            tables.note_glued(page, &cuts);
        }
        // A table without any text (a layout box, an empty frame) is no table.
        let blank = rows
            .iter()
            .flatten()
            .flat_map(|c| &c.runs)
            .chain(additions.iter().flat_map(|a| &a.runs))
            .all(|r| r.text.trim().is_empty());
        if blank {
            continue;
        }
        let mut started = tables.started.borrow_mut();
        let continues = match started.get(&t) {
            Some((p, _)) => *p != page,
            None if rows.is_empty() => continue,
            None => {
                // The column titles a later page may repeat: the header rows, or the first
                // row of a table whose titles are not tagged as headers, unless the row
                // after it reads the same (a table of like rows has no titles).
                let header: Vec<String> = if header_rows > 0 {
                    rows[..header_rows].iter().map(|r| row_text(r)).collect()
                } else {
                    let first = row_text(&rows[0]);
                    match rows.get(1) {
                        Some(next) if row_text(next) == first => Vec::new(),
                        _ => vec![first],
                    }
                };
                started.insert(t, (page, header));
                false
            }
        };
        if continues {
            // Header rows a later page repeats are dropped.
            let header = &started[&t].1;
            let repeated = rows
                .iter()
                .zip(header)
                .take_while(|(row, h)| row_text(row) == **h)
                .count();
            rows.drain(..repeated);
            trs.drain(..repeated);
            header_rows = 0;
        }
        if rows.is_empty() && additions.is_empty() {
            continue;
        }
        let (at, top) = first[&t];
        out.push(Planned {
            at,
            top,
            rows,
            trs,
            additions,
            header_rows,
            tagged: Some(t),
            continues,
            caption: Vec::new(),
            found_by: None,
            kind: layoutregions::Kind::Table,
            ids: ids.remove(&t).unwrap_or_default(),
            by: super::Detector::Tagged,
            drawn: None,
        });
    }
    (out, fallbacks)
}
