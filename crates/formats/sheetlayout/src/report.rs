// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The report rules of spec 6.7 and 8 for a region without a header band: section heading
//! rows (R1, step 6d), label and value blocks (K, 6e), text blocks (X, 6f), header-less
//! tables (6g) and the grid (6h). Then the second passes: continuation below a gap (C, step
//! 7) and note regions below a table (N, step 8). Side regions stay (S).

use std::collections::BTreeMap;

use crate::captions;
use crate::detect::{self, Ctx, Halt, Line, Lines, Tick, keep, retain_panes};
use crate::header;
use crate::input::{Class, Rect, column_letters};
use crate::occupancy::Prep;
use crate::rows;
use crate::{Certainty, Evidence, Issue, IssueKind, Kind, Origin, Region, RowKind, RowNote, Work};

/// Empty rows a note region may stand below its table (rule N).
const NOTE_GAP: u32 = 2;
/// Empty rows a continuation may stand below its table (rule C).
const CONTINUATION_GAP: u32 = 1;

/// The index range of the lines in the rows `top` to `bottom`.
fn line_range(lines: &Lines, top: u32, bottom: u32) -> std::ops::Range<usize> {
    let lo = lines.first_at(top);
    let hi = lines.lines.partition_point(|l| l.row <= bottom);
    lo..hi.max(lo)
}

/// The bounds of the cells in the rows of `data`.
fn trimmed(lines: &Lines, data: Rect, tick: &Tick) -> Result<Option<Rect>, Halt> {
    let mut out: Option<Rect> = None;
    for l in &lines.lines[line_range(lines, data.top, data.bottom)] {
        if tick.step() {
            return Err(Halt::Stopped);
        }
        let cells = &lines.cells[l.from as usize..l.to as usize];
        let (Some(&a), Some(&b)) = (cells.first(), cells.last()) else {
            continue;
        };
        let (a, b) = (
            lines.prep.cell(a as usize).pos.col,
            lines.prep.cell(b as usize).pos.col,
        );
        let r = Rect {
            top: l.row,
            left: a,
            bottom: l.row,
            right: b,
        };
        out = Some(out.map_or(r, |o| o.union(&r)));
    }
    Ok(out)
}

/// Whether a region of caption rows only has one caption row whose text lies in a merged
/// area of more than one cell.
fn one_merged_text(lines: &Lines, w: &Work) -> bool {
    let [row] = w.region.captions[..] else {
        return false;
    };
    let prep = lines.prep;
    let r = w.region.rect;
    let Some(cell) = crate::captions::single_text(prep, &w.merges, row, r.left, r.right) else {
        return false;
    };
    w.merges.iter().any(|&m| {
        let m = &prep.merges[m as usize];
        m.contains(cell.pos) && (m.top != m.bottom || m.left != m.right)
    })
}

