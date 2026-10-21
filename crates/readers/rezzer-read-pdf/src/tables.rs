// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables of a page, in the order the table spike found reliable: the tag tree of a tagged
//! file first, then a grid of drawn rules, then white space gutters in text without rules.
//! Each detector only claims the runs of a table and plans its cells; the layout builds the
//! cells' text, so every limit of the layout holds for tables too.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pdfparts::StructTree;

use crate::layout::{Drawn, Flow, Run, Slot};

/// A drawn line or rectangle, in points in the visible page box like the runs, `y` upward.
/// A line has `x0 == x1` or `y0 == y1`; a rectangle is its bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rule {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub kind: RuleKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RuleKind {
    Line,
    Rect,
}

/// A table found on a page, with the runs of each cell. Its cells' text is not built yet.
#[derive(Debug)]
pub(crate) struct Planned {
    /// The place of the table's first run in the page's reading order: the table stands
    /// before the runs that come after it.
    pub at: usize,
    /// The top of the table, in the frame of the runs.
    pub top: f32,
    pub rows: Vec<Vec<PlannedCell>>,
    /// The leading rows made only of header cells.
    pub header_rows: usize,
    /// The tagged table the rows belong to, which may have started on an earlier page.
    pub tagged: Option<usize>,
    /// The rows go on a table that started on an earlier page.
    pub continues: bool,
}

#[derive(Debug, Default)]
pub(crate) struct PlannedCell {
    pub runs: Vec<Run>,
    pub row_span: u32,
    pub col_span: u32,
}

/// What a detector found on a page besides tables: a region that may be a table and is left as
/// paragraphs, with why. The layout writes it as a note where the region stands and as a loss.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Unsure {
    pub at: usize,
    pub top: f32,
    pub reason: String,
}

/// The rows of the tagged tables met so far, each table read once for all its pages, and what
/// the first page of each table set.
#[derive(Default)]
pub(crate) struct Tables {
    rows: RefCell<HashMap<usize, Rc<TableRows>>>,
    /// Tagged table to the page it started on and the text of its header rows.
    started: RefCell<HashMap<usize, (u32, Vec<String>)>>,
    /// What was found broken in the tag tree, once per table, in words.
    issues: RefCell<Vec<String>>,
}

/// The rows of one tagged table: the `TD` and `TH` cells of each row, the row of each cell
/// and, per page, how many cells hold marked content there.
struct TableRows {
    rows: Vec<Vec<usize>>,
    row_of: HashMap<usize, usize>,
    content_cells: HashMap<u32, usize>,
}

/// Elements below one cell looked at to find the pages it has content on; a deeper cell
/// counts only what was seen.
const CELL_WALK: usize = 4_096;

impl Tables {
    fn rows(&self, tree: &StructTree, table: usize, page: u32) -> Option<Rc<TableRows>> {
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
        let mut row_of = HashMap::new();
        let mut twice = 0usize;
        for (i, tr) in trs.into_iter().enumerate() {
            let mut cells: Vec<usize> = Vec::new();
            for &c in &elems[tr].children {
                if !elems
                    .get(c)
                    .is_some_and(|e| matches!(e.role.as_str(), "TD" | "TH"))
                {
                    continue;
                }
                // A cell that a broken tree puts in two rows stays in the first.
                if row_of.contains_key(&c) {
                    twice += 1;
                    continue;
                }
                row_of.insert(c, i);
                cells.push(c);
            }
            rows.push(cells);
        }
        if twice > 0 {
            let (cells, stand, kept) = if twice == 1 {
                ("1 cell", "stands", "it is")
            } else {
                (&*format!("{twice} cells"), "stand", "each is")
            };
            self.issues.borrow_mut().push(format!(
                "{cells} of a tagged table on page {page} {stand} in more than one row of the \
                 tag tree; {kept} kept in its first row"
            ));
        }
        let mut content_cells: HashMap<u32, usize> = HashMap::new();
        for &c in row_of.keys() {
            for p in content_pages(tree, c) {
                *content_cells.entry(p).or_default() += 1;
            }
        }
        let found = Rc::new(TableRows {
            rows,
            row_of,
            content_cells,
        });
        self.rows.borrow_mut().insert(table, found.clone());
        Some(found)
    }

    /// What was found broken in the tag tree since the last call, in words.
    pub(crate) fn take_issues(&self) -> Vec<String> {
        std::mem::take(&mut self.issues.borrow_mut())
    }
}

