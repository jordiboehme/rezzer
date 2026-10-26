// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The layout of a sheet table: what kind of region it is, where its header and data stand,
//! what found it, and notes per row. Plus the table key and the row-range split.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::block::{Block, Cell, column_letter};
use crate::inline::plain_text;

/// The layout of a region of a sheet, kept on its [`Block::Table`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct TableLayout {
    pub kind: LayoutKind,
    /// A key that does not depend on the region's place in a list (see [`table_key`]).
    pub key: String,
    /// The whole region in A1, with its titles and notes, for example `A1:F68`.
    pub range: String,
    /// The data rows in A1, for example `A5:F62`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// The sheet rows of the header band, 1-based; empty for none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header_rows: Vec<u32>,
    /// One header path per column, in column order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header_paths: Vec<String>,
    #[serde(default)]
    pub label_cols: u32,
    /// The sheet columns, 1-based, of a pivot table's grand total column.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub total_columns: Vec<u32>,
    pub certainty: Certainty,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub found_by: Vec<Evidence>,
    /// Why the region is unsure; empty unless it is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsure: Vec<String>,
    /// Sparse: only rows with a kind or a level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<RowNote>,
    /// True when the file says the table has no header row shown (an Excel table with
    /// `headerRowCount="0"`): the header paths then come from its column names.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden_header: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutKind {
    Table,
    KeyValue,
    Title,
    Text,
    Grid,
}

/// How sure the layout is: from facts in the file, from rules, or not sure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    Exact,
    Rule,
    Unsure,
}

/// What found a region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Evidence {
    /// An Excel table.
    ListObject {
        name: String,
    },
    /// An ODS database range.
    DatabaseRange {
        name: String,
    },
    Pivot {
        name: String,
    },
    Autofilter,
    PrintTitles,
    FrozenPanes,
    DefinedName {
        name: String,
    },
    /// A layout rule with the words of the layout line.
    Rule {
        id: String,
        detail: String,
    },
}

/// A note on one sheet row of a region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct RowNote {
    /// The sheet row, 1-based.
    pub row: u32,
    pub kind: RowKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// The sheet row of the parent row, 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    Data,
    Subtotal,
    Total,
    Group,
    Blank,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The prefix LibreOffice gives the database range it makes per sheet without a name.
const ANONYMOUS_DB: &str = "__Anonymous_Sheet_DB__";

/// The key of a region (spec 9.4): `table:<name>` for an Excel table, `dbrange:<name>` for a
/// named ODS database range, `pivot:<sheet>!<name>` for a pivot, else `<sheet>!<top left>`.
/// `sheet_q` is the sheet name as a reference writes it, quoted where needed; it is used as
/// it is. The kind of fact decides, not its place in `found_by`.
pub fn table_key(sheet_q: &str, found_by: &[Evidence], top_left: &str) -> String {
    if let Some(name) = found_by.iter().find_map(|e| match e {
        Evidence::ListObject { name } => Some(name),
        _ => None,
    }) {
        return format!("table:{name}");
    }
    if let Some(name) = found_by.iter().find_map(|e| match e {
        Evidence::DatabaseRange { name } if !name.starts_with(ANONYMOUS_DB) => Some(name),
        _ => None,
    }) {
        return format!("dbrange:{name}");
    }
    if let Some(name) = found_by.iter().find_map(|e| match e {
        Evidence::Pivot { name } => Some(name),
        _ => None,
    }) {
        return format!("pivot:{sheet_q}!{name}");
    }
    format!("{sheet_q}!{top_left}")
}

/// A 0-based (row, column) from an A1 cell address such as `B5` or `$B$5`. `None` for
/// anything else, a sheet prefix included.
pub fn a1_position(text: &str) -> Option<(u32, u32)> {
    let text = text.trim();
    let text = text.strip_prefix('$').unwrap_or(text);
    let letters = text.bytes().take_while(u8::is_ascii_alphabetic).count();
    if letters == 0 || letters > 3 {
        return None;
    }
    let (l, d) = text.split_at(letters);
    let d = d.strip_prefix('$').unwrap_or(d);
    if d.is_empty() || d.len() > 7 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let col = l.bytes().fold(0u32, |n, c| {
        n * 26 + u32::from(c.to_ascii_uppercase() - b'A' + 1)
    });
    let row: u32 = d.parse().ok()?;
    if !(1..=16_384).contains(&col) || !(1..=1_048_576).contains(&row) {
        return None;
    }
    Some((row - 1, col - 1))
}

