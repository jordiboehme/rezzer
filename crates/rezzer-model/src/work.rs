// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A count of the work done, for the tests of the work bounds. Time depends on the machine
//! (a CI runner can be twice as slow as a laptop); this count is the same everywhere for the
//! same input. It is kept in debug builds only, so a release build pays nothing for it.
//!
//! The count goes up by one for each row the unit scan of [`crate::split_rows`] looks at, and
//! in [`crate::locate_cells`] for each point, each tree node an area is put into or a point
//! looks at, and each ended area taken out of a heap. [`crate::Geometry::add_page`] and
//! [`crate::Geometry::push`] count one step for each page they look at (`add_page` one more
//! for its check of the last page).

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

/// The steps of row splitting, cell locating and page geometry filling done on this thread
/// so far; `None` in a release build, which does not count them. A test reads it before and
/// after a call.
pub fn work_steps() -> Option<u64> {
    #[cfg(debug_assertions)]
    {
        Some(STEPS.with(std::cell::Cell::get))
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}
