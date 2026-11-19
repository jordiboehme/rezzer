// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Page and region image preprocessing and the image token rule.

use image::RgbImage;

use crate::types::BBox;
use crate::{error::PageError, model::ImageRule};

/// The margin around a region crop, in page pixels: room for strokes that snapping cut close.
pub const CROP_MARGIN: u32 = 16;

/// A page image ready for the model: packed RGB8 at the model's grid size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
}

/// The size the model sees: each side at the nearest multiple, a tie to the even multiple.
pub fn target_size(width: u32, height: u32, rule: &ImageRule) -> (u32, u32) {
    (
        round_side(width, rule.multiple),
        round_side(height, rule.multiple),
    )
}

fn round_side(side: u32, multiple: u32) -> u32 {
    let q = side / multiple;
    let r = side % multiple;
    let steps = match (r * 2).cmp(&multiple) {
        std::cmp::Ordering::Less => q,
        std::cmp::Ordering::Greater => q + 1,
        std::cmp::Ordering::Equal => q + (q % 2),
    };
    (steps * multiple).max(multiple)
}

/// Decode a PNG and composite alpha onto white, at the source size.
/// Returns packed RGB8 with the width and height.
pub fn decode_on_white(png: &[u8]) -> Result<(Vec<u8>, u32, u32), PageError> {
    let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| PageError::Image(e.to_string()))?;
    let (width, height) = (decoded.width(), decoded.height());
    let rgba = decoded.to_rgba8();
    let mut rgb = image::RgbImage::new(width, height);
    for (src, dst) in rgba.pixels().zip(rgb.pixels_mut()) {
        let a = u32::from(src[3]);
        for c in 0..3 {
            let v = (u32::from(src[c]) * a + 255 * (255 - a) + 127) / 255;
            dst[c] = v as u8;
        }
    }
    Ok((rgb.into_raw(), width, height))
}

/// Decode a PNG, composite alpha onto white and resize to the grid size.
pub fn prepare(png: &[u8], rule: &ImageRule) -> Result<Prepared, PageError> {
    let (raw, source_width, source_height) = decode_on_white(png)?;
    let rgb = image::RgbImage::from_raw(source_width, source_height, raw)
        .expect("decode_on_white returns width * height * 3 bytes");
    let (width, height) = target_size(source_width, source_height, rule);
    let rgb = if (width, height) == (source_width, source_height) {
        rgb
    } else {
        image::imageops::resize(&rgb, width, height, image::imageops::FilterType::CatmullRom)
    };
    Ok(Prepared {
        rgb: rgb.into_raw(),
        width,
        height,
        source_width,
        source_height,
    })
}

/// The image token count llama.cpp gives for an image of this exact size: one token per
/// `rule.multiple` x `rule.multiple` tile, when both sides are on the grid and the tile count is
/// at least `rule.min_tokens`. Otherwise None, since llama.cpp would resize the image itself.
pub fn expected_image_tokens(width: u32, height: u32, rule: &ImageRule) -> Option<u32> {
    let m = rule.multiple;
    if m == 0 || width == 0 || height == 0 || !width.is_multiple_of(m) || !height.is_multiple_of(m)
    {
        return None;
    }
    let tiles = (width / m).checked_mul(height / m)?;
    let min = u32::try_from(rule.min_tokens).unwrap_or(0);
    (tiles >= min).then_some(tiles)
}

/// The tile count of an image of this size once padded to multiples of `multiple`.
fn padded_tiles(width: u32, height: u32, multiple: u32) -> u64 {
    u64::from(width.div_ceil(multiple)) * u64::from(height.div_ceil(multiple))
}

/// The pixel box of a 0..=1000 grid box on a `width` x `height` page: the start rounded down and
/// the end rounded up, so the crop holds the whole region.
pub fn grid_to_px(bbox: BBox, width: u32, height: u32) -> [u32; 4] {
    let down = |v: u16, size: u32| ((u64::from(v) * u64::from(size)) / 1000) as u32;
    let up = |v: u16, size: u32| (u64::from(v) * u64::from(size)).div_ceil(1000) as u32;
    [
        down(bbox.x0, width).min(width),
        down(bbox.y0, height).min(height),
        up(bbox.x1, width).min(width),
        up(bbox.y1, height).min(height),
    ]
}

