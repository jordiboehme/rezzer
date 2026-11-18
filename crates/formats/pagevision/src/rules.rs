// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The ink rules of the region bake-off and the C combination.
//!
//! A port of `merge_lines` and `ink_rules` in the bake-off's `common.py` and of `hybrid.py`, with
//! the same loop orders and break points, so the regions match the bake-off pixel for pixel. The
//! pixel constants are bake-off values, measured in gate 1, and not tuned again here.

use crate::ink::{Mask, components, dilate_cross, label4};

/// Bumped whenever the rules give other regions for the same input.
pub const RULES_VERSION: u32 = 2;

/// A box in page pixels: x0, y0, x1, y1.
pub type Rect = [f64; 4];

/// What the ink rules saw in a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Drawing,
    Text,
}

/// The fixes to the bake-off rules, each behind its own flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleFixes {
    /// A thin wide stroke right under a text line is an underline, so text.
    pub underline_is_text: bool,
    /// A long sparse stroke that touches two drawing parts joins them.
    pub arrow_joins_parts: bool,
}

impl RuleFixes {
    /// The bake-off rules as they are.
    pub const NONE: Self = Self {
        underline_is_text: false,
        arrow_joins_parts: false,
    };
    /// Every fix on.
    pub const ALL: Self = Self {
        underline_is_text: true,
        arrow_joins_parts: true,
    };
}

/// A proposed region, before ink snapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proposed {
    /// The layout detector drew this region.
    pub drawn: bool,
    /// x0, y0, x1, y1 in page pixels.
    pub bbox: [i64; 4],
    /// The ink rules saw a drawing here that the layout detector missed.
    pub possible_drawing: bool,
}

// Bake-off values, gate 1. Each moves the result both ways in a test below (the component
// dilation in ink.rs `components_split_beyond_the_dilation`).
/// Ink components closer than this many pixels are one component.
const COMPONENT_DILATE: u32 = 3;
/// Smaller components are noise.
const MIN_COMPONENT_PIXELS: u32 = 30;
/// The line height when the page has no text line.
const DEFAULT_LINE_HEIGHT: f64 = 60.0;
/// Drawing strokes closer than this many pixels are one drawing.
const DRAWING_DILATE: u32 = 30;
/// A drawing group with fewer drawing pixels is dropped.
const MIN_DRAWING_PIXELS: usize = 400;
/// The margin of a drawing that absorbs a text line or an orphan.
const ABSORB_GROW: f64 = 20.0;
/// The share of a text box inside the absorb margin that moves it into the drawing.
const ABSORB_SHARE: f64 = 0.3;
/// The margin of a drawing that absorbs a touching text block or drawing.
const JOIN_GROW: f64 = 35.0;
/// A text line below another joins its block when the gap is under this share of its height.
const BLOCK_GAP: f64 = 0.9;
/// The margin of a drawn box under which ink and text lines belong to the drawn region.
const DRAWN_GROW: f64 = 15.0;
/// The share of a text line under a drawn box that drops the line.
const DRAWN_LINE_SHARE: f64 = 0.5;

// The two fixes. Each is pinned both ways by a test. The underline values come from the real
// underline case of the bake-off pages, which lies well inside each of them.
/// An underline is at most this share of the line height tall,
const UNDERLINE_MAX_HEIGHT: f64 = 0.35;
/// at least this share of the line height wide (a test pins it from above; below 1.2 it changes
/// nothing, because a stroke this thin is only a drawing when it is wider than 1.2 line heights),
const UNDERLINE_MIN_WIDTH: f64 = 1.2;
/// its top at most this share of the line height below the bottom of a text line,
const UNDERLINE_MAX_GAP: f64 = 0.5;
/// and at least this share of its width under that line.
const UNDERLINE_MIN_OVERLAP: f64 = 0.5;
/// A connector stroke reaches drawing parts this many pixels from its box.
const ARROW_REACH: f64 = 35.0;
/// A connector stroke fills less than this share of its box (a thin slanted stroke),
const ARROW_MAX_FILL: f64 = 0.04;
/// and its long side is at least this many times its short side.
const ARROW_MIN_ASPECT: f64 = 6.0;

fn inter(a: &Rect, b: &Rect) -> f64 {
    (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0)
}

fn area(a: &Rect) -> f64 {
    ((a[2] - a[0]) * (a[3] - a[1])).max(1.0)
}

