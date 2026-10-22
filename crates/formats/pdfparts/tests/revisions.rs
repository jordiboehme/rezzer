// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

mod common;

use common::updates::*;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use lopdf::xref::XrefType;
use lopdf::{Document, Object, dictionary};
use pdfparts::{IssueKind, Limits, Pdf, Revisions};

/// The allocator of the Rezzer binary: the memory these tests measure is the memory it
/// ships with.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

fn never() -> bool {
    false
}

fn corpus_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf")
        .join(name);
    std::fs::read(path).unwrap()
}

fn revisions_of(bytes: &[u8]) -> Revisions {
    let pdf = Pdf::open(bytes, None, &Limits::default(), &never).unwrap();
    pdf.revisions(&never).into_owned()
}

#[test]
fn three_revisions_with_page_content_change() {
    let bytes = corpus_bytes("qpdf-incremental-updates.pdf");
    let r = revisions_of(&bytes);
    assert_eq!(r.skipped, 0);
    let numbers: Vec<u32> = r.list.iter().map(|v| v.number).collect();
    assert_eq!(numbers, [1, 2, 3]);
    let ends: Vec<u64> = r.list.iter().map(|v| v.end).collect();
    assert_eq!(ends, [763, 930, bytes.len() as u64]);
    assert!(r.list[0].changes.is_empty(), "{:?}", r.list[0].changes);
    assert!(
        r.list[2]
            .changes
            .contains(&"changed page 1 content".to_string()),
        "{:?}",
        r.list[2].changes
    );
    assert!(r.issues.is_empty(), "{:?}", r.issues);
}

#[test]
fn signature_added_in_last_revision() {
    let bytes = corpus_bytes("irs-f1040-2025.pdf");
    let r = revisions_of(&bytes);
    // The file is linearized: its first section and the main section are one save.
    assert_eq!(r.list.len(), 2, "{:?}", r.list);
    let last = &r.list[1];
    assert_eq!(last.number, 2);
    assert_eq!(last.end, bytes.len() as u64);
    assert!(
        last.changes.contains(&"changed the catalog".to_string()),
        "{:?}",
        last.changes
    );
    assert_eq!(
        last.changes,
        [
            "changed the catalog",
            "changed object 2422 (dictionary)",
            "added object 2448 (Sig)",
            "added object 2449 (dictionary)",
        ]
    );
}

#[test]
fn field_value_changes() {
    let bytes = corpus_bytes("pdfbox-sf1199a-direct-deposit.pdf");
    let r = revisions_of(&bytes);
    // Linearized: the first section belongs to the first save, whose end is /L 71856.
    assert_eq!(r.list.len(), 2, "{:?}", r.list);
    assert_eq!(r.list[0].end, 71856);
    assert!(r.list[0].changes.is_empty());
    // The second save adds a hidden text field with a widget on each page. It changes no
    // value: value changes are covered by the fixtures below.
    assert_eq!(
        r.list[1].changes,
        [
            "changed object 153 (Metadata)",
            "changed document information",
            "changed object 157 (dictionary)",
            "added annotation Widget on page 1",
            "added object 258 (dictionary)",
            "added object 259 (dictionary)",
            "added annotation Widget on page 2",
            "added object 263 (dictionary)",
            "added annotation Widget on page 3",
            "added annotation Widget on page 4",
        ]
    );
    assert!(r.issues.is_empty(), "{:?}", r.issues);

    // A third save fills two fields of the real form.
    let mut bytes = bytes;
    let fields = [
        (
            106,
            b"<</T(Name)/DA(/Helv 9 Tf 0 0 0.5 rg)/FT/Tx/Ff 12582912/V(Jane Doe)/Kids[160 0 R 39 0 R 78 0 R]>>".to_vec(),
        ),
        (
            109,
            b"<</T(Phone)/DA(/Helv 9 Tf 0 0 0.5 rg)/FT/Tx/AA<</F 143 0 R/K 144 0 R>>/Ff 12582912/V(5551234)/Kids[163 0 R 40 0 R 81 0 R]>>".to_vec(),
        ),
    ];
    update_with(&mut bytes, &fields, "/Size 271 /Root 156 0 R /Info 154 0 R");
    let r = revisions_of(&bytes);
    assert_eq!(r.list.len(), 3);
    assert_eq!(
        r.list[2].changes,
        [
            "field Name value changed from nothing to Jane Doe",
            "field Phone value changed from nothing to 5551234",
        ]
    );
}

