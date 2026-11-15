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
//! into one tree of candidate regions per page. The [`typers`] score each segment for each
//! kind from named signals; [`arbitrate()`] picks one kind per segment by score and margin, and
//! leaves what it is unsure of `Spatial`, which [`extract::spatial::spatial`] reads line by
//! line.
//!
//! The line number of a run ([`TextRun::line`]) is the number of its line in reading order:
//! the host gives it, and the engine does not search it. So that the order the runs have in
//! a file decides no read, a host hands a page with its runs in the order of their places:
//! by line number, and inside a line by their place. [`RunOrder::of`] gives that page. That
//! order feeds the segmentation, the typers and the arbitration only. It is not the reading
//! order of the output: a host places the regions by its own flow.
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
mod canon;
pub mod extract;
pub mod file;
mod input;
mod limits;
pub mod line;
mod order;
mod region;
mod report;
pub mod segment;
pub mod typers;
mod whole;
mod work;

pub use arbitrate::{Decision, Finder, InOrder, Margins, Offer, arbitrate, find_in_order};
pub use canon::RunOrder;
pub use input::{
    BoxIn, CodeIn, FontKind, GapIn, GivenRegion, PageIn, PictureIn, PrimId, RuleIn, Shift, TextRun,
};
pub use limits::Limits;
#[doc(hidden)]
pub use order::with_place_work;
pub use order::{order, order_by_source};
pub use region::{
    Alternative, Bounds, Candidate, Certainty, Issue, IssueKind, Kind, PageRegions, Region,
    RegionId, Score, Signal,
};
pub use report::{count_by_kind, count_lines, moved_lines};
pub use whole::{Parted, part_lines};
pub use work::work_steps;
