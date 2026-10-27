// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bounded reading of barcodes and QR codes in untrusted pictures.
//!
//! A picture comes in as 8-bit gray or RGBA pixels. It is read at its own resolution in
//! overlapping tiles, each tile in a few attempts (gray, inverted, the color channel that
//! splits best, a black and white version, a doubled small picture). A code found is painted
//! over and the attempt runs again, so several codes in one picture are all found. The scan
//! stops when an attempt finds nothing new, at the time limit or at the code cap.
//!
//! Two passes look for different kinds of codes: QR Code, Micro QR Code, Data Matrix, Aztec
//! and PDF417 first, then Code 128, Code 39, EAN-13, UPC-A and ITF. A linear code found inside
//! a two-dimensional one is a false hit and is dropped. MaxiCode, EAN-8 and UPC-E are not
//! read: the first can loop for a very long time on small pictures, the other two give many
//! false hits.
//!
//! The decoder (rxing) cannot be stopped inside a call. [`decode`] runs in the calling thread
//! and checks the time between calls. [`Worker::decode`] hands each call to a worker thread
//! on one shared queue and stops waiting at the time limit: the call that runs then is left
//! to end on its own, and a new thread takes the next call. At most two such calls run at
//! once; past that, calls wait for them.

mod prep;
mod read;
mod work;
mod worker;

#[cfg(feature = "encode")]
pub mod encode;

use std::time::{Duration, Instant};

pub use work::work_steps;
pub use worker::Worker;

/// The pixels of a picture, row by row from the top left.
#[derive(Debug, Clone, Copy)]
pub enum Pixels<'a> {
    /// One byte per pixel, 0 is black.
    Gray(&'a [u8]),
    /// Four bytes per pixel: red, green, blue and alpha, not premultiplied. Transparent
    /// pixels are read as white.
    Rgba(&'a [u8]),
}

/// A picture to scan.
#[derive(Debug, Clone, Copy)]
pub struct Picture<'a> {
    pub pixels: Pixels<'a>,
    pub width: u32,
    pub height: u32,
}

/// The bounds of one scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most pixels a picture may have; a larger one is not scanned.
    pub max_pixels: u64,
    /// The most pixels of one tile. A larger picture is read in overlapping tiles.
    pub tile_pixels: u64,
    /// How far tiles overlap, in pixels: a code up to this size lies wholly in one tile.
    pub overlap: u32,
    /// The most codes kept from one picture.
    pub max_codes: usize,
    /// The most payload bytes kept from one code; a larger payload is only counted.
    pub max_payload: usize,
    /// The time one picture may take.
    pub time: Duration,
    /// Only the first two attempts (gray and inverted): a cheaper scan for renders of whole
    /// pages, where a finer look at the likely codes follows.
    pub quick: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_pixels: 100_000_000,
            tile_pixels: 4_000_000,
            overlap: 600,
            max_codes: 64,
            max_payload: 64 * 1024,
            time: Duration::from_secs(10),
            quick: false,
        }
    }
}

/// The kinds of codes that are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Symbology {
    QrCode,
    MicroQrCode,
    DataMatrix,
    Aztec,
    Pdf417,
    Code128,
    Code39,
    Ean13,
    UpcA,
    Itf,
}

impl Symbology {
    /// The usual name: `QR Code`, `Data Matrix`, `Code 128`.
    pub fn name(self) -> &'static str {
        match self {
            Symbology::QrCode => "QR Code",
            Symbology::MicroQrCode => "Micro QR Code",
            Symbology::DataMatrix => "Data Matrix",
            Symbology::Aztec => "Aztec",
            Symbology::Pdf417 => "PDF417",
            Symbology::Code128 => "Code 128",
            Symbology::Code39 => "Code 39",
            Symbology::Ean13 => "EAN-13",
            Symbology::UpcA => "UPC-A",
            Symbology::Itf => "ITF",
        }
    }

    /// Whether it is a two-dimensional code (a matrix or stacked code).
    pub fn two_d(self) -> bool {
        matches!(
            self,
            Symbology::QrCode
                | Symbology::MicroQrCode
                | Symbology::DataMatrix
                | Symbology::Aztec
                | Symbology::Pdf417
        )
    }
}

