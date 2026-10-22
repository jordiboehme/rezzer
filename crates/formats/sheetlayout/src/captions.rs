// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rule T (caption rows) and rule T2 (title regions).

use crate::input::{CellIn, Class, Rect};
use crate::occupancy::Prep;
use crate::{Certainty, Evidence, Kind, Origin, Work};

/// Caption rows a region may have.
const MAX_CAPTIONS: usize = 3;
/// Empty rows a title may stand above the region it names.
const TITLE_GAP: u32 = 2;

/// The text cell of a single-text row: the row holds exactly one non-empty cell, a text, and
/// at most one merged area, which holds that cell. A cell with an empty text counts as empty.
pub(crate) fn single_text<'a>(
    prep: &Prep<'a>,
    merges: &[u32],
    row: u32,
    left: u32,
    right: u32,
) -> Option<&'a CellIn> {
    let mut found = None;
    for k in prep.row_span(row, left, right) {
        let c = prep.cell(k);
        if c.text.is_empty() {
            continue;
        }
        if found.is_some() || c.class != Class::Text {
            return None;
        }
        found = Some(c);
    }
    let cell = found?;
    let mut seen = false;
    for &m in merges {
        let m = &prep.merges[m as usize];
        if m.top <= row && row <= m.bottom && m.left <= right && left <= m.right {
            if seen || !m.contains(cell.pos) {
                return None;
            }
            seen = true;
        }
    }
    Some(cell)
}

/// The number of marks (bold, fill) of a cell, or `None` when one of them is unknown.
fn marks(c: &CellIn) -> Option<u8> {
    Some(u8::from(c.style.bold?) + u8::from(c.style.filled?))
}

/// One-based row words for a layout line: `row 18` or `rows 1 to 2`.
fn row_words(rows: &[u32]) -> String {
    match rows {
        [one] => format!("row {}", u64::from(*one) + 1),
        [first, .., last] => format!("rows {} to {}", u64::from(*first) + 1, u64::from(*last) + 1),
        [] => String::new(),
    }
}

/// Rule T on a rule region: its leading single-text rows, at most 3, are caption rows. Where
/// styles are read, the run stops at a row with fewer marks than the row before it.
pub(crate) fn caption_rows(prep: &Prep, w: &mut Work) {
    let r = w.region.rect;
    let mut rows = Vec::new();
    let mut heading = None;
    let mut prev: Option<u8> = None;
    let mut row = r.top;
    // The last row of the caption lines so far.
    let mut end = r.top;
    while rows.len() < MAX_CAPTIONS && row <= r.bottom {
        let Some(cell) = single_text(prep, &w.merges, row, r.left, r.right) else {
            break;
        };
        if prep.input.styles_read {
            let now = marks(cell);
            if let (Some(p), Some(n)) = (prev, now)
                && n < p
            {
                break;
            }
            prev = now;
        }
        if heading.is_none() && cell.style.bold == Some(true) {
            heading = Some(row);
        }
        rows.push(row);
        // Rows below that only the caption's own merge covers belong to the caption line.
        end = covered_below(prep, &w.merges, cell, row, r);
        if end >= r.bottom {
            break;
        }
        row = end + 1;
    }
    if rows.is_empty() {
        return;
    }
    w.region.caption_heading = heading.or(Some(rows[0]));
    w.region.data = end
        .checked_add(1)
        .filter(|&top| top <= r.bottom)
        .map(|top| Rect { top, ..r });
    w.region.evidence.push(Evidence::Rule {
        id: "T",
        detail: format!("caption {}", row_words(&rows)),
    });
    w.region.captions = rows;
}

/// The last row of the caption line of `cell` in `row`: the rows below that the merge
/// holding the cell covers and that hold no other value and no other merge inside the
/// region's columns. Works on the cells and merges of those rows, never per covered row.
fn covered_below(prep: &Prep, merges: &[u32], cell: &CellIn, row: u32, r: Rect) -> u32 {
    let Some(own) = merges
        .iter()
        .map(|&m| &prep.merges[m as usize])
        .find(|m| m.contains(cell.pos))
    else {
        return row;
    };
    let limit = own.bottom.min(r.bottom);
    if limit <= row {
        return row;
    }
    let mut stop = limit;
    for k in prep.rows(row + 1, limit) {
        let c = prep.cell(k);
        // A formula cell counts even with an empty result; an empty styled cell does not.
        let value = !c.text.is_empty() || c.formula.is_some();
        if c.pos.col >= r.left && c.pos.col <= r.right && value {
            stop = stop.min(c.pos.row - 1);
            break;
        }
    }
    for &m in merges {
        let m = &prep.merges[m as usize];
        if m != own && m.top > row && m.top <= stop && m.left <= r.right && r.left <= m.right {
            stop = stop.min(m.top - 1);
        }
    }
    stop
}

