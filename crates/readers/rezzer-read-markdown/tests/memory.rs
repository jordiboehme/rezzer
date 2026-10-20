// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Memory check in its own test binary: it installs a counting allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Bytes, Completeness, Confidence, Detection, FormatId, LossReason, ReadOptions, Reader, Source,
};
use rezzer_read_markdown::MarkdownReader;

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
fn forty_megabytes_of_inline_markup_stay_well_under_two_gigabytes() {
    // One paragraph of 10 million emphasized words: the parser builds its tree for the
    // whole input before the first event, so the input itself must be bounded.
    let text = format!("# T\n\n{}\n", "*a* ".repeat(10_000_000));
    let src = Source {
        path: None,
        name: "huge.md".into(),
        size: text.len() as u64,
        bytes: Bytes::Owned(Arc::new(text.into_bytes())),
        sha256: "00".repeat(32),
        format: FormatId::Markdown,
        detection: Detection {
            confidence: Confidence::Likely,
            evidence: "md".into(),
        },
        modified: None,
    };
    let budget = Budget::new(Limits::default());
    let base = CURRENT.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let out = MarkdownReader.read(&src, &ReadOptions::default(), &budget);
    let extra = PEAK.load(Ordering::Relaxed) - base;
    eprintln!("peak extra heap: {extra} bytes");
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.starts_with("element limit reached"))
    );
    assert!(extra < 1024 * 1024 * 1024, "extra heap: {extra} bytes");
}
