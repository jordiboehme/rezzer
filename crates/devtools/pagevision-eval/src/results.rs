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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    #[serde(rename = "crate")]
    pub crate_version: String,
    pub llama_cpp_2: String,
    pub model_sha256: Vec<String>,
    pub prompt_sha256: String,
    pub preprocess: String,
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
    fn bbox_may_be_null() {
        let json = r#"{"kind":"paragraph","label":null,"bbox":null,"markdown":"x"}"#;
        let b: RunBlock = serde_json::from_str(json).unwrap();
        assert_eq!(b.bbox, None);
        assert_eq!(b.label, None);
    }
}
