// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::{Cursor, Write};
use std::time::Instant;

use sheetparts::odf::{self, DataPilot, DatabaseRange, OdsParts, OdsSheet};
use sheetparts::ooxml::Frozen;
use sheetparts::{CellPos, IssueKind, Limits, Look, PropValue, RowSpan};
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

fn read_bytes(bytes: &[u8], limits: &Limits) -> OdsParts {
    let mut archive = Archive::open(bytes, zipguard::Limits::default()).unwrap();
    odf::read(&mut archive, limits, &|| false)
}

fn read_file(path: &str) -> OdsParts {
    read_bytes(&corpus(path), &Limits::default())
}

const NS: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:number=\"urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\"";

fn content(styles: &str, tables: &str) -> String {
    format!(
        "<office:document-content {NS}><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:spreadsheet>{tables}</office:spreadsheet></office:body></office:document-content>"
    )
}

fn package(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let o = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", o).unwrap();
    zip.write_all(b"application/vnd.oasis.opendocument.spreadsheet")
        .unwrap();
    for (name, data) in parts {
        zip.start_file(*name, o).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn one_sheet(styles: &str, table_inner: &str) -> OdsParts {
    let xml = content(
        styles,
        &format!("<table:table table:name=\"S\">{table_inner}</table:table>"),
    );
    read_bytes(&package(&[("content.xml", &xml)]), &Limits::default())
}

fn sheet<'a>(parts: &'a OdsParts, name: &str) -> &'a OdsSheet {
    parts
        .sheets
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no sheet {name}"))
}

#[test]
fn hidden_sheet_by_table_style() {
    let parts = read_file("any_sheets.ods");
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    let names: Vec<_> = parts.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Visible", "Hidden", "VeryHidden", "Chart"]);
    assert!(!sheet(&parts, "Visible").hidden);
    assert!(sheet(&parts, "Hidden").hidden);
    // ODF has no very hidden state: the file only says display="false".
    assert!(sheet(&parts, "VeryHidden").hidden);
    assert!(!sheet(&parts, "Chart").hidden);
}

#[test]
fn hidden_rows_and_cols() {
    let parts = one_sheet(
        "",
        "<table:table-column table:number-columns-repeated=\"2\"/>\
         <table:table-column table:visibility=\"collapse\" table:number-columns-repeated=\"2\"/>\
         <table:table-column-group><table:table-column table:visibility=\"collapse\"/></table:table-column-group>\
         <table:table-column table:visibility=\"visible\"/>\
         <table:table-row><table:table-cell/></table:table-row>\
         <table:table-row table:visibility=\"collapse\" table:number-rows-repeated=\"3\"><table:table-cell/></table:table-row>\
         <table:table-row table:visibility=\"filter\"><table:table-cell/></table:table-row>\
         <table:table-row table:number-rows-repeated=\"5\"/>\
         <table:table-header-rows><table:table-row table:visibility=\"collapse\"/></table:table-header-rows>",
    );
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    let s = sheet(&parts, "S");
    // Columns C, D and E (0-based 2..=4), one merged span.
    assert_eq!(s.hidden_cols, vec![RowSpan { first: 2, last: 4 }]);
    // Row 2 to 5 (0-based 1..=4) merged, then row 10 (0-based 9).
    assert_eq!(
        s.hidden_rows,
        vec![
            RowSpan { first: 1, last: 4 },
            RowSpan {
                first: 10,
                last: 10
            }
        ]
    );
}

#[test]
fn annotation_with_author_and_date_at_cell() {
    let parts = read_file("with-annotation.ods");
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    let s = &parts.sheets[0];
    assert_eq!(s.annotations.len(), 1);
    let a = &s.annotations[0];
    assert_eq!(a.cell, "A1");
    assert_eq!(a.author.as_deref(), Some("Marc Schreiber"));
    assert_eq!(a.date.as_deref(), Some("2024-09-18T18:27:00"));
    assert_eq!(a.text, "This is annotation of a.1");

    // Position arithmetic: repeated rows and columns, covered cells, several paragraphs.
    let parts = one_sheet(
        "",
        "<table:table-row table:number-rows-repeated=\"4\"/>\
         <table:table-row>\
         <table:table-cell table:number-columns-repeated=\"2\"/>\
         <table:table-cell table:number-columns-spanned=\"2\" office:value-type=\"string\">\
         <office:annotation><dc:creator></dc:creator><text:p>one</text:p><text:p>two<text:s text:c=\"2\"/>x</text:p></office:annotation>\
         </table:table-cell>\
         <table:covered-table-cell/>\
         <table:table-cell><office:annotation><dc:creator>Ann</dc:creator><dc:date>2025-01-02T03:04:05</dc:date><text:p>five</text:p></office:annotation></table:table-cell>\
         </table:table-row>",
    );
    let s = sheet(&parts, "S");
    assert_eq!(s.annotations.len(), 2);
    assert_eq!(s.annotations[0].cell, "C5");
    assert_eq!(s.annotations[0].author, None);
    assert_eq!(s.annotations[0].text, "one\ntwo  x");
    assert_eq!(s.annotations[1].cell, "E5");
    assert_eq!(s.annotations[1].author.as_deref(), Some("Ann"));
}

