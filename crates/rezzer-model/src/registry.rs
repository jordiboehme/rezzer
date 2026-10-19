// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{HashMap, HashSet};

use crate::{FormatId, Reader};

/// Maps a [`FormatId`] to the [`Reader`] that handles it.
///
/// When two readers claim the same format, the later registration wins.
#[derive(Default)]
pub struct Registry {
    readers: Vec<Box<dyn Reader>>,
    by_format: HashMap<FormatId, usize>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Registers a reader for every format it declares. A format that already has a reader
    /// is taken over by this one.
    pub fn register(&mut self, reader: Box<dyn Reader>) {
        let index = self.readers.len();
        for format in reader.formats() {
            self.by_format.insert(*format, index);
        }
        self.readers.push(reader);
    }

    pub fn for_format(&self, f: FormatId) -> Option<&dyn Reader> {
        self.by_format.get(&f).map(|i| self.readers[*i].as_ref())
    }

    /// One entry per registered format: (format, reader id, reader version), ordered by
    /// registration and then by the reader's own format order.
    pub fn list(&self) -> Vec<(FormatId, &'static str, &'static str)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (index, reader) in self.readers.iter().enumerate() {
            for format in reader.formats() {
                if self.by_format.get(format) == Some(&index) && seen.insert(*format) {
                    out.push((*format, reader.id(), reader.version()));
                }
            }
        }
        out
    }
}
