// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pages that need vision, rendered to PNG on request, and the fallback parser for files
//! the object layer cannot read.

mod common;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use common::Builder;
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Asset, Block, Bytes, Completeness, Confidence, Detection, FormatId, LossReason,
    ReadOptions, ReadOutcome, Reader, SectionKind, Severity, Source, plain_text,
};
use rezzer_read_pdf::PdfReader;
use sha2::{Digest, Sha256};

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn corpus(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/corpus/pdf");
    std::fs::read(path.join(name)).expect("corpus file")
}

fn source(bytes: &[u8]) -> Source {
    Source {
        path: None,
        name: "file.pdf".into(),
        bytes: Bytes::Owned(Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format: FormatId::Pdf,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "%PDF-".into(),
        },
        modified: None,
    }
}

fn read_opts(bytes: &[u8], extract: bool, limits: Limits) -> ReadOutcome {
    let opts = ReadOptions {
        extract_assets: extract,
        ..ReadOptions::default()
    };
    PdfReader.read(&source(bytes), &opts, &Budget::new(limits))
}

fn read(bytes: &[u8]) -> ReadOutcome {
    read_opts(bytes, false, Limits::default())
}

fn page(out: &ReadOutcome, n: u32) -> &[Block] {
    let want = Anchor(format!("p{n}"));
    out.document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section {
                kind: SectionKind::Page,
                anchor: Some(a),
                blocks,
                ..
            } if *a == want => Some(blocks.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no section p{n} in {:#?}", out.document.body))
}

/// One line per paragraph or heading, and `[media <asset>]` for a picture.
fn lines(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    for b in blocks {
        match b {
            Block::Heading { inlines, .. } | Block::Paragraph { inlines, .. } => {
                out.push(plain_text(inlines))
            }
            Block::Media { asset, .. } => out.push(format!("[media {asset}]")),
            _ => {}
        }
    }
    out
}

fn asset<'a>(out: &'a ReadOutcome, id: &str) -> &'a Asset {
    out.document
        .assets
        .iter()
        .find(|a| a.id == id)
        .unwrap_or_else(|| panic!("no asset {id} in {:#?}", out.document.assets))
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(data).expect("deflate");
    z.finish().expect("deflate")
}

/// A one-page file (`w` by `h` points) whose content is `content`, with Helvetica as `/F1`
/// and a gray image of `iw` by `ih` pixels as `/Im1` (its data, deflated, is `pixels`).
fn image_page(w: u32, h: u32, content: &str, iw: u32, ih: u32, pixels: &[u8]) -> Vec<u8> {
    image_page_bits(w, h, content, iw, ih, 8, pixels)
}

/// [`image_page`] with `bits` per pixel.
fn image_page_bits(
    w: u32,
    h: u32,
    content: &str,
    iw: u32,
    ih: u32,
    bits: u32,
    pixels: &[u8],
) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let image = b.stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceGray \
             /BitsPerComponent {bits} /Filter /FlateDecode"
        ),
        &deflate(pixels),
    );
    let c = b.stream("", content.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] \
         /Resources << /Font << /F1 {font} 0 R >> /XObject << /Im1 {image} 0 R >> >> \
         /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// An A4 page with a paragraph above a picture over half the page and one below it.
fn figure_page() -> Vec<u8> {
    let content = "BT /F1 12 Tf 72 780 Td (Above the picture.) Tj ET\n\
                   q 451 0 0 421 72 300 cm /Im1 Do Q\n\
                   BT /F1 12 Tf 72 200 Td (Below the picture.) Tj ET\n";
    image_page(595, 842, content, 40, 40, &[128u8; 1600])
}

/// The width and height a PNG states in its header.
fn png_size(png: &[u8]) -> (u32, u32) {
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
    (w, h)
}

#[test]
fn scanned_page_needs_vision() {
    let out = read(&corpus("scan-truman-letter-nara.pdf"));
    let a = asset(&out, "page-1");
    assert!(a.needs_vision);
    assert_eq!(a.mime, "image/png");
    assert!(
        a.alt
            .as_deref()
            .is_some_and(|t| t.contains("no text layer")),
        "{a:?}"
    );
    assert!(lines(page(&out, 1)).contains(&"[media page-1]".to_string()));
    // Nothing is rendered unless asked.
    assert!(out.blobs.is_empty());
}

