// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Files that tried to get unmeasured bytes past the old guard, rebuilt by hand. Each object
//! stream here holds a member the document uses, so it must be read.

mod common;

use std::time::Instant;

use common::{Raw, append_update, assert_rss_bounded, bomb_2g, zlib_fill};
use lopdf::{Document, Object, Stream, dictionary};
use pdfparts::{Limits, OpenError, Pdf};

/// The allocator of the Rezzer binary: the memory these tests measure is the memory it
/// ships with.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

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

fn open(bytes: &[u8]) -> Result<Pdf, OpenError> {
    Pdf::open(bytes, None, &limits(), &never)
}

fn assert_limit_bounded(bytes: &[u8]) {
    let result = open(bytes);
    assert_rss_bounded();
    let err = result.unwrap_err();
    assert!(matches!(err, OpenError::Limit(_)), "{err:?}");
}

/// Catalog 1, page tree 2 and page 3. `extra` goes into the catalog.
fn base(raw: &mut Raw, extra: &str) {
    raw.object(1, &format!("<< /Type /Catalog /Pages 2 0 R {extra} >>"));
    raw.object(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    raw.object(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>");
}

#[test]
fn comment_in_object_header() {
    // An object stream whose header has a comment between the generation and `obj`.
    let mut raw = Raw::new();
    base(&mut raw, "/Bomb 150 0 R");
    raw.bomb_object_stream("10 0 %c\nobj", 10, 150, &bomb_2g());
    assert_limit_bounded(&raw.finish(1, ""));
}

#[test]
fn comment_in_object_header_is_read() {
    let mut raw = Raw::new();
    raw.object_with_header(
        "1 %one\n0 %zero\nobj",
        1,
        b"<< /Type /Catalog /Pages 2 0 R >>",
    );
    raw.object(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    raw.object(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>");
    let pdf = open(&raw.finish(1, "")).unwrap();
    assert_eq!(pdf.page_count(), 1);
}

#[test]
fn decoy_stream_before_object_stream() {
    // An unterminated stream right before the object stream swallowed it for the old scan.
    let mut raw = Raw::new();
    base(&mut raw, "/Bomb 150 0 R");
    raw.junk(b"200 0 obj\n<< /Length 1 0 R >>\nstream\n");
    raw.bomb_object_stream("10 0 obj", 10, 150, &bomb_2g());
    assert_limit_bounded(&raw.finish(1, ""));
}

#[test]
fn leading_zero_object_numbers() {
    let mut raw = Raw::new();
    base(&mut raw, "/Bomb 150 0 R");
    raw.bomb_object_stream("0010 00 obj", 10, 150, &bomb_2g());
    assert_limit_bounded(&raw.finish(1, ""));
}

#[test]
fn leading_zero_object_numbers_are_read() {
    let mut raw = Raw::new();
    raw.object_with_header("0001 0 obj", 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    raw.object_with_header(
        "00002 000 obj",
        2,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    );
    raw.object(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>");
    let pdf = open(&raw.finish(1, "")).unwrap();
    assert_eq!(pdf.page_count(), 1);
}

#[test]
fn literal_encrypt_in_plain_file() {
    // `/Encrypt` as a key and inside a string of a plain file.
    let mut raw = Raw::new();
    base(&mut raw, "/Encrypt 5 0 R /Note (/Encrypt) /Bomb 150 0 R");
    raw.object(5, "<< /Filter /Standard /V 2 /R 3 >>");
    raw.bomb_object_stream("10 0 obj", 10, 150, &bomb_2g());
    assert_limit_bounded(&raw.finish(1, "%/Encrypt\n"));
}

#[test]
fn literal_encrypt_in_plain_file_opens_as_plain() {
    let mut raw = Raw::new();
    base(&mut raw, "/Encrypt 5 0 R /Note (/Encrypt)");
    raw.object(5, "<< /Filter /Standard /V 2 /R 3 >>");
    let pdf = open(&raw.finish(1, "")).unwrap();
    assert_eq!(pdf.encryption(), None);
    assert_eq!(pdf.page_count(), 1);
}

/// Pages whose content streams take their `/Length` from member 150 of one object stream that
/// also holds a string of `mib` MiB.
fn length_in_object_stream(pages: u32, mib: usize) -> Vec<u8> {
    let mut raw = Raw::new();
    raw.object(1, "<< /Type /Catalog /Pages 2 0 R /Big 151 0 R >>");
    let kids: String = (0..pages).map(|i| format!("{} 0 R ", 1000 + i)).collect();
    raw.object(
        2,
        &format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"),
    );
    let mut big = b"(".to_vec();
    big.resize(mib << 20, b'a');
    big.push(b')');
    raw.object_stream(10, &[(150, b"5"), (151, &big)]);
    drop(big);
    for i in 0..pages {
        raw.object(
            1000 + i,
            &format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R >>",
                5000 + i
            ),
        );
        raw.object(5000 + i, "<< /Length 150 0 R >>\nstream\nabcde\nendstream");
    }
    raw.finish(1, "")
}

/// Opens the file of [`length_in_object_stream`] and checks it, with the work bound of
/// `measured` steps (see [`common::assert_work`]).
fn check_length_in_object_stream(pages: u32, mib: usize, measured: u64) {
    let bytes = length_in_object_stream(pages, mib);
    eprintln!("file {} KB", bytes.len() / 1024);
    let (pdf, steps, took) = common::counted(|| open(&bytes).unwrap());
    eprintln!("unpacked {}", pdf.unpacked());
    assert_eq!(pdf.page_count(), pages);
    assert!(pdf.refused_pages().is_empty(), "{:?}", pdf.issues());
    // The object stream is unpacked once, not once per page.
    assert!(
        pdf.unpacked() < (2 * mib as u64) << 20,
        "{}",
        pdf.unpacked()
    );
    common::assert_work(&format!("{pages} pages, {mib} MiB"), steps, measured, took);
    assert_rss_bounded();
}

#[test]
fn length_points_into_object_stream() {
    // The steps were measured once (2026-10-27).
    check_length_in_object_stream(20, 4, 8_572_668);
}

#[test]
fn length_points_into_large_object_stream() {
    // The steps were measured once (2026-10-27).
    check_length_in_object_stream(2000, 15, 32_645_866);
}

#[test]
fn many_small_object_streams_charged() {
    // 15 MiB of `0 ` deflates to a few KB and parses to millions of numbers.
    let mut raw = Raw::new();
    base(&mut raw, "/Big 150 0 R");
    let mut zeros = b"[".to_vec();
    while zeros.len() < 15 << 20 {
        zeros.extend(b"0 ");
    }
    zeros.push(b']');
    raw.object_stream(10, &[(150, &zeros)]);
    drop(zeros);
    let bytes = raw.finish(1, "");
    eprintln!("file {} KB", bytes.len() / 1024);
    let result = open(&bytes);
    assert_rss_bounded();
    let err = result.unwrap_err();
    assert!(matches!(err, OpenError::Limit(_)), "{err:?}");
}

#[test]
fn object_stream_header_of_zeros_is_charged() {
    // `/N` claims millions of members and the header is all `0 0`.
    let mut raw = Raw::new();
    base(&mut raw, "/Big 150 0 R");
    let mut spaced = Vec::new();
    {
        use std::io::Write;
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        let chunk: Vec<u8> = b"0 ".iter().copied().cycle().take(1 << 20).collect();
        for _ in 0..15 {
            e.write_all(&chunk).unwrap();
        }
        spaced.extend(e.finish().unwrap());
    }
    raw.object(
        10,
        &format!(
            "<< /Type /ObjStm /N 4000000 /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            15 << 20,
            spaced.len()
        ),
    );
    // The object above ends early; the data follows as its own piece.
    let at = raw.out.len() - b"\nendobj\n".len();
    raw.out.truncate(at);
    raw.out.extend(&spaced);
    raw.out.extend(b"\nendstream\nendobj\n");
    raw.member(150, 10, 0);
    let bytes = raw.finish(1, "");
    let result = open(&bytes);
    assert_rss_bounded();
    // Each listed member costs a value, so four million of them pass the item limit.
    let err = result.unwrap_err();
    assert!(matches!(err, OpenError::Limit(_)), "{err:?}");
}

#[test]
fn encrypted_object_stream_bomb() {
    use lopdf::{EncryptionState, EncryptionVersion, Permissions};
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    // lopdf's writer leaves out object streams, so the type is patched in afterwards.
    let container = doc.add_object(
        Stream::new(
            dictionary! { "Type" => "ObjStX", "N" => 1, "First" => 4, "Filter" => "FlateDecode" },
            bomb_2g(),
        )
        .with_compression(false),
    );
    let id = Object::string_literal(b"0123456789abcdef".to_vec());
    doc.trailer.set("ID", vec![id.clone(), id]);
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &doc,
        owner_password: "owner",
        user_password: "",
        key_length: 128,
        permissions: Permissions::all(),
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    let encrypt = doc.trailer.get(b"Encrypt").unwrap().as_reference().unwrap();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    let at = bytes.windows(7).position(|w| w == b"/ObjStX").unwrap();
    bytes[at + 6] = b'm';
    // A new catalog uses member 900 of the object stream. It holds no string, so it needs no
    // encryption.
    let bytes = append_update(
        &bytes,
        &[(
            800,
            format!(
                "<< /Type /Catalog /Pages {} {} R /Bomb 900 0 R >>",
                pages_id.0, pages_id.1
            ),
        )],
        &[(900, container.0, 0)],
        800,
        &format!(
            "/Encrypt {} {} R /ID [<30313233343536373839616263646566><30313233343536373839616263646566>]",
            encrypt.0, encrypt.1
        ),
    );
    assert_limit_bounded(&bytes);
}

/// `pages` pages under one node whose `/Resources` has `entries` fonts. The last page has a
/// content stream that unpacks past the limits.
fn shared_resources(pages: usize, entries: usize) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = vec![0usize; pages + 4];
    offs[1] = out.len();
    out.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    offs[2] = out.len();
    let mut d = String::from("2 0 obj\n<< /Type /Pages /Resources << /Font << ");
    for i in 0..entries {
        d.push_str(&format!("/F{i} 1 0 R "));
    }
    d.push_str(">> >> /MediaBox [0 0 612 792] /Kids [");
    for p in 0..pages {
        d.push_str(&format!("{} 0 R ", p + 4));
    }
    d.push_str(&format!("] /Count {pages} >>\nendobj\n"));
    out.extend(d.as_bytes());
    let bomb = zlib_fill(b' ', 32 << 20);
    offs[3] = out.len();
    out.extend(
        format!(
            "3 0 obj\n<< /Filter /FlateDecode /Length {} >>\nstream\n",
            bomb.len()
        )
        .as_bytes(),
    );
    out.extend(&bomb);
    out.extend(b"\nendstream\nendobj\n");
    for p in 0..pages {
        offs[p + 4] = out.len();
        let contents = if p + 1 == pages {
            "/Contents 3 0 R"
        } else {
            ""
        };
        out.extend(
            format!(
                "{} 0 obj\n<< /Type /Page /Parent 2 0 R {contents} >>\nendobj\n",
                p + 4
            )
            .as_bytes(),
        );
    }
    let x = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", pages + 4).as_bytes());
    for o in &offs[1..] {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
            pages + 4
        )
        .as_bytes(),
    );
    out
}

#[test]
fn resources_clone_per_page() {
    let bytes = shared_resources(500, 20_000);
    eprintln!("file {} KB", bytes.len() / 1024);
    let start = Instant::now();
    let pdf = open(&bytes).unwrap();
    let pages = pdf.pages(&never);
    eprintln!("took {:?}", start.elapsed());
    assert_eq!(pages.len(), 500);
    assert!(pages[0].has_fonts);
    assert_eq!(pdf.refused_pages(), &[500]);
    assert_rss_bounded();
}

#[test]
fn label_prefix_decode_time() {
    let pages = 200;
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = vec![0usize; pages + 3];
    offs[1] = out.len();
    out.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [0 << /S /D /P (");
    out.resize(out.len() + (16 << 20), b'a');
    out.extend(b") >>] >> >>\nendobj\n");
    offs[2] = out.len();
    let mut d = String::from("2 0 obj\n<< /Type /Pages /MediaBox [0 0 612 792] /Kids [");
    for p in 0..pages {
        d.push_str(&format!("{} 0 R ", p + 3));
    }
    d.push_str(&format!("] /Count {pages} >>\nendobj\n"));
    out.extend(d.as_bytes());
    for p in 0..pages {
        offs[p + 3] = out.len();
        out.extend(
            format!("{} 0 obj\n<< /Type /Page /Parent 2 0 R >>\nendobj\n", p + 3).as_bytes(),
        );
    }
    let x = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", pages + 3).as_bytes());
    for o in &offs[1..] {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
            pages + 3
        )
        .as_bytes(),
    );
    let pdf = open(&out).unwrap();
    let (labels, steps, took) = common::counted(|| pdf.pages(&never));
    assert_eq!(labels.len(), pages);
    assert_eq!(labels[0].label.as_ref().map(|l| l.len()), Some(257));
    // Measured once (2026-10-27): the prefix is cut before it is decoded, so the 16 MiB are
    // not decoded once per page, nor once at all.
    common::assert_work("page labels", steps, 1_034, took);
}

/// A classic file whose catalog uses object `n` as `/Big`, and `extra` objects (number, body).
fn classic_with(n: u64, extra: &[(u64, String)]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    let mut objects = vec![
        (
            1u64,
            format!("<< /Type /Catalog /Pages 2 0 R /Big {n} 0 R >>"),
        ),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
        (
            3,
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string(),
        ),
    ];
    objects.extend(extra.iter().cloned());
    for (k, body) in &objects {
        offs.push((*k, out.len()));
        out.extend(format!("{k} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let x = out.len();
    out.extend(b"xref\n0 1\n0000000000 65535 f \n");
    for (k, at) in &offs {
        out.extend(format!("{k} 1\n{at:010} 00000 n \n").as_bytes());
    }
    let size = objects.iter().map(|o| o.0).max().unwrap() + 1;
    out.extend(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n").as_bytes(),
    );
    out
}

#[test]
fn huge_object_numbers_are_bounded() {
    // The last figure is the steps of each case, measured once (2026-10-27).
    for (n, measured) in [(4_294_967_295u64, 872), (4_000_000_000, 875)] {
        let bytes = classic_with(n, &[(n, "<< /A 1 >>".to_string())]);
        let (pdf, steps, took) = common::counted(|| open(&bytes).unwrap());
        common::assert_work(&format!("object number {n}"), steps, measured, took);
        let doc = Document::load_mem(pdf.canonical()).unwrap();
        assert_eq!(doc.get_pages().len(), 1);
    }
}

#[test]
fn deep_nesting_matches_lopdf() {
    // lopdf reads at most 100 levels of arrays, dictionaries and string parentheses.
    let deep = format!("{}{}", "[".repeat(150), "]".repeat(150));
    let parens = format!("({}x{})", "(".repeat(150), ")".repeat(150));
    let bytes = classic_with(9, &[(9, deep), (10, parens.clone())]);
    let bytes = {
        // The catalog also uses object 10.
        let s = String::from_utf8(bytes).unwrap();
        s.replacen("/Big 9 0 R", "/Big 9 0 R /Str 10 0 R", 1)
    };
    // The offsets after the catalog moved, so the table is rebuilt from a scan.
    let pdf = open(bytes.as_bytes()).unwrap();
    let doc = Document::load_mem(pdf.canonical()).unwrap();
    // Every object of the canonical copy is read by lopdf too.
    for (&n, entry) in &doc.reference_table.entries {
        if let lopdf::xref::XrefEntry::Normal { generation, .. } = entry {
            assert!(
                doc.objects.contains_key(&(n, *generation)),
                "object {n} is lost"
            );
        }
    }
    let catalog = doc.catalog().unwrap();
    let s = catalog.get(b"Str").unwrap().as_reference().unwrap();
    assert_eq!(doc.get_object(s).unwrap().as_str().unwrap().len(), 301);
    let again = open(pdf.canonical()).unwrap();
    assert_eq!(again.canonical(), pdf.canonical());
}

#[test]
fn many_values_under_the_byte_budget_open() {
    // Three arrays of a million numbers each: more than 2M values in the file, but each
    // array is under max_items and all of them are under max_object_bytes.
    let array = |n: usize| {
        let mut s = String::from("[");
        for _ in 0..n {
            s.push_str("0 ");
        }
        s.push(']');
        s
    };
    let extra: Vec<(u64, String)> = (0..3).map(|i| (10 + i, array(1_000_000))).collect();
    let bytes = classic_with(10, &extra);
    let bytes =
        String::from_utf8(bytes)
            .unwrap()
            .replacen("/Big 10 0 R", "/Big [10 0 R 11 0 R 12 0 R]", 1);
    let pdf = Pdf::open(bytes.as_bytes(), None, &Limits::default(), &never).unwrap();
    assert_eq!(pdf.page_count(), 1);
    let debug = format!("{pdf:?}");
    eprintln!("{debug}");
    let values: usize = debug
        .split("values: ")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap();
    assert!(values > 2_000_000, "{values}");
}
