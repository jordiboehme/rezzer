// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use lopdf::{Document, Object, Stream, dictionary};
use pdfparts::objects::{pdf_date, text_string, xmp_date};
use pdfparts::{IssueKind, Limits, Pdf};

fn never() -> bool {
    false
}

fn corpus(name: &str) -> Pdf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf")
        .join(name);
    let bytes = std::fs::read(path).unwrap();
    Pdf::open(&bytes, None, &Limits::default(), &never).unwrap()
}

/// A one-page document whose catalog carries `metadata` as its XMP stream.
fn with_xmp(metadata: &str) -> Pdf {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let xmp = doc.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        metadata.as_bytes().to_vec(),
    ));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "Metadata" => xmp,
    });
    doc.trailer.set("Root", catalog_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    Pdf::open(&out, None, &Limits::default(), &never).unwrap()
}

#[test]
fn pdfdoc_title_decodes() {
    assert_eq!(text_string(b"Is\xEDs"), "Is\u{ed}s");
    assert_eq!(
        text_string(b"\x84 \x85 \x80 \xA0"),
        "\u{2014} \u{2013} \u{2022} \u{20ac}"
    );
    assert_eq!(text_string(b"a\x18"), "a\u{2d8}");
    assert_eq!(text_string(&[0xFE, 0xFF, 0, 0x41, 0x03, 0xA3]), "A\u{3a3}");
    assert_eq!(
        text_string(b"\xEF\xBB\xBFgr\xC3\xBC\xC3\x9F"),
        "gr\u{fc}\u{df}"
    );
    // A language escape inside UTF-16 is dropped.
    assert_eq!(
        text_string(&[
            0xFE, 0xFF, 0, 0x1B, 0, b'e', 0, b'n', 0, 0x1B, 0, b'h', 0, b'i'
        ]),
        "hi"
    );

    let pdf = corpus("qpdf-page-labels-and-outlines.pdf");
    let titles: Vec<String> = pdf
        .outline()
        .into_iter()
        .map(|o| o.title.to_string())
        .collect();
    assert!(
        titles.iter().any(|t| t.starts_with("Is\u{ed}s 1 ")),
        "{titles:?}"
    );
    assert!(
        titles
            .iter()
            .any(|t| t.starts_with("Sandy \u{f7}\u{3a3}\u{3b1}\u{3bd}\u{3b4}\u{3b9}\u{f7} 1.2")),
        "{titles:?}"
    );
}

/// UTF-16BE bytes with a byte order mark for `s`.
fn utf16(s: &str) -> Vec<u8> {
    let mut out = vec![0xFE, 0xFF];
    for u in s.encode_utf16() {
        out.extend(u.to_be_bytes());
    }
    out
}

#[test]
fn a_language_escape_ends_after_two_code_units() {
    use pdfparts::objects::text_string_checked;
    // ISO 32000 7.9.2.2: a language code of one unit, or a language and a country code.
    assert_eq!(
        text_string_checked(&utf16("\u{1b}\u{656e}\u{1b}A")),
        ("A".to_string(), false)
    );
    assert_eq!(
        text_string_checked(&utf16("\u{1b}\u{656e}\u{5553}\u{1b}A")),
        ("A".to_string(), false)
    );
    // An escape that does not close keeps the text after it.
    assert_eq!(
        text_string_checked(&utf16("\u{1b}x")),
        ("x".to_string(), true)
    );
    assert_eq!(
        text_string_checked(&utf16("\u{0}\u{1b}[31mred\u{85}nel")),
        ("[31mred\u{85}nel".to_string(), true)
    );
    // A third unit before the closing escape is too many: the escape does not close there.
    assert_eq!(
        text_string_checked(&utf16("\u{1b}abc\u{1b}def")),
        ("abcdef".to_string(), true)
    );
    assert_eq!(text_string(&utf16("\u{1b}x")), "x");
}

/// A one-page document whose Info dictionary has `subject` as its raw subject string.
fn with_subject(subject: Vec<u8>) -> Pdf {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    let info = doc.add_object(dictionary! {
        "Subject" => Object::String(subject, lopdf::StringFormat::Hexadecimal),
    });
    doc.trailer.set("Root", catalog_id);
    doc.trailer.set("Info", info);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    Pdf::open(&out, None, &Limits::default(), &never).unwrap()
}