#[test]
fn picture_page_has_media_block() {
    let out = read(&figure_page());
    assert_eq!(
        lines(page(&out, 1)),
        [
            "Page 1",
            "Above the picture.",
            "[media page-1]",
            "Below the picture."
        ]
    );
    let a = asset(&out, "page-1");
    assert_eq!(a.alt.as_deref(), Some("1 picture"));
    assert!(a.needs_vision);
    // The arXiv report has figures: some of its pages need a look. It is fetched, not kept.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf/arxiv-2408.09869-docling-report.pdf");
    if !path.exists() {
        eprintln!("skipped: {} is not fetched", path.display());
        return;
    }
    let arxiv = read(&std::fs::read(path).expect("corpus file"));
    assert!(
        arxiv
            .document
            .assets
            .iter()
            .any(|a| a.needs_vision && a.alt.as_deref().is_some_and(|t| t.contains("picture"))),
        "{:#?}",
        arxiv.document.assets
    );
}

#[test]
fn render_written_only_with_extract_assets() {
    let file = figure_page();
    let plain = read_opts(&file, false, Limits::default());
    assert!(plain.blobs.is_empty());
    assert!(asset(&plain, "page-1").sha256.is_empty());
    let out = read_opts(&file, true, Limits::default());
    let blob = out
        .blobs
        .iter()
        .find(|b| b.asset == "page-1")
        .expect("a rendered page");
    assert!(blob.bytes.starts_with(PNG_SIGNATURE));
    let (w, h) = png_size(&blob.bytes);
    // 144 dpi: twice the page size in points.
    assert_eq!((w, h), (1190, 1684));
    assert!(u64::from(w) * u64::from(h) <= 16_000_000);
    let sha: String = Sha256::digest(&blob.bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(asset(&out, "page-1").sha256, sha);
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
}

#[test]
fn render_cap_gives_limit_loss() {
    // A picture of 20,000 by 20,000 pixels: 400,000,000 pixels, past the render cap. Its
    // data stays encoded, so the guard lets the page through.
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 20000 /Height 20000 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /Filter /DCTDecode",
        b"not a jpeg",
    );
    let c = b.stream("", b"q 595 0 0 842 0 0 cm /Im1 Do Q\n");
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /XObject << /Im1 {image} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read_opts(&b.finish(catalog), true, Limits::default());
    assert!(asset(&out, "page-1").needs_vision);
    assert!(out.blobs.is_empty());
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.why == LossReason::Limit && l.what.contains("not rendered"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(loss.location, "p1");
    assert!(loss.what.contains("its pictures"), "{loss:?}");
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn render_room_gives_limit_loss() {
    // Noise does not pack: twelve pages of it take more than the eighth of the memory limit
    // that extracted pictures may take, though each page renders within its render room.
    let mut state = 12345u32;
    let noise: Vec<u8> = (0..1000 * 1000)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (state >> 16) as u8
        })
        .collect();
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 1000 /Height 1000 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /Filter /FlateDecode",
        &deflate(&noise),
    );
    let c = b.stream("", b"q 595 0 0 842 0 0 cm /Im1 Do Q\n");
    let mut kids = String::new();
    for _ in 0..12 {
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /XObject << /Im1 {image} 0 R >> >> /Contents {c} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(tree, format!("<< /Type /Pages /Kids [{kids}] /Count 12 >>"));
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let limits = Limits {
        max_memory_bytes: 128 << 20,
        ..Limits::default()
    };
    let out = read_opts(&b.finish(catalog), true, limits);
    assert!(
        !out.blobs.is_empty() && out.blobs.len() < 12,
        "{}",
        out.blobs.len()
    );
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.why == LossReason::Limit && l.what.contains("not rendered"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert!(loss.what.contains("bytes left"), "{loss:?}");
}

/// The text of every page section.
fn all_text(out: &ReadOutcome) -> String {
    out.document
        .body
        .iter()
        .filter_map(|b| match b {
            Block::Section { blocks, .. } => Some(lines(blocks).join("\n")),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn bad_xref_falls_back_with_loss() {
    // The object layer rebuilds the table from a scan, so the fallback parser is not needed.
    let out = read(&corpus("qpdf-malformed-bad-xref-entry.pdf"));
    assert!(all_text(&out).contains("Potato"));
    let losses = &out.document.losses;
    assert!(
        losses
            .iter()
            .any(|l| l.what.contains("rebuilt from a file scan")),
        "{losses:#?}"
    );
    assert!(
        !losses.iter().any(|l| l.what.contains("fallback parser")),
        "{losses:#?}"
    );
}

#[test]
fn prefixed_file_falls_back_with_loss() {
    // ISO 32000 puts the header in the first 1024 bytes; this one comes later.
    let mut file = vec![b' '; 2000];
    file.extend(corpus("qpdf-malformed-bad-xref-entry.pdf"));
    let out = read(&file);
    assert!(all_text(&out).contains("Potato"), "{:#?}", out.document);
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.what.contains("fallback parser"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(
        loss.what,
        "the object layer could not be read (not a PDF file); text read with the fallback \
         parser, annotations, forms, scripts and signatures not read"
    );
    assert_eq!(loss.why, LossReason::Uncertain);
    assert_eq!(loss.severity, Severity::Warning);
    assert_eq!(loss.location, "document");
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn page_with_fonts_but_no_text_uses_fallback() {
    // The content stream's dictionary holds a bare keyword: the object layer drops the
    // object, the fallback parser reads it. The bytes keep their length, so the table holds.
    let file = corpus("qpdf-malformed-damaged-stream.pdf");
    let text = unsafe { String::from_utf8_unchecked(file) };
    let twin = text
        .replacen("<< /Length 44 >>", "<</Length 44 x>>", 1)
        .into_bytes();
    let out = read(&twin);
    assert!(all_text(&out).contains("Potato"), "{:#?}", out.document);
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.what.contains("fallback parser"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(loss.what, "page 1: text read with the fallback parser");
    assert_eq!(loss.location, "p1");
    assert_eq!(loss.why, LossReason::Uncertain);
    assert_eq!(loss.severity, Severity::Warning);
}

#[test]
fn damaged_stream_is_partial() {
    let out = read(&corpus("qpdf-malformed-damaged-stream.pdf"));
    assert!(all_text(&out).contains("Potato"));
    assert_eq!(out.completeness, Completeness::Partial);
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.why == LossReason::Damaged)
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(loss.location, "p1");
    assert!(loss.what.contains("stream 5 0 is damaged"), "{loss:?}");
}

#[test]
fn a_broken_content_stream_names_its_page() {
    // Task 11's carry: a FlateDecode content stream that does not unpack in full (a copy from
    // before the start of the data, after some text).
    let mut d = common::Deflate::zlib();
    d.fixed_block(true);
    for c in b"BT /F1 12 Tf 72 700 Td (Read.) Tj ET\n" {
        d.symbol(u32::from(*c));
    }
    d.far_copy();
    d.symbol(256);
    let packed = d.finish();
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let c = b.stream("/Filter /FlateDecode", &packed);
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.location == "p1" && l.why == LossReason::Damaged),
        "{:#?}",
        out.document.losses
    );
}

/// Three A4 pages of text, each with a small picture (a logo of 120 by 100 points, 2.4 % of
/// the page) at `spots[i]`.
fn logo_pages(spots: [(u32, u32); 3]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /Filter /FlateDecode",
        &deflate(&[30u8; 16]),
    );
    let mut kids = String::new();
    for (x, y) in spots {
        let c = b.stream(
            "",
            format!(
                "BT /F1 12 Tf 72 700 Td (Some text.) Tj ET\nq 120 0 0 100 {x} {y} cm /Im1 Do Q\n"
            )
            .as_bytes(),
        );
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> /XObject << /Im1 {image} 0 R >> >> \
             /Contents {c} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(tree, format!("<< /Type /Pages /Kids [{kids}] /Count 3 >>"));
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn a_small_picture_on_every_page_is_furniture() {
    let out = read(&logo_pages([(10, 10); 3]));
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
    // The same pictures at other places are content.
    let out = read(&logo_pages([(10, 10), (200, 300), (400, 100)]));
    assert_eq!(out.document.assets.len(), 3, "{:#?}", out.document.assets);
}

#[test]
fn a_fallback_page_that_needs_vision_says_it_is_not_rendered() {
    let mut file = vec![b' '; 2000];
    file.extend(figure_page());
    let out = read_opts(&file, true, Limits::default());
    assert!(asset(&out, "page-1").needs_vision);
    assert!(out.blobs.is_empty());
    assert!(
        out.document.losses.iter().any(|l| l.location == "p1"
            && l.what == "page 1 is not rendered: the file is read with the fallback parser"),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn small_square_picture_is_listed() {
    // A gray square picture of 85 by 85 points (1.4 % of the page, under the share that makes
    // a page need vision) between two paragraphs, as a QR code on a ticket.
    let content = "BT /F1 12 Tf 72 780 Td (Above the code.) Tj ET\n\
                   q 85 0 0 85 400 600 cm /Im1 Do Q\n\
                   BT /F1 12 Tf 72 500 Td (Below the code.) Tj ET\n";
    // 21 by 21 pixels of 1 bit: 3 bytes a row.
    let out = read(&image_page_bits(
        595,
        842,
        content,
        21,
        21,
        1,
        &[0x5au8; 63],
    ));
    let a = asset(&out, "page-1");
    assert!(a.needs_vision);
    let alt = a.alt.as_deref().unwrap_or_default();
    assert!(alt.contains("1 small picture"), "{alt}");
    assert!(alt.contains("400, 600 to 485, 685"), "{alt}");
    assert!(
        alt.contains("likely a barcode or QR code, not decoded"),
        "{alt}"
    );
    assert_eq!(
        lines(page(&out, 1)),
        [
            "Page 1",
            "Above the code.",
            "[media page-1]",
            "Below the code."
        ]
    );
    // A small picture that is not near square is listed, but not named a code.
    let content = "BT /F1 12 Tf 72 780 Td (Text.) Tj ET\nq 85 0 0 40 400 600 cm /Im1 Do Q\n";
    let out = read(&image_page(595, 842, content, 21, 10, &[0u8; 210]));
    let alt = asset(&out, "page-1").alt.clone().unwrap_or_default();
    assert!(alt.contains("1 small picture"), "{alt}");
    assert!(!alt.contains("barcode"), "{alt}");
    // Eleven small pictures: the first ten are named with their box, then how many more.
    let mut content = String::from("BT /F1 12 Tf 72 780 Td (Text.) Tj ET\n");
    for k in 0..11 {
        content += &format!("q 30 0 0 30 {} 600 cm /Im1 Do Q\n", 40 + 45 * k);
    }
    let out = read(&image_page(595, 842, &content, 21, 21, &[0u8; 441]));
    let alt = asset(&out, "page-1").alt.clone().unwrap_or_default();
    assert!(
        alt.starts_with("11 small pictures (box 40, 600 to 70, 630"),
        "{alt}"
    );
    assert_eq!(alt.matches("box ").count(), 10, "{alt}");
    assert!(alt.ends_with("; 1 more)"), "{alt}");
    // Not named a code: a gray photo of 8 by 8 pixels drawn square, a stencil icon of 8 by 8
    // pixels, and a picture of 21 by 21 gray pixels of 8 bits.
    let content = "BT /F1 12 Tf 72 780 Td (Text.) Tj ET\nq 60 0 0 60 72 600 cm /Im1 Do Q\n";
    for out in [
        read(&image_page(595, 842, content, 8, 8, &[90u8; 64])),
        read(&image_page_bits(595, 842, content, 8, 8, 1, &[0x5au8; 8])),
        read(&image_page(595, 842, content, 21, 21, &[0u8; 441])),
    ] {
        let alt = asset(&out, "page-1").alt.clone().unwrap_or_default();
        assert!(alt.contains("1 small picture"), "{alt}");
        assert!(!alt.contains("barcode"), "{alt}");
    }
    // A picture under 1 square centimeter is not listed.
    let content = "BT /F1 12 Tf 72 780 Td (Text.) Tj ET\nq 20 0 0 20 400 600 cm /Im1 Do Q\n";
    let out = read(&image_page(595, 842, content, 21, 21, &[0u8; 441]));
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
}

#[test]
fn a_small_logo_on_every_page_is_not_listed() {
    // 30 pages with the same small picture at the same place: furniture, no asset. A page with
    // a small picture of its own is one asset.
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 3 /ColorSpace /DeviceRGB \
         /BitsPerComponent 8 /Filter /FlateDecode",
        &deflate(&[200u8; 36]),
    );
    let mut kids = String::new();
    for n in 1..=30 {
        let mut c = format!("BT /F1 11 Tf 72 700 Td (Body text of page {n}.) Tj ET\n");
        c += "q 32 0 0 32 480 780 cm /Im1 Do Q\n";
        if n == 7 {
            c += "q 40 0 0 40 200 300 cm /Im1 Do Q\n";
        }
        let cs = b.stream("", c.as_bytes());
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> /XObject << /Im1 {image} 0 R >> >> \
             /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(tree, format!("<< /Type /Pages /Kids [{kids}] /Count 30 >>"));
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    let ids: Vec<&str> = out.document.assets.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["page-7"], "{:#?}", out.document.assets);
    let alt = asset(&out, "page-7").alt.clone().unwrap_or_default();
    assert!(
        alt.starts_with("1 small picture (box 200, 300 to 240, 340"),
        "{alt}"
    );
}
