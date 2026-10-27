// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables as Markdown: pipe tables and the grid of a model table.

use super::*;

/// A GFM table from cells that are already escaped single-line Markdown.
pub(crate) fn pipe_table(header: &[String], aligns: &[Align], rows: &[Vec<String>]) -> String {
    let width = rows
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(header.len()))
        .max()
        .unwrap_or(0)
        .max(1);
    let line = |cells: &[String]| {
        let mut s = String::from("|");
        for i in 0..width {
            s.push(' ');
            s.push_str(cells.get(i).map_or("", String::as_str));
            s.push_str(" |");
        }
        s
    };
    let mut out = line(header);
    out.push_str("\n|");
    for i in 0..width {
        out.push_str(match aligns.get(i).copied().unwrap_or(Align::None) {
            Align::None => " --- |",
            Align::Left => " :--- |",
            Align::Center => " :---: |",
            Align::Right => " ---: |",
        });
    }
    for row in rows {
        out.push('\n');
        out.push_str(&line(row));
    }
    out
}

/// A table shows at most this many columns.
pub(crate) const MAX_COLUMNS: usize = 1_000;

/// A table's rendered grid (rows times width) holds at most this many cells.
const MAX_CELLS: usize = 200_000;

/// The mark for a grid position a cell merged from above covers (U+3003, ditto mark).
const SAME_AS_ABOVE: &str = "\u{3003}";

/// The mark for a grid position a cell merged from the left covers.
const SAME_AS_LEFT: &str = "\u{2190}";

/// What a grid position of a rendered table shows: the cell that starts there, or a mark for
/// a position a merged cell covers. Each names the cell by its place in the table's texts.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spot {
    Own(usize),
    Above(usize),
    Left(usize),
}

impl Spot {
    pub(crate) fn cell(self) -> usize {
        match self {
            Spot::Own(k) | Spot::Above(k) | Spot::Left(k) => k,
        }
    }
}

/// A merged cell of a rendered table: its table row, its spans as placed, and the 0-based
/// sheet (row, column) of its anchor when it has a sheet anchor.
struct Merged {
    r: usize,
    cs: usize,
    rs: usize,
    at: Option<(u32, u32)>,
}

/// Where a cell of a rendered table stands: its first grid row and column, and the 0-based
/// sheet (row, column) of its anchor when it has a sheet anchor.
#[derive(Clone, Copy)]
struct Place {
    r: usize,
    c: usize,
    at: Option<(u32, u32)>,
    /// The cell in the model table: its row and its index in that row.
    source: (usize, usize),
    /// The rows and columns the cell covers as placed.
    rs: usize,
    cs: usize,
}

/// The 0-based sheet (row, column) of a cell's `Sheet!A1` anchor. The sheet name may hold a
/// `!` itself, so the address is what follows the last one.
fn sheet_position(cell: &Cell) -> Option<(u32, u32)> {
    sheet_cell(&cell.anchor.as_ref()?.0).map(|at| (at.row, at.col))
}

/// A table's cells placed on a grid, the one grid every table is written from. A merged
/// cell's text stands once, in its first position; the other positions of its first row say
/// "same as left", every position below its first row "same as above". Rows stop once the
/// grid would pass MAX_CELLS; the first row is always shown.
pub(crate) struct Grid {
    /// The escaped one-line text of each cell, in reading order.
    texts: Vec<String>,
    places: Vec<Place>,
    /// The shown rows.
    pub(crate) spots: Vec<Vec<Option<Spot>>>,
    /// The rows of the model table.
    pub(crate) rows: usize,
    /// The widest shown row.
    pub(crate) width: usize,
    /// The columns the cells ask for, past MAX_COLUMNS too.
    wanted: usize,
    merged: Vec<Merged>,
    /// True when a cell carries a sheet anchor: the table is a sheet's.
    sheet: bool,
}

