// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

mod common;

use common::*;

#[test]
fn values_and_number_formats() {
    let rows = "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>0.09</v></c><c r=\"B1\" t=\"inlineStr\"><is><t>Hello</t></is></c><c r=\"C1\" s=\"2\"><v>45000</v></c><c r=\"D1\" s=\"3\"><v>1.5</v></c><c r=\"E1\" s=\"4\"><v>12.34</v></c><c r=\"F1\" t=\"b\"><v>1</v></c><c r=\"G1\"><v>0.1</v></c></row><row r=\"3\"><c r=\"B3\" s=\"3\"><v>2.5</v></c></row>";
    let out = read(&Book::default().sheet("S", &ws("", rows, "")));
    let doc = &out.document;
    assert_eq!(cell_text(doc, "S!A1").as_deref(), Some("9%"));
    assert_eq!(cell_text(doc, "S!B1").as_deref(), Some("Hello"));
    assert_eq!(cell_text(doc, "S!C1").as_deref(), Some("03-15-23"));
    assert_eq!(cell_text(doc, "S!E1").as_deref(), Some("12.3 m"));
    assert_eq!(cell_text(doc, "S!F1").as_deref(), Some("TRUE"));
    assert_eq!(cell_text(doc, "S!G1").as_deref(), Some("0.1"));
    // The table runs from A1, so empty cells in between are there too.
    assert_eq!(cell_text(doc, "S!A2").as_deref(), Some(""));
    // A locale-dependent built-in format: the raw value, and one loss for both cells.
    assert_eq!(cell_text(doc, "S!D1").as_deref(), Some("1.5"));
    assert_eq!(cell_text(doc, "S!B3").as_deref(), Some("2.5"));
    let unapplied: Vec<_> = doc
        .losses
        .iter()
        .filter(|l| l.what.contains("format not applied"))
        .collect();
    assert_eq!(unapplied.len(), 1, "{:#?}", doc.losses);
    assert!(unapplied[0].what.contains("number format 27"));
    assert!(unapplied[0].what.contains("2 cells"));
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:#?}",
        doc.losses
    );
    // The section is the sheet, anchored by its name.
    assert!(matches!(&doc.body[0], Block::Section { anchor: Some(a), .. } if a.0 == "S"));
}

#[test]
fn dates_in_1904_workbooks() {
    let mut book = Book::default().sheet(
        "S",
        &ws(
            "",
            "<row r=\"1\"><c r=\"A1\" s=\"2\"><v>0</v></c></row>",
            "",
        ),
    );
    book.workbook_pr = "date1904=\"1\"".into();
    let out = read(&book);
    assert_eq!(
        cell_text(&out.document, "S!A1").as_deref(),
        Some("01-01-04")
    );
    assert_eq!(
        out.document.properties.custom.get("date_system"),
        Some(&Value::Text {
            value: "1904".into()
        })
    );
}

#[test]
fn formula_with_reads_and_cached_value() {
    let s = ws(
        "",
        "<row r=\"1\"><c r=\"A1\"><v>2</v></c><c r=\"B1\"><v>3</v></c><c r=\"C1\"><f>A1*B1</f><v>6</v></c></row><row r=\"2\"><c r=\"A2\" t=\"str\"><f>_xlfn.CONCAT(\"a\",\"b\")</f><v>ab</v></c></row>",
        "",
    );
    let t = ws(
        "",
        "<row r=\"1\"><c r=\"A1\"><f>S!C1+1</f><v>7</v></c></row>",
        "",
    );
    let out = read(&Book::default().sheet("S", &s).sheet("My Sheet", &t));
    let doc = &out.document;
    let c1 = formula(doc, "S!C1");
    assert_eq!(c1.expression, "=A1*B1");
    assert_eq!(c1.dialect, "excel");
    assert_eq!(refs(c1), ["S!A1", "S!B1"]);
    assert_eq!(c1.cached, Some(Value::number(6.0, None)));
    assert_eq!(c1.kind, Some(FormulaKind::Cell));
    assert_eq!(c1.original, None);
    assert_eq!(c1.note, None);
    assert!(c1.latex.as_deref().is_some_and(|l| l.contains("\\cdot")));
    assert_eq!(cell_text(doc, "S!C1").as_deref(), Some("6"));

    let a2 = formula(doc, "S!A2");
    assert_eq!(a2.expression, "=CONCAT(\"a\",\"b\")");
    assert_eq!(a2.original.as_deref(), Some("=_xlfn.CONCAT(\"a\",\"b\")"));
    assert_eq!(a2.cached, Some(Value::Text { value: "ab".into() }));

    let cross = formula(doc, "'My Sheet'!A1");
    assert_eq!(refs(cross), ["S!C1"]);
    assert!(matches!(&doc.body[1], Block::Section { anchor: Some(a), .. } if a.0 == "'My Sheet'"));
}

#[test]
fn missing_cached_value_says_not_calculated() {
    let s = ws(
        "",
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><f>A1+1</f></c></row>",
        "",
    );
    let out = read(&Book::default().sheet("S", &s));
    let f = formula(&out.document, "S!B1");
    assert_eq!(f.cached, None);
    assert_eq!(f.note.as_deref(), Some("not calculated in file"));
}

#[test]
fn hidden_sheet_and_rows_are_marked() {
    let s = ws(
        "<cols><col min=\"2\" max=\"3\" hidden=\"1\"/></cols>",
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"2\" hidden=\"1\"><c r=\"A2\"><v>2</v></c></row>",
        "",
    );
    let plain = ws("", "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>", "");
    let book = Book::default()
        .sheet("S", &s)
        .sheet("H", &plain)
        .state("hidden")
        .sheet("V", &plain)
        .state("veryHidden");
    let out = read(&book);
    let doc = &out.document;
    let status = |title: &str| -> Option<String> {
        section(doc, title).iter().find_map(|b| match b {
            Block::Paragraph { inlines, .. } => inlines.iter().find_map(|i| match i {
                Inline::Status { text, .. } => Some(text.clone()),
                _ => None,
            }),
            _ => None,
        })
    };
    assert_eq!(status("S"), None);
    assert_eq!(status("H").as_deref(), Some("hidden"));
    assert_eq!(status("V").as_deref(), Some("very hidden"));
    let notes = paragraphs(section(doc, "S"));
    assert!(notes.contains(&"Hidden rows: 2.".to_string()), "{notes:?}");
    assert!(
        notes.contains(&"Hidden columns: B to C.".to_string()),
        "{notes:?}"
    );
    // Hidden rows stay in the table.
    assert_eq!(cell_text(doc, "S!A2").as_deref(), Some("2"));

    let corpus = corpus("hidden_sheets.xlsx");
    let hidden: Vec<_> = ["Sheet1", "Sheet3"]
        .iter()
        .map(|t| {
            section(&corpus.document, t).iter().any(|b| {
                matches!(b, Block::Paragraph { inlines, .. }
                    if inlines.iter().any(|i| matches!(i, Inline::Status { text, .. } if text == "hidden")))
            })
        })
        .collect();
    assert_eq!(hidden, [true, true]);
}

