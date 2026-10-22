// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The form script analysis of pdfparts on arbitrary text: the fields a script reads and the
//! words for an AF call. Scripts are text, never run. Nothing may panic.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let script = String::from_utf8_lossy(data);
    let _ = pdfparts::script::field_refs(&script);
    if let Some(words) = pdfparts::script::describe(&script) {
        assert!(!words.is_empty());
    }
});