/// Steps 6d to 6h for a rule region whose rows hold no header band.
pub(crate) fn no_band(
    lines: &Lines,
    w: &mut Work,
    depth: u32,
    ctx: &mut Ctx,
) -> Result<Vec<Work>, Halt> {
    let input = lines.prep.input;
    let tick = ctx.tick;
    if w.region.data.is_none() && one_merged_text(lines, w) {
        // Its only value is one merged text (ruling P4-AQ): a title, never lines of text or
        // an empty grid.
        let region = &mut w.region;
        region.kind = Kind::Title;
        region.certainty = Certainty::Rule;
        region.unsure.clear();
        region.evidence.push(Evidence::Rule {
            id: "T2",
            detail: "title rows only".to_string(),
        });
        return Ok(Vec::new());
    }
    if w.region.data.is_none() {
        // A region of caption rows only that no table took: its rows are lines of text
        // (spec 6.3), not a title.
        let region = &mut w.region;
        region.captions.clear();
        region.caption_heading = None;
        region
            .evidence
            .retain(|e| !matches!(e, Evidence::Rule { id: "T", .. }));
        region.data = Some(lines.rect);
    }
    let data = w.region.data.unwrap_or(lines.rect);

    // 6d. Section heading rows.
    if depth <= ctx.limits.max_section_depth
        && let Some(pieces) = sections(lines, w, data, depth, ctx)?
    {
        return Ok(pieces);
    }

    let region = &mut w.region;
    region.header_rows.clear();
    region.header_paths.clear();
    region.body = None;
    let Some(t) = trimmed(lines, data, tick)? else {
        grid(region);
        retain_panes(input, lines.rect, region);
        return Ok(Vec::new());
    };

    // 6e. Label and value block.
    match key_value(lines, t, tick)? {
        Pairs::Yes { unit } => {
            let words = if unit {
                "label, value and unit columns"
            } else {
                "label and value pairs"
            };
            decided(region, Kind::KeyValue, t, "K", words);
            retain_panes(input, lines.rect, region);
            return Ok(Vec::new());
        }
        Pairs::NotUnits => {
            let words = format!("column {} is not a unit column", column_letters(t.left + 2));
            decided(region, Kind::Table, t, "K", &words);
            region.evidence.push(Evidence::Rule {
                id: "H0",
                detail: "no header found".to_string(),
            });
            headerless_rows(lines, w, t, ctx)?;
            retain_panes(input, lines.rect, &mut w.region);
            return Ok(Vec::new());
        }
        Pairs::No => {}
    }

    // 6f. Text block.
    if let Some(words) = text_block(lines, t, tick)? {
        decided(region, Kind::Text, t, "X", words);
        retain_panes(input, lines.rect, region);
        return Ok(Vec::new());
    }

    // 6g. A header-less table of values.
    let mut values = false;
    for l in &lines.lines[line_range(lines, t.top, t.bottom)] {
        if tick.step() {
            return Err(Halt::Stopped);
        }
        if lines
            .cells_of(l)
            .any(|c| matches!(c.class, Class::Number | Class::Date | Class::Bool))
        {
            values = true;
            break;
        }
    }
    if values {
        decided(region, Kind::Table, t, "H0", "no header found");
        headerless_rows(lines, w, t, ctx)?;
    } else {
        // 6h.
        grid(region);
    }
    retain_panes(input, lines.rect, &mut w.region);
    Ok(Vec::new())
}

/// Step 6c on a header-less table, read in its own columns `t`, so its rows get kinds and
/// notes whether or not a part joins it later.
fn headerless_rows(lines: &Lines, w: &mut Work, t: Rect, ctx: &mut Ctx) -> Result<(), Halt> {
    let cells = lines.cells_in(t.top, t.bottom);
    let Some(own) = Lines::new(lines.prep, t, cells, &w.merges, ctx.tick) else {
        return Err(Halt::Stopped);
    };
    if rows::table_rows(&own, &mut w.region, None, false, ctx.outline, ctx.tick) {
        Ok(())
    } else {
        Err(Halt::Stopped)
    }
}

fn grid(region: &mut Region) {
    region.kind = Kind::Grid;
    region.certainty = Certainty::Unsure;
    region.unsure = vec!["no header found".to_string()];
}

fn decided(region: &mut Region, kind: Kind, data: Rect, id: &'static str, words: &str) {
    region.kind = kind;
    region.certainty = Certainty::Rule;
    region.unsure.clear();
    region.data = Some(data);
    region.label_cols = 0;
    region.rows.clear();
    region.evidence.push(Evidence::Rule {
        id,
        detail: words.to_string(),
    });
}

/// What rule K found.
enum Pairs {
    No,
    Yes {
        unit: bool,
    },
    /// Three columns whose third is not a unit column: a table.
    NotUnits,
}

