// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Text from fonts the plain Latin path does not cover, glyphs without a Unicode mapping,
//! invisible text and content that tries to make the text reader work without end.

mod common;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Builder, Page, pdf, show};
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Block, Bytes, CalloutKind, Completeness, Confidence, Detection, FormatId, Loss,
    LossReason, ReadOptions, ReadOutcome, Reader, SectionKind, Severity, Source, plain_text,
};
use rezzer_read_pdf::PdfReader;

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

fn read_with(bytes: &[u8], limits: Limits) -> ReadOutcome {
    PdfReader.read(
        &source(bytes),
        &ReadOptions::default(),
        &Budget::new(limits),
    )
}

fn read(bytes: &[u8]) -> ReadOutcome {
    read_with(bytes, Limits::default())
}

/// The limits of the memory tests: 512 MiB, so pdfparts keeps its objects to 256 MiB.
fn small_memory() -> Limits {
    Limits {
        max_memory_bytes: 512 << 20,
        max_time: Duration::from_secs(120),
        ..Limits::default()
    }
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
        .unwrap_or_else(|| panic!("no section p{n}"))
}

fn block_text(block: &Block, out: &mut String) {
    match block {
        Block::Heading { inlines, .. } | Block::Paragraph { inlines, .. } => {
            out.push_str(&plain_text(inlines));
            out.push('\n');
        }
        Block::Section { blocks, .. } | Block::Callout { blocks, .. } => {
            blocks.iter().for_each(|b| block_text(b, out))
        }
        Block::List { items, .. } => items.iter().flatten().for_each(|b| block_text(b, out)),
        _ => {}
    }
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    blocks.iter().for_each(|b| block_text(b, &mut out));
    out
}

/// The page text without the folded callouts.
fn body_text(blocks: &[Block]) -> String {
    let kept: Vec<Block> = blocks
        .iter()
        .filter(|b| !matches!(b, Block::Callout { .. }))
        .cloned()
        .collect();
    text(&kept)
}

fn losses_at<'a>(out: &'a ReadOutcome, location: &str) -> Vec<&'a Loss> {
    out.document
        .losses
        .iter()
        .filter(|l| l.location == location)
        .collect()
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(data).expect("deflate");
    z.finish().expect("deflate")
}

/// A one-page file whose content stream is `packed` (deflated), with Helvetica as `/F1`.
/// `extra` objects are added first; `resources` goes into the page's resource dictionary.
fn one_page(packed: &[u8], build: impl FnOnce(&mut Builder) -> String) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let more = build(&mut b);
    let content = b.stream("/Filter /FlateDecode", packed);
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> {more} >> /Contents {content} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

fn assert_rss_under_512_mib() {
    let rss = common::max_rss_bytes();
    assert!(rss < 512 << 20, "max RSS {rss}");
}

#[test]
fn cjk_text_read() {
    // Chinese in a font that is not embedded, through the predefined CMap GBKp-EUC-H.
    let out = read(&corpus("pdfjs-cjk-gbk-issue3521.pdf"));
    assert_eq!(out.completeness, Completeness::Complete);
    let t = text(page(&out, 1));
    assert!(t.contains("我们都是黑体字"), "{t}");
}

