// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Block, Bytes, Completeness, Confidence, Detection, FormatId, Inline, LossReason, ReadOptions,
    Reader, SectionKind, Severity, Source,
};
use rezzer_read_text::TextReader;

fn source(bytes: &[u8]) -> Source {
    Source {
        path: None,
        name: "a.txt".into(),
        bytes: Bytes::Owned(Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format: FormatId::Text,
        detection: Detection {
            confidence: Confidence::Likely,
            evidence: "text".into(),
        },
        modified: None,
    }
}

fn read(bytes: &[u8]) -> rezzer_model::ReadOutcome {
    TextReader.read(
        &source(bytes),
        &ReadOptions::default(),
        &Budget::new(Limits::default()),
    )
}

fn paragraphs(out: &rezzer_model::ReadOutcome) -> Vec<String> {
    let [Block::Section { kind, blocks, .. }] = out.document.body.as_slice() else {
        panic!("expected one section, got {:?}", out.document.body);
    };
    assert_eq!(*kind, SectionKind::Other);
    blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph { inlines, .. } => match inlines.as_slice() {
                [Inline::Text { text, .. }] => text.clone(),
                other => panic!("unexpected inlines {other:?}"),
            },
            other => panic!("unexpected block {other:?}"),
        })
        .collect()
}

#[test]
fn identity() {
    assert_eq!(TextReader.id(), "text");
    assert_eq!(TextReader.formats(), &[FormatId::Text]);
}

#[test]
fn paragraphs_split_on_blank_lines() {
    let out = read(b"one\ntwo\n\nthree\n\n\n  \nfour");
    assert_eq!(paragraphs(&out), vec!["one\ntwo", "three", "four"]);
    assert_eq!(out.completeness, Completeness::Complete);
    assert!(out.document.losses.is_empty());
}

#[test]
fn line_endings_are_normalized() {
    let out = read(b"a\r\nb\r\n\r\nc\rd\r\re");
    assert_eq!(paragraphs(&out), vec!["a\nb", "c\nd", "e"]);
}

#[test]
fn encoding_recorded_in_source() {
    let out = read(&[0x93, 0x48, 0x69, 0x94]);
    assert_eq!(
        out.document.source.encoding.as_deref(),
        Some("windows-1252")
    );
    assert_eq!(paragraphs(&out), vec!["\u{201C}Hi\u{201D}"]);
    let reader = out.document.source.reader.as_ref().unwrap();
    assert_eq!(reader.id, "text");
    assert_eq!(reader.version, TextReader.version());
    assert_eq!(out.document.source.name, "a.txt");
}

#[test]
fn undecodable_bytes_give_a_damaged_warning() {
    let out = read(&[0xFF, 0xFE, 0x00, 0xD8]);
    assert_eq!(out.document.losses.len(), 1);
    let loss = &out.document.losses[0];
    assert_eq!(loss.why, LossReason::Damaged);
    assert_eq!(loss.severity, Severity::Warning);
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn empty_file_gives_an_empty_section() {
    let out = read(b"");
    assert!(paragraphs(&out).is_empty());
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn time_limit_returns_partial() {
    let text = "p\n\n".repeat(2500);
    let budget = Budget::new(Limits::default());
    budget.cancel();
    let out = TextReader.read(&source(text.as_bytes()), &ReadOptions::default(), &budget);
    assert_eq!(out.completeness, Completeness::Partial);
    assert_eq!(paragraphs(&out).len(), 1000);
    let loss = out.document.losses.last().unwrap();
    assert_eq!(loss.location, "paragraph 1001");
    assert_eq!(loss.what, "rest of the text not read");
    assert_eq!(loss.why, LossReason::Limit);
    assert_eq!(loss.severity, Severity::Warning);
}