/// Rule K: 2 or 3 columns, at least 2 rows, a different text label in the first column of
/// every row (the label callback agrees), a value in the second column of at least one row,
/// and a third column only when the unit callback accepts its texts.
fn key_value(lines: &Lines, t: Rect, tick: &Tick) -> Result<Pairs, Halt> {
    let input = lines.prep.input;
    let width = t.width();
    if !(2..=3).contains(&width) {
        return Ok(Pairs::No);
    }
    let range = line_range(lines, t.top, t.bottom);
    if range.len() < 2 {
        return Ok(Pairs::No);
    }
    let mut labels: Vec<&str> = Vec::with_capacity(range.len());
    let mut units: Vec<&str> = Vec::new();
    let mut value = false;
    for l in &lines.lines[range] {
        if tick.step() {
            return Err(Halt::Stopped);
        }
        let Some(label) = lines.cell_at(l.row, t.left) else {
            return Ok(Pairs::No);
        };
        if label.class != Class::Text || label.text.is_empty() || !(input.is_label)(&label.text) {
            return Ok(Pairs::No);
        }
        labels.push(&label.text);
        value |= lines
            .cell_at(l.row, t.left + 1)
            .is_some_and(|c| !c.text.is_empty());
        if width == 3
            && let Some(c) = lines.cell_at(l.row, t.left + 2)
            && !c.text.is_empty()
        {
            units.push(&c.text);
        }
    }
    if !value {
        return Ok(Pairs::No);
    }
    labels.sort_unstable();
    if labels.windows(2).any(|w| w[0] == w[1]) {
        return Ok(Pairs::No);
    }
    if width == 3 {
        if (input.is_unit_column)(&units) {
            return Ok(Pairs::Yes { unit: true });
        }
        return Ok(Pairs::NotUnits);
    }
    Ok(Pairs::Yes { unit: false })
}

/// Rule X: only text, in one column or in rows of one merged area each.
fn text_block(lines: &Lines, t: Rect, tick: &Tick) -> Result<Option<&'static str>, Halt> {
    let mut merged = true;
    for l in &lines.lines[line_range(lines, t.top, t.bottom)] {
        if tick.step() {
            return Err(Halt::Stopped);
        }
        if lines
            .cells_of(l)
            .any(|c| c.class != Class::Text || c.text.is_empty())
        {
            return Ok(None);
        }
        merged &= l.merges == 1 && lines.single_text(l).is_some();
    }
    Ok(if t.width() == 1 {
        Some("one column of text")
    } else if merged {
        Some("rows of one merged area each")
    } else {
        None
    })
}

/// Rule R1 on one line: one text in the first column (or one merged area from it over at
/// least 80 % of the width), bold or filled where styles are read, or merged, and followed by
/// a row that holds cells and is not a single-text row itself.
///
/// The last condition reads "followed by a non-empty row" so that a run of single-text rows
/// (a list of merged lines, merged notes) is never cut into one section per row.
fn heading(lines: &Lines, i: usize, data: Rect) -> bool {
    let l = &lines.lines[i];
    let Some(cell) = lines.single_text(l) else {
        return false;
    };
    let merge = lines.merge_of(l);
    let wide = merge.is_some_and(|m| {
        let left = m.left.max(data.left);
        let right = m.right.min(data.right);
        m.left <= data.left && (u64::from(right) - u64::from(left) + 1) * 5 >= data.width() * 4
    });
    if cell.pos.col != data.left && !wide {
        return false;
    }
    let marked =
        cell.style.bold == Some(true) || cell.style.filled == Some(true) || merge.is_some();
    if !marked {
        return false;
    }
    let next: Option<&Line> = lines.lines.get(i + 1);
    next.is_some_and(|n| {
        u64::from(n.row) == u64::from(l.row) + 1
            && n.row <= data.bottom
            && lines.single_text(n).is_none()
    })
}

/// A new rule region for a section piece.
fn piece_region(parent: &Region, rect: Rect, head: u32, depth: u32) -> Region {
    let mut region = crate::rule_region(rect, "");
    region.evidence = parent
        .evidence
        .iter()
        .filter(|e| {
            matches!(
                e,
                Evidence::Rule { id: "G", .. } | Evidence::DefinedName { .. }
            )
        })
        .cloned()
        .collect();
    region.evidence.push(Evidence::Rule {
        id: "R1",
        detail: format!("section heading in {}", row_words(&[head])),
    });
    region.heading = Some((head, depth.min(u32::from(u8::MAX)) as u8));
    region.data = Some(Rect {
        top: head + 1,
        ..rect
    });
    region
}

