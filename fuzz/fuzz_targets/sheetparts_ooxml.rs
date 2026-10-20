// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! sheetparts' OOXML readers on arbitrary part XML. Raw zip bytes mostly die at the CRC check,
//! so the input is cut at NUL bytes into the parts of a small package, stored without
//! compression; parts the input does not give get a valid default. An input that starts
//! with `PK` is used as the package itself. It must never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sheetparts::Limits;
use sheetparts::ooxml::{read_doc_props, read_sheet, read_workbook};

#[path = "common/package.rs"]
mod package;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn defaults() -> Vec<(&'static str, String)> {
    vec![
        ("xl/worksheets/sheet1.xml", format!("<worksheet xmlns=\"{MAIN}\"><sheetData/></worksheet>")),
        ("xl/workbook.xml", format!("<workbook xmlns=\"{MAIN}\" xmlns:r=\"{REL}\"><sheets><sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\"/></sheets><definedNames><definedName name=\"x\">S!$A$1</definedName></definedNames></workbook>")),
        ("xl/styles.xml", format!("<styleSheet xmlns=\"{MAIN}\"/>")),
        ("xl/worksheets/_rels/sheet1.xml.rels", format!("<Relationships xmlns=\"{PKG}\"><Relationship Id=\"c1\" Type=\"{REL}/comments\" Target=\"../comments1.xml\"/><Relationship Id=\"t1\" Type=\"http://schemas.microsoft.com/office/2017/10/relationships/threadedComment\" Target=\"../threadedComments/threadedComment1.xml\"/><Relationship Id=\"tb1\" Type=\"{REL}/table\" Target=\"../tables/table1.xml\"/></Relationships>")),
        ("xl/comments1.xml", format!("<comments xmlns=\"{MAIN}\"/>")),
        ("xl/threadedComments/threadedComment1.xml", "<ThreadedComments xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments\"/>".to_string()),
        ("xl/tables/table1.xml", format!("<table xmlns=\"{MAIN}\" name=\"T\" ref=\"A1:B2\"/>")),
        ("xl/persons/person.xml", "<personList xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments\"/>".to_string()),
        ("xl/_rels/workbook.xml.rels", format!("<Relationships xmlns=\"{PKG}\"><Relationship Id=\"rId1\" Type=\"{REL}/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"{REL}/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.microsoft.com/office/2017/10/relationships/person\" Target=\"persons/person.xml\"/></Relationships>")),
        ("docProps/core.xml", "<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\"/>".to_string()),
        ("docProps/custom.xml", "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/custom-properties\"/>".to_string()),
        ("_rels/.rels", format!("<Relationships xmlns=\"{PKG}\"><Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>")),
        ("[Content_Types].xml", "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>".to_string()),
    ]
}

fuzz_target!(|data: &[u8]| {
    let bytes = package::build(data, &defaults());
    let Ok(mut zip) = zipguard::Archive::open(&bytes, package::ZIP_LIMITS) else {
        return;
    };
    let limits = Limits {
        max_depth: 64,
        max_items: 100_000,
    };
    let stop = || false;
    let wb = read_workbook(&mut zip, &limits, &stop);
    for sheet in wb.sheets.iter().take(8) {
        let _ = read_sheet(&mut zip, sheet, &wb, &limits, &stop);
    }
    for i in 0..wb.styles.len().min(64) {
        let _ = wb.styles.number_format(i as u32);
    }
    let _ = read_doc_props(&mut zip, &limits, &stop);
});
