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
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
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
    let a = asset(&out, "page-1-pic-1");
    assert!(a.needs_vision);
    assert_eq!(
        a.alt.as_deref(),
        Some("small picture, box 400, 600 to 485, 685, barcode or QR code found, not decoded")
    );
    assert_eq!(
        lines(page(&out, 1)),
        [
            "Page 1",
            "Above the code.",
            "[media page-1-pic-1]",
            "Below the code."
        ]
    );
    // A small picture that is not near square is listed, but not named a code.
    let content = "BT /F1 12 Tf 72 780 Td (Text.) Tj ET\nq 85 0 0 40 400 600 cm /Im1 Do Q\n";
    let out = read(&image_page(595, 842, content, 21, 10, &[0u8; 210]));
    let alt = asset(&out, "page-1-pic-1").alt.clone().unwrap_or_default();
    assert_eq!(alt, "small picture, box 400, 600 to 485, 640");
    // One picture drawn eleven times: one asset, eleven places; its alt text names the first
    // ten with their box, then how many more.
    let mut content = String::from("BT /F1 12 Tf 72 780 Td (Text.) Tj ET\n");
    for k in 0..11 {
        content += &format!("q 30 0 0 30 {} 600 cm /Im1 Do Q\n", 40 + 45 * k);
    }
    let out = read(&image_page(595, 842, &content, 21, 21, &[0u8; 441]));
    assert_eq!(out.document.assets.len(), 1, "{:#?}", out.document.assets);
    let alt = asset(&out, "page-1-pic-1").alt.clone().unwrap_or_default();
    assert!(
        alt.starts_with("small picture, 11 places on page 1: box 40, 600 to 70, 630 on page 1;"),
        "{alt}"
    );
    assert_eq!(alt.matches("box ").count(), 10, "{alt}");
    assert!(alt.ends_with("; 1 more place"), "{alt}");
    let shown = lines(page(&out, 1));
    assert_eq!(
        shown
            .iter()
            .filter(|l| *l == "[media page-1-pic-1]")
            .count(),
        11,
        "{shown:?}"
    );
    // Not named a code: a gray photo of 8 by 8 pixels drawn square, a stencil icon of 8 by 8
    // pixels, and a picture of 21 by 21 gray pixels of 8 bits.
    let content = "BT /F1 12 Tf 72 780 Td (Text.) Tj ET\nq 60 0 0 60 72 600 cm /Im1 Do Q\n";
    for out in [
        read(&image_page(595, 842, content, 8, 8, &[90u8; 64])),
        read(&image_page_bits(595, 842, content, 8, 8, 1, &[0x5au8; 8])),
        read(&image_page(595, 842, content, 21, 21, &[0u8; 441])),
    ] {
        let alt = asset(&out, "page-1-pic-1").alt.clone().unwrap_or_default();
        assert_eq!(alt, "small picture, box 72, 600 to 132, 660");
    }
    // A picture under 1 square centimeter is not listed.
    let content = "BT /F1 12 Tf 72 780 Td (Text.) Tj ET\nq 20 0 0 20 400 600 cm /Im1 Do Q\n";
    let out = read(&image_page(595, 842, content, 21, 21, &[0u8; 441]));
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
}

#[test]
fn a_small_logo_on_every_page_is_not_listed() {
    // 30 pages with the same small picture at the same place: furniture, no asset. A page that
    // draws it once more at another place lists that place.
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
    assert_eq!(ids, ["page-7-pic-1"], "{:#?}", out.document.assets);
    let alt = asset(&out, "page-7-pic-1").alt.clone().unwrap_or_default();
    // It is the picture drawn on every page: a logo, also at its other place.
    assert_eq!(alt, "logo or small picture, box 200, 300 to 240, 340");
}

#[test]
fn a_repeated_logo_whose_size_changes_a_little_is_furniture() {
    // 30 pages with one logo at one place, 60, 60.4 or 60.8 points wide: one picture.
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
        let w = 60.0 + 0.4 * (n % 3) as f32;
        let c = format!(
            "BT /F1 11 Tf 72 700 Td (Body {n}.) Tj ET\nq {w} 0 0 45 480 780 cm /Im1 Do Q\n"
        );
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
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
}

/// A picture drawn on a page: x, y, width, height and its index in the pictures.
type Draw = (f32, f32, f32, f32, usize);

/// Pages of A4, each with a line of text and the pictures `draws` names, by their index in
/// `pictures` (each given as dictionary and data).
fn picture_pages(draws: &[Vec<Draw>], pictures: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut xo = String::new();
    for (k, (dict, data)) in pictures.iter().enumerate() {
        let n = b.stream(dict, data);
        xo.push_str(&format!("/P{k} {n} 0 R "));
    }
    let mut kids = String::new();
    for (n, page) in draws.iter().enumerate() {
        let mut c = format!("BT /F1 11 Tf 72 700 Td (Page {n}.) Tj ET\n");
        for (x, y, w, h, k) in page {
            c += &format!("q {w} 0 0 {h} {x} {y} cm /P{k} Do Q\n");
        }
        let cs = b.stream("", c.as_bytes());
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> /XObject << {xo}>> >> /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", draws.len()),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn different_pictures_at_one_place_are_not_furniture() {
    // Four tickets, each with its own code (a 24 by 24 stencil) at the same place: four
    // assets.
    let mask = "/Type /XObject /Subtype /Image /Width 24 /Height 24 /ImageMask true \
                /BitsPerComponent 1";
    let codes: Vec<(&str, Vec<u8>)> = (0..4u32)
        .map(|k| (mask, (0..72u32).map(|i| (i * 7 + k * 13) as u8).collect()))
        .collect();
    let draws: Vec<_> = (0..4)
        .map(|k| vec![(400.0, 600.0, 85.0, 85.0, k)])
        .collect();
    let out = read(&picture_pages(&draws, &codes));
    assert_eq!(out.document.assets.len(), 4, "{:#?}", out.document.assets);
    // Two logos taking turns 1.5 points apart on four pages: each is on two pages only, so
    // not furniture, and each is one asset for its two pages.
    let rgb = "/Type /XObject /Subtype /Image /Width 4 /Height 3 /ColorSpace /DeviceRGB \
               /BitsPerComponent 8";
    let logos = [(rgb, vec![200u8; 36]), (rgb, vec![50u8; 36])];
    let draws: Vec<_> = (0..4)
        .map(|n| {
            if n % 2 == 0 {
                vec![(480.0, 780.0, 60.0, 45.0, 0)]
            } else {
                vec![(481.5, 780.0, 60.0, 45.0, 1)]
            }
        })
        .collect();
    let out = read(&picture_pages(&draws, &logos));
    let ids: Vec<&str> = out.document.assets.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["page-1-pic-1", "page-2-pic-1"]);
    // The same two logos taking turns at exactly one place: not furniture either.
    let draws: Vec<_> = (0..4)
        .map(|n| vec![(480.0, 780.0, 60.0, 45.0, n % 2)])
        .collect();
    let out = read(&picture_pages(&draws, &logos));
    let ids: Vec<&str> = out.document.assets.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["page-1-pic-1", "page-2-pic-1"]);
    // The same logo on four pages stays furniture.
    let draws: Vec<_> = (0..4)
        .map(|_| vec![(480.0, 780.0, 60.0, 45.0, 0)])
        .collect();
    let out = read(&picture_pages(&draws, &logos));
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
    // One stencil object on four pages stays furniture too.
    let draws: Vec<_> = (0..4)
        .map(|_| vec![(400.0, 600.0, 85.0, 85.0, 0)])
        .collect();
    let out = read(&picture_pages(&draws, &codes[..1]));
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
}

