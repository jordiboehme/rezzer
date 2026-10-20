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
    Anchor, Block, Bytes, CalloutKind, Completeness, Confidence, Detection, FormatId, Loss,
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
fn links_on_a_turned_page_are_said() {
    // All text runs upward, so the page is read in a turned frame.
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
    assert!(text(page(&out, 1)).contains("Read the guide"));
    assert!(
        out.document
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

/// The one Limit loss that says link targets were left out.
fn link_loss(out: &ReadOutcome) -> &Loss {
    let found: Vec<&Loss> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(found.len(), 1, "{:?}", out.document.losses);
    assert!(found[0].what.contains("link"), "{:?}", found[0]);
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