#[test]
fn defined_names_with_scope() {
    let names = "<definedName name=\"gg\">S!$A$1</definedName><definedName name=\"gg\" localSheetId=\"0\">S!$B$1</definedName><definedName name=\"Rate\">0.05</definedName><definedName name=\"Twice\">_xlfn.LAMBDA(_xlpm.x,_xlpm.x*2)</definedName>";
    let s = ws(
        "",
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2</v></c><c r=\"C1\"><f>gg*2</f><v>4</v></c></row>",
        "",
    );
    let t = ws(
        "",
        "<row r=\"1\"><c r=\"A1\"><f>gg+_xludf.Twice(3)*Rate</f><v>1.3</v></c></row>",
        "",
    );
    let out = read(&Book::default().sheet("S", &s).sheet("T", &t).names(names));
    let doc = &out.document;
    // The sheet-local name wins on its own sheet.
    assert_eq!(refs(formula(doc, "S!C1")), ["name:S!gg"]);
    assert_eq!(refs(formula(doc, "T!A1")), ["name:gg", "name:Rate"]);

    let gg = formula(doc, "name:gg");
    assert_eq!(gg.kind, Some(FormulaKind::Name));
    assert_eq!(gg.expression, "S!$A$1");
    assert_eq!(refs(gg), ["S!A1"]);
    let local = formula(doc, "name:S!gg");
    assert_eq!(refs(local), ["S!B1"]);
    let rate = formula(doc, "name:Rate");
    assert_eq!(rate.expression, "0.05");
    let twice = formula(doc, "name:Twice");
    assert_eq!(twice.kind, Some(FormulaKind::Lambda));
    assert_eq!(twice.note.as_deref(), Some("parameters: x"));
    // Names show the display form, like cell formulas, with the stored text beside it.
    assert_eq!(twice.expression, "LAMBDA(x,x*2)");
    assert_eq!(
        twice.original.as_deref(),
        Some("_xlfn.LAMBDA(_xlpm.x,_xlpm.x*2)")
    );
    assert_eq!(gg.original, None);
    // A workbook lambda is not an unknown user function.
    assert!(
        !doc.facets
            .embedded
            .iter()
            .any(|e| e.kind == "user-function"),
        "{:#?}",
        doc.facets.embedded
    );

    let lambda = corpus("LAMBDA.xlsx");
    let doc = &lambda.document;
    let count = formula(doc, "name:CountWords");
    assert_eq!(count.kind, Some(FormulaKind::Lambda));
    assert_eq!(count.note.as_deref(), Some("parameters: text"));
    assert!(
        !doc.facets
            .embedded
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case("CountWords")),
        "{:#?}",
        doc.facets.embedded
    );
}

#[test]
fn lambda_only_when_the_whole_name_is_one() {
    let names = "<definedName name=\"f\">_xlfn.LET(_xlpm.g,_xlfn.LAMBDA(_xlpm.x,_xlpm.x+1),_xlpm.g(1))</definedName><definedName name=\"now\">_xlfn.LAMBDA(_xlpm.x,_xlpm.x)(2)</definedName><definedName name=\"two\">LAMBDA(a, b, a*b)</definedName>";
    let s = ws("", "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>", "");
    let out = read(&Book::default().sheet("S", &s).names(names));
    let doc = &out.document;
    assert_eq!(formula(doc, "name:f").kind, Some(FormulaKind::Name));
    assert_eq!(formula(doc, "name:now").kind, Some(FormulaKind::Name));
    let two = formula(doc, "name:two");
    assert_eq!(two.kind, Some(FormulaKind::Lambda));
    assert_eq!(two.note.as_deref(), Some("parameters: a, b"));
}

#[test]
fn threaded_comment_lands_at_cell() {
    let out = corpus("threadedComment.xlsx");
    let doc = &out.document;
    let threads: Vec<_> = doc
        .facets
        .annotations
        .iter()
        .filter(|a| a.kind == "comment")
        .collect();
    assert_eq!(threads.len(), 1, "{:#?}", doc.facets.annotations);
    let t = threads[0];
    assert_eq!(t.anchor.as_ref().map(|a| a.0.as_str()), Some("Sheet1!A2"));
    assert_eq!(t.author.as_deref(), Some("Mike Kaganski"));
    assert_eq!(t.date.as_deref(), Some("2026-03-24T09:25:49.80"));
    assert_eq!(plain_text(&t.body), "a comment on A2");
    assert_eq!(t.resolved, Some(false));
    assert_eq!(t.replies.len(), 1);
    assert_eq!(plain_text(&t.replies[0].body), "A reply");
    assert_eq!(t.replies[0].author.as_deref(), Some("Mike Kaganski"));
    // The placeholder note Excel writes for old programs is not a second comment; the
    // real note on A5 stays.
    let notes: Vec<_> = doc
        .facets
        .annotations
        .iter()
        .filter(|a| a.kind == "note")
        .collect();
    assert_eq!(notes.len(), 1, "{notes:#?}");
    assert_eq!(notes[0].anchor.as_ref().unwrap().0, "Sheet1!A5");
}

