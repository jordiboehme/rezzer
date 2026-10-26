// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small files that try to make a page render, or the text parser on a damaged stream, take
//! more memory than the read may use. Each runs in its own process under nextest, so a file
//! that made the process abort fails only its own test.

mod common;

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Builder, lenient_bomb};
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Block, Bytes, Completeness, Confidence, Detection, FormatId, Loss, LossReason, ReadOptions,
    ReadOutcome, Reader, Severity, Source, plain_text,
};
use rezzer_read_pdf::PdfReader;

/// The allocator of the Rezzer binary, counted: the memory these tests measure is the memory
/// it ships with. Most bounds here hold the resident set size; the test at a low memory limit
/// holds the heap peak, as the resident set size varies by a few MiB between runs and systems
/// and that limit leaves no room for it (see the memory tests in `hostile.rs`).
#[global_allocator]
static ALLOCATOR: common::Counting = common::Counting;

/// The brief's hostile setting: 512 MiB of memory.
fn hostile() -> Limits {
    Limits {
        max_memory_bytes: 512 << 20,
        max_time: Duration::from_secs(120),
        ..Limits::default()
    }
}

fn read_with(bytes: &[u8], extract: bool) -> ReadOutcome {
    let src = Source {
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
    };
    let opts = ReadOptions {
        extract_assets: extract,
        ..ReadOptions::default()
    };
    PdfReader.read(&src, &opts, &Budget::new(hostile()))
}

fn read_with_limits(bytes: &[u8], extract: bool, limits: Limits) -> ReadOutcome {
    let src = Source {
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
    };
    let opts = ReadOptions {
        extract_assets: extract,
        ..ReadOptions::default()
    };
    PdfReader.read(&src, &opts, &Budget::new(limits))
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(data).expect("deflate");
    z.finish().expect("deflate")
}

/// The bound the memory tests hold: a quarter of the hostile memory limit.
fn assert_rss_under_256_mib() {
    let rss = common::max_rss_bytes();
    assert!(rss < 256 << 20, "max RSS {} MiB", rss >> 20);
}

/// More entries of a twin: in its `/XObject` resources, in its other resources, in the page.
#[derive(Default)]
struct Extra {
    xobjects: String,
    resources: String,
    page: String,
}

/// A one-page file of `w` by `h` points that first draws a small gray picture over the whole
/// page, so the page needs vision, then `content`. `build` adds objects and returns more
/// entries.
fn render_twin(
    w: u32,
    h: u32,
    content: &str,
    build: impl FnOnce(&mut Builder) -> Extra,
) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /Filter /FlateDecode",
        &deflate(&[90u8; 64]),
    );
    let Extra {
        xobjects,
        resources,
        page: page_extra,
    } = build(&mut b);
    let c = b.stream(
        "",
        format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q\n{content}").as_bytes(),
    );
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] \
         /Resources << /XObject << /Im0 {image} 0 R {xobjects} >> {resources} >> \
         /Contents {c} 0 R \
         {page_extra} >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// The render of page 1 was left out with a Limit loss, the read finished in time and stayed
/// small. Returns the loss.
fn assert_render_skipped(out: &ReadOutcome, start: Instant) -> &Loss {
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert!(
        out.document
            .assets
            .iter()
            .any(|a| a.id == "page-1" && a.needs_vision),
        "{:#?}",
        out.document.assets
    );
    assert!(out.blobs.is_empty());
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| {
            l.location == "p1" && l.why == LossReason::Limit && l.what.contains("not rendered")
        })
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_rss_under_256_mib();
    loss
}

/// A colored tiling pattern `/P1` with a cell of one point and a step of 100,000 points: the
/// renderer would size its cell pixmap from the step.
fn huge_step_pattern(b: &mut Builder) -> u32 {
    b.stream(
        "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 1 1] /XStep 100000 \
         /YStep 100000 /Resources << >>",
        b"0 0 1 1 re f",
    )
}

const PATTERN_FILL: &str = "/Pattern cs /P1 scn 0 0 300 300 re f\n";

#[test]
fn a_tiling_pattern_with_a_huge_step_is_not_rendered() {
    let start = Instant::now();
    let file = render_twin(595, 842, PATTERN_FILL, |b| {
        let p = huge_step_pattern(b);
        Extra {
            resources: format!("/Pattern << /P1 {p} 0 R >>"),
            ..Extra::default()
        }
    });
    let out = read_with(&file, true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("pattern"), "{loss:?}");
}

#[test]
fn a_tiling_pattern_with_a_huge_step_in_a_form_is_not_rendered() {
    let start = Instant::now();
    let file = render_twin(595, 842, "/Fm1 Do\n", |b| {
        let p = huge_step_pattern(b);
        let form = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] \
                 /Resources << /Pattern << /P1 {p} 0 R >> >>"
            ),
            PATTERN_FILL.as_bytes(),
        );
        Extra {
            xobjects: format!("/Fm1 {form} 0 R"),
            ..Extra::default()
        }
    });
    let out = read_with(&file, true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("pattern"), "{loss:?}");
}

#[test]
fn a_tiling_pattern_with_a_huge_step_in_an_annotation_is_not_rendered() {
    let start = Instant::now();
    let file = render_twin(595, 842, "", |b| {
        let p = huge_step_pattern(b);
        let ap = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 300 300] \
                 /Resources << /Pattern << /P1 {p} 0 R >> >>"
            ),
            PATTERN_FILL.as_bytes(),
        );
        let annot = b.add(format!(
            "<< /Type /Annot /Subtype /Square /Rect [0 0 300 300] /AP << /N {ap} 0 R >> >>"
        ));
        Extra {
            page: format!("/Annots [{annot} 0 R]"),
            ..Extra::default()
        }
    });
    let out = read_with(&file, true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("pattern"), "{loss:?}");
}

/// The start of a baseline JPEG whose frame is `w` by `h` pixels, then bytes that are no
/// image data.
fn jpeg_head(w: u16, h: u16) -> Vec<u8> {
    let mut j = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
    j.extend(h.to_be_bytes());
    j.extend(w.to_be_bytes());
    j.extend([3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1]);
    j.extend([0u8; 64]);
    j
}

/// A page whose picture `/Im1` is a DCT image stated as `dw` by `dh` with a frame of `w` by
/// `h`.
fn dct_twin(dw: u32, dh: u32, w: u16, h: u16) -> Vec<u8> {
    render_twin(595, 842, "q 595 0 0 842 0 0 cm /Im1 Do Q\n", |b| {
        let image = b.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {dw} /Height {dh} \
                 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"
            ),
            &jpeg_head(w, h),
        );
        Extra {
            xobjects: format!("/Im1 {image} 0 R"),
            ..Extra::default()
        }
    })
}

#[test]
fn a_dct_picture_of_15000_by_15000_is_not_rendered() {
    let start = Instant::now();
    let out = read_with(&dct_twin(15_000, 15_000, 15_000, 15_000), true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("pictures"), "{loss:?}");
}

#[test]
fn a_dct_picture_whose_dictionary_is_smaller_than_its_frame_is_not_rendered() {
    // The dictionary says 100 by 100; the JPEG frame says 15,000 by 15,000.
    let start = Instant::now();
    let out = read_with(&dct_twin(100, 100, 15_000, 15_000), true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("pictures"), "{loss:?}");
}

#[test]
fn many_soft_masks_are_not_rendered() {
    // A large page (rendered at 16,000,000 pixels) with 40 different soft masks: each one is
    // a mask of the page's size.
    let start = Instant::now();
    let mut content = String::new();
    for i in 0..40 {
        content.push_str(&format!("/GS{i} gs 0 0 1 rg 0 0 100 100 re f\n"));
    }
    let file = render_twin(2000, 2000, &content, |b| {
        let mut states = String::new();
        for i in 0..40 {
            let group = b.stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 2000 2000] \
                 /Group << /S /Transparency /CS /DeviceGray >>",
                format!("{} g 0 0 2000 2000 re f", f32::from(i as u8) / 40.0).as_bytes(),
            );
            let gs = b.add(format!(
                "<< /Type /ExtGState /SMask << /Type /Mask /S /Luminosity /G {group} 0 R >> >>"
            ));
            states.push_str(&format!("/GS{i} {gs} 0 R "));
        }
        Extra {
            resources: format!("/ExtGState << {states} >>"),
            ..Extra::default()
        }
    });
    let out = read_with(&file, true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("soft mask"), "{loss:?}");
}

#[test]
fn a_large_inline_picture_is_not_rendered() {
    // 20,000 by 20,000 one-bit pixels from 8 bytes of data: the decoder builds a byte per
    // pixel.
    let start = Instant::now();
    let content = "q 595 0 0 842 0 0 cm BI /W 20000 /H 20000 /BPC 1 /CS /G ID \
                   \x00\x00\x00\x00\x00\x00\x00\x00 EI Q\n";
    let out = read_with(&render_twin(595, 842, content, |_| Extra::default()), true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("pictures"), "{loss:?}");
}

