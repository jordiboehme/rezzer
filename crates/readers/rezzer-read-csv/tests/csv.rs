// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Block, Bytes, Cell, Completeness, Confidence, Detection, FormatId, Inline, LossReason,
    ReadOptions, ReadOutcome, Reader, Role, SectionKind, Severity, Source, Value,
};
use rezzer_read_csv::CsvReader;

fn source(name: &str, format: FormatId, bytes: &[u8]) -> Source {
    Source {
        path: None,
        name: name.into(),
        bytes: Bytes::Owned(Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format,
        detection: Detection {
            confidence: Confidence::Likely,
            evidence: "csv".into(),
        },
        modified: None,
    }
}

fn read_as(name: &str, format: FormatId, bytes: &[u8]) -> ReadOutcome {
    CsvReader.read(
        &source(name, format, bytes),
        &ReadOptions::default(),
        &Budget::new(Limits::default()),
    )
}

fn read(bytes: &[u8]) -> ReadOutcome {
    read_as("data.csv", FormatId::Csv, bytes)
}

fn text(cell: &Cell) -> String {
    cell.inlines
        .iter()
        .map(|i| match i {
            Inline::Text { text, .. } => text.as_str(),
            Inline::LineBreak => " ",
            other => panic!("unexpected inline {other:?}"),
        })
        .collect()
}

fn tables(out: &ReadOutcome) -> (String, Vec<&Block>) {
    let [
        Block::Section {
            kind,
            title,
            blocks,
            ..
        },
    ] = out.document.body.as_slice()
    else {
        panic!("expected one section, got {:?}", out.document.body);
    };
    assert_eq!(*kind, SectionKind::Sheet);
    (title.clone().unwrap(), blocks.iter().collect())
}

fn rows_of(block: &Block) -> Vec<Vec<String>> {
    let Block::Table { rows, .. } = block else {
        panic!("expected table, got {block:?}");
    };
    rows.iter().map(|r| r.iter().map(text).collect()).collect()
}

#[test]
fn identity() {
    assert_eq!(CsvReader.id(), "csv");
    assert_eq!(CsvReader.formats(), &[FormatId::Csv, FormatId::Tsv]);
}

#[test]
fn table_with_header() {
    let out = read(b"name,age,city\nAnna,31,Bonn\nBen,25,Koeln\nCleo,40,Essen\n");
    assert_eq!(out.completeness, Completeness::Complete);
    let (title, blocks) = tables(&out);
    assert_eq!(title, "data");
    let Block::Table {
        header_rows, rows, ..
    } = blocks[0]
    else {
        panic!()
    };
    assert_eq!(*header_rows, 1);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows_of(blocks[0])[0], ["name", "age", "city"]);
    assert_eq!(rows_of(blocks[0])[2], ["Ben", "25", "Koeln"]);
    assert_eq!(rows[2][1].anchor.as_ref().unwrap().0, "data!B3");
    let c = &out.document.properties.custom;
    assert_eq!(c["delimiter"], Value::Text { value: ",".into() });
    assert_eq!(c["rows"], Value::number(3.0, None));
    assert_eq!(c["columns"], Value::number(3.0, None));
    assert_eq!(c["has_header"], Value::Bool { value: true });
    assert_eq!(out.document.source.reader.as_ref().unwrap().id, "csv");
    assert_eq!(out.document.source.encoding.as_deref(), Some("UTF-8"));
}

#[test]
fn columns_table_lists_kinds() {
    let out = read(
        b"id,price (EUR),day,ok,note\n1,1.50,2024-01-01,yes,a\n2,2.25,2024-01-02,no,\n3,3.00,2024-01-03,yes,c\n",
    );
    let (_, blocks) = tables(&out);
    let cols = rows_of(blocks[1]);
    assert_eq!(
        cols[0],
        ["Column", "Type", "Unit", "Missing", "Unique", "Key"]
    );
    assert_eq!(cols[1][..2], ["id", "integer"]);
    assert_eq!(cols[2][..3], ["price (EUR)", "decimal (2 places)", "EUR"]);
    assert_eq!(cols[3][1], "date");
    assert_eq!(cols[4][1], "bool");
    assert_eq!(cols[5][1], "text");
    assert_eq!(cols[5][3], "1");
    assert_eq!(cols[1][4], "yes");
    assert_eq!(cols[1][5], "yes");
}

