// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Workbook fixtures built in memory and helpers shared by the test files.

#![allow(
    dead_code,
    unused_imports,
    reason = "each test file uses a part of the helpers"
)]

pub use std::io::{Cursor, Write};
pub use std::sync::Arc;

pub use rezzer_limits::{Budget, Limits};
pub use rezzer_model::{
    Block, Bytes, Completeness, Confidence, Detection, Document, FormatId, Formula, FormulaKind,
    Inline, LossReason, ReadOptions, ReadOutcome, Reader, Source, Value, plain_text,
};
pub use rezzer_read_sheet::SheetReader;
pub use zip::CompressionMethod;
pub use zip::write::{SimpleFileOptions, ZipWriter};

pub const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
pub const RELS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// Cell styles of the fixtures: 0 General, 1 `0%`, 2 `mm-dd-yy`, 3 the locale-dependent
/// id 27, 4 `0.0" m"`, 5 `#,##0.00 [$€-407]`, 6 `"$"#,##0`, 7 `0.00" kN/m3"`, 8 `"Week "0`.
pub const STYLES: &str = "<numFmts count=\"5\"><numFmt numFmtId=\"164\" formatCode=\"0.0&quot; m&quot;\"/><numFmt numFmtId=\"165\" formatCode=\"#,##0.00 [$€-407]\"/><numFmt numFmtId=\"166\" formatCode=\"&quot;$&quot;#,##0\"/><numFmt numFmtId=\"167\" formatCode=\"0.00&quot; kN/m3&quot;\"/><numFmt numFmtId=\"168\" formatCode=\"&quot;Week &quot;0\"/></numFmts><cellXfs count=\"9\"><xf numFmtId=\"0\"/><xf numFmtId=\"9\"/><xf numFmtId=\"14\"/><xf numFmtId=\"27\"/><xf numFmtId=\"164\"/><xf numFmtId=\"165\"/><xf numFmtId=\"166\"/><xf numFmtId=\"167\"/><xf numFmtId=\"168\"/></cellXfs>";

/// One sheet of a fixture workbook.
pub struct Sheet {
    pub name: String,
    pub state: Option<&'static str>,
    pub xml: String,
    /// Related parts: (relationship type suffix, part name, content).
    pub related: Vec<(&'static str, String, String)>,
    /// The relationship target in place of `worksheets/sheet<n>.xml`.
    pub target: Option<String>,
    /// The relationship type in place of the full worksheet type.
    pub rel_type: Option<String>,
}

/// A small workbook built in memory.
#[derive(Default)]
pub struct Book {
    pub sheets: Vec<Sheet>,
    pub names: String,
    pub workbook_pr: String,
    pub calc_pr: String,
    /// More relationships in `_rels/.rels` and in the workbook relationships.
    pub root_rels_extra: String,
    pub wb_rels_extra: String,
    /// More entries, written after the others.
    pub extra: Vec<(String, String)>,
}

impl Book {
    pub fn sheet(mut self, name: &str, xml: &str) -> Book {
        self.sheets.push(Sheet {
            name: name.into(),
            state: None,
            xml: xml.into(),
            related: Vec::new(),
            target: None,
            rel_type: None,
        });
        self
    }

    pub fn target(mut self, target: &str) -> Book {
        self.sheets.last_mut().unwrap().target = Some(target.into());
        self
    }

    pub fn rel_type(mut self, kind: &str) -> Book {
        self.sheets.last_mut().unwrap().rel_type = Some(kind.into());
        self
    }

    pub fn entry(mut self, name: &str, content: String) -> Book {
        self.extra.push((name.into(), content));
        self
    }

    pub fn state(mut self, state: &'static str) -> Book {
        self.sheets.last_mut().unwrap().state = Some(state);
        self
    }

    pub fn related(mut self, kind: &'static str, part: &str, xml: &str) -> Book {
        self.sheets
            .last_mut()
            .unwrap()
            .related
            .push((kind, part.into(), xml.into()));
        self
    }

    pub fn names(mut self, names: &str) -> Book {
        self.names = names.into();
        self
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut files: Vec<(String, String)> = Vec::new();
        files.push((
            "[Content_Types].xml".into(),
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>"
                .into(),
        ));
        files.push((
            "_rels/.rels".into(),
            format!(
                "<Relationships xmlns=\"{PKG}\"><Relationship Id=\"rId1\" Type=\"{RELS}/officeDocument\" Target=\"xl/workbook.xml\"/>{}</Relationships>",
                self.root_rels_extra
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
            let target = s
                .target
                .clone()
                .unwrap_or_else(|| format!("worksheets/sheet{n}.xml"));
            let kind = s
                .rel_type
                .clone()
                .unwrap_or_else(|| format!("{RELS}/worksheet"));
            rels.push_str(&format!(
                "<Relationship Id=\"rId{n}\" Type=\"{kind}\" Target=\"{target}\"/>"
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
        rels.push_str(&self.wb_rels_extra);
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
        files.extend(self.extra.iter().cloned());
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
pub fn ws(before: &str, rows: &str, after: &str) -> String {
    format!(
        "<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{RELS}\">{before}<sheetData>{rows}</sheetData>{after}</worksheet>"
    )
}

pub fn source(bytes: Vec<u8>, name: &str, format: FormatId) -> Source {
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

pub fn read_with(bytes: Vec<u8>, limits: Limits) -> ReadOutcome {
    SheetReader.read(
        &source(bytes, "book.xlsx", FormatId::Xlsx),
        &ReadOptions::default(),
        &Budget::new(limits),
    )
}

pub fn read(book: &Book) -> ReadOutcome {
    read_with(book.bytes(), Limits::default())
}

pub fn corpus(name: &str) -> ReadOutcome {
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
pub fn all_blocks(doc: &Document) -> Vec<&Block> {
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
pub fn section<'a>(doc: &'a Document, title: &str) -> &'a [Block] {
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
pub fn cell_text(doc: &Document, anchor: &str) -> Option<String> {
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

pub fn formula<'a>(doc: &'a Document, anchor: &str) -> &'a Formula {
    doc.facets
        .formulas
        .iter()
        .find(|f| f.anchor.0 == anchor)
        .unwrap_or_else(|| panic!("no formula {anchor}: {:#?}", doc.facets.formulas))
}

pub fn refs(f: &Formula) -> Vec<&str> {
    f.references.iter().map(|a| a.0.as_str()).collect()
}

pub fn paragraphs(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph { inlines, .. } => Some(plain_text(inlines)),
            _ => None,
        })
        .collect()
}

pub fn has_loss(doc: &Document, why: LossReason, location: &str, text: &str) -> bool {
    doc.losses
        .iter()
        .any(|l| l.why == why && l.location == location && l.what.contains(text))
}
