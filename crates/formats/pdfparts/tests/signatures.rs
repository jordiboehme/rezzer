// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use lopdf::{Document, Object, StringFormat, dictionary};
use pdfparts::{Limits, Pdf, SigKind};

fn never() -> bool {
    false
}

fn corpus_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf")
        .join(name);
    std::fs::read(path).unwrap()
}

fn open_bytes(bytes: &[u8]) -> Pdf {
    Pdf::open(bytes, None, &Limits::default(), &never).unwrap()
}

#[test]
fn field_signature_covers_whole_file() {
    let bytes = corpus_bytes("pdfjs-signed.pdf");
    let pdf = open_bytes(&bytes);
    let sigs = pdf.signatures();
    assert_eq!(sigs.len(), 1);
    let s = &sigs[0];
    assert_eq!(s.field.as_deref(), Some("Signature1"));
    assert_eq!(s.kind, SigKind::Field);
    assert_eq!(s.sub_filter.as_deref(), Some("adbe.pkcs7.detached"));
    assert_eq!(
        s.reason.as_deref(),
        Some("Test signature for pdf.js Digital signature properties UI")
    );
    assert_eq!(
        s.date.as_ref().and_then(|d| d.iso.as_deref()),
        Some("2026-05-09T00:00:00Z")
    );
    assert_eq!(s.byte_range, [0, 644, 8836, 1416]);
    assert_eq!(bytes.len(), 8836 + 1416);
    assert!(s.covers_whole_file);
    assert!(
        std::ptr::eq(pdf.signatures(), pdf.signatures()),
        "read once"
    );

    // A save after the signature leaves bytes it does not cover.
    let mut more = bytes.clone();
    more.extend(b"\n% a later save\n");
    let pdf = open_bytes(&more);
    assert!(!pdf.signatures()[0].covers_whole_file);
}

#[test]
fn usage_rights_signature_found() {
    let bytes = corpus_bytes("irs-f1040-2025.pdf");
    let pdf = open_bytes(&bytes);
    let sigs = pdf.signatures();
    assert_eq!(sigs.len(), 1);
    let s = &sigs[0];
    assert_eq!(s.kind, SigKind::UsageRights);
    assert_eq!(s.field, None);
    assert_eq!(
        s.signer.as_deref(),
        Some("ARE Production V8.1 G3 P24 1007685")
    );
    assert_eq!(s.sub_filter.as_deref(), Some("adbe.pkcs7.detached"));
    assert_eq!(s.byte_range, [0, 206928, 219198, 1039]);
    assert_eq!(bytes.len(), 220237);
    assert!(s.covers_whole_file);
}

#[test]
fn certification_signature_is_listed_once() {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let sig = doc.add_object(dictionary! {
        "Type" => "Sig", "Name" => Object::string_literal("Jane Roe"),
        "Location" => Object::string_literal("Berlin"), "ContactInfo" => Object::string_literal("jane@example.org"),
        "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
        "Contents" => Object::String(vec![0; 4], StringFormat::Hexadecimal),
    });
    let field = doc.add_object(dictionary! {
        "T" => Object::string_literal("Cert"), "FT" => "Sig", "V" => sig,
        "Type" => "Annot", "Subtype" => "Widget", "P" => page_id,
    });
    let empty =
        doc.add_object(dictionary! { "T" => Object::string_literal("Empty"), "FT" => "Sig" });
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id,
        "AcroForm" => dictionary! { "Fields" => vec![field.into(), empty.into()] },
        "Perms" => dictionary! { "DocMDP" => sig },
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let pdf = open_bytes(&out);
    let sigs = pdf.signatures();
    assert_eq!(sigs.len(), 1);
    let s = &sigs[0];
    assert_eq!(s.kind, SigKind::DocMdp);
    assert_eq!(s.field.as_deref(), Some("Cert"));
    assert_eq!(s.signer.as_deref(), Some("Jane Roe"));
    assert_eq!(s.location.as_deref(), Some("Berlin"));
    assert_eq!(s.contact.as_deref(), Some("jane@example.org"));
    assert_eq!(s.byte_range, [0, 10, 20, 30]);
    assert!(!s.covers_whole_file);
    // The unsigned field is still a form field.
    assert_eq!(pdf.form().unwrap().fields.len(), 2);
}

#[test]
fn texts_many_signatures_name_are_read_once() {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let name = doc.add_object(Object::string_literal("Jane Roe"));
    let fields: Vec<Object> = (0..2)
        .map(|i| {
            let sig = doc.add_object(dictionary! {
                "Type" => "Sig", "Name" => name, "Reason" => name,
                "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
            });
            doc.add_object(dictionary! {
                "T" => Object::string_literal(format!("S{i}")), "FT" => "Sig", "V" => sig,
            })
            .into()
        })
        .collect();
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id,
        "AcroForm" => dictionary! { "Fields" => fields },
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let pdf = open_bytes(&out);
    let sigs = pdf.signatures();
    assert_eq!(sigs.len(), 2);
    assert_eq!(sigs[0].signer.as_deref(), Some("Jane Roe"));
    assert!(std::sync::Arc::ptr_eq(
        sigs[0].signer.as_ref().unwrap(),
        sigs[1].reason.as_ref().unwrap()
    ));
}