#[test]
fn a_picture_under_the_old_pixel_cap_that_takes_too_much_is_not_rendered() {
    // 15,000 by 15,000 one-bit pixels (225,000,000, under the brief's 256,000,000) from one
    // row of data.
    let start = Instant::now();
    let out = read_with(
        &render_twin(595, 842, "q 595 0 0 842 0 0 cm /Im1 Do Q\n", |b| {
            let image = b.stream(
                "/Type /XObject /Subtype /Image /Width 15000 /Height 15000 \
                 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /FlateDecode",
                &deflate(&[0u8; 1875]),
            );
            Extra {
                xobjects: format!("/Im1 {image} 0 R"),
                ..Extra::default()
            }
        }),
        true,
    );
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("pictures"), "{loss:?}");
}

/// A page with Helvetica as `/F1` whose content stream is `packed`, given as FlateDecode.
fn flate_content_page(packed: &[u8]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let c = b.stream("/Filter /FlateDecode", packed);
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// 1,200,000 copies of 258 bytes (about 310 MB) after a copy from before the start.
const BOMB_RUNS: usize = 1_200_000;

#[test]
fn a_damaged_content_stream_is_read_only_as_far_as_measured() {
    // Inflate stops at the first copy; a lenient decoder would go on to 310 MB.
    let start = Instant::now();
    let out = read_with(&flate_content_page(&lenient_bomb(BOMB_RUNS)), false);
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.location == "p1" && l.why == LossReason::Damaged),
        "{:#?}",
        out.document.losses
    );
    assert_eq!(out.completeness, Completeness::Partial);
    assert_rss_under_256_mib();
}

fn fallback_not_used(out: &ReadOutcome) -> &Loss {
    out.document
        .losses
        .iter()
        .find(|l| l.what.contains("fallback parser was not used"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses))
}

#[test]
fn a_damaged_stream_keeps_the_fallback_out() {
    let start = Instant::now();
    let mut file = vec![b' '; 2000];
    file.extend(flate_content_page(&lenient_bomb(BOMB_RUNS)));
    let out = read_with(&file, false);
    assert!(start.elapsed() < Duration::from_secs(60));
    // The fallback reads the file, but not the page that uses the damaged stream.
    assert!(
        out.document.losses.iter().any(|l| l.location == "p1"
            && l.why == LossReason::Damaged
            && l.what.contains("stops with a decoding error")),
        "{:#?}",
        out.document.losses
    );
    assert_eq!(out.completeness, Completeness::Partial);
    assert_rss_under_256_mib();
}

#[test]
fn a_stream_the_fallback_parser_reads_further_is_measured_that_far() {
    // A stored block holds the bytes `endstream`, so a search for the end of the data stops
    // inside it; after the data comes `endstreamX`, which only a parser that takes /Length and
    // a prefix of `endstream` accepts. The rest of the data unpacks to 310 MB.
    let mut d = common::Deflate::zlib();
    d.stored(false, b"\nendstream\n");
    d.fixed_block(true);
    d.symbol(u32::from(b' '));
    for _ in 0..BOMB_RUNS {
        d.run();
    }
    d.symbol(256);
    let data = d.finish();
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut body = format!(
        "<< /Filter /FlateDecode /Length {} >>\nstream\n",
        data.len()
    )
    .into_bytes();
    body.extend(&data);
    body.extend(b"\nendstreamX");
    let c = b.add(body);
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let mut file = vec![b' '; 2000];
    file.extend(b.finish(catalog));
    let start = Instant::now();
    let out = read_with(&file, false);
    assert!(start.elapsed() < Duration::from_secs(60));
    fallback_not_used(&out);
    assert_rss_under_256_mib();
}

/// The text of page 1.
fn page_one(out: &ReadOutcome) -> String {
    out.document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section { blocks, .. } => Some(
                blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::Paragraph { inlines, .. } => Some(plain_text(inlines)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn a_damaged_content_stream_keeps_its_measured_text() {
    // Text, then a copy from before the start: the text before the damage is still read.
    let mut d = common::Deflate::zlib();
    d.fixed_block(true);
    for c in b"BT /F1 12 Tf 72 700 Td (Kept.) Tj ET\n" {
        d.symbol(u32::from(*c));
    }
    d.far_copy();
    d.symbol(256);
    let out = read_with(&flate_content_page(&d.finish()), false);
    assert!(page_one(&out).contains("Kept."), "{:#?}", out.document);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.location == "p1" && l.why == LossReason::Damaged),
        "{:#?}",
        out.document.losses
    );
}

/// An A4 page scanned at `dpi`, gray (`color` false) or RGB, as one Flate picture over the
/// whole page. Rows of noise repeat a few times, so the file stays small and the stream
/// under the guard's ratio cap.
fn scan_twin(dpi: u32, color: bool) -> Vec<u8> {
    let (w, h) = (dpi * 827 / 100, dpi * 1169 / 100);
    let comps = if color { 3 } else { 1 };
    let row = (w * comps) as usize;
    let repeat = (32_000 / row).clamp(1, 50);
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut state = 7u32;
    let mut line = vec![0u8; row];
    for y in 0..h as usize {
        if y % repeat == 0 {
            for b in &mut line {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
                *b = (state >> 16) as u8;
            }
        }
        z.write_all(&line).expect("deflate");
    }
    let packed = z.finish().expect("deflate");
    let cs = if color { "/DeviceRGB" } else { "/DeviceGray" };
    render_twin(595, 842, "q 595 0 0 842 0 0 cm /Im1 Do Q\n", |b| {
        let image = b.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace {cs} \
                 /BitsPerComponent 8 /Filter /FlateDecode"
            ),
            &packed,
        );
        Extra {
            xobjects: format!("/Im1 {image} 0 R"),
            ..Extra::default()
        }
    })
}

/// Page 1 was rendered, within the memory bound.
fn assert_rendered(out: &ReadOutcome) {
    assert!(
        out.blobs.iter().any(|b| b.asset == "page-1"),
        "{:#?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

#[test]
fn a_300_dpi_color_scan_renders() {
    assert_rendered(&read_with(&scan_twin(300, true), true));
}

#[test]
fn a_600_dpi_gray_scan_renders() {
    assert_rendered(&read_with(&scan_twin(600, false), true));
}

#[test]
fn the_scanned_letter_renders() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf/scan-truman-letter-nara.pdf");
    assert_rendered(&read_with(&std::fs::read(path).expect("corpus file"), true));
}

/// A page that needs vision and then fills the whole page `fills` times: in the page's own
/// content, or (`form`) through one form of 100 fills drawn `fills / 100` times.
fn fill_twin(fills: usize, form: bool) -> Vec<u8> {
    let fill = "0 0 595 842 re f\n";
    let content = if form {
        "/Fm1 Do\n".repeat(fills / 100)
    } else {
        fill.repeat(fills)
    };
    let packed = deflate(content.as_bytes());
    render_twin(595, 842, "/Fill Do\n", |b| {
        let body = b.stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /Filter /FlateDecode",
            &packed,
        );
        let mut xobjects = format!("/Fill {body} 0 R");
        if form {
            let fm = b.stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /Filter /FlateDecode",
                &deflate(fill.repeat(100).as_bytes()),
            );
            // The inner form is named in the page's resources, which the outer form inherits
            // through its own resources.
            xobjects.push_str(&format!(" /Fm1 {fm} 0 R"));
        }
        Extra {
            xobjects,
            ..Extra::default()
        }
    })
}

/// The twin read with `extract_assets`: the read ends in time, stays small, and either
/// renders or says why not.
fn assert_fills_bounded(file: &[u8]) {
    let start = Instant::now();
    let out = read_with(file, true);
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let rendered = out.blobs.iter().any(|b| b.asset == "page-1");
    let said = out.document.losses.iter().any(|l| {
        l.location == "p1" && l.why == LossReason::Limit && l.what.contains("not rendered")
    });
    assert!(rendered || said, "{:#?}", out.document.losses);
    assert_rss_under_256_mib();
}

#[test]
fn five_thousand_page_fills_stay_small() {
    assert_fills_bounded(&fill_twin(5_000, false));
}

#[test]
fn fifty_thousand_page_fills_stay_small() {
    assert_fills_bounded(&fill_twin(50_000, false));
}

#[test]
fn two_hundred_thousand_page_fills_stay_small() {
    assert_fills_bounded(&fill_twin(200_000, false));
}

#[test]
fn fifty_thousand_page_fills_through_a_form_stay_small() {
    assert_fills_bounded(&fill_twin(50_000, true));
}

