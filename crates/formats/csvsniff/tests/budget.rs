// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use csvsniff::{analyze, sniff};

/// The uniqueness hashes share one budget of 2,000,000 per file.
#[test]
fn uniqueness_budget_is_shared_across_columns() {
    let cols = 20;
    let mut text = (0..cols)
        .map(|c| format!("c{c}"))
        .collect::<Vec<_>>()
        .join(",");
    text.push('\n');
    for r in 0..120_000u32 {
        let row: Vec<String> = (0..cols).map(|c| format!("r{r}c{c}")).collect();
        text.push_str(&row.join(","));
        text.push('\n');
    }
    let d = sniff(&text).unwrap();
    let a = analyze(&text, &d, 1000);
    let unique = a.columns.iter().filter(|c| c.unique).count();
    assert!(unique > 0 && unique < cols, "unique columns: {unique}");
    // Columns that ran out of budget are not key-like either.
    assert!(a.columns.iter().all(|c| c.unique || !c.key_like));
}
