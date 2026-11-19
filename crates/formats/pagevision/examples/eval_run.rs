// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Runs pagevision over the pages of one or more manifests and writes a `pagevision-results/1`
//! file for `pagevision-eval score`.
//!
//! ```text
//! PAGEVISION_MODEL_DIR=<models> cargo run --release -p pagevision --features detect,serde \
//!     --example eval_run -- --regions-only --manifest <m>... --out <results.json>
//! PAGEVISION_MODEL_DIR=<models> cargo run --release -p pagevision --features engine,serde \
//!     --example eval_run -- --manifest <m>... --out <results.json>
//! ```
//!
//! Without `--regions-only` (feature `engine`) it reads every page with `PageVision` (engine
//! `pagevision-c`) as `read_page` does: the regions, then Qwen. The results hold the blocks, the page
//! Markdown and the regions. The raw answers of each region (the classifier's answer, the reasons
//! and the content answer) go to stderr only. A page that gives no block is written with an empty
//! Markdown and no regions, so it is scored as read. The fingerprint holds the measured SHA-256
//! of all four files.
//!
//! `--regions-only` runs the two detectors, the ink rules and snapping (engine
//! `pagevision-c-regions`): every page gets its regions and no blocks. The detector time of a
//! page goes into `encode_ms`; it is printed, never checked. The fingerprint holds the measured
//! SHA-256 of both detector files; a file other than the pinned one stops the run.

use std::path::{Path, PathBuf};
use std::time::Instant;

use pagevision::detect::Detectors;
use pagevision::model::{HubFile, LAYOUT_V3, ModelFiles, TEXT_DET_V6, file_sha256};
use pagevision::regions::regions_fingerprint;
use pagevision::{RegionKind, Regions};
use serde_json::{Value, json};

const USAGE: &str =
    "usage: eval_run [--regions-only] --manifest <manifest.toml>... --out <results.json>";
const ENGINE: &str = "pagevision-c-regions";
#[cfg(feature = "engine")]
const FULL_ENGINE: &str = "pagevision-c";

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

/// The SHA-256 of a detector file, measured. A file that is not the pinned one stops the run, so a
/// results file never names a model it did not run.
fn checked_sha256(path: &Path, pinned: &HubFile) -> Result<String, String> {
    let hash = file_sha256(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if hash != pinned.sha256 {
        return Err(format!(
            "{}: SHA-256 is {hash}, expected {} ({})",
            path.display(),
            pinned.sha256,
            pinned.local
        ));
    }
    Ok(hash)
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
    let Some(out) = out else {
        return Err(USAGE.to_string());
    };
    if manifests.is_empty() {
        return Err(USAGE.to_string());
    }
    if !regions_only {
        return full_run(&manifests, &out);
    }
    let dir = std::env::var_os("PAGEVISION_MODEL_DIR")
        .map(PathBuf::from)
        .ok_or("set PAGEVISION_MODEL_DIR to the folder with the model files")?;
    let files = ModelFiles::in_dir(&dir);
    let layout_sha256 = checked_sha256(&files.layout, &LAYOUT_V3)?;
    let text_det_sha256 = checked_sha256(&files.text_det, &TEXT_DET_V6)?;
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
            "layout_sha256": layout_sha256,
            "text_det_sha256": text_det_sha256,
            "candle": f.candle,
            "rules": f.rules,
            "ink": f.ink,
        },
        "pages": pages,
    });
    write_results(&out, &results, count)
}

fn write_results(out: &Path, results: &Value, count: usize) -> Result<(), String> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut text = serde_json::to_string_pretty(results).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(out, text).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("{count} pages written to {}", out.display());
    Ok(())
}

#[cfg(not(feature = "engine"))]
fn full_run(_: &[PathBuf], _: &Path) -> Result<(), String> {
    Err(format!(
        "a full run needs the feature engine; without it only --regions-only works; {USAGE}"
    ))
}

/// The lowercase kind of a block in the results format.
#[cfg(feature = "engine")]
fn kind_str(kind: pagevision::BlockKind) -> &'static str {
    use pagevision::BlockKind;
    match kind {
        BlockKind::Paragraph => "paragraph",
        BlockKind::Heading => "heading",
        BlockKind::List => "list",
        BlockKind::Diagram => "diagram",
        BlockKind::Table => "table",
        BlockKind::Picture => "picture",
        _ => "text",
    }
}

