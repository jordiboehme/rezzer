// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Ways around the stream guard, each built as a file that tries one.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use pdfparts::{Limits, OpenError, Pdf};

mod common;

/// The allocator of the Rezzer binary: the memory these tests measure is the memory it
/// ships with.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

fn never() -> bool {
    false
}

fn open(bytes: &[u8]) -> Result<Pdf, OpenError> {
    // The memory tests keep the objects to 256 MiB, so the process stays under its bound.
    let limits = Limits {
        max_object_bytes: 256 << 20,
        ..Limits::default()
    };
    Pdf::open(bytes, None, &limits, &never)
}

/// Writes `n` copies of `byte` into `w` in chunks, so the plain data is never held.
fn fill(w: &mut impl Write, byte: u8, n: usize) {
    let chunk = vec![byte; 1 << 20];
    let mut left = n;
    while left > 0 {
        let k = left.min(chunk.len());
        w.write_all(&chunk[..k]).unwrap();
        left -= k;
    }
}

fn zlib_fill(byte: u8, n: usize) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    fill(&mut e, byte, n);
    e.finish().unwrap()
}

fn page(doc: &mut Document, parent: ObjectId, extra: Dictionary) -> ObjectId {
    let mut d = dictionary! {
        "Type" => "Page",
        "Parent" => parent,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    d.extend(&extra);
    doc.add_object(d)
}

/// Saves a document whose page tree root has `kids`. `build` adds the objects.
fn save(build: impl FnOnce(&mut Document, ObjectId) -> Vec<Object>) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let kids = build(&mut doc, pages_id);
    let n = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => n }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// One page whose content stream has `filters` and `data`.
fn content_page(filters: &[&str], data: Vec<u8>) -> Vec<u8> {
    save(|doc, pages| {
        let names: Vec<Object> = filters
            .iter()
            .map(|f| Object::Name(f.as_bytes().to_vec()))
            .collect();
        let content = doc.add_object(
            Stream::new(dictionary! { "Filter" => names }, data).with_compression(false),
        );
        vec![page(doc, pages, dictionary! { "Contents" => content }).into()]
    })
}

/// A zlib stream of about 2 MB that inflates to 2 GiB.
fn objstm_bomb_data() -> Vec<u8> {
    zlib_fill(0, 2 << 30)
}

/// A one-page document with an extra stream whose type is `placeholder` (lopdf's writer leaves
/// out object streams). `keys` come before `/Type` in its dictionary.
fn with_typed_stream(placeholder: &str, keys: Dictionary, data: Vec<u8>) -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let p = page(&mut doc, pages_id, dictionary! {});
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![p.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut d = keys;
    d.set("Type", Object::Name(placeholder.as_bytes().to_vec()));
    d.set("N", 1);
    d.set("First", 4);
    d.set("Filter", "FlateDecode");
    doc.add_object(Stream::new(d, data).with_compression(false));
    doc
}

