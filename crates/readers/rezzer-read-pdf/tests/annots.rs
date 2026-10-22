// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Annotations at their passage: the quoted text under highlights, underlines and strike-outs,
//! the block each annotation stands at, and replies under their parent.

mod common;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use common::{Builder, Page, pdf, show};
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Annotation, Block, Bytes, Completeness, Confidence, Detection, FormatId, Inline,
    LossReason, ReadOptions, ReadOutcome, Reader, Source, plain_text,
};
use rezzer_read_pdf::PdfReader;

fn corpus(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/corpus/pdf");
    std::fs::read(path.join(name)).expect("corpus file")
}

fn read(bytes: &[u8]) -> ReadOutcome {
    let src = Source {
        path: None,
        name: "file.pdf".into(),
        bytes: Bytes::Owned(Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format: FormatId::Pdf,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "%PDF-".into(),
        },
        modified: None,
    };
    PdfReader.read(
        &src,
        &ReadOptions::default(),
        &Budget::new(Limits::default()),
    )
}

/// Every block anchor of the body with the block's text, pages included.
fn anchors(out: &ReadOutcome) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for b in &out.document.body {
        if let Block::Section {
            anchor: Some(a),
            blocks,
            ..
        } = b
        {
            found.push((a.0.clone(), String::new()));
            for inner in blocks {
                let (anchor, text) = match inner {
                    Block::Paragraph {
                        anchor, inlines, ..
                    }
                    | Block::Heading {
                        anchor, inlines, ..
                    } => (anchor, plain_text(inlines)),
                    Block::List { anchor, .. } | Block::Table { anchor, .. } => {
                        (anchor, String::new())
                    }
                    _ => continue,
                };
                if let Some(a) = anchor {
                    found.push((a.0.clone(), text));
                }
            }
        }
    }
    found
}

/// The text of the block anchored `anchor`.
fn block_text(out: &ReadOutcome, anchor: &Anchor) -> String {
    anchors(out)
        .into_iter()
        .find(|(a, _)| *a == anchor.0)
        .map(|(_, t)| t)
        .unwrap_or_else(|| panic!("no block {anchor:?}"))
}

fn one(out: &ReadOutcome) -> &Annotation {
    let list = &out.document.facets.annotations;
    assert_eq!(list.len(), 1, "{list:?} {:?}", out.document.losses);
    &list[0]
}

