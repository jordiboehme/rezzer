// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Evaluation of OCR runs: manifests of test pages, the results format and the text score.

pub mod boxes;
pub mod edges;
pub mod manifest;
pub mod results;
pub mod scorecard;
pub mod text;

pub use boxes::{BoxScore, RegionScore, score_boxes, score_regions};
pub use edges::{
    Dir, Edge, EdgeScore, NodeCheck, TruthEdges, check_nodes, mermaid_edges, parse_truth,
    parse_truth_with_nodes, score_edges,
};
pub use manifest::{Manifest, PageEntry, load_manifest};
pub use results::{Fingerprint, PageRun, ResultsFile, RunBlock, RunRegion, load_results};
pub use scorecard::{Aggregate, EngineScore, PageScore, Scorecard, check_fingerprints, score};
pub use text::line_cer;

/// Everything that can go wrong while reading evaluation inputs.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("invalid manifest: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid results file: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "unknown format {found:?}, expected \"pagevision-results/1\" (results) or \"pagevision-scorecard/1\" (scorecard)"
    )]
    UnknownFormat { found: String },
    #[error("cannot decode the page image: {0}")]
    Page(#[from] pagevision::PageError),
    #[error("scoring failed: {0}")]
    Scoring(String),
    #[error("invalid command line: {0}")]
    Usage(String),
}
