// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reads arbitrary bytes as Markdown, enriches the document and renders it, as the pipeline
//! does. The renderer must never panic on any document a reader can produce.
#![no_main]

use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use rezzer_limits::{Budget, Limits};
use rezzer_model::{ReadOptions, Reader};

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_depth: 32,
        max_time: Duration::from_secs(5),
        ..Limits::default()
    };
    let Ok(src) = rezzer_detect::from_bytes("fuzz.md", data.to_vec(), &limits) else {
        return;
    };
    let budget = Budget::new(limits);
    let mut out = rezzer_read_markdown::MarkdownReader.read(&src, &ReadOptions::default(), &budget);
    rezzer_enrich::enrich(&mut out.document, &rezzer_enrich::EnrichOptions::default());
    let opts = rezzer_render::RenderOptions {
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        rezzer_version: "0.0.0".to_string(),
        ..Default::default()
    };
    let _ = rezzer_render::render(&out.document, out.completeness, &opts);
    let _ = rezzer_render::render_overview(&out.document);
});
