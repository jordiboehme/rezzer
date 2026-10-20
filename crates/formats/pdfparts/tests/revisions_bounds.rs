// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Revisions of large and hostile files: time and memory stay bounded, and nothing is lost
//! without an issue.

mod common;

use std::collections::BTreeMap;
use std::time::Instant;

use common::updates::*;
use lopdf::xref::XrefType;
use lopdf::{Document, Object, Stream, dictionary};
use pdfparts::{IssueKind, Limits, Pdf, Revisions};

fn never() -> bool {
    false
}

fn hostile() -> Limits {
    Limits {
        max_object_bytes: 256 << 20,
        ..Limits::default()
    }
}

/// A document with `pages` pages (objects from 10), catalog 1, pages 2, info 5 and an XMP
/// stream 6. With `shared`, every page names one /Annots array 7 of that many annotations.
fn pages_doc(pages: u32, shared: Option<u32>) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    doc.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
    let kids: Vec<Object> = (0..pages).map(|k| Object::Reference((10 + k, 0))).collect();
    doc.objects.insert(
        (1, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Catalog", "Pages" => (2, 0), "Metadata" => (6, 0),
        }),
    );
    doc.objects.insert(
        (2, 0),
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages }),
    );
    doc.objects.insert(
        (5, 0),
        Object::Dictionary(dictionary! { "Title" => Object::string_literal("Pages") }),
    );
    doc.objects.insert(
        (6, 0),
        Object::Stream(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x:xmpmeta/>".to_vec(),
        )),
    );
    let first_annot = 10 + pages;
    if let Some(n) = shared {
        let list: Vec<Object> = (0..n)
            .map(|k| Object::Reference((first_annot + k, 0)))
            .collect();
        doc.objects.insert((7, 0), Object::Array(list));
        for k in 0..n {
            doc.objects.insert(
                (first_annot + k, 0),
                Object::Dictionary(dictionary! {
                    "Type" => "Annot", "Subtype" => "Text",
                    "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
                }),
            );
        }
    }
    for k in 0..pages {
        let mut page = dictionary! { "Type" => "Page", "Parent" => (2, 0) };
        if shared.is_some() {
            page.set("Annots", (7, 0));
        }
        doc.objects.insert((10 + k, 0), Object::Dictionary(page));
    }
    doc.max_id = first_annot + shared.unwrap_or(0);
    doc.trailer.set("Root", (1, 0));
    doc.trailer.set("Info", (5, 0));
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out.push(b'\n');
    out
}

fn xmp(k: u32) -> Vec<u8> {
    stream(
        "/Type /Metadata /Subtype /XML",
        format!("<x:xmpmeta n='{k}'/>").as_bytes(),
    )
}

/// The section walk is linear in the number of sections.
#[test]
fn eighty_thousand_sections_in_time() {
    let mut file = base(b"BT (x) Tj ET");
    for k in 0..79_999u32 {
        let date = format!("2026{:02}{:02}{:06}Z", 1 + k % 12, 1 + k % 28, k % 240_000);
        update(&mut file, &[(5, info(&date))], 6);
    }
    let started = Instant::now();
    let pdf = Pdf::open(&file, None, &Limits::default(), &never).unwrap();
    let opened = started.elapsed();
    let r = pdf.revisions(&never);
    let took = started.elapsed() - opened;
    eprintln!("{} bytes, open {opened:?}, revisions {took:?}", file.len());
    assert!(opened.as_secs_f64() < 2.0, "{opened:?}");
    assert!(took.as_secs_f64() < 2.0, "{took:?}");
    assert_eq!(r.skipped, 80_000 - 64);
    assert_eq!(pdf.page_count(), 1);
}

/// An entry that points at the wrong offset is a word and an issue, never silence.
#[test]
fn wrong_offset_is_a_word_and_an_issue() {
    let mut file = base(b"BT (x) Tj ET");
    update(&mut file, &[(6, b"<< /Foo 1 >>".to_vec())], 7);
    let xref = file.windows(6).rposition(|w| w == b"\nxref\n").unwrap();
    let at = xref + b"\nxref\n6 1\n".len();
    let off: usize = std::str::from_utf8(&file[at..at + 10])
        .unwrap()
        .parse()
        .unwrap();
    file[at..at + 10].copy_from_slice(format!("{:010}", off + 3).as_bytes());
    let pdf = Pdf::open(&file, None, &Limits::default(), &never).unwrap();
    let r = pdf.revisions(&never);
    assert_eq!(r.list[1].changes, ["changed object 6 (unreadable)"]);
    assert!(
        r.issues.iter().any(|i| i.kind == IssueKind::Damaged),
        "{:?}",
        r.issues
    );
}

