// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! zipguard on arbitrary bytes: open, then measure and read every entry under small limits.
//! It must never panic and never unpack more than the limits allow.
#![no_main]

use libfuzzer_sys::fuzz_target;
use zipguard::{Archive, Limits};

const LIMITS: Limits = Limits {
    max_entries: 1_000,
    max_entry_bytes: 4 << 20,
    max_total_bytes: 16 << 20,
    max_ratio: 100,
};

fuzz_target!(|data: &[u8]| {
    let Ok(mut zip) = Archive::open(data, LIMITS) else {
        return;
    };
    let names: Vec<String> = zip.names().into_iter().map(str::to_string).collect();
    for name in &names {
        let _ = zip.contains(name);
        let _ = zip.measure(name);
        if let Ok(bytes) = zip.read(name) {
            assert!(bytes.len() as u64 <= LIMITS.max_entry_bytes);
        }
    }
    assert!(zip.unpacked() <= LIMITS.max_total_bytes);
});
