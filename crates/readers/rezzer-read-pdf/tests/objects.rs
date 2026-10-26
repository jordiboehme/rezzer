// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The object layer in the model: metadata, outline, links, attachments, layers, threads and
//! revisions.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{Builder, Page, append_update, pdf, show};
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Block, Bytes, Completeness, Confidence, Detection, FormatId, LossReason, ReadOptions,
    ReadOutcome, Reader, SectionKind, Severity, Source, Value, plain_text,
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
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
    }
}

fn read_opts(bytes: &[u8], opts: &ReadOptions, limits: Limits) -> ReadOutcome {
    PdfReader.read(&source(bytes), opts, &Budget::new(limits))
}

fn read(bytes: &[u8]) -> ReadOutcome {
    read_opts(bytes, &ReadOptions::default(), Limits::default())
}

fn text(v: &str) -> Value {
    Value::Text {
        value: v.to_string(),
    }
}

/// The texts of a nested list: each item's own text with its depth, in reading order.
fn list_items(block: &Block, depth: usize, out: &mut Vec<(usize, String)>) {
    if let Block::List { items, .. } = block {
        for item in items {
            for b in item {
                match b {
                    Block::Paragraph { inlines, .. } => out.push((depth, plain_text(inlines))),
                    Block::List { .. } => list_items(b, depth + 1, out),
                    other => panic!("{other:?}"),
                }
            }
        }
    }
}

#[test]
fn info_dates_keep_offset() {
    let out = read(&corpus("irs-f1040-2025.pdf"));
    let p = &out.document.properties;
    let date = Value::DateTime {
        value: "2026-01-02T07:24:31-06:00".into(),
    };
    assert_eq!(p.created.as_ref(), Some(&date));
    assert_eq!(p.modified.as_ref(), Some(&date));
    // One author: the Info author is not split.
    assert_eq!(p.authors, ["C:DC:TS:CAR:MP"]);
    assert_eq!(p.keywords, ["Fillable"]);
    assert_eq!(p.language.as_deref(), Some("en-US"));
    assert_eq!(p.custom.get("producer"), Some(&text("Designer 6.5")));
    assert_eq!(p.custom.get("creator"), Some(&text("Designer 6.5")));
    assert_eq!(p.custom.get("pdf_version"), Some(&text("1.7")));
    assert!(
        matches!(p.custom.get("subject"), Some(Value::Text { value }) if value.starts_with("U.S. Individual Income T")),
        "{:?}",
        p.custom
    );
    assert_eq!(p.facts.encrypted, Some(false));
    assert_eq!(p.facts.pages, Some(2));
}

/// A one-page file with `info` as its Info dictionary and `xmp` as its XMP stream.
fn with_metadata(info: &str, xmp: &str) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream("", show("F1", 12.0, 72.0, 700.0, "Hello.").as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    let meta = b.stream("/Type /Metadata /Subtype /XML", xmp.as_bytes());
    let info = b.add(info);
    b.set(
        catalog,
        format!("<< /Type /Catalog /Pages {tree} 0 R /Metadata {meta} 0 R >>"),
    );
    b.finish_with(catalog, &format!("/Info {info} 0 R"))
}

const XMP: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:pdf="http://ns.adobe.com/pdf/1.3/"
    xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/"
    pdfaid:part="2" pdfaid:conformance="B">
   <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Title From XMP</rdf:li></rdf:Alt></dc:title>
   <dc:creator><rdf:Seq><rdf:li>Ada</rdf:li><rdf:li>Grace</rdf:li></rdf:Seq></dc:creator>
   <pdf:Keywords>tax; form, 2025</pdf:Keywords>
   <pdf:Producer>XMP Producer</pdf:Producer>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

