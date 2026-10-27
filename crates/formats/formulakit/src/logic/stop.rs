// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stop callback, asked every few thousand steps inside long loops.

/// How often the stop callback is asked inside long loops.
const STOP_EVERY: u32 = 4096;

pub(super) struct Stopper<'a> {
    pub(super) stop: &'a dyn Fn() -> bool,
    pub(super) ticks: u32,
    pub(super) stopped: bool,
}

impl Stopper<'_> {
    /// Asks the callback now.
    pub(super) fn check(&mut self) -> bool {
        if !self.stopped && (self.stop)() {
            self.stopped = true;
        }
        self.stopped
    }

    /// Asks the callback every [`STOP_EVERY`] calls.
    pub(super) fn tick(&mut self) -> bool {
        self.ticks += 1;
        if self.ticks >= STOP_EVERY {
            self.ticks = 0;
            return self.check();
        }
        self.stopped
    }
}
