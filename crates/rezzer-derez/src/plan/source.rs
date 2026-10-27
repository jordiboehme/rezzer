// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Grid plans from a read document.
//!
//! Every document gets a generic plan from its block tree alone, so any format gives a grid.
//! A sheet with tables refines its region: the cells of its tables become a cell grid and its
//! formula cells towers.

use rezzer_model::{
    Block, Cell, Document, Formula, SectionKind, a1_position, plain_text, sheet_cell,
};

use super::{Plan, Shape, ShapeKind, builtin, fit};

/// The most plans one document gives.
pub const MAX_PLANS: usize = 50;
/// The most items of one plan; a longer region is split into chunks.
const CHUNK: usize = 16;
/// The rows and columns of a sheet that count, from the top left of its tables.
const SHEET_ROWS: u32 = 200;
const SHEET_COLS: u32 = 60;
/// The most row and column bands of a cell grid that fills a region.
const ROW_BANDS: u32 = 8;
const COL_BANDS: u32 = 6;
/// The most row and column bands of a table among other items.
const ITEM_ROW_BANDS: u32 = 5;
const ITEM_COL_BANDS: u32 = 4;
/// The share of a column band left free at each end of a row border piece, so the cells of a
/// grid stay open to each other beside every column border.
const GAP: f32 = 0.25;
/// The most formula towers of one sheet; more are thinned out evenly.
const MAX_TOWERS: usize = 64;
/// The longest label part taken from the document, in characters.
const LABEL_CHARS: usize = 30;
/// The grid size a sheet plan must fit to be used instead of the generic plan.
const REFERENCE: (u16, u16) = (80, 40);
/// Where bars start on the left and end on the right.
const LEFT: f32 = 0.05;
const RIGHT: f32 = 0.95;

/// The plans of a run: the file name for the status line, the plans in round order and a note
/// when the file gives no grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Rounds {
    pub name: String,
    pub plans: Vec<Plan>,
    pub note: Option<String>,
}

/// The plans of `doc`, at most [`MAX_PLANS`]; empty when it has no content.
///
/// Each section of the body is a region, and so is each run of other blocks of the body;
/// without sections the body is one region. A sheet section with tables gives one plan of its
/// cells when that plan fits an 80 by 40 grid. Every other region gives chunks of 16 items.
pub fn plans(doc: &Document) -> Vec<Plan> {
    let mut out = Vec::new();
    let body = &doc.body;
    let (mut i, mut k) = (0, 0);
    while i < body.len() && out.len() < MAX_PLANS {
        k += 1;
        let Block::Section {
            kind,
            title,
            anchor,
            blocks,
        } = &body[i]
        else {
            let end = body[i..]
                .iter()
                .position(|b| matches!(b, Block::Section { .. }))
                .map_or(body.len(), |p| i + p);
            region(None, &body[i..end], &mut out);
            i = end;
            continue;
        };
        i += 1;
        let title = title.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let base = format!(
            "{} {}",
            kind_name(*kind),
            title.map_or_else(|| k.to_string(), shorten)
        );
        if *kind == SectionKind::Sheet {
            let key = anchor.as_ref().map(|a| a.0.as_str()).or(title);
            if let Some(plan) = sheet_plan(&base, blocks, key, title, &doc.facets.formulas) {
                out.push(plan);
                continue;
            }
        }
        region(Some(&base), blocks, &mut out);
    }
    out.truncate(MAX_PLANS);
    out
}

/// The plans for a run on `name`: the Grid without a file (`name` empty), the Grid and a
/// note when the file gives no grid (`why_not` says why), else the plans of `doc`.
pub fn rounds(name: &str, doc: Option<&Document>, why_not: Option<&str>) -> Rounds {
    let found = match (name.is_empty(), why_not, doc) {
        (false, None, Some(doc)) => plans(doc),
        _ => Vec::new(),
    };
    let note = (!name.is_empty() && found.is_empty()).then(|| {
        let why = why_not.unwrap_or("nothing to build a grid from");
        format!("{name} gives no grid: {why}. Using the Grid.")
    });
    Rounds {
        name: name.to_string(),
        plans: if found.is_empty() {
            vec![builtin()]
        } else {
            found
        },
        note,
    }
}

