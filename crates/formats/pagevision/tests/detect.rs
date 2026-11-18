// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The detectors against the ONNX Runtime CPU reference of the runtime spike.
//!
//! All tests are opt-in (`--run-ignored`). They need the model files in
//! `PAGEVISION_MODEL_DIR`. The public tests compare with `tests/fixtures/detect/`. The private
//! tests also need `PAGEVISION_SPIKE_OUT` (the spike's `out` folder with the reference) and
//! `PAGEVISION_PRIVATE_EVAL` (the page images). They find their pages through the reference
//! files and name a failing page by its index only.

#![cfg(feature = "detect")]

use std::path::{Path, PathBuf};

use image::RgbImage;
use pagevision::detect::TextLineDetector;
use pagevision::ink::{flatten, gray_to_rgb};
use pagevision::model::TEXT_DET_V6;

/// The public pages with detector fixtures.
const PUBLIC_PAGES: [&str; 5] = ["test1", "test2", "diag1", "diag2", "diag3"];

/// Probability maps may differ from the reference by at most this (ruling C1).
const MAX_PROB_DIFF: f32 = 2e-4;
/// DB's pixel threshold; no pixel may change sides of it.
const FLIP_AT: f32 = 0.2;

fn env_dir(name: &str, what: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name} to {what}"))
}

fn model_dir() -> PathBuf {
    env_dir("PAGEVISION_MODEL_DIR", "the folder with the model files")
}

fn spike_out() -> PathBuf {
    env_dir(
        "PAGEVISION_SPIKE_OUT",
        "the runtime spike's out folder (with ort/pages)",
    )
}

fn private_eval() -> PathBuf {
    env_dir("PAGEVISION_PRIVATE_EVAL", "the private-eval folder")
}

fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The grey-on-white page the detectors read (as the bake-off fed Paddle).
fn page_image(png: &Path) -> RgbImage {
    let bytes = std::fs::read(png).unwrap_or_else(|e| panic!("{}: {e}", png.display()));
    gray_to_rgb(&flatten(&bytes).unwrap())
}

fn public_png(page: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../../tests/corpus/vision/{page}.png"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/detect/{name}"))
}

/// Integer boxes, sorted, so two line sets compare as sets.
fn box_set(v: &serde_json::Value) -> Vec<[i64; 4]> {
    let mut boxes: Vec<[i64; 4]> = v
        .as_array()
        .expect("a list of boxes")
        .iter()
        .map(|b| {
            let c: Vec<i64> = b
                .as_array()
                .expect("a box")
                .iter()
                .map(|n| n.as_f64().expect("a number") as i64)
                .collect();
            [c[0], c[1], c[2], c[3]]
        })
        .collect();
    boxes.sort_unstable();
    boxes
}

fn line_set(lines: &[[f64; 4]]) -> Vec<[i64; 4]> {
    let mut boxes: Vec<[i64; 4]> = lines
        .iter()
        .map(|b| {
            b.map(|c| {
                assert_eq!(c.fract(), 0., "line boxes are whole pixels");
                c as i64
            })
        })
        .collect();
    boxes.sort_unstable();
    boxes
}

fn text_detector() -> TextLineDetector {
    TextLineDetector::load(&model_dir().join(TEXT_DET_V6.local)).unwrap()
}

#[test]
#[ignore = "needs PAGEVISION_MODEL_DIR"]
fn v6_lines_match_reference_on_public_pages() {
    let det = text_detector();
    let mut failures = Vec::new();
    let mut total = 0;
    for page in PUBLIC_PAGES {
        let want = read_json(&fixture(&format!("{page}.v6.json")));
        let img = page_image(&public_png(page));
        assert_eq!(
            u64::from(img.width()),
            want["w"].as_u64().unwrap(),
            "{page} width"
        );
        assert_eq!(
            u64::from(img.height()),
            want["h"].as_u64().unwrap(),
            "{page} height"
        );
        let got = line_set(&det.lines(&img).unwrap());
        let want = box_set(&want["lines"]);
        total += want.len();
        if got != want {
            failures.push(format!("{page}: got {got:?}, want {want:?}"));
        }
    }
    println!("v6 public: {total} lines on {} pages", PUBLIC_PAGES.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A private page image; an error names the page index only.
fn private_image(png: &Path, n: usize) -> RgbImage {
    let bytes = std::fs::read(png).unwrap_or_else(|_| panic!("page {n}: cannot read the image"));
    gray_to_rgb(&flatten(&bytes).unwrap_or_else(|_| panic!("page {n}: cannot decode the image")))
}

/// A private reference file; an error names the page index only.
fn private_json(path: &Path, n: usize) -> serde_json::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("page {n}: cannot read the reference"));
    serde_json::from_str(&text).unwrap_or_else(|_| panic!("page {n}: bad reference json"))
}

/// The private pages of the reference: (set, page stem), sorted. Never printed.
fn private_pages(kind: &str) -> Vec<(String, String)> {
    let out = spike_out();
    let mut pages = Vec::new();
    for set in ["pages", "pages1856"] {
        let dir = out.join("ort").join(set);
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        for entry in entries {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(kind) {
                pages.push((set.to_string(), stem.to_string()));
            }
        }
    }
    pages.sort();
    assert!(
        !pages.is_empty(),
        "no {kind} reference in PAGEVISION_SPIKE_OUT"
    );
    pages
}

#[test]
#[ignore = "needs PAGEVISION_MODEL_DIR, PAGEVISION_SPIKE_OUT and PAGEVISION_PRIVATE_EVAL"]
fn v6_prob_map_matches_reference_on_private_pages() {
    let det = text_detector();
    let (out, eval) = (spike_out(), private_eval());
    let mut failures = Vec::new();
    let (mut worst, mut flips, mut lines) = (0f32, 0usize, 0usize);
    let pages = private_pages(".v6.f32");
    for (n, (set, stem)) in pages.iter().enumerate() {
        let dir = out.join("ort").join(set);
        // Errors name the page index only, never the page or its path.
        let bytes = std::fs::read(dir.join(format!("{stem}.v6.f32")))
            .unwrap_or_else(|_| panic!("page {n}: cannot read the reference map"));
        let want: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        let img = private_image(&eval.join(set).join(format!("{stem}.png")), n);
        let (got, w, h) = det.probability_map(&img).unwrap();
        if got.len() != want.len() {
            failures.push(format!(
                "page {n}: map {w}x{h}, reference {} values",
                want.len()
            ));
            continue;
        }
        let diff = got
            .iter()
            .zip(&want)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        let flipped = got
            .iter()
            .zip(&want)
            .filter(|&(a, b)| (*a > FLIP_AT) != (*b > FLIP_AT))
            .count();
        worst = worst.max(diff);
        flips += flipped;
        if diff > MAX_PROB_DIFF || flipped > 0 {
            failures.push(format!(
                "page {n}: max diff {diff}, {flipped} pixels flip at 0.2"
            ));
        }
        let want_lines = box_set(&private_json(&dir.join(format!("{stem}.v6.json")), n)["lines"]);
        let got_lines = line_set(&pagevision::textdet::db_lines(&got, w, h, img.dimensions()));
        lines += want_lines.len();
        if got_lines != want_lines {
            failures.push(format!(
                "page {n}: {} lines, reference {}",
                got_lines.len(),
                want_lines.len()
            ));
        }
    }
    println!(
        "v6 private: {} pages, {lines} lines, max abs diff {worst:e}, {flips} flips",
        pages.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
