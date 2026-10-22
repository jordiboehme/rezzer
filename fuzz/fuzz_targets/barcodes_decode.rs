// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The barcode scan on arbitrary pixels, in the calling thread, so a hang shows as a
//! libFuzzer timeout. The first byte picks the width (1 to 256) and gray or RGBA; the rest
//! are the pixels. Nothing may panic.
#![no_main]

use std::time::Duration;

use barcodes::{Limits, Picture, Pixels, decode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&head, pixels)) = data.split_first() else {
        return;
    };
    let rgba = head & 0x80 != 0;
    let width = u32::from(head & 0x7f) * 2 + 1;
    let per = if rgba { 4 } else { 1 };
    let height = (pixels.len() / per) as u32 / width;
    if height == 0 {
        return;
    }
    let used = (width * height) as usize * per;
    let pixels = &pixels[..used];
    let picture = Picture {
        pixels: if rgba {
            Pixels::Rgba(pixels)
        } else {
            Pixels::Gray(pixels)
        },
        width,
        height,
    };
    let limits = Limits {
        time: Duration::from_secs(5),
        ..Limits::default()
    };
    let _ = decode(&picture, &limits);
});