/// The scaled size of a `width` x `height` crop: unchanged when its padded size already has
/// `rule.min_tokens` tiles, otherwise the long side grows one pixel at a time (the short side
/// follows, rounded half up) until the padded size has them.
pub(crate) fn scaled_size(width: u32, height: u32, rule: &ImageRule) -> (u32, u32) {
    let (m, min) = (
        rule.multiple.max(1),
        u64::try_from(rule.min_tokens).unwrap_or(0),
    );
    let (width, height) = (width.max(1), height.max(1));
    if padded_tiles(width, height, m) >= min {
        return (width, height);
    }
    let (long, short) = (width.max(height), width.min(height));
    let mut l = long;
    loop {
        l += 1;
        // short * l / long, rounded half up.
        let s = ((u64::from(short) * u64::from(l) * 2 + u64::from(long)) / (2 * u64::from(long)))
            as u32;
        let (w, h) = if width >= height { (l, s) } else { (s, l) };
        if padded_tiles(w, h, m) >= min {
            return (w, h);
        }
    }
}

/// A region crop ready for the model, cut from the colour page (`decode_on_white`):
/// 1. the box grown by [`CROP_MARGIN`] px on each side, clipped to the page;
/// 2. scaled up with CatmullRom by the smallest factor (in 1 px steps of the long side) whose
///    padded size reaches `rule.min_tokens` tiles, when the crop alone does not;
/// 3. padded with white on the right and bottom to multiples of `rule.multiple`.
///
/// `source_width` and `source_height` hold the crop size before scaling.
pub fn prepare_crop(page_rgb: &RgbImage, bbox_px: [u32; 4], rule: &ImageRule) -> Prepared {
    let (pw, ph) = page_rgb.dimensions();
    // Grown and clipped; a box outside the page still gives a crop of at least one pixel.
    let x0 = bbox_px[0]
        .min(pw.saturating_sub(1))
        .saturating_sub(CROP_MARGIN);
    let y0 = bbox_px[1]
        .min(ph.saturating_sub(1))
        .saturating_sub(CROP_MARGIN);
    let x1 = bbox_px[2].saturating_add(CROP_MARGIN).min(pw).max(x0 + 1);
    let y1 = bbox_px[3].saturating_add(CROP_MARGIN).min(ph).max(y0 + 1);
    let (cw, ch) = (x1 - x0, y1 - y0);
    let crop = image::imageops::crop_imm(page_rgb, x0, y0, cw, ch).to_image();
    let (sw, sh) = scaled_size(cw, ch, rule);
    let crop = if (sw, sh) == (cw, ch) {
        crop
    } else {
        image::imageops::resize(&crop, sw, sh, image::imageops::FilterType::CatmullRom)
    };
    let m = rule.multiple.max(1);
    let (width, height) = (sw.div_ceil(m) * m, sh.div_ceil(m) * m);
    let mut out = RgbImage::from_pixel(width, height, image::Rgb([255, 255, 255]));
    image::imageops::replace(&mut out, &crop, 0, 0);
    Prepared {
        rgb: out.into_raw(),
        width,
        height,
        source_width: cw,
        source_height: ch,
    }
}

/// A whole colour page ready for the model: resized to the nearest multiples of 32 (1408 x 1856
/// for a Supernote page), as the page reads of the content test did. A page too small for
/// `rule.min_tokens` tiles at that size goes through [`prepare_crop`] of the whole page instead
/// (scaled up and padded with white).
pub fn prepare_page(page_rgb: &RgbImage, rule: &ImageRule) -> Prepared {
    let (sw, sh) = page_rgb.dimensions();
    let (width, height) = target_size(sw, sh, rule);
    if expected_image_tokens(width, height, rule).is_none() {
        return prepare_crop(page_rgb, [0, 0, sw, sh], rule);
    }
    let rgb = if (width, height) == (sw, sh) {
        page_rgb.clone()
    } else {
        image::imageops::resize(
            page_rgb,
            width,
            height,
            image::imageops::FilterType::CatmullRom,
        )
    };
    Prepared {
        rgb: rgb.into_raw(),
        width,
        height,
        source_width: sw,
        source_height: sh,
    }
}

/// A copy of the page with each pixel box (grown by [`CROP_MARGIN`] and clipped) filled white.
pub fn blank_boxes(page_rgb: &RgbImage, boxes_px: &[[u32; 4]]) -> RgbImage {
    let mut out = page_rgb.clone();
    let (w, h) = out.dimensions();
    for b in boxes_px {
        let (x0, y0) = (
            b[0].saturating_sub(CROP_MARGIN),
            b[1].saturating_sub(CROP_MARGIN),
        );
        let x1 = b[2].saturating_add(CROP_MARGIN).min(w);
        let y1 = b[3].saturating_add(CROP_MARGIN).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                out.put_pixel(x, y, image::Rgb([255, 255, 255]));
            }
        }
    }
    out
}

