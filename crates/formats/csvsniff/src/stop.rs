// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The text as a reader that asks a stop callback while one record is read, so a single
//! long record cannot run past a time limit.

use std::cell::Cell;
use std::io::{self, Read};

/// Bytes of the text between two calls of the stop callback, also inside one record.
const STOP_CHECK_BYTES: usize = 64 * 1024;

/// Hands out the text and asks `stop` after each [`STOP_CHECK_BYTES`]. Once `stop` says yes,
/// it fails every read: the CSV reader then drops the record it was reading, and
/// [`StopRead::stopped`] is set.
pub(crate) struct StopRead<'a> {
    rest: &'a [u8],
    stop: &'a dyn Fn() -> bool,
    stopped: &'a Cell<bool>,
    unchecked: usize,
}

impl<'a> StopRead<'a> {
    pub(crate) fn new(text: &'a str, stop: &'a dyn Fn() -> bool, stopped: &'a Cell<bool>) -> Self {
        StopRead {
            rest: text.as_bytes(),
            stop,
            stopped,
            unchecked: 0,
        }
    }
}

impl Read for StopRead<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.stopped.get() {
            return Err(io::Error::other("stopped"));
        }
        if self.rest.is_empty() {
            return Ok(0);
        }
        if self.unchecked >= STOP_CHECK_BYTES {
            self.unchecked = 0;
            if (self.stop)() {
                self.stopped.set(true);
                return Err(io::Error::other("stopped"));
            }
        }
        let n = buf.len().min(self.rest.len());
        let (now, rest) = self.rest.split_at(n);
        buf[..n].copy_from_slice(now);
        self.rest = rest;
        self.unchecked += n;
        Ok(n)
    }
}
