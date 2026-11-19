// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The detectors against the ONNX Runtime CPU reference of the runtime spike.
//!
//! All tests are opt-in (`--run-ignored`). They need the model files in
//! `PAGEVISION_MODEL_DIR`. The public tests compare with `tests/fixtures/detect/`. The private
//! tests also need `PAGEVISION_SPIKE_OUT` (the spike's `out` folder: the ONNX Runtime
//! reference in `ort/` and, for the layout order, the spike's Candle output in `candle-cpu/`) and
//! `PAGEVISION_PRIVATE_EVAL` (the page images). They find their pages through the reference
//! files and name a failing page by its index only.

#![cfg(feature = "detect")]

use std::path::{Path, PathBuf};

use image::RgbImage;
use pagevision::detect::{Detectors, LayoutDetector, TextLineDetector};
use pagevision::ink::{flatten, gray_to_rgb};
use pagevision::layoutpost::{DRAWN_CLASSES, LayoutBox, v3_filter};
use pagevision::model::{LAYOUT_V3, ModelFiles, TEXT_DET_V6};
use pagevision::regions_from;

/// The public pages with detector fixtures.
const PUBLIC_PAGES: [&str; 5] = ["test1", "test2", "diag1", "diag2", "diag3"];

/// Probability maps may differ from the reference by at most this (ruling C1).
const MAX_PROB_DIFF: f32 = 2e-4;
/// DB's pixel threshold; no pixel may change sides of it.
const FLIP_AT: f32 = 0.2;
/// Every layout box corner may differ from the reference by at most this (pixels, ruling C1).
const MAX_BOX_DIFF: f64 = 1.;

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

fn layout_detector() -> LayoutDetector {
    LayoutDetector::load(&model_dir().join(LAYOUT_V3.local)).unwrap()
}

/// The reference boxes: `[[class, score, [x0, y0, x1, y1]], ...]`.
fn reference_boxes(v: &serde_json::Value) -> Vec<(usize, [f64; 4])> {
    v.as_array()
        .expect("a list of boxes")
        .iter()
        .map(|r| {
            let b: Vec<f64> = r[2]
                .as_array()
                .expect("a box")
                .iter()
                .map(|n| n.as_f64().expect("a number"))
                .collect();
            (
                r[0].as_u64().expect("a class") as usize,
                [b[0], b[1], b[2], b[3]],
            )
        })
        .collect()
}

/// Compares the filtered boxes with the reference (ruling C1): the same count and classes,
/// and every box within 1 px. Boxes are paired by class and the smallest corner distance,
/// since the ONNX reference lists them by score. Returns the largest corner difference, or
/// why the page fails.
fn compare_layout(got: &[LayoutBox], want: &[(usize, [f64; 4])]) -> Result<f64, String> {
    if got.len() != want.len() {
        return Err(format!("{} boxes, reference {}", got.len(), want.len()));
    }
    let mut used = vec![false; got.len()];
    let mut worst = 0f64;
    for (class, bbox) in want {
        let dist = |g: &LayoutBox| {
            (0..4)
                .map(|k| (g.bbox[k] - bbox[k]).abs())
                .fold(0f64, f64::max)
        };
        let best = got
            .iter()
            .enumerate()
            .filter(|(i, g)| !used[*i] && g.class == *class)
            .min_by(|(_, a), (_, b)| dist(a).total_cmp(&dist(b)));
        let Some((i, g)) = best else {
            return Err(format!("no box of class {class} left to pair"));
        };
        used[i] = true;
        worst = worst.max(dist(g));
    }
    if worst > MAX_BOX_DIFF {
        return Err(format!("a box differs by {worst} px"));
    }
    Ok(worst)
}

/// The output order (ruling C18): the boxes in the order of the spike's Candle runner
/// (oar-ocr-vl, sorted by the order head's reading-order rank), within 1 px.
fn compare_order(got: &[LayoutBox], spike: &serde_json::Value) -> Result<(), String> {
    let spike: Vec<(usize, [f64; 4])> = spike
        .as_array()
        .expect("a list of boxes")
        .iter()
        .map(|r| {
            let b: Vec<f64> = r[r.as_array().unwrap().len() - 1]
                .as_array()
                .expect("a box")
                .iter()
                .map(|n| n.as_f64().expect("a number"))
                .collect();
            (
                r[0].as_u64().expect("a class") as usize,
                [b[0], b[1], b[2], b[3]],
            )
        })
        .collect();
    let same = got.len() == spike.len()
        && got.iter().zip(&spike).all(|(g, (class, b))| {
            g.class == *class && (0..4).all(|k| (g.bbox[k] - b[k]).abs() <= MAX_BOX_DIFF)
        });
    if same {
        Ok(())
    } else {
        Err("the output order differs from the spike's Candle runner".to_string())
    }
}

fn drawn_count(boxes: &[(usize, [f64; 4])]) -> usize {
    boxes
        .iter()
        .filter(|(c, _)| DRAWN_CLASSES.contains(c))
        .count()
}

