// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::{Cursor, Write};

use sheetparts::ooxml::{CalcMode, SheetState, WorkbookParts, read_workbook};
use sheetparts::{IssueKind, Limits, PropValue};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};
use zipguard::Archive;

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn zip_of(entries: &[(&str, String)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file(*name, options).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn rels(items: &[(&str, &str, &str)]) -> String {
    let mut out = format!("<Relationships xmlns=\"{PKG_REL}\">");
    for (id, kind, target) in items {
        out.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    out.push_str("</Relationships>");
    out
}

fn office_rel(kind: &str) -> String {
    format!("{REL_NS}/{kind}")
}

fn workbook_xml(inner: &str) -> String {
    format!("<workbook xmlns=\"{NS}\" xmlns:r=\"{REL_NS}\">{inner}</workbook>")
}

/// A minimal package: root rels, workbook, workbook rels and any extra parts.
fn package(
    workbook_inner: &str,
    extra_rels: &[(&str, &str, &str)],
    extra: &[(&str, String)],
) -> Vec<u8> {
    let mut list: Vec<(&str, String)> = vec![
        (
            "_rels/.rels",
            rels(&[("rId1", &office_rel("officeDocument"), "xl/workbook.xml")]),
        ),
        ("xl/workbook.xml", workbook_xml(workbook_inner)),
        ("xl/_rels/workbook.xml.rels", rels(extra_rels)),
    ];
    for (name, data) in extra {
        list.push((name, data.clone()));
    }
    zip_of(&list)
}

fn read(bytes: &[u8]) -> WorkbookParts {
    let mut archive = Archive::open(bytes, zipguard::Limits::default()).unwrap();
    read_workbook(&mut archive, &Limits::default(), &|| false)
}

fn read_file(path: &str) -> WorkbookParts {
    let full = format!(
        "{}/../../../tests/corpus/office/{path}",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&full).unwrap_or_else(|e| panic!("corpus file {full}: {e}"));
    read(&bytes)
}

#[test]
fn sheets_with_state_and_part() {
    let ws = office_rel("worksheet");
    let bytes = package(
        "<sheets><sheet name=\"A\" sheetId=\"1\" r:id=\"rId5\"/>\
         <sheet name=\"B &amp; C\" sheetId=\"2\" state=\"hidden\" r:id=\"rId2\"/>\
         <sheet name=\"D\" sheetId=\"3\" state=\"veryHidden\" r:id=\"rId3\"/></sheets>",
        &[
            ("rId5", &ws, "worksheets/sheet1.xml"),
            ("rId2", &ws, "/xl/worksheets/sheet2.xml"),
            ("rId3", &ws, "../xl/worksheets/sheet3.xml"),
        ],
        &[],
    );
    let parts = read(&bytes);
    assert_eq!(parts.sheets.len(), 3);
    assert_eq!(parts.sheets[0].name, "A");
    assert_eq!(parts.sheets[0].state, SheetState::Visible);
    assert_eq!(parts.sheets[0].part, "xl/worksheets/sheet1.xml");
    assert_eq!(parts.sheets[1].name, "B & C");
    assert_eq!(parts.sheets[1].state, SheetState::Hidden);
    assert_eq!(parts.sheets[1].part, "xl/worksheets/sheet2.xml");
    assert_eq!(parts.sheets[2].state, SheetState::VeryHidden);
    assert_eq!(parts.sheets[2].part, "xl/worksheets/sheet3.xml");
    assert_eq!(parts.workbook_part, "xl/workbook.xml");
}

#[test]
fn sheet_states_from_a_real_file() {
    let parts = read_file("hidden_sheets.xlsx");
    let states: Vec<_> = parts
        .sheets
        .iter()
        .map(|s| (s.name.as_str(), s.state))
        .collect();
    assert_eq!(
        states,
        vec![
            ("Sheet1", SheetState::Hidden),
            ("Sheet2", SheetState::Visible),
            ("Sheet3", SheetState::Hidden)
        ]
    );
    assert_eq!(parts.sheets[0].part, "xl/worksheets/sheet1.xml");
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
}

#[test]
fn names_with_scope_and_hidden() {
    let ws = office_rel("worksheet");
    let bytes = package(
        "<sheets><sheet name=\"One\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"Two\" sheetId=\"2\" r:id=\"rId2\"/></sheets>\
         <definedNames>\
         <definedName name=\"Global\" comment=\"the rate\">Two!$A$1</definedName>\
         <definedName name=\"Local\" localSheetId=\"1\">Two!$B$2:$C$3</definedName>\
         <definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"0\" hidden=\"1\">One!$A$1:$D$9</definedName>\
         <definedName name=\"Lost\" localSheetId=\"9\">1+1</definedName>\
         </definedNames>",
        &[
            ("rId1", &ws, "worksheets/sheet1.xml"),
            ("rId2", &ws, "worksheets/sheet2.xml"),
        ],
        &[],
    );
    let parts = read(&bytes);
    assert_eq!(parts.names.len(), 4);
    let global = &parts.names[0];
    assert_eq!(global.name, "Global");
    assert_eq!(global.scope, None);
    assert_eq!(global.refers_to, "Two!$A$1");
    assert_eq!(global.comment.as_deref(), Some("the rate"));
    assert!(!global.hidden);
    assert_eq!(parts.names[1].scope.as_deref(), Some("Two"));
    assert!(parts.names[2].hidden);
    assert_eq!(parts.names[2].scope.as_deref(), Some("One"));
    // A localSheetId without a sheet is a problem, not a panic.
    assert_eq!(parts.names[3].scope, None);
    assert!(
        parts
            .issues
            .iter()
            .any(|i| i.part == "xl/workbook.xml" && i.kind == IssueKind::Damaged)
    );
}

#[test]
fn array_constant_name_is_kept_whole() {
    let parts = read_file("testNames.xlsm");
    let array = parts.names.iter().find(|n| n.name == "n_array").unwrap();
    assert_eq!(array.refers_to, "{\"A\",\"CC\",\"BBB\"}");
    let offset = parts.names.iter().find(|n| n.name == "nr_OFFSET").unwrap();
    assert_eq!(offset.refers_to, "OFFSET(Sheet1!$A$1, 3, Sheet1!$A$2)");
    assert_eq!(parts.names.len(), 5);
}

#[test]
fn macro_workbook_reports_vba() {
    let parts = read_file("testNames.xlsm");
    assert!(parts.has_vba);
    assert!(!read_file("hidden_sheets.xlsx").has_vba);
}

#[test]
fn calc_manual_and_iterate() {
    let bytes = package(
        "<calcPr calcMode=\"manual\" iterate=\"1\" iterateCount=\"50\" iterateDelta=\"0.01\" fullCalcOnLoad=\"1\"/>",
        &[],
        &[],
    );
    let calc = read(&bytes).calc;
    assert_eq!(calc.mode, CalcMode::Manual);
    assert!(calc.iterate);
    assert_eq!(calc.iterate_count, Some(50));
    assert_eq!(calc.iterate_delta, Some(0.01));
    assert!(calc.full_calc_on_load);

    let none = read(&package("<sheets/>", &[], &[])).calc;
    assert_eq!(none.mode, CalcMode::Auto);
    assert!(!none.iterate);
    assert_eq!(none.iterate_count, None);

    let table = read(&package("<calcPr calcMode=\"autoNoTable\"/>", &[], &[])).calc;
    assert_eq!(table.mode, CalcMode::AutoNoTable);

    let real = read_file("hidden_sheets.xlsx").calc;
    assert!(!real.iterate);
    assert_eq!(real.iterate_count, Some(100));
    assert_eq!(real.iterate_delta, Some(0.001));
}

#[test]
fn external_link_target_and_addin() {
    let parts = read_file("external-refs.xlsx");
    assert_eq!(parts.external_links.len(), 1);
    assert_eq!(parts.external_links[0].index, 1);
    assert_eq!(parts.external_links[0].target, "data.xlsx");
    assert_eq!(parts.external_links[0].kind, "workbook");

    let ext = office_rel("externalLink");
    let path = office_rel("externalLinkPath");
    let addin = format!("{REL_NS}/xlExternalLinkPath/xlLibrary");
    let link = |inner: &str| {
        format!("<externalLink xmlns=\"{NS}\" xmlns:r=\"{REL_NS}\">{inner}</externalLink>")
    };
    let bytes = package(
        "<externalReferences><externalReference r:id=\"rId1\"/><externalReference r:id=\"rId2\"/>\
         <externalReference r:id=\"rId3\"/><externalReference r:id=\"rId4\"/></externalReferences>",
        &[
            ("rId1", &ext, "externalLinks/externalLink1.xml"),
            ("rId2", &ext, "externalLinks/externalLink2.xml"),
            ("rId3", &ext, "externalLinks/externalLink3.xml"),
            ("rId4", &ext, "externalLinks/externalLink4.xml"),
        ],
        &[
            (
                "xl/externalLinks/externalLink1.xml",
                link("<externalBook r:id=\"rId1\"/>"),
            ),
            (
                "xl/externalLinks/externalLink2.xml",
                link("<externalBook r:id=\"rId1\"/>"),
            ),
            (
                "xl/externalLinks/externalLink3.xml",
                link("<ddeLink ddeService=\"excel\" ddeTopic=\"book.xlsx\"/>"),
            ),
            (
                "xl/externalLinks/externalLink4.xml",
                link("<oleLink r:id=\"rId1\" progId=\"Word.Document.12\"/>"),
            ),
            (
                "xl/externalLinks/_rels/externalLink1.xml.rels",
                format!(
                    "<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{path}\" Target=\"file:///C:/Budget%202020.xlsx\" TargetMode=\"External\"/></Relationships>"
                ),
            ),
            (
                "xl/externalLinks/_rels/externalLink2.xml.rels",
                format!(
                    "<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{addin}\" Target=\"ANALYS32.XLL\" TargetMode=\"External\"/></Relationships>"
                ),
            ),
            (
                "xl/externalLinks/_rels/externalLink4.xml.rels",
                format!(
                    "<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{REL_NS}/oleObject\" Target=\"report.docx\" TargetMode=\"External\"/></Relationships>"
                ),
            ),
        ],
    );
    let links = read(&bytes).external_links;
    let got: Vec<_> = links
        .iter()
        .map(|l| (l.index, l.kind.as_str(), l.target.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            (1, "workbook", "file:///C:/Budget%202020.xlsx"),
            (2, "add-in", "ANALYS32.XLL"),
            (3, "dde", "excel|book.xlsx"),
            (4, "ole", "report.docx"),
        ]
    );
}

#[test]
fn custom_props_typed() {
    let parts = read_file("testEXCEL_custom_props.xlsx");
    let custom: std::collections::BTreeMap<_, _> = parts.props.custom.iter().cloned().collect();
    assert_eq!(
        custom["MyCustomString"],
        PropValue::Text("MyStringValue".into())
    );
    assert_eq!(custom["myCustomNumber"], PropValue::Integer(3));
    assert_eq!(custom["myCustomBoolean"], PropValue::Bool(true));
    assert_eq!(
        custom["MyCustomDate"],
        PropValue::DateTime("2010-12-30T22:00:00Z".into())
    );
    assert_eq!(parts.props.custom.len(), 5);
    assert_eq!(parts.props.core["created"], "2006-09-12T15:06:44Z");
    assert!(
        !parts.props.core.contains_key("creator"),
        "empty values are left out"
    );
    assert_eq!(parts.props.app["Application"], "Microsoft Excel");

    let bytes = package(
        "",
        &[],
        &[
            (
                "docProps/custom.xml",
                "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/custom-properties\" xmlns:vt=\"x\">\
                 <property pid=\"2\" name=\"Ratio\"><vt:r8>0.25</vt:r8></property>\
                 <property pid=\"3\" name=\"Blob\"><vt:blob>AAAA</vt:blob></property></Properties>"
                    .to_string(),
            ),
        ],
    );
    let props = read(&bytes).props;
    assert_eq!(
        props.custom[0],
        ("Ratio".to_string(), PropValue::Number(0.25))
    );
    assert_eq!(
        props.custom[1],
        (
            "Blob".to_string(),
            PropValue::Other {
                ty: "blob".into(),
                value: "AAAA".into()
            }
        )
    );
}

#[test]
fn persons_from_person_part() {
    let parts = read_file("threadedComment.xlsx");
    assert_eq!(parts.persons.len(), 1);
    assert_eq!(parts.persons[0].name, "Mike Kaganski");
    assert_eq!(
        parts.persons[0].id,
        "{4A73A60B-EAFA-41CF-BAF3-B061059D7A5C}"
    );
}

#[test]
fn styles_resolve_number_formats() {
    let parts = read_file("Formatting.xlsx");
    let styles = &parts.styles;
    assert_eq!(styles.number_format(0), Some("General"));
    assert_eq!(styles.number_format(1), Some("mm-dd-yy"));
    assert_eq!(styles.number_format(3), Some("yyyy/mm/dd"));
    assert_eq!(styles.number_format(10_000), None);
    assert!(
        (0..styles.len() as u32)
            .filter_map(|i| styles.number_format(i))
            .any(|c| c.contains('\u{a3}')),
        "the pound format is used by a cell style"
    );

    let bytes = package(
        "",
        &[("rId1", &office_rel("styles"), "styles.xml")],
        &[(
            "xl/styles.xml",
            format!(
                "<styleSheet xmlns=\"{NS}\"><numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"0.0&quot; m&quot;\"/></numFmts>\
                 <cellStyleXfs count=\"1\"><xf numFmtId=\"9\"/></cellStyleXfs>\
                 <dxfs count=\"1\"><dxf><numFmt numFmtId=\"164\" formatCode=\"0.000000\"/></dxf></dxfs>\
                 <cellXfs count=\"3\"><xf numFmtId=\"164\"/><xf numFmtId=\"10\"/><xf numFmtId=\"200\"/></cellXfs></styleSheet>"
            ),
        )],
    );
    let styles = read(&bytes).styles;
    assert_eq!(styles.number_format(0), Some("0.0\" m\""));
    assert_eq!(styles.number_format(1), Some("0.00%"));
    // An id that is neither built in nor defined has no format, but its id is known.
    assert_eq!(styles.number_format(2), None);
    assert_eq!(styles.format_id(2), Some(200));
    assert_eq!(styles.format_id(0), Some(164));
    assert_eq!(styles.format_id(3), None);
}

#[test]
fn date_system_is_read() {
    let parts = read(&package("<workbookPr date1904=\"1\"/>", &[], &[]));
    assert!(parts.date1904);
    assert!(!read_file("hidden_sheets.xlsx").date1904);
}

#[test]
fn doctype_is_refused() {
    let evil = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE workbook [<!ENTITY a \"aaaa\"><!ENTITY b \"&a;&a;&a;&a;\">]>\
         <workbook xmlns=\"{NS}\"><sheets><sheet name=\"&b;\" sheetId=\"1\"/></sheets></workbook>"
    );
    let bytes = zip_of(&[
        (
            "_rels/.rels",
            rels(&[("rId1", &office_rel("officeDocument"), "xl/workbook.xml")]),
        ),
        ("xl/workbook.xml", evil),
    ]);
    let parts = read(&bytes);
    assert!(parts.sheets.is_empty(), "nothing is read after a DOCTYPE");
    assert!(
        parts.issues.iter().any(|i| i.part == "xl/workbook.xml"
            && i.kind == IssueKind::Unsupported
            && i.what.contains("DOCTYPE")),
        "{:?}",
        parts.issues
    );
}

#[test]
fn deep_nesting_stops_at_limit() {
    let depth = 5_000;
    let deep = format!(
        "<workbook xmlns=\"{NS}\">{}{}</workbook>",
        "<a>".repeat(depth),
        "</a>".repeat(depth)
    );
    let bytes = zip_of(&[
        (
            "_rels/.rels",
            rels(&[("rId1", &office_rel("officeDocument"), "xl/workbook.xml")]),
        ),
        ("xl/workbook.xml", deep),
    ]);
    // Repeated tags compress far beyond the default ratio limit of zipguard.
    let zip_limits = zipguard::Limits {
        max_ratio: 1_000_000,
        ..zipguard::Limits::default()
    };
    let mut archive = Archive::open(&bytes, zip_limits).unwrap();
    let parts = read_workbook(&mut archive, &Limits::default(), &|| false);
    assert!(
        parts
            .issues
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.what.contains("nesting")),
        "{:?}",
        parts.issues
    );

    // With the default ratio limit the part is refused by zipguard, and that is an issue too.
    let parts = read(&bytes);
    assert!(
        parts.issues.iter().any(|i| i.kind == IssueKind::Limit),
        "{:?}",
        parts.issues
    );
}

#[test]
fn item_limit_and_stop_callback_end_a_part() {
    let many = format!(
        "<sheets>{}</sheets>",
        "<sheet name=\"s\" sheetId=\"1\"/>".repeat(100)
    );
    let bytes = package(&many, &[], &[]);
    let mut archive = Archive::open(&bytes, zipguard::Limits::default()).unwrap();
    let limits = Limits {
        max_depth: 256,
        max_items: 50,
    };
    let parts = read_workbook(&mut archive, &limits, &|| false);
    assert!(
        parts
            .issues
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.what.contains("items"))
    );
    assert!(parts.sheets.len() < 100);

    let mut archive = Archive::open(&bytes, zipguard::Limits::default()).unwrap();
    let parts = read_workbook(&mut archive, &Limits::default(), &|| true);
    assert!(
        parts
            .issues
            .iter()
            .any(|i| i.kind == IssueKind::Limit && i.what.contains("stopped"))
    );
}

#[test]
fn broken_xml_and_missing_workbook_become_issues() {
    let bytes = zip_of(&[("readme.txt", "no workbook here".to_string())]);
    let parts = read(&bytes);
    assert!(parts.sheets.is_empty());
    assert!(!parts.issues.is_empty());

    let bytes = package("<sheets><sheet name=\"a\"", &[], &[]);
    let parts = read(&bytes);
    assert!(
        parts
            .issues
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.part == "xl/workbook.xml")
    );
}

