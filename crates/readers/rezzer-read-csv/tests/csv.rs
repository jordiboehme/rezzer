// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Block, Bytes, Cell, Certainty, Completeness, Confidence, Detection, Evidence, FormatId, Inline,
    LayoutKind, LossReason, ReadOptions, ReadOutcome, Reader, Role, RowKind, SectionKind, Severity,
    Source, TableLayout, Value,
};
use rezzer_read_csv::CsvReader;
use rezzer_render::{OutputSection, RenderOptions, TableStyle};

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
    // The header cell past the header record is empty, and so is its header path, as for
    // sheets; the renderer names the column.
    assert_eq!(rows[0], ["id", "qty", "price", ""]);
    let paths = &layout_of(blocks[0]).header_paths;
    assert_eq!(paths[..3], ["id", "qty", "price"]);
    assert!(paths.iter().skip(3).all(String::is_empty), "{paths:?}");
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
    // The table is capped at 1,000 columns; 200,000 / 1,000 = the header plus 199 data rows.
    let w = widths(blocks[0]);
    assert_eq!(w.len(), 200);
    assert!(w.iter().all(|&x| x == 1_000), "{w:?}");
    let limits: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .map(|l| (l.what.as_str(), l.severity))
        .collect();
    assert_eq!(
        limits,
        [
            ("row 200 not shown", Severity::Info),
            ("2000 columns after column 1000 not shown", Severity::Info),
            (
                "columns 501 to 3000 not listed in the Columns table",
                Severity::Info
            ),
        ]
    );
    // The property counts every column, also those not shown.
    assert_eq!(property(&out, "columns"), Value::number(3000.0, None));
    assert_eq!(rows_of(blocks[1]).len(), 501);
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn the_element_budget_stops_the_table_with_a_limit_loss() {
    let mut bytes = b"id,name\n".to_vec();
    for i in 0..1_000 {
        bytes.extend_from_slice(format!("{i},n{i}\n").as_bytes());
    }
    let budget = Budget::new(Limits {
        max_elements: 100,
        ..Limits::default()
    });
    let out = CsvReader.read(
        &source("data.csv", FormatId::Csv, &bytes),
        &ReadOptions::default(),
        &budget,
    );
    assert_eq!(out.completeness, Completeness::Partial);
    let (_, blocks) = tables(&out);
    let rows = rows_of(blocks[0]);
    // 100 elements are 50 rows of two cells: the header and 49 data rows.
    assert_eq!(rows.len(), 50);
    let limits: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .collect();
    assert_eq!(limits.len(), 1, "{limits:?}");
    assert_eq!(limits[0].location, "row 51");
    assert_eq!(
        limits[0].what,
        "element limit reached, rest of the file not read"
    );
    assert_eq!(limits[0].severity, Severity::Warning);
    // The record that would pass the limit is not charged, so the layout can still place
    // the records read.
    assert_eq!(budget.elements(), 100);
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

/// The layout of a table block.
fn layout_of(block: &Block) -> &TableLayout {
    match block {
        Block::Table {
            layout: Some(l), ..
        } => l,
        other => panic!("expected a table with a layout, got {other:?}"),
    }
}

/// The table blocks of the sheet section, the Columns facts table left out.
fn data_tables<'a>(blocks: &[&'a Block]) -> Vec<&'a Block> {
    blocks
        .iter()
        .copied()
        .filter(|b| {
            matches!(
                b,
                Block::Table {
                    layout: Some(_),
                    ..
                }
            )
        })
        .collect()
}

fn inline_text(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .map(|i| match i {
            Inline::Text { text, .. } => text.as_str(),
            Inline::LineBreak => " ",
            _ => "",
        })
        .collect()
}

/// The ragged-row losses (`row has N fields, expected M`).
fn ragged_losses(out: &ReadOutcome) -> Vec<String> {
    out.document
        .losses
        .iter()
        .filter(|l| l.what.starts_with("row has "))
        .map(|l| format!("{}: {}", l.location, l.what))
        .collect()
}

fn markdown(out: &ReadOutcome, style: TableStyle) -> String {
    rezzer_render::render(
        &out.document,
        out.completeness,
        &RenderOptions {
            sections: Some(vec![OutputSection::Content]),
            table_style: style,
            ..RenderOptions::default()
        },
    )
}

