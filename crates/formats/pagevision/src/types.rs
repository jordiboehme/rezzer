// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Public result types.

/// A box on the 0..=1000 grid of the page (x0 <= x1, y0 <= y1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BBox {
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BlockKind {
    Paragraph,
    Heading,
    List,
    Diagram,
    Table,
    Picture,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Cell {
    pub text: String,
    pub row_span: u16,
    pub col_span: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Table {
    pub rows: Vec<Vec<Cell>>,
    pub header_rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Content {
    Heading {
        level: u8,
        text: String,
    },
    /// Inline Markdown marks allowed.
    Paragraph {
        text: String,
    },
    List {
        items: Vec<String>,
    },
    Table(Table),
    Diagram {
        mermaid: String,
        description: String,
    },
    Picture {
        description: String,
    },
    /// Content the crate could not parse into its kind; `reason` says why.
    Text {
        text: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Block {
    pub bbox: BBox,
    pub certainty: f32,
    pub content: Content,
    pub markdown: String,
}

impl Block {
    /// The kind that goes with the content.
    pub fn kind(&self) -> BlockKind {
        match &self.content {
            Content::Heading { .. } => BlockKind::Heading,
            Content::Paragraph { .. } => BlockKind::Paragraph,
            Content::List { .. } => BlockKind::List,
            Content::Table(_) => BlockKind::Table,
            Content::Diagram { .. } => BlockKind::Diagram,
            Content::Picture { .. } => BlockKind::Picture,
            Content::Text { .. } => BlockKind::Text,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Stats {
    pub prompt_tokens: u32,
    pub image_tokens: u32,
    pub output_tokens: u32,
    pub encode_ms: u64,
    pub decode_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Fingerprint {
    pub crate_version: String,
    pub llama_cpp_2: String,
    pub model_sha256: Vec<String>,
    pub prompt_sha256: String,
    pub preprocess: u32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PageResult {
    pub width_px: u32,
    pub height_px: u32,
    pub blocks: Vec<Block>,
    pub markdown: String,
    pub truncated: bool,
    pub stats: Stats,
    pub fingerprint: Fingerprint,
    pub raw: Option<String>,
}

/// A region given by the caller; the crate then skips the layout pass for this page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Region {
    pub bbox: BBox,
    pub kind_hint: Option<BlockKind>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(content: Content) -> Block {
        Block {
            bbox: BBox {
                x0: 0,
                y0: 0,
                x1: 1000,
                y1: 1000,
            },
            certainty: 1.0,
            content,
            markdown: String::new(),
        }
    }

    #[test]
    fn block_kind_follows_content() {
        let cases = [
            (
                Content::Heading {
                    level: 1,
                    text: "h".into(),
                },
                BlockKind::Heading,
            ),
            (
                Content::Paragraph { text: "p".into() },
                BlockKind::Paragraph,
            ),
            (
                Content::List {
                    items: vec!["a".into()],
                },
                BlockKind::List,
            ),
            (
                Content::Table(Table {
                    rows: vec![],
                    header_rows: 0,
                }),
                BlockKind::Table,
            ),
            (
                Content::Diagram {
                    mermaid: "graph TD".into(),
                    description: "d".into(),
                },
                BlockKind::Diagram,
            ),
            (
                Content::Picture {
                    description: "d".into(),
                },
                BlockKind::Picture,
            ),
            (
                Content::Text {
                    text: "t".into(),
                    reason: "r".into(),
                },
                BlockKind::Text,
            ),
        ];
        for (content, kind) in cases {
            assert_eq!(block(content).kind(), kind);
        }
    }
}
