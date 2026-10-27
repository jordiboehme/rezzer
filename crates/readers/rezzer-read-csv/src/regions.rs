// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The records of a region, the facts per column and the rows that fill a table.

use csvsniff::{ColumnKind, Dialect};
use rezzer_model::{
    Anchor, Block, Cell, Document, Formula, Inline, Loss, LossReason, Role, Severity, column_letter,
};
use sheetlayout::{Kind, Layout, RowKind};

use super::{MAX_FACT_ROWS, MAX_TABLE_COLUMNS, MAX_TABLE_ROWS};

/// The records a CSV reader read, for the column facts of its table regions.
pub(super) struct Records<'a> {
    pub(super) text: &'a str,
    pub(super) dialect: &'a Dialect,
    /// The sheet row and the first byte of each record read, in order.
    pub(super) starts: &'a [(u32, usize)],
    /// The positions of the cells with text, in reading order.
    pub(super) filled: &'a [(u32, u32)],
    /// The field count of the widest record read.
    pub(super) widest: usize,
}

/// The Columns tables of a file's table regions, the losses of those that could not be
/// made and the derived relations found in them.
pub(super) struct RegionFacts {
    pub(super) tables: Vec<Block>,
    pub(super) losses: Vec<Loss>,
    pub(super) formulas: Vec<Formula>,
}

