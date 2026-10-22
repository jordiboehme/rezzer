// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The layout of XLSX and XLSM sheets: regions, header bands and paths, captions, notes,
//! report blocks and row kinds, on corpus files and on small fixtures. Corpus files are
//! untrusted data, read only through the reader.

mod common;

use common::*;
use rezzer_model::{Cell, Certainty, Evidence, LayoutKind, RowKind, Severity, TableLayout};

/// A row of cells from column A: `a` text, `1` number, `=SUM(B2:B3)~5` a formula with a
/// cached number, `x@2` the cell with style 2, an empty part no cell.
fn row(n: u32, parts: &[&str]) -> String {
    let mut out = format!("<row r=\"{n}\">");
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let at = format!("{}{n}", rezzer_model::column_letter(i));
        let (value, style) = match part.rsplit_once('@') {
            Some((v, s)) if s.parse::<u32>().is_ok() => (v, format!(" s=\"{s}\"")),
            _ => (*part, String::new()),
        };
        if let Some(f) = value.strip_prefix('=') {
            let (f, v) = f.split_once('~').unwrap_or((f, ""));
            let v = if v.is_empty() {
                String::new()
            } else {
                format!("<v>{v}</v>")
            };
            out.push_str(&format!("<c r=\"{at}\"{style}><f>{f}</f>{v}</c>"));
        } else if value.parse::<f64>().is_ok() {
            out.push_str(&format!("<c r=\"{at}\"{style}><v>{value}</v></c>"));
        } else {
            out.push_str(&format!(
                "<c r=\"{at}\"{style} t=\"inlineStr\"><is><t>{value}</t></is></c>"
            ));
        }
    }
    out.push_str("</row>");
    out
}

/// Rows from row 1 down; `None` leaves a row out.
fn rows(list: &[&[&str]]) -> String {
    list.iter()
        .enumerate()
        .filter(|(_, r)| !r.is_empty())
        .map(|(i, r)| row(i as u32 + 1, r))
        .collect()
}

/// Styles with looks: 0 plain, 1 bold, 2 indent 1, 3 indent 2, 4 bold and solid fill, 5 center
/// across selection.
const LOOKS: &str = "<fonts count=\"2\"><font/><font><b/></font></fonts><fills count=\"3\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill><fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFFFFF00\"/></patternFill></fill></fills><borders count=\"1\"><border/></borders><cellXfs count=\"6\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/><xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" applyFont=\"1\"/><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" applyAlignment=\"1\"><alignment indent=\"1\"/></xf><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" applyAlignment=\"1\"><alignment indent=\"2\"/></xf><xf numFmtId=\"0\" fontId=\"1\" fillId=\"2\" borderId=\"0\" applyFont=\"1\" applyFill=\"1\"/><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" applyAlignment=\"1\"><alignment horizontal=\"centerContinuous\"/></xf></cellXfs>";

struct Region<'a> {
    layout: &'a TableLayout,
    rows: &'a [Vec<Cell>],
    header_rows: usize,
}

fn regions<'a>(doc: &'a Document, sheet: &str) -> Vec<Region<'a>> {
    section(doc, sheet)
        .iter()
        .filter_map(|b| match b {
            Block::Table {
                rows,
                header_rows,
                layout: Some(l),
                ..
            } => Some(Region {
                layout: l,
                rows,
                header_rows: *header_rows,
            }),
            _ => None,
        })
        .collect()
}

fn ranges(doc: &Document, sheet: &str) -> Vec<String> {
    regions(doc, sheet)
        .iter()
        .map(|r| r.layout.range.clone())
        .collect()
}

fn region<'a>(doc: &'a Document, sheet: &str, range: &str) -> Region<'a> {
    regions(doc, sheet)
        .into_iter()
        .find(|r| r.layout.range == range)
        .unwrap_or_else(|| panic!("no region {range} in {sheet}: {:?}", ranges(doc, sheet)))
}

/// (level, text, anchor) of the headings of a sheet.
fn headings(doc: &Document, sheet: &str) -> Vec<(u8, String, Option<String>)> {
    section(doc, sheet)
        .iter()
        .filter_map(|b| match b {
            Block::Heading {
                level,
                inlines,
                anchor,
            } => Some((
                *level,
                plain_text(inlines),
                anchor.as_ref().map(|a| a.0.clone()),
            )),
            _ => None,
        })
        .collect()
}

/// (text, anchor) of the paragraphs of a sheet with this role.
fn lines(doc: &Document, sheet: &str, role: &str) -> Vec<(String, String)> {
    section(doc, sheet)
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph {
                inlines,
                role: Some(r),
                anchor: Some(a),
            } if r == role => Some((plain_text(inlines), a.0.clone())),
            _ => None,
        })
        .collect()
}

fn rows_of(r: &Region, kind: RowKind) -> Vec<u32> {
    r.layout
        .rows
        .iter()
        .filter(|n| n.kind == kind)
        .map(|n| n.row)
        .collect()
}

fn has_name(l: &TableLayout, name: &str) -> bool {
    l.found_by
        .iter()
        .any(|e| matches!(e, Evidence::DefinedName { name: n } if n == name))
}

