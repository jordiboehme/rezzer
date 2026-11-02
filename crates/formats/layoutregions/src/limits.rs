// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Work limits per page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Regions per page.
    pub max_regions: usize,
    /// Levels of the recursive cut.
    pub max_cut_depth: u32,
    /// Alternatives kept per segment.
    pub max_alternatives: usize,
    /// Candidates offered per page.
    pub max_candidates: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_regions: 10_000,
            max_cut_depth: 64,
            max_alternatives: 4,
            max_candidates: 100_000,
        }
    }
}
