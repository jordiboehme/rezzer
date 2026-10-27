// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Memory and work bounds of the row-range split, in their own test binary: it installs a
//! counting allocator. The counters are per thread, so tests that run side by side do not
//! see each other's allocations; work is counted in steps ([`rezzer_model::work_steps`]).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell as StdCell;
use std::time::{Duration, Instant};

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

/// The wall-clock stop of the work-bound tests: far above any machine, so only a hang fails
/// it. The work itself is bounded by counted steps.
const SAFETY: Duration = Duration::from_secs(60);

/// Runs `f` and gives its result, the steps of work it did ([`rezzer_model::work_steps`],
/// `None` in a release build) and its wall time.
fn counted<T>(f: impl FnOnce() -> T) -> (T, Option<u64>, Duration) {
    let before = rezzer_model::work_steps();
    let start = Instant::now();
    let out = f();
    let took = start.elapsed();
    let steps = before.zip(rezzer_model::work_steps()).map(|(b, a)| a - b);
    (out, steps, took)
}

/// Checks a work bound: at most the `measured` steps with half of them again as margin (in
/// a debug build, which counts them), and the wall time under [`SAFETY`]. Each `measured`
/// was taken once (2026-10-27, debug build, `--no-capture`): the count is the same on every
/// machine, and a growth past the margin is a change in the work, not a slow machine.
fn assert_work(what: &str, steps: Option<u64>, measured: u64, took: Duration) {
    let max = measured + measured / 2;
    eprintln!("{what}: {steps:?} steps (bound {max}) in {took:?}");
    if cfg!(debug_assertions) {
        assert!(
            steps.is_some_and(|n| n <= max),
            "{what}: {steps:?} steps of work, bound {max}"
        );
    } else {
        assert_eq!(steps, None);
    }
    assert!(took < SAFETY, "{what}: took {took:?}");
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
        total_columns: Vec::new(),
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
    // 5,000 times (3,000,000 cells); with half the part for the data it takes 29 ranges.
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
    assert!(ranges.len() <= 30, "{} ranges", ranges.len());
    assert!(
        cells <= 20 * input_cells,
        "{cells} cells from {input_cells}"
    );
    assert!(peak < 64 * MB, "peak heap {peak}");
}

#[test]
fn band_just_below_the_part_does_not_multiply_ranges() {
    // The re-review's shape: a one-row band of 237 columns just below the part size (each
    // cell 26 path chars, 10 text chars and 6: ceil((10 + 237 * 42) / 4) = 2,491 tokens)
    // over 5,000 data rows of 11 tokens. With the fixed part counted in full that was one
    // range per row (1,190,000 cloned cells); with half the part for the data, 113 rows fit
    // a range: 45 ranges.
    let cols = 237usize;
    let path = |c: usize| format!("Population estimate / {c:04}");
    let band: Vec<Cell> = (0..cols)
        .map(|c| cell(&format!("{c:010}"), format!("{}1", column_letter(c))))
        .collect();
    let mut rows = vec![band];
    rows.extend((2..5002).map(|r| vec![cell("ab", format!("A{r}"))]));
    let input_cells: usize = rows.iter().map(Vec::len).sum();
    let t = Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: Some(Box::new(layout(
            vec![1],
            (0..cols).map(path).collect(),
            Vec::new(),
        ))),
    };
    let (ranges, peak) = peak_heap(|| split_rows(&[], &t, &[], 2_500));
    eprintln!(
        "band below the part: {} ranges, peak heap {peak} bytes",
        ranges.len()
    );
    let cells: usize = ranges
        .iter()
        .map(|r| match &r.blocks[r.table.unwrap()] {
            Block::Table { rows, .. } => rows.iter().map(Vec::len).sum::<usize>(),
            _ => 0,
        })
        .sum();
    assert_eq!(
        ranges.len(),
        5_000usize.div_ceil(113),
        "{} ranges",
        ranges.len()
    );
    assert!(cells <= 4 * input_cells, "{cells} cells from {input_cells}");
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
    let (ranges, steps, took) = counted(|| split_rows(&[], &t, &[], 2_500));
    eprintln!("{n} group rows: {} ranges", ranges.len());
    // All group rows chain into one unit, which is one range of its own.
    assert_eq!(ranges.len(), 1);
    assert_eq!((ranges[0].first, ranges[0].last), (2, n + 1));
    // Measured once (2026-10-27): 200,000, one step for each row. The quadratic scan
    // it replaced would take about 2 x 10^10 steps here.
    assert_work("group rows", steps, 200_000, took);
}

#[test]
fn locate_cells_is_bounded_with_many_overlapping_areas() {
    use rezzer_model::{CellArea, locate_cells};
    // A damaged file: 10,000 one-column ranges over a million rows, one of them overlapped
    // by another range, and a million cells right of them all that no range holds, plus a
    // few that one does. A search that walks the open ranges left of each cell would take
    // 10 billion steps.
    let rows = 1_000_000u32;
    let mut areas: Vec<CellArea> = (0..10_000u32)
        .map(|col| CellArea {
            top: 0,
            left: col,
            bottom: rows - 1,
            right: col,
        })
        .collect();
    areas.push(CellArea {
        top: 0,
        left: 0,
        bottom: 9,
        right: 1,
    });
    let mut points: Vec<(u32, u32)> = (0..rows).map(|row| (row, 20_000)).collect();
    points.extend([(5, 0), (5, 1), (500, 7_000)]);
    let ((found, peak), steps, took) = counted(|| peak_heap(|| locate_cells(&areas, &points)));
    eprintln!("locate_cells: peak heap {} MB", peak >> 20);
    // Measured once (2026-10-27): 1,277,303, about one step for each point and a few for
    // each area; the 10 billion steps of a search through the open ranges would blow it.
    assert_work("locate_cells", steps, 1_277_303, took);
    assert!(peak < 256 << 20, "peak heap {peak} bytes");
    let n = rows as usize;
    assert!(found[..n].iter().all(Option::is_none));
    // The small range A1:B10 is inside the one-column ranges A and B and holds their cells
    // there; elsewhere the one-column range holds them.
    assert_eq!(found[n..], [Some(10_000), Some(10_000), Some(7_000)]);
}