#[test]
fn every_corpus_workbook_reads_without_panic() {
    let dir = format!(
        "{}/../../../tests/corpus/office",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "xlsx" | "xlsm") {
            continue;
        }
        let parts = read(&std::fs::read(&path).unwrap());
        assert!(!parts.sheets.is_empty(), "{path:?}");
        assert!(parts.issues.is_empty(), "{path:?}: {:?}", parts.issues);
        seen += 1;
    }
    assert!(seen >= 20, "{seen}");
}

fn styles_from(inner: &str) -> sheetparts::ooxml::Styles {
    let bytes = package(
        "",
        &[("rId1", &office_rel("styles"), "styles.xml")],
        &[(
            "xl/styles.xml",
            format!("<styleSheet xmlns=\"{NS}\">{inner}</styleSheet>"),
        )],
    );
    read(&bytes).styles
}

fn xfs(list: &[&str]) -> String {
    format!(
        "<cellXfs count=\"{}\">{}</cellXfs>",
        list.len(),
        list.concat()
    )
}

#[test]
fn bold_from_named_style_counts() {
    let styles = styles_from(&format!(
        "<fonts count=\"3\"><font><sz val=\"11\"/></font><font><b/></font><font><b val=\"0\"/></font></fonts>\
         <fills count=\"1\"><fill><patternFill patternType=\"none\"/></fill></fills>\
         <borders count=\"1\"><border><left/><right/><top/><bottom/></border></borders>\
         <cellStyleXfs count=\"2\"><xf fontId=\"0\"/><xf fontId=\"1\"/></cellStyleXfs>{}",
        xfs(&[
            "<xf xfId=\"0\" fontId=\"0\"/>",
            "<xf xfId=\"1\"/>",
            "<xf xfId=\"1\" fontId=\"2\"/>",
            "<xf/>",
            "<xf fontId=\"1\"/>",
            "<xf fontId=\"9\"/>",
        ])
    ));
    assert!(styles.has_looks());
    let bold: Vec<bool> = (0..6).map(|i| styles.look(i).unwrap().bold).collect();
    // A named bold style counts; a cell font with b val=0 over a bold style is not bold.
    assert_eq!(bold, vec![false, true, false, false, true, false]);
    assert_eq!(styles.look(6), None);
}

