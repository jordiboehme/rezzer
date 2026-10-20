// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Memory check in its own test binary: it installs a counting allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use csvsniff::{ColumnKind, analyze, sniff};

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

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

#[test]
fn twenty_megabytes_analyze_in_bounded_memory() {
    let mut text = String::from("id,count,label,price\n");
    let mut i = 0u64;
    while text.len() < 20 * 1024 * 1024 {
        text.push_str(&format!(
            "{i},{},item{},{}.{}\n",
            i % 7,
            i % 13,
            i % 90,
            i % 10
        ));
        i += 1;
    }
    let d = sniff(&text).expect("dialect");
    let base = CURRENT.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let a = analyze(&text, &d, 10_000);
    let extra = PEAK.load(Ordering::Relaxed) - base;
    assert_eq!(a.rows as u64, i);
    assert_eq!(a.columns[0].kind, ColumnKind::Integer);
    assert_eq!(a.columns[2].kind, ColumnKind::Text);
    assert_eq!(a.columns[3].kind, ColumnKind::Decimal { places: 1 });
    // More than 200,000 distinct ids: the uniqueness set gave up.
    assert!(!a.columns[0].unique && !a.columns[0].key_like);
    assert!(a.ragged_rows.is_empty());
    // Bounded: far less than the text itself (20 MB), not a copy of every cell.
    assert!(extra < 8 * 1024 * 1024, "extra heap: {extra} bytes");
}
