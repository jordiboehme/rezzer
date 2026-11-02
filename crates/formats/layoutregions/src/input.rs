// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use crate::region::{Bounds, Kind};

/// What a format reader hands over for one page. `y` grows upward.
#[derive(Debug, Clone, PartialEq)]
pub struct PageIn {
    pub page: u32,
    /// Width and height of the frame the primitives are in.
    pub size: [f32; 2],
    /// Quarter turns the reader applied, 0 to 3.
    pub turn: u8,
    pub texts: Vec<TextRun>,
    pub rules: Vec<RuleIn>,
    pub boxes: Vec<BoxIn>,
    pub pictures: Vec<PictureIn>,
    pub codes: Vec<CodeIn>,
    pub given: Vec<GivenRegion>,
}

/// One run of text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    /// Left edge.
    pub x: f32,
    /// Baseline.
    pub y: f32,
    /// Advance width.
    pub w: f32,
    /// Rendered height.
    pub h: f32,
    pub size: f32,
    /// Family key, 0 when not known.
    pub font: u32,
    pub font_kind: FontKind,
    pub bold: bool,
    pub italic: bool,
    pub color: Option<[u8; 3]>,
    pub shift: Shift,
    /// Line number in the reader's line order.
    pub line: u32,
    pub off_axis: bool,
    /// White space between words, x from and to.
    pub gaps: Vec<[f32; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontKind {
    Unknown,
    Proportional,
    Monospace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shift {
    Base,
    Raised,
    Lowered,
}

/// A line or a rectangle outline.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleIn {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub thick: f32,
    pub dashed: bool,
    pub rect: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoxIn {
    pub bounds: Bounds,
    pub filled: bool,
    pub stroked: bool,
    pub widget: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PictureIn {
    pub bounds: Bounds,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeIn {
    pub bounds: Bounds,
}

/// A region the file states itself.
#[derive(Debug, Clone, PartialEq)]
pub struct GivenRegion {
    pub kind: Kind,
    pub prims: Vec<PrimId>,
    pub order: u32,
}

/// A primitive of a page: the index into one of the lists of [`PageIn`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PrimId {
    Text(u32),
    Rule(u32),
    Box(u32),
    Picture(u32),
    Code(u32),
}
