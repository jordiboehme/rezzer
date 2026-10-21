// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small files that try to make the text reader build too much, and content the interpreter
//! skips or cuts, which must always say so.

mod common;

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::Builder;
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Block, Bytes, CalloutKind, Completeness, Confidence, Detection, FormatId, Inline, Loss,
    LossReason, ReadOptions, ReadOutcome, Reader, SectionKind, Severity, Source, plain_text,
};
use rezzer_read_pdf::PdfReader;

/// The brief's hostile setting: 512 MiB of memory, so pdfparts keeps its objects to 256 MiB.
fn hostile() -> Limits {
    Limits {
        max_memory_bytes: 512 << 20,
        max_time: Duration::from_secs(120),
        ..Limits::default()
    }
}

fn read(bytes: &[u8]) -> ReadOutcome {
    read_with(bytes, &Budget::new(hostile()))
}

fn read_with(bytes: &[u8], budget: &Budget) -> ReadOutcome {
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
    };
    PdfReader.read(&src, &ReadOptions::default(), budget)
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(data).expect("deflate");
    z.finish().expect("deflate")
}

/// A one-page file with Helvetica as `/F1`. `build` adds objects and returns more resource
/// entries and catalog entries; its `F1` argument is the font's object number.
fn one_page(content: &[u8], build: impl FnOnce(&mut Builder, u32) -> (String, String)) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let (more, catalog_extra) = build(&mut b, font);
    let packed = deflate(content);
    let c = b.stream("/Filter /FlateDecode", &packed);
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> {more} >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(
        catalog,
        format!("<< /Type /Catalog /Pages {tree} 0 R {catalog_extra} >>"),
    );
    b.finish(catalog)
}

fn nothing(_: &mut Builder, _: u32) -> (String, String) {
    (String::new(), String::new())
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

fn body_text(blocks: &[Block]) -> String {
    let kept: Vec<Block> = blocks
        .iter()
        .filter(|b| !matches!(b, Block::Callout { .. }))
        .cloned()
        .collect();
    text(&kept)
}

fn losses(out: &ReadOutcome, why: LossReason) -> Vec<&Loss> {
    out.document
        .losses
        .iter()
        .filter(|l| l.location == "p1" && l.why == why)
        .collect()
}

/// The bound the memory tests hold: a quarter of the hostile memory limit.
fn assert_rss_under_256_mib() {
    let rss = common::max_rss_bytes();
    assert!(rss < 256 << 20, "max RSS {} MiB", rss >> 20);
}

/// The read is cut with one Limit loss on page 1, keeps the text before the cut and stays
/// small and quick.
fn assert_cut(out: &ReadOutcome, start: Instant) {
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(out.completeness, Completeness::Partial);
    let cut = losses(out, LossReason::Limit);
    assert_eq!(cut.len(), 1, "{:?}", out.document.losses);
    assert!(cut[0].what.contains("page 1"), "{:?}", cut[0]);
    assert!(
        text(page(out, 1)).contains("Before."),
        "{:?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

/// One show of `n` glyphs after `setup`, in a page that first shows `Before.`.
fn big_show(setup: &str, glyph: u8, n: usize) -> Vec<u8> {
    let mut c = format!("BT /F1 12 Tf 72 700 Td (Before.) Tj {setup} 72 600 Td (").into_bytes();
    c.extend(std::iter::repeat_n(glyph, n));
    c.extend(b") Tj ET\n");
    c
}

#[test]
fn clip_mode_show_is_bounded() {
    // Render mode 4 (fill and clip) makes the interpreter build every glyph outline.
    let start = Instant::now();
    let out = read(&one_page(&big_show("4 Tr", b'W', 500_000), nothing));
    assert_cut(&out, start);
}

#[test]
fn clip_mode_show_in_a_form_is_bounded() {
    // The same inside a form, where the page content cannot be changed: the font program's
    // outlines are built for each glyph.
    let start = Instant::now();
    let mut form = b"BT /F1 12 Tf 4 Tr 72 600 Td (".to_vec();
    form.extend(std::iter::repeat_n(b'W', 500_000));
    form.extend(b") Tj ET");
    let content = b"BT /F1 12 Tf 72 700 Td (Before.) Tj ET\n/X Do\n";
    let out = read(&one_page(content, |b, font| {
        let x = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] \
                 /Resources << /Font << /F1 {font} 0 R >> >>"
            ),
            &form,
        );
        (format!("/XObject << /X {x} 0 R >>"), String::new())
    }));
    assert_cut(&out, start);
}

#[test]
fn type3_show_is_bounded() {
    let start = Instant::now();
    let content = big_show("/T3 12 Tf", b'a', 500_000);
    let out = read(&one_page(&content, |b, _| {
        let proc_ = b.stream("", b"500 0 0 0 500 500 d1 0 0 500 500 re f");
        let f = b.add(format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 500 500] \
             /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a {proc_} 0 R >> \
             /Encoding << /Type /Encoding /Differences [97 /a] >> /FirstChar 97 \
             /LastChar 97 /Widths [500] >>"
        ));
        (format!("/Font << /T3 {f} 0 R >>"), String::new())
    }));
    assert_cut(&out, start);
}

#[test]
fn spread_glyphs_are_bounded() {
    // Character spacing puts every glyph in a run of its own.
    let start = Instant::now();
    let out = read(&one_page(&big_show("30 Tc", b'a', 1_900_000), nothing));
    assert_cut(&out, start);
}

#[test]
fn shared_actual_text_is_bounded() {
    // One property with 2,000,000 bytes of ActualText, named by 300 spans.
    let start = Instant::now();
    let mut c = b"BT /F1 12 Tf 72 700 Td (Before.) Tj 72 600 Td ".to_vec();
    for _ in 0..300 {
        c.extend(b"/Span /P1 BDC (x) Tj EMC ");
    }
    c.extend(b"ET\n");
    let actual = "y".repeat(2_000_000);
    let out = read(&one_page(&c, |b, _| {
        let p = b.add(format!("<< /ActualText ({actual}) >>"));
        (format!("/Properties << /P1 {p} 0 R >>"), String::new())
    }));
    assert_cut(&out, start);
}

/// A TrueType font program with `n` empty glyphs whose cmap maps U+20000 onwards to glyphs 1
/// and up: little data that describes many glyphs.
fn many_glyph_font(n: u16) -> Vec<u8> {
    fn be16(v: &mut Vec<u8>, x: u16) {
        v.extend(x.to_be_bytes());
    }
    fn be32(v: &mut Vec<u8>, x: u32) {
        v.extend(x.to_be_bytes());
    }
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0);
    be16(&mut head, 1000);
    head.extend([0u8; 16]);
    for _ in 0..4 {
        be16(&mut head, 0);
    }
    be16(&mut head, 0);
    be16(&mut head, 8);
    be16(&mut head, 2);
    be16(&mut head, 0);
    be16(&mut head, 0);
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    be16(&mut hhea, 800);
    be16(&mut hhea, 0xFF38);
    be16(&mut hhea, 0);
    be16(&mut hhea, 1000);
    hhea.extend([0u8; 22]);
    be16(&mut hhea, 1);
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, n);
    maxp.extend([0u8; 26]);
    let mut hmtx = Vec::new();
    be16(&mut hmtx, 1000);
    be16(&mut hmtx, 0);
    for _ in 1..n {
        be16(&mut hmtx, 0);
    }
    let mut loca = Vec::new();
    for _ in 0..=n {
        be16(&mut loca, 0);
    }
    let mut cmap = Vec::new();
    be16(&mut cmap, 0);
    be16(&mut cmap, 1);
    be16(&mut cmap, 3);
    be16(&mut cmap, 10);
    be32(&mut cmap, 12);
    be16(&mut cmap, 12);
    be16(&mut cmap, 0);
    be32(&mut cmap, 28);
    be32(&mut cmap, 0);
    be32(&mut cmap, 1);
    be32(&mut cmap, 0x2_0000);
    be32(&mut cmap, 0x2_0000 + u32::from(n) - 2);
    be32(&mut cmap, 1);
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    post.extend([0u8; 28]);
    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap),
        (b"glyf", Vec::new()),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"post", post),
    ];
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, tables.len() as u16);
    be16(&mut out, 128);
    be16(&mut out, 3);
    be16(&mut out, 0);
    let mut offset = 12 + 16 * tables.len() as u32;
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend(*tag);
        be32(&mut out, 0);
        be32(&mut out, offset);
        be32(&mut out, data.len() as u32);
        body.extend(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * tables.len() as u32 + body.len() as u32;
    }
    out.extend(body);
    out
}

#[test]
fn glyph_names_of_a_shared_font_program_are_built_once() {
    // 200 CID font dictionaries share one small program that describes 65,535 glyphs, and
    // none has a ToUnicode CMap.
    let start = Instant::now();
    let program = many_glyph_font(65_535);
    let mut c = b"BT /F1 12 Tf 72 700 Td (Before.) Tj ".to_vec();
    for i in 0..200 {
        c.extend(format!("/G{i} 12 Tf <0001> Tj ").as_bytes());
    }
    c.extend(b"ET\n");
    let out = read(&one_page(&c, |b, _| {
        let ff = b.stream(&format!("/Length1 {}", program.len()), &program);
        let desc = b.add(format!(
            "<< /Type /FontDescriptor /FontName /Probe /Flags 4 /FontBBox [0 0 1000 1000] \
             /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 \
             /FontFile2 {ff} 0 R >>"
        ));
        let mut fonts = String::new();
        for i in 0..200 {
            let cid = b.add(format!(
                "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Probe \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                 /FontDescriptor {desc} 0 R /DW 1000 /CIDToGIDMap /Identity >>"
            ));
            let f = b.add(format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /Probe /Encoding /Identity-H \
                 /DescendantFonts [{cid} 0 R] >>"
            ));
            fonts.push_str(&format!("/G{i} {f} 0 R "));
        }
        (format!("/Font << {fonts} >>"), String::new())
    }));
    let t = text(page(&out, 1));
    assert!(t.contains("Before."), "{t}");
    assert_eq!(
        t.matches('\u{20000}').count(),
        200,
        "{:?}",
        out.document.losses
    );
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    assert_rss_under_256_mib();
}

/// A page whose catalog switches the optional content group `L1` off. `build` writes the
/// content's extra objects, with the group's object number.
fn with_hidden_group(
    content: &[u8],
    build: impl FnOnce(&mut Builder, u32, u32) -> String,
) -> Vec<u8> {
    one_page(content, |b, font| {
        let group = b.add("<< /Type /OCG /Name (Hidden) >>");
        let more = build(b, font, group);
        (
            format!("/Properties << /L1 {group} 0 R >> {more}"),
            format!("/OCProperties << /OCGs [{group} 0 R] /D << /OFF [{group} 0 R] >> >>"),
        )
    })
}

#[test]
fn text_in_a_switched_off_layer_is_read_into_the_callout() {
    let content = b"BT /F1 12 Tf 72 700 Td (Visible words.) Tj ET\n\
        /OC /L1 BDC BT /F1 12 Tf 72 600 Td (Words in the hidden layer.) Tj ET EMC\n";
    let out = read(&with_hidden_group(content, |_, _, _| String::new()));
    let one = page(&out, 1);
    assert!(body_text(one).contains("Visible words."));
    assert!(!body_text(one).contains("hidden layer"), "{one:?}");
    match one.last() {
        Some(Block::Callout {
            kind: CalloutKind::Details,
            blocks,
            ..
        }) => {
            let t = text(blocks);
            assert!(t.contains("Words in the hidden layer."), "{t}");
            assert!(t.contains("Hidden"), "{t}");
        }
        other => panic!("no callout: {other:?}"),
    }
    let infos: Vec<&str> = losses(&out, LossReason::Unsupported)
        .iter()
        .filter(|l| l.severity == Severity::Info)
        .map(|l| l.what.as_str())
        .collect();
    assert!(
        infos
            .iter()
            .any(|w| w.starts_with("1 run in hidden layer Hidden")),
        "{infos:?}"
    );
}

#[test]
fn a_hidden_form_does_not_shift_actual_text() {
    let content = b"/X Do\nBT /F1 12 Tf 72 600 Td /Span << /ActualText (RIGHT) >> BDC \
        (visible) Tj EMC ET\n";
    let out = read(&with_hidden_group(content, |b, font, group| {
        let x = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /OC {group} 0 R \
                 /Resources << /Font << /F1 {font} 0 R >> >>"
            ),
            b"BT /F1 12 Tf 72 700 Td /Span << /ActualText (WRONG) >> BDC (hidden) Tj EMC ET",
        );
        format!("/XObject << /X {x} 0 R >>")
    }));
    let t = text(page(&out, 1));
    assert!(t.contains("RIGHT"), "{t}");
    assert!(!t.contains("WRONG"), "{t}");
    // The hidden form's text is not read, and that is said.
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.location == "p1" && l.what.contains("hidden layer Hidden")),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn a_broken_operator_ends_the_page_with_a_loss() {
    let out = read(&one_page(
        b"BT /F1 12 Tf 72 700 Td (Before.) Tj ET\n1 m\n\
          BT /F1 12 Tf 72 600 Td (After the break.) Tj ET\n",
        nothing,
    ));
    assert!(text(page(&out, 1)).contains("Before."));
    let damaged = losses(&out, LossReason::Damaged);
    assert_eq!(damaged.len(), 1, "{:?}", out.document.losses);
    assert!(damaged[0].what.contains("page 1"), "{:?}", damaged[0]);
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn a_form_without_a_bounding_box_gets_a_loss() {
    let content = b"BT /F1 12 Tf 72 700 Td (Before.) Tj ET\n/X Do\n";
    let out = read(&one_page(content, |b, font| {
        let x = b.stream(
            &format!("/Type /XObject /Subtype /Form /Resources << /Font << /F1 {font} 0 R >> >>"),
            b"BT /F1 12 Tf 72 600 Td (In the form.) Tj ET",
        );
        (format!("/XObject << /X {x} 0 R >>"), String::new())
    }));
    let damaged = losses(&out, LossReason::Damaged);
    assert_eq!(damaged.len(), 1, "{:?}", out.document.losses);
    assert!(damaged[0].what.contains("bounding box"), "{:?}", damaged[0]);
}

#[test]
fn forms_nested_too_deep_get_a_loss() {
    let content = b"BT /F1 12 Tf 72 700 Td (Before.) Tj ET\n/X Do\n";
    let out = read(&one_page(content, |b, font| {
        let mut below = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] \
                 /Resources << /Font << /F1 {font} 0 R >> >>"
            ),
            b"BT /F1 12 Tf 72 600 Td (Deep down.) Tj ET",
        );
        for _ in 0..60 {
            below = b.stream(
                &format!(
                    "/Type /XObject /Subtype /Form /BBox [0 0 595 842] \
                     /Resources << /XObject << /X {below} 0 R >> >>"
                ),
                b"/X Do",
            );
        }
        (format!("/XObject << /X {below} 0 R >>"), String::new())
    }));
    let cut = losses(&out, LossReason::Limit);
    assert_eq!(cut.len(), 1, "{:?}", out.document.losses);
    assert!(
        cut[0].what.contains("nested deeper than 50"),
        "{:?}",
        cut[0]
    );
}

#[test]
fn a_broken_embedded_cid_font_counts_its_glyphs_as_unmapped() {
    let content = b"BT /F2 12 Tf 72 700 Td <002B00480049> Tj ET\n";
    let out = read(&one_page(content, |b, _| {
        let ff = b.stream("/Length1 64", &[0x42u8; 64]);
        let desc = b.add(format!(
            "<< /Type /FontDescriptor /FontName /Junk /Flags 4 /FontBBox [0 0 1000 1000] \
             /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 \
             /FontFile2 {ff} 0 R >>"
        ));
        let cid = b.add(format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Junk \
             /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
             /FontDescriptor {desc} 0 R /DW 1000 >>"
        ));
        let f = b.add(format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /Junk /Encoding /Identity-H \
             /DescendantFonts [{cid} 0 R] >>"
        ));
        (format!("/Font << /F2 {f} 0 R >>"), String::new())
    }));
    let t = text(page(&out, 1));
    assert_eq!(t.matches('\u{FFFD}').count(), 3, "{t}");
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what == "3 glyphs without a Unicode mapping on page 1"),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn an_overprint_with_other_letters_is_kept() {
    let content = b"BT /F1 12 Tf 72 700 Td (abc) Tj ET\nBT /F1 12 Tf 72 700 Td (xyz) Tj ET\n";
    let out = read(&one_page(content, nothing));
    let t = text(page(&out, 1));
    assert!(t.contains("abc") && t.contains("xyz"), "{t}");
}

