// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use lopdf::{Document, Object, dictionary};
use pdfparts::{IssueKind, Limits, LinkTarget, Pdf};

fn never() -> bool {
    false
}

fn open_bytes(bytes: &[u8]) -> Pdf {
    Pdf::open(bytes, None, &Limits::default(), &never).unwrap()
}

fn corpus(name: &str) -> Pdf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf")
        .join(name);
    open_bytes(&std::fs::read(path).unwrap())
}

#[test]
fn page_labels_roman_and_arabic() {
    let pdf = corpus("qpdf-page-labels-and-outlines.pdf");
    let pages = pdf.pages(&never);
    assert_eq!(pages.len(), 30);
    let labels: Vec<Option<&str>> = pages.iter().take(14).map(|p| p.label.as_deref()).collect();
    let expected = [
        "", "", "i", "ii", "iii", "iv", "v", "", "", "vi", "vii", "", "2", "3",
    ];
    assert_eq!(labels, expected.map(Some).to_vec());
    // Later ranges: decimal from 16059, roman from 50 and 54.
    assert_eq!(pages[22].label.as_deref(), Some("16059"));
    assert_eq!(pages[23].label.as_deref(), Some("l"));
    assert_eq!(pages[29].label.as_deref(), Some("liv"));
    assert_eq!(pages[0].number, 1);
    assert_eq!(pages[29].number, 30);
    assert!(pages[0].has_fonts);
    assert!(pages[0].width > 0.0 && pages[0].height > 0.0);
}

#[test]
fn pages_without_labels_have_none() {
    let pdf = corpus("pypdf-outline.pdf");
    assert!(pdf.pages(&never).iter().all(|p| p.label.is_none()));
}

/// A document with `n` pages and the given /PageLabels number tree.
fn labelled(n: usize, labels: Object) -> Pdf {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let kids: Vec<Object> = (0..n)
        .map(|_| {
            doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            })
            .into()
        })
        .collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => n as i64 }),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "PageLabels" => labels,
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    open_bytes(&out)
}

#[test]
fn page_label_styles() {
    let pdf = labelled(
        8,
        Object::Dictionary(dictionary! {
            "Nums" => vec![
                1.into(), Object::Dictionary(dictionary! { "S" => "A" }),
                3.into(), Object::Dictionary(dictionary! { "S" => "a", "St" => 26 }),
                5.into(), Object::Dictionary(dictionary! { "S" => "R", "St" => 1999, "P" => Object::string_literal("App-") }),
                6.into(), Object::Dictionary(dictionary! {}),
            ],
        }),
    );
    let labels: Vec<Option<String>> = pdf.pages(&never).into_iter().map(|p| p.label).collect();
    assert_eq!(
        labels,
        vec![
            None,
            Some("A".into()),
            Some("B".into()),
            Some("z".into()),
            Some("aa".into()),
            Some("App-MCMXCIX".into()),
            Some(String::new()),
            Some(String::new()),
        ]
    );
}

#[test]
fn outline_levels_and_pages() {
    let pdf = corpus("qpdf-page-labels-and-outlines.pdf");
    let outline = pdf.outline();
    assert_eq!(outline.len(), 11, "{outline:?}");
    let levels: Vec<u8> = outline.iter().map(|o| o.level).collect();
    assert_eq!(levels, vec![1, 2, 3, 4, 4, 3, 4, 2, 3, 3, 1]);
    // Each title names its 0-based target page after "->".
    for item in &outline {
        let n: u32 = item
            .title
            .split("-> ")
            .nth(1)
            .and_then(|r| r.split(':').next())
            .and_then(|n| n.parse().ok())
            .unwrap();
        assert_eq!(item.page, Some(n + 1), "{item:?}");
        assert_eq!(item.uri, None);
    }
}

#[test]
fn outline_cycle_is_damaged() {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let outlines = doc.new_object_id();
    let a = doc.new_object_id();
    let b = doc.new_object_id();
    doc.objects.insert(
        a,
        Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("A"), "Parent" => outlines, "Next" => b,
            "Dest" => vec![page_id.into(), "Fit".into()],
        }),
    );
    doc.objects.insert(
        b,
        Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("B"), "Parent" => outlines, "Next" => a, "First" => a,
        }),
    );
    doc.objects.insert(
        outlines,
        Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => a }),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id, "Outlines" => outlines,
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let pdf = open_bytes(&out);
    let outline = pdf.outline();
    assert_eq!(outline.len(), 2, "{outline:?}");
    assert_eq!(outline[0].page, Some(1));
    // A second call gives the same result and does not repeat the issue.
    assert_eq!(pdf.outline(), outline);
    let cycles = pdf
        .issues()
        .into_iter()
        .filter(|i| i.kind == IssueKind::Damaged && i.what.contains("cycle"))
        .count();
    assert_eq!(cycles, 1, "{:?}", pdf.issues());
}

