// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

mod common;

use std::path::PathBuf;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use pdfparts::{IssueKind, Limits, Pdf};

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

fn text(s: &str) -> Object {
    Object::string_literal(s)
}

/// A document with `n` pages. `build` gets the page ids and returns extra catalog entries.
fn document(n: usize, build: impl FnOnce(&mut Document, &[ObjectId]) -> Dictionary) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let pages: Vec<ObjectId> = (0..n)
        .map(|_| {
            doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            })
        })
        .collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => pages.iter().map(|p| Object::Reference(*p)).collect::<Vec<_>>(),
            "Count" => n as i64,
        }),
    );
    let extra = build(&mut doc, &pages);
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
    catalog.extend(&extra);
    let catalog = doc.add_object(catalog);
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// Catalog entries for an /EmbeddedFiles name tree with the given (key, file spec) pairs.
fn attachments(specs: Vec<(&str, Object)>) -> Dictionary {
    let mut names = Vec::new();
    for (k, v) in specs {
        names.push(text(k));
        names.push(v);
    }
    dictionary! { "Names" => dictionary! { "EmbeddedFiles" => dictionary! { "Names" => names } } }
}

#[test]
fn embedded_file_listed_and_bytes_guarded() {
    let pdf = corpus("pypdf-attachment.pdf");
    let files = pdf.embedded_files();
    assert_eq!(files.len(), 1);
    let f = &files[0];
    assert_eq!(f.name, "image.png");
    assert_eq!(f.description, None);
    assert_eq!(f.mime, None);
    // No /Params /Size: the size is measured.
    assert_eq!(f.size, Some(6669));
    let bytes = pdf.embedded_file_bytes(0).unwrap();
    assert_eq!(bytes.len(), 6669);
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    let missing = pdf.embedded_file_bytes(1).unwrap_err();
    assert_eq!(missing.kind, IssueKind::Damaged);
}

#[test]
fn embedded_file_details() {
    let bytes = document(1, |doc, _| {
        let mut file = Stream::new(
            dictionary! {
                "Type" => "EmbeddedFile",
                "Subtype" => "text/csv",
                "Params" => dictionary! {
                    "Size" => 11,
                    "CreationDate" => text("D:20240102030405Z"),
                    "ModDate" => text("D:20240203040506+01'00'"),
                },
            },
            b"a,b\n1,2\n3,4".to_vec(),
        );
        file.compress().unwrap();
        let file = doc.add_object(file);
        attachments(vec![(
            "key.csv",
            Object::Dictionary(dictionary! {
                "Type" => "Filespec",
                "F" => text("old.csv"),
                "UF" => Object::String(
                    b"\xFE\xFF\x00d\x00a\x00t\x00a\x00.\x00c\x00s\x00v".to_vec(),
                    lopdf::StringFormat::Literal,
                ),
                "Desc" => text("The numbers"),
                "EF" => dictionary! { "F" => file },
            }),
        )])
    });
    let pdf = open_bytes(&bytes);
    let files = pdf.embedded_files();
    assert_eq!(files.len(), 1);
    let f = &files[0];
    assert_eq!(f.name, "data.csv");
    assert_eq!(f.description.as_deref(), Some("The numbers"));
    assert_eq!(f.mime.as_deref(), Some("text/csv"));
    assert_eq!(f.size, Some(11));
    assert_eq!(
        f.created.as_ref().and_then(|d| d.iso.as_deref()),
        Some("2024-01-02T03:04:05Z")
    );
    assert_eq!(
        f.modified.as_ref().and_then(|d| d.iso.as_deref()),
        Some("2024-02-03T04:05:06+01:00")
    );
    assert_eq!(pdf.embedded_file_bytes(0).unwrap(), b"a,b\n1,2\n3,4");
}

#[test]
fn embedded_file_bomb_is_refused() {
    let bytes = document(1, |doc, _| {
        let bomb = doc.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile", "Filter" => "FlateDecode" },
            common::zlib_fill(0, 8 << 20),
        ));
        attachments(vec![(
            "bomb.bin",
            Object::Dictionary(
                dictionary! { "Type" => "Filespec", "EF" => dictionary! { "F" => bomb } },
            ),
        )])
    });
    let limits = Limits {
        max_stream_bytes: 1 << 20,
        ..Limits::default()
    };
    let pdf = Pdf::open(&bytes, None, &limits, &never).unwrap();
    let files = pdf.embedded_files();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "bomb.bin");
    assert_eq!(files[0].size, None);
    let err = pdf.embedded_file_bytes(0).unwrap_err();
    assert_eq!(err.kind, IssueKind::Limit);
    assert_eq!(err.location, "attachment 1");
}

