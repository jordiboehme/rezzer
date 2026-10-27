// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Maps the regions `sheetlayout` found in a sheet into model blocks: per region its
//! headings, caption and note paragraphs and one table with its
//! [`TableLayout`](rezzer_model::TableLayout). The sheet reader and the CSV reader share it.
//!
//! Every non-empty cell ends up in exactly one block or in a loss: a caption row cell is a
//! heading or a caption paragraph, a note row cell a note paragraph, every other cell of a
//! region a table cell with its `Sheet!A1` anchor. A cell under a merge that is not the
//! merge's top-left cell is not shown and gets a loss.

mod table;

use std::collections::{BTreeMap, HashSet};

use rezzer_limits::Budget;
use rezzer_model::{Anchor, Block, CellArea, Inline, Loss, LossReason, Severity};
use sheetlayout::{IssueKind, Kind, Layout, Rect, column_letters};

use table::map_region;

/// Cells the tables of one sheet show, over all its regions.
pub const MAX_CELLS: u64 = 200_000;

/// Columns one region shows.
pub const MAX_COLS: u32 = 1_000;

/// Covered cells listed one by one per sheet; the rest are counted in one loss.
const LISTED_COVERED: usize = 20;

/// How a table without a header row names its columns (decision 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnNames {
    /// Sheet column letters: `A`, `B`, `C`.
    Letters,
    /// Column numbers from 1, for CSV.
    Numbers,
}

/// What the tables of one sheet may still show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Cells (positions, empty ones included) left for the sheet.
    pub cells_left: u64,
    /// Columns each region shows.
    pub max_cols: u32,
}

impl Default for Caps {
    /// The caps of one sheet: [`MAX_CELLS`] and [`MAX_COLS`].
    fn default() -> Caps {
        Caps {
            cells_left: MAX_CELLS,
            max_cols: MAX_COLS,
        }
    }
}

/// The shown text of one cell. A list of them is sorted by row, then column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellText {
    pub row: u32,
    pub col: u32,
    pub text: String,
}

/// A defined name that may stand as a region's caption: not hidden and not written by the
/// application for itself. `anchor` is the name's anchor (`name:Prices`, or
/// `name:Sheet1!Prices` for a name scoped to a sheet), so its heading is anchored like any
/// author text. Only the region headings Rezzer generates (`Table B3:D5`) have no anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCaption {
    pub name: String,
    pub anchor: String,
}

/// The blocks of a sheet's regions and what they could not show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mapped {
    pub blocks: Vec<Block>,
    pub losses: Vec<Loss>,
    /// Something was not shown at a limit: the read is partial.
    pub partial: bool,
    /// The element budget ran out while the tables were built.
    pub elements_out: bool,
}

/// The `sheetlayout` limits for a read: the defaults, with the path text bounded by the
/// text limit of one run.
pub fn limits(budget: &Budget) -> sheetlayout::Limits {
    let d = sheetlayout::Limits::default();
    let text = usize::try_from(budget.limits().max_text_bytes).unwrap_or(usize::MAX);
    sheetlayout::Limits {
        max_path_bytes: d.max_path_bytes.min(text),
        ..d
    }
}

/// Inlines of a text with line breaks: each line break becomes [`Inline::LineBreak`].
pub fn text_inlines(text: &str) -> Vec<Inline> {
    if !text.contains(['\n', '\r']) {
        return vec![Inline::plain(text)];
    }
    let mut out = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push(Inline::LineBreak);
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !line.is_empty() {
            out.push(Inline::plain(line));
        }
    }
    out
}

/// The merged areas a sheet shows: sorted by their top-left cell, one-cell areas left out,
/// and an area that overlaps one before it ignored. Gives the kept areas and the number
/// ignored. Works on the rectangles only, never on the cells they cover.
pub fn clean_merges(merges: &[Rect]) -> (Vec<Rect>, u64) {
    let mut sorted: Vec<Rect> = merges
        .iter()
        .map(|m| m.normalized())
        .filter(|m| m.height() > 1 || m.width() > 1)
        .collect();
    sorted.sort_by_key(|m| (m.top, m.left));
    let mut kept = Vec::with_capacity(sorted.len());
    let mut ignored = 0u64;
    // Kept areas that reach the current top row, by left column. They do not overlap, so
    // the one with the largest left column at or before a column is the only one that can
    // hold it.
    let mut active: BTreeMap<u32, Rect> = BTreeMap::new();
    for m in sorted {
        let mut overlaps = false;
        while let Some((&left, &a)) = active.range(..=m.right).next_back() {
            if a.bottom < m.top {
                active.remove(&left);
                continue;
            }
            overlaps = a.right >= m.left;
            break;
        }
        if overlaps {
            ignored += 1;
            continue;
        }
        active.insert(m.left, m);
        kept.push(m);
    }
    (kept, ignored)
}

