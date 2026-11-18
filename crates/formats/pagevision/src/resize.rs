// Adapted from oar-ocr-vl 0.10.3 src/runtime/image.rs (Apache-2.0, Copyright the oar-ocr
// authors, https://github.com/GreatV/oar-ocr), which adapts OpenCV 4.13.0
// imgproc/src/resize.cpp (Apache-2.0). Changes: cv2_cubic_resize as there, plus
// cv2_linear_resize written on the same fixed-point pattern. Licensed under the Apache
// License, Version 2.0; see LICENSE-APACHE in this crate.

//! OpenCV's uint8 resizes: INTER_CUBIC (PP-DocLayoutV3 input) and INTER_LINEAR (PP-OCRv6 det
//! input). Both use 11-bit fixed-point coefficients and round once, as OpenCV's generic
//! (scalar) path. OpenCV's SIMD path for INTER_LINEAR rounds the vertical pass in steps,
//! `((b0 * (S0 >> 4)) >> 16) + ((b1 * (S1 >> 4)) >> 16) + 2) >> 2`, so cv2 on a SIMD machine
//! can differ by 1 in some pixels. The detector references were made with this rounding.

use image::RgbImage;

/// Fixed-point scale of the coefficients (11 bits).
const ONE: f32 = 2048.;
/// Rounds the product of two 11-bit coefficient sums back to 8 bits.
const ROUND: i32 = 1 << 21;
const SHIFT: u32 = 22;

struct CubicTap {
    indices: [usize; 4],
    weights: [i32; 4],
}

fn cubic_taps(input: u32, output: u32) -> Vec<CubicTap> {
    let scale = 1. / (f64::from(output) / f64::from(input));
    (0..output)
        .map(|position| {
            let center = ((f64::from(position) + 0.5) * scale - 0.5) as f32;
            let base = center.floor() as i64;
            let x = center - base as f32;
            // OpenCV's cubic coefficient A.
            let a = -0.75f32;
            let c0 = ((a * (x + 1.) - 5. * a) * (x + 1.) + 8. * a) * (x + 1.) - 4. * a;
            let c1 = ((a + 2.) * x - (a + 3.)) * x * x + 1.;
            let c2 = ((a + 2.) * (1. - x) - (a + 3.)) * (1. - x) * (1. - x) + 1.;
            CubicTap {
                indices: std::array::from_fn(|k| {
                    (base + k as i64 - 1).clamp(0, i64::from(input) - 1) as usize
                }),
                weights: [c0, c1, c2, 1. - c0 - c1 - c2]
                    .map(|weight| (weight * ONE).round_ties_even() as i32),
            }
        })
        .collect()
}

/// OpenCV's generic uint8 INTER_CUBIC path (what PaddleX uses for PP-DocLayout).
pub fn cv2_cubic_resize(image: &RgbImage, width: u32, height: u32) -> RgbImage {
    if image.dimensions() == (width, height) {
        return image.clone();
    }
    let horizontal = cubic_taps(image.width(), width);
    let vertical = cubic_taps(image.height(), height);
    let src_stride = image.width() as usize * 3;
    let src = image.as_raw();
    let stride = width as usize * 3;
    let mut rows = vec![0i32; stride * image.height() as usize];
    for (y, row) in rows.chunks_mut(stride).enumerate() {
        let line = &src[y * src_stride..(y + 1) * src_stride];
        for (x, tap) in horizontal.iter().enumerate() {
            for channel in 0..3 {
                row[x * 3 + channel] = tap
                    .indices
                    .iter()
                    .zip(tap.weights)
                    .map(|(&sx, weight)| i32::from(line[sx * 3 + channel]) * weight)
                    .sum();
            }
        }
    }
    let mut output = RgbImage::new(width, height);
    for (y, row) in output.chunks_mut(stride).enumerate() {
        let tap = &vertical[y];
        for (index, value) in row.iter_mut().enumerate() {
            let sum: i32 = tap
                .indices
                .iter()
                .zip(tap.weights)
                .map(|(&sy, weight)| rows[sy * stride + index] * weight)
                .sum();
            *value = ((sum + ROUND) >> SHIFT).clamp(0, 255) as u8;
        }
    }
    output
}

/// One linear tap: the two source indices and their weights.
type LinearTap = (usize, usize, i32, i32);

fn linear_taps(input: u32, output: u32) -> Vec<LinearTap> {
    let scale = f64::from(input) / f64::from(output);
    let last = i64::from(input) - 1;
    (0..output)
        .map(|d| {
            let mut f = ((f64::from(d) + 0.5) * scale - 0.5) as f32;
            let mut s = f.floor() as i64;
            f -= s as f32;
            if s < 0 {
                f = 0.;
                s = 0;
            }
            if s >= last {
                f = 0.;
                s = last;
            }
            let s1 = (s + 1).min(last) as usize;
            let c1 = (f * ONE).round_ties_even() as i32;
            (s as usize, s1, 2048 - c1, c1)
        })
        .collect()
}

