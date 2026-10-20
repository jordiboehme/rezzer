// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dialect sniffing and analysis of arbitrary text (at most 1,000 rows for the derived check).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(dialect) = csvsniff::sniff(&text) {
        let _ = csvsniff::analyze(&text, &dialect, 1_000);
    }
});