#[test]
fn layers_with_utf16_names_and_visibility() {
    let layers = corpus("pypdf-ocg-layers.pdf").layers();
    let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "North Arrow",
            "Scale Bar",
            "Map Frame",
            "Map",
            "Light Gray Base",
            "Data Source Attribution"
        ]
    );
    assert!(layers.iter().all(|l| l.visible));

    let ocg = |doc: &mut Document, name: &str| {
        doc.add_object(dictionary! { "Type" => "OCG", "Name" => text(name) })
    };
    let bytes = document(1, |doc, _| {
        let (a, b, c) = (ocg(doc, "Roads"), ocg(doc, "Rivers"), ocg(doc, "Labels"));
        dictionary! { "OCProperties" => dictionary! {
            "OCGs" => vec![a.into(), b.into(), c.into(), a.into()],
            "D" => dictionary! { "OFF" => vec![b.into()] },
        } }
    });
    let layers = open_bytes(&bytes).layers();
    let seen: Vec<(&str, bool)> = layers
        .iter()
        .map(|l| (l.name.as_str(), l.visible))
        .collect();
    assert_eq!(seen, [("Roads", true), ("Rivers", false), ("Labels", true)]);

    let bytes = document(1, |doc, _| {
        let (a, b) = (ocg(doc, "Roads"), ocg(doc, "Rivers"));
        dictionary! { "OCProperties" => dictionary! {
            "OCGs" => vec![a.into(), b.into()],
            "D" => dictionary! { "BaseState" => "OFF", "ON" => vec![b.into()] },
        } }
    });
    let layers = open_bytes(&bytes).layers();
    let seen: Vec<(&str, bool)> = layers
        .iter()
        .map(|l| (l.name.as_str(), l.visible))
        .collect();
    assert_eq!(seen, [("Roads", false), ("Rivers", true)]);
}

#[test]
fn article_threads_with_titles() {
    let threads = corpus("pdfbox-poems-article-threads.pdf").threads();
    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0].title.as_deref(), Some("Erlkönig"));
    assert_eq!(threads[0].beads, 7);
    assert_eq!(threads[0].pages, [1]);
    assert_eq!(threads[1].title.as_deref(), Some("Moulière"));
    assert_eq!(threads[1].beads, 4);
    assert_eq!(threads[1].pages, [1, 2]);
}

#[test]
fn broken_bead_chains_are_damaged() {
    let bytes = document(2, |doc, pages| {
        // Thread one: bead 3 points back at bead 2, not at the first one.
        let ids: Vec<ObjectId> = (0..3).map(|_| doc.new_object_id()).collect();
        let next = [ids[1], ids[2], ids[1]];
        for (i, id) in ids.iter().enumerate() {
            doc.objects.insert(
                *id,
                Object::Dictionary(dictionary! { "N" => next[i], "P" => pages[i % 2] }),
            );
        }
        let one = doc.add_object(
            dictionary! { "F" => ids[0], "I" => dictionary! { "Title" => text("Loop") } },
        );
        // Thread two: the second bead has no /N.
        let last = doc.add_object(dictionary! { "P" => pages[1] });
        let first = doc.add_object(dictionary! { "N" => last, "P" => pages[0] });
        let two = doc.add_object(dictionary! { "F" => first });
        dictionary! { "Threads" => vec![one.into(), two.into()] }
    });
    let pdf = open_bytes(&bytes);
    let threads = pdf.threads();
    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0].title.as_deref(), Some("Loop"));
    assert_eq!(threads[0].beads, 3);
    assert_eq!(threads[0].pages, [1, 2]);
    assert_eq!(threads[1].title, None);
    assert_eq!(threads[1].beads, 2);
    assert_eq!(threads[1].pages, [1, 2]);
    let damaged = pdf
        .issues()
        .into_iter()
        .filter(|i| i.kind == IssueKind::Damaged && i.location == "article threads")
        .count();
    assert_eq!(damaged, 2, "{:?}", pdf.issues());
}

#[test]
fn files_without_extras_have_none() {
    let pdf = corpus("pypdf-annotated.pdf");
    assert!(pdf.embedded_files().is_empty());
    assert!(pdf.layers().is_empty());
    assert!(pdf.threads().is_empty());
}

#[test]
fn direct_bead_link_has_its_own_message() {
    let bytes = document(1, |doc, pages| {
        let first = doc.add_object(dictionary! {
            "P" => pages[0],
            "N" => dictionary! { "P" => pages[0] },
        });
        let thread = doc.add_object(dictionary! { "F" => first });
        dictionary! { "Threads" => vec![thread.into()] }
    });
    let pdf = open_bytes(&bytes);
    let threads = pdf.threads();
    assert_eq!(threads[0].beads, 1);
    let issues = pdf.issues();
    assert!(
        issues
            .iter()
            .any(|i| i.location == "article threads" && i.what.contains("not an indirect")),
        "{issues:?}"
    );
    assert!(!issues.iter().any(|i| i.what.contains("loops back")));
}