#[test]
fn legacy_note_lands_at_cell() {
    let comments = format!(
        "<comments xmlns=\"{MAIN}\"><authors><author>Ann</author></authors><commentList><comment ref=\"B2\" authorId=\"0\"><text><r><t>check this</t></r></text></comment></commentList></comments>"
    );
    let s = ws("", "<row r=\"2\"><c r=\"B2\"><v>1</v></c></row>", "");
    let out = read(&Book::default().sheet("S", &s).related(
        "comments",
        "xl/comments1.xml",
        &comments,
    ));
    let notes = &out.document.facets.annotations;
    assert_eq!(notes.len(), 1, "{notes:#?}");
    assert_eq!(notes[0].kind, "note");
    assert_eq!(notes[0].author.as_deref(), Some("Ann"));
    assert_eq!(notes[0].anchor.as_ref().unwrap().0, "S!B2");
    assert_eq!(plain_text(&notes[0].body), "check this");
}

#[test]
fn vba_and_udf_are_embedded_never_run() {
    let out = corpus("testNames.xlsm");
    let doc = &out.document;
    let embedded = &doc.facets.embedded;
    let macros: Vec<_> = embedded.iter().filter(|e| e.kind == "macro").collect();
    assert!(!macros.is_empty(), "{embedded:#?}");
    assert!(
        macros
            .iter()
            .all(|m| m.detail.as_deref() == Some("VBA module, never run"))
    );
    let udf = |name: &str| {
        embedded
            .iter()
            .find(|e| e.kind == "user-function" && e.name == name)
            .unwrap_or_else(|| panic!("no user function {name}: {embedded:#?}"))
    };
    let my_func = udf("myFunc");
    assert_eq!(my_func.detail.as_deref(), Some("VBA module"));
    assert!(
        macros
            .iter()
            .any(|m| Some(&m.name) == my_func.target.as_ref()),
        "{embedded:#?}"
    );
    assert_eq!(udf("myFunc2").detail.as_deref(), Some("VBA module"));
    // The cached results stay as stored; nothing was run.
    assert_eq!(formula(doc, "Sheet1!B1").expression, "=myFunc(\"_\")");
    assert_eq!(
        formula(doc, "Sheet1!B1").cached,
        Some(Value::Text {
            value: "_abc".into()
        })
    );

    let addin = corpus("ex054.MF_friction_factor.xlsm");
    let embedded = &addin.document.facets.embedded;
    let path = "/unifloc/unifloc_vba/UniflocVBA_7.xlam";
    assert!(
        embedded
            .iter()
            .any(|e| e.kind == "add-in" && e.target.as_deref() == Some(path)),
        "{embedded:#?}"
    );
    assert!(
        embedded
            .iter()
            .filter(|e| e.kind == "user-function" && e.name.starts_with("[1]!"))
            .all(|e| e.detail.as_deref() == Some(path)),
        "{embedded:#?}"
    );
    assert!(
        embedded
            .iter()
            .any(|e| e.kind == "user-function" && e.name.starts_with("[1]!")),
        "{embedded:#?}"
    );
}

#[test]
fn damaged_sheet_xml_is_partial_with_loss() {
    let bad = format!(
        "<worksheet xmlns=\"{MAIN}\"><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2"
    );
    let good = ws("", "<row r=\"1\"><c r=\"A1\"><v>5</v></c></row>", "");
    let out = read(&Book::default().sheet("Bad", &bad).sheet("Good", &good));
    let doc = &out.document;
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(
        has_loss(doc, LossReason::Damaged, "Bad", "could not be read"),
        "{:#?}",
        doc.losses
    );
    assert_eq!(cell_text(doc, "Bad!A1").as_deref(), Some("1"));
    assert_eq!(cell_text(doc, "Good!A1").as_deref(), Some("5"));
}

#[test]
fn part_over_unpacked_limit_is_skipped_with_loss() {
    // Every entry fits the preflight, but reading the big sheet would take the unpacked
    // total past the limit: that sheet is skipped, the one before it is read.
    let mut rows = String::new();
    for r in 1..=5_000 {
        rows.push_str(&format!(
            "<row r=\"{r}\"><c r=\"A{r}\"><v>{r}</v></c></row>"
        ));
    }
    let big = ws("", &rows, "");
    assert!(big.len() > 200_000);
    let small = ws("", "<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>", "");
    let limits = Limits {
        max_unpacked_bytes: 500_000,
        max_ratio: 100_000,
        ..Limits::default()
    };
    let out = read_with(
        Book::default()
            .sheet("Small", &small)
            .sheet("Big", &big)
            .bytes(),
        limits,
    );
    let doc = &out.document;
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(
        has_loss(doc, LossReason::Limit, "Big", "sheet not read"),
        "{:#?}",
        doc.losses
    );
    assert!(cell_text(doc, "Big!A1").is_none());
    assert_eq!(cell_text(doc, "Small!A1").as_deref(), Some("7"));
}

#[test]
fn entry_over_unpacked_limit_fails_the_read() {
    let mut rows = String::new();
    for r in 1..=5_000 {
        rows.push_str(&format!(
            "<row r=\"{r}\"><c r=\"A{r}\"><v>{r}</v></c></row>"
        ));
    }
    let limits = Limits {
        max_unpacked_bytes: 100_000,
        max_ratio: 100_000,
        ..Limits::default()
    };
    let out = read_with(
        Book::default()
            .sheet("Small", &small_sheet())
            .sheet("Big", &ws("", &rows, ""))
            .bytes(),
        limits,
    );
    refused(&out, &[LossReason::Limit]);
}

#[test]
fn workbook_part_over_limit_fails() {
    let mut book = Book::default().sheet("S", &ws("", "", ""));
    book.names = "<definedName name=\"x\">1</definedName>".repeat(5_000);
    let limits = Limits {
        max_unpacked_bytes: 50_000,
        max_ratio: 100_000,
        ..Limits::default()
    };
    let out = read_with(book.bytes(), limits);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("the whole file")),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn formula_longer_than_excel_maximum_is_refused() {
    let long = vec!["A1"; 3_000].join("+");
    assert!(long.len() > 8_192);
    let s = ws(
        "",
        &format!(
            "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><f>{long}</f><v>3000</v></c></row>"
        ),
        "",
    );
    let out = read(&Book::default().sheet("S", &s));
    let doc = &out.document;
    let f = formula(doc, "S!B1");
    assert!(f.references.is_empty());
    assert!(
        f.note
            .as_deref()
            .is_some_and(|n| n.contains("not analyzed"))
    );
    assert_eq!(f.expression, format!("={long}"));
    assert!(
        has_loss(doc, LossReason::Damaged, "S!B1", "Excel's maximum"),
        "{:#?}",
        doc.losses
    );
}