/// One cell named by a `Sheet!A1` anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SheetCell<'a> {
    /// The sheet as the anchor writes it, quoted where needed.
    pub sheet: &'a str,
    /// The local address, for example `B5`.
    pub address: &'a str,
    /// 0-based.
    pub row: u32,
    /// 0-based.
    pub col: u32,
}

/// The cell a `Sheet!A1` anchor names; the sheet name may hold a `!` itself. `None` for an
/// anchor that names no cell, such as a defined name's `name:'Sheet'!Local` or a range.
pub fn sheet_cell(anchor: &str) -> Option<SheetCell<'_>> {
    let (sheet, address) = anchor.rsplit_once('!')?;
    let (row, col) = a1_position(address)?;
    Some(SheetCell {
        sheet,
        address,
        row,
        col,
    })
}

/// A table as one of its consecutive row ranges, or the notes under it.
#[derive(Debug, Clone, PartialEq)]
pub struct RowRange {
    /// The first sheet row the range holds, 1-based: of its data rows, or for the notes
    /// range of its notes. 0 when the table stores no data row.
    pub first: u32,
    /// The last sheet row the range holds, 1-based, as `first`.
    pub last: u32,
    /// The lead blocks, then the table with the header band and the range's rows, then in
    /// the last range the tail blocks. The notes range holds the lead and the tail only.
    pub blocks: Vec<Block>,
    /// The index of the table in `blocks`; `None` for the notes range (and for a block that
    /// is no table).
    pub table: Option<usize>,
}

/// The fixed part of a row's size in characters, and what each non-empty cell adds besides
/// its header path and text.
const ROW_BASE: usize = 10;
const CELL_BASE: usize = 6;