#[test]
fn a_large_stream_the_render_does_not_use_is_not_unpacked() {
    // 300 MB of rows of noise, each repeated 30 times, in a stream the page names but no
    // render draws.
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut state = 3u32;
    let mut row = vec![0u8; 1000];
    for i in 0..300_000 {
        if i % 30 == 0 {
            for b in &mut row {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
                *b = (state >> 16) as u8;
            }
        }
        z.write_all(&row).expect("deflate");
    }
    let packed = z.finish().expect("deflate");
    let file = render_twin(595, 842, "", |b| {
        let big = b.stream("/Type /EmbeddedFile /Filter /FlateDecode", &packed);
        Extra {
            page: format!("/PieceInfo << /Data {big} 0 R >>"),
            ..Extra::default()
        }
    });
    drop(packed);
    let out = read_with(&file, true);
    assert!(
        out.blobs.iter().any(|b| b.asset == "page-1"),
        "{:#?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

#[test]
fn many_damaged_streams_stay_small() {
    // Twelve streams of 16 MB each from about 100 KB of data, each ending in a code that
    // does not exist: the canonical copy holds what was measured, packed again.
    let mut d = common::Deflate::zlib();
    d.fixed_block(true);
    d.symbol(u32::from(b' '));
    for _ in 0..65_000 {
        d.run();
    }
    d.bad_symbol();
    d.symbol(256);
    let broken = d.finish();
    let file = render_twin(595, 842, "", |b| {
        let refs: Vec<String> = (0..12)
            .map(|_| format!("{} 0 R", b.stream("/Filter /FlateDecode", &broken)))
            .collect();
        Extra {
            page: format!("/PieceInfo << /Data [{}] >>", refs.join(" ")),
            ..Extra::default()
        }
    });
    let out = read_with(&file, false);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what.contains("is damaged")),
        "{:#?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

#[test]
fn a_picture_with_a_large_mask_counts_the_mask() {
    // A small picture whose /Mask is a stencil of 8,000 by 8,000 pixels: alone just under the
    // render room, with the page over it.
    let start = Instant::now();
    let file = render_twin(595, 842, "q 595 0 0 842 0 0 cm /Im1 Do Q\n", |b| {
        let mask = b.stream(
            "/Type /XObject /Subtype /Image /Width 8000 /Height 8000 /ImageMask true \
             /BitsPerComponent 1 /Filter /FlateDecode",
            &deflate(&[0u8; 800]),
        );
        let image = b.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray \
                 /BitsPerComponent 8 /Mask {mask} 0 R /Filter /FlateDecode"
            ),
            &deflate(&[90u8; 64]),
        );
        Extra {
            xobjects: format!("/Im1 {image} 0 R"),
            ..Extra::default()
        }
    });
    let out = read_with(&file, true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("pictures"), "{loss:?}");
}

#[test]
fn a_thousand_page_fills_render() {
    // Under the render room by the per-draw estimate, and well under the memory bound.
    let out = read_with(&fill_twin(1_000, false), true);
    assert!(
        out.blobs.iter().any(|b| b.asset == "page-1"),
        "{:#?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

/// A page that needs vision, then `content` drawn on it, with Helvetica as `/F1` and the tiling
/// patterns `/P1` (a cell of 20 points) and `/P2` (a cell of 20 points filled with `/P1`).
fn draw_twin(content: &str) -> Vec<u8> {
    let packed = deflate(content.as_bytes());
    render_twin(595, 842, "/Body Do\n", |b| {
        let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
        let p1 = b.stream(
            "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 20 20] /XStep 20 \
             /YStep 20 /Resources << >>",
            b"0 0 10 10 re f",
        );
        let p2 = b.stream(
            &format!(
                "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 20 20] /XStep 20 \
                 /YStep 20 /Resources << /Pattern << /P1 {p1} 0 R >> >>"
            ),
            b"/Pattern cs /P1 scn 0 0 20 20 re f",
        );
        let body = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /Filter /FlateDecode \
                 /Resources << /Font << /F1 {font} 0 R >> \
                 /Pattern << /P1 {p1} 0 R /P2 {p2} 0 R >> >>"
            ),
            &packed,
        );
        Extra {
            xobjects: format!("/Body {body} 0 R"),
            ..Extra::default()
        }
    })
}

#[test]
fn finely_dashed_lines_stay_small() {
    // 800 full-height lines with dashes of 0.01 points: about 67 million dashes.
    let line = "0 0 m 0 842 l S\n";
    let content = format!("[0.01] 0 d\n{}", line.repeat(800));
    assert_fills_bounded(&draw_twin(&content));
}

#[test]
fn a_long_finely_dashed_curve_stays_small() {
    let mut c = String::from("[0.05] 0 d 10 10 m\n");
    for i in 0..300 {
        let y = if i % 2 == 0 { 832 } else { 10 };
        c.push_str(&format!(
            "{} {y} {} {y} {} {} c\n",
            10 + i % 500,
            20 + i % 500,
            30 + i % 500,
            842 - y
        ));
    }
    c.push_str("S\n");
    assert_fills_bounded(&draw_twin(&c));
}

#[test]
fn a_dash_array_with_many_entries_stays_small() {
    let dashes = vec!["0.01"; 1000].join(" ");
    let content = format!("[{dashes}] 0 d\n{}", "0 0 m 0 842 l S\n".repeat(800));
    assert_fills_bounded(&draw_twin(&content));
}

#[test]
fn a_dense_hatch_in_one_path_stays_small() {
    let mut c = String::new();
    for i in 0..1_000 {
        let x = (i % 595) as f32 + 0.5;
        c.push_str(&format!("{x} 0 m {x} 842 l {} 842 l h\n", x + 0.1));
    }
    c.push_str("f\n");
    assert_fills_bounded(&draw_twin(&c));
}

#[test]
fn nested_clips_stay_small() {
    let clip = "q 0 0 m 595 0 l 297 842 l h W n\n";
    let content = format!(
        "{}0 0 595 842 re f\n{}",
        clip.repeat(800),
        "Q\n".repeat(800)
    );
    assert_fills_bounded(&draw_twin(&content));
}

#[test]
fn huge_stroke_widths_stay_small() {
    assert_fills_bounded(&draw_twin(&"1000000 w 0 0 m 595 842 l S\n".repeat(800)));
}

#[test]
fn text_that_fills_strokes_and_clips_stays_small() {
    let mut c = String::from("BT /F1 400 Tf 2 Tr\n");
    for _ in 0..200 {
        c.push_str("0 100 Td (WM) Tj 0 -100 Td\n");
    }
    c.push_str("6 Tr (WMWM) Tj ET\n");
    assert_fills_bounded(&draw_twin(&c));
}

#[test]
fn patterns_inside_patterns_stay_small() {
    let content = "/Pattern cs /P2 scn 0 0 595 842 re f\n".repeat(600);
    assert_fills_bounded(&draw_twin(&content));
}

#[test]
fn coarsely_dashed_lines_stay_small() {
    // 100 full-height lines with dashes of 2 points: about 21,000 dashes, under the room.
    let content = format!("[2] 0 d\n{}", "0 0 m 0 842 l S\n".repeat(100));
    assert_fills_bounded(&draw_twin(&content));
}

#[test]
fn a_large_form_is_scanned_for_inline_pictures_with_a_small_buffer() {
    // A form of 200 MB of comment lines (rows of noise, each repeated 40 times), named in the
    // page's resources, with a small inline stencil at its end. The scan for inline pictures
    // reads it piece by piece; the page renders.
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut state = 5u32;
    let mut line = vec![b'%'; 1000];
    line[999] = b'\n';
    for i in 0..200_000 {
        if i % 40 == 0 {
            for b in &mut line[1..999] {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
                *b = b'a' + ((state >> 16) % 26) as u8;
            }
        }
        z.write_all(&line).expect("deflate");
    }
    z.write_all(b"BI /W 8 /H 8 /IM true /BPC 1 ID \x00\x00\x00\x00\x00\x00\x00\x00 EI\n")
        .expect("deflate");
    let packed = z.finish().expect("deflate");
    let file = render_twin(595, 842, "", |b| {
        let form = b.stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /Filter /FlateDecode",
            &packed,
        );
        Extra {
            xobjects: format!("/Big {form} 0 R"),
            ..Extra::default()
        }
    });
    drop(packed);
    let out = read_with(&file, true);
    assert!(
        out.blobs.iter().any(|b| b.asset == "page-1"),
        "{:#?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

/// A technical drawing: `n` dashed lines of 0 to 300 points at seeded places.
fn dashed_drawing(n: usize) -> String {
    let mut c = String::from("0 g 0.5 w [6 3] 0 d\n");
    let mut s: u64 = 7;
    for _ in 0..n {
        let mut r = || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) % 560
        };
        let (x, y, dx, dy) = (r() + 10, r() + 140, r() % 300, r() % 300);
        c.push_str(&format!(
            "{x} {y} m {} {} l S\n",
            (x + dx).min(585),
            (y + dy).min(830)
        ));
    }
    c
}