fn row_words(rows: &[u32]) -> String {
    match rows {
        [] => String::new(),
        [one] => format!("row {}", u64::from(*one) + 1),
        [first, .., last] => header::row_words(*first, *last),
    }
}

/// Step 6d: cuts the region at its section heading rows. The rows before the first heading
/// stay the region itself; each heading starts a piece with its own caption rows, decided
/// again from step 6a. When no rows come before the first heading, the region's caption rows
/// become a `Title` region. Gives `None` when the region has no heading rows, or when the
/// pieces would pass the region limit (with a `Limit` issue).
fn sections(
    lines: &Lines,
    w: &mut Work,
    data: Rect,
    depth: u32,
    ctx: &mut Ctx,
) -> Result<Option<Vec<Work>>, Halt> {
    let prep = lines.prep;
    let tick = ctx.tick;
    let range = line_range(lines, data.top, data.bottom);
    if range.is_empty() {
        return Ok(None);
    }
    let mut heads: Vec<usize> = Vec::new();
    for i in range.clone() {
        if tick.step() {
            return Err(Halt::Stopped);
        }
        if heading(lines, i, data) {
            heads.push(i);
        }
    }
    let Some(&first) = heads.first() else {
        return Ok(None);
    };
    let before = first > range.start;
    let title = !before && !w.region.captions.is_empty();
    let total = heads.len() + usize::from(before) + usize::from(title);
    let extra = total.saturating_sub(1);
    if extra > ctx.room {
        ctx.issues.push(Issue {
            what: format!(
                "region limit of {} reached: the sections of {} are not split",
                ctx.limits.max_regions,
                w.region.rect.a1()
            ),
            kind: IssueKind::Limit,
            rect: None,
        });
        return Ok(None);
    }
    ctx.room -= extra;

    let mut out = Vec::new();
    // The region's merges by top row (indices into the sorted merge list are in top order),
    // walked once: each piece takes those that start in its rows or still reach into them.
    let mut by_top = w.merges.clone();
    by_top.sort_unstable();
    by_top.dedup();
    let mut next = 0usize;
    let mut open: Vec<u32> = Vec::new();
    for (k, &i) in heads.iter().enumerate() {
        let head = lines.lines[i].row;
        let end = heads.get(k + 1).copied().unwrap_or(range.end);
        let last = lines.lines[end - 1].row;
        let rect = Rect {
            top: head,
            left: data.left,
            bottom: last,
            right: data.right,
        };
        open.retain(|&m| prep.merges[m as usize].bottom >= rect.top);
        while next < by_top.len() && prep.merges[by_top[next] as usize].top <= rect.bottom {
            if tick.step() {
                return Err(Halt::Stopped);
            }
            if prep.merges[by_top[next] as usize].bottom >= rect.top {
                open.push(by_top[next]);
            }
            next += 1;
        }
        let mut merges: Vec<u32> = Vec::with_capacity(open.len());
        for &m in &open {
            if tick.step() {
                return Err(Halt::Stopped);
            }
            if prep.merges[m as usize].intersects(&rect) {
                merges.push(m);
            }
        }
        let mut piece = Work {
            region: piece_region(&w.region, rect, head, depth),
            merges,
            origin: Origin::Rule,
            dead: false,
            coarse: false,
            fact: None,
            cells: Vec::new(),
        };
        // Its own caption rows, on the rows after the heading.
        piece.region.rect = Rect {
            top: head + 1,
            ..rect
        };
        captions::caption_rows(prep, &mut piece);
        piece.region.rect = rect;
        let body = piece.region.data.unwrap_or(Rect {
            top: head + 1,
            ..rect
        });
        let cells = lines.cells_in(body.top, body.bottom);
        let Some(plines) = Lines::new(prep, body, cells, &piece.merges, tick) else {
            return Err(Halt::Stopped);
        };
        let more = detect::rule(&plines, &mut piece, depth + 1, ctx)?;
        let prect = piece.region.rect;
        piece.cells = keep(prep, plines.into_cells(), prect);
        if !piece.dead {
            out.push(piece);
        }
        out.extend(more);
    }

    if before {
        let last = lines.lines[first - 1].row;
        w.region.rect.bottom = last;
        let body = Rect {
            bottom: last,
            ..data
        };
        w.region.data = Some(body);
        let cells = lines.cells_in(body.top, body.bottom);
        let Some(plines) = Lines::new(prep, body, cells, &w.merges, tick) else {
            return Err(Halt::Stopped);
        };
        let more = detect::rule(&plines, w, depth + 1, ctx)?;
        out.extend(more);
    } else if title {
        let region = &mut w.region;
        let last = region.captions.last().copied().unwrap_or(region.rect.top);
        region.rect.bottom = last;
        region.data = None;
        region.kind = Kind::Title;
        region.certainty = Certainty::Rule;
        region.unsure.clear();
        region.evidence.push(Evidence::Rule {
            id: "T2",
            detail: "title rows only".to_string(),
        });
    } else {
        w.dead = true;
    }
    Ok(Some(out))
}