/// Splits a table into consecutive row ranges of at most `max_tokens` tokens each (spec
/// 9.3). `max_tokens` is the part size the caller chose; 0 means no limit.
///
/// Every range repeats `lead` (heading and caption blocks) and the table's header band (its
/// first `header_rows` rows). Rows are cut only at row borders, never between a
/// [`RowKind::Group`] row and its next row that is no group or blank row, and never inside
/// the rows a data cell's `row_span` covers. A unit larger than the limit is a range of its
/// own. Each range's table names its own data rows in `layout.data`. A range keeps the row
/// notes from the row after the previous range's last row to its own last row; the first
/// range also those before it, the last range also those after it.
///
/// `tail` (notes) is counted like the lead, but only once: it goes into the last range when
/// that range still fits with it. Else the last range closes and a notes range follows with
/// the lead and the tail, no table; it is never cut, and its `first` and `last` are the
/// sheet rows of the tail's anchors, or the row after the last data row without anchors.
///
/// The size of a row is estimated from the model, the same for every table style:
/// `ceil((10 + sum over non-empty cells of (chars of its header path + chars of its text +
/// 6)) / 4)` tokens. The fixed part is `ceil(chars / 4)` per lead block plus the header band
/// rows by the same formula. The data rows always get at least half of `max_tokens`: the
/// fixed part counts at most `max_tokens / 2` against the limit. So a wide band never gives
/// one range per row, and a range holds at most about 1.5 times `max_tokens` while the fixed
/// part is below `max_tokens`, or the fixed part plus half of `max_tokens` above it.
///
/// A table without layout, a block that is no table, a table that stores no data row, or a
/// table that fits with its notes gives one range with the blocks unchanged. Its `first` and
/// `last` are the sheet rows of its data part; without layout they count the table's own
/// rows, 1-based. With no data row they are 0 and 0, with or without layout.
pub fn split_rows(
    lead: &[Block],
    table: &Block,
    tail: &[Block],
    max_tokens: usize,
) -> Vec<RowRange> {
    let is_table = matches!(table, Block::Table { .. });
    let whole = |first: u32, last: u32, with_tail: bool| RowRange {
        first,
        last,
        blocks: lead
            .iter()
            .chain(std::iter::once(table))
            .chain(tail.iter().filter(|_| with_tail))
            .cloned()
            .collect(),
        table: is_table.then_some(lead.len()),
    };
    let Block::Table {
        rows,
        header_rows,
        layout,
        ..
    } = table
    else {
        return vec![whole(0, 0, true)];
    };
    let head = (*header_rows).min(rows.len());
    let Some(layout) = layout else {
        return vec![if rows.len() > head {
            whole(head as u32 + 1, rows.len() as u32, true)
        } else {
            whole(0, 0, true)
        }];
    };
    let data = &rows[head..];
    let numbers = sheet_rows(layout, data);
    let (Some(&first), Some(&last)) = (numbers.first(), numbers.last()) else {
        return vec![whole(0, 0, true)];
    };
    let columns = columns(rows, layout.header_paths.len());
    let tokens: Vec<usize> = rows
        .iter()
        .zip(&columns)
        .map(|(row, cols)| row_tokens(row, cols, &layout.header_paths))
        .collect();
    let block_tokens = |list: &[Block]| -> usize {
        list.iter()
            .map(|b| block_chars(b).div_ceil(4))
            .fold(0, usize::saturating_add)
    };
    let fixed = block_tokens(lead).saturating_add(tokens[..head].iter().sum::<usize>());
    // The data rows always get at least half the part, however large the fixed part.
    let base = if max_tokens > 0 {
        fixed.min(max_tokens / 2)
    } else {
        fixed
    };
    let fits = |size: usize| max_tokens == 0 || base.saturating_add(size) <= max_tokens;
    let units = units(layout, data, &numbers);
    // Greedy: a range takes units while they fit; an empty range takes one unit always.
    let mut parts: Vec<(usize, usize)> = Vec::new();
    let mut size = 0usize;
    for (start, end) in units {
        let unit: usize = tokens[head + start..=head + end].iter().sum();
        match parts.last_mut() {
            Some(part) if fits(size.saturating_add(unit)) => {
                part.1 = end;
                size += unit;
            }
            _ => {
                parts.push((start, end));
                size = unit;
            }
        }
    }
    let tail_fits = tail.is_empty() || fits(size.saturating_add(block_tokens(tail)));
    let notes = (!tail_fits).then(|| {
        let (first, last) = tail_rows(tail).unwrap_or((last + 1, last + 1));
        RowRange {
            first,
            last,
            blocks: lead.iter().chain(tail).cloned().collect(),
            table: None,
        }
    });
    if parts.len() <= 1 {
        let mut out = vec![whole(first, last, tail_fits)];
        out.extend(notes);
        return out;
    }
    let n = parts.len();
    let mut out: Vec<RowRange> = Vec::with_capacity(n + 1);
    let mut prev_last: Option<u32> = None;
    for (i, (start, end)) in parts.into_iter().enumerate() {
        let (first, last) = (numbers[start], numbers[end]);
        let is_last = i + 1 == n;
        let mut part = layout.as_ref().clone();
        part.data = Some(data_range(layout, first, last));
        part.rows
            .retain(|note| prev_last.is_none_or(|p| note.row > p) && (is_last || note.row <= last));
        prev_last = Some(last);
        let mut cut = table.clone();
        if let Block::Table {
            rows: r, layout: l, ..
        } = &mut cut
        {
            *r = rows[..head]
                .iter()
                .chain(&data[start..=end])
                .cloned()
                .collect();
            *l = Some(Box::new(part));
        }
        let mut blocks: Vec<Block> = lead.to_vec();
        blocks.push(cut);
        if is_last && tail_fits {
            blocks.extend_from_slice(tail);
        }
        out.push(RowRange {
            first,
            last,
            blocks,
            table: Some(lead.len()),
        });
    }
    out.extend(notes);
    out
}

