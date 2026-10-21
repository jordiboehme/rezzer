// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use lopdf::{Dictionary, Document, Object, dictionary};
use pdfparts::{IssueKind, Limits, Pdf};

fn never() -> bool {
    false
}

fn open_bytes(bytes: &[u8]) -> Pdf {
    Pdf::open(bytes, None, &Limits::default(), &never).unwrap()
}

fn corpus(name: &str) -> Pdf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf")
        .join(name);
    open_bytes(&std::fs::read(path).unwrap())
}

/// A one-page document. `build` may add objects and returns the page's /Annots.
fn with_annots(build: impl FnOnce(&mut Document, lopdf::ObjectId) -> Object) -> Pdf {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let annots = build(&mut doc, page_id);
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Annots" => annots,
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    open_bytes(&out)
}

fn text(s: &str) -> Object {
    Object::string_literal(s)
}

fn note(contents: &str, extra: Dictionary) -> Dictionary {
    let mut d = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Text",
        "Rect" => vec![10.into(), 10.into(), 30.into(), 30.into()],
        "Contents" => text(contents),
        "T" => text("Ana"),
    };
    d.extend(&extra);
    d
}

#[test]
fn markup_annotations_with_author_dates_color_quads() {
    // File, subtype, modified, created, colour, contents, quads.
    type Row = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
        usize,
    );
    let rows: [(&str, &[Row]); 6] = [
        (
            "pdfjs-annotation-strikeout.pdf",
            &[(
                "19 0",
                "StrikeOut",
                "2015-12-29T15:05:16+01:00",
                "2015-12-29T15:05:01+01:00",
                Some("#f94244"),
                Some("StrikeOut content"),
                1,
            )],
        ),
        (
            "pdfjs-annotation-highlight.pdf",
            &[(
                "19 0",
                "Highlight",
                "2015-12-30T15:22:48+01:00",
                "2015-12-30T15:22:31+01:00",
                Some("#ffed00"),
                Some("Highlight content"),
                1,
            )],
        ),
        (
            "pdfjs-annotation-underline.pdf",
            &[(
                "26 0",
                "Underline",
                "2015-12-28T00:27:36+01:00",
                "2015-12-28T00:27:04+01:00",
                Some("#339e00"),
                Some("Underline content"),
                1,
            )],
        ),
        (
            "pdfjs-annotation-caret-ink.pdf",
            &[
                (
                    "22 0",
                    "Caret",
                    "2015-12-20T23:46:04+01:00",
                    "2015-12-20T23:44:42+01:00",
                    Some("#ff0000"),
                    Some("Caret content"),
                    0,
                ),
                (
                    "25 0",
                    "Ink",
                    "2015-12-20T23:46:43+01:00",
                    "2015-12-20T23:45:01+01:00",
                    Some("#0000bf"),
                    Some("Ink content"),
                    0,
                ),
            ],
        ),
        (
            "pdfjs-annotation-freetext.pdf",
            &[
                (
                    "20 0",
                    "FreeText",
                    "2015-12-20T23:32:13+01:00",
                    "2015-12-20T23:32:02+01:00",
                    None,
                    Some("FreeText content"),
                    0,
                ),
                (
                    "23 0",
                    "FreeText",
                    "2015-12-20T23:32:14+01:00",
                    "2015-12-20T23:32:13+01:00",
                    None,
                    None,
                    0,
                ),
            ],
        ),
        (
            "pdfjs-annotation-link-text-popup.pdf",
            &[(
                "27 0",
                "Text",
                "2015-12-20T23:37:35+01:00",
                "2015-12-20T23:37:20+01:00",
                Some("#ffff00"),
                Some("Text content"),
                0,
            )],
        ),
    ];
    for (file, expected) in rows {
        let annots = corpus(file).annotations();
        assert_eq!(annots.len(), expected.len(), "{file}: {annots:?}");
        for (a, (id, subtype, modified, created, color, contents, quads)) in
            annots.iter().zip(expected.iter())
        {
            assert_eq!(a.id, *id, "{file}");
            assert_eq!(a.page, 1, "{file}");
            assert_eq!(a.subtype, *subtype, "{file}");
            assert_eq!(a.author.as_deref(), Some("Tim van der Meij"), "{file}");
            assert_eq!(
                a.modified.as_ref().and_then(|d| d.iso.as_deref()),
                Some(*modified),
                "{file}"
            );
            assert_eq!(
                a.created.as_ref().and_then(|d| d.iso.as_deref()),
                Some(*created),
                "{file}"
            );
            assert_eq!(a.color.as_deref(), *color, "{file}");
            assert_eq!(a.contents.as_deref(), *contents, "{file}");
            assert_eq!(a.quads.len(), *quads, "{file}");
            assert!(a.in_reply_to.is_none() && a.reply_type.is_none());
        }
    }
    let strike = &corpus("pdfjs-annotation-strikeout.pdf").annotations()[0];
    assert_eq!(strike.subject.as_deref(), Some("Strikeout"));
    assert_eq!(
        strike.modified.as_ref().map(|d| d.raw.as_str()),
        Some("D:20151229150516+01'00'")
    );
    assert_eq!(strike.rect, [70.86, 736.177, 193.88, 745.586]);
    assert_eq!(
        strike.quads[0],
        [
            70.86, 745.586, 193.88, 745.586, 70.86, 736.177, 193.88, 736.177
        ]
    );
}