#[test]
fn links_on_a_turned_page_are_attached() {
    // All text runs upward, so the page is read in a turned frame; the link over its glyphs
    // still lands on its words.
    let content = b"BT /F1 12 Tf 0 1 -1 0 300 100 Tm (Read the guide on the web site) Tj ET\n";
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let link = b.add(
        "<< /Type /Annot /Subtype /Link /Rect [280 90 310 300] \
         /A << /S /URI /URI (https://example.org/guide) >> >>",
    );
    let packed = deflate(content);
    let c = b.stream("/Filter /FlateDecode", &packed);
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R /Annots [{link} 0 R] >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    let blocks = page(&out, 1);
    assert!(text(blocks).contains("Read the guide"));
    let linked = blocks.iter().any(|b| match b {
        Block::Paragraph { inlines, .. } => inlines.iter().any(
            |i| matches!(i, Inline::Link { target, .. } if target == "https://example.org/guide"),
        ),
        _ => false,
    });
    assert!(linked, "{blocks:#?}");
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.location == "p1" && l.what.contains("URI link")),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn glyphs_just_under_the_cap_stay_small() {
    // 220,000 glyphs, each a run of its own: under the cap of about 223,000 glyphs that a
    // 512 MiB read gets, so all of them are read.
    let start = Instant::now();
    let out = read(&one_page(&big_show("30 Tc", b'a', 220_000), nothing));
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert!(
        losses(&out, LossReason::Limit).is_empty(),
        "{:?}",
        out.document.losses
    );
    assert!(text(page(&out, 1)).contains("Before."));
    assert_rss_under_256_mib();
}

#[test]
fn clipping_glyphs_in_a_form_just_under_the_cap_stay_small() {
    // 27,000 glyphs that clip inside a form count eight times each: just under the cap. The
    // interpreter builds the font program's outline for every one of them.
    let start = Instant::now();
    let mut form = b"BT /F1 12 Tf 4 Tr 72 600 Td (".to_vec();
    form.extend(std::iter::repeat_n(b'W', 27_000));
    form.extend(b") Tj ET");
    let content = b"BT /F1 12 Tf 72 700 Td (Before.) Tj ET\n/X Do\n";
    let out = read(&one_page(content, |b, font| {
        let x = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] \
                 /Resources << /Font << /F1 {font} 0 R >> >>"
            ),
            &form,
        );
        (format!("/XObject << /X {x} 0 R >>"), String::new())
    }));
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert!(
        losses(&out, LossReason::Limit).is_empty(),
        "{:?}",
        out.document.losses
    );
    assert!(text(page(&out, 1)).contains("WWWW"));
    assert_rss_under_256_mib();
}

#[test]
fn type3_font_set_through_a_graphics_state_is_bounded() {
    // The font comes from an ExtGState's /Font entry, not from Tf.
    let start = Instant::now();
    let content = big_show("/G0 gs", b'a', 223_000);
    let out = read(&one_page(&content, |b, _| {
        let proc_ = b.stream("", b"500 0 0 0 500 500 d1 0 0 500 500 re f");
        let f = b.add(format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 500 500] \
             /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a {proc_} 0 R >> \
             /Encoding << /Type /Encoding /Differences [97 /a] >> /FirstChar 97 \
             /LastChar 97 /Widths [500] >>"
        ));
        (
            format!("/ExtGState << /G0 << /Font [{f} 0 R 12] >> >>"),
            String::new(),
        )
    }));
    assert_cut(&out, start);
}

/// A file of `pages` pages that all share one content stream: `Before.`, then either one
/// show of `glyphs` glyphs each in a run of its own, or one span whose ActualText is a named
/// property of `actual` bytes.
fn many_pages(pages: usize, glyphs: usize, actual: usize) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut c = b"BT /F1 12 Tf 72 700 Td (Before.) Tj 30 Tc 72 600 Td ".to_vec();
    let props = if actual > 0 {
        let p = b.add(format!("<< /ActualText ({}) >>", "y".repeat(actual)));
        c.extend(b"/Span /P1 BDC (x) Tj EMC ");
        format!("/Properties << /P1 {p} 0 R >>")
    } else {
        c.push(b'(');
        c.extend(std::iter::repeat_n(b'a', glyphs));
        c.extend(b") Tj ");
        String::new()
    };
    c.extend(b"ET\n");
    let packed = deflate(&c);
    let cs = b.stream("/Filter /FlateDecode", &packed);
    let mut kids = String::new();
    for _ in 0..pages {
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> {props} >> /Contents {cs} 0 R >>"
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

/// The read keeps page 1, stops keeping text on a later page with one Limit loss that names
/// the pages not read, and stays small.
fn assert_file_cut(out: &ReadOutcome, pages: usize, start: Instant) {
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(text(page(out, 1)).contains("Before."));
    let cut: Vec<&Loss> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(cut.len(), 1, "{:?}", out.document.losses);
    assert!(cut[0].what.contains(&format!("to {pages}")), "{:?}", cut[0]);
    assert_rss_under_256_mib();
}

#[test]
fn glyphs_spread_over_five_pages_are_bounded() {
    let start = Instant::now();
    let out = read(&many_pages(5, 220_000, 0));
    assert_file_cut(&out, 5, start);
}

#[test]
fn glyphs_spread_over_nine_pages_are_bounded() {
    let start = Instant::now();
    let out = read(&many_pages(9, 220_000, 0));
    assert_file_cut(&out, 9, start);
}

#[test]
fn glyphs_spread_over_twenty_pages_are_bounded() {
    let start = Instant::now();
    let out = read(&many_pages(20, 220_000, 0));
    assert_file_cut(&out, 20, start);
}

#[test]
fn actual_text_used_on_130_pages_is_bounded() {
    let start = Instant::now();
    let out = read(&many_pages(130, 0, 1_900_000));
    assert_file_cut(&out, 130, start);
}

#[test]
fn actual_text_used_on_200_pages_is_bounded() {
    let start = Instant::now();
    let out = read(&many_pages(200, 0, 1_900_000));
    assert_file_cut(&out, 200, start);
}

#[test]
fn a_real_rz_tag_stays_visible_text() {
    // The file's own /Rz0 tags look like the tag a switched-off layer is rewritten to.
    let content = b"/OC /L1 BDC BT /F1 12 Tf 72 700 Td (Hidden words.) Tj ET EMC\n\
        /Rz0 BMC BT /F1 12 Tf 72 600 Td (Visible page words.) Tj ET EMC\n/X Do\n";
    let out = read(&with_hidden_group(content, |b, font, _| {
        let x = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 595 842] \
                 /Resources << /Font << /F1 {font} 0 R >> >>"
            ),
            b"/Rz0 <<>> BDC BT /F1 12 Tf 72 500 Td (Visible form words.) Tj ET EMC",
        );
        format!("/XObject << /X {x} 0 R >>")
    }));
    let one = page(&out, 1);
    let body = body_text(one);
    assert!(body.contains("Visible page words."), "{body}");
    assert!(body.contains("Visible form words."), "{body}");
    assert!(!body.contains("Hidden words."), "{body}");
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what.starts_with("1 run in hidden layer Hidden")),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn a_switched_off_span_keeps_its_actual_text() {
    // An inline property list that names the group (object 4, see `with_hidden_group`) and
    // carries ActualText for its glyphs.
    let content = b"BT /F1 12 Tf 72 700 Td (Visible words.) Tj ET\n\
        /OC << /OC 4 0 R /ActualText (Shown instead) >> BDC \
        BT /F1 12 Tf 72 600 Td (xyz) Tj ET EMC\n";
    let out = read(&with_hidden_group(content, |_, _, group| {
        assert_eq!(group, 4);
        String::new()
    }));
    let all = text(page(&out, 1));
    assert!(all.contains("Shown instead"), "{all}");
    assert!(!all.contains("xyz"), "{all}");
    assert!(!body_text(page(&out, 1)).contains("Shown instead"));
}

