// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Memory checks in their own test binary: it installs a counting allocator. Tests here run
//! one after the other through a lock, so the peak of one is not seen by another.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Block, Bytes, Confidence, Detection, FormatId, LossReason, ReadOptions, ReadOutcome, Reader,
    Severity, Source,
};
use rezzer_read_csv::CsvReader;

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static SERIAL: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let now = CURRENT.fetch_add(l.size(), Ordering::Relaxed) + l.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        CURRENT.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let now = CURRENT.fetch_add(new, Ordering::Relaxed) + new;
        CURRENT.fetch_sub(l.size(), Ordering::Relaxed);
        PEAK.fetch_max(now, Ordering::Relaxed);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn source(bytes: Vec<u8>) -> Source {
    Source {
        path: None,
        name: "wide.csv".into(),
        size: bytes.len() as u64,
        bytes: Bytes::Owned(Arc::new(bytes)),
        sha256: "00".repeat(32),
        format: FormatId::Csv,
        detection: Detection {
            confidence: Confidence::Likely,
            evidence: "csv".into(),
        },
        modified: None,
    }
}

/// `a,b`, `rows` short data rows and one row of 10,000 commas (10,001 fields).
fn one_wide_row(rows: usize) -> Vec<u8> {
    let mut csv = String::from("a,b\n");
    for i in 0..rows {
        csv.push_str(&format!("{i},{i}\n"));
    }
    csv.push_str(&",".repeat(10_000));
    csv.push('\n');
    csv.into_bytes()
}

/// Reads and returns the outcome with the peak heap above the start, in bytes.
fn read_measured(bytes: Vec<u8>) -> (ReadOutcome, usize) {
    let src = source(bytes);
    let budget = Budget::new(Limits::default());
    let base = CURRENT.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let out = CsvReader.read(&src, &ReadOptions::default(), &budget);
    let extra = PEAK.load(Ordering::Relaxed) - base;
    eprintln!("peak extra heap: {extra} bytes");
    (out, extra)
}

fn table_cells(out: &ReadOutcome) -> (usize, usize, usize) {
    let Some(Block::Section { blocks, .. }) = out.document.body.first() else {
        panic!("no sheet");
    };
    let Some(Block::Table { rows, .. }) = blocks.first() else {
        panic!("no table");
    };
    let cells = rows.iter().map(Vec::len).sum();
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    (cells, rows.len(), width)
}

fn facts_rows(out: &ReadOutcome) -> usize {
    let Some(Block::Section { blocks, .. }) = out.document.body.first() else {
        panic!("no sheet");
    };
    let Some(Block::Table { rows, .. }) = blocks.get(1) else {
        panic!("no facts table");
    };
    rows.len()
}

#[test]
fn one_wide_row_stays_small() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (out, extra) = read_measured(one_wide_row(100));
    let (cells, rows, width) = table_cells(&out);
    // The wide row holds no value: the table spans the two columns with values, and the
    // empty row is no table row.
    assert_eq!(width, 2);
    assert_eq!(rows, 101);
    assert_eq!(cells, 202, "the table is rectangular");
    assert!(cells <= 200_000, "{cells} cells");
    assert!(extra < 64 * 1024 * 1024, "extra heap: {extra} bytes");
    let cut = out
        .document
        .losses
        .iter()
        .find(|l| l.what.contains("columns after column"))
        .expect("column cut loss");
    assert_eq!(cut.what, "9001 columns after column 1000 not shown");
    assert_eq!(cut.why, LossReason::Limit);
    assert_eq!(cut.severity, Severity::Info);
    // The Columns table describes the table's two columns; the empty fields of the wide
    // row belong to no table.
    assert_eq!(facts_rows(&out), 3);
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("not listed in")),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn many_short_rows_and_one_wide_row_stay_inside_the_cell_cap() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // On the old code this shape asked for about 100 million padded cells.
    let (out, extra) = read_measured(one_wide_row(10_000));
    let (cells, rows, width) = table_cells(&out);
    assert!(cells <= 200_000, "{cells} cells");
    assert!(rows * width <= 200_000, "{rows} x {width}");
    assert!(extra < 128 * 1024 * 1024, "extra heap: {extra} bytes");
    let not_shown = out
        .document
        .losses
        .iter()
        .find(|l| l.what.ends_with("not shown"))
        .expect("rows loss");
    // The wide last row would pass the cell cap after padding, so it is the row not shown.
    assert_eq!(not_shown.what, "row 10001 not shown");
    assert_eq!(rows, 10_001);
    assert_eq!(not_shown.severity, Severity::Info);
}