#[test]
fn bold_val_false_is_not_bold() {
    let styles = styles_from(&format!(
        "<fonts count=\"2\"><font><b val=\"false\"/></font><font><b val=\"1\"/></font></fonts>\
         <fills count=\"0\"/><borders count=\"0\"/>{}",
        xfs(&["<xf fontId=\"0\"/>", "<xf fontId=\"1\"/>"])
    ));
    assert!(!styles.look(0).unwrap().bold);
    assert!(styles.look(1).unwrap().bold);
}

#[test]
fn white_solid_and_gray125_are_not_filled() {
    let fill = |p: &str, c: &str| {
        format!("<fill><patternFill patternType=\"{p}\">{c}</patternFill></fill>")
    };
    let fills = [
        "<fill><patternFill patternType=\"none\"/></fill>".to_string(),
        "<fill><patternFill patternType=\"gray125\"/></fill>".to_string(),
        fill("solid", "<fgColor rgb=\"FFFFFFFF\"/>"),
        fill("solid", "<fgColor theme=\"0\"/>"),
        fill("solid", "<fgColor indexed=\"64\"/>"),
        fill("solid", "<fgColor auto=\"1\"/>"),
        fill("solid", "<fgColor indexed=\"9\"/>"),
        fill("solid", "<fgColor rgb=\"FFFFFF00\"/>"),
        fill("lightGray", "<fgColor rgb=\"FFFFFF00\"/>"),
        fill("solid", "<fgColor theme=\"4\"/>"),
        fill("solid", ""),
    ];
    let list: Vec<String> = (0..fills.len())
        .map(|i| format!("<xf fillId=\"{i}\"/>"))
        .collect();
    let refs: Vec<&str> = list.iter().map(String::as_str).collect();
    let styles = styles_from(&format!(
        "<fonts count=\"1\"><font/></fonts><fills count=\"{}\">{}</fills><borders count=\"0\"/>{}",
        fills.len(),
        fills.concat(),
        xfs(&refs)
    ));
    let filled: Vec<bool> = (0..fills.len() as u32)
        .map(|i| styles.look(i).unwrap().filled)
        .collect();
    assert_eq!(
        filled,
        vec![
            false, false, false, false, false, false, false, true, false, true, false
        ]
    );
}