#[test]
fn derived_column_becomes_formula() {
    let mut csv = String::from("Year,Total,Gas,Liquid,Solid\n");
    for i in 0..20 {
        let (g, l, s) = (
            10 + i * i % 7 * 13,
            20 + i * 5 % 11 * 9,
            30 + i * 3 % 13 * 17,
        );
        csv.push_str(&format!("{},{},{g},{l},{s}\n", 2000 + i, g + l + s));
    }
    let out = read(csv.as_bytes());
    let f = &out.document.facets.formulas;
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].expression, "Total = Gas + Liquid + Solid");
    assert_eq!(f[0].dialect, "derived");
    assert_eq!(f[0].role, Some(Role::Result));
    assert_eq!(f[0].pattern, None);
    assert_eq!(f[0].note.as_deref(), Some("holds 20 of 20 rows"));
    assert_eq!(f[0].anchor.0, "data!B:B");
    let refs: Vec<_> = f[0].references.iter().map(|a| a.0.as_str()).collect();
    assert_eq!(refs, ["data!C:C", "data!D:D", "data!E:E"]);
}

#[test]
fn broken_csv_is_partial_with_losses() {
    let out = read(b"a,b,c\n1,2,3\n4,5\n6,7,8\n9,10,11,12\n13,14,15\n\"open,1,2\n");
    assert_eq!(out.completeness, Completeness::Partial);
    let damaged: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Damaged)
        .collect();
    assert!(damaged.len() >= 2, "{damaged:?}");
    assert!(
        damaged
            .iter()
            .any(|l| l.location == "record 3 (line 3)" && l.what == "row has 2 fields, expected 3")
    );
}

#[test]
fn empty_csv_is_complete_and_says_empty() {
    let out = read(b"");
    assert_eq!(out.completeness, Completeness::Complete);
    let [Block::Section { blocks, .. }] = out.document.body.as_slice() else {
        panic!("{:?}", out.document.body)
    };
    let [Block::Paragraph { inlines, .. }] = blocks.as_slice() else {
        panic!("{blocks:?}")
    };
    assert!(matches!(&inlines[0], Inline::Text { text, .. } if text == "The file is empty."));
}

#[test]
fn latin1_semicolon_decoded() {
    let out = read(b"Stadt;Land\nM\xfcnchen;Bayern\nK\xf6ln;NRW\nBonn;NRW\n");
    let (_, blocks) = tables(&out);
    assert_eq!(rows_of(blocks[0])[1][0], "M\u{fc}nchen");
    assert_eq!(
        out.document.properties.custom["delimiter"],
        Value::Text { value: ";".into() }
    );
}

#[test]
fn rows_beyond_cap_are_counted_with_info_loss() {
    let mut csv = String::from("n,v\n");
    for i in 0..10_005 {
        csv.push_str(&format!("{i},x{i}\n"));
    }
    let out = read(csv.as_bytes());
    let (_, blocks) = tables(&out);
    assert_eq!(rows_of(blocks[0]).len(), 10_001);
    let l = out
        .document
        .losses
        .iter()
        .find(|l| l.why == LossReason::Limit)
        .unwrap();
    assert_eq!(l.what, "rows 10001 to 10005 not shown");
    assert_eq!(l.severity, Severity::Info);
    assert_eq!(out.completeness, Completeness::Complete);
    assert_eq!(
        out.document.properties.custom["rows"],
        Value::number(10_005.0, None)
    );
}

#[test]
fn tsv_without_sniffable_dialect_is_guessed() {
    let out = read_as("t.tsv", FormatId::Tsv, b"only one line\n");
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.severity == Severity::Info
                && l.why == LossReason::Uncertain
                && l.what == "dialect guessed from the file type")
    );
}

fn widths(block: &Block) -> Vec<usize> {
    let Block::Table { rows, .. } = block else {
        panic!()
    };
    rows.iter().map(Vec::len).collect()
}