#[test]
fn csv_title_lines_above_header() {
    let out = read(
        b"Station report 2024\nSource: test data\nid,name,value\n1,alpha,10\n2,beta,20\n3,gamma,30\n",
    );
    assert_eq!(out.completeness, Completeness::Complete);
    assert!(ragged_losses(&out).is_empty(), "{:?}", out.document.losses);
    let (_, blocks) = tables(&out);
    let Block::Heading {
        inlines, anchor, ..
    } = blocks[0]
    else {
        panic!("expected the title as heading, got {:?}", blocks[0]);
    };
    assert_eq!(inline_text(inlines), "Station report 2024");
    assert_eq!(anchor.as_ref().unwrap().0, "data!A1");
    let Block::Paragraph {
        inlines,
        role,
        anchor,
    } = blocks[1]
    else {
        panic!(
            "expected the second title line as caption, got {:?}",
            blocks[1]
        );
    };
    assert_eq!(inline_text(inlines), "Source: test data");
    assert_eq!(role.as_deref(), Some("caption"));
    assert_eq!(anchor.as_ref().unwrap().0, "data!A2");
    let shown = data_tables(&blocks);
    assert_eq!(shown.len(), 1);
    let l = layout_of(shown[0]);
    assert_eq!(l.kind, LayoutKind::Table);
    assert_eq!(l.range, "A1:C6");
    assert_eq!(l.header_rows, [3]);
    assert_eq!(l.header_paths, ["id", "name", "value"]);
    let rows = rows_of(shown[0]);
    assert_eq!(rows[0], ["id", "name", "value"]);
    assert_eq!(rows[1], ["1", "alpha", "10"]);
    let Block::Table { rows, .. } = shown[0] else {
        panic!()
    };
    assert_eq!(rows[1][2].anchor.as_ref().unwrap().0, "data!C4");
    // The Columns table names the columns by the header row, not by the title.
    let facts = rows_of(blocks.last().unwrap());
    assert_eq!(facts[1][..2], ["id", "integer"]);
    assert_eq!(property(&out, "rows"), Value::number(3.0, None));
}

#[test]
fn csv_blank_first_line() {
    let out = read(b"\nid,name,value\n1,alpha,10\n2,beta,20\n3,gamma,30\n");
    assert_eq!(out.completeness, Completeness::Complete);
    let (_, blocks) = tables(&out);
    let shown = data_tables(&blocks);
    assert_eq!(shown.len(), 1);
    let l = layout_of(shown[0]);
    assert_eq!(l.header_rows, [1]);
    assert_eq!(l.header_paths, ["id", "name", "value"]);
    assert_eq!(l.range, "A1:C4");
    assert_eq!(rows_of(shown[0])[3], ["3", "gamma", "30"]);
}