/// OpenCV's uint8 INTER_LINEAR (fixed point, 11-bit coefficients).
pub fn cv2_linear_resize(image: &RgbImage, width: u32, height: u32) -> RgbImage {
    if image.dimensions() == (width, height) {
        return image.clone();
    }
    let horizontal = linear_taps(image.width(), width);
    let vertical = linear_taps(image.height(), height);
    let src_stride = image.width() as usize * 3;
    let src = image.as_raw();
    let stride = width as usize * 3;
    let mut rows = vec![0i32; stride * image.height() as usize];
    for (y, row) in rows.chunks_mut(stride).enumerate() {
        let line = &src[y * src_stride..(y + 1) * src_stride];
        for (x, &(s0, s1, c0, c1)) in horizontal.iter().enumerate() {
            for c in 0..3 {
                row[x * 3 + c] =
                    i32::from(line[s0 * 3 + c]) * c0 + i32::from(line[s1 * 3 + c]) * c1;
            }
        }
    }
    let mut output = RgbImage::new(width, height);
    for (y, row) in output.chunks_mut(stride).enumerate() {
        let (s0, s1, c0, c1) = vertical[y];
        for (i, value) in row.iter_mut().enumerate() {
            let sum = rows[s0 * stride + i] * c0 + rows[s1 * stride + i] * c1;
            *value = ((sum + ROUND) >> SHIFT).clamp(0, 255) as u8;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x4 RGB image with varied channels.
    fn source() -> RgbImage {
        let data = vec![
            0, 89, 178, 61, 150, 239, 122, 211, 44, 183, 16, 105, 17, 106, 195, 78, 167, 0, 255, 0,
            7, 200, 33, 122, 34, 123, 212, 95, 184, 17, 156, 245, 78, 217, 50, 139, 0, 255, 128,
            112, 201, 34, 173, 6, 95, 234, 67, 156,
        ];
        RgbImage::from_raw(4, 4, data).unwrap()
    }

    // The expected values were computed once with OpenCV 4.13.0 (opencv-python-headless
    // 4.13.0.92): cv2.resize(src, (w, h), interpolation=...), run with `python3 -I`.

    /// The positions where `got` differs from cv2; every difference must be at most 1.
    fn off_by_one(got: &[u8], cv2: &[u8]) -> Vec<usize> {
        assert_eq!(got.len(), cv2.len());
        got.iter()
            .zip(cv2)
            .enumerate()
            .filter(|&(_, (a, b))| a != b)
            .map(|(i, (a, b))| {
                assert_eq!(a.abs_diff(*b), 1, "value {i}: {a} against cv2 {b}");
                i
            })
            .collect()
    }

    #[test]
    fn linear_resize_matches_cv2_golden() {
        // cv2 here runs the SIMD vertical pass, which rounds in steps (see the module doc).
        // The port rounds once, as the reference the detector tests compare with. So a few
        // values are 1 above cv2; this test pins exactly which.
        let wide = cv2_linear_resize(&source(), 7, 3);
        let cv2_wide = [
            3, 92, 181, 24, 114, 187, 59, 148, 198, 104, 164, 118, 147, 165, 43, 171, 75, 83, 186,
            19, 108, 26, 115, 204, 47, 136, 134, 82, 171, 22, 146, 149, 26, 206, 117, 49, 207, 70,
            99, 209, 42, 131, 6, 233, 142, 43, 220, 102, 102, 200, 39, 140, 122, 62, 174, 47, 96,
            209, 57, 131, 231, 64, 153,
        ];
        let tall = cv2_linear_resize(&source(), 3, 5);
        let cv2_tall = [
            10, 99, 188, 91, 180, 141, 173, 48, 95, 22, 111, 170, 144, 112, 45, 198, 34, 100, 36,
            125, 171, 146, 149, 26, 208, 55, 116, 36, 167, 159, 130, 181, 53, 212, 75, 134, 19,
            246, 112, 142, 103, 64, 224, 57, 146,
        ];
        assert_eq!(
            off_by_one(wide.as_raw(), &cv2_wide),
            [3, 11, 46, 49, 54, 56, 58]
        );
        assert_eq!(
            off_by_one(tall.as_raw(), &cv2_tall),
            [3, 4, 5, 13, 27, 30, 39, 40, 41]
        );
    }

    #[test]
    fn cubic_resize_matches_cv2_golden() {
        let wide = cv2_cubic_resize(&source(), 7, 3);
        assert_eq!(
            wide.as_raw(),
            &[
                0, 85, 178, 18, 110, 199, 56, 147, 211, 105, 185, 117, 149, 168, 34, 176, 74, 73,
                189, 2, 114, 25, 97, 236, 41, 131, 132, 77, 174, 0, 157, 165, 0, 222, 118, 45, 219,
                67, 102, 208, 33, 140, 0, 240, 151, 37, 233, 99, 101, 208, 35, 143, 116, 47, 175,
                38, 100, 214, 48, 137, 238, 67, 160,
            ]
        );
        let tall = cv2_cubic_resize(&source(), 3, 5);
        assert_eq!(
            tall.as_raw(),
            &[
                7, 95, 191, 86, 212, 152, 173, 46, 92, 16, 110, 171, 159, 102, 16, 201, 25, 99, 36,
                115, 179, 157, 165, 0, 212, 53, 118, 36, 165, 165, 126, 215, 36, 214, 78, 139, 13,
                255, 109, 149, 84, 52, 227, 53, 149,
            ]
        );
    }

    #[test]
    fn same_size_is_a_copy() {
        assert_eq!(cv2_cubic_resize(&source(), 4, 4), source());
        assert_eq!(cv2_linear_resize(&source(), 4, 4), source());
    }
}
