// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Runs pagevision over the pages of one or more manifests and writes a `pagevision-results/1`
//! file for `pagevision-eval score`.
//!
//! ```text
//! PAGEVISION_MODEL_DIR=<models> cargo run --release -p pagevision --features detect,serde \
//!     --example eval_run -- --regions-only --manifest <m>... --out <results.json>
//! ```
//!
//! `--regions-only` runs the two detectors, the ink rules and snapping (engine
//! `pagevision-c-regions`): every page gets its regions and no blocks. The detector time of a
//! page goes into `encode_ms`; it is printed, never checked.

use std::path::{Path, PathBuf};
use std::time::Instant;

use pagevision::detect::Detectors;
use pagevision::model::ModelFiles;
use pagevision::regions::regions_fingerprint;
use pagevision::{RegionKind, Regions};
use serde_json::{Value, json};

const USAGE: &str =
    "usage: eval_run --regions-only --manifest <manifest.toml>... --out <results.json>";
const ENGINE: &str = "pagevision-c-regions";

/// The pages of a manifest: the id and the image path, resolved against the manifest's folder.
fn manifest_pages(path: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let doc: toml::Table =
        toml::from_str(&raw).map_err(|e| format!("invalid manifest {}: {e}", path.display()))?;
    let base = path.parent().unwrap_or_else(|| Path::new(""));
    let pages = match doc.get("pages") {
        Some(toml::Value::Array(pages)) => pages,
        None => return Ok(Vec::new()),
        Some(_) => return Err(format!("{}: pages is not a list", path.display())),
    };
    pages
        .iter()
        .map(|p| {
            let field = |name: &str| {
                p.get(name)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("{}: a page without {name}", path.display()))
            };
            Ok((field("id")?.to_string(), base.join(field("image")?)))
        })
        .collect()
}

fn region_json(r: &Regions) -> Vec<Value> {
    r.regions
        .iter()
        .map(|x| {
            let kind = match x.kind {
                RegionKind::Drawing => "drawing",
                _ => "text",
            };
            json!({
                "kind": kind,
                "bbox": [x.bbox.x0, x.bbox.y0, x.bbox.x1, x.bbox.y1],
            })
        })
        .collect()
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (mut manifests, mut out, mut regions_only) = (Vec::new(), None, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--regions-only" => regions_only = true,
            "--manifest" => manifests.push(PathBuf::from(
                args.next()
                    .ok_or(format!("--manifest needs a value; {USAGE}"))?,
            )),
            "--out" => {
                out = Some(PathBuf::from(
                    args.next().ok_or(format!("--out needs a value; {USAGE}"))?,
                ))
            }
            other => return Err(format!("unknown argument {other}; {USAGE}")),
        }
    }
    if !regions_only {
        return Err(format!("only --regions-only is supported; {USAGE}"));
    }
    let Some(out) = out else {
        return Err(USAGE.to_string());
    };
    if manifests.is_empty() {
        return Err(USAGE.to_string());
    }
    let dir = std::env::var_os("PAGEVISION_MODEL_DIR")
        .map(PathBuf::from)
        .ok_or("set PAGEVISION_MODEL_DIR to the folder with the model files")?;
    let files = ModelFiles::in_dir(&dir);
    let started = Instant::now();
    let detectors = Detectors::load(&files.layout, &files.text_det).map_err(|e| e.to_string())?;
    eprintln!("detectors loaded in {} ms", started.elapsed().as_millis());

    let mut pages = Vec::new();
    for manifest in &manifests {
        for (id, image) in manifest_pages(manifest)? {
            let png = std::fs::read(&image).map_err(|e| format!("page {id}: {e}"))?;
            let started = Instant::now();
            let regions = detectors
                .regions(&png)
                .map_err(|e| format!("page {id}: {e}"))?;
            let ms = started.elapsed().as_millis() as u64;
            let count = |k: RegionKind| regions.regions.iter().filter(|r| r.kind == k).count();
            let possible = regions.regions.iter().filter(|r| r.note.is_some()).count();
            eprintln!(
                "{id}: {} drawings, {} text, {possible} possible missed drawings, {ms} ms",
                count(RegionKind::Drawing),
                count(RegionKind::Text),
            );
            for note in &regions.notes {
                eprintln!("{id}: note: {note}");
            }
            pages.push(json!({
                "id": id,
                "markdown": "",
                "truncated": false,
                "prompt_tokens": 0,
                "output_tokens": 0,
                "encode_ms": ms,
                "decode_ms": 0,
                "blocks": [],
                "regions": region_json(&regions),
            }));
        }
    }

    let count = pages.len();
    let f = regions_fingerprint();
    let results = json!({
        "format": "pagevision-results/1",
        "engine": ENGINE,
        "fingerprint": {
            "crate": f.crate_version,
            "llama_cpp_2": f.llama_cpp_2,
            "model_sha256": f.model_sha256,
            "prompt_sha256": f.prompt_sha256,
            "preprocess": f.preprocess.to_string(),
            "layout_sha256": f.layout_sha256,
            "text_det_sha256": f.text_det_sha256,
            "candle": f.candle,
            "rules": f.rules,
            "ink": f.ink,
        },
        "pages": pages,
    });
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut text = serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(&out, text).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("{count} pages written to {}", out.display());
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("eval_run: {e}");
        std::process::exit(1);
    }
}
