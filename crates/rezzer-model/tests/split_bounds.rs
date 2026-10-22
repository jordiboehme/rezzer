// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Memory and work bounds of the row-range split, in their own test binary: it installs a
//! counting allocator. The counters are per thread, so tests that run side by side do not
//! see each other's allocations; work is measured as thread CPU time.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell as StdCell;
use std::time::Duration;

use rezzer_model::{
    Anchor, Block, Cell, Certainty, Inline, LayoutKind, RowKind, RowNote, TableLayout,
    column_letter, split_rows,
};

thread_local! {
    static CURRENT: StdCell<isize> = const { StdCell::new(0) };
    static PEAK: StdCell<isize> = const { StdCell::new(0) };
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

/// The CPU time the calling thread has used: work bounds do not depend on a busy machine.
#[cfg(unix)]
fn thread_cpu() -> Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime fills the timespec we pass.
    let r = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    assert_eq!(r, 0, "clock_gettime");
    Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

#[cfg(not(unix))]
fn thread_cpu() -> Duration {
    Duration::ZERO
}

const MB: usize = 1024 * 1024;

fn cell(text: &str, at: String) -> Cell {
    let mut c = Cell::new(vec![Inline::plain(text)]);
    c.anchor = Some(Anchor(format!("S!{at}")));
    c
}

fn layout(band: Vec<u32>, paths: Vec<String>, rows: Vec<RowNote>) -> TableLayout {
    TableLayout {
        kind: LayoutKind::Table,
        key: "S!A1".into(),
        range: "A1:GR200000".into(),
        data: None,
        header_rows: band,
        header_paths: paths,
        label_cols: 0,
        certainty: Certainty::Rule,
        found_by: Vec::new(),
        unsure: Vec::new(),
        rows,
        hidden_header: false,
    }
}

#[test]
fn wide_band_does_not_multiply_ranges() {
    // The review's shape, with a band above the part size: 200 columns by 3 band rows over
    // 5,000 one-cell data rows (5,600 input cells). One range per row would clone the band
    // 5,000 times (3,000,000 cells); with its own budget the data takes 15 ranges.
    let cols = 200usize;
    let band_row = |r: u32| -> Vec<Cell> {
        (0..cols)
            .map(|c| cell(&format!("Column {c}"), format!("{}{r}", column_letter(c))))
            .collect()
    };
    let mut rows = vec![band_row(1), band_row(2), band_row(3)];
    rows.extend((4..5004).map(|r| vec![cell("a", format!("A{r}"))]));
    let paths = (0..cols).map(|c| format!("Column {c}")).collect();
    let input_cells: usize = rows.iter().map(Vec::len).sum();
    let t = Block::Table {
        caption: None,
        header_rows: 3,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: Some(Box::new(layout(vec![1, 2, 3], paths, Vec::new()))),
    };
    let (ranges, peak) = peak_heap(|| split_rows(&[], &t, &[], 2_500));
    eprintln!("wide band: {} ranges, peak heap {peak} bytes", ranges.len());
    let cells: usize = ranges
        .iter()
        .map(|r| match &r.blocks[r.table.unwrap()] {
            Block::Table { rows, .. } => rows.iter().map(Vec::len).sum::<usize>(),
            _ => 0,
        })
        .sum();
    assert!(ranges.len() <= 20, "{} ranges", ranges.len());
    assert!(
        cells <= 20 * input_cells,
        "{cells} cells from {input_cells}"
    );
    assert!(peak < 64 * MB, "peak heap {peak}");
}

#[test]
fn many_group_rows_are_linear() {
    // 200,000 group rows in a row: the group scan visits each row a bounded number of
    // times (it was quadratic: 50,000 rows took 4 s).
    let n = 200_000u32;
    let rows: Vec<Vec<Cell>> = std::iter::once(vec![cell("Name", "A1".into())])
        .chain((2..n + 2).map(|r| vec![cell("G", format!("A{r}"))]))
        .collect();
    let notes = (2..n + 2)
        .map(|row| RowNote {
            row,
            kind: RowKind::Group,
            level: None,
            parent: None,
            why: String::new(),
        })
        .collect();
    let t = Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: Some(Box::new(layout(vec![1], vec!["Name".into()], notes))),
    };
    let start = thread_cpu();
    let ranges = split_rows(&[], &t, &[], 2_500);
    let used = thread_cpu() - start;
    eprintln!("{n} group rows: {} ranges in {used:?}", ranges.len());
    // All group rows chain into one unit, which is one range of its own.
    assert_eq!(ranges.len(), 1);
    assert_eq!((ranges[0].first, ranges[0].last), (2, n + 1));
    assert!(used < Duration::from_secs(2), "{used:?}");
}