impl Grid {
    pub(crate) fn new(rows: &[Vec<Cell>]) -> Grid {
        // The real column count: per row the sum of its spans where that is plausible, else
        // its number of cells. Spans never reach past it.
        let real = rows
            .iter()
            .map(|row| {
                let sum = row
                    .iter()
                    .map(|c| c.col_span.max(1) as usize)
                    .fold(0usize, usize::saturating_add);
                if sum <= MAX_COLUMNS { sum } else { row.len() }
            })
            .max()
            .unwrap_or(1)
            .clamp(1, MAX_COLUMNS);
        let mut grid: Vec<Vec<Option<Spot>>> = vec![Vec::new(); rows.len()];
        let mut texts: Vec<String> = Vec::new();
        let mut places: Vec<Place> = Vec::new();
        let mut merged: Vec<Merged> = Vec::new();
        let mut filled = 0usize;
        let mut width = 0usize;
        let mut wanted = 0usize;
        let mut shown = rows.len();
        for (r, row) in rows.iter().enumerate() {
            let mut c = 0;
            for (i, cell) in row.iter().enumerate() {
                while grid[r].get(c).is_some_and(Option::is_some) {
                    c += 1;
                }
                if c >= MAX_COLUMNS {
                    wanted = wanted.max(c + 1);
                    c += 1;
                    continue;
                }
                let rs = (cell.row_span.max(1) as usize).min(rows.len() - r);
                let cs = (cell.col_span.max(1) as usize)
                    .min(real.saturating_sub(c).max(1))
                    .min(MAX_COLUMNS - c);
                // Past the cell budget a span fills only its own cell; the note still names
                // it.
                let (fill_rows, fill_cols) = if filled.saturating_add(rs * cs) > MAX_CELLS {
                    (1, 1)
                } else {
                    (rs, cs)
                };
                filled += fill_rows * fill_cols;
                let k = texts.len();
                texts.push(inlines(&cell.inlines, Mode::Cell).trim().to_string());
                let at = sheet_position(cell);
                places.push(Place {
                    r,
                    c,
                    at,
                    source: (r, i),
                    rs,
                    cs,
                });
                for (dr, grid_row) in grid[r..r + fill_rows].iter_mut().enumerate() {
                    if grid_row.len() < c + fill_cols {
                        grid_row.resize(c + fill_cols, None);
                    }
                    for (dc, slot) in grid_row[c..c + fill_cols].iter_mut().enumerate() {
                        *slot = Some(match (dr, dc) {
                            (0, 0) => Spot::Own(k),
                            (0, _) => Spot::Left(k),
                            _ => Spot::Above(k),
                        });
                    }
                }
                if rs > 1 || cs > 1 {
                    merged.push(Merged { r, cs, rs, at });
                }
                c += cs;
                wanted = wanted.max(c);
            }
            width = width.max(grid[r].len());
            if r > 0 && (r + 1).saturating_mul(width.max(1)) > MAX_CELLS {
                shown = r;
                break;
            }
        }
        grid.truncate(shown);
        let sheet = rows
            .iter()
            .flatten()
            .any(|c| c.anchor.as_ref().is_some_and(|a| a.0.contains('!')));
        Grid {
            texts,
            places,
            spots: grid,
            rows: rows.len(),
            width,
            wanted,
            merged,
            sheet,
        }
    }

    /// A sheet's table (its cells carry sheet anchors) names its merged ranges as the sheet
    /// does, from each merged cell's own anchor and its spans; only rows that are shown are
    /// named, a range into hidden rows ends at the last one, and a merged cell without a
    /// sheet anchor is not named. Other tables name none.
    pub(crate) fn ranges(&self) -> Vec<String> {
        self.ranges_from(0)
    }

    /// [`Grid::ranges`] of the merged cells that reach shown row `from` or a row after it.
    pub(crate) fn ranges_from(&self, from: usize) -> Vec<String> {
        if !self.sheet {
            return Vec::new();
        }
        let shown = self.spots.len();
        self.merged
            .iter()
            .filter(|m| m.r < shown && m.r + m.rs > from)
            .filter_map(|m| self.range_text(m.at?, m.r, m.rs, m.cs))
            .collect()
    }

    /// The sheet range of a merged cell anchored at `at` (0-based sheet row and column) that
    /// starts in shown row `r` and covers `rs` rows and `cs` columns; it ends at the last
    /// shown row.
    fn range_text(&self, at: (u32, u32), r: usize, rs: usize, cs: usize) -> Option<String> {
        let shown = self.spots.len();
        if r >= shown {
            return None;
        }
        let (row, col) = (at.0 as usize, at.1 as usize);
        Some(format!(
            "{}{}:{}{}",
            column_letter(col),
            row + 1,
            column_letter(col + cs - 1),
            row + rs.min(shown - r)
        ))
    }

    /// The cell `k` stands for in the model table: its row and its index in that row.
    pub(crate) fn source(&self, k: usize) -> (usize, usize) {
        self.places[k].source
    }

    /// The sheet range of cell `k` when it is merged and has a sheet anchor, as
    /// [`Grid::ranges`] names it.
    pub(crate) fn merged_range(&self, k: usize) -> Option<String> {
        let p = self.places[k];
        if !self.sheet || (p.rs < 2 && p.cs < 2) {
            return None;
        }
        self.range_text(p.at?, p.r, p.rs, p.cs)
    }

    /// True when a shown row from `from` on holds a mark.
    pub(crate) fn marked(&self, from: usize) -> bool {
        self.spots.iter().skip(from).any(|row| {
            row.iter()
                .any(|s| matches!(s, Some(Spot::Above(_) | Spot::Left(_))))
        })
    }

