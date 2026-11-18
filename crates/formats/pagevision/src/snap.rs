// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Ink snapping: every proposed region is shrunk or grown to the ink components it covers.
//!
//! A component belongs to a region when at least half of its ink lies inside the region box.
//! Regions that share a component are resolved until none do:
//! - two drawn regions merge (one drawing found as several boxes);
//! - two text regions merge;
//! - a drawn region keeps the component and the text region loses it, so text under a drawing
//!   is not read twice.
//!
//! A region without ink is dropped, with a note.

use std::collections::BTreeSet;

use crate::ink::{Component, Mask, components};
use crate::rules::Proposed;

/// Ink closer than this many pixels is one component, as in the ink rules (pinned in ink.rs
/// `components_split_beyond_the_dilation`).
const SNAP_DILATE: u32 = 3;
/// The snapped box is grown by this many pixels: the ink mask stops at grey 110, so the lighter
/// edge of a stroke lies just outside it.
const SNAP_MARGIN: i64 = 4;

/// A region snapped to its ink.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapped {
    /// The layout detector drew this region.
    pub drawn: bool,
    /// x0, y0, x1, y1 in page pixels, x1 and y1 exclusive.
    pub bbox: [i64; 4],
    /// The ink rules saw a drawing here that the layout detector missed.
    pub possible_drawing: bool,
    /// The share of the ink pixels inside the proposed box that the region's own components
    /// hold. Low when the box cuts through ink that belongs elsewhere.
    pub coverage: f32,
}

/// A region while it is being resolved.
struct Work {
    drawn: bool,
    possible_drawing: bool,
    /// The proposed box, merged boxes as their union. Ownership and coverage use it.
    proposed: [i64; 4],
    /// Components a drawn region took from this text region.
    lost: BTreeSet<usize>,
    owned: Vec<usize>,
}

/// The ink pixels of component `c` inside `b`, and all of `b`'s ink pixels, counted by label.
struct Page<'a> {
    ink: &'a Mask,
    labels: &'a [u32],
    comps: &'a [Component],
}

impl Page<'_> {
    /// The box clipped to the page, as a pixel range.
    fn clip(&self, b: [i64; 4]) -> [usize; 4] {
        let cx = |v: i64| v.clamp(0, i64::from(self.ink.width)) as usize;
        let cy = |v: i64| v.clamp(0, i64::from(self.ink.height)) as usize;
        [cx(b[0]), cy(b[1]), cx(b[2]), cy(b[3])]
    }

    /// Ink pixels per component label inside the box (index 0 unused).
    fn pixels_inside(&self, b: [i64; 4]) -> Vec<u32> {
        let [x0, y0, x1, y1] = self.clip(b);
        let w = self.ink.width as usize;
        let mut counts = vec![0u32; self.comps.len() + 1];
        for y in y0..y1 {
            for x in x0..x1 {
                counts[self.labels[y * w + x] as usize] += 1;
            }
        }
        counts
    }

    /// The components with at least half of their ink inside the box, without `lost`.
    fn owned(&self, b: [i64; 4], lost: &BTreeSet<usize>) -> Vec<usize> {
        let inside = self.pixels_inside(b);
        self.comps
            .iter()
            .enumerate()
            // At least half: a component goes with the region that holds most of it.
            .filter(|&(i, c)| !lost.contains(&i) && 2 * inside[c.label as usize] >= c.pixels)
            .map(|(i, _)| i)
            .collect()
    }
}

