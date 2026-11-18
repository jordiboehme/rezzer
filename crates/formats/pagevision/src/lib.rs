// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Page OCR with vision models.

#[cfg(feature = "detect")]
pub mod detect;
mod error;
pub mod ink;
#[cfg(feature = "llama")]
pub mod llama;
pub mod model;
pub mod prepare;
pub mod rules;
pub mod snap;
mod types;

pub use error::PageError;
pub use types::{
    BBox, Block, BlockKind, Cell, Content, Fingerprint, PageResult, Region, Stats, Table,
};
