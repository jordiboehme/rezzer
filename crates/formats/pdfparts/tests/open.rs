// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::Write;
use std::path::PathBuf;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use pdfparts::{IssueKind, Limits, OpenError, Pdf};

mod common;

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

/// Deflate of `n` copies of `byte`, fed in chunks so the plain data is never held.
fn zlib_fill(byte: u8, n: usize) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    let chunk = vec![byte; 1 << 20];
    let mut left = n;
    while left > 0 {
        let k = left.min(chunk.len());
        e.write_all(&chunk[..k]).unwrap();
        left -= k;
    }
    e.finish().unwrap()
}

/// Past the 16 MiB size from which the ratio limit applies.
const BOMB: usize = 32 << 20;

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
        let pages = pdf.pages(&never);
        assert!(!pages.is_empty(), "{name}: no pages");
        assert!(
            pdf.refused_pages().is_empty(),
            "{name}: refused {:?}, issues {:?}",
            pdf.refused_pages(),
            pdf.issues()
        );
        assert!(!pdf.unsafe_page_tree(), "{name}: {:?}", pdf.issues());
        let _ = (pdf.info(), pdf.xmp(), pdf.language(), pdf.tagged());
        let _ = (pdf.outline(), pdf.named_destinations(), pdf.links());
        eprintln!(
            "{name}: {} pages, {} bytes unpacked, issues {:?}, {pdf:?}",
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
        assert_eq!(pdf.pages(&never).len(), 30, "{name}");
        assert_eq!(pdf.outline().len(), 11, "{name}");

        // The canonical copy is a plain PDF the other parsers can read.
        let plain = pdf.canonical();
        let reloaded = Document::load_mem(plain).unwrap();
        assert!(reloaded.trailer.get(b"Encrypt").is_err(), "{name}");
        let again = open(plain, None).unwrap();
        assert_eq!(again.encryption(), None, "{name}");
        assert_eq!(again.pages(&never).len(), 30, "{name}");
        assert_eq!(again.outline().len(), 11, "{name}");
    }
}

#[test]
fn plain_file_has_a_canonical_copy() {
    let pdf = open(&corpus("pypdf-outline.pdf"), None).unwrap();
    assert_eq!(pdf.encryption(), None);
    assert!(Document::load_mem(pdf.canonical()).is_ok());
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
    assert!(!pdf.pages(&never).is_empty());
    let debug = format!("{pdf:?}");
    assert!(!debug.contains("view"), "{debug}");
    assert!(Document::load_mem(pdf.canonical()).is_ok());
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
    // Page numbers follow lopdf, which yields the page again while it walks the cycle, up to
    // its own bound. The tree is marked unsafe for other parsers.
    assert!(!pdf.pages(&never).is_empty());
    assert!(pdf.unsafe_page_tree());
    // lopdf numbers the pages of the canonical copy the same way.
    let doc = Document::load_mem(pdf.canonical()).unwrap();
    assert_eq!(doc.get_pages().len() as u32, pdf.page_count());
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
    assert_eq!(pdf.pages(&never).len(), 1);
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
    // lopdf's own depth bound is 256, so it still finds the page; the bounded check does not
    // and marks the tree unsafe.
    assert_eq!(pdf.pages(&never).len(), 1);
    assert!(pdf.unsafe_page_tree());
    assert!(
        pdf.issues().iter().any(|i| i.kind == IssueKind::Limit),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn stream_bomb_is_refused() {
    // About 32 KB of deflate that inflates to 32 MiB: far over a ratio of 100.
    let packed = zlib_fill(b' ', BOMB);
    assert!(packed.len() < 64 << 10, "{}", packed.len());
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
    let packed = zlib_fill(0, BOMB);
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
    let packed = zlib_fill(0, BOMB);
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

/// A page whose content stream is about 32 KB of deflate that inflates to 32 MiB.
fn bomb_page() -> Vec<u8> {
    let packed = zlib_fill(b' ', BOMB);
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
    // and gets 32 MiB, so the guard must count the same.
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), Compression::best());
    e.write_all(&vec![b' '; BOMB]).unwrap();
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
    let mut ids = None;
    let mut bytes = one_page(plain_content(), |doc, pages, _| {
        let container = doc.add_object(
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
        ids = Some((pages, container));
    });
    let at = bytes.windows(7).position(|w| w == b"/ObjStX").unwrap();
    bytes[at + 6] = b'm';
    // An update section makes a member of the object stream part of the document, so the
    // stream must be unpacked.
    let (pages, container) = ids.unwrap();
    common::append_update(
        &bytes,
        &[(
            800,
            format!(
                "<< /Type /Catalog /Pages {} {} R /Bomb 900 0 R >>",
                pages.0, pages.1
            ),
        )],
        &[(900, container.0, 0)],
        800,
        "",
    )
}

/// Maximum resident set size of the process. It covers lopdf's rayon threads too. It is the
/// figure of one test only under nextest, which runs every test in its own process.
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
    let limits = Limits {
        max_object_bytes: 256 << 20,
        ..Limits::default()
    };
    let err = Pdf::open(&bytes, None, &limits, &never).unwrap_err();
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

fn corpus_names() -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(corpus_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".pdf"))
        .collect();
    names.sort();
    names
}

