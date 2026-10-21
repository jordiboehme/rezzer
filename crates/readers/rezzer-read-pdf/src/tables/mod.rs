// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables of a page, in the order the table spike found reliable: the tag tree of a tagged
//! file first, then a grid of drawn rules, then white space gutters in text without rules.
//! Each detector only claims the runs of a table and plans its cells; the layout builds the
//! cells' text, so every limit of the layout holds for tables too.

mod borderless;
mod ruled;
mod tagged;

use std::collections::HashMap;

use crate::layout::{Drawn, Flow, Run};

pub(crate) use tagged::{Tables, tagged};

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
    /// For a tagged table, the row of the tag tree each of `rows` is.
    pub trs: Vec<usize>,
    /// Text for rows of a tagged table that earlier pages began.
    pub additions: Vec<Addition>,
    /// The leading rows made only of header cells.
    pub header_rows: usize,
    /// The tagged table the rows belong to, which may have started on an earlier page.
    pub tagged: Option<usize>,
    /// The rows go on a table that started on an earlier page.
    pub continues: bool,
}

/// Runs for a cell of a row an earlier page began: the row's place in its table and the
/// cell's place in the row.
#[derive(Debug)]
pub(crate) struct Addition {
    pub row: usize,
    pub cell: usize,
    pub runs: Vec<Run>,
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
    /// The whole note, when it speaks of several regions at once.
    pub text: Option<String>,
}

/// What the detectors for untagged pages found: tables, undecided regions, and what they
/// could not look at, in words.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub tables: Vec<Planned>,
    pub unsure: Vec<Unsure>,
    pub skipped: Vec<String>,
    /// For each run of the flow, whether a refused grid with a note holds it: no detector
    /// after it may take the run into a table. Empty when no grid holds a run.
    pub held: Vec<bool>,
}

/// Lines this close are one line, and line ends this close meet, in points.
pub(crate) const SNAP: f32 = 2.0;

/// A grid of drawn rules with more columns than this is a form or a chart, not a table.
const MAX_COLUMNS: usize = 15;

/// A grid whose cells are more often empty than this is a form to fill in.
const EMPTY_SHARE: f32 = 0.6;

/// A grid or region that covers more of the page than this is the page's frame.
pub(crate) const PAGE_SHARE: f32 = 0.6;

/// Straight segments the ruled grid search looks at on one page, and pairs of segments it
/// tests; past them it gives up on the page and says so.
pub(crate) const MAX_SEGMENTS: usize = 20_000;
pub(crate) const GRID_WORK: usize = 4_000_000;

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
    ruled::ruled(page, flow, drawn.rules, &area, stop, &mut found);
    borderless::borderless(page, flow, &area, stop, &mut found);
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
/// Lines `skip` names (the box's own) are not looked at. `sizes` are the font sizes of the
/// box's text: their median sets how far from the box a caption may stand.
fn captions_near(
    flow: &Flow,
    lines: &[FlowLine],
    by_y: &ByY,
    b: [f32; 4],
    sizes: &[f32],
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
    // Only lines in a band around the box can be near it: a caption is set about as large as
    // the table's own text.
    let mut sorted = sizes.to_vec();
    sorted.sort_by(f32::total_cmp);
    let size = sorted
        .get(sorted.len() / 2)
        .copied()
        .unwrap_or(10.0)
        .clamp(1.0, MAX_CAPTION_SIZE);
    let band = 4.0 * size;
    let lo = by_y.line_y.partition_point(|y| *y < b[1] - band);
    let hi = by_y.line_y.partition_point(|y| *y <= b[3] + band);
    for &k in &by_y.lines[lo..hi] {
        let l = &lines[k];
        if skip(l.start) || !across(l) {
            continue;
        }
        let near = 3.0 * l.size.min(2.0 * size);
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

/// Where a run of the page goes: the table, row and cell of `planned`.
type Target = Option<(u32, u32, u32)>;

/// Moves the runs `target` names out of `flow` into the cells of `planned`. Each cell is sized
/// for its runs first: a list grown one run at a time keeps room for four.
fn fill(planned: &mut [Planned], flow: &mut Flow, target: &[Target]) {
    let mut counts: HashMap<(u32, u32, u32), usize> = HashMap::new();
    for t in target.iter().flatten() {
        *counts.entry(*t).or_default() += 1;
    }
    for ((t, r, c), n) in counts {
        planned[t as usize].rows[r as usize][c as usize]
            .runs
            .reserve_exact(n);
    }
    let stay = target.iter().filter(|t| t.is_none()).count()
        + flow.runs.len().saturating_sub(target.len());
    flow.take_each(stay, |i, r| match target.get(i).copied().flatten() {
        Some((t, row, cell)) => {
            planned[t as usize].rows[row as usize][cell as usize]
                .runs
                .push(r);
            None
        }
        None => Some(r),
    });
}

/// The largest text size a caption search assumes for a table, in points.
const MAX_CAPTION_SIZE: f32 = 36.0;

/// Tables without tags one page may have; past them the text stays paragraphs, with a loss.
const MAX_TABLES: usize = 5_000;

/// The loss for a page that has more tables than it may.
fn too_many_tables(page: u32) -> String {
    format!(
        "tables on page {page} past the first {MAX_TABLES} are not rebuilt: the page has more \
         tables than it may; their text stays as paragraphs"
    )
}

/// Grids and regions one page may have looked at between two looks at the clock.
const CHECK_EVERY: usize = 64;

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
