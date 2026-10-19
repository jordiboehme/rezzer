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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<Block>,
    },
    Heading {
        level: u8,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inlines: Vec<Inline>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    Paragraph {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inlines: Vec<Inline>,
        /// The source style as a role, for example `caption` or `title`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    List {
        #[serde(default)]
        ordered: bool,
        /// One entry per list item, each a sequence of blocks.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<Vec<Block>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    Table {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(default)]
        header_rows: usize,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rows: Vec<Vec<Cell>>,
        /// Column alignments; empty when the source has none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        alignments: Vec<Align>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    Code {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lang: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    Quote {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<Block>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    Math {
        latex: String,
        /// Where the formula came from, for example `omml`, `mathml` or `latex`.
        source_kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    /// A picture or other file, by [`crate::Asset`] id.
    Media {
        asset: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    Callout {
        kind: CalloutKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folded: Option<bool>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<Block>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    /// A thematic break.
    Rule {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    Task {
        state: TaskState,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inlines: Vec<Inline>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
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

/// Horizontal alignment of a table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Open,
    Done,
    Cancelled,
}

/// A table cell. Spans are at least 1; missing spans in JSON read as 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Cell {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inlines: Vec<Inline>,
    #[serde(default = "one")]
    pub row_span: u32,
    #[serde(default = "one")]
    pub col_span: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
}

fn one() -> u32 {
    1
}

impl Cell {
    /// A cell with spans of 1 and no anchor.
    pub fn new(inlines: Vec<Inline>) -> Cell {
        Cell {
            inlines,
            row_span: 1,
            col_span: 1,
            anchor: None,
        }
    }
}