#[test]
fn a_drawing_of_two_thousand_dashed_lines_renders() {
    assert_rendered(&read_with(&draw_twin(&dashed_drawing(2_000)), true));
}

#[test]
fn a_form_with_a_thousand_dashed_field_borders_renders() {
    let mut c = String::from("0 g 1 w [3 2] 0 d\n");
    for i in 0..1_000 {
        let (x, y) = (40 + (i % 2) * 270, 20 + (i / 2) * 5 % 800);
        c.push_str(&format!("{x} {y} 240 18 re S\n"));
    }
    assert_rendered(&read_with(&draw_twin(&c), true));
}

#[test]
fn dashes_in_one_path_just_under_the_room_render_small() {
    // 380 full-height lines in one path, stroked once with round caps and dashes of 1 point:
    // about 152,000 dashes whose outlines are all built at once, just under the room by the
    // model.
    let mut c = String::from("1 J [1 1] 0 d\n");
    for i in 0..380 {
        let x = 10 + i * 3 / 2;
        c.push_str(&format!("{x} 21 m {x} 821 l\n"));
    }
    c.push_str("S\n");
    let start = Instant::now();
    let out = read_with(&draw_twin(&c), true);
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert_rendered(&out);
}

#[test]
fn dashes_in_a_form_drawn_many_times_stay_small() {
    // A form of 100 full-height lines with dashes of 1 point, drawn 100 times: 4 million
    // dashes, about 316 MiB of strips when rendered.
    let start = Instant::now();
    let mut lines = String::from("[1 1] 0 d\n");
    for i in 0..100 {
        lines.push_str(&format!("{} 21 m {} 821 l S\n", 20 + i * 5, 20 + i * 5));
    }
    let file = render_twin(595, 842, &"/Dash Do\n".repeat(100), |b| {
        let form = b.stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /Filter /FlateDecode",
            &deflate(lines.as_bytes()),
        );
        Extra {
            xobjects: format!("/Dash {form} 0 R"),
            ..Extra::default()
        }
    });
    let out = read_with(&file, true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("drawing operations"), "{loss:?}");
}

#[test]
fn a_dash_pattern_of_negative_length_is_not_rendered() {
    // The renderer's dasher never finishes such a pattern.
    for pattern in ["[1 -2]", "[-1 -1]"] {
        let start = Instant::now();
        let content = format!("{pattern} 0 d 0 0 m 0 842 l S\n");
        let out = read_with(&draw_twin(&content), true);
        let loss = assert_render_skipped(&out, start);
        assert!(loss.what.contains("dash pattern"), "{loss:?}");
    }
}

/// `n` bytes of zeros, deflated as small as they go.
fn zeros_tight(n: usize) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    let chunk = vec![0u8; 1 << 20];
    let mut left = n;
    while left > 0 {
        let k = left.min(chunk.len());
        z.write_all(&chunk[..k]).expect("deflate");
        left -= k;
    }
    z.finish().expect("deflate")
}

/// Plain line drawing of about `bytes` bytes.
fn plain_lines(bytes: usize) -> String {
    let mut c = String::from("0 g 0.5 w\n");
    let mut i = 0;
    while c.len() < bytes {
        c.push_str(&format!(
            "{} {} m {} {} l S\n",
            20 + i % 500,
            20 + i % 700,
            30 + i % 500,
            30 + i % 700
        ));
        i += 1;
    }
    c
}

/// An 8 by 8 inline stencil whose 300 MB of data packs into a few hundred KB, placed over
/// the whole page: the dry run charges only its 64 pixels, so only the stencil check sees it.
fn bomb_stencil() -> Vec<u8> {
    let mut c = b"q 595 0 0 842 0 0 cm BI /W 8 /H 8 /IM true /BPC 1 /F /Fl ID ".to_vec();
    c.extend(zeros_tight(300 << 20));
    c.extend(b" EI Q\n");
    c
}

/// A page whose text says "Quarterly BI report", then `gap` bytes of plain drawing, then
/// `tail`.
fn bi_text_twin(gap: usize, tail: &[u8]) -> Vec<u8> {
    let mut c = b"BT /F1 12 Tf 72 800 Td (Quarterly BI report) Tj ET\n".to_vec();
    c.extend(plain_lines(gap).as_bytes());
    c.extend(tail);
    // Text after it with the bytes ID and EI, as the old byte search took them.
    c.extend(b"BT /F1 12 Tf 72 700 Td (ID card, EI unit) Tj ET\n");
    draw_twin_bytes(&c)
}

/// [`draw_twin`] with content that need not be text.
fn draw_twin_bytes(content: &[u8]) -> Vec<u8> {
    let packed = deflate(content);
    render_twin(595, 842, "/Body Do\n", |b| {
        let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
        let body = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /Filter /FlateDecode \
                 /Resources << /Font << /F1 {font} 0 R >> >>"
            ),
            &packed,
        );
        Extra {
            xobjects: format!("/Body {body} 0 R"),
            ..Extra::default()
        }
    })
}

#[test]
fn bi_in_a_text_string_does_not_stop_the_render() {
    // 1.3 MB of plain drawing after the words, and no inline picture.
    assert_rendered(&read_with(&bi_text_twin(1_300_000, b""), true));
}

#[test]
fn an_inline_stencil_after_bi_in_a_text_string_is_counted() {
    let start = Instant::now();
    let out = read_with(&bi_text_twin(1_000, &bomb_stencil()), true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("stencil mask"), "{loss:?}");
}

#[test]
fn an_inline_stencil_far_after_bi_in_a_text_string_is_counted() {
    // The stencil comes 1.3 MB after the words, past the scan's window.
    let start = Instant::now();
    let out = read_with(&bi_text_twin(1_300_000, &bomb_stencil()), true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("stencil mask"), "{loss:?}");
}

#[test]
fn an_inline_picture_before_much_drawing_renders() {
    // The scan decides where the picture ends with a window of the content after it, and
    // follows the rest piece by piece.
    let mut c = b"q 10 0 0 10 0 0 cm BI /W 8 /H 8 /BPC 8 /CS /G /F /Fl ID ".to_vec();
    c.extend(deflate(&[40u8; 64]));
    c.extend(b" EI Q\n");
    c.extend(plain_lines(1_100_000).as_bytes());
    assert_rendered(&read_with(&draw_twin_bytes(&c), true));
}

#[test]
fn an_inline_picture_that_cannot_be_read_is_not_rendered() {
    // `/H` has no value: the renderer stops at this `BI`.
    let start = Instant::now();
    let content = "q 8 0 0 8 0 0 cm BI /W 8 /H ID \x00\x00\x00\x00\x00\x00\x00\x00 EI Q\n";
    let out = read_with(&render_twin(595, 842, content, |_| Extra::default()), true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("cannot be read"), "{loss:?}");
}

#[test]
fn an_inline_stencil_split_over_two_content_streams_is_counted() {
    // The renderer joins a page's content streams with a space: the stencil's dictionary ends
    // the first and its data starts the second.
    let start = Instant::now();
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /Filter /FlateDecode",
        &deflate(&[90u8; 64]),
    );
    let first = b.stream(
        "",
        b"q 595 0 0 842 0 0 cm /Im0 Do Q\nq 595 0 0 842 0 0 cm BI /W 8 /H 8 /IM true /BPC 1 \
          /F /Fl ID",
    );
    let mut data = zeros_tight(300 << 20);
    data.extend(b" EI Q\n");
    let second = b.stream("", &data);
    drop(data);
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /XObject << /Im0 {image} 0 R >> >> \
         /Contents [{first} 0 R {second} 0 R] >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read_with(&b.finish(catalog), true);
    let loss = assert_render_skipped(&out, start);
    assert!(loss.what.contains("stencil mask"), "{loss:?}");
}

/// `data` in ASCII85, with a line break after every 75 characters, as Distiller wrote it.
fn ascii85_lines(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut col = 0;
    let mut put = |c: u8, out: &mut Vec<u8>| {
        out.push(c);
        col += 1;
        if col == 75 {
            out.push(b'\n');
            col = 0;
        }
    };
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        let mut v = u32::from_be_bytes(word);
        if chunk.len() == 4 && v == 0 {
            put(b'z', &mut out);
            continue;
        }
        let mut digits = [0u8; 5];
        for d in digits.iter_mut().rev() {
            *d = b'!' + (v % 85) as u8;
            v /= 85;
        }
        for &d in &digits[..chunk.len() + 1] {
            put(d, &mut out);
        }
    }
    out.extend(b"~>");
    out
}