/// One Columns table per table region, in region order, from the region's header and data
/// rows (C8): a title, caption or note line is never counted as data, and a second table in
/// the file gets its own facts. The records from the region's last header row to its last
/// data row are analyzed on their own; the last region of a file whose rows were not all
/// shown reaches to the end of the file. A region whose analysis the time limit cuts gets an
/// Uncertain loss instead of a table. `None` when the layout found no table region.
pub(super) fn region_facts(
    part: &Records,
    layout: &Layout,
    sheet_q: &str,
    stop: &dyn Fn() -> bool,
) -> Option<RegionFacts> {
    let regions: Vec<&sheetlayout::Region> = layout
        .regions
        .iter()
        .filter(|r| r.kind == Kind::Table && r.data.is_some())
        .collect();
    if regions.is_empty() {
        return None;
    }
    let mut out = RegionFacts {
        tables: Vec::new(),
        losses: Vec::new(),
        formulas: Vec::new(),
    };
    let mut not_counted: Vec<String> = Vec::new();
    // Tables side by side share their records: the last analysis is kept for the next.
    let mut last: Option<((usize, usize, bool), csvsniff::Analysis)> = None;
    for region in regions {
        let Some(data) = region.data else {
            continue;
        };
        let range = region.rect.a1();
        if !not_counted.is_empty() || stop() {
            not_counted.push(range);
            continue;
        }
        let first = region.header_rows.last().copied().unwrap_or(data.top);
        let at = |row: u32| part.starts.partition_point(|s| s.0 < row);
        let from = at(first);
        let to = part.starts.partition_point(|s| s.0 <= data.bottom);
        let Some(&(_, start)) = part.starts.get(from) else {
            continue;
        };
        // The last region of the file also counts the records that were not shown.
        let end = part.starts.get(to).map_or(part.text.len(), |s| s.1);
        let shown = to.saturating_sub(at(data.top));
        let has_header = !region.header_rows.is_empty();
        let slice = (start, end, has_header);
        if last.as_ref().is_none_or(|(at, _)| *at != slice) {
            let dialect = Dialect {
                lead_records: 0,
                has_header,
                ..part.dialect.clone()
            };
            let found =
                csvsniff::analyze_until(&part.text[start..end], &dialect, MAX_TABLE_ROWS, stop);
            last = Some((slice, found));
        }
        let Some((_, analysis)) = last.as_ref() else {
            continue;
        };
        if analysis.truncated {
            out.losses.push(Loss {
                location: format!("{sheet_q}!{range}"),
                what: format!(
                    "time limit reached while the column facts of the table {range} were counted; it has no Columns table"
                ),
                why: LossReason::Uncertain,
                severity: Severity::Info,
            });
            continue;
        }
        let body = region.body.unwrap_or(data);
        let left = body.left as usize;
        // A table that reaches the last shown column also has the fields not shown.
        let mut right = body.right as usize;
        if right + 1 >= MAX_TABLE_COLUMNS {
            right = right.max(part.widest.saturating_sub(1));
        }
        let known = analysis.columns.len();
        // A header band of several rows names the columns by their paths.
        let path = |j: usize| -> Option<String> {
            if region.header_rows.len() < 2 {
                return None;
            }
            let p = region.header_paths.get(j.checked_sub(left)?)?;
            let p = p.split(['\n', '\r']).collect::<Vec<_>>().join(" ");
            let p = p.trim();
            (!p.is_empty()).then(|| p.to_string())
        };
        let name = |j: usize| -> String {
            path(j).unwrap_or_else(|| match analysis.columns.get(j) {
                Some(c) => c.name.clone(),
                None => format!("Column {}", j + 1),
            })
        };
        // Fields of columns that only ragged rows have, counted in the data rows shown.
        let mut extra = vec![0usize; right.saturating_add(1).saturating_sub(known.max(left))];
        let lo = part.filled.partition_point(|p| p.0 < data.top);
        let hi = part.filled.partition_point(|p| p.0 <= data.bottom);
        for &(_, col) in &part.filled[lo..hi] {
            let col = col as usize;
            if col >= known.max(left)
                && col <= right
                && let Some(n) = extra.get_mut(col - known.max(left))
            {
                *n += 1;
            }
        }
        let listed = right.min(left + MAX_FACT_ROWS - 1);
        let facts: Vec<(Vec<String>, bool)> = (left..=listed)
            .map(|j| match analysis.columns.get(j) {
                Some(c) => (fact_row(c, name(j)), false),
                None => {
                    let filled = extra.get(j - known.max(left)).copied().unwrap_or(0);
                    (ragged_row(j, filled, shown), true)
                }
            })
            .collect();
        let caption = format!("Columns {range}");
        if listed < right {
            out.losses.push(Loss {
                location: sheet_q.to_string(),
                what: format!(
                    "columns {} to {} not listed in {caption}",
                    listed + 2,
                    right + 1
                ),
                why: LossReason::Limit,
                severity: Severity::Info,
            });
        }
        out.tables.push(columns_block(caption, facts));
        let inside = |j: usize| (left..=right).contains(&j);
        for d in &analysis.derived {
            if inside(d.target) && d.parts.iter().all(|&j| inside(j)) {
                out.formulas.push(derived_formula(sheet_q, d, &name));
            }
        }
    }
    if !not_counted.is_empty() {
        let n = not_counted.len();
        let mut named: Vec<String> = not_counted.into_iter().take(10).collect();
        if n > named.len() {
            named.push(format!("{} more", n - named.len()));
        }
        out.losses.push(Loss {
            location: sheet_q.to_string(),
            what: format!(
                "time limit reached before the column facts were counted; no Columns table for the tables {}",
                named.join(", ")
            ),
            why: LossReason::Uncertain,
            severity: Severity::Info,
        });
    }
    Some(out)
}

/// The Columns table row of a column csvsniff analyzed, named `name`.
pub(super) fn fact_row(c: &csvsniff::Column, name: String) -> Vec<String> {
    let unique = if c.unique {
        "yes"
    } else if c.unique_exhausted {
        "not checked"
    } else {
        "no"
    };
    vec![
        name,
        kind_label(&c.kind),
        c.unit.clone().unwrap_or_default(),
        c.missing.to_string(),
        unique.to_string(),
        if c.key_like { "yes" } else { "no" }.to_string(),
    ]
}

/// The Columns table row of column `j` (0-based) that only ragged rows have: `filled` of the
/// `rows` data rows have a field there.
pub(super) fn ragged_row(j: usize, filled: usize, rows: usize) -> Vec<String> {
    vec![
        format!("Column {}", j + 1),
        if filled > 0 { "unknown" } else { "empty" }.to_string(),
        String::new(),
        rows.saturating_sub(filled).to_string(),
        "not checked".to_string(),
        "no".to_string(),
    ]
}

