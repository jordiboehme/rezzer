// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tiles and the pixels of each attempt.

use crate::{Code, Limits, Picture, Pixels};

/// A rectangle of the picture: left, top, width and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The starts of tiles of `size` along a side of `len` pixels, `step` apart; the last tile
/// ends at the side's end.
pub(crate) fn starts(len: u32, size: u32, step: u32) -> Vec<u32> {
    if size >= len {
        return vec![0];
    }
    let mut out = Vec::new();
    let mut at = 0;
    while at + size < len {
        out.push(at);
        at += step.max(1);
    }
    out.push(len - size);
    out
}

/// The tiles of a picture of `w` by `h` pixels: the whole picture when it is no larger than
/// one tile, else overlapping tiles of at most [`Limits::tile_pixels`] each.
pub(crate) fn tiles(w: u32, h: u32, limits: &Limits) -> Vec<Rect> {
    let max = limits.tile_pixels.max(1);
    if u64::from(w) * u64::from(h) <= max {
        return vec![Rect { x: 0, y: 0, w, h }];
    }
    // A step of at least half a tile, so a small tile size cannot make the tiles too many.
    let step = |size: u32| size.saturating_sub(limits.overlap).max(size / 2).max(1);
    let grid = |tw: u32, th: u32| {
        let (tw, th) = (tw.clamp(1, w), th.clamp(1, h));
        let mut out = Vec::new();
        for y in starts(h, th, step(th)) {
            for x in starts(w, tw, step(tw)) {
                out.push(Rect { x, y, w: tw, h: th });
            }
        }
        out
    };
    let side = (max as f64).sqrt().floor() as u32;
    let fit = |len: u32| (max / u64::from(len.max(1))).min(u64::from(u32::MAX)) as u32;
    // Square tiles, or strips as wide or as high as the picture: the layout that reads the
    // fewest pixels.
    let mut layouts = vec![grid(side, side)];
    if fit(w) > limits.overlap {
        layouts.push(grid(w, fit(w)));
    }
    if fit(h) > limits.overlap {
        layouts.push(grid(fit(h), h));
    }
    let read = |t: &Vec<Rect>| {
        t.iter()
            .map(|r| u64::from(r.w) * u64::from(r.h))
            .sum::<u64>()
    };
    layouts.into_iter().min_by_key(read).unwrap_or_default()
}

/// Rec. 709 luma of a color already put onto white.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 2126 + u32::from(g) * 7152 + u32::from(b) * 722) / 10_000) as u8
}

/// A color channel put onto white by its alpha.
fn on_white(c: u8, a: u8) -> u8 {
    ((u32::from(c) * u32::from(a) + 255 * (255 - u32::from(a))) / 255) as u8
}

/// The gray pixels of one tile.
pub(crate) struct Tile {
    pub rect: Rect,
    pub luma: Vec<u8>,
}

impl Tile {
    /// The tile `rect` of `picture` in gray: alpha put onto white, then luma.
    pub(crate) fn cut(picture: &Picture<'_>, rect: Rect) -> Tile {
        let mut luma_px = Vec::with_capacity(rect.w as usize * rect.h as usize);
        let width = picture.width as usize;
        for row in rect.y..rect.y + rect.h {
            let from = row as usize * width + rect.x as usize;
            match picture.pixels {
                Pixels::Gray(p) => luma_px.extend_from_slice(&p[from..from + rect.w as usize]),
                Pixels::Rgba(p) => {
                    for px in p[from * 4..(from + rect.w as usize) * 4].as_chunks::<4>().0 {
                        let a = px[3];
                        luma_px.push(luma(
                            on_white(px[0], a),
                            on_white(px[1], a),
                            on_white(px[2], a),
                        ));
                    }
                }
            }
        }
        Tile {
            rect,
            luma: luma_px,
        }
    }

