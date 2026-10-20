// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::Write;
use std::path::PathBuf;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use pdfparts::{IssueKind, Limits, OpenError, Pdf};

fn corpus(name: &str) -> Vec<u8> {
    std::fs::read(corpus_dir().join(name)).unwrap()
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/corpus/pdf")
}

fn never() -> bool {
    false
}

fn open(bytes: &[u8], password: Option<&str>) -> Result<Pdf, OpenError> {
    Pdf::open(bytes, password, &Limits::default(), &never)
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// A document with one page whose content stream is `content`. `tweak` may change the page
/// tree node before the file is written.
fn one_page(content: Stream, tweak: impl FnOnce(&mut Document, ObjectId, ObjectId)) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(content);
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Contents" => content_id,
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    tweak(&mut doc, pages_id, page_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn plain_content() -> Stream {
    Stream::new(
        dictionary! {},
        b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET".to_vec(),
    )
}

#[test]
fn opens_corpus_files_without_panic() {
    let mut names: Vec<_> = std::fs::read_dir(corpus_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".pdf"))
        .collect();
    names.sort();
    assert!(names.len() >= 31, "{names:?}");
    for name in &names {
        let bytes = corpus(name);
        let password = (name == "qpdf-user-password.pdf").then_some("view");
        let pdf = open(&bytes, password).unwrap_or_else(|e| panic!("{name}: {e}"));
        let pages = pdf.pages();
        assert!(!pages.is_empty(), "{name}: no pages");
        assert!(
            pdf.refused_pages().is_empty(),
            "{name}: refused {:?}, issues {:?}",
            pdf.refused_pages(),
            pdf.issues()
        );
        let _ = (pdf.info(), pdf.xmp(), pdf.language(), pdf.tagged());
        let _ = (pdf.outline(), pdf.named_destinations(), pdf.links());
        eprintln!(
            "{name}: {} pages, {} bytes unpacked, issues {:?}",
            pages.len(),
            pdf.unpacked(),
            pdf.issues()
        );
    }
}

#[test]
fn empty_user_password_opens() {
    for (name, method) in [
        ("qpdf-aes128-empty-user-password.pdf", "AES-128"),
        ("qpdf-aes256-empty-user-password.pdf", "AES-256"),
    ] {
        let pdf = open(&corpus(name), None).unwrap();
        let enc = pdf.encryption().expect("encrypted");
        assert_eq!(enc.method, method, "{name}");
        assert!(enc.empty_user_password, "{name}");
        assert_eq!(pdf.pages().len(), 30, "{name}");
        assert_eq!(pdf.outline().len(), 11, "{name}");

        // The decrypted copy is a plain PDF the other parsers can read.
        let plain = pdf.decrypted().expect("decrypted bytes");
        let reloaded = Document::load_mem(&plain).unwrap();
        assert!(reloaded.trailer.get(b"Encrypt").is_err(), "{name}");
        let again = open(&plain, None).unwrap();
        assert_eq!(again.encryption(), None, "{name}");
        assert_eq!(again.pages().len(), 30, "{name}");
        assert_eq!(again.outline().len(), 11, "{name}");
    }
}

#[test]
fn plain_file_has_no_decrypted_copy() {
    let pdf = open(&corpus("pypdf-outline.pdf"), None).unwrap();
    assert_eq!(pdf.encryption(), None);
    assert!(pdf.decrypted().is_none());
}

#[test]
fn missing_password_is_encrypted() {
    let bytes = corpus("qpdf-user-password.pdf");
    assert_eq!(open(&bytes, None).unwrap_err(), OpenError::NeedsPassword);
}

#[test]
fn wrong_password_is_reported() {
    let bytes = corpus("qpdf-user-password.pdf");
    let secret = "s3cr3t-pa55";
    let err = open(&bytes, Some(secret)).unwrap_err();
    assert_eq!(err, OpenError::WrongPassword);
    assert!(!err.to_string().contains(secret));
    assert!(!format!("{err:?}").contains(secret));
}

#[test]
fn user_password_opens() {
    let pdf = open(&corpus("qpdf-user-password.pdf"), Some("view")).unwrap();
    let enc = pdf.encryption().unwrap();
    assert_eq!(enc.method, "AES-256");
    assert!(!enc.empty_user_password);
    assert!(!pdf.pages().is_empty());
    let debug = format!("{pdf:?}");
    assert!(!debug.contains("view"), "{debug}");
    assert!(pdf.decrypted().is_some());
}

#[test]
fn not_pdf_is_reported() {
    assert_eq!(
        open(b"hello, this is text", None).unwrap_err(),
        OpenError::NotPdf
    );
    assert_eq!(open(b"", None).unwrap_err(), OpenError::NotPdf);
}

#[test]
fn page_tree_cycle_is_damaged() {
    let bytes = one_page(plain_content(), |doc, pages_id, page_id| {
        let pages = doc.get_dictionary_mut(pages_id).unwrap();
        pages.set("Kids", vec![page_id.into(), pages_id.into()]);
        pages.set("Count", 2);
    });
    let pdf = open(&bytes, None).unwrap();
    assert_eq!(pdf.pages().len(), 1);
    let issues = pdf.issues();
    assert!(
        issues
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.what.contains("cycle")),
        "{issues:?}"
    );
}

