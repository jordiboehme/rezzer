// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Memory bounds in their own test binary: it installs a counting allocator. The counters are
//! per thread, so tests that run side by side do not see each other's allocations.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::{Sheet, every_cell_once, ranges, thread_cpu};
use sheetlayout::{Limits, Pos, Rect, detect, segment};

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

#[test]
fn overlapping_facts_give_few_issues() {
    // 4,000 database ranges of one level, each over the one before.
    let mut sheet = Sheet::new().rows("A1", &["a|b", "1|2"]);
    for k in 0..4_000u32 {
        sheet.facts.push(sheetlayout::Fact {
            rect: Rect {
                top: 0,
                left: 0,
                bottom: 1 + k,
                right: 1,
            },
            kind: sheetlayout::FactKind::DatabaseRange {
                name: format!("r{k}"),
                header: true,
                footer: false,
            },
        });
    }
    let input = sheet.input();
    let start = thread_cpu();
    let (layout, peak) = peak_heap(|| segment(&input, &Limits::default(), &never));
    let used = thread_cpu() - start;
    eprintln!(
        "overlapping facts: {} issues, peak heap {peak} bytes, {used:?} CPU",
        layout.issues.len()
    );
    assert!(used < std::time::Duration::from_secs(2), "{used:?}");
    assert!(peak < 16 * MB, "peak heap {peak}");
    assert!(layout.issues.len() <= 102, "{} issues", layout.issues.len());
    assert!(
        layout
            .issues
            .iter()
            .any(|i| i.what.contains("more ranges overlap"))
    );
}

#[test]
fn dense_block_heap_follows_the_cells() {
    // 1000 by 1000 cells that no cut splits: the corner split runs on all of them.
    let mut sheet = Sheet::new();
    for row in 0..1000u32 {
        for col in 0..1000u32 {
            sheet.put(Pos { row, col }, "1");
        }
    }
    let input = sheet.input();
    let start = thread_cpu();
    let (layout, peak) = peak_heap(|| segment(&input, &Limits::default(), &never));
    let used = thread_cpu() - start;
    let per_cell = peak / 1_000_000;
    eprintln!("dense block: peak heap {peak} bytes ({per_cell} per cell), {used:?} CPU");
    assert!(per_cell <= 64, "{per_cell} bytes per cell");
    assert_eq!(ranges(&layout), ["A1:ALL1000"]);
}

#[test]
fn wide_merged_header_with_long_text_is_bounded() {
    // A1:XFC1 merged with a text of 32,767 bytes, XFD1 a second header cell, numbers below:
    // the paths would be 16,383 copies of the long text.
    let long = "x".repeat(32_767);
    let mut sheet = Sheet::new()
        .cell("A1", &long)
        .cell("XFD1", "Tail")
        .rows("A2", &["1|2"]);
    sheet.merges.push(Rect {
        top: 0,
        left: 0,
        bottom: 0,
        right: 16_382,
    });
    let input = sheet.input();
    let start = thread_cpu();
    let (layout, peak) = peak_heap(|| detect(&input, &Limits::default(), &never));
    let used = thread_cpu() - start;
    eprintln!("wide merged header: peak heap {peak} bytes, {used:?} CPU");
    assert!(used < std::time::Duration::from_secs(2), "{used:?}");
    assert!(peak < 16 * MB, "peak heap {peak}");
    every_cell_once(&input, &layout);
    assert_eq!(layout.regions.len(), 1);
    assert!(layout.regions[0].rest);
    assert!(layout.issues[0].what.contains("header path limit"));
    // A short text over the same width fits: 16,384 paths.
    let mut sheet = Sheet::new()
        .cell("A1", "Head")
        .cell("XFD1", "Tail")
        .rows("A2", &["1|2"]);
    sheet.merges.push(Rect {
        top: 0,
        left: 0,
        bottom: 0,
        right: 16_382,
    });
    let input = sheet.input();
    let start = thread_cpu();
    let (layout, peak) = peak_heap(|| detect(&input, &Limits::default(), &never));
    let used = thread_cpu() - start;
    eprintln!("wide short header: peak heap {peak} bytes, {used:?} CPU");
    assert!(used < std::time::Duration::from_secs(2), "{used:?}");
    assert!(peak < 16 * MB, "peak heap {peak}");
    let r = &layout.regions[0];
    assert_eq!(r.header_paths.len(), 16_384);
    assert_eq!(r.header_paths[16_382], "Head");
    assert_eq!(r.header_paths[16_383], "Tail");
}

#[test]
fn many_merged_single_text_rows_are_bounded() {
    // A header row, 200,000 single-text rows merged over the width, then a number row: H1 and
    // H2 walk past the single-text rows once each.
    let n = 200_000u32;
    let mut sheet = Sheet::styled().rows("A1", &["*a|*b|*c"]);
    for k in 0..n {
        let row = 1 + k;
        sheet.put(Pos { row, col: 0 }, "t");
        sheet.merges.push(Rect {
            top: row,
            left: 0,
            bottom: row,
            right: 2,
        });
    }
    sheet.put(Pos { row: n + 1, col: 0 }, "1");
    sheet.put(Pos { row: n + 1, col: 1 }, "2");
    let input = sheet.input();
    let start = thread_cpu();
    let (layout, peak) = peak_heap(|| detect(&input, &Limits::default(), &never));
    let used = thread_cpu() - start;
    eprintln!("many single-text rows: peak heap {peak} bytes, {used:?} CPU");
    assert!(used < std::time::Duration::from_secs(2), "{used:?}");
    assert!(peak < 64 * MB, "peak heap {peak}");
    let r = &layout.regions[0];
    assert_eq!(r.header_rows, [0]);
    assert_eq!(r.header_paths, ["a", "b", "c"]);
}

#[test]
fn dense_block_detect_heap_follows_the_cells() {
    let mut sheet = Sheet::new();
    sheet.put(Pos { row: 0, col: 0 }, "h");
    for col in 1..1000u32 {
        sheet.put(Pos { row: 0, col }, "h");
    }
    for row in 1..1000u32 {
        for col in 0..1000u32 {
            sheet.put(Pos { row, col }, "1");
        }
    }
    let input = sheet.input();
    let start = thread_cpu();
    let (layout, peak) = peak_heap(|| detect(&input, &Limits::default(), &never));
    let used = thread_cpu() - start;
    let per_cell = peak / 1_000_000;
    eprintln!("dense block detect: peak heap {peak} bytes ({per_cell} per cell), {used:?} CPU");
    assert!(per_cell <= 64, "{per_cell} bytes per cell");
    assert!(used < std::time::Duration::from_secs(4), "{used:?}");
    assert_eq!(layout.regions[0].header_rows, [0]);
    assert_eq!(layout.regions[0].header_paths.len(), 1000);
}
