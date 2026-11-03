// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The region engine: the input of a page, the regions found on it and the arbitration that
//! gives every piece to one region.
//!
//! A format reader fills a [`PageIn`] (text runs, rules, boxes, pictures, codes and regions the
//! file states). Finders offer [`Candidate`]s to [`InOrder`], which keeps the first claim on a
//! piece. [`find_in_order`] runs the finders in order and [`InOrder::finish`] returns the
//! [`PageRegions`]. [`segment::drawn::drawn`] finds the grids and boxes the drawn lines of a
//! page make, once per page, for every finder that looks at them. [`segment::segment_page`]
//! nests the regions the file states, the drawn structure, the whitespace cut and style steps
//! into one tree of candidate regions per page. Later stages (the scored rule, the typing
//! functions) arrive in further steps.
//!
//! ```
//! use layoutregions::*;
//!
//! let page = PageIn {
//!     page: 0,
//!     size: [600.0, 800.0],
//!     turn: 0,
//!     texts: Vec::new(),
//!     rules: Vec::new(),
//!     boxes: Vec::new(),
//!     pictures: Vec::new(),
//!     codes: Vec::new(),
//!     given: Vec::new(),
//! };
//! let mut arb = InOrder::new(&page, &Limits::default());
//! let c = Candidate {
//!     kind: Kind::Paragraph,
//!     prims: vec![PrimId::Text(0)],
//!     bounds: Bounds { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 },
//!     score: Score(500),
//!     signals: Vec::new(),
//!     host: 0,
//! };
//! assert_eq!(arb.offer(c), Offer::Accepted(RegionId(0)));
//! assert_eq!(arb.finish().regions.len(), 1);
//! ```

mod arbitrate;
pub mod file;
mod input;
mod limits;
pub mod line;
mod region;
mod report;
pub mod segment;
mod work;

pub use arbitrate::{Finder, InOrder, Offer, find_in_order};
pub use input::{
    BoxIn, CodeIn, FontKind, GapIn, GivenRegion, PageIn, PictureIn, PrimId, RuleIn, Shift, TextRun,
};
pub use limits::Limits;
pub use region::{
    Alternative, Bounds, Candidate, Certainty, Issue, IssueKind, Kind, PageRegions, Region,
    RegionId, Score, Signal,
};
pub use report::{count_by_kind, count_lines, moved_lines};
pub use work::work_steps;