/// Pages of A4, each with a line of text and one picture drawn `draws` times at 400, 600 (85
/// points square); `make` adds the picture object for a page (and anything it refers to) and
/// gives its number.
fn copied_pages(pages: usize, draws: usize, make: &dyn Fn(&mut Builder, usize) -> u32) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut kids = String::new();
    for n in 0..pages {
        let image = make(&mut b, n);
        let mut c = format!("BT /F1 11 Tf 72 700 Td (Page {n}.) Tj ET\n");
        for _ in 0..draws {
            c += "q 85 0 0 85 400 600 cm /Im Do Q\n";
        }
        let cs = b.stream("", c.as_bytes());
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> /XObject << /Im {image} 0 R >> >> \
             /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn a_logo_copied_per_page_with_its_own_objects_is_furniture() {
    // Single-page files merged into one: each page has its own copy of the logo, and each
    // copy its own soft mask object with the same bytes. One picture: furniture.
    let rgb = "/Type /XObject /Subtype /Image /Width 4 /Height 3 /BitsPerComponent 8";
    let mask = "/Type /XObject /Subtype /Image /Width 4 /Height 3 /ColorSpace /DeviceGray \
                /BitsPerComponent 8";
    let out = read(&copied_pages(4, 1, &|b, _| {
        let m = b.stream(mask, &[255u8; 12]);
        b.stream(
            &format!("{rgb} /ColorSpace /DeviceRGB /SMask {m} 0 R"),
            &[200u8; 36],
        )
    }));
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
    // The same with its own ICC profile object per copy.
    let out = read(&copied_pages(4, 1, &|b, _| {
        let icc = b.stream("/N 3 /Alternate /DeviceRGB", &[9u8; 128]);
        b.stream(
            &format!("{rgb} /ColorSpace [/ICCBased {icc} 0 R]"),
            &[200u8; 36],
        )
    }));
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
    // The same with its own palette object per copy.
    let out = read(&copied_pages(4, 1, &|b, _| {
        let table = b.stream("", &[10, 20, 30, 40, 50, 60]);
        b.stream(
            &format!("{rgb} /ColorSpace [/Indexed /DeviceRGB 1 {table} 0 R]"),
            &[1u8; 12],
        )
    }));
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
}

#[test]
fn pictures_that_differ_in_mask_or_palette_are_not_furniture() {
    // The same pixels at one place on four pages, each with another soft mask: four pictures.
    let rgb = "/Type /XObject /Subtype /Image /Width 4 /Height 3 /BitsPerComponent 8";
    let mask = "/Type /XObject /Subtype /Image /Width 4 /Height 3 /ColorSpace /DeviceGray \
                /BitsPerComponent 8";
    let out = read(&copied_pages(4, 1, &|b, n| {
        let m = b.stream(mask, &[(n * 60) as u8; 12]);
        b.stream(
            &format!("{rgb} /ColorSpace /DeviceRGB /SMask {m} 0 R"),
            &[200u8; 36],
        )
    }));
    assert_eq!(out.document.assets.len(), 4, "{:#?}", out.document.assets);
    // The same indices with another palette on each page: four pictures.
    let out = read(&copied_pages(4, 1, &|b, n| {
        let table = b.stream("", &[10, 20, 30, 40, 50, (60 + n) as u8]);
        b.stream(
            &format!("{rgb} /ColorSpace [/Indexed /DeviceRGB 1 {table} 0 R]"),
            &[1u8; 12],
        )
    }));
    assert_eq!(out.document.assets.len(), 4, "{:#?}", out.document.assets);
}

#[test]
fn pictures_that_differ_only_after_64_kib_are_not_furniture() {
    // Four 300 by 300 gray pictures, unpacked, the same in their first 70,000 bytes, one per
    // page at one place: four pictures.
    let gray = "/Type /XObject /Subtype /Image /Width 300 /Height 300 /ColorSpace /DeviceGray \
                /BitsPerComponent 8";
    let out = read(&copied_pages(4, 1, &|b, n| {
        let mut v = vec![128u8; 90_000];
        for (i, p) in v.iter_mut().enumerate().skip(70_000) {
            *p = (i * (n + 1)) as u8;
        }
        b.stream(gray, &v)
    }));
    assert_eq!(out.document.assets.len(), 4, "{:#?}", out.document.assets);
}

#[test]
fn a_big_picture_drawn_many_times_is_known_once() {
    // One page draws a 1 MB unpacked picture 100,000 times, another a 4 by 3 one as often.
    // Knowing the picture is done once per object, so the big one takes not much longer.
    let big = copied_pages(1, 100_000, &|b, _| {
        b.stream(
            "/Type /XObject /Subtype /Image /Width 1000 /Height 1000 /ColorSpace /DeviceGray \
             /BitsPerComponent 8",
            &vec![7u8; 1_000_000],
        )
    });
    let small = copied_pages(1, 100_000, &|b, _| {
        b.stream(
            "/Type /XObject /Subtype /Image /Width 4 /Height 3 /ColorSpace /DeviceGray \
             /BitsPerComponent 8",
            &[7u8; 12],
        )
    });
    let start = std::time::Instant::now();
    let out = read(&small);
    let small_time = start.elapsed();
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
    let start = std::time::Instant::now();
    let out = read(&big);
    let big_time = start.elapsed();
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
    eprintln!("small {small_time:?}, big {big_time:?}");
    assert!(
        big_time < small_time * 2 + std::time::Duration::from_millis(300),
        "small {small_time:?}, big {big_time:?}"
    );
}

/// One page that draws `n` picture objects of 4 by 3 pixels, each with its own data, at one
/// place; with `mask` they all share one soft mask of 1000 by 1000 pixels (1 MB unpacked).
fn many_pictures_page(n: usize, mask: bool) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let extra = if mask {
        let m = b.stream(
            "/Type /XObject /Subtype /Image /Width 1000 /Height 1000 /ColorSpace /DeviceGray \
             /BitsPerComponent 8",
            &vec![255u8; 1_000_000],
        );
        format!(" /SMask {m} 0 R")
    } else {
        String::new()
    };
    let mut xo = String::new();
    let mut c = String::from("BT /F1 11 Tf 72 700 Td (Page.) Tj ET\n");
    for k in 0..n {
        let data: Vec<u8> = (0..36).map(|i| (k * 7 + i * 13 + k / 256) as u8).collect();
        let im = b.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width 4 /Height 3 /ColorSpace /DeviceRGB \
                 /BitsPerComponent 8{extra}"
            ),
            &data,
        );
        xo.push_str(&format!("/I{k} {im} 0 R "));
        c += &format!("q 85 0 0 85 400 600 cm /I{k} Do Q\n");
    }
    let cs = b.stream("", c.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> /XObject << {xo}>> >> /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn a_soft_mask_many_pictures_share_is_read_once() {
    // 50,000 pictures share one soft mask of 1 MB: knowing them takes not much longer than
    // knowing 50,000 pictures without a mask.
    let plain = many_pictures_page(50_000, false);
    let masked = many_pictures_page(50_000, true);
    let start = std::time::Instant::now();
    let out = read(&plain);
    let plain_time = start.elapsed();
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
    let start = std::time::Instant::now();
    let out = read(&masked);
    let masked_time = start.elapsed();
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
    eprintln!("plain {plain_time:?}, masked {masked_time:?}");
    assert!(
        masked_time < plain_time * 3 / 2 + std::time::Duration::from_millis(300),
        "plain {plain_time:?}, masked {masked_time:?}"
    );
}

#[test]
fn an_inline_picture_with_a_named_color_space_is_its_own() {
    // An inline logo whose color space is a page resource (/CS0): its colors are not known
    // where the picture is drawn, so it is never furniture and is listed on each page.
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut kids = String::new();
    for n in 0..4 {
        let mut c = format!("BT /F1 11 Tf 72 700 Td (Page {n}.) Tj ET\n").into_bytes();
        c.extend_from_slice(b"q 85 0 0 85 400 600 cm BI /W 4 /H 3 /CS /CS0 /BPC 8 ID ");
        c.extend_from_slice(&[200u8; 36]);
        c.extend_from_slice(b"\nEI Q\n");
        let cs = b.stream("", &c);
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> /ColorSpace << /CS0 /DeviceRGB >> >> \
             /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(tree, format!("<< /Type /Pages /Kids [{kids}] /Count 4 >>"));
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    assert_eq!(out.document.assets.len(), 4, "{:#?}", out.document.assets);
}

#[test]
fn small_square_picture_on_a_turned_page_stays_in_place() {
    // The ticket shape drawn upward on a page turned for display: the picture marker stands
    // between the paragraphs in the turned frame, as on an upright page.
    let content = "q 0 1 -1 0 595 0 cm\n\
                   BT /F1 12 Tf 72 500 Td (Above the code.) Tj ET\n\
                   q 85 0 0 85 400 300 cm /Im1 Do Q\n\
                   BT /F1 12 Tf 72 200 Td (Below the code.) Tj ET\nQ\n";
    let out = read(&image_page_bits(
        595,
        842,
        content,
        21,
        21,
        1,
        &[0x5au8; 63],
    ));
    assert_eq!(
        lines(page(&out, 1)),
        [
            "Page 1",
            "Above the code.",
            "[media page-1-pic-1]",
            "Below the code."
        ]
    );
}

/// The ids of the assets that need vision.
fn needing(out: &ReadOutcome) -> Vec<&str> {
    out.document
        .assets
        .iter()
        .filter(|a| a.needs_vision)
        .map(|a| a.id.as_str())
        .collect()
}

/// The Markdown of a read.
fn markdown(out: &ReadOutcome) -> String {
    rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    )
}

/// The bytes written for asset `id`.
fn blob<'a>(out: &'a ReadOutcome, id: &str) -> &'a [u8] {
    &out.blobs
        .iter()
        .find(|b| b.asset == id)
        .unwrap_or_else(|| panic!("no blob {id}"))
        .bytes
}

/// A gray picture of 4 by 4 pixels with values from `base` on, as dictionary and data.
fn gray(base: u8) -> (&'static str, Vec<u8>) {
    (
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
         /BitsPerComponent 8",
        (0..16u8).map(|i| base.wrapping_add(i * 9)).collect(),
    )
}

#[test]
fn logo_on_a_two_page_invoice_needs_no_vision() {
    // An invoice of two pages: the same logo top left on both, text below it.
    let draws = vec![
        vec![(40.0, 760.0, 60.0, 40.0, 0)],
        vec![(40.0, 760.0, 60.0, 40.0, 0)],
    ];
    let out = read_opts(&picture_pages(&draws, &[gray(10)]), true, Limits::default());
    assert!(needing(&out).is_empty(), "{:#?}", out.document.assets);
    // One file for the logo, linked from both pages.
    let ids: Vec<&str> = out.document.assets.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["page-1-pic-1"]);
    for n in 1..=2 {
        let l = lines(page(&out, n));
        assert_eq!(
            l.iter().filter(|l| *l == "[media page-1-pic-1]").count(),
            1,
            "{l:?}"
        );
    }
    // The crop is the box at 144 dpi plus 8 pixels on each side: 60 by 40 points.
    assert_eq!(png_size(blob(&out, "page-1-pic-1")), (136, 96));
    assert_eq!(out.blobs.len(), 1);
    let md = markdown(&out);
    assert_eq!(
        md.matches("logo or small picture, box 40, 760 to 100, 800")
            .count(),
        2,
        "{md}"
    );
    assert!(!md.contains("Needs vision"), "{md}");
    assert!(!md.contains("Pictures that need vision"), "{md}");
}

