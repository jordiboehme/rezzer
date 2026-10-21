// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables: from the tag tree, from drawn rules and from white space gutters, and the places
//! where none may be found.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{Builder, Page, pdf, show};
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Block, Bytes, Cell, Completeness, Confidence, Detection, FormatId, LossReason,
    ReadOptions, ReadOutcome, Reader, SectionKind, Severity, Source, plain_text,
};
use rezzer_read_pdf::PdfReader;

fn corpus_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf")
        .join(name)
}

fn read_with(bytes: &[u8], limits: Limits) -> ReadOutcome {
    let src = Source {
        path: None,
        name: "file.pdf".into(),
        bytes: Bytes::Owned(Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format: FormatId::Pdf,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "%PDF-".into(),
        },
        modified: None,
    };
    PdfReader.read(&src, &ReadOptions::default(), &Budget::new(limits))
}

fn read(bytes: &[u8]) -> ReadOutcome {
    read_with(bytes, Limits::default())
}

fn read_corpus(name: &str) -> ReadOutcome {
    read(&std::fs::read(corpus_path(name)).expect("corpus file"))
}

fn page(out: &ReadOutcome, n: u32) -> &[Block] {
    let want = Anchor(format!("p{n}"));
    out.document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section {
                kind: SectionKind::Page,
                anchor: Some(a),
                blocks,
                ..
            } if *a == want => Some(blocks.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no section p{n}"))
}

/// The tables of a page: header rows and the text of each cell.
fn tables(blocks: &[Block]) -> Vec<(usize, Vec<Vec<String>>)> {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Table {
                header_rows, rows, ..
            } => Some((*header_rows, texts(rows))),
            _ => None,
        })
        .collect()
}

fn texts(rows: &[Vec<Cell>]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|c| plain_text(&c.inlines)).collect())
        .collect()
}

fn all_tables(out: &ReadOutcome) -> usize {
    out.document
        .body
        .iter()
        .map(|s| match s {
            Block::Section { blocks, .. } => tables(blocks).len(),
            _ => 0,
        })
        .sum()
}

fn paragraphs(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph { inlines, role, .. } if role.as_deref() != Some("page") => {
                Some(plain_text(inlines))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn tagged_table_with_spans_and_header() {
    let out = read_corpus("irs-f1040-2025.pdf");
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    let (header_rows, rows) = &found[0];
    assert_eq!(*header_rows, 1);
    assert_eq!(rows.len(), 8);
    assert!(rows.iter().all(|r| r.len() == 5), "{rows:?}");
    assert_eq!(
        rows[0],
        [
            "",
            "Dependent 1",
            "Dependent 2",
            "Dependent 3",
            "Dependent 4"
        ]
    );
    assert!(rows[1][0].ends_with("First name"), "{:?}", rows[1]);
}

/// A tagged page with a table of two rows: a header cell over two columns, then two cells.
/// The span comes from the cell's `/A` attributes.
fn tagged_span_table() -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let kid = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let content = "BT /F1 12 Tf /TH <</MCID 0>> BDC 1 0 0 1 72 700 Tm (Both) Tj EMC \
                   /TD <</MCID 1>> BDC 1 0 0 1 72 680 Tm (Left) Tj EMC \
                   /TD <</MCID 2>> BDC 1 0 0 1 200 680 Tm (Right) Tj EMC ET";
    let cs = b.stream("", content.as_bytes());
    let root = b.reserve();
    let table = b.reserve();
    let (tr1, tr2) = (b.reserve(), b.reserve());
    let th = b.add(format!(
        "<< /Type /StructElem /S /TH /P {tr1} 0 R /Pg {kid} 0 R /K 0 \
         /A << /O /Table /ColSpan 2 >> >>"
    ));
    let td1 = b.add(format!(
        "<< /Type /StructElem /S /TD /P {tr2} 0 R /Pg {kid} 0 R /K 1 >>"
    ));
    let td2 = b.add(format!(
        "<< /Type /StructElem /S /TD /P {tr2} 0 R /Pg {kid} 0 R /K 2 >>"
    ));
    b.set(
        tr1,
        format!("<< /Type /StructElem /S /TR /P {table} 0 R /K [{th} 0 R] >>"),
    );
    b.set(
        tr2,
        format!("<< /Type /StructElem /S /TR /P {table} 0 R /K [{td1} 0 R {td2} 0 R] >>"),
    );
    b.set(
        table,
        format!("<< /Type /StructElem /S /Table /P {root} 0 R /K [{tr1} 0 R {tr2} 0 R] >>"),
    );
    b.set(
        root,
        format!(
            "<< /Type /StructTreeRoot /K {table} 0 R \
             /ParentTree << /Nums [0 [{th} 0 R {td1} 0 R {td2} 0 R]] >> >>"
        ),
    );
    b.set(
        kid,
        format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /StructParents 0 \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
        ),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R /MarkInfo << /Marked true >> \
             /StructTreeRoot {root} 0 R >>"
        ),
    );
    b.finish(catalog)
}