#[test]
fn eof_inside_stream_is_not_a_boundary() {
    let noise = b"BT (x) Tj ET\n%%EOF\nstartxref\n0\n%%EOF\n";
    let mut file = base(noise);
    let first_end = file.len();
    update(
        &mut file,
        &[(4, stream("", b"BT (changed) Tj ET\n%%EOF\n"))],
        6,
    );
    let r = revisions_of(&file);
    let numbers: Vec<u32> = r.list.iter().map(|v| v.number).collect();
    assert_eq!(numbers, [1, 2]);
    assert_eq!(r.list[0].end, first_end as u64);
    assert_eq!(r.list[1].end, file.len() as u64);
    assert_eq!(r.list[1].changes, ["changed page 1 content"]);
}

/// A linearized file: the first-page section at the start, the main section at the end, and
/// one update.
fn linearized() -> (Vec<u8>, usize) {
    let mut f = b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let lin = f.len();
    f.extend(b"6 0 obj\n<< /Linearized 1 /L 0000000000 /N 1 /O 3 /E 0 /T 0 >>\nendobj\n");
    let first_xref = f.len();
    // Filled in below, once the main section is written.
    let first_section = |main: usize, l6: usize| {
        format!(
            "xref\n6 1\n{l6:010} 00000 n \ntrailer\n<< /Size 7 /Root 1 0 R /Info 5 0 R /Prev {main:010} >>\nstartxref\n0\n%%EOF\n"
        )
    };
    let placeholder = first_section(0, lin);
    f.extend(placeholder.as_bytes());
    let mut offs = BTreeMap::new();
    let bodies: [(u32, Vec<u8>); 5] = [
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_vec(),
        ),
        (4, stream("", b"BT (Hello) Tj ET")),
        (5, info("20260101120000Z")),
    ];
    for (n, body) in bodies {
        offs.insert(n, f.len());
        f.extend(format!("{n} 0 obj\n").as_bytes());
        f.extend(body);
        f.extend(b"\nendobj\n");
    }
    let main = f.len();
    f.extend(b"xref\n0 6\n0000000000 65535 f \n");
    for off in offs.values() {
        f.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    f.extend(format!("trailer\n<< /Size 6 >>\nstartxref\n{first_xref}\n%%EOF\n").as_bytes());
    let first_end = f.len();
    // Now the real offsets of the first-page section and the length.
    let real = first_section(main, lin);
    assert_eq!(real.len(), placeholder.len());
    f[first_xref..first_xref + real.len()].copy_from_slice(real.as_bytes());
    let l = format!("{first_end:010}");
    let at = find(&f, b"/L ", 0).unwrap() + 3;
    f[at..at + 10].copy_from_slice(l.as_bytes());
    update(&mut f, &[(5, info("20260202120000Z"))], 7);
    (f, first_end)
}

#[test]
fn linearized_first_section_skipped() {
    let (file, first_end) = linearized();
    let r = revisions_of(&file);
    let numbers: Vec<u32> = r.list.iter().map(|v| v.number).collect();
    assert_eq!(numbers, [1, 2], "{:?}", r.list);
    assert_eq!(r.list[0].end, first_end as u64);
    assert_eq!(r.list[1].end, file.len() as u64);
    assert_eq!(r.list[1].changes, ["changed document information"]);
    let iso = |i: usize| r.list[i].modified.as_ref().and_then(|d| d.iso.clone());
    assert_eq!(iso(0).as_deref(), Some("2026-01-01T12:00:00Z"));
    assert_eq!(iso(1).as_deref(), Some("2026-02-02T12:00:00Z"));
    assert!(r.issues.is_empty(), "{:?}", r.issues);
}