#[test]
fn an_unclosed_language_escape_keeps_the_text_and_says_so() {
    for (subject, kept) in [
        ("\u{1b}x", "x"),
        ("\u{0}\u{1b}[31mred\u{85}nel", "[31mred\u{85}nel"),
    ] {
        let pdf = with_subject(utf16(subject));
        assert_eq!(pdf.info().subject.as_deref(), Some(kept));
        let issues = pdf.issues();
        assert!(
            issues.iter().any(|i| i.kind == IssueKind::Damaged
                && i.location == "metadata"
                && i.what.contains("language escape")),
            "{issues:?}"
        );
    }
    let fine = with_subject(utf16("\u{1b}\u{656e}\u{1b}plain"));
    assert_eq!(fine.info().subject.as_deref(), Some("plain"));
    assert!(fine.issues().is_empty(), "{:?}", fine.issues());
}

#[test]
fn pdf_dates_keep_offset() {
    let cases = [
        ("D:20151230152248+01'00'", Some("2015-12-30T15:22:48+01:00")),
        ("D:20151230152248-05'30", Some("2015-12-30T15:22:48-05:30")),
        ("D:20151230152248-0500", Some("2015-12-30T15:22:48-05:00")),
        ("D:20151230152248Z", Some("2015-12-30T15:22:48Z")),
        ("D:20151230152248Z00'00'", Some("2015-12-30T15:22:48Z")),
        ("D:20230410074654Z07'46'", Some("2023-04-10T07:46:54Z")),
        ("20151230152248", Some("2015-12-30T15:22:48")),
        ("D:2015", Some("2015-01-01T00:00:00")),
        ("D:201512", Some("2015-12-01T00:00:00")),
        ("D:2015123015", Some("2015-12-30T15:00:00")),
        ("D:20151230152248+01", Some("2015-12-30T15:22:48+01:00")),
        ("garbage", None),
        ("", None),
        ("D:", None),
        ("D:20151330", None),
        ("D:20151232", None),
        ("D:2015123025", None),
        ("D:20151230152248+25'00'", None),
        ("D:20151230152248 trailing", None),
        // Real calendar days only.
        ("D:20260231", None),
        ("D:20230229", None),
        ("D:20240229", Some("2024-02-29T00:00:00")),
        ("D:21000229", None),
        ("D:20000229", Some("2000-02-29T00:00:00")),
    ];
    for (raw, iso) in cases {
        assert_eq!(pdf_date(raw).as_deref(), iso, "{raw}");
    }
}

#[test]
fn info_and_xmp_fields() {
    let irs = corpus("irs-f1040-2025.pdf");
    let info = irs.info();
    assert_eq!(info.author.as_deref(), Some("C:DC:TS:CAR:MP"));
    assert_eq!(info.creator.as_deref(), Some("Designer 6.5"));
    assert_eq!(info.producer.as_deref(), Some("Designer 6.5"));
    assert_eq!(info.keywords.as_deref(), Some("Fillable"));
    assert!(
        info.subject
            .as_deref()
            .unwrap()
            .starts_with("U.S. Individual Income T"),
        "{info:?}"
    );
    let created = info.created.unwrap();
    assert_eq!(created.raw, "D:20260102072431-06'00'");
    assert_eq!(created.iso.as_deref(), Some("2026-01-02T07:24:31-06:00"));
    assert_eq!(
        info.modified.unwrap().iso.as_deref(),
        Some("2026-01-02T07:24:31-06:00")
    );
    assert_eq!(irs.language().as_deref(), Some("en-US"));
    assert!(irs.tagged());

    let xmp = irs.xmp();
    let get = |k: &str| {
        xmp.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("dc:title"), Some("2025 Form 1040"));
    assert_eq!(get("dc:creator"), Some("C:DC:TS:CAR:MP"));
    assert_eq!(
        get("dc:description"),
        Some("U.S. Individual Income Tax Return")
    );
    assert_eq!(get("xmp:CreateDate"), Some("2026-01-02T07:24:31-06:00"));
    assert_eq!(get("xmp:ModifyDate"), Some("2026-01-02T07:24:31-06:00"));
    assert_eq!(get("xmp:CreatorTool"), Some("Designer 6.5"));
    assert_eq!(get("pdf:Producer"), Some("Designer 6.5"));
    assert_eq!(get("pdf:Keywords"), Some("Fillable"));
    assert_eq!(get("pdfaid:part"), None);

    let pypdf = corpus("pypdf-xmp.pdf");
    let info = pypdf.info();
    let created = info.created.unwrap();
    assert_eq!(created.raw, "D:20230410074654Z07'46'");
    assert_eq!(created.iso.as_deref(), Some("2023-04-10T07:46:54Z"));
    assert_eq!(info.title, None);
    assert!(!pypdf.tagged());
    assert_eq!(pypdf.language(), None);
    let xmp = pypdf.xmp();
    assert_eq!(
        xmp,
        vec![
            (
                "dc:title".to_string(),
                "Sample PDF with XMP Metadata".to_string()
            ),
            ("dc:creator".to_string(), "John Doe".to_string()),
            ("dc:description".to_string(), "This is a text".to_string()),
        ]
    );
}