/// How a section kind reads in a label.
fn kind_name(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Chapter => "chapter",
        SectionKind::Sheet => "sheet",
        SectionKind::Slide => "slide",
        SectionKind::Page => "page",
        SectionKind::Message => "message",
        SectionKind::Part => "part",
        SectionKind::Layer => "layer",
        SectionKind::Other => "section",
    }
}

/// At most [`LABEL_CHARS`] characters of `text`, trimmed.
fn shorten(text: &str) -> String {
    let short: String = text.trim().chars().take(LABEL_CHARS).collect();
    short.trim_end().to_string()
}

/// One item of the generic plan.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Item {
    /// A heading: its characters and depth.
    Heading { chars: usize, depth: u8 },
    /// A paragraph or formula of this many characters.
    Text(usize),
    /// A list, quote or task run of this many entries.
    Run(usize),
    /// A thematic break.
    Rule,
    /// Code or a callout.
    Boxed,
    /// A picture or other file.
    Media,
    /// A table of this many rows and columns.
    Table { rows: u32, cols: u32 },
}

/// Pushes the generic plans of `blocks` to `out`: chunks of [`CHUNK`] items, labelled
/// `base` (with `, part <k> of <n>` when there are several), else by the chunk's first
/// heading, else `part <k> of <n>`.
fn region(base: Option<&str>, blocks: &[Block], out: &mut Vec<Plan>) {
    let room = MAX_PLANS.saturating_sub(out.len());
    let items = items(blocks, room * CHUNK);
    if items.is_empty() {
        return;
    }
    let lone = items.len() == 1;
    let n = items.len().div_ceil(CHUNK);
    for (c, chunk) in items.chunks(CHUNK).enumerate() {
        let label = match base {
            Some(base) if n == 1 => base.to_string(),
            Some(base) => format!("{base}, part {} of {n}", c + 1),
            None => chunk
                .iter()
                .find_map(|(_, heading)| heading.clone())
                .unwrap_or_else(|| format!("part {} of {n}", c + 1)),
        };
        let mut shapes = Vec::new();
        for (i, (item, _)) in chunk.iter().enumerate() {
            place(*item, i, chunk.len(), lone, &mut shapes);
        }
        out.push(Plan { label, shapes });
    }
}

/// The items of `blocks` in document order, nested sections opened, at most `cap`; with each
/// the label it gives when it is a heading.
fn items(blocks: &[Block], cap: usize) -> Vec<(Item, Option<String>)> {
    let mut items = Vec::new();
    // A stack, so sections nested to any depth cannot overflow the call stack.
    let mut stack = vec![blocks.iter()];
    let mut after_task = false;
    while items.len() < cap {
        let Some(top) = stack.last_mut() else {
            break;
        };
        let Some(block) = top.next() else {
            stack.pop();
            after_task = false;
            continue;
        };
        let was_task = std::mem::replace(&mut after_task, matches!(block, Block::Task { .. }));
        let mut heading = None;
        let item = match block {
            Block::Section { blocks, .. } => {
                stack.push(blocks.iter());
                continue;
            }
            Block::Task { .. } if was_task => {
                if let Some((Item::Run(entries), _)) = items.last_mut() {
                    *entries += 1;
                }
                continue;
            }
            Block::Task { .. } => Item::Run(1),
            Block::Heading { level, inlines, .. } => {
                let text = plain_text(inlines);
                heading = Some(shorten(&text)).filter(|t| !t.is_empty());
                Item::Heading {
                    chars: text.trim().chars().count(),
                    depth: (*level).max(1),
                }
            }
            Block::Paragraph { inlines, .. } => match plain_text(inlines).trim().chars().count() {
                0 => continue,
                chars => Item::Text(chars),
            },
            Block::Math { latex, .. } => Item::Text(latex.trim().chars().count()),
            Block::List { items, .. } => Item::Run(items.len()),
            Block::Quote { blocks, .. } => Item::Run(blocks.len()),
            Block::Code { .. } | Block::Callout { .. } => Item::Boxed,
            Block::Media { .. } => Item::Media,
            Block::Rule { .. } => Item::Rule,
            Block::Table { rows, .. } => Item::Table {
                rows: count(rows.len()),
                cols: count(rows.iter().map(|r| row_width(r)).max().unwrap_or(0)),
            },
        };
        items.push((item, heading));
    }
    items
}

