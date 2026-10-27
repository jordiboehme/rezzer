// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A hidden command that runs programs on the grid of a read file.
//!
//! The crate is reachable only through the hidden subcommand. It never writes to the model or
//! to the output.

#![forbid(unsafe_code)]

pub mod app;
pub mod grid;
pub mod plan;
pub mod rng;
pub mod round;
mod run;
pub mod steer;
mod term;
pub mod ui;
mod work;

pub use plan::{Rounds, rounds};
pub use work::work_steps;

#[doc(hidden)]
pub use term::{Crossterm, Guard, Screen, install_restore_hook};

/// Runs `rounds` on the terminal until the user quits; round `n` uses the seed `seed + n`.
/// The terminal is restored on quit, on an error (returned as `terminal error: ...`) and on a
/// panic. The caller checks that stdin and stdout are terminals.
pub fn derez(rounds: Rounds, seed: u64) -> Result<(), String> {
    run::run(rounds, seed)
}
