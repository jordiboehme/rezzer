// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The held-out split: every file of a public set is in the half `dev` or in the half
//! `held`, by a hash of its name that never changes. Later tasks are told about `dev` files
//! only; a gain on `dev` with a loss on `held` is a finding.

use std::path::Path;

/// One half of a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Half {
    Dev,
    Held,
}

impl Half {
    pub const ALL: [Half; 2] = [Half::Dev, Half::Held];

    pub fn name(self) -> &'static str {
        match self {
            Half::Dev => "dev",
            Half::Held => "held",
        }
    }
}

/// The 64-bit FNV-1a hash of `bytes`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The name a file is split by: the first part of its path below the set (the file itself, or
/// the folder of a document or of a generated shape, so that every variant of a shape is in
/// one half), without the endings the tools add (`.facts`, `.json`, `.md`).
pub fn key(relative: &Path) -> String {
    let first = relative
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default();
    for ending in [".facts", ".json", ".md"] {
        if let Some(stem) = first.strip_suffix(ending) {
            return stem.to_string();
        }
    }
    first
}

/// The half of the file split by `key`: the parity of the bits of its hash.
pub fn half(key: &str) -> Half {
    if fnv1a(key.as_bytes()).count_ones().is_multiple_of(2) {
        Half::Dev
    } else {
        Half::Held
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_fnv_1a() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn a_file_and_its_reads_share_a_key() {
        assert_eq!(key(Path::new("a.pdf.facts")), "a.pdf");
        assert_eq!(key(Path::new("a.pdf.json")), "a.pdf");
        assert_eq!(key(Path::new("a.pdf")), "a.pdf");
        assert_eq!(key(Path::new("shape one/base.facts")), "shape one");
        assert_eq!(key(Path::new("shape one/courier.json")), "shape one");
    }

    #[test]
    fn the_split_is_stable_and_about_even() {
        // Pinned: a change of the split would move files between the halves.
        assert_eq!(half("a.pdf"), half("a.pdf"));
        let names: Vec<String> = (0..1000).map(|n| format!("file-{n}.pdf")).collect();
        let dev = names.iter().filter(|n| half(n) == Half::Dev).count();
        assert!((400..=600).contains(&dev), "{dev}");
        let pinned: Vec<&str> = ["a.pdf", "b.pdf", "c.pdf", "d.pdf"]
            .into_iter()
            .map(|n| half(n).name())
            .collect();
        assert_eq!(pinned, PINNED);
    }

    const PINNED: [&str; 4] = ["dev", "dev", "dev", "held"];
}