/// Saves that only rewrite the XMP stream walk the page tree at most once.
#[test]
fn metadata_saves_do_not_walk_pages_again() {
    let mut file = pages_doc(40_000, None);
    for k in 0..60 {
        update(&mut file, &[(6, xmp(k))], 10 + 40_000);
    }
    let pdf = Pdf::open(&file, None, &hostile(), &never).unwrap();
    let started = Instant::now();
    let r = pdf.revisions(&never);
    eprintln!("revisions in {:?}", started.elapsed());
    assert_eq!(r.list.len(), 61);
    for v in &r.list[1..] {
        assert_eq!(v.changes, ["changed object 6 (Metadata)"], "{}", v.number);
    }
    assert!(r.issues.is_empty(), "{:?}", r.issues);
    common::assert_rss_bounded();
}

/// Pages that share one /Annots array: it is walked once, with one issue that says so.
#[test]
fn shared_annots_array_is_walked_once() {
    let mut file = pages_doc(2_500, Some(1_000));
    update(&mut file, &[(6, xmp(1))], 10 + 2_500 + 1_000);
    let pdf = Pdf::open(&file, None, &hostile(), &never).unwrap();
    let r = pdf.revisions(&never);
    assert_eq!(r.list.len(), 2);
    assert_eq!(r.list[1].changes, ["changed object 6 (Metadata)"]);
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.what.contains("share")),
        "{:?}",
        r.issues
    );
    assert!(!r.issues.iter().any(|i| i.kind == IssueKind::Limit));
}

/// A valid file with more saves than `max_revisions` whose newest save rewrites the catalog
/// opens with all its pages.
#[test]
fn open_reads_the_whole_chain() {
    let mut file = pages_doc(3, None);
    for k in 0..70u32 {
        update(
            &mut file,
            &[(5, info(&format!("2026010{}120000Z", k % 9 + 1)))],
            13,
        );
    }
    update(
        &mut file,
        &[(
            1,
            b"<< /Type /Catalog /Pages 2 0 R /Metadata 6 0 R /Lang (en) >>".to_vec(),
        )],
        13,
    );
    let pdf = Pdf::open(&file, None, &Limits::default(), &never).unwrap();
    assert_eq!(pdf.page_count(), 3, "{:?}", pdf.issues());
    let r = pdf.revisions(&never);
    assert_eq!(r.list.last().unwrap().number, 72);
    assert_eq!(r.list.last().unwrap().changes, ["changed the catalog"]);
}

/// When the chain is longer than the work bound, open says which sections it did not read
/// and finds the objects only they define by a scan.
#[test]
fn cut_chain_falls_back_to_a_scan() {
    let mut file = pages_doc(3, None);
    for k in 0..70u32 {
        update(
            &mut file,
            &[(5, info(&format!("2026010{}120000Z", k % 9 + 1)))],
            13,
        );
    }
    update(
        &mut file,
        &[(1, b"<< /Type /Catalog /Pages 2 0 R /Lang (en) >>".to_vec())],
        13,
    );
    let limits = Limits {
        max_items: 200,
        ..Limits::default()
    };
    let pdf = Pdf::open(&file, None, &limits, &never).unwrap();
    assert_eq!(pdf.page_count(), 3, "{:?}", pdf.issues());
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.what.contains("not read")),
        "{:?}",
        pdf.issues()
    ); // The file is not damaged: the cut is said once, as a limit.
    assert!(
        !pdf.issues().iter().any(|i| i.what.contains("rebuilt")),
        "{:?}",
        pdf.issues()
    );
}

/// A call stopped by the caller is not kept: a later call reads again.
#[test]
fn stopped_revisions_are_read_again() {
    let mut file = base(b"BT (x) Tj ET");
    update(&mut file, &[(5, info("20260202120000Z"))], 6);
    update(&mut file, &[(5, info("20260303120000Z"))], 6);
    let pdf = Pdf::open(&file, None, &Limits::default(), &never).unwrap();
    let stopped = pdf.revisions(&|| true);
    assert!(stopped.issues.iter().any(|i| i.kind == IssueKind::Limit));
    let full = pdf.revisions(&never);
    assert_eq!(full.list.len(), 3, "{:?}", full.list);
    assert!(full.issues.is_empty(), "{:?}", full.issues);
    assert_eq!(full.list[2].changes, ["changed document information"]);
}

