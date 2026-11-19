// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Box metrics measured against the ink on the page image.
//!
//! Ink is the detector ink of pagevision (`pagevision::ink`): the page grey on white, grey below
//! 110, ruled template lines removed. It is the ink of the region bake-off.

use pagevision::ink::{Mask, flatten, ink_mask};

use crate::{EvalError, RunBlock, RunRegion};

/// The ink of a PNG page.
fn page_ink(png: &[u8]) -> Result<Mask, EvalError> {
    Ok(ink_mask(&flatten(png)?))
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BoxScore {
    /// Share of ink pixels inside at least one block box. A page without ink scores 1.0.
    pub ink_coverage: f64,
    /// Boxes with no ink pixel inside.
    pub empty_boxes: u32,
    /// max(0, diagram blocks - truth diagrams).
    pub split_diagrams: u32,
}

/// Maps a box on the 0-1000 grid to a pixel rectangle `[x0, y0, x1, y1)`.
fn pixel_rect(bbox: [u32; 4], width: u32, height: u32) -> [usize; 4] {
    let scale = |v: u32, size: u32| (u64::from(v.min(1000)) * u64::from(size) / 1000) as usize;
    [
        scale(bbox[0], width),
        scale(bbox[1], height),
        scale(bbox[2], width),
        scale(bbox[3], height),
    ]
}

pub fn score_boxes(
    png: &[u8],
    blocks: &[RunBlock],
    truth_diagrams: u32,
) -> Result<BoxScore, EvalError> {
    let mask = page_ink(png)?;
    let (width, height) = (mask.width, mask.height);
    let ink = mask.bits;
    let w = width as usize;
    let mut covered = vec![false; ink.len()];
    let mut empty_boxes = 0u32;
    for block in blocks {
        let [x0, y0, x1, y1] = pixel_rect(block.bbox.unwrap_or([0, 0, 1000, 1000]), width, height);
        let mut has_ink = false;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * w + x;
                covered[i] = true;
                has_ink |= ink[i];
            }
        }
        if !has_ink {
            empty_boxes += 1;
        }
    }
    let total = ink.iter().filter(|&&b| b).count();
    let inside = ink.iter().zip(&covered).filter(|&(&i, &c)| i && c).count();
    let ink_coverage = if total == 0 {
        1.0
    } else {
        inside as f64 / total as f64
    };
    let diagrams = blocks.iter().filter(|b| b.kind == "diagram").count() as u32;
    Ok(BoxScore {
        ink_coverage,
        empty_boxes,
        split_diagrams: diagrams.saturating_sub(truth_diagrams),
    })
}

/// The region score of one page, as the bake-off's `score_page` (`bakeoff-scripts/common.py`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RegionScore {
    /// Truth diagrams.
    pub truth: u32,
    /// Truth diagrams with a drawn region at IoU 0.5 or more.
    pub hit: u32,
    /// Truth diagrams with two or more drawn regions mostly inside them.
    pub split: u32,
    /// Drawn regions that lie mostly outside every truth diagram.
    pub false_drawings: u32,
    /// The share of the text ink (outside the truth diagrams) that the text regions cover.
    pub text_cov: f64,
    /// Every truth diagram hit, nothing split, no false drawing and text_cov at least 0.95.
    pub ok: bool,
}

// The bake-off page rule (gate 1). Pixel margins and shares as in `score_page`.
/// A truth diagram is hit by a drawn region at this IoU or more.
const HIT_IOU: f64 = 0.5;
/// The margin of a truth diagram for the split and false checks.
const TRUTH_GROW: i64 = 40;
/// A drawn region with at least this share inside the grown truth counts towards a split.
const SPLIT_SHARE: f64 = 0.5;
/// A drawn region with less than this share inside every grown truth is false.
const FALSE_SHARE: f64 = 0.3;
/// Text ink is the ink outside the truth diagrams grown by this.
const TEXT_TRUTH_GROW: i64 = 20;
/// A text region covers the ink in its box grown by this.
const TEXT_REGION_GROW: i64 = 10;
/// A page with this much text ink or less counts as covered.
const MIN_TEXT_PIXELS: usize = 300;
/// The text coverage a page needs.
const MIN_TEXT_COV: f64 = 0.95;

type Px = [i64; 4];

fn inter(a: &Px, b: &Px) -> i64 {
    (a[2].min(b[2]) - a[0].max(b[0])).max(0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0)
}

fn area(a: &Px) -> i64 {
    ((a[2] - a[0]) * (a[3] - a[1])).max(1)
}