#[test]
fn single_logo_in_corner_needs_no_vision() {
    // Bottom right, in the corner but above the bottom band.
    let out = read(&picture_pages(
        &[vec![(500.0, 130.0, 60.0, 35.0, 0)]],
        &[gray(10)],
    ));
    assert!(needing(&out).is_empty(), "{:#?}", out.document.assets);
    let alt = asset(&out, "page-1-pic-1").alt.clone().unwrap_or_default();
    assert_eq!(alt, "logo or small picture, box 500, 130 to 560, 165");
}

#[test]
fn small_code_keeps_needs_vision_with_crop() {
    // A 1-bit square code in the bottom right corner.
    let content = "BT /F1 12 Tf 72 780 Td (Ticket.) Tj ET\nq 85 0 0 85 480 20 cm /Im1 Do Q\n";
    let file = image_page_bits(595, 842, content, 21, 21, 1, &[0x5au8; 63]);
    let out = read_opts(&file, true, Limits::default());
    assert_eq!(needing(&out), ["page-1-pic-1"]);
    let alt = asset(&out, "page-1-pic-1").alt.clone().unwrap_or_default();
    assert_eq!(
        alt,
        "small picture, box 480, 20 to 565, 105, barcode or QR code found, not decoded"
    );
    // The crop, not the page.
    assert_eq!(png_size(blob(&out, "page-1-pic-1")), (186, 186));
    assert_eq!(out.blobs.len(), 1);
    let md = markdown(&out);
    assert!(md.contains("### Pictures that need vision"), "{md}");
    assert!(md.contains("| page-1-pic-1 |"), "{md}");
    assert!(md.contains("Needs vision"), "{md}");
}

#[test]
fn small_picture_mid_page_keeps_needs_vision_with_crop() {
    let file = picture_pages(&[vec![(250.0, 400.0, 60.0, 40.0, 0)]], &[gray(10)]);
    let out = read_opts(&file, true, Limits::default());
    assert_eq!(needing(&out), ["page-1-pic-1"]);
    assert_eq!(
        asset(&out, "page-1-pic-1").alt.as_deref(),
        Some("small picture, box 250, 400 to 310, 440")
    );
    assert_eq!(png_size(blob(&out, "page-1-pic-1")), (136, 96));
    assert_eq!(out.blobs.len(), 1);
}

#[test]
fn scan_page_still_renders_whole_page() {
    let out = read_opts(
        &corpus("scan-truman-letter-nara.pdf"),
        true,
        Limits::default(),
    );
    let ids: Vec<&str> = out.blobs.iter().map(|b| b.asset.as_str()).collect();
    assert_eq!(ids, ["page-1"]);
    // A page with a large picture and a small logo: the whole page, the logo not cut twice.
    let content = "BT /F1 12 Tf 72 700 Td (Text.) Tj ET\n\
                   q 300 0 0 300 150 200 cm /Im1 Do Q\nq 60 0 0 40 40 760 cm /Im1 Do Q\n";
    let out = read_opts(
        &image_page(595, 842, content, 4, 4, &[90u8; 16]),
        true,
        Limits::default(),
    );
    let ids: Vec<&str> = out.document.assets.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["page-1"]);
    assert_eq!(png_size(blob(&out, "page-1")), (1190, 1684));
}

#[test]
fn turned_page_logo_band_in_turned_frame() {
    // Text that reads upward, and a picture at x 20 to 80, y 400 to 460: in no band or corner
    // of the upright page, but in the top band of the turned frame.
    let content = "q 0 1 -1 0 595 0 cm\n\
                   BT /F1 12 Tf 72 500 Td (First line of the turned page.) Tj ET\n\
                   BT /F1 12 Tf 72 480 Td (Second line of the turned page.) Tj ET\n\
                   BT /F1 12 Tf 72 200 Td (Third line of the turned page.) Tj ET\nQ\n\
                   q 60 0 0 60 20 400 cm /Im1 Do Q\n";
    let out = read(&image_page(595, 842, content, 4, 4, &[90u8; 16]));
    assert!(needing(&out).is_empty(), "{:#?}", out.document.assets);
    assert_eq!(
        asset(&out, "page-1-pic-1").alt.as_deref(),
        Some("logo or small picture, box 400, 515 to 460, 575")
    );
}

#[test]
fn same_picture_on_every_page_is_a_logo() {
    // The same picture mid-page on both pages of a document: a logo.
    let draws = vec![
        vec![(250.0, 400.0, 60.0, 40.0, 0)],
        vec![(250.0, 400.0, 60.0, 40.0, 0)],
    ];
    let out = read(&picture_pages(&draws, &[gray(10)]));
    assert!(needing(&out).is_empty(), "{:#?}", out.document.assets);
    // On one of two pages only: it needs vision.
    let draws = vec![
        vec![(250.0, 400.0, 60.0, 40.0, 0)],
        vec![(250.0, 400.0, 60.0, 40.0, 1)],
    ];
    let out = read(&picture_pages(&draws, &[gray(10), gray(100)]));
    assert_eq!(needing(&out), ["page-1-pic-1", "page-2-pic-1"]);
}

#[test]
fn equal_pictures_share_one_asset() {
    // One picture at three places on two pages of three: one file, three links.
    let draws = vec![
        vec![(250.0, 400.0, 60.0, 40.0, 0), (100.0, 300.0, 60.0, 40.0, 0)],
        vec![(300.0, 300.0, 60.0, 40.0, 0)],
        vec![],
    ];
    let out = read_opts(&picture_pages(&draws, &[gray(10)]), true, Limits::default());
    assert_eq!(needing(&out), ["page-1-pic-1"]);
    assert_eq!(out.document.assets.len(), 1);
    assert_eq!(out.blobs.len(), 1);
    let links: usize = (1..=3)
        .map(|n| {
            lines(page(&out, n))
                .iter()
                .filter(|l| *l == "[media page-1-pic-1]")
                .count()
        })
        .sum();
    assert_eq!(links, 3);
    // Each place says its own box.
    let md = markdown(&out);
    for b in [
        "box 250, 400 to 310, 440",
        "box 100, 300 to 160, 340",
        "box 300, 300 to 360, 340",
    ] {
        assert!(md.contains(&format!("small picture, {b})")), "{b}: {md}");
    }
}

#[test]
fn different_codes_at_one_place_stay_separate() {
    // Four tickets, each with its own code at the same place: four files.
    let mask = "/Type /XObject /Subtype /Image /Width 24 /Height 24 /ImageMask true \
                /BitsPerComponent 1";
    let codes: Vec<(&str, Vec<u8>)> = (0..4u32)
        .map(|k| (mask, (0..72u32).map(|i| (i * 7 + k * 13) as u8).collect()))
        .collect();
    let draws: Vec<_> = (0..4)
        .map(|k| vec![(400.0, 600.0, 85.0, 85.0, k)])
        .collect();
    let out = read_opts(&picture_pages(&draws, &codes), true, Limits::default());
    assert_eq!(
        needing(&out),
        [
            "page-1-pic-1",
            "page-2-pic-1",
            "page-3-pic-1",
            "page-4-pic-1"
        ]
    );
    assert_eq!(out.blobs.len(), 4);
}

#[test]
fn vision_table_lists_shared_asset_once_with_pages() {
    // One picture on pages 1, 2 and 4 of four, each time at another place (at one place on
    // three pages it would be page furniture).
    let at = |y: f32| vec![(250.0, y, 60.0, 40.0, 0)];
    let draws = vec![at(400.0), at(300.0), vec![], at(200.0)];
    let out = read(&picture_pages(&draws, &[gray(10)]));
    assert_eq!(needing(&out), ["page-1-pic-1"]);
    let md = markdown(&out);
    let rows: Vec<&str> = md
        .lines()
        .filter(|l| l.starts_with("| page-1-pic-1 |"))
        .collect();
    assert_eq!(rows.len(), 1, "{md}");
    assert!(rows[0].contains("pages 1, 2, 4"), "{}", rows[0]);
}

/// Two gray pictures of 300 by 300 pixels, uncompressed (90,000 bytes each, longer than
/// what a partial hash reads), that differ only in rows 230 to 279: black in the second.
fn two_large_rasters() -> Vec<(&'static str, Vec<u8>)> {
    let dict = "/Type /XObject /Subtype /Image /Width 300 /Height 300 /ColorSpace /DeviceGray \
                /BitsPerComponent 8";
    let first = vec![200u8; 90_000];
    let mut second = first.clone();
    second[230 * 300..280 * 300].fill(0);
    vec![(dict, first), (dict, second)]
}

#[test]
fn large_pictures_that_differ_in_the_middle_are_two_assets() {
    // On one page, at two places.
    let draws = vec![vec![
        (250.0, 400.0, 60.0, 40.0, 0),
        (250.0, 300.0, 60.0, 40.0, 1),
    ]];
    let out = read_opts(
        &picture_pages(&draws, &two_large_rasters()),
        true,
        Limits::default(),
    );
    assert_eq!(needing(&out), ["page-1-pic-1", "page-1-pic-2"]);
    assert_eq!(out.blobs.len(), 2);
    // On two pages at one place, as two tickets: not one picture on every page, not a logo.
    let draws = vec![
        vec![(250.0, 400.0, 60.0, 40.0, 0)],
        vec![(250.0, 400.0, 60.0, 40.0, 1)],
    ];
    let out = read(&picture_pages(&draws, &two_large_rasters()));
    assert_eq!(needing(&out), ["page-1-pic-1", "page-2-pic-1"]);
}