/// The cells of one table row, spans counted.
fn row_width(row: &[Cell]) -> usize {
    row.iter().map(|c| c.col_span.max(1) as usize).sum()
}

/// `n` as `u32`, saturated.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Pushes the shapes of item `i` of `n` to `out`. Item `i` sits at y `(i + 0.5) / n`; bars
/// start at the left on even items and end at the right on odd ones. A `lone` table fills
/// the region. Heights scale with the slot of `1 / n`, so items of a full chunk stay apart.
fn place(item: Item, i: usize, n: usize, lone: bool, out: &mut Vec<Shape>) {
    let slot = 1.0 / n as f32;
    let y = (i as f32 + 0.5) * slot;
    let bar = |w: f32, h: f32| {
        let (x0, x1) = if i.is_multiple_of(2) {
            (LEFT, LEFT + w)
        } else {
            (RIGHT - w, RIGHT)
        };
        [x0, y - h / 2.0, x1, y + h / 2.0]
    };
    let centred = |w: f32, h: f32| [0.5 - w / 2.0, y - h / 2.0, 0.5 + w / 2.0, y + h / 2.0];
    let boxed = (0.6 * slot).min(0.12);
    let (kind, rect) = match item {
        Item::Heading { chars, depth } => {
            let w = (chars as f32 / 60.0).clamp(0.15, 0.4) - 0.05 * f32::from(depth - 1);
            (ShapeKind::Tower, bar(w.max(0.05), (0.25 * slot).min(0.03)))
        }
        Item::Text(chars) => {
            let w = (chars.min(1_000_000) as f32 / 400.0).clamp(0.08, 0.4);
            (ShapeKind::Line, bar(w, 0.0))
        }
        Item::Run(entries) => {
            let w = (0.1 + 0.03 * entries.min(1_000) as f32).clamp(0.1, 0.5);
            (ShapeKind::Line, bar(w, 0.0))
        }
        Item::Rule => (ShapeKind::Line, centred(0.8, 0.0)),
        Item::Boxed => (ShapeKind::Outline, centred(0.3, boxed)),
        Item::Media => (ShapeKind::Block, bar(0.1, (0.6 * slot).min(0.1))),
        Item::Table { rows, cols } if lone => {
            cell_grid(
                [0.0, 0.0, 1.0, 1.0],
                rows,
                cols,
                (ROW_BANDS, COL_BANDS),
                out,
            );
            return;
        }
        Item::Table { rows, cols } => {
            let w = (0.06 * cols.min(100) as f32).clamp(0.12, 0.24);
            let bands = (ITEM_ROW_BANDS, ITEM_COL_BANDS);
            cell_grid(centred(w, boxed), rows, cols, bands, out);
            return;
        }
    };
    out.push(Shape { kind, rect });
}

/// Pushes a cell grid of `rows` by `cols` in `rect` to `out`: an outline, a full line at each
/// shown column border, and at each shown row border one piece per column band that leaves
/// [`GAP`] of the band free at both ends. At most `bands` row and column bands are shown.
fn cell_grid(rect: [f32; 4], rows: u32, cols: u32, bands: (u32, u32), out: &mut Vec<Shape>) {
    let [x0, y0, x1, y1] = rect;
    out.push(Shape {
        kind: ShapeKind::Outline,
        rect,
    });
    let at = |lo: f32, hi: f32, n: u32, k: u32| lo + (hi - lo) * k as f32 / n as f32;
    let mut xs = vec![x0];
    for k in borders(cols, bands.1) {
        let x = at(x0, x1, cols, k);
        xs.push(x);
        out.push(Shape {
            kind: ShapeKind::Line,
            rect: [x, y0, x, y1],
        });
    }
    xs.push(x1);
    for k in borders(rows, bands.0) {
        let y = at(y0, y1, rows, k);
        for band in xs.windows(2) {
            let gap = GAP * (band[1] - band[0]);
            out.push(Shape {
                kind: ShapeKind::Line,
                rect: [band[0] + gap, y, band[1] - gap, y],
            });
        }
    }
}