fn union(a: [i64; 4], b: [i64; 4]) -> [i64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn fmt_box(b: [i64; 4]) -> String {
    format!("{},{},{},{}", b[0], b[1], b[2], b[3])
}

/// Snap every region to the ink components it covers. Returns the snapped regions, in the order
/// of the regions given (a merged region at the place of its first part), and the debug notes.
pub fn snap(ink: &Mask, regions: &[Proposed]) -> (Vec<Snapped>, Vec<String>) {
    let (labels, comps) = components(ink, SNAP_DILATE);
    let page = Page {
        ink,
        labels: &labels,
        comps: &comps,
    };
    let mut notes = Vec::new();
    let mut work: Vec<Work> = regions
        .iter()
        .map(|r| {
            let lost = BTreeSet::new();
            Work {
                drawn: r.drawn,
                possible_drawing: r.possible_drawing,
                proposed: r.bbox,
                owned: page.owned(r.bbox, &lost),
                lost,
            }
        })
        .collect();

    // Resolve one shared component at a time, until no component has two owners.
    while let Some((c, i, j)) = first_shared(&work, comps.len()) {
        if work[i].drawn == work[j].drawn {
            let other = work.remove(j);
            let w = &mut work[i];
            w.proposed = union(w.proposed, other.proposed);
            w.possible_drawing |= other.possible_drawing;
            w.lost.extend(other.lost);
            w.owned = page.owned(w.proposed, &w.lost);
        } else {
            let text = if work[i].drawn { j } else { i };
            work[text].lost.insert(c);
            work[text].owned.retain(|&o| o != c);
        }
    }

    let mut out = Vec::new();
    for w in work {
        if w.owned.is_empty() {
            if w.lost.is_empty() {
                notes.push(format!("empty region dropped at {}", fmt_box(w.proposed)));
            } else {
                notes.push(format!(
                    "text region dropped at {}: its ink is in a drawn region",
                    fmt_box(w.proposed)
                ));
            }
            continue;
        }
        let mut b = [i64::MAX, i64::MAX, i64::MIN, i64::MIN];
        for &o in &w.owned {
            let c = &comps[o];
            b = union(
                b,
                [
                    i64::from(c.x0),
                    i64::from(c.y0),
                    i64::from(c.x1),
                    i64::from(c.y1),
                ],
            );
        }
        let grown = [
            b[0] - SNAP_MARGIN,
            b[1] - SNAP_MARGIN,
            b[2] + SNAP_MARGIN,
            b[3] + SNAP_MARGIN,
        ];
        let clipped = page.clip(grown).map(|v| v as i64);
        let inside = page.pixels_inside(w.proposed);
        let all: u32 = inside[1..].iter().sum();
        let own: u32 = w
            .owned
            .iter()
            .map(|&o| inside[comps[o].label as usize])
            .sum();
        out.push(Snapped {
            drawn: w.drawn,
            bbox: clipped,
            possible_drawing: w.possible_drawing,
            // An owned component has at least one pixel inside, so `all` is not 0.
            coverage: own as f32 / all as f32,
        });
    }
    (out, notes)
}

/// The first component (in label order) owned by two regions, with the first two owners.
fn first_shared(work: &[Work], n: usize) -> Option<(usize, usize, usize)> {
    let mut first_owner: Vec<Option<usize>> = vec![None; n];
    let mut found: Option<(usize, usize, usize)> = None;
    for (r, w) in work.iter().enumerate() {
        for &c in &w.owned {
            match first_owner[c] {
                None => first_owner[c] = Some(r),
                Some(i) if found.is_none_or(|(fc, _, _)| c < fc) => found = Some((c, i, r)),
                Some(_) => {}
            }
        }
    }
    found
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

    fn region(drawn: bool, bbox: [i64; 4]) -> Proposed {
        Proposed {
            drawn,
            bbox,
            possible_drawing: false,
        }
    }

    #[test]
    fn box_shrinks_to_its_ink() {
        let mut ink = Mask::new(200, 200);
        fill(&mut ink, 40, 40, 60, 50);
        let (out, notes) = snap(&ink, &[region(false, [0, 0, 100, 100])]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].bbox, [36, 36, 64, 54]);
        assert_eq!(out[0].coverage, 1.0);
        assert!(notes.is_empty());
    }

    #[test]
    fn box_grows_to_a_component_that_is_mostly_inside() {
        let mut ink = Mask::new(200, 200);
        // 20 of 30 columns inside the box.
        fill(&mut ink, 30, 10, 60, 20);
        let (out, _) = snap(&ink, &[region(false, [0, 0, 50, 50])]);
        assert_eq!(out[0].bbox, [26, 6, 64, 24]);
    }

    #[test]
    fn component_mostly_outside_is_not_taken() {
        let mut ink = Mask::new(300, 100);
        fill(&mut ink, 10, 10, 30, 20);
        // 49 of 100 columns inside the box: not taken.
        fill(&mut ink, 51, 50, 151, 51);
        let (out, _) = snap(&ink, &[region(false, [0, 0, 100, 100])]);
        assert_eq!(out[0].bbox, [6, 6, 34, 24]);
        // 50 of 100 columns: taken.
        let mut ink = Mask::new(300, 100);
        fill(&mut ink, 10, 10, 30, 20);
        fill(&mut ink, 50, 50, 150, 51);
        let (out, _) = snap(&ink, &[region(false, [0, 0, 100, 100])]);
        assert_eq!(out[0].bbox, [6, 6, 154, 55]);
    }

    #[test]
    fn coverage_is_own_ink_over_all_ink_in_the_proposed_box() {
        let mut ink = Mask::new(300, 100);
        // 200 owned pixels, and 40 of a 100 px stroke that is mostly outside.
        fill(&mut ink, 10, 10, 30, 20);
        fill(&mut ink, 60, 50, 160, 51);
        let (out, _) = snap(&ink, &[region(false, [0, 0, 100, 100])]);
        assert_eq!(out[0].coverage, 200.0 / 240.0);
    }

    #[test]
    fn snapped_box_is_clipped_to_the_page() {
        let mut ink = Mask::new(50, 50);
        fill(&mut ink, 0, 0, 10, 10);
        let (out, _) = snap(&ink, &[region(true, [-5, -5, 20, 20])]);
        assert_eq!(out[0].bbox, [0, 0, 14, 14]);
    }

    #[test]
    fn empty_box_is_dropped_with_a_note() {
        let mut ink = Mask::new(200, 200);
        fill(&mut ink, 10, 10, 20, 20);
        let (out, notes) = snap(
            &ink,
            &[
                region(false, [0, 0, 50, 50]),
                region(true, [100, 100, 150, 150]),
            ],
        );
        assert_eq!(out.len(), 1);
        assert!(!out[0].drawn);
        assert_eq!(notes, vec!["empty region dropped at 100,100,150,150"]);
    }

    #[test]
    fn two_drawn_boxes_sharing_ink_merge() {
        let mut ink = Mask::new(300, 200);
        // One drawing: two shapes and a stroke that joins them; the detector drew one box per
        // shape, and both boxes hold the joining stroke's middle.
        fill(&mut ink, 10, 10, 60, 60);
        fill(&mut ink, 140, 10, 190, 60);
        fill(&mut ink, 60, 30, 140, 32);
        let regions = [
            region(true, [0, 0, 105, 70]),
            region(true, [95, 0, 200, 70]),
        ];
        let (out, notes) = snap(&ink, &regions);
        assert_eq!(out.len(), 1);
        assert!(out[0].drawn);
        assert_eq!(out[0].bbox, [6, 6, 194, 64]);
        assert!(notes.is_empty());
    }

    #[test]
    fn two_text_boxes_sharing_ink_merge() {
        let mut ink = Mask::new(300, 100);
        fill(&mut ink, 10, 10, 100, 20);
        let regions = [
            region(false, [0, 0, 60, 30]),
            region(false, [50, 0, 120, 30]),
        ];
        let (out, _) = snap(&ink, &regions);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].bbox, [6, 6, 104, 24]);
    }

    #[test]
    fn text_line_under_drawing_is_not_doubled() {
        let mut ink = Mask::new(300, 300);
        // A drawn frame with a text line inside it.
        fill(&mut ink, 20, 20, 200, 23);
        fill(&mut ink, 20, 197, 200, 200);
        fill(&mut ink, 20, 20, 23, 200);
        fill(&mut ink, 197, 20, 200, 200);
        fill(&mut ink, 50, 100, 150, 110);
        let regions = [
            region(true, [10, 10, 210, 210]),
            region(false, [40, 90, 160, 120]),
        ];
        let (out, notes) = snap(&ink, &regions);
        assert_eq!(out.len(), 1);
        assert!(out[0].drawn);
        assert_eq!(
            notes,
            vec!["text region dropped at 40,90,160,120: its ink is in a drawn region"]
        );
    }

    #[test]
    fn text_keeps_its_own_ink_next_to_a_drawing() {
        let mut ink = Mask::new(400, 300);
        fill(&mut ink, 20, 20, 200, 200);
        fill(&mut ink, 250, 100, 350, 110);
        // The text box reaches into the drawing and holds the whole drawing stroke too.
        let regions = [
            region(true, [10, 10, 210, 210]),
            region(false, [0, 0, 360, 220]),
        ];
        let (out, _) = snap(&ink, &regions);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].bbox, [246, 96, 354, 114]);
        // The drawing's ink stays inside the text box, so the text covers little of it.
        assert!(out[1].coverage < 0.05);
    }

    #[test]
    fn merge_keeps_the_possible_drawing_note() {
        let mut ink = Mask::new(300, 100);
        fill(&mut ink, 10, 10, 100, 20);
        let mut flagged = region(false, [50, 0, 120, 30]);
        flagged.possible_drawing = true;
        let (out, _) = snap(&ink, &[region(false, [0, 0, 60, 30]), flagged]);
        assert!(out[0].possible_drawing);
    }

    #[test]
    fn snap_keeps_a_page_without_regions_empty() {
        let mut ink = Mask::new(50, 50);
        fill(&mut ink, 0, 0, 10, 10);
        let (out, notes) = snap(&ink, &[]);
        assert!(out.is_empty() && notes.is_empty());
    }
}
