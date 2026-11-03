// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The regions a paged reader found on its pages: per region its type, where it stands, how
//! sure the type is and what found it. They are listed in the model JSON only; the Markdown
//! does not show them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::inline::Anchor;
use crate::layout::{Certainty, Evidence};

/// One region of a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct RegionNote {
    /// Where the region stands in the body, one of:
    /// - `p<N>:b<K>` or `p<N>:t<K>`: the block or table it became;
    /// - `fn:<label>`: the footnote section a footnote legend became;
    /// - an anchor on an earlier page (`p<M>:t<K>`): the table whose rows it became, for a
    ///   tagged table continued from that page;
    /// - for a picture or a code, whose media block has no anchor, and for anything that
    ///   became no block: the block of the page nearest to it by place (the one after the
    ///   nearest region that stands above it, else that region's), which may be a neighbour
    ///   of the media block rather than the block right after it;
    /// - `p<N>`: the page, when it has no block at all (a scanned page).
    pub anchor: Anchor,
    /// The 1-based page.
    pub page: u32,
    pub kind: RegionKind,
    /// `x0, y0, x1, y1` in points, `y` upward, in the visible page box with its origin at the
    /// lower left corner and the page's `/Rotate` not applied; rounded to 0.01. On a page
    /// whose text was read in a turned frame this is the unturned box, unlike a
    /// [`crate::Barcode`]'s `box`, which is in the frame of the page's text.
    pub bounds: [f32; 4],
    pub certainty: Certainty,
    /// From 0 to 1000.
    pub score: u16,
    /// What found the region: a rule with its signal id and its words.
    pub evidence: Vec<Evidence>,
    /// Types that claimed the same parts and lost.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<RegionAlternative>,
}

/// A type that also claimed a region's parts and lost, with its score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct RegionAlternative {
    pub kind: RegionKind,
    pub score: u16,
}

/// The type of a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RegionKind {
    HeaderBand,
    FooterBand,
    TitleBlock,
    Heading,
    Paragraph,
    List,
    Table,
    LabelValue,
    Totals,
    Figure,
    Code,
    FootnoteLegend,
    Spatial,
}