fn rule_ids(l: &TableLayout) -> Vec<&str> {
    l.found_by
        .iter()
        .filter_map(|e| match e {
            Evidence::Rule { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect()
}

/// The (anchor, text) of every non-empty cell the blocks of a sheet hold.
fn held(doc: &Document, sheet: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for b in section(doc, sheet) {
        match b {
            Block::Heading {
                inlines,
                anchor: Some(a),
                ..
            }
            | Block::Paragraph {
                inlines,
                anchor: Some(a),
                ..
            } => out.push((a.0.clone(), plain_text(inlines))),
            Block::Table {
                rows, role: None, ..
            } => {
                for c in rows.iter().flatten() {
                    let t = plain_text(&c.inlines);
                    if !t.is_empty() {
                        out.push((c.anchor.as_ref().unwrap().0.clone(), t));
                    }
                }
            }
            _ => {}
        }
    }
    out.sort();
    out
}

/// Fetched corpus files may be absent; the test then says so and passes.
fn corpus_or_skip(name: &str) -> Option<ReadOutcome> {
    let path = format!(
        "{}/../../../tests/corpus/office/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::path::Path::new(&path).exists() {
        Some(corpus(name))
    } else {
        eprintln!("{name} is absent; skipped");
        None
    }
}

#[test]
fn census_two_row_header_paths() {
    let Some(out) = corpus_or_skip("NST-EST2023-POP.xlsx") else {
        return;
    };
    let doc = &out.document;
    let sheet = "NST-EST2023-POP";
    let q = "'NST-EST2023-POP'";
    let r = region(doc, sheet, "A1:F68");
    let l = r.layout;
    assert_eq!(l.kind, LayoutKind::Table);
    assert_eq!(l.header_rows, [3, 4]);
    assert_eq!(r.header_rows, 2);
    assert_eq!(
        l.header_paths[3],
        "Population Estimate (as of July 1) / 2021"
    );
    assert!(has_name(l, "_NST01"), "{:?}", l.found_by);
    let h = headings(doc, sheet);
    assert_eq!(h[0].0, 3);
    assert_eq!(h[0].2.as_deref(), Some(format!("{q}!A2").as_str()));
    assert!(
        h[0].1
            .starts_with("Annual Estimates of the Resident Population")
    );
    let captions = lines(doc, sheet, "caption");
    assert_eq!(captions.len(), 1);
    assert_eq!(captions[0].1, format!("{q}!A1"));
    let notes: Vec<String> = lines(doc, sheet, "note").into_iter().map(|n| n.1).collect();
    let expected: Vec<String> = (63..=68).map(|n| format!("{q}!A{n}")).collect();
    assert_eq!(notes, expected);
    assert!(l.rows.iter().all(|n| n.parent.is_none()), "{:?}", l.rows);
}

#[test]
fn eia_group_rows_and_paths() {
    let Some(out) = corpus_or_skip("table_1_01.xlsx") else {
        return;
    };
    let doc = &out.document;
    let sheet = "Table_1_01";
    let all = regions(doc, sheet);
    assert_eq!(all.len(), 1, "{:?}", ranges(doc, sheet));
    let r = &all[0];
    assert_eq!(r.layout.header_rows, [3, 4]);
    assert_eq!(
        r.layout.header_paths[1],
        "Generation at Utility Scale Facilities / Coal"
    );
    assert_eq!(rows_of(r, RowKind::Group), [5, 16, 29, 42, 50, 54]);
    let h = headings(doc, sheet);
    assert_eq!(h[0].2.as_deref(), Some("Table_1_01!A1"));
    assert_eq!(
        lines(doc, sheet, "caption")
            .iter()
            .map(|c| c.1.as_str())
            .collect::<Vec<_>>(),
        ["Table_1_01!A2"]
    );
}

#[test]
fn two_excel_tables_on_one_sheet() {
    let out = corpus("table-multiple.xlsx");
    let all = regions(&out.document, "Sheet3");
    let keys: Vec<&str> = all.iter().map(|r| r.layout.key.as_str()).collect();
    assert_eq!(keys, ["table:Sales_Alice", "table:Sales_Bob"]);
    assert!(all.iter().all(|r| r.layout.certainty == Certainty::Exact));
}

#[test]
fn excel_table_off_a1() {
    let out = corpus("simple-table-named-range.xlsx");
    let r = region(&out.document, "Sheet1", "B3:D5");
    assert!(
        r.rows.iter().all(|row| row.len() == 3),
        "no padding column A"
    );
    assert_eq!(r.rows[0][0].anchor.as_ref().unwrap().0, "Sheet1!B3");
    assert_eq!(r.layout.certainty, Certainty::Exact);
}

#[test]
fn side_by_side_regions_by_column_gap() {
    let out = corpus("CONVERT.xlsx");
    let all = ranges(&out.document, "Weight");
    assert!(all.contains(&"A1:C16".to_string()), "{all:?}");
    assert!(all.contains(&"F2:G16".to_string()), "{all:?}");
}

#[test]
fn blocks_off_a1() {
    let out = corpus("55906-MultiSheetRefs.xlsx");
    let all = ranges(&out.document, "Sheet1");
    assert!(all.contains(&"A3:E6".to_string()), "{all:?}");
    assert!(all.contains(&"H3:I6".to_string()), "{all:?}");
}

/// The regions of every XLOOKUP sheet, written after a review by eye of the cells (see the
/// task report); every non-empty cell is in exactly one block.
#[test]
fn many_small_tables_per_sheet() {
    let out = corpus("XLOOKUP.xlsx");
    let doc = &out.document;
    // From the cells by the gap rule, then rule T2: a title at most 2 empty rows above a
    // region and inside its columns joins it (Errors A7, ExactMatch A1 and A14, the A1 and
    // A16 titles of ReturnSmaller and ReturnLarger). A text beside a region stays (P1, I17,
    // RowArray A1 and A14).
    let expected: &[(&str, &[&str])] = &[
        (
            "Errors",
            &[
                "A1:B4", "L1:O6", "E2:F2", "H3:I3", "H5:I6", "A7:D12", "A14:B15", "D14:E15",
            ],
        ),
        (
            "ExactMatch",
            &[
                "A1:B8", "P1", "R1:Y20", "D3:G11", "I3:I4", "A14:B21", "D17:F25", "I17",
            ],
        ),
        (
            "LargerRows",
            &["B1:F2", "Q1:U2", "A4:K5", "P4:Z5", "A8:K9", "P8:Z9"],
        ),
        (
            "MultipleElements",
            &["A1:B13", "D1:F11", "I1:J13", "L1:N11"],
        ),
        (
            "ReturnLarger",
            &[
                "A1:B10", "D3:H14", "A16:B23", "D18:F28", "H18:J28", "D33:F43", "H33:J43",
                "A34:B38",
            ],
        ),
        (
            "ReturnSmaller",
            &[
                "A1:B10", "D3:G13", "L3:L4", "A16:B23", "D18:F28", "H18:J28", "D33:F43", "H33:J43",
                "A34:B38",
            ],
        ),
        (
            "RowArray",
            &["A1", "C1:H2", "C4:K7", "A14", "C14:H15", "C17:L20"],
        ),
        (
            "SmallerRow",
            &["A1", "D1:H2", "S1:W2", "C4:M5", "R4:AB5", "C8:M9", "R8:AB9"],
        ),
        (
            "WilcardRows",
            &["A1:G2", "L1:Q2", "T1:U2", "A4:G6", "L4:M5", "S4:T5"],
        ),
        (
            "Wildcards",
            &["A1:B7", "D1:F7", "A12:B17", "D12:E13", "D19:E20", "A20:B21"],
        ),
    ];
    for (sheet, want) in expected {
        assert_eq!(ranges(doc, sheet), *want, "{sheet}");
    }
    for b in &doc.body {
        let Block::Section {
            title: Some(sheet), ..
        } = b
        else {
            continue;
        };
        let cells = held(doc, sheet);
        let mut seen = cells.clone();
        seen.dedup_by(|a, b| a.0 == b.0);
        assert_eq!(seen.len(), cells.len(), "{sheet}: a cell is written twice");
    }
}

#[test]
fn tunnel_report_sections() {
    let out = corpus("tunnel_design_calculations.xlsx");
    let doc = &out.document;
    let sheet = "Design_Basis";
    let h: Vec<Option<String>> = headings(doc, sheet)
        .into_iter()
        .filter(|h| h.0 == 3)
        .map(|h| h.2)
        .collect();
    let rows: Vec<String> = [1, 4, 11, 18, 27]
        .iter()
        .map(|n| format!("Design_Basis!A{n}"))
        .collect();
    for r in &rows {
        assert!(h.contains(&Some(r.clone())), "{r} in {h:?}");
    }
    let kv = region(doc, sheet, "A4:C9");
    assert_eq!(kv.layout.kind, LayoutKind::KeyValue);
    assert_eq!(kv.layout.data.as_deref(), Some("A5:B9"));
    let pairs = kv
        .rows
        .iter()
        .filter(|r| r.iter().any(|c| !plain_text(&c.inlines).is_empty()))
        .count();
    assert_eq!(pairs, 5);
    let text = regions(doc, sheet)
        .into_iter()
        .find(|r| r.layout.kind == LayoutKind::Text && r.layout.data.as_deref() == Some("A19:A25"));
    assert!(text.is_some(), "{:?}", ranges(doc, sheet));
    // No merged text twice.
    let cells = held(doc, sheet);
    let mut texts: Vec<&str> = cells.iter().map(|c| c.1.as_str()).collect();
    let n = texts.len();
    texts.sort();
    texts.dedup();
    assert_eq!(texts.len(), n, "a text is written twice: {cells:?}");
}

#[test]
fn tunnel_side_table() {
    let out = corpus("tunnel_design_calculations.xlsx");
    // The merged title in A47:C47 stands right on top of the side table: it is its title.
    let r = region(&out.document, "Structural_Lining", "A47:F72");
    assert_eq!(r.layout.kind, LayoutKind::Table);
    assert_eq!(r.layout.data.as_deref(), Some("E49:F72"));
    assert_eq!(r.layout.header_rows, [48]);
    assert_eq!(r.layout.header_paths, ["t (m)", "FOS"]);
}

#[test]
fn tunnel_jet_fan_block_reads_count_units() {
    // `Pa`, `N` and `nos`: a count word beside real unit symbols makes a unit column, so the
    // block is a label and value list.
    let out = corpus("tunnel_design_calculations.xlsx");
    let doc = &out.document;
    let kv = region(doc, "Ventilation", "A36:C39");
    assert_eq!(kv.layout.kind, LayoutKind::KeyValue);
    assert_eq!(kv.layout.data.as_deref(), Some("A37:C39"));
    let md = rezzer_render::render(
        doc,
        out.completeness,
        &rezzer_render::RenderOptions {
            sections: Some(vec![rezzer_render::OutputSection::Content]),
            ..rezzer_render::RenderOptions::default()
        },
    );
    assert!(
        md.contains("- No. of jet fans required: 1 nos (B39)\n"),
        "{md}"
    );
}

/// The brief expected the regions A1:F3, A5:F15, D17:F17 and A18:F36. Row 2 holds cells in A
/// to C only, so D3:F3 touches A1:F2 only at a corner and is a region of its own (rule G,
/// spec 6.2). The same holds for D27, which touches the block of rows 18 to 26 (cells in A to C
/// on row 26) only at a corner; D27 then joins A28:F36 as its title (rule T2).
/// The merged sub-header that spreads is E28:F28, on the header row of A27:F36.
#[test]
fn merged_subheader_spreads() {
    let out = corpus("DataValidations-49244.xlsx");
    let doc = &out.document;
    assert_eq!(
        ranges(doc, "Sheet1"),
        ["A1:F2", "D3:F3", "A5:F15", "D17:F17", "A18:F26", "A27:F36"]
    );
    let r = region(doc, "Sheet1", "A27:F36");
    assert_eq!(r.layout.header_rows, [28]);
    let paths = &r.layout.header_paths;
    assert_eq!(paths[4], "Cells Referenced in Formulae");
    assert_eq!(paths[5], "Cells Referenced in Formulae");
    let merged = r.rows[0]
        .iter()
        .find(|c| c.col_span == 2)
        .expect("the merge");
    assert_eq!(merged.anchor.as_ref().unwrap().0, "Sheet1!E28");
}

/// merged_range.xlsx has vertical merges in rows 1 and 2, but every cell is plain text with no
/// bold or fill: rule H1 finds no band in an all-text region and rule H3 needs a style signal,
/// so the region stays a grid and says so (the brief expected header rows 1 to 2). The merges
/// are kept as spans. A bold two-row header with the same merges shows the paths.
#[test]
fn vertical_merges_in_header_count_once() {
    let out = corpus("merged_range.xlsx");
    let doc = &out.document;
    let r = region(doc, "Sheet1", "A1:H4");
    assert_eq!(r.layout.kind, LayoutKind::Grid);
    assert_eq!(r.layout.unsure, ["no header found"]);
    assert_eq!(r.rows[0][2].row_span, 2);
    assert_eq!(r.rows[0][2].col_span, 2);

    let s = ws(
        "",
        &rows(&[
            &["Name@1", "Sales@1", ""],
            &["", "2024@1", "2025@1"],
            &["Ann", "1", "2"],
            &["Bob", "3", "4"],
        ]),
        "<mergeCells count=\"2\"><mergeCell ref=\"A1:A2\"/><mergeCell ref=\"B1:C1\"/></mergeCells>",
    );
    let out = read(&Book::default().styles(LOOKS).sheet("S", &s));
    let r = &regions(&out.document, "S")[0];
    assert_eq!(r.layout.header_rows, [1, 2]);
    assert_eq!(
        r.layout.header_paths,
        ["Name", "Sales / 2024", "Sales / 2025"]
    );
}

/// A known limit, stated in the sheetlayout README: header rows that stand above and right
/// of the corner labels of a cross table become a table of their own. In ex012 row 37 holds
/// T and T_K in C and D only, so the rows 35 and 36 (P, P_Mpa) over E:AK touch the block
/// below only at the corner D37/E36 (rule G, spec 6.2): a header-less table E35:AK36, and the
/// table C37:AK58 with header row 37 by type change, whose data columns get no P path. Every
/// cell is still kept once.
#[test]
fn cross_table_header_above_corner_labels_is_its_own_table() {
    let out = corpus("ex012.PVT_z_factor.xlsm");
    let doc = &out.document;
    let top = region(doc, "z", "E35:AK36");
    assert_eq!(top.layout.kind, LayoutKind::Table);
    assert!(top.layout.header_rows.is_empty());
    let table = region(doc, "z", "C37:AK58");
    assert_eq!(table.layout.header_rows, [37]);
    assert_eq!(&table.layout.header_paths[..2], ["T", "T_K"]);
    let cells = held(doc, "z");
    let mut anchors: Vec<&str> = cells.iter().map(|c| c.0.as_str()).collect();
    anchors.dedup();
    assert_eq!(anchors.len(), cells.len(), "a cell is written twice");
    for a in ["z!E35", "z!E36", "z!F35", "z!C37", "z!D37", "z!E38"] {
        assert!(anchors.contains(&a), "{a} is kept");
    }
}

#[test]
fn fill_everywhere_is_no_header_signal() {
    for (file, sheet) in [
        ("ex051.MF_pipeline.xlsx", "Лист1"),
        ("ex054.MF_friction_factor.xlsm", "Лист1"),
    ] {
        let out = corpus(file);
        for r in regions(&out.document, sheet) {
            assert!(
                !rule_ids(r.layout).contains(&"H2"),
                "{file} {}: {:?}",
                r.layout.range,
                r.layout.found_by
            );
        }
    }
}

#[test]
fn outline_levels_give_parents() {
    let s = ws(
        "",
        &[
            row(1, &["Item", "Qty"]),
            row(2, &["Fruit", "5"]),
            "<row r=\"3\" outlineLevel=\"1\"><c r=\"A3\" t=\"inlineStr\"><is><t>Apple</t></is></c><c r=\"B3\"><v>2</v></c></row>".to_string(),
            "<row r=\"4\" outlineLevel=\"1\"><c r=\"A4\" t=\"inlineStr\"><is><t>Pear</t></is></c><c r=\"B4\"><v>3</v></c></row>".to_string(),
            row(5, &["Nuts", "1"]),
        ]
        .concat(),
        "",
    );
    let out = read(&Book::default().sheet("S", &s));
    let r = &regions(&out.document, "S")[0];
    let parent = |n: u32| {
        r.layout
            .rows
            .iter()
            .find(|x| x.row == n)
            .and_then(|x| x.parent)
    };
    assert_eq!(parent(3), Some(2));
    assert_eq!(parent(4), Some(2));
    assert_eq!(parent(5), None);
    // GroupTest.xlsx is one text column: its regions are text blocks and keep every cell.
    let out = corpus("GroupTest.xlsx");
    let all = regions(&out.document, "Sheet1");
    assert!(!all.is_empty());
    assert!(
        all.iter().all(|r| r.layout.kind == LayoutKind::Text),
        "{:?}",
        all.iter()
            .map(|r| (r.layout.range.clone(), r.layout.kind))
            .collect::<Vec<_>>()
    );
    assert_eq!(held(&out.document, "Sheet1").len(), 25);
}

/// The location of each pivot, from the summary table the reader writes from the pivot
/// definition.
fn pivot_locations(doc: &Document, sheet: &str) -> Vec<(String, String)> {
    section(doc, sheet)
        .iter()
        .filter_map(|b| match b {
            Block::Table {
                role: Some(r),
                rows,
                ..
            } if r == "pivot" => Some(rows),
            _ => None,
        })
        .flat_map(|rows| {
            rows.iter().skip(1).map(|r| {
                (
                    plain_text(&r[0].inlines),
                    plain_text(&r[1].inlines).replace('$', ""),
                )
            })
        })
        .collect()
}

#[test]
fn pivot_location_is_exact() {
    for (file, sheets) in [
        (
            "pivots.xlsx",
            &["PivotSheet1", "PivotSheet2", "PivotSheet3", "PivotSheet4"][..],
        ),
        ("ExcelPivotTableSample.xlsx", &["Tabelle2", "Tabelle3"][..]),
    ] {
        let out = corpus(file);
        let doc = &out.document;
        for sheet in sheets {
            let locations = pivot_locations(doc, sheet);
            assert!(!locations.is_empty(), "{file} {sheet}");
            for (name, location) in locations {
                let r = regions(doc, sheet)
                    .into_iter()
                    .find(|r| r.layout.key == format!("pivot:{sheet}!{name}"))
                    .unwrap_or_else(|| panic!("{file} {sheet} {name}: {:?}", ranges(doc, sheet)));
                assert_eq!(r.layout.certainty, Certainty::Exact);
                assert_eq!(r.layout.range, location, "{file} {sheet} {name}");
            }
        }
    }
}

/// no-header.xlsx holds `a | 0` and `b | 1` in A3:B4: two columns, different labels, values.
/// Spec 6.7 runs the label and value rule (step 6e) before the header-less table (step 6g),
/// so it is a `key_value` block, not the plain table the brief expected; no loss either way.
/// A sheet of numbers only is the header-less table of decision 16.
#[test]
fn no_header_file_is_key_value_and_numbers_are_plain_table() {
    let out = corpus("no-header.xlsx");
    let doc = &out.document;
    let all = regions(doc, "Feuille 1");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].layout.kind, LayoutKind::KeyValue);
    assert_eq!(all[0].layout.range, "A3:B4");
    assert!(doc.losses.is_empty(), "{:?}", doc.losses);

    let s = ws("", &rows(&[&[], &["", "1", "2"], &["", "3", "4"]]), "");
    let out = read(&Book::default().sheet("S", &s));
    let doc = &out.document;
    let all = regions(doc, "S");
    assert_eq!(all.len(), 1);
    let l = all[0].layout;
    assert_eq!(l.kind, LayoutKind::Table);
    assert_eq!(l.certainty, Certainty::Rule);
    assert!(l.header_rows.is_empty());
    assert_eq!(l.header_paths, ["B", "C"]);
    assert_eq!(all[0].header_rows, 0);
    assert!(doc.losses.is_empty(), "{:?}", doc.losses);
    assert!(headings(doc, "S").is_empty());
}

#[test]
fn table_key_stable_when_other_table_changes() {
    let lower: Vec<String> = vec![
        row(8, &["Name", "Score"]),
        row(9, &["Ann", "3"]),
        row(10, &["Bob", "4"]),
    ];
    let first = [
        rows(&[&["City", "Pop"], &["Rome", "9"], &["Oslo", "2"]]),
        lower.concat(),
    ]
    .concat();
    let second = [
        rows(&[
            &["Town", "Area", "Pop"],
            &["Graz", "1", "2"],
            &["Linz", "3", "4"],
            &["Wels", "5", "6"],
        ]),
        lower.concat(),
    ]
    .concat();
    let key = |xml: String| {
        let out = read(&Book::default().sheet("S", &ws("", &xml, "")));
        let all = regions(&out.document, "S");
        assert_eq!(all.len(), 2, "{:?}", ranges(&out.document, "S"));
        all[1].layout.key.clone()
    };
    assert_eq!(key(first), "S!A8");
    assert_eq!(key(second), "S!A8");
}

#[test]
fn cell_under_merge_is_not_lost() {
    let s = ws(
        "",
        &rows(&[
            &["Name", "Q1", "Q2"],
            &["Ann", "5", "under"],
            &["Bob", "3", "4"],
        ]),
        "<mergeCells count=\"1\"><mergeCell ref=\"B2:C2\"/></mergeCells>",
    );
    let out = read(&Book::default().sheet("S", &s));
    let doc = &out.document;
    let loss = doc
        .losses
        .iter()
        .find(|l| l.location == "S!C2")
        .unwrap_or_else(|| panic!("{:#?}", doc.losses));
    assert_eq!(loss.why, LossReason::Unsupported);
    assert_eq!(loss.severity, Severity::Info);
    assert!(
        loss.what.contains("under") && loss.what.contains("B2:C2"),
        "{}",
        loss.what
    );
    assert!(!held(doc, "S").iter().any(|c| c.1 == "under"));
    let r = &regions(doc, "S")[0];
    assert_eq!(r.rows[1].len(), 2);
    assert_eq!(r.rows[1][1].col_span, 2);
    assert_eq!(r.rows[1][1].anchor.as_ref().unwrap().0, "S!B2");
}

#[test]
fn subtotal_formula_rows_marked() {
    let subtotals = rows(&[
        &["Item", "Sales"],
        &["a", "1"],
        &["b", "2"],
        &["Sum ab", "=SUBTOTAL(9,B2:B3)~3"],
        &["c", "4"],
        &["d", "5"],
        &["Sum cd", "=SUBTOTAL(9,B5:B6)~9"],
        &["All", "=SUBTOTAL(9,B2:B7)~12"],
    ]);
    let sums = rows(&[
        &["Item", "Sales"],
        &["a", "1"],
        &["b", "2"],
        &["All", "=SUM(B2:B3)~3"],
    ]);
    let out = read(
        &Book::default()
            .sheet("A", &ws("", &subtotals, ""))
            .sheet("B", &ws("", &sums, "")),
    );
    let doc = &out.document;
    let a = &regions(doc, "A")[0];
    assert_eq!(rows_of(a, RowKind::Subtotal), [4, 7]);
    assert_eq!(rows_of(a, RowKind::Total), [8]);
    let b = &regions(doc, "B")[0];
    assert_eq!(rows_of(b, RowKind::Total), [4]);
}

fn table_part(name: &str, range: &str, extra: &str, columns: &[&str]) -> String {
    let cols: String = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("<tableColumn id=\"{}\" name=\"{c}\"/>", i + 1))
        .collect();
    format!(
        "<table xmlns=\"{MAIN}\" id=\"1\" name=\"{name}\" displayName=\"{name}\" ref=\"{range}\"{extra}><tableColumns count=\"{}\">{cols}</tableColumns></table>",
        columns.len()
    )
}