fn grow(a: &Px, d: i64) -> Px {
    [a[0] - d, a[1] - d, a[2] + d, a[3] + d]
}

fn iou(a: &Px, b: &Px) -> f64 {
    let i = inter(a, b);
    i as f64 / (area(a) + area(b) - i) as f64
}

/// The share of `b` inside `t` grown by the truth margin.
fn share(t: &Px, b: &Px) -> f64 {
    inter(&grow(t, TRUTH_GROW), b) as f64 / area(b) as f64
}

/// Sets the pixels of `b` (clipped to the mask) in `bits`.
fn fill(bits: &mut [bool], width: u32, height: u32, b: &Px) {
    let cx = |v: i64| v.clamp(0, i64::from(width)) as usize;
    let cy = |v: i64| v.clamp(0, i64::from(height)) as usize;
    let w = width as usize;
    for y in cy(b[1])..cy(b[3]) {
        bits[y * w + cx(b[0])..y * w + cx(b[2]).max(cx(b[0]))].fill(true);
    }
}

/// Scores proposed regions against the diagram truth of a page: the bake-off's `score_page`.
/// The truth is in page pixels; the regions are mapped from the grid to pixels. A region of kind
/// "drawing" is a drawn region, any other kind is text.
pub fn score_regions(
    gray_png: &[u8],
    truth_px: &[[i64; 4]],
    regions: &[RunRegion],
) -> Result<RegionScore, EvalError> {
    let ink = page_ink(gray_png)?;
    let (width, height) = (ink.width, ink.height);
    let to_px = |r: &RunRegion| -> Px { pixel_rect(r.bbox, width, height).map(|v| v as i64) };
    let drawn: Vec<Px> = regions
        .iter()
        .filter(|r| r.kind == "drawing")
        .map(to_px)
        .collect();
    let text: Vec<Px> = regions
        .iter()
        .filter(|r| r.kind != "drawing")
        .map(to_px)
        .collect();
    let hit = truth_px
        .iter()
        .filter(|t| drawn.iter().any(|b| iou(t, b) >= HIT_IOU))
        .count();
    let split = truth_px
        .iter()
        .filter(|t| drawn.iter().filter(|b| share(t, b) >= SPLIT_SHARE).count() >= 2)
        .count();
    let false_drawings = drawn
        .iter()
        .filter(|b| truth_px.iter().map(|t| share(t, b)).fold(0.0, f64::max) < FALSE_SHARE)
        .count();
    let n = ink.bits.len();
    let mut in_truth = vec![false; n];
    for t in truth_px {
        fill(&mut in_truth, width, height, &grow(t, TEXT_TRUTH_GROW));
    }
    let mut covered = vec![false; n];
    for b in &text {
        fill(&mut covered, width, height, &grow(b, TEXT_REGION_GROW));
    }
    let (mut text_ink, mut text_covered) = (0usize, 0usize);
    for i in 0..n {
        if ink.bits[i] && !in_truth[i] {
            text_ink += 1;
            if covered[i] {
                text_covered += 1;
            }
        }
    }
    let text_cov = if text_ink > MIN_TEXT_PIXELS {
        text_covered as f64 / text_ink as f64
    } else {
        1.0
    };
    let truth = truth_px.len();
    Ok(RegionScore {
        truth: truth as u32,
        hit: hit as u32,
        split: split as u32,
        false_drawings: false_drawings as u32,
        text_cov,
        ok: hit == truth && split == 0 && false_drawings == 0 && text_cov >= MIN_TEXT_COV,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn png_sized(color: image::ExtendedColorType, data: &[u8], size: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(data, size, size, color)
            .unwrap();
        out
    }

    fn png(color: image::ExtendedColorType, data: &[u8]) -> Vec<u8> {
        png_sized(color, data, 100)
    }

    /// A white square page of `size` pixels with squares of grey `value` given as (x0, y0, x1,
    /// y1) in pixels.
    fn grey_page(size: usize, value: u8, squares: &[[usize; 4]]) -> Vec<u8> {
        let mut data = vec![255u8; size * size * 3];
        for s in squares {
            for y in s[1]..s[3] {
                for x in s[0]..s[2] {
                    let i = (y * size + x) * 3;
                    data[i..i + 3].fill(value);
                }
            }
        }
        png_sized(image::ExtendedColorType::Rgb8, &data, size as u32)
    }

    /// White 100x100 with black squares given as (x0, y0, x1, y1) in pixels.
    fn page(squares: &[[usize; 4]]) -> Vec<u8> {
        grey_page(100, 0, squares)
    }

    fn block(kind: &str, bbox: Option<[u32; 4]>) -> RunBlock {
        RunBlock {
            kind: kind.into(),
            label: None,
            bbox,
            markdown: String::new(),
        }
    }

    #[test]
    fn full_coverage() {
        let p = page(&[[10, 10, 30, 30]]);
        let s = score_boxes(&p, &[block("text", Some([0, 0, 500, 500]))], 0).unwrap();
        assert_eq!(s.ink_coverage, 1.0);
        assert_eq!(s.empty_boxes, 0);
    }

    #[test]
    fn half_coverage() {
        let p = page(&[[10, 10, 30, 30], [60, 60, 80, 80]]);
        let s = score_boxes(&p, &[block("text", Some([0, 0, 500, 500]))], 0).unwrap();
        assert_eq!(s.ink_coverage, 0.5);
    }

    #[test]
    fn empty_box_counted() {
        let p = page(&[[10, 10, 30, 30]]);
        let blocks = [
            block("text", Some([0, 0, 500, 500])),
            block("text", Some([600, 600, 900, 900])),
        ];
        assert_eq!(score_boxes(&p, &blocks, 0).unwrap().empty_boxes, 1);
    }

    #[test]
    fn diagram_split_counted() {
        let p = page(&[[10, 10, 30, 30]]);
        let blocks = [
            block("diagram", None),
            block("diagram", None),
            block("diagram", None),
            block("text", None),
        ];
        assert_eq!(score_boxes(&p, &blocks, 1).unwrap().split_diagrams, 2);
        assert_eq!(score_boxes(&p, &blocks, 5).unwrap().split_diagrams, 0);
    }

    #[test]
    fn boxless_block_is_the_whole_page() {
        let p = page(&[[10, 10, 30, 30], [60, 60, 80, 80]]);
        let s = score_boxes(&p, &[block("text", None)], 0).unwrap();
        assert_eq!(s.ink_coverage, 1.0);
        assert_eq!(s.empty_boxes, 0);
    }

    #[test]
    fn transparent_png_has_no_ink() {
        let data = [0u8; 100 * 100 * 4];
        let p = png(image::ExtendedColorType::Rgba8, &data);
        let s = score_boxes(&p, &[block("text", None)], 0).unwrap();
        assert_eq!(s.ink_coverage, 1.0);
        assert_eq!(s.empty_boxes, 1);
    }

    /// The ink changed from luma below 128 to the bake-off ink (grey below 110, ruled lines
    /// removed): a grey of 120 and a line across the page are no ink any more.
    #[test]
    fn ink_is_the_bakeoff_ink() {
        let blocks = [block("text", Some([0, 0, 500, 500]))];
        // Grey 109 inside the box, grey 120 outside it.
        let p = grey_page(100, 109, &[[10, 10, 30, 30]]);
        assert_eq!(score_boxes(&p, &blocks, 0).unwrap().ink_coverage, 1.0);
        let mut data = vec![255u8; 100 * 100 * 3];
        for y in 60..80 {
            for x in 60..80 {
                let i = (y * 100 + x) * 3;
                data[i..i + 3].fill(120);
            }
        }
        for y in 10..30 {
            for x in 10..30 {
                let i = (y * 100 + x) * 3;
                data[i..i + 3].fill(0);
            }
        }
        let p = png(image::ExtendedColorType::Rgb8, &data);
        assert_eq!(score_boxes(&p, &blocks, 0).unwrap().ink_coverage, 1.0);
        // A ruled line (half the page wide or more) outside the box is no ink either.
        let p = page(&[[10, 10, 30, 30], [0, 90, 60, 91]]);
        assert_eq!(score_boxes(&p, &blocks, 0).unwrap().ink_coverage, 1.0);
        // A short stroke outside the box is ink.
        let p = page(&[[10, 10, 30, 30], [0, 90, 40, 91]]);
        assert!(score_boxes(&p, &blocks, 0).unwrap().ink_coverage < 1.0);
    }

    fn region(kind: &str, bbox: [u32; 4]) -> RunRegion {
        RunRegion {
            kind: kind.into(),
            bbox,
        }
    }

    /// On a 1000 px page the grid is the pixel grid.
    #[test]
    fn score_regions_hit_split_false() {
        let p = grey_page(1000, 255, &[]);
        let truth = [[100, 100, 400, 400], [500, 500, 900, 900]];
        let regions = [
            // Hits the first diagram.
            region("drawing", [100, 100, 400, 400]),
            // Two halves of the second (IoU 0.475 each): split, no hit.
            region("drawing", [500, 500, 690, 900]),
            region("drawing", [710, 500, 900, 900]),
            // Far from both: false.
            region("drawing", [0, 950, 100, 1000]),
            // Text regions never count as drawings.
            region("text", [0, 0, 50, 50]),
        ];
        let s = score_regions(&p, &truth, &regions).unwrap();
        assert_eq!(
            (s.truth, s.hit, s.split, s.false_drawings),
            (2, 1, 1, 1),
            "{s:?}"
        );
        assert_eq!(s.text_cov, 1.0);
        assert!(!s.ok);
        // Exactly the truth: ok.
        let exact = [
            region("drawing", [100, 100, 400, 400]),
            region("drawing", [500, 500, 900, 900]),
        ];
        let s = score_regions(&p, &truth, &exact).unwrap();
        assert_eq!((s.hit, s.split, s.false_drawings), (2, 0, 0));
        assert!(s.ok);
        // IoU 0.5 is a hit, just below is not.
        let half = [region("drawing", [100, 100, 400, 250])];
        assert_eq!(score_regions(&p, &truth[..1], &half).unwrap().hit, 1);
        let less = [region("drawing", [100, 100, 400, 249])];
        assert_eq!(score_regions(&p, &truth[..1], &less).unwrap().hit, 0);
        // A share of 0.3 inside the grown truth is not false, just below is.
        let edge = [region("drawing", [410, 100, 510, 200])];
        assert_eq!(
            score_regions(&p, &truth[..1], &edge)
                .unwrap()
                .false_drawings,
            0
        );
        let out = [region("drawing", [411, 100, 511, 200])];
        assert_eq!(
            score_regions(&p, &truth[..1], &out).unwrap().false_drawings,
            1
        );
        // No truth: every drawing is false.
        let s = score_regions(&p, &[], &exact).unwrap();
        assert_eq!((s.truth, s.false_drawings), (0, 2));
    }

    #[test]
    fn text_cov_counts_only_ink_outside_truth() {
        // Diagram ink at 100..400, its margin ink at 410..415 (inside grow 20), text ink at
        // 600..640 x 100..120 (800 px).
        let p = grey_page(
            1000,
            0,
            &[
                [150, 150, 350, 350],
                [410, 150, 415, 350],
                [600, 100, 640, 120],
            ],
        );
        let truth = [[100, 100, 400, 400]];
        let diagram = region("drawing", [100, 100, 400, 400]);
        // The text region covers the text ink: 1.0, though no text region covers the diagram.
        let s = score_regions(
            &p,
            &truth,
            &[diagram.clone(), region("text", [600, 100, 640, 120])],
        )
        .unwrap();
        assert_eq!(s.text_cov, 1.0);
        assert!(s.ok);
        // The grow of 10 counts: a box 10 px short on each side still covers it all.
        let s = score_regions(
            &p,
            &truth,
            &[diagram.clone(), region("text", [610, 110, 630, 110])],
        )
        .unwrap();
        assert_eq!(s.text_cov, 1.0);
        // Half of the text ink covered.
        let s = score_regions(
            &p,
            &truth,
            &[diagram.clone(), region("text", [610, 90, 610, 130])],
        )
        .unwrap();
        assert_eq!(s.text_cov, 0.5);
        assert!(!s.ok);
        // No text region: 0.
        let s = score_regions(&p, &truth, &[diagram]).unwrap();
        assert_eq!(s.text_cov, 0.0);
    }

    #[test]
    fn few_text_pixels_count_as_covered() {
        let truth: [[i64; 4]; 0] = [];
        // 300 ink pixels and no text region: covered.
        let p = grey_page(1000, 0, &[[100, 100, 130, 110]]);
        let s = score_regions(&p, &truth, &[]).unwrap();
        assert_eq!(s.text_cov, 1.0);
        assert!(s.ok);
        // 301: not covered.
        let p = grey_page(1000, 0, &[[100, 100, 130, 110], [200, 200, 201, 201]]);
        let s = score_regions(&p, &truth, &[]).unwrap();
        assert_eq!(s.text_cov, 0.0);
        assert!(!s.ok);
    }

    #[test]
    fn bad_png_is_an_error_for_regions() {
        assert!(matches!(
            score_regions(b"nope", &[], &[]),
            Err(EvalError::Page(_))
        ));
    }

    #[test]
    fn bad_png_is_an_error() {
        assert!(matches!(
            score_boxes(b"nope", &[], 0),
            Err(EvalError::Page(_))
        ));
    }
}