    /// The text of cell `k`. A real cell whose whole text is a mark is escaped where the
    /// table carries marks (`legend`), so it is never read as one; elsewhere it is written as
    /// it is.
    pub(crate) fn own(&self, k: usize, legend: bool) -> String {
        let t = &self.texts[k];
        if legend && (t == SAME_AS_ABOVE || t == SAME_AS_LEFT) {
            format!("\\{t}")
        } else {
            t.clone()
        }
    }

    /// The cells of shown row `r` as written.
    pub(crate) fn line(&self, r: usize, legend: bool) -> Vec<String> {
        self.spots[r]
            .iter()
            .map(|spot| match spot {
                None => String::new(),
                Some(Spot::Own(k)) => self.own(*k, legend),
                Some(Spot::Above(_)) => SAME_AS_ABOVE.to_string(),
                Some(Spot::Left(_)) => SAME_AS_LEFT.to_string(),
            })
            .collect()
    }

    /// The 1-based sheet row of shown row `r`, from the anchor of a cell that starts in or
    /// covers it (a merged cell covers consecutive sheet rows). `None` when nothing there has
    /// a sheet anchor.
    pub(crate) fn sheet_row(&self, r: usize) -> Option<u32> {
        self.spots[r].iter().flatten().find_map(|spot| {
            let p = self.places[spot.cell()];
            let (row, _) = p.at?;
            let offset = u32::try_from(r - p.r).ok()?;
            row.checked_add(offset)?.checked_add(1)
        })
    }

    /// The 0-based sheet column of the grid's first column, from the anchor of a placed cell.
    pub(crate) fn left_column(&self) -> Option<u32> {
        self.places.iter().find_map(|p| {
            let (_, col) = p.at?;
            col.checked_sub(u32::try_from(p.c).ok()?)
        })
    }

    /// The legend of the marks, with the merged ranges when there are any.
    pub(crate) fn merge_note(&self, legend: bool, ranges: &[String]) -> Option<String> {
        if !legend {
            return None;
        }
        let mut note =
            format!("Merged cells: {SAME_AS_ABOVE} same as above, {SAME_AS_LEFT} same as left.");
        if !ranges.is_empty() {
            note.push_str(&format!(" Ranges: {}", ranges.join(", ")));
        }
        Some(note)
    }

    /// The notes on rows and columns the grid does not show.
    pub(crate) fn cut_notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.spots.len() < self.rows {
            notes.push(format!(
                "{} more rows not shown (see the model JSON).",
                self.rows - self.spots.len()
            ));
        }
        if self.wanted > MAX_COLUMNS {
            notes.push(format!(
                "{} more columns not shown (see the model JSON).",
                self.wanted - MAX_COLUMNS
            ));
        }
        notes
    }
}

/// A table without a sheet layout: a Markdown, PDF or CSV table, or one of a sheet read
/// without layout.
pub(super) fn model_table(
    caption: Option<&str>,
    header_rows: usize,
    rows: &[Vec<Cell>],
    aligns: &[Align],
) -> String {
    if rows.is_empty() {
        return caption
            .filter(|c| !c.trim().is_empty())
            .map(|c| format!("**{}** (empty table)", line_text(c.trim())))
            .unwrap_or_default();
    }
    let grid = Grid::new(rows);
    let ranges = grid.ranges();
    // Several header rows are one header line, so their marks are never shown; the marks of
    // the other rows are, and then the legend explains them.
    let head = header_rows.min(grid.spots.len());
    let marked = grid.marked(if head < 2 { 0 } else { head });
    let legend = marked || !ranges.is_empty();
    // Several header rows are one header line: per column the texts of the cells over it,
    // top down, a cell over several rows once.
    let header: Vec<String> = match head {
        0 => Vec::new(),
        1 => grid.line(0, legend),
        _ => (0..grid.width)
            .map(|c| {
                let mut parts: Vec<String> = Vec::new();
                let mut last = None;
                for row in &grid.spots[..head] {
                    let Some(spot) = row.get(c).copied().flatten() else {
                        continue;
                    };
                    if last == Some(spot.cell()) {
                        continue;
                    }
                    last = Some(spot.cell());
                    let t = grid.own(spot.cell(), legend);
                    if !t.is_empty() {
                        parts.push(t);
                    }
                }
                parts.join(" ")
            })
            .collect(),
    };
    let body: Vec<Vec<String>> = (head..grid.spots.len())
        .map(|r| grid.line(r, legend))
        .collect();
    let mut out = String::new();
    if let Some(caption) = caption.filter(|c| !c.trim().is_empty()) {
        out.push_str(&format!("**{}**\n\n", line_text(caption.trim())));
    }
    out.push_str(&pipe_table(&header, aligns, &body));
    let mut notes: Vec<String> = Vec::new();
    notes.extend(grid.merge_note(legend, &ranges));
    notes.extend(grid.cut_notes());
    if !notes.is_empty() {
        out.push_str("\n\n");
        out.push_str(&notes.join("\n"));
    }
    out
}