#[test]
fn hairline_is_no_rule() {
    let border = |top: &str, bottom: &str| {
        let side = |n: &str, s: &str| {
            if s.is_empty() {
                format!("<{n}/>")
            } else {
                format!("<{n} style=\"{s}\"><color auto=\"1\"/></{n}>")
            }
        };
        format!(
            "<border><left/><right/>{}{}</border>",
            side("top", top),
            side("bottom", bottom)
        )
    };
    let borders = [
        border("", ""),
        border("hair", "dotted"),
        border("dashed", "dashDot"),
        border("thin", ""),
        border("", "medium"),
        border("thick", "double"),
    ];
    let list: Vec<String> = (0..borders.len())
        .map(|i| format!("<xf borderId=\"{i}\"/>"))
        .collect();
    let refs: Vec<&str> = list.iter().map(String::as_str).collect();
    let styles = styles_from(&format!(
        "<fonts count=\"1\"><font/></fonts><fills count=\"0\"/><borders count=\"{}\">{}</borders>{}",
        borders.len(),
        borders.concat(),
        xfs(&refs)
    ));
    let rules: Vec<(bool, bool)> = (0..borders.len() as u32)
        .map(|i| {
            let l = styles.look(i).unwrap();
            (l.rule_above, l.rule_below)
        })
        .collect();
    assert_eq!(
        rules,
        vec![
            (false, false),
            (false, false),
            (false, false),
            (true, false),
            (false, true),
            (true, true)
        ]
    );
}

