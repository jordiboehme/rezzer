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
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let image = b.stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceGray \
             /BitsPerComponent 8 /Filter /FlateDecode"
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
    assert!(loss.what.contains("256000000"), "{loss:?}");
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn render_room_gives_limit_loss() {
    // Noise does not pack: the PNG of the page is larger than the eighth of the memory limit
    // that extracted pictures may take.
    let mut state = 12345u32;
    let noise: Vec<u8> = (0..1000 * 1000)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (state >> 16) as u8
        })
        .collect();
    let file = image_page(
        595,
        842,
        "q 595 0 0 842 0 0 cm /Im1 Do Q\n",
        1000,
        1000,
        &noise,
    );
    let limits = Limits {
        max_memory_bytes: 8 << 20,
        ..Limits::default()
    };
    let out = read_opts(&file, true, limits);
    assert!(out.blobs.is_empty());
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
    // Task 11's carry: a FlateDecode content stream that does not unpack in full.
    let text: String = (0..4000)
        .map(|i| format!("BT /F1 9 Tf 72 700 Td ({i}) Tj ET\n"))
        .collect();
    let mut packed = deflate(text.as_bytes());
    let mid = packed.len() / 2;
    for b in &mut packed[mid..mid + 64] {
        *b = 0xFF;
    }
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
