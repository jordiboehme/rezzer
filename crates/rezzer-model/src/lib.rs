// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The document model every reader fills and the renderer reads, plus the [`Reader`] trait.
//!
//! The model is serialized as versioned JSON ([`MODEL_VERSION`]); names and shapes are part of
//! the contract with every reader and renderer.

mod block;
mod document;
mod facet;
mod format;
mod inline;
mod loss;
mod reader;
mod registry;
mod value;

pub use block::{Block, CalloutKind, Cell, SectionKind, TaskState};
pub use document::{
    Asset, Confidence, Detection, Document, MODEL_VERSION, Properties, ReaderInfo, SourceInfo,
};
pub use facet::{Annotation, Edge, Embedded, Event, Facets, Formula, Graph, Node, Role};
pub use format::{Completeness, FormatId};
pub use inline::{Anchor, Inline, StatusColor};
pub use loss::{Loss, LossReason, Severity};
pub use reader::{Bytes, ReadOptions, ReadOutcome, Reader, Source};
pub use registry::Registry;
pub use value::Value;