#[test]
fn excel_table_totals_and_hidden_header() {
    let totals = rows(&[
        &["Name", "Sales"],
        &["Ann", "1"],
        &["Bob", "2"],
        &["Total", "=SUBTOTAL(109,B2:B3)~3"],
    ]);
    let hidden = rows(&[&["Ann", "1"], &["Bob", "2"]]);
    let out = read(
        &Book::default()
            .sheet("T", &ws("", &totals, ""))
            .related(
                "table",
                "xl/tables/table1.xml",
                &table_part("Sums", "A1:B4", " totalsRowCount=\"1\"", &["Name", "Sales"]),
            )
            .sheet("H", &ws("", &hidden, ""))
            .related(
                "table",
                "xl/tables/table2.xml",
                &table_part("Bare", "A1:B2", " headerRowCount=\"0\"", &["Name", "Sales"]),
            ),
    );
    let doc = &out.document;
    let t = &regions(doc, "T")[0];
    assert_eq!(t.layout.key, "table:Sums");
    assert_eq!(rows_of(t, RowKind::Total), [4]);
    let h = &regions(doc, "H")[0];
    assert_eq!(h.layout.key, "table:Bare");
    assert!(h.layout.hidden_header);
    assert!(h.layout.header_rows.is_empty());
    assert_eq!(h.layout.header_paths, ["Name", "Sales"]);
    assert_eq!(h.header_rows, 0);
}

