// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables: from the tag tree, from drawn rules and from white space gutters, and the places
//! where none may be found.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{Builder, Page, helvetica_width, pdf, show, sized_page};
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
        md.contains(
            "| Reserved seats only | \u{2190} | \u{2190} | \u{2190} | \u{2190} | \u{2190} |\n\
             | Ticket 1000001 | \u{2190} | \u{2190} | Price 10.00 | \u{2190} | \u{2190} |\n"
        ),
        "{md}"
    );
    assert_eq!(md.matches("Reserved seats only").count(), 1, "{md}");
    assert!(
        md.contains("\n\nMerged cells: \u{3003} same as above, \u{2190} same as left.\n"),
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
        for (k, x) in xs[..xs.len() - 1].iter().enumerate() {
            let y = 686.0 - 20.0 * r as f32;
            c += &show("F1", 8.0, x + 4.0, y, &format!("{tag}{r}{k}"));
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

#[test]
fn form_boxes_give_one_note_per_page() {
    // Twelve ruled boxes of three columns and two rows with a label in each row of the first
    // column: one note for the page that counts them.
    let mut c = String::from("0.5 w ");
    for i in 0..12 {
        let top = 780.0 - 60.0 * i as f32;
        for k in 0..3 {
            let y = top - 20.0 * k as f32;
            c += &format!("50 {y} m 500 {y} l S ");
        }
        for k in 0..4 {
            let x = 50.0 + 150.0 * k as f32;
            c += &format!("{x} {} m {x} {top} l S ", top - 40.0);
        }
        c += &show("F1", 8.0, 54.0, top - 14.0, &format!("Name {i}"));
        c += &show("F1", 8.0, 54.0, top - 34.0, &format!("Address {i}"));
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let note = "Possible tables on page 1 not rebuilt: 12 (most of their cells are empty)";
    let paras = paragraphs(page(&out, 1));
    assert_eq!(
        paras
            .iter()
            .filter(|p| p.contains("Possible table"))
            .count(),
        1,
        "{paras:?}"
    );
    assert!(paras.iter().any(|p| p == note), "{paras:?}");
    assert!(paras.iter().any(|p| p.contains("Address 11")), "{paras:?}");
    let losses: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.contains("Possible table"))
        .collect();
    assert_eq!(losses.len(), 1, "{losses:?}");
    assert_eq!(losses[0].why, LossReason::Uncertain);
}

#[test]
fn a_grid_next_to_a_figure_caption_gets_no_note() {
    // A ruled grid with text in every cell, its first row spanning, under a figure caption:
    // not a table, and no note says it may be one.
    let mut c = String::from("0.5 w ");
    c += &ruled_cells(&[50.0, 130.0, 210.0, 290.0], "f");
    c += &show("F1", 9.0, 50.0, 630.0, "Figure 3: A chart of the results");
    let out = read(&pdf(&[Page::text(c)], ""));
    assert!(tables(page(&out, 1)).is_empty());
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
fn side_by_side_lists_are_no_table() {
    // Left: labels and values; right, 260 points away: a label and value column of another
    // pitch. The baselines of the sides do not line up. No table may mix the two sides, each
    // side reads whole and is a table of labels and values.
    let left = [
        ("Guest", "A. Sample"),
        ("Room", "101"),
        ("Arrival", "2030-01-02"),
        ("Departure", "2030-01-05"),
    ];
    let right = [("Folio", "1000"), ("Copy", "Original"), ("Cashier", "77")];
    let mut c = String::new();
    for (k, (l, v)) in left.iter().enumerate() {
        let y = 650.6 - 11.6 * k as f32;
        c += &show("F1", 8.0, 18.0, y, l);
        c += &show("F1", 8.0, 90.0, y, v);
    }
    for (k, (l, v)) in right.iter().enumerate() {
        let y = 646.3 - 15.9 * k as f32;
        c += &show("F1", 8.0, 432.0, y, l);
        c += &show("F1", 8.0, 500.0, y, v);
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let blocks = page(&out, 1);
    let left_words: Vec<&str> = left.iter().flat_map(|(l, v)| [*l, *v]).collect();
    let right_words: Vec<&str> = right.iter().flat_map(|(l, v)| [*l, *v]).collect();
    for (_, rows) in tables(blocks) {
        let cells: Vec<&String> = rows.iter().flatten().collect();
        let has = |words: &[&str]| cells.iter().any(|c| words.iter().any(|w| c.contains(w)));
        assert!(
            !(has(&left_words) && has(&right_words)),
            "a table mixes both sides: {rows:?}"
        );
    }
    // Each side is a table of labels and values.
    let kv = |rows: &[(&str, &str)]| -> Vec<Vec<String>> {
        rows.iter()
            .map(|(l, v)| vec![l.to_string(), v.to_string()])
            .collect()
    };
    assert_eq!(tables(blocks), [(0, kv(&left)), (0, kv(&right))]);
    // Each side is one stretch of the page's text.
    let mut flat = String::new();
    for b in blocks {
        match b {
            Block::Paragraph { inlines, .. } => {
                flat.push_str(&plain_text(inlines));
                flat.push('\n');
            }
            Block::Table { rows, .. } => {
                for c in rows.iter().flatten() {
                    flat.push_str(&plain_text(&c.inlines));
                    flat.push('\n');
                }
            }
            _ => {}
        }
    }
    let at = |w: &str| flat.find(w).unwrap_or_else(|| panic!("{w} in {flat}"));
    let (l0, l1) = (at("Guest"), at("2030-01-05"));
    for w in &right_words {
        let p = at(w);
        assert!(p > l1 || p < l0, "{w} inside the left side: {flat}");
    }
    let (r0, r1) = (at("Folio"), at("77"));
    for w in &left_words {
        let p = at(w);
        assert!(p > r1 || p < r0, "{w} inside the right side: {flat}");
    }
}

/// The page of rows whose two cells have different font sizes: `shape` 0 is label 8 pt and
/// value 12 pt top-aligned, 1 is label 8 pt and value 14 pt centred, 2 is code with smaller
/// line numbers a little above its baseline, 3 a glossary of bold 9 pt terms 2 points below
/// the first line of 11 pt definitions that wrap. Shapes 4 to 7 are shape 0 with a run of
/// spaces in 12 pt on each value line: before the value on its baseline, after it 0.5 point
/// higher, after it on its baseline, and after it 0.5 point lower.
fn two_size_rows(shape: usize) -> Vec<u8> {
    let mut c = String::new();
    match shape {
        4..=7 => {
            let (x, dy) = [(140.0, 0.0), (300.0, 0.5), (300.0, 0.0), (300.0, -0.5)][shape - 4];
            for r in 0..6 {
                let top = 700.0 - 20.0 * r as f32;
                let base = top - 12.0 * 0.718;
                c += &show("F1", 8.0, 50.0, top - 8.0 * 0.718, &format!("Label {r}"));
                if x < 200.0 {
                    c += &show("F1", 12.0, x, base + dy, "      ");
                }
                c += &show("F1", 12.0, 200.0, base, &format!("Value {r}"));
                if x > 200.0 {
                    c += &show("F1", 12.0, x, base + dy, "      ");
                }
            }
        }
        0 | 1 => {
            let (value, k) = if shape == 0 {
                (12.0, 0.718)
            } else {
                (14.0, 0.36)
            };
            for r in 0..6 {
                let top = 700.0 - 20.0 * r as f32;
                c += &show("F1", 8.0, 50.0, top - 8.0 * k, &format!("Label {r}"));
                c += &show("F1", value, 200.0, top - value * k, &format!("Value {r}"));
            }
        }
        2 => {
            for r in 0..15 {
                let y = 700.0 - 12.0 * r as f32;
                c += &show("F1", 7.0, 50.0, y + 1.5, &format!("{}", r + 1));
                c += &show(
                    "F1",
                    9.5,
                    75.0,
                    y,
                    &format!("let value_{r} = compute(x, {r});"),
                );
            }
        }
        _ => {
            let mut y = 720.0f32;
            for r in 0..8 {
                c += &show("F2", 9.0, 50.0, y - 2.0, &format!("Term{r}"));
                let lines = 1 + r % 3;
                for j in 0..lines {
                    let t = format!("Definition {r} part {j}");
                    c += &show("F1", 11.0, 160.0, y - 13.5 * j as f32, &t);
                }
                y -= 13.5 * lines as f32 + 6.0;
            }
        }
    }
    pdf(&[Page::text(c)], "")
}

#[test]
fn rows_of_two_sizes_stay_rows() {
    // The baselines of the two cells differ by what the size difference explains, the same
    // on every row: the cells are rows, not two columns.
    let first = [
        ["Label 0", "Value 0"],
        ["Label 0", "Value 0"],
        ["1", "let value_0 = compute(x, 0);"],
        ["Term0", "Definition 0 part 0"],
        ["Label 0", "Value 0"],
        ["Label 0", "Value 0"],
        ["Label 0", "Value 0"],
        ["Label 0", "Value 0"],
    ];
    let mut wrong = Vec::new();
    for (shape, want) in first.iter().enumerate() {
        let out = read(&two_size_rows(shape));
        let found = tables(page(&out, 1));
        let row = found.first().map(|t| t.1[0].clone()).unwrap_or_default();
        let holds = |w: &str| row.iter().any(|c| c.trim() == w);
        // With a run of spaces on the value lines, rows may stay lines of text: each label
        // is then read right before its value.
        let flat = format!("{:?} {:?}", paragraphs(page(&out, 1)), found);
        let at = |w: &str| flat.find(w).unwrap_or(usize::MAX);
        let as_rows = at("Label 0") < at("Value 0") && at("Value 0") < at("Label 1");
        let table = found.len() == 1 && want.iter().all(|w| holds(w));
        if !(table || (shape >= 4 && as_rows)) {
            wrong.push((shape, paragraphs(page(&out, 1))));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn two_refused_captioned_tables_keep_two_notes() {
    // Two captioned regions on one page, each with text between its columns: each caption
    // gets its own note.
    let table = |top: f32, n: u32| {
        let mut c = show("F2", 10.0, 50.0, top, &format!("Table {n}: Sample figures"));
        for r in 0..5 {
            let y = top - 16.0 - 14.0 * r as f32;
            if r == 2 {
                c += &show("F1", 9.0, 50.0, y, "Row 2");
                c += &show("F1", 9.0, 215.0, y, "between");
                c += &show("F1", 9.0, 400.0, y, "302");
                continue;
            }
            c += &show("F1", 9.0, 50.0, y, &format!("Row {r}"));
            c += &show("F1", 9.0, 160.0, y, &format!("{}", 100 + r));
            c += &show("F1", 9.0, 280.0, y, &format!("{}", 200 + r));
            c += &show("F1", 9.0, 400.0, y, &format!("{}", 300 + r));
        }
        c
    };
    let mut c = table(760.0, 1);
    for k in 0..10 {
        let line =
            format!("Body line {k} goes on with plain words about nothing in particular here");
        c += &show("F1", 10.0, 50.0, 660.0 - 13.0 * k as f32, &line);
    }
    c += &table(480.0, 2);
    let out = read(&pdf(&[Page::text(c)], ""));
    let notes = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.starts_with("Possible table on page 1"))
        .count();
    assert_eq!(notes, 2, "{:?}", out.document.losses);
}

#[test]
fn a_dot_plot_of_many_squares_stays_complete() {
    // 4,160 small squares without text and a caption under them: no box holds text, so the
    // page has no limit to name and stays complete.
    let mut c = String::new();
    for i in 0..64 {
        for j in 0..65 {
            c += &format!(
                "{} {} 3 3 re f\n",
                60.0 + 7.0 * i as f32,
                300.0 + 7.0 * j as f32
            );
        }
    }
    c += &show(
        "F1",
        10.0,
        60.0,
        280.0,
        "Figure 1: Sample dot plot of invented values.",
    );
    let out = read(&pdf(&[Page::text(c)], ""));
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("drawn boxes")),
        "{:?}",
        out.document.losses
    );
    assert_eq!(out.completeness, Completeness::Complete);
}

/// An invoice table in a landscape frame of 842 by 595 points: seven columns between column
/// rules, an outer border, a header rule, a "VAT" cell over "RATE" and "AMOUNT" and no rules
/// between the records. Two of the three records have a second line, set closer to the
/// record below than to their own first line.
fn invoice_table() -> String {
    let xs = [40.0, 130.0, 240.0, 470.0, 520.0, 600.0, 690.0, 800.0];
    let (top, sub, head, bottom) = (500.0, 485.0, 470.0, 380.0);
    let mut c = String::from("0.5 w ");
    for y in [top, head, bottom] {
        c += &format!("{} {y} m {} {y} l S ", xs[0], xs[7]);
    }
    c += &format!("{} {sub} m {} {sub} l S ", xs[4], xs[6]);
    for (k, x) in xs.iter().enumerate() {
        let upper = if k == 5 { sub } else { top };
        c += &format!("{x} {bottom} m {x} {upper} l S ");
    }
    for (k, t) in ["ISSUING DATE", "DOCUMENT NUMBER", "TYPE OF SERVICE", "QTY"]
        .iter()
        .enumerate()
    {
        c += &show("F1", 7.0, xs[k] + 4.0, 482.0, t);
    }
    c += &show("F1", 7.0, xs[4] + 4.0, 489.0, "VAT");
    c += &show("F1", 7.0, xs[4] + 4.0, 474.0, "RATE");
    c += &show("F1", 7.0, xs[5] + 4.0, 474.0, "AMOUNT");
    c += &show("F1", 7.0, xs[6] + 4.0, 482.0, "GROSS AMOUNT");
    let records = [
        (
            455.0,
            [
                "2030-01-02",
                "1000001",
                "Sample ticket AAA /",
                "1",
                "0.00",
                "0.00",
                "10.00",
            ],
            Some("BBB / CCC"),
        ),
        (
            435.0,
            [
                "2030-01-02",
                "1000002",
                "Seat Linked to",
                "1",
                "0.00",
                "0.00",
                "2.50",
            ],
            Some("document 1000001"),
        ),
        (
            410.0,
            ["2030-01-03", "1000003", "Seat", "1", "0.00", "0.00", "2.50"],
            None,
        ),
    ];
    for (y, cells, more) in records {
        for (k, t) in cells.iter().enumerate() {
            c += &show("F1", 8.0, xs[k] + 4.0, y, t);
        }
        if let Some(t) = more {
            c += &show("F1", 8.0, xs[2] + 4.0, y - 12.0, t);
        }
    }
    c
}

const INVOICE_TABLE: &str = "\
| ISSUING DATE | DOCUMENT NUMBER | TYPE OF SERVICE | QTY | VAT RATE | VAT AMOUNT | GROSS AMOUNT |
| --- | --- | --- | --- | --- | --- | --- |
| 2030-01-02 | 1000001 | Sample ticket AAA / BBB / CCC | 1 | 0.00 | 0.00 | 10.00 |
| 2030-01-02 | 1000002 | Seat Linked to document 1000001 | 1 | 0.00 | 0.00 | 2.50 |
| 2030-01-03 | 1000003 | Seat | 1 | 0.00 | 0.00 | 2.50 |
";

fn assert_invoice_table(out: &ReadOutcome) {
    let md = markdown(out);
    assert!(md.contains(INVOICE_TABLE), "{md}");
    assert!(!md.contains("Merged cells"), "{md}");
    assert!(
        !out.document
            .losses
            .iter()
            .any(|l| l.what.contains("Possible table")),
        "{:?}",
        out.document.losses
    );
    let found = tables(page(out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, 2, "{found:?}");
}

#[test]
fn column_rules_without_row_rules() {
    // The records of a table whose rules part only the columns and the header are rows by
    // their baselines; a second line joins the record above it.
    let out = read(&sized_page(&invoice_table(), 842.0, 595.0, ""));
    assert_invoice_table(&out);
}

#[test]
fn turned_invoice_table() {
    // The same table on a portrait page turned for display, its content drawn upward: read
    // in the turned frame, it is the same table.
    let content = format!("q 0 1 -1 0 595 0 cm\n{}Q\n", invoice_table());
    let out = read(&sized_page(&content, 595.0, 842.0, "/Rotate 90"));
    assert_invoice_table(&out);
}

/// A page with a header band of filled cells over items without rules: on the right of an
/// address, a label and value column at other baselines; the band's four labelled cells
/// leave a gap with no label between "Description" and "Charges"; 25 items of a date, a
/// text and an amount, one with a fourth piece in the gap and the last with its amount under
/// "Credits"; a rule under the last item.
fn header_band_page(body_lines: usize, prose: bool) -> String {
    let mut c = String::new();
    for (k, t) in ["A. Sample", "Main Road 1", "1000 Town"].iter().enumerate() {
        c += &show("F1", 9.0, 40.0, 760.0 - 11.6 * k as f32, t);
    }
    for (k, (l, v)) in [
        ("Room:", "101"),
        ("Arrival:", "2030-01-02"),
        ("Departure:", "2030-01-05"),
        ("Folio:", "1000"),
    ]
    .iter()
    .enumerate()
    {
        let y = 756.3 - 15.9 * k as f32;
        c += &show("F1", 9.0, 380.0, y, l);
        c += &show("F1", 9.0, 470.0, y, v);
    }
    c += "0.9 g 30 666 60 14 re f 90 666 170 14 re f 400 666 80 14 re f 480 666 85 14 re f 0 g ";
    c += "0.5 w 30 680 m 565 680 l S 30 666 m 565 666 l S ";
    for (x, t) in [
        (34.0, "Date"),
        (94.0, "Description"),
        (404.0, "Charges"),
        (484.0, "Credits"),
    ] {
        c += &show("F2", 8.0, x, 670.0, t);
    }
    for k in 0..body_lines {
        let y = 650.0 - 14.0 * k as f32;
        if prose {
            c += &show(
                "F1",
                8.0,
                34.0,
                y,
                "and the text below the band runs on as a long line of prose across all the columns",
            );
            continue;
        }
        c += &show("F1", 8.0, 34.0, y, &format!("2030-01-{:02}", k % 28 + 1));
        c += &show("F1", 8.0, 94.0, y, &format!("Item {k}"));
        if k == 7 {
            c += &show("F1", 8.0, 300.0, y, "Card 1234");
        }
        let x = if k + 1 == body_lines { 520.0 } else { 440.0 };
        c += &show("F1", 8.0, x, y, &format!("{}.50", 10 + k));
    }
    let bottom = 650.0 - 14.0 * body_lines as f32 + 6.0;
    c += &format!("30 {bottom} m 565 {bottom} l S ");
    c += &show("F1", 9.0, 40.0, bottom - 30.0, "Thank you for your stay.");
    c
}

#[test]
fn header_band_table() {
    let out = read(&pdf(&[Page::text(header_band_page(25, false))], ""));
    let blocks = page(&out, 1);
    let found = tables(blocks);
    assert_eq!(found.len(), 2, "{found:#?}");
    let (head, kv) = &found[0];
    assert_eq!(*head, 0);
    assert_eq!(
        kv,
        &[
            ["Room:", "101"],
            ["Arrival:", "2030-01-02"],
            ["Departure:", "2030-01-05"],
            ["Folio:", "1000"],
        ]
        .iter()
        .map(|r| r.iter().map(|c| c.to_string()).collect::<Vec<_>>())
        .collect::<Vec<_>>()
    );
    let (head, rows) = &found[1];
    assert_eq!(*head, 1, "{rows:?}");
    assert_eq!(rows.len(), 26, "{rows:?}");
    assert_eq!(rows[0], ["Date", "Description", "", "Charges", "Credits"]);
    assert_eq!(rows[1], ["2030-01-01", "Item 0", "", "10.50", ""]);
    assert_eq!(rows[8], ["2030-01-08", "Item 7", "Card 1234", "17.50", ""]);
    assert_eq!(rows[25], ["2030-01-25", "Item 24", "", "", "34.50"]);
    let paras = paragraphs(blocks);
    assert!(!paras.iter().any(|p| p.contains("Item")), "{paras:?}");
    assert!(
        paras.iter().any(|p| p == "Thank you for your stay."),
        "{paras:?}"
    );
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
fn header_band_over_prose_says_so() {
    // A header band of four labelled cells over lines of prose that cross its columns: no
    // table, and one note says a table may stand there.
    let out = read(&pdf(&[Page::text(header_band_page(5, true))], ""));
    let blocks = page(&out, 1);
    let found = tables(blocks);
    assert!(
        !found
            .iter()
            .any(|(_, rows)| rows.iter().flatten().any(|c| c.contains("prose"))),
        "{found:?}"
    );
    let notes: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| {
            l.what
                .starts_with("Possible table on page 1 not rebuilt (reason: ")
        })
        .collect();
    assert_eq!(notes.len(), 1, "{:?}", out.document.losses);
    assert_eq!(notes[0].why, LossReason::Uncertain);
}

#[test]
fn key_value_totals_box() {
    // A box of five labels on the left and amounts set flush right.
    let mut c = String::from("0.5 w 300 100 260 95 re S ");
    let rows = [
        ("Total excluded", "100.00"),
        ("Total tax", "0.00"),
        ("Shipping", "5.00"),
        ("Discount", "2.00"),
        ("Total", "103.00"),
    ];
    for (k, (l, v)) in rows.iter().enumerate() {
        let y = 175.0 - 15.0 * k as f32;
        c += &show("F1", 10.0, 310.0, y, l);
        c += &show("F1", 10.0, 550.0 - helvetica_width(v, 10.0), y, v);
    }
    c += &show("F1", 10.0, 40.0, 400.0, "Thank you for your order.");
    let out = read(&pdf(&[Page::text(c)], ""));
    let md = markdown(&out);
    let expected = "\
|  |  |
| --- | --- |
| Total excluded | 100.00 |
| Total tax | 0.00 |
| Shipping | 5.00 |
| Discount | 2.00 |
| Total | 103.00 |
";
    assert!(md.contains(expected), "{md}");
}

#[test]
fn header_icons_count_as_header_cells() {
    // A ruled table whose first title is a small drawn calendar, not text: the first row is
    // still the header, and the icon is said.
    let mut c = String::from("0.5 w ");
    c += &ruled_cells(&[50.0, 130.0, 210.0, 290.0], "c");
    let out = read(&pdf(&[Page::text(c.clone())], ""));
    assert_eq!(tables(page(&out, 1))[0].0, 0);
    // The same grid with an icon and titles in the first row, times below.
    let mut c = String::from("0.5 w ");
    for k in 0..4 {
        let y = 700.0 - 20.0 * k as f32;
        c += &format!("50 {y} m 290 {y} l S ");
    }
    for x in [50.0, 130.0, 210.0, 290.0] {
        c += &format!("{x} 640 m {x} 700 l S ");
    }
    c += "84 684 m 96 684 l S 84 692 m 96 692 l S 84 684 m 84 694 l S 96 684 m 96 694 l S ";
    c += &show("F1", 8.0, 134.0, 686.0, "Time");
    c += &show("F1", 8.0, 214.0, 686.0, "Train");
    for (r, (d, t, n)) in [
        ("2030-01-02", "10:15", "IC 101"),
        ("2030-01-03", "11:20", "IC 205"),
    ]
    .iter()
    .enumerate()
    {
        let y = 666.0 - 20.0 * r as f32;
        c += &show("F1", 8.0, 54.0, y, d);
        c += &show("F1", 8.0, 134.0, y, t);
        c += &show("F1", 8.0, 214.0, y, n);
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, 1, "{found:?}");
    assert_eq!(found[0].1[0], ["(icon)", "Time", "Train"], "{found:?}");
}

#[test]
fn a_page_wide_grid_full_of_text_says_so() {
    // A ruled grid of 20 columns and 16 rows over most of a landscape page, with a number in
    // every cell: too many columns for a table, and as large as the page; still a note says a
    // table may stand there, as text fills it.
    let mut c = String::from("0.5 w ");
    let (cols, rows) = (20usize, 16usize);
    let (x0, top, w, h) = (30.0f32, 580.0f32, 39.0f32, 34.0f32);
    for k in 0..=rows {
        let y = top - h * k as f32;
        c += &format!("{x0} {y} m {} {y} l S ", x0 + w * cols as f32);
    }
    for k in 0..=cols {
        let x = x0 + w * k as f32;
        c += &format!("{x} {} m {x} {top} l S ", top - h * rows as f32);
    }
    for r in 0..rows {
        for k in 0..cols {
            let x = x0 + 3.0 + w * k as f32;
            let y = top - h * r as f32 - 20.0;
            c += &show("F1", 8.0, x, y, &format!("{}", r * 100 + k));
        }
    }
    let out = read(&sized_page(&c, 842.0, 595.0, ""));
    let note = "Possible table on page 1 not rebuilt (reason: it has more than 15 columns)";
    assert!(
        out.document.losses.iter().any(|l| l.what == note),
        "{:?}",
        out.document.losses
    );
}