#[test]
fn unreadable_formula_keeps_text_with_loss() {
    let s = ws("", "<row r=\"1\"><c r=\"A1\"><f>SUM(1,,</f></c></row>", "");
    let out = read(&Book::default().sheet("S", &s));
    let doc = &out.document;
    let f = formula(doc, "S!A1");
    assert_eq!(f.expression, "=SUM(1,,");
    assert!(
        f.note
            .as_deref()
            .is_some_and(|n| n.contains("formula not readable"))
    );
    assert!(has_loss(
        doc,
        LossReason::Unsupported,
        "S!A1",
        "text is kept"
    ));
}

fn table_xml(name: &str, range: &str, columns: &[&str]) -> String {
    let cols: String = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("<tableColumn id=\"{}\" name=\"{c}\"/>", i + 1))
        .collect();
    format!(
        "<table xmlns=\"{MAIN}\" id=\"1\" name=\"{name}\" displayName=\"{name}\" ref=\"{range}\"><tableColumns count=\"{}\">{cols}</tableColumns></table>",
        columns.len()
    )
}

#[test]
fn table_reference_without_name_reads_containing_table() {
    let s = ws(
        "",
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Qty</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>Total</t></is></c></row><row r=\"2\"><c r=\"A2\"><v>4</v></c><c r=\"B2\"><f>[@Qty]*2</f><v>8</v></c></row><row r=\"3\"><c r=\"A3\"><v>5</v></c><c r=\"B3\"><f>Sales[[#This Row],[Qty]]*2</f><v>10</v></c></row><row r=\"4\"><c r=\"D4\"><f>SUM(Sales[Total])</f><v>18</v></c></row>",
        "",
    );
    let out = read(&Book::default().sheet("S", &s).related(
        "table",
        "xl/tables/table1.xml",
        &table_xml("Sales", "A1:B3", &["Qty", "Total"]),
    ));
    let doc = &out.document;
    assert_eq!(refs(formula(doc, "S!B2")), ["table:Sales"]);
    assert_eq!(refs(formula(doc, "S!B3")), ["table:Sales"]);
    assert_eq!(refs(formula(doc, "S!D4")), ["table:Sales"]);
    let tables: Vec<_> = section(doc, "S")
        .iter()
        .filter(|b| matches!(b, Block::Table { role: Some(r), .. } if r == "excel-table"))
        .collect();
    assert_eq!(tables.len(), 1);
    assert!(out.document.losses.is_empty(), "{:#?}", out.document.losses);
}