/// One page with `runs` glyphs each in a run of its own, under one link over the whole page
/// whose URI has `uri` bytes.
fn page_under_a_long_link(runs: usize, uri: usize) -> Vec<u8> {
    let mut c = b"BT /F1 12 Tf 72 700 Td (Before.) Tj 30 Tc 72 600 Td (".to_vec();
    c.extend(std::iter::repeat_n(b'a', runs));
    c.extend(b") Tj ET\n");
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let annot = b.add(format!(
        "<< /Type /Annot /Subtype /Link /Rect [-1000000000 -1000000000 1000000000 1000000000] \
         /A << /S /URI /URI (http://x/{}) >> >>",
        "u".repeat(uri)
    ));
    let packed = deflate(&c);
    let cs = b.stream("/Filter /FlateDecode", &packed);
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Annots [{annot} 0 R] /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn a_long_link_over_many_runs_is_bounded() {
    // About 20 KB: 20,000 runs under one link with a 20,000 byte URI.
    let start = Instant::now();
    let bytes = page_under_a_long_link(20_000, 20_000);
    assert!(bytes.len() < 30_000, "{}", bytes.len());
    let out = read(&bytes);
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert!(
        text(page(&out, 1)).contains("Before."),
        "{:?}",
        out.document.losses
    );
    // The runs under the link make one link per paragraph, each with one copy of its target.
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
    let mut links = 0;
    for b in page(&out, 1) {
        if let Block::Paragraph { inlines, .. } = b {
            links += inlines
                .iter()
                .filter(|i| matches!(i, rezzer_model::Inline::Link { .. }))
                .count();
        }
    }
    assert!((1..=2).contains(&links), "{links} links");
    assert_rss_under_256_mib();
}

#[test]
fn a_very_long_link_over_many_runs_is_bounded() {
    // About 100 KB: 50,000 runs under one link with a 100,000 byte URI.
    let start = Instant::now();
    let bytes = page_under_a_long_link(50_000, 100_000);
    assert!(bytes.len() < 120_000, "{}", bytes.len());
    let out = read(&bytes);
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert!(
        text(page(&out, 1)).contains("Before."),
        "{:?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

/// Pages of short lines far apart, one paragraph each: page k has `lines[k]` lines `w<i>.`.
/// With `uri`, one link with a URI of that many bytes lies over each page.
fn far_lines(lines: &[usize], uri: Option<usize>) -> Vec<u8> {
    far_lines_of(lines, uri.map(|m| (m, "u")))
}

/// Like `far_lines`, with a URI made of `m` copies of `piece`, a PDF string body as written.
fn far_lines_of(lines: &[usize], uri: Option<(usize, &str)>) -> Vec<u8> {
    far_lines_at(lines, uri, "http://x/")
}

/// Like `far_lines_of`, with the URI starting with `prefix`.
fn far_lines_at(lines: &[usize], uri: Option<(usize, &str)>, prefix: &str) -> Vec<u8> {
    const GAP: usize = 40;
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let annots = match uri {
        Some((m, piece)) => {
            let annot = b.add(format!(
                "<< /Type /Annot /Subtype /Link \
                 /Rect [-1000000000 -1000000000 1000000000 1000000000] \
                 /A << /S /URI /URI ({prefix}{}) >> >>",
                piece.repeat(m)
            ));
            format!("/Annots [{annot} 0 R]")
        }
        None => String::new(),
    };
    let mut kids = String::new();
    for &n in lines {
        let h = (n + 2) * GAP;
        let mut c = b"BT /F1 12 Tf ".to_vec();
        for i in 0..n {
            c.extend(format!("1 0 0 1 72 {} Tm (w{i}.) Tj ", h - (i + 1) * GAP).as_bytes());
        }
        c.extend(b"ET\n");
        let packed = deflate(&c);
        let cs = b.stream("/Filter /FlateDecode", &packed);
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 {h}] \
             /Resources << /Font << /F1 {font} 0 R >> >> {annots} /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", lines.len()),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// One tall page of `n` lines `gap` points apart with `k` one-letter words each, and `k` link
/// stripes, each over one column of words and with a URI of its own: `http://x/<j>/` and
/// `len` spaces. Lines `gap` apart that close form one paragraph.
fn stripes(n: usize, gap: usize, k: usize, len: usize) -> Vec<u8> {
    stripes_after(0, n, gap, k, len)
}

/// Like `stripes`, with `pre` lines 40 points apart before them, one paragraph each.
fn stripes_after(pre: usize, n: usize, gap: usize, k: usize, len: usize) -> Vec<u8> {
    let h = pre * 40 + (n + 2) * gap;
    let mut c = b"BT /F1 12 Tf ".to_vec();
    for i in 0..pre + n {
        let y = if i < pre {
            h - (i + 1) * 40
        } else {
            h - pre * 40 - (i - pre + 1) * gap
        };
        for j in 0..k {
            c.extend(format!("1 0 0 1 {} {y} Tm (a) Tj ", 72 + 60 * j).as_bytes());
        }
    }
    c.extend(b"ET\n");
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut annots = String::new();
    for j in 0..k {
        let x = 72 + 60 * j;
        let a = b.add(format!(
            "<< /Type /Annot /Subtype /Link /Rect [{} -1000000000 {} 1000000000] \
             /A << /S /URI /URI (http://x/{j}/{}) >> >>",
            x as i64 - 5,
            x + 30,
            " ".repeat(len)
        ));
        annots.push_str(&format!("{a} 0 R "));
    }
    let packed = deflate(&c);
    let cs = b.stream("/Filter /FlateDecode", &packed);
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {h}] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Annots [{annots}] /Contents {cs} 0 R >>",
        72 + 60 * k + 72
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// A loss that says the time ran out, if there is one.
fn time_loss(out: &ReadOutcome) -> Option<&Loss> {
    out.document
        .losses
        .iter()
        .find(|l| l.why == LossReason::Limit && l.what.contains("time limit"))
}

/// The links in `blocks` and the bytes of their targets.
fn links_in(blocks: &[Block]) -> (usize, usize) {
    let (mut n, mut bytes) = (0, 0);
    for b in blocks {
        if let Block::Paragraph { inlines, .. } = b {
            for i in inlines {
                if let rezzer_model::Inline::Link { target, .. } = i {
                    n += 1;
                    bytes += target.len();
                }
            }
        }
    }
    (n, bytes)
}

/// The one Limit loss that says link targets were left out of the text. The link graph draws
/// on what the pages leave, so it may add one more for the links it could not keep.
fn link_loss(out: &ReadOutcome) -> &Loss {
    let (found, graph): (Vec<&Loss>, Vec<&Loss>) = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .partition(|l| l.what.contains("targets of"));
    assert_eq!(found.len(), 1, "{:?}", out.document.losses);
    assert!(
        graph.len() <= 1 && graph.iter().all(|l| l.what.contains("more link entr")),
        "{:?}",
        out.document.losses
    );
    found[0]
}

#[test]
fn links_over_many_paragraphs_are_bounded_through_render() {
    // About 107 KB: 1,250 paragraphs under one link with a 100,000 byte URI. Each paragraph
    // is a link of its own with its own copy of the target, and the renderer writes each
    // again.
    let start = Instant::now();
    let bytes = far_lines(&[1_250], Some(100_000));
    assert!(bytes.len() < 120_000, "{}", bytes.len());
    let out = read(&bytes);
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    let pages = rezzer_render::paginate(&md, hostile().page_tokens);
    assert!(!pages.is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let t = text(page(&out, 1));
    assert!(
        t.contains("w0.") && t.contains("w1249."),
        "{:?}",
        out.document.losses
    );
    let (n, target_bytes) = links_in(page(&out, 1));
    assert!(n > 0, "no link kept");
    assert!(target_bytes <= (512 << 20) / 16, "{target_bytes}");
    let loss = link_loss(&out);
    assert!(
        loss.what.contains(&format!("{} links", 1_250 - n)),
        "{loss:?}"
    );
    drop(pages);
    drop(md);
    assert_rss_under_256_mib();
}

/// Reads a file of `paras` paragraphs under one link with a URI of `m` copies of `piece` after
/// `prefix`, renders it as the pipeline does (and writes JSON when asked) and checks time,
/// loss and memory.
fn assert_link_pieces_bounded(prefix: &str, piece: &str, paras: usize, m: usize, json: bool) {
    let start = Instant::now();
    let bytes = far_lines_at(&[paras], Some((m, piece)), prefix);
    let out = read(&bytes);
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    let pages = rezzer_render::paginate(&md, hostile().page_tokens);
    drop(md);
    if json {
        let j = serde_json::to_string_pretty(&out.document).expect("the model serializes");
        assert!(!j.is_empty());
    }
    assert!(!pages.is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let t = text(page(&out, 1));
    assert!(
        t.contains("w0.") && t.contains(&format!("w{}.", paras - 1)),
        "{:?}",
        out.document.losses
    );
    let (n, _) = links_in(page(&out, 1));
    let loss = link_loss(&out);
    assert!(
        loss.what.contains(&format!("{} links", paras - n)),
        "{loss:?}"
    );
    drop(pages);
    assert_rss_under_256_mib();
}

#[test]
fn link_targets_of_spaces_are_charged_by_what_the_renderer_writes() {
    // The renderer writes each space as three bytes.
    assert_link_pieces_bounded("http://x/", " ", 1_250, 100_000, false);
}

#[test]
fn link_targets_of_control_characters_are_bounded_through_json() {
    // The renderer writes each as three bytes, JSON as six.
    assert_link_pieces_bounded("http://x/", "\\001", 1_250, 100_000, true);
}

#[test]
fn link_targets_of_backticks_in_code_spans_are_bounded_through_json() {
    // A target with an unsafe scheme is written as a code span, three bytes per backtick.
    assert_link_pieces_bounded("javascript:", "`", 1_250, 100_000, true);
}

#[test]
fn many_short_paragraphs_with_backtick_code_span_targets_are_bounded() {
    assert_link_pieces_bounded("javascript:", "`", 20_000, 20_000, true);
}

#[test]
fn a_target_that_never_fits_is_not_scanned_for_every_paragraph() {
    // A 12 MB URI of spaces: its length fits the room, its written size (three times as
    // much) does not. Every paragraph must not scan it again, and the read must stay in time.
    let start = Instant::now();
    let out = read(&far_lines_of(&[40_000], Some((12_000_000, " "))));
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let (n, _) = links_in(page(&out, 1));
    assert_eq!(n, 0, "{:?}", out.document.losses);
    assert_rss_under_256_mib();
}

#[test]
fn alternating_long_targets_are_scanned_once_each() {
    // 24,000 paragraphs, five link stripes with a 3 MB URI of spaces each: the length of each
    // fits the room, its written size does not.
    let start = Instant::now();
    let out = read(&stripes(24_000, 40, 5, 3_000_000));
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    assert!(time_loss(&out).is_none(), "{:?}", out.document.losses);
    assert_eq!(links_in(page(&out, 1)).0, 0);
    assert_rss_under_256_mib();
}

#[test]
fn a_stop_inside_a_big_paragraph_keeps_the_blocks_before_it_with_agreeing_losses() {
    // 50 small paragraphs, then one paragraph of 199,750 runs under five links. The text
    // limit the test sets is far below the big paragraph and above every small one, so the
    // stop is reached for sure at the start of the big block. The link cuts of the small
    // paragraphs stay; the big block adds none.
    let budget = Budget::new(Limits {
        max_text_bytes: 100_000,
        ..hostile()
    });
    let out = read_with(&stripes_after(50, 39_950, 14, 5, 400_000), &budget);
    assert_eq!(out.completeness, Completeness::Partial);
    let kept = page(&out, 1);
    let n = kept
        .iter()
        .filter(|b| matches!(b, Block::Paragraph { role: None, .. }))
        .count();
    assert!(n >= 45, "{:?}", out.document.losses);
    let (links, _) = links_in(kept);
    let cut = losses(&out, LossReason::Limit);
    let stop = cut
        .iter()
        .find(|l| l.what.contains("text"))
        .unwrap_or_else(|| panic!("{:?}", out.document.losses));
    assert!(
        stop.what.contains(&format!("from block {} on", n + 1)),
        "{stop:?} n={n}"
    );
    // Five links in each kept paragraph: the cuts are the kept paragraphs' own.
    let cut_links = 5 * n - links;
    let link_loss = out
        .document
        .losses
        .iter()
        .find(|l| l.what.contains("kept without the link"))
        .unwrap_or_else(|| panic!("{:?}", out.document.losses));
    assert!(
        link_loss.what.contains(&format!("{cut_links} links")),
        "{link_loss:?}"
    );
    assert_rss_under_256_mib();
}

#[test]
fn links_over_2000_paragraphs_keep_the_page() {
    // The link copies once stopped the page and dropped all its blocks.
    let start = Instant::now();
    let out = read(&far_lines(&[2_000], Some(100_000)));
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let t = text(page(&out, 1));
    assert!(
        t.contains("w0.") && t.contains("w1999."),
        "{:?}",
        out.document.losses
    );
    let (n, _) = links_in(page(&out, 1));
    let loss = link_loss(&out);
    assert!(
        loss.what.contains(&format!("{} links", 2_000 - n)),
        "{loss:?}"
    );
    assert!(loss.what.contains("page 1"), "{loss:?}");
    assert_rss_under_256_mib();
}

#[test]
fn an_element_stop_keeps_the_blocks_before_it() {
    const N: usize = 400;
    let bytes = far_lines(&[N, 1, 1], None);
    let full = Budget::new(hostile());
    let out = read_with(&bytes, &full);
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
    // Pages 2 and 3 take four elements each: section, marker, run and block. The elements a
    // read counts first stand in for an earlier part of a larger read, so the work caps,
    // which come from the element limit, stay as they are.
    let at = full.elements() - 8 - N as u64 / 2;
    let cut = Budget::new(hostile());
    cut.add_elements(hostile().max_elements - at).expect("room");
    let out = read_with(&bytes, &cut);
    let one = page(&out, 1);
    // The marker and the blocks built before the stop.
    let kept = one.len() - 1;
    assert!(kept > 1 && kept < N, "{kept} blocks kept");
    let t = text(one);
    assert!(
        t.contains("w0.") && t.contains(&format!("w{}.", kept - 1)),
        "{t}"
    );
    assert!(!t.contains(&format!("w{kept}.")), "{t}");
    let cut = losses(&out, LossReason::Limit);
    assert_eq!(cut.len(), 1, "{:?}", out.document.losses);
    assert!(
        cut[0].what.ends_with(&format!(
            "page 1 from block {} on and pages 2 to 3 are not read",
            kept + 1
        )),
        "{:?}",
        cut[0]
    );
}

#[test]
fn a_tagged_table_of_40000_rows_is_bounded_through_render() {
    // 40,000 rows of one cell each under one link: 120,000 structure elements, an 8 MB file.
    // The objects take most of what the read may use, so the text gets less; the table and
    // its render add little to the peak of the read.
    let start = Instant::now();
    let bytes = common::tagged_lines(40_000, "table", Some("http://x/u"));
    let out = read(&bytes);
    let read_peak = common::max_rss_bytes();
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    let pages = rezzer_render::paginate(&md, hostile().page_tokens);
    assert!(!pages.is_empty());
    drop(pages);
    drop(md);
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let tables: Vec<usize> = page(&out, 1)
        .iter()
        .filter_map(|b| match b {
            Block::Table { rows, .. } => Some(rows.len()),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 1, "{:?}", out.document.losses);
    // Every row is kept, or a Limit loss on page 1 says the page was cut.
    if tables[0] < 40_000 {
        assert!(
            losses(&out, LossReason::Limit)
                .iter()
                .any(|l| l.what.contains("page 1")),
            "{} rows, {:?}",
            tables[0],
            out.document.losses
        );
    }
    let render_peak = common::max_rss_bytes();
    assert!(
        render_peak <= read_peak + (16 << 20),
        "read {} MiB, render {} MiB",
        read_peak >> 20,
        render_peak >> 20
    );
    assert_rss_under_256_mib();
}

#[test]
fn a_page_of_many_drawn_lines_is_not_searched_for_tables() {
    // 30,000 short lines in a lattice, with text in it.
    let start = Instant::now();
    let mut c = b"BT /F1 12 Tf 72 700 Td (Before.) Tj ET 0.1 w ".to_vec();
    for k in 0..15_000 {
        let (x, y) = (20 + (k % 150) * 3, 20 + (k / 150) * 6);
        c.extend(format!("{x} {y} m {} {y} l S {x} {y} m {x} {} l S ", x + 3, y + 6).as_bytes());
    }
    let out = read(&one_page(&c, nothing));
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("ruled tables on page 1")),
        "{:?}",
        out.document.losses
    );
    assert!(text(page(&out, 1)).contains("Before."));
    assert_rss_under_256_mib();
}

#[test]
fn a_dense_ruled_grid_is_bounded() {
    // 400 by 400 lines 3 points apart: 159,201 squares, more than a table may have.
    let start = Instant::now();
    let mut c = b"BT /F1 6 Tf 30 30 Td (x) Tj ET 0.1 w ".to_vec();
    for k in 0..400 {
        let p = 10 + 3 * k;
        c.extend(format!("10 {p} m 1207 {p} l S {p} 10 m {p} 1207 l S ").as_bytes());
    }
    let out = read(&one_page(&c, nothing));
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("drawn cells on page 1")),
        "{:?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

#[test]
fn a_borderless_table_of_40000_rows_is_bounded() {
    // One tall page of 40,000 rows in two columns of short distinct cells.
    let start = Instant::now();
    let n = 40_000usize;
    let h = (n + 2) * 14;
    let mut c = b"BT /F1 10 Tf ".to_vec();
    for i in 0..n {
        let y = h - (i + 1) * 14;
        c.extend(
            format!(
                "1 0 0 1 72 {y} Tm (row {i}) Tj 1 0 0 1 300 {y} Tm ({}) Tj ",
                (i * 7919) % 10007
            )
            .as_bytes(),
        );
    }
    c.extend(b"ET\n");
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let cs = b.stream("/Filter /FlateDecode", &deflate(&c));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 {h}] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let out = read(&b.finish(catalog));
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    let pages = rezzer_render::paginate(&md, hostile().page_tokens);
    assert!(!pages.is_empty());
    drop(pages);
    drop(md);
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    let rows: usize = page(&out, 1)
        .iter()
        .map(|b| match b {
            Block::Table { rows, .. } => rows.len(),
            _ => 0,
        })
        .sum();
    assert!(rows > 1_000, "{rows} rows, {:?}", out.document.losses);
    if rows < n {
        assert!(
            !losses(&out, LossReason::Limit).is_empty(),
            "{rows} rows, {:?}",
            out.document.losses
        );
    }
    assert_rss_under_256_mib();
}

/// One tall page with Helvetica as `/F1` and `content`, `h` points high.
fn tall_page(content: &[u8], h: usize) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let cs = b.stream("/Filter /FlateDecode", &deflate(content));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 {h}] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn many_small_ruled_tables_on_one_page_are_bounded() {
    // 3,000 ruled boxes of two by two cells, each with a word in every cell.
    let start = Instant::now();
    let n = 3_000usize;
    let h = (n + 2) * 30;
    let mut c = b"0.5 w ".to_vec();
    for i in 0..n {
        let top = h - 10 - i * 30;
        for k in 0..3 {
            let y = top - k * 10;
            c.extend(format!("72 {y} m 272 {y} l S ").as_bytes());
            let x = 72 + k * 100;
            c.extend(format!("{x} {top} m {x} {} l S ", top - 20).as_bytes());
        }
        c.extend(b"BT /F1 6 Tf ");
        for (k, (x, y)) in [
            (76, top - 8),
            (176, top - 8),
            (76, top - 18),
            (176, top - 18),
        ]
        .into_iter()
        .enumerate()
        {
            c.extend(format!("1 0 0 1 {x} {y} Tm (c{i}x{k}) Tj ").as_bytes());
        }
        c.extend(b"ET\n");
    }
    let out = read(&tall_page(&c, h));
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    let tables = page(&out, 1)
        .iter()
        .filter(|b| matches!(b, Block::Table { .. }))
        .count();
    assert!(tables > 0, "{:?}", out.document.losses);
    if tables < n {
        assert!(
            !losses(&out, LossReason::Limit).is_empty(),
            "{tables} tables, {:?}",
            out.document.losses
        );
    }
    assert_rss_under_256_mib();
}

#[test]
fn many_refused_ruled_grids_with_notes_are_bounded() {
    // 2,000 ruled grids of three by two cells with a word in two cells of the first column:
    // each is refused (most of its cells are empty), the page gets one note for all of them,
    // and their text stays paragraphs.
    let start = Instant::now();
    let n = 2_000usize;
    let h = (n + 2) * 30;
    let mut c = b"0.5 w ".to_vec();
    for i in 0..n {
        let top = h - 10 - i * 30;
        for k in 0..3 {
            let y = top - k * 10;
            c.extend(format!("72 {y} m 372 {y} l S ").as_bytes());
        }
        for k in 0..4 {
            let x = 72 + k * 100;
            c.extend(format!("{x} {top} m {x} {} l S ", top - 20).as_bytes());
        }
        c.extend(
            format!(
                "BT /F1 6 Tf 1 0 0 1 76 {} Tm (r{i}a) Tj 1 0 0 1 76 {} Tm (r{i}b) Tj ET\n",
                top - 8,
                top - 18
            )
            .as_bytes(),
        );
    }
    let out = read(&tall_page(&c, h));
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    let blocks = page(&out, 1);
    assert!(!blocks.iter().any(|b| matches!(b, Block::Table { .. })));
    let notes = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.contains("Possible table"))
        .count();
    assert_eq!(notes, 1, "{notes}");
    assert_rss_under_256_mib();
}

#[test]
fn many_small_tables_without_rules_on_one_page_are_bounded() {
    // 6,000 blocks of three rows in two columns, 60 points apart.
    let start = Instant::now();
    let n = 6_000usize;
    let h = (n + 2) * 60;
    let mut c = b"BT /F1 8 Tf ".to_vec();
    for i in 0..n {
        let top = h - 20 - i * 60;
        for r in 0..3 {
            let y = top - r * 10;
            c.extend(
                format!(
                    "1 0 0 1 72 {y} Tm (k{i}r{r}) Tj 1 0 0 1 300 {y} Tm ({}) Tj ",
                    (i * 31 + r * 7) % 97
                )
                .as_bytes(),
            );
        }
    }
    c.extend(b"ET\n");
    let out = read(&tall_page(&c, h));
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    let tables = page(&out, 1)
        .iter()
        .filter(|b| matches!(b, Block::Table { .. }))
        .count();
    assert!(tables > 0, "{:?}", out.document.losses);
    if tables < n {
        assert!(
            !losses(&out, LossReason::Limit).is_empty(),
            "{tables} tables, {:?}",
            out.document.losses
        );
    }
    assert_rss_under_256_mib();
}

#[test]
fn a_row_over_3000_pages_is_bounded() {
    // One tagged row with a cell on each of 3,000 pages: the row is planned once and every
    // page adds its cell's text to it.
    let start = Instant::now();
    let out = read(&common::wide_row(3_000));
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let rows: Vec<usize> = page(&out, 1)
        .iter()
        .filter_map(|b| match b {
            Block::Table { rows, .. } => Some(rows.len()),
            _ => None,
        })
        .collect();
    assert_eq!(rows, [1], "{:?}", out.document.losses);
    assert_rss_under_256_mib();
}

/// One page `w` points wide and `h` high with Helvetica as `/F1` and `content`.
fn page_of(content: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let cs = b.stream("/Filter /FlateDecode", &deflate(content));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// `n` small tables of three rows in two columns, 60 points apart, on one page; with `big`,
/// one glyph as high as the page as well.
fn tiny_tables(n: usize, big: bool) -> Vec<u8> {
    let h = (n + 2) * 60;
    let size = if big { h } else { 8 };
    let mut c = format!("BT /F1 {size} Tf 1 0 0 1 400 1 Tm (Z) Tj /F1 8 Tf ").into_bytes();
    for i in 0..n {
        let top = h - 20 - i * 60;
        for r in 0..3 {
            let y = top - r * 10;
            c.extend(
                format!(
                    "1 0 0 1 72 {y} Tm (a) Tj 1 0 0 1 300 {y} Tm ({}) Tj ",
                    (i + r) % 10
                )
                .as_bytes(),
            );
        }
    }
    c.extend(b"ET\n");
    page_of(&c, 595, h)
}

#[test]
fn the_cells_of_36000_small_tables_are_charged() {
    let start = Instant::now();
    let out = read(&tiny_tables(36_000, false));
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    let tables = page(&out, 1)
        .iter()
        .filter(|b| matches!(b, Block::Table { .. }))
        .count();
    assert!(tables > 0);
    if tables < 36_000 {
        assert!(
            !losses(&out, LossReason::Limit).is_empty(),
            "{tables} tables, {:?}",
            out.document.losses
        );
    }
    assert_rss_under_256_mib();
}

#[test]
fn one_huge_glyph_does_not_widen_the_caption_search() {
    // The caption band once came from the largest size on the page: one glyph as high as the
    // page made every region look at every line.
    let start = Instant::now();
    let out = read(&tiny_tables(36_000, true));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    assert!(
        page(&out, 1)
            .iter()
            .any(|b| matches!(b, Block::Table { .. }))
    );
    assert_rss_under_256_mib();
}

#[test]
fn a_grid_of_20000_columns_is_refused_quickly() {
    // Six full-width rules and 19,990 vertical rules 2.5 points apart, one word inside.
    let n = 19_990usize;
    let w = 100 + n * 5 / 2;
    let mut c = b"0.1 w ".to_vec();
    for k in 0..6 {
        let y = 100 + k * 20;
        c.extend(format!("50 {y} m {} {y} l S ", w - 40).as_bytes());
    }
    for k in 0..n {
        let x = 50.0 + 2.5 * k as f64;
        c.extend(format!("{x} 100 m {x} 200 l S ").as_bytes());
    }
    c.extend(b"BT /F1 1 Tf 1 0 0 1 51 105 Tm (x) Tj ET");
    let bytes = page_of(&c, w, 400);
    let start = Instant::now();
    let out = read(&bytes);
    let took = start.elapsed();
    assert!(took < Duration::from_millis(500), "{took:?}");
    assert!(
        !page(&out, 1)
            .iter()
            .any(|b| matches!(b, Block::Table { .. }))
    );
}

#[test]
fn a_built_row_over_5000_pages_costs_about_what_its_text_costs() {
    // One tagged row of 150,000 cells, one word on each of 5,000 pages. Each page once walked
    // every cell of the row to find its own.
    let time = |table: bool| {
        let bytes = common::row_pages(5_000, 150_000, table);
        let start = Instant::now();
        let out = read(&bytes);
        (start.elapsed(), out)
    };
    let (plain, _) = time(false);
    let (tabled, out) = time(true);
    assert!(
        tabled < plain * 3 + Duration::from_millis(500),
        "table {tabled:?}, without {plain:?}"
    );
    assert!(
        page(&out, 1)
            .iter()
            .any(|b| matches!(b, Block::Table { .. }))
    );
}

/// `n` pages, each with the same `k` small tables of three rows in two columns (or, with
/// `same`, rows that all read the same, which stay paragraphs).
fn table_pages(n: usize, k: usize, same: bool) -> Vec<u8> {
    let h = (k + 2) * 60;
    let mut c = b"BT /F1 8 Tf ".to_vec();
    for i in 0..k {
        let top = h - 20 - i * 60;
        for r in 0..3 {
            let y = top - r * 10;
            let v = if same { 0 } else { (i + r) % 10 };
            c.extend(format!("1 0 0 1 72 {y} Tm (a) Tj 1 0 0 1 300 {y} Tm ({v}) Tj ").as_bytes());
        }
    }
    c.extend(b"ET\n");
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let cs = b.stream("/Filter /FlateDecode", &deflate(&c));
    let mut kids = String::new();
    for _ in 0..n {
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 {h}] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
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

/// The pages of a read with blocks besides their marker.
fn pages_with_text(out: &ReadOutcome) -> usize {
    out.document
        .body
        .iter()
        .filter(|b| matches!(b, Block::Section { blocks, .. } if blocks.len() > 1))
        .count()
}

#[test]
fn tables_cost_about_what_their_text_costs_as_paragraphs() {
    // 20 pages of 3,000 small tables: the file keeps the text of only some pages. As tables
    // they must read about as many pages as the same text as paragraphs.
    let start = Instant::now();
    let paragraphs = read(&table_pages(20, 3_000, true));
    let tables = read(&table_pages(20, 3_000, false));
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let (p, t) = (pages_with_text(&paragraphs), pages_with_text(&tables));
    assert!(p < 20, "the shape must cut the file: {p}");
    assert!(
        t + 1 >= p,
        "tables read {t} pages, paragraphs {p}: {:?}",
        tables.document.losses
    );
    let cut: Vec<&Loss> = tables
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(cut.len(), 1, "{cut:?}");
    assert!(cut[0].what.contains("not read"), "{cut:?}");
    assert_rss_under_256_mib();
}

/// `n` pages, each with 1,000 ruled grids of 15 by 2 cells with a word in each cell of the
/// first row and the second row empty; without `rules` only the words.
fn half_ruled_pages(n: usize, rules: bool) -> Vec<u8> {
    let (grids, cols, rows, cw, rh) = (1_000usize, 15usize, 2usize, 10usize, 10usize);
    let gh = rows * rh + 10;
    let h = grids * gh + 100;
    let w = cols * cw + 200;
    let mut c = b"0.1 w ".to_vec();
    let mut t = b"BT /F1 6 Tf ".to_vec();
    for g in 0..grids {
        let top = h - 50 - g * gh;
        for r in 0..=rows {
            let y = top - r * rh;
            c.extend(format!("50 {y} m {} {y} l S ", 50 + cols * cw).as_bytes());
        }
        for k in 0..=cols {
            let x = 50 + k * cw;
            c.extend(format!("{x} {} m {x} {top} l S ", top - rows * rh).as_bytes());
        }
        for k in 0..cols {
            t.extend(format!("1 0 0 1 {} {} Tm (x) Tj ", 52 + k * cw, top - rh + 2).as_bytes());
        }
    }
    t.extend(b"ET\n");
    if rules {
        c.extend(t);
    } else {
        c = t;
    }
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let cs = b.stream("/Filter /FlateDecode", &deflate(&c));
    let mut kids = String::new();
    for _ in 0..n {
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
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

#[test]
fn table_overhead_never_stops_the_text() {
    // 17 pages of half-empty ruled tables in a file whose kept text is near its bound: the
    // empty cells give nothing back, so the overhead runs out. The tables that do not fit
    // stay paragraphs and every page is read, as without the rules.
    let start = Instant::now();
    let plain = read(&half_ruled_pages(17, false));
    let ruled = read(&half_ruled_pages(17, true));
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let (p, r) = (pages_with_text(&plain), pages_with_text(&ruled));
    assert!(
        r + 1 >= p,
        "ruled read {r} pages, plain {p}: {:?}",
        ruled.document.losses
    );
    assert!(
        !ruled
            .document
            .losses
            .iter()
            .any(|l| l.what.contains("kept text limit")),
        "{:?}",
        ruled.document.losses
    );
    assert!(
        ruled
            .document
            .losses
            .iter()
            .any(|l| l.severity == Severity::Info && l.what.contains("kept as paragraphs")),
        "{:?}",
        ruled.document.losses
    );
    assert_rss_under_256_mib();
}

/// A tagged table whose row 1 is on page 1 ("first" and `empty` empty cells) and whose row 2
/// is on page 2 ("second"), each page with an untagged line after it, then `filler` pages of
/// 150 untagged lines.
fn tagged_table_over_two_pages(empty: usize, filler: usize) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let (root, doc, table) = (b.reserve(), b.reserve(), b.reserve());
    let (tr1, tr2) = (b.reserve(), b.reserve());
    let (p1, p2) = (b.reserve(), b.reserve());
    let c1 = b.stream(
        "",
        b"/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (first) Tj ET EMC \
          BT /F1 12 Tf 72 600 Td (plain one) Tj ET",
    );
    let c2 = b.stream(
        "",
        b"/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (second) Tj ET EMC \
          BT /F1 12 Tf 72 600 Td (plain two) Tj ET",
    );
    for (p, c, k) in [(p1, c1, 0), (p2, c2, 1)] {
        b.set(
            p,
            format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /StructParents {k} \
                 /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
            ),
        );
    }
    let ta = b.add(format!("<< /S /TD /P {tr1} 0 R /Pg {p1} 0 R /K 0 >>"));
    let tb = b.add(format!("<< /S /TD /P {tr2} 0 R /Pg {p2} 0 R /K 0 >>"));
    let mut tds = vec![ta];
    for _ in 0..empty {
        tds.push(b.add(format!("<< /S /TD /P {tr1} 0 R >>")));
    }
    let refs = |ids: &[u32]| -> String { ids.iter().map(|r| format!("{r} 0 R ")).collect() };
    b.set(
        tr1,
        format!("<< /S /TR /P {table} 0 R /K [{}] >>", refs(&tds)),
    );
    b.set(tr2, format!("<< /S /TR /P {table} 0 R /K [{tb} 0 R] >>"));
    b.set(
        table,
        format!("<< /S /Table /P {doc} 0 R /K [{tr1} 0 R {tr2} 0 R] >>"),
    );
    b.set(
        doc,
        format!("<< /S /Document /P {root} 0 R /K [{table} 0 R] >>"),
    );
    let mut text = b"BT /F1 6 Tf ".to_vec();
    for i in 0..150 {
        text.extend(
            format!(
                "1 0 0 1 20 {} Tm (filler line {i} with some words in it to fill) Tj ",
                800 - (i % 130) * 6
            )
            .as_bytes(),
        );
    }
    text.extend(b"ET");
    let cs = b.stream("/Filter /FlateDecode", &deflate(&text));
    let mut kids = vec![p1, p2];
    for _ in 0..filler {
        kids.push(b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
        )));
    }
    b.set(
        root,
        format!(
            "<< /Type /StructTreeRoot /K {doc} 0 R \
             /ParentTree << /Nums [0 [{ta} 0 R] 1 [{tb} 0 R]] >> >>"
        ),
    );
    b.set(
        tree,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            refs(&kids),
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

/// The tagged table of [`tagged_table_over_two_pages`] read at 64 MiB, where 150 filler pages
/// leave its 20,000 empty cells no room.
fn read_tagged_table_over_two_pages(filler: usize) -> ReadOutcome {
    let limits = Limits {
        max_memory_bytes: 64 << 20,
        ..hostile()
    };
    read_with(
        &tagged_table_over_two_pages(20_000, filler),
        &Budget::new(limits),
    )
}

/// The words of the tables in `blocks`.
fn table_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        if let Block::Table { rows, .. } = b {
            for c in rows.iter().flatten() {
                out.push_str(&plain_text(&c.inlines));
                out.push('\n');
            }
        }
    }
    out
}

#[test]
fn a_tagged_table_kept_as_paragraphs_keeps_its_later_pages_as_text() {
    let out = read_tagged_table_over_two_pages(150);
    assert_eq!(out.document.losses.len(), 1, "{:?}", out.document.losses);
    let loss = &out.document.losses[0];
    assert_eq!(
        (loss.why, loss.severity),
        (LossReason::Limit, Severity::Info),
        "{loss:?}"
    );
    assert!(loss.what.contains("kept as paragraphs"), "{loss:?}");
    let one = text(page(&out, 1));
    assert!(one.contains("first"), "{one:?}");
    // The loss names the table by its first paragraph.
    let named = page(&out, 1).iter().any(|b| {
        matches!(b, Block::Paragraph { inlines, anchor: Some(a), .. }
            if a.0 == "p1:b1" && plain_text(inlines) == "first")
    });
    assert!(named && loss.what.contains("p1:b1"), "{:?}", page(&out, 1));
    let two = text(page(&out, 2));
    let (second, plain) = (two.find("second"), two.find("plain two"));
    assert!(
        matches!((second, plain), (Some(s), Some(p)) if s < p),
        "{two:?}"
    );
    assert!(table_text(&out.document.body).is_empty());
    assert!(loss.what.contains("later pages"), "{loss:?}");
}

#[test]
fn a_tagged_table_with_room_keeps_both_pages_in_the_table() {
    // The control: without filler pages the room is not short, and the table holds both rows.
    let out = read_tagged_table_over_two_pages(0);
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("kept as paragraphs")),
        "{:?}",
        out.document.losses
    );
    let cells = table_text(page(&out, 1));
    assert!(
        cells.contains("first") && cells.contains("second"),
        "{cells:?}"
    );
}

