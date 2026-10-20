// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::{Cursor, Write};
use std::sync::Arc;

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Block, Bytes, Completeness, Confidence, Detection, Document, FormatId, Formula, FormulaKind,
    Inline, LossReason, ReadOptions, ReadOutcome, Reader, Source, Value, plain_text,
};
use rezzer_read_sheet::SheetReader;
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// Cell styles of the fixtures: 0 General, 1 `0%`, 2 `mm-dd-yy`, 3 the locale-dependent
/// id 27, 4 `0.0" m"`.
const STYLES: &str = "<numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"0.0&quot; m&quot;\"/></numFmts><cellXfs count=\"5\"><xf numFmtId=\"0\"/><xf numFmtId=\"9\"/><xf numFmtId=\"14\"/><xf numFmtId=\"27\"/><xf numFmtId=\"164\"/></cellXfs>";

/// One sheet of a fixture workbook.
struct Sheet {
    name: String,
    state: Option<&'static str>,
    xml: String,
    /// Related parts: (relationship type suffix, part name, content).
    related: Vec<(&'static str, String, String)>,
}

/// A small workbook built in memory.
#[derive(Default)]
struct Book {
    sheets: Vec<Sheet>,
    names: String,
    workbook_pr: String,
    calc_pr: String,
}

impl Book {
    fn sheet(mut self, name: &str, xml: &str) -> Book {
        self.sheets.push(Sheet {
            name: name.into(),
            state: None,
            xml: xml.into(),
            related: Vec::new(),
        });
        self
    }

    fn state(mut self, state: &'static str) -> Book {
        self.sheets.last_mut().unwrap().state = Some(state);
        self
    }

    fn related(mut self, kind: &'static str, part: &str, xml: &str) -> Book {
        self.sheets
            .last_mut()
            .unwrap()
            .related
            .push((kind, part.into(), xml.into()));
        self
    }

    fn names(mut self, names: &str) -> Book {
        self.names = names.into();
        self
    }