fn saved(doc: &mut Document) -> Vec<u8> {
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// Adds an update section whose catalog uses member 900 of the typed stream of `doc`, so the
/// object stream is part of the document and must be unpacked. `extra` goes into the trailer.
fn needing_member(bytes: &[u8], doc: &Document, extra: &str) -> Vec<u8> {
    let pages = doc.catalog().unwrap().get(b"Pages").unwrap();
    let pages = pages.as_reference().unwrap();
    let container = doc
        .objects
        .iter()
        .find(|(_, o)| o.as_stream().is_ok_and(|s| s.dict.get(b"First").is_ok()))
        .map(|(id, _)| *id)
        .unwrap();
    common::append_update(
        bytes,
        &[(
            800,
            format!(
                "<< /Type /Catalog /Pages {} {} R /Bomb 900 0 R >>",
                pages.0, pages.1
            ),
        )],
        &[(900, container.0, 0)],
        800,
        extra,
    )
}

/// Replaces `from` with `to` (same length) once.
fn patch(bytes: &mut [u8], from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len());
    let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
    bytes[at..at + to.len()].copy_from_slice(to);
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

fn assert_limit_bounded(bytes: &[u8]) {
    let result = open(bytes);
    #[cfg(unix)]
    {
        let rss = max_rss_bytes();
        eprintln!("max RSS {} MB", rss / 1_000_000);
        assert!(rss < 512 * 1024 * 1024, "max RSS {rss}");
    }
    let err = result.unwrap_err();
    assert!(matches!(err, OpenError::Limit(_)), "{err:?}");
}

#[test]
fn escaped_objstm_name_is_scanned() {
    // `/Obj#53tm` is the name ObjStm written with an escape.
    let mut doc = with_typed_stream("ObjStXYZ", dictionary! {}, objstm_bomb_data());
    let mut bytes = saved(&mut doc);
    patch(&mut bytes, b"/ObjStXYZ", b"/Obj#53tm");
    assert_limit_bounded(&needing_member(&bytes, &doc, ""));
}

#[test]
fn padded_objstm_dict_is_scanned() {
    // 70 KB of dictionary before `/Type`.
    let pad = Object::string_literal(vec![b'x'; 70_000]);
    let mut doc = with_typed_stream("ObjStX", dictionary! { "Pad" => pad }, objstm_bomb_data());
    let mut bytes = saved(&mut doc);
    patch(&mut bytes, b"/ObjStX", b"/ObjStm");
    assert_limit_bounded(&needing_member(&bytes, &doc, ""));
}

#[test]
fn untyped_xref_stream_is_measured() {
    // A cross-reference stream without /Type at startxref. lopdf decodes it whatever its type.
    let mut bytes = content_page(&[], b"BT ET".to_vec());
    let data = objstm_bomb_data();
    let offset = bytes.len() + 1;
    bytes.extend(
        format!(
            "\n99 0 obj\n<< /Size 100 /W [1 2 1] /Filter /FlateDecode /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    bytes.extend(&data);
    bytes.extend(format!("\nendstream\nendobj\nstartxref\n{offset}\n%%EOF\n").as_bytes());
    assert_limit_bounded(&bytes);
}

#[test]
fn encrypted_objstm_bomb_is_bounded() {
    use lopdf::{EncryptionState, EncryptionVersion, Permissions};
    let mut doc = with_typed_stream("ObjStX", dictionary! {}, objstm_bomb_data());
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
    let mut bytes = saved(&mut doc);
    patch(&mut bytes, b"/ObjStX", b"/ObjStm");
    let e = doc.trailer.get(b"Encrypt").unwrap().as_reference().unwrap();
    let hex = "30313233343536373839616263646566";
    let extra = format!("/Encrypt {} {} R /ID [<{hex}><{hex}>]", e.0, e.1);
    assert_limit_bounded(&needing_member(&bytes, &doc, &extra));
}

#[test]
fn flate_then_hex_whitespace_is_capped() {
    // The Flate layer unpacks to 256 MiB of spaces; the hex layer turns them into nothing.
    let bytes = content_page(
        &["FlateDecode", "ASCIIHexDecode"],
        zlib_fill(b' ', 256 << 20),
    );
    let pdf = open(&bytes).unwrap();
    assert_eq!(pdf.refused_pages(), &[1]);
}

#[test]
fn short_inner_flate_is_capped() {
    // The outer layer unpacks to a tiny zlib stream followed by 256 MiB of zeros. The inner
    // layer stops after its own end, so only the outer output is large.
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    let mut inner = ZlibEncoder::new(Vec::new(), Compression::best());
    inner.write_all(b"hello").unwrap();
    e.write_all(&inner.finish().unwrap()).unwrap();
    fill(&mut e, 0, 256 << 20);
    let bytes = content_page(&["FlateDecode", "FlateDecode"], e.finish().unwrap());
    let pdf = open(&bytes).unwrap();
    assert_eq!(pdf.refused_pages(), &[1]);
}

#[test]
fn honest_high_ratio_stream_is_kept() {
    // A blank 8 MiB image layer deflates far past 100:1, but stays under 16 MiB.
    let bytes = content_page(&["FlateDecode"], zlib_fill(0, 8 << 20));
    let pdf = open(&bytes).unwrap();
    assert!(pdf.refused_pages().is_empty(), "{:?}", pdf.issues());
}

#[test]
fn duplicate_kid_numbers_pages_like_lopdf() {
    // Kids [P1 P2 P1 Bomb]: lopdf yields P1 twice, so the bomb is page 4.
    let bomb = zlib_fill(b' ', 32 << 20);
    let bytes = save(|doc, pages| {
        let p1 = page(doc, pages, dictionary! {});
        let p2 = page(doc, pages, dictionary! {});
        let content = doc.add_object(
            Stream::new(dictionary! { "Filter" => "FlateDecode" }, bomb).with_compression(false),
        );
        let b = page(doc, pages, dictionary! { "Contents" => content });
        vec![p1.into(), p2.into(), p1.into(), b.into()]
    });
    let pdf = open(&bytes).unwrap();
    assert_eq!(pdf.pages(&never).len(), 4);
    assert_eq!(pdf.refused_pages(), &[4]);
    assert!(pdf.unsafe_page_tree());
    let doc = Document::load_mem(pdf.canonical()).unwrap();
    assert_eq!(doc.get_pages().len(), 4);
}

#[test]
fn untyped_page_is_skipped_like_lopdf() {
    let bytes = save(|doc, pages| {
        let untyped = doc.add_object(dictionary! {
            "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        });
        let p2 = page(doc, pages, dictionary! {});
        vec![untyped.into(), p2.into()]
    });
    let pdf = open(&bytes).unwrap();
    assert_eq!(pdf.pages(&never).len(), 1);
    assert!(pdf.unsafe_page_tree());
}

#[test]
fn regular_tree_is_safe() {
    let bytes = content_page(&[], b"BT ET".to_vec());
    assert!(!open(&bytes).unwrap().unsafe_page_tree());
}

#[test]
fn shared_fonts_do_not_exhaust_the_refusal_walk() {
    // 400 pages share 20 fonts with long width arrays; only the last page has a bomb.
    let bomb = zlib_fill(b' ', 32 << 20);
    let bytes = save(|doc, pages| {
        let widths: Vec<Object> = (0..300).map(Object::Integer).collect();
        let mut fonts = Dictionary::new();
        for i in 0..20 {
            let f = doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "F",
                "FirstChar" => 0, "Widths" => widths.clone(),
            });
            fonts.set(format!("F{i}"), f);
        }
        let res = doc.add_object(dictionary! { "Font" => fonts });
        let mut kids = Vec::new();
        for _ in 0..399 {
            kids.push(page(doc, pages, dictionary! { "Resources" => res }).into());
        }
        let content = doc.add_object(
            Stream::new(dictionary! { "Filter" => "FlateDecode" }, bomb).with_compression(false),
        );
        kids.push(
            page(
                doc,
                pages,
                dictionary! { "Resources" => res, "Contents" => content },
            )
            .into(),
        );
        kids
    });
    let pdf = open(&bytes).unwrap();
    assert_eq!(pdf.refused_pages(), &[400]);
}

#[test]
fn pages_naming_one_bad_resources_object_are_all_refused() {
    // Three pages name one /Resources object whose image is a bomb; a fourth has none.
    let bytes = save(|doc, pages| {
        let bomb = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "Filter" => "FlateDecode",
            },
            // Zeros unpack more than 100 times their size.
            zlib_fill(0, 64 << 20),
        ));
        let res = doc.add_object(dictionary! { "XObject" => dictionary! { "Im" => bomb } });
        let mut kids: Vec<Object> = (0..3)
            .map(|_| page(doc, pages, dictionary! { "Resources" => res }).into())
            .collect();
        kids.push(page(doc, pages, dictionary! {}).into());
        kids
    });
    let pdf = open(&bytes).unwrap();
    assert_eq!(pdf.refused_pages(), &[1, 2, 3]);
}