#[test]
fn validation_rule() {
    let parts = one_sheet(
        "",
        "<table:content-validations>\
         <table:content-validation table:name=\"val1\" table:condition=\"of:cell-content-is-whole-number() and of:cell-content-is-between(1;10)\" table:allow-empty-cell=\"true\">\
         <table:help-message table:title=\"Pick\" table:display=\"true\"><text:p>One to ten</text:p></table:help-message>\
         <table:error-message table:message-type=\"stop\" table:title=\"Bad\" table:display=\"true\"><text:p>Out of range</text:p></table:error-message>\
         </table:content-validation>\
         <table:content-validation table:name=\"val2\" table:condition=\"of:cell-content-is-in-list(&quot;a&quot;;&quot;b;c&quot;;&quot;d&quot;)\"/>\
         <table:content-validation table:name=\"val3\" table:condition=\"of:is-true-formula(AND([.A1]&gt;0;[.B1]&lt;5))\"/>\
         <table:content-validation table:name=\"val4\" table:condition=\"of:cell-content-text-length-is-not-between(2;4)\"/>\
         <table:content-validation table:name=\"val5\" table:condition=\"of:cell-content-is-date() and of:cell-content-is-greater-than-or-equal-to(&quot;2020-01-01&quot;)\"/>\
         <table:content-validation table:name=\"val6\" table:condition=\"of:somethingnew()\"/>\
         </table:content-validations>\
         <table:table-row table:number-rows-repeated=\"1048576\">\
         <table:table-cell table:content-validation-name=\"val1\" table:number-columns-repeated=\"2\"/>\
         <table:table-cell table:content-validation-name=\"val2\"/>\
         </table:table-row>",
    );
    let s = sheet(&parts, "S");
    let by = |range: &str| {
        s.validations
            .iter()
            .find(|v| v.range == range)
            .unwrap_or_else(|| panic!("no validation at {range}: {:?}", s.validations))
    };
    let v = by("A1:B1048576");
    assert_eq!(v.kind, "whole");
    assert_eq!(v.operator.as_deref(), Some("between"));
    assert_eq!(v.formula1.as_deref(), Some("1"));
    assert_eq!(v.formula2.as_deref(), Some("10"));
    assert_eq!(v.prompt.as_deref(), Some("One to ten"));
    assert_eq!(v.error.as_deref(), Some("Out of range"));
    let v = by("C1:C1048576");
    assert_eq!(v.kind, "list");
    assert_eq!(
        v.list.as_deref(),
        Some(&["a".to_string(), "b;c".to_string(), "d".to_string()][..])
    );
    assert_eq!(s.validations.len(), 2);

    // Rules that no cell uses are still rules of the sheet, anchored by name only when used.
    // Conditions are checked on their own through a one-cell sheet each.
    let check = |condition: &str| {
        let xml = format!(
            "<table:content-validations><table:content-validation table:name=\"v\" table:condition=\"{condition}\"/></table:content-validations>\
             <table:table-row><table:table-cell table:content-validation-name=\"v\"/></table:table-row>"
        );
        let p = one_sheet("", &xml);
        (sheet(&p, "S").validations.clone(), p.issues)
    };
    let (v, _) = check("of:is-true-formula(AND([.A1]&gt;0;[.B1]&lt;5))");
    assert_eq!(v[0].range, "A1");
    assert_eq!(v[0].kind, "custom");
    assert_eq!(v[0].formula1.as_deref(), Some("AND([.A1]>0;[.B1]<5)"));
    let (v, _) = check("of:cell-content-text-length-is-not-between(2;4)");
    assert_eq!(v[0].kind, "textLength");
    assert_eq!(v[0].operator.as_deref(), Some("notBetween"));
    let (v, _) = check(
        "of:cell-content-is-date() and of:cell-content-is-greater-than-or-equal-to(&quot;2020-01-01&quot;)",
    );
    assert_eq!(v[0].kind, "date");
    assert_eq!(v[0].operator.as_deref(), Some("greaterThanOrEqual"));
    assert_eq!(v[0].formula1.as_deref(), Some("\"2020-01-01\""));
    let (v, issues) = check("of:somethingnew()");
    assert_eq!(v[0].kind, "unknown");
    assert!(issues.iter().any(|i| i.kind == IssueKind::Unsupported));
}

#[test]
fn percent_data_style_becomes_format_code() {
    let styles = "\
        <number:percentage-style style:name=\"N11\"><number:number number:decimal-places=\"2\" number:min-integer-digits=\"1\"/><number:text>%</number:text></number:percentage-style>\
        <number:number-style style:name=\"N2\"><number:number number:decimal-places=\"2\" number:min-integer-digits=\"1\" number:grouping=\"true\"/></number:number-style>\
        <number:date-style style:name=\"D1\"><number:day number:style=\"long\"/><number:text>.</number:text><number:month number:style=\"long\"/><number:text>.</number:text><number:year number:style=\"long\"/></number:date-style>\
        <number:currency-style style:name=\"C1P0\"><number:number number:decimal-places=\"2\" number:min-integer-digits=\"1\" number:grouping=\"true\"/><number:text> </number:text><number:currency-symbol>EUR</number:currency-symbol></number:currency-style>\
        <number:currency-style style:name=\"C1\"><number:text>-</number:text><number:number number:decimal-places=\"2\" number:min-integer-digits=\"1\" number:grouping=\"true\"/><number:text> </number:text><number:currency-symbol>EUR</number:currency-symbol><style:map style:condition=\"value()&gt;=0\" style:apply-style-name=\"C1P0\"/></number:currency-style>\
        <number:time-style style:name=\"T1\" number:truncate-on-overflow=\"false\"><number:hours/><number:text>:</number:text><number:minutes number:style=\"long\"/><number:text>:</number:text><number:seconds number:style=\"long\" number:decimal-places=\"1\"/></number:time-style>\
        <number:number-style style:name=\"E1\"><number:scientific-number number:decimal-places=\"1\" number:min-integer-digits=\"3\" number:min-exponent-digits=\"1\"/></number:number-style>\
        <style:style style:name=\"ce1\" style:family=\"table-cell\" style:data-style-name=\"N11\"/>\
        <style:style style:name=\"ce2\" style:family=\"table-cell\" style:data-style-name=\"N2\"/>\
        <style:style style:name=\"ce3\" style:family=\"table-cell\" style:parent-style-name=\"ce2\"/>\
        <style:style style:name=\"ce4\" style:family=\"table-cell\" style:data-style-name=\"D1\"/>\
        <style:style style:name=\"ce5\" style:family=\"table-cell\" style:data-style-name=\"C1\"/>\
        <style:style style:name=\"ce6\" style:family=\"table-cell\" style:data-style-name=\"T1\"/>\
        <style:style style:name=\"ce7\" style:family=\"table-cell\" style:data-style-name=\"E1\"/>";
    let parts = one_sheet(
        styles,
        "<table:table-column table:default-cell-style-name=\"ce2\"/>\
         <table:table-column table:default-cell-style-name=\"ce1\" table:number-columns-repeated=\"2\"/>\
         <table:table-row>\
         <table:table-cell table:style-name=\"ce1\" office:value-type=\"percentage\" office:value=\"0.5\"/>\
         <table:table-cell office:value-type=\"float\" office:value=\"1\"/>\
         <table:table-cell table:style-name=\"ce3\" office:value-type=\"float\" office:value=\"2\"/>\
         <table:table-cell table:style-name=\"ce4\" office:value-type=\"date\"/>\
         <table:table-cell table:style-name=\"ce5\" office:value-type=\"float\"/>\
         <table:table-cell table:style-name=\"ce6\" office:value-type=\"time\"/>\
         <table:table-cell table:style-name=\"ce7\" office:value-type=\"float\"/>\
         <table:table-cell table:style-name=\"ce1\"/>\
         </table:table-row>",
    );
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    let f = &sheet(&parts, "S").cell_formats;
    let at = |col: u32| {
        f.iter()
            .find(|(p, _)| *p == CellPos { row: 0, col })
            .map(|(_, c)| c.as_str())
    };
    assert_eq!(at(0), Some("0.00%"));
    // No cell style: the column default applies.
    assert_eq!(at(1), Some("0.00%"));
    // The style has no data style of its own: it comes from the parent.
    assert_eq!(at(2), Some("#,##0.00"));
    assert_eq!(at(3), Some("dd\\.mm\\.yyyy"));
    assert_eq!(at(4), Some("[>=0]#,##0.00 \"EUR\";-#,##0.00 \"EUR\""));
    assert_eq!(at(5), Some("[h]:mm:ss.0"));
    assert_eq!(at(6), Some("000.0E+0"));
    // An empty cell carries no format entry.
    assert_eq!(at(7), None);
    assert_eq!(f.len(), 7);
}

