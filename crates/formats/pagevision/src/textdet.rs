// Adapted from PaddleOCR DBPostProcess and DetResizeForTest (Apache-2.0, Copyright the
// PaddlePaddle Authors) as ported in oar-ocr-core db.rs (Apache-2.0, Copyright the oar-ocr
// authors, https://github.com/GreatV/oar-ocr). Changes: outer contours only, quad boxes
// with the fast score, axis-aligned output boxes. Licensed under the Apache License,
// Version 2.0; see LICENSE-APACHE in this crate.

//! PP-OCRv6 small det: the input size and tensor, and the DB post-processing that turns the
//! probability map into text line boxes. Default code; the model runs in `detect::v6`.

use image::RgbImage;

use crate::resize::cv2_linear_resize;
use crate::rules::Rect;

/// DetResizeForTest: the longer side is scaled down to this limit (limit type "max").
pub const DET_LIMIT: u32 = 960;
/// DetResizeForTest's cap on the longer side after the limit. With the limit of 960 it is
/// never reached; it stays as in PaddleOCR.
pub const DET_MAX_SIDE: u32 = 4000;
/// A pixel belongs to text when its probability is above this.
pub const THRESH: f32 = 0.2;
/// A box is kept when the mean probability inside it is at least this.
pub const BOX_THRESH: f64 = 0.45;
/// The box grows by area * ratio / perimeter on every side.
pub const UNCLIP_RATIO: f64 = 1.4;
/// At most this many connected regions are looked at.
pub const MAX_CANDIDATES: u32 = 3000;
/// A box whose short side is below this (map pixels) is dropped before unclip.
const MIN_SIDE: f64 = 3.;
/// A box whose short side is below this (map pixels) is dropped after unclip. With
/// `MIN_SIDE` 3 and `UNCLIP_RATIO` 1.4 a kept box grows to at least 3 + 1.4 * 9 / 6 = 5.1,
/// so this never drops a box; it stays as in PaddleOCR.
const MIN_SIDE_UNCLIPPED: f64 = 5.;

/// The model input size for a page: the longer side down to 960, then multiples of 32.
/// A 1404x1872 page gives 704x960.
pub fn det_size(w: u32, h: u32) -> (u32, u32) {
    let (wf, hf) = (f64::from(w), f64::from(h));
    let ratio = if wf.max(hf) > f64::from(DET_LIMIT) {
        f64::from(DET_LIMIT) / wf.max(hf)
    } else {
        1.
    };
    let mut rh = (hf * ratio) as i64;
    let mut rw = (wf * ratio) as i64;
    if rh.max(rw) > i64::from(DET_MAX_SIDE) {
        let r = f64::from(DET_MAX_SIDE) / rh.max(rw) as f64;
        rh = (rh as f64 * r) as i64;
        rw = (rw as f64 * r) as i64;
    }
    let rh = (((rh as f64) / 32.).round_ties_even() as i64 * 32).max(32);
    let rw = (((rw as f64) / 32.).round_ties_even() as i64 * 32).max(32);
    (rw as u32, rh as u32)
}

/// The model input: OpenCV linear resize to [`det_size`], BGR, ImageNet mean and std in BGR
/// order (as Paddle), CHW. Returns the data and the input width and height.
pub fn v6_input(rgb: &RgbImage) -> (Vec<f32>, u32, u32) {
    let (rw, rh) = det_size(rgb.width(), rgb.height());
    let resized = cv2_linear_resize(rgb, rw, rh);
    let n = (rw * rh) as usize;
    let mean = [0.485f32, 0.456, 0.406];
    let std = [0.229f32, 0.224, 0.225];
    let mut data = vec![0f32; 3 * n];
    for (i, p) in resized.pixels().enumerate() {
        let bgr = [p[2], p[1], p[0]];
        for c in 0..3 {
            data[c * n + i] = (f32::from(bgr[c]) / 255.0 - mean[c]) / std[c];
        }
    }
    (data, rw, rh)
}

/// How far a box of `w` x `h` grows on every side: area * ratio / perimeter.
pub fn unclip_distance(w: f64, h: f64) -> f64 {
    w * h * UNCLIP_RATIO / (2. * (w + h))
}

/// A rotated rectangle: centre, unit axes and half sizes.
#[derive(Clone, Copy, Debug)]
struct Rotated {
    c: (f64, f64),
    u: (f64, f64),
    v: (f64, f64),
    hw: f64,
    hh: f64,
}

impl Rotated {
    fn corners(&self) -> [(f64, f64); 4] {
        let p = |a: f64, b: f64| {
            (
                self.c.0 + a * self.u.0 + b * self.v.0,
                self.c.1 + a * self.u.1 + b * self.v.1,
            )
        };
        [
            p(-self.hw, -self.hh),
            p(self.hw, -self.hh),
            p(self.hw, self.hh),
            p(-self.hw, self.hh),
        ]
    }

    fn short_side(&self) -> f64 {
        (2. * self.hw).min(2. * self.hh)
    }
}

