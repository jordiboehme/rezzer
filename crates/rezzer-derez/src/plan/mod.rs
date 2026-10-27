// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Grid plans: shapes in unit coordinates that [`fit()`] maps to a grid of any size.

mod builtin;
mod fit;
mod source;

pub use builtin::builtin;
pub use fit::{
    DOOR, Fitted, MARGIN, MAX_GRID, MAX_SHAPES, MAX_WALL_SHARE, MIN_GRID, Refused, START_RUN, fit,
};
pub use source::{MAX_PLANS, Rounds, plans, rounds};

/// A resolution-free grid plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// What the plan was made from, for the status line.
    pub label: String,
    pub shapes: Vec<Shape>,
}

/// One shape of a plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    /// `x0, y0, x1, y1` in `0..=1`, y downward.
    pub rect: [f32; 4],
}

/// What a shape becomes on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    /// One row (when wider than tall) or one column of wall through the centre of the rect.
    Line,
    /// The border of the rect as wall, with doors.
    Outline,
    /// The rect filled with wall.
    Block,
    /// The rect filled with tower.
    Tower,
}