/// What a code holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// Valid UTF-8 text without control characters other than tab, line breaks and the
    /// separators GS1 and ISO/IEC 15434 use (EOT, FS, GS, RS).
    Text(String),
    /// Any other bytes.
    Bytes(Vec<u8>),
    /// A payload over [`Limits::max_payload`]: only its size in bytes.
    TooLarge(usize),
}

/// One code read from a picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub symbology: Symbology,
    pub payload: Payload,
    /// The size of the payload in bytes, as the code holds it.
    pub bytes: usize,
    /// About where it lies, in pixels of the picture: left, top, right and bottom. For a
    /// linear code it is the line that was read, a little widened.
    pub bounds: [u32; 4],
}

/// The codes of one picture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub codes: Vec<Code>,
    /// More codes were found than [`Limits::max_codes`]: the rest are not kept.
    pub capped: bool,
}

/// Why a picture was not scanned, or not to the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The picture has more pixels than [`Limits::max_pixels`].
    TooLarge { pixels: u64, max: u64 },
    /// The pixel buffer does not match the width and height.
    BadSize,
    /// The time limit was reached. The codes found until then.
    Time(Scan),
    /// The decoder stopped with an internal error.
    Internal,
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stop::TooLarge { pixels, max } => write!(
                f,
                "the picture has {pixels} pixels, more than the {max} a barcode scan may take"
            ),
            Stop::BadSize => write!(f, "the picture's pixels do not match its size"),
            Stop::Time(_) => write!(f, "barcode scan stopped: time limit"),
            Stop::Internal => write!(f, "the barcode reader stopped with an internal error"),
        }
    }
}

/// Scans `picture` in the calling thread. The time is checked between decoder calls, so one
/// call can run past [`Limits::time`].
pub fn decode(picture: &Picture<'_>, limits: &Limits) -> Result<Scan, Stop> {
    scan(picture, limits, &mut read::Inline)
}

/// Runs one decoder call on a prepared gray picture; the worker or the calling thread.
pub(crate) trait Runner {
    fn run(&mut self, job: read::Job, deadline: Instant) -> Result<Vec<read::Hit>, Stop>;
}

/// The attempts, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attempt {
    Luma,
    Inverted,
    Channel,
    Otsu,
    Doubled,
}

/// The most times one attempt runs again after painting over what it found.
const ROUNDS: usize = 8;

/// The white quiet zone put around each attempt, in pixels: a picture cut tight around a
/// code has none.
const PAD: u32 = 16;

/// How many modules a window around an Aztec finder reaches out on each side: the largest
/// Aztec code has 151 modules.
const WINDOW_MODULES: f32 = 80.0;

