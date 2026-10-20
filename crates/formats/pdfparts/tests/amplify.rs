// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small files that make the reader repeat or keep work: the same stream read through many
//! offsets, many large object streams, shared images, forms and links on many pages, many
//! entries that point at the wrong object, many references, overlapping pieces that the
//! catalog search or the members of an object stream read again and again.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Classic, assert_rss_bounded, zlib_fill};
use pdfparts::{IssueKind, Limits, OpenError, Pdf};

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

/// A stop callback that says stop once `after` has passed.
fn deadline(after: Duration) -> impl Fn() -> bool {
    let start = Instant::now();
    move || start.elapsed() > after
}

#[test]
fn xref_stream_aliases_decode_once() {
    // Sixteen classic sections whose /XRefStm points one byte earlier each, into the padding
    // before one cross-reference stream that unpacks to almost 64 MiB.
    let k = 16;
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    c.object(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    c.object(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>");
    c.out.extend(vec![b' '; k]);
    let x = c.out.len();
    // Padding keeps the stream under the ratio limit, so only the size counts.
    let mut data = zlib_fill(0, (64 << 20) - 1);
    data.extend(vec![b' '; 700 << 10]);
    c.out.extend(
        format!(
            "9 0 obj\n<< /Type /XRef /W [1 0 0] /Index [0 1] /Size 10 /Filter /FlateDecode /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    c.out.extend(&data);
    c.out.extend(b"\nendstream\nendobj\n");
    let mut prev: Option<usize> = None;
    for i in 0..k {
        let at = c.out.len();
        let mut t = format!(
            "xref\n0 0\ntrailer\n<< /Size 10 /Root 1 0 R /XRefStm {}",
            x - i
        );
        if let Some(p) = prev {
            t.push_str(&format!(" /Prev {p}"));
        }
        t.push_str(" >>\n");
        c.out.extend(t.as_bytes());
        prev = Some(at);
    }
    let bytes = c.finish(&format!("/Prev {}", prev.unwrap()));
    eprintln!("file {} KB", bytes.len() / 1024);
    let result = Pdf::open(&bytes, None, &limits(), &deadline(Duration::from_secs(20)));
    assert_rss_bounded();
    let pdf = result.unwrap();
    assert_eq!(pdf.page_count(), 1);
}

#[test]
fn object_stream_bytes_are_charged() {
    // Twelve object streams that each unpack to 60 MiB, all used by the catalog.
    let n = 12u32;
    let mut c = Classic::new();
    let refs: String = (0..n).map(|i| format!("{} 0 R ", 1000 + i)).collect();
    c.object(
        1,
        format!("<< /Type /Catalog /Pages 2 0 R /X [{refs}] >>").as_bytes(),
    );
    c.object(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    c.object(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>");
    let mut members = Vec::new();
    for i in 0..n {
        let head = format!("{} 0 ", 1000 + i);
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut e, head.as_bytes()).unwrap();
        std::io::Write::write_all(&mut e, b"null").unwrap();
        common::fill(&mut e, b' ', 60 << 20);
        let mut z = e.finish().unwrap();
        // Padding keeps the stream under the ratio limit, so only the size counts.
        z.extend(vec![b' '; 700 << 10]);
        let mut body = format!(
            "<< /Type /ObjStm /N 1 /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            head.len(),
            z.len()
        )
        .into_bytes();
        body.extend(&z);
        body.extend(b"\nendstream");
        c.object(10 + i, &body);
        members.push((1000 + i, 10 + i));
    }
    // An update section with a cross-reference stream lists the members.
    let bytes = c.finish("");
    let bytes = common::append_update(
        &bytes,
        &[],
        &members.iter().map(|(m, c)| (*m, *c, 0)).collect::<Vec<_>>(),
        1,
        "",
    );
    eprintln!("file {} KB", bytes.len() / 1024);
    let result = Pdf::open(&bytes, None, &limits(), &never);
    assert_rss_bounded();
    let err = result.unwrap_err();
    assert!(matches!(err, OpenError::Limit(_)), "{err:?}");
}

/// `pages` pages under one node whose `/Resources` has `images` image XObjects.
fn shared_images(pages: usize, images: usize) -> Vec<u8> {
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    let mut node =
        String::from("<< /Type /Pages /MediaBox [0 0 612 792] /Resources << /XObject << ");
    for i in 0..images {
        node.push_str(&format!("/i{i}<</Subtype/Image/Width 1/Height 1>>"));
    }
    node.push_str(" >> >> /Kids [");
    for p in 0..pages {
        node.push_str(&format!("{} 0 R ", p + 3));
    }
    node.push_str(&format!("] /Count {pages} >>"));
    c.object(2, node.as_bytes());
    for p in 0..pages {
        c.object(p as u32 + 3, b"<< /Type /Page /Parent 2 0 R >>");
    }
    c.finish("")
}

#[test]
fn shared_images_are_not_copied_per_page() {
    let bytes = shared_images(2000, 50_000);
    eprintln!("file {} KB", bytes.len() / 1024);
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let start = Instant::now();
    let pages = pdf.pages(&never);
    eprintln!("pages() took {:?}", start.elapsed());
    assert_eq!(pages.len(), 2000);
    assert_eq!(pages[0].images.len(), 50_000);
    assert_eq!(pages[1999].images.len(), 50_000);
    assert_rss_bounded();
}

#[test]
fn pages_stops_when_asked() {
    let bytes = shared_images(50, 10);
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let pages = pdf.pages(&|| true);
    assert!(pages.len() < 50, "{}", pages.len());
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.what.contains("stop")),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn entries_pointing_at_one_wrong_object_are_cheap() {
    // 20,000 table entries all point at `999999 0 obj` followed by 5 MB of spaces.
    let r = 20_000u32;
    let mut out = b"%PDF-1.4\n".to_vec();
    let refs: String = (0..r).map(|k| format!("{} 0 R ", k + 3)).collect();
    let one = out.len();
    out.extend(
        format!("1 0 obj\n<< /Type /Catalog /Pages 2 0 R /X [{refs}] >>\nendobj\n").as_bytes(),
    );
    let two = out.len();
    out.extend(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    let wrong = out.len();
    out.extend(b"999999 0 obj");
    out.extend(vec![b' '; 5 << 20]);
    out.extend(b"null\nendobj\n");
    let x = out.len();
    let n = r + 3;
    out.extend(format!("xref\n0 {n}\n0000000000 65535 f \n").as_bytes());
    out.extend(format!("{one:010} 00000 n \n{two:010} 00000 n \n").as_bytes());
    for _ in 0..r {
        out.extend(format!("{wrong:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n").as_bytes());
    let start = Instant::now();
    let result = Pdf::open(&out, None, &limits(), &deadline(Duration::from_secs(10)));
    let took = start.elapsed();
    eprintln!("open took {took:?}");
    let pdf = result.unwrap();
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert_eq!(pdf.page_count(), 0);
}

#[test]
fn many_references_do_not_refuse_every_page() {
    // Over two million references in three arrays, and one bad stream on page 2.
    let mut c = Classic::new();
    let refs = |n: usize| -> String {
        let mut s = String::from("[");
        for _ in 0..n {
            s.push_str("3 0 R ");
        }
        s.push(']');
        s
    };
    c.object(
        1,
        b"<< /Type /Catalog /Pages 2 0 R /X [10 0 R 11 0 R 12 0 R] >>",
    );
    c.object(2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>");
    c.object(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>");
    c.object(
        4,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Contents 5 0 R >>",
    );
    let bomb = zlib_fill(b' ', 32 << 20);
    let mut body = format!(
        "<< /Filter /FlateDecode /Length {} >>\nstream\n",
        bomb.len()
    )
    .into_bytes();
    body.extend(&bomb);
    body.extend(b"\nendstream");
    c.object(5, &body);
    for i in 0..3 {
        c.object(10 + i, refs(800_000).as_bytes());
    }
    let bytes = c.finish("");
    let pdf = Pdf::open(&bytes, None, &Limits::default(), &never).unwrap();
    assert_eq!(pdf.refused_pages(), &[2]);
}

#[test]
fn rebuilt_files_are_charged_once() {
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    c.object(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    let mut big = String::from("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /W [");
    for _ in 0..100_000 {
        big.push_str("1 ");
    }
    big.push_str("] >>");
    c.object(3, big.as_bytes());
    let bytes = c.finish("");
    let charged = |b: &[u8]| -> u64 {
        let pdf = Pdf::open(b, None, &Limits::default(), &never).unwrap();
        let debug = format!("{pdf:?}");
        debug
            .split("object_bytes: ")
            .nth(1)
            .and_then(|s| s.split(',').next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap()
    };
    let intact = charged(&bytes);
    // Without its table the file is rebuilt from a scan.
    let at = bytes.windows(6).position(|w| w == b"\nxref\n").unwrap();
    let broken = bytes[..at + 1].to_vec();
    let rebuilt = charged(&broken);
    eprintln!("intact {intact}, rebuilt {rebuilt}");
    assert!(
        rebuilt < intact + intact / 4,
        "intact {intact}, rebuilt {rebuilt}"
    );
}

/// `pages` pages that each name one shared `/Resources` object with `images` images.
fn shared_resources_object(pages: usize, images: usize) -> Vec<u8> {
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: String = (0..pages).map(|p| format!("{} 0 R ", p + 4)).collect();
    c.object(
        2,
        format!("<< /Type /Pages /MediaBox [0 0 612 792] /Kids [{kids}] /Count {pages} >>")
            .as_bytes(),
    );
    let mut res = String::from("<< /XObject << ");
    for i in 0..images {
        res.push_str(&format!("/i{i}<</Subtype/Image/Width 1/Height 1>>"));
    }
    res.push_str(" >> >>");
    c.object(3, res.as_bytes());
    for p in 0..pages {
        c.object(
            p as u32 + 4,
            b"<< /Type /Page /Parent 2 0 R /Resources 3 0 R >>",
        );
    }
    c.finish("")
}

#[test]
fn shared_resources_object_is_scanned_once() {
    let bytes = shared_resources_object(2000, 50_000);
    eprintln!("file {} KB", bytes.len() / 1024);
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let start = Instant::now();
    let pages = pdf.pages(&deadline(Duration::from_secs(2)));
    eprintln!("pages() took {:?}", start.elapsed());
    assert_eq!(pages.len(), 2000);
    assert_eq!(pages[1999].images.len(), 50_000);
    assert!(Arc::ptr_eq(&pages[0].images, &pages[1999].images));
    assert_rss_bounded();
}

#[test]
fn shared_form_images_are_bounded() {
    // Every page has its own small /Resources that names one shared form with 50,000 images.
    let pages = 2000;
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: String = (0..pages).map(|p| format!("{} 0 R ", p + 4)).collect();
    c.object(
        2,
        format!("<< /Type /Pages /MediaBox [0 0 612 792] /Kids [{kids}] /Count {pages} >>")
            .as_bytes(),
    );
    let mut form = String::from("<< /Subtype /Form /Resources << /XObject << ");
    for i in 0..50_000 {
        form.push_str(&format!("/i{i}<</Subtype/Image/Width 1/Height 1>>"));
    }
    form.push_str(" >> >> /Length 0 >>\nstream\n\nendstream");
    c.object(3, form.as_bytes());
    for p in 0..pages {
        c.object(
            p as u32 + 4,
            b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /F 3 0 R >> >> >>",
        );
    }
    let bytes = c.finish("");
    eprintln!("file {} KB", bytes.len() / 1024);
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let start = Instant::now();
    let pages = pdf.pages(&deadline(Duration::from_secs(5)));
    let took = start.elapsed();
    eprintln!("pages() took {took:?}");
    assert_eq!(pages.len(), 2000);
    assert_eq!(pages[0].images.len(), 50_000);
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.what.contains("images")),
        "{:?}",
        pdf.issues()
    );
    assert_rss_bounded();
}

#[test]
fn shared_link_text_is_bounded() {
    // One link with a 1 MB address on every one of 1000 pages.
    let pages = 1000;
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: String = (0..pages).map(|p| format!("{} 0 R ", p + 5)).collect();
    c.object(
        2,
        format!("<< /Type /Pages /MediaBox [0 0 612 792] /Kids [{kids}] /Count {pages} >>")
            .as_bytes(),
    );
    let mut address = b"(".to_vec();
    address.extend(vec![b'a'; 1 << 20]);
    address.extend(b")");
    c.object(3, &address);
    // Each page has its own link, and every link names the one address.
    for p in 0..pages {
        c.object(
            p as u32 + 5,
            b"<< /Type /Page /Parent 2 0 R /Annots [<< /Type /Annot /Subtype /Link /Rect [0 0 1 1] /A << /S /URI /URI 3 0 R >> >>] >>",
        );
    }
    let bytes = c.finish("");
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    let links = pdf.links();
    eprintln!("{} links", links.len());
    assert_rss_bounded();
    assert!(!links.is_empty());
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.what.contains("link")),
        "{:?}",
        pdf.issues()
    );
}

/// `k` pieces that each open a literal string and never close it (`(` alone, inside an object
/// dictionary or inside a trailer), with no usable table, so the catalog must be looked for.
fn nested_strings(k: u32, shape: &str, close: bool) -> Vec<u8> {
    let mut b = b"%PDF-1.4\n".to_vec();
    b.extend(b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    b.extend(b"2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj\n");
    for n in 0..k {
        match shape {
            "object" => b.extend(format!("{} 0 obj (", n + 3).as_bytes()),
            "dict" => b.extend(format!("{} 0 obj << /A (", n + 3).as_bytes()),
            _ => b.extend(b"trailer << /A ("),
        }
    }
    if close {
        b.extend(vec![b')'; k as usize]);
    }
    b.extend(b"\nstartxref\n999999999\n%%EOF\n");
    b
}

#[test]
fn catalog_look_ahead_is_linear() {
    for shape in ["object", "dict", "trailer"] {
        for close in [false, true] {
            let bytes = nested_strings(60_000, shape, close);
            let start = Instant::now();
            let result = Pdf::open(&bytes, None, &limits(), &deadline(Duration::from_secs(10)));
            let took = start.elapsed();
            eprintln!(
                "{shape}, close {close}: {} KB, {took:?}, {:?}",
                bytes.len() / 1024,
                result.as_ref().map(|p| p.page_count())
            );
            assert!(took < Duration::from_secs(2), "{took:?}");
            match result {
                Ok(pdf) => assert_eq!(pdf.page_count(), 0),
                Err(OpenError::Limit(why)) => assert!(why.contains("catalog"), "{why}"),
                Err(e) => panic!("{e:?}"),
            }
        }
    }
}

#[test]
fn xref_stream_header_zeros_decode_once() {
    // Sixteen /XRefStm offsets into the leading zeros of one cross-reference stream header.
    let k = 16;
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    c.object(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    c.object(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>");
    let x = c.out.len();
    let mut data = zlib_fill(0, (64 << 20) - 1);
    data.extend(vec![b' '; 700 << 10]);
    c.out.extend(
        format!(
            "000000000000000009 0 obj\n<< /Type /XRef /W [1 0 0] /Index [0 1] /Size 10 /Filter /FlateDecode /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    c.out.extend(&data);
    c.out.extend(b"\nendstream\nendobj\n");
    let mut prev: Option<usize> = None;
    for i in 0..k {
        let at = c.out.len();
        let mut t = format!(
            "xref\n0 0\ntrailer\n<< /Size 10 /Root 1 0 R /XRefStm {}",
            x + i
        );
        if let Some(p) = prev {
            t.push_str(&format!(" /Prev {p}"));
        }
        t.push_str(" >>\n");
        c.out.extend(t.as_bytes());
        prev = Some(at);
    }
    let bytes = c.finish(&format!("/Prev {}", prev.unwrap()));
    let result = Pdf::open(&bytes, None, &limits(), &deadline(Duration::from_secs(20)));
    assert_rss_bounded();
    let pdf = result.unwrap();
    assert_eq!(pdf.page_count(), 1);
}

#[test]
fn white_space_runs_have_their_own_message() {
    // 100 entries point at different places in 5 MB of white space before a wrong header.
    let r = 100usize;
    let w = 5usize << 20;
    let mut out = b"%PDF-1.4\n".to_vec();
    let refs: String = (0..r).map(|k| format!("{} 0 R ", k + 3)).collect();
    let one = out.len();
    out.extend(
        format!("1 0 obj\n<< /Type /Catalog /Pages 2 0 R /X [{refs}] >>\nendobj\n").as_bytes(),
    );
    let two = out.len();
    out.extend(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    let run = out.len();
    out.extend(vec![b' '; w]);
    out.extend(b"999999 0 obj null\nendobj\n");
    let x = out.len();
    let n = r + 3;
    out.extend(format!("xref\n0 {n}\n0000000000 65535 f \n").as_bytes());
    out.extend(format!("{one:010} 00000 n \n{two:010} 00000 n \n").as_bytes());
    for k in 0..r {
        out.extend(format!("{:010} 00000 n \n", run + k * (w / r)).as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n").as_bytes());
    let err = Pdf::open(&out, None, &limits(), &deadline(Duration::from_secs(10))).unwrap_err();
    let OpenError::Limit(why) = err else {
        panic!("{err:?}");
    };
    assert!(why.contains("white space"), "{why}");
}

#[test]
fn object_stream_members_share_one_work_bound() {
    // 100,000 members of one object stream all start at 60 MiB of spaces before `null`.
    let n = 100_000u32;
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R /X 1000 0 R >>");
    c.object(2, b"<< /Type /Pages /Kids [] /Count 0 >>");
    let mut head = String::new();
    for i in 0..n {
        head.push_str(&format!("{} 0 ", 1000 + i));
    }
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut e, head.as_bytes()).unwrap();
    common::fill(&mut e, b' ', 60 << 20);
    std::io::Write::write_all(&mut e, b"null").unwrap();
    let mut z = e.finish().unwrap();
    // Padding keeps the stream under the ratio limit.
    z.extend(vec![b' '; 700 << 10]);
    let mut body = format!(
        "<< /Type /ObjStm /N {n} /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
        head.len(),
        z.len()
    )
    .into_bytes();
    body.extend(&z);
    body.extend(b"\nendstream");
    c.object(10, &body);
    let bytes = c.finish("");
    let members: Vec<(u32, u32, u32)> = (0..n).map(|i| (1000 + i, 10, i)).collect();
    let bytes = common::append_update(&bytes, &[], &members, 1, "");
    eprintln!("file {} KB", bytes.len() / 1024);
    let start = Instant::now();
    let result = Pdf::open(&bytes, None, &limits(), &deadline(Duration::from_secs(10)));
    let took = start.elapsed();
    eprintln!(
        "open took {took:?}: {:?}",
        result.as_ref().map(|p| p.page_count())
    );
    assert!(took < Duration::from_secs(2), "{took:?}");
    match result {
        Ok(pdf) => assert_eq!(pdf.page_count(), 0),
        Err(OpenError::Limit(why)) => assert!(why.contains("white space"), "{why}"),
        Err(e) => panic!("{e:?}"),
    }
    assert_rss_bounded();
}

/// One page whose `k` annotations hold a string that never closes, written before 1 MB of
/// content. Without `endobj` the broken objects run into the content.
fn unclosed_notes(k: u32, endobj: bool) -> Vec<u8> {
    let mut c = Classic::new();
    c.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    for i in 0..k {
        let body = format!(
            "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (broken note {i} >>"
        );
        if endobj {
            c.object(10 + i, body.as_bytes());
        } else {
            c.offs.push((10 + i, c.out.len()));
            c.out
                .extend(format!("{} 0 obj\n{body}\n", 10 + i).as_bytes());
        }
    }
    let annots: Vec<String> = (0..k).map(|i| format!("{} 0 R", 10 + i)).collect();
    c.object(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    c.object(
        3,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [{}] >>",
            annots.join(" ")
        )
        .as_bytes(),
    );
    let mut content = b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET\n% ".to_vec();
    content.extend(vec![b'x'; 1_000_000]);
    content.push(b'\n');
    let mut body = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    body.extend(&content);
    body.extend(b"\nendstream");
    c.object(4, &body);
    c.finish("")
}

#[test]
fn broken_values_do_not_end_the_open() {
    let cases = [(16, true), (32, true), (64, true), (2000, false)];
    for (k, endobj) in cases {
        let bytes = unclosed_notes(k, endobj);
        let start = Instant::now();
        let result = Pdf::open(&bytes, None, &limits(), &deadline(Duration::from_secs(10)));
        let took = start.elapsed();
        eprintln!(
            "k {k}, endobj {endobj}: {} KB, {took:?}, {:?}",
            bytes.len() / 1024,
            result.as_ref().map(|p| p.page_count())
        );
        assert!(took < Duration::from_secs(1), "{took:?}");
        let pdf = result.unwrap();
        assert_eq!(pdf.page_count(), 1);
        let issues = pdf.issues();
        let damaged: Vec<_> = issues
            .iter()
            .filter(|i| i.kind == IssueKind::Damaged)
            .collect();
        eprintln!(
            "{} damaged issues, last {:?}",
            damaged.len(),
            damaged.last()
        );
        let grouped = damaged
            .iter()
            .any(|i| i.what.contains("too many broken values"));
        if endobj {
            // Each broken note costs a bounded read, so each one is named.
            let named = damaged
                .iter()
                .filter(|i| i.what.contains("string is not closed"))
                .count();
            assert_eq!(named, k as usize, "{damaged:?}");
            assert!(!grouped, "{damaged:?}");
        } else {
            assert!(grouped, "{damaged:?}");
        }
    }
    assert_rss_bounded();
}

#[test]
fn members_are_read_after_broken_values() {
    // Many notes whose `/Contents` never closes and that have no `endobj`, then two valid
    // annotations in an object stream with no separator between them.
    let k = 4000;
    let mut raw = common::Raw::new();
    let notes: Vec<String> = (0..k).map(|i| format!("{} 0 R", 100 + i)).collect();
    raw.object(1, "<< /Type /Catalog /Pages 2 0 R >>");
    raw.object(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    raw.object(
        3,
        &format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [{} 50 0 R] >>",
            notes.join(" ")
        ),
    );
    for i in 0..k {
        raw.unclosed(
            100 + i,
            &format!("<< /Type /Annot /Subtype /Text /Rect [0 0 1 1] /Contents (broken {i} >>"),
        );
    }
    let members: [&[u8]; 2] = [
        b"<</Type/Annot/Subtype/Text/Rect[0 0 1 1]/Contents(fine)/Popup 51 0 R>>",
        b"<</Type/Annot/Subtype/Popup/Rect[0 0 1 1]>>",
    ];
    let head = format!("50 0 51 {} ", members[0].len());
    let mut plain = head.clone().into_bytes();
    plain.extend(members.concat());
    let dict = format!(
        "/Type /ObjStm /N 2 /First {} /Filter /FlateDecode",
        head.len()
    );
    raw.stream(60, &dict, &common::zlib(&plain));
    raw.member(50, 60, 0);
    raw.member(51, 60, 1);
    let bytes = raw.finish(1, "");
    let start = Instant::now();
    let pdf = Pdf::open(&bytes, None, &limits(), &never).unwrap();
    eprintln!("{} KB, {:?}", bytes.len() / 1024, start.elapsed());
    assert_eq!(pdf.page_count(), 1);
    // The bound for broken values is spent.
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.what.contains("too many broken values")),
        "{:?}",
        pdf.issues()
    );
    let doc = lopdf::Document::load_mem(pdf.canonical()).unwrap();
    let fine = doc.get_object((50, 0)).unwrap().as_dict().unwrap();
    assert_eq!(
        fine.get(b"Contents").unwrap().as_str().unwrap(),
        b"fine".as_slice()
    );
    assert!(doc.get_object((51, 0)).is_ok());
}
