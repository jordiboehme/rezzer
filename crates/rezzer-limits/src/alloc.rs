// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The allocator of the Rezzer binary: the system allocator, except that outside Linux a block
//! that grows to 1 MiB or more, by at least half its size at once, is moved by allocate, copy
//! and free; on Linux every `realloc` goes to the system and glibc maps every block from 1 MiB
//! on (see the Linux section below).
//!
//! Why: decoders grow their output by reallocating (the Flate decoder of a page picture goes
//! from 4 to 8 to 16 MiB). On macOS the system's realloc of large blocks leaves memory
//! resident that no program structure holds: reading 60 scanned pages kept about 9.5 MiB a
//! page resident while the live heap stayed under 40 MiB, so the process went far past its
//! memory limit. Moving large blocks by hand keeps the resident memory at the live heap.
//!
//! The move has a cost of its own: the old and the new block are both held while the bytes
//! are copied, and on macOS a buffer grown by doubling past 1 MiB can end higher than with the
//! system's realloc (a field name grown to 20 MB took 568 MiB against 302 MiB until it was
//! built at its size). Large buffers whose size is known are best made at that size.
//!
//! Only a growth by half or more is moved: a block grown in small exact steps (a buffer that
//! reserves 64 KiB more each time) would be copied whole at every step, which is quadratic.
//! Growth by doubling, as `Vec` does it, is moved; smaller growth and shrinking go to the
//! system's realloc, which does them in place on macOS. On Windows, HeapReAlloc copies a large
//! block at each small step. The policy still forwards small growth to the system on purpose,
//! so the small-step test checks no time there.
//!
//! # Linux
//!
//! Linux does not move by hand: every `realloc` goes to the system. glibc and musl both keep a
//! large block in its own mapping, grow it in place by remapping its pages (`mremap`) and give
//! the pages back when it is freed, so the move would only add a copy and a moment with both
//! blocks held. With glibc that holds only for blocks above its mapping threshold, which it
//! raises up to 32 MiB after a mapped block is freed; a freed block below the threshold stays
//! in the heap and can stay resident. So on glibc the first allocation through this allocator
//! sets the threshold to 1 MiB once with `mallopt(M_MMAP_THRESHOLD)`, which also stops it from
//! moving: every block from 1 MiB on is its own mapping. musl has no such setting and needs
//! none: its allocator maps large blocks by itself.
//!
//! The fixed threshold starts at 1 MiB, not at glibc's default of 128 KiB, so on glibc a block
//! between 128 KiB and 1 MiB now lives in the heap instead of its own mapping. The figures
//! below show that this costs nothing there: no shape keeps more resident after free, and the page
//! pictures (whose 256 KiB pieces are such blocks) keep less.
//!
//! The Linux release binaries are static musl, so CI measures musl as well as glibc, and a
//! glibc result never decides alone. The policy was kept on both after these figures of
//! 2026-10-27 (GitHub Actions on `ubuntu-latest`, x86_64, `cargo nextest`, debug build), from
//! CI run 40552689712 before the policy (every growth of 1 MiB or more by half moved by hand)
//! and CI run 40559816952 after it. The probe shapes of `tests/linux_rss.rs` give the peak and
//! the resident set after free, in MiB, before and after; the resident set at the start is
//! 2.7 to 2.8 MiB on glibc and 0.9 to 1.0 MiB on musl:
//!
//! | shape | glibc peak | glibc after free | musl peak | musl after free |
//! |---|---|---|---|---|
//! | `a_known_size_buffer` | 40.8 to 31.2 | 2.8 to 2.7 | 38.8 to 29.3 | 1.0 to 0.9 |
//! | `doubling_to_64_mib` | 66.7 to 66.9 | 2.9 to 2.9 | 65.0 to 64.8 | 1.1 to 1.0 |
//! | `many_threads_small_blocks` | 206.4 to 211.4 | 4.0 to 3.9 | 236.1 to 258.2 | 1.1 to 1.0 |
//! | `small_exact_growth` | 128.7 to 128.8 | 2.8 to 2.8 | 127.0 to 126.8 | 1.1 to 1.0 |
//! | `page_pictures_grow_and_go` | 45.7 to 33.7 | 30.0 to 18.0 | 31.7 to 31.8 | 16.1 to 16.2 |
//!
//! `page_pictures_grow_and_go` still holds 15.0 MiB live after free; on musl its start moved
//! from 0.9 to 1.0 MiB, so its 0.1 MiB more after free is the start, not the policy.
//! `many_threads_small_blocks` makes no `realloc` and only blocks of 1 to 4 KiB, which the
//! policy does not touch, so its higher peaks (2.4 % on glibc, 9.4 % on musl) are run to run
//! noise. The wall time of `small_exact_growth` went from 233 to 90 ms on glibc and from 78 to
//! 101 ms on musl. The hostile PDF tests of the binary (`crates/rezzer/tests/hostile.rs`) print
//! their maximum resident set: no line rose by more than 2 MiB or 5 % on either libc. The
//! largest are `hundred_thousand_pages_stop_in_time` (glibc 382 to 383 MiB, musl 425 to 425
//! MiB), `hundred_thousand_pages_stay_under_the_memory_limit` (glibc 305 to 305, musl 303 to
//! 303), `huge_script_is_bounded` (glibc 214 to 215, musl 211 to 211),
//! `pages_with_their_own_resources_are_read_under_the_memory_limit` (glibc 117 to 119, musl
//! 115 to 114) and `objstm_bomb_is_bounded` (glibc 77 to 76, musl 74 to 74). The other five
//! lines are small and rose by 1 MiB at most: on glibc from 22 to 23 MiB
//! (`many_merges_without_values_is_bounded` 23 to 23), on musl from 21 to 22 MiB.