#[test]
fn huge_count_is_bounded() {
    let bytes = one_page(plain_content(), |doc, pages_id, _| {
        doc.get_dictionary_mut(pages_id)
            .unwrap()
            .set("Count", i64::from(i32::MAX));
    });
    let start = std::time::Instant::now();
    let pdf = open(&bytes, None).unwrap();
    assert_eq!(pdf.pages().len(), 1);
    assert!(start.elapsed().as_secs() < 5);
}

#[test]
fn deep_page_tree_is_limited() {
    // A chain of Pages nodes deeper than max_depth.
    let limits = Limits {
        max_depth: 8,
        ..Limits::default()
    };
    let bytes = one_page(plain_content(), |doc, pages_id, page_id| {
        let mut child = page_id;
        for _ in 0..20 {
            let id = doc.add_object(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![child.into()],
                "Count" => 1,
            });
            child = id;
        }
        doc.get_dictionary_mut(pages_id)
            .unwrap()
            .set("Kids", vec![child.into()]);
    });
    let pdf = Pdf::open(&bytes, None, &limits, &never).unwrap();
    assert!(pdf.pages().is_empty());
    assert!(
        pdf.issues().iter().any(|i| i.kind == IssueKind::Limit),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn stream_bomb_is_refused() {
    // About 1 KB of deflate that inflates to 1 MB: far over a ratio of 100.
    let packed = zlib(&vec![b' '; 1 << 20]);
    assert!(packed.len() < 2048, "{}", packed.len());
    let content = Stream::new(dictionary! { "Filter" => "FlateDecode" }, packed);
    let bytes = one_page(content.with_compression(false), |_, _, _| {});
    let pdf = open(&bytes, None).unwrap();
    assert_eq!(pdf.refused_pages(), &[1]);
    let issues = pdf.issues();
    assert!(
        issues
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.location == "page 1"),
        "{issues:?}"
    );
}

#[test]
fn plain_stream_is_measured() {
    let packed = zlib(b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET");
    let content = Stream::new(dictionary! { "Filter" => "FlateDecode" }, packed);
    let bytes = one_page(content.with_compression(false), |_, _, _| {});
    let pdf = open(&bytes, None).unwrap();
    assert!(pdf.refused_pages().is_empty());
    assert!(pdf.unpacked() >= 36, "{}", pdf.unpacked());
}

#[test]
fn font_bomb_refuses_the_page() {
    // A font file reachable only through the page resources.
    let packed = zlib(&vec![0u8; 1 << 20]);
    let bytes = one_page(plain_content(), |doc, _, page_id| {
        let file = doc.add_object(
            Stream::new(dictionary! { "Filter" => "FlateDecode" }, packed).with_compression(false),
        );
        let descriptor = doc.add_object(dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => "Bomb",
            "FontFile2" => file,
        });
        let font = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "TrueType",
            "BaseFont" => "Bomb",
            "FontDescriptor" => descriptor,
        });
        doc.get_dictionary_mut(page_id).unwrap().set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
    });
    let pdf = open(&bytes, None).unwrap();
    assert_eq!(pdf.refused_pages(), &[1]);
}

#[test]
fn inherited_font_bomb_refuses_the_page() {
    // The same font bomb, but the resources sit on the page tree node above the page.
    let packed = zlib(&vec![0u8; 1 << 20]);
    let bytes = one_page(plain_content(), |doc, pages_id, _| {
        let file = doc.add_object(
            Stream::new(dictionary! { "Filter" => "FlateDecode" }, packed).with_compression(false),
        );
        let descriptor = doc.add_object(dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => "Bomb",
            "FontFile2" => file,
        });
        let font = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "TrueType",
            "BaseFont" => "Bomb",
            "FontDescriptor" => descriptor,
        });
        doc.get_dictionary_mut(pages_id).unwrap().set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
    });
    let pdf = open(&bytes, None).unwrap();
    assert_eq!(pdf.refused_pages(), &[1]);
}

/// A page whose content stream is about 1 KB of deflate that inflates to 1 MB.
fn bomb_page() -> Vec<u8> {
    let packed = zlib(&vec![b' '; 1 << 20]);
    let content = Stream::new(dictionary! { "Filter" => "FlateDecode" }, packed);
    one_page(content.with_compression(false), |_, _, _| {})
}