#[test]
fn meta_props() {
    let meta = format!(
        "<office:document-meta {NS}><office:meta>\
         <meta:initial-creator>Ann</meta:initial-creator>\
         <meta:creation-date>2024-09-18T18:26:16</meta:creation-date>\
         <dc:date>2024-09-19T10:00:00</dc:date><dc:creator>Bob</dc:creator>\
         <dc:title>Budget</dc:title><dc:subject>Money</dc:subject><dc:description>For &amp; about</dc:description>\
         <meta:keyword>a</meta:keyword><meta:keyword>b</meta:keyword>\
         <meta:generator>LibreOffice/24.8</meta:generator>\
         <meta:editing-cycles>2</meta:editing-cycles>\
         <meta:user-defined meta:name=\"Project\">Apollo</meta:user-defined>\
         <meta:user-defined meta:name=\"Rate\" meta:value-type=\"float\">1.5</meta:user-defined>\
         <meta:user-defined meta:name=\"Done\" meta:value-type=\"boolean\">true</meta:user-defined>\
         <meta:user-defined meta:name=\"Due\" meta:value-type=\"date\">2025-03-01</meta:user-defined>\
         </office:meta></office:document-meta>"
    );
    let xml = content("", "<table:table table:name=\"S\"/>");
    let bytes = package(&[("content.xml", &xml), ("meta.xml", &meta)]);
    let parts = read_bytes(&bytes, &Limits::default());
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    let p = &parts.props;
    assert_eq!(p.core["creator"], "Ann");
    assert_eq!(p.core["created"], "2024-09-18T18:26:16");
    assert_eq!(p.core["lastModifiedBy"], "Bob");
    assert_eq!(p.core["modified"], "2024-09-19T10:00:00");
    assert_eq!(p.core["title"], "Budget");
    assert_eq!(p.core["subject"], "Money");
    assert_eq!(p.core["description"], "For & about");
    assert_eq!(p.core["keywords"], "a, b");
    assert_eq!(p.app["Application"], "LibreOffice/24.8");
    assert_eq!(p.app["EditingCycles"], "2");
    assert_eq!(
        p.custom,
        vec![
            ("Project".to_string(), PropValue::Text("Apollo".to_string())),
            ("Rate".to_string(), PropValue::Number(1.5)),
            ("Done".to_string(), PropValue::Bool(true)),
            (
                "Due".to_string(),
                PropValue::DateTime("2025-03-01".to_string())
            ),
        ]
    );
    // The corpus file has real metadata.
    let real = read_file("with-annotation.ods");
    assert_eq!(real.props.core["creator"], "Marc Schreiber");
    assert!(real.props.app["Application"].starts_with("LibreOffice/"));
}

#[test]
fn named_ranges_global_and_local() {
    let xml = content(
        "",
        "<table:named-expressions>\
         <table:named-range table:name=\"Total\" table:base-cell-address=\"$S.$A$1\" table:cell-range-address=\"$S.$A$1:.$B$2\"/>\
         <table:named-range table:name=\"Other\" table:cell-range-address=\"$'My Sheet'.$C$3\"/>\
         <table:named-expression table:name=\"Rate\" table:expression=\"of:=0.19\"/>\
         </table:named-expressions>\
         <table:table table:name=\"S\"><table:named-expressions>\
         <table:named-range table:name=\"Local\" table:cell-range-address=\"$S.$D$4:$S.$D$9\"/>\
         </table:named-expressions></table:table>",
    );
    let parts = read_bytes(&package(&[("content.xml", &xml)]), &Limits::default());
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    let n = |name: &str| parts.names.iter().find(|n| n.name == name).unwrap();
    assert_eq!(n("Total").refers_to, "S!$A$1:$B$2");
    assert_eq!(n("Total").scope, None);
    assert_eq!(n("Other").refers_to, "'My Sheet'!$C$3");
    assert_eq!(n("Rate").refers_to, "of:=0.19");
    assert_eq!(n("Local").refers_to, "S!$D$4:$D$9");
    assert_eq!(n("Local").scope.as_deref(), Some("S"));
    assert!(!n("Local").hidden);

    let real = read_file("formula-across-sheets.ods");
    assert_eq!(real.sheets.len(), 3);
}

#[test]
fn huge_repeat_counts_do_not_expand() {
    let start = Instant::now();
    let xml = content(
        "",
        "<table:table table:name=\"S\"><table:table-row table:number-rows-repeated=\"1048576\">\
         <table:table-cell table:number-columns-repeated=\"16384\"/></table:table-row></table:table>",
    );
    let parts = read_bytes(&package(&[("content.xml", &xml)]), &Limits::default());
    assert!(start.elapsed().as_secs_f64() < 1.0);
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);

    // Content in a repeated row counts its cells against the item limit.
    let start = Instant::now();
    let parts = read_file("malformed/issue_594_dos.ods");
    assert!(start.elapsed().as_secs_f64() < 1.0);
    assert_eq!(parts.sheets.len(), 1);
    assert!(parts.sheets[0].cell_formats.is_empty());
    assert!(parts.issues.iter().any(|i| i.kind == IssueKind::Limit));

    // Counts past the sheet size are clamped and said.
    let parts = one_sheet(
        "<style:style style:name=\"ce1\" style:family=\"table-cell\" style:data-style-name=\"N\"/>\
         <number:number-style style:name=\"N\"><number:number number:decimal-places=\"1\"/></number:number-style>",
        "<table:table-row table:number-rows-repeated=\"2000000\" table:visibility=\"collapse\"/>\
         <table:table-row><table:table-cell table:number-columns-repeated=\"99999\"/></table:table-row>",
    );
    let s = sheet(&parts, "S");
    assert_eq!(
        s.hidden_rows,
        vec![RowSpan {
            first: 0,
            last: 1_048_575
        }]
    );
    assert!(parts.issues.iter().any(|i| i.kind == IssueKind::Limit));
}