/// One A4 page with a line of text and a black picture drawn at `x`, `y`, 60 by 40 points;
/// `page` adds keys to the page dictionary (a crop box, a rotation).
fn black_picture_page(x: f32, y: f32, page: &str) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
         /BitsPerComponent 8",
        &[0u8; 16],
    );
    let c = b.stream(
        "",
        format!("BT /F1 11 Tf 150 650 Td (Text.) Tj ET\nq 60 0 0 40 {x} {y} cm /Im1 Do Q\n")
            .as_bytes(),
    );
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] {page} \
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

/// The crop `png` is `w` by `h` pixels; its dark pixels are exactly the rectangle inside the
/// margin of 8 pixels, and the margin is white.
fn assert_cut_at_the_box(png: &[u8], w: u16, h: u16) {
    let pixmap = hayro::vello_cpu::Pixmap::from_png(std::io::Cursor::new(png)).expect("a PNG file");
    assert_eq!((pixmap.width(), pixmap.height()), (w, h));
    for y in 0..h {
        for x in 0..w {
            let p = pixmap.sample(x, y);
            let inside = (8..w - 8).contains(&x) && (8..h - 8).contains(&y);
            if inside {
                assert!(p.r < 64 && p.g < 64 && p.b < 64, "{x}, {y}: {p:?}");
            } else {
                assert!(p.r > 250 && p.g > 250 && p.b > 250, "{x}, {y}: {p:?}");
            }
        }
    }
}

#[test]
fn a_crop_is_cut_at_the_box_with_a_crop_box() {
    // The visible page starts at 100, 100: the picture at 300, 400 stands at 200, 300 on it.
    let file = black_picture_page(300.0, 400.0, "/CropBox [100 100 495 742]");
    let out = read_opts(&file, true, Limits::default());
    assert_eq!(
        asset(&out, "page-1-pic-1").alt.as_deref(),
        Some("small picture, box 200, 300 to 260, 340")
    );
    assert_cut_at_the_box(blob(&out, "page-1-pic-1"), 136, 96);
}

#[test]
fn a_crop_is_cut_at_the_box_on_a_rotated_page() {
    // The page is shown turned a quarter: the crop is 40 by 60 points, upright as shown.
    let file = black_picture_page(250.0, 400.0, "/Rotate 90");
    let out = read_opts(&file, true, Limits::default());
    assert_cut_at_the_box(blob(&out, "page-1-pic-1"), 96, 136);
}

#[test]
fn two_pictures_at_one_block_are_numbered_from_the_top() {
    // Both below the one line of text; the lower one is drawn first.
    let draws = vec![vec![
        (250.0, 300.0, 60.0, 40.0, 0),
        (250.0, 400.0, 60.0, 40.0, 1),
    ]];
    let out = read(&picture_pages(&draws, &[gray(10), gray(100)]));
    assert_eq!(
        lines(page(&out, 1)),
        [
            "Page 1",
            "Page 0.",
            "[media page-1-pic-1]",
            "[media page-1-pic-2]"
        ]
    );
    assert_eq!(
        asset(&out, "page-1-pic-1").alt.as_deref(),
        Some("small picture, box 250, 400 to 310, 440")
    );
}

#[test]
fn side_strips_and_the_band_edge_are_not_logo_places() {
    // At the left edge but in the middle of the height; and in the middle of the width, between
    // the bottom band (15 %) and the corner height (20 %).
    for (x, y, w, h) in [(10.0, 400.0, 30.0, 40.0), (250.0, 130.0, 60.0, 30.0)] {
        let out = read(&picture_pages(&[vec![(x, y, w, h, 0)]], &[gray(10)]));
        assert_eq!(needing(&out), ["page-1-pic-1"], "{x}, {y}");
    }
}

#[test]
fn a_note_on_a_turned_page_stays_at_the_page_beside_a_picture() {
    // A turned page puts its annotations at the page. The picture's point must not give the
    // note the picture's block.
    let content = "q 0 1 -1 0 595 0 cm\n\
                   BT /F1 12 Tf 72 500 Td (First line of the turned page.) Tj ET\n\
                   BT /F1 12 Tf 72 480 Td (Second line of the turned page.) Tj ET\n\
                   BT /F1 12 Tf 72 200 Td (Third line of the turned page.) Tj ET\nQ\n\
                   q 60 0 0 40 250 400 cm BI /W 1 /H 1 /CS /G /BPC 8 /F /AHx ID 80> EI Q\n";
    let note = "<< /Type /Annot /Subtype /Text /Rect [500 290 520 310] /Contents (Look.) >>";
    let file = common::pdf(
        &[common::Page {
            annots: vec![note.into()],
            ..common::Page::text(content)
        }],
        "",
    );
    let out = read(&file);
    assert_eq!(needing(&out), ["page-1-pic-1"]);
    let notes = &out.document.facets.annotations;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].anchor, Some(Anchor("p1".into())));
}

#[test]
fn small_pictures_not_listed_are_one_loss() {
    // 300 pages with 8 pictures each at a small memory limit: the kept bytes run out (and come
    // back in part as pages are laid out). One loss says how many pictures and on which pages; every picture listed before stays, with
    // its block.
    let limits = Limits {
        max_memory_bytes: 32 << 20,
        ..Limits::default()
    };
    let out = read_opts(&common::small_picture_pages(300, 8), false, limits);
    let cut: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.contains("not listed"))
        .collect();
    assert_eq!(cut.len(), 1, "{:#?}", out.document.losses);
    assert_eq!(cut[0].why, LossReason::Limit);
    let what = &cut[0].what;
    let count: usize = what
        .split(' ')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{what}"));
    assert!(what.contains(" small pictures on pages "), "{what}");
    assert!(
        what.contains(" not listed: kept text limit reached"),
        "{what}"
    );
    let media: usize = (1..=300)
        .map(|n| {
            lines(page(&out, n))
                .iter()
                .filter(|l| l.starts_with("[media "))
                .count()
        })
        .sum();
    assert!(media > 0, "{what}");
    assert_eq!(media + count, 2_400, "{what}");
    assert_eq!(out.document.assets.len(), media);
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn code_words_come_from_any_place_of_a_picture() {
    // A 1-bit picture of 21 by 21 pixels, drawn stretched on page 1 and square on page 2: the
    // square place looks like a code, so the asset says so.
    let dict = "/Type /XObject /Subtype /Image /Width 21 /Height 21 /ColorSpace /DeviceGray \
                /BitsPerComponent 1";
    let code = vec![(dict, vec![0x5au8; 63])];
    let draws = vec![
        vec![(250.0, 400.0, 85.0, 40.0, 0)],
        vec![(250.0, 300.0, 85.0, 85.0, 0)],
        vec![],
    ];
    let out = read(&picture_pages(&draws, &code));
    let alt = asset(&out, "page-1-pic-1").alt.clone().unwrap_or_default();
    assert!(
        alt.contains("barcode or QR code found, not decoded"),
        "{alt}"
    );
}

/// A tagged A4 page with one Figure (alt "A chart of sales.") on marked content 0. With
/// `on_chart` the marked content is the chart's draw, else a line of text. With `logo` a
/// second picture stands top left, outside the Figure.
fn tagged_figure_page(on_chart: bool, logo: bool) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let root = b.reserve();
    let figure = b.reserve();
    let page = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let picture = |b: &mut Builder, v: u8| {
        b.stream(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
             /BitsPerComponent 8",
            &[v; 16],
        )
    };
    let l = picture(&mut b, 30);
    let ch = picture(&mut b, 120);
    let chart = "q 60 0 0 40 250 400 cm /C Do Q\n";
    let mut c = String::new();
    if on_chart {
        c.push_str("BT /F1 12 Tf 72 700 Td (Sales.) Tj ET\n");
        c.push_str(&format!("/Figure <</MCID 0>> BDC {chart}EMC\n"));
    } else {
        c.push_str("/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (Sales.) Tj ET EMC\n");
        c.push_str(chart);
    }
    if logo {
        c.push_str("q 60 0 0 40 40 760 cm /L Do Q\n");
    }
    let cs = b.stream("", c.as_bytes());
    let parents = b.add(format!("<< /Nums [0 [{figure} 0 R]] >>"));
    b.set(
        figure,
        format!(
            "<< /Type /StructElem /S /Figure /P {root} 0 R /Pg {page} 0 R /K 0 \
             /Alt (A chart of sales.) >>"
        ),
    );
    b.set(
        root,
        format!("<< /Type /StructTreeRoot /K [{figure} 0 R] /ParentTree {parents} 0 R >>"),
    );
    b.set(
        page,
        format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /StructParents 0 \
             /Resources << /Font << /F1 {font} 0 R >> /XObject << /L {l} 0 R /C {ch} 0 R >> >> \
             /Contents {cs} 0 R >>"
        ),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"),
    );
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R /MarkInfo << /Marked true >> \
             /StructTreeRoot {root} 0 R >>"
        ),
    );
    b.finish(catalog)
}

