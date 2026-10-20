// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};
use pdfparts::{IssueKind, Limits, Pdf, StructTree};

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

/// The index of the element whose children hold `child`.
fn parent_of(tree: &StructTree, child: usize) -> Option<usize> {
    tree.elems.iter().position(|e| e.children.contains(&child))
}

/// A one-page tagged document. `build` gets the page id and the struct tree root id, adds
/// the elements and returns the root's /K; `root` gets extra entries such as /RoleMap.
fn tagged(build: impl FnOnce(&mut Document, ObjectId, ObjectId) -> (Object, Dictionary)) -> Pdf {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let root_id = doc.new_object_id();
    let (k, extra) = build(&mut doc, page_id, root_id);
    let mut root = dictionary! { "Type" => "StructTreeRoot", "K" => k };
    root.extend(&extra);
    doc.objects.insert(root_id, Object::Dictionary(root));
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "StructParents" => 0,
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "StructTreeRoot" => root_id,
        "MarkInfo" => dictionary! { "Marked" => true },
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    open_bytes(&out)
}

#[test]
fn struct_roles_and_mcids() {
    let pdf = corpus("pdfjs-structure-simple.pdf");
    let tree = pdf.struct_tree().unwrap();
    assert_eq!(tree.roots.len(), 1);
    let doc = &tree.elems[tree.roots[0]];
    assert_eq!(doc.role, "Document");
    assert_eq!(doc.lang.as_deref(), Some("en-US"));
    assert_eq!(doc.id.as_deref(), Some("node00000010"));
    let roles: Vec<&str> = doc
        .children
        .iter()
        .map(|&c| tree.elems[c].role.as_str())
        .collect();
    assert_eq!(roles, ["H1", "P", "H2", "P"]);
    // The marked content sits on NonStruct children; the ParentTree names them.
    for (mcid, &heading) in doc.children.iter().enumerate() {
        let leaf = pdf.role_of(1, mcid as u32).unwrap();
        assert_eq!(tree.elems[leaf].role, "NonStruct");
        assert_eq!(tree.elems[leaf].mcids, [mcid as u32]);
        assert_eq!(tree.elems[leaf].page, Some(1));
        assert_eq!(parent_of(&tree, leaf), Some(heading));
        assert_eq!(tree.elems[leaf].parent, Some(heading));
    }
    // Walking up from the first marked content reaches the H1, then the root.
    let mut up = pdf.role_of(1, 0);
    let mut roles = Vec::new();
    while let Some(i) = up {
        roles.push(tree.elems[i].role.as_str());
        up = tree.elems[i].parent;
    }
    assert_eq!(roles, ["NonStruct", "H1", "Document"]);
    assert_eq!(doc.parent, None);
    assert_eq!(pdf.role_of(1, 4), None);
    assert_eq!(pdf.role_of(2, 0), None);
}

#[test]
fn role_map_applied() {
    let pdf = tagged(|doc, page, root| {
        let leaf = doc.add_object(dictionary! {
            "Type" => "StructElem", "S" => "Fancy", "P" => root, "Pg" => page, "K" => 0,
        });
        let footnote = doc.add_object(dictionary! {
            "Type" => "StructElem", "S" => "Loop", "P" => root, "Pg" => page,
            "K" => vec![Object::Dictionary(dictionary! { "MCID" => 1 })],
        });
        let parents = doc.add_object(Object::Array(vec![leaf.into(), footnote.into()]));
        (
            Object::Array(vec![leaf.into(), footnote.into()]),
            dictionary! {
                // A chain of two hops, and a loop that ends where it started.
                "RoleMap" => dictionary! { "Fancy" => "Heading1", "Heading1" => "H1", "Loop" => "Loop2", "Loop2" => "Loop" },
                "ParentTree" => dictionary! { "Nums" => vec![0.into(), parents.into()] },
            },
        )
    });
    let tree = pdf.struct_tree().unwrap();
    let roles: Vec<&str> = tree
        .roots
        .iter()
        .map(|&r| tree.elems[r].role.as_str())
        .collect();
    assert_eq!(roles[0], "H1");
    // A role map loop keeps the last role reached before the loop, and says so.
    assert_eq!(roles[1], "Loop2");
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.what.contains("Loop")),
        "{:?}",
        pdf.issues()
    );
    assert_eq!(pdf.role_of(1, 0), Some(tree.roots[0]));
    assert_eq!(pdf.role_of(1, 1), Some(tree.roots[1]));
    assert_eq!(tree.elems[tree.roots[1]].mcids, [1]);
}