/// The smallest and largest 1-based sheet row the anchors of `blocks` name (`Sheet!A5` or
/// `Sheet!A5:F6`); `None` when none of them names one.
fn tail_rows(blocks: &[Block]) -> Option<(u32, u32)> {
    blocks
        .iter()
        .filter_map(block_anchor)
        .filter_map(|a| area(a.rsplit_once('!')?.1))
        .map(|((top, bottom), _)| (top + 1, bottom + 1))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

fn block_anchor(b: &Block) -> Option<&str> {
    match b {
        Block::Section { anchor, .. }
        | Block::Heading { anchor, .. }
        | Block::Paragraph { anchor, .. }
        | Block::List { anchor, .. }
        | Block::Table { anchor, .. }
        | Block::Code { anchor, .. }
        | Block::Quote { anchor, .. }
        | Block::Math { anchor, .. }
        | Block::Media { anchor, .. }
        | Block::Callout { anchor, .. }
        | Block::Rule { anchor, .. }
        | Block::Task { anchor, .. } => anchor.as_ref().map(|a| a.0.as_str()),
    }
}

/// The rows (top, bottom) and columns (left, right) of an A1 area `A5:F62` or a cell `B3`,
/// 0-based.
fn area(text: &str) -> Option<((u32, u32), (u32, u32))> {
    let (a, b) = match text.split_once(':') {
        Some((a, b)) => (a1_position(a)?, a1_position(b)?),
        None => {
            let a = a1_position(text)?;
            (a, a)
        }
    };
    Some(((a.0.min(b.0), a.0.max(b.0)), (a.1.min(b.1), a.1.max(b.1))))
}

/// The 1-based sheet rows of the layout's data part.
fn data_rows(layout: &TableLayout) -> Option<(u32, u32)> {
    let (rows, _) = area(layout.data.as_deref()?)?;
    Some((rows.0 + 1, rows.1 + 1))
}

/// The data range of rows `first` to `last` in the columns of the layout's data part (or
/// of its whole range); the old data range when neither reads.
fn data_range(layout: &TableLayout, first: u32, last: u32) -> String {
    let cols = layout
        .data
        .as_deref()
        .and_then(area)
        .or_else(|| area(&layout.range))
        .map(|(_, cols)| cols);
    match cols {
        Some((left, right)) => format!(
            "{}{first}:{}{last}",
            column_letter(left as usize),
            column_letter(right as usize)
        ),
        None => layout.data.clone().unwrap_or_default(),
    }
}

/// The 1-based sheet row of a table row: the row of its first cell with a sheet anchor.
fn anchor_row(row: &[Cell]) -> Option<u32> {
    row.iter()
        .find_map(|c| sheet_cell(&c.anchor.as_ref()?.0).map(|at| at.row + 1))
}

/// The 1-based sheet row of every data row: its anchor's row, else the row after the one
/// before it; the first one without an anchor is the first row of the layout's data part
/// (or the row after its header band, or the region's top row).
fn sheet_rows(layout: &TableLayout, data: &[Vec<Cell>]) -> Vec<u32> {
    let base = data_rows(layout)
        .map(|(top, _)| top)
        .or_else(|| layout.header_rows.iter().max().map(|r| r.saturating_add(1)))
        .or_else(|| area(&layout.range).map(|((top, _), _)| top + 1))
        .unwrap_or(1);
    let mut prev: Option<u32> = None;
    data.iter()
        .map(|row| {
            let n = anchor_row(row).unwrap_or_else(|| prev.map_or(base, |p| p.saturating_add(1)));
            prev = Some(n);
            n
        })
        .collect()
}

/// The 0-based table column of every cell, placed as the renderer places them: left to
/// right, past positions a cell from a row above covers. Only the first `width` columns are
/// tracked: a cell at or past them has no header path either way.
fn columns(rows: &[Vec<Cell>], width: usize) -> Vec<Vec<usize>> {
    // Per column, the first row it is free again.
    let mut free: Vec<usize> = Vec::new();
    rows.iter()
        .enumerate()
        .map(|(r, row)| {
            let mut c = 0usize;
            row.iter()
                .map(|cell| {
                    while free.get(c).is_some_and(|&f| f > r) {
                        c += 1;
                    }
                    let at = c;
                    let cs = cell.col_span.max(1) as usize;
                    let until = r
                        .saturating_add(cell.row_span.max(1) as usize)
                        .min(rows.len());
                    let fill = at.saturating_add(cs).min(width);
                    if free.len() < fill {
                        free.resize(fill, 0);
                    }
                    for f in free.iter_mut().take(fill).skip(at) {
                        *f = until;
                    }
                    c = at.saturating_add(cs);
                    at
                })
                .collect()
        })
        .collect()
}

/// The tokens of a row by the formula of [`split_rows`].
fn row_tokens(row: &[Cell], cols: &[usize], paths: &[String]) -> usize {
    let chars: usize = row
        .iter()
        .zip(cols)
        .filter_map(|(cell, &col)| {
            let text = plain_text(&cell.inlines);
            (!text.is_empty()).then(|| {
                let path = paths.get(col).map_or(0, |p| p.chars().count());
                path + text.chars().count() + CELL_BASE
            })
        })
        .fold(0, usize::saturating_add);
    ROW_BASE.saturating_add(chars).div_ceil(4)
}

/// The data rows that may not be cut apart, as (first, last) indexes into `data`: a group
/// row goes with the next row that is neither a group nor a blank row, and a cell over
/// several rows keeps them together.
fn units(layout: &TableLayout, data: &[Vec<Cell>], numbers: &[u32]) -> Vec<(usize, usize)> {
    let of_kind = |kind: RowKind| -> HashSet<u32> {
        layout
            .rows
            .iter()
            .filter(|n| n.kind == kind)
            .map(|n| n.row)
            .collect()
    };
    let groups = of_kind(RowKind::Group);
    let blanks = of_kind(RowKind::Blank);
    let end = data.len() - 1;
    let mut out = Vec::new();
    let mut start = 0usize;
    while start <= end {
        let mut reach = start;
        let mut j = start;
        loop {
            let span = data[j]
                .iter()
                .map(|c| c.row_span.max(1) as usize)
                .max()
                .unwrap_or(1);
            reach = reach.max(j.saturating_add(span - 1).min(end));
            if groups.contains(&numbers[j]) {
                let mut k = j + 1;
                while k < end && blanks.contains(&numbers[k]) {
                    k += 1;
                }
                reach = reach.max(k.min(end));
            }
            if j >= reach {
                break;
            }
            j += 1;
        }
        out.push((start, reach));
        start = reach + 1;
    }
    out
}

/// The characters of a block's text, walked with a stack.
fn block_chars(block: &Block) -> usize {
    let mut n = 0usize;
    let mut stack: Vec<&Block> = vec![block];
    let cells = |rows: &[Vec<Cell>]| -> usize {
        rows.iter()
            .flatten()
            .map(|c| plain_text(&c.inlines).chars().count())
            .sum()
    };
    while let Some(b) = stack.pop() {
        let count = |s: &str| s.chars().count();
        n += match b {
            Block::Section { title, blocks, .. } => {
                stack.extend(blocks);
                title.as_deref().map_or(0, count)
            }
            Block::Heading { inlines, .. }
            | Block::Paragraph { inlines, .. }
            | Block::Task { inlines, .. } => count(&plain_text(inlines)),
            Block::List { items, .. } => {
                stack.extend(items.iter().flatten());
                0
            }
            Block::Table { caption, rows, .. } => caption.as_deref().map_or(0, count) + cells(rows),
            Block::Code { text, .. } => count(text),
            Block::Quote { blocks, .. } => {
                stack.extend(blocks);
                0
            }
            Block::Callout { title, blocks, .. } => {
                stack.extend(blocks);
                title.as_deref().map_or(0, count)
            }
            Block::Math { latex, .. } => count(latex),
            Block::Media { alt, .. } => alt.as_deref().map_or(0, count),
            Block::Rule { .. } => 0,
        };
    }
    n
}
