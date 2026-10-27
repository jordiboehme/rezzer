// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Grid plans from a read document.
//!
//! A document with page geometry gives one plan per page from its drawn boxes: rules and
//! frames become walls, pictures blocks and headings towers. Without it, every document gets
//! a generic plan from its block tree alone, so any format gives a grid. A sheet with tables
//! refines its region: the cells of its tables become a cell grid and its formula cells
//! towers.

use std::borrow::Cow;
use std::collections::HashMap;

use rezzer_model::{
    Block, BoxKind, Cell, Document, Formula, PageGeometry, PlacedBox, SectionKind, a1_position,
    plain_text, sheet_cell,
};

use super::{MAX_SHAPES, Plan, Shape, ShapeKind, builtin, fit};

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
/// The share of its page above which a picture is drawn as an outline, not a block.
const LARGE_PICTURE: f32 = 0.25;
/// The least height of a heading tower, in unit coordinates.
const TOWER_HEIGHT: f32 = 0.02;
/// The widest heading tower of depth 1; each deeper depth is 0.1 narrower, down to
/// [`NARROWEST_TOWER`].
const WIDEST_TOWER: f32 = 0.6;
const NARROWEST_TOWER: f32 = 0.2;
/// The most pages with shapes whose plans are tried, so many crowded pages stay fast.
const PAGES_TRIED: usize = 4 * MAX_PLANS;
/// How close, in unit coordinates, two cell edges are to count as one, and how far a cell
/// may stick out of its table.
const EDGE: f32 = 1e-3;

/// The plans of a run: the file name for the status line, the plans in round order and, when
/// the file gives no grid, a note for the status line and the same note without the file name
/// for the message of the first round.
#[derive(Debug, Clone, PartialEq)]
pub struct Rounds {
    pub name: String,
    pub plans: Vec<Plan>,
    pub note: Option<String>,
    pub short_note: Option<String>,
}

/// The plans of `doc`, at most [`MAX_PLANS`]; empty when it has no content.
///
/// When the page geometry of `doc` gives a page plan that fits an 80 by 40 grid, those page
/// plans are all: geometry wins over text. Otherwise each section of the body is a region,
/// and so is each run of other blocks of the body; without sections the body is one region.
/// A sheet section with tables gives one plan of its cells when that plan fits an 80 by 40
/// grid. Every other region gives chunks of 16 items.
pub fn plans(doc: &Document) -> Vec<Plan> {
    let pages = page_plans(doc);
    if !pages.is_empty() {
        return pages;
    }
    let mut out = Vec::new();
    let body = &doc.body;
    let mut i = 0;
    // The sections of each kind so far, for labels such as `slide 2`.
    // A list keyed by kind, so a new kind needs no change here.
    let mut seen: Vec<(SectionKind, usize)> = Vec::new();
    let mut formulas = None;
    while i < body.len() && out.len() < MAX_PLANS {
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
        let k = match seen.iter_mut().find(|(s, _)| s == kind) {
            Some((_, k)) => {
                *k += 1;
                *k
            }
            None => {
                seen.push((*kind, 1));
                1
            }
        };
        let title = title.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let base = format!(
            "{} {}",
            kind_name(*kind),
            title.map_or_else(|| k.to_string(), shorten)
        );
        if *kind == SectionKind::Sheet {
            let by_sheet = formulas.get_or_insert_with(|| formula_cells(&doc.facets.formulas));
            // Formula anchors name the sheet as its anchor does; the title is the fallback.
            let named = anchor.as_ref().map(|a| sheet_name(&a.0));
            let mut cells: Vec<(u32, u32)> = Vec::new();
            if let Some(name) = &named {
                cells.extend(by_sheet.get(name.as_ref()).into_iter().flatten());
            }
            if let Some(t) = title.filter(|&t| named.as_deref() != Some(t)) {
                cells.extend(by_sheet.get(t).into_iter().flatten());
            }
            if let Some(plan) = sheet_plan(&base, blocks, &cells) {
                out.push(plan);
                continue;
            }
        }
        region(Some(&base), blocks, &mut out);
    }
    out.truncate(MAX_PLANS);
    out
}

/// The plans of the pages of `doc.geometry` in page order, at most [`MAX_PLANS`]: one per
/// page that gives a shape and whose plan fits a [`REFERENCE`] grid, out of the first
/// [`PAGES_TRIED`] pages that give a shape. Labelled `page <n> of <pages>`, `<pages>` from the
/// page count of the file when set, else (and never less than) the largest page number in the
/// geometry. Two pages with the same number, built past `Geometry::add_page` (which refuses
/// them), give two plans with the same label.
fn page_plans(doc: &Document) -> Vec<Plan> {
    let Some(geometry) = doc.geometry.as_deref() else {
        return Vec::new();
    };
    let mut pages: Vec<&PageGeometry> = geometry.pages.iter().collect();
    // Pages added past `Geometry::add_page` may come in any order.
    pages.sort_by_key(|p| p.page);
    let largest = pages.last().map_or(0, |p| p.page);
    let total = doc.properties.facts.pages.unwrap_or(0).max(largest);
    let mut out = Vec::new();
    let mut tried = 0;
    for page in pages {
        if out.len() >= MAX_PLANS || tried >= PAGES_TRIED {
            break;
        }
        let shapes = page_shapes(page);
        if shapes.is_empty() {
            continue;
        }
        tried += 1;
        let plan = Plan {
            label: format!("page {} of {total}", page.page),
            shapes,
        };
        if fit(&plan, REFERENCE.0, REFERENCE.1).is_ok() {
            out.push(plan);
        }
    }
    out
}

