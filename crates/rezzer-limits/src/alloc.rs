// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The allocator of the Rezzer binary: the system allocator, except that a block that grows
//! to 1 MiB or more, by at least half its size at once, is moved by allocate, copy and free.
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
//! system's realloc, which does them in place on macOS and Linux. On Windows, HeapReAlloc
//! copies a large block at each small step. The policy still forwards small growth to the
//! system on purpose, so the small-step test checks no time there.
//!
//! The move is the same on every system, though the cause was seen on macOS only. On Linux,
//! glibc can grow a large block in place by remapping its pages, and the move gives that up
//! for one copy of each large doubling. It is kept there so the binary behaves the same on
//! every system, and so the memory tests that CI runs on Linux measure the allocator that
//! ships, not a variant of it.

use std::alloc::{GlobalAlloc, Layout, System};

/// New sizes from which a block is moved by allocate, copy and free.
const MOVE_FROM: usize = 1 << 20;

/// Whether a block of `old` bytes that is to hold `new` is moved by hand: from 1 MiB on, and
/// only when it grows by half or more.
fn moved_by_hand(old: usize, new: usize) -> bool {
    new >= MOVE_FROM
        && old
            .checked_add(old / 2)
            .is_some_and(|half_more| new >= half_more)
}

/// The system allocator, with large blocks moved by hand when they grow by half or more.
pub struct Allocator;

// SAFETY: every call goes to `System` with the layout it was given; `realloc` keeps the
// contract of `GlobalAlloc::realloc` (the old block stays valid on failure, the contents up
// to the smaller size are kept).
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if !moved_by_hand(layout.size(), new_size) {
            // SAFETY: forwarded unchanged.
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        // SAFETY: the caller guarantees `new_size`, rounded up to the alignment, does not
        // overflow (the `realloc` contract), so the layout is valid.
        let new = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        // SAFETY: a valid layout of non-zero size.
        let moved = unsafe { System.alloc(new) };
        if !moved.is_null() {
            // SAFETY: both blocks are valid for the smaller of the two sizes and do not
            // overlap; the old block is freed with the layout it was allocated with.
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, moved, layout.size().min(new_size));
                System.dealloc(ptr, layout);
            }
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

    #[test]
    fn growth_in_small_exact_steps_stays_fast() {
        // 2,000 steps of 64 KiB, from 1 MiB to about 126 MiB: copied whole at every step
        // this takes seconds; in place it takes milliseconds.
        let a = Allocator;
        let step = 64 * 1024;
        let start = Instant::now();
        // SAFETY: each realloc gets the layout of the block it was given; the last block is
        // freed with its own layout.
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
        // Windows' own realloc copies large blocks at each small step, and small growth goes
        // to the system on purpose, so the bound would test the OS there.
        let took = start.elapsed();
        if cfg!(not(windows)) {
            assert!(took < Duration::from_secs(2), "{took:?}");
        }
    }
}
