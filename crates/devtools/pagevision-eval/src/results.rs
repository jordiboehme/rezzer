// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `pagevision-results/1` format written by engines and baselines.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::EvalError;

pub const FORMAT: &str = "pagevision-results/1";

/// A serde mirror of the run fingerprint. `preprocess` is a string because baselines write
/// "none".
///
/// The `crate` field of a pagevision engine is the bare workspace version (for example
/// "0.1.0"), exactly as `pagevision::llama` writes it, because `check_fingerprints` compares it
/// with the build's version. A baseline engine may write any text there (the Chandra baseline
/// uses "notedown-ocr 0.1.0"); baselines are skipped by that check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    #[serde(rename = "crate")]
    pub crate_version: String,
    pub llama_cpp_2: String,
    pub model_sha256: Vec<String>,
    pub prompt_sha256: String,
    pub preprocess: String,
    /// The detector fields; empty or 0 in files written before them (and for baselines).
    #[serde(default)]
    pub layout_sha256: String,
    #[serde(default)]
    pub text_det_sha256: String,
    #[serde(default)]
    pub candle: String,
    #[serde(default)]
    pub rules: u32,
    #[serde(default)]
    pub ink: u32,
}

impl From<&pagevision::Fingerprint> for Fingerprint {
    fn from(f: &pagevision::Fingerprint) -> Self {
        Self {
            crate_version: f.crate_version.clone(),
            llama_cpp_2: f.llama_cpp_2.clone(),
            model_sha256: f.model_sha256.clone(),
            prompt_sha256: f.prompt_sha256.clone(),
            preprocess: f.preprocess.to_string(),
            layout_sha256: f.layout_sha256.clone(),
            text_det_sha256: f.text_det_sha256.clone(),
            candle: f.candle.clone(),
            rules: f.rules,
            ink: f.ink,
        }
    }
}

/// The kind string of a region: "text" or "drawing" (the one `score_regions` counts as drawn).
pub fn region_kind_str(kind: pagevision::RegionKind) -> &'static str {
    match kind {
        pagevision::RegionKind::Drawing => "drawing",
        _ => "text",
    }
}

impl From<&pagevision::Region> for RunRegion {
    fn from(r: &pagevision::Region) -> Self {
        Self {
            kind: region_kind_str(r.kind).to_string(),
            bbox: [
                u32::from(r.bbox.x0),
                u32::from(r.bbox.y0),
                u32::from(r.bbox.x1),
                u32::from(r.bbox.y1),
            ],
        }
    }
}

/// The lowercase kind string of the results format ("diagram" is the one `score_boxes` counts).
pub fn kind_str(kind: pagevision::BlockKind) -> &'static str {
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

impl From<&pagevision::Block> for RunBlock {
    fn from(b: &pagevision::Block) -> Self {
        Self {
            kind: kind_str(b.kind()).to_string(),
            label: None,
            bbox: Some([
                u32::from(b.bbox.x0),
                u32::from(b.bbox.y0),
                u32::from(b.bbox.x1),
                u32::from(b.bbox.y1),
            ]),
            markdown: b.markdown.clone(),
        }
    }
}