/// Adds objects to a file with the page object numbers; returns more catalog entries and
/// trailer entries.
type Extra<'a> = &'a mut dyn FnMut(&mut Builder, &[u32]) -> (String, String);

/// A file of `n` pages that each show `Page.`. `annots` gives the annotation dictionaries of
/// page `i` (from 0), with the page object numbers; `extra` adds objects and returns more
/// catalog entries and trailer entries.
fn object_file(
    n: usize,
    annots: &dyn Fn(usize, &[u32]) -> Vec<String>,
    extra: Extra<'_>,
) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream("", b"BT /F1 12 Tf 72 700 Td (Page.) Tj ET");
    let kids: Vec<u32> = (0..n).map(|_| b.reserve()).collect();
    for (i, kid) in kids.iter().enumerate() {
        let refs: Vec<String> = annots(i, &kids)
            .into_iter()
            .map(|a| format!("{} 0 R", b.add(a)))
            .collect();
        b.set(
            *kid,
            format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
                 /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R \
                 /Annots [{}] >>",
                refs.join(" ")
            ),
        );
    }
    let list: Vec<String> = kids.iter().map(|k| format!("{k} 0 R")).collect();
    b.set(
        tree,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            list.join(" "),
            kids.len()
        ),
    );
    let (more, trailer) = extra(&mut b, &kids);
    b.set(
        catalog,
        format!("<< /Type /Catalog /Pages {tree} 0 R {more} >>"),
    );
    b.finish_with(catalog, &trailer)
}

fn no_annots(_: usize, _: &[u32]) -> Vec<String> {
    Vec::new()
}

fn read_extracting(bytes: &[u8]) -> ReadOutcome {
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
    };
    let opts = ReadOptions {
        extract_assets: true,
        ..ReadOptions::default()
    };
    PdfReader.read(&src, &opts, &Budget::new(hostile()))
}

/// The read renders and serializes, stays within the time and memory bounds and is partial
/// with a Limit loss at `location` that contains `words`.
fn assert_objects_cut(out: &ReadOutcome, start: Instant, location: &str, words: &str) {
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    let pages = rezzer_render::paginate(&md, hostile().page_tokens);
    drop(md);
    assert!(!pages.is_empty());
    drop(pages);
    let j = serde_json::to_string(&out.document).expect("the model serializes");
    drop(j);
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit
                && l.location == location
                && l.what.contains(words)),
        "{:?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

#[test]
fn many_outline_items_are_bounded() {
    // 6,000 outline items whose titles all name one 8 KB string: a file of about 700 KB whose
    // titles copy to 48 MB, more than the room the file may keep at the 512 MiB limits.
    let start = Instant::now();
    let n = 6_000usize;
    let bytes = object_file(4, &no_annots, &mut |b, kids| {
        let title = b.add(format!("({})", "t".repeat(8_000)));
        let outlines = b.reserve();
        let ids: Vec<u32> = (0..n).map(|_| b.reserve()).collect();
        for (k, id) in ids.iter().enumerate() {
            let prev = if k > 0 {
                format!("/Prev {} 0 R", ids[k - 1])
            } else {
                String::new()
            };
            let next = match ids.get(k + 1) {
                Some(next) => format!("/Next {next} 0 R"),
                None => String::new(),
            };
            b.set(
                *id,
                format!(
                    "<< /Title {title} 0 R /Parent {outlines} 0 R {prev} {next} \
                     /Dest [{} 0 R /Fit] >>",
                    kids[k % kids.len()]
                ),
            );
        }
        b.set(
            outlines,
            format!(
                "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {n} >>",
                ids[0],
                ids[n - 1]
            ),
        );
        (format!("/Outlines {outlines} 0 R"), String::new())
    });
    assert!(bytes.len() < 1_000_000, "{}", bytes.len());
    let out = read(&bytes);
    let Block::Section {
        kind: SectionKind::Other,
        blocks,
        ..
    } = &out.document.body[0]
    else {
        panic!("no outline section")
    };
    let kept = match blocks.first() {
        Some(Block::List { items, .. }) => items.len(),
        _ => 0,
    };
    assert!(kept > 1_000 && kept < n, "{kept}");
    assert_objects_cut(&out, start, "outline", "outline items");
}

#[test]
fn many_links_are_bounded() {
    // 10,000 links over 10 pages: half to pages, half to distinct URIs of 400 bytes. At the
    // 512 MiB limits the graph keeps them all; at 128 MiB its share is a quarter as large and
    // the graph keeps what fits and says how many links it left out. The object layer of a
    // much larger file of links takes more than the memory bound by itself.
    let tail = "u".repeat(380);
    let bytes = object_file(
        10,
        &|i, kids| {
            (0..1_000)
                .map(|k| {
                    let target = if k % 2 == 0 {
                        format!("/Dest [{} 0 R /Fit]", kids[(k / 2) % kids.len()])
                    } else {
                        format!("/A << /S /URI /URI (http://x/{i}/{k}/{tail}) >>")
                    };
                    format!("<< /Type /Annot /Subtype /Link /Rect [300 300 310 310] {target} >>")
                })
                .collect()
        },
        &mut |_, _| (String::new(), String::new()),
    );
    let edges = |out: &ReadOutcome| {
        out.document
            .facets
            .graphs
            .iter()
            .find(|g| g.id == "links")
            .map_or(0, |g| g.edges.len())
    };
    let start = Instant::now();
    let out = read_with(
        &bytes,
        &Budget::new(Limits {
            max_memory_bytes: 128 << 20,
            ..hostile()
        }),
    );
    let kept = edges(&out);
    assert!(kept > 1_000 && kept < 10_000, "{kept}");
    let loss = format!("{} more link entries are not kept", 10_000 - kept);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.starts_with(&loss)),
        "{:?}",
        out.document.losses
    );
    drop(out);
    let out = read(&bytes);
    assert_eq!(edges(&out), 10_000);
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    assert!(!md.is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert_rss_under_256_mib();
}

#[test]
fn huge_document_information_is_bounded() {
    // Six fields of the document information name one 7 MiB string: the copies take more
    // than the room the file may keep, so the later ones are left out with one loss.
    let start = Instant::now();
    let bytes = object_file(1, &no_annots, &mut |b, _| {
        let big = b.add(format!("({})", "A".repeat(7 << 20)));
        let info = b.add(format!(
            "<< /Title {big} 0 R /Author {big} 0 R /Subject {big} 0 R /Keywords {big} 0 R \
             /Creator {big} 0 R /Producer {big} 0 R >>"
        ));
        (String::new(), format!("/Info {info} 0 R"))
    });
    let out = read(&bytes);
    drop(bytes);
    let p = &out.document.properties;
    assert_eq!(p.title.as_ref().map(String::len), Some(7 << 20));
    assert_objects_cut(&out, start, "metadata", "not kept");
}

