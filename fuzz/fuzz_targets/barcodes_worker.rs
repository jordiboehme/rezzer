// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The barcode scan through the shared worker on structured pictures: a real code written
//! from the input, some of its modules flipped by the input, placed somewhere on a page large
//! enough for the Aztec windows and the tiles. Nothing may panic.
#![no_main]

use std::time::Duration;

use barcodes::encode::{self, Options};
use barcodes::{Limits, Picture, Pixels, Symbology, Worker};
use libfuzzer_sys::fuzz_target;

const KINDS: [Symbology; 6] = [
    Symbology::QrCode,
    Symbology::DataMatrix,
    Symbology::Aztec,
    Symbology::Pdf417,
    Symbology::Code128,
    Symbology::Code39,
];

fuzz_target!(|data: &[u8]| {
    let [kind, px, at, len, rest @ ..] = data else {
        return;
    };
    let len = usize::from(*len % 64).min(rest.len());
    let (text, flips) = rest.split_at(len);
    let text: String = text.iter().map(|b| char::from(b' ' + b % 95)).collect();
    let kind = KINDS[usize::from(*kind) % KINDS.len()];
    let Ok(mut m) = encode::text(kind, &text, &Options::default()) else {
        return;
    };
    for pair in flips.chunks(2).take(64) {
        let k = pair.iter().fold(0usize, |a, &b| a * 256 + usize::from(b)) % m.dark.len().max(1);
        if let Some(d) = m.dark.get_mut(k) {
            *d = !*d;
        }
    }
    let px = u32::from(px % 3) + 1;
    let (gray, w, h) = m.gray(px, 2, 20);
    let side = 320u32;
    if w >= side || h >= side {
        return;
    }
    let (ox, oy) = (u32::from(*at) * (side - w) / 255, u32::from(at.rotate_left(3)) * (side - h) / 255);
    let mut page = vec![255u8; (side * side) as usize];
    for row in 0..h {
        let to = ((oy + row) * side + ox) as usize;
        page[to..to + w as usize].copy_from_slice(&gray[(row * w) as usize..((row + 1) * w) as usize]);
    }
    let limits = Limits {
        time: Duration::from_secs(5),
        tile_pixels: 30_000,
        overlap: 60,
        ..Limits::default()
    };
    let _ = Worker::global().decode(
        &Picture {
            pixels: Pixels::Gray(&page),
            width: side,
            height: side,
        },
        &limits,
    );
});
