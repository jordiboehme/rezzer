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

/// The lines `kind\t<count>...` for each kind in [`Kind`] order, one count per column (0 for a
/// kind a column lacks), then `total\t<sum>...`: the region count table of a region report.
pub fn count_lines(columns: &[BTreeMap<Kind, usize>]) -> Vec<String> {
    let mut lines: Vec<String> = Kind::ALL
        .iter()
        .map(|k| {
            let counts: String = columns
                .iter()
                .map(|c| format!("\t{}", c.get(k).copied().unwrap_or(0)))
                .collect();
            format!("{}{counts}", k.name())
        })
        .collect();
    let totals: String = columns
        .iter()
        .map(|c| format!("\t{}", c.values().sum::<usize>()))
        .collect();
    lines.push(format!("total{totals}"));
    lines
}

/// The lines of `old` and `new` that differ, by place, each as `old: <line>` and `new: <line>`
/// on two lines (`(none)` where one side has fewer lines).
pub fn moved_lines(old: &[String], new: &[String]) -> Vec<String> {
    (0..old.len().max(new.len()))
        .filter(|&i| old.get(i) != new.get(i))
        .map(|i| {
            format!(
                "old: {}\nnew: {}",
                old.get(i).map_or("(none)", String::as_str),
                new.get(i).map_or("(none)", String::as_str)
            )
        })
        .collect()
}