#[test]
fn tagged_cell_spans_come_from_the_attributes() {
    let out = read(&tagged_span_table());
    let found: Vec<&Block> = page(&out, 1)
        .iter()
        .filter(|b| matches!(b, Block::Table { .. }))
        .collect();
    match found.as_slice() {
        [
            Block::Table {
                header_rows: 1,
                rows,
                anchor: Some(a),
                ..
            },
        ] => {
            assert_eq!(texts(rows), [vec!["Both"], vec!["Left", "Right"]]);
            assert_eq!(rows[0][0].col_span, 2);
            assert_eq!(a.0, "p1:t1");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn nist_tagged_tables() {
    let path = corpus_path("nist-sp800-53r5.pdf");
    if !path.exists() {
        eprintln!("skipped: {} is not fetched", path.display());
        return;
    }
    let out = read(&std::fs::read(path).expect("corpus file"));
    // 26 `/Table` elements; one holds no text and is no table.
    assert_eq!(all_tables(&out), 25);
    // The errata table goes on over nine pages as one table, with its column titles once.
    let errata = tables(page(&out, 19));
    assert_eq!(errata.len(), 1);
    let (header_rows, rows) = &errata[0];
    assert_eq!(*header_rows, 1);
    assert_eq!(rows[0], ["DATE", "TYPE", "REVISION", "PAGE"]);
    assert_eq!(rows.iter().filter(|r| r[0] == "DATE").count(), 1);
    assert!(tables(page(&out, 20)).is_empty());
    // The glossary keeps each term with its definition, in one table over its pages.
    let glossary = tables(page(&out, 421));
    assert_eq!(glossary.len(), 1);
    let rows = &glossary[0].1;
    assert!(rows.len() > 250, "{}", rows.len());
    let pair = rows
        .iter()
        .find(|r| r.first().is_some_and(|t| t == "external system service"))
        .expect("the term is a row");
    assert!(
        pair[1].starts_with("A system service that is provided"),
        "{pair:?}"
    );
    assert!(rows.iter().all(|r| r.len() == 2), "a row without two cells");
    assert!(tables(page(&out, 430)).is_empty());
}

/// A ruled table of `rows` rows of the given cells, 30 points high and 120 wide each, with
/// every line of the grid drawn, below a paragraph.
fn ruled_page(rows: &[[&str; 3]]) -> Vec<u8> {
    let (left, top) = (72.0, 700.0);
    let mut c = show("F1", 10.0, 72.0, 760.0, "A paragraph before the table.");
    c += "0.5 w ";
    let height = 30.0 * rows.len() as f32;
    for k in 0..=rows.len() {
        let y = top - 30.0 * k as f32;
        c += &format!("{left} {y} m {} {y} l S ", left + 360.0);
    }
    for k in 0..=3 {
        let x = left + 120.0 * k as f32;
        c += &format!("{x} {top} m {x} {} l S ", top - height);
    }
    for (r, row) in rows.iter().enumerate() {
        for (k, text) in row.iter().enumerate() {
            let x = left + 6.0 + 120.0 * k as f32;
            let y = top - 30.0 * r as f32 - 20.0;
            c += &show("F1", 10.0, x, y, text);
        }
    }
    c += &show("F1", 10.0, 72.0, 520.0, "A paragraph after the table.");
    pdf(&[Page::text(c)], "")
}

#[test]
fn ruled_table_grid() {
    let out = read(&ruled_page(&[
        ["Fruit", "Count", "Price"],
        ["Apples", "3", "1.20"],
        ["Pears", "12", "0.80"],
        ["Plums", "7", "2.05"],
    ]));
    let blocks = page(&out, 1);
    let found = tables(blocks);
    assert_eq!(
        found,
        [(
            1,
            vec![
                vec!["Fruit", "Count", "Price"],
                vec!["Apples", "3", "1.20"],
                vec!["Pears", "12", "0.80"],
                vec!["Plums", "7", "2.05"],
            ]
            .into_iter()
            .map(|r| r.into_iter().map(String::from).collect())
            .collect()
        )]
    );
    // The table stands between the paragraphs, in reading order.
    let order: Vec<&str> = blocks
        .iter()
        .skip(1)
        .map(|b| match b {
            Block::Table { .. } => "table",
            Block::Paragraph { .. } => "paragraph",
            _ => "other",
        })
        .collect();
    assert_eq!(order, ["paragraph", "table", "paragraph"]);
    assert_eq!(
        paragraphs(blocks),
        [
            "A paragraph before the table.",
            "A paragraph after the table."
        ]
    );
}

#[test]
fn missing_inner_rules_join_ruled_cells() {
    // A 3 x 3 grid: no rule between the first two cells of row 1, so they are one cell over
    // two columns; no rule below the first cell of row 2, so it goes down over row 3.
    let mut c = String::from("0.5 w ");
    for (y, x0) in [(700, 72), (670, 72), (640, 192), (610, 72)] {
        c += &format!("{x0} {y} m 432 {y} l S ");
    }
    for (x, y1) in [(72, 700), (192, 670), (312, 700), (432, 700)] {
        c += &format!("{x} 610 m {x} {y1} l S ");
    }
    for (x, y, text) in [
        (78.0, 680.0, "Region"),
        (318.0, 680.0, "Total"),
        (78.0, 650.0, "North"),
        (198.0, 650.0, "3"),
        (318.0, 650.0, "1.20"),
        (198.0, 620.0, "12"),
        (318.0, 620.0, "0.80"),
    ] {
        c += &show("F1", 10.0, x, y, text);
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let rows = page(&out, 1)
        .iter()
        .find_map(|b| match b {
            Block::Table { rows, .. } => Some(rows.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no table: {:?}", out.document.losses));
    assert_eq!(
        texts(&rows),
        [
            vec!["Region", "Total"],
            vec!["North", "3", "1.20"],
            vec!["12", "0.80"]
        ]
    );
    assert_eq!((rows[0][0].col_span, rows[0][0].row_span), (2, 1));
    assert_eq!((rows[1][0].col_span, rows[1][0].row_span), (1, 2));
}

#[test]
fn form_boxes_are_not_a_table() {
    // The tax form's only table is the tagged one.
    let irs = read_corpus("irs-f1040-2025.pdf");
    assert_eq!(all_tables(&irs), 1);
    // The direct deposit form is not tagged: its boxes and its sample check are no table.
    let sf = read_corpus("pdfbox-sf1199a-direct-deposit.pdf");
    assert_eq!(all_tables(&sf), 0);
    assert!(
        paragraphs(page(&sf, 4))
            .iter()
            .any(|p| p.contains("AUSTIN, TEXAS"))
    );
}

#[test]
fn borderless_table_from_gutters() {
    let out = read_corpus("pypdf-multicolumn.pdf");
    let blocks = page(&out, 3);
    let found = tables(blocks);
    assert_eq!(found.len(), 1, "{found:?}");
    let (header_rows, rows) = &found[0];
    assert_eq!(*header_rows, 1);
    assert_eq!(
        rows[0],
        [
            "Country",
            "Population (millions)",
            "Area (km2)",
            "Capital",
            "Official Language"
        ]
    );
    assert_eq!(rows.len(), 6);
    assert_eq!(
        rows[2],
        [
            "Belgium",
            "11.5",
            "30,689",
            "Brussels",
            "Dutch, French, German"
        ]
    );
    // The caption stays a paragraph before the table.
    assert_eq!(
        paragraphs(blocks).first().map(String::as_str),
        Some("Table 1: EU Countries Information")
    );
}

#[test]
fn two_column_prose_is_not_a_table() {
    let out = read_corpus("pypdf-multicolumn.pdf");
    for n in [1, 2] {
        assert!(tables(page(&out, n)).is_empty(), "page {n}");
    }
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("Possible table")),
        "{:?}",
        out.document.losses
    );
}

#[test]
fn rotated_margin_text_is_not_a_table() {
    // Prose in the body, and three lines of margin text turned a quarter, each with two
    // parts far apart along it, as a stamp or a running link down the margin. Read in its
    // own frame, the turned text would line up in two columns.
    //
    // This cannot isolate the guard that keeps turned runs out of a table: the reading order
    // gives turned text lines of their own after the page's text, and all runs of one
    // turned line start at the same x on the page, so their pieces never stand side by
    // side. The test holds the result either way.
    let mut c = String::new();
    for k in 0..6 {
        c += &show(
            "F1",
            10.0,
            72.0,
            700.0 - 14.0 * k as f32,
            &format!("Body line {k} of a short paragraph that reads as prose here."),
        );
    }
    for k in 0..3 {
        let x = 560.0 + 12.0 * k as f32;
        c += &format!("BT /F1 10 Tf 0 1 -1 0 {x} 100 Tm (doi {k}) Tj ET ");
        c += &format!("BT /F1 10 Tf 0 1 -1 0 {x} 400 Tm (page {k}) Tj ET ");
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let blocks = page(&out, 1);
    assert!(tables(blocks).is_empty(), "{:?}", tables(blocks));
    let all = paragraphs(blocks).join("\n");
    assert!(all.contains("doi 0") && all.contains("page 2"), "{all}");
}

#[test]
fn unsure_region_keeps_paragraphs_with_note() {
    // A caption names a table and short pieces stand in columns, but one line has text in
    // the gutter between two columns: no grid holds it. The text stays paragraphs, with a
    // note where the region stands and a loss.
    let mut c = show("F1", 10.0, 72.0, 760.0, "Table 3: Sample sizes");
    let rows = [
        ("Site A", "12", "spring"),
        ("Site B", "15", "summer"),
        ("Site C", "9", "autumn"),
    ];
    for (k, (a, b, s)) in rows.iter().enumerate() {
        let y = 736.0 - 24.0 * k as f32;
        c += &show("F1", 10.0, 72.0, y, a);
        c += &show("F1", 10.0, 200.0, y, b);
        c += &show("F1", 10.0, 300.0, y, s);
    }
    c += &show("F1", 10.0, 250.0, 724.0, "late");
    let out = read(&pdf(&[Page::text(c)], ""));
    let blocks = page(&out, 1);
    assert!(tables(blocks).is_empty(), "{:?}", tables(blocks));
    let note = "Possible table on page 1 not rebuilt (reason: some of its text stands between \
                its columns)";
    let paras = paragraphs(blocks);
    assert!(paras.iter().any(|p| p == note), "{paras:?}");
    assert!(paras.iter().any(|p| p.contains("Site B")), "{paras:?}");
    let loss = out
        .document
        .losses
        .iter()
        .find(|l| l.what == note)
        .unwrap_or_else(|| panic!("{:?}", out.document.losses));
    assert_eq!(loss.why, LossReason::Uncertain);
    assert_eq!(loss.severity, Severity::Info);
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn a_stop_inside_a_tagged_list_names_the_item_it_cut() {
    // Items `w0.` to `w11.`: the text limit lets the items with one digit through, so the
    // list keeps ten items and the stop cuts the eleventh.
    let out = read_with(
        &common::tagged_lines(12, "list", None),
        Limits {
            max_text_bytes: 4,
            ..Limits::default()
        },
    );
    match &page(&out, 1)[1..] {
        [Block::List { items, .. }] => assert_eq!(items.len(), 10),
        other => panic!("{other:?}"),
    }
    let cut: Vec<&str> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .map(|l| l.what.as_str())
        .collect();
    assert_eq!(cut.len(), 1, "{cut:?}");
    assert!(
        cut[0].ends_with("page 1 from item 11 of list p1:b1 on is not read"),
        "{cut:?}"
    );
}

#[test]
fn a_stop_inside_a_tagged_table_keeps_its_rows_before() {
    let out = read_with(
        &common::tagged_lines(12, "table", None),
        Limits {
            max_text_bytes: 4,
            ..Limits::default()
        },
    );
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.len(), 10);
    let cut: Vec<&str> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit)
        .map(|l| l.what.as_str())
        .collect();
    assert_eq!(cut.len(), 1, "{cut:?}");
    assert!(
        cut[0].ends_with("page 1 from row 11 of table p1:t1 on is not read"),
        "{cut:?}"
    );
}

#[test]
fn a_row_split_by_a_page_break_is_one_row() {
    // One tagged row whose first cell stands on page 1 and second cell on page 2.
    let out = read(&common::wide_row(2));
    assert_eq!(
        tables(page(&out, 1)),
        [(0, vec![vec!["w0".to_string(), "w1".to_string()]])]
    );
    assert!(tables(page(&out, 2)).is_empty());
    assert_eq!(out.completeness, Completeness::Complete);
}

fn markdown(out: &ReadOutcome) -> String {
    rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    )
}

#[test]
fn ticket_box_next_to_table() {
    // A 6-column ruled table, and above it a box whose left side is drawn twice, 1.5 and
    // 0.8 points left of the table's fourth column rule. The column rules stop in row 3 (one
    // cell over the whole row); in row 4 only the fourth one is drawn (two cells).
    let xs = [56.0, 105.0, 143.0, 413.0, 461.0, 500.0, 539.0];
    let ys = [600.0, 580.0, 560.0, 540.0, 520.0];
    let mut c = String::from("0.5 w ");
    c += "411.5 640 127 50 re S 412.2 640.5 126.1 49 re S ";
    c += &show("F1", 9.0, 420.0, 670.0, "Fare");
    c += &show("F1", 9.0, 420.0, 652.0, "10.00");
    for y in ys {
        c += &format!("56 {y} m 539 {y} l S ");
    }
    for x in [56.0, 539.0] {
        c += &format!("{x} 520 m {x} 600 l S ");
    }
    for x in &xs[1..6] {
        c += &format!("{x} 560 m {x} 600 l S ");
    }
    c += "413 520 m 413 540 l S ";
    let rows = [
        ["Date", "Time", "From / To", "Car", "Seat", "Class"],
        ["2030-01-02", "10:15", "Alpha / Beta", "3", "*", "2"],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (k, text) in row.iter().enumerate() {
            c += &show("F1", 8.0, xs[k] + 4.0, ys[r] - 14.0, text);
        }
    }
    c += &show("F1", 8.0, 60.0, 546.0, "Reserved seats only");
    c += &show("F1", 8.0, 60.0, 526.0, "Ticket 1000001");
    c += &show("F1", 8.0, 417.0, 526.0, "Price 10.00");
    let out = read(&pdf(&[Page::text(c)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    let rows = &found[0].1;
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert_eq!(rows[0].len(), 6, "{rows:?}");
    assert_eq!(rows[1].len(), 6, "{rows:?}");
    assert_eq!(rows[1][4], "*", "{rows:?}");
    assert_eq!(rows[2], ["Reserved seats only"], "{rows:?}");
    assert_eq!(rows[3], ["Ticket 1000001", "Price 10.00"], "{rows:?}");
    let md = markdown(&out);
    assert!(
        md.contains("| Reserved seats only |  |  |  |  |  |\n"),
        "{md}"
    );
    assert_eq!(md.matches("Reserved seats only").count(), 1, "{md}");
    assert!(
        md.contains(
            "Merged cells: row 3, columns 1 to 6; row 4, columns 1 to 3; row 4, columns 4 to 6"
        ),
        "{md}"
    );
}

#[test]
fn wide_ruled_grid_says_so() {
    // A ruled grid of 20 columns and 3 rows with text in every cell, and no caption.
    let mut c = String::from("0.5 w ");
    for k in 0..=3 {
        let y = 700.0 - 20.0 * k as f32;
        c += &format!("40 {y} m 540 {y} l S ");
    }
    for k in 0..=20 {
        let x = 40.0 + 25.0 * k as f32;
        c += &format!("{x} 640 m {x} 700 l S ");
    }
    for r in 0..3 {
        for k in 0..20 {
            let x = 43.0 + 25.0 * k as f32;
            let y = 686.0 - 20.0 * r as f32;
            c += &show("F1", 8.0, x, y, &format!("{}{k}", ['a', 'b', 'c'][r]));
        }
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let blocks = page(&out, 1);
    assert!(tables(blocks).is_empty(), "{:?}", tables(blocks));
    let note = "Possible table on page 1 not rebuilt (reason: it has more than 15 columns)";
    let paras = paragraphs(blocks);
    assert!(paras.iter().any(|p| p == note), "{paras:?}");
    let losses: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.contains("Possible table"))
        .collect();
    assert_eq!(losses.len(), 1, "{losses:?}");
    assert_eq!(losses[0].what, note);
    assert_eq!(losses[0].why, LossReason::Uncertain);
    assert_eq!(losses[0].severity, Severity::Info);
}

/// A ruled table of 3 by `xs.len() - 1` cells between the column rules `xs`, rows of 20
/// points from y 700 down, each cell holding `<tag><row><column>`.
fn ruled_cells(xs: &[f32], tag: &str) -> String {
    let (x0, x1) = (xs[0], xs[xs.len() - 1]);
    let mut c = String::new();
    for k in 0..4 {
        let y = 700.0 - 20.0 * k as f32;
        c += &format!("{x0} {y} m {x1} {y} l S ");
    }
    for x in xs {
        c += &format!("{x} 640 m {x} 700 l S ");
    }
    for r in 0..3 {
        for k in 0..xs.len() - 1 {
            let y = 686.0 - 20.0 * r as f32;
            c += &show("F1", 8.0, xs[k] + 4.0, y, &format!("{tag}{r}{k}"));
        }
    }
    c
}

#[test]
fn rules_close_together_do_not_chain() {
    // Two tables 3.6 points apart with a short tick between their positions far below: two
    // tables of three columns.
    let mut c = String::from("0.5 w ");
    c += &ruled_cells(&[50.0, 130.0, 210.0, 290.0], "L");
    c += &ruled_cells(&[293.6, 373.6, 453.6, 533.6], "R");
    c += "291.8 100 m 291.8 110 l S ";
    let out = read(&pdf(&[Page::text(c)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .all(|(_, rows)| rows.iter().all(|r| r.len() == 3)),
        "{found:?}"
    );
    // A barcode of bars every 1.8 points, stroked or filled thin, below a table whose column
    // rules lie in the bars' range: the table keeps its four columns.
    let xs = [330.0, 370.0, 410.0, 450.0, 490.0];
    for filled in [false, true] {
        let mut c = String::from("0.5 w ");
        let mut x = 400.0f32;
        while x < 500.0 {
            c += &if filled {
                format!("{x} 100 0.5 40 re f ")
            } else {
                format!("{x} 100 m {x} 140 l S ")
            };
            x += 1.8;
        }
        c += &ruled_cells(&xs, "c");
        let out = read(&pdf(&[Page::text(c)], ""));
        let found = tables(page(&out, 1));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].1[0], ["c00", "c01", "c02", "c03"], "{found:?}");
    }
}