/// The empty rows between the caption region `i` (rectangle `c`) and the nearest live region
/// that ends at most [`TITLE_GAP`] empty rows above it and shares a column with it.
fn gap_above(works: &[Work], by_bottom: &[(u32, u32, usize)], i: usize, c: Rect) -> Option<u32> {
    (0..=TITLE_GAP).find(|&gap| {
        let Some(bottom) = c.top.checked_sub(gap + 1) else {
            return false;
        };
        // Regions that share a bottom row do not share a column, so the one that starts
        // nearest left of the title's right edge is the only one that can reach it.
        let k = by_bottom.partition_point(|&(b, l, _)| (b, l) <= (bottom, c.right));
        by_bottom[..k].last().is_some_and(|&(b, _, j)| {
            let r = works[j].region.rect;
            b == bottom && j != i && !works[j].dead && r.right >= c.left
        })
    })
}

fn caption_only(w: &Work) -> bool {
    !w.dead && w.origin == Origin::Rule && !w.region.captions.is_empty() && w.region.data.is_none()
}

/// Rule T2: a region of caption rows only, at most 2 empty rows above another region and
/// starting inside its columns, becomes that region's caption when it has none of its own.
/// A title right on top of a region (no empty row between) that starts at most one empty
/// column right of it is in reach too: a title over a side table.
///
/// A title in reach of a region above it as well (that region ends at most 2 empty rows
/// above the title and shares a column with it) names the region below only when no empty
/// row parts it from that one, or when its gap below is not larger than its gap above.
/// Otherwise it is left to the later rules, which make it a note of the region above.
///
/// When the region below has captions, or the joined rectangle would reach another region,
/// the caption rows stay a region of kind `Title`. A caption region with no region in reach
/// is left to the later rules.
pub(crate) fn attach_titles(works: &mut [Work]) {
    let mut order: Vec<usize> = (0..works.len()).filter(|&i| !works[i].dead).collect();
    order.sort_by_key(|&i| (works[i].region.rect.top, works[i].region.rect.left));
    let tops: Vec<u32> = order.iter().map(|&i| works[i].region.rect.top).collect();
    // The regions by bottom row and left column, to find the region right above a title.
    let mut by_bottom: Vec<(u32, u32, usize)> = order
        .iter()
        .map(|&i| {
            let r = works[i].region.rect;
            (r.bottom, r.left, i)
        })
        .collect();
    by_bottom.sort_unstable();
    for oi in 0..order.len() {
        let i = order[oi];
        if !caption_only(&works[i]) {
            continue;
        }
        let c = works[i].region.rect;
        if c.bottom == u32::MAX {
            continue;
        }
        let reach = c.bottom.saturating_add(TITLE_GAP + 1);
        let from = tops.partition_point(|&t| t <= c.bottom);
        let to = tops.partition_point(|&t| t <= reach);
        let target = order[from..to].iter().copied().find(|&j| {
            let w = &works[j];
            let r = w.region.rect;
            let inside = r.left <= c.left && c.left <= r.right;
            let beside = r.top == c.bottom + 1 && r.left > c.right && r.left - c.right <= 2;
            !w.dead && j != i && w.origin != Origin::Rest && r.top > c.bottom && (inside || beside)
        });
        let Some(t) = target else {
            continue;
        };
        let below = works[t].region.rect.top - c.bottom - 1;
        if below > 0
            && let Some(above) = gap_above(works, &by_bottom, i, c)
            && below > above
        {
            continue;
        }
        let joined = c.union(&works[t].region.rect);
        let free = works[t].region.captions.is_empty()
            && !works
                .iter()
                .enumerate()
                .any(|(k, w)| k != i && k != t && !w.dead && w.region.rect.intersects(&joined));
        if free {
            let caption = std::mem::take(&mut works[i].region.captions);
            let heading = works[i].region.caption_heading;
            let merges = std::mem::take(&mut works[i].merges);
            works[i].dead = true;
            let coarse = works[i].coarse;
            let w = &mut works[t];
            w.coarse |= coarse;
            w.region.evidence.push(Evidence::Rule {
                id: "T2",
                detail: format!("title in {}", row_words(&caption)),
            });
            w.region.captions = caption;
            w.region.caption_heading = heading;
            w.region.rect = joined;
            w.merges.extend(merges);
        } else {
            let w = &mut works[i].region;
            w.kind = Kind::Title;
            w.certainty = Certainty::Rule;
            w.unsure.clear();
            w.evidence.push(Evidence::Rule {
                id: "T2",
                detail: "title rows only".to_string(),
            });
        }
    }
}