#[test]
fn many_attachments_are_bounded() {
    // 3,000 attachments of 64 KiB each: an eighth of the memory limit takes 1,024 of them.
    let start = Instant::now();
    let packed = deflate(&vec![0u8; 64 << 10]);
    let bytes = object_file(1, &no_annots, &mut |b, _| {
        let mut names = Vec::new();
        for k in 0..3_000 {
            let file = b.stream("/Type /EmbeddedFile /Filter /FlateDecode", &packed);
            let spec = b.add(format!(
                "<< /Type /Filespec /F (f{k}.bin) /EF << /F {file} 0 R >> >>"
            ));
            names.push(format!("(f{k:05}.bin) {spec} 0 R"));
        }
        (
            format!(
                "/Names << /EmbeddedFiles << /Names [{}] >> >>",
                names.join(" ")
            ),
            String::new(),
        )
    });
    let out = read_extracting(&bytes);
    assert_eq!(out.document.facets.embedded.len(), 3_000);
    assert_eq!(out.blobs.len(), 1_024);
    assert_eq!(out.document.assets.len(), 1_024);
    let total: usize = out.blobs.iter().map(|b| b.bytes.len()).sum();
    assert!(total as u64 <= hostile().max_memory_bytes / 8, "{total}");
    let limits: Vec<&Loss> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(limits.len(), 1, "{limits:?}");
    assert_objects_cut(
        &out,
        start,
        "attachment 1025",
        "1976 attachments are not extracted",
    );
}

#[test]
fn many_revisions_are_bounded() {
    // The first save and 9,999 updates that each change the document information.
    let start = Instant::now();
    let mut info = 0;
    let mut bytes = object_file(1, &no_annots, &mut |b, _| {
        info = b.add("<< /Title (Base) /ModDate (D:20260101120000Z) >>");
        (String::new(), format!("/Info {info} 0 R"))
    });
    for k in 0..9_999u32 {
        let date = format!("2026{:02}{:02}{:06}Z", 1 + k % 12, 1 + k % 28, k % 240_000);
        common::append_update(
            &mut bytes,
            &[(
                info,
                format!("<< /Title (Base) /ModDate (D:{date}) >>").into_bytes(),
            )],
            &format!("/Size {} /Root 1 0 R /Info {info} 0 R", info + 1),
        );
    }
    let out = read(&bytes);
    assert_eq!(out.document.properties.facts.revisions, Some(10_000));
    assert_eq!(out.document.facets.timeline.len(), 64);
    let skipped = out
        .document
        .losses
        .iter()
        .find(|l| l.what.contains("only the newest"))
        .expect("a loss for the revisions not compared");
    assert_eq!(
        (skipped.why, skipped.severity),
        (LossReason::Limit, Severity::Info)
    );
    assert_objects_cut(&out, start, "revisions", "only the newest 64");
}

/// Reads a file at the hostile limits, then renders, pages and serializes it as the pipeline
/// does.
fn read_through(bytes: &[u8]) -> ReadOutcome {
    let out = read(bytes);
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    let pages = rezzer_render::paginate(&md, hostile().page_tokens);
    drop(md);
    assert!(!pages.is_empty());
    drop(pages);
    let j = serde_json::to_string(&out.document).expect("the model serializes");
    drop(j);
    out
}

/// Reads `twin`, the same objects where no reader looks, then `shape`. The shape stays under
/// 256 MiB and within 64 MiB of what its twin took (each list of the object layer keeps at most
/// a 64th of the memory limit in one of its shares, the metadata, outline and links in one and
/// the form logic in another, and the renderer and JSON writer copy what is kept about four
/// times), and finishes in time. Returns the shape's outcome.
fn against_twin(shape: &[u8], twin: &[u8]) -> ReadOutcome {
    let start = Instant::now();
    drop(read_through(twin));
    let twin_rss = common::max_rss_bytes();
    let out = read_through(shape);
    let rss = common::max_rss_bytes();
    assert!(
        rss < twin_rss + (64 << 20),
        "max RSS {} MiB, twin {} MiB",
        rss >> 20,
        twin_rss >> 20
    );
    assert_rss_under_256_mib();
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    out
}

/// The Limit loss at `location` whose text contains `words`.
fn limit_at<'a>(out: &'a ReadOutcome, location: &str, words: &str) -> &'a Loss {
    out.document
        .losses
        .iter()
        .find(|l| l.why == LossReason::Limit && l.location == location && l.what.contains(words))
        .unwrap_or_else(|| panic!("{:?}", out.document.losses))
}

/// No loss holds more than a few hundred bytes of text.
fn assert_short_losses(out: &ReadOutcome) {
    for l in &out.document.losses {
        assert!(
            l.what.len() < 600,
            "{} bytes: {:.200}",
            l.what.len(),
            l.what
        );
    }
}

/// A one-page file with `catalog` entries and Info `info` (an empty string for none); `objects`
/// adds objects first and gets their number back through `n`.
fn catalog_file(objects: &mut dyn FnMut(&mut Builder) -> (String, String)) -> Vec<u8> {
    object_file(1, &no_annots, &mut |b, _| {
        let (catalog, info) = objects(b);
        if info.is_empty() {
            return (catalog, String::new());
        }
        let info = b.add(info);
        (catalog, format!("/Info {info} 0 R"))
    })
}

#[test]
fn many_keywords_are_charged_one_by_one() {
    // A 10 MB /Keywords of about 1.1 million different words, against one 10 MB word.
    let words: Vec<String> = (0..1_100_000).map(|k| format!("k{k}")).collect();
    let list = words.join(",");
    drop(words);
    let shape = catalog_file(&mut |b| {
        let k = b.add(format!("({list})"));
        (
            String::new(),
            format!("<< /Keywords {k} 0 R /Title (Kept) /Producer (Also kept) >>"),
        )
    });
    let one = "k".repeat(list.len());
    let twin = catalog_file(&mut |b| {
        let k = b.add(format!("({one})"));
        (String::new(), format!("<< /Keywords {k} 0 R >>"))
    });
    let out = against_twin(&shape, &twin);
    let kept = out.document.properties.keywords.len();
    assert!(kept > 1_000 && kept < 1_100_000, "{kept}");
    assert_eq!(out.document.properties.keywords[0], "k0");
    // The single values are kept before any list, and the lists leave room for the rest.
    let p = &out.document.properties;
    assert_eq!(p.title.as_deref(), Some("Kept"));
    assert!(
        p.custom.contains_key("producer") && p.custom.contains_key("pdf_version"),
        "{:?}",
        p.custom
    );
    assert_eq!(out.completeness, Completeness::Partial);
    let loss = limit_at(&out, "metadata", "keywords");
    assert!(
        loss.what.starts_with(&format!(
            "{} keywords are not kept, from keyword {} on",
            1_100_000 - kept,
            kept + 1
        )),
        "{loss:?}"
    );
}

#[test]
fn repeated_keywords_are_kept_once() {
    let list = "a,".repeat(2_000_000);
    let bytes = catalog_file(&mut |b| {
        let k = b.add(format!("({list})"));
        (String::new(), format!("<< /Keywords {k} 0 R >>"))
    });
    let out = read_through(&bytes);
    assert_eq!(out.document.properties.keywords, ["a"]);
    assert_eq!(out.completeness, Completeness::Complete);
    assert_rss_under_256_mib();
}

#[test]
fn many_xmp_authors_are_charged_one_by_one() {
    // 400,000 creators in XMP, against the same list as contributors, which nobody reads.
    let xmp = |tag: &str| {
        let li: String = (0..400_000)
            .map(|k| format!("<rdf:li>a{k}</rdf:li>"))
            .collect();
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:{tag}><rdf:Seq>{li}</rdf:Seq></dc:{tag}></rdf:Description></rdf:RDF></x:xmpmeta>"#
        )
    };
    let file = |tag: &str| {
        let x = xmp(tag);
        catalog_file(&mut |b| {
            let m = b.stream("/Type /Metadata /Subtype /XML", x.as_bytes());
            (format!("/Metadata {m} 0 R"), String::new())
        })
    };
    let out = against_twin(&file("creator"), &file("contributor"));
    let kept = out.document.properties.authors.len();
    assert!(kept > 1_000 && kept < 400_000, "{kept}");
    limit_at(&out, "metadata", "authors");
}

#[test]
fn bad_dates_are_quoted_short() {
    // The creation and modification dates name one 12 MB string of control bytes; the twin
    // names it as the subject, which is charged and cut.
    let mut body = b"(".to_vec();
    body.extend(vec![1u8; 12 << 20]);
    body.push(b')');
    let file = |info: &str| {
        let body = body.clone();
        let info = info.to_string();
        let mut b = Builder::default();
        let catalog = b.reserve();
        let tree = b.reserve();
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] >>"
        ));
        b.set(
            tree,
            format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
        );
        let d = b.add(body);
        let info = b.add(info.replace('@', &d.to_string()));
        b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
        b.finish_with(catalog, &format!("/Info {info} 0 R"))
    };
    let shape = file("<< /CreationDate @ 0 R /ModDate @ 0 R >>");
    let twin = file("<< /Subject @ 0 R >>");
    let out = against_twin(&shape, &twin);
    assert_eq!(out.document.properties.created, None);
    assert_short_losses(&out);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.location == "metadata" && l.what.contains("creation date")),
        "{:?}",
        out.document.losses
    );
}

/// `n` outline items naming `title` (a string or a reference), on page `kid`, under `key`.
fn outline_items(b: &mut Builder, kid: u32, n: usize, title: &str, key: &str) -> String {
    let outlines = b.reserve();
    let ids: Vec<u32> = (0..n).map(|_| b.reserve()).collect();
    for (k, id) in ids.iter().enumerate() {
        let prev = if k > 0 {
            format!("/Prev {} 0 R", ids[k - 1])
        } else {
            String::new()
        };
        let next = ids
            .get(k + 1)
            .map(|n| format!("/Next {n} 0 R"))
            .unwrap_or_default();
        b.set(
            *id,
            format!(
                "<< /Title {title} /Parent {outlines} 0 R {prev} {next} /Dest [{kid} 0 R /Fit] >>"
            ),
        );
    }
    b.set(
        outlines,
        format!(
            "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {n} >>",
            ids[0],
            ids[n - 1]
        ),
    );
    format!("{key} {outlines} 0 R")
}

#[test]
fn a_title_shared_by_many_outline_items_is_read_once() {
    // 30,000 outline items name one 8 KB title: 240 MB if each item copied it.
    let file = |key: &str| {
        object_file(1, &no_annots, &mut |b, kids| {
            let t = b.add(format!("({})", "t".repeat(8 << 10)));
            (
                outline_items(b, kids[0], 30_000, &format!("{t} 0 R"), key),
                String::new(),
            )
        })
    };
    let out = against_twin(&file("/Outlines"), &file("/Junk"));
    let Block::Section {
        kind: SectionKind::Other,
        blocks,
        ..
    } = &out.document.body[0]
    else {
        panic!("no outline section")
    };
    let kept = match blocks.first() {
        Some(Block::List { items, .. }) => items.len(),
        _ => 0,
    };
    assert!(kept > 1_000 && kept < 30_000, "{kept}");
    limit_at(
        &out,
        "outline",
        &format!("outline items {} to 30000", kept + 1),
    );
    // The revisions are read before the lists, so nothing spends what they need.
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.location == "revisions"),
        "{:?}",
        out.document.losses
    );
    assert_short_losses(&out);
}

#[test]
fn a_name_shared_by_many_layers_is_read_once() {
    let file = |key: &str| {
        catalog_file(&mut |b| {
            let name = b.add(format!("({})", "L".repeat(8 << 10)));
            let ids: Vec<String> = (0..30_000)
                .map(|_| {
                    format!(
                        "{} 0 R",
                        b.add(format!("<< /Type /OCG /Name {name} 0 R >>"))
                    )
                })
                .collect();
            let list = ids.join(" ");
            (format!("{key} << /OCGs [{list}] >>"), String::new())
        })
    };
    let out = against_twin(&file("/OCProperties"), &file("/Junk"));
    assert_eq!(out.document.properties.facts.layers, Some(30_000));
    let loss = limit_at(&out, "metadata", "layers");
    assert!(
        loss.what.contains("layers are not listed, from layer"),
        "{loss:?}"
    );
    assert_short_losses(&out);
}

#[test]
fn a_title_shared_by_many_threads_is_read_once() {
    let file = |key: &str| {
        object_file(1, &no_annots, &mut |b, kids| {
            let t = b.add(format!("({})", "T".repeat(8 << 10)));
            let info = b.add(format!("<< /Title {t} 0 R >>"));
            let mut list = Vec::new();
            for _ in 0..30_000 {
                let th = b.reserve();
                let bead = b.reserve();
                b.set(
                    bead,
                    format!(
                        "<< /Type /Bead /T {th} 0 R /N {bead} 0 R /V {bead} 0 R /P {} 0 R \
                         /R [0 0 10 10] >>",
                        kids[0]
                    ),
                );
                b.set(
                    th,
                    format!("<< /Type /Thread /F {bead} 0 R /I {info} 0 R >>"),
                );
                list.push(format!("{th} 0 R"));
            }
            (format!("{key} [{}]", list.join(" ")), String::new())
        })
    };
    let out = against_twin(&file("/Threads"), &file("/Junk"));
    let threads = out
        .document
        .facets
        .embedded
        .iter()
        .filter(|e| e.kind == "article-thread")
        .count();
    assert!(threads > 1_000 && threads < 30_000, "{threads}");
    limit_at(&out, "article threads", "not listed");
}

#[test]
fn a_description_shared_by_many_attachments_is_read_once() {
    let file = |key: &str| {
        catalog_file(&mut |b| {
            let d = b.add(format!("({})", "D".repeat(8 << 10)));
            let mut names = Vec::new();
            for k in 0..30_000 {
                let f = b.stream("/Type /EmbeddedFile", format!("{k}").as_bytes());
                let spec = b.add(format!(
                    "<< /Type /Filespec /F (f{k}.bin) /Desc {d} 0 R /EF << /F {f} 0 R >> >>"
                ));
                names.push(format!("(f{k:07}.bin) {spec} 0 R"));
            }
            let list = names.join(" ");
            let catalog = if key == "/Junk" {
                format!("/Junk [{list}]")
            } else {
                format!("/Names << /EmbeddedFiles << /Names [{list}] >> >>")
            };
            (catalog, String::new())
        })
    };
    let out = against_twin(&file("/Names"), &file("/Junk"));
    let listed = out
        .document
        .facets
        .embedded
        .iter()
        .filter(|e| e.kind == "attachment")
        .count();
    assert!(listed > 1_000 && listed < 30_000, "{listed}");
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.why == LossReason::Limit && l.what.contains("not listed"))
        .unwrap_or_else(|| panic!("{:?}", out.document.losses));
    assert!(loss.location.starts_with("attachment "), "{loss:?}");
}

/// A one-page file with `n` link annotations whose action is `action`, given the number of a
/// shared 8 KB string object; under `key` (`/Annots`, or `/Junk` for the twin).
fn shared_links(n: usize, action: &dyn Fn(u32) -> String, key: &str) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let s = b.add(format!("(http://x/{})", "u".repeat(8 << 10)));
    let annots: Vec<String> = (0..n)
        .map(|_| {
            format!(
                "{} 0 R",
                b.add(format!(
                    "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << {} >> >>",
                    action(s)
                ))
            )
        })
        .collect();
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] {key} [{}] >>",
        annots.join(" ")
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn a_file_name_shared_by_many_launch_links_is_read_once() {
    let action = |s: u32| format!("/S /Launch /F {s} 0 R");
    let out = against_twin(
        &shared_links(30_000, &action, "/Annots"),
        &shared_links(30_000, &action, "/Junk"),
    );
    let listed = out
        .document
        .facets
        .embedded
        .iter()
        .filter(|e| e.kind == "launch-action")
        .count();
    assert!(listed > 100 && listed < 30_000, "{listed}");
    limit_at(&out, "p1", "more link entries are not kept");
    assert_short_losses(&out);
}

#[test]
fn a_uri_shared_by_many_links_is_read_once() {
    let action = |s: u32| format!("/S /URI /URI {s} 0 R");
    let out = against_twin(
        &shared_links(30_000, &action, "/Annots"),
        &shared_links(30_000, &action, "/Junk"),
    );
    let graph = out
        .document
        .facets
        .graphs
        .iter()
        .find(|g| g.id == "links")
        .expect("a links graph");
    let uris = graph
        .nodes
        .iter()
        .filter(|n| n.kind.as_deref() == Some("uri"))
        .count();
    assert_eq!((uris, graph.edges.len()), (1, 30_000));
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
}

