// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Boxes as reading regions: the reader's side of the drawn structure the region engine finds
//! (`layoutregions::segment::drawn`). A grid of at least two rows and two columns is a table
//! (the ruled detector reads it); a box that stands alone, or a cell of a grid of one row or
//! one column, is a reading region: its text is read whole, before or after the text around
//! it. Here the text pass maps where a page has text ([`Inked`]), finds the drawn structure of
//! a page once ([`structure`]) and looks up the box each fragment lies in ([`regions_of`]).

use layoutregions::segment::drawn::{self, DrawnStructure, Grids, Unsearched};
use layoutregions::{Bounds, PageIn};

pub(crate) use layoutregions::segment::drawn::BOX_WORK;

use crate::tables::Rule;

/// Places in the lookup grid boxes may take together; past them the page uses no boxes.
const INDEX_ENTRIES: usize = 262_144;

/// Cells of the lookup grid along each side.
const INDEX_SIDE: usize = 16;

/// The drawn structure of page `page`, found once in the text pass: the grids of its `rules`
/// and the boxes its text is read in, in the frame of the page box (`width` by `height`).
/// `widgets` are the boxes of its form fields there; `inked` where it has text.
pub(crate) fn structure(
    page: u32,
    rules: &[Rule],
    (width, height): (f32, f32),
    widgets: &[[f32; 4]],
    inked: &Inked,
) -> DrawnStructure {
    let page = PageIn {
        page,
        size: [width, height],
        turn: 0,
        texts: Vec::new(),
        rules: rules.iter().map(crate::engine::rule_in).collect(),
        boxes: widgets
            .iter()
            .map(|&b| crate::engine::box_in(&b, true))
            .collect(),
        pictures: Vec::new(),
        codes: Vec::new(),
        given: Vec::new(),
    };
    drawn::drawn(&page, &|b: Bounds| inked.may_hold([b.x0, b.y0, b.x1, b.y1]))
}

/// The grids of `rules` alone, without the boxes: for the layout (see
/// [`structure_in_frame`]) and for a search that runs before the page's structure is found
/// (see `tables::field_frames`).
pub(crate) fn grids(rules: &[Rule]) -> Result<Grids, Unsearched> {
    let rules: Vec<_> = rules.iter().map(crate::engine::rule_in).collect();
    drawn::grids(&rules)
}

/// The drawn structure of a page as it is laid out: the grids of its `rules` and the boxes its
/// runs were read in (`boxes`), both in the frame of the runs. Found once per page for every
/// detector of the layout: the text pass does not keep its own (see [`structure`]) until the
/// page is laid out, and on a turned page that one is in another frame.
pub(crate) fn structure_in_frame(rules: &[Rule], boxes: &[[f32; 4]]) -> DrawnStructure {
    DrawnStructure {
        grids: grids(rules),
        boxes: boxes.iter().map(|&b| crate::engine::bounds(b)).collect(),
        boxes_note: None,
    }
}

/// A point this share of its text size from a box's edge, or nearer, lies on the edge (see
/// [`Index::innermost`]): far under any place text is set in a box, far over float noise.
pub(crate) const EDGE_TIE: f32 = 0.01;