#[test]
fn title_falls_back_to_xmp() {
    let out = read(&with_metadata("<< /Producer (Info Producer) >>", XMP));
    // A file saved once has no history to tell.
    assert!(out.document.facets.timeline.is_empty());
    let p = &out.document.properties;
    assert_eq!(p.facts.revisions, Some(1));
    assert_eq!(p.title.as_deref(), Some("Title From XMP"));
    // The XMP creator list gives the authors; keywords split at commas and semicolons.
    assert_eq!(p.authors, ["Ada", "Grace"]);
    assert_eq!(p.keywords, ["tax", "form", "2025"]);
    assert_eq!(p.custom.get("pdfa"), Some(&text("PDF/A-2b")));
    // Info wins over XMP; the XMP value that differs is kept apart.
    assert_eq!(p.custom.get("producer"), Some(&text("Info Producer")));
    assert_eq!(p.custom.get("xmp_producer"), Some(&text("XMP Producer")));
    assert_eq!(p.custom.get("xmp_title"), None);

    let out = read(&with_metadata(
        "<< /Title (Info Title) /Keywords (\"one, two\") >>",
        XMP,
    ));
    let p = &out.document.properties;
    assert_eq!(p.title.as_deref(), Some("Info Title"));
    assert_eq!(p.custom.get("xmp_title"), Some(&text("Title From XMP")));
    // A list written in quotes loses them.
    assert_eq!(p.keywords, ["one", "two"]);
}

#[test]
fn outline_section_with_page_labels() {
    let out = read(&corpus("qpdf-page-labels-and-outlines.pdf"));
    let body = &out.document.body;
    let Block::Section {
        kind: SectionKind::Other,
        title: Some(title),
        anchor: Some(Anchor(anchor)),
        blocks,
    } = &body[0]
    else {
        panic!("{:?}", body[0]);
    };
    assert_eq!((title.as_str(), anchor.as_str()), ("Outline", "outline"));
    assert!(
        matches!(&body[1], Block::Section { kind: SectionKind::Page, anchor: Some(a), .. } if a.0 == "p1")
    );
    assert_eq!(blocks.len(), 1);
    let mut items = Vec::new();
    list_items(&blocks[0], 1, &mut items);
    assert_eq!(items.len(), 11, "{items:?}");
    let depths: Vec<usize> = items.iter().map(|(d, _)| *d).collect();
    assert_eq!(depths, [1, 2, 3, 4, 4, 3, 4, 2, 3, 3, 1]);
    // Each item names its page by label, the first pages by their number. Pages 2 and 13
    // are both called 2, so their items add the page number of the file.
    let texts: Vec<&str> = items.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(texts[0], "Isís 1 -> 5: /XYZ null null null, page iv");
    assert!(
        texts[2].ends_with(", page 2 (page 13 of the file)"),
        "{texts:?}"
    );
    assert!(
        texts[8].ends_with(", page 2 (page 2 of the file)"),
        "{texts:?}"
    );
    assert!(texts[9].ends_with(", page 1"), "{texts:?}");
    assert!(
        items.iter().all(|(_, t)| !t.ends_with("page ")),
        "{items:?}"
    );
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    let outline = md
        .split("## Outline")
        .nth(1)
        .and_then(|rest| rest.split("Page 1").next())
        .expect("an outline section");
    assert!(outline.contains(", page iv"), "{outline}");
    assert!(!outline.contains("]("), "{outline}");
    assert!(!outline.contains("](#"), "{outline}");
}

#[test]
fn outline_entries_with_levels_and_pages() {
    let out = read(&corpus("qpdf-page-labels-and-outlines.pdf"));
    let entries = &out.document.facets.outline;
    // The levels the Outline section shows, and the page in the file (1-based, as the page
    // anchors `p<n>`), not the page label: the title's `-> n` is the 0-based page.
    let levels: Vec<u8> = entries.iter().map(|e| e.level).collect();
    assert_eq!(levels, [1, 2, 3, 4, 4, 3, 4, 2, 3, 3, 1]);
    let pages: Vec<Option<u32>> = entries.iter().map(|e| e.page).collect();
    assert_eq!(pages, [6, 12, 13, 19, 20, 13, 23, 14, 2, 1, 16].map(Some));
    assert_eq!(entries[0].title, "Isís 1 -> 5: /XYZ null null null");
    assert_eq!(entries[10].title, "Trepak 2 -> 15: /XYZ 66 756 3");
    // The same items in the same order as the section.
    let Block::Section { blocks, .. } = &out.document.body[0] else {
        panic!("an outline section");
    };
    let mut items = Vec::new();
    list_items(&blocks[0], 1, &mut items);
    assert_eq!(entries.len(), items.len());
    for (entry, (depth, text)) in entries.iter().zip(&items) {
        assert_eq!(usize::from(entry.level), *depth);
        assert!(
            text.starts_with(&format!("{}, page ", entry.title)),
            "{text}"
        );
    }
}