#[test]
fn cid_font_without_tounicode_uses_embedded_cmap() {
    // The ToUnicode CMap of the CID font maps nothing; the embedded TrueType program's cmap
    // table gives the characters.
    let out = read(&corpus("pdfjs-cid-truetype-empty-tounicode.pdf"));
    let t = text(page(&out, 1));
    assert!(t.contains("LINE"), "{t}");
    assert!(!t.contains('\u{FFFD}'), "{t}");
    assert!(
        out.document
            .losses
            .iter()
            .all(|l| !l.what.contains("without a Unicode mapping")),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn type3_glyphs_counted_or_read() {
    // A Type3 font whose glyphs are named square and triangle and that has no ToUnicode.
    let out = read(&corpus("pdfjs-simpletype3font.pdf"));
    let said: Vec<&str> = losses_at(&out, "p1")
        .iter()
        .filter(|l| l.severity == Severity::Info)
        .map(|l| l.what.as_str())
        .collect();
    assert!(
        said.contains(&"6 glyphs without a Unicode mapping on page 1"),
        "{said:?}"
    );
    // Each glyph keeps its place in the text.
    let t = text(page(&out, 1));
    assert_eq!(t.matches('\u{FFFD}').count(), 6, "{t}");
}

#[test]
fn vertical_text_in_column_order() {
    // Japanese set top to bottom in columns that read from right to left.
    let out = read(&corpus("pdfjs-vertical.pdf"));
    let t = text(page(&out, 1));
    let first = t.find("あいうえお").unwrap_or_else(|| panic!("{t}"));
    // The second column (further left) holds three kanji and comes after the first.
    let lines: Vec<&str> = t.lines().filter(|l| !l.starts_with("Page ")).collect();
    assert!(lines.len() >= 2, "{lines:?}");
    assert_eq!(lines[0], "あいうえお", "{lines:?}");
    assert_eq!(lines[1].chars().count(), 3, "{lines:?}");
    assert!(first < t.find(lines[1]).expect("second column"));
    assert!(!t.contains('\u{FFFD}'), "{t}");
}

#[test]
fn unmapped_glyphs_reported() {
    // A CID font that is not embedded, with the Identity ordering and no ToUnicode: nothing
    // says what its glyphs are.
    let content = deflate(
        b"BT /F2 12 Tf 72 700 Td <000100020003> Tj ET\n\
          BT /F1 12 Tf 72 650 Td (Mapped words.) Tj ET\n",
    );
    let bytes = one_page(&content, |b| {
        let desc = b.add(
            "<< /Type /FontDescriptor /FontName /NoSuchFont /Flags 4 \
             /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 \
             /CapHeight 700 /StemV 80 >>",
        );
        let cid = b.add(format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /NoSuchFont \
             /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
             /FontDescriptor {desc} 0 R /DW 1000 >>"
        ));
        let font = b.add(format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /NoSuchFont /Encoding /Identity-H \
             /DescendantFonts [{cid} 0 R] >>"
        ));
        format!("/Font << /F2 {font} 0 R >>")
    });
    let out = read(&bytes);
    let t = text(page(&out, 1));
    assert!(t.contains("Mapped words."), "{t}");
    assert_eq!(t.matches('\u{FFFD}').count(), 3, "{t}");
    let infos: Vec<&str> = losses_at(&out, "p1")
        .iter()
        .filter(|l| l.severity == Severity::Info && l.why == LossReason::Unsupported)
        .map(|l| l.what.as_str())
        .collect();
    assert_eq!(
        infos,
        ["3 glyphs without a Unicode mapping on page 1"],
        "{:?}",
        out.document.losses
    );
}

#[test]
fn invisible_text_on_mixed_page_is_read() {
    let content = format!(
        "{}BT /F1 12 Tf 3 Tr 72 600 Td (Hidden words on a mixed page) Tj ET\n",
        show("F1", 12.0, 72.0, 700.0, "Visible words on page one."),
    );
    let out = read(&pdf(&[Page::text(content)], ""));
    let one = page(&out, 1);
    assert!(body_text(one).contains("Visible words on page one."));
    assert!(!body_text(one).contains("Hidden words"));
    match one.last() {
        Some(Block::Callout {
            kind: CalloutKind::Details,
            blocks,
            ..
        }) => assert!(
            text(blocks).contains("Hidden words on a mixed page"),
            "{blocks:?}"
        ),
        other => panic!("no invisible text callout: {other:?}"),
    }
    // Nothing is left unread, so nothing says so.
    assert!(
        out.document
            .losses
            .iter()
            .all(|l| !l.what.contains("not read")),
        "{:?}",
        out.document.losses
    );
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn font_bomb_stops_at_glyph_cap() {
    // Form 0 shows 1,000 glyphs; each further form calls the one below it ten times, so the
    // page would draw 10^9 glyphs.
    let start = Instant::now();
    let content = deflate(b"BT /F1 12 Tf 72 700 Td (Before the bomb.) Tj ET\n/X6 Do\n");
    let bytes = one_page(&content, |b| {
        let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
        let glyphs = "a".repeat(1_000);
        let mut below = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] \
                 /Resources << /Font << /F1 {font} 0 R >> >>"
            ),
            format!("BT /F1 1 Tf 0 0 Td ({glyphs}) Tj ET").as_bytes(),
        );
        for _ in 0..6 {
            let calls = "/X Do\n".repeat(10);
            below = b.stream(
                &format!(
                    "/Type /XObject /Subtype /Form /BBox [0 0 595 842] \
                     /Resources << /XObject << /X {below} 0 R >> >>"
                ),
                calls.as_bytes(),
            );
        }
        format!("/XObject << /X6 {below} 0 R >>")
    });
    let out = read_with(&bytes, small_memory());
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(text(page(&out, 1)).contains("Before the bomb."));
    let limits: Vec<&Loss> = losses_at(&out, "p1")
        .into_iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(limits.len(), 1, "{:?}", out.document.losses);
    assert!(limits[0].what.contains("page 1"), "{:?}", limits[0]);
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    assert_rss_under_512_mib();
}