#[test]
fn indent_and_center_across() {
    let styles = styles_from(&format!(
        "<fonts count=\"1\"><font/></fonts><fills count=\"0\"/><borders count=\"0\"/>{}",
        xfs(&[
            "<xf/>",
            "<xf><alignment indent=\"2\"/></xf>",
            "<xf><alignment horizontal=\"centerContinuous\"/></xf>",
            "<xf><alignment horizontal=\"center\" indent=\"999\"/></xf>",
        ])
    ));
    let l: Vec<_> = (0..4).map(|i| styles.look(i).unwrap()).collect();
    assert_eq!((l[0].indent, l[0].center_across), (0, false));
    assert_eq!((l[1].indent, l[1].center_across), (2, false));
    assert_eq!((l[2].indent, l[2].center_across), (0, true));
    // Plain centering is not center across selection; a huge indent is clamped.
    assert_eq!((l[3].indent, l[3].center_across), (255, false));
}

#[test]
fn missing_styles_part_has_no_looks() {
    let none = read(&package("", &[], &[])).styles;
    assert!(!none.has_looks());
    assert_eq!(none.look(0), None);
    // A part without fills is a missing signal too.
    let partial = styles_from(&format!(
        "<fonts count=\"1\"><font><b/></font></fonts><borders count=\"0\"/>{}",
        xfs(&["<xf/>"])
    ));
    assert!(!partial.has_looks());
    // A numFmt-only styles part keeps its number formats.
    let only = styles_from(&xfs(&["<xf numFmtId=\"10\"/>"]));
    assert!(!only.has_looks());
    assert_eq!(only.number_format(0), Some("0.00%"));
}

