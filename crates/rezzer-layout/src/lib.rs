// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Maps the regions `sheetlayout` found in a sheet into model blocks: per region its
//! headings, caption and note paragraphs and one table with its [`TableLayout`]. The sheet
//! reader and the CSV reader share it.
//!
//! Every non-empty cell ends up in exactly one block or in a loss: a caption row cell is a
//! heading or a caption paragraph, a note row cell a note paragraph, every other cell of a
//! region a table cell with its `Sheet!A1` anchor. A cell under a merge that is not the
//! merge's top-left cell is not shown and gets a loss.

use std::collections::{BTreeMap, HashMap};

use rezzer_limits::Budget;
use rezzer_model::{
    Anchor, Block, Cell, CellArea, Certainty, Evidence, Inline, LayoutKind, Loss, LossReason,
    RowKind, RowNote, Severity, TableLayout, table_key,
};
use sheetlayout::{IssueKind, Kind, Layout, Rect, Region, column_letters};

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

/// `row 4` or `rows 12 to 40`, 1-based.
fn row_words(first: u32, last: u32) -> String {
    let (a, b) = (u64::from(first) + 1, u64::from(last) + 1);
    if a == b {
        format!("row {a}")
    } else {
        format!("rows {a} to {b}")
    }
}

fn col_words(first: u32, last: u32) -> String {
    if first == last {
        format!("column {}", column_letters(first))
    } else {
        format!(
            "columns {} to {}",
            column_letters(first),
            column_letters(last)
        )
    }
}

fn a1(row: u32, col: u32) -> String {
    format!("{}{}", column_letters(col), u64::from(row) + 1)
}

