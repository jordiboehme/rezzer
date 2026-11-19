// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Page OCR with vision models.

#[cfg(feature = "detect")]
pub mod detect;
mod error;
pub mod ink;
pub mod layoutpost;
#[cfg(feature = "llama")]
pub mod llama;
pub mod mermaid;
pub mod model;
pub mod onnx;
pub mod parse;
pub mod prepare;
pub mod prompts;
pub mod regions;
pub mod resize;
pub mod rules;
pub mod snap;
pub mod textdet;
mod types;

pub use error::PageError;
pub use parse::DrawingClass;
pub use regions::regions_from;
pub use types::{
    BBox, Block, BlockKind, Cell, Content, Fingerprint, PageResult, Region, RegionKind, Regions,
    Stats, Table,
};
