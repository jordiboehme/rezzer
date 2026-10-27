// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A hidden command that runs programs on the grid of a read file.
//!
//! The crate is reachable only through the hidden subcommand. It never writes to the model or
//! to the output.

#![forbid(unsafe_code)]

pub mod grid;
pub mod rng;
pub mod round;