#[test]
fn ragged_table_is_rectangular() {
    let out = read(ragged_input().as_bytes());
    let (_, blocks) = tables(&out);
    let w = widths(blocks[0]);
    assert!(w.iter().all(|&x| x == 4), "{w:?}");
    let rows = rows_of(blocks[0]);
    assert_eq!(rows[0], ["id", "qty", "price", "Column 4"]);
    assert_eq!(rows[3], ["3", "4", "", ""]);
    assert_eq!(rows[4][3], "9");
    assert_eq!(out.completeness, Completeness::Partial);
    assert_eq!(
        out.document
            .losses
            .iter()
            .filter(|l| l.why == LossReason::Damaged)
            .count(),
        2
    );
}

#[test]
fn wide_file_is_capped_by_cells() {
    let cols = 3000;
    let header: Vec<String> = (0..cols).map(|i| format!("h{i}")).collect();
    let mut csv = header.join(",");
    csv.push('\n');
    for r in 0..200 {
        let row: Vec<String> = (0..cols).map(|i| (r + i).to_string()).collect();
        csv.push_str(&row.join(","));
        csv.push('\n');
    }
    let out = read(csv.as_bytes());
    let (_, blocks) = tables(&out);
    // 200,000 / 3,000 = 66 data rows plus the header
    assert_eq!(widths(blocks[0]).len(), 67);
    let l = out
        .document
        .losses
        .iter()
        .find(|l| l.why == LossReason::Limit)
        .unwrap();
    assert_eq!(l.what, "rows 67 to 200 not shown");
    assert_eq!(l.severity, Severity::Info);
}

#[test]
fn single_unsniffable_record_is_a_header() {
    let out = read(b"a,b,c\n");
    let [Block::Section { blocks, .. }] = out.document.body.as_slice() else {
        panic!()
    };
    let [Block::Paragraph { inlines, .. }] = blocks.as_slice() else {
        panic!("{blocks:?}")
    };
    assert!(
        matches!(&inlines[0], Inline::Text { text, .. } if text == "The file has a header row but no data rows.")
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.why == LossReason::Uncertain)
    );
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn bom_is_not_part_of_the_first_header() {
    let out = read(b"\xef\xbb\xbfname,age\nAnna,31\nBen,25\nCleo,40\n");
    let (_, blocks) = tables(&out);
    assert_eq!(rows_of(blocks[0])[0], ["name", "age"]);
}

#[test]
fn tsv_happy_path() {
    let out = read_as(
        "t.tsv",
        FormatId::Tsv,
        b"name\tage\nAnna\t31\nBen\t25\nCleo\t40\n",
    );
    let (title, blocks) = tables(&out);
    assert_eq!(title, "t");
    assert_eq!(rows_of(blocks[0])[2], ["Ben", "25"]);
    assert_eq!(
        out.document.properties.custom["delimiter"],
        Value::Text {
            value: "tab".into()
        }
    );
    assert_eq!(out.completeness, Completeness::Complete);
}

fn ragged_input() -> String {
    let mut csv = String::from("id,qty,price\n");
    for i in 1..=30 {
        match i {
            3 => csv.push_str("3,4\n"),
            4 => csv.push_str("4,7,8,9\n"),
            _ => csv.push_str(&format!("{i},{},{}\n", i * 3 % 7, i * 5 % 11)),
        }
    }
    csv
}

#[test]
fn columns_table_is_marked_as_summary() {
    let out = read(b"a,b\n1,2\n3,4\n");
    let [Block::Section { blocks, .. }] = out.document.body.as_slice() else {
        panic!("one section");
    };
    let roles: Vec<(Option<&str>, Option<&str>)> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Table { caption, role, .. } => Some((caption.as_deref(), role.as_deref())),
            _ => None,
        })
        .collect();
    assert_eq!(
        roles,
        vec![(None, None), (Some("Columns"), Some("summary"))]
    );
}

fn property(out: &ReadOutcome, key: &str) -> Value {
    out.document.properties.custom[key].clone()
}

