// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The allocator of the Rezzer binary: the system allocator, except that a block that grows
//! to 1 MiB or more is moved by allocate, copy and free.
//!
//! Why: decoders grow their output by reallocating (the Flate decoder of a page picture goes
//! from 4 to 8 to 16 MiB). On macOS the system's realloc of large blocks leaves memory
//! resident that no program structure holds: reading 60 scanned pages kept about 9.5 MiB a
//! page resident while the live heap stayed under 40 MiB, so the process went far past its
//! memory limit. Moving large blocks by hand keeps the resident memory at the live heap.

use std::alloc::{GlobalAlloc, Layout, System};

/// New sizes from which a block is moved by allocate, copy and free.
const MOVE_FROM: usize = 1 << 20;

/// The system allocator, with large blocks moved by hand when they grow or shrink.
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
        if new_size < MOVE_FROM {
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

    #[test]
    fn large_blocks_keep_their_contents_when_they_move() {
        let a = Allocator;
        let layout = Layout::from_size_align(3 << 20, 8).unwrap();
        // SAFETY: valid layouts; each block is freed once with its own layout.
        unsafe {
            let p = a.alloc(layout);
            assert!(!p.is_null());
            for k in 0..layout.size() {
                *p.add(k) = (k % 251) as u8;
            }
            let q = a.realloc(p, layout, 5 << 20);
            assert!(!q.is_null());
            assert!((0..layout.size()).all(|k| *q.add(k) == (k % 251) as u8));
            let grown = Layout::from_size_align(5 << 20, 8).unwrap();
            let r = a.realloc(q, grown, 2 << 20);
            assert!((0..(2 << 20)).all(|k| *r.add(k) == (k % 251) as u8));
            a.dealloc(r, Layout::from_size_align(2 << 20, 8).unwrap());
        }
    }
}