use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(debug_assertions)]
use std::cell::Cell;

/// New sizes from which a block is moved by allocate, copy and free, outside Linux. Linux
/// never moves by hand (see [`MOVES_BY_HAND`]); there glibc maps every block from this same
/// 1 MiB on instead (see `map_large_blocks`).
const MOVE_FROM: usize = 1 << 20;

/// Whether this system moves large growth by hand at all. Linux leaves every `realloc` to the
/// system (see the module docs).
const MOVES_BY_HAND: bool = !cfg!(target_os = "linux");

/// Whether a block of `old` bytes that is to hold `new` is moved by hand: from 1 MiB on, and
/// only when it grows by half or more.
fn moved_by_hand(old: usize, new: usize) -> bool {
    new >= MOVE_FROM
        && old
            .checked_add(old / 2)
            .is_some_and(|half_more| new >= half_more)
}

#[cfg(debug_assertions)]
thread_local! {
    /// The bytes the move copied on this thread. Const-initialized and without a destructor,
    /// so the allocator can use it: it never allocates and never registers a destructor.
    static MOVED: Cell<u64> = const { Cell::new(0) };
}

/// The bytes the move by hand copied on this thread so far, so a test can check which growth
/// is copied. Debug builds only: release builds do not count and give `None`. On Linux, which
/// does not move by hand, the count stays 0.
pub fn moved_bytes() -> Option<u64> {
    #[cfg(debug_assertions)]
    {
        MOVED.try_with(Cell::get).ok()
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// Adds `n` copied bytes to [`MOVED`]. A thread whose locals are gone counts nothing.
#[cfg(debug_assertions)]
fn count_moved(n: usize) {
    let _ = MOVED.try_with(|m| m.set(m.get().saturating_add(n as u64)));
}

/// Sets glibc's mapping threshold to 1 MiB, once, on the first block made through this
/// allocator.
///
/// Safe inside the allocator: it does not allocate (an atomic flag and one `mallopt` call,
/// which takes glibc's arena lock and allocates nothing), cannot panic and keeps no thread
/// state. The swap of the flag lets exactly one thread call `mallopt`. Another thread that
/// sees the flag set before that call is done makes its block under glibc's default, dynamic
/// threshold (128 KiB at start), as before this policy. What matters is the first Rust block
/// of 1 MiB or more, not the first allocation of the process, and the threshold at start maps
/// such a block anyway. Blocks the C library makes do not pass through this allocator.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn map_large_blocks() {
    use std::sync::atomic::{AtomicBool, Ordering};

    /// 1 MiB, the size from which glibc gives every block its own mapping.
    const MAPPED_FROM: libc::c_int = 1 << 20;
    static DONE: AtomicBool = AtomicBool::new(false);

    if DONE.load(Ordering::Relaxed) || DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    // SAFETY: `mallopt` takes two integers and touches only glibc's own malloc settings. The
    // result is not checked: if glibc refuses the value, its own default stays, which is the
    // state before this policy.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, MAPPED_FROM);
    }
}