#[test]
fn highlight_quotes_exact_words() {
    let out = read(&corpus("pdfjs-annotation-highlight.pdf"));
    let a = one(&out);
    assert_eq!(a.kind, "highlight");
    assert_eq!(a.quoted.as_deref(), Some("Lorem ipsum dolor sit amet"));
    assert_eq!(a.author.as_deref(), Some("Tim van der Meij"));
    assert_eq!(a.date.as_deref(), Some("2015-12-30T15:22:48+01:00"));
    assert_eq!(plain_text(&a.body), "Highlight content");
    let anchor = a.anchor.as_ref().expect("an anchor");
    assert!(
        block_text(&out, anchor).contains("Lorem ipsum dolor sit amet"),
        "{anchor:?}"
    );
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn strikeout_and_underline_quotes() {
    let out = read(&corpus("pdfjs-annotation-strikeout.pdf"));
    let a = one(&out);
    assert_eq!(a.kind, "strikeout");
    assert_eq!(a.quoted.as_deref(), Some("Lorem ipsum dolor sit amet"));
    let out = read(&corpus("pdfjs-annotation-underline.pdf"));
    let a = one(&out);
    assert_eq!(a.kind, "underline");
    assert_eq!(a.quoted.as_deref(), Some("Lorem ipsum dolor sit amet"));
    assert_eq!(plain_text(&a.body), "Underline content");
}

#[test]
fn pypdf_highlight_spans_two_lines() {
    let out = read(&corpus("pypdf-annotated.pdf"));
    let list = &out.document.facets.annotations;
    let kinds: Vec<&str> = list.iter().map(|a| a.kind.as_str()).collect();
    assert_eq!(kinds, ["note", "highlight", "ink"]);
    let hl = &list[1];
    assert_eq!(hl.quoted.as_deref(), Some("Line 1 Line 2"));
    let anchor = hl.anchor.as_ref().expect("an anchor");
    assert_eq!(block_text(&out, anchor), "Line 1 Line 2 Not highlighted");
}

/// A one-page file with two paragraphs, one at the top and one low on the page, and the
/// annotation dictionaries `annots`.
fn two_paragraphs(annots: Vec<String>) -> Vec<u8> {
    let content = format!(
        "{}{}",
        show("F1", 12.0, 72.0, 700.0, "The first paragraph."),
        show("F1", 12.0, 72.0, 300.0, "The second paragraph.")
    );
    pdf(
        &[Page {
            annots,
            ..Page::text(content)
        }],
        "",
    )
}

#[test]
fn sticky_note_without_quads_anchors_to_nearest_block() {
    let note = "<< /Type /Annot /Subtype /Text /Rect [500 290 520 310] \
                /Contents (Check this.\\nAnd this.) /T (Ana) /M (D:20240102030405Z) >>";
    let out = read(&two_paragraphs(vec![note.into()]));
    let a = one(&out);
    assert_eq!(a.kind, "note");
    assert_eq!(a.quoted, None);
    assert_eq!(a.date.as_deref(), Some("2024-01-02T03:04:05Z"));
    let anchor = a.anchor.as_ref().expect("an anchor");
    assert_eq!(block_text(&out, anchor), "The second paragraph.");
    // The lines of the comment stay lines; nothing is read as Markdown.
    assert_eq!(
        a.body,
        [
            Inline::plain("Check this."),
            Inline::LineBreak,
            Inline::plain("And this.")
        ]
    );
}

#[test]
fn free_text_and_ink_listed() {
    let out = read(&corpus("pdfjs-annotation-freetext.pdf"));
    let kinds: Vec<&str> = out
        .document
        .facets
        .annotations
        .iter()
        .map(|a| a.kind.as_str())
        .collect();
    assert_eq!(kinds, ["free-text", "free-text"]);
    assert_eq!(
        plain_text(&out.document.facets.annotations[0].body),
        "FreeText content"
    );
    let out = read(&corpus("pdfjs-annotation-caret-ink.pdf"));
    let kinds: Vec<&str> = out
        .document
        .facets
        .annotations
        .iter()
        .map(|a| a.kind.as_str())
        .collect();
    assert_eq!(kinds, ["caret", "ink"]);
    assert!(
        out.document
            .facets
            .annotations
            .iter()
            .all(|a| a.anchor.is_some() && a.author.as_deref() == Some("Tim van der Meij"))
    );
}

#[test]
fn reply_nests_under_parent() {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let kid = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream("", show("F1", 12.0, 72.0, 700.0, "Some words.").as_bytes());
    let note = |extra: &str, text: &str| {
        format!(
            "<< /Type /Annot /Subtype /Text /Rect [10 690 30 710] /Contents ({text}) \
             /T (Ana) {extra} >>"
        )
    };
    let parent = b.add(note("", "Please check"));
    let reply = b.add(note(
        &format!("/IRT {parent} 0 R /RT /R /State (Accepted) /StateModel (Review)"),
        "Done",
    ));
    let group = b.add(note(&format!("/IRT {parent} 0 R /RT /Group"), "More"));
    let answer = b.add(note(&format!("/IRT {reply} 0 R"), "Thanks"));
    b.set(
        kid,
        format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R \
             /Annots [{parent} 0 R {reply} 0 R {group} 0 R {answer} 0 R] >>"
        ),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    let a = one(&out);
    // The group member's comment is part of the parent's.
    assert_eq!(
        a.body,
        [
            Inline::plain("Please check"),
            Inline::LineBreak,
            Inline::plain("More")
        ]
    );
    assert_eq!(a.resolved, Some(true));
    assert_eq!(a.replies.len(), 1, "{:?}", a.replies);
    let r = &a.replies[0];
    assert_eq!(plain_text(&r.body), "Done");
    assert_eq!(r.anchor, a.anchor);
    assert_eq!(r.replies.len(), 1);
    assert_eq!(plain_text(&r.replies[0].body), "Thanks");
}

#[test]
fn annotation_anchor_is_a_block() {
    for file in [
        "pdfjs-annotation-highlight.pdf",
        "pdfjs-annotation-strikeout.pdf",
        "pdfjs-annotation-underline.pdf",
        "pdfjs-annotation-freetext.pdf",
        "pdfjs-annotation-caret-ink.pdf",
        "pdfjs-annotation-link-text-popup.pdf",
        "pypdf-annotated.pdf",
    ] {
        let out = read(&corpus(file));
        let known: HashSet<String> = anchors(&out).into_iter().map(|(a, _)| a).collect();
        assert!(!out.document.facets.annotations.is_empty(), "{file}");
        for a in &out.document.facets.annotations {
            let anchor = a.anchor.as_ref().expect("an anchor");
            assert!(anchor.0.contains(":b"), "{file}: {anchor:?}");
            assert!(known.contains(&anchor.0), "{file}: {anchor:?}");
        }
    }
}

#[test]
fn a_note_on_a_page_without_text_anchors_to_the_page() {
    let note = "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (Empty page) >>";
    let out = read(&pdf(
        &[Page {
            annots: vec![note.into()],
            ..Page::text("")
        }],
        "",
    ));
    assert_eq!(one(&out).anchor, Some(Anchor("p1".into())));
}

#[test]
fn a_highlight_over_no_text_is_said() {
    // The quad lies where the page shows nothing.
    let hl = "<< /Type /Annot /Subtype /Highlight /Rect [72 100 200 120] \
              /QuadPoints [72 120 200 120 72 100 200 100] >>";
    let out = read(&two_paragraphs(vec![hl.into()]));
    let a = one(&out);
    assert_eq!(a.quoted, None);
    // It stands at the block nearest to its top.
    assert_eq!(
        block_text(&out, a.anchor.as_ref().unwrap()),
        "The second paragraph."
    );
}

#[test]
fn a_highlight_on_a_page_whose_text_is_not_read_is_said() {
    // The page tree lists the page twice, so no page's text is read.
    let hl = "<< /Type /Annot /Subtype /Highlight /Rect [72 690 200 712] \
              /QuadPoints [72 712 200 712 72 690 200 690] >>";
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let annot = b.add(hl);
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream(
        "",
        show("F1", 12.0, 72.0, 700.0, "Marked words.").as_bytes(),
    );
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /Contents {content} 0 R \
         /Resources << /Font << /F1 {font} 0 R >> >> /Annots [{annot} 0 R] >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R {kid} 0 R] /Count 2 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    assert_eq!(
        out.completeness,
        Completeness::Partial,
        "{:?}",
        out.document.losses
    );
    let a = one(&out);
    assert_eq!(a.quoted, None);
    assert!(
        out.document.losses.iter().any(|l| l.location == "p1"
            && l.what
                .contains("1 annotation that marks text is not quoted")),
        "{:?}",
        out.document.losses
    );
    assert!(
        out.document
            .losses
            .iter()
            .all(|l| l.why != LossReason::Internal)
    );
}

