// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for the block writer.

use super::{Mode, destination_char, inline_into};
use rezzer_model::Inline;

#[test]
fn a_destination_never_takes_more_than_the_model_says() {
    let chars = (0..0x3000u32)
        .filter_map(char::from_u32)
        .chain(['\u{1F600}', '\u{10FFFF}']);
    for c in chars {
        let mut out = String::new();
        destination_char(c, &mut out);
        let s = c.to_string();
        assert!(
            out.len() as u64 <= rezzer_model::link_target_written_len(&s),
            "{c:?} is written as {out:?}"
        );
    }
}

#[test]
fn a_whole_link_never_takes_more_than_the_model_says_plus_its_overhead() {
    // Both written forms (destination and code span), block and cell mode, wiki on and
    // off. The overhead is what the reader charges besides the target: 64 bytes.
    let prefixes = ["", "http://x/", "javascript:", "mailto:"];
    let pieces = (0..0x300u32)
        .filter_map(char::from_u32)
        .chain(['\u{1F600}', '\u{10FFFF}']);
    for c in pieces {
        for prefix in prefixes {
            let target = format!("{prefix}{}", c.to_string().repeat(40));
            for mode in [Mode::Block, Mode::Cell] {
                for wiki in [false, true] {
                    let link = Inline::Link {
                        target: target.clone(),
                        title: None,
                        internal: false,
                        wiki,
                        inlines: Vec::new(),
                    };
                    let mut out = String::new();
                    inline_into(&link, mode, 0, false, &mut out);
                    let allowed = rezzer_model::link_target_written_len(&target) + 64;
                    assert!(
                        out.len() as u64 <= allowed,
                        "{target:?} in {mode:?} wiki {wiki} is written as {out:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_link_target_in_json_never_takes_more_than_the_model_says() {
    let link = |target: String| Inline::Link {
        target,
        title: None,
        internal: false,
        wiki: false,
        inlines: Vec::new(),
    };
    let size = |target: String| {
        serde_json::to_string_pretty(&link(target))
            .expect("the model serializes")
            .len() as u64
    };
    let empty = size(String::new());
    let pieces = (0..0x300u32)
        .filter_map(char::from_u32)
        .chain(['\u{1F600}', '\u{10FFFF}']);
    for c in pieces {
        for prefix in ["", "http://x/", "javascript:"] {
            let target = format!("{prefix}{}", c.to_string().repeat(40));
            let written = size(target.clone()) - empty;
            assert!(
                written <= rezzer_model::link_target_written_len(&target),
                "{target:?} is written as {written} bytes in JSON"
            );
        }
    }
}