/// Nothing to set outside glibc (musl has no `mallopt`).
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn map_large_blocks() {}

/// The system allocator, with large blocks moved by hand when they grow by half or more
/// (outside Linux), and on glibc every block from 1 MiB on in its own mapping.
pub struct Allocator;

// SAFETY: every call goes to `System` with the layout it was given; `realloc` keeps the
// contract of `GlobalAlloc::realloc` (the old block stays valid on failure, the contents up
// to the smaller size are kept). `map_large_blocks` neither allocates nor panics.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        map_large_blocks();
        // SAFETY: forwarded unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        map_large_blocks();
        // SAFETY: forwarded unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if !MOVES_BY_HAND || !moved_by_hand(layout.size(), new_size) {
            // SAFETY: forwarded unchanged.
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        // SAFETY: the caller guarantees `new_size`, rounded up to the alignment, does not
        // overflow (the `realloc` contract), so the layout is valid.
        let new = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        // SAFETY: a valid layout of non-zero size.
        let moved = unsafe { System.alloc(new) };
        if !moved.is_null() {
            let kept = layout.size().min(new_size);
            // SAFETY: both blocks are valid for the smaller of the two sizes and do not
            // overlap; the old block is freed with the layout it was allocated with.
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, moved, kept);
                System.dealloc(ptr, layout);
            }
            #[cfg(debug_assertions)]
            count_moved(kept);
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const MIB: usize = 1 << 20;

    fn fill(p: *mut u8, n: usize) {
        for k in 0..n {
            // SAFETY: the caller's block holds `n` bytes.
            unsafe { *p.add(k) = (k % 251) as u8 };
        }
    }

    fn holds(p: *const u8, n: usize) -> bool {
        // SAFETY: the caller's block holds `n` bytes.
        (0..n).all(|k| unsafe { *p.add(k) } == (k % 251) as u8)
    }

    #[test]
    fn large_blocks_keep_their_contents_when_they_move() {
        let a = Allocator;
        let layout = Layout::from_size_align(3 * MIB, 8).unwrap();
        // SAFETY: valid layouts; each block is freed once with its own layout.
        unsafe {
            let p = a.alloc(layout);
            assert!(!p.is_null());
            fill(p, layout.size());
            let q = a.realloc(p, layout, 5 * MIB);
            assert!(!q.is_null());
            assert!(holds(q, layout.size()));
            a.dealloc(q, Layout::from_size_align(5 * MIB, 8).unwrap());
        }
    }

    #[test]
    fn which_growth_is_moved_by_hand() {
        // The 1 MiB edge.
        assert!(!moved_by_hand(MIB / 4, MIB - 1));
        assert!(moved_by_hand(MIB / 4, MIB));
        assert!(moved_by_hand(MIB / 4, MIB + 1));
        // Growth by half or more, not less; shrinking never.
        assert!(moved_by_hand(2 * MIB, 3 * MIB));
        assert!(!moved_by_hand(2 * MIB, 3 * MIB - 1));
        assert!(!moved_by_hand(4 * MIB, 2 * MIB));
        // No overflow at the top.
        assert!(!moved_by_hand(usize::MAX, usize::MAX));
    }

    #[test]
    fn blocks_across_the_1_mib_edge_keep_their_contents() {
        let a = Allocator;
        for new in [MIB - 1, MIB, MIB + 1] {
            let old = Layout::from_size_align(MIB / 4, 16).unwrap();
            // SAFETY: valid layouts; each block is freed once with its own layout.
            unsafe {
                let p = a.alloc(old);
                fill(p, old.size());
                let q = a.realloc(p, old, new);
                assert!(!q.is_null(), "{new}");
                assert!(holds(q, old.size()), "{new}");
                assert_eq!(q as usize % 16, 0);
                a.dealloc(q, Layout::from_size_align(new, 16).unwrap());
            }
        }
    }

    #[test]
    fn a_large_alignment_is_kept() {
        let a = Allocator;
        let old = Layout::from_size_align(MIB, 65_536).unwrap();
        // SAFETY: valid layouts; each block is freed once with its own layout.
        unsafe {
            let p = a.alloc(old);
            assert_eq!(p as usize % 65_536, 0);
            fill(p, old.size());
            let q = a.realloc(p, old, 4 * MIB);
            assert_eq!(q as usize % 65_536, 0);
            assert!(holds(q, old.size()));
            a.dealloc(q, Layout::from_size_align(4 * MIB, 65_536).unwrap());
        }
    }

    #[test]
    fn a_refused_request_leaves_the_old_block() {
        let a = Allocator;
        let old = Layout::from_size_align(2 * MIB, 8).unwrap();
        // SAFETY: valid layouts; the huge size stays under isize::MAX after rounding, as the
        // realloc contract asks; the old block is freed once.
        unsafe {
            let p = a.alloc(old);
            fill(p, old.size());
            let q = a.realloc(p, old, isize::MAX as usize / 2);
            assert!(q.is_null());
            assert!(holds(p, old.size()));
            a.dealloc(p, old);
        }
    }

    #[test]
    fn shrinking_keeps_the_front() {
        let a = Allocator;
        let old = Layout::from_size_align(8 * MIB, 8).unwrap();
        // SAFETY: valid layouts; each block is freed once with its own layout.
        unsafe {
            let p = a.alloc(old);
            fill(p, old.size());
            let q = a.realloc(p, old, 2 * MIB);
            assert!(holds(q, 2 * MIB));
            a.dealloc(q, Layout::from_size_align(2 * MIB, 8).unwrap());
        }
    }

    /// The bytes [`moved_bytes`] counted on this thread while `f` ran: `None` in release
    /// builds, which do not count.
    fn moved_by(f: impl FnOnce()) -> Option<u64> {
        let before = moved_bytes();
        f();
        Some(moved_bytes()? - before?)
    }

    #[test]
    fn a_move_counts_the_bytes_it_copies() {
        let a = Allocator;
        let old = Layout::from_size_align(3 * MIB, 8).unwrap();
        let moved = moved_by(|| {
            // SAFETY: valid layouts; each block is freed once with its own layout.
            unsafe {
                let p = a.alloc(old);
                assert!(!p.is_null());
                let q = a.realloc(p, old, 5 * MIB);
                assert!(!q.is_null());
                a.dealloc(q, Layout::from_size_align(5 * MIB, 8).unwrap());
            }
        });
        // Linux leaves the growth to the system's realloc, so nothing is copied by hand there.
        let copied = if MOVES_BY_HAND { 3 * MIB as u64 } else { 0 };
        assert_eq!(moved, cfg!(debug_assertions).then_some(copied));
    }

    /// Rezzer's move leaves small exact growth to the system's realloc. Whether that realloc
    /// itself is fast (musl's may copy) is measured by the Linux allocator probe, not here.
    #[test]
    fn small_exact_growth_is_never_moved_by_hand() {
        // 2,000 steps of 64 KiB, from 1 MiB to about 126 MiB. None of them grows by half, so
        // the move copies nothing: copied whole at every step this would be quadratic. The
        // count of copied bytes is the check; the time is only a stop for a hang.
        let a = Allocator;
        let step = 64 * 1024;
        let start = Instant::now();
        let moved = moved_by(|| {
            // SAFETY: each realloc gets the layout of the block it was given; the last block
            // is freed with its own layout.
            unsafe {
                let mut layout = Layout::from_size_align(MIB, 8).unwrap();
                let mut p = a.alloc(layout);
                assert!(!p.is_null());
                for _ in 0..2_000 {
                    let new = layout.size() + step;
                    p = a.realloc(p, layout, new);
                    assert!(!p.is_null());
                    layout = Layout::from_size_align(new, 8).unwrap();
                }
                a.dealloc(p, layout);
            }
        });
        assert_eq!(moved, cfg!(debug_assertions).then_some(0));
        // Windows' own realloc copies large blocks at each small step, and small growth goes
        // to the system on purpose, so the stop would test the OS there.
        let took = start.elapsed();
        if cfg!(not(windows)) {
            assert!(took < Duration::from_secs(30), "{took:?}");
        }
    }
}
