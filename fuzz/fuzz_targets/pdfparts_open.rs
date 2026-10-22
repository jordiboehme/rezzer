// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! pdfparts on arbitrary bytes: open under small limits, then read every part (metadata,
//! pages, outline, links, annotations, tags, attachments, layers, threads, form, scripts,
//! signatures and revisions). Nothing may panic, and no attachment may come back larger
//! than a stream may unpack to.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pdfparts::{Limits, Pdf};

const LIMITS: Limits = Limits {
    max_depth: 64,
    max_items: 100_000,
    max_stream_bytes: 4 << 20,
    max_total_bytes: 16 << 20,
    max_ratio: 100,
    max_revisions: 16,
    max_object_bytes: 64 << 20,
    max_script_bytes: 1 << 20,
    max_script_total_bytes: 4 << 20,
};

fuzz_target!(|data: &[u8]| {
    let never = || false;
    // The last byte picks a password, so the encrypted seeds are opened both ways.
    let password = (data.last() == Some(&b'v')).then_some("view");
    let Ok(pdf) = Pdf::open(data, password, &LIMITS, &never) else {
        return;
    };
    let _ = pdf.info();
    let _ = pdf.language();
    let _ = pdf.version();
    let _ = pdf.tagged();
    let _ = pdf.xmp();
    let _ = pdf.encryption();
    let pages = pdf.pages(&never);
    if pdf.struct_tree().is_some() {
        for page in &pages {
            let _ = pdf.role_of(page.number, 0);
        }
    }
    let _ = pdf.outline();
    let _ = pdf.named_destinations();
    let _ = pdf.links();
    let _ = pdf.annotations();
    let _ = pdf.widget_boxes();
    let _ = pdf.layers();
    let _ = pdf.threads();
    let _ = pdf.form();
    let _ = pdf.scripts();
    let _ = pdf.signatures();
    let _ = pdf.revisions(&never);
    for i in 0..pdf.embedded_files().len() {
        let _ = pdf.embedded_file_measured(i);
        if let Ok(bytes) = pdf.embedded_file_bytes(i) {
            assert!(bytes.len() as u64 <= LIMITS.max_stream_bytes);
        }
    }
    let _ = pdf.issues();
    assert!(pdf.unpacked() <= LIMITS.max_total_bytes);
});