fn kind_of(k: Kind) -> LayoutKind {
    match k {
        Kind::Table => LayoutKind::Table,
        Kind::KeyValue => LayoutKind::KeyValue,
        Kind::Text => LayoutKind::Text,
        Kind::Title => LayoutKind::Title,
        Kind::Grid => LayoutKind::Grid,
    }
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

fn certainty_of(c: sheetlayout::Certainty) -> Certainty {
    match c {
        sheetlayout::Certainty::Exact => Certainty::Exact,
        sheetlayout::Certainty::Rule => Certainty::Rule,
        sheetlayout::Certainty::Unsure => Certainty::Unsure,
    }
}

fn evidence_of(e: &sheetlayout::Evidence) -> Evidence {
    use sheetlayout::Evidence as E;
    match e {
        E::ListObject { name } => Evidence::ListObject { name: name.clone() },
        E::DatabaseRange { name } => Evidence::DatabaseRange { name: name.clone() },
        E::Pivot { name } => Evidence::Pivot { name: name.clone() },
        E::AutoFilter => Evidence::Autofilter,
        E::PrintTitles => Evidence::PrintTitles,
        E::FrozenPanes => Evidence::FrozenPanes,
        E::DefinedName { name } => Evidence::DefinedName { name: name.clone() },
        E::Rule { id, detail } => Evidence::Rule {
            id: (*id).to_string(),
            detail: detail.clone(),
        },
    }
}

fn row_kind(k: sheetlayout::RowKind) -> RowKind {
    match k {
        sheetlayout::RowKind::Data => RowKind::Data,
        sheetlayout::RowKind::Subtotal => RowKind::Subtotal,
        sheetlayout::RowKind::Total => RowKind::Total,
        sheetlayout::RowKind::Group => RowKind::Group,
        sheetlayout::RowKind::Blank => RowKind::Blank,
    }
}

/// What a row of a region is outside its table.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Line {
    Caption,
    CaptionHeading,
    Heading(u8),
    AboveHeader,
    Note,
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

fn issue_losses(ctx: &mut Ctx, layout: &Layout, skipped: &[Rect]) {
    for issue in &layout.issues {
        if issue
            .rect
            .is_some_and(|r| skipped.iter().any(|s| s.covers(&r)))
        {
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

/// The rows of a region outside its table, with what each one is.
fn lines_of(region: &Region) -> HashMap<u32, Line> {
    let mut lines = HashMap::new();
    for &r in &region.notes {
        lines.insert(r, Line::Note);
    }
    for &r in &region.above_header {
        lines.insert(r, Line::AboveHeader);
    }
    for &r in &region.captions {
        lines.insert(r, Line::Caption);
    }
    if let Some(r) = region.caption_heading {
        lines.insert(r, Line::CaptionHeading);
    }
    if let Some((r, depth)) = region.heading {
        lines.insert(r, Line::Heading(depth));
    }
    lines
}

/// The columns and rows the table of a region spans: its body, else its data area, else the
/// whole region; from the first header row down.
fn table_span(region: &Region) -> Option<Rect> {
    let base = region.body.or(region.data)?;
    let top = region
        .header_rows
        .first()
        .map_or(base.top, |&h| h.min(base.top));
    let bottom = region
        .data
        .map_or(base.bottom, |d| d.bottom.max(base.bottom));
    Some(Rect {
        top,
        bottom,
        ..base
    })
}

fn map_region(
    ctx: &mut Ctx,
    region: &Region,
    held: &[usize],
    merges: &[Rect],
    names: ColumnNames,
    caption_names: &[NameCaption],
    only: bool,
) {
    let sheet_q = ctx.sheet_q;
    let lines = lines_of(region);
    let span = if region.kind == Kind::Title {
        None
    } else {
        table_span(region)
    };
    // Sort the held cells into the lines, the table and the rest.
    let mut by_line: Vec<(Line, usize)> = Vec::new();
    let mut in_table: Vec<usize> = Vec::new();
    let mut rest: Vec<usize> = Vec::new();
    for &k in held {
        let c = &ctx.cells[k];
        if let Some(&line) = lines.get(&c.row) {
            by_line.push((line, k));
        } else if span.is_some_and(|s| {
            s.top <= c.row
                && c.row <= s.bottom
                && s.left <= c.col
                && c.col <= s.right
                && (region.header_rows.contains(&c.row)
                    || region.data.is_none_or(|d| d.top <= c.row))
        }) {
            in_table.push(k);
        } else {
            rest.push(k);
        }
    }

    // Headings.
    for &(line, k) in &by_line {
        if let Line::Heading(depth) = line {
            let c = &ctx.cells[k];
            let level = (3 + depth).min(6);
            let block = Block::Heading {
                level,
                inlines: text_inlines(&c.text),
                anchor: Some(ctx.anchor(c.row, c.col)),
            };
            ctx.out.blocks.push(block);
            break;
        }
    }
    // The region heading sits one level below the section heading the region has, so a
    // section piece's heading is never shallower than its section's.
    let region_level = region.heading.map_or(3, |(_, depth)| (4 + depth).min(6));
    let heading_cell = by_line
        .iter()
        .find(|(l, _)| *l == Line::CaptionHeading)
        .map(|&(_, k)| k);
    // A sheet's only table without caption rows gets no heading of its own (R8).
    let bare = only && region.kind == Kind::Table && region.captions.is_empty();
    // Without caption rows, a defined name stands as the region's heading, and the name of
    // an Excel table instead of a generated `Table <range>`: the file names the range. Its
    // anchor is the name's.
    let name_caption = if region.captions.is_empty() {
        let defined = region.evidence.iter().find_map(|e| match e {
            sheetlayout::Evidence::DefinedName { name } => {
                caption_names.iter().find(|c| &c.name == name).cloned()
            }
            _ => None,
        });
        defined.or_else(|| {
            if bare {
                return None;
            }
            region.evidence.iter().find_map(|e| match e {
                sheetlayout::Evidence::ListObject { name } => Some(NameCaption {
                    name: name.clone(),
                    anchor: format!("table:{name}"),
                }),
                _ => None,
            })
        })
    } else {
        None
    };
    match (heading_cell, &name_caption) {
        (Some(k), _) => {
            let c = &ctx.cells[k];
            let block = Block::Heading {
                level: region_level,
                inlines: text_inlines(&c.text),
                anchor: Some(ctx.anchor(c.row, c.col)),
            };
            ctx.out.blocks.push(block);
        }
        (None, Some(name)) => ctx.out.blocks.push(Block::Heading {
            level: region_level,
            inlines: vec![Inline::plain(name.name.clone())],
            anchor: Some(Anchor(name.anchor.clone())),
        }),
        (None, None) => {
            if !bare {
                ctx.out.blocks.push(Block::Heading {
                    level: region_level,
                    inlines: vec![Inline::plain(format!(
                        "{} {}",
                        kind_word(region.kind),
                        region.rect.a1()
                    ))],
                    anchor: None,
                });
            }
        }
    }
    // The heading row's other cells, caption rows and the rows above a header.
    let mut heading_seen = false;
    let mut first_heading = false;
    for &(line, k) in &by_line {
        match line {
            Line::Heading(_) if !heading_seen => heading_seen = true,
            Line::CaptionHeading if Some(k) == heading_cell && !first_heading => {
                first_heading = true
            }
            Line::Heading(_) | Line::Caption | Line::CaptionHeading | Line::AboveHeader => {
                ctx.paragraph(k, "caption")
            }
            Line::Note => {}
        }
    }
    let mut caption_text: Vec<&str> = by_line
        .iter()
        .filter(|(l, _)| matches!(l, Line::Caption | Line::CaptionHeading))
        .map(|&(_, k)| ctx.cells[k].text.as_str())
        .collect();
    if caption_text.is_empty()
        && let Some(n) = &name_caption
    {
        caption_text.push(&n.name);
    }
    let caption =
        (!caption_text.is_empty() && region.kind != Kind::Title).then(|| caption_text.join(" "));

    let table = build_table(ctx, region, span, &lines, &in_table, merges, names, caption);
    ctx.out.blocks.push(table);
    for &k in &rest {
        ctx.paragraph(k, "cell");
    }
    for &(line, k) in &by_line {
        if line == Line::Note {
            ctx.paragraph(k, "note");
        }
    }
    if region.certainty == sheetlayout::Certainty::Unsure && !region.rest {
        let what = if region.unsure.is_empty() {
            "region unsure".to_string()
        } else {
            region.unsure.join("; ")
        };
        ctx.loss(
            format!("{sheet_q}!{}", region.rect.a1()),
            what,
            LossReason::Uncertain,
            Severity::Info,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn build_table(
    ctx: &mut Ctx,
    region: &Region,
    span: Option<Rect>,
    lines: &HashMap<u32, Line>,
    in_table: &[usize],
    merges: &[Rect],
    names: ColumnNames,
    caption: Option<String>,
) -> Block {
    let sheet_q = ctx.sheet_q;
    let range = region.rect.a1();
    let what = format!("{} {range}", kind_word(region.kind));
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut shown_cols = 0u32;
    if let Some(s) = span {
        let width = u32::try_from(s.width()).unwrap_or(u32::MAX);
        shown_cols = width.min(ctx.caps.max_cols.max(1));
        if shown_cols < width {
            ctx.loss(
                format!("{sheet_q}!{range}"),
                format!(
                    "{} of {what} not shown (at most {MAX_COLS} columns per region); their formulas are still listed",
                    col_words(s.left + shown_cols, s.right)
                ),
                LossReason::Limit,
                Severity::Warning,
            );
        }
        // The sheet rows of the table: header rows, then the data rows, without the rows
        // that are written as lines. Bounded by the cells left: a row costs the positions it
        // shows (at least 1); positions under a merged area other than its first cell in the
        // table are not shown and cost nothing.
        let shown = Rect {
            right: s.left + (shown_cols - 1),
            ..s
        };
        let clipped = clip_merges(merges, shown);
        let mut sweep = Sweep::new(&clipped);
        let mut spent = 0u64;
        let mut table_rows: Vec<u32> = Vec::new();
        let mut cut: Option<u32> = None;
        let mut row = s.top;
        loop {
            sweep.advance(row);
            if !lines.contains_key(&row)
                && (region.header_rows.contains(&row) || region.data.is_none_or(|d| d.top <= row))
            {
                let cost = sweep.table_row(shown_cols).max(1);
                if spent + cost > ctx.caps.cells_left {
                    cut = Some(row);
                    break;
                }
                spent += cost;
                table_rows.push(row);
            } else {
                sweep.line_row();
            }
            if row >= s.bottom {
                break;
            }
            row += 1;
        }
        if let Some(first) = cut {
            ctx.loss(
                format!("{sheet_q}!{range}"),
                format!(
                    "{} of {what} not shown (at most {MAX_CELLS} cells per sheet); their formulas are still listed",
                    row_words(first, s.bottom)
                ),
                LossReason::Limit,
                Severity::Warning,
            );
        }
        ctx.caps.cells_left = ctx.caps.cells_left.saturating_sub(spent);
        rows = table_cells(ctx, shown, &table_rows, in_table, &clipped, &what);
    }
    let header_rows: Vec<u32> = region.header_rows.iter().map(|r| r + 1).collect();
    let band = region.header_rows.len();
    let mut header_paths = if region.kind == Kind::Table && band == 0 && !region.hidden_header {
        let left = region.body.or(span).map_or(region.rect.left, |b| b.left);
        let width = span.map_or(0, |s| u32::try_from(s.width()).unwrap_or(u32::MAX));
        (left..left.saturating_add(width))
            .map(|c| match names {
                ColumnNames::Letters => column_letters(c),
                ColumnNames::Numbers => (u64::from(c) + 1).to_string(),
            })
            .collect()
    } else {
        region.header_paths.clone()
    };
    if span.is_some() {
        header_paths.truncate(shown_cols as usize);
    }
    let found_by: Vec<Evidence> = region.evidence.iter().map(evidence_of).collect();
    let top_left = a1(region.rect.top, region.rect.left);
    let layout = TableLayout {
        kind: kind_of(region.kind),
        key: table_key(sheet_q, &found_by, &top_left),
        range: range.clone(),
        data: region.data.map(|d| d.a1()),
        header_rows,
        header_paths,
        label_cols: region.label_cols,
        // Only total columns the table shows.
        total_columns: region
            .total_cols
            .iter()
            .filter(|&&c| span.is_some_and(|s| s.left <= c && c < s.left + shown_cols))
            .map(|c| c + 1)
            .collect(),
        certainty: certainty_of(region.certainty),
        found_by,
        unsure: region.unsure.clone(),
        rows: region
            .rows
            .iter()
            .map(|n| RowNote {
                row: n.row + 1,
                kind: row_kind(n.kind),
                level: n.level,
                parent: n.parent.map(|p| p + 1),
                why: n.why.clone(),
            })
            .collect(),
        hidden_header: region.hidden_header,
    };
    Block::Table {
        caption,
        header_rows: band.min(rows.len()),
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: Some(Anchor(format!("{sheet_q}!{range}"))),
        layout: Some(Box::new(layout)),
    }
}

/// The merged areas that reach `shown`, clipped to it and sorted by top row.
fn clip_merges(merges: &[Rect], shown: Rect) -> Vec<Rect> {
    let mut out: Vec<Rect> = merges
        .iter()
        .filter_map(|m| m.intersection(&shown))
        .collect();
    out.sort_by_key(|m| (m.top, m.left));
    out
}

/// A sweep down the rows of a table over its clipped merged areas (none overlap). A merged
/// area shows one cell, with spans, at the first table row it reaches after the start or
/// after a row written as a line; the other positions it covers in the table are not shown.
/// Works on the areas, never on the positions they cover.
struct Sweep<'a> {
    merges: &'a [Rect],
    next: usize,
    /// Active areas by (bottom row, index), with the table row count when each was added.
    active: BTreeMap<(u32, usize), u64>,
    width: u64,
    /// Table rows seen so far, and their count at the last row written as a line.
    rows: u64,
    last_break: u64,
    /// Active areas that already show their cell since the last break.
    running: u64,
}

impl<'a> Sweep<'a> {
    fn new(merges: &'a [Rect]) -> Sweep<'a> {
        Sweep {
            merges,
            next: 0,
            active: BTreeMap::new(),
            width: 0,
            rows: 0,
            last_break: 0,
            running: 0,
        }
    }

    /// Takes in the areas that start at or above `row` and drops those that end above it.
    fn advance(&mut self, row: u32) {
        while let Some(m) = self.merges.get(self.next).filter(|m| m.top <= row) {
            if m.bottom >= row {
                self.active.insert((m.bottom, self.next), self.rows);
                self.width += m.width();
            }
            self.next += 1;
        }
        while let Some((&(bottom, k), &added)) = self.active.first_key_value() {
            if bottom >= row {
                break;
            }
            self.active.pop_first();
            self.width -= self.merges[k].width();
            if self.rows > added.max(self.last_break) {
                self.running -= 1;
            }
        }
    }

    /// The positions a table row shows: the columns no area covers, plus one cell per area
    /// that starts showing here.
    fn table_row(&mut self, cols: u32) -> u64 {
        let starting = self.active.len() as u64 - self.running;
        self.rows += 1;
        self.running = self.active.len() as u64;
        u64::from(cols) - self.width + starting
    }

    /// A row written as a line: the areas show a new cell at the next table row.
    fn line_row(&mut self) {
        self.last_break = self.rows;
        self.running = 0;
    }
}

/// The cells of the table rows `table_rows` over the columns of `shown`: one cell per
/// position, each piece of a merged area that runs down consecutive table rows as one cell
/// with spans (its first table row), the positions it covers left out. `merges` are
/// clipped to `shown` and sorted by top row. Works per area and per row, never on a grid.
fn table_cells(
    ctx: &mut Ctx,
    shown: Rect,
    table_rows: &[u32],
    in_table: &[usize],
    merges: &[Rect],
    what: &str,
) -> Vec<Vec<Cell>> {
    let h = table_rows.len();
    // The last index of the run of consecutive table rows each index is in.
    let mut run_end = vec![0usize; h];
    for i in (0..h).rev() {
        run_end[i] = if i + 1 < h && table_rows[i + 1] == table_rows[i] + 1 {
            run_end[i + 1]
        } else {
            i
        };
    }
    // Pieces of the areas: (first index, last index, left column, width).
    let mut pieces: Vec<(usize, usize, u32, u32)> = Vec::new();
    for m in merges {
        let mut i = table_rows.partition_point(|&r| r < m.top);
        let below = table_rows.partition_point(|&r| r <= m.bottom);
        let cs = u32::try_from(m.width()).unwrap_or(u32::MAX);
        while i < below {
            let last = run_end[i].min(below - 1);
            if last > i || cs > 1 {
                pieces.push((i, last, m.left, cs));
            }
            i = last + 1;
        }
    }
    pieces.sort_unstable();
    let mut texts: HashMap<(u32, u32), usize> = HashMap::with_capacity(in_table.len());
    for &k in in_table {
        let c = &ctx.cells[k];
        texts.insert((c.row, c.col), k);
    }
    let mut rows = Vec::with_capacity(h);
    // Pieces running through the current row below their first one, by left column:
    // (width, last index).
    let mut through: BTreeMap<u32, (u32, usize)> = BTreeMap::new();
    let mut next = 0usize;
    for (i, &row) in table_rows.iter().enumerate() {
        through.retain(|_, &mut (_, last)| last >= i);
        while let Some(&(first, last, left, cs)) = pieces.get(next).filter(|p| p.0 < i) {
            if last >= i && first < i {
                through.insert(left, (cs, last));
            }
            next += 1;
        }
        let here: HashMap<u32, (u32, u32)> = pieces[next..]
            .iter()
            .take_while(|p| p.0 == i)
            .map(|&(first, last, left, cs)| (left, ((last - first + 1) as u32, cs)))
            .collect();
        let mut out = Vec::new();
        let mut padding = 0u64;
        let mut col = shown.left;
        while col <= shown.right {
            if let Some(&(cs, _)) = through.get(&col) {
                col = col.saturating_add(cs);
                continue;
            }
            let found = texts.remove(&(row, col));
            let inlines = match found {
                Some(k) => text_inlines(&ctx.cells[k].text),
                None => {
                    padding += 1;
                    Vec::new()
                }
            };
            let mut cell = Cell::new(inlines);
            cell.anchor = Some(ctx.anchor(row, col));
            let step = match here.get(&col) {
                Some(&(rs, cs)) => {
                    cell.row_span = rs;
                    cell.col_span = cs;
                    cs
                }
                None => 1,
            };
            out.push(cell);
            if col == u32::MAX {
                break;
            }
            col = col.saturating_add(step);
        }
        if ctx.budget.add_elements(padding).is_err() {
            ctx.out.elements_out = true;
            ctx.loss(
                format!("{}!{}", ctx.sheet_q, shown.a1()),
                format!(
                    "element limit reached, the rows of {what} from row {} on not shown",
                    u64::from(row) + 1
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            return rows;
        }
        rows.push(out);
    }
    // Values left under a merged area inside the shown rows and columns.
    let shown_rows: std::collections::HashSet<u32> = table_rows.iter().copied().collect();
    let mut leftover: Vec<usize> = texts
        .into_values()
        .filter(|&k| {
            let c = &ctx.cells[k];
            shown_rows.contains(&c.row) && shown.left <= c.col && c.col <= shown.right
        })
        .collect();
    leftover.sort_unstable();
    // Cells past the shown columns or rows, or under a merge clipped to the table, are said
    // by the cap losses; any other cell left is written as a line so none is lost.
    for k in leftover {
        ctx.paragraph(k, "cell");
    }
    rows
}