#[test]
fn link_graph_counts_internal_links() {
    let out = read(&corpus("pypdf-outline.pdf"));
    let graph = out
        .document
        .facets
        .graphs
        .iter()
        .find(|g| g.id == "links")
        .expect("a links graph");
    assert_eq!(graph.kind, "link");
    let edges: Vec<(&str, &str)> = graph
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    assert_eq!(edges.len(), 9, "{edges:?}");
    assert!(graph.edges.iter().all(|e| e.kind == "links_to"));
    assert!(edges.iter().all(|(from, _)| *from == "p1"), "{edges:?}");
    assert!(edges.contains(&("p1", "p2")), "{edges:?}");
    for (_, to) in &edges {
        let node = graph.nodes.iter().find(|n| n.id == *to).expect("node");
        assert_eq!(node.kind.as_deref(), Some("page"));
        assert_eq!(node.anchor.as_ref().map(|a| a.0.as_str()), Some(*to));
    }
    // Node ids are unique.
    let mut ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), graph.nodes.len());
}

#[test]
fn uri_launch_and_remote_links_are_listed_not_followed() {
    let link = |action: &str| {
        format!("<< /Type /Annot /Subtype /Link /Rect [72 690 200 712] /A << {action} >> >>")
    };
    let page = Page {
        content: show("F1", 12.0, 72.0, 700.0, "Links."),
        annots: vec![
            link("/S /URI /URI (https://example.com/a)"),
            link("/S /URI /URI (https://example.com/a)"),
            link("/S /Launch /F (run.exe)"),
            link("/S /GoToR /F (other.pdf) /D [2 /Fit]"),
            link("/S /JavaScript /JS (app.alert(1))"),
        ],
        ..Page::default()
    };
    let out = read(&pdf(&[page], ""));
    let graph = &out.document.facets.graphs[0];
    let uris: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.kind.as_deref() == Some("uri"))
        .map(|n| n.label.as_str())
        .collect();
    // One node per URI, one edge per link.
    assert_eq!(uris, ["https://example.com/a"]);
    assert_eq!(graph.edges.len(), 2);
    let embedded: Vec<(&str, &str, Option<&str>)> = out
        .document
        .facets
        .embedded
        .iter()
        .map(|e| (e.kind.as_str(), e.name.as_str(), e.target.as_deref()))
        .collect();
    assert_eq!(
        embedded,
        [
            ("launch-action", "run.exe", Some("run.exe")),
            ("remote-link", "other.pdf", Some("other.pdf")),
            // The script link is listed with the scripts, with its text.
            ("script", "link page 1", None),
        ]
    );
    let remote = &out.document.facets.embedded[1];
    assert_eq!(
        remote.detail.as_deref(),
        Some("from page 1, to page 3 of that file")
    );
    let script = &out.document.facets.embedded[2];
    assert_eq!(script.detail.as_deref(), Some("javascript"));
    assert_eq!(script.text.as_deref(), Some("app.alert(1)"));
}