    fn bytes(&self) -> Vec<u8> {
        let mut files: Vec<(String, String)> = Vec::new();
        files.push((
            "[Content_Types].xml".into(),
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>"
                .into(),
        ));
        files.push((
            "_rels/.rels".into(),
            format!(
                "<Relationships xmlns=\"{PKG}\"><Relationship Id=\"rId1\" Type=\"{RELS}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
            ),
        ));
        let mut sheets = String::new();
        let mut rels = String::new();
        for (i, s) in self.sheets.iter().enumerate() {
            let n = i + 1;
            let state = s
                .state
                .map(|v| format!(" state=\"{v}\""))
                .unwrap_or_default();
            sheets.push_str(&format!(
                "<sheet name=\"{}\" sheetId=\"{n}\"{state} r:id=\"rId{n}\"/>",
                s.name
            ));
            rels.push_str(&format!(
                "<Relationship Id=\"rId{n}\" Type=\"{RELS}/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>"
            ));
            files.push((format!("xl/worksheets/sheet{n}.xml"), s.xml.clone()));
            if !s.related.is_empty() {
                let mut srels = String::new();
                for (j, (kind, part, xml)) in s.related.iter().enumerate() {
                    srels.push_str(&format!(
                        "<Relationship Id=\"rIdx{j}\" Type=\"{RELS}/{kind}\" Target=\"/{part}\"/>"
                    ));
                    files.push((part.clone(), xml.clone()));
                }
                files.push((
                    format!("xl/worksheets/_rels/sheet{n}.xml.rels"),
                    format!("<Relationships xmlns=\"{PKG}\">{srels}</Relationships>"),
                ));
            }
        }
        rels.push_str(&format!(
            "<Relationship Id=\"rIdS\" Type=\"{RELS}/styles\" Target=\"styles.xml\"/>"
        ));
        let names = if self.names.is_empty() {
            String::new()
        } else {
            format!("<definedNames>{}</definedNames>", self.names)
        };
        files.push((
            "xl/workbook.xml".into(),
            format!(
                "<workbook xmlns=\"{MAIN}\" xmlns:r=\"{RELS}\"><workbookPr {}/><sheets>{sheets}</sheets>{names}{}</workbook>",
                self.workbook_pr, self.calc_pr
            ),
        ));
        files.push((
            "xl/_rels/workbook.xml.rels".into(),
            format!("<Relationships xmlns=\"{PKG}\">{rels}</Relationships>"),
        ));
        files.push((
            "xl/styles.xml".into(),
            format!("<styleSheet xmlns=\"{MAIN}\">{STYLES}</styleSheet>"),
        ));
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, content) in files {
            w.start_file(name, opts).unwrap();
            w.write_all(content.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }
}

/// A worksheet with these rows (and anything before or after `sheetData`).
fn ws(before: &str, rows: &str, after: &str) -> String {
    format!(
        "<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{RELS}\">{before}<sheetData>{rows}</sheetData>{after}</worksheet>"
    )
}

fn source(bytes: Vec<u8>, name: &str, format: FormatId) -> Source {
    Source {
        path: None,
        name: name.into(),
        size: bytes.len() as u64,
        bytes: Bytes::Owned(Arc::new(bytes)),
        sha256: String::new(),
        format,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "test".into(),
        },
        modified: None,
    }
}

fn read_with(bytes: Vec<u8>, limits: Limits) -> ReadOutcome {
    SheetReader.read(
        &source(bytes, "book.xlsx", FormatId::Xlsx),
        &ReadOptions::default(),
        &Budget::new(limits),
    )
}

fn read(book: &Book) -> ReadOutcome {
    read_with(book.bytes(), Limits::default())
}

fn corpus(name: &str) -> ReadOutcome {
    let path = format!(
        "{}/../../../tests/corpus/office/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let format = if name.ends_with(".xlsm") {
        FormatId::Xlsm
    } else {
        FormatId::Xlsx
    };
    SheetReader.read(
        &source(bytes, name, format),
        &ReadOptions::default(),
        &Budget::new(Limits::default()),
    )
}

/// Every block in the body, sections walked.
fn all_blocks(doc: &Document) -> Vec<&Block> {
    let mut out = Vec::new();
    let mut stack: Vec<&Block> = doc.body.iter().rev().collect();
    while let Some(b) = stack.pop() {
        out.push(b);
        if let Block::Section { blocks, .. } = b {
            stack.extend(blocks.iter().rev());
        }
    }
    out
}

/// The section of a sheet.
fn section<'a>(doc: &'a Document, title: &str) -> &'a [Block] {
    doc.body
        .iter()
        .find_map(|b| match b {
            Block::Section {
                title: Some(t),
                blocks,
                ..
            } if t == title => Some(blocks.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no section {title}"))
}

/// The text of the value table cell with this anchor.
fn cell_text(doc: &Document, anchor: &str) -> Option<String> {
    all_blocks(doc).into_iter().find_map(|b| match b {
        Block::Table {
            rows, role: None, ..
        } => rows
            .iter()
            .flatten()
            .find(|c| c.anchor.as_ref().is_some_and(|a| a.0 == anchor))
            .map(|c| plain_text(&c.inlines)),
        _ => None,
    })
}

fn formula<'a>(doc: &'a Document, anchor: &str) -> &'a Formula {
    doc.facets
        .formulas
        .iter()
        .find(|f| f.anchor.0 == anchor)
        .unwrap_or_else(|| panic!("no formula {anchor}: {:#?}", doc.facets.formulas))
}

fn refs(f: &Formula) -> Vec<&str> {
    f.references.iter().map(|a| a.0.as_str()).collect()
}

fn paragraphs(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph { inlines, .. } => Some(plain_text(inlines)),
            _ => None,
        })
        .collect()
}

fn has_loss(doc: &Document, why: LossReason, location: &str, text: &str) -> bool {
    doc.losses
        .iter()
        .any(|l| l.why == why && l.location == location && l.what.contains(text))
}

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
    // Names keep their text as written.
    assert_eq!(twice.expression, "_xlfn.LAMBDA(_xlpm.x,_xlpm.x*2)");
    assert_eq!(twice.original, None);
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
    let mut rows = String::new();
    for r in 1..=5_000 {
        rows.push_str(&format!(
            "<row r=\"{r}\"><c r=\"A{r}\"><v>{r}</v></c></row>"
        ));
    }
    let big = ws("", &rows, "");
    let small = ws("", "<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>", "");
    let limits = Limits {
        max_unpacked_bytes: 100_000,
        max_ratio: 100_000,
        ..Limits::default()
    };
    let out = read_with(
        Book::default()
            .sheet("Big", &big)
            .sheet("Small", &small)
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
