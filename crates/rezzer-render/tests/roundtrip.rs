// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Markdown in, Rezzer Markdown out, read again: the model stays the same.

use std::sync::Arc;

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Block, Bytes, Completeness, Confidence, Detection, Document, FormatId, ReadOptions, Reader,
    Source,
};
use rezzer_read_markdown::MarkdownReader;
use rezzer_render::{OutputSection, RenderOptions, render};

fn read(text: &str) -> Document {
    let src = Source {
        path: None,
        name: "note.md".into(),
        bytes: Bytes::Owned(Arc::new(text.as_bytes().to_vec())),
        sha256: "00".repeat(32),
        size: text.len() as u64,
        format: FormatId::Markdown,
        detection: Detection {
            confidence: Confidence::Likely,
            evidence: "md".into(),
        },
        modified: None,
    };
    MarkdownReader
        .read(
            &src,
            &ReadOptions::default(),
            &Budget::new(Limits::default()),
        )
        .document
}

/// The body part of Rezzer's output: after the frontmatter and the title line.
fn rendered_body(doc: &Document) -> String {
    let md = render(
        doc,
        Completeness::Complete,
        &RenderOptions {
            sections: Some(vec![OutputSection::Content]),
            generated_at: "2026-10-20T12:00:00Z".into(),
            rezzer_version: "0.1.0".into(),
            archive_ref: None,
            ..RenderOptions::default()
        },
    );
    let rest = md.strip_prefix("---\n").expect("frontmatter");
    let end = rest.find("\n---\n").expect("frontmatter end");
    let after = &rest[end + 5..];
    let after = after.trim_start_matches('\n');
    // The title line Rezzer always writes.
    let after = after.split_once('\n').map_or("", |(_, b)| b);
    after.trim_start_matches('\n').to_string()
}

/// Reads `text`, renders it, reads the result and returns both bodies and the Markdown.
fn round_trip(text: &str) -> (Vec<Block>, Vec<Block>, String) {
    let first = read(text);
    let md = rendered_body(&first);
    let second = read(&md);
    (first.body, second.body, md)
}

#[test]
fn task_lists_round_trip() {
    let (first, second, md) = round_trip(
        "- [ ] open\n- [x] done\n- plain\n\n1. [ ] first\n2. [x] second\n\n- [ ] parent\n  - [x] child\n",
    );
    assert!(md.contains("- [ ] open\n- [x] done\n- plain"), "{md}");
    assert!(md.contains("1. [ ] first\n2. [x] second"), "{md}");
    assert!(md.contains("- [ ] parent\n  - [x] child"), "{md}");
    assert!(!md.contains("- - ["), "{md}");
    assert_eq!(first, second, "{md}");
}

#[test]
fn inline_math_round_trips() {
    let (first, second, md) = round_trip("Area $`a^2`$ and $x + y$ and $$`b`$$.\n");
    assert!(md.contains("$`a^2`$"), "{md}");
    assert!(md.contains("$`x + y`$"), "{md}");
    assert_eq!(first, second, "{md}");
}