/// The pages a cell has marked content on, from its element and the elements below it.
fn content_pages(tree: &StructTree, cell: usize) -> Vec<u32> {
    let mut pages: Vec<u32> = Vec::new();
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
            && !pages.contains(&p)
        {
            pages.push(p);
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
/// can be looked at for tables as if it were untagged.
pub(crate) fn tagged(
    page: u32,
    flow: &mut Flow,
    tree: &StructTree,
    tables: &Tables,
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
        let Some(index) = tables.rows(tree, t, page) else {
            continue;
        };
        let resolved = with_text[&t].len();
        let content = index.content_cells.get(&page).copied().unwrap_or(0);
        if resolved * 2 < content {
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
    // A table not used leaves its text as text.
    for s in &mut flow.slots {
        if matches!(s, Slot::Cell { .. }) {
            *s = Slot::Free;
        }
    }
    let mut out = Vec::new();
    for t in order {
        let Some(index) = used.get(&t) else { continue };
        let mut touched: Vec<usize> = with_text[&t]
            .iter()
            .filter_map(|c| index.row_of.get(c).copied())
            .collect();
        touched.sort_unstable();
        touched.dedup();
        let mut rows: Vec<Vec<PlannedCell>> = Vec::with_capacity(touched.len());
        let mut header_rows = 0;
        let mut in_header = true;
        for row in touched {
            let row_cells = &index.rows[row];
            let built: Vec<PlannedCell> = row_cells
                .iter()
                .map(|c| PlannedCell {
                    runs: cells.remove(c).unwrap_or_default(),
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
        }
        // A table without any text (a layout box, an empty frame) is no table.
        let blank = rows
            .iter()
            .flatten()
            .flat_map(|c| &c.runs)
            .all(|r| r.text.trim().is_empty());
        if blank {
            continue;
        }
        let mut started = tables.started.borrow_mut();
        let continues = match started.get(&t) {
            Some((p, _)) => *p != page,
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
            header_rows = 0;
        }
        if rows.is_empty() {
            continue;
        }
        let (at, top) = first[&t];
        out.push(Planned {
            at,
            top,
            rows,
            header_rows,
            tagged: Some(t),
            continues,
        });
    }
    (out, fallbacks)
}

/// What the detectors for untagged pages found: tables, undecided regions, and what they
/// could not look at, in words.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub tables: Vec<Planned>,
    pub unsure: Vec<Unsure>,
    pub skipped: Vec<String>,
}

/// Lines this close are one line, and line ends this close meet, in points.
const SNAP: f32 = 2.0;

/// A grid of drawn rules with more columns than this is a form or a chart, not a table.
const MAX_COLUMNS: usize = 15;

/// A grid whose cells are more often empty than this is a form to fill in.
const EMPTY_SHARE: f32 = 0.6;

/// A grid or region that covers more of the page than this is the page's frame.
const PAGE_SHARE: f32 = 0.6;

/// Straight segments the ruled grid search looks at on one page, and pairs of segments it
/// tests; past them it gives up on the page and says so.
const MAX_SEGMENTS: usize = 20_000;
const GRID_WORK: usize = 4_000_000;

/// Lines of a caption paragraph above a table looked at for its start.
const CAPTION_LINES: usize = 8;

/// Squares one grid of drawn rules may have to be read as a table.
const MAX_SQUARES: usize = 100_000;

/// Text smaller than this share of the body text is a picture of a page, not a table.
const SMALL_TEXT: f32 = 0.6;

/// A gap wider than this many font sizes between two runs of a line separates two cells.
const CELL_GAP: f32 = 1.0;

/// Cells of a table without rules are short: their median length is under this many
/// characters, unless most of them are numbers.
const SHORT_CELL: usize = 40;

/// A cell of at least this many words and characters that goes on in lower case in the cell
/// below reads as a line of prose.
const PROSE_WORDS: usize = 3;
const PROSE_LINE: usize = 20;

/// Share of the rows of a table without rules whose pieces stay inside the columns.
const CLEAN_ROWS: f32 = 0.8;

/// The tables of an untagged page (or a tagged page whose tag tree did not hold): first
/// grids of drawn rules, then white space gutters in the text left. Their runs are taken out
/// of `flow`. `height` is the page height; text in other directions is never in a table.
pub(crate) fn untagged(
    page: u32,
    flow: &mut Flow,
    drawn: &Drawn<'_>,
    height: f32,
    stop: &dyn Fn() -> bool,
) -> Found {
    let mut found = Found::default();
    // On a page with form fields, boxes and aligned labels are the form.
    if !drawn.widgets.is_empty() {
        return found;
    }
    let area = Area {
        width: drawn.width,
        height,
        images: drawn.images,
        body_size: drawn.body_size,
    };
    ruled(page, flow, drawn.rules, &area, stop, &mut found);
    borderless(page, flow, &area, stop, &mut found);
    found
}

/// The page a detector looks at: its size and what rules out a table.
struct Area<'a> {
    width: f32,
    height: f32,
    images: &'a [[f32; 4]],
    /// The most common font size of the file's text.
    body_size: f32,
}

impl Area<'_> {
    /// Why a table cannot stand in `b` (x0, y0, x1, y1) with text of `sizes`, if it cannot.
    fn refuses(&self, b: [f32; 4], sizes: &[f32]) -> Option<&'static str> {
        let size = (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
        if size > PAGE_SHARE * self.width * self.height {
            return Some("it covers most of the page");
        }
        if self.images.iter().any(|i| overlap(*i, b) > 0.5 * size) {
            return Some("it lies on a picture");
        }
        let mut sizes = sizes.to_vec();
        sizes.sort_by(f32::total_cmp);
        if let Some(m) = sizes.get(sizes.len() / 2)
            && *m < SMALL_TEXT * self.body_size
        {
            return Some("its text is much smaller than the body text, as in a picture of a page");
        }
        None
    }
}

/// The area two boxes share.
fn overlap(a: [f32; 4], b: [f32; 4]) -> f32 {
    let w = a[2].min(b[2]) - a[0].max(b[0]);
    let h = a[3].min(b[3]) - a[1].max(b[1]);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

/// The caption kind of a line: `Some(true)` for "Table 3", `Some(false)` for "Figure 3".
fn caption(text: &str) -> Option<bool> {
    let t = text.trim_start();
    let (table, rest) = match t.strip_prefix("Table") {
        Some(r) => (true, r),
        None => (
            false,
            t.strip_prefix("Figure")
                .or_else(|| t.strip_prefix("Fig."))?,
        ),
    };
    let rest = rest.trim_start();
    let first = rest.chars().next()?;
    (first.is_ascii_digit()
        || (first.is_ascii_uppercase()
            && rest.len() > 1
            && rest[1..].starts_with(['.', ':', ' ', '-'])))
    .then_some(table)
}

/// One line of the flow: the range of its runs and its baseline and size.
#[derive(Debug, Clone, Copy)]
struct FlowLine {
    start: usize,
    end: usize,
    y: f32,
    size: f32,
}

fn flow_lines(flow: &Flow) -> Vec<FlowLine> {
    let mut out: Vec<FlowLine> = Vec::new();
    for (i, r) in flow.runs.iter().enumerate() {
        match out.last_mut() {
            Some(l) if flow.runs[l.start].line == r.line => {
                l.end = i + 1;
                l.size = l.size.max(r.size);
            }
            _ => out.push(FlowLine {
                start: i,
                end: i + 1,
                y: r.y,
                size: r.size.max(1.0),
            }),
        }
    }
    out
}

/// The caption kinds next to a box: a caption whose paragraph ends within three line heights
/// above the box, or starts within three line heights below it, and overlaps it across.
/// Lines `skip` names (the box's own) are not looked at.
fn captions_near(
    flow: &Flow,
    lines: &[FlowLine],
    by_y: &ByY,
    b: [f32; 4],
    skip: &dyn Fn(usize) -> bool,
) -> (bool, bool) {
    let (mut table, mut figure) = (false, false);
    let text = |l: &FlowLine| -> String {
        flow.runs[l.start..l.end]
            .iter()
            .map(|r| r.text.as_str())
            .collect()
    };
    let across = |l: &FlowLine| {
        let runs = &flow.runs[l.start..l.end];
        let x0 = runs.iter().map(|r| r.x).fold(f32::MAX, f32::min);
        let x1 = runs.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max);
        !runs.iter().any(|r| r.off_axis) && x1 >= b[0] && x0 <= b[2]
    };
    let mut note = |l: &FlowLine| match caption(&text(l)) {
        Some(true) => table = true,
        Some(false) => figure = true,
        None => {}
    };
    // Only lines in a band around the box can be near it.
    let band = 4.0 * by_y.size;
    let lo = by_y.line_y.partition_point(|y| *y < b[1] - band);
    let hi = by_y.line_y.partition_point(|y| *y <= b[3] + band);
    for &k in &by_y.lines[lo..hi] {
        let l = &lines[k];
        if skip(l.start) || !across(l) {
            continue;
        }
        let near = 3.0 * l.size;
        if l.y < b[1] && b[1] - (l.y + l.size) <= near {
            note(l);
        } else if l.y > b[3] && l.y - b[3] <= near {
            // The last line of a paragraph above: its first lines may hold the caption.
            note(l);
            let mut at = k;
            for _ in 0..CAPTION_LINES {
                let Some(prev) = at.checked_sub(1).map(|p| &lines[p]) else {
                    break;
                };
                let gap = prev.y - lines[at].y;
                if gap <= 0.0 || gap > 1.6 * prev.size.max(lines[at].size) || !across(prev) {
                    break;
                }
                note(prev);
                at -= 1;
            }
        }
    }
    (table, figure)
}

/// The runs and lines of a page sorted by height, so a box finds what lies in it by search.
struct ByY {
    /// Run indices by the height of their middle, and those heights.
    runs: Vec<usize>,
    run_y: Vec<f32>,
    /// Line indices by baseline, and the baselines.
    lines: Vec<usize>,
    line_y: Vec<f32>,
    /// The largest font size of the page.
    size: f32,
}

impl ByY {
    fn new(flow: &Flow, lines: &[FlowLine]) -> ByY {
        let mut runs: Vec<(f32, usize)> = flow
            .runs
            .iter()
            .enumerate()
            .map(|(i, r)| (middle(r).1, i))
            .collect();
        runs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut ls: Vec<(f32, usize)> = lines.iter().enumerate().map(|(k, l)| (l.y, k)).collect();
        ls.sort_by(|a, b| a.0.total_cmp(&b.0));
        ByY {
            run_y: runs.iter().map(|r| r.0).collect(),
            runs: runs.into_iter().map(|r| r.1).collect(),
            line_y: ls.iter().map(|l| l.0).collect(),
            lines: ls.into_iter().map(|l| l.1).collect(),
            size: lines.iter().map(|l| l.size).fold(1.0, f32::max),
        }
    }

    /// The runs whose middle lies between the heights `lo` and `hi`, in page order.
    fn runs_between(&self, lo: f32, hi: f32) -> Vec<usize> {
        let a = self.run_y.partition_point(|y| *y <= lo);
        let z = self.run_y.partition_point(|y| *y < hi);
        let mut out = self.runs[a..z].to_vec();
        out.sort_unstable();
        out
    }
}

/// The point of a run that decides which cell it is in.
fn middle(r: &Run) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h * 0.3)
}

