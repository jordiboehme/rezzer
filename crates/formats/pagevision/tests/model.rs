// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests that run the real model. Opt in with `PAGEVISION_MODEL_DIR` and `--ignored`.
#![cfg(feature = "llama")]

use std::path::PathBuf;

use pagevision::PageError;
use pagevision::llama::{RunOptions, VisionModel};
use pagevision::model::QWEN35_4B;
use pagevision::prepare::{Prepared, expected_image_tokens, prepare};

fn model_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("PAGEVISION_MODEL_DIR").map(PathBuf::from);
    if dir.is_none() {
        eprintln!("skipped: PAGEVISION_MODEL_DIR is not set");
    }
    dir
}

fn test2() -> Prepared {
    let path = std::env::var_os("PAGEVISION_TEST2")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/vision/test2.png")
        });
    let png = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    prepare(&png, &QWEN35_4B.image).unwrap()
}

#[test]
#[ignore = "needs the model files"]
fn qwen_reads_test2_with_the_table_token_count() {
    let Some(dir) = model_dir() else { return };
    let p = test2();
    assert_eq!((p.width, p.height), (1408, 1856));
    let mut model = VisionModel::load(&QWEN35_4B, &dir, RunOptions::default()).unwrap();
    let g = model
        .generate(&p, "Transcribe the first line of this page.")
        .unwrap();
    eprintln!(
        "stats: {:?}\ntruncated: {}\ntext: {}",
        g.stats, g.truncated, g.text
    );
    assert_eq!(
        g.stats.image_tokens,
        expected_image_tokens(1408, 1856).unwrap()
    );
    assert!(!g.truncated);
    assert!(!g.text.trim().is_empty());
}

#[test]
#[ignore = "needs the model files"]
fn two_models_share_the_backend() {
    let Some(dir) = model_dir() else { return };
    let p = test2();
    let opts = RunOptions {
        max_tokens: 32,
        threads: None,
    };
    let mut a = VisionModel::load(&QWEN35_4B, &dir, opts.clone()).unwrap();
    let mut b = VisionModel::load(&QWEN35_4B, &dir, opts).unwrap();
    for m in [&mut a, &mut b] {
        let g = m
            .generate(&p, "Transcribe the first line of this page.")
            .unwrap();
        assert!(!g.text.trim().is_empty());
    }
}

#[test]
#[ignore = "needs the model files"]
fn unknown_size_fails_before_decoding() {
    let Some(dir) = model_dir() else { return };
    let white = Prepared {
        rgb: vec![255; 640 * 640 * 3],
        width: 640,
        height: 640,
        source_width: 640,
        source_height: 640,
    };
    let mut model = VisionModel::load(&QWEN35_4B, &dir, RunOptions::default()).unwrap();
    let err = model.generate(&white, "Describe the image.").unwrap_err();
    assert!(
        matches!(
            err,
            PageError::UnknownImageSize {
                width: 640,
                height: 640
            }
        ),
        "{err}"
    );
}

#[test]
#[ignore = "information run, nothing asserted"]
fn layout_prompt_dump() {
    let Some(dir) = model_dir() else { return };
    if std::env::var("PAGEVISION_DUMP").as_deref() != Ok("1") {
        eprintln!("skipped: PAGEVISION_DUMP is not 1");
        return;
    }
    let p = test2();
    let mut model = VisionModel::load(&QWEN35_4B, &dir, RunOptions::default()).unwrap();
    let prompt = "Locate every instance that belongs to the following categories: \"paragraph\", \"heading\", \"list\", \"diagram\". A diagram is one instance covering all its shapes, arrows and labels. Report bbox coordinates in JSON format, as [{\"bbox_2d\": [x1, y1, x2, y2], \"label\": \"category\", \"text_content\": \"first words\"}, ...].";
    let g = model.generate(&p, prompt).unwrap();
    eprintln!(
        "stats: {:?}\ntruncated: {}\nDUMP>>>\n{}\n<<<DUMP",
        g.stats, g.truncated, g.text
    );
}