/// `n` bare `T*` operators after `before`, deflated.
fn t_star_flood(before: &str, n: usize) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(before.as_bytes()).expect("deflate");
    z.write_all(b"BT\n").expect("deflate");
    let chunk = "T*\n".repeat(100_000);
    let mut left = n;
    while left > 0 {
        let k = left.min(100_000);
        z.write_all(&chunk.as_bytes()[..3 * k]).expect("deflate");
        left -= k;
    }
    z.write_all(b"ET\n").expect("deflate");
    z.finish().expect("deflate")
}

#[test]
fn operator_flood_stops_at_the_work_cap() {
    // 999,990 operators that draw nothing, in a file of a few kilobytes.
    let bytes = one_page(&t_star_flood("", 999_990), |_| String::new());
    assert!(bytes.len() < 20_000, "{}", bytes.len());
    let limits = Limits {
        max_elements: 500_000,
        ..small_memory()
    };
    let out = read_with(&bytes, limits);
    assert_eq!(out.completeness, Completeness::Partial);
    let cut: Vec<&Loss> = losses_at(&out, "p1")
        .into_iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(cut.len(), 1, "{:?}", out.document.losses);
    assert!(cut[0].what.contains("page 1"), "{:?}", cut[0]);
    // With the default limits, a flood past the cap is cut the same way.
    let out = read(&one_page(&t_star_flood("", 2_500_000), |_| String::new()));
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(
        losses_at(&out, "p1")
            .iter()
            .any(|l| l.why == LossReason::Limit),
        "{:?}",
        out.document.losses
    );
    assert_rss_under_512_mib();
}

#[test]
fn visible_sentence_before_a_flood_is_read() {
    let before = "BT /F1 12 Tf 72 700 Td (Visible words on the page.) Tj ET\n";
    let out = read_with(
        &one_page(&t_star_flood(before, 1_000_010), |_| String::new()),
        small_memory(),
    );
    assert!(
        text(page(&out, 1)).contains("Visible words on the page."),
        "{:?}",
        out.document.losses
    );
    assert_rss_under_512_mib();
}

#[test]
fn flood_past_the_cap_keeps_the_text_before_it() {
    let before = "BT /F1 12 Tf 72 700 Td (Visible words on the page.) Tj ET\n";
    let out = read(&one_page(&t_star_flood(before, 2_500_000), |_| {
        String::new()
    }));
    assert!(text(page(&out, 1)).contains("Visible words on the page."));
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn many_short_lines_on_one_page_are_bounded() {
    // About 88 KB: two-letter strings, each on its own line.
    let start = Instant::now();
    let mut content = b"BT /F1 12 Tf 72 800 Td\n".to_vec();
    for _ in 0..466_000 {
        content.extend_from_slice(b"(ab)Tj 0 1 Td\n");
    }
    content.extend_from_slice(b"ET\n");
    let packed = deflate(&content);
    let bytes = one_page(&packed, |_| String::new());
    let out = read_with(&bytes, small_memory());
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert!(matches!(
        out.completeness,
        Completeness::Complete | Completeness::Partial
    ));
    assert!(text(page(&out, 1)).contains("ab"));
    assert_rss_under_512_mib();
}

/// About `n` bytes of content that deflate 50 to 90 times: drawing operators with a changing
/// number now and then.
fn big_content(n: usize) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut seed: u64 = 7;
    let mut written = 0;
    let mut line = Vec::with_capacity(64);
    while written < n {
        line.clear();
        for _ in 0..16 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            line.extend_from_slice(b"0 0 m 1 1 l S\n");
        }
        line.extend_from_slice(format!("% {}\n", seed >> 40).as_bytes());
        z.write_all(&line).expect("deflate");
        written += line.len();
    }
    z.finish().expect("deflate")
}

#[test]
fn huge_content_stream_inside_the_guard_is_bounded() {
    let packed = big_content(300 << 20);
    let ratio = (300u64 << 20) / packed.len() as u64;
    assert!((20..100).contains(&ratio), "ratio {ratio}");
    let bytes = one_page(&packed, |_| String::new());
    let out = read_with(&bytes, small_memory());
    assert_eq!(out.completeness, Completeness::Partial);
    let cut: Vec<&Loss> = losses_at(&out, "p1")
        .into_iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(cut.len(), 1, "{:?}", out.document.losses);
    assert!(cut[0].what.contains("page 1"), "{:?}", cut[0]);
    assert_rss_under_512_mib();
}

#[test]
fn pdf_inspector_not_in_tree() {
    // The resolved dependency tree: pdf-inspector and the ttf-parser it brought are gone.
    let lock = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../Cargo.lock"),
    )
    .expect("Cargo.lock");
    for name in ["pdf-inspector", "ttf-parser"] {
        assert!(
            !lock.contains(&format!("name = \"{name}\"")),
            "{name} is still in Cargo.lock"
        );
    }
    assert!(lock.contains("name = \"hayro-interpret\""));
}