#[test]
fn refused_streams_are_empty_in_the_canonical_copy() {
    // The page's content unpacks to 256 MiB of spaces through two layers.
    let bytes = content_page(
        &["FlateDecode", "ASCIIHexDecode"],
        zlib_fill(b' ', 256 << 20),
    );
    let pdf = open(&bytes).unwrap();
    assert_eq!(pdf.refused_pages(), &[1]);
    // The canonical copy keeps the stream's dictionary but not its data, so a parser that
    // reads it finds nothing to unpack.
    let copy = open(pdf.canonical()).unwrap();
    assert!(copy.refused_pages().is_empty(), "{:?}", copy.issues());
    assert_eq!(copy.page_count(), 1);
    assert!(
        pdf.canonical().len() < 4096,
        "canonical copy holds {} bytes",
        pdf.canonical().len()
    );
}

#[test]
fn unpacked_size_is_the_measured_size() {
    let bytes = content_page(&["FlateDecode"], zlib_fill(b' ', 5_000));
    let pdf = open(&bytes).unwrap();
    // The canonical copy keeps object numbers: find the content stream's number there.
    let canonical = String::from_utf8_lossy(pdf.canonical()).into_owned();
    let at = canonical.find("/Contents ").expect("contents key");
    let number: u32 = canonical[at + 10..]
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .expect("object number");
    assert_eq!(pdf.unpacked_size(number, 0), Some(5_000));
    assert_eq!(pdf.unpacked_size(number, 1), None);
    assert_eq!(pdf.unpacked_size(9_999, 0), None);
}

