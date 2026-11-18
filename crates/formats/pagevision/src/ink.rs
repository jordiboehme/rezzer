// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The detector page and its ink mask.
//!
//! This is the page of the region bake-off: grey on white, ink below grey 110, ruled template
//! lines removed. The morphology follows scipy.ndimage defaults (border value 0, the 4-connected
//! cross as structure), so the counts match the bake-off code pixel for pixel.

use crate::error::PageError;

/// Bumped whenever the ink mask or the components change for the same page.
pub const INK_VERSION: u32 = 1;

/// Alpha below this is paper, not ink. Bake-off value, gate 1.
const ALPHA_PAPER: u8 = 128;
/// Grey below this is ink. Bake-off value, gate 1.
const INK_GREY: u8 = 110;

/// A grey page, row-major, one byte per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gray {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// A bit mask over the page, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub bits: Vec<bool>,
}

impl Mask {
    /// An empty mask of this size.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bits: vec![false; width as usize * height as usize],
        }
    }

    /// The bit at (x, y). Panics outside the mask.
    pub fn get(&self, x: u32, y: u32) -> bool {
        assert!(
            x < self.width && y < self.height,
            "({x}, {y}) is outside the mask"
        );
        self.bits[y as usize * self.width as usize + x as usize]
    }

    /// Set the bit at (x, y). Panics outside the mask.
    pub fn set(&mut self, x: u32, y: u32, on: bool) {
        assert!(
            x < self.width && y < self.height,
            "({x}, {y}) is outside the mask"
        );
        self.bits[y as usize * self.width as usize + x as usize] = on;
    }

    /// The number of set bits.
    pub fn count(&self) -> usize {
        self.bits.iter().filter(|&&b| b).count()
    }
}

/// A connected component: its label and the box of its ink pixels.
/// `x1` and `y1` are exclusive, as numpy slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    pub label: u32,
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    pub pixels: u32,
}

/// Decode a PNG to the grey detector page: alpha below 128 is white, any other pixel is the
/// truncated mean of R, G and B.
pub fn flatten(png: &[u8]) -> Result<Gray, PageError> {
    let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| PageError::Image(e.to_string()))?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    let data = rgba
        .pixels()
        .map(|p| {
            if p[3] < ALPHA_PAPER {
                255
            } else {
                // The mean of three bytes is at most 255.
                ((u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2])) / 3) as u8
            }
        })
        .collect();
    Ok(Gray {
        width,
        height,
        data,
    })
}

/// The grey page as RGB, for the detectors.
pub fn gray_to_rgb(g: &Gray) -> image::RgbImage {
    let mut out = image::RgbImage::new(g.width, g.height);
    for (dst, &v) in out.pixels_mut().zip(&g.data) {
        *dst = image::Rgb([v, v, v]);
    }
    out
}

/// Ink without the ruled template lines.
///
/// A ruled line is a horizontal run of ink at least half the page wide (a binary opening with a
/// one-row structure of `width / 2` pixels). The row above and below it goes too (a 3x1
/// dilation), so the anti-aliased edges of the rule are not left behind as ink.
pub fn ink_mask(g: &Gray) -> Mask {
    let (w, h) = (g.width as usize, g.height as usize);
    let ink: Vec<bool> = g.data.iter().map(|&v| v < INK_GREY).collect();
    // Half the page width: a ruled line spans the page, handwriting does not.
    // Bake-off value, gate 1.
    let min_run = w / 2;
    let mut rules = vec![false; w * h];
    if min_run > 0 {
        for y in 0..h {
            let row = &ink[y * w..(y + 1) * w];
            let mut x = 0;
            while x < w {
                if !row[x] {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < w && row[x] {
                    x += 1;
                }
                // An opening keeps exactly the runs that hold a full structure.
                if x - start >= min_run {
                    rules[y * w + start..y * w + x].fill(true);
                }
            }
        }
    }
    let mut out = Mask::new(g.width, g.height);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let near_rule = rules[i] || (y > 0 && rules[i - w]) || (y + 1 < h && rules[i + w]);
            out.bits[i] = ink[i] && !near_rule;
        }
    }
    out
}

