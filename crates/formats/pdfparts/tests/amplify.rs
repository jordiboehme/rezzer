// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small files that make the reader repeat or keep work: the same stream read through many
//! offsets, many large object streams, shared images on many pages, many entries that point at
//! the wrong object, and many references.

mod common;

use std::time::{Duration, Instant};

use common::{assert_rss_bounded, zlib_fill};
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

/// Writes plain objects and a classic table.
struct Classic {
    out: Vec<u8>,
    offs: Vec<(u32, usize)>,
}

impl Classic {
    fn new() -> Classic {
        Classic {
            out: b"%PDF-1.5\n".to_vec(),
            offs: Vec::new(),
        }
    }

    fn object(&mut self, n: u32, body: &[u8]) {
        self.offs.push((n, self.out.len()));
        self.out.extend(format!("{n} 0 obj\n").as_bytes());
        self.out.extend(body);
        self.out.extend(b"\nendobj\n");
    }

    /// A table with one entry per object, each in its own subsection, and the trailer.
    fn table(&mut self, trailer: &str) -> usize {
        let at = self.out.len();
        self.out.extend(b"xref\n0 1\n0000000000 65535 f \n");
        for (n, o) in &self.offs {
            self.out
                .extend(format!("{n} 1\n{o:010} 00000 n \n").as_bytes());
        }
        let size = self.offs.iter().map(|o| o.0).max().unwrap_or(0) + 1;
        self.out
            .extend(format!("trailer\n<< /Size {size} /Root 1 0 R {trailer} >>\n").as_bytes());
        at
    }

    fn finish(mut self, trailer: &str) -> Vec<u8> {
        let at = self.table(trailer);
        self.out
            .extend(format!("startxref\n{at}\n%%EOF\n").as_bytes());
        self.out
    }
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
