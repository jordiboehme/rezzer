// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The bounded lists of the reader and the share of the item limit each one gets.

/// The lists that are bounded one by one, so that running out of one never drops another.
#[derive(Clone, Copy)]
pub(super) enum List {
    /// Cells with content that get a number format, and notes.
    Cells,
    Merges,
    Looks,
    Outline,
    Ranges,
    Pilots,
}

impl List {
    pub(super) const ALL: [List; 6] = [
        List::Cells,
        List::Merges,
        List::Looks,
        List::Outline,
        List::Ranges,
        List::Pilots,
    ];

    pub(super) fn what(self) -> &'static str {
        match self {
            List::Cells => {
                "cells with content for number formats and notes (counted with their repeats)"
            }
            List::Merges => "cells that start a merged range (counted with their repeats)",
            List::Looks => "cells with a look (counted with their repeats)",
            List::Outline => "row group spans",
            List::Ranges => "database ranges",
            List::Pilots => "pivot tables",
        }
    }
}

/// Most database ranges, or pivot tables, in one file.
const MAX_STRUCTURES: u64 = 10_000;

pub(super) struct Share {
    pub(super) used: u64,
    pub(super) limit: u64,
    pub(super) exhausted: bool,
}

impl Share {
    /// The layout lists draw from the one total (`max_items`) in fixed shares: looks 30 %,
    /// merges 20 %, row group spans 5 %, database ranges 2.5 % and pivot tables 2.5 % (those
    /// two also at most 10,000 each). Cells with content (and copied text) get the whole total
    /// less what the layout lists have charged, so a sheet without layout signals keeps all of it.
    pub(super) fn new(list: List, items: u64) -> Share {
        let limit = match list {
            List::Cells => items,
            List::Looks => items / 10 * 3,
            List::Merges => items / 10 * 2,
            List::Outline => items / 20,
            List::Ranges | List::Pilots => (items / 40).min(MAX_STRUCTURES),
        };
        Share {
            used: 0,
            limit,
            exhausted: false,
        }
    }
}