/// The shapes of one page in box order, at most [`MAX_SHAPES`]; none for a size that is not
/// finite and positive. A table is its outline, with the cells inside it as its inner borders
/// (see [`table_borders`]); a cell inside no table is an outline of its own.
fn page_shapes(page: &PageGeometry) -> Vec<Shape> {
    let [w, h] = page.size;
    if ![w, h].iter().all(|v| v.is_finite() && *v > 0.0) {
        return Vec::new();
    }
    let mapped: Vec<(BoxKind, Shape)> = page
        .boxes
        .iter()
        .filter_map(|b| page_shape(b, w, h).map(|s| (b.kind, s)))
        .collect();
    let tables: Vec<usize> = (0..mapped.len())
        .filter(|&i| mapped[i].0 == BoxKind::Table)
        .collect();
    // The cell edges of each table: x edges, then y edges.
    let mut edges: Vec<(Vec<f32>, Vec<f32>)> = vec![Default::default(); tables.len()];
    let mut owned = vec![false; mapped.len()];
    for (i, (kind, cell)) in mapped.iter().enumerate() {
        if *kind != BoxKind::Cell {
            continue;
        }
        // Cells usually follow their table: the last table before the cell is tried first.
        let before = tables.partition_point(|&t| t < i);
        let order = (0..before).rev().chain(before..tables.len());
        let Some(t) = order
            .into_iter()
            .find(|&t| contains(mapped[tables[t]].1.rect, cell.rect))
        else {
            continue;
        };
        owned[i] = true;
        let [x0, y0, x1, y1] = cell.rect;
        edges[t].0.extend([x0, x1]);
        edges[t].1.extend([y0, y1]);
    }
    let mut out = Vec::new();
    let mut table = 0;
    for (i, (kind, shape)) in mapped.iter().enumerate() {
        if out.len() >= MAX_SHAPES {
            break;
        }
        if owned[i] {
            continue;
        }
        out.push(*shape);
        if *kind == BoxKind::Table {
            let (xs, ys) = &edges[table];
            table_borders(shape.rect, xs, ys, &mut out);
            table += 1;
        }
    }
    out.truncate(MAX_SHAPES);
    out
}

/// True when `inner` lies in `outer`, give or take [`EDGE`].
fn contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
    inner[0] >= outer[0] - EDGE
        && inner[1] >= outer[1] - EDGE
        && inner[2] <= outer[2] + EDGE
        && inner[3] <= outer[3] + EDGE
}

/// Pushes the inner borders of a table in `rect` to `out`, as [`cell_grid`] draws them: its
/// cells' distinct `xs` and `ys` edges inside the rect become column and row borders, at most
/// [`COL_BANDS`] and [`ROW_BANDS`] bands of them.
fn table_borders(rect: [f32; 4], xs: &[f32], ys: &[f32], out: &mut Vec<Shape>) {
    let inner = |edges: &[f32], lo: f32, hi: f32, bands: u32| -> Vec<f32> {
        let mut v: Vec<f32> = edges
            .iter()
            .copied()
            .filter(|&e| e > lo + EDGE && e < hi - EDGE)
            .collect();
        v.sort_by(f32::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() <= EDGE);
        // `n` edges part `n + 1` bands; edge `k - 1` comes before band `k`.
        borders(count(v.len() + 1), bands)
            .map(|k| v[k as usize - 1])
            .collect()
    };
    let cols = inner(xs, rect[0], rect[2], COL_BANDS);
    let rows = inner(ys, rect[1], rect[3], ROW_BANDS);
    grid_borders(rect, &cols, &rows, out);
}