#[test]
fn ascii85_bands_holding_ei_render() {
    // A scan in four inline bands of about 400 KB of ASCII85 each, wrapped at 75 columns: a
    // line that ends in `EI` is common in such data.
    let mut c = Vec::new();
    let mut state = 11u32;
    let mut inner = 0;
    for band in 0..4 {
        let mut px = vec![0u8; 595 * 200 * 3];
        for p in px.iter_mut() {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            *p = (state >> 16) as u8;
        }
        let data = ascii85_lines(&deflate(&px));
        inner += data
            .windows(3)
            .filter(|w| &w[..2] == b"EI" && w[2] == b'\n')
            .count();
        c.extend(
            format!(
                "q 595 0 0 200 0 {} cm BI /W 595 /H 200 /CS /RGB /BPC 8 /F [/A85 /Fl] ID ",
                band * 200
            )
            .as_bytes(),
        );
        c.extend(data);
        c.extend(b" EI Q\n");
    }
    assert!(inner > 0, "no band holds EI at a line end");
    assert_rendered(&read_with(&draw_twin_bytes(&c), true));
}

#[test]
fn a_page_at_the_pixel_cap_renders_small_enough_for_its_png() {
    // 200,000 points square, with a picture of noise over it: the pixmap and a PNG that
    // hardly packs, in a buffer that doubles, take 12 bytes a pixel, which must fit the render
    // room of 128 MiB.
    let mut b = Builder::default();
    let mut state = 3u32;
    let noise: Vec<u8> = (0..512 * 512 * 3)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) as u8
        })
        .collect();
    let catalog = b.reserve();
    let tree = b.reserve();
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 512 /Height 512 /ColorSpace /DeviceRGB \
         /BitsPerComponent 8 /Filter /FlateDecode",
        &deflate(&noise),
    );
    let c = b.stream("", b"q 200000 0 0 200000 0 0 cm /Im0 Do Q");
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 200000 200000] \
         /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read_with(&b.finish(catalog), true);
    let blob = out
        .blobs
        .iter()
        .find(|b| b.asset == "page-1")
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    let w = u64::from(u32::from_be_bytes(blob.bytes[16..20].try_into().unwrap()));
    let h = u64::from(u32::from_be_bytes(blob.bytes[20..24].try_into().unwrap()));
    assert!(w * h * 12 <= 128 << 20, "{w} x {h}");
    assert!(w > 3000 && h > 3000, "{w} x {h}");
    assert_rss_under_256_mib();
}

#[test]
fn many_small_pictures_crops_bounded() {
    // The twin with one picture, for the time a page with crops takes.
    let start = Instant::now();
    let one = read_with(&common::small_pictures_twin(1), true);
    let one_took = start.elapsed();
    assert_eq!(one.blobs.len(), 1, "{:#?}", one.document.losses);
    let file = common::small_pictures_twin(2_000);
    let start = Instant::now();
    let out = read_with(&file, true);
    let took = start.elapsed();
    eprintln!(
        "1 picture: {one_took:?}; 2,000 pictures: {took:?}, max RSS {} MiB",
        common::max_rss_bytes() >> 20
    );
    assert!(took < Duration::from_secs(60), "{took:?}");
    assert_rss_under_256_mib();
    // Each picture is an asset of its own that needs vision; the page is not one.
    assert_eq!(out.document.assets.len(), 2_000);
    assert!(out.document.assets.iter().all(|a| a.needs_vision));
    assert!(!out.document.assets.iter().any(|a| a.id == "page-1"));
    // The crops fit the blob room, an eighth of the memory limit.
    let kept: u64 = out.blobs.iter().map(|b| b.bytes.len() as u64).sum();
    assert!(kept <= hostile().max_memory_bytes / 8, "{kept}");
    assert_eq!(out.blobs.len(), 2_000, "{:#?}", out.document.losses);
    // The blob room running out is in the reader's own tests (lib.rs): a memory limit small
    // enough for that leaves too little for the render itself.
}

#[test]
fn many_places_of_one_picture_stay_linear() {
    // The twin with one place, then 100,000 places of one picture on one page: one asset, and
    // a read that grows with the places, not with their square. Nothing is rendered: the page
    // with 100,000 draws is over the render room, which other tests cover.
    let start = Instant::now();
    // At 1 GiB, so the kept bytes (a sixteenth) take the 100,000 places and their work.
    let limits = Limits {
        max_memory_bytes: 1 << 30,
        ..hostile()
    };
    let read = |bytes: &[u8]| read_with_limits(bytes, false, limits.clone());
    let one = read(&common::many_places_twin(1));
    let one_took = start.elapsed();
    assert_eq!(one.document.assets.len(), 1, "{:#?}", one.document.losses);
    let n = 100_000;
    let file = common::many_places_twin(n);
    let start = Instant::now();
    let out = read(&file);
    let took = start.elapsed();
    eprintln!(
        "1 place: {one_took:?}; {n} places: {took:?}, max RSS {} MiB",
        common::max_rss_bytes() >> 20
    );
    assert!(
        took < one_took * 20 + Duration::from_secs(5),
        "{took:?} against {one_took:?}"
    );
    assert_eq!(out.document.assets.len(), 1, "{:#?}", out.document.losses);
    let media = out
        .document
        .body
        .iter()
        .map(|s| match s {
            Block::Section { blocks, .. } => blocks
                .iter()
                .filter(|b| matches!(b, Block::Media { .. }))
                .count(),
            _ => 0,
        })
        .sum::<usize>();
    assert_eq!(media, n);
    assert_rss_under_256_mib();
}

/// Pages with a line of text and one shared gray picture (60 by 40 points, 50 points lower on
/// each page). A page
/// marked in `bomb` also draws, at 10 by 10 points (too small to be listed), a JPEG that says
/// it is 15,000 by 15,000 pixels, so that page is not rendered.
fn shared_picture_pages(bomb: &[bool]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let logo = b.stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
         /BitsPerComponent 8",
        &[30u8; 16],
    );
    let jpeg = b.stream(
        "/Type /XObject /Subtype /Image /Width 15000 /Height 15000 /ColorSpace /DeviceRGB \
         /BitsPerComponent 8 /Filter /DCTDecode",
        &jpeg_head(15_000, 15_000),
    );
    let mut kids = String::new();
    for (n, &bad) in bomb.iter().enumerate() {
        // Each page at another place, so the picture is no page furniture.
        let y = 400 - 50 * n;
        let mut c =
            format!("BT /F1 12 Tf 72 700 Td (Page {n}.) Tj ET\nq 60 0 0 40 250 {y} cm /L Do Q\n");
        if bad {
            c.push_str("q 10 0 0 10 300 200 cm /B Do Q\n");
        }
        let cs = b.stream("", c.as_bytes());
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> /XObject << /L {logo} 0 R /B {jpeg} 0 R >> \
             >> /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", bomb.len()),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn a_shared_picture_is_cut_at_the_next_page_that_renders() {
    // Page 1 is not rendered; page 2, with the same picture, is: the crop comes from page 2.
    let out = read_with(&shared_picture_pages(&[true, false, false]), true);
    let ids: Vec<&str> = out.document.assets.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["page-1-pic-1"]);
    let blobs: Vec<&str> = out.blobs.iter().map(|b| b.asset.as_str()).collect();
    assert_eq!(
        blobs,
        ["page-1-pic-1"],
        "{:#?} {:#?}",
        out.document.losses,
        out.document.assets
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.location == "p1" && l.what.starts_with("page 1 is not rendered")),
        "{:#?}",
        out.document.losses
    );
    // No page with it renders: the loss names the picture and its pages.
    let out = read_with(&shared_picture_pages(&[true, true, false]), true);
    assert_eq!(out.blobs.len(), 1, "{:#?}", out.document.losses);
    let out = read_with(&shared_picture_pages(&[true, true]), true);
    assert!(out.blobs.is_empty());
    let named: Vec<&Loss> = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.contains("page-1-pic-1"))
        .collect();
    assert_eq!(named.len(), 1, "{:#?}", out.document.losses);
    assert!(named[0].what.contains("pages 1, 2"), "{}", named[0].what);
}

#[test]
fn many_places_at_a_low_limit_stay_under_it() {
    // 160,000 places of one picture at a memory limit of 64 MiB: the work for each drawn
    // picture is charged, so the read stays under the limit and says what it did not list.
    let limit = 64u64 << 20;
    let limits = Limits {
        max_memory_bytes: limit,
        ..hostile()
    };
    let (out, peak) =
        common::peak_of(|| read_with_limits(&common::many_places_twin(160_000), false, limits));
    let rss = common::max_rss_bytes();
    eprintln!(
        "160,000 places at 64 MiB: heap peak {} MiB, max RSS {} MiB",
        peak >> 20,
        rss >> 20
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what.contains("small pictures on page 1 not listed")),
        "{:#?}",
        out.document.losses
    );
    assert!(peak < limit as usize, "heap peak {} MiB", peak >> 20);
    // The loose guard, as in `hostile.rs`.
    assert!(rss < 512 << 20, "max RSS {} MiB", rss >> 20);
}

