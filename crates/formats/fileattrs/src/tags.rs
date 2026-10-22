// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tag names from macOS Finder tags and the freedesktop `user.xdg.tags` attribute.

use crate::bplist::string_array;

/// The tag names in a `com.apple.metadata:_kMDItemUserTags` attribute.
///
/// Each entry is a name, then a line feed and a colour number; the line feed and number are
/// removed. An entry without them is taken whole. Empty names and repeats are left out, the
/// order is kept. A value that is no readable string array gives no tags.
#[must_use]
pub fn finder_tags(bytes: &[u8]) -> Vec<String> {
    let Ok(entries) = string_array(bytes) else {
        return Vec::new();
    };
    let names = entries
        .into_iter()
        .map(|entry| match entry.rsplit_once('\n') {
            Some((name, colour)) if colour.bytes().all(|b| b.is_ascii_digit()) => name.to_owned(),
            _ => entry,
        });
    unique(names)
}

/// The tag names in a `user.xdg.tags` value: a comma-separated list.
///
/// Names are trimmed; empty names and repeats are left out, the order is kept. No escape for
/// a comma inside a name is known, so every comma separates.
#[must_use]
pub fn xdg_tags(text: &str) -> Vec<String> {
    unique(text.split(',').map(|name| name.trim().to_owned()))
}

fn unique(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}