#[test]
fn named_dests_and_links() {
    let pdf = corpus("pypdf-outline.pdf");
    let dests = pdf.named_destinations();
    assert!(
        dests.contains(&("section.1".to_string(), Some(2))),
        "{dests:?}"
    );
    assert!(dests.iter().all(|(_, page)| page.is_some()), "{dests:?}");
    let links = pdf.links();
    assert_eq!(links.len(), 9, "{links:?}");
    assert!(links.iter().all(|l| l.page == 1));
    assert_eq!(
        links[0].target,
        LinkTarget::Named("section.1".into(), Some(2))
    );
    assert!(links[0].rect[2] > links[0].rect[0]);
    assert_eq!(pdf.outline().len(), 9);
}

#[test]
fn uri_link_listed_not_followed() {
    let pdf = corpus("pdfjs-annotation-link-text-popup.pdf");
    let links = pdf.links();
    assert_eq!(links.len(), 1, "{links:?}");
    match &links[0].target {
        LinkTarget::Uri(uri) => assert!(uri.starts_with("http"), "{uri}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn link_actions_are_listed() {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let link = |action: lopdf::Dictionary| {
        Object::Dictionary(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => vec![10.into(), 20.into(), 30.into(), 40.into()],
            "A" => action,
        })
    };
    let annots = vec![
        link(
            dictionary! { "S" => "GoToR", "F" => Object::string_literal("other.pdf"), "D" => vec![2.into(), "Fit".into()] },
        ),
        link(
            dictionary! { "S" => "Launch", "F" => dictionary! { "Type" => "Filespec", "UF" => Object::string_literal("run.exe") } },
        ),
        link(dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.alert(1)") }),
        link(dictionary! { "S" => "GoTo", "D" => vec![page_id.into(), "Fit".into()] }),
        Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "Rect" => vec![1.into(), 2.into(), 3.into(), 4.into()],
            "Dest" => vec![0.into(), "Fit".into()],
        }),
    ];
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Annots" => annots,
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let pdf = open_bytes(&out);
    let targets: Vec<LinkTarget> = pdf.links().into_iter().map(|l| l.target).collect();
    assert_eq!(
        targets,
        vec![
            LinkTarget::Remote {
                file: "other.pdf".into(),
                page: Some(3)
            },
            LinkTarget::Launch("run.exe".into()),
            LinkTarget::Other("JavaScript".into()),
            LinkTarget::Page(1),
            LinkTarget::Page(1),
        ]
    );
    assert_eq!(pdf.links()[0].rect, [10.0, 20.0, 30.0, 40.0]);
}

#[test]
fn page_images_are_listed() {
    let pdf = corpus("scan-truman-letter-nara.pdf");
    let pages = pdf.pages(&never);
    assert!(pages.iter().any(|p| !p.images.is_empty()), "{pages:?}");
    let image = &pages.iter().find(|p| !p.images.is_empty()).unwrap().images[0];
    assert!(image.width > 100 && image.height > 100, "{image:?}");
    assert!(matches!(image.components, 1 | 3 | 4), "{image:?}");
}

#[test]
fn long_label_prefix_is_cut() {
    let prefix = "p".repeat(300);
    let pdf = labelled(
        3,
        Object::Dictionary(dictionary! {
            "Nums" => vec![0.into(), Object::Dictionary(dictionary! { "S" => "D", "P" => Object::string_literal(prefix) })],
        }),
    );
    let pages = pdf.pages(&never);
    let label = pages[2].label.as_deref().unwrap();
    assert_eq!(label, format!("{}3", "p".repeat(256)));
    let limits: Vec<_> = pdf
        .issues()
        .into_iter()
        .filter(|i| i.kind == IssueKind::Limit && i.what.contains("prefix"))
        .collect();
    assert_eq!(limits.len(), 1, "{:?}", pdf.issues());
}