#[test]
fn print_titles_and_frozen_panes_fixture() {
    let s = ws(
        "<sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"2\" topLeftCell=\"A3\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews>",
        &rows(&[
            &["Region", "Sales", "Sales"],
            &["", "2024", "2025"],
            &["North", "1", "2"],
            &["South", "3", "4"],
        ]),
        "",
    );
    let out = read(&Book::default().sheet("S", &s).names(
        "<definedName name=\"_xlnm.Print_Titles\" localSheetId=\"0\">S!$1:$2</definedName>",
    ));
    let r = &regions(&out.document, "S")[0];
    assert_eq!(r.layout.header_rows, [1, 2]);
    assert!(
        r.layout.found_by.contains(&Evidence::PrintTitles),
        "{:?}",
        r.layout.found_by
    );
    assert!(
        r.layout.found_by.contains(&Evidence::FrozenPanes),
        "{:?}",
        r.layout.found_by
    );
    assert_eq!(r.layout.header_paths[1], "Sales / 2024");
}

#[test]
fn indent_hierarchy_by_style() {
    let s = ws(
        "",
        &rows(&[
            &["Item@1", "Value@1"],
            &["Region A", "3"],
            &["Town a1@2", "1"],
            &["Town a2@2", "2"],
            &["Region B", "5"],
            &["Town b1@2", "5"],
        ]),
        "",
    );
    let out = read(&Book::default().styles(LOOKS).sheet("S", &s));
    let r = &regions(&out.document, "S")[0];
    assert_eq!(r.layout.header_rows, [1]);
    let parent = |n: u32| {
        r.layout
            .rows
            .iter()
            .find(|x| x.row == n)
            .and_then(|x| x.parent)
    };
    assert_eq!(parent(3), Some(2));
    assert_eq!(parent(4), Some(2));
    assert_eq!(parent(6), Some(5));
    assert_eq!(parent(5), None);
}

