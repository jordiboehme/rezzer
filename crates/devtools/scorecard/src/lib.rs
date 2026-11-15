// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The scorecard: a measure of Rezzer's reads against truth.
//!
//! An assertion file states relational facts about one document, one per line ([`facts`]).
//! The checker ([`check`]) asks each fact of Rezzer's model JSON of that document ([`doc`])
//! and counts, per set, half and class, how many hold ([`report`]). Truth comes from the
//! files themselves ([`generate`]): the tags of a tagged PDF, read once with them and scored
//! with them taken out ([`strip`]); the invoice XML a PDF carries; the filled fields of a
//! form. Every public set is split into a `dev` and a `held` half by a hash of the file name
//! ([`split`]). [`trace`] names the path of the reader that made a region, with no text.
//!
//! This is a development tool. It is never linked into the `rezzer` binary.

pub mod check;
pub mod doc;
pub mod facts;
pub mod generate;
pub mod report;
pub mod split;
pub mod strip;
pub mod text;
pub mod trace;
