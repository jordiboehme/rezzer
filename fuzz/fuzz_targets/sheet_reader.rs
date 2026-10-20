// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The spreadsheet reader on arbitrary bytes as XLSX, XLSM, XLSB or ODS, under small limits.
//! The document is enriched and rendered as the pipeline does. Nothing may panic.
#![no_main]

use std::sync::Arc;
use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use rezzer_limits::{Budget, Limits};
use rezzer_model::{Bytes, Confidence, Detection, FormatId, ReadOptions, Reader, Source};

fuzz_target!(|data: &[u8]| {
    // The last byte picks the format, so the seeds (real files) keep their own format most
    // of the time when it is mutated.
    let Some(&pick) = data.last() else {
        return;
    };
    let (format, name) = match pick % 4 {
        0 => (FormatId::Xlsx, "fuzz.xlsx"),
        1 => (FormatId::Xlsm, "fuzz.xlsm"),
        2 => (FormatId::Xlsb, "fuzz.xlsb"),
        _ => (FormatId::Ods, "fuzz.ods"),
    };
    let limits = Limits {
        max_depth: 64,
        max_time: Duration::from_secs(5),
        max_elements: 100_000,
        max_unpacked_bytes: 64 << 20,
        max_memory_bytes: 256 << 20,
        ..Limits::default()
    };
    let src = Source {
        path: None,
        name: name.to_string(),
        bytes: Bytes::Owned(Arc::new(data.to_vec())),
        sha256: String::new(),
        size: data.len() as u64,
        format,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "fuzz".to_string(),
        },
        modified: None,
    };
    let budget = Budget::new(limits);
    let mut out = rezzer_read_sheet::SheetReader.read(&src, &ReadOptions::default(), &budget);
    rezzer_enrich::enrich(&mut out.document, &rezzer_enrich::EnrichOptions::default());
    let opts = rezzer_render::RenderOptions {
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        rezzer_version: "0.0.0".to_string(),
        ..Default::default()
    };
    let _ = rezzer_render::render(&out.document, out.completeness, &opts);
    let _ = rezzer_render::render_overview(&out.document);
});