#[test]
fn wide_region_is_cut_at_the_column_cap() {
    let cells: String = (0..1002u32)
        .map(|c| {
            format!(
                "<c r=\"{}1\"><v>{c}</v></c>",
                rezzer_model::column_letter(c as usize)
            )
        })
        .collect();
    let s = ws("", &format!("<row r=\"1\">{cells}</row>"), "");
    let out = read(&Book::default().sheet("S", &s));
    let doc = &out.document;
    let r = &regions(doc, "S")[0];
    assert_eq!(r.layout.range, "A1:ALN1");
    assert_eq!(r.rows[0].len(), 1000);
    assert_eq!(r.layout.header_paths.len(), 1000);
    assert!(has_loss(
        doc,
        LossReason::Limit,
        "S!A1:ALN1",
        "columns ALM to ALN of Table A1:ALN1 not shown"
    ));
    assert_eq!(out.completeness, Completeness::Partial);
}

#[test]
fn center_across_spreads_a_header() {
    // B1 is centered across B1:C1; C1 is an empty styled cell with the same alignment.
    let s = ws(
        "",
        &[
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Region</t></is></c><c r=\"B1\" s=\"5\" t=\"inlineStr\"><is><t>Sales</t></is></c><c r=\"C1\" s=\"5\"/></row>".to_string(),
            row(2, &["", "Q1", "Q2"]),
            row(3, &["North", "1", "2"]),
            row(4, &["South", "3", "4"]),
        ]
        .concat(),
        "",
    );
    let out = read(&Book::default().styles(LOOKS).sheet("S", &s));
    let r = &regions(&out.document, "S")[0];
    assert_eq!(r.layout.header_rows, [1, 2]);
    assert_eq!(
        r.layout.header_paths,
        ["Region", "Sales / Q1", "Sales / Q2"]
    );
}

#[test]
fn merges_without_values_leave_the_sheet_empty() {
    let s = ws(
        "",
        "",
        "<mergeCells count=\"2\"><mergeCell ref=\"A1:C3\"/><mergeCell ref=\"A5:B9\"/></mergeCells>",
    );
    let out = read(&Book::default().sheet("S", &s));
    let doc = &out.document;
    assert!(regions(doc, "S").is_empty());
    assert_eq!(paragraphs(section(doc, "S")), ["The sheet is empty."]);
    assert!(doc.losses.is_empty(), "{:?}", doc.losses);
}