#[test]
fn attachment_listed_and_extracted_only_on_request() {
    let bytes = corpus("pypdf-attachment.pdf");
    let out = read(&bytes);
    let listed: Vec<(&str, &str, Option<&str>)> = out
        .document
        .facets
        .embedded
        .iter()
        .map(|e| (e.kind.as_str(), e.name.as_str(), e.detail.as_deref()))
        .collect();
    assert_eq!(listed, [("attachment", "image.png", Some("6669 bytes"))]);
    assert!(out.document.assets.is_empty());
    assert!(out.blobs.is_empty());

    let opts = ReadOptions {
        extract_assets: true,
        ..ReadOptions::default()
    };
    let out = read_opts(&bytes, &opts, Limits::default());
    assert_eq!(out.document.assets.len(), 1);
    let asset = &out.document.assets[0];
    assert_eq!(asset.id, "attachment-1");
    assert_eq!(asset.mime, "application/octet-stream");
    assert_eq!(asset.caption.as_deref(), Some("image.png"));
    assert!(!asset.needs_vision);
    assert_eq!(asset.sha256.len(), 64);
    assert_eq!(out.blobs.len(), 1);
    assert_eq!(out.blobs[0].asset, "attachment-1");
    assert_eq!(out.blobs[0].bytes.len(), 6669);
    assert!(out.blobs[0].bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn attachment_over_the_limit_has_no_blob() {
    // Two megabytes of zeros, packed, with a description and a type.
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream("", show("F1", 12.0, 72.0, 700.0, "Hello.").as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    let packed = common::zeros_packed(2 << 20);
    let file = b.stream(
        "/Type /EmbeddedFile /Subtype /application#2Foctet-stream /Filter /FlateDecode",
        &packed,
    );
    let spec = b.add(format!(
        "<< /Type /Filespec /F (zeros.bin) /Desc (Only zeros) /EF << /F {file} 0 R >> >>"
    ));
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R \
             /Names << /EmbeddedFiles << /Names [(zeros.bin) {spec} 0 R] >> >> >>"
        ),
    );
    let bytes = b.finish(catalog);
    let opts = ReadOptions {
        extract_assets: true,
        ..ReadOptions::default()
    };
    // An eighth of 8 MiB is less than the file.
    let limits = Limits {
        max_memory_bytes: 8 << 20,
        ..Limits::default()
    };
    let out = read_opts(&bytes, &opts, limits);
    let e = &out.document.facets.embedded[0];
    assert_eq!(e.name, "zeros.bin");
    assert_eq!(
        e.detail.as_deref(),
        Some("application/octet-stream, 2097152 bytes, Only zeros")
    );
    assert!(out.document.assets.is_empty());
    assert!(out.blobs.is_empty());
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.location == "attachment 1")
        .expect("a loss for the attachment");
    assert_eq!(loss.why, LossReason::Limit);
    assert!(loss.what.contains("zeros.bin"), "{loss:?}");
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn layers_in_properties_and_facts() {
    let out = read(&corpus("pypdf-ocg-layers.pdf"));
    let p = &out.document.properties;
    assert_eq!(p.facts.layers, Some(6));
    assert_eq!(
        p.custom.get("layers"),
        Some(&text(
            "North Arrow (shown), Scale Bar (shown), Map Frame (shown), Map (shown), \
             Light Gray Base (shown), Data Source Attribution (shown)"
        ))
    );
}

#[test]
fn threads_listed() {
    let out = read(&corpus("pdfbox-poems-article-threads.pdf"));
    let threads: Vec<(&str, Option<&str>)> = out
        .document
        .facets
        .embedded
        .iter()
        .filter(|e| e.kind == "article-thread")
        .map(|e| (e.name.as_str(), e.detail.as_deref()))
        .collect();
    assert_eq!(
        threads,
        [
            ("Erlkönig", Some("page 1")),
            ("Moulière", Some("pages 1, 2"))
        ]
    );
}

#[test]
fn revisions_on_timeline() {
    // The file holds two saves: it is linearized, and its first section is part of the first.
    let out = read(&corpus("pdfbox-sf1199a-direct-deposit.pdf"));
    let events = &out.document.facets.timeline;
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(events.iter().all(|e| e.kind == "revision"));
    assert_eq!(events[0].text.as_deref(), Some("first save"));
    assert_eq!(out.document.properties.facts.revisions, Some(2));
}

#[test]
fn a_third_save_adds_an_event() {
    // A third save fills two fields, as a person would.
    let mut bytes = corpus("pdfbox-sf1199a-direct-deposit.pdf");
    append_update(
        &mut bytes,
        &[
            (
                106,
                b"<</T(Name)/DA(/Helv 9 Tf 0 0 0.5 rg)/FT/Tx/Ff 12582912/V(Jane Doe)/Kids[160 0 R 39 0 R 78 0 R]>>".to_vec(),
            ),
            (
                109,
                b"<</T(Phone)/DA(/Helv 9 Tf 0 0 0.5 rg)/FT/Tx/AA<</F 143 0 R/K 144 0 R>>/Ff 12582912/V(5551234)/Kids[163 0 R 40 0 R 81 0 R]>>".to_vec(),
            ),
        ],
        "/Size 271 /Root 156 0 R /Info 154 0 R",
    );
    let out = read(&bytes);
    let events = &out.document.facets.timeline;
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(events.iter().all(|e| e.kind == "revision"));
    let texts: Vec<&str> = events.iter().filter_map(|e| e.text.as_deref()).collect();
    assert_eq!(texts[0], "first save");
    assert!(
        texts[1].starts_with("10 changes: changed object 153 (Metadata); changed document"),
        "{texts:?}"
    );
    assert_eq!(
        texts[2],
        "2 changes: field Name value changed from nothing to Jane Doe; field Phone value \
         changed from nothing to 5551234"
    );
    assert!(events[1].time.is_some(), "{events:?}");
    assert_eq!(out.document.properties.facts.revisions, Some(3));
}

#[test]
fn many_changes_are_cut_after_twenty() {
    // A save that changes 25 objects.
    let mut bytes = pdf(&[Page::text(show("F1", 12.0, 72.0, 700.0, "Hi."))], "");
    let objects: Vec<(u32, Vec<u8>)> = (0..25u32)
        .map(|k| (100 + k, format!("<< /K {k} >>").into_bytes()))
        .collect();
    append_update(&mut bytes, &objects, "/Size 125 /Root 1 0 R");
    let out = read(&bytes);
    let last = out.document.facets.timeline.last().expect("events");
    let t = last.text.as_deref().unwrap_or_default();
    assert!(t.starts_with("25 changes: added object 100"), "{t}");
    assert!(t.ends_with(", and 5 more"), "{t}");
    assert_eq!(t.matches("; ").count(), 19, "{t}");
}

#[test]
fn issues_become_losses() {
    // An outline whose first item points back at itself: pdfparts says so, the read is partial.
    let page = Page::text(show("F1", 12.0, 72.0, 700.0, "Hi."));
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = b.stream("", page.content.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    let outlines = b.reserve();
    let item = b.reserve();
    b.set(
        item,
        format!(
            "<< /Title (Loop) /Parent {outlines} 0 R /Next {item} 0 R /Dest [{kid} 0 R /Fit] >>"
        ),
    );
    b.set(
        outlines,
        format!("<< /Type /Outlines /First {item} 0 R /Last {item} 0 R >>"),
    );
    b.set(
        catalog,
        format!("<< /Type /Catalog /Pages {tree} 0 R /Outlines {outlines} 0 R >>"),
    );
    let out = read(&b.finish(catalog));
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.location == "outline")
        .expect("an outline loss");
    assert_eq!(loss.why, LossReason::Damaged);
    assert_eq!(loss.severity, Severity::Warning);
    assert_eq!(out.completeness, Completeness::Partial);
    // The item read before the loop is kept.
    let Block::Section { blocks, .. } = &out.document.body[0] else {
        panic!()
    };
    let mut items = Vec::new();
    list_items(&blocks[0], 1, &mut items);
    assert_eq!(items, [(1, "Loop, page 1".to_string())]);
    assert!(matches!(
        &out.document.body[1],
        Block::Section {
            kind: SectionKind::Page,
            ..
        }
    ));
}

#[test]
fn xmp_dates_are_checked() {
    let xmp = |create: &str, modify: &str| {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreateDate="{create}" xmp:ModifyDate="{modify}"/></rdf:RDF></x:xmpmeta>"#
        )
    };
    // Not dates: both are said and left out, and the Info date that does not parse keeps its
    // own loss.
    let out = read(&with_metadata(
        "<< /CreationDate (garbage) >>",
        &xmp("yesterday&#10;title: injected", "2026-02-31T10:00:00Z"),
    ));
    let p = &out.document.properties;
    assert_eq!((&p.created, &p.modified), (&None, &None));
    let said: Vec<(&str, LossReason)> = out
        .document
        .losses
        .iter()
        .map(|l| (l.what.as_str(), l.why))
        .collect();
    assert!(
        said.iter().any(|(w, r)| *r == LossReason::Damaged
            && w.starts_with("the XMP xmp:CreateDate is not a date: \"yesterday\\ntitle")),
        "{said:?}"
    );
    assert!(
        said.iter().any(|(w, r)| *r == LossReason::Damaged
            && w.contains("xmp:ModifyDate is not a date: \"2026-02-31T10:00:00Z\"")),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|(w, _)| w.contains("the creation date of the document information")),
        "{said:?}"
    );
    // Real dates in the forms XMP allows are used, with their offset.
    let out = read(&with_metadata(
        "<< >>",
        &xmp("2024-02-29T23:59:30.25+05:30", "2026-01"),
    ));
    let p = &out.document.properties;
    assert_eq!(
        p.created,
        Some(Value::DateTime {
            value: "2024-02-29T23:59:30.25+05:30".into()
        })
    );
    assert_eq!(
        p.modified,
        Some(Value::DateTime {
            value: "2026-01".into()
        })
    );
}