#[test]
fn damaged_table_area_is_a_loss() {
    let s = ws("", "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>", "");
    let out = read(&Book::default().sheet("S", &s).related(
        "table",
        "xl/tables/table1.xml",
        &table_xml("Odd", "A1:B3", &["a", "b", "c"]),
    ));
    assert!(
        has_loss(&out.document, LossReason::Damaged, "table:Odd", "table Odd"),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn data_table_cells_come_from_the_xml() {
    let out = corpus("one-variable.xlsx");
    let doc = &out.document;
    let dt = formula(doc, "'One-Var'!B5");
    assert_eq!(dt.kind, Some(FormulaKind::DataTable));
    assert_eq!(
        dt.range.as_ref().map(|a| a.0.as_str()),
        Some("'One-Var'!B5:B11")
    );
    assert_eq!(refs(dt), ["'One-Var'!A2"]);
    assert_eq!(dt.expression, "=TABLE(,A2)");
    let row = formula(doc, "'One-Var'!E5");
    assert_eq!(row.expression, "=TABLE(B2,)");
    assert_eq!(
        row.range.as_ref().map(|a| a.0.as_str()),
        Some("'One-Var'!E5:I5")
    );
}

#[test]
fn spill_and_array_formulas_have_their_range() {
    let out = corpus("DynamicArrays.xlsx");
    let doc = &out.document;
    let spill = doc
        .facets
        .formulas
        .iter()
        .find(|f| f.expression.contains("SEQUENCE"))
        .expect("SEQUENCE formula");
    assert_eq!(spill.kind, Some(FormulaKind::Spill), "{spill:#?}");
    assert!(
        spill
            .range
            .as_ref()
            .is_some_and(|r| r.0.ends_with("!A3:B12"))
    );
}

#[test]
fn merged_cells_become_spans() {
    let s = ws(
        "",
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"C1\"><v>3</v></c></row><row r=\"2\"><c r=\"C2\"><v>4</v></c></row>",
        "<mergeCells count=\"1\"><mergeCell ref=\"A1:B2\"/></mergeCells>",
    );
    let out = read(&Book::default().sheet("S", &s));
    let rows = all_blocks(&out.document)
        .into_iter()
        .find_map(|b| match b {
            Block::Table {
                rows, role: None, ..
            } => Some(rows.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0][0].row_span, rows[0][0].col_span), (2, 2));
    // Covered cells are left out: row 1 is A1 (merged) and C1, row 2 only C2.
    let anchors = |r: &Vec<rezzer_model::Cell>| {
        r.iter()
            .map(|c| c.anchor.as_ref().unwrap().0.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(anchors(&rows[0]), ["S!A1", "S!C1"]);
    assert_eq!(anchors(&rows[1]), ["S!C2"]);
}

#[test]
fn rules_and_calc_settings() {
    let s = ws(
        "",
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>",
        "<conditionalFormatting sqref=\"A1:A9\"><cfRule type=\"cellIs\" priority=\"1\" operator=\"greaterThan\"><formula>5</formula></cfRule></conditionalFormatting><dataValidations count=\"1\"><dataValidation type=\"whole\" operator=\"between\" sqref=\"B1:B5 D1\" promptTitle=\"x\" prompt=\"1 to 10\"><formula1>1</formula1><formula2>10</formula2></dataValidation></dataValidations>",
    );
    let mut book = Book::default().sheet("S", &s);
    book.calc_pr = "<calcPr calcMode=\"manual\" iterate=\"1\" iterateCount=\"50\"/>".into();
    let out = read(&book);
    let doc = &out.document;
    let rules = &doc.facets.rules;
    let validations: Vec<_> = rules.iter().filter(|r| r.kind == "validation").collect();
    assert_eq!(validations.len(), 2, "{rules:#?}");
    assert_eq!(validations[0].anchor.0, "S!B1:B5");
    assert_eq!(validations[1].anchor.0, "S!D1");
    assert_eq!(validations[0].detail, "whole between");
    assert_eq!(validations[0].formulas, ["1", "10"]);
    assert_eq!(validations[0].message.as_deref(), Some("prompt: 1 to 10"));
    let cf: Vec<_> = rules
        .iter()
        .filter(|r| r.kind == "conditional-format")
        .collect();
    assert_eq!(cf.len(), 1);
    assert_eq!(cf[0].anchor.0, "S!A1:A9");
    assert_eq!(cf[0].formulas, ["5"]);
    let custom = &doc.properties.custom;
    assert_eq!(
        custom.get("calc_mode"),
        Some(&Value::Text {
            value: "manual".into()
        })
    );
    assert_eq!(
        custom.get("iterative_calculation"),
        Some(&Value::Bool { value: true })
    );
}

#[test]
fn document_properties_are_read() {
    let out = corpus("testEXCEL_custom_props.xlsx");
    let props = &out.document.properties;
    // Five custom properties next to the calc settings.
    let own = [
        "calc_mode",
        "iterative_calculation",
        "application",
        "company",
        "subject",
        "last_modified_by",
        "category",
    ];
    let custom = props
        .custom
        .keys()
        .filter(|k| !own.contains(&k.as_str()))
        .count();
    assert_eq!(custom, 5, "{:#?}", props.custom);
}

#[test]
fn quote_sheet_rules_agree() {
    for name in [
        "Data",
        "a.b",
        "2024",
        "1abc",
        "A1",
        "xfd1048576",
        "R1C1",
        "R",
        "C2",
        "rc",
        "it's",
        "My Sheet",
        "Übersicht",
        "Лист1",
        "_x",
        "Sheet-1",
        "",
        "'",
        "a'b c",
    ] {
        assert_eq!(
            formulakit::quote_sheet(name),
            sheetparts::quote_sheet(name),
            "sheet name {name:?}"
        );
    }
}

#[test]
fn xlsb_and_ods_are_not_read_yet() {
    for format in [FormatId::Xlsb, FormatId::Ods] {
        let out = SheetReader.read(
            &source(vec![1, 2, 3], "x", format),
            &ReadOptions::default(),
            &Budget::new(Limits::default()),
        );
        assert_eq!(out.completeness, Completeness::Failed);
        assert!(
            out.document
                .losses
                .iter()
                .any(|l| l.why == LossReason::NotImplemented)
        );
    }
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

#[test]
fn not_a_zip_fails() {
    let out = read_with(b"not a zip at all".to_vec(), Limits::default());
    assert_eq!(out.completeness, Completeness::Failed);
}

#[test]
fn every_corpus_workbook_reads() {
    let dir = format!(
        "{}/../../../tests/corpus/office",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !(name.ends_with(".xlsx") || name.ends_with(".xlsm")) {
            continue;
        }
        seen += 1;
        let out = corpus(&name);
        assert_ne!(
            out.completeness,
            Completeness::Failed,
            "{name}: {:#?}",
            out.document.losses
        );
        assert!(
            out.document
                .losses
                .iter()
                .all(|l| l.why != LossReason::Damaged && l.why != LossReason::Internal),
            "{name}: {:#?}",
            out.document.losses
        );
        assert!(!out.document.body.is_empty(), "{name}");
    }
    assert!(seen >= 25, "{seen}");
}

#[test]
fn far_cell_keeps_the_table_within_the_caps() {
    let s = ws(
        "<dimension ref=\"A1:XFD1048576\"/>",
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"1048576\"><c r=\"XFD1048576\"><f>A1*2</f><v>2</v></c></row>",
        "<mergeCells count=\"1\"><mergeCell ref=\"A1:XFD1048576\"/></mergeCells>",
    );
    let started = std::time::Instant::now();
    let out = read(&Book::default().sheet("S", &s));
    assert!(started.elapsed().as_secs() < 10);
    let doc = &out.document;
    let rows = all_blocks(doc)
        .into_iter()
        .find_map(|b| match b {
            Block::Table {
                rows, role: None, ..
            } => Some(rows.len()),
            _ => None,
        })
        .unwrap();
    assert_eq!(rows, 200);
    assert!(has_loss(
        doc,
        LossReason::Limit,
        "S",
        "rows 201 to 1048576 not shown"
    ));
    assert!(has_loss(
        doc,
        LossReason::Limit,
        "S",
        "columns ALM to XFD not shown"
    ));
    // The formula past the caps is still listed.
    assert_eq!(refs(formula(doc, "S!XFD1048576")), ["S!A1"]);
}

#[test]
fn uncalculated_data_table_is_still_listed() {
    let s = ws(
        "",
        "<row r=\"2\"><c r=\"A2\"><v>1</v></c></row><row r=\"5\"><c r=\"B5\"><f t=\"dataTable\" ref=\"B5:B7\" dt2D=\"0\" dtr=\"0\" r1=\"A2\"/></c></row>",
        "",
    );
    let out = read(&Book::default().sheet("S", &s));
    let f = formula(&out.document, "S!B5");
    assert_eq!(f.kind, Some(FormulaKind::DataTable));
    assert_eq!(f.cached, None);
    assert!(
        f.note
            .as_deref()
            .is_some_and(|n| n.contains("not calculated in file")),
        "{f:#?}"
    );
}

/// A worksheet whose only cell holds `BOMB` plus 5 MB of `x`: far above 100:1.
fn bomb_sheet() -> String {
    ws(
        "",
        &format!(
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>BOMB{}</t></is></c></row>",
            "x".repeat(5_000_000)
        ),
        "",
    )
}

fn small_sheet() -> String {
    ws("", "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>", "")
}

/// The read failed, said why, and no text of the bomb part reached the document.
fn refused(out: &ReadOutcome, why: &[LossReason]) {
    let doc = &out.document;
    assert_eq!(out.completeness, Completeness::Failed, "{:#?}", doc.losses);
    assert!(
        doc.losses
            .iter()
            .any(|l| why.contains(&l.why) && l.what.contains("the whole file")),
        "{:#?}",
        doc.losses
    );
    let json = format!("{doc:?}");
    assert!(!json.contains("BOMB"));
}

#[test]
fn second_office_document_relationship_is_refused() {
    let evil_sst = format!(
        "<sst xmlns=\"{MAIN}\" count=\"1\" uniqueCount=\"1\"><si><t>BOMB{}</t></si></sst>",
        "x".repeat(5_000_000)
    );
    let mut book = Book::default()
        .sheet("S", &small_sheet())
        .entry(
            "evil/workbook.xml",
            format!(
                "<workbook xmlns=\"{MAIN}\" xmlns:r=\"{RELS}\"><sheets><sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
            ),
        )
        .entry(
            "evil/_rels/workbook.xml.rels",
            format!(
                "<Relationships xmlns=\"{PKG}\"><Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"/xl/worksheets/sheet1.xml\"/></Relationships>"
            ),
        );
    book.root_rels_extra = format!(
        "<Relationship Id=\"rId9\" Type=\"{RELS}/officeDocument\" Target=\"evil/workbook.xml\"/>"
    );
    // Without a bomb: the structure itself is refused.
    refused(&read(&book), &[LossReason::Damaged]);
    let book = book.entry("evil/sharedStrings.xml", evil_sst);
    refused(&read(&book), &[LossReason::Limit, LossReason::Damaged]);
}

#[test]
fn duplicate_relationship_id_is_refused() {
    let mut book = Book::default().sheet("S", &small_sheet());
    book.wb_rels_extra = format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"worksheets/other.xml\"/>"
    );
    let plain = book.entry("xl/worksheets/other.xml", small_sheet());
    refused(&read(&plain), &[LossReason::Damaged]);

    let mut book = Book::default().sheet("S", &small_sheet());
    book.wb_rels_extra = format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"worksheets/bomb.xml\"/>"
    );
    let bomb = book.entry("xl/worksheets/bomb.xml", bomb_sheet());
    refused(&read(&bomb), &[LossReason::Limit, LossReason::Damaged]);
}

#[test]
fn duplicate_sheet_names_are_refused() {
    let plain = Book::default()
        .sheet("A", &small_sheet())
        .sheet("a", &small_sheet());
    refused(&read(&plain), &[LossReason::Damaged]);
    let bomb = Book::default()
        .sheet("A", &bomb_sheet())
        .sheet("A", &small_sheet());
    refused(&read(&bomb), &[LossReason::Limit, LossReason::Damaged]);
}

#[test]
fn dotted_part_paths_are_refused() {
    let plain = Book::default()
        .sheet("S", &small_sheet())
        .target("./worksheets/sheet1.xml");
    refused(&read(&plain), &[LossReason::Damaged]);
    let bomb = Book::default()
        .sheet("S", &small_sheet())
        .target("./worksheets/sheet1.xml")
        .entry("xl/./worksheets/sheet1.xml", bomb_sheet());
    refused(&read(&bomb), &[LossReason::Limit, LossReason::Damaged]);
    let external = format!(
        "<Relationship Id=\"rIdE\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet1.xml\" TargetMode=\"External\"/>"
    );
    let mut ext = Book::default()
        .sheet("S", &small_sheet())
        .target("worksheets/missing.xml");
    ext.wb_rels_extra = external;
    refused(&read(&ext), &[LossReason::Damaged]);
}

#[test]
fn entries_differing_only_in_case_are_refused() {
    let book = Book::default()
        .sheet("S", &small_sheet())
        .entry("XL/worksheets/Sheet1.xml", small_sheet());
    refused(&read(&book), &[LossReason::Damaged]);
}

#[test]
fn unread_entry_over_the_ratio_fails_the_read() {
    // calamine never opens this entry, but nothing is unpacked unmeasured.
    let book = Book::default()
        .sheet("S", &small_sheet())
        .entry("xl/media/huge.bin", " ".repeat(5_000_000));
    refused(&read(&book), &[LossReason::Limit]);
}

#[test]
fn shared_formula_copies_stay_within_the_text_limit() {
    let master = "A1+".repeat(2_666) + "A1";
    assert!(master.len() > 7_990 && master.len() <= 8_192);
    let mut rows = format!(
        "<row r=\"1\"><c r=\"B1\"><f t=\"shared\" ref=\"B1:B20000\" si=\"0\">{master}</f><v>1</v></c></row>"
    );
    for r in 2..=20_000 {
        rows.push_str(&format!(
            "<row r=\"{r}\"><c r=\"B{r}\"><f t=\"shared\" si=\"0\"/><v>1</v></c></row>"
        ));
    }
    let limits = Limits {
        max_text_bytes: 1_000_000,
        ..Limits::default()
    };
    let out = read_with(
        Book::default().sheet("S", &ws("", &rows, "")).bytes(),
        limits,
    );
    let doc = &out.document;
    let text: usize = doc
        .facets
        .formulas
        .iter()
        .map(|f| f.expression.len() + f.original.as_ref().map_or(0, String::len))
        .sum();
    assert!(
        text <= 1_000_000 + 2 * 8_200,
        "{text} bytes of formula text"
    );
    assert!(
        doc.losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("formula text")),
        "{:#?}",
        doc.losses
    );
    assert_eq!(out.completeness, Completeness::Partial);
    // The values are still all there.
    assert_eq!(cell_text(doc, "S!B20000").as_deref(), Some("1"));
}

#[test]
fn shared_copies_before_their_master_stay_within_the_text_limit() {
    let master = "A1+".repeat(2_666) + "A1";
    let mut rows = String::new();
    for r in 1..20_000 {
        rows.push_str(&format!(
            "<row r=\"{r}\"><c r=\"B{r}\"><f t=\"shared\" si=\"0\"/><v>1</v></c></row>"
        ));
    }
    rows.push_str(&format!(
        "<row r=\"20000\"><c r=\"B20000\"><f t=\"shared\" ref=\"B1:B20000\" si=\"0\">{master}</f><v>1</v></c></row>"
    ));
    let limits = Limits {
        max_text_bytes: 1_000_000,
        ..Limits::default()
    };
    let out = read_with(
        Book::default().sheet("S", &ws("", &rows, "")).bytes(),
        limits,
    );
    let doc = &out.document;
    let text: usize = doc
        .facets
        .formulas
        .iter()
        .map(|f| f.expression.len() + f.original.as_ref().map_or(0, String::len))
        .sum();
    assert!(
        text <= 1_000_000 + 2 * 8_200,
        "{text} bytes of formula text"
    );
    assert!(
        doc.losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("formula text")),
        "{:#?}",
        doc.losses
    );
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn sheet_relationship_type_without_slash_is_still_checked() {
    let plain = Book::default()
        .sheet("S", &small_sheet())
        .rel_type("worksheet")
        .target("./worksheets/sheet1.xml");
    refused(&read(&plain), &[LossReason::Damaged]);
    let other = ws(
        "",
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>BOMB other part</t></is></c></row>",
        "",
    );
    let split = Book::default()
        .sheet("S", &small_sheet())
        .rel_type("worksheet")
        .target("./worksheets/sheet1.xml")
        .entry("xl/./worksheets/sheet1.xml", other);
    refused(&read(&split), &[LossReason::Damaged]);
}

#[test]
fn defined_name_texts_count_against_the_text_limit() {
    let text = format!("\"{}\"", "n".repeat(4_000));
    let names: String = (0..300)
        .map(|i| format!("<definedName name=\"big{i}\">{text}</definedName>"))
        .collect();
    let limits = Limits {
        max_text_bytes: 100_000,
        max_ratio: 100_000,
        ..Limits::default()
    };
    let out = read_with(
        Book::default()
            .sheet("S", &small_sheet())
            .names(&names)
            .bytes(),
        limits,
    );
    let doc = &out.document;
    let kept: usize = doc
        .facets
        .formulas
        .iter()
        .map(|f| f.expression.len() + f.original.as_ref().map_or(0, String::len))
        .sum();
    assert!(kept <= 100_000, "{kept}");
    assert!(
        doc.losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("defined names")),
        "{:#?}",
        doc.losses
    );
}

fn raw_zip(files: &[(&str, String)]) -> Vec<u8> {
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, content) in files {
        w.start_file(*name, opts).unwrap();
        w.write_all(content.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// A package whose sheet relationships and sheet elements are given, with a small sheet at
/// `xl/worksheets/sheet1.xml` and a different part at `xl/./worksheets/sheet1.xml`.
fn split_pkg(sheet_rels: &str, sheet_elems: &str, rels_ns: &str) -> Vec<u8> {
    split_pkg_ns(sheet_rels, sheet_elems, rels_ns, " xmlns:x=\"urn:x\"")
}

/// Like `split_pkg`, with the extra namespace declarations of the workbook root given.
fn split_pkg_ns(sheet_rels: &str, sheet_elems: &str, rels_ns: &str, wb_ns: &str) -> Vec<u8> {
    let bomb = ws(
        "",
        &format!(
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>BOMB{}</t></is></c></row>",
            " ".repeat(2_000_000)
        ),
        "",
    );
    raw_zip(&[
        (
            "[Content_Types].xml",
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>"
                .into(),
        ),
        (
            "_rels/.rels",
            format!(
                "<Relationships xmlns=\"{PKG}\"><Relationship Id=\"rId1\" Type=\"{RELS}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
            ),
        ),
        (
            "xl/workbook.xml",
            format!(
                "<workbook xmlns=\"{MAIN}\" xmlns:r=\"{RELS}\"{wb_ns}><sheets>{sheet_elems}</sheets></workbook>"
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!("<Relationships xmlns=\"{PKG}\"{rels_ns}>{sheet_rels}</Relationships>"),
        ),
        ("xl/worksheets/sheet1.xml", small_sheet()),
        ("xl/./worksheets/sheet1.xml", bomb),
    ])
}

fn split_limits() -> Limits {
    Limits {
        max_unpacked_bytes: 3_000_000,
        max_ratio: 100_000,
        ..Limits::default()
    }
}

const SHEET: &str = "<sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\"/>";

#[test]
fn canonical_package_reads() {
    let rels = format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet1.xml\"/>"
    );
    let out = read_with(split_pkg(&rels, SHEET, ""), split_limits());
    // The dotted entry is never named, so the package is plain; only the small sheet is read.
    assert_ne!(
        out.completeness,
        Completeness::Failed,
        "{:#?}",
        out.document.losses
    );
    assert_eq!(cell_text(&out.document, "S!A1").as_deref(), Some("1"));
}

#[test]
fn prefixed_relationship_id_is_refused() {
    let rels = format!(
        "<Relationship x:Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         <Relationship Id=\"rId1\" x:Id=\"rIdZ\" Type=\"{RELS}/worksheet\" Target=\"./worksheets/sheet1.xml\"/>"
    );
    let out = read_with(
        split_pkg(&rels, SHEET, " xmlns:x=\"urn:x\""),
        split_limits(),
    );
    refused(&out, &[LossReason::Damaged]);
}

#[test]
fn prefixed_relationship_target_is_refused() {
    let rels = format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" x:Target=\"./worksheets/sheet1.xml\" Target=\"worksheets/sheet1.xml\"/>"
    );
    let out = read_with(
        split_pkg(&rels, SHEET, " xmlns:x=\"urn:x\""),
        split_limits(),
    );
    refused(&out, &[LossReason::Damaged]);
}

#[test]
fn duplicate_relationship_type_is_refused() {
    let rels = format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" x:Type=\"{RELS}/styles\" Target=\"worksheets/sheet1.xml\"/>"
    );
    let out = read_with(
        split_pkg(&rels, SHEET, " xmlns:x=\"urn:x\""),
        split_limits(),
    );
    refused(&out, &[LossReason::Damaged]);
    // A repeated attribute is refused as well.
    let rels = format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Type=\"{RELS}/styles\" Target=\"worksheets/sheet1.xml\"/>"
    );
    let out = read_with(split_pkg(&rels, SHEET, ""), split_limits());
    refused(&out, &[LossReason::Damaged]);
}