/// Entries cost what they take in memory, so a save with two million entries stops at the
/// object limit.
#[test]
fn entries_are_charged_what_they_cost() {
    let mut file = base(b"BT (x) Tj ET");
    let prev = file.windows(9).rposition(|w| w == b"startxref").unwrap();
    let prev: String = file[prev + 9..]
        .iter()
        .skip_while(|c| c.is_ascii_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .map(|c| char::from(*c))
        .collect();
    let n = 1_990_000usize;
    let xref = file.len();
    file.extend(format!("xref\n0 {n}\n").as_bytes());
    file.extend(b"0000000000 65535 f \n");
    for _ in 1..n {
        file.extend(b"0000000000 00001 f \n");
    }
    file.extend(
        format!(
            "trailer\n<< /Size {n} /Root 1 0 R /Info 5 0 R /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let pdf = Pdf::open(&file, None, &hostile(), &never).unwrap();
    drop(file);
    let r = pdf.revisions(&never);
    assert!(
        r.issues.iter().any(|i| i.kind == IssueKind::Limit),
        "{:?}",
        r.issues
    );
    common::assert_rss_bounded();
}

/// The revisions are read once per file; a second call costs nothing.
#[test]
fn revisions_are_read_once() {
    let mut file = base(b"BT (x) Tj ET");
    update(&mut file, &[(5, info("20260202120000Z"))], 6);
    let pdf = Pdf::open(&file, None, &Limits::default(), &never).unwrap();
    let a: *const Revisions = &*pdf.revisions(&never);
    let b: *const Revisions = &*pdf.revisions(&never);
    assert!(std::ptr::eq(a, b));
}

/// A later save that changes only the /Length object of a page's content stream changes
/// that page's content.
#[test]
fn later_length_change_is_seen() {
    let mut f = b"%PDF-1.7\n".to_vec();
    let mut offs = BTreeMap::new();
    let mut put = |f: &mut Vec<u8>, n: u32, body: &[u8]| {
        offs.insert(n, f.len());
        f.extend(format!("{n} 0 obj\n").as_bytes());
        f.extend(body);
        f.extend(b"\nendobj\n");
    };
    put(&mut f, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    put(&mut f, 2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    put(
        &mut f,
        3,
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
    );
    put(&mut f, 4, b"<< /Length 6 0 R >>\nstream\nBT ET\nendstream");
    put(&mut f, 5, &info("20260101120000Z"));
    put(&mut f, 6, b"5");
    let xref = f.len();
    f.extend(b"xref\n0 7\n0000000000 65535 f \n");
    for off in offs.values() {
        f.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    f.extend(
        format!("trailer\n<< /Size 7 /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    update(&mut f, &[(6, b"6".to_vec())], 7);
    let pdf = Pdf::open(&f, None, &Limits::default(), &never).unwrap();
    let r = pdf.revisions(&never);
    assert!(
        r.list[1]
            .changes
            .contains(&"changed page 1 content".to_string()),
        "{:?}",
        r.list[1].changes
    );
}

/// When the chain is cut, the issue says the numbers count from the oldest save read.
#[test]
fn cut_chain_numbering_is_relative() {
    let mut file = base(b"BT (x) Tj ET");
    update(&mut file, &[(5, info("20260202120000Z"))], 6);
    update(&mut file, &[(5, info("20260303120000Z"))], 6);
    // The newest section names a /Prev that leads nowhere.
    let at = file.windows(6).rposition(|w| w == b"/Prev ").unwrap() + 6;
    let end = at + file[at..].iter().take_while(|c| c.is_ascii_digit()).count();
    let digits = end - at;
    file[at..end].copy_from_slice(format!("{:0digits$}", 3).as_bytes());
    let pdf = Pdf::open(&file, None, &Limits::default(), &never).unwrap();
    let r = pdf.revisions(&never);
    assert!(
        r.issues
            .iter()
            .any(|i| i.what.contains("count from the oldest save read")),
        "{:?}",
        r.issues
    );
}