#[test]
fn an_attachment_shows_the_size_it_unpacks_to() {
    // The file states 3 bytes; the stream unpacks to 5,000.
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
    let file = b.stream(
        "/Type /EmbeddedFile /Params << /Size 3 >>",
        &vec![b'x'; 5000],
    );
    let spec = b.add(format!(
        "<< /Type /Filespec /F (x.txt) /EF << /F {file} 0 R >> >>"
    ));
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R \
             /Names << /EmbeddedFiles << /Names [(x.txt) {spec} 0 R] >> >> >>"
        ),
    );
    let out = read(&b.finish(catalog));
    let e = &out.document.facets.embedded[0];
    assert_eq!(e.detail.as_deref(), Some("3 bytes stated, 5000 unpacked"));
}

#[test]
fn one_keyword_too_long_is_said_in_the_singular() {
    // At 8 MiB the lists may take 64 KiB; the one keyword is 100 KB.
    let info = format!("<< /Keywords ({}) /Title (T) >>", "k".repeat(100_000));
    let bytes = with_metadata(&info, "");
    let limits = Limits {
        max_memory_bytes: 8 << 20,
        ..Limits::default()
    };
    let out = read_opts(&bytes, &ReadOptions::default(), limits);
    assert!(out.document.properties.keywords.is_empty());
    assert_eq!(out.document.properties.title.as_deref(), Some("T"));
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.what.contains("keyword"))
        .unwrap_or_else(|| panic!("{:?}", out.document.losses));
    assert!(
        loss.what
            .starts_with("1 keyword is not kept, from keyword 1 on: "),
        "{loss:?}"
    );
    // The reason names only what the reader can see.
    assert!(
        loss.what.contains(
            "the keywords and authors take more than the 65536 bytes left of their room, half \
             of what the metadata, outline and links may keep (a 64th of the memory limit)"
        ),
        "{loss:?}"
    );
}

#[test]
fn a_title_behind_a_reference_loop_is_a_damaged_loss() {
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
    let (x, y) = (b.reserve(), b.reserve());
    b.set(x, format!("{y} 0 R"));
    b.set(y, format!("{x} 0 R"));
    let outlines = b.reserve();
    let item = b.add(format!(
        "<< /Title {x} 0 R /Parent {outlines} 0 R /Dest [{kid} 0 R /Fit] >>"
    ));
    b.set(
        outlines,
        format!("<< /Type /Outlines /First {item} 0 R /Last {item} 0 R /Count 1 >>"),
    );
    b.set(
        catalog,
        format!("<< /Type /Catalog /Pages {tree} 0 R /Outlines {outlines} 0 R >>"),
    );
    let out = read(&b.finish(catalog));
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.location == "outline" && l.why == LossReason::Damaged)
        .unwrap_or_else(|| panic!("{:?}", out.document.losses));
    assert!(
        loss.what
            .starts_with("1 text could not be read, the first in outline item 1: "),
        "{loss:?}"
    );
    assert_eq!(out.completeness, Completeness::Partial);
}