/// Looks for Aztec codes around their finders in `tile`, on its gray pixels with the codes
/// found so far painted over: rxing looks for the finder only near the middle of a picture.
/// Adds what it finds to `out`.
fn aztec_windows(
    tile: &prep::Tile,
    out: &mut Scan,
    limits: &Limits,
    deadline: Instant,
    runner: &mut dyn Runner,
    (w, h): (u32, u32),
) -> Result<(), Stop> {
    let r = tile.rect;
    let mut gray = tile.luma.clone();
    for c in &out.codes {
        let b = c.bounds;
        let to = |v: u32, o: u32| i64::from(v) - i64::from(o);
        prep::paint(
            &mut gray,
            r.w,
            r.h,
            [to(b[0], r.x), to(b[1], r.y), to(b[2], r.x), to(b[3], r.y)],
        );
    }
    for (cx, cy, m) in prep::bullseyes(&gray, r.w, r.h) {
        let reach = (WINDOW_MODULES * m).ceil() as u32;
        // A window centred on the finder, as far as the tile allows on each side alike.
        let half = reach.min(cx).min(cy).min(r.w - cx).min(r.h - cy).max(1);
        let (x, y, side) = (cx - half, cy - half, 2 * half);
        let mut window = Vec::with_capacity((side * side) as usize);
        for row in y..y + side {
            let at = (row * r.w + x) as usize;
            window.extend_from_slice(&gray[at..at + side as usize]);
        }
        // A finder at the tile's edge leaves only a small window: pad it out to the reach.
        let pad = PAD.max(reach.saturating_sub(half));
        let job = read::Job {
            gray: prep::pad(&window, side, side, pad),
            width: side + 2 * pad,
            height: side + 2 * pad,
            aztec_only: true,
        };
        work::add(1);
        let hits = runner.run(job, deadline)?;
        let map = |px: f32, py: f32| {
            (
                px - pad as f32 + (r.x + x) as f32,
                py - pad as f32 + (r.y + y) as f32,
            )
        };
        for hit in hits {
            let window = [
                r.x + x,
                r.y + y,
                (r.x + x + side).min(w),
                (r.y + y + side).min(h),
            ];
            let Some(code) = read::code_of(&hit, &map, (w, h), window, limits.max_payload) else {
                continue;
            };
            if out.codes.iter().any(|c| prep::same(c, &code)) {
                continue;
            }
            if out.codes.len() >= limits.max_codes {
                out.capped = true;
                return Ok(());
            }
            out.codes.push(code);
        }
    }
    Ok(())
}

