// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tag names from macOS Finder tags and the freedesktop `user.xdg.tags` attribute.

use std::collections::HashSet;

use crate::bplist::string_array;

/// At most this many tag names are returned; the rest are not looked at.
pub const MAX_TAGS: usize = 64;

/// The tag names in a `com.apple.metadata:_kMDItemUserTags` attribute.
///
/// Each entry is a name, then a line feed and a colour number; the line feed and number are
/// removed. An entry without them is taken whole. Empty names and repeats are left out, the
/// order is kept, at most [`MAX_TAGS`] names. A value that is no readable string array gives
/// no tags.
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
/// Names are trimmed; empty names and repeats are left out, the order is kept, at most
/// [`MAX_TAGS`] names. No escape for a comma inside a name is known, so every comma
/// separates.
#[must_use]
pub fn xdg_tags(text: &str) -> Vec<String> {
    unique(text.split(',').map(|name| name.trim().to_owned()))
}

/// The first [`MAX_TAGS`] distinct non-empty names, in order. Stops reading at the cap.
fn unique(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for name in names {
        if out.len() == MAX_TAGS {
            break;
        }
        if !name.is_empty() && seen.insert(name.clone()) {
            out.push(name);
        }
    }
    out
}