fn union(a: &Rect, b: &Rect) -> Rect {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn grow(a: &Rect, d: f64) -> Rect {
    [a[0] - d, a[1] - d, a[2] + d, a[3] + d]
}

/// Remove the first box equal to `b`, as Python's `list.remove`.
fn remove_first(list: &mut Vec<Rect>, b: &Rect) {
    if let Some(i) = list.iter().position(|x| x == b) {
        list.remove(i);
    }
}

/// The range of a numpy slice `start:stop` (start already at least 0) over `len` items.
fn py_range(start: i64, stop: i64, len: u32) -> std::ops::Range<usize> {
    let len = i64::from(len);
    let start = start.clamp(0, len);
    let stop = if stop < 0 {
        (stop + len).max(0)
    } else {
        stop.min(len)
    };
    start as usize..stop.max(start) as usize
}

/// Join text lines that overlap by more than half of the smaller one, until none do.
pub fn merge_lines(lines: &[Rect]) -> Vec<Rect> {
    // More than half of the smaller box: one line found twice. Bake-off value, gate 1.
    const MERGE_SHARE: f64 = 0.5;
    let mut bs = lines.to_vec();
    loop {
        let mut merged = false;
        'outer: for i in 0..bs.len() {
            for j in i + 1..bs.len() {
                if inter(&bs[i], &bs[j]) / area(&bs[i]).min(area(&bs[j])) > MERGE_SHARE {
                    bs[i] = union(&bs[i], &bs[j]);
                    bs.remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            return bs;
        }
    }
}

/// The median line height (numpy's: the mean of the two middle values for an even count), or 60
/// without lines.
fn line_height(lines: &[Rect]) -> f64 {
    if lines.is_empty() {
        return DEFAULT_LINE_HEIGHT;
    }
    let mut hs: Vec<f64> = lines.iter().map(|b| b[3] - b[1]).collect();
    hs.sort_by(f64::total_cmp);
    let n = hs.len();
    if n % 2 == 1 {
        hs[n / 2]
    } else {
        (hs[n / 2 - 1] + hs[n / 2]) / 2.0
    }
}

/// The pixels under the text lines, the edges rounded half to even as Python's `round`.
fn text_mask(lines: &[Rect], width: u32, height: u32) -> Mask {
    let mut m = Mask::new(width, height);
    for b in lines {
        let [x0, y0, x1, y1] = b.map(|c| c.round_ties_even() as i64);
        for y in py_range(y0.max(0), y1, height) {
            let row = y * width as usize;
            m.bits[row..row + width as usize][py_range(x0.max(0), x1, width)].fill(true);
        }
    }
    m
}

/// The text and drawing regions the ink rules see, from the ink and the text lines.
/// Drawings first, then text blocks, as the bake-off returns them.
pub fn ink_rules(ink: &Mask, lines: &[Rect], fixes: RuleFixes) -> Vec<(RuleKind, [i64; 4])> {
    let (w, h) = (ink.width, ink.height);
    let wu = w as usize;
    let lines = merge_lines(lines);
    let (lab, comps) = components(ink, COMPONENT_DILATE);
    let textmask = text_mask(&lines, w, h);
    let lh = line_height(&lines);
    let mut drawing = Mask::new(w, h);
    let mut orphans: Vec<Rect> = Vec::new();
    // Thin long drawing strokes that may connect two drawing parts.
    let mut arrows: Vec<Rect> = Vec::new();
    for c in &comps {
        let npx = c.pixels;
        if npx < MIN_COMPONENT_PIXELS {
            continue;
        }
        let mut in_text = 0u32;
        for y in c.y0..c.y1 {
            for x in c.x0..c.x1 {
                let i = y as usize * wu + x as usize;
                if lab[i] == c.label && textmask.bits[i] {
                    in_text += 1;
                }
            }
        }
        let inside = f64::from(in_text) / f64::from(npx);
        let (hh, ww) = (f64::from(c.y1 - c.y0), f64::from(c.x1 - c.x0));
        let big = hh.max(ww);
        // Bake-off values, gate 1: a big stroke mostly outside the text lines, a mid-size one
        // nearly all outside, or a large sparse one (an outline or a connector) is drawing.
        let fill = f64::from(npx) / (hh * ww).max(1.0);
        let sparse = fill < 0.04;
        let cbox = [
            f64::from(c.x0),
            f64::from(c.y0),
            f64::from(c.x1),
            f64::from(c.y1),
        ];
        let looks_drawn = (big > 2.5 * lh && inside < 0.85)
            || (big > 1.2 * lh && inside < 0.3)
            || (big > 1.5 * lh && sparse);
        if looks_drawn && !(fixes.underline_is_text && is_underline(&cbox, &lines, lh)) {
            if fixes.arrow_joins_parts
                && fill < ARROW_MAX_FILL
                && big >= ARROW_MIN_ASPECT * hh.min(ww)
            {
                arrows.push(cbox);
            }
            for y in c.y0..c.y1 {
                for x in c.x0..c.x1 {
                    let i = y as usize * wu + x as usize;
                    if lab[i] == c.label {
                        drawing.bits[i] = true;
                    }
                }
            }
        } else if inside < 0.5 {
            // Ink mostly outside the text lines that the line detector missed.
            orphans.push(cbox);
        }
    }

    let mut diags = drawing_groups(&drawing);
    for a in &arrows {
        join_by_arrow(&mut diags, a);
    }

    let mut rest: Vec<Rect> = lines.iter().copied().chain(orphans).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for b in rest.clone() {
            for d in diags.iter_mut() {
                if inter(&grow(d, ABSORB_GROW), &b) / area(&b) > ABSORB_SHARE {
                    *d = union(d, &b);
                    remove_first(&mut rest, &b);
                    changed = true;
                    break;
                }
            }
        }
        for i in 0..diags.len() {
            for j in i + 1..diags.len() {
                if inter(&diags[i], &diags[j]) > 0.0 {
                    diags[i] = union(&diags[i], &diags[j]);
                    diags.remove(j);
                    changed = true;
                    break;
                }
            }
            if changed {
                break;
            }
        }
    }

    rest.sort_by(|a, b| a[1].total_cmp(&b[1]));
    let mut blocks: Vec<Rect> = Vec::new();
    for b in rest {
        let bh = b[3] - b[1];
        let joined = blocks.iter_mut().find(|t| {
            b[1] - t[3] < BLOCK_GAP * bh && b[2].min(t[2]) > b[0].max(t[0]) && b[1] > t[1]
        });
        match joined {
            Some(t) => *t = union(t, &b),
            None => blocks.push(b),
        }
    }

    changed = true;
    while changed {
        changed = false;
        'blocks: for t in blocks.clone() {
            for d in diags.iter_mut() {
                if inter(&grow(d, JOIN_GROW), &t) > 0.0 {
                    *d = union(d, &t);
                    remove_first(&mut blocks, &t);
                    changed = true;
                    break 'blocks;
                }
            }
        }
        for i in 0..diags.len() {
            for j in i + 1..diags.len() {
                if inter(&grow(&diags[i], JOIN_GROW), &diags[j]) > 0.0 {
                    diags[i] = union(&diags[i], &diags[j]);
                    diags.remove(j);
                    changed = true;
                    break;
                }
            }
            if changed {
                break;
            }
        }
    }

    // `int()` truncates toward zero, as `as` does.
    let to_int = |b: &Rect| b.map(|c| c as i64);
    diags
        .iter()
        .map(|d| (RuleKind::Drawing, to_int(d)))
        .chain(blocks.iter().map(|b| (RuleKind::Text, to_int(b))))
        .collect()
}