// ---------------------------------------------------------------------------------------
// ODS
// ---------------------------------------------------------------------------------------

/// One ODS row: `a` text, `1` number, `x@b` the cell with the automatic style `b`, an empty
/// part an empty cell.
fn ods_row(parts: &[&str]) -> String {
    let mut out = String::from("<table:table-row>");
    for part in parts {
        let (value, style) = match part.rsplit_once('@') {
            Some((v, s)) => (v, format!(" table:style-name=\"{s}\"")),
            None => (*part, String::new()),
        };
        if value.is_empty() {
            out.push_str(&format!("<table:table-cell{style}/>"));
        } else if value.parse::<f64>().is_ok() {
            out.push_str(&format!(
                "<table:table-cell{style} office:value-type=\"float\" office:value=\"{value}\"><text:p>{value}</text:p></table:table-cell>"
            ));
        } else {
            out.push_str(&format!(
                "<table:table-cell{style} office:value-type=\"string\"><text:p>{value}</text:p></table:table-cell>"
            ));
        }
    }
    out.push_str("</table:table-row>");
    out
}

/// A bold automatic cell style named `b`.
const ODS_BOLD: &str = "<style:style style:name=\"b\" style:family=\"table-cell\"><style:text-properties fo:font-weight=\"bold\"/></style:style>";

fn ods_sheet(name: &str, rows: &str) -> String {
    format!("<table:table table:name=\"{name}\">{rows}</table:table>")
}

fn has_db_range(l: &TableLayout, name: &str) -> bool {
    l.found_by
        .iter()
        .any(|e| matches!(e, Evidence::DatabaseRange { name: n } if n == name))
}

#[test]
fn ods_database_range_is_exact() {
    let out = corpus("database.ods");
    let doc = &out.document;
    // The anonymous sheet range has no name of its own: the range key, no caption.
    let r = region(doc, "Sheet1", "A1:D6");
    assert_eq!(r.layout.key, "Sheet1!A1");
    assert_eq!(r.layout.certainty, Certainty::Exact);
    assert!(has_db_range(r.layout, "__Anonymous_Sheet_DB__0"));
    assert_eq!(r.layout.header_rows, [1]);
    assert_eq!(r.layout.header_paths, ["Col1", "Col2", "Col3", "Col4"]);
    // `display-filter-buttons`: the range is an autofilter too, shown as evidence.
    assert!(r.layout.found_by.contains(&Evidence::Autofilter));
    let named = region(doc, "Sheet2", "A1:C7");
    assert!(!named.layout.found_by.contains(&Evidence::Autofilter));
    assert_eq!(named.layout.key, "dbrange:database1");
    assert_eq!(named.layout.certainty, Certainty::Exact);
    assert_eq!(named.layout.header_rows, [1]);
    for sheet in ["Sheet1", "Sheet2"] {
        assert!(
            !headings(doc, sheet)
                .iter()
                .any(|h| h.1.contains("__Anonymous")),
            "{:?}",
            headings(doc, sheet)
        );
    }

    let out = corpus("cool15769_minimal_DB_ranges.ods");
    let doc = &out.document;
    let key_of = |range: &str| region(doc, "Sheet1", range).layout.key.clone();
    assert_eq!(key_of("C4"), "dbrange:singleheader");
    assert_eq!(key_of("A4"), "dbrange:singlenoheader");
    assert_eq!(key_of("E4:E5"), "dbrange:twowheader");
    assert_eq!(key_of("G4:G5"), "dbrange:twowheadertotals");
    assert_eq!(region(doc, "Sheet1", "C4").layout.header_rows, [4]);
    // `table:contains-header="false"`: no header row, the column letter names the column.
    let no_header = region(doc, "Sheet1", "A4");
    assert!(no_header.layout.header_rows.is_empty());
    assert_eq!(no_header.layout.header_paths, ["A"]);
    assert_eq!(region(doc, "Sheet1", "E4:E5").layout.header_rows, [4]);
    // `calcext:contains-footer="true"`: the last row is the total row.
    let totals = region(doc, "Sheet1", "G4:G5");
    assert_eq!(totals.layout.header_rows, [4]);
    assert_eq!(rows_of(&totals, RowKind::Total), [5]);
    for range in ["C4", "A4", "E4:E5", "G4:G5"] {
        assert_eq!(
            region(doc, "Sheet1", range).layout.certainty,
            Certainty::Exact
        );
    }

    let out = corpus("autofilter-colbutton.ods");
    let r = region(&out.document, "Sheet1", "B1:C4");
    assert_eq!(r.layout.key, "Sheet1!B1");
    assert_eq!(r.layout.certainty, Certainty::Exact);
    assert_eq!(r.layout.header_paths, ["A", "B"]);
    assert!(r.layout.found_by.contains(&Evidence::Autofilter));
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("overlaps")),
        "{:#?}",
        out.document.losses
    );
}