/// For each position (sorted by row, then column), whether a merged area of `merges` (kept
/// by [`clean_merges`], so none overlap) covers it without being its top-left cell.
pub fn covered(at: &[(u32, u32)], merges: &[Rect]) -> Vec<bool> {
    let mut sorted: Vec<&Rect> = merges.iter().collect();
    sorted.sort_by_key(|m| (m.top, m.left));
    let mut next = 0usize;
    let mut active: BTreeMap<u32, &Rect> = BTreeMap::new();
    at.iter()
        .map(|&(row, col)| {
            while let Some(m) = sorted.get(next).filter(|m| m.top <= row) {
                active.insert(m.left, m);
                next += 1;
            }
            loop {
                let Some((&left, m)) = active.range(..=col).next_back() else {
                    return false;
                };
                if m.bottom < row {
                    active.remove(&left);
                    continue;
                }
                return col <= m.right && (row, col) != (m.top, m.left);
            }
        })
        .collect()
}

/// For each point, the index of the rectangle that holds it. Rectangles overlap only when
/// facts of a damaged file do; then the smallest one that holds a point has it, as the
/// detector decides (see [`rezzer_model::locate_cells`]).
fn locate(rects: &[Rect], points: &[(u32, u32)]) -> Vec<Option<usize>> {
    let areas: Vec<CellArea> = rects
        .iter()
        .map(|r| CellArea {
            top: r.top,
            left: r.left,
            bottom: r.bottom,
            right: r.right,
        })
        .collect();
    rezzer_model::locate_cells(&areas, points)
}

fn a1(row: u32, col: u32) -> String {
    format!("{}{}", column_letters(col), u64::from(row) + 1)
}

/// The word of a region in a generated heading and in losses.
fn kind_word(k: Kind) -> &'static str {
    match k {
        Kind::Table => "Table",
        Kind::KeyValue => "Label and value pairs",
        Kind::Text => "Text",
        Kind::Title => "Title",
        Kind::Grid => "Grid",
    }
}

/// The work of one sheet.
struct Ctx<'a> {
    sheet_q: &'a str,
    cells: &'a [CellText],
    budget: &'a Budget,
    caps: &'a mut Caps,
    out: Mapped,
}

impl Ctx<'_> {
    fn anchor(&self, row: u32, col: u32) -> Anchor {
        Anchor(format!("{}!{}", self.sheet_q, a1(row, col)))
    }

    fn loss(&mut self, location: String, what: String, why: LossReason, severity: Severity) {
        if why == LossReason::Limit {
            self.out.partial = true;
        }
        self.out.losses.push(Loss {
            location,
            what,
            why,
            severity,
        });
    }

    fn paragraph(&mut self, k: usize, role: &str) {
        let c = &self.cells[k];
        let block = Block::Paragraph {
            inlines: text_inlines(&c.text),
            role: Some(role.to_string()),
            anchor: Some(self.anchor(c.row, c.col)),
        };
        self.out.blocks.push(block);
    }
}