/// The innermost of `boxes` each of `points` (a place and the size of its text) lies in, as an
/// index into `boxes`. A box that
/// holds another box a point lies in is no region: the inner one is, and the points of the
/// outer one outside it have none. Past the bounds of the lookup, the reason.
pub(crate) fn regions_of(
    points: &[(f32, f32, f32)],
    boxes: &[[f32; 4]],
) -> Result<Vec<Option<u32>>, String> {
    let Some(mut index) = Index::new(boxes) else {
        return Err("its boxes overlap too much".into());
    };
    let too_much = || Err("its boxes take too many steps to look up".to_string());
    let mut held = vec![false; boxes.len()];
    let mut out = Vec::with_capacity(points.len());
    for &(x, y, size) in points {
        let r = index.innermost(x, y, size);
        if let Some(b) = r {
            held[b as usize] = true;
        }
        out.push(r);
        if index.work > BOX_WORK {
            return too_much();
        }
    }
    // Boxes that hold a box with text are outer ones.
    let mut outer = vec![false; boxes.len()];
    for (k, b) in boxes.iter().enumerate() {
        if !held[k] {
            continue;
        }
        index.holders((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0, 0.0, |o| {
            let a = boxes[o as usize];
            if o as usize != k && a[0] <= b[0] && a[1] <= b[1] && a[2] >= b[2] && a[3] >= b[3] {
                outer[o as usize] = true;
            }
        });
        if index.work > BOX_WORK {
            return too_much();
        }
    }
    for r in &mut out {
        if r.is_some_and(|b| outer[b as usize]) {
            *r = None;
        }
    }
    Ok(out)
}

/// Cells of the map of where a page has text, along each side.
const INKED_SIDE: usize = 64;

/// Where a page has text, coarsely: the number of glyphs in each cell of a grid over the
/// page, summed so that any box asks in four steps whether a glyph may lie in it.
pub(crate) struct Inked {
    step: (f32, f32),
    /// Summed counts: entry (r, c) holds the glyphs in cells below row r and left of column
    /// c, with one row and one column of zeros first.
    sums: Vec<u64>,
}

impl Inked {
    /// The map of `points` (glyph places) on a page of `width` by `height`.
    pub(crate) fn new(width: f32, height: f32, points: impl Iterator<Item = (f32, f32)>) -> Inked {
        let n = INKED_SIDE;
        let step = (
            (width / n as f32).max(f32::MIN_POSITIVE),
            (height / n as f32).max(f32::MIN_POSITIVE),
        );
        let mut counts = vec![0u32; n * n];
        for (x, y) in points {
            if let (Some(c), Some(r)) = (cell_of(x, step.0, n), cell_of(y, step.1, n)) {
                counts[r * n + c] = counts[r * n + c].saturating_add(1);
            }
        }
        let mut sums = vec![0u64; (n + 1) * (n + 1)];
        for r in 0..n {
            for c in 0..n {
                sums[(r + 1) * (n + 1) + c + 1] = u64::from(counts[r * n + c])
                    + sums[r * (n + 1) + c + 1]
                    + sums[(r + 1) * (n + 1) + c]
                    - sums[r * (n + 1) + c];
            }
        }
        Inked { step, sums }
    }

    /// Whether a glyph may lie in `b`: one lies in a cell the box reaches into.
    pub(crate) fn may_hold(&self, b: [f32; 4]) -> bool {
        let n = INKED_SIDE;
        let clamp = |v: f32, step: f32| ((v / step).floor().max(0.0) as usize).min(n - 1);
        if !(b[0] <= b[2] && b[1] <= b[3]) {
            return false;
        }
        let (c0, c1) = (clamp(b[0], self.step.0), clamp(b[2], self.step.0) + 1);
        let (r0, r1) = (clamp(b[1], self.step.1), clamp(b[3], self.step.1) + 1);
        let at = |r: usize, c: usize| self.sums[r * (n + 1) + c];
        at(r1, c1) + at(r0, c0) > at(r0, c1) + at(r1, c0)
    }
}

/// The cell of a value on a side of `n` cells of `step`, `None` off the page.
fn cell_of(v: f32, step: f32, n: usize) -> Option<usize> {
    let k = (v / step).floor();
    (k >= 0.0 && k < n as f32).then_some(k as usize)
}

/// Boxes found by the points they hold: a grid over their extent, each cell listing the boxes
/// that reach into it.
pub(crate) struct Index<'b> {
    boxes: &'b [[f32; 4]],
    lo: (f32, f32),
    step: (f32, f32),
    cells: Vec<Vec<u32>>,
    /// Steps taken by lookups so far.
    pub work: usize,
}

impl<'b> Index<'b> {
    /// The index, or `None` when the boxes would take too many places in it.
    pub(crate) fn new(boxes: &'b [[f32; 4]]) -> Option<Index<'b>> {
        let x0 = boxes.iter().map(|b| b[0]).fold(f32::MAX, f32::min);
        let y0 = boxes.iter().map(|b| b[1]).fold(f32::MAX, f32::min);
        let x1 = boxes.iter().map(|b| b[2]).fold(f32::MIN, f32::max);
        let y1 = boxes.iter().map(|b| b[3]).fold(f32::MIN, f32::max);
        let side = INDEX_SIDE as f32;
        let step = (
            ((x1 - x0) / side).max(f32::MIN_POSITIVE),
            ((y1 - y0) / side).max(f32::MIN_POSITIVE),
        );
        let mut index = Index {
            boxes,
            lo: (x0, y0),
            step,
            cells: vec![Vec::new(); INDEX_SIDE * INDEX_SIDE],
            work: 0,
        };
        let mut entries = 0usize;
        for (i, b) in boxes.iter().enumerate() {
            let (c0, r0) = index.cell(b[0], b[1]);
            let (c1, r1) = index.cell(b[2], b[3]);
            entries += (c1 - c0 + 1) * (r1 - r0 + 1);
            if entries > INDEX_ENTRIES {
                return None;
            }
            for r in r0..=r1 {
                for c in c0..=c1 {
                    index.cells[r * INDEX_SIDE + c].push(i as u32);
                }
            }
        }
        Some(index)
    }

    fn cell(&self, x: f32, y: f32) -> (usize, usize) {
        let at = |v: f32, lo: f32, step: f32| {
            let k = ((v - lo) / step).floor();
            if k.is_nan() || k < 0.0 {
                0
            } else {
                (k as usize).min(INDEX_SIDE - 1)
            }
        };
        (at(x, self.lo.0, self.step.0), at(y, self.lo.1, self.step.1))
    }

    /// The smallest box that holds the point of text of `size`, by its index. A point within
    /// [`EDGE_TIE`] of its size of a box's edge lies on the edge, and in neither box on its
    /// two sides: a run whose middle falls on the edge two digit boxes share spans both and is
    /// the text of neither, at every scale (float rounding of the edge or of the point does
    /// not pick one).
    pub(crate) fn innermost(&mut self, x: f32, y: f32, size: f32) -> Option<u32> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let tie = EDGE_TIE * size.max(0.0);
        let (c, r) = self.cell(x, y);
        let mut best: Option<(f32, u32)> = None;
        for &i in &self.cells[r * INDEX_SIDE + c] {
            self.work += 1;
            let b = self.boxes[i as usize];
            if x > b[0] + tie && x < b[2] - tie && y > b[1] + tie && y < b[3] - tie {
                let area = (b[2] - b[0]) * (b[3] - b[1]);
                if best.is_none_or(|(a, _)| area < a) {
                    best = Some((area, i));
                }
            }
        }
        best.map(|(_, i)| i)
    }