#[test]
fn page_issues_are_grouped() {
    // 50 pages without a media box give one issue that lists them.
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let kids: Vec<Object> = (0..50)
        .map(|_| {
            doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id })
                .into()
        })
        .collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 50 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let pdf = open_bytes(&out);
    assert_eq!(pdf.pages(&never).len(), 50);
    let boxes: Vec<_> = pdf
        .issues()
        .into_iter()
        .filter(|i| i.what.contains("media box"))
        .collect();
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    assert!(boxes[0].what.contains("50 pages"), "{boxes:?}");
}

/// A one-page document with the page's `/Annots` and the catalog's extra entries, both from
/// `build`, which gets the page id.
fn one_page(
    build: impl FnOnce(&mut Document, lopdf::ObjectId) -> (Object, lopdf::Dictionary),
) -> Pdf {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let (annots, extra) = build(&mut doc, page_id);
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Annots" => annots,
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
    catalog.extend(&extra);
    let catalog = doc.add_object(catalog);
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    open_bytes(&out)
}

/// `n` objects that each only refer to the next, the last to `to`. Returns the first.
fn hops(doc: &mut Document, n: usize, to: lopdf::ObjectId) -> lopdf::ObjectId {
    let mut at = to;
    for _ in 0..n {
        at = doc.add_object(Object::Reference(at));
    }
    at
}

fn uri_link(uri: Object) -> lopdf::Dictionary {
    dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![10.into(), 20.into(), 30.into(), 40.into()],
        "A" => dictionary! { "S" => "URI", "URI" => uri },
    }
}

#[test]
fn a_link_reached_through_hops_is_listed_once() {
    let pdf = one_page(|doc, _| {
        let link = doc.add_object(uri_link(Object::string_literal("http://x/")));
        let (a, b) = (hops(doc, 1, link), hops(doc, 2, link));
        (
            Object::Array(vec![a.into(), b.into()]),
            lopdf::Dictionary::new(),
        )
    });
    assert_eq!(pdf.links().len(), 1, "{:?}", pdf.links());
}

#[test]
fn an_outline_title_behind_a_reference_loop_is_damaged() {
    let pdf = one_page(|doc, page| {
        let (x, y) = (doc.new_object_id(), doc.new_object_id());
        doc.objects.insert(x, Object::Reference(y));
        doc.objects.insert(y, Object::Reference(x));
        let outlines = doc.new_object_id();
        let b = doc.add_object(dictionary! {
            "Title" => x, "Parent" => outlines, "Dest" => vec![page.into(), "Fit".into()],
        });
        let a = doc.add_object(dictionary! {
            "Title" => Object::string_literal("A"), "Parent" => outlines, "Next" => b,
            "Dest" => vec![page.into(), "Fit".into()],
        });
        doc.objects.insert(
            outlines,
            Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => a }),
        );
        (
            Object::Array(Vec::new()),
            dictionary! { "Outlines" => outlines },
        )
    });
    let outline = pdf.outline();
    assert_eq!(outline.len(), 2);
    assert_eq!(&*outline[1].title, "");
    let issue = pdf
        .issues()
        .into_iter()
        .find(|i| i.location == "outline" && i.kind == IssueKind::Damaged)
        .unwrap_or_else(|| panic!("{:?}", pdf.issues()));
    assert!(
        issue
            .what
            .starts_with("1 text could not be read, the first in outline item 2: "),
        "{issue:?}"
    );
}

#[test]
fn a_uri_behind_a_chain_past_the_bound_is_damaged() {
    let pdf = one_page(|doc, _| {
        let s = doc.add_object(Object::string_literal("http://x/"));
        // 130 hops: past lopdf's bound of 128.
        let far = hops(doc, 130, s);
        let near = hops(doc, 3, s);
        let links = vec![
            doc.add_object(uri_link(far.into())).into(),
            doc.add_object(uri_link(near.into())).into(),
        ];
        (Object::Array(links), lopdf::Dictionary::new())
    });
    let targets: Vec<LinkTarget> = pdf.links().into_iter().map(|l| l.target).collect();
    assert_eq!(
        targets,
        [
            LinkTarget::Other("URI".into()),
            LinkTarget::Uri("http://x/".into())
        ]
    );
    assert!(
        pdf.issues().iter().any(|i| i.kind == IssueKind::Damaged
            && i.what
                .starts_with("1 text could not be read, the first in link 1 on page 1: ")),
        "{:?}",
        pdf.issues()
    );
}