    /// The color channel of the tile (red, green, blue, the darkest or the lightest of the
    /// three) that splits best into two classes, when one splits clearly better than luma.
    /// `None` for a gray picture.
    pub(crate) fn channel(&self, picture: &Picture<'_>) -> Option<Vec<u8>> {
        let Pixels::Rgba(p) = picture.pixels else {
            return None;
        };
        let rect = self.rect;
        let width = picture.width as usize;
        let pick = |k: usize| -> Vec<u8> {
            let mut out = Vec::with_capacity(rect.w as usize * rect.h as usize);
            for row in rect.y..rect.y + rect.h {
                let from = row as usize * width + rect.x as usize;
                for px in p[from * 4..(from + rect.w as usize) * 4].as_chunks::<4>().0 {
                    let (r, g, b) = (
                        on_white(px[0], px[3]),
                        on_white(px[1], px[3]),
                        on_white(px[2], px[3]),
                    );
                    out.push(match k {
                        0 => r,
                        1 => g,
                        2 => b,
                        3 => r.min(g).min(b),
                        _ => r.max(g).max(b),
                    });
                }
            }
            out
        };
        let base = otsu(&self.luma).1;
        let mut best: Option<(f64, Vec<u8>)> = None;
        for k in 0..5 {
            let plane = pick(k);
            let score = otsu(&plane).1;
            if score > base + 0.02 && best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, plane));
            }
        }
        best.map(|(_, plane)| plane)
    }
}

/// The Otsu threshold of `v` and how well it splits the values (0 to 1, weighted by the
/// contrast).
pub(crate) fn otsu(v: &[u8]) -> (u8, f64) {
    let mut hist = [0u64; 256];
    for &x in v {
        hist[x as usize] += 1;
    }
    let total = v.len() as f64;
    if total == 0.0 {
        return (127, 0.0);
    }
    let sum: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let mean = sum / total;
    let var: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| c as f64 * (i as f64 - mean).powi(2))
        .sum::<f64>()
        / total;
    let (mut wb, mut sb) = (0f64, 0f64);
    let (mut best, mut threshold) = (0f64, 127u8);
    for (t, &count) in hist.iter().enumerate() {
        wb += count as f64;
        if wb == 0.0 {
            continue;
        }
        let wf = total - wb;
        if wf == 0.0 {
            break;
        }
        sb += t as f64 * count as f64;
        let (mb, mf) = (sb / wb, (sum - sb) / wf);
        let between = wb * wf * (mb - mf).powi(2) / (total * total);
        if between > best {
            best = between;
            threshold = t as u8;
        }
    }
    let split = if var > 0.0 { best / var } else { 0.0 };
    (threshold, split * (var.sqrt() / 128.0).min(1.0))
}

/// `v` in black and white at its Otsu threshold.
pub(crate) fn otsu_binary(v: &[u8]) -> Vec<u8> {
    let (t, _) = otsu(v);
    v.iter().map(|&x| if x > t { 255 } else { 0 }).collect()
}

/// `v` of `w` by `h` pixels at twice the size, each pixel copied.
pub(crate) fn doubled(v: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    for row in v.chunks_exact(w.max(1)).take(h) {
        let line: Vec<u8> = row.iter().flat_map(|&p| [p, p]).collect();
        out.extend_from_slice(&line);
        out.extend_from_slice(&line);
    }
    out
}

/// `v` of `w` by `h` pixels with `pad` pixels on each side in the color of its border, black
/// or white: a code cut tight has no quiet zone, and an inverted code needs a dark one.
pub(crate) fn pad(v: &[u8], w: u32, h: u32, pad: u32) -> Vec<u8> {
    let (w, h, p) = (w as usize, h as usize, pad as usize);
    let mut sum = 0u64;
    let mut n = 0u64;
    for (i, &x) in v.iter().enumerate() {
        let (cx, cy) = (i % w.max(1), i / w.max(1));
        if cx == 0 || cy == 0 || cx + 1 == w || cy + 1 == h {
            sum += u64::from(x);
            n += 1;
        }
    }
    let fill = if n == 0 || sum / n >= 128 { 255 } else { 0 };
    let pw = w + 2 * p;
    let mut out = vec![fill; pw * (h + 2 * p)];
    for (row, line) in v.chunks_exact(w.max(1)).take(h).enumerate() {
        let at = (row + p) * pw + p;
        out[at..at + w].copy_from_slice(line);
    }
    out
}