/// A white page with only the pixel boxes (grown by [`CROP_MARGIN`] and clipped) copied from
/// the page.
pub fn keep_boxes(page_rgb: &RgbImage, boxes_px: &[[u32; 4]]) -> RgbImage {
    let (w, h) = page_rgb.dimensions();
    let mut out = RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
    for b in boxes_px {
        let (x0, y0) = (
            b[0].saturating_sub(CROP_MARGIN),
            b[1].saturating_sub(CROP_MARGIN),
        );
        let x1 = b[2].saturating_add(CROP_MARGIN).min(w);
        let y1 = b[3].saturating_add(CROP_MARGIN).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                out.put_pixel(x, y, *page_rgb.get_pixel(x, y));
            }
        }
    }
    out
}

/// A pixel darker than this in any channel is content for [`content_box`]. Looser than the ink
/// mask (grey below 110), so light printed text counts too; paper white and the light ruling of
/// a Supernote page stay above it.
pub const CONTENT_LEVEL: u8 = 160;

/// Fewer content pixels than this are noise for [`content_box`] (about a 7 x 7 px speck).
pub const CONTENT_MIN_PIXELS: usize = 50;

/// The pixel box (half-open) of all content on a page: pixels darker than [`CONTENT_LEVEL`] in
/// any channel, or None when there are fewer than [`CONTENT_MIN_PIXELS`].
pub fn content_box(page_rgb: &RgbImage) -> Option<[u32; 4]> {
    let (mut count, mut b) = (0usize, [u32::MAX, u32::MAX, 0, 0]);
    for (x, y, p) in page_rgb.enumerate_pixels() {
        if p.0.iter().any(|&c| c < CONTENT_LEVEL) {
            count += 1;
            b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
        }
    }
    (count >= CONTENT_MIN_PIXELS).then_some(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::QWEN35_4B;
    use image::ImageEncoder;

    fn png(width: u32, height: u32, color: image::ExtendedColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(data, width, height, color)
            .unwrap();
        out
    }

    #[test]
    fn supernote_page_size() {
        assert_eq!(target_size(1404, 1872, &QWEN35_4B.image), (1408, 1856));
    }

    #[test]
    fn ties_go_to_even() {
        let rule = &QWEN35_4B.image;
        assert_eq!(target_size(528, 528, rule), (512, 512));
        assert_eq!(target_size(560, 560, rule), (576, 576));
    }

    #[test]
    fn transparent_becomes_white() {
        let data = vec![0u8; 64 * 64 * 4];
        let p = prepare(
            &png(64, 64, image::ExtendedColorType::Rgba8, &data),
            &QWEN35_4B.image,
        )
        .unwrap();
        assert_eq!((p.width, p.height), (64, 64));
        assert!(p.rgb.iter().all(|&b| b == 255));
    }

    #[test]
    fn fitting_size_is_not_resampled() {
        let mut state = 12345u32;
        let data: Vec<u8> = (0..64 * 96 * 3)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();
        let p = prepare(
            &png(64, 96, image::ExtendedColorType::Rgb8, &data),
            &QWEN35_4B.image,
        )
        .unwrap();
        assert_eq!(p.rgb, data);
    }

    #[test]
    fn test2_fixture() {
        let data = vec![200u8; 1404 * 1872 * 3];
        let p = prepare(
            &png(1404, 1872, image::ExtendedColorType::Rgb8, &data),
            &QWEN35_4B.image,
        )
        .unwrap();
        assert_eq!((p.width, p.height), (1408, 1856));
        assert_eq!((p.source_width, p.source_height), (1404, 1872));
        assert_eq!(p.rgb.len(), 1408 * 1856 * 3);
    }

    #[test]
    fn token_rule_keeps_the_supernote_value() {
        assert_eq!(
            expected_image_tokens(1408, 1856, &QWEN35_4B.image),
            Some(2552)
        );
    }

    #[test]
    fn token_rule_rejects_sizes_off_the_grid() {
        let rule = &QWEN35_4B.image;
        assert_eq!(expected_image_tokens(1404, 1872, rule), None);
        assert_eq!(expected_image_tokens(1408, 1860, rule), None);
        assert_eq!(expected_image_tokens(0, 0, rule), None);
    }

    #[test]
    fn token_rule_rejects_too_few_tiles() {
        let rule = &QWEN35_4B.image;
        // 32 x 32 tiles of 32 px.
        assert_eq!(expected_image_tokens(1024, 1024, rule), Some(1024));
        // 31 x 33 tiles.
        assert_eq!(expected_image_tokens(992, 1056, rule), None);
    }

    /// A page of `width` x `height` with a black rectangle over `ink` (x0, y0, x1, y1).
    fn page_with(width: u32, height: u32, ink: [u32; 4]) -> RgbImage {
        let mut img = RgbImage::from_pixel(width, height, image::Rgb([255, 255, 255]));
        for y in ink[1]..ink[3] {
            for x in ink[0]..ink[2] {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        img
    }

    fn assert_on_grid(p: &Prepared) {
        let rule = &QWEN35_4B.image;
        assert_eq!(p.width % 32, 0, "{}", p.width);
        assert_eq!(p.height % 32, 0, "{}", p.height);
        assert_eq!(p.rgb.len(), (p.width * p.height * 3) as usize);
        let tokens = expected_image_tokens(p.width, p.height, rule);
        assert!(
            tokens.is_some_and(|t| t >= 1024),
            "{}x{}",
            p.width,
            p.height
        );
    }

    #[test]
    fn crop_reaches_min_tokens() {
        let page = page_with(1404, 1872, [400, 400, 600, 480]);
        let p = prepare_crop(&page, [400, 400, 600, 480], &QWEN35_4B.image);
        // Grown by 16 px on each side.
        assert_eq!((p.source_width, p.source_height), (232, 112));
        assert_on_grid(&p);
        // The smallest factor in 1 px steps of the long side: one step less misses 1024 tiles.
        let tiles = |w: u32, h: u32| w.div_ceil(32) * h.div_ceil(32);
        let (sw, sh) = scaled_size(232, 112, &QWEN35_4B.image);
        assert_eq!(
            (p.width, p.height),
            (sw.div_ceil(32) * 32, sh.div_ceil(32) * 32)
        );
        assert!(tiles(sw, sh) >= 1024);
        let short = (112 * (sw - 1) + 116) / 232;
        assert!(tiles(sw - 1, short) < 1024, "{sw}x{sh}");
        // The aspect is kept.
        assert_eq!(sh, (112 * sw + 116) / 232);
    }

    #[test]
    fn crop_of_a_large_box_is_not_scaled() {
        let page = page_with(1404, 1872, [100, 100, 1300, 1000]);
        let p = prepare_crop(&page, [100, 100, 1300, 1000], &QWEN35_4B.image);
        assert_eq!((p.source_width, p.source_height), (1232, 932));
        assert_eq!((p.width, p.height), (1248, 960));
        assert_on_grid(&p);
        // Pixels are copied, not resampled: the crop starts 16 px before the ink.
        let px = |x: u32, y: u32| p.rgb[((y * p.width + x) * 3) as usize];
        assert_eq!(px(15, 15), 255);
        assert_eq!(px(16, 16), 0);
        assert_eq!(px(1215, 915), 0);
        assert_eq!(px(1216, 916), 255);
        // The padding is white.
        assert_eq!(px(1240, 950), 255);
    }

    #[test]
    fn crop_near_the_edge_is_clipped() {
        let page = page_with(1404, 1872, [0, 1800, 1404, 1872]);
        let p = prepare_crop(&page, [0, 1790, 1404, 1872], &QWEN35_4B.image);
        // No room for the margin left, right and below.
        assert_eq!((p.source_width, p.source_height), (1404, 98));
        assert_on_grid(&p);
        // The whole page: 1404 x 1872 padded to 1408 x 1888.
        let full = prepare_crop(&page, [0, 0, 1404, 1872], &QWEN35_4B.image);
        assert_eq!((full.width, full.height), (1408, 1888));
        assert_eq!(
            expected_image_tokens(full.width, full.height, &QWEN35_4B.image),
            Some(2596)
        );
    }

    #[test]
    fn grid_box_maps_to_covering_pixels() {
        let b = BBox {
            x0: 0,
            y0: 1,
            x1: 500,
            y1: 1000,
        };
        // 1 of 1000 on 1872 px is 1.872: down to 1. 500 of 1404 is 702 exactly.
        assert_eq!(grid_to_px(b, 1404, 1872), [0, 1, 702, 1872]);
        let b = BBox {
            x0: 333,
            y0: 333,
            x1: 334,
            y1: 334,
        };
        // 333 * 1.404 = 467.5 down to 467; 334 * 1.404 = 468.9 up to 469.
        assert_eq!(grid_to_px(b, 1404, 1404), [467, 467, 469, 469]);
    }

    #[test]
    fn not_a_png_is_an_image_error() {
        assert!(matches!(
            prepare(b"nope", &QWEN35_4B.image),
            Err(PageError::Image(_))
        ));
    }

    #[test]
    fn page_is_resized_to_the_grid() {
        let page = page_with(1404, 1872, [100, 100, 200, 200]);
        let p = prepare_page(&page, &QWEN35_4B.image);
        assert_eq!((p.width, p.height), (1408, 1856));
        assert_eq!((p.source_width, p.source_height), (1404, 1872));
        assert_eq!(
            expected_image_tokens(p.width, p.height, &QWEN35_4B.image),
            Some(2552)
        );
        // A page already on the grid is copied.
        let page = page_with(1408, 1856, [100, 100, 200, 200]);
        assert_eq!(prepare_page(&page, &QWEN35_4B.image).rgb, page.into_raw());
    }

    #[test]
    fn small_page_reaches_min_tokens() {
        // 1110 x 705 is 35 x 22 tiles on the grid: too few.
        let page = page_with(1110, 705, [10, 10, 20, 20]);
        let p = prepare_page(&page, &QWEN35_4B.image);
        assert!(expected_image_tokens(p.width, p.height, &QWEN35_4B.image).is_some());
        assert_eq!((p.source_width, p.source_height), (1110, 705));
    }

    #[test]
    fn blanked_boxes_are_white_with_margin() {
        let page = page_with(400, 400, [0, 0, 400, 400]);
        let out = blank_boxes(&page, &[[100, 100, 200, 150], [390, 390, 400, 400]]);
        let px = |x: u32, y: u32| out.get_pixel(x, y)[0];
        // The box grown by 16 px is white; outside it the ink stays.
        assert_eq!(px(84, 84), 255);
        assert_eq!(px(215, 165), 255);
        assert_eq!(px(83, 100), 0);
        assert_eq!(px(216, 100), 0);
        assert_eq!(px(150, 166), 0);
        // Clipped at the page edge.
        assert_eq!(px(399, 399), 255);
        assert_eq!(px(373, 373), 0);
    }

    #[test]
    fn kept_boxes_are_the_only_ink() {
        let page = page_with(400, 400, [0, 0, 400, 400]);
        let out = keep_boxes(&page, &[[100, 100, 200, 150]]);
        let px = |x: u32, y: u32| out.get_pixel(x, y)[0];
        // The box grown by 16 px keeps its ink; everything else is white.
        assert_eq!(px(84, 84), 0);
        assert_eq!(px(215, 165), 0);
        assert_eq!(px(83, 100), 255);
        assert_eq!(px(216, 100), 255);
        assert_eq!(px(150, 166), 255);
        assert_eq!(px(0, 0), 255);
        // No box: a white page.
        assert!(keep_boxes(&page, &[]).pixels().all(|p| p[0] == 255));
    }

    #[test]
    fn content_box_finds_light_text_and_ignores_specks() {
        // A white page has no content.
        let white = RgbImage::from_pixel(200, 100, image::Rgb([255, 255, 255]));
        assert_eq!(content_box(&white), None);
        // A light grey title (150, lighter than ink) is content.
        let mut page = white.clone();
        for y in 10..20 {
            for x in 30..80 {
                page.put_pixel(x, y, image::Rgb([150, 150, 150]));
            }
        }
        assert_eq!(content_box(&page), Some([30, 10, 80, 20]));
        // Grey at the level is not content.
        let mut faint = white.clone();
        for y in 10..20 {
            for x in 30..80 {
                faint.put_pixel(
                    x,
                    y,
                    image::Rgb([CONTENT_LEVEL, CONTENT_LEVEL, CONTENT_LEVEL]),
                );
            }
        }
        assert_eq!(content_box(&faint), None);
        // 49 dark pixels are a speck; 50 are content.
        let mut speck = white.clone();
        for i in 0..49 {
            speck.put_pixel(100 + i % 7, 50 + i / 7, image::Rgb([0, 0, 0]));
        }
        assert_eq!(content_box(&speck), None);
        speck.put_pixel(120, 60, image::Rgb([0, 0, 0]));
        assert_eq!(content_box(&speck), Some([100, 50, 121, 61]));
    }
}