/// A thin wide stroke right under a text line: an underline, not a drawing.
fn is_underline(b: &Rect, lines: &[Rect], lh: f64) -> bool {
    let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
    bh <= UNDERLINE_MAX_HEIGHT * lh
        && bw >= UNDERLINE_MIN_WIDTH * lh
        && lines.iter().any(|l| {
            b[1] - l[3] <= UNDERLINE_MAX_GAP * lh
                && b[2].min(l[2]) - b[0].max(l[0]) >= UNDERLINE_MIN_OVERLAP * bw
        })
}

/// Join every drawing group that the connector stroke `a`, grown by its reach, touches.
fn join_by_arrow(diags: &mut Vec<Rect>, a: &Rect) {
    let reach = grow(a, ARROW_REACH);
    let touched: Vec<usize> = (0..diags.len())
        .filter(|&i| inter(&reach, &diags[i]) > 0.0)
        .collect();
    if let [first, others @ ..] = touched.as_slice() {
        for &j in others.iter().rev() {
            diags[*first] = union(&diags[*first], &diags[j]);
            diags.remove(j);
        }
    }
}

/// Drawing strokes within 30 px of each other, as boxes of their drawing pixels. A group with
/// fewer than 400 drawing pixels in its box is dropped. The box ends at the last pixel
/// (inclusive), as the bake-off computes it.
fn drawing_groups(drawing: &Mask) -> Vec<Rect> {
    let wu = drawing.width as usize;
    let (dl, n) = label4(&dilate_cross(drawing, DRAWING_DILATE));
    // find_objects: the box of each label of the dilated drawing.
    let mut boxes: Vec<Option<[usize; 4]>> = vec![None; n as usize];
    for (i, &l) in dl.iter().enumerate() {
        if l == 0 {
            continue;
        }
        let (x, y) = (i % wu, i / wu);
        let b = boxes[l as usize - 1].get_or_insert([x, y, x + 1, y + 1]);
        *b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
    }
    let mut diags = Vec::new();
    for [x0, y0, x1, y1] in boxes.into_iter().flatten() {
        // All drawing pixels in the box count, also those of a neighbouring group.
        let mut count = 0usize;
        let (mut mx0, mut my0, mut mx1, mut my1) = (usize::MAX, usize::MAX, 0, 0);
        for y in y0..y1 {
            for x in x0..x1 {
                if drawing.bits[y * wu + x] {
                    count += 1;
                    mx0 = mx0.min(x);
                    my0 = my0.min(y);
                    mx1 = mx1.max(x);
                    my1 = my1.max(y);
                }
            }
        }
        if count < MIN_DRAWING_PIXELS {
            continue;
        }
        diags.push([mx0 as f64, my0 as f64, mx1 as f64, my1 as f64]);
    }
    diags
}