#[test]
fn canonical_bytes_have_no_object_streams_and_reload() {
    for name in corpus_names() {
        let bytes = corpus(&name);
        let password = (name == "qpdf-user-password.pdf").then_some("view");
        let pdf = open(&bytes, password).unwrap();
        let canon = pdf.canonical();
        // A classic table, and lopdf reads the copy with nothing to unpack.
        assert!(canon.windows(6).any(|w| w == b"\nxref\n"), "{name}");
        let doc = Document::load_mem(canon).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(doc.trailer.get(b"Encrypt").is_err(), "{name}");
        assert!(
            doc.objects
                .values()
                .all(|o| !matches!(o.type_name(), Ok(b"ObjStm" | b"XRef"))),
            "{name}"
        );
        assert_eq!(doc.get_pages().len() as u32, pdf.page_count(), "{name}");
        let again = open(canon, None).unwrap();
        assert_eq!(again.page_count(), pdf.page_count(), "{name}");
        assert_eq!(again.info(), pdf.info(), "{name}");
        assert_eq!(again.outline(), pdf.outline(), "{name}");
        assert_eq!(again.canonical(), canon, "{name}: the copy is not stable");
    }
}

#[test]
fn bad_xref_is_rebuilt() {
    let pdf = open(&corpus("qpdf-malformed-bad-xref-entry.pdf"), None).unwrap();
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.what.contains("rebuilt")),
        "{:?}",
        pdf.issues()
    );
    let doc = Document::load_mem(pdf.canonical()).unwrap();
    let page = doc.page_iter().next().unwrap();
    let content = doc.get_page_content(page);
    assert!(
        content.windows(6).any(|w| w == b"Potato"),
        "{}",
        String::from_utf8_lossy(&content)
    );
}

/// Every dictionary object with `key`, by id.
fn with_key(doc: &Document, key: &[u8]) -> Vec<(ObjectId, lopdf::Dictionary)> {
    doc.objects
        .iter()
        .filter_map(|(id, o)| {
            let d = match o {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &s.dict,
                _ => return None,
            };
            d.get(key).is_ok().then(|| (*id, d.clone()))
        })
        .collect()
}