/// One highlight over the line at height 700 of a page with `content`.
fn highlighted(content: &str) -> Vec<u8> {
    let hl = "<< /Type /Annot /Subtype /Highlight /Rect [60 695 400 715] \
              /QuadPoints [60 715 400 715 60 695 400 695] >>";
    pdf(
        &[Page {
            annots: vec![hl.into()],
            ..Page::text(content)
        }],
        "",
    )
}

#[test]
fn hidden_text_under_a_highlight_is_not_quoted() {
    // Text that puts no glyphs on the page lies under the highlight too.
    let out = read(&highlighted(
        "BT /F1 12 Tf 72 700 Td (Seen words) Tj ET BT 3 Tr /F1 12 Tf 74 701 Td (Other text) Tj ET",
    ));
    assert_eq!(one(&out).quoted.as_deref(), Some("Seen words"));
}

#[test]
fn fake_bold_is_quoted_once() {
    // The line is drawn twice, half a point apart.
    let out = read(&highlighted(
        "BT /F1 12 Tf 72 700 Td (Bold words) Tj ET BT /F1 12 Tf 72.5 700 Td (Bold words) Tj ET",
    ));
    assert_eq!(one(&out).quoted.as_deref(), Some("Bold words"));
}

#[test]
fn letters_twice_in_a_word_are_kept() {
    let out = read(&highlighted(
        "BT /F1 12 Tf 72 700 Td (Hello all, 1100 sheep) Tj ET",
    ));
    assert_eq!(one(&out).quoted.as_deref(), Some("Hello all, 1100 sheep"));
}