// ---------------------------------------------------------------------------------------
// Barcodes and QR codes

use barcodes::Symbology;
use barcodes::encode::{self as code, Options};

/// A picture drawn on a page: x, y, width, height and its index in the pictures.
type Draw = (f32, f32, f32, f32, usize);

/// Pages of `w` by `h` points, each drawing its pictures: (x, y, width, height, picture).
/// The pictures are gray, given as width, height and pixels.
fn picture_file(w: u32, h: u32, pages: &[Vec<Draw>], pictures: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let mut xo = String::new();
    for (k, (pw, ph, px)) in pictures.iter().enumerate() {
        let n = b.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {pw} /Height {ph} /ColorSpace /DeviceGray \
                 /BitsPerComponent 8 /Filter /FlateDecode"
            ),
            &deflate(px),
        );
        xo.push_str(&format!("/P{k} {n} 0 R "));
    }
    let mut kids = String::new();
    for draws in pages {
        let mut c = String::new();
        for (x, y, dw, dh, k) in draws {
            c += &format!("q {dw} 0 0 {dh} {x} {y} cm /P{k} Do Q\n");
        }
        let cs = b.stream("", c.as_bytes());
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] \
             /Resources << /XObject << {xo}>> >> /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// The hostile memory limit with a shorter time limit: 3 seconds a picture, 9 for the file.
fn scan_limits() -> Limits {
    Limits {
        max_time: Duration::from_secs(36),
        ..hostile()
    }
}

fn timed(file: &[u8], extract: bool, limits: Limits) -> (ReadOutcome, Duration) {
    let start = Instant::now();
    let out = read_with_limits(file, extract, limits);
    (out, start.elapsed())
}

/// Square blocks of noise, `block` pixels a side, `w` by `h` pixels: they look like modules.
fn block_noise(w: u32, h: u32, block: u32) -> Vec<u8> {
    let mut state = 0x9e37_79b9u32;
    let cols = w.div_ceil(block);
    let mut row_blocks = vec![0u8; cols as usize];
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        if y % block == 0 {
            for v in &mut row_blocks {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *v = if state & 1 == 0 { 0 } else { 255 };
            }
        }
        for x in 0..w {
            out.push(row_blocks[(x / block) as usize]);
        }
    }
    out
}

#[test]
fn a_noise_page_at_the_render_cap_is_scanned_in_tiles_within_its_time() {
    // A page of 4,000 by 4,000 points covered by noise of 4 pixel blocks: its render has the
    // most pixels a render may have (16 MP), read in tiles. The twin is an A4 page.
    let noise = block_noise(4000, 4000, 4);
    let big = picture_file(
        4000,
        4000,
        &[vec![(0.0, 0.0, 4000.0, 4000.0, 0)]],
        &[(4000, 4000, noise)],
    );
    let small = picture_file(
        595,
        842,
        &[vec![(0.0, 0.0, 595.0, 842.0, 0)]],
        &[(64, 64, block_noise(64, 64, 4))],
    );
    let (twin, twin_time) = timed(&small, false, scan_limits());
    assert!(twin.document.assets.iter().all(|a| a.codes.is_empty()));
    let (out, took) = timed(&big, false, scan_limits());
    eprintln!(
        "twin {twin_time:?}, noise {took:?}, max RSS {} MiB",
        common::max_rss_bytes() >> 20
    );
    let found: Vec<_> = out.document.assets.iter().flat_map(|a| &a.codes).collect();
    assert!(found.is_empty(), "{found:#?}");
    // One picture's time and the render, at most.
    assert!(took < Duration::from_secs(15), "{took:?}");
    assert_rss_under_256_mib();
}

/// Finder-like squares (a QR code's corner pattern) on a grid, `n` of them, 4 pixels a module.
fn finders(n: u32) -> (u32, u32, Vec<u8>) {
    let per = f64::from(n).sqrt().ceil() as u32;
    let side = per * 36;
    let mut out = vec![255u8; (side * side) as usize];
    for k in 0..n {
        let (cx, cy) = ((k % per) * 36 + 4, (k / per) * 36 + 4);
        for y in 0..28 {
            for x in 0..28 {
                let ring = x.min(y).min(27 - x).min(27 - y);
                if !(4..8).contains(&ring) {
                    out[((cy + y) * side + cx + x) as usize] = 0;
                }
            }
        }
    }
    (side, side, out)
}

#[test]
fn five_thousand_finder_patterns_stop_at_the_time_limit() {
    let file = |n: u32| {
        let pic = finders(n);
        picture_file(595, 842, &[vec![(0.0, 0.0, 595.0, 842.0, 0)]], &[pic])
    };
    let (twin, twin_time) = timed(&file(50), false, scan_limits());
    assert!(
        twin.document.losses.is_empty(),
        "{:#?}",
        twin.document.losses
    );
    let (out, took) = timed(&file(5_000), false, scan_limits());
    eprintln!(
        "twin {twin_time:?}, 5,000 finders {took:?}, max RSS {} MiB",
        common::max_rss_bytes() >> 20
    );
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.what.starts_with("barcode scan stopped: time limit"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    // The page keeps its need of vision, so the read stays complete (R14e-C).
    assert_eq!(
        (loss.why, loss.severity),
        (LossReason::Limit, Severity::Info)
    );
    assert_eq!(out.completeness, Completeness::Complete);
    // The scan stops waiting at 3 seconds a picture; the page still needs vision.
    assert!(took < twin_time + Duration::from_secs(8), "{took:?}");
    assert!(
        out.document
            .assets
            .iter()
            .any(|a| a.id == "page-1" && a.needs_vision)
    );
    assert_rss_under_256_mib();
}

#[test]
fn a_picture_of_400_mp_is_refused_before_it_is_decoded() {
    // A 1-bit picture (it looks like a code, so it is read again at its own resolution): the
    // dictionary says 20,000 by 20,000 pixels; the data is short. The twin says 2,000.
    let file = |side: u32| {
        let mut b = Builder::default();
        let catalog = b.reserve();
        let tree = b.reserve();
        let image = b.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {side} /Height {side} \
                 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /FlateDecode"
            ),
            &deflate(&[200u8; 4096]),
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
        b.finish(catalog)
    };
    let (_, twin_time) = timed(&file(2_000), false, scan_limits());
    let (out, took) = timed(&file(20_000), false, scan_limits());
    eprintln!(
        "twin {twin_time:?}, 400 MP {took:?}, max RSS {} MiB",
        common::max_rss_bytes() >> 20
    );
    assert!(took < Duration::from_secs(5), "{took:?}");
    // Nothing decodes it: the page render is refused by what it would take, before any
    // decoding, and says so.
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| {
            l.what.contains("too large to scan for codes")
                || l.what.starts_with("page 1 is not scanned for barcodes")
        })
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(loss.why, LossReason::Limit);
    assert_rss_under_256_mib();
}

