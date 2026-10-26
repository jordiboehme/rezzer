// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Binary plist, tag and Zone.Identifier reading on arbitrary bytes. It must never panic and
//! stay within its caps.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(strings) = fileattrs::string_array(data) {
        assert!(strings.len() <= fileattrs::MAX_ITEMS);
        assert!(strings.iter().map(String::len).sum::<usize>() <= fileattrs::MAX_TEXT_BYTES);
    }
    let _ = fileattrs::finder_tags(data);
    if let Some(text) = fileattrs::zone_text(data) {
        let _ = fileattrs::zone_host_url(&text);
        let _ = fileattrs::xdg_tags(&text);
    }
});
