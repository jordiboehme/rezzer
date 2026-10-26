// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reads where a file came from, from metadata the file system keeps beside it.
//!
//! - [`string_array`] reads a binary property list (`bplist00`) holding an array of
//!   strings, the shape of the macOS attributes `com.apple.metadata:kMDItemWhereFroms`
//!   (download addresses) and `com.apple.metadata:_kMDItemUserTags` (Finder tags)
//! - [`finder_tags`] and [`xdg_tags`] turn the macOS and freedesktop tag attributes into
//!   tag names
//! - [`zone_text`] and [`zone_host_url`] read the Windows `Zone.Identifier` stream
//! - [`read_xattr`] reads one extended attribute of an open file, with a size cap
//!
//! Every reader is bounded and never panics on hostile input. The crate has no dependency
//! on the rest of Rezzer.

mod bplist;
mod tags;
#[cfg(all(
    feature = "test-support",
    any(target_os = "macos", target_os = "linux")
))]
pub mod test_support;
#[cfg(unix)]
mod xattr;
mod zone;

pub use bplist::{BplistError, MAX_DEPTH, MAX_ITEMS, MAX_TEXT_BYTES, MAX_VISITS, string_array};
pub use tags::{MAX_TAGS, finder_tags, xdg_tags};
#[cfg(unix)]
pub use xattr::read_xattr;
pub use zone::{zone_host_url, zone_text};

/// A size cap for [`read_xattr`] that fits every attribute this crate reads.
pub const MAX_ATTR_BYTES: usize = 64 * 1024;
