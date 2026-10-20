// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Detection on arbitrary bytes, file name and MIME hint. It must never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The first byte picks how many of the following bytes are the file name; the rest is
    // the content. The name and a MIME hint come from the input too.
    let Some((&n, rest)) = data.split_first() else {
        return;
    };
    let n = usize::from(n).min(rest.len());
    let (name, content) = rest.split_at(n);
    let name = String::from_utf8_lossy(name);
    let mime = name.split('/').next_back().map(str::to_owned);
    let _ = rezzer_detect::detect_with_hints(&name, mime.as_deref(), content);
    let _ = rezzer_detect::detect_with_hints(&name, None, content);
});