#[test]
fn xmp_attributes_and_lists() {
    let pdf = with_xmp(
        r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:a="http://www.aiim.org/pdfa/ns/id/"
     xmlns:x2="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/"
     a:part="1" a:conformance="B" x2:CreateDate="2020-01-02T03:04:05Z">
   <dc:creator><rdf:Seq><rdf:li>Ann</rdf:li><rdf:li>Bob</rdf:li></rdf:Seq></dc:creator>
   <dc:title><rdf:Alt><rdf:li xml:lang="de">Titel</rdf:li><rdf:li xml:lang="x-default">Title &amp; more</rdf:li></rdf:Alt></dc:title>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#,
    );
    assert_eq!(
        pdf.xmp(),
        vec![
            ("dc:title".to_string(), "Title & more".to_string()),
            ("dc:creator".to_string(), "Ann; Bob".to_string()),
            (
                "xmp:CreateDate".to_string(),
                "2020-01-02T03:04:05Z".to_string()
            ),
            ("pdfaid:part".to_string(), "1".to_string()),
            ("pdfaid:conformance".to_string(), "B".to_string()),
        ]
    );
}

#[test]
fn xmp_doctype_is_refused() {
    let pdf = with_xmp(
        r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "aaaa">]>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:description>&a;</dc:description></rdf:Description>
</rdf:RDF></x:xmpmeta>"#,
    );
    assert!(pdf.xmp().is_empty());
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Unsupported && i.what.contains("DOCTYPE")),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn version_from_header_or_catalog() {
    let open = |header: &str, catalog: Option<&str>| {
        let mut doc = Document::with_version(header);
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let mut cat = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
        if let Some(v) = catalog {
            cat.set("Version", Object::Name(v.as_bytes().to_vec()));
        }
        let catalog_id = doc.add_object(cat);
        doc.trailer.set("Root", catalog_id);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        Pdf::open(&out, None, &Limits::default(), &never)
            .unwrap()
            .version()
    };
    assert_eq!(open("1.4", None), "1.4");
    assert_eq!(open("1.4", Some("1.7")), "1.7");
    // An earlier or broken catalog version does not lower the header's.
    assert_eq!(open("1.6", Some("1.3")), "1.6");
    assert_eq!(open("1.6", Some("seven")), "1.6");
    assert_eq!(corpus("irs-f1040-2025.pdf").version(), "1.7");
}

#[test]
fn xmp_dates_are_checked() {
    for ok in [
        "2026",
        "2026-01",
        "2026-01-02",
        "2026-01-02T07:24",
        "2026-01-02T07:24:31",
        "2026-01-02T07:24:31.123",
        "2026-01-02T07:24:31Z",
        "2026-01-02T07:24:31-06:00",
        "2024-02-29T23:59:30.25+05:30",
    ] {
        assert_eq!(xmp_date(ok).as_deref(), Some(ok), "{ok}");
    }
    assert_eq!(xmp_date(" 2026-01-02 ").as_deref(), Some("2026-01-02"));
    for bad in [
        "",
        "yesterday",
        "yesterday\ntitle: injected",
        "2026-02-31T10:00:00Z",
        "2023-02-29",
        "2026-13-01",
        "2026-01-02T25:00",
        "2026-01-02T07:24:31+24:00",
        "2026-01-02T07:24:31.Z",
        "2026-01-02T07",
        "2026-01-02 07:24",
        "2026-1-2",
        "2026-01-02T07:24:31Zjunk",
        "20260102",
    ] {
        assert_eq!(xmp_date(bad), None, "{bad:?}");
    }
}