#[test]
fn a_title_reached_through_a_chain_of_references_is_read_once() {
    // 30,000 outline items each name an object of their own that only refers to one 8 KB
    // title.
    let file = |key: &str| {
        object_file(1, &no_annots, &mut |b, kids| {
            let t = b.add(format!("({})", "t".repeat(8 << 10)));
            let outlines = b.reserve();
            let ids: Vec<u32> = (0..30_000).map(|_| b.reserve()).collect();
            for (k, id) in ids.iter().enumerate() {
                let hop = b.add(format!("{t} 0 R"));
                let prev = if k > 0 {
                    format!("/Prev {} 0 R", ids[k - 1])
                } else {
                    String::new()
                };
                let next = ids
                    .get(k + 1)
                    .map(|n| format!("/Next {n} 0 R"))
                    .unwrap_or_default();
                b.set(
                    *id,
                    format!(
                        "<< /Title {hop} 0 R /Parent {outlines} 0 R {prev} {next} \
                         /Dest [{} 0 R /Fit] >>",
                        kids[0]
                    ),
                );
            }
            b.set(
                outlines,
                format!(
                    "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count 30000 >>",
                    ids[0], ids[29_999]
                ),
            );
            (format!("{key} {outlines} 0 R"), String::new())
        })
    };
    let out = against_twin(&file("/Outlines"), &file("/Junk"));
    limit_at(&out, "outline", "outline items");
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.location == "revisions"),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn a_uri_reached_through_a_chain_of_references_is_read_once() {
    let file = |key: &str| {
        let mut b = Builder::default();
        let catalog = b.reserve();
        let tree = b.reserve();
        let u = b.add(format!("(http://x/{})", "u".repeat(8 << 10)));
        let annots: Vec<String> = (0..30_000)
            .map(|_| {
                let hop = b.add(format!("{u} 0 R"));
                format!(
                    "{} 0 R",
                    b.add(format!(
                        "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] \
                         /A << /S /URI /URI {hop} 0 R >> >>"
                    ))
                )
            })
            .collect();
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] {key} [{}] >>",
            annots.join(" ")
        ));
        b.set(
            tree,
            format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
        );
        b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
        b.finish(catalog)
    };
    let out = against_twin(&file("/Annots"), &file("/Junk"));
    let graph = out
        .document
        .facets
        .graphs
        .iter()
        .find(|g| g.id == "links")
        .expect("a links graph");
    assert_eq!(graph.edges.len(), 30_000);
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.location == "revisions"),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn a_layer_name_reached_through_a_chain_of_references_is_read_once() {
    let file = |key: &str| {
        catalog_file(&mut |b| {
            let name = b.add(format!("({})", "L".repeat(8 << 10)));
            let ids: Vec<String> = (0..30_000)
                .map(|_| {
                    let hop = b.add(format!("{name} 0 R"));
                    format!("{} 0 R", b.add(format!("<< /Type /OCG /Name {hop} 0 R >>")))
                })
                .collect();
            (
                format!("{key} << /OCGs [{}] >>", ids.join(" ")),
                String::new(),
            )
        })
    };
    let out = against_twin(&file("/OCProperties"), &file("/Junk"));
    assert_eq!(out.document.properties.facts.layers, Some(30_000));
}

/// A one-page file that shows `lines` lines of `per_line` letters, with the annotations
/// `annots` adds and returns by number. The page lists them under `/Annots`.
fn annotated(
    lines: usize,
    per_line: usize,
    annots: &mut dyn FnMut(&mut Builder) -> Vec<u32>,
) -> Vec<u8> {
    let h = 20 + lines * 14;
    let mut c = b"BT /F1 12 Tf 14 TL ".to_vec();
    c.extend(format!("1 0 0 1 20 {} Tm ", h - 20).as_bytes());
    let line = "w".repeat(per_line);
    for _ in 0..lines {
        c.extend(format!("({line}) Tj T* ").as_bytes());
    }
    c.extend(b"ET");
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream("/Filter /FlateDecode", &deflate(&c));
    let refs: Vec<String> = annots(&mut b)
        .into_iter()
        .map(|a| format!("{a} 0 R"))
        .collect();
    let w = 40 + per_line * 12;
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R /Annots [{}] >>",
        refs.join(" ")
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn a_comment_shared_by_many_annotations_is_bounded() {
    // 30,000 notes name one 8 KB comment: read once by the object layer, but each note in the
    // model holds its own copy, so the room the file may keep ends the list.
    let file = |key: &str| {
        annotated(10, 40, &mut |b| {
            let s = b.add(format!("({})", "c".repeat(8 << 10)));
            (0..30_000)
                .map(|_| {
                    b.add(format!(
                        "<< /Type /Annot /Subtype /Text /Rect [20 20 40 40] {key} {s} 0 R \
                             /T (Ana) >>"
                    ))
                })
                .collect()
        })
    };
    let out = against_twin(&file("/Contents"), &file("/Junk"));
    let kept = out.document.facets.annotations.len();
    assert!(kept > 100 && kept < 30_000, "{kept}");
    limit_at(&out, "document", "are not kept");
    assert_short_losses(&out);
}

#[test]
fn many_quads_per_annotation_are_bounded() {
    // 9 highlights name one array of 25,000 quads that each cover the whole page of 4,000
    // glyphs: 900 million glyph visits, far past the bound.
    let file = |key: &str| {
        annotated(100, 40, &mut |b| {
            let quad = "0 2000 2000 2000 0 0 2000 0 ";
            let quads = b.add(format!("[{}]", quad.repeat(25_000)));
            (0..9)
                .map(|_| {
                    b.add(format!(
                        "<< /Type /Annot /Subtype /Highlight /Rect [0 0 2000 2000] \
                             {key} {quads} 0 R >>"
                    ))
                })
                .collect()
        })
    };
    let out = against_twin(&file("/QuadPoints"), &file("/Junk"));
    assert_eq!(out.document.facets.annotations.len(), 9);
    let loss = limit_at(&out, "p1", "is not quoted");
    assert!(loss.what.contains("quoting visits more than"), "{loss:?}");
    assert_short_losses(&out);
}

#[test]
fn many_highlights_over_a_page_of_many_glyphs_are_bounded() {
    // 20,000 highlights each cover the whole page of 50,000 glyphs.
    let file = |key: &str| {
        annotated(500, 100, &mut |b| {
            (0..20_000)
                .map(|_| {
                    b.add(format!(
                        "<< /Type /Annot /Subtype /Highlight /Rect [0 0 2000 8000] \
                             {key} [0 8000 2000 8000 0 0 2000 0] >>"
                    ))
                })
                .collect()
        })
    };
    let out = against_twin(&file("/QuadPoints"), &file("/Junk"));
    let quoted = out
        .document
        .facets
        .annotations
        .iter()
        .filter(|a| a.quoted.is_some())
        .count();
    assert!(quoted > 0 && quoted < 20_000, "{quoted}");
    limit_at(&out, "p1", "is not quoted");
    assert_short_losses(&out);
}

#[test]
fn a_long_reply_chain_and_a_loop_of_replies_are_bounded() {
    // 50,000 notes, each a reply to the one before, and two that reply to each other.
    let file = |key: &str| {
        annotated(10, 40, &mut |b| {
            let (x, y) = (b.reserve(), b.reserve());
            b.set(
                x,
                format!("<< /Type /Annot /Subtype /Text /Rect [0 0 1 1] {key} {y} 0 R >>"),
            );
            b.set(
                y,
                format!("<< /Type /Annot /Subtype /Text /Rect [0 0 1 1] {key} {x} 0 R >>"),
            );
            let mut list = vec![x, y];
            let mut prev: Option<u32> = None;
            for _ in 0..50_000 {
                let irt = prev.map_or(String::new(), |p| format!("{key} {p} 0 R"));
                let n = b.add(format!(
                    "<< /Type /Annot /Subtype /Text /Rect [20 20 40 40] /Contents (r) {irt} >>"
                ));
                list.push(n);
                prev = Some(n);
            }
            list
        })
    };
    let out = against_twin(&file("/IRT"), &file("/Junk"));
    let roots = &out.document.facets.annotations;
    assert!(roots.len() >= 2);
    let mut depth = 0;
    let mut at = &roots[1];
    while let Some(next) = at.replies.first() {
        depth += 1;
        at = next;
    }
    assert_eq!(depth, 16);
    limit_at(&out, "document", "what the annotations may keep");
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged && l.what.contains("loop of replies")),
        "{:?}",
        out.document.losses
    );
    assert_short_losses(&out);
}

/// One block of `runs` overlapping runs at height 700 and `notes` notes whose nearest run
/// lies in all of them.
fn overlapping_runs(runs: usize, notes: usize) -> Vec<u8> {
    let mut c = String::from("BT /F1 12 Tf ");
    for i in 0..runs {
        c.push_str(&format!(
            "1 0 0 1 {} 700 Tm (w{i}) Tj ",
            72.0 + (i % 7) as f32 * 0.05
        ));
    }
    c.push_str("ET");
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream("/Filter /FlateDecode", &deflate(c.as_bytes()));
    let refs: Vec<String> = (0..notes)
        .map(|_| {
            let n = b.add("<< /Type /Annot /Subtype /Text /Rect [70 690 90 712] /Contents (n) >>");
            format!("{n} 0 R")
        })
        .collect();
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R /Annots [{}] >>",
        refs.join(" ")
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn notes_near_many_overlapping_runs_are_placed_once_each() {
    // 4,000 notes whose nearest run lies in each of 4,000 overlapping runs of one block. The
    // points of the block are listed once each, not once per run that holds them, so the
    // shape stays within 32 MiB of its twin without notes: the 4,000 notes themselves take
    // about 16 MiB through the model, the renderer and JSON (listed once per run, the points
    // took about 140 MiB more).
    drop(read_through(&overlapping_runs(4000, 0)));
    let twin = common::max_rss_bytes();
    let out = read_through(&overlapping_runs(4000, 4000));
    let rss = common::max_rss_bytes();
    assert!(
        rss < twin + (32 << 20),
        "max RSS {} MiB, twin {} MiB",
        rss >> 20,
        twin >> 20
    );
    let anchors: std::collections::HashSet<_> = out
        .document
        .facets
        .annotations
        .iter()
        .map(|a| a.anchor.clone())
        .collect();
    assert_eq!(anchors.len(), 1, "{anchors:?}");
    assert_eq!(out.document.facets.annotations.len(), 4000);
}

#[test]
fn a_probe_bound_with_every_note_placed_says_nothing() {
    // 8,000 runs and 8,000 notes reach the bound on run and point pairs, but every note has
    // its block by then.
    let out = read(&overlapping_runs(8000, 8000));
    assert!(
        out.document
            .facets
            .annotations
            .iter()
            .all(|a| a.anchor.as_ref().is_some_and(|a| a.0.contains(":b")))
    );
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("stand at the page")),
        "{:?}",
        out.document.losses
    );
}

fn read_assets(bytes: &[u8]) -> ReadOutcome {
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
    };
    let opts = ReadOptions {
        extract_assets: true,
        ..ReadOptions::default()
    };
    PdfReader.read(&src, &opts, &Budget::new(hostile()))
}

/// `pages` pages of `w` by `h` points, each drawing one shared gray picture of 8 by 8 pixels
/// over the whole page and no text.
fn picture_pages(pages: usize, w: u32, h: u32) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /Filter /FlateDecode",
        &deflate(&[90u8; 64]),
    );
    let c = b.stream("", format!("q {w} 0 0 {h} 0 0 cm /Im1 Do Q\n").as_bytes());
    let mut kids = String::new();
    for _ in 0..pages {
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] \
             /Resources << /XObject << /Im1 {image} 0 R >> >> /Contents {c} 0 R >>"
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
fn a_huge_media_box_renders_at_most_16_million_pixels() {
    // 200,000 points square: at 144 dpi that would be 160 billion pixels.
    let out = read_assets(&picture_pages(1, 200_000, 200_000));
    let blob = out
        .blobs
        .iter()
        .find(|b| b.asset == "page-1")
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    let w = u32::from_be_bytes(blob.bytes[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(blob.bytes[20..24].try_into().unwrap());
    assert!(u64::from(w) * u64::from(h) <= 16_000_000, "{w} x {h}");
    assert!(w > 3000 && h > 3000, "{w} x {h}");
    assert_rss_under_256_mib();
}

#[test]
fn many_pages_that_need_vision_render_at_most_1000() {
    let start = Instant::now();
    let out = read_assets(&picture_pages(1010, 20, 20));
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(out.blobs.len(), 1000);
    assert_eq!(out.document.assets.len(), 1010);
    let cut: Vec<&Loss> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit && l.what.contains("not rendered"))
        .collect();
    assert_eq!(cut.len(), 1, "{:?}", out.document.losses);
    assert_eq!(cut[0].location, "p1001");
    assert!(cut[0].what.contains("10 pages"), "{:?}", cut[0]);
    assert_eq!(out.completeness, Completeness::Partial);
    assert_rss_under_256_mib();
}

/// `pages` pages of `glyphs` glyphs each, behind 2,000 bytes before the header, so only the
/// fallback parser reads the file. With `looped`, the page tree also lists itself as a kid.
fn prefixed_pages(pages: usize, glyphs: usize, looped: bool) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut c = b"BT /F1 12 Tf 72 700 Td (Before.) Tj 72 600 Td (".to_vec();
    c.extend(std::iter::repeat_n(b'a', glyphs));
    c.extend(b") Tj ET\n");
    let cs = b.stream("/Filter /FlateDecode", &deflate(&c));
    let mut kids = String::new();
    for _ in 0..pages {
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    if looped {
        kids.push_str(&format!("{tree} 0 R"));
    }
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let mut file = vec![b' '; 2000];
    file.extend(b.finish(catalog));
    file
}

fn fallback_loss(out: &ReadOutcome) -> &Loss {
    out.document
        .losses
        .iter()
        .find(|l| l.what.contains("fallback parser"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses))
}

#[test]
fn huge_fallback_text_is_bounded() {
    // The glyphs of the spread twins, behind 2,000 bytes before the header: only the fallback
    // parser reads them, under the same kept budget.
    let start = Instant::now();
    let mut file = vec![b' '; 2000];
    file.extend(many_pages(20, 220_000, 0));
    let out = read(&file);
    assert_eq!(fallback_loss(&out).location, "document");
    assert_file_cut(&out, 20, start);
}

/// `many_pages` where the content stream's dictionary holds a bare keyword: the object layer
/// drops it, so every page has fonts and no text, and the fallback parser reads them all.
fn fallback_pages(pages: usize, glyphs: usize) -> Vec<u8> {
    let file = many_pages(pages, glyphs, 0);
    let from = b"<< /Filter /FlateDecode /Length ";
    let at = file
        .windows(from.len())
        .position(|w| w == from)
        .expect("the content stream");
    let mut out = file[..at].to_vec();
    // `<< /Filter /FlateDecode /Length N >>` becomes `<</Filter /FlateDecode /Length N x>>`:
    // the same length, so the table holds.
    let close = at + file[at..].windows(2).position(|w| w == b">>").unwrap();
    let length = std::str::from_utf8(&file[at + from.len()..close])
        .unwrap()
        .trim();
    let swapped = format!("<</Filter /FlateDecode /Length {length} x>>");
    assert_eq!(swapped.len(), close + 2 - at);
    out.extend(swapped.as_bytes());
    out.extend(&file[close + 2..]);
    out
}

#[test]
fn huge_text_of_the_per_page_fallback_is_bounded() {
    let start = Instant::now();
    let out = read(&fallback_pages(20, 220_000));
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let losses = &out.document.losses;
    assert!(
        losses
            .iter()
            .any(|l| l.location == "p1" && l.what == "page 1: text read with the fallback parser"),
        "{losses:?}"
    );
    assert!(text(page(&out, 1)).contains("Before."));
    // The kept bytes end the fallback's reading before page 20, and that is said.
    assert!(
        losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("to 20")),
        "{losses:?}"
    );
    assert_eq!(out.completeness, Completeness::Partial);
    assert_rss_under_256_mib();
}

#[test]
fn a_page_tree_loop_in_a_fallback_file_ends() {
    let start = Instant::now();
    let out = read(&prefixed_pages(3, 10, true));
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(fallback_loss(&out).location, "document");
    assert!(text(page(&out, 3)).contains("Before."));
    assert_rss_under_256_mib();
}

#[test]
fn a_bomb_in_a_fallback_file_is_not_unpacked() {
    // The stream guard of the fallback refuses the file before the parser unpacks anything.
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let bomb = b.stream("/Filter /FlateDecode", &common::zeros_packed(1 << 30));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /Contents {bomb} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let mut file = vec![b' '; 2000];
    file.extend(b.finish(catalog));
    let out = read(&file);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what.contains("fallback parser") && l.what.contains("not used")),
        "{:?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

/// A one-page file whose catalog names `fields` (dictionaries, written straight into one
/// array object) under `key`: `/AcroForm` for the shape, `/Junk` for its twin, where no reader
/// looks.
fn form_file(key: &str, fields: &mut dyn FnMut(&mut Builder) -> Vec<String>) -> Vec<u8> {
    catalog_file(&mut |b| {
        let list = format!("[{}]", fields(b).join("\n"));
        let list = b.add(list);
        (format!("{key} << /Fields {list} 0 R >>"), String::new())
    })
}