#[test]
fn unterminated_quote_at_the_end_is_an_error_loss() {
    let out = read(b"a,b\n1,2\n3,\"oops\n4,5\n6,7\n");
    assert_eq!(out.completeness, Completeness::Partial);
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.what.starts_with("unterminated quote"))
        .expect("loss");
    assert_eq!(loss.why, LossReason::Damaged);
    assert_eq!(loss.severity, Severity::Error);
    assert_eq!(loss.location, "record 3 (line 3)");
    assert_eq!(
        loss.what,
        "unterminated quote: the rest of the file was read as one cell"
    );
}

#[test]
fn quote_closed_inside_a_later_field_is_reported() {
    let out = read(b"a,b\n1,\"open\n2,3\n4,\"x y\",5\n6,7\n");
    assert_eq!(out.completeness, Completeness::Partial);
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.what.starts_with("unterminated quote") && l.location == "record 2 (line 2)")
        .expect("loss");
    assert_eq!(loss.severity, Severity::Error);
}

#[test]
fn column_count_is_the_padded_width_everywhere() {
    let out = read(ragged_input().as_bytes());
    assert_eq!(property(&out, "columns"), Value::number(4.0, None));
    let (_, blocks) = tables(&out);
    let facts = rows_of(blocks[1]);
    assert_eq!(
        facts[0],
        ["Column", "Type", "Unit", "Missing", "Unique", "Key", "Note"]
    );
    assert_eq!(facts.len(), 5);
    assert_eq!(facts[4][0], "Column 4");
    assert_eq!(facts[4][6], "only in ragged rows");
    assert_eq!(facts[1][6], "");
}

#[test]
fn columns_table_has_no_note_column_without_ragged_rows() {
    let out = read(b"a,b\n1,2\n3,4\n");
    let (_, blocks) = tables(&out);
    assert_eq!(rows_of(blocks[1])[0].len(), 6);
}

#[test]
fn newline_runs_in_cells_become_one_space() {
    let out = read(b"a,b\n1,\"ha \n\"\"ha\"\" \r\n  ha\"\n3,\"x  y\"\n");
    let (_, blocks) = tables(&out);
    let rows = rows_of(blocks[0]);
    assert_eq!(rows[1][1], "ha \"ha\" ha");
    assert_eq!(rows[2][1], "x  y");
}

#[test]
fn a_mid_file_and_an_end_of_file_quote_each_get_a_loss() {
    let out = read(b"a,b\n1,2\n2,\"open\n3,4\n4,\"x y\",5\n9,\"oops,1\n7,8\n7,8\n");
    let places: Vec<&str> = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.starts_with("unterminated quote"))
        .map(|l| l.location.as_str())
        .collect();
    assert_eq!(places, ["record 3 (line 3)", "record 4 (line 6)"]);
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn line_breaks_stay_in_the_model() {
    let out = read(b"a,b\n1,\"x \n y\"\n3,4\n");
    let Block::Section { blocks, .. } = &out.document.body[0] else {
        panic!()
    };
    let Block::Table { rows, .. } = &blocks[0] else {
        panic!()
    };
    let kinds: Vec<&str> = rows[1][1]
        .inlines
        .iter()
        .map(|i| match i {
            Inline::Text { .. } => "text",
            Inline::LineBreak => "break",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["text", "break", "text"]);
}

#[test]
fn a_budget_out_of_time_truncates_the_analysis_with_one_loss() {
    let mut bytes = b"id,name\n".to_vec();
    for i in 0..10_000 {
        bytes.extend_from_slice(format!("{i},n{i}\n").as_bytes());
    }
    let budget = Budget::new(Limits {
        max_time: std::time::Duration::ZERO,
        ..Limits::default()
    });
    std::thread::sleep(std::time::Duration::from_millis(5));
    let out = CsvReader.read(
        &source("data.csv", FormatId::Csv, &bytes),
        &ReadOptions::default(),
        &budget,
    );
    assert_eq!(out.completeness, Completeness::Partial);
    let limits: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(limits.len(), 1, "{limits:?}");
    assert_eq!(
        limits[0].what,
        "time limit reached during the analysis, only the first 4094 rows are counted, table not built"
    );
}