/// Binary dilation with the 4-connected cross, `iterations` times: every pixel within Manhattan
/// distance `iterations` of a set pixel. Nothing grows past the page edge.
pub fn dilate_cross(m: &Mask, iterations: u32) -> Mask {
    let (w, h) = (m.width as usize, m.height as usize);
    let far = u32::MAX / 2;
    // Two-pass L1 distance transform. On a rectangle it equals the repeated cross dilation.
    let mut d: Vec<u32> = m.bits.iter().map(|&b| if b { 0 } else { far }).collect();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x > 0 {
                d[i] = d[i].min(d[i - 1] + 1);
            }
            if y > 0 {
                d[i] = d[i].min(d[i - w] + 1);
            }
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if x + 1 < w {
                d[i] = d[i].min(d[i + 1] + 1);
            }
            if y + 1 < h {
                d[i] = d[i].min(d[i + w] + 1);
            }
        }
    }
    Mask {
        width: m.width,
        height: m.height,
        bits: d.into_iter().map(|v| v <= iterations).collect(),
    }
}

/// 4-connected labels, row-major, 0 for background, with the count. Labels are numbered in the
/// raster order of each component's first pixel, as scipy.ndimage.label does.
pub fn label4(m: &Mask) -> (Vec<u32>, u32) {
    let (w, h) = (m.width as usize, m.height as usize);
    let mut labels = vec![0u32; w * h];
    let mut next = 0u32;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !m.bits[start] || labels[start] != 0 {
            continue;
        }
        next += 1;
        labels[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                if m.bits[j] && labels[j] == 0 {
                    labels[j] = next;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
    }
    (labels, next)
}

/// The ink components when ink whose cross dilations by `dilate` touch (up to 2 * dilate + 1 px
/// apart) counts as joined: the labels of
/// `dilate_cross(ink, dilate)` kept only on ink pixels, and the box of each label's ink pixels in
/// label order (scipy's `find_objects`).
pub fn components(ink: &Mask, dilate: u32) -> (Vec<u32>, Vec<Component>) {
    let (mut labels, n) = label4(&dilate_cross(ink, dilate));
    let w = ink.width as usize;
    let mut comps: Vec<Option<Component>> = vec![None; n as usize];
    for (i, l) in labels.iter_mut().enumerate() {
        if !ink.bits[i] {
            *l = 0;
        }
        if *l == 0 {
            continue;
        }
        let (x, y) = ((i % w) as u32, (i / w) as u32);
        let c = comps[*l as usize - 1].get_or_insert(Component {
            label: *l,
            x0: x,
            y0: y,
            x1: x + 1,
            y1: y + 1,
            pixels: 0,
        });
        c.x0 = c.x0.min(x);
        c.y0 = c.y0.min(y);
        c.x1 = c.x1.max(x + 1);
        c.y1 = c.y1.max(y + 1);
        c.pixels += 1;
    }
    // Every dilated component holds at least one ink pixel, so no label is empty.
    (labels, comps.into_iter().flatten().collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let img = image::RgbaImage::from_raw(width, height, rgba.to_vec()).unwrap();
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn gray(width: u32, height: u32, fill: u8) -> Gray {
        Gray {
            width,
            height,
            data: vec![fill; (width * height) as usize],
        }
    }

    fn single_pixel(width: u32, height: u32, x: u32, y: u32) -> Mask {
        let mut m = Mask::new(width, height);
        m.set(x, y, true);
        m
    }

    #[test]
    fn flatten_half_alpha_is_white_and_mean_truncates() {
        // 2x1 RGBA PNG: (10,20,31,255) and (0,0,0,127)
        let g = flatten(&png_rgba(2, 1, &[10, 20, 31, 255, 0, 0, 0, 127])).unwrap();
        assert_eq!(g.data, vec![20, 255]); // 61/3 = 20.33 -> 20
    }

    #[test]
    fn flatten_alpha_128_is_ink() {
        let g = flatten(&png_rgba(1, 1, &[0, 0, 0, 128])).unwrap();
        assert_eq!(g.data, vec![0]);
    }

    #[test]
    fn flatten_rejects_a_non_png() {
        assert!(matches!(flatten(b"no png"), Err(PageError::Image(_))));
    }

    #[test]
    fn gray_to_rgb_copies_the_grey() {
        let g = Gray {
            width: 2,
            height: 1,
            data: vec![7, 200],
        };
        assert_eq!(gray_to_rgb(&g).into_raw(), vec![7, 7, 7, 200, 200, 200]);
    }

    #[test]
    fn ink_threshold_is_110_both_ways() {
        let mut g = gray(10, 1, 255);
        g.data[2] = 109;
        g.data[6] = 110;
        let m = ink_mask(&g);
        assert!(m.get(2, 0));
        assert!(!m.get(6, 0));
    }

    #[test]
    fn ruled_line_wider_than_half_page_is_removed() {
        // 100x20 page, a 60 px row of ink and a 40 px row: 60 removed, 40 kept.
        let mut g = gray(100, 20, 255);
        for x in 10..70 {
            g.data[5 * 100 + x] = 0;
        }
        for x in 10..50 {
            g.data[15 * 100 + x] = 0;
        }
        let m = ink_mask(&g);
        assert!((0..100).all(|x| !m.get(x, 5)));
        assert_eq!((0..100).filter(|&x| m.get(x, 15)).count(), 40);
    }

    #[test]
    fn ruled_line_of_exactly_half_the_page_is_removed() {
        // Half of 101 is 50 (integer division): a 50 px run is a rule, 49 px is not.
        let mut g = gray(101, 10, 255);
        for x in 0..50 {
            g.data[2 * 101 + x] = 0;
        }
        for x in 0..49 {
            g.data[7 * 101 + x] = 0;
        }
        let m = ink_mask(&g);
        assert_eq!((0..101).filter(|&x| m.get(x, 2)).count(), 0);
        assert_eq!((0..101).filter(|&x| m.get(x, 7)).count(), 49);
    }

    #[test]
    fn ruled_line_removal_also_takes_the_rows_above_and_below() {
        let mut g = gray(20, 10, 255);
        for x in 0..20 {
            g.data[5 * 20 + x] = 0;
        }
        // Short ink one, two and three rows away from the rule.
        for (x, y) in [(3, 4), (4, 6), (5, 3), (6, 7)] {
            g.data[y * 20 + x] = 0;
        }
        let m = ink_mask(&g);
        assert!(!m.get(3, 4) && !m.get(4, 6));
        assert!(m.get(5, 3) && m.get(6, 7));
    }

    #[test]
    fn dilation_is_a_diamond() {
        let m = dilate_cross(&single_pixel(21, 21, 10, 10), 3);
        assert!(m.get(13, 10) && m.get(11, 12) && !m.get(12, 12));
        assert!(!m.get(14, 10));
        assert_eq!(m.count(), 25);
    }

    #[test]
    fn dilation_stops_at_the_page_edge() {
        let m = dilate_cross(&single_pixel(5, 5, 0, 0), 2);
        assert_eq!(m.count(), 6);
    }

    #[test]
    fn label4_does_not_join_diagonals() {
        let mut m = Mask::new(3, 3);
        m.set(0, 0, true);
        m.set(1, 1, true);
        assert_eq!(label4(&m).1, 2);
    }

    #[test]
    fn label4_numbers_in_raster_order_of_the_first_pixel() {
        // A U shape opens at the top: its two arms meet only below, after the dot on row 0.
        let mut m = Mask::new(7, 3);
        for (x, y) in [
            (0, 0),
            (2, 0),
            (0, 1),
            (2, 1),
            (0, 2),
            (1, 2),
            (2, 2),
            (5, 0),
        ] {
            m.set(x, y, true);
        }
        let (labels, n) = label4(&m);
        assert_eq!(n, 2);
        assert_eq!(labels[0], 1);
        assert_eq!(labels[2], 1);
        assert_eq!(labels[5], 2);
    }

    #[test]
    fn components_keep_only_ink_pixels() {
        // Two dots 4 px apart, dilate 3: one label, two pixels.
        let mut m = Mask::new(20, 5);
        m.set(5, 2, true);
        m.set(9, 2, true);
        let (labels, comps) = components(&m, 3);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].pixels, 2);
        assert_eq!(
            (comps[0].x0, comps[0].y0, comps[0].x1, comps[0].y1),
            (5, 2, 10, 3)
        );
        assert_eq!(labels.iter().filter(|&&l| l != 0).count(), 2);
    }

    #[test]
    fn components_split_beyond_the_dilation() {
        // Two dilations of 3 touch (side by side) up to 7 px apart, not at 8.
        let mut m = Mask::new(20, 5);
        m.set(5, 2, true);
        m.set(13, 2, true);
        assert_eq!(components(&m, 3).1.len(), 2);
        m.set(13, 2, false);
        m.set(12, 2, true);
        assert_eq!(components(&m, 3).1.len(), 1);
    }
}