#[test]
fn second_sheet_id_attribute_is_refused() {
    let rels = format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         <Relationship Id=\"rId2\" Type=\"{RELS}/worksheet\" Target=\"./worksheets/sheet1.xml\"/>"
    );
    let sheet = "<sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\" x:id=\"rId2\"/>";
    let out = read_with(split_pkg(&rels, sheet, ""), split_limits());
    refused(&out, &[LossReason::Damaged]);
    let foreign = "<sheet name=\"S\" sheetId=\"1\" x:id=\"rId1\"/>";
    let rels = format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet1.xml\"/>"
    );
    let out = read_with(split_pkg(&rels, foreign, ""), split_limits());
    refused(&out, &[LossReason::Damaged]);
}

/// Two sheet relationships: `rId1` names the small sheet, `rId2` the dotted part, which
/// calamine would read as `xl/./worksheets/sheet1.xml`.
fn two_sheet_rels() -> String {
    format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         <Relationship Id=\"rId2\" Type=\"{RELS}/worksheet\" Target=\"./worksheets/sheet1.xml\"/>"
    )
}

#[test]
fn sheet_id_name_with_two_colons_is_refused() {
    // calamine takes the last attribute whose name ends in `:id` (rId2), sheetparts the first
    // local `id` (rId1).
    let sheet = "<sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\" r:x:id=\"rId2\"/>";
    let out = read_with(split_pkg(&two_sheet_rels(), sheet, ""), split_limits());
    refused(&out, &[LossReason::Damaged]);
    // The same with a prefix the root declares.
    let sheet = "<sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\" x:b:id=\"rId2\"/>";
    let out = read_with(split_pkg(&two_sheet_rels(), sheet, ""), split_limits());
    refused(&out, &[LossReason::Damaged]);
    // The reviewer's probe 20c as written.
    let sheet = "<sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\" a:b:id=\"rId2\"/>";
    let out = read_with(
        split_pkg_ns(&two_sheet_rels(), sheet, "", " xmlns:a=\"urn:a\""),
        split_limits(),
    );
    refused(&out, &[LossReason::Damaged]);
}

