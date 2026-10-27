// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A probe of the resident memory that Rezzer's allocator leaves on Linux, under glibc and
//! under musl. It decides nothing by itself: each shape prints one line
//!
//! `linux_rss <shape>: peak <MiB> MiB, resident after free <MiB> MiB, start <MiB> MiB`
//!
//! (some shapes add fields), and the numbers of a glibc and a musl run are compared by hand
//! before the Linux `realloc` policy of `src/alloc.rs` changes. `start` is the resident set
//! when the shape begins: a static musl binary and a glibc one start from different levels,
//! so the figures are compared above it. The only checks are a 512 MiB safety bound on the
//! peak and, for `small_exact_growth`, a 60 s limit on its wall time. That limit is checked
//! afterwards; a run that hangs is ended by nextest after 2 minutes (`.config/nextest.toml`).
//!
//! Each shape is its own test, so nextest runs it in its own process and the process figures
//! are the figures of that shape. Under `cargo test` the shapes share one process: the lines
//! say so and the bound is not checked.
//!
//! All figures come from `/proc/self/status` with std only: `VmHWM` (the peak resident set,
//! the figure `getrusage` gives as `ru_maxrss`) and `VmRSS` (the resident set now), both in
//! kB. That file is the kernel's, so the probe reads the same figures under glibc and musl and
//! calls nothing of either C library.
//!
//! Every buffer is filled with a non-zero byte: a page that is only reserved, or zeroed by a
//! fresh mapping, is not resident and would measure nothing.
//!
//! Note for the policy step: if Linux later forwards `realloc` to `System`, the unit test
//! `a_move_counts_the_bytes_it_copies` in `src/alloc.rs` (which expects 3 MiB moved in debug
//! builds) needs a Linux cfg.

#![cfg(target_os = "linux")]

use std::hint::black_box;
use std::thread;
use std::time::{Duration, Instant};

#[global_allocator]
static A: rezzer_limits::Allocator = rezzer_limits::Allocator;

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

/// The peak every shape stays under.
const BOUND: u64 = 512 * MIB as u64;

/// A byte that is not zero, so every written page is resident.
const FILL: u8 = 0xA5;

/// One `kB` figure of `/proc/self/status`, in bytes.
fn status_bytes(key: &str) -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    let line = status
        .lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .unwrap_or_else(|| panic!("{key} in /proc/self/status"));
    let kb: u64 = line
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("{key}: {e}"));
    kb * 1024
}

/// The resident set now, in bytes.
fn resident() -> u64 {
    status_bytes("VmRSS")
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / MIB as f64
}

/// The printed line of one shape.
struct Line<'a> {
    shape: &'a str,
    /// The resident set when the shape began.
    start: u64,
    /// Text after the resident figure, such as what is still live.
    after: String,
    /// Text at the end of the line.
    extra: String,
}

impl<'a> Line<'a> {
    fn new(shape: &'a str, start: u64) -> Self {
        Line {
            shape,
            start,
            after: String::new(),
            extra: String::new(),
        }
    }

    /// Prints the line and checks the bound when the shape has its own process.
    fn report(self) {
        let peak = status_bytes("VmHWM");
        let now = resident();
        let own = std::env::var_os("NEXTEST").is_some();
        let shared = if own {
            ""
        } else {
            " (shared process, not this shape alone: run under nextest)"
        };
        eprintln!(
            "linux_rss {}: peak {:.1} MiB, resident after free {:.1} MiB{}, start {:.1} MiB{}{shared}",
            self.shape,
            mib(peak),
            mib(now),
            self.after,
            mib(self.start),
            self.extra,
        );
        if own {
            assert!(peak < BOUND, "{}: peak {:.1} MiB", self.shape, mib(peak));
        }
    }
}

/// A `Vec<u8>` grown by doubling to 64 MiB, then dropped.
#[test]
fn doubling_to_64_mib() {
    let start = resident();
    let chunk = vec![FILL; 64 * KIB];
    let mut v = Vec::new();
    while v.len() < 64 * MIB {
        v.extend_from_slice(&chunk);
    }
    black_box(&v);
    drop(v);
    Line::new("doubling_to_64_mib", start).report();
}

/// 60 rounds of a page picture: a buffer grown from 4 to 8 to 16 MiB, then cut to its first
/// 256 KiB, which are kept; the rest is freed. The 60 kept pieces (15 MiB) are still live
/// when the resident memory is read, and the line says so next to that figure.
#[test]
fn page_pictures_grow_and_go() {
    let start = resident();
    let mut kept = Vec::with_capacity(60);
    for _ in 0..60 {
        let mut buf = Vec::with_capacity(4 * MIB);
        for size in [4 * MIB, 8 * MIB, 16 * MIB] {
            buf.reserve_exact(size - buf.len());
            buf.resize(size, FILL);
            black_box(&buf);
        }
        buf.truncate(256 * KIB);
        buf.shrink_to_fit();
        kept.push(buf);
    }
    black_box(&kept);
    let live: usize = kept.iter().map(Vec::capacity).sum();
    let mut line = Line::new("page_pictures_grow_and_go", start);
    line.after = format!(" (with {:.1} MiB live)", mib(live as u64));
    line.report();
    drop(kept);
}

/// A 20 MB buffer made at its size, then grown by a half to 30 MB, then dropped.
#[test]
fn a_known_size_buffer() {
    let start = resident();
    let mut v = Vec::with_capacity(20_000_000);
    v.resize(20_000_000, FILL);
    black_box(&v);
    v.reserve_exact(10_000_000);
    v.resize(30_000_000, FILL);
    black_box(&v);
    drop(v);
    Line::new("a_known_size_buffer", start).report();
}

/// 8 threads with 12,500 blocks of 1 to 4 KiB each (100,000 blocks, about 244 MiB, in all),
/// all held until the thread's last step, then freed. The threads are joined before the
/// resident memory is read, so it shows what the per-thread arenas keep.
#[test]
fn many_threads_small_blocks() {
    let start = resident();
    let threads: Vec<_> = (0..8)
        .map(|t| {
            thread::spawn(move || {
                let blocks: Vec<Vec<u8>> = (0..12_500)
                    .map(|i| vec![FILL; KIB + (i * 7 + t * 13) % (3 * KIB + 1)])
                    .collect();
                black_box(&blocks);
                drop(blocks);
            })
        })
        .collect();
    for t in threads {
        t.join().expect("a probe thread panicked");
    }
    Line::new("many_threads_small_blocks", start).report();
}

/// A `Vec<u8>` of 1 MiB grown in 2,000 exact steps of 64 KiB (to about 126 MiB), then
/// dropped: the shape of `small_exact_growth_is_never_moved_by_hand` in `src/alloc.rs`. Rezzer
/// leaves these steps to the system's realloc; a realloc that copies at every step shows up
/// here as a long wall time and a higher peak. The 60 s limit is checked afterwards; nextest
/// ends a run that hangs after 2 minutes.
#[test]
fn small_exact_growth() {
    let start = resident();
    let step = 64 * KIB;
    let chunk = vec![FILL; step];
    let began = Instant::now();
    let mut v = Vec::with_capacity(MIB);
    v.resize(MIB, FILL);
    for _ in 0..2_000 {
        v.reserve_exact(step);
        v.extend_from_slice(&chunk);
    }
    black_box(&v);
    drop(v);
    let took = began.elapsed();
    let mut line = Line::new("small_exact_growth", start);
    line.extra = format!(", wall {} ms", took.as_millis());
    line.report();
    assert!(
        took < Duration::from_secs(60),
        "small_exact_growth: {took:?}"
    );
}