/// The borders between `n` rows or columns that are shown, by how many come before each: all
/// of them for at most `bands`, else every `ceil(n / bands)`-th.
fn borders(n: u32, bands: u32) -> impl Iterator<Item = u32> {
    let step = if n <= bands { 1 } else { n.div_ceil(bands) };
    (step..n.max(1)).step_by(step as usize)
}

/// The cell grid plan of a sheet section, or `None` when it has no table or the plan does not
/// fit a [`REFERENCE`] grid. `key` is the sheet as formula anchors write it, `title` its name.
fn sheet_plan(
    label: &str,
    blocks: &[Block],
    key: Option<&str>,
    title: Option<&str>,
    formulas: &[Formula],
) -> Option<Plan> {
    let regions: Vec<[u32; 4]> = blocks.iter().filter_map(table_region).collect();
    let r0 = regions.iter().map(|r| r[0]).min()?;
    let c0 = regions.iter().map(|r| r[1]).min()?;
    let r1 = regions.iter().map(|r| r[2]).max()?.min(r0 + SHEET_ROWS - 1);
    let c1 = regions.iter().map(|r| r[3]).max()?.min(c0 + SHEET_COLS - 1);
    let (nr, nc) = (r1 - r0 + 1, c1 - c0 + 1);
    let x = |c: u32| (c - c0) as f32 / nc as f32;
    let y = |r: u32| (r - r0) as f32 / nr as f32;
    let mut shapes = Vec::new();
    for [a, b, c, d] in regions {
        if a > r1 || b > c1 {
            continue;
        }
        let (c, d) = (c.min(r1), d.min(c1));
        let rect = [x(b), y(a), x(d + 1), y(c + 1)];
        cell_grid(
            rect,
            c - a + 1,
            d - b + 1,
            (ROW_BANDS, COL_BANDS),
            &mut shapes,
        );
    }
    let on_sheet = |sheet: &str| {
        key == Some(sheet) || title.is_some_and(|t| unquote(sheet).is_some_and(|s| s == t))
    };
    let cells: Vec<(u32, u32)> = formulas
        .iter()
        .filter(|f| !f.is_name())
        .filter_map(|f| sheet_cell(&f.anchor.0))
        .filter(|c| on_sheet(c.sheet))
        .filter(|c| (r0..=r1).contains(&c.row) && (c0..=c1).contains(&c.col))
        .map(|c| (c.row, c.col))
        .collect();
    // Towers come after the grid, so the shape cap of `fit` never drops the grid.
    let step = cells.len().div_ceil(MAX_TOWERS).max(1);
    for &(r, c) in cells.iter().step_by(step) {
        shapes.push(Shape {
            kind: ShapeKind::Tower,
            rect: [x(c), y(r), x(c + 1), y(r + 1)],
        });
    }
    let plan = Plan {
        label: label.to_string(),
        shapes,
    };
    fit(&plan, REFERENCE.0, REFERENCE.1).is_ok().then_some(plan)
}

/// A quoted sheet name without its quotes: `'My Costs'` is `My Costs`, `'it''s'` is `it's`.
fn unquote(sheet: &str) -> Option<String> {
    let inner = sheet.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(inner.replace("''", "'"))
}

/// The cells `[row0, col0, row1, col1]` (0-based, inclusive) of a table block: its layout
/// range, else its rows and widest row from `A1`. `None` for no table or an empty one.
fn table_region(block: &Block) -> Option<[u32; 4]> {
    let Block::Table { rows, layout, .. } = block else {
        return None;
    };
    if let Some(cells) = layout.as_ref().and_then(|l| range(&l.range)) {
        return Some(cells);
    }
    let widest = rows.iter().map(|r| row_width(r)).max().unwrap_or(0);
    if rows.is_empty() || widest == 0 {
        return None;
    }
    Some([0, 0, count(rows.len()) - 1, count(widest) - 1])
}

/// The cells of an A1 range such as `A1:F68`, `B2` or `Sheet1!A1:F68`, bounds sorted.
fn range(text: &str) -> Option<[u32; 4]> {
    let text = text.rsplit_once('!').map_or(text, |(_, r)| r);
    let (a, b) = text.split_once(':').unwrap_or((text, text));
    let (ra, ca) = a1_position(a)?;
    let (rb, cb) = a1_position(b)?;
    Some([ra.min(rb), ca.min(cb), ra.max(rb), ca.max(cb)])
}