#[test]
fn small_repeated_content_is_listed_per_cell() {
    let parts = one_sheet(
        "<style:style style:name=\"ce1\" style:family=\"table-cell\" style:data-style-name=\"N\"/>\
         <number:number-style style:name=\"N\"><number:number number:decimal-places=\"1\"/></number:number-style>",
        "<table:table-row table:number-rows-repeated=\"2\">\
         <table:table-cell table:style-name=\"ce1\" table:number-columns-repeated=\"2\" office:value-type=\"float\"/>\
         </table:table-row>",
    );
    let f = &sheet(&parts, "S").cell_formats;
    let pos: Vec<_> = f.iter().map(|(p, _)| (p.row, p.col)).collect();
    assert_eq!(pos, vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
    assert_eq!(f[0].1, "0.0");
}

#[test]
fn broken_parts_become_issues() {
    let parts = read_bytes(&package(&[]), &Limits::default());
    assert!(parts.sheets.is_empty());
    assert!(parts.issues.iter().any(|i| i.part == "content.xml"));
    let parts = read_bytes(
        &package(&[("content.xml", "<office:document-content><oops")]),
        &Limits::default(),
    );
    assert!(parts.issues.iter().any(|i| i.kind == IssueKind::Damaged));
}

#[test]
fn every_corpus_ods_reads_without_panic() {
    let dir = format!(
        "{}/../../../tests/corpus/office",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut seen = 0;
    for entry in walk(std::path::Path::new(&dir)) {
        if entry.extension().is_some_and(|e| e == "ods") {
            let bytes = std::fs::read(&entry).unwrap();
            let parts = read_bytes(&bytes, &Limits::default());
            let malformed = entry.to_string_lossy().contains("malformed");
            if !malformed {
                assert!(parts.issues.is_empty(), "{entry:?}: {:?}", parts.issues);
                assert!(!parts.sheets.is_empty(), "{entry:?}");
            }
            seen += 1;
        }
    }
    assert!(seen >= 4);
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn validations_at_spreadsheet_level_are_resolved() {
    // Real files put content-validations beside the tables, not inside one.
    let xml = content(
        "",
        "<table:content-validations>\
         <table:content-validation table:name=\"val1\" table:condition=\"of:cell-content-is-whole-number()\">\
         <table:help-message table:display=\"true\"><text:p>Whole</text:p></table:help-message>\
         </table:content-validation></table:content-validations>\
         <table:table table:name=\"A\"><table:table-row>\
         <table:table-cell table:content-validation-name=\"val1\"/>\
         <table:table-cell table:content-validation-name=\"nope\"/>\
         </table:table-row></table:table>\
         <table:table table:name=\"B\"><table:table-row><table:table-cell/>\
         <table:table-cell table:content-validation-name=\"val1\"/></table:table-row></table:table>",
    );
    let parts = read_bytes(&package(&[("content.xml", &xml)]), &Limits::default());
    let a = sheet(&parts, "A");
    assert_eq!(a.validations.len(), 1);
    assert_eq!(a.validations[0].range, "A1");
    assert_eq!(a.validations[0].kind, "whole");
    assert_eq!(a.validations[0].prompt.as_deref(), Some("Whole"));
    let b = sheet(&parts, "B");
    assert_eq!(b.validations.len(), 1);
    assert_eq!(b.validations[0].range, "B1");
    assert!(
        parts
            .issues
            .iter()
            .any(|i| i.what.contains("nope") && i.kind == IssueKind::Damaged),
        "{:?}",
        parts.issues
    );
}

#[test]
fn u64_max_counts_do_not_overflow() {
    let m = u64::MAX;
    let parts = one_sheet(
        "<style:style style:name=\"ce1\" style:family=\"table-cell\" style:data-style-name=\"N\"/>\
         <number:number-style style:name=\"N\"><number:number/></number:number-style>",
        &format!(
            "<table:table-column/><table:table-column table:number-columns-repeated=\"{m}\" table:visibility=\"collapse\"/>\
             <table:table-row/>\
             <table:table-row table:number-rows-repeated=\"{m}\" table:visibility=\"collapse\">\
             <table:table-cell/><table:table-cell table:number-columns-repeated=\"{m}\" office:value-type=\"float\" table:style-name=\"ce1\"/>\
             </table:table-row>\
             <table:table-row table:number-rows-repeated=\"{m}\"><table:table-cell office:value-type=\"float\"/></table:table-row>"
        ),
    );
    let s = sheet(&parts, "S");
    assert_eq!(
        s.hidden_cols,
        vec![RowSpan {
            first: 1,
            last: 16383
        }]
    );
    assert!(parts.issues.iter().any(|i| i.kind == IssueKind::Limit));
}

#[test]
fn hostile_style_maps_are_bounded() {
    let maps: String = (0..300)
        .map(|_| "<style:map style:condition=\"value()&gt;0\" style:apply-style-name=\"N\"/>")
        .collect();
    let parts = one_sheet(
        &format!(
            "<number:number-style style:name=\"N\"><number:number number:decimal-places=\"2\"/>{maps}</number:number-style>\
             <style:style style:name=\"ce1\" style:family=\"table-cell\" style:data-style-name=\"N\"/>"
        ),
        "<table:table-row><table:table-cell table:style-name=\"ce1\" office:value-type=\"float\"/></table:table-row>",
    );
    for (_, code) in &sheet(&parts, "S").cell_formats {
        assert!(code.len() <= 255, "{} bytes", code.len());
    }
    assert!(
        parts
            .issues
            .iter()
            .any(|i| i.kind == IssueKind::Unsupported)
    );
}

#[test]
fn huge_digit_attributes_are_capped() {
    let big = "9000000000000000000";
    let parts = one_sheet(
        &format!(
            "<number:number-style style:name=\"E\"><number:scientific-number number:min-exponent-digits=\"{big}\" number:min-integer-digits=\"{big}\"/></number:number-style>\
             <number:number-style style:name=\"F\"><number:fraction number:min-numerator-digits=\"{big}\" number:min-denominator-digits=\"{big}\"/></number:number-style>\
             <style:style style:name=\"ce1\" style:family=\"table-cell\" style:data-style-name=\"E\"/>\
             <style:style style:name=\"ce2\" style:family=\"table-cell\" style:data-style-name=\"F\"/>"
        ),
        "<table:table-row><table:table-cell table:style-name=\"ce1\" office:value-type=\"float\"/>\
         <table:table-cell table:style-name=\"ce2\" office:value-type=\"float\"/></table:table-row>",
    );
    for (_, code) in &sheet(&parts, "S").cell_formats {
        assert!(code.len() <= 255, "{} bytes", code.len());
    }
}

#[test]
fn repeated_notes_are_charged_by_size() {
    let note = "x".repeat(10_000);
    let parts = one_sheet(
        "",
        &format!(
            "<table:table-row><table:table-cell table:number-columns-repeated=\"16384\">\
             <office:annotation><text:p>{note}</text:p></office:annotation></table:table-cell></table:table-row>"
        ),
    );
    let s = sheet(&parts, "S");
    assert!(s.annotations.len() < 1000, "{}", s.annotations.len());
    assert!(parts.issues.iter().any(|i| i.kind == IssueKind::Limit));
}

#[test]
fn bad_repeat_attributes_give_few_issues() {
    let cells = "<table:table-cell table:number-columns-repeated=\"x\"/>".repeat(1000);
    let parts = one_sheet("", &format!("<table:table-row>{cells}</table:table-row>"));
    assert!(parts.issues.len() <= 3, "{}", parts.issues.len());
    assert!(parts.issues.iter().any(|i| i.what.contains("1000")));
}

fn pos(row: u32, col: u32) -> CellPos {
    CellPos { row, col }
}

/// The CPU time the calling thread has used: work bounds do not depend on a busy machine.
#[cfg(unix)]
fn thread_cpu() -> std::time::Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime fills the timespec we pass.
    let r = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    assert_eq!(r, 0, "clock_gettime");
    std::time::Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

#[cfg(not(unix))]
fn thread_cpu() -> std::time::Duration {
    std::time::Duration::ZERO
}

fn with_parts(extra: &[(&str, &str)], styles: &str, tables: &str) -> OdsParts {
    let xml = content(styles, tables);
    let mut parts = vec![("content.xml", xml.as_str())];
    parts.extend_from_slice(extra);
    read_bytes(&package(&parts), &Limits::default())
}

#[test]
fn ods_merges_read() {
    let parts = read_file("merged_cells.ods");
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    // A cell spanned 1 x 2 (columns x rows) at A2 merges A2:A3; the covered cell adds nothing.
    assert_eq!(parts.sheets[0].merges, vec![(pos(1, 0), pos(2, 0))]);

    let parts = read_file("covered.ods");
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    assert_eq!(parts.sheets[0].merges, vec![(pos(1, 0), pos(2, 0))]);

    // Spans count from the position after repeats; a span of 1 x 1 is no merge.
    let parts = one_sheet(
        "",
        "<table:table-row table:number-rows-repeated=\"3\"/>\
         <table:table-row><table:table-cell table:number-columns-repeated=\"2\"/>\
         <table:table-cell table:number-columns-spanned=\"3\" table:number-rows-spanned=\"2\"/>\
         <table:covered-table-cell table:number-columns-repeated=\"2\"/>\
         <table:table-cell table:number-columns-spanned=\"1\" table:number-rows-spanned=\"1\"/></table:table-row>",
    );
    assert_eq!(sheet(&parts, "S").merges, vec![(pos(3, 2), pos(4, 4))]);
}

#[test]
fn database_ranges_with_header_footer_and_buttons() {
    let parts = read_file("database.ods");
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    assert_eq!(
        parts.database_ranges,
        vec![
            DatabaseRange {
                name: "database1".to_string(),
                sheet: "Sheet2".to_string(),
                range: "A1:C7".to_string(),
                header: true,
                footer: false,
                filter_buttons: false,
            },
            DatabaseRange {
                name: "__Anonymous_Sheet_DB__0".to_string(),
                sheet: "Sheet1".to_string(),
                range: "A1:D6".to_string(),
                header: true,
                footer: false,
                filter_buttons: true,
            },
        ]
    );

    let parts = read_file("cool15769_minimal_DB_ranges.ods");
    let by = |n: &str| {
        parts
            .database_ranges
            .iter()
            .find(|d| d.name == n)
            .unwrap_or_else(|| panic!("no range {n}"))
    };
    assert_eq!(parts.database_ranges.len(), 4);
    assert_eq!(by("singleheader").range, "C4");
    assert!(by("singleheader").header);
    assert!(!by("singlenoheader").header);
    assert_eq!(by("singlenoheader").range, "A4");
    assert_eq!(by("twowheader").range, "E4:E5");
    assert!(!by("twowheader").footer);
    assert!(by("twowheadertotals").footer);
    assert!(parts.database_ranges.iter().all(|d| !d.filter_buttons));

    let parts = read_file("autofilter-colbutton.ods");
    assert_eq!(parts.database_ranges.len(), 1);
    let d = &parts.database_ranges[0];
    assert_eq!(d.name, "__Anonymous_Sheet_DB__0");
    assert_eq!((d.sheet.as_str(), d.range.as_str()), ("Sheet1", "B1:C4"));
    assert!(d.filter_buttons);

    // Quoted sheet names with absolute markers, and an address that is none.
    let parts = with_parts(
        &[],
        "",
        "<table:table table:name=\"S\"/>\
         <table:database-ranges>\
         <table:database-range table:name=\"q\" table:target-range-address=\"$'My ''Sheet'.$A$1:.$C$10\" table:contains-header=\"false\"/>\
         <table:database-range table:name=\"bad\" table:target-range-address=\"nonsense\"/>\
         </table:database-ranges>",
    );
    assert_eq!(parts.database_ranges.len(), 1);
    assert_eq!(parts.database_ranges[0].sheet, "My 'Sheet");
    assert_eq!(parts.database_ranges[0].range, "A1:C10");
    assert!(!parts.database_ranges[0].header);
    assert!(parts.issues.iter().any(|i| i.kind == IssueKind::Damaged));
}

#[test]
fn data_pilot_target_range() {
    let parts = read_file("pivot-table-basic.ods");
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    assert_eq!(
        parts.data_pilots,
        vec![DataPilot {
            name: "DataPilot1".to_string(),
            sheet: "Pivot".to_string(),
            range: "A1:E10".to_string(),
        }]
    );
}

#[test]
fn header_rows_for_print() {
    let parts = read_file("print-range.ods");
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    assert_eq!(
        sheet(&parts, "Sheet1").header_rows,
        Some(RowSpan { first: 0, last: 1 })
    );
    // Two repeated rows, then a repeated pair and one more inside the header group.
    assert_eq!(
        sheet(&parts, "Sheet2").header_rows,
        Some(RowSpan { first: 2, last: 4 })
    );
    let parts = one_sheet("", "<table:table-row/>");
    assert_eq!(sheet(&parts, "S").header_rows, None);
}

#[test]
fn row_groups_as_spans() {
    let parts = read_file("outline.ods");
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    let span = |first, last| RowSpan { first, last };
    assert_eq!(
        sheet(&parts, "OutlineSheet").outline,
        vec![
            (span(1, 1), 1),
            (span(2, 3), 2),
            (span(4, 5), 3),
            (span(6, 20), 4),
            (span(21, 23), 3),
            (span(24, 26), 2),
            (span(27, 29), 1),
        ]
    );

    // Sibling groups stay apart; a repeat is one span.
    let parts = one_sheet(
        "",
        "<table:table-row-group><table:table-row table:number-rows-repeated=\"1000000\"/></table:table-row-group>\
         <table:table-row-group><table:table-row/></table:table-row-group>",
    );
    assert_eq!(
        sheet(&parts, "S").outline,
        vec![
            (
                RowSpan {
                    first: 0,
                    last: 999_999
                },
                1
            ),
            (
                RowSpan {
                    first: 1_000_000,
                    last: 1_000_000
                },
                1
            ),
        ]
    );
}

const STYLES_XML: &str = "<office:document-styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\"><office:styles>\
    <style:style style:name=\"Default\" style:family=\"table-cell\"/>\
    <style:style style:name=\"Heading\" style:family=\"table-cell\" style:parent-style-name=\"Default\">\
    <style:text-properties fo:font-weight=\"bold\"/>\
    <style:table-cell-properties fo:background-color=\"#dddddd\" fo:border-bottom=\"0.74pt solid #000000\"/></style:style>\
    <style:style style:name=\"Sub\" style:family=\"table-cell\" style:parent-style-name=\"Heading\"/>\
    </office:styles></office:document-styles>";

#[test]
fn bold_inherited_from_parent_style() {
    let parts = with_parts(
        &[("styles.xml", STYLES_XML)],
        "<style:style style:name=\"ce1\" style:family=\"table-cell\" style:parent-style-name=\"Sub\"/>\
         <style:style style:name=\"ce2\" style:family=\"table-cell\" style:parent-style-name=\"Heading\">\
         <style:text-properties fo:font-weight=\"400\"/><style:table-cell-properties fo:background-color=\"transparent\" fo:border=\"0.25pt solid #000000\" fo:border-bottom=\"none\"/>\
         <style:paragraph-properties fo:margin-left=\"0.4in\"/></style:style>\
         <style:style style:name=\"ce3\" style:family=\"table-cell\" style:parent-style-name=\"Default\">\
         <style:text-properties fo:font-weight=\"700\"/><style:table-cell-properties fo:background-color=\"#FFFFFF\" fo:border-top=\"1pt double #000000\"/>\
         <style:paragraph-properties fo:margin-left=\"3pt\"/></style:style>\
         <style:style style:name=\"co1\" style:family=\"table-column\"/>",
        "<table:table table:name=\"S\">\
         <table:table-column table:default-cell-style-name=\"ce3\"/>\
         <table:table-column table:default-cell-style-name=\"Default\" table:number-columns-repeated=\"2\"/>\
         <table:table-row>\
         <table:table-cell table:style-name=\"ce1\" office:value-type=\"string\"/>\
         <table:table-cell table:style-name=\"ce2\" office:value-type=\"string\"/>\
         <table:table-cell office:value-type=\"string\"/></table:table-row>\
         <table:table-row><table:table-cell office:value-type=\"string\"/>\
         <table:table-cell table:style-name=\"ce3\"/></table:table-row></table:table>",
    );
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    let s = sheet(&parts, "S");
    assert!(s.has_looks);
    let heading = Look {
        bold: true,
        filled: true,
        rule_below: true,
        ..Look::default()
    };
    let ce3 = Look {
        bold: true,
        indent: 1,
        rule_above: true,
        ..Look::default()
    };
    let ce2 = Look {
        indent: 3,
        ..Look::default()
    };
    // The third cell of row 1 takes the plain column default, so it has no look.
    assert_eq!(
        s.looks,
        vec![
            (pos(0, 0), heading),
            (pos(0, 1), ce2),
            (pos(1, 0), ce3),
            (pos(1, 1), ce3),
        ]
    );

    // A file without any cell style says nothing about looks.
    let parts = one_sheet(
        "",
        "<table:table-row><table:table-cell office:value-type=\"string\"/></table:table-row>",
    );
    assert!(!sheet(&parts, "S").has_looks);
    assert!(sheet(&parts, "S").looks.is_empty());
}

const SETTINGS: &str = "<office:document-settings xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\"><office:settings>\
    <config:config-item-set config:name=\"ooo:view-settings\">\
    <config:config-item config:name=\"VisibleAreaTop\" config:type=\"int\">0</config:config-item>\
    <config:config-item-map-indexed config:name=\"Views\"><config:config-item-map-entry>\
    <config:config-item config:name=\"ViewId\" config:type=\"string\">view1</config:config-item>\
    <config:config-item-map-named config:name=\"Tables\">\
    <config:config-item-map-entry config:name=\"S\">\
    <config:config-item config:name=\"HorizontalSplitMode\" config:type=\"short\">2</config:config-item>\
    <config:config-item config:name=\"VerticalSplitMode\" config:type=\"short\">2</config:config-item>\
    <config:config-item config:name=\"HorizontalSplitPosition\" config:type=\"int\">1</config:config-item>\
    <config:config-item config:name=\"VerticalSplitPosition\" config:type=\"int\">3</config:config-item>\
    </config:config-item-map-entry>\
    <config:config-item-map-entry config:name=\"Rows\">\
    <config:config-item config:name=\"HorizontalSplitMode\" config:type=\"short\">0</config:config-item>\
    <config:config-item config:name=\"VerticalSplitMode\" config:type=\"short\">2</config:config-item>\
    <config:config-item config:name=\"VerticalSplitPosition\" config:type=\"int\">2</config:config-item>\
    </config:config-item-map-entry>\
    <config:config-item-map-entry config:name=\"Dynamic\">\
    <config:config-item config:name=\"HorizontalSplitMode\" config:type=\"short\">1</config:config-item>\
    <config:config-item config:name=\"VerticalSplitMode\" config:type=\"short\">1</config:config-item>\
    <config:config-item config:name=\"VerticalSplitPosition\" config:type=\"int\">2</config:config-item>\
    </config:config-item-map-entry>\
    </config:config-item-map-named>\
    </config:config-item-map-entry></config:config-item-map-indexed>\
    </config:config-item-set></office:settings></office:document-settings>";

#[test]
fn frozen_pane_from_settings() {
    let tables = "<table:table table:name=\"S\"/><table:table table:name=\"Rows\"/>\
                  <table:table table:name=\"Dynamic\"/><table:table table:name=\"None\"/>";
    let parts = with_parts(&[("settings.xml", SETTINGS)], "", tables);
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    assert_eq!(sheet(&parts, "S").frozen, Some(Frozen { rows: 3, cols: 1 }));
    assert_eq!(
        sheet(&parts, "Rows").frozen,
        Some(Frozen { rows: 2, cols: 0 })
    );
    // Split mode 1 is a movable split, not a freeze.
    assert_eq!(sheet(&parts, "Dynamic").frozen, None);
    assert_eq!(sheet(&parts, "None").frozen, None);

    // No settings part, or a damaged one, leaves the other parts alone.
    let parts = with_parts(&[], "", tables);
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
    assert_eq!(sheet(&parts, "S").frozen, None);
    let parts = with_parts(
        &[("settings.xml", "<office:document-settings><oops")],
        "",
        tables,
    );
    assert_eq!(parts.sheets.len(), 4);
    assert_eq!(sheet(&parts, "S").frozen, None);
}

#[test]
fn repeated_spanned_cells_do_not_expand() {
    if !cfg!(unix) {
        eprintln!("skipped: this platform has no thread CPU clock here");
        return;
    }
    let start = thread_cpu();
    let parts = one_sheet(
        "",
        "<table:table-row table:number-rows-repeated=\"1048576\">\
         <table:table-cell table:number-columns-repeated=\"16384\" table:number-columns-spanned=\"2\"/></table:table-row>",
    );
    assert!(thread_cpu() - start < std::time::Duration::from_secs(1));
    let s = sheet(&parts, "S");
    // The merges are bounded by the item limit, not by the sheet size.
    assert!(s.merges.len() as u64 <= Limits::default().max_items as u64);
    assert!(parts.issues.iter().any(|i| i.kind == IssueKind::Limit));

    // One spanned cell repeated across a row stays one entry per copy and is fast.
    let start = thread_cpu();
    let parts = one_sheet(
        "",
        "<table:table-row><table:table-cell table:number-columns-repeated=\"16384\" table:number-columns-spanned=\"2\"/></table:table-row>",
    );
    assert!(thread_cpu() - start < std::time::Duration::from_secs(1));
    assert_eq!(sheet(&parts, "S").merges.len(), 16384);
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
}

#[test]
fn layout_lists_count_against_the_item_limit() {
    let limits = Limits {
        max_items: 20_000,
        ..Limits::default()
    };
    let rows =
        "<table:table-row><table:table-cell table:number-rows-spanned=\"2\"/></table:table-row>"
            .repeat(300);
    let xml = content(
        "",
        &format!("<table:table table:name=\"S\">{rows}</table:table>"),
    );
    let parts = read_bytes(&package(&[("content.xml", &xml)]), &limits);
    assert_eq!(sheet(&parts, "S").merges.len(), 300);

    let cells = "<table:table-cell table:number-rows-spanned=\"2\"/>".repeat(20);
    let rows =
        format!("<table:table-row table:number-rows-repeated=\"5000\">{cells}</table:table-row>");
    let xml = content(
        "",
        &format!("<table:table table:name=\"S\">{rows}</table:table>"),
    );
    let parts = read_bytes(&package(&[("content.xml", &xml)]), &limits);
    assert!(sheet(&parts, "S").merges.len() <= 20_000);
    assert!(parts.issues.iter().any(|i| i.kind == IssueKind::Limit));
}

#[test]
fn long_style_chain_resolves_in_linear_time() {
    if !cfg!(unix) {
        eprintln!("skipped: this platform has no thread CPU clock here");
        return;
    }
    let n = 50_000;
    let mut styles = String::from(
        "<style:style style:name=\"s0\" style:family=\"table-cell\"><style:text-properties fo:font-weight=\"bold\"/></style:style>",
    );
    let mut cells = String::new();
    for i in 1..n {
        styles.push_str(&format!(
            "<style:style style:name=\"s{i}\" style:family=\"table-cell\" style:parent-style-name=\"s{}\"/>",
            i - 1
        ));
    }
    for i in 0..n {
        cells.push_str(&format!(
            "<table:table-cell table:style-name=\"s{i}\" office:value-type=\"float\"/>"
        ));
    }
    let xml = content(
        &styles,
        &format!(
            "<table:table table:name=\"S\"><table:table-row>{cells}</table:table-row></table:table>"
        ),
    );
    let bytes = package(&[("content.xml", &xml)]);
    let start = thread_cpu();
    let parts = read_bytes(&bytes, &Limits::default());
    assert!(thread_cpu() - start < std::time::Duration::from_secs(1));
    let s = sheet(&parts, "S");
    // Styles within the cap inherit; the chain past it is said, once.
    assert!(s.looks.iter().any(|(p, l)| p.col == 10 && l.bold));
    assert!(!s.looks.iter().any(|(p, _)| p.col == 40_000));
    let deep: Vec<_> = parts
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Damaged && i.what.contains("64 levels"))
        .collect();
    assert_eq!(deep.len(), 1, "{:?}", parts.issues);
}

#[test]
fn looks_running_out_keep_merges_ranges_and_later_sheets() {
    let limits = Limits {
        max_items: 20_000,
        ..Limits::default()
    };
    let xml = content(
        "<style:style style:name=\"b\" style:family=\"table-cell\"><style:text-properties fo:font-weight=\"bold\"/></style:style>",
        "<table:table table:name=\"A\"><table:table-row table:number-rows-repeated=\"10\">\
         <table:table-cell table:style-name=\"b\" table:number-columns-repeated=\"16000\"/></table:table-row></table:table>\
         <table:table table:name=\"B\"><table:table-row>\
         <table:table-cell table:number-columns-spanned=\"2\" office:value-type=\"float\"/></table:table-row></table:table>\
         <table:database-ranges><table:database-range table:name=\"d\" table:target-range-address=\"B.A1:B.B2\"/></table:database-ranges>\
         <table:data-pilots><table:data-pilot-table table:name=\"p\" table:target-range-address=\"B.D1:B.E2\"/></table:data-pilots>",
    );
    let parts = read_bytes(&package(&[("content.xml", &xml)]), &limits);
    assert_eq!(sheet(&parts, "B").merges, vec![(pos(0, 0), pos(0, 1))]);
    assert_eq!(parts.database_ranges.len(), 1);
    assert_eq!(parts.data_pilots.len(), 1);
    let limit_issues: Vec<_> = parts
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Limit)
        .collect();
    assert_eq!(limit_issues.len(), 1, "{limit_issues:?}");
    assert!(limit_issues[0].what.contains("look"), "{limit_issues:?}");
    assert!(!limit_issues[0].what.contains("merge"));
}

#[test]
fn thresholds_of_bold_and_rules() {
    let style = |name: &str, weight: &str, border: &str| {
        format!(
            "<style:style style:name=\"{name}\" style:family=\"table-cell\">\
             <style:text-properties fo:font-weight=\"{weight}\"/>\
             <style:table-cell-properties fo:border-bottom=\"{border}\"/></style:style>"
        )
    };
    let styles = [
        style("a", "599", "0.49pt solid #000000"),
        style("b", "600", "0.5pt solid #000000"),
        style("c", "normal", "0.0069in solid #000000"),
        style("d", "normal", "0.0070in double #000000"),
        style("e", "normal", "1pt dotted #000000"),
    ]
    .concat();
    let cells: String = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|n| {
            format!("<table:table-cell table:style-name=\"{n}\" office:value-type=\"float\"/>")
        })
        .collect();
    let parts = one_sheet(
        &styles,
        &format!("<table:table-row>{cells}</table:table-row>"),
    );
    let got: Vec<(u32, bool, bool)> = sheet(&parts, "S")
        .looks
        .iter()
        .map(|(p, l)| (p.col, l.bold, l.rule_below))
        .collect();
    // a and c are plain and not listed; e is no solid or double line.
    assert_eq!(got, vec![(1, true, true), (3, false, true)]);
}