/// Grids and regions one page may have looked at between two looks at the clock.
const CHECK_EVERY: usize = 64;

/// A straight horizontal (`at` is y, `from`..`to` along x) or vertical segment.
#[derive(Debug, Clone, Copy)]
struct Seg {
    at: f32,
    from: f32,
    to: f32,
}

/// Collinear segments that touch become one.
fn merged(mut segs: Vec<Seg>) -> Vec<Seg> {
    segs.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
    let mut out: Vec<Seg> = Vec::with_capacity(segs.len());
    let mut group_start = 0;
    for s in segs {
        let joined = out[group_start..]
            .iter_mut()
            .rev()
            .find(|o| (o.at - s.at).abs() <= SNAP && s.from <= o.to + SNAP);
        match joined {
            Some(o) => o.to = o.to.max(s.to),
            None => {
                if out.last().is_none_or(|o| (o.at - s.at).abs() > SNAP) {
                    group_start = out.len();
                }
                out.push(s);
            }
        }
    }
    out
}

/// Values closer than [`SNAP`] are one, sorted.
fn clustered(mut values: Vec<f32>) -> Vec<f32> {
    values.sort_by(f32::total_cmp);
    let mut out: Vec<f32> = Vec::new();
    for v in values {
        match out.last() {
            Some(l) if v - l <= SNAP => {}
            _ => out.push(v),
        }
    }
    out
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Grids of drawn rules: horizontal and vertical lines that meet, with text in their cells.
fn ruled(
    page: u32,
    flow: &mut Flow,
    rules: &[Rule],
    area: &Area<'_>,
    stop: &dyn Fn() -> bool,
    found: &mut Found,
) {
    let mut hs = Vec::new();
    let mut vs = Vec::new();
    for r in rules {
        let (x0, x1) = (r.x0.min(r.x1), r.x0.max(r.x1));
        let (y0, y1) = (r.y0.min(r.y1), r.y0.max(r.y1));
        match r.kind {
            RuleKind::Line if y1 - y0 <= SNAP / 2.0 => hs.push(Seg {
                at: (y0 + y1) / 2.0,
                from: x0,
                to: x1,
            }),
            RuleKind::Line => vs.push(Seg {
                at: (x0 + x1) / 2.0,
                from: y0,
                to: y1,
            }),
            RuleKind::Rect => {
                hs.push(Seg {
                    at: y0,
                    from: x0,
                    to: x1,
                });
                hs.push(Seg {
                    at: y1,
                    from: x0,
                    to: x1,
                });
                vs.push(Seg {
                    at: x0,
                    from: y0,
                    to: y1,
                });
                vs.push(Seg {
                    at: x1,
                    from: y0,
                    to: y1,
                });
            }
        }
    }
    if hs.is_empty() || vs.is_empty() {
        return;
    }
    if hs.len() + vs.len() > MAX_SEGMENTS {
        found.skipped.push(format!(
            "ruled tables on page {page} are not looked for: the page draws more than \
             {MAX_SEGMENTS} straight lines"
        ));
        return;
    }
    let hs = merged(hs);
    let vs = merged(vs);
    // Segments that meet are one grid. Horizontal ones are sorted by y.
    let n = hs.len();
    let mut parent: Vec<usize> = (0..n + vs.len()).collect();
    let mut work = 0usize;
    for (k, v) in vs.iter().enumerate() {
        let lo = hs.partition_point(|h| h.at < v.from - SNAP);
        for (j, h) in hs.iter().enumerate().skip(lo) {
            if h.at > v.to + SNAP {
                break;
            }
            work += 1;
            if work > GRID_WORK {
                found.skipped.push(format!(
                    "ruled tables on page {page} are not looked for: its lines meet too often"
                ));
                return;
            }
            if v.at >= h.from - SNAP && v.at <= h.to + SNAP {
                let (a, b) = (find(&mut parent, j), find(&mut parent, n + k));
                parent[a] = b;
            }
        }
    }
    let mut groups: HashMap<usize, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for j in 0..n {
        let root = find(&mut parent, j);
        groups.entry(root).or_default().0.push(j);
    }
    for k in 0..vs.len() {
        let root = find(&mut parent, n + k);
        groups.entry(root).or_default().1.push(k);
    }
    let mut grids: Vec<(Vec<usize>, Vec<usize>)> = groups
        .into_values()
        .filter(|(h, v)| h.len() >= 2 && v.len() >= 2)
        .collect();
    // Top to bottom.
    grids.sort_by(|a, b| {
        let top =
            |g: &(Vec<usize>, Vec<usize>)| g.0.iter().map(|&j| hs[j].at).fold(f32::MIN, f32::max);
        top(b).total_cmp(&top(a))
    });
    let lines = flow_lines(flow);
    let by_y = ByY::new(flow, &lines);
    // Runs a grid took, and for each the table and the row and cell it goes to.
    let mut claimed: Vec<Option<(usize, usize, usize)>> = vec![None; flow.runs.len()];
    let mut tables: Vec<Planned> = Vec::new();
    for (g, (gh, gv)) in grids.into_iter().enumerate() {
        if g % CHECK_EVERY == CHECK_EVERY - 1 && stop() {
            found.skipped.push(format!(
                "ruled tables on page {page} are looked for only in part: the time limit was \
                 reached"
            ));
            break;
        }
        let h: Vec<Seg> = gh.iter().map(|&j| hs[j]).collect();
        let v: Vec<Seg> = gv.iter().map(|&k| vs[k]).collect();
        let mut xs: Vec<f32> = v.iter().map(|s| s.at).collect();
        xs.push(h.iter().map(|s| s.from).fold(f32::MAX, f32::min));
        xs.push(h.iter().map(|s| s.to).fold(f32::MIN, f32::max));
        let mut ys: Vec<f32> = h.iter().map(|s| s.at).collect();
        ys.push(v.iter().map(|s| s.from).fold(f32::MAX, f32::min));
        ys.push(v.iter().map(|s| s.to).fold(f32::MIN, f32::max));
        let xs = clustered(xs);
        let mut ys = clustered(ys);
        // Rows from the top.
        ys.reverse();
        let (cols, rows) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
        if cols < 2 || rows < 2 {
            continue;
        }
        if rows.saturating_mul(cols) > MAX_SQUARES {
            found.skipped.push(format!(
                "a grid of {rows} by {cols} drawn cells on page {page} is not read as a table: \
                 it has more than {MAX_SQUARES} cells"
            ));
            continue;
        }
        let b = [xs[0], ys[rows], xs[cols], ys[0]];
        let inside = |i: usize| {
            let r = &flow.runs[i];
            let (cx, _) = middle(r);
            !r.off_axis && claimed[i].is_none() && cx > b[0] && cx < b[2]
        };
        let mine: Vec<usize> = by_y
            .runs_between(b[1], b[3])
            .into_iter()
            .filter(|&i| inside(i))
            .collect();
        let within: Vec<usize> = mine
            .iter()
            .copied()
            .filter(|&i| !flow.runs[i].text.trim().is_empty())
            .collect();
        if within.is_empty() {
            continue;
        }
        let at = flow.at[within[0]];
        let (table_caption, figure_caption) = captions_near(flow, &lines, &by_y, b, &|i| {
            let (_, cy) = middle(&flow.runs[i]);
            inside(i) && cy > b[1] && cy < b[3]
        });
        let sizes: Vec<f32> = within.iter().map(|&i| flow.runs[i].size).collect();
        // Whether a boundary is drawn over at least half of a cell's side.
        let covered = |segs: &[Seg], at: f32, lo: f32, hi: f32| {
            let mut len = 0.0;
            for s in segs.iter().filter(|s| (s.at - at).abs() <= SNAP) {
                len += (s.to.min(hi) - s.from.max(lo)).max(0.0);
            }
            len >= 0.5 * (hi - lo)
        };
        // The cell each grid square belongs to: a missing line between two squares joins
        // them, across first, then down where whole cells line up.
        let mut owner: Vec<Vec<(usize, usize)>> = vec![vec![(0, 0); cols]; rows];
        for (r, row) in owner.iter_mut().enumerate() {
            for c in 0..cols {
                row[c] = if c > 0 && !covered(&v, xs[c], ys[r + 1], ys[r]) {
                    row[c - 1]
                } else {
                    (r, c)
                };
            }
        }
        let span_of =
            |row: &[(usize, usize)], c: usize| row.iter().filter(|o| **o == row[c]).count();
        for r in 1..rows {
            let mut c = 0;
            while c < cols {
                let span = span_of(&owner[r], c);
                let above = owner[r - 1][c];
                let lined_up =
                    owner[r][c] == (r, c) && above.1 == c && span_of(&owner[r - 1], c) == span;
                if lined_up && !(c..c + span).any(|k| covered(&h, ys[r], xs[k], xs[k + 1])) {
                    owner[r][c..c + span].fill(above);
                }
                c += span;
            }
        }
        // Each cell's extent in squares: rows down and columns across.
        let mut extent: HashMap<(usize, usize), (u32, u32)> = HashMap::new();
        for (r, row) in owner.iter().enumerate() {
            for (c, o) in row.iter().enumerate() {
                let e = extent.entry(*o).or_insert((0, 0));
                if c == o.1 {
                    e.0 += 1;
                }
                if r == o.0 {
                    e.1 += 1;
                }
            }
        }
        let cell_at = |x: f32, y: f32| -> (usize, usize) {
            let r = ys
                .partition_point(|e| *e > y)
                .saturating_sub(1)
                .min(rows - 1);
            let c = xs
                .partition_point(|e| *e < x)
                .saturating_sub(1)
                .min(cols - 1);
            owner[r][c]
        };
        let mut with_text: HashSet<(usize, usize)> = HashSet::new();
        let mut crossing = 0usize;
        let mut caption_inside = false;
        for &i in &within {
            let r = &flow.runs[i];
            let (cx, cy) = middle(r);
            let o = cell_at(cx, cy);
            with_text.insert(o);
            // A run that reaches over its cell's side crosses a drawn line.
            let (x0, x1) = (xs[o.1], xs[(o.1 + extent[&o].1 as usize).min(cols)]);
            if r.x < x0 - SNAP || r.x + r.w > x1 + SNAP {
                crossing += 1;
            }
            if caption(&r.text) == Some(false) {
                caption_inside = true;
            }
        }
        let spanning = extent.values().filter(|(rs, cs)| rs * cs > 1).count();
        let empty = extent.len() - with_text.len();
        let refused = if cols > MAX_COLUMNS {
            Some("it has more than 15 columns")
        } else if figure_caption || caption_inside {
            Some("a figure caption stands next to it")
        } else if empty as f32 > EMPTY_SHARE * extent.len() as f32 {
            Some("most of its cells are empty")
        } else if 10 * spanning > 3 * extent.len() {
            Some("its boxes do not form rows and columns")
        } else if 5 * crossing > within.len() {
            Some("its text crosses its lines")
        } else {
            area.refuses(b, &sizes)
        };
        if let Some(why) = refused {
            if table_caption {
                found.unsure.push(Unsure {
                    at,
                    top: b[3],
                    reason: why.to_string(),
                });
            }
            continue;
        }
        // The place of each cell in the planned rows.
        let mut place: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
        let mut planned_rows: Vec<Vec<PlannedCell>> = Vec::with_capacity(rows);
        for (r, row) in owner.iter().enumerate() {
            let mut cells = Vec::new();
            for (c, o) in row.iter().enumerate() {
                if *o != (r, c) {
                    continue;
                }
                let (row_span, col_span) = extent[o];
                place.insert(*o, (r, cells.len()));
                cells.push(PlannedCell {
                    runs: Vec::new(),
                    row_span,
                    col_span,
                });
            }
            planned_rows.push(cells);
        }
        let t = tables.len();
        for &i in &mine {
            let (cx, cy) = middle(&flow.runs[i]);
            let (r, c) = place[&cell_at(cx, cy)];
            claimed[i] = Some((t, r, c));
        }
        tables.push(Planned {
            at,
            top: b[3],
            rows: planned_rows,
            header_rows: 0,
            tagged: None,
            continues: false,
        });
    }
    if tables.is_empty() {
        return;
    }
    // The runs of all grids leave the flow at once.
    let mut k = 0;
    let taken = flow.take(|_, _| {
        k += 1;
        claimed[k - 1].is_some()
    });
    for ((r, _, _), (t, row, cell)) in taken.into_iter().zip(claimed.into_iter().flatten()) {
        tables[t].rows[row][cell].runs.push(r);
    }
    for mut t in tables {
        t.header_rows = header_of(&t.rows);
        found.tables.push(t);
    }
}

/// 1 when the first row reads as column titles: every cell has text and either a column has
/// a word on top of numbers, or the first row is bold and the rest is not.
fn header_of(rows: &[Vec<PlannedCell>]) -> usize {
    let text = |c: &PlannedCell| {
        c.runs
            .iter()
            .flat_map(|r| r.text.split_whitespace())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let Some(first) = rows.first() else { return 0 };
    if rows.len() < 2 || first.iter().any(|c| text(c).is_empty()) {
        return 0;
    }
    let titles = (0..first.len()).any(|c| {
        let below: Vec<String> = rows[1..]
            .iter()
            .filter_map(|r| r.get(c))
            .map(text)
            .collect();
        let numbers = below.iter().filter(|t| numeric(t)).count();
        !numeric(&text(&first[c])) && numbers > 0 && 2 * numbers >= below.len()
    });
    let bold = |row: &[PlannedCell]| row.iter().flat_map(|c| &c.runs).all(|r| r.bold);
    let bold_titles = bold(first) && rows[1..].iter().any(|r| !bold(r));
    usize::from(titles || bold_titles)
}

/// A run of text of one line, apart from its neighbours by a gap: one cell's text.
#[derive(Debug, Clone)]
struct Piece {
    x0: f32,
    x1: f32,
    /// Range of runs in the flow.
    start: usize,
    end: usize,
}

fn pieces_of(flow: &Flow, l: &FlowLine) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for i in l.start..l.end {
        let r = &flow.runs[i];
        let x0 = r.x;
        match out.last_mut() {
            Some(p) if x0 - p.x1 <= CELL_GAP * r.size.max(1.0) => {
                p.x1 = p.x1.max(r.x + r.w);
                p.end = i + 1;
            }
            _ => out.push(Piece {
                x0,
                x1: r.x + r.w,
                start: i,
                end: i + 1,
            }),
        }
    }
    out
}

/// The text of runs `start..end` of the flow, white space collapsed.
fn text_of(flow: &Flow, start: usize, end: usize) -> String {
    let mut s = String::new();
    for r in &flow.runs[start..end] {
        for w in r.text.split_whitespace() {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(w);
        }
    }
    s
}

/// Whether a cell's text is a number: most of its characters are digits or signs.
fn numeric(text: &str) -> bool {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let digits = chars.iter().filter(|c| c.is_ascii_digit()).count();
    let signs = chars
        .iter()
        .filter(|c| matches!(c, '.' | ',' | '%' | '-' | '+' | '$' | '(' | ')' | '/'))
        .count();
    digits > 0 && 2 * (digits + signs) >= chars.len()
}

/// Tables without rules: regions of lines whose pieces stand in columns with white space
/// between them, accepted only on strong evidence.
fn borderless(
    page: u32,
    flow: &mut Flow,
    area: &Area<'_>,
    stop: &dyn Fn() -> bool,
    found: &mut Found,
) {
    let lines = flow_lines(flow);
    let by_y = ByY::new(flow, &lines);
    let pieces: Vec<Vec<Piece>> = lines.iter().map(|l| pieces_of(flow, l)).collect();
    let off: Vec<bool> = lines
        .iter()
        .map(|l| flow.runs[l.start..l.end].iter().any(|r| r.off_axis))
        .collect();
    // Regions: lines that follow each other down the page, most of them in two pieces or
    // more; at most two lines of one piece in a row (cell text that wraps).
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if off[i] || pieces[i].len() < 2 {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        let mut last_multi = i;
        while end < lines.len() && !off[end] {
            let (prev, cur) = (&lines[end - 1], &lines[end]);
            let gap = prev.y - cur.y;
            if gap <= 0.0 || gap > 3.0 * prev.size.max(cur.size) {
                break;
            }
            if pieces[end].len() >= 2 {
                last_multi = end;
            } else if end - last_multi > 2 {
                break;
            }
            end += 1;
        }
        regions.push((i, last_multi + 1));
        i = last_multi + 1;
    }
    let mut claim: Vec<(usize, usize, Planned, Grid)> = Vec::new();
    for (g, (a, z)) in regions.into_iter().enumerate() {
        if g % CHECK_EVERY == CHECK_EVERY - 1 && stop() {
            found.skipped.push(format!(
                "tables without rules on page {page} are looked for only in part: the time \
                 limit was reached"
            ));
            break;
        }
        let multi = (a..z).filter(|&k| pieces[k].len() >= 2).count();
        if multi < 2 {
            continue;
        }
        let x0 = (a..z)
            .flat_map(|k| pieces[k].iter().map(|p| p.x0))
            .fold(f32::MAX, f32::min);
        let x1 = (a..z)
            .flat_map(|k| pieces[k].iter().map(|p| p.x1))
            .fold(f32::MIN, f32::max);
        let b = [
            x0,
            lines[z - 1].y - 0.25 * lines[z - 1].size,
            x1,
            lines[a].y + lines[a].size,
        ];
        let in_region = |i: usize| (lines[a].start..lines[z - 1].end).contains(&i);
        let (table_caption, figure_caption) = captions_near(flow, &lines, &by_y, b, &in_region);
        let rows_needed = if table_caption { 2 } else { 3 };
        let verdict = grid_of(
            flow,
            &lines[a..z],
            &pieces[a..z],
            rows_needed,
            table_caption,
        )
        .and_then(|grid| {
            if figure_caption {
                return Err("a figure caption stands next to it");
            }
            let sizes: Vec<f32> = flow.runs[lines[a].start..lines[z - 1].end]
                .iter()
                .map(|r| r.size)
                .collect();
            match area.refuses(b, &sizes) {
                Some(why) => Err(why),
                None => Ok(grid),
            }
        });
        match verdict {
            Ok(grid) => {
                let planned = Planned {
                    at: flow.at[lines[a].start],
                    top: b[3],
                    rows: grid
                        .rows
                        .iter()
                        .map(|r| {
                            r.iter()
                                .map(|c| PlannedCell {
                                    // Filled in below, once the runs are taken out.
                                    runs: Vec::new(),
                                    row_span: 1,
                                    col_span: c.span,
                                })
                                .collect()
                        })
                        .collect(),
                    header_rows: 0,
                    tagged: None,
                    continues: false,
                };
                claim.push((a, z, planned, grid));
            }
            Err(why) => {
                // A grid of rules over the same text may have said so already.
                let (lo, hi) = (flow.at[lines[a].start], flow.at[lines[z - 1].end - 1]);
                let said = found.unsure.iter().any(|u| (lo..=hi).contains(&u.at));
                if table_caption && !said {
                    found.unsure.push(Unsure {
                        at: flow.at[lines[a].start],
                        top: b[3],
                        reason: why.to_string(),
                    });
                }
            }
        }
    }
    if claim.is_empty() {
        return;
    }
    // The runs of all accepted regions leave the flow at once; each goes to its cell.
    let mut target: Vec<Option<(usize, usize, usize)>> = vec![None; flow.runs.len()];
    for (t, (_, _, _, grid)) in claim.iter().enumerate() {
        for (r, cells) in grid.rows.iter().enumerate() {
            for (c, g) in cells.iter().enumerate() {
                for &(s, e) in &g.ranges {
                    for slot in &mut target[s..e] {
                        *slot = Some((t, r, c));
                    }
                }
            }
        }
    }
    let mut k = 0;
    let taken = flow.take(|_, _| {
        k += 1;
        target[k - 1].is_some()
    });
    let mut planned: Vec<Planned> = claim.into_iter().map(|c| c.2).collect();
    for ((r, _, _), (t, row, cell)) in taken.into_iter().zip(target.into_iter().flatten()) {
        planned[t].rows[row][cell].runs.push(r);
    }
    for mut p in planned {
        p.header_rows = header_of(&p.rows);
        found.tables.push(p);
    }
    found.tables.sort_by_key(|t| t.at);
}

/// A grid rebuilt from white space gutters: rows of cells, each cell its runs as flow ranges.
struct Grid {
    rows: Vec<Vec<GridCell>>,
}

struct GridCell {
    ranges: Vec<(usize, usize)>,
    span: u32,
}

/// Rebuilds the grid of a region and tests it, or says why it is no table.
fn grid_of(
    flow: &Flow,
    lines: &[FlowLine],
    pieces: &[Vec<Piece>],
    rows_needed: usize,
    table_caption: bool,
) -> Result<Grid, &'static str> {
    // Columns: the x ranges the pieces of the lines with the most common number of pieces
    // fill; the first line may be a header that spans columns.
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for p in pieces.iter().filter(|p| p.len() >= 2) {
        *counts.entry(p.len()).or_default() += 1;
    }
    let mode = counts
        .iter()
        .max_by_key(|(n, k)| (**k, **n))
        .map(|(n, _)| *n)
        .unwrap_or(0);
    let mut spans: Vec<(f32, f32)> = pieces
        .iter()
        .enumerate()
        .filter(|(k, p)| p.len() == mode && (*k > 0 || pieces.len() == 1))
        .flat_map(|(_, p)| p.iter().map(|p| (p.x0, p.x1)))
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut columns: Vec<(f32, f32)> = Vec::new();
    for (s, e) in spans {
        match columns.last_mut() {
            Some(c) if s <= c.1 => c.1 = c.1.max(e),
            _ => columns.push((s, e)),
        }
    }
    let cols = columns.len();
    if cols < 2 {
        return Err("no columns");
    }
    let over = |p: &Piece| -> Vec<usize> {
        (0..cols)
            .filter(|&c| p.x1 > columns[c].0 && p.x0 < columns[c].1)
            .collect()
    };
    // The column of each piece; a line whose piece crosses a gutter is not clean.
    let mut clean = 0usize;
    let mut placed: Vec<Vec<(usize, u32, &Piece)>> = Vec::with_capacity(lines.len());
    for (k, line) in pieces.iter().enumerate() {
        let mut row = Vec::new();
        let mut ok = true;
        for p in line {
            let hit = over(p);
            let (col, span) = match hit.as_slice() {
                // Text in a gutter belongs to no column: the grid does not hold.
                [] => return Err("some of its text stands between its columns"),
                [c] => (*c, 1),
                many => {
                    if k > 0 {
                        ok = false;
                    }
                    (many[0], (many[many.len() - 1] - many[0] + 1) as u32)
                }
            };
            row.push((col, span, p));
        }
        if ok {
            clean += 1;
        }
        placed.push(row);
    }
    if (clean as f32) < CLEAN_ROWS * lines.len() as f32 {
        return Err("its columns are not clean");
    }
    // Rows: lines that fill at least half the columns; other lines join the nearest one.
    let filled = |row: &Vec<(usize, u32, &Piece)>| {
        let mut c: Vec<usize> = row.iter().map(|x| x.0).collect();
        c.dedup();
        c.len()
    };
    let anchors: Vec<usize> = (0..lines.len())
        .filter(|&k| 2 * filled(&placed[k]) >= cols)
        .collect();
    if anchors.len() < rows_needed.max(1) {
        return Err("too few rows");
    }
    let mut rows: Vec<Vec<GridCell>> = anchors
        .iter()
        .map(|_| {
            (0..cols)
                .map(|_| GridCell {
                    ranges: Vec::new(),
                    span: 1,
                })
                .collect()
        })
        .collect();
    for (k, row) in placed.iter().enumerate() {
        // The anchor rows are in order: the nearest is the one before or the one after.
        let after = anchors.partition_point(|&a| a < k);
        let target = match (after.checked_sub(1), anchors.get(after)) {
            (Some(b), Some(&a)) => {
                let d = |x: usize| (lines[x].y - lines[k].y).abs();
                if d(a) < d(anchors[b]) { after } else { b }
            }
            (Some(b), None) => b,
            (None, _) => after,
        };
        for &(col, span, p) in row {
            let cell = &mut rows[target][col];
            cell.ranges.push((p.start, p.end));
            cell.span = cell.span.max(span);
        }
    }
    // A cell that spans hides the cells it covers.
    for row in &mut rows {
        let mut c = 0;
        while c < row.len() {
            let span = row[c].span as usize;
            for k in c + 1..(c + span).min(row.len()) {
                let moved = std::mem::take(&mut row[k].ranges);
                row[c].ranges.extend(moved);
                row[k].span = 0;
            }
            c += span.max(1);
        }
        row.retain(|c| c.span > 0);
    }
    // Short cells, or mostly numbers.
    let texts: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| {
                    let mut ranges = c.ranges.clone();
                    ranges.sort_unstable();
                    ranges
                        .iter()
                        .map(|&(s, e)| text_of(flow, s, e))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()
        })
        .collect();
    let mut lens: Vec<usize> = texts
        .iter()
        .flatten()
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().count())
        .collect();
    lens.sort_unstable();
    if texts.len() > 1 && texts.iter().all(|r| *r == texts[0]) {
        return Err("all its rows read the same");
    }
    let numbers = texts.iter().flatten().filter(|t| numeric(t)).count();
    let short = if table_caption { 60 } else { SHORT_CELL };
    if lens.is_empty() || (lens[lens.len() / 2] >= short && 2 * numbers < lens.len()) {
        return Err("its cells hold long text");
    }
    // Text that runs on from a cell into the cell below like prose is two columns of text.
    for c in 0..cols {
        for r in 1..texts.len() {
            let (Some(above), Some(below)) = (texts[r - 1].get(c), texts[r].get(c)) else {
                continue;
            };
            // A line of prose: a few words that end without a stop, and the next line goes on
            // in lower case.
            let words = above.split_whitespace().count();
            let ends = above.ends_with(['.', ':', ';', '!', '?']);
            let line = words >= PROSE_WORDS && above.chars().count() >= PROSE_LINE;
            if line && !ends && below.chars().next().is_some_and(char::is_lowercase) {
                return Err("its text runs on from row to row like prose");
            }
        }
    }
    // Numbered entries with page numbers in the last column, going up, are a table of
    // contents.
    let pages: Vec<Option<u32>> = texts
        .iter()
        .map(|r| r.last().and_then(|t| t.parse::<u32>().ok()))
        .collect();
    let numbered = texts.iter().all(|r| {
        r.first()
            .and_then(|t| t.split_whitespace().next())
            .is_some_and(|w| {
                w.trim_end_matches('.')
                    .split('.')
                    .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            })
    });
    if texts.len() >= 3
        && numbered
        && pages.iter().all(Option::is_some)
        && pages.windows(2).all(|w| w[0] <= w[1])
    {
        return Err("it reads like a table of contents");
    }
    Ok(Grid { rows })
}
