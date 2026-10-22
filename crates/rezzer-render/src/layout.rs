// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sheet regions with a layout (tables, grids, label and value pairs, text blocks, titles)
//! and the blocks around them in a sheet section.
//!
//! - A `table` region: the layout line, then a Markdown table whose header is the header
//!   paths (`Column <letter>` for an empty one), with a `Parent` column when rows have a
//!   parent; under it the merge note and one line of row kinds
//! - A `grid` region: the layout line, then the raw cells with the sheet column letters as
//!   the header and the sheet row numbers as the first column
//! - `key_value` and `text` regions: a lead line with the range, then one list item per pair
//!   or cell, each with the address of its cell
//! - A `title` region: nothing; its heading and caption lines are blocks of their own
//!
//! In a sheet section, caption and note lines end with the address of their cell, captions
//! of one sheet row are one line joined by ` | `, and a heading over 120 characters is cut
//! with `...` and followed by its full text. Positions come from the cells' `Sheet!A1`
//! anchors, never from places in the table.

use std::collections::HashMap;

use rezzer_model::{
    Align, Block, Cell, Evidence, Inline, LayoutKind, RowKind, RowNote, TableLayout, a1_position,
    column_letter, plain_text,
};

use crate::blocks::{
    self, Ctx, Grid, MAX_COLUMNS, MAX_DEPTH, cell_text, heading_line, line_text, list_block,
    paragraph, pipe_table,
};

/// A heading in a sheet is cut after this many characters.
const MAX_HEADING_CHARS: usize = 120;

/// The local address of a `Sheet!A1` anchor (`A1`); the sheet name may hold a `!` itself.
/// `None` for an anchor that names no cell, such as a defined name's `name:'Sheet'!Local`.
fn address(anchor: Option<&rezzer_model::Anchor>) -> Option<&str> {
    anchor
        .and_then(|a| a.0.rsplit_once('!'))
        .map(|(_, at)| at)
        .filter(|at| a1_position(at).is_some())
}

/// The 0-based sheet column of a cell, from its anchor.
fn column_of(cell: &Cell) -> Option<u32> {
    a1_position(address(cell.anchor.as_ref())?).map(|(_, col)| col)
}

/// `text` with ` (<address>)` after it; empty text stays empty.
fn with_address(text: String, at: Option<&str>) -> String {
    match at {
        Some(at) if !text.is_empty() => format!("{text} ({})", line_text(at)),
        _ => text,
    }
}

// ---------------------------------------------------------------------------------------
// Sheet sections

/// The sheet and the 1-based row of a caption paragraph's anchor.
fn caption_row(b: &Block) -> Option<(&str, u32)> {
    match b {
        Block::Paragraph {
            role: Some(role),
            anchor: Some(anchor),
            ..
        } if role == "caption" => {
            let (sheet, at) = anchor.0.rsplit_once('!')?;
            let (row, _) = a1_position(at)?;
            Some((sheet, row))
        }
        _ => None,
    }
}

/// A paragraph that stands for one cell: a caption, a note or a cell outside every region,
/// with a sheet anchor.
fn cell_line(b: &Block) -> Option<(&[Inline], &str)> {
    match b {
        Block::Paragraph {
            inlines,
            role: Some(role),
            anchor,
        } if matches!(role.as_str(), "caption" | "note" | "cell") => {
            Some((inlines.as_slice(), address(anchor.as_ref())?))
        }
        _ => None,
    }
}

/// The blocks of a sheet section. Caption, note and cell lines end with their cell's
/// address; the captions of one sheet row (a pivot's rows above its header) are one line
/// joined by ` | `; a generated heading that says what a list's lead line says stands for
/// it; a heading over 120 characters is cut, and its full text follows as a line with its
/// address. Everything else renders as in any section.
pub(crate) fn sheet_blocks(list: &[Block], ctx: &Ctx, depth: usize, level: u8) -> String {
    if depth > MAX_DEPTH {
        return blocks::blocks(list, ctx, depth, level, false);
    }
    let mut parts: Vec<String> = Vec::new();
    let mut from = 0;
    let mut i = 0;
    while i < list.len() {
        let own = if let Some((content, at)) = cell_line(&list[i]) {
            let mut lines = vec![with_address(paragraph(content), Some(at))];
            let mut next = i + 1;
            if let Some(row) = caption_row(&list[i]) {
                while next < list.len() && caption_row(&list[next]) == Some(row) {
                    if let Some((content, at)) = cell_line(&list[next]) {
                        lines.push(with_address(paragraph(content), Some(at)));
                    }
                    next += 1;
                }
            }
            lines.retain(|l| !l.is_empty());
            Some((lines.join(" | "), next))
        } else if let Some(text) = list
            .get(i..i + 2)
            .and_then(|pair| heading_with_list(pair, ctx, depth, level))
        {
            Some((text, i + 2))
        } else {
            long_heading(&list[i], ctx).map(|text| (text, i + 1))
        };
        match own {
            Some((text, next)) => {
                parts.push(blocks::blocks(&list[from..i], ctx, depth, level, false));
                parts.push(text);
                i = next;
                from = next;
            }
            None => i += 1,
        }
    }
    parts.push(blocks::blocks(&list[from..], ctx, depth, level, false));
    parts.retain(|p| !p.is_empty());
    parts.join("\n\n")
}

