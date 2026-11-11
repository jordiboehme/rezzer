// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A count of the work done, for the tests of the work bounds. Time depends on the machine;
//! this count is the same everywhere for the same input. It is kept in debug builds only, so a
//! release build pays nothing for it, as the region engine's (`layoutregions::work_steps`).
//!
//! The count goes up where a hostile file once made the work grow faster than its text: the
//! runs the layout lays out, the lines and indexes the table detectors build, the runs a
//! header band or a grid looks at, the dates a date label is checked against, the cells of a
//! tagged row, the bytes a picture is known by and the steps that place pictures between
//! blocks.

#[cfg(debug_assertions)]
thread_local! {
    static STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Counts `n` steps of work on this thread.
#[inline]
pub(crate) fn add(n: u64) {
    #[cfg(debug_assertions)]
    STEPS.with(|s| s.set(s.get().saturating_add(n)));
    #[cfg(not(debug_assertions))]
    let _ = n;
}

/// The steps of work this reader, its region engine and the code decoder (its calls, counted
/// on the thread that scans) did on this thread so far; `None` in a release build, which does
/// not count them. A test reads it before and after a read.
pub fn work_steps() -> Option<u64> {
    #[cfg(debug_assertions)]
    {
        let own = STEPS.with(std::cell::Cell::get);
        layoutregions::work_steps()
            .zip(barcodes::work_steps())
            .map(|(e, b)| e.saturating_add(b).saturating_add(own))
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}