#[test]
fn first_view_that_names_a_sheet_decides() {
    let settings = SETTINGS.replace(
        "<config:config-item-map-indexed config:name=\"Views\"><config:config-item-map-entry>",
        "<config:config-item-map-indexed config:name=\"Views\">\
         <config:config-item-map-entry><config:config-item-map-named config:name=\"Tables\">\
         <config:config-item-map-entry config:name=\"S\">\
         <config:config-item config:name=\"VerticalSplitMode\" config:type=\"short\">0</config:config-item>\
         </config:config-item-map-entry></config:config-item-map-named></config:config-item-map-entry>\
         <config:config-item-map-entry>",
    );
    let parts = with_parts(
        &[("settings.xml", &settings)],
        "",
        "<table:table table:name=\"S\"/>",
    );
    assert_eq!(sheet(&parts, "S").frozen, None);
}

#[test]
fn range_across_two_sheets_is_damaged() {
    let parts = with_parts(
        &[],
        "",
        "<table:table table:name=\"S\"/><table:database-ranges>\
         <table:database-range table:name=\"x\" table:target-range-address=\"S.A1:T.B2\"/></table:database-ranges>",
    );
    assert!(parts.database_ranges.is_empty());
    assert_eq!(
        parts
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Damaged)
            .count(),
        1
    );
}

