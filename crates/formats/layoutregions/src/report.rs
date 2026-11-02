// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use crate::region::{Kind, Region};

/// How many regions there are of each kind, in [`Kind`] order.
pub fn count_by_kind<'a>(regions: impl IntoIterator<Item = &'a Region>) -> BTreeMap<Kind, usize> {
    let mut counts = BTreeMap::new();
    for r in regions {
        crate::work::add(1);
        *counts.entry(r.kind).or_insert(0) += 1;
    }
    counts
}