/// The caption of the asset whose place box is `b`.
fn caption_at(out: &ReadOutcome, b: &str) -> Option<String> {
    out.document
        .assets
        .iter()
        .find(|a| a.alt.as_deref().is_some_and(|t| t.contains(b)))
        .unwrap_or_else(|| panic!("no asset at {b}: {:#?}", out.document.assets))
        .caption
        .clone()
}

#[test]
fn a_figure_caption_goes_to_its_own_picture() {
    let chart = "box 250, 400 to 310, 440";
    let logo = "box 40, 760 to 100, 800";
    // The Figure marks the chart's draw: the chart has the caption, the logo does not.
    let out = read(&tagged_figure_page(true, true));
    assert_eq!(
        caption_at(&out, chart).as_deref(),
        Some("A chart of sales.")
    );
    assert_eq!(caption_at(&out, logo), None);
    // The Figure marks no picture, and the page has two: neither gets it.
    let out = read(&tagged_figure_page(false, true));
    assert_eq!(caption_at(&out, chart), None);
    assert_eq!(caption_at(&out, logo), None);
    // The page has only one new small picture: it gets the page's Figure caption.
    let out = read(&tagged_figure_page(false, false));
    assert_eq!(
        caption_at(&out, chart).as_deref(),
        Some("A chart of sales.")
    );
}

#[test]
fn a_figure_matched_to_no_picture_stays_as_page_text() {
    // Two new small pictures and a Figure whose marked content is a line of text: the alt text
    // is written once as page text at the first picture, and given to no picture.
    let out = read(&tagged_figure_page(false, true));
    let shown = lines(page(&out, 1));
    let text = "Figure (not matched to a picture): A chart of sales.";
    assert_eq!(shown.iter().filter(|l| *l == text).count(), 1, "{shown:?}");
    let at = shown.iter().position(|l| l == text).unwrap();
    assert_eq!(shown[at + 1], "[media page-1-pic-1]", "{shown:?}");
    assert!(
        out.document.assets.iter().all(|a| a.caption.is_none()),
        "{:#?}",
        out.document.assets
    );
}

/// Tagged A4 pages: each with its content (which may use the pictures `/L`, a dark logo, and
/// `/C`, a gray chart) and its Figures, each an alt text on marked content id `k` (the k-th
/// Figure of the page, counted from 0).
fn tagged_pages(pages: &[(&str, &[&str])]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let root = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let picture = |b: &mut Builder, v: u8| {
        b.stream(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
             /BitsPerComponent 8",
            &[v; 16],
        )
    };
    let l = picture(&mut b, 30);
    let ch = picture(&mut b, 120);
    let mut kids = Vec::new();
    let mut elems = Vec::new();
    let mut nums = String::new();
    for (i, (content, figures)) in pages.iter().enumerate() {
        let page = b.reserve();
        let cs = b.stream("", content.as_bytes());
        b.set(
            page,
            format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /StructParents {i} \
                 /Resources << /Font << /F1 {font} 0 R >> /XObject << /L {l} 0 R /C {ch} 0 R >> \
                 >> /Contents {cs} 0 R >>"
            ),
        );
        let mut here = Vec::new();
        for (k, alt) in figures.iter().enumerate() {
            let e = b.add(format!(
                "<< /Type /StructElem /S /Figure /P {root} 0 R /Pg {page} 0 R /K {k} /Alt ({alt}) >>"
            ));
            here.push(format!("{e} 0 R"));
            elems.push(format!("{e} 0 R"));
        }
        nums.push_str(&format!("{i} [{}] ", here.join(" ")));
        kids.push(format!("{page} 0 R"));
    }
    let parents = b.add(format!("<< /Nums [{nums}] >>"));
    b.set(
        root,
        format!(
            "<< /Type /StructTreeRoot /K [{}] /ParentTree {parents} 0 R >>",
            elems.join(" ")
        ),
    );
    b.set(
        tree,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            kids.len()
        ),
    );
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R /MarkInfo << /Marked true >> \
             /StructTreeRoot {root} 0 R >>"
        ),
    );
    b.finish(catalog)
}

/// The "Figure (not matched to a picture)" paragraphs of a page, and what follows each.
fn unmatched(out: &ReadOutcome, n: u32) -> Vec<(String, Option<String>)> {
    let shown = lines(page(out, n));
    shown
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            l.strip_prefix("Figure (not matched to a picture): ")
                .map(|alt| (alt.to_string(), shown.get(i + 1).cloned()))
        })
        .collect()
}

#[test]
fn a_figure_on_vector_art_stays_when_another_figure_has_the_picture() {
    // Figure 0 marks a drawn chart (paths, no picture), Figure 1 the logo's draw.
    let content = "BT /F1 12 Tf 72 700 Td (Revenue.) Tj ET\n\
                   /Figure <</MCID 0>> BDC 200 300 100 80 re f EMC\n\
                   /Figure <</MCID 1>> BDC q 60 0 0 40 40 760 cm /L Do Q EMC\n";
    let out = read(&tagged_pages(&[(
        content,
        &["Revenue chart 2020", "Company logo"],
    )]));
    assert_eq!(
        asset(&out, "page-1-pic-1").caption.as_deref(),
        Some("Company logo")
    );
    assert_eq!(
        unmatched(&out, 1),
        [(
            "Revenue chart 2020".to_string(),
            Some("[media page-1-pic-1]".to_string())
        )]
    );
}

#[test]
fn a_figure_on_a_picture_seen_before_stays_at_its_place() {
    // The chart is new on page 1; on page 2 a Figure marks its draw, but the asset has no
    // caption to share: the alt text stays as page text at that place.
    let page1 = "BT /F1 12 Tf 72 700 Td (One.) Tj ET\nq 60 0 0 40 250 400 cm /C Do Q\n";
    let page2 = "BT /F1 12 Tf 72 700 Td (Two.) Tj ET\n\
                 /Figure <</MCID 0>> BDC q 60 0 0 40 250 300 cm /C Do Q EMC\n";
    let out = read(&tagged_pages(&[
        (page1, &[]),
        (page2, &["The chart again"]),
    ]));
    assert_eq!(asset(&out, "page-1-pic-1").caption, None);
    assert_eq!(
        unmatched(&out, 2),
        [(
            "The chart again".to_string(),
            Some("[media page-1-pic-1]".to_string())
        )]
    );
}

#[test]
fn a_second_unmatched_figure_stays_as_page_text() {
    // Two Figures mark text, the page has one new picture: the first alt text is its caption,
    // the second stays as page text at the picture.
    let content = "/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (First.) Tj ET EMC\n\
                   /P <</MCID 1>> BDC BT /F1 12 Tf 72 600 Td (Second.) Tj ET EMC\n\
                   q 60 0 0 40 250 400 cm /C Do Q\n";
    let out = read(&tagged_pages(&[(
        content,
        &["First figure", "Second figure"],
    )]));
    assert_eq!(
        asset(&out, "page-1-pic-1").caption.as_deref(),
        Some("First figure")
    );
    assert_eq!(
        unmatched(&out, 1),
        [(
            "Second figure".to_string(),
            Some("[media page-1-pic-1]".to_string())
        )]
    );
}

#[test]
fn a_figure_on_a_page_without_pictures_stays_at_the_page_end() {
    let content = "BT /F1 12 Tf 72 700 Td (Only text.) Tj ET\n\
                   /Figure <</MCID 0>> BDC 200 300 100 80 re f EMC\n";
    let out = read(&tagged_pages(&[(content, &["A drawn diagram"])]));
    assert!(out.document.assets.is_empty());
    assert_eq!(unmatched(&out, 1), [("A drawn diagram".to_string(), None)]);
}

// ---------------------------------------------------------------------------------------
// Barcodes and QR codes

use barcodes::Symbology;
use barcodes::encode::{self as code, Options};

const TICKET_URL: &str = "https://example.org/t/dNuKThV1";

/// The modules of a code with `text`.
fn modules(s: Symbology, text: &str) -> code::Modules {
    code::text(s, text, &Options::default()).expect("encode")
}

/// A gray picture of a code, `px` pixels a module and a quiet zone of `quiet` modules, as
/// dictionary and deflated data.
fn code_picture(m: &code::Modules, px: u32, quiet: u32) -> (String, Vec<u8>) {
    let (gray, w, h) = m.gray(px, quiet, 30);
    (
        format!(
            "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray \
             /BitsPerComponent 8 /Filter /FlateDecode"
        ),
        deflate(&gray),
    )
}

/// One A4 page with `content`, Helvetica as `/F1` and the pictures `pictures` as `/P0`, `/P1`.
fn code_page(content: &str, pictures: &[(String, Vec<u8>)]) -> Vec<u8> {
    code_pages(&[content], pictures)
}