/// The Limit loss at `location` whose text contains `words`, with a short text.
fn form_cut<'a>(out: &'a ReadOutcome, location: &str, words: &str) -> &'a Loss {
    assert_short_losses(out);
    assert_eq!(out.completeness, Completeness::Partial);
    limit_at(out, location, words)
}

#[test]
fn many_form_fields_are_bounded() {
    // 100,000 text fields, each with a value.
    let file = |key: &str| {
        form_file(key, &mut |_| {
            (0..100_000)
                .map(|k| format!("<< /T (field{k}) /FT /Tx /V (value {k}) >>"))
                .collect()
        })
    };
    let out = against_twin(&file("/AcroForm"), &file("/Junk"));
    let kept = out.document.facets.fields.len();
    assert!(kept > 1_000 && kept < 100_000, "{kept}");
    assert_eq!(out.document.facets.fields[0].name, "field0");
    let loss = form_cut(&out, "form", "form fields are not kept");
    assert!(
        loss.what.starts_with(&format!(
            "{} of 100000 form fields are not kept",
            100_000 - kept
        )),
        "{loss:?}"
    );
    assert_eq!(out.document.properties.facts.form_fields, Some(100_000));
}

#[test]
fn many_scripted_form_fields_stay_within_the_object_budget() {
    // 100,000 fields with a format script each. pdfparts keeps what it reads of the form
    // within its object budget (half the memory limit, shared with the objects) and cuts the
    // form there; its form data, which the twin never builds, takes about 80 MiB of it. The
    // reader keeps a part of what pdfparts read, within its own share.
    let file = |key: &str| {
        form_file(key, &mut |b| {
            let js =
                b.add(r#"<< /S /JavaScript /JS (AFNumber_Format\(2, 0, 0, 0, "", true\);) >>"#);
            (0..100_000)
                .map(|k| {
                    format!("<< /T (field{k}) /FT /Tx /V (value {k}) /AA << /F {js} 0 R >> >>")
                })
                .collect()
        })
    };
    let start = Instant::now();
    drop(read_through(&file("/Junk")));
    let twin = common::max_rss_bytes();
    let out = read_through(&file("/AcroForm"));
    let rss = common::max_rss_bytes();
    assert!(
        rss < twin + (128 << 20),
        "max RSS {} MiB, twin {} MiB",
        rss >> 20,
        twin >> 20
    );
    // The binding bound; the margin over the twin rule is pdfparts' own form data.
    assert_rss_under_256_mib();
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let kept = out.document.facets.fields.len();
    assert!(kept > 1_000 && kept < 100_000, "{kept}");
    form_cut(&out, "form", "form fields are not kept");
    form_cut(&out, "form", "the object limit");
    // The count pdfparts cut is not the file's count.
    assert_eq!(out.document.properties.facts.form_fields, None);
}

#[test]
fn one_field_read_by_many_calculations_is_bounded() {
    // 50,000 fields calculate from one field through one shared script.
    let file = |key: &str| {
        form_file(key, &mut |b| {
            let js = b.add(
                r#"<< /S /JavaScript /JS (event.value = this.getField\("Source"\).value;) >>"#,
            );
            let mut fields = vec!["<< /T (Source) /FT /Tx >>".to_string()];
            fields.extend(
                (0..50_000).map(|k| format!("<< /T (c{k}) /FT /Tx /AA << /C {js} 0 R >> >>")),
            );
            fields
        })
    };
    // Measured at the 512 MiB limits: the twin 68 MiB, pdfparts' own form data for 50,000
    // fields about 35 MiB more, the reader's copies and the renderer 24 MiB more. The rule of
    // the twins (64 MiB) leaves only 5 MiB for a busy machine, so the bound here also counts
    // pdfparts' part, and the binding 256 MiB holds.
    let start = Instant::now();
    drop(read_through(&file("/Junk")));
    let twin = common::max_rss_bytes();
    let out = read_through(&file("/AcroForm"));
    let rss = common::max_rss_bytes();
    assert!(
        rss < twin + (96 << 20),
        "max RSS {} MiB, twin {} MiB",
        rss >> 20,
        twin >> 20
    );
    assert_rss_under_256_mib();
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let facets = &out.document.facets;
    let kept = facets.fields.len();
    assert!(kept > 100 && kept < 50_001, "{kept}");
    // Every kept calculation is a formula that reads Source, and an edge of the graph.
    let formulas = facets.formulas.len();
    assert!(formulas > 0 && formulas < kept, "{formulas} of {kept}");
    let graph = facets.graphs.iter().find(|g| g.id == "form").unwrap();
    assert!(graph.edges.len() <= formulas, "{}", graph.edges.len());
    assert!(graph.nodes.len() <= kept);
    form_cut(&out, "form", "form fields are not kept");
}

/// Exactly `n` bytes of script text, packed with FlateDecode about 10 to 1, so the guard's ratio check
/// lets it pass. The text is packed piece by piece and never held whole.
fn packed_script(n: usize) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut x: u64 = 12345;
    let mut piece = Vec::with_capacity(1 << 16);
    let mut written = 0;
    while written < n {
        piece.clear();
        while piece.len() < 1 << 16 {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let c = (x >> 59) as u8;
            piece.push(if c < 26 { b'a' + c } else { b' ' });
            piece.extend_from_slice(&[b'x'; 47]);
            piece.push(b'\n');
        }
        piece.truncate(n - written);
        z.write_all(&piece).expect("deflate");
        written += piece.len();
    }
    z.finish().expect("deflate")
}

#[test]
fn a_huge_script_is_not_unpacked() {
    // A packed calculate script of 100 MiB, beside a small field that reads nothing. pdfparts
    // refuses a script over its size before it unpacks it.
    let packed = packed_script(100 << 20);
    let file = |key: &str| {
        form_file(key, &mut |b| {
            let js = b.stream("/Filter /FlateDecode", &packed);
            let action = b.add(format!("<< /S /JavaScript /JS {js} 0 R >>"));
            vec![
                format!("<< /T (Big) /FT /Tx /AA << /C {action} 0 R >> >>"),
                "<< /T (Small) /FT /Tx /V (kept) >>".to_string(),
            ]
        })
    };
    let (shape, twin) = (file("/AcroForm"), file("/Junk"));
    drop(packed);
    let out = against_twin(&shape, &twin);
    let facets = &out.document.facets;
    let names: Vec<&str> = facets.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Big", "Small"]);
    assert_eq!(facets.fields[0].role, Some(rezzer_model::Role::Result));
    assert!(facets.fields[0].actions.is_empty());
    assert!(facets.formulas.is_empty());
    let script = facets
        .embedded
        .iter()
        .find(|e| e.name == "field Big calculate")
        .expect("the script is listed");
    assert_eq!(script.text, None);
    form_cut(&out, "form", "a script may have");
    assert_eq!(out.document.properties.facts.has_javascript, Some(true));
}

#[test]
fn both_shares_of_the_object_layer_full_at_once_are_bounded() {
    // Keywords fill the share of the metadata, scripted fields the share of the form logic;
    // the twin names both under keys nobody reads.
    // 200,000 keywords and 50,000 fields fill each share many times over.
    let words: Vec<String> = (0..200_000).map(|k| format!("k{k}")).collect();
    let list = words.join(",");
    drop(words);
    let file = |shape: bool| {
        catalog_file(&mut |b| {
            let js =
                b.add(r#"<< /S /JavaScript /JS (AFNumber_Format\(2, 0, 0, 0, "", true\);) >>"#);
            let fields: Vec<String> = (0..50_000)
                .map(|k| {
                    format!("<< /T (field{k}) /FT /Tx /V (value {k}) /AA << /F {js} 0 R >> >>")
                })
                .collect();
            let fields = format!("[{}]", fields.join("\n"));
            let fields = b.add(fields);
            let k = b.add(format!("({list})"));
            if shape {
                (
                    format!("/AcroForm << /Fields {fields} 0 R >>"),
                    format!("<< /Keywords {k} 0 R /Title (Kept) >>"),
                )
            } else {
                (
                    format!("/Junk << /Fields {fields} 0 R >>"),
                    format!("<< /JunkKeywords {k} 0 R >>"),
                )
            }
        })
    };
    let out = against_twin(&file(true), &file(false));
    assert!(out.document.properties.keywords.len() > 1_000);
    assert!(out.document.facets.fields.len() > 1_000);
    limit_at(&out, "metadata", "keywords");
    form_cut(&out, "form", "form fields are not kept");
}

#[test]
fn megabyte_field_names_are_bounded() {
    // 40 fields under one parent whose name is 1 MB, named by one string object.
    let file = |key: &str| {
        form_file(key, &mut |b| {
            let name = b.add(format!("({})", "n".repeat(1 << 20)));
            let kids: Vec<String> = (0..40)
                .map(|k| format!("{} 0 R", b.add(format!("<< /T (k{k}) /FT /Tx /V (v) >>"))))
                .collect();
            vec![format!("<< /T {name} 0 R /Kids [{}] >>", kids.join(" "))]
        })
    };
    let out = against_twin(&file("/AcroForm"), &file("/Junk"));
    let kept = out.document.facets.fields.len();
    assert!(kept < 40, "{kept}");
    form_cut(&out, "form", "form fields are not kept");
}

#[test]
fn deep_field_trees_are_bounded() {
    // A chain of 10,000 nested fields: pdfparts stops at the depth limit and says so.
    let bytes = form_file("/AcroForm", &mut |b| {
        let mut inner = b.add("<< /T (leaf) /FT /Tx /V (deep) >>");
        for k in 0..10_000 {
            inner = b.add(format!("<< /T (n{k}) /Kids [{inner} 0 R] >>"));
        }
        vec![format!("<< /T (root) /Kids [{inner} 0 R] >>")]
    });
    let start = Instant::now();
    let out = read_through(&bytes);
    assert!(start.elapsed() < Duration::from_secs(60));
    assert!(out.document.facets.fields.is_empty());
    form_cut(&out, "form", "deeper than");
    assert_rss_under_256_mib();
}

/// Bytes just under the size one script may have at the hostile limits (4 MiB).
const NEAR_CAP: usize = 4_194_000;

#[test]
fn many_scripts_just_under_the_cap_are_bounded() {
    // 80 fields, each with its own packed calculate script just under the size a script may
    // have: together they are over the script text the document may hold.
    let packed = packed_script(NEAR_CAP);
    let file = |key: &str| {
        form_file(key, &mut |b| {
            (0..80)
                .map(|k| {
                    let js = b.stream("/Filter /FlateDecode", &packed);
                    let action = b.add(format!("<< /S /JavaScript /JS {js} 0 R >>"));
                    format!("<< /T (f{k}) /FT /Tx /AA << /C {action} 0 R >> >>")
                })
                .collect()
        })
    };
    let out = against_twin(&file("/AcroForm"), &file("/Junk"));
    let fields = &out.document.facets.fields;
    assert_eq!(fields.len(), 80);
    assert!(
        fields
            .iter()
            .all(|f| f.role == Some(rezzer_model::Role::Result))
    );
    form_cut(&out, "scripts", "listed without their text");
}

#[test]
fn many_scripts_in_one_chain_are_bounded() {
    // The open action starts a /Next chain of 80 packed scripts just under the size a script
    // may have; the twin names the chain under a key nobody reads.
    let packed = packed_script(NEAR_CAP);
    let file = |key: &str| {
        catalog_file(&mut |b| {
            let mut next = String::new();
            for _ in 0..80 {
                let js = b.stream("/Filter /FlateDecode", &packed);
                let action = b.add(format!("<< /S /JavaScript /JS {js} 0 R {next} >>"));
                next = format!("/Next {action} 0 R");
            }
            let first = next.trim_start_matches("/Next ").to_string();
            (format!("{key} {first}"), String::new())
        })
    };
    let out = against_twin(&file("/OpenAction"), &file("/JunkAction"));
    let opens = out
        .document
        .facets
        .embedded
        .iter()
        .filter(|e| e.name == "open")
        .count();
    assert_eq!(opens, 80);
    form_cut(&out, "scripts", "listed without their text");
}

#[test]
fn a_shared_script_counts_once() {
    // 80 fields share one format script just under the size a script may have: pdfparts reads
    // and holds it once, within the script text the document may hold.
    let packed = packed_script(NEAR_CAP);
    let bytes = form_file("/AcroForm", &mut |b| {
        let js = b.stream("/Filter /FlateDecode", &packed);
        let action = b.add(format!("<< /S /JavaScript /JS {js} 0 R >>"));
        (0..80)
            .map(|k| format!("<< /T (f{k}) /FT /Tx /AA << /F {action} 0 R >> >>"))
            .collect()
    });
    let out = read(&bytes);
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("listed without their text")),
        "{:?}",
        out.document.losses
    );
    // pdfparts holds the text for every field: each field's format is its start in words.
    let fields = &out.document.facets.fields;
    assert_eq!(fields.len(), 80);
    assert!(fields.iter().all(|f| f.format.is_some()), "{fields:?}");
    // The reader's own share keeps only a few copies of the text in its list of scripts.
    let loss = limit_at(
        &out,
        "scripts",
        "scripts are kept without their text, the first",
    );
    assert!(
        loss.what.starts_with(|c: char| c.is_ascii_digit()),
        "{loss:?}"
    );
}

#[test]
fn a_link_chain_of_many_addresses_is_bounded() {
    // One link to an address whose /Next names 200,000 more: the first is in the links
    // graph, the others are listed as link actions as far as the share of the form logic
    // goes. The twin names the same array under a key nobody reads.
    // Shape and twin are built and read one after the other.
    let start = Instant::now();
    drop(read_through(&annotated_chain("JunkNext")));
    let twin = common::max_rss_bytes();
    let out = read_through(&annotated_chain("Next"));
    let rss = common::max_rss_bytes();
    // Measured at the 512 MiB limits: the twin takes 210 MiB in this process (building the
    // file and parsing 200,000 dictionaries), the shape 55 MiB more. Of that, 39 MiB come
    // before the form logic and the scripts are read at all, 10 MiB are pdfparts' list of
    // actions, which the room for actions now bounds (it took 85 MiB before), and 6 MiB the
    // reader's copies. The twin alone is near 256 MiB here, so the bound over it is the
    // twin rule, and the whole read stays far under the memory limit.
    assert!(
        rss < twin + (64 << 20) && rss < 320 << 20,
        "max RSS {} MiB, twin {} MiB",
        rss >> 20,
        twin >> 20
    );
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
    let listed = out
        .document
        .facets
        .embedded
        .iter()
        .filter(|e| e.detail.as_deref() == Some("uri"))
        .count();
    assert!(listed > 1_000 && listed < 200_000, "{listed}");
    limit_at(&out, "scripts", "action chain is cut short");
}

/// A one-page file with one link to an address whose `key` (`Next` for the shape) names an
/// array of 200,000 more address actions.
fn annotated_chain(key: &str) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let content = b.stream("", b"BT ET");
    let list: Vec<String> = (0..200_000)
        .map(|k| format!("<< /S /URI /URI (http://h{k}.example/) >>"))
        .collect();
    let next = b.add(format!("[{}]", list.join("\n")));
    drop(list);
    let link = b.add(format!(
        "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /URI \
         /URI (http://first.example/) /{key} {next} 0 R >> >>"
    ));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /Contents {content} 0 R \
         /Annots [{link} 0 R] >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// A page of 80 lines of 50 words, a URI link over each word, and with `tall` one more link
/// over the whole page.
fn linked_words(tall: bool) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut content = String::from("BT /F1 6 Tf ");
    let mut annots = Vec::new();
    let mut link = |b: &mut Builder, rect: String, k: usize| {
        annots.push(format!(
            "{} 0 R",
            b.add(format!(
                "<< /Type /Annot /Subtype /Link /Rect [{rect}] \
                 /A << /S /URI /URI (https://example.org/{k}) >> >>"
            ))
        ));
    };
    if tall {
        link(&mut b, "0 0 595 842".into(), 0);
    }
    for r in 0..80 {
        let y = 820 - 10 * r;
        for c in 0..50 {
            let x = 10 + 11 * c;
            content += &format!("1 0 0 1 {x} {y} Tm (ab) Tj ");
            link(
                &mut b,
                format!("{x} {} {} {}", y - 1, x + 7, y + 5),
                r * 50 + c + 1,
            );
        }
    }
    content += "ET\n";
    let c = b.stream("/Filter /FlateDecode", &deflate(content.as_bytes()));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R /Annots [{}] >>",
        annots.join(" ")
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn links_are_matched_per_glyph_within_bounds() {
    // 4,000 small links: each glyph tests only the links at its height, and every word
    // keeps its own link.
    let start = Instant::now();
    let out = read(&linked_words(false));
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    assert!(
        md.contains("[ab](https://example.org/1)"),
        "{}",
        &md[..2000]
    );
    assert!(
        md.contains("[ab](https://example.org/4000)"),
        "{}",
        &md[..2000]
    );
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("too many links")),
        "{:?}",
        out.document.losses
    );
    // One link over the whole page makes every glyph test the links below it: past the
    // work bound no glyph keeps a link, and the loss says so.
    let start = Instant::now();
    let out = read(&linked_words(true));
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
    assert!(
        losses(&out, LossReason::Limit)
            .iter()
            .any(|l| l.what.contains("too many links")),
        "{:?}",
        out.document.losses
    );
    assert_rss_under_256_mib();
}

