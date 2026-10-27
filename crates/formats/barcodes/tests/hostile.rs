// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pictures that try to make a scan take long or much memory, each against a small twin.
//! Under nextest every test runs in its own process, so the RSS figure is its own.

use std::time::{Duration, Instant};

use barcodes::{Limits, Picture, Pixels, Stop, Worker};

/// The allocator of the Rezzer binary: the memory these tests measure is the memory it
/// ships with.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

/// Maximum resident set size of the process.
#[cfg(unix)]
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

/// No `getrusage` here: the figure is 0, so the work bounds are still checked and only the
/// memory bound is not.
#[cfg(not(unix))]
fn max_rss_bytes() -> u64 {
    0
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

/// The wall-clock stop of these tests: far above any machine, so only a hang fails it. The
/// time limit of a scan is wall clock by design (`Limits::time`), so the work a scan does
/// before it depends on the machine; that the wait for a running call ends at the deadline is
/// shown by the worker's own tests, with calls that block until they are released.
const SAFETY: Duration = Duration::from_secs(60);

/// A scan on the worker with its result, its wall time and the decoder calls it made
/// ([`barcodes::work_steps`], `None` in a release build).
fn scan(
    gray: &[u8],
    w: u32,
    h: u32,
    time: Duration,
) -> (Result<barcodes::Scan, Stop>, Duration, Option<u64>) {
    let before = barcodes::work_steps();
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
    let took = start.elapsed();
    let steps = before.zip(barcodes::work_steps()).map(|(b, a)| a - b);
    (got, took, steps)
}

/// Checks that a scan made at most `max` decoder calls (in a debug build, which counts them).
fn assert_steps(steps: Option<u64>, max: u64) {
    if cfg!(debug_assertions) {
        assert!(
            steps.is_some_and(|n| (1..=max).contains(&n)),
            "{steps:?} decoder calls, at most {max}"
        );
    } else {
        assert_eq!(steps, None);
    }
}

#[test]
fn noise_of_64_mp_is_read_in_tiles_with_bounded_memory_and_calls() {
    let twin = block_noise(1000, 1000, 3);
    let (_, twin_time, _) = scan(&twin, 1000, 1000, Duration::from_secs(3));
    let noise = block_noise(8000, 8000, 3);
    let (got, took, steps) = scan(&noise, 8000, 8000, Duration::from_secs(3));
    let rss = max_rss_bytes();
    eprintln!(
        "twin {twin_time:?}, 64 MP {took:?}, {steps:?} decoder calls, max RSS {} MiB",
        rss >> 20
    );
    match got {
        Ok(s) | Err(Stop::Time(s)) => assert!(s.codes.is_empty(), "{s:?}"),
        other => panic!("{other:?}"),
    }
    // FULL was measured once (2026-10-27, debug build, a 600 s limit): the whole scan of the
    // 64 MP in tiles makes 1,497 decoder calls (10.4 s). The 3 s limit cuts it on this Mac;
    // a machine fast enough to finish it still stays inside the tile walk's own calls.
    const FULL: u64 = 1_497;
    assert_steps(steps, FULL);
    assert!(took < SAFETY, "{took:?}");
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
    let (_, twin_time, _) = scan(&twin, ts, ts, Duration::from_secs(2));
    let (gray, side) = finders(5_000);
    let (got, took, steps) = scan(&gray, side, side, Duration::from_secs(2));
    let rss = max_rss_bytes();
    eprintln!(
        "twin {twin_time:?}, 5,000 finders {took:?}, {steps:?} decoder calls, max RSS {} MiB",
        rss >> 20
    );
    assert!(matches!(got, Err(Stop::Time(_))), "{got:?}");
    // FULL was measured once (2026-10-27, debug build, a 600 s limit): the whole scan makes
    // 9 decoder calls and takes 59 s, so only the time stop ends it within 2 s; the
    // `Stop::Time` above is that stop.
    const FULL: u64 = 9;
    assert_steps(steps, FULL);
    assert!(took < SAFETY, "{took:?}");
    assert!(rss < 400 << 20, "max RSS {} MiB", rss >> 20);
}
