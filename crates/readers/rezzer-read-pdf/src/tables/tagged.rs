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
}

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
/// can be looked at for tables as if it were untagged. A table kept as paragraphs on an earlier
/// page leaves its runs in the flow, in their cells, so they are laid out as text where they
/// stand.
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
    let taken =
        flow.take(|_, s| matches!(s, Slot::Cell { table, .. } if used.contains_key(&table)));
    for (r, s, at) in taken {
        if let Slot::Cell { table, cell } = s {
            let top = r.y + r.h;
            let f = first.entry(table).or_insert((at, top));
            f.1 = f.1.max(top);
            cells.entry(cell).or_default().push(r);
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
        });
    }
    (out, fallbacks)
}
