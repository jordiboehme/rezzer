// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The page geometry of a read that asks for it: the size of each page and the boxes of its
//! blocks, table cells, pictures and drawn rules, in the frame of the page's runs. It is not
//! part of the output: a box the caps cut is left out without a loss, and nothing here
//! charges the budget, so the Markdown and the JSON of a read stay the same.

use rezzer_model::{Anchor, Block, BoxKind, Geometry, MAX_BOXES_PER_PAGE, PlacedBox};

use crate::layout::{self, Laid, Run};
use crate::tables::RuleKind;
use crate::text::{PageItems, THIN};

/// The box of a run as the region engine takes it: its advance, from the baseline up its
/// rendered height. `None` when a coordinate is not finite.
pub(crate) fn run_box(x: f32, y: f32, w: f32, h: f32) -> Option<[f32; 4]> {
    ordered([x, y, x + w, y + h])
}

/// The union of the boxes of `runs`; `None` for no run with a finite box.
pub(crate) fn runs_box(runs: &[Run]) -> Option<[f32; 4]> {
    runs.iter()
        .filter_map(|r| run_box(r.x, r.y, r.w, r.h))
        .reduce(union)
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// Adds page `page` of `size` points (the page in the frame of its runs: its visible box,
/// turned as its text was read) with the boxes of `items` and, for a page laid out, of the
/// blocks `laid` built. The boxes of the blocks come first, in reading order, each table
/// followed by its cells; then the pictures; then the drawn rules and rectangles. The boxes stop
/// at [`MAX_BOXES_PER_PAGE`], so a page of many rules keeps its text. Each list of the page
/// is walked once, the lists of what it draws only up to the cap.
pub(crate) fn page_geometry(
    geometry: &mut Geometry,
    page: u32,
    size: [f32; 2],
    items: &PageItems,
    laid: Option<&Laid>,
) {
    if !geometry.add_page(page, size) {
        return;
    }
    // Whether the page holds as many boxes as it may: then nothing more is tried.
    let full = |g: &Geometry| {
        g.pages
            .last()
            .is_none_or(|p| p.page != page || p.boxes.len() >= MAX_BOXES_PER_PAGE)
    };
    let mut put = |g: &mut Geometry, kind: BoxKind, bounds: [f32; 4], anchor: Option<Anchor>| {
        if full(g) {
            return false;
        }
        g.push(
            page,
            PlacedBox {
                kind,
                bounds,
                anchor,
            },
        );
        true
    };
    if let Some(laid) = laid {
        blocks(geometry, laid, &mut put);
    }
    // The pictures are in the visible page box; on a page read turned they go into the frame
    // of the runs, as the layout takes them. Turning the size back is turning it again.
    let (w, h) = items.turn.size(size[0], size[1]);
    for b in &items.images {
        let b = if items.turned {
            items.turn.place_box(*b, w, h)
        } else {
            *b
        };
        let Some(b) = ordered(b) else { continue };
        if !put(geometry, BoxKind::Picture, b, None) {
            return;
        }
    }
    for r in &items.rules {
        let Some(b) = ordered([r.x0, r.y0, r.x1, r.y1]) else {
            continue;
        };
        // The text pass writes a filled bar this thin as a line already (see `bounds_rule`);
        // a rectangle from any other source is held to the same edge.
        let thin = (b[2] - b[0]).min(b[3] - b[1]) <= THIN;
        let kind = match r.kind {
            RuleKind::Rect if !thin => BoxKind::Drawn,
            _ => BoxKind::Rule,
        };
        if !put(geometry, kind, b, None) {
            return;
        }
    }
}

/// The boxes of the blocks of a laid out page: each block the union of the runs written in
/// it, a table also the boxes of its cells.
fn blocks(
    geometry: &mut Geometry,
    laid: &Laid,
    put: &mut impl FnMut(&mut Geometry, BoxKind, [f32; 4], Option<Anchor>) -> bool,
) {
    let Some(input) = laid.engine_in.as_ref() else {
        return;
    };
    let mut found: Vec<Option<[f32; 4]>> = vec![None; laid.blocks.len()];
    for (t, &b) in input.texts.iter().zip(&laid.run_blocks) {
        let (Some(slot), Some(rb)) = (found.get_mut(b as usize), run_box(t.x, t.y, t.w, t.h))
        else {
            continue;
        };
        *slot = Some(slot.map_or(rb, |s| union(s, rb)));
    }
    // The cells of each table, by the table's place among the blocks, in block order.
    let mut cells = laid.cells.iter().peekable();
    for (k, block) in laid.blocks.iter().enumerate() {
        while cells.next_if(|c| c.0 < k).is_some() {}
        let own: &[[f32; 4]] = cells.next_if(|c| c.0 == k).map_or(&[], |c| c.1.as_slice());
        let kind = match block {
            Block::Heading { level: depth, .. } => BoxKind::Heading(*depth),
            Block::Table { .. } => BoxKind::Table,
            _ => BoxKind::Text,
        };
        // A table reaches over its cells: the box of the runs of a cell cut by a column rule
        // may stand past the runs the table holds.
        let bounds = own
            .iter()
            .copied()
            .fold(found[k], |acc, c| Some(acc.map_or(c, |a| union(a, c))));
        let Some(bounds) = bounds else { continue };
        if !put(geometry, kind, bounds, layout::block_anchor(block)) {
            return;
        }
        if kind == BoxKind::Table {
            for &c in own {
                if !put(geometry, BoxKind::Cell, c, None) {
                    return;
                }
            }
        }
    }
}

/// `b` with its sides in order; `None` when a side is not finite.
fn ordered(b: [f32; 4]) -> Option<[f32; 4]> {
    // Checked before the sides are ordered: `min` and `max` pass over a NaN.
    b.iter().all(|v| v.is_finite()).then(|| {
        [
            b[0].min(b[2]),
            b[1].min(b[3]),
            b[0].max(b[2]),
            b[1].max(b[3]),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::Rule;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rule {
        Rule {
            x0,
            y0,
            x1,
            y1,
            kind: RuleKind::Rect,
        }
    }

    fn kinds(items: &PageItems) -> Vec<BoxKind> {
        let mut g = Geometry::default();
        page_geometry(&mut g, 1, [595.0, 842.0], items, None);
        g.pages[0].boxes.iter().map(|b| b.kind).collect()
    }

    /// A rectangle as thin as [`THIN`] on its short side is a rule, one just over it a drawn
    /// shape, whichever side is the short one.
    #[test]
    fn a_thin_rectangle_is_a_rule() {
        let items = PageItems {
            rules: vec![
                rect(10.0, 10.0, 200.0, 12.0),
                rect(10.0, 20.0, 200.0, 22.1),
                rect(10.0, 30.0, 12.0, 200.0),
                rect(20.0, 30.0, 22.1, 200.0),
            ],
            ..PageItems::default()
        };
        assert_eq!(
            kinds(&items),
            [BoxKind::Rule, BoxKind::Drawn, BoxKind::Rule, BoxKind::Drawn]
        );
    }

    /// A box with a side that is not finite is left out; the boxes after it are kept.
    #[test]
    fn a_box_that_is_not_finite_is_left_out() {
        let items = PageItems {
            images: vec![[f32::NAN, 0.0, 10.0, 10.0], [30.0, 40.0, 10.0, 20.0]],
            ..PageItems::default()
        };
        let mut g = Geometry::default();
        page_geometry(&mut g, 1, [595.0, 842.0], &items, None);
        assert_eq!(g.pages[0].boxes.len(), 1);
        assert_eq!(g.pages[0].boxes[0].bounds, [10.0, 20.0, 30.0, 40.0]);
    }

    /// On a page read turned the pictures go into the frame of the runs.
    #[test]
    fn pictures_of_a_turned_page_are_turned() {
        let items = PageItems {
            images: vec![[100.0, 200.0, 150.0, 260.0]],
            turned: true,
            turn: crate::order::Turn::Up,
            ..PageItems::default()
        };
        let mut g = Geometry::default();
        // The page is 595 wide and 842 high; turned, its frame is 842 by 595.
        page_geometry(&mut g, 1, [842.0, 595.0], &items, None);
        let want = crate::order::Turn::Up.place_box([100.0, 200.0, 150.0, 260.0], 595.0, 842.0);
        assert_eq!(g.pages[0].boxes[0].bounds, want);
        assert!(want.iter().all(|v| (0.0..=842.0).contains(v)), "{want:?}");
    }

    /// A page added before gives no boxes again.
    #[test]
    fn a_page_is_added_once() {
        let items = PageItems {
            rules: vec![rect(10.0, 10.0, 200.0, 100.0)],
            ..PageItems::default()
        };
        let mut g = Geometry::default();
        page_geometry(&mut g, 1, [595.0, 842.0], &items, None);
        page_geometry(&mut g, 1, [595.0, 842.0], &items, None);
        assert_eq!(g.pages.len(), 1);
        assert_eq!(g.pages[0].boxes.len(), 1);
    }
}
