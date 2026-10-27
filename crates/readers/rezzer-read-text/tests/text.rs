// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use rezzer_limits::{Budget, CancelToken, Clock, Limits};
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
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
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
            Block::Paragraph { inlines, .. } => inlines
                .iter()
                .map(|i| match i {
                    Inline::Text { text, .. } => text.as_str(),
                    Inline::LineBreak => "\n",
                    other => panic!("unexpected inline {other:?}"),
                })
                .collect::<String>(),
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

#[test]
fn one_huge_paragraph_is_time_checked() {
    let text = "x\n".repeat(600_000);
    let budget = Budget::new(Limits {
        max_time: std::time::Duration::ZERO,
        ..Limits::default()
    });
    let out = TextReader.read(&source(text.as_bytes()), &ReadOptions::default(), &budget);
    assert_eq!(out.completeness, Completeness::Partial);
    let loss = out.document.losses.last().unwrap();
    assert_eq!(loss.why, LossReason::Limit);
    assert_eq!(loss.location, "paragraph 1");
    let got = paragraphs(&out);
    assert_eq!(got.len(), 1);
    assert!(got[0].len() < text.len());
}

#[test]
fn one_huge_line_is_time_checked_inside() {
    // One line of 1 MiB after a short paragraph. The time is checked every 64 KiB of the
    // line: the third check is past the limit, so three blocks of 64 KiB are kept.
    let mut text = String::from("Muster GmbH\n\n");
    text.push_str(&"é".repeat(512 * 1024));
    let clock = Clock::steps(std::time::Duration::from_millis(10));
    let budget = Budget::with_clock(
        Limits {
            max_time: std::time::Duration::from_millis(25),
            ..Limits::default()
        },
        CancelToken::default(),
        clock.clone(),
    );
    let out = TextReader.read(&source(text.as_bytes()), &ReadOptions::default(), &budget);
    assert_eq!(out.completeness, Completeness::Partial);
    assert_eq!(clock.reads(), 3);
    let loss = out.document.losses.last().unwrap();
    assert_eq!(loss.why, LossReason::Limit);
    assert_eq!(loss.location, "paragraph 2");
    assert_eq!(loss.what, "rest of the text not read");
    let got = paragraphs(&out);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], "Muster GmbH");
    // The cut is where the third check came, on a character boundary.
    assert_eq!(got[1].len(), 3 * 64 * 1024);
    assert!(got[1].chars().all(|c| c == 'é'));
}

#[test]
fn the_element_budget_stops_reading_with_a_limit_loss() {
    let text = "a\n\n".repeat(100);
    let budget = Budget::new(Limits {
        max_elements: 20,
        ..Limits::default()
    });
    let out = TextReader.read(&source(text.as_bytes()), &ReadOptions::default(), &budget);
    assert_eq!(out.completeness, Completeness::Partial);
    // Each paragraph is two elements (the block and its text).
    assert_eq!(paragraphs(&out).len(), 10);
    let limits: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(limits.len(), 1, "{limits:?}");
    assert_eq!(limits[0].location, "paragraph 11");
    assert_eq!(
        limits[0].what,
        "element limit reached, rest of the text not read"
    );
    assert_eq!(limits[0].severity, Severity::Warning);
}

#[test]
fn the_element_budget_ends_on_the_last_paragraph_too() {
    // The last paragraph has no blank line after it; it is charged as well.
    let text = "a\n\nb";
    let budget = Budget::new(Limits {
        max_elements: 2,
        ..Limits::default()
    });
    let out = TextReader.read(&source(text.as_bytes()), &ReadOptions::default(), &budget);
    assert_eq!(out.completeness, Completeness::Partial);
    assert_eq!(paragraphs(&out), ["a"]);
}

#[test]
fn text_lines_join_with_line_break_inline() {
    let out = read(b"one\ntwo\n\nsolo");
    let [Block::Section { blocks, .. }] = out.document.body.as_slice() else {
        panic!("{:?}", out.document.body);
    };
    let Block::Paragraph { inlines, .. } = &blocks[0] else {
        panic!("{blocks:?}");
    };
    assert!(
        matches!(
            inlines.as_slice(),
            [Inline::Text { text: a, .. }, Inline::LineBreak, Inline::Text { text: b, .. }]
                if a == "one" && b == "two"
        ),
        "{inlines:?}"
    );
    let Block::Paragraph { inlines, .. } = &blocks[1] else {
        panic!("{blocks:?}");
    };
    assert!(matches!(inlines.as_slice(), [Inline::Text { .. }]));
}

#[test]
fn a_text_run_over_the_text_limit_is_cut_with_a_loss() {
    let text = format!("first\n\n{}\nmore\n\nlast", "x".repeat(100));
    let budget = Budget::new(Limits {
        max_text_bytes: 40,
        ..Limits::default()
    });
    let out = TextReader.read(&source(text.as_bytes()), &ReadOptions::default(), &budget);
    assert_eq!(out.completeness, Completeness::Partial);
    let got = paragraphs(&out);
    assert_eq!(got, ["first".to_string(), "x".repeat(40)]);
    let loss = out.document.losses.last().unwrap();
    assert_eq!(loss.why, LossReason::Limit);
    assert_eq!(loss.location, "paragraph 2");
    assert!(loss.what.starts_with("text limit reached"), "{loss:?}");
}

#[test]
fn every_line_of_a_paragraph_counts_as_elements() {
    // Block plus text for the first line, a break and a text for each further line.
    let text = "a\n".repeat(100);
    let budget = Budget::new(Limits {
        max_elements: 50,
        ..Limits::default()
    });
    let out = TextReader.read(&source(text.as_bytes()), &ReadOptions::default(), &budget);
    assert_eq!(out.completeness, Completeness::Partial);
    let got = paragraphs(&out);
    assert_eq!(got.len(), 1);
    assert!(got[0].lines().count() < 30, "{}", got[0].lines().count());
    assert!(budget.elements() <= 50 + 2);
}