#[test]
fn second_relationship_prefix_on_a_sheet_is_refused() {
    // Both prefixes are the relationships namespace; still only one id is allowed.
    let sheet = "<sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\" r2:id=\"rId2\"/>";
    let ns = format!(" xmlns:r2=\"{RELS}\"");
    let out = read_with(
        split_pkg_ns(&two_sheet_rels(), sheet, "", &ns),
        split_limits(),
    );
    refused(&out, &[LossReason::Damaged]);
}

#[test]
fn unknown_sheet_attribute_is_refused() {
    let rels = format!(
        "<Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet1.xml\"/>"
    );
    for sheet in [
        "<sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\" foo=\"bar\"/>",
        "<sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\" x:foo=\"bar\"/>",
        "<sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\" xmlns:r=\"urn:x\"/>",
    ] {
        let out = read_with(split_pkg(&rels, sheet, ""), split_limits());
        refused(&out, &[LossReason::Damaged]);
    }
    // `state` is allowed.
    let hidden = "<sheet name=\"S\" sheetId=\"1\" state=\"hidden\" r:id=\"rId1\"/>";
    let out = read_with(split_pkg(&rels, hidden, ""), split_limits());
    assert_ne!(
        out.completeness,
        Completeness::Failed,
        "{:#?}",
        out.document.losses
    );
    assert_eq!(cell_text(&out.document, "S!A1").as_deref(), Some("1"));
}

