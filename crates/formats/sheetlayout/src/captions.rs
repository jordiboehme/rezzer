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
        if row == u32::MAX {
            break;
        }
        row += 1;
    }
    if rows.is_empty() {
        return;
    }
    let n = rows.len() as u32;
    w.region.caption_heading = heading.or(Some(rows[0]));
    w.region.data = (u64::from(n) < r.height()).then(|| Rect {
        top: r.top + n,
        ..r
    });
    w.region.evidence.push(Evidence::Rule {
        id: "T",
        detail: format!("caption {}", row_words(&rows)),
    });
    w.region.captions = rows;
}

fn caption_only(w: &Work) -> bool {
    !w.dead && w.origin == Origin::Rule && !w.region.captions.is_empty() && w.region.data.is_none()
}

/// Rule T2: a region of caption rows only, at most 2 empty rows above another region and
/// starting inside its columns, becomes that region's caption when it has none of its own.
/// When that region has captions, or the joined rectangle would reach another region, the
/// caption rows stay a region of kind `Title`. A caption region with no region in reach is
/// left to the later rules.
pub(crate) fn attach_titles(works: &mut [Work]) {
    let mut order: Vec<usize> = (0..works.len()).filter(|&i| !works[i].dead).collect();
    order.sort_by_key(|&i| (works[i].region.rect.top, works[i].region.rect.left));
    let tops: Vec<u32> = order.iter().map(|&i| works[i].region.rect.top).collect();
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
            !w.dead
                && j != i
                && w.origin != Origin::Rest
                && w.region.rect.top > c.bottom
                && w.region.rect.left <= c.left
                && c.left <= w.region.rect.right
        });
        let Some(t) = target else {
            continue;
        };
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
