// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A rectangle; `y` grows upward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

/// The type of a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
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

impl Kind {
    /// Every kind, in `Kind` order.
    pub const ALL: [Kind; 13] = [
        Kind::HeaderBand,
        Kind::FooterBand,
        Kind::TitleBlock,
        Kind::Heading,
        Kind::Paragraph,
        Kind::List,
        Kind::Table,
        Kind::LabelValue,
        Kind::Totals,
        Kind::Figure,
        Kind::Code,
        Kind::FootnoteLegend,
        Kind::Spatial,
    ];

    /// The name in snake case: `header_band`, `label_value` and so on.
    pub fn name(self) -> &'static str {
        match self {
            Kind::HeaderBand => "header_band",
            Kind::FooterBand => "footer_band",
            Kind::TitleBlock => "title_block",
            Kind::Heading => "heading",
            Kind::Paragraph => "paragraph",
            Kind::List => "list",
            Kind::Table => "table",
            Kind::LabelValue => "label_value",
            Kind::Totals => "totals",
            Kind::Figure => "figure",
            Kind::Code => "code",
            Kind::FootnoteLegend => "footnote_legend",
            Kind::Spatial => "spatial",
        }
    }
}

/// How a region was decided: given, decided by a rule, or spatial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Certainty {
    Exact,
    Rule,
    Unsure,
}

/// A score from 0 to 1000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Score(pub u16);

/// One named piece of evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub id: &'static str,
    pub detail: String,
    pub weight: u16,
}

/// A type that also claimed the same pieces and lost.
#[derive(Debug, Clone, PartialEq)]
pub struct Alternative {
    pub kind: Kind,
    pub score: Score,
    pub signals: Vec<Signal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub id: RegionId,
    pub page: u32,
    pub kind: Kind,
    pub bounds: Bounds,
    pub prims: Vec<crate::PrimId>,
    pub certainty: Certainty,
    pub score: Score,
    pub signals: Vec<Signal>,
    pub alternatives: Vec<Alternative>,
    pub parent: Option<RegionId>,
    pub unsure: Vec<String>,
    /// Opaque handle the host maps back to its own result.
    pub host: u32,
}

impl Region {
    /// A region from a candidate: kind, pieces, bounds, score, signals and host are copied;
    /// no parent, no alternatives and no unsure notes.
    pub fn decided(id: RegionId, page: u32, c: Candidate, certainty: Certainty) -> Region {
        Region {
            id,
            page,
            kind: c.kind,
            bounds: c.bounds,
            prims: c.prims,
            certainty,
            score: c.score,
            signals: c.signals,
            alternatives: Vec::new(),
            parent: None,
            unsure: Vec::new(),
            host: c.host,
        }
    }
}

/// What a finder offers for a region.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub kind: Kind,
    pub prims: Vec<crate::PrimId>,
    pub bounds: Bounds,
    pub score: Score,
    pub signals: Vec<Signal>,
    pub host: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// A limit was hit.
    Limit,
    /// The stop callback said stop.
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub what: String,
    pub kind: IssueKind,
}

/// The regions of one page, in the order they were accepted.
#[derive(Debug, Clone, PartialEq)]
pub struct PageRegions {
    pub page: u32,
    pub regions: Vec<Region>,
    pub issues: Vec<Issue>,
}
