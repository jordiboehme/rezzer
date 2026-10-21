// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pictures that try to make a scan take long or much memory, each against a small twin.
//! Under nextest every test runs in its own process, so the RSS figure is its own.

use std::time::{Duration, Instant};

use barcodes::{Limits, Picture, Pixels, Stop, Worker};

/// Maximum resident set size of the process.
fn max_rss_bytes() -> u64 {
    // SAFETY: getrusage fills the struct we pass.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let v = ru.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        v
    } else {
        v * 1024
    }
}

/// Square blocks of noise, `block` pixels a side.
fn block_noise(w: u32, h: u32, block: u32) -> Vec<u8> {
    let mut state = 0x2545_f491u32;
    let cols = w.div_ceil(block);
    let mut blocks = vec![0u8; cols as usize];
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        if y % block == 0 {
            for v in &mut blocks {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *v = if state & 1 == 0 { 0 } else { 255 };
            }
        }
        for x in 0..w {
            out.push(blocks[(x / block) as usize]);
        }
    }
    out
}

fn scan(gray: &[u8], w: u32, h: u32, time: Duration) -> (Result<barcodes::Scan, Stop>, Duration) {
    let start = Instant::now();
    let got = Worker::global().decode(
        &Picture {
            pixels: Pixels::Gray(gray),
            width: w,
            height: h,
        },
        &Limits {
            time,
            ..Limits::default()
        },
    );
    (got, start.elapsed())
}

#[test]
fn noise_of_64_mp_is_read_in_tiles_and_stops_at_its_time() {
    let twin = block_noise(1000, 1000, 3);
    let (_, twin_time) = scan(&twin, 1000, 1000, Duration::from_secs(3));
    let noise = block_noise(8000, 8000, 3);
    let (got, took) = scan(&noise, 8000, 8000, Duration::from_secs(3));
    let rss = max_rss_bytes();
    eprintln!(
        "twin {twin_time:?}, 64 MP {took:?}, max RSS {} MiB",
        rss >> 20
    );
    match got {
        Ok(s) | Err(Stop::Time(s)) => assert!(s.codes.is_empty(), "{s:?}"),
        other => panic!("{other:?}"),
    }
    assert!(took < Duration::from_millis(3_500), "{took:?}");
    // The picture itself (64 MB), a tile in its attempts and rxing's working set for one
    // tile of 4 MP.
    assert!(rss < 400 << 20, "max RSS {} MiB", rss >> 20);
}

/// Finder-like squares on a grid, 4 pixels a module.
fn finders(n: u32) -> (Vec<u8>, u32) {
    let per = f64::from(n).sqrt().ceil() as u32;
    let side = per * 36;
    let mut out = vec![255u8; (side * side) as usize];
    for k in 0..n {
        let (cx, cy) = ((k % per) * 36 + 4, (k / per) * 36 + 4);
        for y in 0..28 {
            for x in 0..28 {
                let ring = x.min(y).min(27 - x).min(27 - y);
                if !(4..8).contains(&ring) {
                    out[((cy + y) * side + cx + x) as usize] = 0;
                }
            }
        }
    }
    (out, side)
}

#[test]
fn five_thousand_finders_stop_at_the_time_limit() {
    let (twin, ts) = finders(50);
    let (_, twin_time) = scan(&twin, ts, ts, Duration::from_secs(2));
    let (gray, side) = finders(5_000);
    let (got, took) = scan(&gray, side, side, Duration::from_secs(2));
    let rss = max_rss_bytes();
    eprintln!(
        "twin {twin_time:?}, 5,000 finders {took:?}, max RSS {} MiB",
        rss >> 20
    );
    assert!(matches!(got, Err(Stop::Time(_))), "{got:?}");
    assert!(took < Duration::from_millis(2_500), "{took:?}");
    assert!(rss < 400 << 20, "max RSS {} MiB", rss >> 20);
}
