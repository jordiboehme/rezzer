// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The regions of a page from the detector boxes: the ink rules, then ink snapping, then the
//! 0..=1000 grid in reading order. Default code, so it is tested without the models.

use crate::ink::{Gray, INK_VERSION, ink_mask};
use crate::model::{
    CANDLE_VERSION, LAYOUT_V3, LLAMA_CPP_2_VERSION, PREPROCESS_VERSION, TEXT_DET_V6,
};
use crate::rules::{RULES_VERSION, Rect, RuleFixes, combine_c};
use crate::snap::snap;
use crate::types::{BBox, Fingerprint, Region, RegionKind, Regions};

/// The note on a text region that the ink rules saw as a drawing the layout detector missed.
pub const POSSIBLE_DRAWING_NOTE: &str = "possible missed drawing";

/// The certainty of a possible missed drawing is its coverage times this. It only has to put
/// such a region below every region without the note that has the same coverage; the note says
/// why.
const POSSIBLE_DRAWING_FACTOR: f32 = 0.7;

/// The grid value of pixel `v` on an axis of `size` pixels: `v * 1000 / size`, rounded half up,
/// with `v` clamped to the page.
fn to_grid(v: i64, size: u32) -> u16 {
    if size == 0 {
        return 0;
    }
    let size = i64::from(size);
    let v = v.clamp(0, size);
    // floor(v * 1000 / size + 1/2) in integers; at most 1000.
    ((v * 2000 + size) / (2 * size)) as u16
}

/// A half-open pixel box on the 0..=1000 grid of a `width` x `height` page.
pub(crate) fn grid_box(b: [i64; 4], width: u32, height: u32) -> BBox {
    BBox {
        x0: to_grid(b[0], width),
        y0: to_grid(b[1], height),
        x1: to_grid(b[2], width),
        y1: to_grid(b[3], height),
    }
}

/// The regions of a page from the layout detector's drawn boxes and the text lines (both in
/// page pixels):
/// 1. the ink mask of the page;
/// 2. the C combination with every rule fix;
/// 3. ink snapping;
/// 4. the boxes on the 0..=1000 grid of the page (rounded half up, clamped);
/// 5. reading order: top to bottom, ties left to right;
/// 6. certainty: the coverage, times 0.7 for a possible missed drawing, which also gets a note.
pub fn regions_from(gray: &Gray, drawn: &[Rect], lines: &[Rect]) -> Regions {
    let ink = ink_mask(gray);
    let proposed = combine_c(&ink, drawn, lines, RuleFixes::ALL);
    let (snapped, notes) = snap(&ink, &proposed);
    let mut regions: Vec<Region> = snapped
        .into_iter()
        .map(|s| Region {
            bbox: grid_box(s.bbox, gray.width, gray.height),
            kind: if s.drawn {
                RegionKind::Drawing
            } else {
                RegionKind::Text
            },
            certainty: if s.possible_drawing {
                s.coverage * POSSIBLE_DRAWING_FACTOR
            } else {
                s.coverage
            },
            note: s
                .possible_drawing
                .then(|| POSSIBLE_DRAWING_NOTE.to_string()),
        })
        .collect();
    // A stable sort: regions with the same corner keep the snapper's order.
    regions.sort_by_key(|r| (r.bbox.y0, r.bbox.x0));
    Regions {
        width_px: gray.width,
        height_px: gray.height,
        regions,
        notes,
    }
}