/// Paints box `b` (left, top, right, bottom, may reach past the edges) of a picture `w` by
/// `h` white.
pub(crate) fn paint(v: &mut [u8], w: u32, h: u32, b: [i64; 4]) {
    let x0 = b[0].clamp(0, i64::from(w)) as usize;
    let x1 = b[2].clamp(0, i64::from(w)) as usize;
    let y0 = b[1].clamp(0, i64::from(h)) as usize;
    let y1 = b[3].clamp(0, i64::from(h)) as usize;
    for row in y0..y1 {
        let at = row * w as usize;
        v[at + x0..at + x1].fill(255);
    }
}

/// Whether boxes `a` and `b` overlap.
fn overlap(a: &[u32; 4], b: &[u32; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// Whether `a` and `b` are the same code read twice: same kind, same payload, overlapping
/// boxes.
pub(crate) fn same(a: &Code, b: &Code) -> bool {
    a.symbology == b.symbology && a.payload == b.payload && overlap(&a.bounds, &b.bounds)
}

/// Whether the linear code `c` lies in one of the two-dimensional `codes` (a box grown by
/// 15 % of its larger side): a stacked or matrix code read as a line by mistake.
pub(crate) fn inside_2d(c: &Code, codes: &[Code]) -> bool {
    let mid = (
        (c.bounds[0] + c.bounds[2]) / 2,
        (c.bounds[1] + c.bounds[3]) / 2,
    );
    codes.iter().filter(|o| o.symbology.two_d()).any(|o| {
        let b = o.bounds;
        let m = (b[2] - b[0]).max(b[3] - b[1]) * 15 / 100;
        let grown = [
            b[0].saturating_sub(m),
            b[1].saturating_sub(m),
            b[2] + m,
            b[3] + m,
        ];
        let at = |x: u32, y: u32| x >= grown[0] && x <= grown[2] && y >= grown[1] && y <= grown[3];
        at(c.bounds[0], mid.1) || at(c.bounds[2], mid.1) || at(mid.0, mid.1)
    })
}

/// Whether a linear code read along a line of `v` (`w` by `h` pixels) between the hit's
/// points has bars: the row a little above or below the line shows the same pattern. Noise
/// can read as a linear code along one row, but its rows differ.
pub(crate) fn bars_hold(v: &[u8], w: u32, h: u32, points: &[(f32, f32)]) -> bool {
    if points.is_empty() {
        return false;
    }
    let y = points.iter().map(|p| p.1).sum::<f32>() / points.len() as f32;
    let x0 = points.iter().map(|p| p.0).fold(f32::MAX, f32::min).max(0.0) as u32;
    let x1 = points
        .iter()
        .map(|p| p.0)
        .fold(f32::MIN, f32::max)
        .min(w as f32 - 1.0) as u32;
    if x1 <= x0 + 8 || !(y >= 0.0 && (y as u32) < h) {
        return false;
    }
    let y = y as u32;
    let d = ((x1 - x0) / 20).max(3);
    let row = |r: u32| &v[(r * w + x0) as usize..=(r * w + x1) as usize];
    let (t, _) = otsu(row(y));
    let line: Vec<bool> = row(y).iter().map(|&p| p <= t).collect();
    let agrees = |r: u32| {
        let same = row(r)
            .iter()
            .zip(&line)
            .filter(|&(&p, &dark)| (p <= t) == dark)
            .count();
        same * 10 >= line.len() * 9
    };
    (y >= d && agrees(y - d)) || (y + d < h && agrees(y + d))
}

/// The most Aztec finders looked at in one tile.
pub(crate) const MAX_BULLSEYES: usize = 64;

/// Runs of equal widths along a line, dark and light in turn: `(start, length, dark)`.
fn runs(line: impl Iterator<Item = bool>) -> Vec<(u32, u32, bool)> {
    let mut out: Vec<(u32, u32, bool)> = Vec::new();
    for (i, dark) in line.enumerate() {
        match out.last_mut() {
            Some(r) if r.2 == dark => r.1 += 1,
            _ => out.push((i as u32, 1, dark)),
        }
    }
    out
}

/// Where along `runs` seven runs of about equal width, light first, lie between two dark
/// runs at least half as wide: the middle of the fourth run and the mean width. The rings of
/// an Aztec finder seen through its centre; its outer dark ring may run on into dark data.
fn equal_seven(runs: &[(u32, u32, bool)]) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    for k in 1..runs.len().saturating_sub(7) {
        let seven = &runs[k..k + 7];
        if seven[0].2 {
            continue;
        }
        let mean = seven.iter().map(|r| r.1 as f32).sum::<f32>() / 7.0;
        if mean < 1.5 || seven.iter().any(|r| (r.1 as f32 - mean).abs() > 0.4 * mean) {
            continue;
        }
        let (before, after) = (runs[k - 1], runs[k + 7]);
        if (before.1 as f32) < 0.5 * mean || (after.1 as f32) < 0.5 * mean {
            continue;
        }
        let mid = seven[3];
        out.push((mid.0 as f32 + mid.1 as f32 / 2.0, mean));
    }
    out
}

/// Likely centres of Aztec finders in `v` of `w` by `h` pixels: seven runs of equal width
/// across a row, and again down the column through their middle. At most
/// [`MAX_BULLSEYES`], each with its module size.
pub(crate) fn bullseyes(v: &[u8], w: u32, h: u32) -> Vec<(u32, u32, f32)> {
    let (t, _) = otsu(v);
    let dark = |x: u32, y: u32| v[(y * w + x) as usize] <= t;
    let mut out: Vec<(u32, u32, f32)> = Vec::new();
    for y in (0..h).step_by(2) {
        let row = runs((0..w).map(|x| dark(x, y)));
        for (cx, m) in equal_seven(&row) {
            let x = cx as u32;
            if out.iter().any(|&(ox, oy, om)| {
                ox.abs_diff(x) as f32 <= 4.0 * om && oy.abs_diff(y) as f32 <= 4.0 * om
            }) {
                continue;
            }
            let col = runs((0..h).map(|yy| dark(x, yy)));
            let down = equal_seven(&col)
                .into_iter()
                .find(|&(cy, cm)| (cy - y as f32).abs() <= m && (cm - m).abs() <= 0.5 * m);
            if let Some((cy, _)) = down {
                out.push((x, cy as u32, m));
                if out.len() >= MAX_BULLSEYES {
                    return out;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_picture_is_cut_into_overlapping_tiles() {
        let limits = Limits::default();
        assert_eq!(tiles(1000, 1000, &limits).len(), 1);
        let t = tiles(8000, 8000, &limits);
        assert!(
            t.iter()
                .all(|r| u64::from(r.w) * u64::from(r.h) <= 4_000_000)
        );
        // Every pixel lies in a tile, and neighbours overlap by at least 600 pixels.
        assert_eq!(t.last().map(|r| (r.x + r.w, r.y + r.h)), Some((8000, 8000)));
        let xs: Vec<u32> = t.iter().filter(|r| r.y == 0).map(|r| r.x).collect();
        for pair in xs.windows(2) {
            assert!(pair[0] + 2000 - pair[1] >= 600, "{xs:?}");
        }
        // A long thin picture keeps its tiles under the cap.
        let thin = tiles(100, 1_000_000, &limits);
        assert!(thin.iter().all(|r| r.w == 100 && r.h == 40_000));
        // An A4 page at 300 dpi is read in strips, not in six square tiles.
        let page = tiles(2481, 3508, &limits);
        assert_eq!(page.len(), 3, "{page:?}");
        assert!(
            page.iter()
                .all(|r| u64::from(r.w) * u64::from(r.h) <= 4_000_000)
        );
    }

    #[test]
    fn padding_takes_the_border_color() {
        let dark = pad(&[0, 0, 0, 0], 2, 2, 1);
        assert!(dark.iter().all(|&v| v == 0));
        let light = pad(&[255, 0, 255, 255], 2, 2, 1);
        assert_eq!(light[0], 255);
        assert_eq!(light.len(), 16);
    }
}