/// A4 pages, each with its content, sharing the pictures `pictures` as `/P0`, `/P1`.
fn code_pages(contents: &[&str], pictures: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut xo = String::new();
    for (k, (dict, data)) in pictures.iter().enumerate() {
        let n = b.stream(dict, data);
        xo.push_str(&format!("/P{k} {n} 0 R "));
    }
    let mut kids = String::new();
    for content in contents {
        let cs = b.stream("", content.as_bytes());
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> /XObject << {xo}>> >> /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(
        tree,
        format!(
            "<< /Type /Pages /Kids [{kids}] /Count {} >>",
            contents.len()
        ),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// A code drawn as filled squares, one `re` for each dark module of `module` points, its
/// lower left corner at (`x`, `y`).
fn vector_code(m: &code::Modules, x: f32, y: f32, module: f32) -> String {
    let mut out = String::from("q 0 g\n");
    for row in 0..m.height {
        for col in 0..m.width {
            if m.dark[(row * m.width + col) as usize] {
                // Row 0 is the top row.
                let mx = x + col as f32 * module;
                let my = y + (m.height - 1 - row) as f32 * module;
                out += &format!("{mx:.3} {my:.3} {module:.3} {module:.3} re\n");
            }
        }
    }
    out + "f Q\n"
}

/// The payloads read from asset `id`, as symbology and text.
fn codes_of(out: &ReadOutcome, id: &str) -> Vec<(String, String)> {
    asset(out, id)
        .codes
        .iter()
        .map(|c| {
            (
                c.symbology.clone(),
                c.text.clone().or(c.base64.clone()).unwrap_or_default(),
            )
        })
        .collect()
}

const INVOICE: &str = "BT /F1 16 Tf 72 780 Td (Invoice 2026-114) Tj ET\n\
                       BT /F1 11 Tf 72 740 Td (Two chairs, 240.00 EUR) Tj ET\n\
                       BT /F1 11 Tf 72 720 Td (Pay within 14 days.) Tj ET\n";

#[test]
fn qr_in_invoice_corner_is_decoded_not_logo() {
    let m = modules(Symbology::QrCode, TICKET_URL);
    // A QR code picture in the bottom right corner, where a logo would be.
    let content = format!("{INVOICE}q 90 0 0 90 480 40 cm /P0 Do Q\n");
    let out = read_opts(
        &code_page(&content, &[code_picture(&m, 4, 4)]),
        true,
        Limits::default(),
    );
    assert_eq!(
        codes_of(&out, "page-1-pic-1"),
        [("QR Code".to_string(), TICKET_URL.to_string())]
    );
    let a = asset(&out, "page-1-pic-1");
    assert!(!a.needs_vision, "{a:#?}");
    assert_eq!(a.alt.as_deref(), Some("QR Code, box 480, 40 to 570, 130"));
    let md = markdown(&out);
    assert!(md.contains("QR Code, box 480, 40 to 570, 130"), "{md}");
    assert!(
        md.contains(&format!("QR Code: [{TICKET_URL}]({TICKET_URL})")),
        "{md}"
    );
    assert!(!md.contains("logo or small picture"), "{md}");
    assert!(!md.contains("Needs vision"), "{md}");
    assert!(!md.contains("Pictures that need vision"), "{md}");
}

#[test]
fn grey_qr_logo_place_becomes_code() {
    // An 8-bit gray QR code of one pixel a module, no quiet zone, in the bottom right corner:
    // Task 14d took it for a logo.
    let m = modules(Symbology::QrCode, "EPC QR test 4711");
    let (dict, data) = code_picture(&m, 1, 0);
    let side = m.width as f32 * 2.0;
    let content = format!("{INVOICE}q {side} 0 0 {side} 500 40 cm /P0 Do Q\n");
    let out = read(&code_page(&content, &[(dict, data)]));
    assert_eq!(
        codes_of(&out, "page-1-pic-1"),
        [("QR Code".to_string(), "EPC QR test 4711".to_string())]
    );
    let a = asset(&out, "page-1-pic-1");
    assert!(!a.needs_vision);
    assert!(
        a.alt
            .as_deref()
            .is_some_and(|t| t.starts_with("QR Code, box ")),
        "{a:#?}"
    );
    let md = markdown(&out);
    assert!(!md.contains("logo or small picture"), "{md}");
}

#[test]
fn vector_drawn_qr_is_decoded() {
    let m = modules(Symbology::QrCode, TICKET_URL);
    let content = format!("{INVOICE}{}", vector_code(&m, 380.0, 500.0, 2.5));
    let out = read_opts(&code_page(&content, &[]), true, Limits::default());
    assert_eq!(
        codes_of(&out, "page-1-code-1"),
        [("QR Code".to_string(), TICKET_URL.to_string())]
    );
    assert!(!asset(&out, "page-1-code-1").needs_vision);
    let shown = lines(page(&out, 1));
    assert!(
        shown.contains(&"[media page-1-code-1]".to_string()),
        "{shown:?}"
    );
    // Its crop is written.
    assert!(blob(&out, "page-1-code-1").starts_with(PNG_SIGNATURE));
}

#[test]
fn aztec_vector_ticket_pixel_aligned() {
    // A rail ticket's Aztec code drawn as paths at 1.2 points a module: 2.4 pixels at the
    // page's 144 dpi, which rxing cannot read; whole pixels a module can be.
    let m = modules(Symbology::Aztec, "TICKET 0042 FRA-MUC 2026-10-21 2nd class");
    let content = format!(
        "BT /F1 12 Tf 72 780 Td (Rail ticket) Tj ET\n{}",
        vector_code(&m, 330.3, 610.7, 1.2)
    );
    let out = read(&code_page(&content, &[]));
    assert_eq!(
        codes_of(&out, "page-1-code-1"),
        [(
            "Aztec".to_string(),
            "TICKET 0042 FRA-MUC 2026-10-21 2nd class".to_string()
        )]
    );
}

#[test]
fn undecodable_code_keeps_needs_vision() {
    // A 1-bit square picture that looks like a code but holds none.
    let content = "BT /F1 12 Tf 72 780 Td (Ticket.) Tj ET\nq 85 0 0 85 480 20 cm /Im1 Do Q\n";
    let file = image_page_bits(595, 842, content, 21, 21, 1, &[0x5au8; 63]);
    let out = read_opts(&file, true, Limits::default());
    assert_eq!(needing(&out), ["page-1-pic-1"]);
    let a = asset(&out, "page-1-pic-1");
    assert!(a.codes.is_empty());
    assert!(a.code_unread);
    assert_eq!(
        a.alt.as_deref(),
        Some("small picture, box 480, 20 to 565, 105, barcode or QR code found, not decoded")
    );
    let md = markdown(&out);
    assert!(md.contains("Needs vision"), "{md}");
    assert!(!md.contains("likely a barcode"), "{md}");
}

#[test]
fn shared_code_decoded_once_at_every_place() {
    // The same code picture on three pages, at three places (at one place it would be page
    // furniture): one asset, decoded, named a code at every place.
    let m = modules(Symbology::DataMatrix, "PARCEL 00340434161094042016");
    let at = |y: u32| format!("{INVOICE}q 60 0 0 60 260 {y} cm /P0 Do Q\n");
    let (a, b, c) = (at(400), at(300), at(200));
    let out = read(&code_pages(&[&a, &b, &c], &[code_picture(&m, 4, 2)]));
    let ids: Vec<&str> = out.document.assets.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["page-1-pic-1"]);
    assert_eq!(
        codes_of(&out, "page-1-pic-1"),
        [(
            "Data Matrix".to_string(),
            "PARCEL 00340434161094042016".to_string()
        )]
    );
    for (n, y) in [(1, 400), (2, 300), (3, 200)] {
        let alts: Vec<String> = page(&out, n)
            .iter()
            .filter_map(|b| match b {
                Block::Media { alt, .. } => alt.clone(),
                _ => None,
            })
            .collect();
        // The codes stand at the first place; later places point there.
        let more = if n == 1 {
            String::new()
        } else {
            ", its codes are listed at its first place on page 1".to_string()
        };
        assert_eq!(
            alts,
            [format!(
                "Data Matrix, box 260, {y} to 320, {}{more}",
                y + 60
            )],
            "page {n}"
        );
    }
}

#[test]
fn decode_without_output_dir() {
    let m = modules(Symbology::QrCode, TICKET_URL);
    let content = format!("{INVOICE}q 90 0 0 90 250 400 cm /P0 Do Q\n");
    let out = read(&code_page(&content, &[code_picture(&m, 4, 4)]));
    assert!(out.blobs.is_empty());
    assert_eq!(
        codes_of(&out, "page-1-pic-1"),
        [("QR Code".to_string(), TICKET_URL.to_string())]
    );
    assert!(needing(&out).is_empty());
    let md = markdown(&out);
    assert!(md.contains(&format!("QR Code: [{TICKET_URL}]")), "{md}");
}

#[test]
fn url_payload_is_link_and_escaped() {
    let tricky = "# not a heading *x* [y](z) <b>";
    let js = "javascript:alert(1)";
    let url = "https://example.org/a_b?c=*d*";
    let pictures: Vec<(String, Vec<u8>)> = [url, tricky, js]
        .iter()
        .map(|t| code_picture(&modules(Symbology::QrCode, t), 4, 4))
        .collect();
    let content = format!(
        "{INVOICE}q 50 0 0 50 72 400 cm /P0 Do Q\nq 50 0 0 50 252 400 cm /P1 Do Q\n\
         q 50 0 0 50 432 400 cm /P2 Do Q\n"
    );
    let out = read(&code_page(&content, &pictures));
    let md = markdown(&out);
    assert!(
        md.contains(r"QR Code: [https://example.org/a\_b?c=\*d\*](https://example.org/a_b?c=*d*)"),
        "{md}"
    );
    assert!(
        md.contains(r"QR Code: # not a heading \*x\* \[y\](z) &lt;b>"),
        "{md}"
    );
    // Only http and https become links.
    assert!(md.contains("QR Code: javascript:alert(1)"), "{md}");
    assert!(!md.contains("](javascript:"), "{md}");
    // The JSON keeps the payload as it is.
    assert_eq!(codes_of(&out, "page-1-pic-2")[0].1, tricky);
}

#[test]
fn binary_payload_is_base64() {
    let raw: Vec<u8> = (0u8..=255).rev().step_by(5).collect();
    let m = code::bytes(Symbology::QrCode, &raw, &Options::default()).expect("encode");
    let content = format!("{INVOICE}q 90 0 0 90 250 400 cm /P0 Do Q\n");
    let out = read(&code_page(&content, &[code_picture(&m, 4, 4)]));
    let c = &asset(&out, "page-1-pic-1").codes[0];
    assert_eq!(c.text, None);
    assert_eq!(c.bytes, raw.len() as u64);
    use base64::Engine;
    let back = base64::engine::general_purpose::STANDARD
        .decode(c.base64.as_deref().unwrap_or_default())
        .expect("base64");
    assert_eq!(back, raw);
    let md = markdown(&out);
    assert!(
        md.contains(&format!("QR Code: {} bytes, base64 `", raw.len())),
        "{md}"
    );
}

#[test]
fn code_on_a_scan_page_is_listed_with_the_page() {
    // A scanned page (one picture over all of it, no text) with a code in it: the page still
    // needs vision, and the code is read from it with its box on the page.
    let m = modules(Symbology::QrCode, TICKET_URL);
    let (qr, qw, qh) = m.gray(4, 4, 0);
    let (w, h) = (595u32, 842u32);
    let mut scan = vec![235u8; (w * h) as usize];
    for y in 0..qh {
        for x in 0..qw {
            scan[((y + 600) * w + x + 300) as usize] = qr[(y * qw + x) as usize];
        }
    }
    let content = "q 595 0 0 842 0 0 cm /Im1 Do Q\n";
    let out = read(&image_page(595, 842, content, w, h, &scan));
    assert_eq!(needing(&out), ["page-1"]);
    let a = asset(&out, "page-1");
    assert_eq!(
        codes_of(&out, "page-1"),
        [("QR Code".to_string(), TICKET_URL.to_string())]
    );
    // The code lies at x 300 to 432, y 842 - 732 = 110 to 242 points, about.
    let b = a.codes[0].bounds.expect("box");
    assert!(
        (b[0] - 300.0).abs() < 25.0 && (b[3] - 242.0).abs() < 25.0,
        "{b:?}"
    );
}

#[test]
fn figure_texts_stay_on_a_page_that_needs_vision_as_a_whole() {
    // The chart covers most of the page, so the page needs vision as a whole: the first
    // Figure is its caption, the second is written as page text.
    let content = "BT /F1 12 Tf 72 780 Td (Sales.) Tj ET\n\
                   /Figure << /MCID 0 >> BDC q 450 0 0 600 72 100 cm /C Do Q EMC\n";
    let out = read(&tagged_pages(&[(
        content,
        &["A chart of sales.", "A logo."],
    )]));
    assert_eq!(needing(&out), ["page-1"]);
    assert_eq!(
        asset(&out, "page-1").caption.as_deref(),
        Some("A chart of sales.")
    );
    let found = unmatched(&out, 1);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "A logo.");
}

#[test]
fn a_payload_of_several_lines_is_a_fenced_block() {
    let card = "BEGIN:VCARD\n# Name\n```\nN:Doe;Jane\nEND:VCARD";
    let content = format!("{INVOICE}q 60 0 0 60 250 400 cm /P0 Do Q\n");
    let out = read(&code_page(
        &content,
        &[code_picture(&modules(Symbology::QrCode, card), 3, 4)],
    ));
    assert_eq!(codes_of(&out, "page-1-pic-1")[0].1, card);
    let md = markdown(&out);
    assert!(
        md.contains("QR Code:\n\n````\nBEGIN:VCARD\n# Name\n```\nN:Doe;Jane\nEND:VCARD\n````"),
        "{md}"
    );
}

#[test]
fn a_small_code_on_a_very_large_page_is_read_at_a_fine_scale() {
    // A page of 14,400 by 14,400 points renders at 0.28 pixels a point: a code of 60 points
    // would be 17 pixels. Its crop is rendered again at 144 dpi.
    let m = modules(Symbology::QrCode, TICKET_URL);
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let (dict, data) = code_picture(&m, 4, 4);
    let image = b.stream(&dict, &data);
    let c = b.stream(
        "",
        b"BT /F1 200 Tf 300 14000 Td (A large plan.) Tj ET\nq 60 0 0 60 7000 7000 cm /P0 Do Q\n",
    );
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 14400 14400] \
         /Resources << /Font << /F1 {font} 0 R >> /XObject << /P0 {image} 0 R >> >> \
         /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read_opts(&b.finish(catalog), true, Limits::default());
    assert_eq!(
        codes_of(&out, "page-1-pic-1"),
        [("QR Code".to_string(), TICKET_URL.to_string())]
    );
    // The file is the crop at 144 dpi: 60 points and the margin.
    let (w, h) = png_size(blob(&out, "page-1-pic-1"));
    assert!(w >= 120 && h >= 120, "{w} by {h}");
}