/// Maps the regions of one sheet into blocks, in reading order. `sheet_q` is the sheet name
/// as a reference writes it; `cells` are the shown texts of the sheet's cells, sorted;
/// `merges` its merged areas (kept by [`clean_merges`]); `names` how a table without a header
/// row names its columns; `caption_names` the defined names that may stand as a region's
/// caption (not hidden, not written by the application for itself). `caps` bounds what the
/// tables show and is shared by the sheet's regions.
///
/// A region that holds no value is left out. Per region: a section heading (level 4) when the
/// region has one; the region heading
/// (level 3, one below the section heading when there is one: its caption heading row,
/// else a defined name caption, else `<Kind> <range>`,
/// left out for a sheet whose only region is a table without a caption); the other caption
/// rows as `caption` paragraphs; the table; the note rows as `note` paragraphs.
#[allow(clippy::too_many_arguments)]
pub fn blocks(
    sheet_q: &str,
    layout: &Layout,
    cells: &[CellText],
    merges: &[Rect],
    names: ColumnNames,
    caption_names: &[NameCaption],
    caps: &mut Caps,
    budget: &Budget,
) -> Mapped {
    let mut ctx = Ctx {
        sheet_q,
        cells,
        budget,
        caps,
        out: Mapped::default(),
    };
    let points: Vec<(u32, u32)> = cells.iter().map(|c| (c.row, c.col)).collect();
    let hidden = covered(&points, merges);
    let rects: Vec<Rect> = layout.regions.iter().map(|r| r.rect).collect();
    let owner = locate(&rects, &points);
    let mut held: Vec<Vec<usize>> = vec![Vec::new(); rects.len()];
    let mut loose: Vec<usize> = Vec::new();
    let mut covered_cells: Vec<usize> = Vec::new();
    for (k, c) in cells.iter().enumerate() {
        if c.text.is_empty() {
            continue;
        }
        if hidden[k] {
            covered_cells.push(k);
            continue;
        }
        match owner[k] {
            Some(i) => held[i].push(k),
            None => loose.push(k),
        }
    }
    // Merges by the region that holds their top-left cell.
    let tops: Vec<(u32, u32)> = merges.iter().map(|m| (m.top, m.left)).collect();
    let mut order: Vec<usize> = (0..merges.len()).collect();
    order.sort_by_key(|&i| tops[i]);
    let sorted_tops: Vec<(u32, u32)> = order.iter().map(|&i| tops[i]).collect();
    let mut region_merges: Vec<Vec<Rect>> = vec![Vec::new(); rects.len()];
    for (j, o) in locate(&rects, &sorted_tops).into_iter().enumerate() {
        if let Some(i) = o {
            region_merges[i].push(merges[order[j]]);
        }
    }

    // A region that holds no value (merged areas without a value, formulas without a
    // result) has nothing to show and is left out: no cell is lost by it.
    let only = held.iter().filter(|h| !h.is_empty()).count() == 1;
    // Cells outside every region go in reading order among the regions (rare: the detector
    // places every cell). Reading order is row, then column: a cell comes before a region
    // when its (row, column) comes before the region's first row and left column, so a cell
    // beside a tall region, below its first row, follows that region.
    let mut loose = loose.into_iter().peekable();
    for (i, region) in layout.regions.iter().enumerate() {
        let at = (region.rect.top, region.rect.left);
        while let Some(k) = loose.next_if(|&k| (cells[k].row, cells[k].col) < at) {
            ctx.paragraph(k, "cell");
        }
        if held[i].is_empty() {
            continue;
        }
        if ctx.out.elements_out {
            ctx.loss(
                format!("{sheet_q}!{}", region.rect.a1()),
                format!(
                    "element limit reached, the regions from {} {} on not shown",
                    kind_word(region.kind),
                    region.rect.a1()
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        map_region(
            &mut ctx,
            region,
            &held[i],
            &region_merges[i],
            names,
            caption_names,
            only,
        );
    }
    for k in loose {
        ctx.paragraph(k, "cell");
    }
    covered_losses(&mut ctx, &covered_cells, merges);
    // Issues about a region left out for holding no value are not said.
    let skipped: Vec<Rect> = layout
        .regions
        .iter()
        .zip(&held)
        .filter(|(_, h)| h.is_empty())
        .map(|(r, _)| r.rect)
        .collect();
    issue_losses(&mut ctx, layout, &skipped);
    ctx.out
}

/// The merged area that holds a covered cell.
fn merge_of(merges: &[Rect], row: u32, col: u32) -> Option<&Rect> {
    merges
        .iter()
        .find(|m| m.top <= row && row <= m.bottom && m.left <= col && col <= m.right)
}

fn covered_losses(ctx: &mut Ctx, covered_cells: &[usize], merges: &[Rect]) {
    for &k in covered_cells.iter().take(LISTED_COVERED) {
        let c = &ctx.cells[k];
        let (row, col) = (c.row, c.col);
        let area = merge_of(merges, row, col).map(Rect::a1).unwrap_or_default();
        let what = format!(
            "cell {} holds {:?} under the merged area {area}, whose top-left cell is shown; the cell is not shown",
            a1(row, col),
            c.text
        );
        let location = ctx.anchor(row, col).0;
        ctx.loss(location, what, LossReason::Unsupported, Severity::Info);
    }
    if covered_cells.len() > LISTED_COVERED {
        let more = covered_cells.len() - LISTED_COVERED;
        let location = ctx.sheet_q.to_string();
        ctx.loss(
            location,
            format!("{more} more cells under merged areas hold a value that is not shown"),
            LossReason::Unsupported,
            Severity::Info,
        );
    }
}

/// The losses of the detector's issues, but those about a place inside a region left out.
/// An issue is inside such a region when its rectangle is the region's, or when the smallest
/// left-out region that holds its first cell covers it. Both lookups cost about the issues
/// plus the regions, never the issues times the regions.
fn issue_losses(ctx: &mut Ctx, layout: &Layout, skipped: &[Rect]) {
    let same: HashSet<Rect> = skipped.iter().copied().collect();
    let firsts: Vec<(u32, u32)> = layout
        .issues
        .iter()
        .filter_map(|i| i.rect)
        .map(|r| (r.top, r.left))
        .collect();
    let mut holders = locate(skipped, &firsts).into_iter();
    for issue in &layout.issues {
        if issue.rect.is_some_and(|r| {
            let holder = holders.next().flatten();
            same.contains(&r) || holder.is_some_and(|k| skipped[k].covers(&r))
        }) {
            continue;
        }
        let location = match issue.rect {
            Some(r) => format!("{}!{}", ctx.sheet_q, r.a1()),
            None => ctx.sheet_q.to_string(),
        };
        let (what, why, severity) = match issue.kind {
            IssueKind::Limit | IssueKind::Stopped => {
                (issue.what.clone(), LossReason::Limit, Severity::Warning)
            }
            IssueKind::Overlap => (issue.what.clone(), LossReason::Uncertain, Severity::Info),
            IssueKind::Coarse => (
                "the region may hold more than one table".to_string(),
                LossReason::Uncertain,
                Severity::Info,
            ),
        };
        ctx.loss(location, what, why, severity);
    }
}