#[test]
fn alt_and_table_headers() {
    let nist = corpus("nist-sp800-53r5.pdf").struct_tree().unwrap();
    assert_eq!(nist.elems.iter().filter(|e| e.alt.is_some()).count(), 36);
    assert!(nist.elems.iter().any(|e| {
        e.alt
            .as_deref()
            .is_some_and(|a| a.starts_with("Table C-20 provides a summary"))
    }));

    let irs = corpus("irs-f1040-2025.pdf").struct_tree().unwrap();
    let th = irs
        .elems
        .iter()
        .find(|e| {
            e.role == "TH"
                && e.id.as_deref()
                    == Some(
                        "topmostSubform[0].Page1[0].Table_Dependents[0].Header[0].Dependent1[0]",
                    )
        })
        .unwrap();
    assert_eq!(th.col_span, 1);
    assert_eq!(th.row_span, 1);
    let td = irs
        .elems
        .iter()
        .find(|e| e.role == "TD" && !e.headers.is_empty())
        .unwrap();
    assert!(td.headers[0].starts_with("topmostSubform[0].Page1[0].Table_Dependents[0].Header[0]."));
    // Each header names a TH by its /ID.
    for h in &td.headers {
        assert!(
            irs.elems
                .iter()
                .any(|e| e.role == "TH" && e.id.as_deref() == Some(h)),
            "{h}"
        );
    }
}

#[test]
fn spans_scope_and_actual_text() {
    let pdf = tagged(|doc, page, root| {
        let th = doc.add_object(dictionary! {
            "Type" => "StructElem", "S" => "TH", "P" => root, "Pg" => page, "K" => 0,
            "A" => vec![
                Object::Dictionary(dictionary! { "O" => "Layout", "Placement" => "Block" }),
                Object::Integer(0),
                Object::Dictionary(dictionary! { "O" => "Table", "Scope" => "Column", "ColSpan" => 2, "RowSpan" => 3 }),
            ],
            "ActualText" => Object::string_literal("Total"),
            "Alt" => Object::string_literal("Sum"),
        });
        (Object::Reference(th), Dictionary::new())
    });
    let tree = pdf.struct_tree().unwrap();
    let th = &tree.elems[tree.roots[0]];
    assert_eq!(th.scope.as_deref(), Some("Column"));
    assert_eq!((th.col_span, th.row_span), (2, 3));
    assert_eq!(th.actual_text.as_deref(), Some("Total"));
    assert_eq!(th.alt.as_deref(), Some("Sum"));
    assert!(th.headers.is_empty());
}

#[test]
fn struct_tree_cycle_is_damaged() {
    let pdf = tagged(|doc, page, root| {
        let a = doc.new_object_id();
        let b = doc.add_object(dictionary! {
            "Type" => "StructElem", "S" => "P", "P" => a, "Pg" => page, "K" => vec![Object::Integer(0), a.into()],
        });
        doc.objects.insert(
            a,
            Object::Dictionary(dictionary! {
                "Type" => "StructElem", "S" => "Sect", "P" => root, "K" => vec![b.into()],
            }),
        );
        (Object::Reference(a), Dictionary::new())
    });
    let tree = pdf.struct_tree().unwrap();
    assert_eq!(tree.elems.len(), 2);
    let sect = &tree.elems[tree.roots[0]];
    assert_eq!(sect.role, "Sect");
    let p = &tree.elems[sect.children[0]];
    assert_eq!(p.role, "P");
    assert_eq!(p.mcids, [0]);
    // The page is inherited down the tree, and the loop back to Sect is not followed.
    assert_eq!(p.page, Some(1));
    assert!(p.children.is_empty());
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.location == "structure tree"),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn untagged_file_has_no_tree() {
    assert!(corpus("pypdf-annotated.pdf").struct_tree().is_none());
    assert_eq!(corpus("pypdf-annotated.pdf").role_of(1, 0), None);
}
