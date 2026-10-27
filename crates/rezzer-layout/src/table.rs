// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Mapping of one table region: the cells, merges, lines and row kinds of a table block.

use std::collections::{BTreeMap, HashMap};

use rezzer_model::{
    Anchor, Block, Cell, Certainty, Evidence, Inline, LayoutKind, LossReason, RowKind, RowNote,
    Severity, TableLayout, table_key,
};
use sheetlayout::{Kind, Rect, Region, column_letters};

use super::{ColumnNames, Ctx, MAX_CELLS, MAX_COLS, NameCaption, a1, kind_word, text_inlines};

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

fn kind_of(k: Kind) -> LayoutKind {
    match k {
        Kind::Table => LayoutKind::Table,
        Kind::KeyValue => LayoutKind::KeyValue,
        Kind::Text => LayoutKind::Text,
        Kind::Title => LayoutKind::Title,
        Kind::Grid => LayoutKind::Grid,
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

pub(super) fn map_region(
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