#[cfg(feature = "engine")]
fn full_run(manifests: &[PathBuf], out: &Path) -> Result<(), String> {
    use pagevision::engine::PageVision;
    use pagevision::llama::RunOptions;
    use pagevision::model::QWEN35_4B;

    let dir = std::env::var_os("PAGEVISION_MODEL_DIR")
        .map(PathBuf::from)
        .ok_or("set PAGEVISION_MODEL_DIR to the folder with the model files")?;
    let files = ModelFiles::in_dir(&dir);
    let [model_file, mmproj_file] = QWEN35_4B.hub_files();
    let model_sha256 = vec![
        checked_sha256(&files.model, &model_file)?,
        checked_sha256(&files.mmproj, &mmproj_file)?,
    ];
    let layout_sha256 = checked_sha256(&files.layout, &LAYOUT_V3)?;
    let text_det_sha256 = checked_sha256(&files.text_det, &TEXT_DET_V6)?;
    let opts = RunOptions {
        keep_raw: true,
        ..RunOptions::default()
    };
    let started = Instant::now();
    let mut pv = PageVision::load(&files, opts).map_err(|e| e.to_string())?;
    eprintln!("models loaded in {} ms", started.elapsed().as_millis());

    let mut pages = Vec::new();
    for manifest in manifests {
        for (id, image) in manifest_pages(manifest)? {
            let png = std::fs::read(&image).map_err(|e| format!("page {id}: {e}"))?;
            let started = Instant::now();
            let regions = pv.regions(&png).map_err(|e| format!("page {id}: {e}"))?;
            let read = pv.read_page_with(&png, &regions);
            let ms = started.elapsed().as_millis();
            let page = match read {
                Ok(r) => {
                    eprintln!(
                        "{id}: {} regions, {} blocks, truncated {}, {} prompt and {} output tokens, {ms} ms",
                        regions.regions.len(),
                        r.blocks.len(),
                        r.truncated,
                        r.stats.prompt_tokens,
                        r.stats.output_tokens,
                    );
                    eprintln!("{id}: raw\n{}", r.raw.as_deref().unwrap_or(""));
                    let blocks: Vec<Value> = r
                        .blocks
                        .iter()
                        .map(|b| {
                            json!({
                                "kind": kind_str(b.kind()),
                                "label": null,
                                "bbox": [b.bbox.x0, b.bbox.y0, b.bbox.x1, b.bbox.y1],
                                "markdown": b.markdown,
                            })
                        })
                        .collect();
                    json!({
                        "id": id,
                        "markdown": r.markdown,
                        "truncated": r.truncated,
                        "prompt_tokens": r.stats.prompt_tokens,
                        "output_tokens": r.stats.output_tokens,
                        "encode_ms": r.stats.encode_ms,
                        "decode_ms": r.stats.decode_ms,
                        "blocks": blocks,
                        "regions": region_json(&regions),
                    })
                }
                Err(e) => {
                    eprintln!("{id}: no block: {e}, {ms} ms");
                    json!({
                        "id": id,
                        "markdown": "",
                        "truncated": false,
                        "prompt_tokens": 0,
                        "output_tokens": 0,
                        "encode_ms": 0,
                        "decode_ms": 0,
                        "blocks": [],
                        "regions": [],
                    })
                }
            };
            pages.push(page);
        }
    }

    let count = pages.len();
    let f = pv.fingerprint();
    let results = json!({
        "format": "pagevision-results/1",
        "engine": FULL_ENGINE,
        "fingerprint": {
            "crate": f.crate_version,
            "llama_cpp_2": f.llama_cpp_2,
            "model_sha256": model_sha256,
            "prompt_sha256": f.prompt_sha256,
            "preprocess": f.preprocess.to_string(),
            "layout_sha256": layout_sha256,
            "text_det_sha256": text_det_sha256,
            "candle": f.candle,
            "rules": f.rules,
            "ink": f.ink,
        },
        "pages": pages,
    });
    write_results(out, &results, count)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("eval_run: {e}");
        std::process::exit(1);
    }
}