#[test]
#[ignore = "needs PAGEVISION_MODEL_DIR"]
fn v3_matches_reference_on_public_pages() {
    let det = layout_detector();
    let mut failures = Vec::new();
    let (mut worst, mut boxes, mut drawn) = (0f64, 0, 0);
    for page in PUBLIC_PAGES {
        let want = read_json(&fixture(&format!("{page}.v3.json")));
        let img = page_image(&public_png(page));
        let raw = det.raw(&img).unwrap();
        let got = v3_filter(&raw, img.width(), img.height());
        let want = reference_boxes(&want["final"]);
        boxes += want.len();
        drawn += drawn_count(&want);
        match compare_layout(&got, &want) {
            Ok(d) => worst = worst.max(d),
            Err(e) => failures.push(format!("{page}: {e}")),
        }
        let fixture = read_json(&fixture(&format!("{page}.v3.json")));
        if let Err(e) = compare_order(&got, &fixture["spike_candle_order"]) {
            failures.push(format!("{page}: {e}"));
        }
        let regions = det.drawn(&img).unwrap();
        if regions.len() != drawn_count(&want) {
            failures.push(format!(
                "{page}: {} drawn regions, reference {}",
                regions.len(),
                drawn_count(&want)
            ));
        }
    }
    println!(
        "v3 public: {boxes} boxes ({drawn} drawn) on {} pages, max corner diff {worst} px",
        PUBLIC_PAGES.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "needs PAGEVISION_MODEL_DIR, PAGEVISION_SPIKE_OUT and PAGEVISION_PRIVATE_EVAL"]
fn v3_matches_reference_on_private_pages() {
    let det = layout_detector();
    let (out, eval) = (spike_out(), private_eval());
    let mut failures = Vec::new();
    let (mut worst, mut boxes, mut drawn) = (0f64, 0, 0);
    let pages = private_pages(".v3.json");
    for (n, (set, stem)) in pages.iter().enumerate() {
        let reference = private_json(
            &out.join("ort").join(set).join(format!("{stem}.v3.json")),
            n,
        );
        let want = reference_boxes(&reference["final"]["0.5"]);
        let img = private_image(&eval.join(set).join(format!("{stem}.png")), n);
        let got = v3_filter(&det.raw(&img).unwrap(), img.width(), img.height());
        boxes += want.len();
        drawn += drawn_count(&want);
        match compare_layout(&got, &want) {
            Ok(d) => worst = worst.max(d),
            Err(e) => failures.push(format!("page {n}: {e}")),
        }
        let spike = out
            .join("candle-cpu")
            .join(set)
            .join(format!("{stem}.v3.json"));
        if let Err(e) = compare_order(&got, &private_json(&spike, n)["final"]["0.5"]) {
            failures.push(format!("page {n}: {e}"));
        }
    }
    println!(
        "v3 private: {} pages, {boxes} boxes ({drawn} drawn), max corner diff {worst} px",
        pages.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The drawn boxes of a fixture's filtered V3 output.
fn fixture_drawn(v3: &serde_json::Value) -> Vec<[f64; 4]> {
    reference_boxes(&v3["final"])
        .into_iter()
        .filter(|(c, _)| DRAWN_CLASSES.contains(c))
        .map(|(_, b)| b)
        .collect()
}

/// Pairs each wanted box with the closest unused got box: the same count and every corner
/// within 1 px (ruling C1). Returns the largest corner difference.
fn compare_rects(got: &[[f64; 4]], want: &[[f64; 4]]) -> Result<f64, String> {
    if got.len() != want.len() {
        return Err(format!("{} boxes, reference {}", got.len(), want.len()));
    }
    let mut used = vec![false; got.len()];
    let mut worst = 0f64;
    for w in want {
        let dist = |g: &[f64; 4]| (0..4).map(|k| (g[k] - w[k]).abs()).fold(0f64, f64::max);
        let Some((i, d)) = got
            .iter()
            .enumerate()
            .filter(|(i, _)| !used[*i])
            .map(|(i, g)| (i, dist(g)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
        else {
            return Err("no box left to pair".to_string());
        };
        used[i] = true;
        worst = worst.max(d);
    }
    if worst > MAX_BOX_DIFF {
        return Err(format!("a box differs by {worst} px"));
    }
    Ok(worst)
}

/// `Detectors` on the public pages (ruling C12): the drawn boxes match the V3 fixtures under the
/// C1 bar, the lines equal the v6 fixtures, and the snapped regions equal `regions_from` over the
/// fixture boxes.
#[test]
#[ignore = "needs PAGEVISION_MODEL_DIR"]
fn regions_of_public_pages_match_fixture_regions() {
    let files = ModelFiles::in_dir(&model_dir());
    let det = Detectors::load(&files.layout, &files.text_det).unwrap();
    let mut failures = Vec::new();
    let (mut worst, mut regions) = (0f64, 0);
    for page in PUBLIC_PAGES {
        let png = std::fs::read(public_png(page)).unwrap();
        let gray = flatten(&png).unwrap();
        let (drawn, lines) = det.boxes(&gray).unwrap();
        let want_drawn = fixture_drawn(&read_json(&fixture(&format!("{page}.v3.json"))));
        match compare_rects(&drawn, &want_drawn) {
            Ok(d) => worst = worst.max(d),
            Err(e) => failures.push(format!("{page}: drawn: {e}")),
        }
        let want_lines = &read_json(&fixture(&format!("{page}.v6.json")))["lines"];
        if line_set(&lines) != box_set(want_lines) {
            failures.push(format!("{page}: the lines differ from the fixture"));
        }
        let fixture_lines: Vec<[f64; 4]> = box_set(want_lines)
            .into_iter()
            .map(|b| b.map(|c| c as f64))
            .collect();
        let got = det.regions(&png).unwrap();
        let want = regions_from(&gray, &want_drawn, &fixture_lines);
        regions += got.regions.len();
        if got != want {
            failures.push(format!(
                "{page}: regions {:?}, from the fixtures {:?}",
                got.regions, want.regions
            ));
        }
    }
    println!(
        "regions public: {regions} regions on {} pages, drawn boxes within {worst} px",
        PUBLIC_PAGES.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