/// The fingerprint of a regions-only run: the detector files, candle, the rules and the ink.
/// The model fields stay empty, since no vision model runs.
pub fn regions_fingerprint() -> Fingerprint {
    Fingerprint {
        crate_version: env!("CARGO_PKG_VERSION").to_string(),
        llama_cpp_2: LLAMA_CPP_2_VERSION.to_string(),
        model_sha256: Vec::new(),
        prompt_sha256: String::new(),
        preprocess: PREPROCESS_VERSION,
        layout_sha256: LAYOUT_V3.sha256.to_string(),
        text_det_sha256: TEXT_DET_V6.sha256.to_string(),
        candle: CANDLE_VERSION.to_string(),
        rules: RULES_VERSION,
        ink: INK_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white page.
    fn page(width: u32, height: u32) -> Gray {
        Gray {
            width,
            height,
            data: vec![255; width as usize * height as usize],
        }
    }

    /// Black ink over x0..x1, y0..y1.
    fn ink(g: &mut Gray, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..y1 {
            for x in x0..x1 {
                g.data[(y * g.width + x) as usize] = 0;
            }
        }
    }

    /// A box outline of 3 px strokes over x0..x1, y0..y1.
    fn outline(g: &mut Gray, x0: u32, y0: u32, x1: u32, y1: u32) {
        ink(g, x0, y0, x1, y0 + 3);
        ink(g, x0, y1 - 3, x1, y1);
        ink(g, x0, y0, x0 + 3, y1);
        ink(g, x1 - 3, y0, x1, y1);
    }

    /// Letter-like blobs (20 x 40 px, 10 px apart) along a text line from x0 to x1 at y0.
    fn words(g: &mut Gray, x0: u32, x1: u32, y0: u32) {
        let mut x = x0;
        while x + 20 <= x1 {
            ink(g, x, y0, x + 20, y0 + 40);
            x += 30;
        }
    }

    fn rect(b: [u32; 4]) -> Rect {
        b.map(f64::from)
    }

    #[test]
    fn regions_from_orders_top_to_bottom() {
        let mut g = page(800, 800);
        // Bottom drawing, then a top right and a top left drawing with the same top.
        ink(&mut g, 100, 600, 200, 700);
        ink(&mut g, 500, 100, 600, 200);
        ink(&mut g, 100, 100, 200, 200);
        let drawn = [
            rect([90, 590, 210, 710]),
            rect([490, 90, 610, 210]),
            rect([90, 90, 210, 210]),
        ];
        let r = regions_from(&g, &drawn, &[]);
        let corners: Vec<(u16, u16)> = r.regions.iter().map(|r| (r.bbox.x0, r.bbox.y0)).collect();
        // Snapped to the ink with the 4 px margin: 96 px is 120 on the grid of 800.
        assert_eq!(corners, [(120, 120), (620, 120), (120, 745)]);
        assert!(r.regions.iter().all(|r| r.kind == RegionKind::Drawing));
        assert_eq!((r.width_px, r.height_px), (800, 800));
    }

    #[test]
    fn grid_mapping_of_1404_by_1872() {
        assert_eq!(to_grid(0, 1404), 0);
        assert_eq!(to_grid(1404, 1404), 1000);
        assert_eq!(to_grid(1872, 1872), 1000);
        // Clamped to the page.
        assert_eq!(to_grid(-5, 1404), 0);
        assert_eq!(to_grid(1500, 1404), 1000);
        // Round half up: 1 px of 2000 is 0.5, 3 px is 1.5.
        assert_eq!(to_grid(1, 2000), 1);
        assert_eq!(to_grid(3, 2000), 2);
        // 702 of 1404 is 500; 1 px of 1404 is 0.71, 1 px of 1872 is 0.53.
        assert_eq!(to_grid(702, 1404), 500);
        assert_eq!(to_grid(1, 1404), 1);
        assert_eq!(to_grid(1, 1872), 1);
        // On a page: drawings in the top left and the bottom right corners.
        let mut g = page(1404, 1872);
        ink(&mut g, 0, 0, 40, 40);
        ink(&mut g, 1364, 1832, 1404, 1872);
        let drawn = [rect([0, 0, 50, 50]), rect([1354, 1822, 1404, 1872])];
        let r = regions_from(&g, &drawn, &[]);
        assert_eq!(r.regions.len(), 2);
        assert_eq!((r.regions[0].bbox.x0, r.regions[0].bbox.y0), (0, 0));
        assert_eq!((r.regions[1].bbox.x1, r.regions[1].bbox.y1), (1000, 1000));
    }

    #[test]
    fn possible_drawing_lowers_certainty_and_adds_note() {
        // A large sparse box outline and no text line: the rules see a drawing that the layout
        // detector did not draw.
        let mut g = page(800, 800);
        outline(&mut g, 100, 100, 400, 400);
        let r = regions_from(&g, &[], &[]);
        assert_eq!(r.regions.len(), 1);
        let region = &r.regions[0];
        assert_eq!(region.kind, RegionKind::Text);
        assert_eq!(region.note.as_deref(), Some(POSSIBLE_DRAWING_NOTE));
        assert_eq!(region.certainty, 0.7);
        // The same ink drawn by the layout detector: a drawing at full certainty, no note.
        let r = regions_from(&g, &[rect([90, 90, 410, 410])], &[]);
        assert_eq!(r.regions.len(), 1);
        assert_eq!(r.regions[0].kind, RegionKind::Drawing);
        assert_eq!(r.regions[0].note, None);
        assert_eq!(r.regions[0].certainty, 1.0);
    }

    /// Review Focus 1: a landscape page (1872 x 1404) through the rules, the snapping and the
    /// grid, with each axis mapped by its own size.
    #[test]
    fn landscape_page_runs_every_stage() {
        let mut g = page(1872, 1404);
        outline(&mut g, 1000, 200, 1600, 700);
        words(&mut g, 204, 600, 904);
        words(&mut g, 204, 600, 994);
        let drawn = [rect([990, 190, 1610, 710])];
        let lines = [rect([200, 900, 600, 950]), rect([200, 990, 600, 1040])];
        let r = regions_from(&g, &drawn, &lines);
        assert_eq!((r.width_px, r.height_px), (1872, 1404));
        let kinds: Vec<RegionKind> = r.regions.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [RegionKind::Drawing, RegionKind::Text], "{r:?}");
        // The drawing snapped to 996..1604 x 196..704 px.
        assert_eq!(
            r.regions[0].bbox,
            BBox {
                x0: 532, // 996 / 1872
                y0: 140, // 196 / 1404
                x1: 857, // 1604 / 1872
                y1: 501, // 704 / 1404
            }
        );
        // The two lines are one text block, snapped to 200..588 x 900..1038 px (the last word
        // ends at 584).
        assert_eq!(
            r.regions[1].bbox,
            BBox {
                x0: 107, // 200 / 1872
                y0: 641, // 900 / 1404
                x1: 314, // 588 / 1872
                y1: 739, // 1038 / 1404
            }
        );
        assert!(r.regions.iter().all(|r| r.note.is_none()));
    }

    #[test]
    fn regions_fingerprint_names_the_detectors() {
        let f = regions_fingerprint();
        assert_eq!(f.layout_sha256, LAYOUT_V3.sha256);
        assert_eq!(f.text_det_sha256, TEXT_DET_V6.sha256);
        assert_eq!(f.candle, CANDLE_VERSION);
        assert_eq!((f.rules, f.ink), (RULES_VERSION, INK_VERSION));
        assert!(f.model_sha256.is_empty());
    }
}
