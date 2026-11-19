// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The page handle on the public pages, with the real models. Opt in with `--features engine`,
//! `--ignored` and `PAGEVISION_MODEL_DIR`.
#![cfg(feature = "engine")]

use std::path::PathBuf;

use pagevision::engine::PageVision;
use pagevision::llama::RunOptions;
use pagevision::model::ModelFiles;
use pagevision::{BBox, BlockKind, Region, RegionKind};

/// The four model files. An opted-in test without the folder fails and names the variable.
fn page_vision() -> PageVision {
    let dir = std::env::var_os("PAGEVISION_MODEL_DIR")
        .map(PathBuf::from)
        .expect("set PAGEVISION_MODEL_DIR to the folder with the four model files");
    let opts = RunOptions {
        keep_raw: true,
        ..RunOptions::default()
    };
    PageVision::load(&ModelFiles::in_dir(&dir), opts).unwrap()
}

fn public_png(page: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../../tests/corpus/vision/{page}.png"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// test2 has one text region and two diagram regions. The page read gives the Mermaid; when it
/// writes both diagrams in one block, the second region is a picture that says so. Every region
/// gives a block, and the page holds the nodes of both diagrams.
#[test]
#[ignore = "needs the model files"]
fn read_page_test2_has_both_diagrams() {
    let mut pv = page_vision();
    let page = pv.read_page(&public_png("test2")).unwrap();
    eprintln!(
        "{}\n---\n{}",
        page.markdown,
        page.raw.as_deref().unwrap_or("")
    );
    let kinds: Vec<BlockKind> = page.blocks.iter().map(|b| b.kind()).collect();
    assert_eq!(kinds.len(), 3, "{kinds:?}");
    assert_eq!(kinds[0], BlockKind::Paragraph);
    assert_eq!(kinds[1], BlockKind::Diagram);
    assert!(
        matches!(kinds[2], BlockKind::Diagram | BlockKind::Picture),
        "{kinds:?}"
    );
    for word in ["search", "Learn", "teach", "App", "DB", "MCP"] {
        assert!(page.markdown.contains(word), "{word}");
    }
    assert!(page.markdown.to_lowercase().contains("real-time"));
    assert!(!page.truncated);
    assert!(page.blocks.iter().all(|b| b.certainty <= 0.95));
}

#[test]
#[ignore = "needs the model files"]
fn read_with_given_regions_skips_detection() {
    let mut pv = page_vision();
    // The text lines at the top of test2, given as one text region.
    let region = Region {
        bbox: BBox {
            x0: 103,
            y0: 114,
            x1: 739,
            y1: 249,
        },
        kind: RegionKind::Text,
        certainty: 1.0,
        note: None,
    };
    let page = pv
        .read(&public_png("test2"), std::slice::from_ref(&region))
        .unwrap();
    eprintln!("{}", page.markdown);
    assert_eq!(page.blocks.len(), 1);
    assert_eq!(page.blocks[0].bbox, region.bbox);
    assert!(page.markdown.to_lowercase().contains("real-time"));
}