/// A heading whose text is longer than 120 characters: cut with `...`, then its full text
/// as a line with its address (the model keeps the full text). `None` for other blocks.
fn long_heading(b: &Block, ctx: &Ctx) -> Option<String> {
    let Block::Heading {
        level,
        inlines: content,
        anchor,
    } = b
    else {
        return None;
    };
    let text = plain_text(content);
    let text = text.trim();
    if text.chars().count() <= MAX_HEADING_CHARS {
        return None;
    }
    let cut: String = text.chars().take(MAX_HEADING_CHARS).collect();
    let level = level.saturating_add(ctx.heading_offset).clamp(1, 6);
    let head = heading_line(level, &format!("{}...", line_text(cut.trim_end())));
    let full = with_address(paragraph(content), address(anchor.as_ref()));
    Some(if full.is_empty() {
        head
    } else {
        format!("{head}\n\n{full}")
    })
}

// ---------------------------------------------------------------------------------------
// Regions

/// A table with a sheet layout, by its kind.
/// `lead` is false where a generated heading above stands for the lead line (P4-AS).
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_table(
    layout: &TableLayout,
    header_rows: usize,
    rows: &[Vec<Cell>],
    aligns: &[Align],
    lead: bool,
    ctx: &Ctx,
    depth: usize,
    level: u8,
) -> String {
    match layout.kind {
        LayoutKind::Title => String::new(),
        LayoutKind::KeyValue => {
            // The label, value and unit columns by sheet column: a position covered by a
            // merge from above has no cell, so a row's cells may start further right.
            let left = rows.iter().flatten().filter_map(column_of).min();
            listed(
                lead.then(|| lead_line(layout)).flatten(),
                rows.iter().filter_map(|row| pair(row, left)).collect(),
                ctx,
                depth,
                level,
            )
        }
        LayoutKind::Text => {
            let cells: Vec<&Cell> = rows.iter().flatten().filter(|c| !is_empty(c)).collect();
            // One cell of text is a plain line of its own (spec 7.4).
            if let [cell] = cells.as_slice() {
                return with_address(
                    paragraph(trimmed(&cell.inlines)),
                    address(cell.anchor.as_ref()),
                );
            }
            listed(
                lead.then(|| lead_line(layout)).flatten(),
                cells
                    .iter()
                    .map(|c| item(c.inlines.clone(), address(c.anchor.as_ref())))
                    .collect(),
                ctx,
                depth,
                level,
            )
        }
        LayoutKind::Grid => grid(layout, rows, aligns),
        LayoutKind::Table => table(layout, header_rows, rows, aligns),
    }
}

fn is_empty(c: &Cell) -> bool {
    plain_text(&c.inlines).trim().is_empty()
}

/// Inlines without line breaks at their ends.
fn trimmed(list: &[Inline]) -> &[Inline] {
    let start = list
        .iter()
        .position(|i| !matches!(i, Inline::LineBreak))
        .unwrap_or(list.len());
    let end = list
        .iter()
        .rposition(|i| !matches!(i, Inline::LineBreak))
        .map_or(start, |e| e + 1);
    &list[start..end.max(start)]
}

/// A list item of one paragraph: the text, then ` (<address>)`.
fn item(mut content: Vec<Inline>, at: Option<&str>) -> Vec<Block> {
    content = trimmed(&content).to_vec();
    if let Some(at) = at {
        content.push(Inline::plain(format!(" ({at})")));
    }
    vec![Block::paragraph(content)]
}