#[test]
fn ods_merges_are_read() {
    // A2:A3 is merged; A3 is an empty covered cell.
    let out = corpus("merged_cells.ods");
    let doc = &out.document;
    let r = &regions(doc, "Sheet1")[0];
    let a2 = r
        .rows
        .iter()
        .flatten()
        .find(|c| c.anchor.as_ref().is_some_and(|a| a.0 == "Sheet1!A2"))
        .unwrap_or_else(|| panic!("{:?}", r.rows));
    assert_eq!((a2.row_span, a2.col_span), (2, 1));
    assert!(
        !r.rows
            .iter()
            .flatten()
            .any(|c| c.anchor.as_ref().is_some_and(|a| a.0 == "Sheet1!A3"))
    );

    // The covered cell A3 holds "a3": it is not shown and a loss says so.
    let out = corpus("covered.ods");
    let doc = &out.document;
    let loss = doc
        .losses
        .iter()
        .find(|l| l.location == "sheet1!A3")
        .unwrap_or_else(|| panic!("{:#?}", doc.losses));
    assert_eq!(loss.why, LossReason::Unsupported);
    assert!(
        loss.what.contains("a3") && loss.what.contains("A2:A3"),
        "{}",
        loss.what
    );
    assert!(!held(doc, "sheet1").iter().any(|c| c.1 == "a3"));
    assert!(held(doc, "sheet1").iter().any(|c| c.1 == "a2"));

    // A region whose only value is the merged text "a2" under A2:A3 is no empty table, and
    // no "no header found" either.
    assert!(
        !doc.losses
            .iter()
            .any(|l| l.what.contains("no header found")),
        "{:#?}",
        doc.losses
    );
    for r in regions(doc, "sheet1") {
        assert!(
            r.rows
                .iter()
                .flatten()
                .all(|c| !plain_text(&c.inlines).is_empty()),
            "{:?}",
            r.rows
        );
    }

    // Large merges: each merged text is a title over its merge (ruling P4-AQ): no table rows,
    // no unsure or limit loss, the heading anchored at the merge's top-left cell.
    let out = corpus("merged.ods");
    let doc = &out.document;
    let mut all = held(doc, "Sheet1");
    all.extend(held(doc, "Sheet2"));
    all.sort();
    assert_eq!(
        all,
        [
            ("Sheet1!A1".to_string(), "merged".to_string()),
            ("Sheet1!D16".into(), "yet another merged".into()),
            ("Sheet1!H3".into(), "another merged".into()),
            ("Sheet2!E4".into(), "merged on another sheet".into()),
        ]
    );
    assert_eq!(ranges(doc, "Sheet1"), ["A1:F12", "H3:J13", "D16:H24"]);
    assert_eq!(ranges(doc, "Sheet2"), ["E4:G16"]);
    for sheet in ["Sheet1", "Sheet2"] {
        for r in regions(doc, sheet) {
            assert_eq!(r.layout.kind, LayoutKind::Title, "{}", r.layout.range);
            assert!(r.rows.is_empty(), "{}: {:?}", r.layout.range, r.rows);
            assert!(r.layout.unsure.is_empty());
        }
    }
    assert_eq!(
        headings(doc, "Sheet1")
            .iter()
            .map(|h| h.2.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["Sheet1!A1", "Sheet1!H3", "Sheet1!D16"]
    );
    assert!(
        doc.losses
            .iter()
            .all(|l| l.why != LossReason::Uncertain && l.why != LossReason::Limit),
        "{:#?}",
        doc.losses
    );
    let not_read = doc
        .losses
        .iter()
        .find(|l| l.what.contains("not read for ODS"))
        .unwrap();
    assert!(!not_read.what.contains("merged"), "{}", not_read.what);
}

#[test]
fn ods_pivot_location_is_exact() {
    let out = corpus("pivot-table-basic.ods");
    let doc = &out.document;
    let r = region(doc, "Pivot", "A1:E10");
    assert_eq!(r.layout.key, "pivot:Pivot!DataPilot1");
    assert_eq!(r.layout.certainty, Certainty::Exact);
    // ODS gives the target range only: no header band, no label columns and no total row
    // from words like "Total Result".
    assert!(r.layout.header_rows.is_empty());
    assert_eq!(r.layout.label_cols, 0);
    assert!(rows_of(&r, RowKind::Total).is_empty());
    let loss = doc
        .losses
        .iter()
        .find(|l| l.location == "Pivot" && l.what.contains("DataPilot1"))
        .unwrap_or_else(|| panic!("{:#?}", doc.losses));
    assert_eq!(loss.why, LossReason::Unsupported);
    assert_eq!(loss.severity, Severity::Info);
    let not_read = doc
        .losses
        .iter()
        .find(|l| l.what.contains("not read for ODS"))
        .unwrap();
    assert!(!not_read.what.contains("pivot"), "{}", not_read.what);
    assert!(!not_read.what.contains("database"), "{}", not_read.what);
}

/// outline.ods has one cell beside its row groups, so no row can have a parent there: the
/// parents are shown on a fixture with a label column inside a table (as for GroupTest.xlsx).
#[test]
fn ods_outline_levels_give_parents() {
    let out = corpus("outline.ods");
    assert_eq!(
        held(&out.document, "OutlineSheet"),
        [("OutlineSheet!A2".to_string(), "1".to_string())]
    );

    let body = ods_sheet(
        "S",
        &[
            ods_row(&["Item@b", "Qty@b"]),
            ods_row(&["Fruit", "5"]),
            format!(
                "<table:table-row-group>{}{}</table:table-row-group>",
                ods_row(&["Apple", "2"]),
                ods_row(&["Pear", "3"])
            ),
            ods_row(&["Nuts", "1"]),
        ]
        .concat(),
    );
    let out = read_ods(ods(&content_styled(ODS_BOLD, &body)));
    let r = &regions(&out.document, "S")[0];
    let parent = |n: u32| {
        r.layout
            .rows
            .iter()
            .find(|x| x.row == n)
            .and_then(|x| x.parent)
    };
    assert_eq!(parent(3), Some(2), "{:?}", r.layout.rows);
    assert_eq!(parent(4), Some(2));
    assert_eq!(parent(5), None);
}

/// print-range.ods has header rows on empty sheets: they stay empty. The header rows set
/// the band on a fixture.
#[test]
fn ods_print_rows_are_header() {
    let out = corpus("print-range.ods");
    for sheet in ["Sheet1", "Sheet2"] {
        assert!(regions(&out.document, sheet).is_empty());
        assert!(paragraphs(section(&out.document, sheet)).contains(&"The sheet is empty.".into()));
    }

    let body = ods_sheet(
        "S",
        &[
            format!(
                "<table:table-header-rows>{}{}</table:table-header-rows>",
                ods_row(&["Region", "Sales", "Sales"]),
                ods_row(&["", "2024", "2025"])
            ),
            ods_row(&["North", "1", "2"]),
            ods_row(&["South", "3", "4"]),
        ]
        .concat(),
    );
    let out = read_ods(ods(&content_styled(ODS_BOLD, &body)));
    let r = &regions(&out.document, "S")[0];
    assert_eq!(r.layout.header_rows, [1, 2]);
    assert!(
        r.layout.found_by.contains(&Evidence::PrintTitles),
        "{:?}",
        r.layout.found_by
    );
    assert_eq!(r.layout.header_paths[1], "Sales / 2024");
}

/// A file with cell styles: a cell without a look of its own is plain, so the bold first
/// row is the header (rule H2). Without any cell style the looks are unknown.
#[test]
fn ods_looks_mark_the_header() {
    let body = ods_sheet(
        "S",
        &[
            ods_row(&["Name@b", "City@b"]),
            ods_row(&["Ann", "Rome"]),
            ods_row(&["Bob", "Oslo"]),
        ]
        .concat(),
    );
    let out = read_ods(ods(&content_styled(ODS_BOLD, &body)));
    let r = &regions(&out.document, "S")[0];
    assert_eq!(r.layout.kind, LayoutKind::Table);
    assert_eq!(r.layout.header_rows, [1]);
    assert_eq!(r.layout.header_paths, ["Name", "City"]);
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

/// A tall merged title (A1 over A1:Z10000) and a table of 6 rows starting `gap` empty rows
/// below it: every row of the table is shown, nothing is cut at the cell cap, and the
/// covered positions cost no cells and no elements (ruling P4-AQ).
fn assert_tall_title(doc: &Document, sheet: &str, table_top: u32, near: bool) {
    let q = sheet;
    let held = held(doc, sheet);
    for n in table_top..table_top + 6 {
        for col in ["A", "B"] {
            assert!(
                held.iter().any(|c| c.0 == format!("{q}!{col}{n}")),
                "{col}{n} missing: {held:?}"
            );
        }
    }
    assert!(
        held.iter()
            .any(|c| c.0 == format!("{q}!A1") && c.1 == "Sales report")
    );
    assert!(
        doc.losses
            .iter()
            .all(|l| l.why != LossReason::Limit && l.why != LossReason::Uncertain),
        "{:#?}",
        doc.losses
    );
    let all = regions(doc, sheet);
    let cells: usize = all.iter().map(|r| r.rows.iter().flatten().count()).sum();
    assert!(cells <= 14, "{cells} table cells: {:?}", ranges(doc, sheet));
    if near {
        // Rule T2: the title names the table below it.
        assert_eq!(all.len(), 1, "{:?}", ranges(doc, sheet));
        assert_eq!(all[0].layout.kind, LayoutKind::Table);
        assert_eq!(
            headings(doc, sheet)[0].2.as_deref(),
            Some(format!("{q}!A1").as_str())
        );
    } else {
        assert_eq!(all.len(), 2, "{:?}", ranges(doc, sheet));
        assert_eq!(all[0].layout.kind, LayoutKind::Title);
        assert_eq!(all[0].layout.range, "A1:Z10000");
        assert!(all[0].rows.is_empty());
        assert_eq!(all[1].layout.kind, LayoutKind::Table);
    }
}

const TALL_TABLE: [[&str; 2]; 6] = [
    ["Name", "Score"],
    ["Ann", "1"],
    ["Bob", "2"],
    ["Cid", "3"],
    ["Dan", "4"],
    ["Eve", "5"],
];

#[test]
fn tall_merged_title_costs_no_cells_ods() {
    for (gap, near) in [(1u32, true), (6, false)] {
        let mut body = String::from(
            "<table:table-row><table:table-cell office:value-type=\"string\" table:number-columns-spanned=\"26\" table:number-rows-spanned=\"10000\"><text:p>Sales report</text:p></table:table-cell><table:covered-table-cell table:number-columns-repeated=\"25\"/></table:table-row>\
             <table:table-row table:number-rows-repeated=\"9999\"><table:covered-table-cell table:number-columns-repeated=\"26\"/></table:table-row>",
        );
        body.push_str(&format!(
            "<table:table-row table:number-rows-repeated=\"{gap}\"><table:table-cell/></table:table-row>"
        ));
        for r in TALL_TABLE {
            body.push_str(&ods_row(&r));
        }
        let out = read_ods(ods(&content(&ods_sheet("S", &body))));
        assert_eq!(
            out.completeness,
            Completeness::Complete,
            "{:#?}",
            out.document.losses
        );
        assert_tall_title(&out.document, "S", 10_001 + gap, near);
    }
}

#[test]
fn tall_merged_title_costs_no_cells_xlsx() {
    for (gap, near) in [(1u32, true), (6, false)] {
        let top = 10_001 + gap;
        let mut xml = row(1, &["Sales report"]);
        for (i, r) in TALL_TABLE.iter().enumerate() {
            xml.push_str(&row(top + i as u32, r));
        }
        let s = ws(
            "",
            &xml,
            "<mergeCells count=\"1\"><mergeCell ref=\"A1:Z10000\"/></mergeCells>",
        );
        // Few elements: the 259,974 covered positions must not be charged.
        let limits = Limits {
            max_elements: 2_000,
            ..Limits::default()
        };
        let start = thread_cpu();
        let out = read_with(Book::default().sheet("S", &s).bytes(), limits);
        let used = thread_cpu() - start;
        assert!(used.as_secs_f64() < 2.0, "{used:?}");
        assert_eq!(
            out.completeness,
            Completeness::Complete,
            "{:#?}",
            out.document.losses
        );
        assert_tall_title(&out.document, "S", top, near);
    }
}

#[test]
fn overlapping_tables_give_each_cell_to_the_innermost() {
    // A damaged file: two Excel tables of one sheet overlap. Every value is written once, in
    // the table of the smallest range that holds it: C5 (inside both) in the inner table, D6
    // (right of the inner one) in the outer table. No value becomes a stray paragraph.
    let s = ws(
        "",
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Title</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Region</t></is></c></row><row r=\"5\"><c r=\"C5\"><v>9</v></c></row><row r=\"6\"><c r=\"D6\"><v>51</v></c></row>",
        "",
    );
    let columns = ["A", "B", "C", "D", "E", "F", "G"];
    let out = read(
        &Book::default()
            .sheet("S", &s)
            .related(
                "table",
                "xl/tables/table1.xml",
                &table_part("Outer", "A1:G7", " headerRowCount=\"0\"", &columns),
            )
            .related(
                "table",
                "xl/tables/table2.xml",
                &table_part(
                    "Inner",
                    "B2:C11",
                    " headerRowCount=\"2\" totalsRowCount=\"1\"",
                    &["X", "Y"],
                ),
            ),
    );
    let doc = &out.document;
    let shown = |key: &str, anchor: &str| {
        regions(doc, "S")
            .iter()
            .find(|r| r.layout.key == key)
            .unwrap_or_else(|| panic!("no region {key}"))
            .rows
            .iter()
            .flatten()
            .find(|c| c.anchor.as_ref().is_some_and(|a| a.0 == anchor))
            .map(|c| plain_text(&c.inlines))
    };
    assert_eq!(shown("table:Outer", "S!D6").as_deref(), Some("51"));
    assert_eq!(shown("table:Inner", "S!C5").as_deref(), Some("9"));
    assert!(
        !all_blocks(doc).iter().any(|b| matches!(
            b,
            Block::Paragraph { role: Some(r), .. } if r == "cell"
        )),
        "a value became a stray paragraph"
    );
    assert!(
        doc.losses
            .iter()
            .any(|l| l.why == LossReason::Uncertain && l.what.contains("overlaps")),
        "{:?}",
        doc.losses
    );
}

#[test]
fn overlapping_tables_keep_every_value() {
    // A damaged file: two Excel tables of one sheet overlap. The value at D6, inside the
    // outer table and right of the inner one, is written once with its anchor.
    let s = ws(
        "",
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Title</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>Region</t></is></c></row><row r=\"6\"><c r=\"D6\"><v>51</v></c></row>",
        "",
    );
    let columns = ["A", "B", "C", "D", "E", "F", "G"];
    let out = read(
        &Book::default()
            .sheet("S", &s)
            .related(
                "table",
                "xl/tables/table1.xml",
                &table_part("Outer", "A1:G7", " headerRowCount=\"0\"", &columns),
            )
            .related(
                "table",
                "xl/tables/table2.xml",
                &table_part(
                    "Inner",
                    "B2:C11",
                    " headerRowCount=\"2\" totalsRowCount=\"1\"",
                    &["X", "Y"],
                ),
            ),
    );
    let at_d6 = |anchor: &Option<rezzer_model::Anchor>, inlines: &[rezzer_model::Inline]| {
        anchor.as_ref().is_some_and(|a| a.0 == "S!D6") && plain_text(inlines) == "51"
    };
    let written: usize = all_blocks(&out.document)
        .into_iter()
        .map(|b| match b {
            Block::Paragraph {
                inlines, anchor, ..
            } => usize::from(at_d6(anchor, inlines)),
            Block::Table { rows, .. } => rows
                .iter()
                .flatten()
                .filter(|c| at_d6(&c.anchor, &c.inlines))
                .count(),
            _ => 0,
        })
        .sum();
    assert_eq!(written, 1);
}
