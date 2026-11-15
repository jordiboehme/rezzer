// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Evaluation of OCR runs: manifests of test pages, the results format and the text score.

pub mod boxes;
pub mod edges;
pub mod manifest;
pub mod results;
pub mod text;

pub use boxes::{BoxScore, ink_mask, score_boxes};
pub use edges::{
    Dir, Edge, EdgeScore, TruthEdges, mermaid_edges, parse_truth, parse_truth_with_nodes,
    score_edges,
};
pub use manifest::{Manifest, PageEntry, load_manifest};
pub use results::{Fingerprint, PageRun, ResultsFile, RunBlock, load_results};
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
    #[error("unknown results format {found:?}, expected \"pagevision-results/1\"")]
    UnknownFormat { found: String },
    #[error("cannot decode the page image: {0}")]
    Page(#[from] pagevision::PageError),
}
