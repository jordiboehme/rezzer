// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small files that make the readers copy or walk the same data again and again: one large
//! string named by many entries, one large array shared by many elements or pages, many
//! attachments extracted one by one.

mod common;

use std::time::{Duration, Instant};

use common::{Classic, assert_rss_bounded};
use pdfparts::{IssueKind, Limits, Pdf};

fn never() -> bool {
    false
}

/// The memory tests keep the objects to 256 MiB, so the process stays under its bound.
fn limits() -> Limits {
    Limits {
        max_object_bytes: 256 << 20,
        ..Limits::default()
    }
}

/// The body of a literal string object of `len` bytes.
fn big_string(len: usize) -> Vec<u8> {
    let mut s = b"(".to_vec();
    s.extend(vec![b'a'; len]);
    s.extend(b")");
    s
}

fn has_limit(pdf: &Pdf, word: &str) -> bool {
    pdf.issues()
        .iter()
        .any(|i| i.kind == IssueKind::Limit && (i.what.contains(word) || i.location.contains(word)))
}

/// A tagged one-page file whose root has `n` elements, each with `element` as its body.
fn tagged(n: usize, element: &[u8], shared: &[u8]) -> Vec<u8> {
    let mut c = Classic::new();
    c.object(
        1,
        b"<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 4 0 R >>",
    );
    c.object(
        2,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>",
    );
    c.object(3, b"<< /Type /Page /Parent 2 0 R /StructParents 0 >>");
    let kids: String = (0..n).map(|i| format!("{} 0 R ", 10 + i)).collect();
    c.object(
        4,
        format!("<< /Type /StructTreeRoot /K [{kids}] >>").as_bytes(),
    );
    c.object(5, shared);
    for i in 0..n {
        c.object(10 + i as u32, element);
    }
    c.finish("")
}

#[test]
fn shared_alt_text_is_bounded() {
    // 500 elements name one 1 MB string as /Alt and /ActualText.
    let bytes = tagged(
        500,
        b"<< /S /Figure /Alt 5 0 R /ActualText 5 0 R >>",
        &big_string(1 << 20),
    );
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let tree = pdf.struct_tree().unwrap();
    eprintln!("{} elements", tree.elems.len());
    assert!(!tree.elems.is_empty() && tree.elems.len() < 500);
    let again = pdf.struct_tree().unwrap();
    assert!(std::ptr::eq(tree, again), "the tree is read once");
    assert_rss_bounded();
    assert!(has_limit(&pdf, "structure tree"), "{:?}", pdf.issues());
}

#[test]
fn shared_marked_content_array_is_bounded() {
    // 100 elements share one /K array of 500,000 marked content ids.
    let ids: String = (0..500_000).map(|i| format!("{} ", i % 10)).collect();
    let bytes = tagged(100, b"<< /S /P /K 5 0 R >>", format!("[{ids}]").as_bytes());
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let start = Instant::now();
    let tree = pdf.struct_tree().unwrap();
    let mcids: usize = tree.elems.iter().map(|e| e.mcids.len()).sum();
    eprintln!("{mcids} ids kept in {:?}", start.elapsed());
    assert!(mcids < 100 * 500_000);
    assert_rss_bounded();
    assert!(has_limit(&pdf, "structure tree"), "{:?}", pdf.issues());
}

#[test]
fn shared_text_in_annotations_attachments_layers_and_threads_is_bounded() {
    // 300 of each name one 1 MB string.
    let n = 300;
    let mut c = Classic::new();
    let specs: String = (0..n)
        .map(|i| format!("(f{i:08}) << /Type /Filespec /Desc 5 0 R >> "))
        .collect();
    let layers: String = (0..n).map(|_| "<< /Type /OCG /Name 5 0 R >> ").collect();
    let threads: String = (0..n).map(|_| "<< /I << /Title 5 0 R >> >> ").collect();
    c.object(
        1,
        format!(
            "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [{specs}] >> >> \
             /OCProperties << /OCGs [{layers}] >> /Threads [{threads}] >>"
        )
        .as_bytes(),
    );
    c.object(
        2,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>",
    );
    let annots: String = (0..n).map(|i| format!("{} 0 R ", 100 + i)).collect();
    c.object(
        3,
        format!("<< /Type /Page /Parent 2 0 R /Annots [{annots}] >>").as_bytes(),
    );
    c.object(5, &big_string(1 << 20));
    c.object(6, b"/R");
    for i in 0..n {
        c.object(
            100 + i as u32,
            b"<< /Type /Annot /Subtype /Text /Rect [0 0 1 1] /State 5 0 R /RT 6 0 R >>",
        );
    }
    let bytes = c.finish("");
    // Each reader gets its own open file, so only one result is held at a time.
    let open = || Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let pdf = open();
    let annots = pdf.annotations();
    assert!(!annots.is_empty() && annots.len() < n);
    drop(annots);
    assert!(has_limit(&pdf, "annotations"), "{:?}", pdf.issues());
    drop(pdf);
    let pdf = open();
    let files = pdf.embedded_files();
    assert!(!files.is_empty() && files.len() < n);
    assert!(has_limit(&pdf, "attachments"), "{:?}", pdf.issues());
    drop(pdf);
    let pdf = open();
    let layers = pdf.layers();
    assert!(!layers.is_empty() && layers.len() < n);
    drop(layers);
    assert!(has_limit(&pdf, "layers"), "{:?}", pdf.issues());
    drop(pdf);
    let pdf = open();
    let threads = pdf.threads();
    assert!(!threads.is_empty() && threads.len() < n);
    drop(threads);
    assert!(has_limit(&pdf, "article threads"), "{:?}", pdf.issues());
    assert_rss_bounded();
}