#[test]
fn dense_vector_codes_are_found() {
    // Payment codes are often drawn as paths with small modules: a version 4 code at half a
    // point a module and a version 10 one at one point.
    let v4 = modules(Symbology::QrCode, &"epc 4711 ".repeat(6));
    let v10 = modules(
        Symbology::QrCode,
        &"spc 0200 1 ch4431999123000889012 ".repeat(7),
    );
    assert!(
        v4.width >= 33 && v10.width >= 57,
        "{} {}",
        v4.width,
        v10.width
    );
    for (m, module) in [(&v4, 0.5), (&v10, 1.0)] {
        let content = format!("{INVOICE}{}", vector_code(m, 300.0, 300.0, module));
        let out = read(&code_page(&content, &[]));
        let found: Vec<usize> = out.document.assets.iter().map(|a| a.codes.len()).collect();
        assert_eq!(
            found,
            [1],
            "{} modules at {module}: {:#?}",
            m.width,
            out.document.assets
        );
    }
}

// ---------------------------------------------------------------------------------------
// Fix round 1

/// `m` with its data modules flipped in a band through the middle (the finders kept): it
/// keeps the look of a code and no longer decodes.
fn spoiled(m: &code::Modules) -> code::Modules {
    let mut out = m.clone();
    let n = m.width as usize;
    // Every module outside the three finder corners flipped in a checker pattern.
    for r in 0..n {
        for c in 0..n {
            let corner = (r < 8 && c < 8) || (r < 8 && c >= n - 8) || (r >= n - 8 && c < 8);
            if !corner && (r + c) % 2 == 0 {
                out.dark[r * n + c] = !out.dark[r * n + c];
            }
        }
    }
    out
}

#[test]
fn a_drawing_that_looks_like_a_code_but_gives_none_is_listed() {
    let m = spoiled(&modules(Symbology::QrCode, TICKET_URL));
    let content = format!("{INVOICE}{}", vector_code(&m, 300.0, 400.0, 2.0));
    let out = read_opts(&code_page(&content, &[]), true, Limits::default());
    let a = asset(&out, "page-1-code-1");
    assert!(a.codes.is_empty());
    assert!(a.needs_vision);
    assert!(a.code_unread);
    assert_eq!(
        a.alt.as_deref(),
        Some(
            "a drawing that looks like a barcode or QR code, box 300, 400 to 350, 450, not decoded"
        )
    );
    assert!(blob(&out, "page-1-code-1").starts_with(PNG_SIGNATURE));
    let md = markdown(&out);
    assert!(md.contains("Needs vision"), "{md}");
}

/// A page with `content` drawn in one path filled with `color` (`0 g` for black).
fn drawing_page(content: &str) -> Vec<u8> {
    code_page(&format!("{INVOICE}{content}"), &[])
}

#[test]
fn drawings_that_are_not_codes_are_not_listed() {
    let cells = |list: Vec<(f32, f32, f32, f32)>, color: &str| {
        let mut c = format!("q {color}\n");
        for (x, y, w, h) in list {
            c += &format!("{x} {y} {w} {h} re\n");
        }
        c + "f Q\n"
    };
    let mut cases: Vec<(&str, String)> = Vec::new();
    // A ruled table: thin lines as filled rectangles.
    let mut rules: Vec<(f32, f32, f32, f32)> = (0..11)
        .map(|k| (50.0 + k as f32 * 25.0, 100.0, 0.5, 150.0))
        .collect();
    rules.extend((0..31).map(|k| (50.0, 100.0 + k as f32 * 5.0, 250.0, 0.5)));
    cases.push(("ruled table", cells(rules, "0 g")));
    // A checkerboard of 12 by 12 cells of 10 points.
    let checker: Vec<(f32, f32, f32, f32)> = (0..144)
        .filter(|k| (k / 12 + k % 12) % 2 == 0)
        .map(|k| {
            (
                50.0 + (k % 12) as f32 * 10.0,
                100.0 + (k / 12) as f32 * 10.0,
                10.0,
                10.0,
            )
        })
        .collect();
    cases.push(("checkerboard", cells(checker.clone(), "0 g")));
    // The same in white.
    cases.push(("white squares", cells(checker, "1 g")));
    // Zebra rows of a table.
    cases.push((
        "zebra rows",
        cells(
            (0..20)
                .map(|k| (50.0, 100.0 + k as f32 * 24.0, 480.0, 12.0))
                .collect(),
            "0.9 g",
        ),
    ));
    // A heatmap of 20 by 20 cells of 8 points, half filled.
    let mut state = 11u32;
    let heat: Vec<(f32, f32, f32, f32)> = (0..400)
        .filter(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.is_multiple_of(2)
        })
        .map(|k| {
            (
                50.0 + (k % 20) as f32 * 8.0,
                100.0 + (k / 20) as f32 * 8.0,
                8.0,
                8.0,
            )
        })
        .collect();
    cases.push(("heatmap", cells(heat, "0.2 g")));
    // A bar chart and 40 equal bars.
    cases.push((
        "bar chart",
        cells(
            (0..30)
                .map(|k| {
                    (
                        50.0 + k as f32 * 6.0,
                        100.0,
                        4.0,
                        10.0 + (k * 37 % 50) as f32,
                    )
                })
                .collect(),
            "0 g",
        ),
    ));
    cases.push((
        "equal bars",
        cells(
            (0..40)
                .map(|k| (50.0 + k as f32 * 2.0, 100.0, 1.0, 40.0))
                .collect(),
            "0 g",
        ),
    ));
    for (name, content) in cases {
        let out = read(&drawing_page(&content));
        assert!(
            out.document.assets.is_empty(),
            "{name}: {:#?}",
            out.document.assets
        );
    }
}