/// The shape of box `b` on a page of `w` by `h` points: its bounds mapped to the unit square
/// with y flipped and clipped to it. `None` for text, bounds that are not finite or are
/// reversed, a box off the page, or one that keeps no extent on it.
fn page_shape(b: &PlacedBox, w: f32, h: f32) -> Option<Shape> {
    let [x0, y0, x1, y1] = b.bounds;
    if !b.bounds.iter().all(|v| v.is_finite()) || x1 < x0 || y1 < y0 {
        return None;
    }
    // Finite over finite and positive is never NaN; a quotient that overflows to infinity
    // on a tiny page is off the page or clamped below.
    let (ux0, ux1) = (x0 / w, x1 / w);
    let (uy0, uy1) = (1.0 - y1 / h, 1.0 - y0 / h);
    // Off the page in x or in y: checked before clipping, so a rule beside the page does not
    // become a line on its edge.
    if ux1 < 0.0 || ux0 > 1.0 || uy1 < 0.0 || uy0 > 1.0 {
        return None;
    }
    let mut rect = [
        ux0.clamp(0.0, 1.0),
        uy0.clamp(0.0, 1.0),
        ux1.clamp(0.0, 1.0),
        uy1.clamp(0.0, 1.0),
    ];
    let (dx, dy) = (rect[2] - rect[0], rect[3] - rect[1]);
    let kind = match b.kind {
        BoxKind::Text => return None,
        BoxKind::Rule => ShapeKind::Line,
        BoxKind::Drawn | BoxKind::Table | BoxKind::Cell => ShapeKind::Outline,
        BoxKind::Picture if dx * dy > LARGE_PICTURE => ShapeKind::Outline,
        BoxKind::Picture => ShapeKind::Block,
        BoxKind::Heading(depth) => {
            let cut = WIDEST_TOWER - 0.1 * f32::from(depth.max(1) - 1);
            rect[2] = rect[2].min(rect[0] + cut.max(NARROWEST_TOWER));
            if dy < TOWER_HEIGHT {
                let mid = (rect[1] + rect[3]) / 2.0;
                let top = (mid - TOWER_HEIGHT / 2.0).clamp(0.0, 1.0 - TOWER_HEIGHT);
                rect[1] = top;
                rect[3] = top + TOWER_HEIGHT;
            }
            ShapeKind::Tower
        }
    };
    let empty = match kind {
        ShapeKind::Line => dx <= 0.0 && dy <= 0.0,
        ShapeKind::Tower => rect[2] <= rect[0],
        _ => dx <= 0.0 || dy <= 0.0,
    };
    (!empty).then_some(Shape { kind, rect })
}

/// The cells `(row, col)` of the formulas in cells, grouped by sheet name without quotes, in
/// one pass over all formulas.
fn formula_cells(formulas: &[Formula]) -> HashMap<Cow<'_, str>, Vec<(u32, u32)>> {
    let mut by_sheet: HashMap<Cow<'_, str>, Vec<(u32, u32)>> = HashMap::new();
    for cell in formulas
        .iter()
        .filter(|f| !f.is_name())
        .filter_map(|f| sheet_cell(&f.anchor.0))
    {
        by_sheet
            .entry(sheet_name(cell.sheet))
            .or_default()
            .push((cell.row, cell.col));
    }
    by_sheet
}

/// A sheet name as an anchor writes it, without its quotes: `'My Costs'` is `My Costs` and
/// `'Q1 ''24'` is `Q1 '24`. Borrowed unless a doubled quote must be undone.
fn sheet_name(sheet: &str) -> Cow<'_, str> {
    match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(inner) if inner.contains("''") => Cow::Owned(inner.replace("''", "'")),
        Some(inner) => Cow::Borrowed(inner),
        None => Cow::Borrowed(sheet),
    }
}

/// The plans for a run on `name`: the Grid without a file (`name` empty), the Grid and a
/// note when the file gives no grid (`why_not` says why), else the plans of `doc`.
pub fn rounds(name: &str, doc: Option<&Document>, why_not: Option<&str>) -> Rounds {
    let found = match (name.is_empty(), why_not, doc) {
        (false, None, Some(doc)) => plans(doc),
        _ => Vec::new(),
    };
    // A reason that ends with a full stop gets no second one.
    let why = (!name.is_empty() && found.is_empty()).then(|| {
        why_not
            .unwrap_or("nothing to build a grid from")
            .trim_end_matches('.')
    });
    let note = why.map(|why| format!("{name} gives no grid: {why}. Using the Grid."));
    let short_note = why.map(|why| format!("No grid from this file: {why}."));
    Rounds {
        name: name.to_string(),
        plans: if found.is_empty() {
            vec![builtin()]
        } else {
            found
        },
        note,
        short_note,
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
    let cols: Vec<f32> = borders(cols, bands.1)
        .map(|k| at(x0, x1, cols, k))
        .collect();
    let rows: Vec<f32> = borders(rows, bands.0)
        .map(|k| at(y0, y1, rows, k))
        .collect();
    grid_borders(rect, &cols, &rows, out);
}

/// Pushes the inner borders of a cell grid in `rect` to `out`: a full line at each column
/// border `x` of `cols`, and at each row border `y` of `rows` one piece per column band that
/// leaves [`GAP`] of the band free at both ends.
fn grid_borders(rect: [f32; 4], cols: &[f32], rows: &[f32], out: &mut Vec<Shape>) {
    let [x0, y0, x1, y1] = rect;
    let mut xs = vec![x0];
    for &x in cols {
        xs.push(x);
        out.push(Shape {
            kind: ShapeKind::Line,
            rect: [x, y0, x, y1],
        });
    }
    xs.push(x1);
    for &y in rows {
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
/// fit a [`REFERENCE`] grid. `formulas` are the sheet's formula cells `(row, col)`.
fn sheet_plan(label: &str, blocks: &[Block], formulas: &[(u32, u32)]) -> Option<Plan> {
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
    let inside = |&&(r, c): &&(u32, u32)| (r0..=r1).contains(&r) && (c0..=c1).contains(&c);
    let cells: Vec<(u32, u32)> = formulas.iter().filter(inside).copied().collect();
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
