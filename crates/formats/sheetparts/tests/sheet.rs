// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::{Cursor, Write};

use sheetparts::ooxml::{AttrKind, SheetParts, WorkbookParts, read_sheet, read_workbook};
use sheetparts::{CellPos, IssueKind, Limits, RowSpan};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};
use zipguard::Archive;

fn corpus(path: &str) -> Vec<u8> {
    let full = format!(
        "{}/../../../tests/corpus/office/{path}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&full).unwrap_or_else(|e| panic!("corpus file {full}: {e}"))
}

/// Reads the sheet with this name from a package.
fn sheet_of(bytes: &[u8], name: &str, limits: &Limits) -> SheetParts {
    let mut archive = Archive::open(bytes, zipguard::Limits::default()).unwrap();
    let wb: WorkbookParts = read_workbook(&mut archive, limits, &|| false);
    let sheet = wb
        .sheets
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no sheet {name}"))
        .clone();
    read_sheet(&mut archive, &sheet, &wb, limits, &|| false)
}

fn file_sheet(path: &str, name: &str) -> SheetParts {
    sheet_of(&corpus(path), name, &Limits::default())
}

fn first_sheet_name(path: &str) -> String {
    let bytes = corpus(path);
    let mut archive = Archive::open(&bytes, zipguard::Limits::default()).unwrap();
    read_workbook(&mut archive, &Limits::default(), &|| false).sheets[0]
        .name
        .clone()
}

fn first(path: &str) -> SheetParts {
    file_sheet(path, &first_sheet_name(path))
}

fn cell(a1: &str) -> CellPos {
    let col = u32::from(a1.as_bytes()[0] - b'A');
    CellPos {
        row: a1[1..].parse::<u32>().unwrap() - 1,
        col,
    }
}

#[test]
fn hidden_rows_and_cols() {
    let sheet = first("GroupTest.xlsx");
    assert!(sheet.issues.is_empty(), "{:?}", sheet.issues);
    // Rows 8 to 11 (1-based) are hidden: 0-based 7..=10, one merged span.
    assert!(sheet.hidden_rows.contains(&RowSpan { first: 7, last: 10 }));
    assert!(sheet.hidden_rows.iter().all(|s| s.first <= s.last));
    assert!(sheet.outline_rows.contains(&(0, 3)));
    assert!(sheet.outline_rows.contains(&(3, 1)));
    assert!(!sheet.outline_rows.iter().any(|&(r, _)| r == 4));

    let xml = |cols: &str, rows: &str| {
        format!(
            "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">{cols}<sheetData>{rows}</sheetData></worksheet>"
        )
    };
    let bytes = fixture(&xml(
        "<cols><col min=\"2\" max=\"3\" hidden=\"1\"/><col min=\"4\" max=\"4\" hidden=\"1\"/><col min=\"9\" max=\"9\" width=\"3\"/></cols>",
        "<row r=\"3\" hidden=\"true\"/><row r=\"4\" hidden=\"1\"/><row><c s=\"2\"/></row><c/>",
    ));
    let s = sheet_of(&bytes, "S", &Limits::default());
    assert_eq!(s.hidden_cols, vec![RowSpan { first: 1, last: 3 }]);
    assert_eq!(s.hidden_rows, vec![RowSpan { first: 2, last: 3 }]);
    // The row without r follows row 4; the cell without r is column A.
    assert_eq!(s.cell_styles, vec![(CellPos { row: 4, col: 0 }, 2)]);
}

fn fixture(sheet_xml: &str) -> Vec<u8> {
    fixture_with(sheet_xml, &[], &[])
}

/// A package with one sheet `S`, optional sheet relationships and extra parts.
fn fixture_with(sheet_xml: &str, sheet_rels: &[(&str, &str)], extra: &[(&str, &str)]) -> Vec<u8> {
    fixture_full(sheet_xml, "", &[], sheet_rels, extra)
}

fn fixture_full(
    sheet_xml: &str,
    workbook_inner: &str,
    workbook_rels: &[(&str, &str)],
    sheet_rels: &[(&str, &str)],
    extra: &[(&str, &str)],
) -> Vec<u8> {
    const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let mut put = |name: &str, data: &str| {
        let o = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file(name, o).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    };
    put(
        "_rels/.rels",
        &format!(
            "<Relationships xmlns=\"{PKG}\"><Relationship Id=\"r1\" Type=\"{REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
        ),
    );
    put(
        "xl/workbook.xml",
        &format!(
            "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"{REL}\"><sheets><sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\"/></sheets>{workbook_inner}</workbook>"
        ),
    );
    let mut wb_rels = format!(
        "<Relationships xmlns=\"{PKG}\"><Relationship Id=\"rId1\" Type=\"{REL}/worksheet\" Target=\"worksheets/sheet1.xml\"/>"
    );
    for (i, (kind, target)) in workbook_rels.iter().enumerate() {
        wb_rels.push_str(&format!(
            "<Relationship Id=\"w{i}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    wb_rels.push_str("</Relationships>");
    put("xl/_rels/workbook.xml.rels", &wb_rels);
    put("xl/worksheets/sheet1.xml", sheet_xml);
    if !sheet_rels.is_empty() {
        let mut r = format!("<Relationships xmlns=\"{PKG}\">");
        for (i, (kind, target)) in sheet_rels.iter().enumerate() {
            r.push_str(&format!(
                "<Relationship Id=\"x{i}\" Type=\"{kind}\" Target=\"{target}\"/>"
            ));
        }
        r.push_str("</Relationships>");
        put("xl/worksheets/_rels/sheet1.xml.rels", &r);
    }
    for (name, data) in extra {
        put(name, data);
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn data_table_attrs() {
    let sheet = first("one-variable.xlsx");
    let tables: Vec<_> = sheet
        .formula_attrs
        .iter()
        .filter(|a| matches!(a.kind, AttrKind::DataTable { .. }))
        .collect();
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].at, cell("B5"));
    assert_eq!(tables[0].range.as_deref(), Some("B5:B11"));
    assert_eq!(
        tables[0].kind,
        AttrKind::DataTable {
            r1: Some("A2".into()),
            r2: None,
            two_d: false,
            row_input: false
        }
    );
    assert_eq!(tables[0].text.as_deref(), Some(""));
    assert_eq!(tables[1].range.as_deref(), Some("E5:I5"));
    assert!(matches!(
        tables[1].kind,
        AttrKind::DataTable {
            row_input: true,
            ..
        }
    ));
}

#[test]
fn dynamic_array_and_empty_array_f() {
    let sheet = file_sheet(
        "DynamicArrays.xlsx",
        &first_sheet_name("DynamicArrays.xlsx"),
    );
    assert!(sheet.issues.is_empty(), "{:?}", sheet.issues);
    let at = |a1: &str| sheet.formula_attrs.iter().find(|f| f.at == cell(a1));
    let seq = at("A3").expect("A3");
    assert_eq!(seq.kind, AttrKind::DynamicArray);
    assert_eq!(seq.range.as_deref(), Some("A3:B12"));
    assert_eq!(seq.text.as_deref(), Some("_xlfn.SEQUENCE(10,2,5,2)"));
    // The other cells of the RANDARRAY range carry an empty f and are reported.
    let spilled = at("B19").expect("B19");
    assert_eq!(spilled.kind, AttrKind::Array);
    assert_eq!(spilled.text.as_deref(), Some(""));
    assert_eq!(spilled.range, None);
    assert!(at("C21").is_some());
    assert_eq!(at("A19").unwrap().kind, AttrKind::DynamicArray);

    // A legacy array formula (no cm) stays an array.
    let second = file_sheet("DynamicArrays.xlsx", "ArrayFormulas");
    let legacy = second
        .formula_attrs
        .iter()
        .find(|f| f.at == cell("A3"))
        .expect("legacy array");
    assert_eq!(legacy.kind, AttrKind::Array);
    assert_eq!(legacy.range.as_deref(), Some("A3:D3"));
}

#[test]
fn shared_formulas_are_reported() {
    let bytes = fixture(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\"><c r=\"A1\"><f t=\"shared\" ref=\"A1:A3\" si=\"4\">B1*2</f><v>2</v></c><c r=\"B1\"><f>1+1</f></c></row><row r=\"2\"><c r=\"A2\"><f t=\"shared\" si=\"4\"/></c></row></sheetData></worksheet>",
    );
    let s = sheet_of(&bytes, "S", &Limits::default());
    assert_eq!(s.formula_attrs.len(), 2);
    assert_eq!(s.formula_attrs[0].kind, AttrKind::Shared { index: 4 });
    assert_eq!(s.formula_attrs[0].range.as_deref(), Some("A1:A3"));
    assert_eq!(s.formula_attrs[0].text.as_deref(), Some("B1*2"));
    assert_eq!(s.formula_attrs[1].at, cell("A2"));
    assert_eq!(s.formula_attrs[1].text.as_deref(), Some(""));
    assert_eq!(s.formula_attrs[1].range, None);
}

#[test]
fn validation_list_and_messages() {
    let name = first_sheet_name("DataValidations-49244.xlsx");
    let sheet = file_sheet("DataValidations-49244.xlsx", &name);
    assert_eq!(sheet.validations.len(), 52);
    let ranged = sheet
        .validations
        .iter()
        .find(|v| v.kind == "list" && v.range == "C2")
        .expect("list on C2");
    assert_eq!(ranged.formula1.as_deref(), Some("$D$1:$F$1"));
    assert_eq!(ranged.list, None);
    assert_eq!(
        ranged.prompt.as_deref(),
        Some("Choose a country code from the list of options.")
    );
    assert_eq!(
        ranged.error.as_deref(),
        Some("The country code specified is not a valid option.")
    );
    let whole = sheet
        .validations
        .iter()
        .find(|v| v.range == "C6")
        .expect("C6");
    assert_eq!(whole.kind, "whole");
    assert_eq!(whole.operator.as_deref(), Some("greaterThan"));
    assert_eq!(whole.formula1.as_deref(), Some("E6"));
    let between = sheet.validations.iter().find(|v| v.range == "C14").unwrap();
    assert_eq!(between.operator, None);
    assert_eq!(between.formula2.as_deref(), Some("F14"));

    // A literal list is split; quotes inside are unescaped; a formula in quotes is no list.
    let bytes = fixture(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><dataValidations count=\"3\"><dataValidation type=\"list\" sqref=\"A1 B1\"><formula1>\"yes,no,say \"\"hi\"\"\"</formula1></dataValidation><dataValidation type=\"list\" sqref=\"A2\"><formula1>\"a\"&amp;\"b\"</formula1></dataValidation><dataValidation type=\"custom\" sqref=\"A3\"><formula1>A3&gt;1</formula1></dataValidation></dataValidations></worksheet>",
    );
    let s = sheet_of(&bytes, "S", &Limits::default());
    assert_eq!(s.validations[0].range, "A1 B1");
    assert_eq!(
        s.validations[0].list,
        Some(vec!["yes".into(), "no".into(), "say \"hi\"".into()])
    );
    assert_eq!(s.validations[1].list, None);
    assert_eq!(s.validations[2].formula1.as_deref(), Some("A3>1"));
}

#[test]
fn conditional_format_rules() {
    let sheet = first("WithConditionalFormatting.xlsx");
    assert!(sheet.issues.is_empty(), "{:?}", sheet.issues);
    let a = &sheet.cond_formats[0];
    assert_eq!(a.range, "A1:A8");
    assert_eq!(a.kind, "cellIs");
    assert_eq!(a.operator.as_deref(), Some("greaterThan"));
    assert_eq!(a.formulas, vec!["3".to_string()]);
    assert_eq!(a.priority, 5);
    let expr = sheet
        .cond_formats
        .iter()
        .find(|c| c.kind == "expression")
        .unwrap();
    assert_eq!(expr.range, "B9");
    assert_eq!(expr.formulas, vec!["$A$8>5".to_string()]);
    let between = sheet
        .cond_formats
        .iter()
        .find(|c| c.operator.as_deref() == Some("between"))
        .unwrap();
    assert_eq!(between.range, "B1:B7");
    assert_eq!(
        between.formulas,
        vec!["\"A\"".to_string(), "\"AAA\"".to_string()]
    );
}

#[test]
fn legacy_note_with_author() {
    let sheet = first("SimpleWithComments.xlsx");
    assert!(sheet.issues.is_empty(), "{:?}", sheet.issues);
    assert_eq!(sheet.notes.len(), 3);
    assert_eq!(sheet.notes[0].cell, "B1");
    assert_eq!(sheet.notes[0].author.as_deref(), Some("Yegor Kozlov"));
    assert_eq!(sheet.notes[0].text, "Yegor Kozlov:\nfirst cell");
    assert!(sheet.threads.is_empty());
}

#[test]
fn threaded_comment_with_reply_person_and_done() {
    let sheet = first("threadedComment.xlsx");
    assert!(sheet.issues.is_empty(), "{:?}", sheet.issues);
    assert_eq!(sheet.threads.len(), 2);
    let root = &sheet.threads[0];
    assert_eq!(root.cell, "A2");
    assert_eq!(root.text, "a comment on A2");
    assert_eq!(root.person.as_deref(), Some("Mike Kaganski"));
    assert_eq!(root.date.as_deref(), Some("2026-03-24T09:25:49.80"));
    assert_eq!(root.parent, None);
    assert!(!root.done);
    let reply = &sheet.threads[1];
    assert_eq!(reply.text, "A reply");
    assert_eq!(reply.parent.as_deref(), Some(root.id.as_str()));
    // The placeholder note of the thread is left out; the real note stays.
    assert_eq!(sheet.notes.len(), 1);
    assert_eq!(sheet.notes[0].cell, "A5");
    assert_eq!(sheet.notes[0].author.as_deref(), Some("Mike Kaganski"));

    let ns = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";
    let bytes = fixture_with(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"/>",
        &[(
            "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment",
            "../tc.xml",
        )],
        &[(
            "xl/tc.xml",
            &format!(
                "<ThreadedComments xmlns=\"{ns}\"><threadedComment ref=\"C3\" id=\"{{1}}\" personId=\"{{nobody}}\" done=\"1\"><text>fixed</text></threadedComment></ThreadedComments>"
            ),
        )],
    );
    let s = sheet_of(&bytes, "S", &Limits::default());
    assert_eq!(s.threads.len(), 1);
    assert!(s.threads[0].done);
    assert_eq!(s.threads[0].person, None);
    assert!(s.issues.iter().any(|i| i.what.contains("{nobody}")));
}

#[test]
fn tables_pivots_and_charts() {
    let multi = file_sheet(
        "table-multiple.xlsx",
        &first_sheet_name("table-multiple.xlsx"),
    );
    assert_eq!(multi.tables.len(), 1);
    assert_eq!(multi.tables[0].name, "Inventory");
    assert_eq!(multi.tables[0].range, "A1:C5");
    assert_eq!(multi.tables[0].columns, vec!["Item", "Type", "Quantity"]);
    assert_eq!(
        (multi.tables[0].header_rows, multi.tables[0].totals_rows),
        (1, 0)
    );

    let named = first("simple-table-named-range.xlsx");
    assert_eq!(named.tables[0].name, "Table1");
    assert_eq!(named.tables[0].range, "B3:D5");
    assert_eq!(named.tables[0].columns, vec!["a", "b", "c"]);

    let bytes = corpus("pivots.xlsx");
    let mut archive = Archive::open(&bytes, zipguard::Limits::default()).unwrap();
    let wb = read_workbook(&mut archive, &Limits::default(), &|| false);
    let mut pivots = Vec::new();
    for sheet in &wb.sheets {
        let parts = read_sheet(&mut archive, sheet, &wb, &Limits::default(), &|| false);
        assert!(parts.issues.is_empty(), "{:?}", parts.issues);
        pivots.extend(parts.pivots);
    }
    assert_eq!(pivots.len(), 5);
    assert_eq!(pivots[0].name, "PivotTable1");
    assert_eq!(pivots[0].location, "A3:E5");
    assert_eq!(pivots[0].source.as_deref(), Some("DataSheet!A1:J11"));
    assert!(
        pivots
            .iter()
            .any(|p| p.source.as_deref() == Some("DataSheet2!A1:J11"))
    );

    let bytes = corpus("WithChart.xlsx");
    let mut archive = Archive::open(&bytes, zipguard::Limits::default()).unwrap();
    let wb = read_workbook(&mut archive, &Limits::default(), &|| false);
    let charts: Vec<_> = wb
        .sheets
        .iter()
        .flat_map(|s| read_sheet(&mut archive, s, &wb, &Limits::default(), &|| false).charts)
        .collect();
    assert_eq!(charts.len(), 1);
    assert_eq!(charts[0].kind, "line");
    assert_eq!(charts[0].title, None);
    assert_eq!(
        charts[0].series,
        vec![
            (
                Some("1st Column".to_string()),
                "Sheet1!$A$1:$A$6".to_string()
            ),
            (
                Some("2nd Column".to_string()),
                "Sheet1!$B$1:$B$6".to_string()
            ),
        ]
    );
}

#[test]
fn chart_title_and_cell_named_series() {
    let drawing_rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
    let chart_rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
    let pkg = "http://schemas.openxmlformats.org/package/2006/relationships";
    let chart = "<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>Sales </a:t></a:r><a:r><a:t>2026</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:ser><c:tx><c:strRef><c:f>S!$A$1</c:f><c:strCache><c:pt idx=\"0\"><c:v>Cached</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:f>S!$B$2:$B$3</c:f></c:strRef></c:cat><c:val><c:numRef><c:f>S!$C$2:$C$3</c:f></c:numRef></c:val></c:ser></c:barChart><c:pieChart><c:ser><c:val><c:numRef><c:f>S!$D$2:$D$3</c:f></c:numRef></c:val></c:ser></c:pieChart><c:catAx><c:title><c:tx><c:rich><a:p><a:r><a:t>Axis</a:t></a:r></a:p></c:rich></c:tx></c:title></c:catAx></c:plotArea></c:chart></c:chartSpace>";
    let bytes = fixture_with(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"/>",
        &[(drawing_rel, "../d.xml")],
        &[
            ("xl/d.xml", "<xdr:wsDr xmlns:xdr=\"x\"/>"),
            (
                "xl/_rels/d.xml.rels",
                &format!(
                    "<Relationships xmlns=\"{pkg}\"><Relationship Id=\"c1\" Type=\"{chart_rel}\" Target=\"charts/c.xml\"/></Relationships>"
                ),
            ),
            ("xl/charts/c.xml", chart),
        ],
    );
    let s = sheet_of(&bytes, "S", &Limits::default());
    assert!(s.issues.is_empty(), "{:?}", s.issues);
    assert_eq!(s.charts.len(), 2);
    assert_eq!(s.charts[0].kind, "bar");
    assert_eq!(s.charts[0].title.as_deref(), Some("Sales 2026"));
    assert_eq!(
        s.charts[0].series,
        vec![(Some("S!$A$1".to_string()), "S!$C$2:$C$3".to_string())]
    );
    assert_eq!(s.charts[1].kind, "pie");
    assert_eq!(s.charts[1].series, vec![(None, "S!$D$2:$D$3".to_string())]);
}

#[test]
fn item_limit_stops_with_issue() {
    let bytes = corpus("DataValidations-49244.xlsx");
    let name = first_sheet_name("DataValidations-49244.xlsx");
    let limits = Limits {
        max_items: 200,
        ..Limits::default()
    };
    let mut archive = Archive::open(&bytes, zipguard::Limits::default()).unwrap();
    let wb = read_workbook(&mut archive, &Limits::default(), &|| false);
    let sheet = wb.sheets.iter().find(|s| s.name == name).unwrap();
    let parts = read_sheet(&mut archive, sheet, &wb, &limits, &|| false);
    assert!(parts.validations.len() < 52);
    assert!(
        parts.issues.iter().any(|i| i.kind == IssueKind::Limit),
        "{:?}",
        parts.issues
    );
    let stopped = read_sheet(&mut archive, sheet, &wb, &Limits::default(), &|| true);
    assert!(stopped.issues.iter().any(|i| i.kind == IssueKind::Limit));
    assert!(stopped.validations.is_empty());
}

#[test]
fn broken_sheets_become_issues() {
    // Not well formed, a DOCTYPE, a missing part and an escaping relationship target.
    let bad =
        fixture("<worksheet><sheetData><row r=\"1\"><c r=\"A1\"></row></sheetData></worksheet>");
    let s = sheet_of(&bad, "S", &Limits::default());
    assert!(s.issues.iter().any(|i| i.kind == IssueKind::Damaged));
    let doctype = fixture("<!DOCTYPE x [<!ENTITY a \"b\">]><worksheet/>");
    let s = sheet_of(&doctype, "S", &Limits::default());
    assert!(s.issues.iter().any(|i| i.kind == IssueKind::Unsupported));
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
    let missing = fixture_with(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"/>",
        &[(rel, "../../../../etc/passwd"), (rel, "../tables/none.xml")],
        &[],
    );
    let s = sheet_of(&missing, "S", &Limits::default());
    assert!(s.tables.is_empty());
    assert_eq!(s.issues.len(), 2);
    let bad_refs = fixture(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><cols><col min=\"0\" max=\"x\" hidden=\"1\"/></cols><sheetData><row r=\"1\"><c r=\"ZZZZ99\" s=\"3\"/><c r=\"A0\" s=\"3\"/></row></sheetData></worksheet>",
    );
    let s = sheet_of(&bad_refs, "S", &Limits::default());
    assert!(s.cell_styles.is_empty());
    assert!(s.issues.len() >= 2);
}

#[test]
fn every_corpus_sheet_reads_without_panic() {
    let dir = format!(
        "{}/../../../tests/corpus/office",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut files = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "xlsx" && ext != "xlsm" {
            continue;
        }
        files += 1;
        let bytes = std::fs::read(&path).unwrap();
        let mut archive = Archive::open(&bytes, zipguard::Limits::default()).unwrap();
        let wb = read_workbook(&mut archive, &Limits::default(), &|| false);
        for sheet in &wb.sheets {
            let parts = read_sheet(&mut archive, sheet, &wb, &Limits::default(), &|| false);
            assert!(
                parts.issues.is_empty(),
                "{}: {:?}",
                path.display(),
                parts.issues
            );
        }
    }
    assert!(files > 10);
}

#[test]
fn impossible_column_and_row_numbers_are_dropped_with_an_issue() {
    let bytes = fixture(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><cols><col min=\"20000\" max=\"20001\" hidden=\"1\"/><col min=\"5\" max=\"3\" hidden=\"1\"/><col min=\"16000\" max=\"99999\" hidden=\"1\"/></cols><sheetData><row r=\"1048577\" hidden=\"1\" outlineLevel=\"2\"/><row r=\"1048576\" hidden=\"1\"/><row r=\"0\" hidden=\"1\"/></sheetData></worksheet>",
    );
    let s = sheet_of(&bytes, "S", &Limits::default());
    assert_eq!(
        s.hidden_cols,
        vec![RowSpan {
            first: 15999,
            last: 16383
        }]
    );
    assert_eq!(
        s.hidden_rows,
        vec![RowSpan {
            first: 1_048_575,
            last: 1_048_575
        }]
    );
    assert!(s.outline_rows.is_empty());
    assert!(s.hidden_cols.iter().all(|c| c.first <= c.last));
    assert!(s.issues.iter().any(|i| i.what.contains("column")));
    assert!(s.issues.iter().any(|i| i.what.contains("row")));
}

#[test]
fn empty_author_keeps_later_authors_aligned() {
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
    let bytes = fixture_with(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"/>",
        &[(rel, "../c.xml")],
        &[(
            "xl/c.xml",
            "<comments xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><authors><author/><author>Ann</author><author></author></authors><commentList><comment ref=\"A1\" authorId=\"0\"><text><t>x</t></text></comment><comment ref=\"A2\" authorId=\"1\"><text><t>y</t></text></comment><comment ref=\"A3\" authorId=\"2\"><text><t>z</t></text></comment></commentList></comments>",
        )],
    );
    let s = sheet_of(&bytes, "S", &Limits::default());
    assert!(s.issues.is_empty(), "{:?}", s.issues);
    let authors: Vec<_> = s.notes.iter().map(|n| n.author.as_deref()).collect();
    assert_eq!(authors, vec![None, Some("Ann"), None]);
}

fn pivot_source(sheet_attrs: &str) -> sheetparts::ooxml::SheetParts {
    let rel = |t: &str| {
        format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{t}")
    };
    let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let bytes = fixture_full(
        &format!("<worksheet xmlns=\"{main}\"/>"),
        "<pivotCaches><pivotCache cacheId=\"7\" r:id=\"w0\"/></pivotCaches>",
        &[(&rel("pivotCacheDefinition"), "pc.xml")],
        &[(&rel("pivotTable"), "../pt.xml")],
        &[
            (
                "xl/pt.xml",
                &format!(
                    "<pivotTableDefinition xmlns=\"{main}\" name=\"P\" cacheId=\"7\"><location ref=\"A1:B2\"/></pivotTableDefinition>"
                ),
            ),
            (
                "xl/pc.xml",
                &format!(
                    "<pivotCacheDefinition xmlns=\"{main}\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><cacheSource type=\"worksheet\"><worksheetSource {sheet_attrs}/></cacheSource></pivotCacheDefinition>"
                ),
            ),
        ],
    );
    sheet_of(&bytes, "S", &Limits::default())
}

#[test]
fn pivot_source_sheet_names_are_quoted_like_formula_anchors() {
    for (name, expected) in [
        ("Data", "Data!A1:B2"),
        ("2024", "'2024'!A1:B2"),
        ("A1", "'A1'!A1:B2"),
        ("R1C1", "'R1C1'!A1:B2"),
        ("My Sheet", "'My Sheet'!A1:B2"),
        ("it's", "'it''s'!A1:B2"),
        ("a.b", "'a.b'!A1:B2"),
        ("_x", "_x!A1:B2"),
    ] {
        let s = pivot_source(&format!(
            "sheet=\"{}\" ref=\"A1:B2\"",
            name.replace('\'', "&apos;")
        ));
        assert_eq!(s.pivots[0].source.as_deref(), Some(expected), "{name}");
    }
}

#[test]
fn external_pivot_source_is_not_shown_as_local() {
    let s = pivot_source("r:id=\"rId1\" sheet=\"Other\" ref=\"A1:B2\"");
    assert_eq!(s.pivots[0].source, None);
    assert!(
        s.issues
            .iter()
            .any(|i| i.kind == IssueKind::Unsupported && i.what.contains("another workbook")),
        "{:?}",
        s.issues
    );
}

#[test]
fn table_header_and_totals_rows() {
    let table = "<table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"1\" name=\"T\" displayName=\"T\" ref=\"A1:B4\" headerRowCount=\"0\" totalsRowCount=\"1\"><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"a\"/><tableColumn id=\"2\" name=\"b\"/></tableColumns></table>";
    let bytes = fixture_with(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData/></worksheet>",
        &[(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table",
            "../tables/table1.xml",
        )],
        &[("xl/tables/table1.xml", table)],
    );
    let s = sheet_of(&bytes, "S", &Limits::default());
    assert_eq!(s.tables.len(), 1, "{:?}", s.issues);
    assert_eq!((s.tables[0].header_rows, s.tables[0].totals_rows), (0, 1));
}

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

fn sheet_xml_of(inner: &str) -> SheetParts {
    let bytes = fixture(&format!("<worksheet xmlns=\"{MAIN}\">{inner}</worksheet>"));
    sheet_of(&bytes, "S", &Limits::default())
}

#[test]
fn autofilter_ref_read() {
    let s = sheet_xml_of(
        "<sheetData/><autoFilter ref=\"A1:D20\"><filterColumn colId=\"0\"/></autoFilter>",
    );
    assert_eq!(s.auto_filter.as_deref(), Some("A1:D20"));
    assert!(sheet_xml_of("<sheetData/>").auto_filter.is_none());
    // A filter in a custom view or an extension is not the sheet's filter.
    let s = sheet_xml_of(
        "<sheetData/><customSheetViews><customSheetView guid=\"x\"><autoFilter ref=\"A1:B2\"/></customSheetView></customSheetViews>",
    );
    assert!(s.auto_filter.is_none());
}

#[test]
fn frozen_pane_rows_and_cols() {
    use sheetparts::ooxml::Frozen;
    let view = |pane: &str| {
        sheet_xml_of(&format!(
            "<sheetViews><sheetView workbookViewId=\"0\">{pane}</sheetView></sheetViews><sheetData/>"
        ))
    };
    let s =
        view("<pane ySplit=\"4\" topLeftCell=\"A5\" activePane=\"bottomLeft\" state=\"frozen\"/>");
    assert_eq!(s.frozen, Some(Frozen { rows: 4, cols: 0 }));
    let s = view("<pane xSplit=\"2\" ySplit=\"1\" state=\"frozen\"/>");
    assert_eq!(s.frozen, Some(Frozen { rows: 1, cols: 2 }));
    let s = view("<pane xSplit=\"2\" state=\"frozen\"/>");
    assert_eq!(s.frozen, Some(Frozen { rows: 0, cols: 2 }));
    assert_eq!(view("<pane ySplit=\"4\" state=\"split\"/>").frozen, None);
    assert_eq!(view("<pane ySplit=\"4\"/>").frozen, None);
    assert_eq!(view("").frozen, None);
    // Hostile numbers are clamped to the sheet size.
    let s = view("<pane xSplit=\"99999999\" ySplit=\"99999999\" state=\"frozen\"/>");
    assert_eq!(
        s.frozen,
        Some(Frozen {
            rows: 1_048_576,
            cols: 16_384
        })
    );
}

#[test]
fn pivot_location_attributes() {
    let bytes = corpus("ExcelPivotTableSample.xlsx");
    let mut archive = Archive::open(&bytes, zipguard::Limits::default()).unwrap();
    let limits = Limits::default();
    let wb = read_workbook(&mut archive, &limits, &|| false);
    let mut pivots = Vec::new();
    for sheet in &wb.sheets {
        pivots.extend(read_sheet(&mut archive, sheet, &wb, &limits, &|| false).pivots);
    }
    assert!(!pivots.is_empty());
    for p in &pivots {
        assert!(p.first_data_row >= p.first_header_row, "{p:?}");
        assert!(p.first_data_row > 0 || p.first_data_col > 0, "{p:?}");
        assert!(p.row_fields + p.col_fields > 0, "{p:?}");
    }

    let rel = |t: &str| {
        format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{t}")
    };
    let bytes = fixture_full(
        &format!("<worksheet xmlns=\"{MAIN}\"/>"),
        "<pivotCaches><pivotCache cacheId=\"7\" r:id=\"w0\"/></pivotCaches>",
        &[(&rel("pivotCacheDefinition"), "pc.xml")],
        &[(&rel("pivotTable"), "../pt.xml")],
        &[
            (
                "xl/pt.xml",
                &format!(
                    "<pivotTableDefinition xmlns=\"{MAIN}\" name=\"P\" cacheId=\"7\" colGrandTotals=\"0\">\
                     <location ref=\"B3:F9\" firstHeaderRow=\"1\" firstDataRow=\"2\" firstDataCol=\"1\"/>\
                     <rowFields count=\"2\"><field x=\"0\"/><field x=\"1\"/></rowFields>\
                     <colFields count=\"1\"><field x=\"-2\"/></colFields></pivotTableDefinition>"
                ),
            ),
            (
                "xl/pc.xml",
                &format!("<pivotCacheDefinition xmlns=\"{MAIN}\"/>"),
            ),
        ],
    );
    let p = &sheet_of(&bytes, "S", &Limits::default()).pivots[0];
    assert_eq!(p.location, "B3:F9");
    assert_eq!(
        (p.first_header_row, p.first_data_row, p.first_data_col),
        (1, 2, 1)
    );
    assert!(p.row_grand_totals);
    assert!(!p.col_grand_totals);
    assert_eq!((p.row_fields, p.col_fields), (2, 1));

    // Absent attributes: grand totals on, no fields, offsets 0.
    let s = pivot_source("sheet=\"A\" ref=\"A1:B2\"");
    let p = &s.pivots[0];
    assert!(p.row_grand_totals && p.col_grand_totals);
    assert_eq!(
        (p.first_header_row, p.first_data_row, p.first_data_col),
        (0, 0, 0)
    );
    assert_eq!((p.row_fields, p.col_fields), (0, 0));
}