#[test]
fn the_code_cap_of_a_file_holds() {
    // 21 pages of 30 different small QR codes each, 630 codes: 500 are kept.
    let pictures: Vec<(u32, u32, Vec<u8>)> = (0..630)
        .map(|k| {
            let m = code::text(
                Symbology::QrCode,
                &format!("CODE-{k:04}"),
                &Options::default(),
            )
            .expect("encode");
            m.gray(3, 4, 0)
        })
        .map(|(g, w, h)| (w, h, g))
        .collect();
    let pages = |count: usize| -> Vec<Vec<Draw>> {
        (0..count)
            .map(|p| {
                (0..30)
                    .map(|i| {
                        let (col, row) = ((i % 6) as f32, (i / 6) as f32);
                        (
                            40.0 + col * 90.0,
                            100.0 + row * 140.0,
                            60.0,
                            60.0,
                            p * 30 + i,
                        )
                    })
                    .collect()
            })
            .collect()
    };
    let (twin, twin_time) = timed(
        &picture_file(595, 842, &pages(1), &pictures[..30]),
        false,
        hostile(),
    );
    let twin_codes: usize = twin.document.assets.iter().map(|a| a.codes.len()).sum();
    assert_eq!(twin_codes, 30, "{:#?}", twin.document.losses);
    let (out, took) = timed(
        &picture_file(595, 842, &pages(21), &pictures),
        false,
        hostile(),
    );
    eprintln!(
        "twin {twin_time:?}, 630 codes {took:?}, max RSS {} MiB",
        common::max_rss_bytes() >> 20
    );
    let codes: usize = out.document.assets.iter().map(|a| a.codes.len()).sum();
    assert_eq!(codes, 500);
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.what.contains("more than 500 codes"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(loss.why, LossReason::Limit);
    assert!(took < twin_time * 30 + Duration::from_secs(10), "{took:?}");
    assert_rss_under_256_mib();
}

#[test]
fn a_payload_of_the_largest_code_is_kept_whole() {
    // A QR code of version 40 with 2,940 bytes, about the most a QR code holds, against a
    // short one.
    let file = |raw: &[u8]| {
        let m = code::bytes(
            Symbology::QrCode,
            raw,
            &Options {
                error_correction: Some("L".into()),
            },
        )
        .expect("encode");
        let (g, w, h) = m.gray(2, 4, 0);
        picture_file(
            595,
            842,
            &[vec![(150.0, 300.0, 300.0, 300.0, 0)]],
            &[(w, h, g)],
        )
    };
    let short: Vec<u8> = (0u8..=255).cycle().take(32).collect();
    let long: Vec<u8> = (0u8..=255).rev().cycle().take(2_940).collect();
    let (twin, twin_time) = timed(&file(&short), false, scan_limits());
    let (out, took) = timed(&file(&long), false, scan_limits());
    eprintln!(
        "twin {twin_time:?}, 2,940 bytes {took:?}, max RSS {} MiB",
        common::max_rss_bytes() >> 20
    );
    let got = |o: &ReadOutcome| {
        o.document
            .assets
            .iter()
            .flat_map(|a| &a.codes)
            .map(|c| c.bytes)
            .collect::<Vec<u64>>()
    };
    assert_eq!(got(&twin), [32]);
    assert_eq!(got(&out), [2_940]);
    assert!(took < twin_time * 4 + Duration::from_secs(2), "{took:?}");
    assert_rss_under_256_mib();
}

/// One page of `w` by `h` points whose content fills the rectangles `cells` in one path.
fn cells_page(w: u32, h: u32, cells: &[[f32; 4]]) -> Vec<u8> {
    let mut c = String::from("0 g\n");
    for r in cells {
        c += &format!("{} {} {} {} re\n", r[0], r[1], r[2] - r[0], r[3] - r[1]);
    }
    c += "f\n";
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let cs = b.stream("/Filter /FlateDecode", &deflate(c.as_bytes()));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn many_small_filled_rectangles_are_searched_within_bounds() {
    // A checkerboard of squares of one point (they touch at their corners: one huge group),
    // tall thin bars, and squares all at one place (every pair is near), each 200,000 of
    // them, against a twin of 2,000 squares.
    let checker = |n: usize| -> Vec<[f32; 4]> {
        let side = (n as f32 * 2.0).sqrt().ceil() as usize;
        (0..n)
            .map(|k| {
                let (row, col) = (k / (side / 2), (k % (side / 2)) * 2 + (k / (side / 2)) % 2);
                let (x, y) = (10.0 + col as f32, 10.0 + row as f32);
                [x, y, x + 1.0, y + 1.0]
            })
            .collect()
    };
    let bars: Vec<[f32; 4]> = (0..200_000)
        .map(|k| {
            let x = 10.0 + (k % 1_000) as f32 * 1.0;
            let y = 10.0 + (k / 1_000) as f32 * 0.01;
            [x, y, x + 0.5, y + 290.0]
        })
        .collect();
    let stacked: Vec<[f32; 4]> = (0..200_000).map(|_| [100.0, 100.0, 101.0, 101.0]).collect();
    let (twin, twin_time) = timed(&cells_page(1_000, 1_000, &checker(2_000)), false, hostile());
    assert!(
        twin.document.assets.is_empty(),
        "{:#?}",
        twin.document.assets
    );
    for (name, cells) in [
        ("checkerboard", checker(200_000)),
        ("bars", bars),
        ("stacked", stacked),
    ] {
        let (out, took) = timed(&cells_page(1_000, 1_000, &cells), false, hostile());
        eprintln!(
            "twin {twin_time:?}, {name} {took:?}, max RSS {} MiB, losses {:?}",
            common::max_rss_bytes() >> 20,
            out.document
                .losses
                .iter()
                .map(|l| &l.what)
                .collect::<Vec<_>>()
        );
        assert!(
            out.document.assets.is_empty(),
            "{name}: {:#?}",
            out.document.assets
        );
        assert!(took < Duration::from_secs(10), "{name}: {took:?}");
    }
    assert_rss_under_256_mib();
}

/// A version 10 QR code of one pixel a module (57 pixels a side): drawn at 30 points it
/// reads only when rendered again finer than the page.
fn small_qr() -> (u32, u32, Vec<u8>) {
    let m = code::text(
        Symbology::QrCode,
        &"spc 0200 1 ch4431999123000889012 ".repeat(7),
        &Options::default(),
    )
    .expect("encode");
    let (g, w, h) = m.gray(1, 0, 0);
    (w, h, g)
}

#[test]
fn a_finer_look_that_does_not_fit_the_memory_is_named() {
    // The QR code in a corner and a picture of 2,000 by 2,000 pixels drawn small: at 24 MiB
    // the page renders, the finer look at the code does not. The twin at 128 MiB reads it.
    let file = picture_file(
        595,
        842,
        &[vec![
            (500.0, 30.0, 30.0, 30.0, 0),
            (300.0, 400.0, 30.0, 30.0, 1),
        ]],
        &[small_qr(), (2000, 2000, block_noise(2000, 2000, 50))],
    );
    let at = |mib: u64| {
        let start = Instant::now();
        let out = read_with_limits(
            &file,
            false,
            Limits {
                max_memory_bytes: mib << 20,
                ..scan_limits()
            },
        );
        (out, start.elapsed())
    };
    let (twin, twin_time) = at(128);
    assert_eq!(
        twin.document
            .assets
            .iter()
            .map(|a| a.codes.len())
            .sum::<usize>(),
        1
    );
    let (out, took) = at(24);
    eprintln!(
        "twin {twin_time:?}, 24 MiB {took:?}, max RSS {} MiB",
        common::max_rss_bytes() >> 20
    );
    assert!(out.document.assets.iter().all(|a| a.codes.is_empty()));
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| {
            l.what
                .starts_with("codes in parts of page 1 are not looked for at a finer scale")
        })
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(
        (loss.why, loss.severity),
        (LossReason::Limit, Severity::Info)
    );
    assert_eq!(out.completeness, Completeness::Complete);
}

/// The modules of a QR code with its middle spoiled: it keeps its finders and gives no code.
fn spoiled_qr(text: &str) -> code::Modules {
    let mut m = code::text(Symbology::QrCode, text, &Options::default()).expect("encode");
    let n = m.width as usize;
    // Every module outside the three finder corners flipped in a checker pattern.
    for r in 0..n {
        for c in 0..n {
            let corner = (r < 8 && c < 8) || (r < 8 && c >= n - 8) || (r >= n - 8 && c < 8);
            if !corner && (r + c) % 2 == 0 {
                m.dark[r * n + c] = !m.dark[r * n + c];
            }
        }
    }
    m
}

/// A code drawn as one `re` a dark module, `module` points each, its corner at (x, y).
fn vector_modules(m: &code::Modules, x: f32, y: f32, module: f32) -> String {
    let mut out = String::from("q 0 g\n");
    for row in 0..m.height {
        for col in 0..m.width {
            if m.dark[(row * m.width + col) as usize] {
                let mx = x + col as f32 * module;
                let my = y + (m.height - 1 - row) as f32 * module;
                out += &format!("{mx:.3} {my:.3} {module:.3} {module:.3} re\n");
            }
        }
    }
    out + "f Q\n"
}

fn content_pages(contents: &[String]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let mut kids = String::new();
    for c in contents {
        let cs = b.stream("/Filter /FlateDecode", &deflate(c.as_bytes()));
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /Contents {cs} 0 R >>"
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

#[test]
fn drawings_past_the_region_cap_are_named() {
    // 63 pages of 8 drawings that look like QR codes and give none (504 parts of pages), then
    // a real QR code drawn at 0.6 points a module: past the cap of 500 it is not looked at
    // finer, and says so. Alone it is read.
    let real = code::text(
        Symbology::QrCode,
        "https://example.org/after-the-cap",
        &Options::default(),
    )
    .expect("encode");
    let last = vector_modules(&real, 300.0, 300.0, 0.6);
    let mut pages: Vec<String> = (0..63)
        .map(|p| {
            (0..8)
                .map(|k| {
                    let m = spoiled_qr(&format!("page {p} drawing {k}"));
                    vector_modules(
                        &m,
                        40.0 + (k % 4) as f32 * 130.0,
                        100.0 + (k / 4) as f32 * 300.0,
                        1.0,
                    )
                })
                .collect()
        })
        .collect();
    pages.push(last.clone());
    let (twin, twin_time) = timed(&content_pages(&[last]), false, hostile());
    assert_eq!(
        twin.document
            .assets
            .iter()
            .map(|a| a.codes.len())
            .sum::<usize>(),
        1
    );
    let (out, took) = timed(&content_pages(&pages), false, hostile());
    eprintln!(
        "twin {twin_time:?}, 505 drawings {took:?}, max RSS {} MiB",
        common::max_rss_bytes() >> 20
    );
    assert!(out.document.assets.iter().all(|a| a.codes.is_empty()));
    let last = out
        .document
        .assets
        .iter()
        .find(|a| a.id == "page-64-code-1")
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert!(
        last.alt
            .as_deref()
            .is_some_and(|t| t.ends_with(", not scanned")),
        "{last:#?}"
    );
    assert!(last.needs_vision);
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| {
            l.what
                .contains("a file renders at most 500 parts of pages again")
        })
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert!(loss.what.contains("pages 63, 64"), "{}", loss.what);
    assert_eq!(loss.severity, Severity::Info);
    // The drawings that were read are listed as not decoded.
    assert_eq!(
        out.document
            .assets
            .iter()
            .filter(|a| a
                .alt
                .as_deref()
                .is_some_and(|t| t.ends_with(", not decoded")))
            .count(),
        500
    );
    assert_rss_under_256_mib();
}

#[test]
fn running_out_of_scan_time_keeps_the_read_complete() {
    // 12 pages of scanned text at 150 dpi with a time limit of 4 seconds: the second of
    // scan time runs out; the pages not read for codes are named in an Info loss and keep
    // their need of vision.
    let (w, h) = (1240u32, 1754u32);
    let mut state = 7u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let scan: Vec<u8> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let ink =
                y % 25 < 14 && x > 100 && x < w - 100 && y > 100 && y < h - 100 && next() % 3 == 0;
            if ink { 30 } else { 245 }
        })
        .collect();
    let pages: Vec<Vec<Draw>> = (0..12).map(|_| vec![(0.0, 0.0, 595.0, 842.0, 0)]).collect();
    let file = picture_file(595, 842, &pages, &[(w, h, scan)]);
    let (out, took) = timed(
        &file,
        false,
        Limits {
            max_time: Duration::from_secs(4),
            ..hostile()
        },
    );
    eprintln!(
        "12 scanned pages, 4 s limit: {took:?}, losses {:?}",
        out.document.losses
    );
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| {
            l.what
                .starts_with("barcode scan stopped: time limit: codes are not read on pages")
        })
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(
        (loss.why, loss.severity),
        (LossReason::Limit, Severity::Info)
    );
    assert!(out.document.assets.iter().all(|a| a.needs_vision));
}