#[test]
fn vertical_code_128_is_read() {
    // A Code 128 picture turned a quarter by its matrix, and the same code drawn as bars lying
    // down.
    let m = modules(Symbology::Code128, "RZ-2026-1021");
    let (dict, data) = code_picture(&m, 3, 10);
    let content = format!("{INVOICE}q 0 200 -60 0 300 300 cm /P0 Do Q\n");
    let out = read(&code_page(&content, &[(dict, data)]));
    let got: Vec<(String, String)> = out
        .document
        .assets
        .iter()
        .flat_map(|a| {
            a.codes
                .iter()
                .map(|c| (c.symbology.clone(), c.text.clone().unwrap_or_default()))
        })
        .collect();
    assert_eq!(
        got,
        [("Code 128".to_string(), "RZ-2026-1021".to_string())],
        "picture"
    );
    let mut bars = String::from("q 0 g\n");
    for col in 0..m.width {
        if m.dark[col as usize] {
            bars += &format!("200 {:.2} 40 0.8 re\n", 300.0 + col as f32 * 0.8);
        }
    }
    bars += "f Q\n";
    let out = read(&drawing_page(&bars));
    assert_eq!(
        codes_of(&out, "page-1-code-1"),
        [("Code 128".to_string(), "RZ-2026-1021".to_string())],
        "paths"
    );
}

#[test]
fn pdf417_drawn_as_paths_is_read() {
    // A boarding pass code drawn at 0.9 points a module, rows 3 modules high.
    const BCBP: &str = "M1DOE/JOHN            EABC123 FRAJFKLH 0400 290Y012A0012 100";
    let m = modules(Symbology::Pdf417, BCBP);
    let module = 0.9f32;
    let mut c = String::from("q 0 g\n");
    for row in 0..m.height {
        // Runs of dark modules in a row, as rectangles.
        let mut col = 0;
        while col < m.width {
            if m.dark[(row * m.width + col) as usize] {
                let start = col;
                while col < m.width && m.dark[(row * m.width + col) as usize] {
                    col += 1;
                }
                let x = 100.0 + start as f32 * module;
                let y = 500.0 + (m.height - 1 - row) as f32 * module;
                c += &format!(
                    "{x:.3} {y:.3} {:.3} {module:.3} re\n",
                    (col - start) as f32 * module
                );
            } else {
                col += 1;
            }
        }
    }
    c += "f Q\n";
    let out = read(&drawing_page(&c));
    assert_eq!(
        codes_of(&out, "page-1-code-1"),
        [("PDF417".to_string(), BCBP.to_string())]
    );
}

#[test]
fn a_code_in_the_page_furniture_is_named_once() {
    // The same Data Matrix at the same place on four pages: page furniture, read once and
    // named once, at its first place, with its pages.
    let m = modules(Symbology::DataMatrix, "DOC-ID 2026-114");
    let content = format!("{INVOICE}q 40 0 0 40 520 20 cm /P0 Do Q\n");
    let out = read(&code_pages(
        &[&content, &content, &content, &content],
        &[code_picture(&m, 4, 2)],
    ));
    let ids: Vec<&str> = out.document.assets.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["page-1-code-1"]);
    assert_eq!(
        codes_of(&out, "page-1-code-1"),
        [("Data Matrix".to_string(), "DOC-ID 2026-114".to_string())]
    );
    let media: Vec<String> = (1..=4)
        .flat_map(|n| lines(page(&out, n)))
        .filter(|l| l.starts_with("[media"))
        .collect();
    assert_eq!(media, ["[media page-1-code-1]"]);
    let alt = page(&out, 1)
        .iter()
        .find_map(|b| match b {
            Block::Media { alt, .. } => alt.clone(),
            _ => None,
        })
        .unwrap_or_default();
    assert_eq!(
        alt,
        "Data Matrix, box 520, 20 to 560, 60, on pages 1, 2, 3, 4"
    );
}

#[test]
fn a_small_8_bit_code_finer_than_the_page_render_is_read_again() {
    // A version 10 QR code of one pixel a module drawn at 30 points: 1 pixel a module at the
    // page's 144 dpi, which does not read; at its own resolution three times, it does.
    let m = modules(
        Symbology::QrCode,
        &"spc 0200 1 ch4431999123000889012 ".repeat(7),
    );
    let (dict, data) = code_picture(&m, 1, 0);
    let content = format!("{INVOICE}q 30 0 0 30 300 400 cm /P0 Do Q\n");
    let out = read(&code_page(&content, &[(dict, data)]));
    assert_eq!(
        codes_of(&out, "page-1-pic-1").len(),
        1,
        "{:#?}",
        out.document.assets
    );
}

#[test]
fn many_codes_in_one_picture_stop_at_the_picture_cap() {
    // A page picture with 60 different QR codes: 50 are kept, one loss says so.
    let codes: Vec<(Vec<u8>, u32, u32)> = (0..60)
        .map(|k| modules(Symbology::QrCode, &format!("CODE-{k:02}")).gray(3, 4, 0))
        .collect();
    let (cw, ch) = (codes[0].1, codes[0].2);
    let (w, h) = (cw * 6, ch * 10);
    let mut sheet = vec![255u8; (w * h) as usize];
    for (k, (g, gw, gh)) in codes.iter().enumerate() {
        let (ox, oy) = ((k as u32 % 6) * cw, (k as u32 / 6) * ch);
        for row in 0..*gh {
            let to = ((oy + row) * w + ox) as usize;
            sheet[to..to + *gw as usize]
                .copy_from_slice(&g[(row * gw) as usize..((row + 1) * gw) as usize]);
        }
    }
    let content = "q 595 0 0 842 0 0 cm /Im1 Do Q\n";
    let out = read(&image_page(595, 842, content, w, h, &sheet));
    let a = asset(&out, "page-1");
    assert_eq!(a.codes.len(), 50, "{:#?}", out.document.losses);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what.contains("hold more than 50 codes each") && l.why == LossReason::Limit),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn page_furniture_does_not_move_figure_texts() {
    // A logo at one place on three pages is page furniture. The Figure of page 2 marks text,
    // not a picture: its alt text stays at the end of the page, as without the logo.
    let page = |n: u32| {
        format!(
            "q 30 0 0 30 500 790 cm /L Do Q\n\
             BT /F1 12 Tf 72 700 Td (Body of page {n}.) Tj ET\n\
             /P << /MCID 0 >> BDC BT /F1 12 Tf 72 650 Td (A note.) Tj ET EMC\n"
        )
    };
    let (p1, p2, p3) = (page(1), page(2), page(3));
    let out = read(&tagged_pages(&[
        (&p1, &[]),
        (&p2, &["A chart of sales."]),
        (&p3, &[]),
    ]));
    let shown = lines(page_blocks(&out, 2));
    assert_eq!(
        shown.last().map(String::as_str),
        Some("Figure (not matched to a picture): A chart of sales."),
        "{shown:?}"
    );
    assert!(out.document.assets.is_empty(), "{:#?}", out.document.assets);
}

fn page_blocks(out: &ReadOutcome, n: u32) -> &[Block] {
    page(out, n)
}

#[test]
fn vector_codes_in_any_color_are_read() {
    // Orange, green and light grey modules, and white modules on a black square.
    let m = modules(Symbology::QrCode, TICKET_URL);
    let colored = |color: &str| {
        vector_code(&m, 300.0, 400.0, 2.0).replacen("q 0 g", &format!("q {color}"), 1)
    };
    // An inverted code: its dark modules white on a black square with a black quiet zone.
    let mut white = String::from("q 0 g 290 390 70 70 re f Q\n");
    white += &vector_code(&m, 300.0, 400.0, 2.0).replacen("q 0 g", "q 1 g", 1);
    for (name, content) in [
        ("orange", colored("1 0.6 0 rg")),
        ("green", colored("0.2 0.8 0.3 rg")),
        ("light grey", colored("0.65 g")),
        ("white on black", white),
    ] {
        let out = read(&drawing_page(&content));
        let got: Vec<String> = out
            .document
            .assets
            .iter()
            .flat_map(|a| a.codes.iter().map(|c| c.text.clone().unwrap_or_default()))
            .collect();
        assert_eq!(
            got,
            [TICKET_URL.to_string()],
            "{name}: {:#?}",
            out.document.assets
        );
    }
}