    /// Hands `each` every box that holds the point of text of `size` (0 for a point of no
    /// text), on its edge within [`EDGE_TIE`] of that size as [`Index::innermost`] has it.
    pub(crate) fn holders(&mut self, x: f32, y: f32, size: f32, mut each: impl FnMut(u32)) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let tie = EDGE_TIE * size.max(0.0);
        let (c, r) = self.cell(x, y);
        for &i in &self.cells[r * INDEX_SIDE + c] {
            self.work += 1;
            let b = self.boxes[i as usize];
            if x > b[0] + tie && x < b[2] - tie && y > b[1] + tie && y < b[3] - tie {
                each(i);
            }
        }
    }
}

pub(crate) use layoutregions::segment::cut::{BASELINE_NEAR, Part, offset_baselines};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::RuleKind;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rule {
        Rule {
            x0,
            y0,
            x1,
            y1,
            kind: RuleKind::Rect,
        }
    }

    /// The text pass finds a page's boxes with its map of where it has text, coarse as it is,
    /// and its form fields: a box with a glyph in it is a region, a box without one is not, nor
    /// is a box that holds a form field. The grids are found once for it.
    #[test]
    fn the_structure_of_a_page_uses_its_text_and_its_fields() {
        let rules = [
            rect(40.0, 600.0, 240.0, 720.0),
            rect(300.0, 600.0, 500.0, 700.0),
            rect(300.0, 100.0, 500.0, 200.0),
        ];
        let inked = Inked::new(595.0, 842.0, [(100.0, 650.0), (350.0, 650.0)].into_iter());
        let searches = || drawn::grid_searches().expect("a test build counts grid searches");
        let before = searches();
        let found = structure(1, &rules, (595.0, 842.0), &[], &inked);
        assert_eq!(searches() - before, 1);
        let at: Vec<f32> = found.boxes.iter().map(|b| b.x0).collect();
        assert_eq!(at, [40.0, 300.0]);
        assert_eq!(found.boxes[1].y0, 600.0);
        assert_eq!(found.boxes_note, None);
        let fields = [[310.0, 610.0, 400.0, 630.0]];
        let found = structure(1, &rules, (595.0, 842.0), &fields, &inked);
        assert_eq!(found.boxes.len(), 1);
        assert_eq!(found.boxes[0].x0, 40.0);
    }

    #[test]
    fn the_innermost_box_holds_a_point() {
        let list = [
            [0.0, 0.0, 100.0, 100.0],
            [10.0, 10.0, 50.0, 50.0],
            [60.0, 60.0, 90.0, 90.0],
        ];
        let mut index = Index::new(&list).expect("index");
        assert_eq!(index.innermost(20.0, 20.0, 8.0), Some(1));
        assert_eq!(index.innermost(55.0, 20.0, 8.0), Some(0));
        assert_eq!(index.innermost(70.0, 70.0, 8.0), Some(2));
        assert_eq!(index.innermost(200.0, 20.0, 8.0), None);
    }

    /// A point on the edge two boxes share lies in neither, at every scale: a point within
    /// [`EDGE_TIE`] of its size of an edge is on it, so float rounding of the edge or the point
    /// does not pick a box.
    #[test]
    fn a_point_on_a_shared_edge_lies_in_no_box() {
        for (size, k) in [6.0f32, 8.0, 9.5, 10.0, 12.0, 14.0]
            .into_iter()
            .flat_map(|s| [0.5f32, 0.7, 0.75, 1.0, 1.3, 1.5, 2.0].map(|k| (s, k)))
        {
            let w = 0.6 * size * k;
            let x0 = 60.0 * k;
            let list: Vec<[f32; 4]> = (0..6)
                .map(|n| {
                    let a = x0 + n as f32 * w;
                    [a, 700.0 * k, a + w, 700.0 * k + 1.4 * size * k]
                })
                .collect();
            let mut index = Index::new(&list).expect("index");
            let y = 700.0 * k + 0.5 * size * k;
            for n in 1..6 {
                let edge = x0 + n as f32 * w;
                let mut x = edge;
                for _ in 0..4 {
                    x = x.next_down();
                }
                for _ in 0..8 {
                    assert_eq!(index.innermost(x, y, size * k), None, "{k} {n} {x} {edge}");
                    x = x.next_up();
                }
                let off = 0.1 * size * k;
                assert_eq!(index.innermost(edge - off, y, size * k), Some(n as u32 - 1));
                assert_eq!(index.innermost(edge + off, y, size * k), Some(n as u32));
            }
        }
    }
}