#[test]
fn style_chain_cap_does_not_depend_on_cell_order() {
    let mut styles = String::from(
        "<style:style style:name=\"s0\" style:family=\"table-cell\"><style:text-properties fo:font-weight=\"bold\"/></style:style>",
    );
    for i in 1..100 {
        styles.push_str(&format!(
            "<style:style style:name=\"s{i}\" style:family=\"table-cell\" style:parent-style-name=\"s{}\"/>",
            i - 1
        ));
    }
    for order in [[99, 40, 20], [20, 40, 99]] {
        let cells: String = order
            .iter()
            .map(|i| {
                format!("<table:table-cell office:value-type=\"float\" table:style-name=\"s{i}\"/>")
            })
            .collect();
        let parts = one_sheet(
            &styles,
            &format!("<table:table-row>{cells}</table:table-row>"),
        );
        let s = sheet(&parts, "S");
        // s99 is cut at the cap and never reaches the bold root; s40 and s20 always do.
        for (col, style) in order.iter().enumerate() {
            let bold = s.looks.iter().any(|(p, l)| p.col == col as u32 && l.bold);
            assert_eq!(bold, *style != 99, "order {order:?} style s{style}");
        }
    }
}

#[test]
fn database_ranges_and_pivots_are_capped_at_ten_thousand() {
    let ranges: String = (0..10_050)
        .map(|i| {
            format!(
                "<table:database-range table:name=\"d{i}\" table:target-range-address=\"S.A{}:S.B{}\"/>",
                i + 1,
                i + 2
            )
        })
        .collect();
    let pilots: String = (0..10_050)
        .map(|i| {
            format!(
                "<table:data-pilot-table table:name=\"p{i}\" table:target-range-address=\"S.D{}:S.E{}\"/>",
                i + 1,
                i + 2
            )
        })
        .collect();
    let parts = with_parts(
        &[],
        "",
        &format!(
            "<table:table table:name=\"S\"/><table:database-ranges>{ranges}</table:database-ranges><table:data-pilots>{pilots}</table:data-pilots>"
        ),
    );
    assert_eq!(parts.database_ranges.len(), 10_000);
    assert_eq!(parts.data_pilots.len(), 10_000);
    let said = |what: &str| {
        parts
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Limit && i.what.contains(what))
            .count()
    };
    assert_eq!(said("database ranges"), 1, "{:?}", parts.issues);
    assert_eq!(said("pivot tables"), 1, "{:?}", parts.issues);
}