#[test]
fn many_revisions_stop_at_cap() {
    let mut file = base(b"BT (x) Tj ET");
    // The first save and 9,999 updates: 10,000 revisions.
    for k in 0..9_999u32 {
        let date = format!("2026{:02}{:02}{:06}Z", 1 + k % 12, 1 + k % 28, k % 240_000);
        update(&mut file, &[(5, info(&date))], 6);
    }
    let started = Instant::now();
    let limits = Limits::default();
    let pdf = Pdf::open(&file, None, &limits, &never).unwrap();
    let r = pdf.revisions(&never);
    let took = started.elapsed();
    eprintln!("{} bytes, open and revisions in {took:?}", file.len());
    assert!(took.as_secs_f64() < 5.0, "{took:?}");
    assert_eq!(r.skipped, 10_000 - limits.max_revisions);
    assert_eq!(r.list.len(), limits.max_revisions);
    assert_eq!(r.list.last().unwrap().number, 10_000);
    assert_eq!(r.list[0].number, 10_000 - 64 + 1);
    assert_eq!(r.list.last().unwrap().end, file.len() as u64);
    for v in &r.list {
        assert_eq!(v.changes, ["changed document information"], "{}", v.number);
    }
    let skipped = r
        .skipped_issue
        .as_ref()
        .expect("the issue for the saves not compared");
    assert_eq!(skipped.kind, IssueKind::Limit);
    assert!(skipped.what.contains("only the newest 64"), "{skipped:?}");
    assert!(r.issues.is_empty(), "{:?}", r.issues);
    common::assert_rss_bounded();
}

#[test]
fn stop_between_revisions_is_a_limit() {
    let mut file = base(b"BT (x) Tj ET");
    for k in 0..5u32 {
        update(
            &mut file,
            &[(5, info(&format!("2026010{}120000Z", k + 1)))],
            6,
        );
    }
    let pdf = Pdf::open(&file, None, &Limits::default(), &never).unwrap();
    let r = pdf.revisions(&|| true);
    assert!(
        r.issues.iter().any(|i| i.kind == IssueKind::Limit),
        "{:?}",
        r.issues
    );
}

/// A form with a text field, a signature field and one page; updates fill the text field,
/// add a page and an annotation, and sign.
#[test]
fn field_page_annotation_and_signature_words() {
    let mut f = b"%PDF-1.7\n".to_vec();
    let mut offs = BTreeMap::new();
    let mut put = |f: &mut Vec<u8>, n: u32, body: &[u8]| {
        offs.insert(n, f.len());
        f.extend(format!("{n} 0 obj\n").as_bytes());
        f.extend(body);
        f.extend(b"\nendobj\n");
    };
    put(
        &mut f,
        1,
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R 7 0 R] >> >>",
    );
    put(&mut f, 2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    put(
        &mut f,
        3,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [6 0 R 7 0 R] >>",
    );
    put(&mut f, 4, &stream("", b"BT (Form) Tj ET"));
    put(&mut f, 5, &info("20260101120000Z"));
    put(
        &mut f,
        6,
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (Name) /V (Alice) /Rect [0 0 10 10] /P 3 0 R >>",
    );
    put(
        &mut f,
        7,
        b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Sign) /Rect [0 0 10 10] /P 3 0 R >>",
    );
    let xref = f.len();
    f.extend(b"xref\n0 8\n0000000000 65535 f \n");
    for off in offs.values() {
        f.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    f.extend(
        format!("trailer\n<< /Size 8 /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    // Revision 2: a new value for Name.
    update(
        &mut f,
        &[(
            6,
            b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (Name) /V (Bob) /Rect [0 0 10 10] /P 3 0 R >>"
                .to_vec(),
        )],
        8,
    );
    // Revision 3: a second page with a text note.
    update(
        &mut f,
        &[
            (2, b"<< /Type /Pages /Kids [3 0 R 8 0 R] /Count 2 >>".to_vec()),
            (
                8,
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 9 0 R /Annots [10 0 R] >>"
                    .to_vec(),
            ),
            (9, stream("", b"BT (Two) Tj ET")),
            (
                10,
                b"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (Note) >>".to_vec(),
            ),
        ],
        11,
    );
    // Revision 4: signed, and a new signature field.
    update(
        &mut f,
        &[
            (
                7,
                b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Sign) /V 11 0 R /Rect [0 0 10 10] /P 3 0 R >>"
                    .to_vec(),
            ),
            (
                11,
                b"<< /Type /Sig /Filter /Adobe.PPKLite /Contents <00> /ByteRange [0 1 2 3] >>"
                    .to_vec(),
            ),
            (
                12,
                b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Second) /Rect [0 0 10 10] /P 8 0 R >>"
                    .to_vec(),
            ),
            (
                1,
                b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R 7 0 R 12 0 R] >> >>"
                    .to_vec(),
            ),
            (
                8,
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 9 0 R /Annots [10 0 R 12 0 R] >>"
                    .to_vec(),
            ),
        ],
        13,
    );
    // Revision 5: the second page is removed again.
    update(
        &mut f,
        &[(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec())],
        13,
    );
    let r = revisions_of(&f);
    let numbers: Vec<u32> = r.list.iter().map(|v| v.number).collect();
    assert_eq!(numbers, [1, 2, 3, 4, 5]);
    assert_eq!(
        r.list[1].changes,
        ["field Name value changed from Alice to Bob"]
    );
    assert_eq!(
        r.list[2].changes,
        ["added page 2", "added annotation Text on page 2"]
    );
    assert_eq!(
        r.list[3].changes,
        [
            "changed the catalog",
            "signed field Sign",
            "added signature field Second",
        ]
    );
    assert_eq!(r.list[4].changes, ["removed page 2"]);
    assert_eq!(after_last_eof(&f), f.len());
}