/// A package with the given `<sheet>` elements, `rId1` naming a small sheet and `rId2` a
/// 1 MB sheet whose A1 reads BOMB.
fn named_pkg(sheet_elems: &str) -> Vec<u8> {
    let bomb = ws(
        "",
        &format!(
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>BOMB{}</t></is></c></row>",
            " ".repeat(1_000_000)
        ),
        "",
    );
    raw_zip(&[
        (
            "[Content_Types].xml",
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>"
                .into(),
        ),
        (
            "_rels/.rels",
            format!(
                "<Relationships xmlns=\"{PKG}\"><Relationship Id=\"rId1\" Type=\"{RELS}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
            ),
        ),
        (
            "xl/workbook.xml",
            format!(
                "<workbook xmlns=\"{MAIN}\" xmlns:r=\"{RELS}\"><sheets>{sheet_elems}</sheets></workbook>"
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                "<Relationships xmlns=\"{PKG}\">\
                 <Relationship Id=\"rId1\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
                 <Relationship Id=\"rId2\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet2.xml\"/>\
                 </Relationships>"
            ),
        ),
        ("xl/worksheets/sheet1.xml", small_sheet()),
        ("xl/worksheets/sheet2.xml", bomb),
    ])
}

fn named_limits() -> Limits {
    Limits {
        max_unpacked_bytes: 16_000_000,
        max_ratio: 100_000,
        ..Limits::default()
    }
}

/// A literal control character in one sheet name and its character reference in the
/// other: sheetparts turns the literal one into a space, calamine names both sheets alike
/// and finds the sheet by name.
fn control_twins(literal: &str, reference: &str) -> String {
    format!(
        "<sheet name=\"X{literal}Y\" sheetId=\"1\" r:id=\"rId2\"/>\
         <sheet name=\"X{reference}Y\" sheetId=\"2\" r:id=\"rId1\"/>"
    )
}

#[test]
fn sheet_names_with_a_literal_tab_are_refused() {
    // Probe N3.
    let out = read_with(named_pkg(&control_twins("\t", "&#9;")), named_limits());
    refused(&out, &[LossReason::Damaged]);
}

#[test]
fn sheet_names_with_a_literal_line_feed_are_refused() {
    // Probe N4.
    let out = read_with(named_pkg(&control_twins("\n", "&#10;")), named_limits());
    refused(&out, &[LossReason::Damaged]);
}

#[test]
fn sheet_names_with_a_literal_crlf_are_refused() {
    // Probe N9.
    let out = read_with(
        named_pkg(&control_twins("\r\n", "&#13;&#10;")),
        named_limits(),
    );
    refused(&out, &[LossReason::Damaged]);
}

#[test]
fn control_characters_in_a_sheet_name_are_refused() {
    for name in ["A&#1;B", "A&#x1F;B", "A&#127;B", "A\u{7f}B", "A&#X9;B"] {
        let sheet = format!("<sheet name=\"{name}\" sheetId=\"1\" r:id=\"rId1\"/>");
        let out = read_with(named_pkg(&sheet), named_limits());
        refused(&out, &[LossReason::Damaged]);
    }
    // Plain names with references to printable characters still read.
    let sheet = "<sheet name=\"A&#38;B &#x41;\" sheetId=\"1\" r:id=\"rId1\"/>";
    let out = read_with(named_pkg(sheet), named_limits());
    assert_ne!(
        out.completeness,
        Completeness::Failed,
        "{:#?}",
        out.document.losses
    );
}
