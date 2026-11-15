// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the regions of a page read as: the blocks of text lines (headings, list items and
//! paragraphs with their line breaks), the cells of tables, and the spatial reading of text
//! whose kind is not certain.

pub mod pairs;
pub mod spatial;
pub mod table;
pub mod text;