#[test]
fn print_title_rows_by_scope() {
    use sheetparts::ooxml::print_title_rows;
    use sheetparts::{DefinedName, RowSpan};
    let name = |n: &str, scope: Option<&str>, to: &str| DefinedName {
        name: n.to_string(),
        scope: scope.map(str::to_string),
        refers_to: to.to_string(),
        hidden: false,
        comment: None,
    };
    let names = vec![
        name("_xlnm.Print_Titles", Some("A"), "A!$1:$2"),
        name(
            "_xlnm.Print_Titles",
            Some("My Sheet!"),
            "'My Sheet!'!$A:$B,'My Sheet!'!$3:$3",
        ),
        name("Print_Titles", Some("C"), "C!$5:$7"),
        name("_xlnm.Print_Titles", Some("D"), "D!$A:$C"),
        name("_xlnm.Print_Titles", None, "A!$9:$9"),
        name("_xlnm.Print_Area", Some("E"), "E!$1:$2"),
    ];
    assert_eq!(
        print_title_rows(&names, "A"),
        Some(RowSpan { first: 0, last: 1 })
    );
    assert_eq!(
        print_title_rows(&names, "My Sheet!"),
        Some(RowSpan { first: 2, last: 2 })
    );
    assert_eq!(
        print_title_rows(&names, "C"),
        Some(RowSpan { first: 4, last: 6 })
    );
    // Columns only, a global name and another name give nothing.
    assert_eq!(print_title_rows(&names, "D"), None);
    assert_eq!(print_title_rows(&names, "E"), None);
    assert_eq!(print_title_rows(&names, "Z"), None);

    let bytes = package(
        "<sheets><sheet name=\"S\" sheetId=\"1\" r:id=\"rId1\"/></sheets>\
         <definedNames><definedName name=\"_xlnm.Print_Titles\" localSheetId=\"0\">S!$1:$2</definedName></definedNames>",
        &[("rId1", &office_rel("worksheet"), "worksheets/sheet1.xml")],
        &[(
            "xl/worksheets/sheet1.xml",
            format!("<worksheet xmlns=\"{NS}\"/>"),
        )],
    );
    let wb = read(&bytes);
    assert_eq!(
        print_title_rows(&wb.names, "S"),
        Some(RowSpan { first: 0, last: 1 })
    );
}
