// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::inline::{Anchor, Inline};

/// Block-level content of a document body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Section {
        kind: SectionKind,
        title: Option<String>,
        anchor: Option<Anchor>,
        blocks: Vec<Block>,
    },
    Heading {
        level: u8,
        inlines: Vec<Inline>,
        anchor: Option<Anchor>,
    },
    Paragraph {
        inlines: Vec<Inline>,
        /// The source style as a role, for example `caption` or `title`.
        role: Option<String>,
        anchor: Option<Anchor>,
    },
    List {
        ordered: bool,
        /// One entry per list item, each a sequence of blocks.
        items: Vec<Vec<Block>>,
    },
    Table {
        caption: Option<String>,
        header_rows: usize,
        rows: Vec<Vec<Cell>>,
        anchor: Option<Anchor>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    Quote {
        blocks: Vec<Block>,
    },
    Math {
        latex: String,
        /// Where the formula came from, for example `omml`, `mathml` or `latex`.
        source_kind: String,
    },
    /// A picture or other file, by [`crate::Asset`] id.
    Media {
        asset: String,
    },
    Callout {
        kind: CalloutKind,
        title: Option<String>,
        folded: Option<bool>,
        blocks: Vec<Block>,
    },
    Task {
        state: TaskState,
        inlines: Vec<Inline>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    Chapter,
    Sheet,
    Slide,
    Page,
    Message,
    Part,
    Layer,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CalloutKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Open,
    Done,
    Cancelled,
}

/// A table cell. Spans are at least 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Cell {
    pub inlines: Vec<Inline>,
    pub row_span: u32,
    pub col_span: u32,
    pub anchor: Option<Anchor>,
}