/// The C combination: the layout detector's drawn boxes, then the ink rules on the ink outside
/// them, every rule region as text.
pub fn combine_c(ink: &Mask, drawn: &[Rect], lines: &[Rect], fixes: RuleFixes) -> Vec<Proposed> {
    let mut masked = ink.clone();
    for d in drawn {
        let g = grow(d, DRAWN_GROW);
        let (x0, y0) = (g[0].floor() as i64, g[1].floor() as i64);
        let (x1, y1) = (g[2].ceil() as i64, g[3].ceil() as i64);
        let w = ink.width as usize;
        for y in py_range(y0.max(0), y1, ink.height) {
            masked.bits[y * w..(y + 1) * w][py_range(x0.max(0), x1, ink.width)].fill(false);
        }
    }
    let lines: Vec<Rect> = lines
        .iter()
        .filter(|b| {
            !drawn
                .iter()
                .any(|d| inter(&grow(d, DRAWN_GROW), b) / area(b) > DRAWN_LINE_SHARE)
        })
        .copied()
        .collect();
    let drawn_regions = drawn.iter().map(|d| Proposed {
        drawn: true,
        bbox: d.map(|c| c as i64),
        possible_drawing: false,
    });
    let rule_regions = ink_rules(&masked, &lines, fixes)
        .into_iter()
        .map(|(kind, bbox)| Proposed {
            drawn: false,
            bbox,
            // The rules saw a drawing that the layout detector did not draw: it is read as text,
            // with a note.
            possible_drawing: kind == RuleKind::Drawing,
        });
    drawn_regions.chain(rule_regions).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(m: &mut Mask, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..y1 {
            for x in x0..x1 {
                m.set(x, y, true);
            }
        }
    }

    #[test]
    fn merge_lines_joins_at_more_than_half_of_the_smaller() {
        let a = [0.0, 0.0, 10.0, 10.0];
        // Overlap 50 of 100: apart.
        assert_eq!(merge_lines(&[a, [5.0, 0.0, 30.0, 10.0]]).len(), 2);
        // Overlap 51 of 100: one line.
        let m = merge_lines(&[a, [4.9, 0.0, 30.0, 10.0]]);
        assert_eq!(m, vec![[0.0, 0.0, 30.0, 10.0]]);
    }

    #[test]
    fn merge_lines_repeats_until_nothing_joins() {
        // The union of the first two then swallows the third.
        let m = merge_lines(&[
            [0.0, 0.0, 10.0, 10.0],
            [2.0, 0.0, 12.0, 10.0],
            [9.0, 0.0, 13.0, 10.0],
        ]);
        assert_eq!(m, vec![[0.0, 0.0, 13.0, 10.0]]);
    }

    #[test]
    fn round_is_half_to_even() {
        // A line edge at 10.5 masks from column 10, at 11.5 from 12.
        let m = text_mask(&[[10.5, 0.0, 20.0, 1.0]], 30, 1);
        assert!(m.get(10, 0) && !m.get(9, 0));
        let m = text_mask(&[[11.5, 0.0, 20.0, 1.0]], 30, 1);
        assert!(m.get(12, 0) && !m.get(11, 0));
    }

    #[test]
    fn text_mask_clips_to_the_page() {
        let m = text_mask(&[[-5.0, -5.0, 50.0, 50.0]], 4, 3);
        assert_eq!(m.count(), 12);
    }

    #[test]
    fn median_line_height_of_two_is_their_mean() {
        let lines = [[0.0, 0.0, 10.0, 10.0], [0.0, 50.0, 10.0, 70.0]];
        assert_eq!(line_height(&lines), 15.0);
        let three = [lines[0], lines[1], [0.0, 100.0, 10.0, 130.0]];
        assert_eq!(line_height(&three), 20.0);
    }

    #[test]
    fn no_lines_uses_60_px_line_height() {
        assert_eq!(line_height(&[]), 60.0);
        // Without lines no ink is inside text, so a stroke longer than 1.2 * 60 = 72 is drawing.
        let mut ink = Mask::new(200, 100);
        fill(&mut ink, 10, 10, 83, 20);
        assert_eq!(
            ink_rules(&ink, &[], RuleFixes::NONE),
            vec![(RuleKind::Drawing, [10, 10, 82, 19])]
        );
        let mut ink = Mask::new(200, 100);
        fill(&mut ink, 10, 10, 82, 20);
        assert_eq!(
            ink_rules(&ink, &[], RuleFixes::NONE),
            vec![(RuleKind::Text, [10, 10, 82, 20])]
        );
    }

    #[test]
    fn small_component_below_30_px_is_ignored() {
        let mut ink = Mask::new(100, 20);
        fill(&mut ink, 10, 5, 39, 6);
        assert!(ink_rules(&ink, &[], RuleFixes::NONE).is_empty());
        fill(&mut ink, 39, 5, 40, 6);
        assert_eq!(
            ink_rules(&ink, &[], RuleFixes::NONE),
            vec![(RuleKind::Text, [10, 5, 40, 6])]
        );
    }

    #[test]
    fn drawing_group_needs_400_pixels() {
        // A one-pixel stroke of 399 px is drawing (long, outside text) but too small a group.
        let mut ink = Mask::new(500, 20);
        fill(&mut ink, 10, 5, 409, 6);
        assert!(ink_rules(&ink, &[], RuleFixes::NONE).is_empty());
        fill(&mut ink, 409, 5, 410, 6);
        // The drawing box ends at its last pixel.
        assert_eq!(
            ink_rules(&ink, &[], RuleFixes::NONE),
            vec![(RuleKind::Drawing, [10, 5, 409, 5])]
        );
    }

    #[test]
    fn drawing_strokes_join_within_30_px() {
        // Two dilations of 30 touch when the strokes are at most 61 px apart.
        let rules_with_second_stroke_at = |x0: u32| {
            let mut ink = Mask::new(400, 100);
            fill(&mut ink, 10, 10, 110, 15);
            fill(&mut ink, x0, 10, x0 + 100, 15);
            ink_rules(&ink, &[], RuleFixes::NONE)
        };
        assert_eq!(
            rules_with_second_stroke_at(170),
            vec![(RuleKind::Drawing, [10, 10, 269, 14])]
        );
        assert_eq!(rules_with_second_stroke_at(171).len(), 2);
    }

    #[test]
    fn line_close_to_a_drawing_is_absorbed() {
        let mut ink = Mask::new(400, 200);
        fill(&mut ink, 10, 10, 200, 15);
        // A text line just under the drawing, inside its 20 px margin.
        let rules = ink_rules(&ink, &[[20.0, 20.0, 100.0, 40.0]], RuleFixes::NONE);
        assert_eq!(rules, vec![(RuleKind::Drawing, [10, 10, 199, 40])]);
    }

    #[test]
    fn text_lines_join_into_a_block() {
        let lines = [[10.0, 10.0, 100.0, 30.0], [10.0, 47.0, 100.0, 67.0]];
        // Gap 17 < 0.9 * 20: one block.
        assert_eq!(
            ink_rules(&Mask::new(200, 200), &lines, RuleFixes::NONE),
            vec![(RuleKind::Text, [10, 10, 100, 67])]
        );
        // Gap 18 is not below 18: two blocks.
        let lines = [lines[0], [10.0, 48.0, 100.0, 68.0]];
        assert_eq!(
            ink_rules(&Mask::new(200, 200), &lines, RuleFixes::NONE).len(),
            2
        );
    }

    #[test]
    fn hybrid_drops_line_mostly_under_drawn_box() {
        let ink = Mask::new(400, 400);
        let drawn = [[100.0, 100.0, 200.0, 200.0]];
        // The grown drawn box ends at 215: 50 of 100 columns under it, the line stays.
        let kept = combine_c(
            &ink,
            &drawn,
            &[[165.0, 150.0, 265.0, 160.0]],
            RuleFixes::NONE,
        );
        assert_eq!(kept.len(), 2);
        assert!(kept[0].drawn && !kept[1].drawn);
        assert_eq!(kept[1].bbox, [165, 150, 265, 160]);
        // 51 of 100 columns: the line goes.
        let dropped = combine_c(
            &ink,
            &drawn,
            &[[164.0, 150.0, 264.0, 160.0]],
            RuleFixes::NONE,
        );
        assert_eq!(dropped.len(), 1);
    }

    #[test]
    fn hybrid_masks_the_ink_under_the_grown_drawn_box() {
        let mut ink = Mask::new(400, 400);
        // A 40 px stroke right of the drawn box: inside the 15 px margin, then just outside it.
        fill(&mut ink, 214, 150, 254, 151);
        let drawn = [[100.0, 100.0, 200.0, 200.0]];
        let regions = combine_c(&ink, &drawn, &[], RuleFixes::NONE);
        // Column 214 is masked, 215 to 253 stay: 39 px, still a component.
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[1].bbox, [215, 150, 254, 151]);
    }

    /// One text line [100, 100, 400, 200], so the line height is 100, and a bar of `bar_h` rows
    /// at x0..x1 whose top is `gap` px below the line.
    fn underline_page(x0: u32, x1: u32, gap: u32, bar_h: u32) -> (Mask, Vec<Rect>) {
        let mut ink = Mask::new(700, 500);
        fill(&mut ink, x0, 200 + gap, x1, 200 + gap + bar_h);
        (ink, vec![[100.0, 100.0, 400.0, 200.0]])
    }

    fn has_drawing(rules: &[(RuleKind, [i64; 4])]) -> bool {
        rules.iter().any(|(k, _)| *k == RuleKind::Drawing)
    }

    #[test]
    fn underline_under_text_is_text() {
        let (ink, lines) = underline_page(100, 400, 0, 4);
        assert!(has_drawing(&ink_rules(&ink, &lines, RuleFixes::NONE)));
        let rules = ink_rules(&ink, &lines, RuleFixes::ALL);
        assert_eq!(rules, vec![(RuleKind::Text, [100, 100, 400, 204])]);
    }

    #[test]
    fn thick_bar_under_text_is_still_drawing() {
        // 0.35 lh is an underline, 0.36 lh is not.
        let (ink, lines) = underline_page(100, 400, 0, 35);
        assert!(!has_drawing(&ink_rules(&ink, &lines, RuleFixes::ALL)));
        let (ink, lines) = underline_page(100, 400, 0, 36);
        assert!(has_drawing(&ink_rules(&ink, &lines, RuleFixes::ALL)));
    }

    #[test]
    fn underline_far_below_text_is_drawing() {
        // A gap of 0.5 lh is an underline, 0.51 lh is not.
        let (ink, lines) = underline_page(100, 400, 50, 4);
        assert!(!has_drawing(&ink_rules(&ink, &lines, RuleFixes::ALL)));
        let (ink, lines) = underline_page(100, 400, 51, 4);
        assert!(has_drawing(&ink_rules(&ink, &lines, RuleFixes::ALL)));
    }

    #[test]
    fn underline_needs_half_its_width_under_the_line() {
        // 150 of 300 columns under the line is an underline, 147 of 300 is not.
        let (ink, lines) = underline_page(250, 550, 0, 4);
        assert!(!has_drawing(&ink_rules(&ink, &lines, RuleFixes::ALL)));
        let (ink, lines) = underline_page(253, 553, 0, 4);
        assert!(has_drawing(&ink_rules(&ink, &lines, RuleFixes::ALL)));
    }

    #[test]
    fn underline_just_wider_than_1_2_lines_is_text() {
        // 121 px at line height 100: a drawing without the fix, text with it. A minimum width
        // above 1.21 line heights would turn it back into a drawing.
        let (ink, lines) = underline_page(100, 221, 0, 4);
        assert!(has_drawing(&ink_rules(&ink, &lines, RuleFixes::NONE)));
        assert!(!has_drawing(&ink_rules(&ink, &lines, RuleFixes::ALL)));
    }

    /// Two L-shaped drawing parts whose boxes are 100 px apart and whose strokes are far from
    /// the middle of the page, and a one-pixel stroke from (x0, 250) to (x1 - 1, 250 + h - 1).
    /// `extra` more pixels go right under the stroke, to set its fill.
    fn arrow_page(x0: u32, x1: u32, h: u32, extra: u32) -> Mask {
        let mut ink = Mask::new(800, 400);
        // Part A: box [0, 0, 299, 299], strokes along its top and left side.
        fill(&mut ink, 0, 0, 300, 10);
        fill(&mut ink, 0, 0, 10, 300);
        // Part B: box [400, 0, 699, 299], strokes along its top and right side.
        fill(&mut ink, 400, 0, 700, 10);
        fill(&mut ink, 690, 0, 700, 300);
        let n = x1 - x0;
        let mut added = 0;
        for i in 0..n {
            let y = 250 + (f64::from(i) * f64::from(h - 1) / f64::from(n - 1)).round() as u32;
            ink.set(x0 + i, y, true);
            if added < extra && y + 1 < 250 + h && !ink.get(x0 + i, y + 1) {
                ink.set(x0 + i, y + 1, true);
                added += 1;
            }
        }
        assert_eq!(added, extra, "room for the extra pixels");
        ink
    }

    fn drawings(ink: &Mask, fixes: RuleFixes) -> usize {
        ink_rules(ink, &[], fixes)
            .iter()
            .filter(|(k, _)| *k == RuleKind::Drawing)
            .count()
    }

    #[test]
    fn long_arrow_joins_two_drawing_parts() {
        // 201 x 33: long side 6.09 times the short side, fill 201 / 6633 = 0.030.
        let ink = arrow_page(250, 451, 33, 0);
        assert_eq!(drawings(&ink, RuleFixes::NONE), 2);
        assert_eq!(drawings(&ink, RuleFixes::ALL), 1);
    }

    #[test]
    fn short_arrow_between_parts_does_not_join() {
        // 201 x 34: long side 5.91 times the short side.
        let ink = arrow_page(250, 451, 34, 0);
        assert_eq!(drawings(&ink, RuleFixes::ALL), 2);
    }

    #[test]
    fn dense_bar_between_parts_does_not_join() {
        // 201 x 33 = 6633 px of box: 265 px is a fill of 0.03995, 266 px is 0.0401.
        let ink = arrow_page(250, 451, 33, 64);
        assert_eq!(drawings(&ink, RuleFixes::ALL), 1);
        let ink = arrow_page(250, 451, 33, 65);
        assert_eq!(drawings(&ink, RuleFixes::ALL), 2);
        // A straight bar is no arrow at all: fill 1.
        let mut ink = arrow_page(250, 451, 33, 0);
        for x in 250..451 {
            for y in 250..283 {
                ink.set(x, y, false);
            }
        }
        fill(&mut ink, 280, 250, 430, 252);
        assert_eq!(drawings(&ink, RuleFixes::ALL), 2);
    }

    #[test]
    fn arrow_reaches_35_px() {
        // The stroke starts right of part A: 333 is within 35 px of A's last column 299 (the
        // grown box overlaps it by one column), 334 is not.
        let ink = arrow_page(333, 534, 33, 0);
        assert_eq!(drawings(&ink, RuleFixes::ALL), 1);
        let ink = arrow_page(334, 535, 33, 0);
        assert_eq!(drawings(&ink, RuleFixes::ALL), 2);
    }

    #[test]
    fn rule_drawing_outside_drawn_boxes_is_a_possible_drawing() {
        let mut ink = Mask::new(400, 100);
        fill(&mut ink, 10, 10, 200, 15);
        let regions = combine_c(&ink, &[], &[], RuleFixes::ALL);
        assert_eq!(
            regions,
            vec![Proposed {
                drawn: false,
                bbox: [10, 10, 199, 14],
                possible_drawing: true,
            }]
        );
        // Text from the rules carries no note.
        let mut ink = Mask::new(400, 100);
        fill(&mut ink, 10, 10, 40, 15);
        assert!(!combine_c(&ink, &[], &[], RuleFixes::ALL)[0].possible_drawing);
    }

    #[test]
    fn text_block_within_35_px_joins_the_drawing() {
        let mut ink = Mask::new(400, 200);
        fill(&mut ink, 10, 10, 200, 15);
        // The drawing box ends at row 14; out of the 20 px absorb reach, in the 35 px join reach.
        let rules = ink_rules(&ink, &[[20.0, 48.0, 100.0, 68.0]], RuleFixes::NONE);
        assert_eq!(rules, vec![(RuleKind::Drawing, [10, 10, 199, 68])]);
        let rules = ink_rules(&ink, &[[20.0, 49.0, 100.0, 69.0]], RuleFixes::NONE);
        assert_eq!(rules.len(), 2);
    }

    /// A drawing [10, 10, 199, 14], a short line A near it and a tall line B 50 px under A.
    /// When A is absorbed first, B is out of the drawing's reach and stays text. When A is not,
    /// A and B form one block and the block joins the drawing.
    fn absorb_page(a: Rect) -> Vec<(RuleKind, [i64; 4])> {
        let mut ink = Mask::new(500, 400);
        fill(&mut ink, 10, 10, 200, 15);
        let b = [a[0] + 10.0, a[3] + 50.0, a[2] + 10.0, a[3] + 150.0];
        ink_rules(&ink, &[a, b], RuleFixes::NONE)
    }

    #[test]
    fn line_more_than_30_percent_in_the_absorb_margin_is_absorbed() {
        // The grown drawing ends at column 219: 31 of 100 columns is absorbed, 30 is not.
        assert_eq!(
            absorb_page([188.0, 24.0, 288.0, 34.0]),
            vec![
                (RuleKind::Drawing, [10, 10, 288, 34]),
                (RuleKind::Text, [198, 84, 298, 184])
            ]
        );
        assert_eq!(
            absorb_page([189.0, 24.0, 289.0, 34.0]),
            vec![(RuleKind::Drawing, [10, 10, 299, 184])]
        );
    }

    #[test]
    fn absorb_margin_is_20_px() {
        // The grown drawing ends at row 34: 4 of 10 rows is absorbed, 3 is not.
        assert_eq!(absorb_page([20.0, 30.0, 120.0, 40.0]).len(), 2);
        assert_eq!(
            absorb_page([20.0, 31.0, 120.0, 41.0]),
            vec![(RuleKind::Drawing, [10, 10, 199, 191])]
        );
    }

    /// Line height 100 (two lines), and a line M [500, 380, m_x1, 480] over part of the ink.
    fn classify(ink: &Mask, m_x1: f64) -> Vec<(RuleKind, [i64; 4])> {
        let lines = [[0.0, 0.0, 100.0, 100.0], [500.0, 380.0, m_x1, 480.0]];
        ink_rules(ink, &lines, RuleFixes::NONE)
    }

    fn bar(width: u32) -> Mask {
        let mut ink = Mask::new(1000, 800);
        fill(&mut ink, 500, 400, 500 + width, 405);
        ink
    }

    #[test]
    fn stroke_longer_than_2_5_lines_is_drawing_when_not_mostly_text() {
        // Half under a text line: 250 px long is not drawing, 251 px is.
        assert!(!has_drawing(&classify(&bar(250), 625.0)));
        assert!(has_drawing(&classify(&bar(251), 625.0)));
        // 300 px long: 85 % under a text line is not drawing, 84 % is.
        assert!(!has_drawing(&classify(&bar(300), 755.0)));
        assert!(has_drawing(&classify(&bar(300), 752.0)));
    }

    #[test]
    fn stroke_longer_than_1_2_lines_is_drawing_when_nearly_all_outside_text() {
        // 200 px long: 30 % under a text line is not drawing, 29 % is.
        assert!(!has_drawing(&classify(&bar(200), 560.0)));
        assert!(has_drawing(&classify(&bar(200), 558.0)));
    }

    /// A one-pixel square outline of side `side` at (500, 400), with `extra` pixels filled
    /// inside it row by row.
    fn outline(side: u32, extra: u32) -> Mask {
        let mut ink = Mask::new(1000, 800);
        let (x0, y0, x1, y1) = (500, 400, 500 + side, 400 + side);
        fill(&mut ink, x0, y0, x1, y0 + 1);
        fill(&mut ink, x0, y1 - 1, x1, y1);
        fill(&mut ink, x0, y0, x0 + 1, y1);
        fill(&mut ink, x1 - 1, y0, x1, y1);
        let mut left = extra;
        'rows: for y in y0 + 1..y1 - 1 {
            for x in x0 + 1..x1 - 1 {
                if left == 0 {
                    break 'rows;
                }
                ink.set(x, y, true);
                left -= 1;
            }
        }
        ink
    }

    /// Line height 100 (the median of 100, 100 and 200), and a line M [500, 380, m_x1, 580]
    /// over the whole height of the outlines below.
    fn classify_tall(ink: &Mask, m_x1: f64) -> Vec<(RuleKind, [i64; 4])> {
        let lines = [
            [0.0, 0.0, 100.0, 100.0],
            [0.0, 200.0, 100.0, 300.0],
            [500.0, 380.0, m_x1, 580.0],
        ];
        ink_rules(ink, &lines, RuleFixes::NONE)
    }

    #[test]
    fn sparse_shape_longer_than_1_5_lines_is_drawing() {
        // About half under a text line, so only the sparse rule can make it a drawing.
        assert!(!has_drawing(&classify_tall(&outline(150, 0), 575.0)));
        assert!(has_drawing(&classify_tall(&outline(151, 0), 575.0)));
        // Side 160, box 25600 px: 1023 px (fill below 0.04) is sparse, 1024 px is not.
        assert_eq!(outline(160, 387).count(), 1023);
        assert!(has_drawing(&classify_tall(&outline(160, 387), 580.0)));
        assert!(!has_drawing(&classify_tall(&outline(160, 388), 580.0)));
    }

    #[test]
    fn ink_less_than_half_in_text_is_an_orphan() {
        // A 100 px stroke, no drawing: half under the line M adds nothing, 49 % is an orphan
        // that joins M's block.
        let rules = classify(&bar(100), 550.0);
        assert_eq!(rules[1], (RuleKind::Text, [500, 380, 550, 480]));
        let rules = classify(&bar(100), 549.0);
        assert_eq!(rules[1], (RuleKind::Text, [500, 380, 600, 480]));
    }
}