#[test]
fn copies_draw_on_the_object_budget_left() {
    let bytes = content_page(&["FlateDecode"], zlib_fill(b' ', 1_000));
    let limits = Limits {
        max_object_bytes: 1 << 20,
        ..Limits::default()
    };
    let pdf = Pdf::open(&bytes, None, &limits, &never).unwrap();
    assert_eq!(pdf.canonical_shared().as_slice(), pdf.canonical());
    assert!(pdf.charge_copy(1_000));
    // More than the whole budget does not fit, and spends what is left.
    assert!(!pdf.charge_copy(1 << 20));
    assert!(!pdf.charge_copy(1));
}

/// A zlib stream of some text whose deflate data is overwritten in the middle, so the
/// inflater stops with an error after some output.
fn broken_zlib() -> Vec<u8> {
    let text: String = (0..4000).map(|i| format!("BT ({i}) Tj ET\n")).collect();
    let mut packed = common::zlib(text.as_bytes());
    let mid = packed.len() / 2;
    for b in &mut packed[mid..mid + 64] {
        *b = 0xFF;
    }
    packed
}

#[test]
fn a_broken_content_stream_names_its_page() {
    let pdf = open(&content_page(&["FlateDecode"], broken_zlib())).unwrap();
    let issues = pdf.issues();
    let hit = issues
        .iter()
        .find(|i| i.what.contains("is damaged"))
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(hit.location, "page 1", "{hit:?}");
    assert_eq!(hit.kind, pdfparts::IssueKind::Damaged);
    assert!(hit.what.contains("breaks off after"), "{hit:?}");
    // The page is still read: only a limit refuses a page.
    assert!(pdf.refused_pages().is_empty());
}

#[test]
fn a_cut_deflate_stream_is_not_damaged() {
    let mut packed = common::zlib(b"BT (Some words) Tj ET");
    packed.truncate(packed.len() - 6);
    let pdf = open(&content_page(&["FlateDecode"], packed)).unwrap();
    let issues = pdf.issues();
    assert!(
        !issues.iter().any(|i| i.what.contains("is damaged")),
        "{issues:?}"
    );
}

#[test]
fn a_broken_stream_no_page_reaches_is_named_by_its_object() {
    // The catalog holds the stream; no page does.
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let p = page(&mut doc, pages_id, dictionary! {});
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![p.into()], "Count" => 1 },
        ),
    );
    let broken = doc.add_object(
        Stream::new(dictionary! { "Filter" => "FlateDecode" }, broken_zlib())
            .with_compression(false),
    );
    let catalog =
        doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id, "Extra" => broken });
    doc.trailer.set("Root", catalog);
    let pdf = open(&saved(&mut doc)).unwrap();
    let issues = pdf.issues();
    let hit = issues
        .iter()
        .find(|i| i.what.contains("is damaged"))
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(
        hit.location,
        format!("object {} {}", broken.0, broken.1),
        "{hit:?}"
    );
}

#[test]
fn a_wrong_checksum_with_all_data_is_not_damage() {
    let mut packed = common::zlib(b"BT /F1 12 Tf 72 700 Td (Some words) Tj ET");
    let n = packed.len();
    packed[n - 1] ^= 0xFF;
    let pdf = open(&content_page(&["FlateDecode"], packed)).unwrap();
    let issues = pdf.issues();
    assert!(
        !issues.iter().any(|i| i.what.contains("is damaged")),
        "{issues:?}"
    );
    // It is said, as a fact that may be wrong, not as data cut off.
    let sum = issues
        .iter()
        .find(|i| i.what.contains("wrong checksum"))
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(sum.kind, pdfparts::IssueKind::Uncertain);
    assert_eq!(sum.location, "page 1");
}

#[test]
fn a_right_checksum_says_nothing() {
    let packed = common::zlib(b"BT /F1 12 Tf 72 700 Td (Some words) Tj ET");
    let pdf = open(&content_page(&["FlateDecode"], packed)).unwrap();
    assert!(pdf.issues().is_empty(), "{:?}", pdf.issues());
}