/// A one-page file with `content` and one highlight over `quads`, each x0, y0, x1, y1.
fn with_quads(content: &str, quads: &[[f32; 4]]) -> Vec<u8> {
    let nums: Vec<String> = quads
        .iter()
        .flat_map(|&[x0, y0, x1, y1]| [x0, y1, x1, y1, x0, y0, x1, y0])
        .map(|v| v.to_string())
        .collect();
    let hl = format!(
        "<< /Type /Annot /Subtype /Highlight /Rect [60 600 400 720] /QuadPoints [{}] >>",
        nums.join(" ")
    );
    pdf(
        &[Page {
            annots: vec![hl],
            ..Page::text(content)
        }],
        "",
    )
}

const LOREM: &str = "BT /F1 12 Tf 72 700 Td (Lorem ipsum dolor) Tj ET";

#[test]
fn a_word_split_over_two_quads_stays_one_word() {
    let out = read(&with_quads(
        LOREM,
        &[[70.0, 697.0, 88.0, 711.0], [88.0, 697.0, 140.0, 711.0]],
    ));
    assert_eq!(one(&out).quoted.as_deref(), Some("Lorem ipsum"));
}

#[test]
fn one_quad_per_glyph_quotes_the_word() {
    // Helvetica advances of L, o, r, e, m at 12 points.
    let mut x = 72.0;
    let mut quads = Vec::new();
    for w in [6.672f32, 6.672, 3.996, 6.672, 9.996] {
        quads.push([x, 697.0, x + w, 711.0]);
        x += w;
    }
    let out = read(&with_quads(LOREM, &quads));
    assert_eq!(one(&out).quoted.as_deref(), Some("Lorem"));
}

#[test]
fn a_word_broken_at_a_line_end_is_joined() {
    let out = read(&with_quads(
        "BT /F1 12 Tf 72 700 Td (An exam-) Tj 0 -14 Td (ple of text) Tj ET",
        &[[70.0, 697.0, 200.0, 711.0], [70.0, 683.0, 200.0, 697.0]],
    ));
    assert_eq!(one(&out).quoted.as_deref(), Some("An exam-ple of text"));
}

#[test]
fn actual_text_partly_under_a_quad_is_quoted_whole() {
    let out = read(&with_quads(
        "BT /F1 12 Tf 72 700 Td /Span << /ActualText (HELLO) >> BDC (Hello) Tj EMC ( world) Tj ET",
        &[[82.0, 697.0, 140.0, 711.0]],
    ));
    assert_eq!(one(&out).quoted.as_deref(), Some("HELLO world"));
}

#[test]
fn the_latest_review_state_counts() {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let kid = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream("", LOREM.as_bytes());
    let note = |extra: &str| {
        format!("<< /Type /Annot /Subtype /Text /Rect [10 690 30 710] /Contents (x) {extra} >>")
    };
    let parent = b.add(note(""));
    // The reopening state is written first but dated later.
    let reopened = b.add(note(&format!(
        "/IRT {parent} 0 R /State (None) /StateModel (Review) /M (D:20240102000000Z)"
    )));
    let accepted = b.add(note(&format!(
        "/IRT {parent} 0 R /State (Accepted) /StateModel (Review) /M (D:20240101000000Z)"
    )));
    b.set(
        kid,
        format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R \
             /Annots [{parent} 0 R {reopened} 0 R {accepted} 0 R] >>"
        ),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    let a = one(&out);
    assert_eq!(a.resolved, None);
    assert_eq!(a.replies.len(), 2);
}

