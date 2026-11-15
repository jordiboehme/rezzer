// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::inline::{Anchor, Inline};
use crate::layout::TableLayout;

/// One line of a [`Block::Spatial`]: its pieces, left to right.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SpatialLine {
    #[serde(default)]
    pub pieces: Vec<SpatialPiece>,
}

/// A piece of text on a spatial line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SpatialPiece {
    /// Character column (Unicode scalar values, not bytes) where the piece starts, counted from
    /// the left edge of its block (of its run for a block of a run, see
    /// [`SpatialGrid::joins`]), no tabs. The Markdown does not align the pieces: it writes the lines
    /// one by one, a wider gap before a piece that stands apart from the one before it, and
    /// the indent of a line against the leftmost line of its block. Only
    /// blocks that follow one another in the same columns ([`spatial_runs`]) are written
    /// aligned, as one fenced block. A piece is one line: a line break in its text is written
    /// as a space.
    pub col: u32,
    pub text: String,
    /// The parts of `text` that carry more than their text, left to right, none overlapping:
    /// a link, a footnote reference, emphasis. Empty for plain text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<SpatialSpan>,
}

/// A part of the text of a [`SpatialPiece`] that carries more than its text. The text stays
/// in the piece as laid out (a footnote marker too); the span says what it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpatialSpan {
    /// Character offsets (Unicode scalar values) into the piece's text: from `start` up to,
    /// not including, `end`.
    pub start: u32,
    pub end: u32,
    /// The link it is: an index into the `links` of its block, so a target long or often
    /// linked is held once per block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<u32>,
    /// The footnote it refers to, as [`Inline::FootnoteRef`] does: the label of the footnote
    /// (the section anchored `fn:{label}`). The span covers the marker as printed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footnote: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub emphasis: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub strong: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Where a [`Block::Spatial`] stands on the grid of its page. A reader that knows the place
/// of the block on its page gives it; without it the block is no row of a run
/// ([`spatial_runs`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpatialGrid {
    /// The column of the page grid where the block's left edge (its column 0) stands. The
    /// grid counts from the leftmost text of the page in the character width of the page's
    /// text, so it is the same at every scale and place of the page.
    pub col: u32,
    /// The block is a further row of the block kept as laid out before it: it starts in the
    /// same column of the page grid, right under that block. The columns of the pieces of
    /// such a run (its first block too) are counted on one grid: from the leftmost edge of
    /// its blocks, in the smallest character width among them, so they stand in common
    /// columns.
    #[serde(default, skip_serializing_if = "is_false")]
    pub joins: bool,
}

/// The runs of blocks kept as laid out among `blocks` that stand in the same columns: two or
/// more [`Block::Spatial`] that follow one another (no other block between, [`Block::Media`]
/// aside), each after the first one a further row of the block before it
/// ([`SpatialGrid::joins`]) at the same column of the page grid. That is a small table cut
/// into regions (a row of labels over a row of values): which piece stands under which is
/// meaning, so the Markdown writes a run as one block aligned in columns and a reader gives
/// it one picture. The reader decides a run, where the places on the page are known: a
/// block without a grid is in none. Each run holds the indices of its spatial blocks, in
/// order.
pub fn spatial_runs(blocks: &[Block]) -> Vec<Vec<usize>> {
    fn close(run: &mut Vec<usize>, runs: &mut Vec<Vec<usize>>) {
        let run = std::mem::take(run);
        if run.len() > 1 {
            runs.push(run);
        }
    }
    let mut runs = Vec::new();
    let mut run: Vec<usize> = Vec::new();
    // The grid column of the run so far.
    let mut col = None;
    for (k, block) in blocks.iter().enumerate() {
        let grid = match block {
            Block::Media { .. } => continue,
            Block::Spatial { grid, .. } => *grid,
            _ => None,
        };
        if !grid.is_some_and(|g| g.joins && col == Some(g.col)) {
            close(&mut run, &mut runs);
        }
        if grid.is_some() {
            run.push(k);
        }
        col = grid.map(|g| g.col);
    }
    close(&mut run, &mut runs);
    runs
}

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
        /// What the table is for when it is not source data, for example `summary` for a table
        /// a reader generated. Passes that read the document's own words skip tables with a role.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rows: Vec<Vec<Cell>>,
        /// Column alignments; empty when the source has none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        alignments: Vec<Align>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
        /// The layout of a sheet region; boxed so a block stays small.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout: Option<Box<TableLayout>>,
    },
    Code {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lang: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
    /// Text kept as laid out: a region no type had clear evidence for. The lines hold the
    /// layout (the column of each piece); the Markdown writes them as plain lines, or as one
    /// aligned block for a run of such blocks in the same columns ([`spatial_runs`]).
    Spatial {
        /// Top to bottom.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lines: Vec<SpatialLine>,
        /// The link targets its spans point to, each once.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        links: Vec<String>,
        /// Where the block stands on the grid of its page, when the reader knows it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grid: Option<SpatialGrid>,
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
        /// What this place shows, when one asset stands at several places: the words the
        /// place is written with instead of the asset's alt text, for example its box.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alt: Option<String>,
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
    /// A list item with a check box. Readers put it as the first block of a `List` item (the
    /// item's text; further blocks of the item follow it). A run of Task blocks outside a list
    /// is a task list too. Either way it renders as `- [ ] text`.
    Task {
        state: TaskState,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inlines: Vec<Inline>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Anchor>,
    },
}

impl Block {
    /// A paragraph without role or anchor.
    pub fn paragraph(inlines: Vec<Inline>) -> Block {
        Block::Paragraph {
            inlines,
            role: None,
            anchor: None,
        }
    }
}

/// Spreadsheet column letters for a 0-based column: 0 is `A`, 25 is `Z`, 26 is `AA`.
pub fn column_letter(mut index: usize) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push(char::from(b'A' + (index % 26) as u8));
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    letters.iter().rev().collect()
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
    /// Pictures that stand in the cell, after its text: a picture in each row of an item
    /// table. Each names its [`crate::Asset`] as [`Block::Media`] does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<CellMedia>,
}

/// A picture in a table cell, by [`crate::Asset`] id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct CellMedia {
    pub asset: String,
    /// What this place shows, when one asset stands at several places (see
    /// [`Block::Media`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

fn one() -> u32 {
    1
}

impl Cell {
    /// A cell with spans of 1, no anchor and no pictures.
    pub fn new(inlines: Vec<Inline>) -> Cell {
        Cell {
            inlines,
            row_span: 1,
            col_span: 1,
            anchor: None,
            media: Vec::new(),
        }
    }
}