#[test]
fn attachments_are_listed_once() {
    let n = 5000;
    let mut c = Classic::new();
    c.object(
        1,
        b"<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 4 0 R >> >>",
    );
    c.object(
        2,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>",
    );
    c.object(3, b"<< /Type /Page /Parent 2 0 R >>");
    let names: String = (0..n)
        .map(|i| {
            format!(
                "(f{i:08}) << /Type /Filespec /F (f{i}) /EF << /F {} 0 R >> >> ",
                10 + i
            )
        })
        .collect();
    c.object(4, format!("<< /Names [{names}] >>").as_bytes());
    for i in 0..n {
        c.object(10 + i as u32, b"<< /Length 1 >>\nstream\nx\nendstream");
    }
    let bytes = c.finish("");
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let start = Instant::now();
    let files = pdf.embedded_files();
    assert_eq!(files.len(), n);
    for i in 0..n {
        assert_eq!(pdf.embedded_file_bytes(i).unwrap(), b"x");
    }
    let took = start.elapsed();
    eprintln!("{n} attachments extracted in {took:?}");
    assert!(took < Duration::from_secs(1), "{took:?}");
}

/// `pages` pages that share one /Annots array of `refs` references to one annotation.
fn shared_annots_array(pages: usize, refs: usize, subtype: &str) -> Vec<u8> {
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: String = (0..pages).map(|p| format!("{} 0 R ", p + 10)).collect();
    c.object(
        2,
        format!("<< /Type /Pages /MediaBox [0 0 612 792] /Kids [{kids}] /Count {pages} >>")
            .as_bytes(),
    );
    c.object(
        3,
        format!("<< /Type /Annot /Subtype /{subtype} /Rect [0 0 1 1] /Dest [10 0 R /Fit] >>")
            .as_bytes(),
    );
    let arr: String = (0..refs).map(|_| "3 0 R ").collect();
    c.object(4, format!("[{arr}]").as_bytes());
    for p in 0..pages {
        c.object(
            p as u32 + 10,
            b"<< /Type /Page /Parent 2 0 R /Annots 4 0 R >>",
        );
    }
    c.finish("")
}

#[test]
fn shared_annots_array_is_walked_once() {
    let bytes = shared_annots_array(10_000, 20_000, "Text");
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let start = Instant::now();
    let annots = pdf.annotations();
    let took = start.elapsed();
    eprintln!("annotations() {took:?}");
    assert_eq!(annots.len(), 1);
    assert_eq!(annots[0].page, 1);
    assert!(took < Duration::from_secs(1), "{took:?}");

    let bytes = shared_annots_array(10_000, 20_000, "Link");
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let start = Instant::now();
    let links = pdf.links();
    let took = start.elapsed();
    eprintln!("links() {took:?}, {} links", links.len());
    assert!(links.iter().all(|l| l.page == 1));
    assert!(took < Duration::from_secs(1), "{took:?}");
    assert_rss_bounded();
}

#[test]
fn shared_outline_title_is_bounded() {
    // 300 outline entries name one 1 MB title.
    let n = 300;
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>");
    c.object(
        2,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>",
    );
    c.object(3, b"<< /Type /Page /Parent 2 0 R >>");
    c.object(4, b"<< /Type /Outlines /First 10 0 R >>");
    c.object(5, &big_string(1 << 20));
    for i in 0..n {
        let next = if i + 1 < n {
            format!("/Next {} 0 R", 11 + i)
        } else {
            String::new()
        };
        c.object(
            10 + i as u32,
            format!("<< /Title 5 0 R /Parent 4 0 R {next} >>").as_bytes(),
        );
    }
    let bytes = c.finish("");
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let outline = pdf.outline();
    assert!(!outline.is_empty() && outline.len() < n);
    assert_rss_bounded();
    assert!(has_limit(&pdf, "outline"), "{:?}", pdf.issues());
}