#[test]
fn signatures_and_form_values_survive() {
    // The corpus files are trusted test data, so lopdf may read them here for comparison.
    for name in ["pdfjs-signed.pdf", "irs-f1040-2025.pdf"] {
        let bytes = corpus(name);
        let pdf = open(&bytes, None).unwrap();
        assert_eq!(pdf.original(), bytes.as_slice());
        let original = Document::load_mem(&bytes).unwrap();
        let canon = Document::load_mem(pdf.canonical()).unwrap();
        let sigs = with_key(&original, b"ByteRange");
        assert!(!sigs.is_empty(), "{name}: no signature dictionary");
        for (id, sig) in sigs {
            let copy = canon
                .get_dictionary(id)
                .unwrap_or_else(|_| panic!("{name}: signature {id:?} is gone"));
            assert_eq!(
                copy.get(b"ByteRange").ok(),
                sig.get(b"ByteRange").ok(),
                "{name}"
            );
            assert_eq!(
                copy.get(b"Contents").ok(),
                sig.get(b"Contents").ok(),
                "{name}"
            );
        }
    }
    let bytes = corpus("pypdf-libreoffice-form.pdf");
    let pdf = open(&bytes, None).unwrap();
    let original = Document::load_mem(&bytes).unwrap();
    let canon = Document::load_mem(pdf.canonical()).unwrap();
    let fields = with_key(&original, b"V");
    assert!(!fields.is_empty());
    for (id, field) in fields {
        let copy = canon.get_dictionary(id).unwrap();
        assert_eq!(copy.get(b"V").ok(), field.get(b"V").ok(), "field {id:?}");
    }
    assert!(canon.catalog().unwrap().get(b"AcroForm").is_ok());
}

#[test]
fn encrypted_signature_contents_stay_unchanged() {
    use lopdf::{EncryptionState, EncryptionVersion, Permissions, StringFormat};
    // A signature's /Contents is not encrypted (ISO 32000-1, 7.6.1); its /Name is.
    let contents = Object::String(vec![0x30, 0x82, 0x01, 0x02], StringFormat::Hexadecimal);
    let mut sig_id = None;
    let mut field_id = None;
    let bytes = one_page(plain_content(), |doc, _, page| {
        let sig = doc.add_object(dictionary! {
            "Type" => "Sig",
            "Filter" => "Adobe.PPKLite",
            "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
            "Contents" => contents.clone(),
            "Name" => Object::string_literal("Signer"),
        });
        // A field whose value is a direct signature dictionary without /Type.
        let field = doc.add_object(dictionary! {
            "FT" => "Sig",
            "T" => Object::string_literal("Sig1"),
            "V" => dictionary! {
                "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
                "Contents" => contents.clone(),
            },
        });
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Annots", vec![sig.into(), field.into()]);
        sig_id = Some(sig);
        field_id = Some(field);
        let id = Object::string_literal(b"0123456789abcdef".to_vec());
        doc.trailer.set("ID", vec![id.clone(), id]);
        let state = EncryptionState::try_from(EncryptionVersion::V2 {
            document: doc,
            owner_password: "owner",
            user_password: "",
            key_length: 128,
            permissions: Permissions::all(),
        })
        .unwrap();
        doc.encrypt(&state).unwrap();
        // lopdf encrypts every string; a signing writer leaves /Contents in the clear.
        doc.get_dictionary_mut(sig)
            .unwrap()
            .set("Contents", contents.clone());
        let field = doc.get_dictionary_mut(field).unwrap();
        let mut v = field.get(b"V").unwrap().as_dict().unwrap().clone();
        v.set("Contents", contents.clone());
        field.set("V", v);
    });
    let pdf = open(&bytes, None).unwrap();
    assert!(pdf.encryption().is_some());
    let canon = Document::load_mem(pdf.canonical()).unwrap();
    let sig = canon.get_dictionary(sig_id.unwrap()).unwrap();
    assert_eq!(sig.get(b"Contents").unwrap(), &contents);
    assert_eq!(sig.get(b"Name").unwrap().as_str().unwrap(), b"Signer");
    let field = canon.get_dictionary(field_id.unwrap()).unwrap();
    assert_eq!(field.get(b"T").unwrap().as_str().unwrap(), b"Sig1");
    let v = field.get(b"V").unwrap().as_dict().unwrap();
    assert_eq!(v.get(b"Contents").unwrap(), &contents);
}