#[test]
fn a_shared_code_is_written_once_whatever_its_places() {
    // One QR code picture at 5 places on each of 800 pages (4,000 places) at 256 MiB, against
    // the same file with a picture that holds no code: the Markdown grows by a few bytes a
    // place, and the payload is written once.
    let (w, h, g) = small_qr();
    let payload = "spc 0200 1 ch4431999123000889012 ".repeat(7);
    let blank = vec![200u8; (w * h) as usize];
    let pages = |n: usize| -> Vec<Vec<Draw>> {
        (0..n)
            .map(|p| {
                (0..5)
                    .map(|k| {
                        (
                            40.0 + k as f32 * 100.0,
                            100.0 + (p % 50) as f32 * 12.0,
                            30.0,
                            30.0,
                            0,
                        )
                    })
                    .collect()
            })
            .collect()
    };
    let limits = Limits {
        max_memory_bytes: 256 << 20,
        ..hostile()
    };
    let md = |file: &[u8]| {
        let start = Instant::now();
        let out = read_with_limits(file, false, limits.clone());
        let md = rezzer_render::render(
            &out.document,
            out.completeness,
            &rezzer_render::RenderOptions::default(),
        );
        (out, md, start.elapsed())
    };
    let (twin, twin_md, twin_time) = md(&picture_file(595, 842, &pages(800), &[(w, h, blank)]));
    let (out, coded, took) = md(&picture_file(595, 842, &pages(800), &[(w, h, g)]));
    eprintln!(
        "twin {twin_time:?} {} B, coded {took:?} {} B, max RSS {} MiB",
        twin_md.len(),
        coded.len(),
        common::max_rss_bytes() >> 20
    );
    assert_eq!(
        out.document
            .assets
            .iter()
            .map(|a| a.codes.len())
            .sum::<usize>(),
        1
    );
    assert_eq!(twin.document.assets.len(), 1);
    assert_eq!(coded.matches(payload.as_str()).count(), 1);
    let per_place = (coded.len() as f64 - twin_md.len() as f64) / 4_000.0;
    assert!(per_place < 80.0, "{per_place} bytes a place");
    assert_rss_under_256_mib();
}

/// `n` A4 pages, each a text-like scan at 300 dpi of its own (each page its own picture
/// object, as a scanner writes it): lines of made-up glyphs, 28 rows in every 50.
fn scanned_pages(n: usize) -> Vec<u8> {
    let (w, h) = (2481u32, 3507u32);
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let mut kids = String::new();
    for p in 0..n {
        let mut state = 7u32 + p as u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        let white = vec![245u8; w as usize];
        let mut ink = white.clone();
        for y in 0..h {
            if y % 50 == 0 {
                // A line of made-up glyphs: columns of 3 pixels of ink, some left out.
                for (x, v) in ink.iter_mut().enumerate() {
                    let glyph = x > 200 && x < w as usize - 200 && x % 5 < 3;
                    *v = if glyph && next() % 3 != 0 { 30 } else { 245 };
                }
            }
            let line = y % 50 < 28 && y > 200 && y < h - 200;
            z.write_all(if line { &ink } else { &white })
                .expect("deflate");
        }
        let image = b.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray \
                 /BitsPerComponent 8 /Filter /FlateDecode"
            ),
            &z.finish().expect("deflate"),
        );
        let cs = b.stream("", b"q 595 0 0 842 0 0 cm /Im1 Do Q\n");
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /XObject << /Im1 {image} 0 R >> >> /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

fn scans_at_256_mib(pages: usize, extract: bool) -> (ReadOutcome, Duration, u64) {
    let file = scanned_pages(pages);
    let limits = Limits {
        max_memory_bytes: 256 << 20,
        ..Limits::default()
    };
    let start = Instant::now();
    let out = read_with_limits(&file, extract, limits);
    let took = start.elapsed();
    (out, took, common::max_rss_bytes())
}

/// Each page scanned or named; every page keeps its need of vision.
fn assert_scanned_or_named(out: &ReadOutcome, pages: usize) {
    assert_eq!(
        out.document
            .assets
            .iter()
            .filter(|a| a.needs_vision)
            .count(),
        pages
    );
    for l in &out.document.losses {
        assert_eq!(l.severity, Severity::Info, "{l:?}");
    }
}

#[test]
fn sixty_scanned_pages_without_files_stay_under_the_memory_limit() {
    let (out, took, rss) = scans_at_256_mib(60, false);
    eprintln!(
        "60 pages without -o: {took:?}, max RSS {} MiB, losses {:?}",
        rss >> 20,
        out.document.losses
    );
    assert_scanned_or_named(&out, 60);
    assert!(rss < 256 << 20, "max RSS {} MiB", rss >> 20);
}

#[test]
fn scan_memory_does_not_grow_with_the_pages() {
    // 30 pages, then 60 in the same process: the peak barely moves.
    let (out, _, after_30) = scans_at_256_mib(30, false);
    assert_scanned_or_named(&out, 30);
    let (out, _, after_60) = scans_at_256_mib(60, false);
    assert_scanned_or_named(&out, 60);
    eprintln!(
        "max RSS after 30 pages {} MiB, after 60 more {} MiB",
        after_30 >> 20,
        after_60 >> 20
    );
    assert!(
        after_60 < after_30 + (32 << 20),
        "{} {}",
        after_30 >> 20,
        after_60 >> 20
    );
}

#[test]
fn one_scanned_page_without_files() {
    let (out, took, rss) = scans_at_256_mib(1, false);
    eprintln!("1 page without -o: {took:?}, max RSS {} MiB", rss >> 20);
    assert_scanned_or_named(&out, 1);
}

#[test]
fn sixty_scanned_pages_with_files_stay_under_the_memory_limit() {
    let (out, took, rss) = scans_at_256_mib(60, true);
    eprintln!("60 pages with -o: {took:?}, max RSS {} MiB", rss >> 20);
    assert!(rss < 256 << 20, "max RSS {} MiB", rss >> 20);
    let _ = out;
}
