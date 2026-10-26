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
mod layout;
mod loss;
mod reader;
mod registry;
mod sheet;
mod value;

pub use block::{Align, Block, CalloutKind, Cell, SectionKind, TaskState, column_letter};
pub use document::{
    Asset, Barcode, Confidence, Detection, Document, MODEL_VERSION, Overview, OverviewPart,
    Properties, ReaderInfo, SourceInfo,
};
pub use facet::{
    Annotation, Edge, Embedded, Event, Facets, FieldAction, FileFacts, FormField, Formula,
    FormulaKind, Graph, Node, OutlineEntry, Parameter, Role, Rule, Signature,
};
pub use format::{Completeness, FormatId};
pub use inline::{Anchor, Inline, StatusColor, link_target_written_len, plain_text};
pub use layout::{
    Certainty, Evidence, LayoutKind, RowKind, RowNote, RowRange, SheetCell, TableLayout,
    a1_position, join_ranges, sheet_cell, split_rows, table_key,
};
pub use loss::{Loss, LossReason, STALE_RESULTS, Severity};
pub use reader::{Blob, Bytes, ReadOptions, ReadOutcome, Reader, Source, source_info};
pub use registry::Registry;
pub use sheet::{CellArea, locate_cells, table_headings};
pub use value::Value;
