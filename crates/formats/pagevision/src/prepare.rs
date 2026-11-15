// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Page image preprocessing and the image token table.

use crate::{error::PageError, model::ImageRule};

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

/// The image token count llama.cpp gives for an image of this exact size, or None.
pub fn expected_image_tokens(width: u32, height: u32) -> Option<u32> {
    match (width, height) {
        (1408, 1856) => Some(2552),
        _ => None,
    }
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
    fn token_table() {
        assert_eq!(expected_image_tokens(1408, 1856), Some(2552));
        assert_eq!(expected_image_tokens(100, 100), None);
    }

    #[test]
    fn not_a_png_is_an_image_error() {
        assert!(matches!(
            prepare(b"nope", &QWEN35_4B.image),
            Err(PageError::Image(_))
        ));
    }
}