#[test]
fn direct_dictionary_annotations_are_kept() {
    let annots = corpus("pypdf-annotated.pdf").annotations();
    let kinds: Vec<&str> = annots.iter().map(|a| a.subtype.as_str()).collect();
    assert_eq!(kinds, ["Text", "Highlight", "Ink"]);
    let ids: Vec<&str> = annots.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["p1#1", "p1#2", "p1#3"]);
    assert_eq!(
        annots[0].contents.as_deref(),
        Some("This is a text annotation.")
    );
    let hl = &annots[1];
    assert_eq!(hl.quads.len(), 2);
    assert_eq!(hl.color.as_deref(), Some("#ffff00"));
    assert_eq!(
        hl.modified.as_ref().and_then(|d| d.iso.as_deref()),
        Some("1990-04-28T00:00:00")
    );
    // The rectangle stays as written, even when it is upside down.
    assert_eq!(hl.rect, [676.16, 719.36, 854.92, 676.16]);
    assert_eq!(annots[2].author.as_deref(), Some("Lucas"));
}

#[test]
fn reply_chain_kept() {
    let pdf = with_annots(|doc, page| {
        let parent = doc.add_object(note("Please check", dictionary! { "P" => page }));
        let reply = doc.add_object(note(
            "Done",
            dictionary! {
                "IRT" => parent,
                "RT" => "R",
                "State" => text("Accepted"),
                "StateModel" => text("Review"),
            },
        ));
        let group = doc.add_object(note(
            "Same thread",
            dictionary! { "IRT" => parent, "RT" => "Group" },
        ));
        Object::Array(vec![parent.into(), reply.into(), group.into()])
    });
    let annots = pdf.annotations();
    assert_eq!(annots.len(), 3);
    let parent = &annots[0];
    assert_eq!(parent.in_reply_to, None);
    assert_eq!(annots[1].in_reply_to.as_deref(), Some(parent.id.as_str()));
    assert_eq!(annots[1].reply_type.as_deref(), Some("R"));
    assert_eq!(annots[1].state.as_deref(), Some("Accepted"));
    assert_eq!(annots[2].in_reply_to.as_deref(), Some(parent.id.as_str()));
    assert_eq!(annots[2].reply_type.as_deref(), Some("Group"));
}

#[test]
fn links_popups_and_widgets_are_left_out() {
    let pdf = with_annots(|_, _| {
        let kinds = ["Link", "Popup", "Widget", "Square", "FileAttachment"];
        Object::Array(
            kinds
                .iter()
                .map(|k| {
                    Object::Dictionary(dictionary! {
                        "Type" => "Annot",
                        "Subtype" => *k,
                        "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
                    })
                })
                .collect(),
        )
    });
    let kinds: Vec<String> = pdf.annotations().into_iter().map(|a| a.subtype).collect();
    assert_eq!(kinds, ["Square", "FileAttachment"]);
}

#[test]
fn widget_boxes_are_listed_by_page() {
    let pdf = with_annots(|_, _| {
        let annot = |kind: &str, r: [i64; 4]| {
            Object::Dictionary(dictionary! {
                "Type" => "Annot",
                "Subtype" => kind,
                "Rect" => r.iter().map(|&v| Object::Integer(v)).collect::<Vec<_>>(),
            })
        };
        Object::Array(vec![
            annot("Widget", [300, 700, 100, 680]),
            annot("Link", [0, 0, 10, 10]),
            annot("Widget", [10, 20, 30, 40]),
        ])
    });
    assert_eq!(
        pdf.widget_boxes(),
        [
            (1, [100.0, 680.0, 300.0, 700.0]),
            (1, [10.0, 20.0, 30.0, 40.0])
        ]
    );
    let none = with_annots(|_, _| Object::Array(Vec::new()));
    assert!(none.widget_boxes().is_empty());
}

#[test]
fn a_widget_two_pages_list_is_listed_on_both() {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let widget = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "Rect" => vec![10.into(), 20.into(), 30.into(), 40.into()],
    });
    let mut kids = Vec::new();
    for _ in 0..2 {
        // Each page has its own `/Annots` array naming the same widget.
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Annots" => vec![widget.into(), widget.into()],
        });
        kids.push(page.into());
    }
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 2 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let pdf = open_bytes(&out);
    assert_eq!(
        pdf.widget_boxes(),
        [(1, [10.0, 20.0, 30.0, 40.0]), (2, [10.0, 20.0, 30.0, 40.0])]
    );
}

#[test]
fn colours_from_gray_and_cmyk() {
    let pdf = with_annots(|_, _| {
        let c = |c: Vec<Object>| Object::Dictionary(note("x", dictionary! { "C" => c }));
        Object::Array(vec![
            c(vec![0.5.into()]),
            c(vec![0.into(), 1.into(), 1.into(), 0.into()]),
            c(vec![]),
        ])
    });
    let colors: Vec<Option<String>> = pdf.annotations().into_iter().map(|a| a.color).collect();
    assert_eq!(
        colors,
        [
            Some("#808080".to_string()),
            Some("#ff0000".to_string()),
            None
        ]
    );
}

#[test]
fn broken_quad_points_keep_whole_quads() {
    let pdf = with_annots(|_, _| {
        let mut q: Vec<Object> = (0..8).map(Object::Integer).collect();
        q.extend([Object::Integer(1), Object::Integer(2)]);
        Object::Array(vec![Object::Dictionary(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Highlight",
            "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "QuadPoints" => q,
        })])
    });
    let annots = pdf.annotations();
    assert_eq!(annots[0].quads, [[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]]);
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.location == "page 1"),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn shared_annotations_are_listed_once() {
    // Two pages share one /Annots array with one indirect annotation.
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let a = doc.add_object(note("shared", Dictionary::new()));
    let annots = doc.add_object(Object::Array(vec![a.into()]));
    let kids: Vec<Object> = (0..2)
        .map(|_| {
            doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Annots" => annots,
            })
            .into()
        })
        .collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 2 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let annots = open_bytes(&out).annotations();
    assert_eq!(annots.len(), 1);
    assert_eq!(annots[0].page, 1);
}