/// A zlib stream of `text` in fixed codes, then a copy from 32,768 bytes back, which inflate
/// refuses, then 1,000 copies of 258 bytes that a lenient decoder unpacks.
fn text_then_far_copy(text: &[u8]) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::new();
    let put = |v: u32, len: u32, bits: &mut Vec<bool>| {
        for i in 0..len {
            bits.push((v >> i) & 1 == 1);
        }
    };
    let code = |c: u32, len: u32, bits: &mut Vec<bool>| {
        for i in (0..len).rev() {
            bits.push((c >> i) & 1 == 1);
        }
    };
    put(1, 1, &mut bits);
    put(1, 2, &mut bits);
    for &c in text {
        let c = u32::from(c);
        if c < 144 {
            code(0x30 + c, 8, &mut bits);
        } else {
            code(0x190 + c - 144, 9, &mut bits);
        }
    }
    // Length 3 (code 257), distance code 29 with 13 extra bits of 8191: 32,768 back.
    code(1, 7, &mut bits);
    code(29, 5, &mut bits);
    put(8191, 13, &mut bits);
    for _ in 0..1000 {
        // Length 258 (code 285), distance 1 (code 0).
        code(0xC5, 8, &mut bits);
        code(0, 5, &mut bits);
    }
    code(0, 7, &mut bits);
    let mut out = vec![0x78, 0x9c];
    for chunk in bits.chunks(8) {
        out.push(
            chunk
                .iter()
                .enumerate()
                .fold(0u8, |b, (i, &on)| b | (u8::from(on) << i)),
        );
    }
    out
}

#[test]
fn the_canonical_copy_of_a_damaged_stream_holds_what_was_measured() {
    let text = b"BT /F1 12 Tf 72 700 Td (Kept.) Tj ET\n";
    let pdf = open(&content_page(&["FlateDecode"], text_then_far_copy(text))).unwrap();
    let issues = pdf.issues();
    assert!(
        issues.iter().any(|i| i.what.contains("is damaged")),
        "{issues:?}"
    );
    // Another parser reads the measured text, packed again, and nothing more.
    let doc = Document::load_mem(pdf.canonical()).unwrap();
    let content = doc
        .objects
        .values()
        .find_map(|o| match o {
            Object::Stream(s) if s.dict.get(b"Length").is_ok() && s.dict.get(b"Filter").is_ok() => {
                Some(s)
            }
            _ => None,
        })
        .expect("the content stream holds its text");
    // Packed again, it unpacks to exactly the measured text.
    assert_eq!(
        content
            .dict
            .get(b"Filter")
            .ok()
            .and_then(|f| f.as_name().ok()),
        Some(&b"FlateDecode"[..]),
        "{:?}",
        content.dict
    );
    let mut unpacked = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::ZlibDecoder::new(&content.content[..]),
        &mut unpacked,
    )
    .unwrap();
    assert_eq!(unpacked, text);
}

#[test]
fn a_damaged_stream_with_a_predictor_keeps_its_whole_rows() {
    // Two whole PNG rows (filter byte 0, then four bytes), part of a third, then damage.
    let rows = b"\x00ABCD\x00EFGH\x00IJ";
    let packed = text_then_far_copy(rows);
    let bytes = save(|doc, pages| {
        let content = doc.add_object(
            Stream::new(
                dictionary! {
                    "Filter" => "FlateDecode",
                    "DecodeParms" => dictionary! { "Predictor" => 12, "Columns" => 4 },
                },
                packed,
            )
            .with_compression(false),
        );
        vec![page(doc, pages, dictionary! { "Contents" => content }).into()]
    });
    let pdf = open(&bytes).unwrap();
    let doc = Document::load_mem(pdf.canonical()).unwrap();
    let content = doc
        .objects
        .values()
        .find_map(|o| match o {
            Object::Stream(s) if s.dict.get(b"Filter").is_ok() => Some(s),
            _ => None,
        })
        .expect("the content stream keeps its rows");
    assert!(
        content.dict.get(b"DecodeParms").is_err(),
        "{:?}",
        content.dict
    );
    let mut unpacked = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::ZlibDecoder::new(&content.content[..]),
        &mut unpacked,
    )
    .unwrap();
    assert_eq!(unpacked, b"ABCDEFGH");
}
