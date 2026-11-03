// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The segmentation of a page: what its drawing and its text part it into. [`drawn`] finds
//! the grids and boxes the drawn lines make, once per page.

pub mod cut;
pub mod drawn;
pub mod style;
