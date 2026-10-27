// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The PDF reader on arbitrary bytes under small limits, with page renders when the last
//! byte is odd. The document is enriched and rendered as the pipeline does. Nothing may
//! panic.
#![no_main]

use std::sync::Arc;
use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use rezzer_limits::{Budget, Limits};
use rezzer_model::{Bytes, Confidence, Detection, FormatId, ReadOptions, Reader, Source};

fuzz_target!(|data: &[u8]| {
    let Some(&pick) = data.last() else {
        return;
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
        name: "fuzz.pdf".to_string(),
        bytes: Bytes::Owned(Arc::new(data.to_vec())),
        sha256: String::new(),
        size: data.len() as u64,
        format: FormatId::Pdf,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "fuzz".to_string(),
        },
        modified: None,
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
    };
    let opts = ReadOptions {
        extract_assets: pick % 2 == 1,
        ..ReadOptions::default()
    };
    let budget = Budget::new(limits);
    let mut out = rezzer_read_pdf::PdfReader.read(&src, &opts, &budget);
    for blob in &out.blobs {
        assert!(out.document.assets.iter().any(|a| a.id == blob.asset));
    }
    rezzer_enrich::enrich(&mut out.document, &rezzer_enrich::EnrichOptions::default());
    let render = rezzer_render::RenderOptions {
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        rezzer_version: "0.0.0".to_string(),
        ..Default::default()
    };
    let _ = rezzer_render::render(&out.document, out.completeness, &render);
    let _ = rezzer_render::render_overview(&out.document);
});