fn cross(o: (i64, i64), a: (i64, i64), b: (i64, i64)) -> i64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// Convex hull (monotone chain).
fn hull(mut pts: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    pts.sort_unstable();
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let mut lower: Vec<(i64, i64)> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(i64, i64)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The minimum-area rectangle around a convex hull (rotating calipers over the hull edges).
fn min_area_rect(h: &[(i64, i64)]) -> Rotated {
    let pts: Vec<(f64, f64)> = h.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
    let single = Rotated {
        c: pts[0],
        u: (1., 0.),
        v: (0., 1.),
        hw: 0.,
        hh: 0.,
    };
    let mut best: Option<(f64, Rotated)> = None;
    let n = pts.len();
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0. {
            continue;
        }
        let u = (dx / len, dy / len);
        let v = (-u.1, u.0);
        let (mut umin, mut umax, mut vmin, mut vmax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in &pts {
            let pu = p.0 * u.0 + p.1 * u.1;
            let pv = p.0 * v.0 + p.1 * v.1;
            umin = umin.min(pu);
            umax = umax.max(pu);
            vmin = vmin.min(pv);
            vmax = vmax.max(pv);
        }
        let area = (umax - umin) * (vmax - vmin);
        if best.as_ref().is_none_or(|(b, _)| area < *b - 1e-9) {
            let cu = (umin + umax) / 2.;
            let cv = (vmin + vmax) / 2.;
            let c = (cu * u.0 + cv * v.0, cu * u.1 + cv * v.1);
            let r = Rotated {
                c,
                u,
                v,
                hw: (umax - umin) / 2.,
                hh: (vmax - vmin) / 2.,
            };
            best = Some((area, r));
        }
    }
    best.map_or(single, |b| b.1)
}

/// The mean probability inside a rotated box (PaddleOCR's fast box score).
fn box_score(prob: &[f32], w: usize, h: usize, r: &Rotated) -> f64 {
    let cs = r.corners();
    let bound = |f: fn(f64, f64) -> f64, start: f64, get: fn(&(f64, f64)) -> f64| {
        cs.iter().map(get).fold(start, f)
    };
    let clamp_x = |v: f64| v.clamp(0., w as f64 - 1.) as usize;
    let clamp_y = |v: f64| v.clamp(0., h as f64 - 1.) as usize;
    let xmin = clamp_x(bound(f64::min, f64::MAX, |p| p.0).floor());
    let xmax = clamp_x(bound(f64::max, f64::MIN, |p| p.0).ceil());
    let ymin = clamp_y(bound(f64::min, f64::MAX, |p| p.1).floor());
    let ymax = clamp_y(bound(f64::max, f64::MIN, |p| p.1).ceil());
    let (mut sum, mut count) = (0f64, 0usize);
    for yy in ymin..=ymax {
        for xx in xmin..=xmax {
            let (px, py) = (xx as f64 - r.c.0, yy as f64 - r.c.1);
            let du = px * r.u.0 + py * r.u.1;
            let dv = px * r.v.0 + py * r.v.1;
            if du.abs() <= r.hw + 1e-6 && dv.abs() <= r.hh + 1e-6 {
                sum += f64::from(prob[yy * w + xx]);
                count += 1;
            }
        }
    }
    if count > 0 { sum / count as f64 } else { 0. }
}

/// DB post-processing of a probability map of `w` x `h` (row-major). Returns one
/// axis-aligned box per text line, `[x0, y0, x1, y1]` in the pixels of `page` (width,
/// height), in the order the regions are found (raster order of their first pixel).
pub fn db_lines(prob: &[f32], w: usize, h: usize, page: (u32, u32)) -> Vec<Rect> {
    let bit: Vec<bool> = prob.iter().map(|&p| p > THRESH).collect();
    let mut label = vec![0u32; w * h];
    let mut out = Vec::new();
    let mut next = 0u32;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !bit[start] || label[start] != 0 {
            continue;
        }
        next += 1;
        if next > MAX_CANDIDATES {
            break;
        }
        // 8-connected region; its border pixels stand for the outer contour.
        let mut border = Vec::new();
        label[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            let mut is_border = false;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        is_border = true;
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if !bit[j] {
                        is_border = true;
                    } else if label[j] == 0 {
                        label[j] = next;
                        stack.push(j);
                    }
                }
            }
            if is_border {
                border.push((x, y));
            }
        }
        let r = min_area_rect(&hull(border));
        if r.short_side() < MIN_SIDE {
            continue;
        }
        if box_score(prob, w, h, &r) < BOX_THRESH {
            continue;
        }
        let d = unclip_distance(2. * r.hw, 2. * r.hh);
        let grown = Rotated {
            hw: r.hw + d,
            hh: r.hh + d,
            ..r
        };
        if grown.short_side() < MIN_SIDE_UNCLIPPED {
            continue;
        }
        let (pw, ph) = (f64::from(page.0), f64::from(page.1));
        let quad = grown.corners().map(|(x, y)| {
            (
                (x / w as f64 * pw).round_ties_even().clamp(0., pw),
                (y / h as f64 * ph).round_ties_even().clamp(0., ph),
            )
        });
        out.push([
            quad.iter().map(|p| p.0).fold(f64::MAX, f64::min),
            quad.iter().map(|p| p.1).fold(f64::MAX, f64::min),
            quad.iter().map(|p| p.0).fold(f64::MIN, f64::max),
            quad.iter().map(|p| p.1).fold(f64::MIN, f64::max),
        ]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(w: usize, h: usize, blobs: &[([usize; 4], f32)]) -> Vec<f32> {
        let mut p = vec![0f32; w * h];
        for &([x0, y0, x1, y1], v) in blobs {
            for y in y0..y1 {
                for x in x0..x1 {
                    p[y * w + x] = v;
                }
            }
        }
        p
    }

    #[test]
    fn det_size_of_a_supernote_page() {
        assert_eq!(det_size(1404, 1872), (704, 960));
    }

    #[test]
    fn det_size_of_a_landscape_page() {
        assert_eq!(det_size(1872, 1404), (960, 704));
    }

    #[test]
    fn det_size_keeps_a_small_page_and_rounds_to_32() {
        assert_eq!(det_size(500, 300), (512, 288));
        assert_eq!(det_size(10, 10), (32, 32));
    }

    #[test]
    fn v6_input_is_bgr_normalized_chw() {
        let mut img = RgbImage::new(1404, 1872);
        img.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        let (data, w, h) = v6_input(&img);
        assert_eq!((w, h), (704, 960));
        let n = 704 * 960;
        assert_eq!(data.len(), 3 * n);
        // Black: (0 - mean) / std per BGR channel.
        assert!((data[n + 5000] - (-0.456 / 0.224)).abs() < 1e-6);
        // The red pixel lands in the last (R) plane.
        assert!(data[2 * n] > data[0]);
    }

    #[test]
    fn db_finds_one_box_per_blob() {
        let p = map(100, 60, &[([10, 10, 40, 20], 0.9), ([60, 30, 90, 45], 0.8)]);
        let lines = db_lines(&p, 100, 60, (100, 60));
        // Blob 1: pixel centres 10..=39 x 10..=19, so 29 x 9; it grows by
        // 29 * 9 * 1.4 / 76 = 4.81 on every side.
        // Blob 2: 29 x 14, grows by 29 * 14 * 1.4 / 86 = 6.61.
        assert_eq!(
            lines,
            vec![[5., 5., 44., 24.], [53., 23., 96., 51.]],
            "{lines:?}"
        );
        // Scaled to a page twice as large.
        let lines = db_lines(&p, 100, 60, (200, 120));
        assert_eq!(lines[0], [10., 10., 88., 48.]);
    }

    #[test]
    fn db_drops_a_blob_below_box_thresh() {
        let p = map(
            100,
            60,
            &[([10, 10, 40, 20], 0.44), ([60, 30, 90, 45], 0.46)],
        );
        let lines = db_lines(&p, 100, 60, (100, 60));
        assert_eq!(lines, vec![[53., 23., 96., 51.]]);
    }

    #[test]
    fn db_drops_a_blob_at_thresh() {
        // 0.2 is not above THRESH, 0.2001 is (and its box score is below 0.45).
        let p = map(50, 50, &[([5, 5, 30, 15], 0.2)]);
        assert!(db_lines(&p, 50, 50, (50, 50)).is_empty());
        let p = map(50, 50, &[([5, 5, 30, 15], 0.2001), ([5, 30, 30, 40], 0.9)]);
        assert_eq!(db_lines(&p, 50, 50, (50, 50)).len(), 1);
    }

    #[test]
    fn unclip_grows_by_area_times_ratio_over_perimeter() {
        assert!((unclip_distance(100., 20.) - 100. * 20. * 1.4 / 240.).abs() < 1e-12);
        assert!((unclip_distance(10., 10.) - 3.5).abs() < 1e-12);
    }

    #[test]
    fn db_drops_thin_blobs() {
        // Short side 2 (pixel centres) is below 3; 3 is kept.
        let p = map(60, 30, &[([5, 5, 40, 8], 0.9), ([5, 15, 40, 19], 0.9)]);
        let lines = db_lines(&p, 60, 30, (60, 30));
        assert_eq!(lines.len(), 1);
        assert!(lines[0][1] > 8.);
    }

    #[test]
    fn db_stops_after_3000_candidates() {
        // 55 x 55 = 3025 blobs of 4 x 4 pixels; only the first 3000 are looked at.
        let (w, h) = (330, 330);
        let mut blobs = Vec::new();
        for by in 0..55 {
            for bx in 0..55 {
                blobs.push(([bx * 6, by * 6, bx * 6 + 4, by * 6 + 4], 0.9));
            }
        }
        let p = map(w, h, &blobs);
        assert_eq!(db_lines(&p, w, h, (330, 330)).len(), 3000);
    }
}