pub(crate) fn scan(
    picture: &Picture<'_>,
    limits: &Limits,
    runner: &mut dyn Runner,
) -> Result<Scan, Stop> {
    let (w, h) = (picture.width, picture.height);
    let pixels = u64::from(w) * u64::from(h);
    if pixels > limits.max_pixels {
        return Err(Stop::TooLarge {
            pixels,
            max: limits.max_pixels,
        });
    }
    let per = match picture.pixels {
        Pixels::Gray(p) => (p, 1u64),
        Pixels::Rgba(p) => (p, 4u64),
    };
    if per.0.len() as u64 != pixels * per.1 {
        return Err(Stop::BadSize);
    }
    let mut out = Scan::default();
    if w == 0 || h == 0 || limits.max_codes == 0 {
        return Ok(out);
    }
    let deadline = Instant::now() + limits.time;
    let small = w.min(h) < 400;
    for tile in prep::tiles(w, h, limits) {
        let base = prep::Tile::cut(picture, tile);
        let mut found_any = false;
        let attempts: &[Attempt] = if limits.quick {
            &[Attempt::Luma, Attempt::Inverted]
        } else {
            &[
                Attempt::Luma,
                Attempt::Inverted,
                Attempt::Channel,
                Attempt::Otsu,
                Attempt::Doubled,
            ]
        };
        for &attempt in attempts {
            let image = match attempt {
                Attempt::Luma => Some(base.luma.clone()),
                Attempt::Inverted => Some(base.luma.iter().map(|v| 255 - v).collect()),
                Attempt::Channel => base.channel(picture),
                Attempt::Otsu => Some(prep::otsu_binary(&base.luma)),
                // Only a small picture is doubled, and only when the result stays a tile.
                Attempt::Doubled => (small
                    && u64::from(tile.w) * u64::from(tile.h) * 4 <= limits.tile_pixels)
                    .then(|| prep::doubled(&base.luma, tile.w, tile.h)),
            };
            let Some(image) = image else {
                continue;
            };
            let scale = if attempt == Attempt::Doubled { 2 } else { 1 };
            let (iw, ih) = (tile.w * scale, tile.h * scale);
            let mut padded = prep::pad(&image, iw, ih, PAD);
            let (pw, ph) = (iw + 2 * PAD, ih + 2 * PAD);
            let mut new = 0;
            for _ in 0..ROUNDS {
                let job = read::Job {
                    gray: padded.clone(),
                    width: pw,
                    height: ph,
                    aztec_only: false,
                };
                work::add(1);
                let hits = match runner.run(job, deadline) {
                    Ok(hits) => hits,
                    Err(Stop::Time(_)) => return Err(Stop::Time(out)),
                    Err(e) => return Err(e),
                };
                let map = |x: f32, y: f32| {
                    (
                        (x - PAD as f32) / scale as f32 + tile.x as f32,
                        (y - PAD as f32) / scale as f32 + tile.y as f32,
                    )
                };
                let mut added = Vec::new();
                for hit in hits {
                    if !hit.symbology.two_d() && !prep::bars_hold(&padded, pw, ph, &hit.points) {
                        continue;
                    }
                    let tile_box = [tile.x, tile.y, tile.x + tile.w, tile.y + tile.h];
                    let Some(code) =
                        read::code_of(&hit, &map, (w, h), tile_box, limits.max_payload)
                    else {
                        continue;
                    };
                    if out.codes.iter().any(|c| prep::same(c, &code))
                        || (!code.symbology.two_d() && prep::inside_2d(&code, &out.codes))
                    {
                        continue;
                    }
                    if out.codes.len() >= limits.max_codes {
                        out.capped = true;
                        return Ok(out);
                    }
                    added.push(code.bounds);
                    out.codes.push(code);
                }
                if added.is_empty() {
                    break;
                }
                new += added.len();
                // Paint over what was found, in the attempt's own pixels, and look again.
                for b in added {
                    let to = |v: u32, origin: u32| {
                        i64::from(v.saturating_sub(origin)) * i64::from(scale) + i64::from(PAD)
                    };
                    prep::paint(
                        &mut padded,
                        pw,
                        ph,
                        [
                            to(b[0], tile.x),
                            to(b[1], tile.y),
                            to(b[2], tile.x),
                            to(b[3], tile.y),
                        ],
                    );
                }
            }
            if found_any && new == 0 {
                break;
            }
            found_any |= new > 0;
        }
        if out.codes.len() >= limits.max_codes {
            break;
        }
        if let Err(stop) = aztec_windows(&base, &mut out, limits, deadline, runner, (w, h)) {
            return Err(match stop {
                Stop::Time(_) => Stop::Time(out),
                e => e,
            });
        }
    }
    // The 1D pass may have found a linear code before the 2D code around it.
    let twod: Vec<Code> = out
        .codes
        .iter()
        .filter(|c| c.symbology.two_d())
        .cloned()
        .collect();
    out.codes
        .retain(|c| c.symbology.two_d() || !prep::inside_2d(c, &twod));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A decoder that gives a QR code and a Code 128 read across its middle, once.
    struct Fake(bool);

    impl Runner for Fake {
        fn run(&mut self, job: read::Job, _: Instant) -> Result<Vec<read::Hit>, Stop> {
            if self.0 || job.aztec_only {
                return Ok(Vec::new());
            }
            self.0 = true;
            let hit = |symbology, text: &str, points: Vec<(f32, f32)>| read::Hit {
                symbology,
                text: text.into(),
                bytes: None,
                points,
            };
            Ok(vec![
                hit(
                    Symbology::QrCode,
                    "https://example.org/q",
                    vec![(40.0, 40.0), (140.0, 40.0), (40.0, 140.0)],
                ),
                hit(Symbology::Code128, "123", vec![(50.0, 90.0), (130.0, 90.0)]),
            ])
        }
    }

    #[test]
    fn a_linear_hit_inside_a_matrix_code_is_dropped() {
        // A white picture: the bars check would drop the fake linear hit anyway, so the
        // pixels show a bar pattern on all rows.
        let (w, h) = (200u32, 200u32);
        let gray: Vec<u8> = (0..w * h)
            .map(|i| if (i % w) / 4 % 2 == 0 { 0 } else { 255 })
            .collect();
        let pic = Picture {
            pixels: Pixels::Gray(&gray),
            width: w,
            height: h,
        };
        let scan = scan(&pic, &Limits::default(), &mut Fake(false)).expect("scan");
        let kinds: Vec<Symbology> = scan.codes.iter().map(|c| c.symbology).collect();
        assert_eq!(kinds, [Symbology::QrCode]);
    }
}