/// A cross-reference stream section with one subsection per entry, as object 9 every time.
fn xref_stream_section(out: &mut Vec<u8>, entries: &[(u32, usize)], prev: Option<usize>) -> usize {
    let index: String = entries.iter().map(|(n, _)| format!("{n} 1 ")).collect();
    let mut data = Vec::new();
    for (_, off) in entries {
        data.push(1u8);
        data.extend((*off as u32).to_be_bytes());
    }
    let prev = prev.map(|p| format!(" /Prev {p}")).unwrap_or_default();
    let at = out.len();
    out.extend(
        format!(
            "9 0 obj\n<< /Type /XRef /W [1 4 0] /Index [{index}] /Size 10 /Root 1 0 R{prev} /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    out.extend(&data);
    out.extend(b"\nendstream\nendobj\n");
    at
}

#[test]
fn update_may_move_the_xref_stream_object() {
    // The update writes its cross-reference stream as object 9 again, at a new place.
    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offs = Vec::new();
    for (n, body) in [
        (1, "<< /Type /Catalog /Pages 2 0 R >>"),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>"),
    ] {
        offs.push((n, out.len()));
        out.extend(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let x1 = xref_stream_section(&mut out, &offs, None);
    out.extend(format!("startxref\n{x1}\n%%EOF\n").as_bytes());
    let four = out.len();
    out.extend(b"4 0 obj\n<< /Title (updated) >>\nendobj\n");
    let at = out.len();
    let x2 = xref_stream_section(&mut out, &[(4, four), (9, at)], Some(x1));
    out.extend(format!("startxref\n{x2}\n%%EOF\n").as_bytes());
    let pdf = open(&out, None).unwrap();
    assert_eq!(pdf.page_count(), 1);
    assert!(
        !pdf.issues().iter().any(|i| i.what.contains("rebuilt")),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn table_without_the_catalog_is_rebuilt() {
    // The table is well formed but has no entry for the catalog it names.
    let mut out = b"%PDF-1.4\n".to_vec();
    out.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let two = out.len();
    out.extend(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    let three = out.len();
    out.extend(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>\nendobj\n");
    let x = out.len();
    out.extend(
        format!("xref\n0 1\n0000000000 65535 f \n2 2\n{two:010} 00000 n \n{three:010} 00000 n \n")
            .as_bytes(),
    );
    out.extend(format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n").as_bytes());
    let pdf = open(&out, None).unwrap();
    assert_eq!(pdf.page_count(), 1);
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.what.contains("rebuilt")),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn catalog_with_another_generation_is_rebuilt() {
    // The table lists object 1 with generation 1, the trailer names 1 0 R.
    let mut out = b"%PDF-1.4\n".to_vec();
    let one = out.len();
    out.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let two = out.len();
    out.extend(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    let three = out.len();
    out.extend(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>\nendobj\n");
    let x = out.len();
    out.extend(
        format!(
            "xref\n0 4\n0000000000 65535 f \n{one:010} 00001 n \n{two:010} 00000 n \n{three:010} 00000 n \n"
        )
        .as_bytes(),
    );
    out.extend(format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n").as_bytes());
    let pdf = open(&out, None).unwrap();
    assert_eq!(pdf.page_count(), 1);
    assert!(
        pdf.issues().iter().any(|i| i.what.contains("rebuilt")),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn catalog_missing_from_its_object_stream_is_rebuilt() {
    // The table puts the catalog in an object stream that does not hold it; the catalog is a
    // plain object in the file.
    let mut raw = common::Raw::new();
    raw.junk(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    raw.object(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    raw.object(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>");
    raw.object_stream(10, &[(5, b"null")]);
    raw.member(1, 10, 1);
    let out = raw.finish(1, "");
    let pdf = open(&out, None).unwrap();
    assert_eq!(pdf.page_count(), 1);
    assert!(
        pdf.issues().iter().any(|i| i.what.contains("rebuilt")),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn values_may_hold_the_obj_keyword() {
    // A string with `endobj` and a header in it, and a name that ends in `obj`.
    let objects: [&[u8]; 5] = [
        b"<< /Type /Catalog /Pages 2 0 R /Note 4 0 R /Tag 5 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Info [(a 9 0 obj) (b endobj)] >>",
        b"(see endobj and 7 0 obj here)",
        b"/Xobj",
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(*body);
        out.extend(b"\nendobj\n");
    }
    let at = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offs.len() + 1).as_bytes());
    for o in &offs {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{at}\n%%EOF\n",
            offs.len() + 1
        )
        .as_bytes(),
    );
    let pdf = open(&out, None).unwrap();
    assert_eq!(pdf.page_count(), 1);
    assert!(pdf.issues().is_empty(), "{:?}", pdf.issues());
    let doc = Document::load_mem(pdf.canonical()).unwrap();
    let get = |n: u32| doc.get_object((n, 0)).unwrap().clone();
    assert_eq!(
        get(4).as_str().unwrap(),
        b"see endobj and 7 0 obj here".as_slice()
    );
    assert_eq!(get(5).as_name().unwrap(), b"Xobj".as_slice());
    let page = get(3);
    let info = page
        .as_dict()
        .unwrap()
        .get(b"Info")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(info[0].as_str().unwrap(), b"a 9 0 obj".as_slice());
}

/// An object stream with no separator between its members, as most writers make them.
/// Object 4 is an array of `n` numbers and is followed by object 5. Returns the file and the
/// length of object 4.
fn abutting_members(n: usize) -> (Vec<u8>, usize) {
    let numbers: Vec<String> = (0..n).map(|i| (i % 1000).to_string()).collect();
    let big = format!("[{}]", numbers.join(" "));
    let members: [(u32, &[u8]); 5] = [
        (1, b"<</Type/Catalog/Pages 2 0 R/Big 4 0 R/Other 5 0 R>>"),
        (2, b"<</Type/Pages/Kids[3 0 R]/Count 1>>"),
        (3, b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]>>"),
        (4, big.as_bytes()),
        (5, b"<</Note(after big)>>"),
    ];
    let mut head = String::new();
    let mut body: Vec<u8> = Vec::new();
    for (m, data) in members {
        head.push_str(&format!("{m} {} ", body.len()));
        body.extend(data);
    }
    let first = head.len();
    let mut plain = head.into_bytes();
    plain.extend(body);
    let mut raw = common::Raw::new();
    let dict = format!("/Type /ObjStm /N 5 /First {first} /Filter /FlateDecode");
    raw.stream(6, &dict, &common::zlib(&plain));
    for (i, (m, _)) in members.iter().enumerate() {
        raw.member(*m, 6, i as u32);
    }
    (raw.finish(1, ""), big.len())
}

#[test]
fn members_without_separators_are_read() {
    for (n, size) in [(10_000, 38_901), (20_000, 77_801), (40_000, 155_601)] {
        let (bytes, len) = abutting_members(n);
        assert_eq!(len, size);
        let pdf = open(&bytes, None).unwrap();
        assert_eq!(pdf.page_count(), 1);
        assert!(pdf.issues().is_empty(), "{n}: {:?}", pdf.issues());
        let doc = Document::load_mem(pdf.canonical()).unwrap();
        let big = doc.get_object((4, 0)).unwrap().as_array().unwrap();
        assert_eq!(big.len(), n);
        let note = doc.get_object((5, 0)).unwrap().as_dict().unwrap();
        assert_eq!(
            note.get(b"Note").unwrap().as_str().unwrap(),
            b"after big".as_slice()
        );
    }
}

#[test]
fn large_value_with_the_obj_keyword_is_read() {
    // A name tree leaf of more than 64 KiB whose keys hold `obj`, as LaTeX writes them.
    let n = 3000;
    let pairs: Vec<String> = (0..n)
        .map(|i| format!("(cite.objdet{i:05}) [3 0 R /XYZ 0 0 0]"))
        .collect();
    let leaf = format!("<< /Names [{}] >>", pairs.join(" "));
    assert!(leaf.len() > 100_000);
    let objects: [&[u8]; 4] = [
        b"<< /Type /Catalog /Pages 2 0 R /Names << /Dests 4 0 R >> >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        leaf.as_bytes(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(*body);
        out.extend(b"\nendobj\n");
    }
    let at = out.len();
    out.extend(b"xref\n0 5\n0000000000 65535 f \n");
    for o in &offs {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{at}\n%%EOF\n").as_bytes());
    let pdf = open(&out, None).unwrap();
    assert_eq!(pdf.page_count(), 1);
    assert!(pdf.issues().is_empty(), "{:?}", pdf.issues());
    let doc = Document::load_mem(pdf.canonical()).unwrap();
    let names = doc.get_object((4, 0)).unwrap().as_dict().unwrap();
    let names = names.get(b"Names").unwrap().as_array().unwrap();
    assert_eq!(names.len(), 2 * n);
    assert_eq!(names[0].as_str().unwrap(), b"cite.objdet00000".as_slice());
}

/// The number of values the open parsed, from the debug output.
fn values_of(pdf: &Pdf) -> usize {
    format!("{pdf:?}")
        .split("values: ")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn values_charged_once() {
    // Each value is counted once, also when its first read is cut and read again.
    let pdf = open(&corpus("irs-f1040-2025.pdf"), None).unwrap();
    assert_eq!(values_of(&pdf), 42_558);
    let nist = corpus_dir().join("nist-sp800-53r5.pdf");
    if nist.exists() {
        let pdf = open(&std::fs::read(nist).unwrap(), None).unwrap();
        assert_eq!(values_of(&pdf), 1_043_799);
    }
}

#[test]
fn abutting_number_members_keep_their_values() {
    // The member `42` is followed by the member `43` with no separator: `4243`.
    let members: [(u32, &[u8]); 5] = [
        (1, b"<</Type/Catalog/Pages 2 0 R/A 7 0 R/B 8 0 R>>"),
        (2, b"<</Type/Pages/Kids[3 0 R]/Count 1>>"),
        (3, b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]>>"),
        (7, b"42"),
        (8, b"43"),
    ];
    let mut head = String::new();
    let mut body: Vec<u8> = Vec::new();
    for (m, data) in members {
        head.push_str(&format!("{m} {} ", body.len()));
        body.extend(data);
    }
    let first = head.len();
    let mut plain = head.into_bytes();
    plain.extend(body);
    let mut raw = common::Raw::new();
    let dict = format!("/Type /ObjStm /N 5 /First {first} /Filter /FlateDecode");
    raw.stream(6, &dict, &common::zlib(&plain));
    for (i, (m, _)) in members.iter().enumerate() {
        raw.member(*m, 6, i as u32);
    }
    let pdf = open(&raw.finish(1, ""), None).unwrap();
    assert!(pdf.issues().is_empty(), "{:?}", pdf.issues());
    let doc = Document::load_mem(pdf.canonical()).unwrap();
    assert_eq!(doc.get_object((7, 0)).unwrap().as_i64().unwrap(), 42);
    assert_eq!(doc.get_object((8, 0)).unwrap().as_i64().unwrap(), 43);
}

/// Five objects with a classic table. Object 4 holds `note` in a string. `damaged` points
/// `startxref` past the table; `stale` points the entry of object 5 at object 2. Both make
/// the file be scanned.
fn note_file(note: &str, damaged: bool, stale: bool) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /A 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Extra 4 0 R >>".to_string(),
        format!("<< /Note ({note}) /K 5 >>"),
        "<< /Five 5 >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    if stale {
        offs[4] = offs[1];
    }
    let at = out.len();
    out.extend(b"xref\n0 6\n0000000000 65535 f \n");
    for o in &offs {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    let sx = if damaged { at + 7 } else { at };
    out.extend(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{sx}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn scan_starts_inside_a_string_do_not_cut_it() {
    // A scan of the file takes the header and the keyword in these strings as starts.
    for note in [
        "the objective of the trailer",
        "write 90 0 obj then 91 0 obj then endobj",
    ] {
        for (damaged, stale) in [(true, false), (false, true)] {
            let pdf = open(&note_file(note, damaged, stale), None).unwrap();
            let issues = pdf.issues();
            assert!(
                issues
                    .iter()
                    .all(|i| !i.what.contains("not closed") && !i.what.contains("missing")),
                "{note} {damaged} {stale}: {issues:?}"
            );
            let doc = Document::load_mem(pdf.canonical()).unwrap();
            let four = doc.get_object((4, 0)).unwrap().as_dict().unwrap();
            assert_eq!(
                four.get(b"Note").unwrap().as_str().unwrap(),
                note.as_bytes()
            );
            assert_eq!(four.get(b"K").unwrap().as_i64().unwrap(), 5);
            assert!(doc.get_object((5, 0)).is_ok());
        }
    }
}
