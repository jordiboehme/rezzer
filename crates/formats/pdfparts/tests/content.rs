// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The count of text shown in render mode 3 (invisible) on a page.

use lopdf::{Document, Object, Stream, dictionary};
use pdfparts::{IssueKind, Limits, Pdf};

mod common;

fn never() -> bool {
    false
}

/// A one-page file with `content` and Form XObjects `/Fm0`, `/Fm1`... (object 10, 11...).
fn page_with_forms(content: &[u8], forms: &[&[u8]]) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    let mut xobjects = lopdf::Dictionary::new();
    for (i, form) in forms.iter().enumerate() {
        let id = (10 + i as u32, 0);
        xobjects.set(format!("Fm{i}"), id);
        doc.objects.insert(
            id,
            Object::Stream(Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()] },
                form.to_vec(),
            )),
        );
    }
    // Every form can call every form.
    for i in 0..forms.len() {
        let id = (10 + i as u32, 0);
        if let Some(Object::Stream(s)) = doc.objects.get_mut(&id) {
            s.dict
                .set("Resources", dictionary! { "XObject" => xobjects.clone() });
        }
    }
    doc.objects.insert(
        (1, 0),
        Object::Dictionary(dictionary! { "Type" => "Catalog", "Pages" => (2, 0) }),
    );
    doc.objects.insert(
        (2, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference((3, 0))], "Count" => 1,
        }),
    );
    doc.objects.insert(
        (3, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => (2, 0),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => (4, 0),
            "Resources" => dictionary! { "XObject" => xobjects },
        }),
    );
    doc.objects.insert(
        (4, 0),
        Object::Stream(Stream::new(dictionary! {}, content.to_vec())),
    );
    doc.max_id = 10 + forms.len() as u32;
    doc.trailer.set("Root", (1, 0));
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn open(bytes: &[u8], limits: &Limits) -> Pdf {
    Pdf::open(bytes, None, limits, &never).unwrap()
}

#[test]
fn counts_text_shown_in_mode_3() {
    let content = b"BT /F1 12 Tf (visible) Tj\n\
        3 Tr (one) Tj [(two) -20 (more)] TJ (three) ' 1 2 (four) \"\n\
        (a \\) Tj and (nested) Tj string) Tj\n\
        0 Tr (visible again) Tj ET\n\
        q 3 Tr BT (five) Tj ET Q BT (visible after Q) Tj ET\n\
        BI /W 2 /H 1 /BPC 8 /CS /G ID \x00Tj EI\n\
        % a comment 3 Tr (x) Tj\n\
        BT 7 Tr (clip only) Tj ET";
    let pdf = open(&page_with_forms(content, &[]), &Limits::default());
    let found = pdf.invisible_text_shows(1, &never);
    assert_eq!(found.count, 6);
    assert!(found.complete);
}

#[test]
fn forms_inherit_the_mode() {
    let content = b"3 Tr /Fm0 Do /Fm0 Do 0 Tr /Fm0 Do";
    let pdf = open(
        &page_with_forms(content, &[b"BT (in form) Tj ET q 0 Tr (shown) Tj Q"]),
        &Limits::default(),
    );
    let found = pdf.invisible_text_shows(1, &never);
    assert_eq!(found.count, 2);
    assert!(found.complete);
}

#[test]
fn forms_calling_themselves_end() {
    let content = b"3 Tr /Fm0 Do";
    let pdf = open(
        &page_with_forms(content, &[b"(a) Tj /Fm1 Do /Fm0 Do", b"(b) Tj /Fm0 Do"]),
        &Limits::default(),
    );
    let found = pdf.invisible_text_shows(1, &never);
    // Fm0 shows one, Fm1 one; a form already on the call stack is not entered again.
    assert_eq!(found.count, 2);
}

#[test]
fn many_calls_of_one_form_are_counted_once_per_mode() {
    let mut content = b"3 Tr ".to_vec();
    for _ in 0..200_000 {
        content.extend(b"/Fm0 Do ");
    }
    let mut form = Vec::new();
    for _ in 0..1_000 {
        form.extend(b"(x) Tj ");
    }
    let limits = Limits {
        max_items: 500_000,
        ..Limits::default()
    };
    let pdf = open(&page_with_forms(&content, &[&form]), &limits);
    let start = std::time::Instant::now();
    let found = pdf.invisible_text_shows(1, &never);
    assert_eq!(found.count, 200_000_000);
    assert!(found.complete);
    assert!(start.elapsed().as_secs() < 5);
}

#[test]
fn the_scan_is_bounded() {
    let mut content = b"3 Tr ".to_vec();
    for _ in 0..10_000 {
        content.extend(b"(x) Tj ");
    }
    let limits = Limits {
        max_items: 1_000,
        ..Limits::default()
    };
    let pdf = open(&page_with_forms(&content, &[]), &limits);
    let found = pdf.invisible_text_shows(1, &never);
    assert!(!found.complete);
    assert!(found.count < 10_000);
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.location == "page 1" && i.kind == IssueKind::Limit),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn the_scan_stops_when_asked() {
    let pdf = open(&page_with_forms(b"3 Tr (x) Tj", &[]), &Limits::default());
    let found = pdf.invisible_text_shows(1, &|| true);
    assert!(!found.complete);
}