#[test]
fn csv_two_blocks_split_by_blank_line() {
    let out = read(b"id,value\n1,10\n2,20\n3,30\n\ncity,count\nBonn,3\nKoeln,4\nEssen,5\n");
    let (_, blocks) = tables(&out);
    let shown = data_tables(&blocks);
    let found: Vec<(&str, &[u32], Vec<&str>)> = shown
        .iter()
        .map(|t| {
            let l = layout_of(t);
            (
                l.range.as_str(),
                l.header_rows.as_slice(),
                l.header_paths.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("A1:B4", &[1][..], vec!["id", "value"]),
            ("A6:B9", &[6][..], vec!["city", "count"]),
        ]
    );
    let Block::Table { rows, .. } = shown[1] else {
        panic!()
    };
    assert_eq!(rows[1][0].anchor.as_ref().unwrap().0, "data!A7");
    assert_eq!(out.completeness, Completeness::Complete);
    // The second header row is no data row.
    assert_eq!(property(&out, "rows"), Value::number(6.0, None));
}

#[test]
fn layout_header_sets_the_properties() {
    // Two records are too few for csvsniff: the dialect is guessed without a header, and
    // the layout finds one by the type change.
    let out = read(b"key,val\n1,\"{\"\"a\"\": 1}\"\n");
    let (_, blocks) = tables(&out);
    assert_eq!(layout_of(data_tables(&blocks)[0]).header_rows, [1]);
    assert_eq!(property(&out, "has_header"), Value::Bool { value: true });
    assert_eq!(property(&out, "rows"), Value::number(1.0, None));
    // The column facts follow the layout's header, not csvsniff's guess.
    let facts = rows_of(blocks.last().unwrap());
    assert_eq!(facts[1][..4], ["key", "integer", "", "0"]);
    assert_eq!(facts[2][0], "val");
}

#[test]
fn header_found_by_csvsniff_but_not_by_the_layout_is_data_in_the_facts() {
    // All text and nothing repeats, so H3 needs the guess; csvsniff gives none here, but
    // with a guess the layout must agree before the facts take a header.
    let out = read(b"Ann,Berlin\nBob,Bonn\nCid,Koeln\n");
    let (_, blocks) = tables(&out);
    let facts = rows_of(blocks.last().unwrap());
    assert_eq!(facts[1][0], "Column 1");
    assert_eq!(facts[1][3], "0");
}

#[test]
fn group_rows_are_not_ragged() {
    let mut csv = String::from("Report\nid,qty,price\n");
    for line in [
        "North", "1,2,3", "2,4,6", "3,6,9", "South", "4,8,12", "5,10,15", "6,12,18",
    ] {
        csv.push_str(line);
        csv.push('\n');
    }
    let out = read(csv.as_bytes());
    let (_, blocks) = tables(&out);
    let l = layout_of(data_tables(&blocks)[0]);
    let groups: Vec<u32> = l
        .rows
        .iter()
        .filter(|n| n.kind == RowKind::Group)
        .map(|n| n.row)
        .collect();
    assert_eq!(groups, [3, 7], "{:?}", l.rows);
    assert!(ragged_losses(&out).is_empty(), "{:?}", ragged_losses(&out));
    assert_eq!(out.completeness, Completeness::Complete);
    // A short data row in the same file is still ragged.
    let csv = csv.replace("2,4,6\n", "2,4\n");
    let out = read(csv.as_bytes());
    assert_eq!(
        ragged_losses(&out),
        ["record 5 (line 5): row has 2 fields, expected 3"]
    );
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn short_header_record_is_not_ragged() {
    let mut csv = String::from("id,qty\n");
    for i in 1..=12 {
        csv.push_str(&format!("{i},{},{}\n", i * 2, i * 3));
    }
    let out = read(csv.as_bytes());
    let (_, blocks) = tables(&out);
    assert_eq!(layout_of(data_tables(&blocks)[0]).header_rows, [1]);
    assert!(ragged_losses(&out).is_empty(), "{:?}", ragged_losses(&out));
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn csv_key_and_records_anchor_quote_the_stem() {
    let out = read_as("it's a!b-c.csv", FormatId::Csv, b"id,qty\n1,2\n3,4\n5,6\n");
    let [Block::Section { anchor, title, .. }] = out.document.body.as_slice() else {
        panic!()
    };
    assert_eq!(title.as_deref(), Some("it's a!b-c"));
    assert_eq!(anchor.as_ref().unwrap().0, "'it''s a!b-c'");
    let (_, blocks) = tables(&out);
    let table = data_tables(&blocks)[0];
    assert_eq!(layout_of(table).key, "'it''s a!b-c'!A1");
    let Block::Table { rows, .. } = table else {
        panic!()
    };
    assert_eq!(rows[1][1].anchor.as_ref().unwrap().0, "'it''s a!b-c'!B2");
    let md = markdown(&out, TableStyle::Records);
    assert!(md.contains("- Row 2 ('it''s a!b-c'!A2:B2)"), "{md}");
}

#[test]
fn csv_header_from_layout_with_csvsniff_signal() {
    // csvsniff guesses a header (values repeat below a unique first row): rule H3 takes it.
    let out = read(b"name,city\nAnn,Berlin\nBob,Berlin\nCid,Bonn\n");
    assert_eq!(property(&out, "has_header"), Value::Bool { value: true });
    let (_, blocks) = tables(&out);
    let shown = data_tables(&blocks);
    assert_eq!(shown.len(), 1);
    let l = layout_of(shown[0]);
    assert_eq!(l.kind, LayoutKind::Table);
    assert_eq!(l.header_rows, [1]);
    assert_eq!(l.header_paths, ["name", "city"]);
    assert!(
        l.found_by.iter().any(|e| matches!(
            e,
            Evidence::Rule { id, detail } if id == "H3" && detail.contains("header guess")
        )),
        "{:?}",
        l.found_by
    );
    // Without the guess the same kind of list has no header: label and value pairs or text.
    let out = read(b"Ann,Berlin\nBob,Bonn\nCid,Koeln\n");
    assert_eq!(property(&out, "has_header"), Value::Bool { value: false });
    let (_, blocks) = tables(&out);
    let kinds: Vec<LayoutKind> = data_tables(&blocks)
        .iter()
        .map(|t| layout_of(t).kind)
        .collect();
    assert!(
        kinds
            .iter()
            .all(|k| matches!(k, LayoutKind::KeyValue | LayoutKind::Text)),
        "{kinds:?}"
    );
    assert!(!kinds.is_empty());
}

/// The corpus files of the plan 2 CSV reader.
fn corpus(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/web-text")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every cell text of the tables and paragraphs of the sheet, with its anchor.
fn shown_cells(blocks: &[&Block]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for b in blocks {
        match b {
            Block::Table {
                rows,
                layout: Some(_),
                ..
            } => {
                for row in rows {
                    for c in row {
                        let t = text(c);
                        if !t.trim().is_empty() {
                            out.push((c.anchor.as_ref().unwrap().0.clone(), t));
                        }
                    }
                }
            }
            Block::Paragraph {
                inlines,
                anchor: Some(a),
                ..
            }
            | Block::Heading {
                inlines,
                anchor: Some(a),
                ..
            } => out.push((a.0.clone(), inline_text(inlines))),
            _ => {}
        }
    }
    out.sort();
    out
}

#[test]
fn plain_csv_keeps_its_header() {
    for name in [
        "csv-datahub-co2-fossil-global.csv",
        "csv-pandas-fdic-banklist.csv",
        "csv-spectrum-comma_in_quotes.csv",
        "csv-spectrum-escaped_quotes.csv",
        "csv-spectrum-newlines_crlf.csv",
        "csv-spectrum-quotes_and_newlines.csv",
        "csv-semicolon-latin1-dwd-stations.txt",
    ] {
        let bytes = corpus(name);
        let out = read_as(name, FormatId::Csv, &bytes);
        assert_eq!(out.completeness, Completeness::Complete, "{name}");
        assert_eq!(
            property(&out, "has_header"),
            Value::Bool { value: true },
            "{name}"
        );
        let (sheet, blocks) = tables(&out);
        let shown = data_tables(&blocks);
        assert_eq!(shown.len(), 1, "{name}");
        let l = layout_of(shown[0]);
        assert_eq!(l.kind, LayoutKind::Table, "{name}");
        assert_eq!(l.header_rows, [1], "{name}");
        // The header and every value, each at its old anchor.
        let text = textenc::decode(&bytes).text;
        let delimiter = if name.contains("semicolon") {
            b';'
        } else {
            b','
        };
        let mut reader = ::csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());
        let mut want = Vec::new();
        for (i, rec) in reader.records().enumerate() {
            let rec = rec.unwrap();
            if i == 0 {
                let head: Vec<&str> = rec.iter().filter(|f| !f.is_empty()).collect();
                let paths: Vec<&str> = l.header_paths.iter().map(String::as_str).collect();
                assert_eq!(paths[..head.len()], head[..], "{name}");
            }
            for (j, f) in rec.iter().enumerate() {
                if !f.trim().is_empty() {
                    want.push((
                        format!(
                            "{}!{}{}",
                            sheetparts::quote_sheet(&sheet),
                            rezzer_model::column_letter(j),
                            i + 1
                        ),
                        f.split_whitespace().collect::<Vec<_>>().join(" "),
                    ));
                }
            }
        }
        want.sort();
        let got: Vec<(String, String)> = shown_cells(&blocks)
            .into_iter()
            .map(|(a, t)| (a, t.split_whitespace().collect::<Vec<_>>().join(" ")))
            .collect();
        assert_eq!(got, want, "{name}");
    }
}

#[test]
fn headerless_csv_is_plain_table() {
    for (name, format) in [
        ("csv-latin1-noheader-derived-geonames.csv", FormatId::Csv),
        ("tsv-noheader-geonames-admin1.txt", FormatId::Tsv),
    ] {
        let out = read_as(name, format, &corpus(name));
        assert_eq!(out.completeness, Completeness::Complete, "{name}");
        assert!(
            out.document.losses.is_empty(),
            "{name}: {:?}",
            out.document.losses
        );
        let (_, blocks) = tables(&out);
        let shown = data_tables(&blocks);
        assert_eq!(shown.len(), 1, "{name}");
        assert!(
            matches!(blocks[0], Block::Table { .. }),
            "{name}: no heading"
        );
        let l = layout_of(shown[0]);
        assert_eq!(l.kind, LayoutKind::Table, "{name}");
        assert_eq!(l.certainty, Certainty::Rule, "{name}");
        assert!(l.header_rows.is_empty(), "{name}");
        let numbers: Vec<String> = (1..=l.header_paths.len()).map(|n| n.to_string()).collect();
        assert_eq!(l.header_paths, numbers, "{name}");
        assert!(l.header_paths.len() >= 4, "{name}");
        let Block::Table {
            header_rows, rows, ..
        } = shown[0]
        else {
            panic!()
        };
        assert_eq!(*header_rows, 0, "{name}");
        assert_eq!(
            rows[0][0].anchor.as_ref().unwrap().0.split('!').nth(1),
            Some("A1")
        );
        let md = markdown(&out, TableStyle::Table);
        assert!(md.contains("| 1 | 2 | 3 | 4 |"), "{name}: {md}");
    }
}

#[test]
fn multi_line_field_is_no_gap() {
    for bytes in [
        &b"a,b\n1,\"x\n\ny\"\n3,4\n5,6\n"[..],
        &b"a,b\r\n1,\"x\r\n\r\ny\"\r\n3,4\r\n5,6\r\n"[..],
        &b"a,b\r\n1,2\r\n3,4\r\n5,6\r\n"[..],
        &b"a,b\r1,2\r3,4\r5,6\r"[..],
    ] {
        let out = read(bytes);
        let (_, blocks) = tables(&out);
        let shown = data_tables(&blocks);
        assert_eq!(shown.len(), 1, "{bytes:?}");
        let l = layout_of(shown[0]);
        assert_eq!(l.range, "A1:B4", "{bytes:?}");
        let Block::Table { rows, .. } = shown[0] else {
            panic!()
        };
        assert_eq!(rows[2][0].anchor.as_ref().unwrap().0, "data!A3");
        assert_eq!(text(&rows[2][0]), "3");
    }
    // A blank line between CRLF records is a gap: the second block starts one row lower.
    let out = read(b"a,b\r\n1,2\r\n3,4\r\n5,6\r\n\r\nc,d\r\n7,8\r\n9,10\r\n11,12\r\n");
    let (_, blocks) = tables(&out);
    let ranges: Vec<&str> = data_tables(&blocks)
        .iter()
        .map(|t| layout_of(t).range.as_str())
        .collect();
    assert_eq!(ranges, ["A1:B4", "A6:B9"]);
    // The same with CR-only line ends and two blank lines: one gap row.
    let out = read(b"a,b\r1,2\r3,4\r5,6\r\r\rc,d\r7,8\r9,10\r11,12\r");
    let (_, blocks) = tables(&out);
    let ranges: Vec<&str> = data_tables(&blocks)
        .iter()
        .map(|t| layout_of(t).range.as_str())
        .collect();
    assert_eq!(ranges, ["A1:B4", "A6:B9"]);
}

#[test]
fn ragged_title_and_note_lines_are_not_damaged() {
    // A title line and a note line with fewer fields are a caption and a note, not ragged
    // rows; a short row inside the table still is one.
    let mut csv = String::from("Report\nid,qty,price\n");
    for i in 1..=10 {
        if i == 4 {
            csv.push_str("4,5\n");
        } else {
            csv.push_str(&format!("{i},{},{}\n", i * 2, i * 3));
        }
    }
    csv.push_str("Source: test data\n");
    let out = read(csv.as_bytes());
    assert_eq!(
        ragged_losses(&out),
        ["record 6 (line 6): row has 2 fields, expected 3"]
    );
    let (_, blocks) = tables(&out);
    let note = blocks.iter().find_map(|b| match b {
        Block::Paragraph {
            inlines,
            role: Some(r),
            ..
        } if r == "note" => Some(inline_text(inlines)),
        _ => None,
    });
    assert_eq!(note.as_deref(), Some("Source: test data"));
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn csv_records_use_column_numbers() {
    // Without a header the fields are named by column numbers.
    let out = read(b"1,2.5,x\n2,3.5,y\n3,4.5,z\n");
    let md = markdown(&out, TableStyle::Records);
    assert!(md.contains("- Row 1 (data!A1:C1)"), "{md}");
    assert!(md.contains("  - 1: 1\n  - 2: 2.5\n  - 3: x"), "{md}");
    // With a header shorter than the rows, the fields past it are named `Column <number>`.
    let out = read(ragged_input().as_bytes());
    let (_, blocks) = tables(&out);
    let l = layout_of(data_tables(&blocks)[0]);
    assert!(
        !l.header_paths.iter().any(|p| p.contains("Column ")),
        "{:?}",
        l.header_paths
    );
    let md = markdown(&out, TableStyle::Records);
    assert!(md.contains("- Row 5 (data!A5:D5)"), "{md}");
    assert!(md.contains("  - Column 4: 9"), "{md}");
    assert!(!md.contains("Column D"), "{md}");
    let md = markdown(&out, TableStyle::Table);
    assert!(md.contains("| id | qty | price | Column 4 |"), "{md}");
    assert!(!md.contains("Column D"), "{md}");
}
