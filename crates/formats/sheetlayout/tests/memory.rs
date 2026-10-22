// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Memory bounds in their own test binary: it installs a counting allocator. The counters are
//! per thread, so tests that run side by side do not see each other's allocations.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::{Sheet, every_cell_once, ranges, thread_cpu};
use sheetlayout::{Limits, Pos, Rect, segment};

thread_local! {
    static CURRENT: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn add(n: isize) {
    let _ = CURRENT.try_with(|c| {
        let now = c.get() + n;
        c.set(now);
        let _ = PEAK.try_with(|p| p.set(p.get().max(now)));
    });
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        add(l.size() as isize);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        add(-(l.size() as isize));
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        add(new as isize - l.size() as isize);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// The live heap peak above the level at the call, in bytes, while `f` runs.
fn peak_heap<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = CURRENT.with(|c| c.get());
    PEAK.with(|p| p.set(base));
    let out = f();
    let peak = PEAK.with(|p| p.get());
    (out, (peak - base).max(0) as usize)
}

fn never() -> bool {
    false
}

const MB: usize = 1024 * 1024;

#[test]
fn far_cells_do_not_allocate() {
    let mut sheet = Sheet::new().cell("A1", "near");
    sheet.put(
        Pos {
            row: 1_048_575,
            col: 16_383,
        },
        "far",
    );
    let input = sheet.input();
    let (layout, peak) = peak_heap(|| segment(&input, &Limits::default(), &never));
    eprintln!("far cells: peak heap {peak} bytes");
    assert!(peak < 16 * MB, "peak heap {peak}");
    assert_eq!(ranges(&layout), ["A1", "XFD1048576"]);
    every_cell_once(&input, &layout);
}

#[test]
fn huge_merge_is_counted_by_intervals() {
    let mut sheet = Sheet::new().cell("A1", "everything");
    sheet.merges.push(Rect {
        top: 0,
        left: 0,
        bottom: 1_048_575,
        right: 16_383,
    });
    for i in 0..50_000u32 {
        let row = 2 + 2 * (i / 100);
        let col = 2 * (i % 100);
        sheet.merges.push(Rect {
            top: row,
            left: col,
            bottom: row,
            right: col + 1,
        });
    }
    let input = sheet.input();
    let start = thread_cpu();
    let (layout, peak) = peak_heap(|| segment(&input, &Limits::default(), &never));
    let used = thread_cpu() - start;
    eprintln!("huge merge: peak heap {peak} bytes, {used:?} CPU");
    assert!(used < std::time::Duration::from_secs(2), "{used:?}");
    assert!(peak < 64 * MB, "peak heap {peak}");
    assert_eq!(ranges(&layout), ["A1:XFD1048576"]);
    every_cell_once(&input, &layout);
}
