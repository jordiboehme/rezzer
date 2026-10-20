// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Builds a small package for the part readers: the input cut at NUL bytes fills the parts in
//! order, the rest keep their default. An input that starts with `PK` is the package itself.

use std::io::Write;

use zip::write::{SimpleFileOptions, ZipWriter};

/// Small limits, so that bounded work stays far below libFuzzer's memory limit.
pub const ZIP_LIMITS: zipguard::Limits = zipguard::Limits {
    max_entries: 1_000,
    max_entry_bytes: 4 << 20,
    max_total_bytes: 32 << 20,
    max_ratio: 100,
};

pub fn build(data: &[u8], defaults: &[(&'static str, String)]) -> Vec<u8> {
    if data.starts_with(b"PK") {
        return data.to_vec();
    }
    let mut pieces = data.split(|b| *b == 0);
    let mut w = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, default) in defaults {
        let content = pieces.next().unwrap_or(default.as_bytes());
        if w.start_file(*name, opts).is_err() || w.write_all(content).is_err() {
            return Vec::new();
        }
    }
    w.finish().map(|c| c.into_inner()).unwrap_or_default()
}
