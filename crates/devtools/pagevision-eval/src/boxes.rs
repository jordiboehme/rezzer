// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Box metrics measured against the ink on the page image.

use crate::{EvalError, RunBlock};

/// Ink pixels: luminance below 128 after compositing onto white.
pub fn ink_mask(rgb: &[u8], width: u32, height: u32) -> Vec<bool> {
    let pixels = width as usize * height as usize;
    rgb.as_chunks::<3>()
        .0
        .iter()
        .take(pixels)
        .map(|p| {
            // Rec. 601 luma in integers: (299 r + 587 g + 114 b) / 1000.
            let luma = 299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2]);
            luma < 128_000
        })
        .collect()
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
    let (rgb, width, height) = pagevision::prepare::decode_on_white(png)?;
    let ink = ink_mask(&rgb, width, height);
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn png(color: image::ExtendedColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(data, 100, 100, color)
            .unwrap();
        out
    }

    /// White 100x100 with black squares given as (x0, y0, x1, y1) in pixels.
    fn page(squares: &[[usize; 4]]) -> Vec<u8> {
        let mut data = vec![255u8; 100 * 100 * 3];
        for s in squares {
            for y in s[1]..s[3] {
                for x in s[0]..s[2] {
                    let i = (y * 100 + x) * 3;
                    data[i..i + 3].fill(0);
                }
            }
        }
        png(image::ExtendedColorType::Rgb8, &data)
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
        assert!(ink_mask(&[255; 12], 2, 2).iter().all(|&b| !b));
    }

    #[test]
    fn bad_png_is_an_error() {
        assert!(matches!(
            score_boxes(b"nope", &[], 0),
            Err(EvalError::Page(_))
        ));
    }
}
