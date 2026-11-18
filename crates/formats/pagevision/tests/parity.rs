// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Parity with the region bake-off code.
//!
//! The private tests are opt-in: they need `PAGEVISION_PRIVATE_EVAL` (the local private-eval
//! folder) and `--run-ignored only`. They find their pages through the numbers written there,
//! never through a page name in this file.

use std::path::{Path, PathBuf};

use pagevision::ink::{components, flatten, ink_mask};

fn private_eval() -> PathBuf {
    std::env::var_os("PAGEVISION_PRIVATE_EVAL")
        .map(PathBuf::from)
        .expect("set PAGEVISION_PRIVATE_EVAL to the private-eval folder")
}

fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
#[ignore = "needs PAGEVISION_PRIVATE_EVAL"]
fn ink_matches_bakeoff_on_private_pages() {
    let eval = private_eval();
    // Written by the local script parity/ink_counts.py from bakeoff-scripts/common.py.
    let counts = read_json(&eval.join("parity/ink_counts.json"));
    let counts = counts.as_object().expect("ink_counts.json is an object");
    assert!(!counts.is_empty(), "ink_counts.json lists no page");
    let mut failures = Vec::new();
    for (n, (page, want)) in counts.iter().enumerate() {
        let png = std::fs::read(eval.join("pages").join(format!("{page}.png"))).unwrap();
        let ink = ink_mask(&flatten(&png).unwrap());
        let (_, comps) = components(&ink, 3);
        let order: u64 = comps
            .iter()
            .map(|c| u64::from(c.label) * (u64::from(c.x0) + 7 * u64::from(c.y0)))
            .sum();
        let got = (ink.count() as u64, comps.len() as u64, order);
        let want = (
            want["ink"].as_u64().unwrap(),
            want["components"].as_u64().unwrap(),
            want["order"].as_u64().unwrap(),
        );
        if got != want {
            // The page index only, so no private name lands in a log.
            failures.push(format!("page {n}: got {got:?}, want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