#[test]
fn long_values_are_cut() {
    let long = "x".repeat(200);
    let mut f = b"%PDF-1.7\n".to_vec();
    let mut offs = BTreeMap::new();
    let mut put = |f: &mut Vec<u8>, n: u32, body: &[u8]| {
        offs.insert(n, f.len());
        f.extend(format!("{n} 0 obj\n").as_bytes());
        f.extend(body);
        f.extend(b"\nendobj\n");
    };
    put(
        &mut f,
        1,
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [3 0 R] >> >>",
    );
    put(&mut f, 2, b"<< /Type /Pages /Kids [] /Count 0 >>");
    put(&mut f, 3, b"<< /FT /Tx /T (Long) /V (short) >>");
    put(&mut f, 4, b"<< >>");
    put(&mut f, 5, &info("20260101120000Z"));
    let xref = f.len();
    f.extend(b"xref\n0 6\n0000000000 65535 f \n");
    for off in offs.values() {
        f.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    f.extend(
        format!("trailer\n<< /Size 6 /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    update(
        &mut f,
        &[(
            3,
            format!("<< /FT /Tx /T (Long) /V ({long}) >>").into_bytes(),
        )],
        6,
    );
    let r = revisions_of(&f);
    let want = format!(
        "field Long value changed from short to {}...",
        "x".repeat(80)
    );
    assert_eq!(r.list[1].changes, [want]);
}

#[test]
fn corpus_files_list_revisions() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/corpus/pdf");
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
        .collect();
    names.sort();
    for path in names {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(pdf) = Pdf::open(&bytes, None, &Limits::default(), &never) else {
            continue;
        };
        let started = Instant::now();
        let r = pdf.revisions(&never);
        let changes: usize = r.list.iter().map(|v| v.changes.len()).sum();
        eprintln!(
            "{}: {} revisions, {changes} changes, {} issues, {:?}",
            path.file_name().unwrap().to_string_lossy(),
            r.list.len(),
            r.issues.len(),
            started.elapsed()
        );
        for i in &r.issues {
            eprintln!("  {i:?}");
        }
        for v in r.list.iter().skip(1) {
            eprintln!("  {}: {:?}", v.number, v.changes);
        }
        assert!(r.list.iter().all(|v| v.end as usize <= bytes.len()));
        assert!(r.list.windows(2).all(|w| w[0].end < w[1].end));
    }
}

/// Every save changes the root of a large page tree, so every save walks all pages. The walks
/// count against one visit bound.
#[test]
fn page_walks_are_bounded_by_visits() {
    let pages = 20_000u32;
    let mut doc = Document::with_version("1.7");
    doc.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
    let kids: Vec<Object> = (0..pages).map(|k| Object::Reference((10 + k, 0))).collect();
    doc.objects.insert(
        (1, 0),
        Object::Dictionary(dictionary! { "Type" => "Catalog", "Pages" => (2, 0) }),
    );
    doc.objects.insert(
        (2, 0),
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages }),
    );
    doc.objects.insert(
        (5, 0),
        Object::Dictionary(dictionary! { "Title" => Object::string_literal("Big") }),
    );
    for k in 0..pages {
        doc.objects.insert(
            (10 + k, 0),
            Object::Dictionary(dictionary! { "Type" => "Page", "Parent" => (2, 0) }),
        );
    }
    doc.max_id = 10 + pages;
    doc.trailer.set("Root", (1, 0));
    doc.trailer.set("Info", (5, 0));
    let mut file = Vec::new();
    doc.save_to(&mut file).unwrap();
    file.push(b'\n');
    let kids: String = (0..pages).map(|k| format!("{} 0 R ", 10 + k)).collect();
    for k in 0..40 {
        let body = format!("<< /Type /Pages /Kids [{kids}] /Count {pages} /Rev {k} >>");
        update(&mut file, &[(2, body.into_bytes())], 10 + pages);
    }
    let limits = Limits {
        max_items: 300_000,
        ..Limits::default()
    };
    let pdf = Pdf::open(&file, None, &limits, &never).unwrap();
    let started = Instant::now();
    let r = pdf.revisions(&never);
    let took = started.elapsed();
    eprintln!("{} bytes, revisions in {took:?}", file.len());
    assert!(took.as_secs_f64() < 2.0, "{took:?}");
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.what.contains("visit")),
        "{:?}",
        r.issues
    );
    common::assert_rss_bounded();
}

