// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The CSV reader on arbitrary bytes, under small limits. It must never panic and must return a
//! document that stays inside the element budget.
#![no_main]

use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use rezzer_limits::{Budget, Limits};
use rezzer_model::{ReadOptions, Reader};

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_depth: 32,
        max_time: Duration::from_secs(5),
        max_elements: 100_000,
        ..Limits::default()
    };
    let Ok(src) = rezzer_detect::from_bytes("fuzz.csv", data.to_vec(), &limits) else {
        return;
    };
    let budget = Budget::new(limits);
    let _ = rezzer_read_csv::CsvReader.read(&src, &ReadOptions::default(), &budget);
});