/// `<label>: <value> <unit> (<cell of the value>)` for a row of a label and value region.
/// A cell's part comes from its sheet column counted from `left` (the region's first
/// column), else from its place in the row. Empty cells are left out; a row without a value
/// names its label's cell. `None` for an empty row.
fn pair(row: &[Cell], left: Option<u32>) -> Option<Vec<Block>> {
    let mut content: Vec<Inline> = Vec::new();
    let mut at: Option<&str> = None;
    for (i, cell) in row.iter().enumerate() {
        if is_empty(cell) {
            continue;
        }
        let part = match (left, column_of(cell)) {
            (Some(left), Some(col)) => col.saturating_sub(left) as usize,
            _ => i,
        };
        if !content.is_empty() {
            content.push(Inline::plain(if part == 1 { ": " } else { " " }));
        }
        content.extend_from_slice(trimmed(&cell.inlines));
        // The value's cell is the pair's address; the label's when there is no value.
        if part == 1 || at.is_none() {
            at = address(cell.anchor.as_ref()).or(at);
        }
    }
    (!content.is_empty()).then(|| item(content, at))
}

/// The lead line `<word> <range>:` and the items as a list under it.
fn listed(
    lead: Option<String>,
    items: Vec<Vec<Block>>,
    ctx: &Ctx,
    depth: usize,
    level: u8,
) -> String {
    let list = if items.is_empty() {
        String::new()
    } else {
        list_block(false, &items, false, ctx, depth, level)
    };
    match lead {
        Some(lead) if list.is_empty() => format!("{}:", line_text(&lead)),
        Some(lead) => format!("{}:\n\n{list}", line_text(&lead)),
        None => list,
    }
}

/// The words of a label and value or text region's lead line before its colon, for example
/// `Text A19:A25`: the kind and the data range, else the range. `None` for other kinds.
fn lead_line(layout: &TableLayout) -> Option<String> {
    let word = match layout.kind {
        LayoutKind::KeyValue => "Label and value pairs",
        LayoutKind::Text => "Text",
        _ => return None,
    };
    let range = layout.data.as_deref().unwrap_or(&layout.range);
    Some(format!("{word} {range}"))
}

/// A generated heading (it has no anchor) right above a label and value or text region that
/// says what its lead line says: the heading stands for the lead (P4-AS). Returns the
/// heading and the region without its lead.
fn heading_with_list(pair: &[Block], ctx: &Ctx, depth: usize, level: u8) -> Option<String> {
    let [
        heading @ Block::Heading {
            inlines: content,
            anchor: None,
            ..
        },
        Block::Table {
            header_rows,
            rows,
            alignments,
            layout: Some(layout),
            ..
        },
    ] = pair
    else {
        return None;
    };
    if Some(plain_text(content).trim()) != lead_line(layout).as_deref() {
        return None;
    }
    let head = blocks::blocks(std::slice::from_ref(heading), ctx, depth, level, false);
    let list = layout_table(
        layout,
        *header_rows,
        rows,
        alignments,
        false,
        ctx,
        depth,
        level,
    );
    Some(if list.is_empty() {
        head
    } else {
        format!("{head}\n\n{list}")
    })
}

/// The 0-based sheet column of a region's first table column: from the cells' anchors, else
/// from the start of its data range or its range.
fn left_column(layout: &TableLayout, grid: &Grid) -> u32 {
    grid.left_column()
        .or_else(|| {
            let range = layout.data.as_deref().unwrap_or(&layout.range);
            a1_position(range.split(':').next()?).map(|(_, col)| col)
        })
        .unwrap_or(0)
}

fn letter(left: u32, offset: usize) -> String {
    column_letter((left as usize).saturating_add(offset))
}

/// The words of a fact that found a region; `None` for a rule.
fn fact_words(e: &Evidence) -> Option<String> {
    Some(match e {
        Evidence::ListObject { name } => format!("Excel table {}", line_text(name)),
        Evidence::DatabaseRange { name } => format!("database range {}", line_text(name)),
        Evidence::Pivot { name } => format!("pivot table {}", line_text(name)),
        Evidence::Autofilter => "filter range".into(),
        Evidence::PrintTitles => "print titles".into(),
        Evidence::FrozenPanes => "frozen panes".into(),
        Evidence::DefinedName { name } => format!("defined name {}", line_text(name)),
        Evidence::Rule { .. } => return None,
    })
}

