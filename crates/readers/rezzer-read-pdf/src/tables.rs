// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables of a page, in the order the table spike found reliable: the tag tree of a tagged
//! file first, then a grid of drawn rules, then white space gutters in text without rules.
//! Each detector only claims the runs of a table and plans its cells; the layout builds the
//! cells' text, so every limit of the layout holds for tables too.

/// A drawn line or rectangle, in points in the visible page box like the runs, `y` upward.
/// A line has `x0 == x1` or `y0 == y1`; a rectangle is its bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rule {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub kind: RuleKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RuleKind {
    Line,
    Rect,
}