/// A one-page file of `w` by `h` points with Helvetica as `/F1`.
fn sized_page(content: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let cs = b.stream("/Filter /FlateDecode", &deflate(content));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// The loss that says the drawn boxes of page 1 are not used as reading regions.
fn box_loss(out: &ReadOutcome) -> Option<&Loss> {
    out.document.losses.iter().find(|l| {
        l.what.contains("page 1")
            && l.what.contains("drawn boxes")
            && l.what.contains("are not used as reading regions")
    })
}

fn assert_quick(start: Instant) {
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    assert_rss_under_256_mib();
}

#[test]
fn many_boxes_with_text_are_bounded() {
    // 4,500 boxes in a column on a tall page, each with a word: more than a page may use as
    // reading regions. The text is read, and a loss says the boxes are not used.
    let start = Instant::now();
    let n = 4_500usize;
    let h = (n + 2) * 20;
    let mut c = b"0.5 w ".to_vec();
    for i in 0..n {
        let y = h - 20 - i * 20;
        c.extend(
            format!(
                "72 {y} 100 16 re S BT /F1 8 Tf 76 {} Td (b{i}) Tj ET\n",
                y + 5
            )
            .as_bytes(),
        );
    }
    let out = read(&tall_page(&c, h));
    assert_quick(start);
    let loss = box_loss(&out).unwrap_or_else(|| panic!("{:?}", out.document.losses));
    assert_eq!(loss.why, LossReason::Limit);
    assert!(text(page(&out, 1)).contains("b4499"));
}

#[test]
fn many_rounded_boxes_are_bounded() {
    // 60,000 small rounded boxes (four corner curves and four sides each): more straight
    // lines than the box search looks at.
    let start = Instant::now();
    let mut c = b"BT /F1 12 Tf 72 2500 Td (Before.) Tj ET 0.1 w ".to_vec();
    for k in 0..60_000usize {
        let (x, y) = ((10 + (k % 250) * 10) as f32, (10 + (k / 250) * 10) as f32);
        let (x1, y1) = (x + 8.0, y + 8.0);
        c.extend(
            format!(
                "{} {y} m {} {y} l {x1} {y} {x1} {y} {x1} {} c {x1} {} l {x1} {y1} {x1} {y1} {} {y1} c \
                 {} {y1} l {x} {y1} {x} {y1} {x} {} c {x} {} l {x} {y} {x} {y} {} {y} c h S\n",
                x + 1.0,
                x1 - 1.0,
                y + 1.0,
                y1 - 1.0,
                x1 - 1.0,
                x + 1.0,
                y1 - 1.0,
                y + 1.0,
                x + 1.0,
            )
            .as_bytes(),
        );
    }
    let out = read(&sized_page(&c, 2_600, 2_600));
    assert_quick(start);
    assert!(text(page(&out, 1)).contains("Before."));
    let loss = box_loss(&out).unwrap_or_else(|| panic!("{:?}", out.document.losses));
    assert_eq!(loss.why, LossReason::Limit);
}

#[test]
fn deeply_nested_boxes_are_bounded() {
    // 1,000 boxes inside each other, 2.5 points apart, with two lines in the innermost one,
    // and two lines on the same baselines to the right of all of them. The inner box is
    // read whole.
    let start = Instant::now();
    let (w, h) = (6_000usize, 6_000usize);
    let mut c = b"0.2 w ".to_vec();
    for k in 0..1_000usize {
        let d = 2.5 * k as f32;
        c.extend(
            format!(
                "{} {} {} {} re S\n",
                2_800.0 - d,
                2_900.0 - d,
                200.0 + 2.0 * d,
                40.0 + 2.0 * d
            )
            .as_bytes(),
        );
    }
    c.extend(b"BT /F1 10 Tf 2810 2925 Td (Inner one) Tj ET BT /F1 10 Tf 2810 2911 Td (Inner two) Tj ET\n");
    c.extend(b"BT /F1 10 Tf 5600 2925 Td (Outer one) Tj ET BT /F1 10 Tf 5600 2911 Td (Outer two) Tj ET\n");
    let out = read(&sized_page(&c, w, h));
    assert_quick(start);
    let body = text(page(&out, 1));
    let at = |t: &str| body.find(t).unwrap_or_else(|| panic!("{t} in {body}"));
    assert!(at("Inner two") < at("Outer one"), "{body}");
}

#[test]
fn overlapping_boxes_are_bounded() {
    // 3,000 boxes of 300 points, each 2.1 points right of and below the one before: their
    // sides cross everywhere. A guard: the page is read and stays small and quick.
    let start = Instant::now();
    let mut c = b"BT /F1 12 Tf 72 7900 Td (Before.) Tj ET 0.1 w ".to_vec();
    for k in 0..3_000usize {
        let (x, y) = (10.0 + 2.1 * k as f32, 7_500.0 - 2.1 * k as f32);
        c.extend(
            format!(
                "{x} {y} 300 300 re S BT /F1 4 Tf {} {} Td (o{k}) Tj ET\n",
                x + 2.0,
                y + 2.0
            )
            .as_bytes(),
        );
    }
    let out = read(&sized_page(&c, 8_000, 8_000));
    assert_quick(start);
    assert!(text(page(&out, 1)).contains("Before."));
}

#[test]
fn thousands_of_boxes_side_by_side_are_read_whole_within_bounds() {
    // 4,000 boxes, two in each row, each with two lines on the same baselines as the box
    // beside it: every box is read whole, quick and small.
    let start = Instant::now();
    let rows = 2_000usize;
    let h = (rows + 2) * 40;
    let mut c = b"0.5 w ".to_vec();
    for r in 0..rows {
        let top = h - 20 - r * 40;
        for (side, x) in [("l", 72usize), ("r", 300)] {
            c.extend(format!("{x} {} 200 32 re S ", top - 32).as_bytes());
            c.extend(
                format!(
                    "BT /F1 8 Tf {} {} Td ({side}{r:04}a) Tj ET BT /F1 8 Tf {} {} Td ({side}{r:04}b) Tj ET\n",
                    x + 6,
                    top - 12,
                    x + 6,
                    top - 24
                )
                .as_bytes(),
            );
        }
    }
    let out = read(&tall_page(&c, h));
    assert_quick(start);
    let body = text(page(&out, 1));
    let at = |t: &str| body.find(t).unwrap_or_else(|| panic!("{t} missing"));
    for r in [0usize, 1_000, 1_999] {
        let (la, lb, ra) = (
            at(&format!("l{r:04}a")),
            at(&format!("l{r:04}b")),
            at(&format!("r{r:04}a")),
        );
        assert!(la < lb && lb < ra, "box {r} not read whole");
    }
    assert!(box_loss(&out).is_none(), "{:?}", out.document.losses);
}

/// A one-page file of `content`, deflated, on a page of `w` by `h` points with `extra` in
/// the page dictionary.
fn turned_page_of(content: &[u8], w: usize, h: usize, extra: &str) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let cs = b.stream("/Filter /FlateDecode", &deflate(content));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] {extra} \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// A table of eight columns between column rules, with a header rule and `records` records
/// of one-letter cells and no rules between them, drawn upward on a page turned for display.
/// Many short ticks beside it give the page many rules.
fn turned_records(records: usize) -> Vec<u8> {
    let long = (records + 4) * 10;
    // The content is drawn in a frame `long` high and 842 wide, turned onto a page `long`
    // wide and 842 high.
    let mut c = format!("q 0 1 -1 0 {long} 0 cm 0.5 w ").into_bytes();
    let xs: Vec<usize> = (0..=8).map(|k| 40 + 60 * k).collect();
    let (top, head, bottom) = (long - 10, long - 30, 10);
    for y in [top, head, bottom] {
        c.extend(format!("{} {y} m {} {y} l S ", xs[0], xs[8]).as_bytes());
    }
    for x in &xs {
        c.extend(format!("{x} {bottom} m {x} {top} l S ").as_bytes());
    }
    for k in 0..8_000usize {
        let y = 20 + k * long / 8_000;
        c.extend(format!("600 {y} m 610 {y} l S ").as_bytes());
    }
    for (k, x) in xs[..8].iter().enumerate() {
        c.extend(format!("BT /F1 8 Tf {} {} Td (H{k}) Tj ET\n", x + 4, head + 6).as_bytes());
    }
    for r in 0..records {
        let y = head - 10 - r * 10;
        c.extend(b"BT /F1 8 Tf ");
        for (k, x) in xs[..8].iter().enumerate() {
            let at = if k == 0 { (*x + 4, y) } else { (60, 0) };
            c.extend(format!("{} {} Td (x) Tj ", at.0, at.1).as_bytes());
        }
        c.extend(b"ET\n");
    }
    c.extend(b"Q\n");
    turned_page_of(&c, long, 842, "/Rotate 90")
}

fn cells_loss(out: &ReadOutcome) -> Option<&Loss> {
    out.document
        .losses
        .iter()
        .find(|l| l.what.contains("page 1") && l.what.contains("more than 100000 cells"))
}

#[test]
fn a_turned_table_of_many_records_is_bounded() {
    // A turned page with many rules and a ruled table of 12,000 records without rules
    // between them: each record is a row, quick and small.
    let start = Instant::now();
    let out = read(&turned_records(12_000));
    assert_quick(start);
    let rows = page(&out, 1)
        .iter()
        .find_map(|b| match b {
            Block::Table { rows, .. } => Some(rows.len()),
            _ => None,
        })
        .unwrap_or(0);
    assert_eq!(rows, 12_001, "{:?}", out.document.losses);
    assert!(cells_loss(&out).is_none(), "{:?}", out.document.losses);
}

#[test]
fn a_turned_table_past_the_cell_bound_says_so() {
    // 13,000 records of eight cells are more cells than one table may have: the text stays
    // paragraphs and a loss says why.
    let start = Instant::now();
    let out = read(&turned_records(13_000));
    assert_quick(start);
    assert!(cells_loss(&out).is_some(), "{:?}", out.document.losses);
    assert!(text(page(&out, 1)).contains('x'));
}

#[test]
fn thousands_of_key_value_lines_are_bounded() {
    // 60,000 lines of a label and a value set flush right: more cells than one table may
    // have. The text is read, quick and small, and a loss says why it is no table.
    let start = Instant::now();
    let n = 60_000usize;
    let h = (n + 4) * 10;
    let mut c = Vec::new();
    for i in 0..n {
        let y = h - 20 - i * 10;
        c.extend(format!("BT /F1 8 Tf 40 {y} Td (k:) Tj 200 0 Td (1) Tj ET\n").as_bytes());
    }
    let out = read(&tall_page(&c, h));
    assert_quick(start);
    assert!(cells_loss(&out).is_some(), "{:?}", out.document.losses);
    assert!(text(page(&out, 1)).contains("k:"));
}

#[test]
fn a_header_band_over_a_huge_body_is_bounded() {
    // A header band of five labelled cells over 25,000 lines of three pieces each: more cells
    // than one table may have. Quick and small, and a loss says why it is no table.
    let start = Instant::now();
    let n = 25_000usize;
    let h = (n + 10) * 10;
    let top = h - 20;
    let mut c = b"0.9 g ".to_vec();
    for x in [30, 130, 230, 330, 430] {
        c.extend(format!("{x} {} 100 14 re f ", top - 14).as_bytes());
    }
    c.extend(
        format!(
            "0 g 0.5 w 30 {top} m 530 {top} l S 30 {} m 530 {} l S ",
            top - 14,
            top - 14
        )
        .as_bytes(),
    );
    for (k, x) in [30, 130, 230, 330, 430].iter().enumerate() {
        let title = ["Date", "Text", "Note", "Charges", "Credits"][k];
        c.extend(format!("BT /F1 8 Tf {} {} Td ({title}) Tj ET\n", x + 4, top - 10).as_bytes());
    }
    for i in 0..n {
        let y = top - 30 - i * 10;
        c.extend(
            format!("BT /F1 8 Tf 34 {y} Td (a) Tj 100 0 Td (b) Tj 200 0 Td (1) Tj ET\n").as_bytes(),
        );
    }
    let out = read(&tall_page(&c, h));
    assert_quick(start);
    assert!(cells_loss(&out).is_some(), "{:?}", out.document.losses);
}

/// A one-page file of `content`, deflated, `w` by `h` points, with Helvetica as `/F1` and
/// Helvetica-Bold as `/F2`.
fn two_font_page(content: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let bold = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>");
    let cs = b.stream("/Filter /FlateDecode", &deflate(content));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {w} {h}] \
         /Resources << /Font << /F1 {font} 0 R /F2 {bold} 0 R >> >> /Contents {cs} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

#[test]
fn a_header_band_of_thousands_of_empty_cells_is_bounded() {
    // A band of three titled cells and 5,700 narrow empty ones over 4,700 lines of three
    // short pieces, on a page of 14,400 points: the table is only as wide as its titled
    // columns, quick and small.
    let start = Instant::now();
    let (w, top) = (14_400usize, 14_390.0f32);
    let mut c = b"0.1 w ".to_vec();
    let mut xs: Vec<f32> = vec![10.0, 30.0, 50.0, 70.0];
    xs.extend((0..5_700).map(|k| 70.0 + 2.5 * (k + 1) as f32));
    let end = xs[xs.len() - 1];
    c.extend(
        format!(
            "10 {top} m {end} {top} l S 10 {0} m {end} {0} l S ",
            top - 3.0
        )
        .as_bytes(),
    );
    for x in &xs {
        c.extend(format!("{x} {} m {x} {top} l S ", top - 3.0).as_bytes());
    }
    for (k, t) in ["Date", "Text", "Amount"].iter().enumerate() {
        c.extend(format!("BT /F1 1 Tf {} {} Td ({t}) Tj ET\n", 11 + 20 * k, top - 2.0).as_bytes());
    }
    for i in 0..4_700usize {
        let y = top - 6.0 - 3.0 * i as f32;
        c.extend(
            format!("BT /F1 1 Tf 11 {y} Td (a) Tj 20 0 Td (b) Tj 20 0 Td (1) Tj ET\n").as_bytes(),
        );
    }
    let out = read(&two_font_page(&c, w, w));
    assert_quick(start);
    let widths: Vec<usize> = page(&out, 1)
        .iter()
        .filter_map(|b| match b {
            Block::Table { rows, .. } => rows.iter().map(Vec::len).max(),
            _ => None,
        })
        .collect();
    assert!(widths.iter().all(|w| *w <= 3), "{widths:?}");
}

/// A page of `lines` lines of 25 one-letter words in two fonts, with `bands` one-row grids of two
/// cells, each as tall as the page.
fn tall_bands(bands: usize, lines: usize) -> Vec<u8> {
    let w = (bands * 12 + 40).max(612);
    let h = lines * 12 + 40;
    let mut c = b"0.2 w ".to_vec();
    for k in 0..bands {
        let x = 20 + k * 12;
        c.extend(
            format!(
                "{x} 10 m {} 10 l S {x} {t} m {} {t} l S {x} 10 m {x} {t} l S {} 10 m {} {t} l S \
                 {} 10 m {} {t} l S ",
                x + 6,
                x + 6,
                x + 3,
                x + 3,
                x + 6,
                x + 6,
                t = h - 10
            )
            .as_bytes(),
        );
    }
    for i in 0..lines {
        let y = h - 25 - i * 12;
        c.extend(format!("BT /F1 9 Tf 20 {y} Td ").as_bytes());
        for j in 0..25 {
            let f = if j % 2 == 1 { "/F2" } else { "/F1" };
            c.extend(format!("{f} 9 Tf (a) Tj 15 0 Td ").as_bytes());
        }
        c.extend(b"ET\n");
    }
    two_font_page(&c, w, h)
}

#[test]
fn many_tall_one_row_grids_cost_about_as_much_as_their_text() {
    // 800 one-row grids of two cells, each as tall as the page, over 8,000 lines of 25 words:
    // about as quick as the same page without the grids.
    let start = Instant::now();
    let _twin = read(&tall_bands(0, 8_000));
    let twin = start.elapsed();
    let start = Instant::now();
    let out = read(&tall_bands(800, 8_000));
    let took = start.elapsed();
    assert!(
        took < twin * 2 + Duration::from_secs(1),
        "{took:?} against {twin:?}"
    );
    // Both files are read in this process, so its peak memory is not that of one read; the
    // time against the twin is the measure here.
    assert!(text(page(&out, 1)).contains('a'));
}