/// One save adds many annotations without /P to a page that lists many already. Finding
/// their page must not scan every listed annotation for each new one.
#[test]
fn added_annotations_find_their_page_by_lookup() {
    let old = 50_000u32;
    let new = 50_000u32;
    let first = 10u32;
    let mut doc = Document::with_version("1.7");
    doc.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
    let annots: Vec<Object> = (0..old)
        .map(|k| Object::Reference((first + k, 0)))
        .collect();
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
            "Type" => "Page", "Parent" => (2, 0), "Annots" => (4, 0),
        }),
    );
    doc.objects.insert((4, 0), Object::Array(annots));
    doc.objects.insert(
        (5, 0),
        Object::Dictionary(dictionary! { "Title" => Object::string_literal("Notes") }),
    );
    for k in 0..old {
        doc.objects.insert(
            (first + k, 0),
            Object::Dictionary(dictionary! {
                "Type" => "Annot", "Subtype" => "Text",
                "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            }),
        );
    }
    doc.max_id = first + old;
    doc.trailer.set("Root", (1, 0));
    doc.trailer.set("Info", (5, 0));
    let mut file = Vec::new();
    doc.save_to(&mut file).unwrap();
    file.push(b'\n');
    let all: String = (0..old + new)
        .map(|k| format!("{} 0 R ", first + k))
        .collect();
    let mut objects = vec![(4, format!("[{all}]").into_bytes())];
    for k in old..old + new {
        objects.push((
            first + k,
            b"<< /Type /Annot /Subtype /Text /Rect [0 0 1 1] >>".to_vec(),
        ));
    }
    update(&mut file, &objects, first + old + new);
    let pdf = Pdf::open(&file, None, &Limits::default(), &never).unwrap();
    let started = Instant::now();
    let r = pdf.revisions(&never);
    let took = started.elapsed();
    eprintln!("{} bytes, revisions in {took:?}", file.len());
    assert!(took.as_secs_f64() < 2.0, "{took:?}");
    let last = r.list.last().unwrap();
    assert_eq!(last.changes.len(), new as usize);
    assert!(
        last.changes
            .iter()
            .all(|c| c == "added annotation Text on page 1")
    );
    common::assert_rss_bounded();
}

#[test]
fn pdf_stays_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Pdf>();
}