/// A Columns table with `caption`. Each fact says whether its column is only in ragged rows;
/// when one is, the table has a Note column that says so.
pub(super) fn columns_block(caption: String, facts: Vec<(Vec<String>, bool)>) -> Block {
    let note = facts.iter().any(|(_, ragged)| *ragged);
    let mut head = vec!["Column", "Type", "Unit", "Missing", "Unique", "Key"];
    if note {
        head.push("Note");
    }
    let cells = |values: &[String]| -> Vec<Cell> {
        values
            .iter()
            .map(|v| Cell::new(vec![Inline::plain(v)]))
            .collect()
    };
    let mut rows = vec![
        head.iter()
            .map(|h| Cell::new(vec![Inline::plain(*h)]))
            .collect::<Vec<_>>(),
    ];
    for (mut values, ragged) in facts {
        if note {
            values.push(if ragged { "only in ragged rows" } else { "" }.to_string());
        }
        rows.push(cells(&values));
    }
    Block::Table {
        caption: Some(caption),
        header_rows: 1,
        role: Some("summary".into()),
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: None,
    }
}

/// An observed sum relation as a formula on whole columns; `name` names a column by its
/// 0-based index.
pub(super) fn derived_formula(
    sheet_q: &str,
    d: &csvsniff::Derived,
    name: &dyn Fn(usize) -> String,
) -> Formula {
    let col_anchor = |j: usize| Anchor(format!("{sheet_q}!{0}:{0}", column_letter(j)));
    let parts: Vec<String> = d.parts.iter().map(|&j| name(j)).collect();
    Formula {
        anchor: col_anchor(d.target),
        expression: format!("{} = {}", name(d.target), parts.join(" + ")),
        dialect: "derived".into(),
        references: d.parts.iter().map(|&j| col_anchor(j)).collect(),
        cached: None,
        role: Some(Role::Result),
        pattern: None,
        latex: None,
        note: Some(format!("holds {} of {} rows", d.holds, d.checked)),
        kind: None,
        range: None,
        original: None,
    }
}

/// Whether sheet row `row` is a data row of a table or grid region: inside its data rows
/// and with no row kind or the kind `data`. Header rows, group rows, caption, heading and
/// note lines and the rows of label and value, text or title regions are not.
pub(super) fn in_table_row(layout: &Layout, row: u32) -> bool {
    layout.regions.iter().any(|r| {
        matches!(r.kind, Kind::Table | Kind::Grid)
            && r.data.is_some_and(|d| d.top <= row && row <= d.bottom)
            && !r.captions.contains(&row)
            && !r.notes.contains(&row)
            && !r.above_header.contains(&row)
            && !r.header_rows.contains(&row)
            && r.caption_heading != Some(row)
            && r.heading.is_none_or(|(h, _)| h != row)
            && r.rows
                .iter()
                .find(|n| n.row == row)
                .is_none_or(|n| n.kind == RowKind::Data)
    })
}

/// Puts the ragged-row losses that `keep` accepts (by the line a record starts on; the
/// count of rows not listed is always kept) at `at` among the losses.
pub(super) fn place_ragged(
    doc: &mut Document,
    at: usize,
    ragged: Vec<(Option<usize>, Loss)>,
    keep: impl Fn(usize) -> bool,
) {
    let kept: Vec<Loss> = ragged
        .into_iter()
        .filter(|(line, _)| line.is_none_or(&keep))
        .map(|(_, loss)| loss)
        .collect();
    doc.losses.splice(at..at, kept);
}

fn kind_label(kind: &ColumnKind) -> String {
    match kind {
        ColumnKind::Integer => "integer".into(),
        ColumnKind::Decimal { places } => {
            let s = if *places == 1 { "" } else { "s" };
            format!("decimal ({places} place{s})")
        }
        ColumnKind::Date => "date".into(),
        ColumnKind::Bool => "bool".into(),
        ColumnKind::Text => "text".into(),
        ColumnKind::Empty => "empty".into(),
    }
}