#[test]
fn dead_object_stream_does_not_hide_a_bomb() {
    // A dummy object stream with the id of the bomb, after the end of the file. lopdf ignores
    // it, so it must not count as the measurement of the real stream.
    let mut bytes = bomb_page();
    let doc = Document::load_mem(&bytes).unwrap();
    let page = doc.page_iter().next().unwrap();
    let content = doc.get_page_contents(page)[0];
    bytes.extend(
        format!(
            "\n{} {} obj\n<< /Type /ObjStm /N 0 /First 0 /Length 0 >>\nstream\n\nendstream\nendobj\n",
            content.0, content.1
        )
        .as_bytes(),
    );
    let pdf = open(&bytes, None).unwrap();
    assert_eq!(pdf.refused_pages(), &[1]);
}

#[test]
fn encrypt_text_does_not_turn_off_the_scan() {
    let mut bytes = objstm_bomb();
    bytes.extend(b"%/Encrypt\n");
    let err = open(&bytes, None).unwrap_err();
    assert!(matches!(err, OpenError::Limit(_)), "{err:?}");
}

#[test]
fn raw_deflate_after_another_filter_is_measured() {
    // Hex around a deflate stream with a broken zlib header. lopdf retries it as raw deflate
    // and gets 1 MB, so the guard must count the same.
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), Compression::best());
    e.write_all(&vec![b' '; 1 << 20]).unwrap();
    let mut data = vec![0xFF, 0xFF];
    data.extend(e.finish().unwrap());
    let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
    let content = Stream::new(
        dictionary! {
            "Filter" => vec![Object::Name(b"ASCIIHexDecode".to_vec()), Object::Name(b"FlateDecode".to_vec())],
        },
        format!("{hex}>").into_bytes(),
    );
    let bytes = one_page(content.with_compression(false), |_, _, _| {});
    let pdf = open(&bytes, None).unwrap();
    assert_eq!(pdf.refused_pages(), &[1]);
}

#[test]
fn empty_stored_blocks_do_not_hide_a_bomb() {
    // Thousands of empty stored deflate blocks give no output at first, then the bomb.
    let mut data = vec![0x78, 0x9C];
    for _ in 0..20_000 {
        data.extend([0x00, 0x00, 0x00, 0xFF, 0xFF]);
    }
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), Compression::best());
    e.write_all(&vec![b' '; 64 << 20]).unwrap();
    data.extend(e.finish().unwrap());
    let content = Stream::new(dictionary! { "Filter" => "FlateDecode" }, data);
    let bytes = one_page(content.with_compression(false), |_, _, _| {});
    let pdf = open(&bytes, None).unwrap();
    assert_eq!(pdf.refused_pages(), &[1]);
}

#[test]
fn stop_ends_the_open() {
    let bytes = corpus("pypdf-outline.pdf");
    let err = Pdf::open(&bytes, None, &Limits::default(), &|| true).unwrap_err();
    assert!(matches!(err, OpenError::Limit(_)), "{err:?}");
}

/// A document with an object stream of about 2 MB that inflates to 2 GiB.
fn objstm_bomb() -> Vec<u8> {
    // The zeros are fed in chunks so building the fixture never holds them.
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    let chunk = vec![0u8; 1 << 20];
    for _ in 0..2048 {
        e.write_all(&chunk).unwrap();
    }
    let packed = e.finish().unwrap();
    assert!(packed.len() < 4 << 20, "{}", packed.len());
    // lopdf's writer leaves out object streams, so the type is patched in afterwards. The
    // name has the same length, so the offsets stay right.
    let mut bytes = one_page(plain_content(), |doc, _, _| {
        doc.add_object(
            Stream::new(
                dictionary! {
                    "Type" => "ObjStX",
                    "N" => 1,
                    "First" => 4,
                    "Filter" => "FlateDecode",
                },
                packed,
            )
            .with_compression(false),
        );
    });
    let at = bytes.windows(7).position(|w| w == b"/ObjStX").unwrap();
    bytes[at + 6] = b'm';
    bytes
}

#[cfg(unix)]
fn max_rss_bytes() -> u64 {
    // SAFETY: getrusage fills the struct we pass.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let v = ru.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        v
    } else {
        v * 1024
    }
}

#[cfg(unix)]
#[test]
fn objstm_bomb_is_bounded() {
    let bytes = objstm_bomb();
    let err = open(&bytes, None).unwrap_err();
    assert!(matches!(err, OpenError::Limit(_)), "{err:?}");
    let rss = max_rss_bytes();
    eprintln!("max RSS {} MB", rss / 1_000_000);
    assert!(rss < 512 * 1024 * 1024, "max RSS {rss}");
}

#[test]
fn issue_kinds_compare() {
    // The error text names the problem in plain words.
    assert_eq!(
        OpenError::NeedsPassword.to_string(),
        "the file needs a password"
    );
    assert_eq!(
        OpenError::WrongPassword.to_string(),
        "the password is wrong"
    );
}