#[test]
fn cells_without_layout_signals_keep_the_whole_total() {
    // 1.6 million counted cells: past 40 % of the total, under all of it.
    let parts = one_sheet(
        "",
        "<table:table-row table:number-rows-repeated=\"100\">\
         <table:table-cell table:number-columns-repeated=\"16384\" office:value-type=\"float\"/></table:table-row>",
    );
    assert!(parts.issues.is_empty(), "{:?}", parts.issues);
}

#[test]
fn long_range_names_are_bounded_by_bytes() {
    let limits = Limits {
        max_items: 20_000,
        ..Limits::default()
    };
    let name = "n".repeat(2000);
    let ranges: String = (0..100)
        .map(|i| {
            format!("<table:database-range table:name=\"{name}{i}\" table:target-range-address=\"S.A1:S.B2\"/>")
        })
        .collect();
    let xml = content(
        "",
        &format!(
            "<table:table table:name=\"S\"/><table:database-ranges>{ranges}</table:database-ranges>"
        ),
    );
    let parts = read_bytes(&package(&[("content.xml", &xml)]), &limits);
    // 20,000 / 16 bytes per unit: about 160 bytes of name per unit, so far fewer than 100 fit.
    assert!(
        parts.database_ranges.len() < 20,
        "{}",
        parts.database_ranges.len()
    );
    assert!(parts.issues.iter().any(|i| i.kind == IssueKind::Limit));
}