/// A one-page file of notes, each `(extra entries, comment)`; `@k` in the entries names note
/// `k` (from 0).
fn notes(list: &[(&str, &str)]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let kid = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream("", LOREM.as_bytes());
    let ids: Vec<u32> = list.iter().map(|_| b.reserve()).collect();
    for (k, (extra, text)) in list.iter().enumerate() {
        let mut extra = extra.to_string();
        for (j, id) in ids.iter().enumerate().rev() {
            extra = extra.replace(&format!("@{j}"), &format!("{id} 0 R"));
        }
        b.set(
            ids[k],
            format!(
                "<< /Type /Annot /Subtype /Text /Rect [10 690 30 710] /Contents ({text}) \
                 {extra} >>"
            ),
        );
    }
    let refs: Vec<String> = ids.iter().map(|i| format!("{i} 0 R")).collect();
    b.set(
        kid,
        format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R \
             /Annots [{}] >>",
            refs.join(" ")
        ),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn a_checkmark_does_not_undo_a_review() {
    for mark in ["Marked", "Unmarked"] {
        let out = read(&notes(&[
            ("", "parent"),
            (
                "/IRT @0 /State (Accepted) /StateModel (Review) /M (D:20240101000000Z)",
                "accepted",
            ),
            (
                &format!("/IRT @0 /State ({mark}) /StateModel (Marked) /M (D:20240102000000Z)"),
                "checked",
            ),
        ]));
        assert_eq!(one(&out).resolved, Some(true), "{mark}");
    }
    // A Marked-model state never closes, whatever its word.
    let out = read(&notes(&[
        ("", "parent"),
        ("/IRT @0 /State (Accepted) /StateModel (Marked)", "odd"),
    ]));
    assert_eq!(one(&out).resolved, None);
}

#[test]
fn an_undated_state_is_ordered_by_its_place_in_the_file() {
    let out = read(&notes(&[
        ("", "parent"),
        (
            "/IRT @0 /State (None) /StateModel (Review) /M (D:20240101000000Z)",
            "reopened",
        ),
        ("/IRT @0 /State (Completed) /StateModel (Review)", "done"),
    ]));
    assert_eq!(one(&out).resolved, Some(true));
}

#[test]
fn a_deep_state_reply_answers_its_own_parent() {
    // A chain of 20 notes; the last, at level 19, accepts note 18.
    let mut list: Vec<(String, String)> = vec![(String::new(), "n0".into())];
    for k in 1..19 {
        list.push((format!("/IRT @{}", k - 1), format!("n{k}")));
    }
    list.push((
        "/IRT @18 /State (Accepted) /StateModel (Review)".into(),
        "accept".into(),
    ));
    let refs: Vec<(&str, &str)> = list.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let out = read(&notes(&refs));
    let mut found: Vec<(String, Option<bool>)> = Vec::new();
    let mut stack: Vec<&Annotation> = out.document.facets.annotations.iter().collect();
    while let Some(a) = stack.pop() {
        found.push((plain_text(&a.body), a.resolved));
        stack.extend(a.replies.iter());
    }
    let mut resolved: Vec<&str> = found
        .iter()
        .filter(|(_, r)| *r == Some(true))
        .map(|(t, _)| t.as_str())
        .collect();
    resolved.sort_unstable();
    assert_eq!(resolved, ["accept", "n18"], "{found:?}");
}

#[test]
fn a_checkmark_under_the_review_model_changes_nothing() {
    let out = read(&notes(&[
        ("", "parent"),
        (
            "/IRT @0 /State (Accepted) /StateModel (Review) /M (D:20240101000000Z)",
            "accepted",
        ),
        (
            "/IRT @0 /State (Marked) /StateModel (Review) /M (D:20240102000000Z)",
            "checked",
        ),
    ]));
    assert_eq!(one(&out).resolved, Some(true));
}

#[test]
fn a_state_that_answers_a_group_member_answers_its_parent() {
    let out = read(&notes(&[
        ("", "parent"),
        ("/IRT @0 /RT /Group", "member"),
        ("/IRT @1 /State (Completed) /StateModel (Review)", "done"),
    ]));
    let a = one(&out);
    assert_eq!(a.resolved, Some(true));
    assert_eq!(plain_text(&a.body), "parent member");
}