impl PageRun {
    /// The run of one page as pagevision returned it, under the page id of the manifest.
    pub fn from_result(id: &str, result: &pagevision::PageResult) -> Self {
        Self {
            id: id.to_string(),
            markdown: result.markdown.clone(),
            truncated: result.truncated,
            prompt_tokens: result.stats.prompt_tokens,
            output_tokens: result.stats.output_tokens,
            encode_ms: result.stats.encode_ms,
            decode_ms: result.stats.decode_ms,
            blocks: result.blocks.iter().map(RunBlock::from).collect(),
            regions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultsFile {
    pub format: String,
    pub engine: String,
    pub fingerprint: Fingerprint,
    pub pages: Vec<PageRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageRun {
    pub id: String,
    pub markdown: String,
    pub truncated: bool,
    pub prompt_tokens: u32,
    pub output_tokens: u32,
    pub encode_ms: u64,
    pub decode_ms: u64,
    pub blocks: Vec<RunBlock>,
    /// The proposed regions of a regions-only run; empty in older files.
    #[serde(default)]
    pub regions: Vec<RunRegion>,
}

/// One proposed region: `kind` is "text" or "drawing", the box is on the 0..=1000 grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRegion {
    pub kind: String,
    pub bbox: [u32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunBlock {
    pub kind: String,
    pub label: Option<String>,
    /// May be null in the file.
    pub bbox: Option<[u32; 4]>,
    pub markdown: String,
}

/// Reads a results file and rejects any format other than `pagevision-results/1`.
pub fn load_results(path: &Path) -> Result<ResultsFile, EvalError> {
    let raw = std::fs::read_to_string(path).map_err(|source| EvalError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    // Check the format first, so a file of another format names it instead of failing on a
    // missing field.
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let found = value
        .get("format")
        .and_then(|f| f.as_str())
        .unwrap_or("")
        .to_string();
    if found != FORMAT {
        return Err(EvalError::UnknownFormat { found });
    }
    Ok(serde_json::from_value(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_rejects_unknown_format() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("r.json");
        std::fs::write(&file, r#"{"format":"pagevision-results/2","pages":[]}"#).unwrap();
        match load_results(&file).unwrap_err() {
            EvalError::UnknownFormat { found } => assert_eq!(found, "pagevision-results/2"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn results_rejects_missing_format() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("r.json");
        std::fs::write(&file, "{}").unwrap();
        assert!(matches!(
            load_results(&file).unwrap_err(),
            EvalError::UnknownFormat { .. }
        ));
    }

    #[test]
    fn reads_n2_example() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chandra-test2.json");
        let r = load_results(&path).unwrap();
        assert_eq!(r.format, FORMAT);
        assert_eq!(r.engine, "chandra-ocr-2");
        assert_eq!(r.fingerprint.crate_version, "notedown-ocr 0.1.0");
        assert_eq!(r.fingerprint.preprocess, "none");
        assert_eq!(r.fingerprint.model_sha256.len(), 2);
        assert_eq!(r.pages.len(), 1);
        let page = &r.pages[0];
        assert_eq!(page.id, "test2");
        assert!(!page.truncated);
        assert_eq!(page.prompt_tokens, 3189);
        assert_eq!(page.blocks.len(), 3);
        assert_eq!(page.blocks[0].bbox, Some([98, 113, 742, 250]));
    }

    #[test]
    fn results_v1_without_regions_still_loads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chandra-test2.json");
        let r = load_results(&path).unwrap();
        assert!(r.pages[0].regions.is_empty());
        assert_eq!(r.fingerprint.layout_sha256, "");
        assert_eq!(r.fingerprint.candle, "");
        assert_eq!((r.fingerprint.rules, r.fingerprint.ink), (0, 0));
    }

    #[test]
    fn regions_round_trip() {
        let region = pagevision::Region {
            bbox: pagevision::BBox {
                x0: 1,
                y0: 2,
                x1: 3,
                y1: 4,
            },
            kind: pagevision::RegionKind::Drawing,
            certainty: 0.5,
            note: None,
        };
        let r = RunRegion::from(&region);
        assert_eq!(r.kind, "drawing");
        assert_eq!(r.bbox, [1, 2, 3, 4]);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"kind":"drawing","bbox":[1,2,3,4]}"#);
        assert_eq!(serde_json::from_str::<RunRegion>(&json).unwrap(), r);
    }

    #[test]
    fn bbox_may_be_null() {
        let json = r#"{"kind":"paragraph","label":null,"bbox":null,"markdown":"x"}"#;
        let b: RunBlock = serde_json::from_str(json).unwrap();
        assert_eq!(b.bbox, None);
        assert_eq!(b.label, None);
    }

    /// Builds a pagevision result, converts it, writes it, reads it with the eval parser and
    /// scores it: the whole seam between the crate and the harness.
    #[test]
    fn pagevision_output_crosses_the_seam() {
        use crate::{Manifest, PageEntry, check_fingerprints, score};
        use pagevision::model::{PREPROCESS_VERSION, QWEN35_4B};
        use pagevision::{BBox, Block, Content, PageResult, Stats};

        let fingerprint = pagevision::Fingerprint {
            crate_version: env!("CARGO_PKG_VERSION").to_string(),
            llama_cpp_2: "0.1.158".to_string(),
            model_sha256: vec![
                QWEN35_4B.model.sha256.to_string(),
                QWEN35_4B.mmproj.sha256.to_string(),
            ],
            prompt_sha256: "p".to_string(),
            preprocess: PREPROCESS_VERSION,
            ..pagevision::regions::regions_fingerprint()
        };
        let result = PageResult {
            width_px: 40,
            height_px: 40,
            blocks: vec![Block {
                bbox: BBox {
                    x0: 0,
                    y0: 0,
                    x1: 1000,
                    y1: 1000,
                },
                certainty: 1.0,
                content: Content::Diagram {
                    mermaid: "graph TD\n  A --> B".to_string(),
                    description: String::new(),
                },
                markdown: "```mermaid\ngraph TD\n  A --> B\n```".to_string(),
            }],
            markdown: "```mermaid\ngraph TD\n  A --> B\n```".to_string(),
            truncated: false,
            stats: Stats::default(),
            fingerprint: fingerprint.clone(),
            raw: None,
        };
        let file = ResultsFile {
            format: FORMAT.to_string(),
            engine: "qwen".to_string(),
            fingerprint: Fingerprint::from(&result.fingerprint),
            pages: vec![PageRun::from_result("p1", &result)],
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.json");
        std::fs::write(&path, serde_json::to_string(&file).unwrap()).unwrap();
        let loaded = load_results(&path).unwrap();
        assert_eq!(loaded, file);
        assert_eq!(loaded.pages[0].blocks[0].kind, "diagram");

        let image = dir.path().join("p1.png");
        // A short stroke: a run of half the page width or more would be a ruled line, not ink.
        let mut img = image::RgbImage::from_pixel(40, 40, image::Rgb([255, 255, 255]));
        for x in 5..15 {
            img.put_pixel(x, 20, image::Rgb([0, 0, 0]));
        }
        img.save(&image).unwrap();
        let manifest = Manifest {
            set: "public".into(),
            pages: vec![PageEntry {
                id: "p1".into(),
                image,
                text: None,
                edges: None,
                nodes: None,
                diagrams: 0,
                regions: None,
                sketches: 0,
                known_fail: false,
            }],
        };
        let entry = score(&[manifest], &loaded, false).unwrap();
        assert_eq!(entry.public[0].boxes.unwrap().split_diagrams, 1);

        // The converted fingerprint passes the CI check against a matching lock.
        let lock = "[[package]]\nname = \"candle-core\"\nversion = \"0.11.1\"\n\n[[package]]\nname = \"candle-nn\"\nversion = \"0.11.1\"\n\n[[package]]\nname = \"llama-cpp-2\"\nversion = \"0.1.158\"\n\n[[package]]\nname = \"llama-cpp-sys-2\"\nversion = \"0.1.158\"\n";
        let card = crate::Scorecard {
            format: crate::scorecard::FORMAT.into(),
            engines: vec![entry],
        };
        assert_eq!(check_fingerprints(&card, lock), Ok(()));
    }
}