/// The line that says where a table or grid region is and how it was found (spec 7.2).
fn layout_line(layout: &TableLayout, left: u32) -> String {
    let grid = layout.kind == LayoutKind::Grid;
    let mut line = format!(
        "{} {}",
        if grid { "Grid" } else { "Table" },
        line_text(&layout.range)
    );
    if let Some(data) = layout.data.as_deref().filter(|d| *d != layout.range) {
        line.push_str(&format!(" (data {})", line_text(data)));
    }
    let rules: Vec<&str> = layout
        .found_by
        .iter()
        .filter_map(|e| match e {
            Evidence::Rule { detail, .. } if !detail.trim().is_empty() => Some(detail.trim()),
            _ => None,
        })
        .collect();
    // What the line says itself, in the words the rules use: the header rows and the label
    // columns. A rule detail never repeats it.
    let header_words = match layout.header_rows.as_slice() {
        [] => None,
        [one] => Some(format!("header row {one}")),
        [first, .., last] => Some(format!("header rows {first} to {last}")),
    };
    let label_words = match layout.label_cols {
        0 => None,
        1 => Some(format!("label column {}", letter(left, 0))),
        n => Some(format!(
            "label columns {} to {}",
            letter(left, 0),
            letter(left, n as usize - 1)
        )),
    };
    let detail_words = |detail: &str| -> Option<String> {
        if label_words.as_deref() == Some(detail) {
            return None;
        }
        if let Some(words) = &header_words
            && let Some(at) = detail.find(words.as_str())
            && !detail[at + words.len()..].starts_with(|c: char| c.is_ascii_digit())
        {
            let rest = format!("{}header{}", &detail[..at], &detail[at + words.len()..]);
            return Some(rest);
        }
        Some(detail.to_string())
    };
    if grid {
        line.push_str(", header unsure");
        let reasons: Vec<String> = layout
            .unsure
            .iter()
            .filter(|u| !u.trim().is_empty())
            .map(|u| line_text(u.trim()))
            .collect();
        if !reasons.is_empty() {
            line.push_str(&format!(": {}", reasons.join(", ")));
        }
    } else {
        match layout.header_rows.as_slice() {
            [] if !layout.hidden_header && !rules.contains(&"no header found") => {
                line.push_str(", no header found");
            }
            [] => {}
            [one] => line.push_str(&format!(", header row {one}")),
            [first, .., last] => line.push_str(&format!(", header rows {first} to {last}")),
        }
        if layout.hidden_header {
            line.push_str(", header row hidden in the file");
        }
        if layout.label_cols > 0 {
            line.push_str(&format!(", row labels in {}", letter(left, 0)));
            if layout.label_cols > 1 {
                line.push_str(&format!(
                    " to {}",
                    letter(left, layout.label_cols as usize - 1)
                ));
            }
        }
        if !layout.header_rows.is_empty() {
            let bare: Vec<String> = layout
                .header_paths
                .iter()
                .take(MAX_COLUMNS)
                .enumerate()
                .filter(|(_, p)| p.trim().is_empty())
                .map(|(c, _)| letter(left, c))
                .collect();
            match bare.len() {
                0 => {}
                1 => line.push_str(&format!(", 1 column without a header: {}", bare[0])),
                n => line.push_str(&format!(
                    ", {n} columns without a header: {}",
                    bare.join(", ")
                )),
            }
        }
    }
    let facts: Vec<String> = layout.found_by.iter().filter_map(fact_words).collect();
    let rules: Vec<String> = rules
        .into_iter()
        .filter_map(detail_words)
        .map(|d| line_text(&d))
        .collect();
    match (facts.is_empty(), rules.is_empty()) {
        (false, false) => line.push_str(&format!(
            ", found by: {}; rules: {}",
            facts.join(", "),
            rules.join(", ")
        )),
        (false, true) => line.push_str(&format!(", found by: {}", facts.join(", "))),
        (true, false) => line.push_str(&format!(", found by rules: {}", rules.join(", "))),
        (true, true) => {}
    }
    line.push('.');
    line
}

