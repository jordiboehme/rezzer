// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! XLSB and ODS workbooks, and encrypted workbooks.

mod common;

use std::time::{Duration, Instant};

use common::*;
use rezzer_model::{Severity, StatusColor};

// ---------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------

fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in entries {
        w.start_file(*name, opts).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn read_as(bytes: Vec<u8>, name: &str, format: FormatId, limits: Limits) -> ReadOutcome {
    SheetReader.read(
        &source(bytes, name, format),
        &ReadOptions::default(),
        &Budget::new(limits),
    )
}

/// One BIFF12 record: type (one or two bytes) and length (7 bits per byte).
fn rec(typ: u16, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if typ < 0x80 {
        out.push(typ as u8);
    } else {
        out.push((typ & 0x7F) as u8 | 0x80);
        out.push((typ >> 7) as u8);
    }
    let mut len = payload.len();
    loop {
        let b = (len & 0x7F) as u8;
        len >>= 7;
        if len == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
    out.extend_from_slice(payload);
    out
}

fn wide(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut out = (units.len() as u32).to_le_bytes().to_vec();
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

/// `workbook.bin` with the sheets (name, relationship id) and defined names (name, rgce).
fn workbook_bin(sheets: &[(&str, &str)], names: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = rec(0x0083, &[]); // BrtBeginBook
    out.extend(rec(0x0099, &[0; 4])); // BrtWbProp
    out.extend(rec(0x008F, &[])); // BrtBeginBundleShs
    for (i, (name, rid)) in sheets.iter().enumerate() {
        let mut p = 0u32.to_le_bytes().to_vec();
        p.extend((i as u32 + 1).to_le_bytes());
        p.extend(wide(rid));
        p.extend(wide(name));
        out.extend(rec(0x009C, &p));
    }
    out.extend(rec(0x0090, &[])); // BrtEndBundleShs
    // BrtExternSheet: one reference entry per sheet, for 3D references.
    let mut xti = (sheets.len() as u32).to_le_bytes().to_vec();
    for i in 0..sheets.len() as i32 {
        xti.extend(0u32.to_le_bytes());
        xti.extend(i.to_le_bytes());
        xti.extend(i.to_le_bytes());
    }
    out.extend(rec(0x016A, &xti));
    for (name, rgce) in names {
        let mut p = vec![0u8; 9];
        p.extend(wide(name));
        p.extend((rgce.len() as u32).to_le_bytes());
        p.extend(rgce);
        p.extend(0u32.to_le_bytes());
        out.extend(rec(0x0027, &p));
    }
    out.extend(rec(0x009D, &[0; 30])); // BrtCalcProp
    out.extend(rec(0x0084, &[])); // BrtEndBook
    out
}

/// A cell record of a sheet part.
enum C<'a> {
    Num(u32, f64),
    Text(u32, &'a str),
    /// Column, cached number, rgce.
    Formula(u32, f64, Vec<u8>),
    /// Column, error code, rgce.
    FormulaErr(u32, u8, Vec<u8>),
}

fn sheet_bin(rows: &[(u32, Vec<C>)]) -> Vec<u8> {
    let mut out = rec(0x0081, &[]); // BrtBeginSheet
    out.extend(rec(0x0094, &[0; 16])); // BrtWsDim
    out.extend(rec(0x0091, &[])); // BrtBeginSheetData
    for (row, cells) in rows {
        let mut hdr = row.to_le_bytes().to_vec();
        hdr.extend([0u8; 13]);
        out.extend(rec(0x0000, &hdr));
        for c in cells {
            match c {
                C::Num(col, v) => {
                    let mut p = col.to_le_bytes().to_vec();
                    p.extend(0u32.to_le_bytes());
                    p.extend(v.to_le_bytes());
                    out.extend(rec(0x0005, &p));
                }
                C::Text(col, s) => {
                    let mut p = col.to_le_bytes().to_vec();
                    p.extend(0u32.to_le_bytes());
                    p.extend(wide(s));
                    out.extend(rec(0x0006, &p));
                }
                C::Formula(col, v, rgce) => {
                    let mut p = col.to_le_bytes().to_vec();
                    p.extend(0u32.to_le_bytes());
                    p.extend(v.to_le_bytes());
                    p.extend(0u16.to_le_bytes());
                    p.extend((rgce.len() as u32).to_le_bytes());
                    p.extend(rgce);
                    p.extend(0u32.to_le_bytes());
                    out.extend(rec(0x0009, &p));
                }
                C::FormulaErr(col, err, rgce) => {
                    let mut p = col.to_le_bytes().to_vec();
                    p.extend(0u32.to_le_bytes());
                    p.push(*err);
                    p.extend(0u16.to_le_bytes());
                    p.extend((rgce.len() as u32).to_le_bytes());
                    p.extend(rgce);
                    p.extend(0u32.to_le_bytes());
                    out.extend(rec(0x000B, &p));
                }
            }
        }
    }
    out.extend(rec(0x0092, &[])); // BrtEndSheetData
    out.extend(rec(0x0082, &[])); // BrtEndSheet
    out
}

/// `A1` style reference token, relative row and column.
fn ptg_ref(row: u32, col: u16) -> Vec<u8> {
    let mut p = vec![0x24];
    p.extend(row.to_le_bytes());
    p.extend((col | 0xC000).to_le_bytes());
    p
}

fn ptg_int(v: u16) -> Vec<u8> {
    let mut p = vec![0x1E];
    p.extend(v.to_le_bytes());
    p
}

/// `A1*2`.
fn times_two(row: u32, col: u16) -> Vec<u8> {
    let mut p = ptg_ref(row, col);
    p.extend(ptg_int(2));
    p.push(0x05);
    p
}

/// A 3D reference to a cell of the sheet with this index, written absolute.
fn ptg_ref3d(sheet: u16, row: u32, col: u16) -> Vec<u8> {
    let mut p = vec![0x3a];
    p.extend(sheet.to_le_bytes());
    p.extend(row.to_le_bytes());
    p.extend(col.to_le_bytes());
    p
}

/// A constant wrapped in `depth` nested PtgMemFunc tokens.
fn nested(depth: usize) -> Vec<u8> {
    let mut inner = ptg_int(1);
    for _ in 0..depth {
        let mut p = vec![0x29];
        p.extend((inner.len() as u16).to_le_bytes());
        p.extend(inner);
        inner = p;
    }
    inner
}

const BIN_SHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const BIN_DOC: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

fn rels(list: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s = format!("<?xml version=\"1.0\"?><Relationships xmlns=\"{PKG}\">");
    for (id, kind, target) in list {
        s.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

/// An XLSB package: the sheets (name, sheet part bytes) with relationship ids rId1, rId2 ...
fn xlsb(sheets: &[(&str, Vec<u8>)], names: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let ids: Vec<String> = (1..=sheets.len()).map(|i| format!("rId{i}")).collect();
    let list: Vec<(&str, &str)> = sheets
        .iter()
        .zip(&ids)
        .map(|((n, _), id)| (*n, id.as_str()))
        .collect();
    let targets: Vec<String> = (1..=sheets.len())
        .map(|i| format!("worksheets/sheet{i}.bin"))
        .collect();
    let wb_rels: Vec<(&str, &str, &str)> = ids
        .iter()
        .zip(&targets)
        .map(|(id, t)| (id.as_str(), BIN_SHEET, t.as_str()))
        .collect();
    let mut entries = vec![
        ("_rels/.rels", rels(&[("rId1", BIN_DOC, "xl/workbook.bin")])),
        ("xl/workbook.bin", workbook_bin(&list, names)),
        ("xl/_rels/workbook.bin.rels", rels(&wb_rels)),
    ];
    let parts: Vec<String> = targets.iter().map(|t| format!("xl/{t}")).collect();
    for (part, (_, bytes)) in parts.iter().zip(sheets) {
        entries.push((part.as_str(), bytes.clone()));
    }
    zip_of(&entries)
}

fn read_xlsb(bytes: Vec<u8>) -> ReadOutcome {
    read_as(bytes, "book.xlsb", FormatId::Xlsb, Limits::default())
}

const ODS_MIME: &str = "application/vnd.oasis.opendocument.spreadsheet";
const MANIFEST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.spreadsheet\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>";

/// content.xml with the standard namespaces around `body` (the inside of office:spreadsheet).
fn content(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" office:version=\"1.2\"><office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>"
    )
}

fn ods_with(mime: &str, manifest: &str, content: &str) -> Vec<u8> {
    zip_of(&[
        ("mimetype", mime.as_bytes().to_vec()),
        ("META-INF/manifest.xml", manifest.as_bytes().to_vec()),
        ("content.xml", content.as_bytes().to_vec()),
    ])
}

fn ods(content: &str) -> Vec<u8> {
    ods_with(ODS_MIME, MANIFEST, content)
}

fn read_ods(bytes: Vec<u8>) -> ReadOutcome {
    read_as(bytes, "book.ods", FormatId::Ods, Limits::default())
}

fn whole_file_loss(out: &ReadOutcome, why: LossReason, text: &str) -> bool {
    out.document
        .losses
        .iter()
        .any(|l| l.why == why && l.location == "document" && l.what.contains(text))
}

fn status_of(blocks: &[Block]) -> Option<(String, StatusColor)> {
    blocks.iter().find_map(|b| match b {
        Block::Paragraph { inlines, .. } => inlines.iter().find_map(|i| match i {
            Inline::Status { text, color } => Some((text.clone(), *color)),
            _ => None,
        }),
        _ => None,
    })
}

/// An OLE compound file with the streams of a password-protected office file.
fn encrypted_package() -> Vec<u8> {
    let mut comp = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    for name in ["EncryptionInfo", "EncryptedPackage"] {
        let mut s = comp.create_stream(format!("/{name}")).unwrap();
        s.write_all(&[7u8; 600]).unwrap();
    }
    comp.flush().unwrap();
    comp.into_inner().into_inner()
}

// ---------------------------------------------------------------------------------------
// XLSB
// ---------------------------------------------------------------------------------------

#[test]
fn xlsb_values_formulas_and_vba() {
    let out = corpus("testEXCEL_various.xlsb");
    assert_ne!(
        out.completeness,
        Completeness::Failed,
        "{:#?}",
        out.document.losses
    );
    let doc = &out.document;
    section(doc, "mySheet1");
    assert_eq!(cell_text(doc, "mySheet1!A1").as_deref(), Some("String"));
    assert_eq!(cell_text(doc, "mySheet1!B2").as_deref(), Some("13"));
    assert_eq!(
        cell_text(doc, "mySheet1!B10").as_deref(),
        Some("2017-03-09")
    );

    let f = formula(doc, "mySheet1!B13");
    assert_eq!(f.expression, "=C13*2");
    assert_eq!(f.dialect, "excel");
    assert_eq!(refs(f), ["mySheet1!C13"]);
    assert_eq!(f.cached, Some(Value::number(4.0, None)));
    // An error result is a stored result, not a missing one.
    let err = formula(doc, "mySheet1!B14");
    assert_eq!(err.expression, "=COS(NA)");
    assert_eq!(
        err.cached,
        Some(Value::Text {
            value: "#NAME?".into()
        })
    );
    assert!(
        !err.note
            .as_deref()
            .unwrap_or("")
            .contains("not calculated in file"),
        "{:?}",
        err.note
    );

    let macros: Vec<&str> = doc
        .facets
        .embedded
        .iter()
        .filter(|e| e.kind == "macro")
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(macros, ["Module1", "Sheet1", "ThisWorkbook"]);
    assert!(
        doc.facets
            .embedded
            .iter()
            .all(|e| e.kind != "macro" || e.detail.as_deref() == Some("VBA module, never run"))
    );
    // Document properties come from the XML parts XLSB still has.
    assert!(!doc.properties.authors.is_empty(), "{:?}", doc.properties);
    assert!(formula(doc, "name:NA").kind == Some(FormulaKind::Name));
}

#[test]
fn xlsb_lists_what_is_not_read() {
    let out = corpus("testEXCEL_various.xlsb");
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.location == "document" && l.what.contains("not read for XLSB"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(loss.severity, Severity::Info);
    for part in [
        "validation",
        "conditional formats",
        "notes and threads",
        "names with scope",
        "calc settings",
        "data tables",
        "number formats beyond dates",
    ] {
        assert!(loss.what.contains(part), "{part}: {}", loss.what);
    }
}

#[test]
fn xlsb_fixture_reads_values_formulas_and_error_results() {
    let sheet = sheet_bin(&[
        (0, vec![C::Num(0, 21.0), C::Text(1, "label")]),
        (
            1,
            vec![
                C::Formula(0, 42.0, times_two(0, 0)),
                C::FormulaErr(1, 0x07, {
                    let mut p = ptg_int(1);
                    p.extend(ptg_int(0));
                    p.push(0x06);
                    p
                }),
            ],
        ),
    ]);
    let out = read_xlsb(xlsb(&[("Data", sheet)], &[]));
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
    let doc = &out.document;
    assert_eq!(cell_text(doc, "Data!A1").as_deref(), Some("21"));
    assert_eq!(cell_text(doc, "Data!B1").as_deref(), Some("label"));
    let f = formula(doc, "Data!A2");
    assert_eq!(f.expression, "=A1*2");
    assert_eq!(f.cached, Some(Value::number(42.0, None)));
    let e = formula(doc, "Data!B2");
    assert_eq!(e.expression, "=1/0");
    assert_eq!(
        e.cached,
        Some(Value::Text {
            value: "#DIV/0!".into()
        })
    );
    assert_eq!(cell_text(doc, "Data!B2").as_deref(), Some("#DIV/0!"));
    // Every formula has a stored result: no stale-results warning.
    assert!(
        !doc.losses.iter().any(|l| l.what.contains("stored results")),
        "{:#?}",
        doc.losses
    );
}

#[test]
fn xlsb_deeply_nested_formula_is_not_parsed() {
    let sheet = sheet_bin(&[(0, vec![C::Num(0, 5.0), C::Formula(1, 1.0, nested(5_000))])]);
    let out = read_xlsb(xlsb(&[("Deep", sheet)], &[]));
    assert_eq!(out.completeness, Completeness::Partial);
    let doc = &out.document;
    assert_eq!(cell_text(doc, "Deep!A1").as_deref(), Some("5"));
    assert!(
        has_loss(
            doc,
            LossReason::Limit,
            "Deep",
            "formulas of this sheet not read"
        ),
        "{:#?}",
        doc.losses
    );
    assert!(doc.facets.formulas.is_empty());
}

#[test]
fn unread_formula_cells_are_never_constants() {
    // Deep!B1 holds a formula Rezzer does not hand to calamine; Other!A1 reads Deep!B1.
    let deep = sheet_bin(&[(0, vec![C::Num(0, 5.0), C::Formula(1, 1.0, nested(5_000))])]);
    let mut reader = ptg_ref3d(0, 0, 1);
    reader.extend(ptg_int(2));
    reader.push(0x05);
    let other = sheet_bin(&[(0, vec![C::Formula(0, 2.0, reader)])]);
    let out = read_xlsb(xlsb(&[("Deep", deep), ("Other", other)], &[]));
    let doc = &out.document;
    let f = formula(doc, "Other!A1");
    assert_eq!(refs(f), ["Deep!B1"]);
    assert!(
        !doc.facets
            .parameters
            .iter()
            .any(|p| p.anchor.0 == "Deep!B1"),
        "{:#?}",
        doc.facets.parameters
    );
    assert_eq!(cell_text(doc, "Deep!B1").as_deref(), Some("1"));
}

#[test]
fn xlsb_sheet_names_in_references_are_quoted() {
    let data = sheet_bin(&[(0, vec![C::Num(0, 5.0)])]);
    let mut rgce = ptg_ref3d(0, 0, 0);
    rgce.extend(ptg_int(2));
    rgce.push(0x05);
    let calc = sheet_bin(&[(0, vec![C::Formula(0, 10.0, rgce.clone())])]);
    let out = read_xlsb(xlsb(
        &[("My Sheet", data.clone()), ("Calc", calc.clone())],
        &[],
    ));
    let f = formula(&out.document, "Calc!A1");
    assert_eq!(f.expression, "='My Sheet'!$A$1*2");
    assert_eq!(refs(f), ["'My Sheet'!A1"]);
    assert!(f.note.is_none(), "{:?}", f.note);

    // With sheets `Q1-2024` and `2024`, calamine's `Q1-2024!$A$1` reads two ways.
    let out = read_xlsb(xlsb(
        &[("Q1-2024", data), ("Calc", calc), ("2024", sheet_bin(&[]))],
        &[],
    ));
    let doc = &out.document;
    let f = formula(doc, "Calc!A1");
    assert!(f.references.is_empty());
    assert!(
        f.note.as_deref().is_some_and(|n| n.contains("Q1-2024")),
        "{:?}",
        f.note
    );
    assert!(has_loss(
        doc,
        LossReason::Unsupported,
        "Calc!A1",
        "not analyzed"
    ));
}

#[test]
fn xlsb_deeply_nested_name_is_refused() {
    let sheet = sheet_bin(&[(0, vec![C::Num(0, 5.0)])]);
    let out = read_xlsb(xlsb(&[("S", sheet)], &[("deep", nested(5_000))]));
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        whole_file_loss(&out, LossReason::Limit, "the whole file"),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn xlsb_shared_formula_cells_say_their_text_is_unknown() {
    // PtgExp: the cell is part of a shared or array formula stored elsewhere.
    let exp = {
        let mut p = vec![0x01];
        p.extend(0u32.to_le_bytes());
        p
    };
    let sheet = sheet_bin(&[(0, vec![C::Num(0, 1.0), C::Formula(1, 2.0, exp)])]);
    let out = read_xlsb(xlsb(&[("S", sheet)], &[]));
    let doc = &out.document;
    assert_eq!(cell_text(doc, "S!B1").as_deref(), Some("2"));
    assert!(
        has_loss(doc, LossReason::Unsupported, "S", "shared or array formula"),
        "{:#?}",
        doc.losses
    );
    assert!(
        doc.losses
            .iter()
            .any(|l| l.why == LossReason::Uncertain && l.what.contains("logic is incomplete")),
        "{:#?}",
        doc.losses
    );
}

#[test]
fn xlsb_package_structure_is_checked() {
    let sheet = sheet_bin(&[(0, vec![C::Num(0, 1.0)])]);
    // A sheet relationship with a dotted target.
    let dotted = zip_of(&[
        ("_rels/.rels", rels(&[("rId1", BIN_DOC, "xl/workbook.bin")])),
        ("xl/workbook.bin", workbook_bin(&[("S", "rId1")], &[])),
        (
            "xl/_rels/workbook.bin.rels",
            rels(&[("rId1", BIN_SHEET, "./worksheets/sheet1.bin")]),
        ),
        ("xl/worksheets/sheet1.bin", sheet.clone()),
    ]);
    let out = read_xlsb(dotted);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(whole_file_loss(
        &out,
        LossReason::Damaged,
        "not a plain path"
    ));

    // A workbook part that is not xl/workbook.bin, which calamine always reads.
    let moved = zip_of(&[
        (
            "_rels/.rels",
            rels(&[("rId1", BIN_DOC, "book/workbook.bin")]),
        ),
        ("book/workbook.bin", workbook_bin(&[("S", "rId1")], &[])),
        ("xl/workbook.bin", workbook_bin(&[("Other", "rId1")], &[])),
        (
            "xl/_rels/workbook.bin.rels",
            rels(&[("rId1", BIN_SHEET, "worksheets/sheet1.bin")]),
        ),
        ("xl/worksheets/sheet1.bin", sheet.clone()),
    ]);
    let out = read_xlsb(moved);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(whole_file_loss(
        &out,
        LossReason::Damaged,
        "xl/workbook.bin"
    ));

    // A sheet name calamine decodes differently (a leading byte order mark).
    let bom = xlsb(&[("\u{feff}S", sheet.clone())], &[]);
    let out = read_xlsb(bom);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        whole_file_loss(&out, LossReason::Damaged, "sheet names"),
        "{:#?}",
        out.document.losses
    );

    // A relationship id that names no relationship.
    let missing = zip_of(&[
        ("_rels/.rels", rels(&[("rId1", BIN_DOC, "xl/workbook.bin")])),
        ("xl/workbook.bin", workbook_bin(&[("S", "rId9")], &[])),
        (
            "xl/_rels/workbook.bin.rels",
            rels(&[("rId1", BIN_SHEET, "worksheets/sheet1.bin")]),
        ),
        ("xl/worksheets/sheet1.bin", sheet.clone()),
    ]);
    let out = read_xlsb(missing);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(whole_file_loss(&out, LossReason::Damaged, "rId9"));

    // The canonical package reads.
    let out = read_xlsb(xlsb(&[("S", sheet)], &[]));
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn xlsb_record_longer_than_its_part_is_refused() {
    // A cell record claiming 256 MB inside the sheet data.
    let mut sheet = rec(0x0081, &[]);
    sheet.extend(rec(0x0094, &[0; 16]));
    sheet.extend(rec(0x0091, &[]));
    sheet.extend(rec(0x0000, &[0; 17]));
    sheet.extend([0x05, 0xFF, 0xFF, 0xFF, 0x7F, 1, 2, 3]);
    let out = read_xlsb(xlsb(&[("S", sheet)], &[]));
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(
        paragraphs(section(&out.document, "S")).contains(&"This sheet was not read.".to_string())
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged && l.what.contains("record")),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn xlsb_unknown_workbook_record_is_skipped() {
    // An unknown record in workbook.bin once made calamine panic (calamine issue 666).
    let out = corpus("malformed/issue_666_panic.xlsb");
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
    assert_eq!(
        paragraphs(section(&out.document, "Sheet1")),
        ["The sheet is empty."]
    );
}

// ---------------------------------------------------------------------------------------
// ODS
// ---------------------------------------------------------------------------------------

#[test]
fn ods_formulas_shown_in_a1_with_original() {
    let out = corpus("formula-across-sheets.ods");
    assert_ne!(
        out.completeness,
        Completeness::Failed,
        "{:#?}",
        out.document.losses
    );
    let doc = &out.document;
    let f = formula(doc, "Calculation!B4");
    assert_eq!(f.expression, "=Data!A3");
    assert_eq!(f.original.as_deref(), Some("of:=[$Data.A3]"));
    assert_eq!(f.dialect, "openformula");
    assert_eq!(refs(f), ["Data!A3"]);
    let g = formula(doc, "Calculation!B5");
    assert_eq!(g.expression, "=(B4-$G$1)/$G$2");
    assert_eq!(g.original.as_deref(), Some("of:=([.B4]-[.$G$1])/[.$G$2]"));
    assert!(g.cached.is_some());
    // Names come with their scope from the ODF parts.
    let n = formula(doc, "name:L_nb");
    assert_eq!(n.kind, Some(FormulaKind::Name));
    assert_eq!(refs(n), ["Calculation!E7"]);
}

#[test]
fn ods_hidden_sheet_and_annotation() {
    let out = corpus("any_sheets.ods");
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
    let doc = &out.document;
    assert_eq!(status_of(section(doc, "Visible")), None);
    for name in ["Hidden", "VeryHidden"] {
        assert_eq!(
            status_of(section(doc, name)),
            Some(("hidden".to_string(), StatusColor::Grey)),
            "{name}"
        );
    }
    assert_eq!(cell_text(doc, "Visible!B3").as_deref(), Some("6"));

    let out = corpus("with-annotation.ods");
    let doc = &out.document;
    assert_eq!(cell_text(doc, "table1!A1").as_deref(), Some("cell a.1"));
    let a = doc
        .facets
        .annotations
        .iter()
        .find(|a| a.anchor.as_ref().is_some_and(|x| x.0 == "table1!A1"))
        .unwrap_or_else(|| panic!("{:#?}", doc.facets.annotations));
    assert_eq!(a.author.as_deref(), Some("Marc Schreiber"));
    assert_eq!(a.date.as_deref(), Some("2024-09-18T18:27:00"));
    assert_eq!(plain_text(&a.body), "This is annotation of a.1");
}

#[test]
fn ods_hidden_rows_validations_and_formats() {
    let body = "<table:content-validations><table:content-validation table:name=\"val1\" table:condition=\"of:cell-content-is-whole-number() and cell-content()&gt;0\"/></table:content-validations><table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"1\" table:content-validation-name=\"val1\"/></table:table-row><table:table-row table:visibility=\"collapse\"><table:table-cell office:value-type=\"float\" office:value=\"2\"/></table:table-row><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"3\" table:formula=\"of:=[.A1]+[.A2]\"/></table:table-row></table:table>";
    let out = read_ods(ods(&content(body)));
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        out.document.losses
    );
    let doc = &out.document;
    assert!(paragraphs(section(doc, "S")).contains(&"Hidden rows: 2.".to_string()));
    assert!(
        doc.facets
            .rules
            .iter()
            .any(|r| r.kind == "validation" && r.anchor.0 == "S!A1"),
        "{:#?}",
        doc.facets.rules
    );
    let f = formula(doc, "S!A3");
    assert_eq!(f.expression, "=A1+A2");
    assert_eq!(f.cached, Some(Value::number(3.0, None)));
    assert_eq!(f.role, Some(rezzer_model::Role::Result));
}

#[test]
fn ods_package_checks() {
    let plain = content(
        "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"1\"/></table:table-row></table:table>",
    );
    assert_eq!(read_ods(ods(&plain)).completeness, Completeness::Complete);

    // A mimetype that is not the spreadsheet type (calamine compares 46 bytes only).
    let out = read_ods(ods_with(&format!("{ODS_MIME}-template"), MANIFEST, &plain));
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(whole_file_loss(&out, LossReason::Damaged, "mimetype"));

    // Two sheets with one name: calamine keeps only one of them.
    let twice = content(
        "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"1\"/></table:table-row></table:table><table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"2\"/></table:table-row></table:table>",
    );
    let out = read_ods(ods(&twice));
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        whole_file_loss(&out, LossReason::Damaged, "sheet name"),
        "{:#?}",
        out.document.losses
    );

    // A table outside office:spreadsheet: calamine reads it, the ODF parts do not.
    let stray = format!(
        "<?xml version=\"1.0\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><table:table table:name=\"Ghost\"><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"9\"/></table:table-row></table:table><office:body><office:spreadsheet>{}</office:spreadsheet></office:body></office:document-content>",
        "<table:table table:name=\"S\"/>"
    );
    let out = read_ods(ods(&stray));
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(whole_file_loss(&out, LossReason::Damaged, "the whole file"));

    // Another prefix for the table namespace: the parts would read cells calamine skips.
    let prefixed = plain.replace(
        "xmlns:style=",
        "xmlns:t=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:style=",
    );
    let prefixed = prefixed.replace(
        "</table:table-row>",
        "<t:table-cell office:value-type=\"float\" office:value=\"7\"/></table:table-row>",
    );
    let out = read_ods(ods(&prefixed));
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        whole_file_loss(&out, LossReason::Damaged, "namespace"),
        "{:#?}",
        out.document.losses
    );

    // A namespace declared below the root.
    let nested = plain.replace(
        "<table:table table:name",
        "<table:table xmlns:x=\"urn:x\" table:name",
    );
    let out = read_ods(ods(&nested));
    assert_eq!(out.completeness, Completeness::Failed);

    // Two entries named content.xml in different case.
    let cased = zip_of(&[
        ("mimetype", ODS_MIME.as_bytes().to_vec()),
        ("META-INF/manifest.xml", MANIFEST.as_bytes().to_vec()),
        ("content.xml", plain.as_bytes().to_vec()),
        ("Content.xml", plain.as_bytes().to_vec()),
    ]);
    let out = read_ods(cased);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(whole_file_loss(
        &out,
        LossReason::Damaged,
        "differ only in case"
    ));
}

#[test]
fn ods_duplicate_entry_names_are_refused() {
    let plain = content(
        "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"1\"/></table:table-row></table:table>",
    );
    let other = content(
        "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"2\"/></table:table-row></table:table>",
    );
    let mut bytes = zip_of(&[
        ("mimetype", ODS_MIME.as_bytes().to_vec()),
        ("META-INF/manifest.xml", MANIFEST.as_bytes().to_vec()),
        ("content.xml", plain.into_bytes()),
        ("contenX.xml", other.into_bytes()),
    ]);
    // Rename the second entry in its local and central headers: two entries, one name.
    let mut patched = 0;
    for i in 0..bytes.len() - 7 {
        if &bytes[i..i + 7] == b"contenX" {
            bytes[i + 6] = b't';
            patched += 1;
        }
    }
    assert_eq!(patched, 2);
    let out = read_ods(bytes);
    assert_eq!(
        out.completeness,
        Completeness::Failed,
        "{:#?}",
        out.document.losses
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Damaged && l.location == "document"),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn ods_dense_area_over_the_memory_limit_is_refused() {
    // 4,000 rows, each 16,384 repeated value cells: calamine would hold 65 million cells.
    let row = "<table:table-row><table:table-cell table:number-columns-repeated=\"16384\" office:value-type=\"float\" office:value=\"1\"/></table:table-row>";
    let body = format!(
        "<table:table table:name=\"S\">{}</table:table>",
        row.repeat(4_000)
    );
    // The repeated rows compress far past the ratio limit, which is not the subject here.
    let limits = Limits {
        max_ratio: 1_000_000,
        ..Limits::default()
    };
    let started = Instant::now();
    let out = read_as(ods(&content(&body)), "book.ods", FormatId::Ods, limits);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        whole_file_loss(&out, LossReason::Limit, "memory"),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn ods_many_empty_rows_count_against_the_memory_limit() {
    // calamine keeps two numbers for every row element, even an empty one.
    let body = format!(
        "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"1\"/></table:table-row>{}</table:table>",
        "<table:table-row/>".repeat(1_000_000)
    );
    let limits = Limits {
        max_ratio: 1_000_000,
        max_memory_bytes: 8 << 20,
        ..Limits::default()
    };
    let out = read_as(ods(&content(&body)), "book.ods", FormatId::Ods, limits);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        whole_file_loss(&out, LossReason::Limit, "memory"),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn ods_space_bomb_is_refused() {
    let body = "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"string\"><text:p>a<text:s text:c=\"4000000000\"/>b</text:p></table:table-cell></table:table-row></table:table>";
    let started = Instant::now();
    let out = read_ods(ods(&content(body)));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(whole_file_loss(&out, LossReason::Limit, "memory"));
}

#[test]
fn ods_malformed_corpus_file_is_refused_fast() {
    let started = Instant::now();
    let out = corpus("malformed/issue_594_dos.ods");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        whole_file_loss(&out, LossReason::Limit, "memory"),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn ods_lists_what_is_not_read() {
    let out = corpus("any_sheets.ods");
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.location == "document" && l.what.contains("not read for ODS"))
        .unwrap_or_else(|| panic!("{:#?}", out.document.losses));
    assert_eq!(loss.severity, Severity::Info);
    assert!(loss.what.contains("conditional formats"));
}

// ---------------------------------------------------------------------------------------
// Encrypted workbooks
// ---------------------------------------------------------------------------------------

#[test]
fn encrypted_workbook_says_encrypted() {
    for format in [
        FormatId::Xlsx,
        FormatId::Xlsm,
        FormatId::Xlsb,
        FormatId::Ods,
    ] {
        let out = read_as(encrypted_package(), "secret", format, Limits::default());
        assert_eq!(out.completeness, Completeness::Failed, "{format:?}");
        assert!(
            whole_file_loss(&out, LossReason::Encrypted, "password"),
            "{format:?}: {:#?}",
            out.document.losses
        );
    }
    // An ODS whose manifest lists encryption data.
    let manifest = MANIFEST.replace(
        "manifest:media-type=\"text/xml\"/>",
        "manifest:media-type=\"text/xml\"><manifest:encryption-data manifest:checksum-type=\"SHA1\"/></manifest:file-entry>",
    );
    let out = read_ods(ods_with(ODS_MIME, &manifest, "not xml, encrypted bytes"));
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        whole_file_loss(&out, LossReason::Encrypted, "password"),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn reader_claims_all_four_formats() {
    assert_eq!(SheetReader.id(), "sheet");
    assert_eq!(
        SheetReader.formats(),
        [
            FormatId::Xlsx,
            FormatId::Xlsm,
            FormatId::Xlsb,
            FormatId::Ods
        ]
    );
}