/// The columns of a table's body.
fn span(r: &Region) -> Rect {
    r.body.or(r.data).unwrap_or(r.rect)
}

/// A table that rule C may continue: a rule table that is not a rest grid.
fn continues(w: &Work) -> bool {
    !w.dead && w.origin == Origin::Rule && !w.region.rest && w.region.kind == Kind::Table
}

/// A region rule C may join to the table above: a rule region with no header band, no caption
/// rows and no section heading.
fn continuation_candidate(w: &Work) -> bool {
    !w.dead
        && w.origin == Origin::Rule
        && !w.region.rest
        && w.region.kind != Kind::Title
        && w.region.header_rows.is_empty()
        && w.region.captions.is_empty()
        && w.region.heading.is_none()
}

/// Whether a region other than `a` and `b` reaches `joined`. `joined` spans the columns of
/// `a`'s range and `b` starts below `a`, so such a region starts in the rows from below `a`
/// to the bottom of `joined`: any other would overlap `a`.
fn blocked(works: &[Work], a: usize, b: usize, joined: Rect, tick: &Tick) -> Result<bool, Halt> {
    let from = joined
        .top
        .max(works[a].region.rect.bottom.saturating_add(1));
    let lo = works.partition_point(|w| w.region.rect.top < from);
    for (k, w) in works.iter().enumerate().skip(lo) {
        if tick.step() {
            return Err(Halt::Stopped);
        }
        if w.region.rect.top > joined.bottom {
            break;
        }
        if k != a && k != b && !w.dead && w.region.rect.intersects(&joined) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Step 7, rule C: a region with no header band whose columns are exactly the columns of the
/// table above it, at most 1 empty row below it, becomes part of that table. The empty row
/// becomes a `Blank` row, and step 6c runs again on the joined table. `works` is in reading
/// order. The joins are found first and each table is read again once, with all its
/// continuations. A stop leaves the tables not yet joined as they were.
pub(crate) fn continuation(prep: &Prep, works: &mut [Work], ctx: &mut Ctx) -> Result<(), Halt> {
    let tick = ctx.tick;
    // Tables by their bottom row and the left column of their body. Two tables with the same
    // bottom row lie side by side, so the key is unique.
    let mut tables: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    // Per table: its range before the joins and the regions it takes, top to bottom.
    let mut joins: BTreeMap<usize, (Rect, Vec<usize>)> = BTreeMap::new();
    let mut found = Ok(());
    for j in 0..works.len() {
        if tick.step() {
            found = Err(Halt::Stopped);
            break;
        }
        if works[j].dead {
            continue;
        }
        if continuation_candidate(&works[j]) {
            let r = works[j].region.rect;
            let upper = (1..=CONTINUATION_GAP + 1).find_map(|g| {
                let bottom = r.top.checked_sub(g)?;
                tables.get(&(bottom, r.left)).copied()
            });
            if let Some(i) = upper
                && span(&works[i].region).right == r.right
            {
                let joined = works[i].region.rect.union(&r);
                let key = (works[i].region.rect.bottom, r.left);
                match blocked(works, i, j, joined, tick) {
                    Err(halt) => {
                        found = Err(halt);
                        break;
                    }
                    Ok(true) => {}
                    Ok(false) => {
                        // Taken now so later regions see the joined range; read at the end.
                        let entry = joins
                            .entry(i)
                            .or_insert_with(|| (works[i].region.rect, Vec::new()));
                        entry.1.push(j);
                        works[i].region.rect = joined;
                        works[j].dead = true;
                        tables.remove(&key);
                        tables.insert((joined.bottom, r.left), i);
                        continue;
                    }
                }
            }
        }
        if continues(&works[j]) {
            let w = &works[j].region;
            tables.insert((w.rect.bottom, span(w).left), j);
        }
    }
    let mut groups = joins.into_iter();
    if found.is_ok() {
        for (i, (old, lowers)) in groups.by_ref() {
            if let Err(halt) = join(prep, works, i, old, &lowers, ctx) {
                undo(works, i, old, &lowers);
                found = Err(halt);
                break;
            }
        }
    }
    for (i, (old, lowers)) in groups {
        undo(works, i, old, &lowers);
    }
    found
}

/// Takes back joins that were found but not read.
fn undo(works: &mut [Work], i: usize, old: Rect, lowers: &[usize]) {
    works[i].region.rect = old;
    for &j in lowers {
        works[j].dead = false;
    }
}

/// Joins the regions `lowers` (top to bottom) into the table `i` above them and runs step 6c
/// again on the joined table. The table's range already spans them; `old` is its range
/// before.
fn join(
    prep: &Prep,
    works: &mut [Work],
    i: usize,
    old: Rect,
    lowers: &[usize],
    ctx: &mut Ctx,
) -> Result<(), Halt> {
    let tick = ctx.tick;
    let upper = &works[i];
    let cols = span(&upper.region);
    let bottom = upper.region.rect.bottom;
    let top = upper.region.body.map_or(cols.top, |b| b.top);
    let body = Rect {
        top,
        bottom,
        ..cols
    };
    let data_top = upper
        .region
        .header_rows
        .last()
        .map_or(cols.top, |&end| end + 1);
    let mut cells = upper.cells.clone();
    let mut merges = upper.merges.clone();
    // The empty rows between the parts.
    let mut gaps: Vec<u32> = Vec::new();
    let mut above = old.bottom;
    for &j in lowers {
        if tick.step() {
            return Err(Halt::Stopped);
        }
        let lower = &works[j];
        cells.extend_from_slice(&lower.cells);
        merges.extend_from_slice(&lower.merges);
        let r = lower.region.rect;
        gaps.extend((u64::from(above) + 1..u64::from(r.top)).map(|x| x as u32));
        above = r.bottom;
    }
    let Some(lines) = Lines::new(prep, body, cells, &merges, tick) else {
        return Err(Halt::Stopped);
    };
    let mut region = works[i].region.clone();
    region.data = Some(Rect {
        top: data_top,
        bottom,
        ..cols
    });
    if region.body.is_some() {
        region.body = Some(body);
    }
    if !rows::table_rows(&lines, &mut region, None, false, ctx.outline, tick) {
        return Err(Halt::Stopped);
    }
    if let Some(data) = region.data {
        let mut rows: Vec<RowNote> = gaps
            .into_iter()
            .filter(|&x| data.top <= x && x <= data.bottom)
            .map(|row| RowNote {
                row,
                kind: RowKind::Blank,
                level: None,
                parent: None,
                why: "empty row before the continuation".to_string(),
            })
            .collect();
        if !rows.is_empty() {
            rows.append(&mut region.rows);
            rows.sort_by_key(|n| n.row);
            region.rows = rows;
        }
    }
    let first = lowers.first().map_or(bottom, |&j| works[j].region.rect.top);
    if region.data.is_some_and(|d| d.bottom >= first) {
        region.evidence.push(Evidence::Rule {
            id: "C",
            detail: format!("continued in {}", header::row_words(first, bottom)),
        });
    } else {
        // The parts below added no data row, only notes.
        let added: Vec<u32> = region
            .notes
            .iter()
            .copied()
            .filter(|&r| r >= first)
            .collect();
        region.evidence.push(Evidence::Rule {
            id: "N",
            detail: format!("notes in {}", row_words(&added)),
        });
    }
    let coarse = lowers.iter().any(|&j| works[j].coarse);
    let upper = &mut works[i];
    upper.region = region;
    upper.cells = lines.into_cells();
    upper.merges = merges;
    upper.coarse |= coarse;
    Ok(())
}

/// Whether a region is a note block for rule N: a text region, or a title region whose rows
/// hold one text each in one column or one merged area each, with no section heading.
fn note_candidate(prep: &Prep, w: &Work) -> bool {
    let r = &w.region;
    if w.dead || w.origin == Origin::Fact || r.rest || r.heading.is_some() {
        return false;
    }
    match r.kind {
        Kind::Text => true,
        Kind::Title => {
            let mut col = None;
            let mut one_column = true;
            let mut merged = true;
            for &row in &r.captions {
                let Some(cell) =
                    captions::single_text(prep, &w.merges, row, r.rect.left, r.rect.right)
                else {
                    return false;
                };
                one_column &= *col.get_or_insert(cell.pos.col) == cell.pos.col;
                merged &= w.merges.iter().any(|&m| {
                    let m = &prep.merges[m as usize];
                    m.top <= row && row <= m.bottom && m.contains(cell.pos)
                });
            }
            one_column || merged
        }
        _ => false,
    }
}

/// The rows of a note region that hold its text: its caption rows and the rows of its data
/// area with a cell.
fn note_rows(prep: &Prep, r: &Region, tick: &Tick) -> Result<Vec<u32>, Halt> {
    let mut rows = r.captions.clone();
    if let Some(d) = r.data {
        for k in prep.rows(d.top, d.bottom) {
            if tick.step() {
                return Err(Halt::Stopped);
            }
            let p = prep.cell(k).pos;
            if d.left <= p.col && p.col <= d.right && rows.last() != Some(&p.row) {
                rows.push(p.row);
            }
        }
    }
    rows.sort_unstable();
    rows.dedup();
    Ok(rows)
}

/// Step 8, rule N: a text region that starts at most 2 empty rows below a table and lies
/// inside its columns becomes a note block of that table. Rule S: text beside a table stays
/// its own region. `works` is in reading order.
pub(crate) fn attach_notes(prep: &Prep, works: &mut [Work], tick: &Tick) -> Result<(), Halt> {
    // Tables by bottom row and left column; tables with the same bottom row lie side by
    // side, so for one column at most one of them holds it.
    let mut tables: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for j in 0..works.len() {
        if tick.step() {
            return Err(Halt::Stopped);
        }
        if works[j].dead {
            continue;
        }
        if note_candidate(prep, &works[j]) {
            let r = works[j].region.rect;
            let mut target = None;
            for g in 1..=NOTE_GAP + 1 {
                let Some(bottom) = r.top.checked_sub(g) else {
                    break;
                };
                let found = tables
                    .range((bottom, 0)..=(bottom, r.left))
                    .next_back()
                    .map(|(_, &i)| i);
                if let Some(i) = found
                    && works[i].region.rect.right >= r.right
                {
                    target = Some(i);
                    break;
                }
            }
            if let Some(i) = target {
                let joined = works[i].region.rect.union(&r);
                if !blocked(works, i, j, joined, tick)? {
                    let rows = note_rows(prep, &works[j].region, tick)?;
                    let key = (works[i].region.rect.bottom, works[i].region.rect.left);
                    let merges = std::mem::take(&mut works[j].merges);
                    works[j].dead = true;
                    let t = &mut works[i];
                    t.merges.extend(merges);
                    t.region.rect = joined;
                    t.region.evidence.push(Evidence::Rule {
                        id: "N",
                        detail: format!("notes in {}", row_words(&rows)),
                    });
                    t.region.notes.extend(rows);
                    tables.remove(&key);
                    tables.insert((joined.bottom, joined.left), i);
                    continue;
                }
            }
        }
        let w = &works[j];
        if w.region.kind == Kind::Table && !w.region.rest {
            tables.insert((w.region.rect.bottom, w.region.rect.left), j);
        }
    }
    Ok(())
}