/// One line of the subtotal, total and group rows, for example `Subtotal rows: 12 (SUBTOTAL
/// formula in B12), 18 (SUBTOTAL formula in B18). Group rows: 5, 16 (one text in the first
/// column).` Rows with the same reason share it. `None` when there are none.
fn kinds_line(notes: &[RowNote]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for (kind, one, many) in [
        (RowKind::Subtotal, "Subtotal row", "Subtotal rows"),
        (RowKind::Total, "Total row", "Total rows"),
        (RowKind::Group, "Group row", "Group rows"),
    ] {
        let mut groups: Vec<(&str, Vec<u32>)> = Vec::new();
        let mut at: HashMap<&str, usize> = HashMap::new();
        let mut count = 0usize;
        for n in notes.iter().filter(|n| n.kind == kind) {
            count += 1;
            let why = n.why.trim();
            match at.get(why) {
                Some(&g) => groups[g].1.push(n.row),
                None => {
                    at.insert(why, groups.len());
                    groups.push((why, vec![n.row]));
                }
            }
        }
        if count == 0 {
            continue;
        }
        let words: Vec<String> = groups
            .into_iter()
            .map(|(why, rows)| {
                let rows: Vec<String> = rows.iter().map(u32::to_string).collect();
                if why.is_empty() {
                    rows.join(", ")
                } else {
                    format!("{} ({})", rows.join(", "), line_text(why))
                }
            })
            .collect();
        parts.push(format!(
            "{}: {}.",
            if count == 1 { one } else { many },
            words.join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The layout line, a blank line and the table with its notes under it.
fn assemble(line: String, table: String, notes: Vec<String>) -> String {
    let mut out = line;
    out.push_str("\n\n");
    out.push_str(&table);
    if !notes.is_empty() {
        out.push_str("\n\n");
        out.push_str(&notes.join("\n"));
    }
    out
}

/// Alignments with one more column in front.
fn shifted(aligns: &[Align]) -> Vec<Align> {
    if aligns.is_empty() {
        return Vec::new();
    }
    std::iter::once(Align::None)
        .chain(aligns.iter().copied())
        .collect()
}

/// A `table` region: the header paths as the header, the rows after the header band as the
/// body, a `Parent` column when a row has a parent (R5).
fn table(layout: &TableLayout, header_rows: usize, rows: &[Vec<Cell>], aligns: &[Align]) -> String {
    let grid = Grid::new(rows);
    let left = left_column(layout, &grid);
    let line = layout_line(layout, left);
    if rows.is_empty() {
        return line;
    }
    // The band rows are the header paths, which spread the band's merges already: only the
    // marks and merges of the body are named.
    let head = header_rows.min(grid.spots.len());
    let ranges = grid.ranges_from(head);
    let legend = grid.marked(head) || !ranges.is_empty();
    let width = grid
        .width
        .max(layout.header_paths.len())
        .clamp(1, MAX_COLUMNS);
    let mut header: Vec<String> = (0..width)
        .map(|c| {
            match layout
                .header_paths
                .get(c)
                .map(|p| p.trim())
                .filter(|p| !p.is_empty())
            {
                Some(path) => cell_text(path),
                None => format!("Column {}", letter(left, c)),
            }
        })
        .collect();
    let parents: HashMap<u32, u32> = layout
        .rows
        .iter()
        .filter_map(|n| Some((n.row, n.parent?)))
        .collect();
    let mut body: Vec<Vec<String>> = (head..grid.spots.len())
        .map(|r| grid.line(r, legend))
        .collect();
    let aligns = if parents.is_empty() {
        aligns.to_vec()
    } else {
        // The parent's text is the first cell of its row in the table.
        let at: HashMap<u32, usize> = (0..grid.spots.len())
            .filter_map(|r| Some((grid.sheet_row(r)?, r)))
            .collect();
        let label = |row: u32| -> String {
            at.get(&row)
                .and_then(|&r| grid.spots[r].first().copied().flatten())
                .map(|spot| grid.own(spot.cell(), legend))
                .unwrap_or_default()
        };
        header.insert(0, "Parent".into());
        for (line, r) in body.iter_mut().zip(head..) {
            let parent = grid
                .sheet_row(r)
                .and_then(|row| parents.get(&row))
                .map(|&p| label(p))
                .unwrap_or_default();
            line.insert(0, parent);
        }
        shifted(aligns)
    };
    let mut notes: Vec<String> = Vec::new();
    notes.extend(grid.merge_note(legend, &ranges));
    notes.extend(kinds_line(&layout.rows));
    notes.extend(grid.cut_notes());
    assemble(line, pipe_table(&header, &aligns, &body), notes)
}

/// A `grid` region (unsure): every row as it is, the sheet column letters as the header and
/// the sheet row numbers as the first column (spec 6.8).
fn grid(layout: &TableLayout, rows: &[Vec<Cell>], aligns: &[Align]) -> String {
    let grid = Grid::new(rows);
    let left = left_column(layout, &grid);
    let line = layout_line(layout, left);
    if rows.is_empty() {
        return line;
    }
    let ranges = grid.ranges();
    let legend = grid.marked(0) || !ranges.is_empty();
    let header: Vec<String> = std::iter::once(String::new())
        .chain((0..grid.width.max(1)).map(|c| letter(left, c)))
        .collect();
    let body: Vec<Vec<String>> = (0..grid.spots.len())
        .map(|r| {
            let number = grid.sheet_row(r).map(|n| n.to_string()).unwrap_or_default();
            std::iter::once(number)
                .chain(grid.line(r, legend))
                .collect()
        })
        .collect();
    let mut notes: Vec<String> = Vec::new();
    notes.extend(grid.merge_note(legend, &ranges));
    notes.extend(grid.cut_notes());
    assemble(line, pipe_table(&header, &shifted(aligns), &body), notes)
}
