// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables: from the tag tree, from drawn rules and from white space gutters, and the places
//! where none may be found.

mod common;

use std::path::PathBuf;

use common::{Builder, Page, helvetica_width, markdown, pdf, show, sized_page};
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Anchor, Block, Cell, Completeness, Inline, LossReason, ReadOptions, ReadOutcome, SectionKind,
    Severity, plain_text,
};

fn corpus_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf")
        .join(name)
}

fn read_with(bytes: &[u8], limits: Limits) -> ReadOutcome {
    common::read_in(bytes, &ReadOptions::default(), &Budget::new(limits))
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

/// Review focus 2 (Task 19): a table the tag tree states stays the given table. Its region
/// is exact (the file states it, no rule typed it again), and each of its cell texts stands
/// once in the Markdown, never also in a paragraph.
#[test]
fn tagged_table_stays_given_and_once() {
    let out = read(&tagged_span_table());
    let notes: Vec<&rezzer_model::RegionNote> = out
        .document
        .regions
        .iter()
        .filter(|r| r.anchor.0 == "p1:t1")
        .collect();
    assert_eq!(notes.len(), 1, "{:#?}", out.document.regions);
    assert_eq!(notes[0].kind, rezzer_model::RegionKind::Table);
    assert_eq!(notes[0].certainty, rezzer_model::Certainty::Exact);
    let md = markdown(&out);
    for cell in ["Both", "Left", "Right"] {
        assert_eq!(md.matches(cell).count(), 1, "{cell}: {md}");
    }
    assert!(paragraphs(page(&out, 1)).is_empty(), "{md}");
}

/// A tagged page with a table of eight visible columns (a bold-free header and four item rows,
/// 64 points apart) whose tags hold two cells a row: the first column, and the rest of the row
/// in one cell (an item table as some producers tag it).
fn tagged_glued_table() -> (Vec<u8>, Vec<Vec<String>>) {
    let titles = [
        "Pos", "Nr", "Text", "Menge", "Einh", "Preis", "MwSt", "Summe",
    ];
    let mut rows: Vec<Vec<String>> = vec![titles.iter().map(|t| t.to_string()).collect()];
    for k in 0..4u32 {
        rows.push(vec![
            format!("{}", k + 1),
            format!("A-10{k}"),
            "Muster".to_string(),
            format!("{}", k + 2),
            "Stk".to_string(),
            "12,50".to_string(),
            "19%".to_string(),
            "25,00".to_string(),
        ]);
    }
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let kid = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut content = String::from("BT /F1 9 Tf ");
    let mut mcid = 0;
    for (r, row) in rows.iter().enumerate() {
        let y = 700.0 - 13.0 * r as f32;
        let tag = if r == 0 { "TH" } else { "TD" };
        // The first cell, then the rest of the row in one marked content.
        content += &format!(
            "/{tag} <</MCID {mcid}>> BDC 1 0 0 1 40 {y} Tm ({}) Tj EMC ",
            row[0]
        );
        content += &format!("/{tag} <</MCID {}>> BDC ", mcid + 1);
        for (c, t) in row.iter().enumerate().skip(1) {
            content += &format!("1 0 0 1 {} {y} Tm ({t}) Tj ", 40.0 + 64.0 * c as f32);
        }
        content += "EMC ";
        mcid += 2;
    }
    content += "ET";
    let cs = b.stream("", content.as_bytes());
    let root = b.reserve();
    let table = b.reserve();
    let trs: Vec<u32> = rows.iter().map(|_| b.reserve()).collect();
    let mut cells = Vec::new();
    for (r, tr) in trs.iter().enumerate() {
        let tag = if r == 0 { "TH" } else { "TD" };
        let a = b.add(format!(
            "<< /Type /StructElem /S /{tag} /P {tr} 0 R /Pg {kid} 0 R /K {} >>",
            2 * r
        ));
        let z = b.add(format!(
            "<< /Type /StructElem /S /{tag} /P {tr} 0 R /Pg {kid} 0 R /K {} >>",
            2 * r + 1
        ));
        b.set(
            *tr,
            format!("<< /Type /StructElem /S /TR /P {table} 0 R /K [{a} 0 R {z} 0 R] >>"),
        );
        cells.push(format!("{a} 0 R {z} 0 R"));
    }
    let kids: Vec<String> = trs.iter().map(|t| format!("{t} 0 R")).collect();
    b.set(
        table,
        format!(
            "<< /Type /StructElem /S /Table /P {root} 0 R /K [{}] >>",
            kids.join(" ")
        ),
    );
    b.set(
        root,
        format!(
            "<< /Type /StructTreeRoot /K {table} 0 R /ParentTree << /Nums [0 [{}]] >> >>",
            cells.join(" ")
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
    (b.finish(catalog), rows)
}

/// Task 19b (carried: tagged tables are checked against geometry): a tagged table whose tags
/// put the rest of each row in one cell, so two tagged columns hold eight visible ones, loses
/// its certainty: its glued cells are cut where their text stands apart, and its eight columns
/// are a table with each value in its own cell. A note says why.
#[test]
fn a_tagged_table_whose_cells_cross_its_columns_is_cut_into_them() {
    let (bytes, rows) = tagged_glued_table();
    let out = read(&bytes);
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}\n{}", markdown(&out));
    assert_eq!(found[0].1, rows, "{}", markdown(&out));
    assert_no_exact_table(&out);
    assert_glued_note(&out, 7, 1);
}

/// No table region of `out` is exact: the tag tree is no exact reading of a table it glues.
fn assert_no_exact_table(out: &ReadOutcome) {
    assert!(
        out.document
            .regions
            .iter()
            .all(|r| r.certainty != rezzer_model::Certainty::Exact
                || r.kind != rezzer_model::RegionKind::Table),
        "{:#?}",
        out.document.regions
    );
}

/// `out` says that a tagged table on page 1 holds the text of `columns` columns in `glued`
/// of its tagged columns.
fn assert_glued_note(out: &ReadOutcome, columns: usize, glued: usize) {
    let want = format!(
        "a tagged table on page 1 holds the text of {columns} columns in {glued} of its tagged \
         columns; its cells are cut where that text lines up apart over its rows"
    );
    let notes: Vec<&str> = out
        .document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .collect();
    assert!(notes.contains(&want.as_str()), "{notes:?}");
}

/// One run of a tagged table: x, baseline, size and text.
type TagRun = (f32, f32, f32, String);

/// A tagged page with a table of two tagged cells a row, the first holding the runs left of
/// `split`, the second all the others (header row `TH`), each run in a marked content of its
/// own (as a producer writes text pieces of one cell), drawn with rules that agree with the
/// tags: one column rule at `split` and a rule under each row (`bottoms`). With `legend` not
/// empty, a tagged list of one item (`L`, `LI`, `LBody`) of those runs stands under the table.
fn two_cell_tagged_table(
    rows: &[Vec<TagRun>],
    split: f32,
    bottoms: &[f32],
    legend: &[TagRun],
) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let kid = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let top = rows[0].iter().map(|r| r.1).fold(f32::MIN, f32::max) + 12.0;
    let low = bottoms.iter().copied().fold(f32::MAX, f32::min);
    let mut content = format!("0.5 w 30 {top} m 520 {top} l S {split} {low} m {split} {top} l S ");
    for y in bottoms {
        content += &format!("30 {y} m 520 {y} l S ");
    }
    // The marked contents of each cell, by row and cell.
    let mut mcids: Vec<[Vec<usize>; 2]> = Vec::new();
    let mut next = 0usize;
    for (r, row) in rows.iter().enumerate() {
        let tag = if r == 0 { "TH" } else { "TD" };
        let mut cells: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        for (cell, ids) in cells.iter_mut().enumerate() {
            for (x, y, size, t) in row.iter().filter(|r| (r.0 < split) == (cell == 0)) {
                content += &format!(
                    "/{tag} <</MCID {next}>> BDC BT /F1 {size} Tf 1 0 0 1 {x} {y} Tm ({t}) Tj \
                     ET EMC "
                );
                ids.push(next);
                next += 1;
            }
        }
        mcids.push(cells);
    }
    let mut legend_ids = Vec::new();
    for (x, y, size, t) in legend {
        content += &format!(
            "/LBody <</MCID {next}>> BDC BT /F1 {size} Tf 1 0 0 1 {x} {y} Tm ({t}) Tj ET EMC "
        );
        legend_ids.push(next);
        next += 1;
    }
    let cs = b.stream("", content.as_bytes());
    let root = b.reserve();
    let table = b.reserve();
    let trs: Vec<u32> = rows.iter().map(|_| b.reserve()).collect();
    let mut parents: Vec<String> = vec![String::new(); next];
    for (r, tr) in trs.iter().enumerate() {
        let tag = if r == 0 { "TH" } else { "TD" };
        let mut elems = Vec::new();
        for ids in &mcids[r] {
            let k: Vec<String> = ids.iter().map(|m| m.to_string()).collect();
            let e = b.add(format!(
                "<< /Type /StructElem /S /{tag} /P {tr} 0 R /Pg {kid} 0 R /K [{}] >>",
                k.join(" ")
            ));
            for &m in ids {
                parents[m] = format!("{e} 0 R");
            }
            elems.push(format!("{e} 0 R"));
        }
        b.set(
            *tr,
            format!(
                "<< /Type /StructElem /S /TR /P {table} 0 R /K [{}] >>",
                elems.join(" ")
            ),
        );
    }
    let mut root_kids = format!("{table} 0 R");
    if !legend_ids.is_empty() {
        let (list, item) = (b.reserve(), b.reserve());
        let k: Vec<String> = legend_ids.iter().map(|m| m.to_string()).collect();
        let body = b.add(format!(
            "<< /Type /StructElem /S /LBody /P {item} 0 R /Pg {kid} 0 R /K [{}] >>",
            k.join(" ")
        ));
        for &m in &legend_ids {
            parents[m] = format!("{body} 0 R");
        }
        b.set(
            item,
            format!("<< /Type /StructElem /S /LI /P {list} 0 R /K [{body} 0 R] >>"),
        );
        b.set(
            list,
            format!("<< /Type /StructElem /S /L /P {root} 0 R /K [{item} 0 R] >>"),
        );
        root_kids += &format!(" {list} 0 R");
    }
    let kids: Vec<String> = trs.iter().map(|t| format!("{t} 0 R")).collect();
    b.set(
        table,
        format!(
            "<< /Type /StructElem /S /Table /P {root} 0 R /K [{}] >>",
            kids.join(" ")
        ),
    );
    b.set(
        root,
        format!(
            "<< /Type /StructTreeRoot /K [{root_kids}] /ParentTree << /Nums [0 [{}]] >> >>",
            parents.join(" ")
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

/// Fix round 1 (F2): an item table tagged as two cells a row (a date, and everything else),
/// with rules that agree with the tags: in the second cell the text stands in seven columns
/// that line up over a header of two lines and every item row; descriptions wrap onto lines
/// with nothing in the other columns, and an item without a price has no tax, net or gross.
/// The tagged cells are cut into the columns: header and every row in eight columns, the
/// wrapped lines in the description cell, empty cells where the free item has no amounts.
#[test]
fn a_tagged_item_table_glued_behind_its_first_cell_is_cut_into_its_columns() {
    let out = read(&glued_item_table(&[]));
    check_glued_items(&out, "1 19 %");
}

/// Fix round 3 (H1): the raised mark before each rate is a reference to a footnote when the
/// page's tags give a list item that opens with the same raised number: the rate cell holds
/// the rate and the reference after it, the item is the footnote, not a list item.
#[test]
fn a_raised_mark_in_a_tagged_cell_refers_to_the_tagged_legend() {
    let legend = [
        (40.0, 402.9, 5.0, "1".to_string()),
        (44.0, 400.0, 8.0, "Muster Hinweis zum Satz".to_string()),
    ];
    let out = read(&glued_item_table(&legend));
    check_glued_items(&out, "19 %");
    // The reference stands after the rate in its cell.
    let cell = page(&out, 1)
        .iter()
        .find_map(|b| match b {
            Block::Table { rows, .. } => Some(rows[1][4].inlines.clone()),
            _ => None,
        })
        .expect("a table");
    assert!(
        matches!(cell.last(), Some(rezzer_model::Inline::FootnoteRef { id }) if id == "p1-1"),
        "{cell:?}"
    );
    let md = markdown(&out);
    assert!(md.contains("[^p1-1]: Muster Hinweis zum Satz"), "{md}");
    assert!(!md.contains("- 1 Muster"), "{md}");
}

/// The item table of [`a_tagged_item_table_glued_behind_its_first_cell_is_cut_into_its_columns`]
/// with the tagged list item `legend` under it.
fn glued_item_table(legend: &[TagRun]) -> Vec<u8> {
    let size = 8.0f32;
    let right = |edge: f32, t: &str| edge - helvetica_width(t, size);
    let run = |x: f32, y: f32, t: &str| (x, y, size, t.to_string());
    // The rate column starts a little more than a quarter size after the hyphenated title
    // over the service dates; a raised mark of size 5 stands right before each rate, under
    // the end of that title and well after the date.
    let rate = 258.0 + helvetica_width("Leistungs-", size) + 2.7;
    let mark = |y: f32| {
        (
            rate - 2.9 - helvetica_width("1", 5.0),
            y + 2.9,
            5.0,
            "1".to_string(),
        )
    };
    let header = |y: f32| {
        vec![
            run(40.0, y, "Datum"),
            run(80.0, y, "Bezeichnung"),
            run(right(250.0, "Preis"), y, "Preis"),
            run(258.0, y, "Leistungs-"),
            run(rate, y, "Satz"),
            run(right(390.0, "Steuer"), y, "Steuer"),
            run(right(440.0, "Netto"), y, "Netto"),
            run(right(490.0, "Brutto"), y, "Brutto"),
            run(258.0, y - 10.0, "datum"),
            run(right(390.0, "Euro"), y - 10.0, "Euro"),
            run(right(440.0, "Euro"), y - 10.0, "Euro"),
            run(right(490.0, "Euro"), y - 10.0, "Euro"),
        ]
    };
    let item = |y: f32, v: [&str; 8], wraps: &[&str]| {
        let mut out = vec![
            run(40.0, y, v[0]),
            run(80.0, y, v[1]),
            run(right(250.0, v[2]), y, v[2]),
            run(258.0, y, v[3]),
        ];
        if !v[4].is_empty() {
            out.push(mark(y));
            out.push(run(rate, y, v[4]));
            out.push(run(right(390.0, v[5]), y, v[5]));
            out.push(run(right(440.0, v[6]), y, v[6]));
            out.push(run(right(490.0, v[7]), y, v[7]));
        }
        for (k, w) in wraps.iter().enumerate() {
            out.push(run(80.0, y - 10.0 * (k + 1) as f32, w));
        }
        out
    };
    let rows = vec![
        header(600.0),
        item(
            574.0,
            [
                "01.02.26",
                "Muster Teil A,",
                "12,50",
                "3.2.26",
                "19 %",
                "2,38",
                "12,50",
                "14,88",
            ],
            &["Lager Nord, Fach", "(Muster)"],
        ),
        item(
            530.0,
            [
                "04.02.26",
                "Muster Teil B",
                "0,00",
                "5.2.26",
                "",
                "",
                "",
                "",
            ],
            &["Lager West"],
        ),
        item(
            496.0,
            [
                "06.02.26",
                "Muster Teil C",
                "8,40",
                "7.2.26",
                "19 %",
                "1,60",
                "8,40",
                "10,00",
            ],
            &[],
        ),
    ];
    two_cell_tagged_table(&rows, 76.0, &[586.0, 542.0, 508.0, 492.0], legend)
}

/// The cells of [`glued_item_table`] as read in `out`, the rate cells holding `rate`.
fn check_glued_items(out: &ReadOutcome, rate: &str) {
    let found = tables(page(out, 1));
    assert_eq!(found.len(), 1, "{found:?}\n{}", markdown(out));
    let want: Vec<Vec<&str>> = vec![
        vec![
            "Datum",
            "Bezeichnung",
            "Preis",
            "Leistungs-datum",
            "Satz",
            "Steuer Euro",
            "Netto Euro",
            "Brutto Euro",
        ],
        vec![
            "01.02.26",
            "Muster Teil A, Lager Nord, Fach (Muster)",
            "12,50",
            // The date alone: the raised mark before the rate is no ink of the date's column; it
            // stands with the rate, the run nearest to it (the model has no inline for a bare
            // mark that no footnote of the page names).
            "3.2.26",
            rate,
            "2,38",
            "12,50",
            "14,88",
        ],
        vec![
            "04.02.26",
            "Muster Teil B Lager West",
            "0,00",
            "5.2.26",
            "",
            "",
            "",
            "",
        ],
        vec![
            "06.02.26",
            "Muster Teil C",
            "8,40",
            "7.2.26",
            rate,
            "1,60",
            "8,40",
            "10,00",
        ],
    ];
    assert_eq!(found[0].1, want, "{}", markdown(out));
    assert_no_exact_table(out);
    assert_glued_note(out, 7, 1);
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

/// A 3 x 3 ruled grid is one table region, found by the ruled pass, and holds the runs of its
/// nine cells.
#[test]
fn ruled_grid_is_a_table_region() {
    let bytes = ruled_page(&[
        ["Fruit", "Count", "Price"],
        ["Apples", "3", "1.20"],
        ["Pears", "12", "0.80"],
    ]);
    common::assert_every_run_in_one_region(&bytes);
    let (_, pages) = common::read_regions(&bytes);
    let tables: Vec<&rezzer_read_pdf::layoutregions::Region> = pages
        .iter()
        .flat_map(|p| &p.regions)
        .filter(|r| r.kind == rezzer_read_pdf::layoutregions::Kind::Table)
        .collect();
    assert_eq!(tables.len(), 1, "{pages:#?}");
    assert_eq!(tables[0].signals[0].id, "pdf.ruled");
    assert_eq!(tables[0].prims.len(), 9, "{:?}", tables[0].prims);
}

/// The bounds of a ruled table reach its outer rules, past the box of its runs, and the
/// paragraphs around it are regions of their own at their blocks.
#[test]
fn a_ruled_table_region_reaches_its_rules() {
    let out = read(&ruled_page(&[
        ["Fruit", "Count", "Price"],
        ["Apples", "3", "1.20"],
        ["Pears", "12", "0.80"],
    ]));
    let regions = &out.document.regions;
    let table = regions
        .iter()
        .find(|n| n.kind == rezzer_model::RegionKind::Table)
        .expect("a table region");
    assert_eq!(table.bounds, [72.0, 610.0, 432.0, 700.0], "{table:#?}");
    assert_eq!(table.anchor.0, "p1:t1");
    assert!(
        matches!(&table.evidence[..], [rezzer_model::Evidence::Rule { id, .. }] if id == "pdf.ruled"),
        "{table:#?}"
    );
    let paragraphs: Vec<&str> = regions
        .iter()
        .filter(|n| n.kind == rezzer_model::RegionKind::Paragraph)
        .map(|n| n.anchor.0.as_str())
        .collect();
    // Task 19: the engine may decide the text above and below the table as one paragraph
    // region (one segment), at its first block; a finder of today gave each block its own.
    assert!(
        paragraphs == ["p1:b1", "p1:b2"] || paragraphs == ["p1:b1"],
        "{regions:#?}"
    );
}

/// A grid of drawn rules over most of the page (the page frame of plan 6) is refused: its
/// runs stay paragraphs, each in a paragraph region that says why the grid is no table.
#[test]
fn refused_grid_keeps_its_text_in_paragraph_regions_with_the_reason() {
    let mut c = String::from("0.5 w ");
    for y in [60.0, 410.0, 760.0] {
        c += &format!("30 {y} m 565 {y} l S ");
    }
    for x in [30.0, 297.0, 565.0] {
        c += &format!("{x} 60 m {x} 760 l S ");
    }
    let texts = ["Muster GmbH", "Erika Mustermann", "1.234,56"];
    c += &show("F1", 10.0, 40.0, 740.0, texts[0]);
    c += &show("F1", 10.0, 307.0, 740.0, texts[1]);
    c += &show("F1", 10.0, 40.0, 390.0, texts[2]);
    let bytes = pdf(&[Page::text(c)], "");
    common::assert_every_run_in_one_region(&bytes);
    let (out, pages) = common::read_engine_pages(&bytes);
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what.contains("it covers most of the page")),
        "{:?}",
        out.document.losses
    );
    let page = &pages[0];
    for t in texts {
        let holders: Vec<&rezzer_read_pdf::layoutregions::Region> = page
            .regions
            .regions
            .iter()
            .filter(|r| common::region_texts(page, r).iter().any(|x| x.contains(t)))
            .collect();
        assert_eq!(holders.len(), 1, "{t}: {holders:#?}");
        assert_eq!(
            holders[0].kind,
            rezzer_read_pdf::layoutregions::Kind::Paragraph,
            "{t}"
        );
        assert!(
            holders[0]
                .unsure
                .iter()
                .any(|u| u.contains("it covers most of the page")),
            "{t}: {:?}",
            holders[0].unsure
        );
    }
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
    // Task 19a-5b: the check's lone line may be kept as laid out (no region type has clear
    // evidence for it; a right read turned Spatial, Task 36): its text stays whole.
    let laid_out = page(&sf, 4).iter().any(|b| match b {
        Block::Spatial { lines, .. } => lines
            .iter()
            .any(|l| l.pieces.iter().any(|p| p.text.contains("AUSTIN, TEXAS"))),
        _ => false,
    });
    assert!(
        laid_out
            || paragraphs(page(&sf, 4))
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

/// A caption names a table and short pieces stand in columns, but one line has text in the
/// gutter between two columns: no grid holds it.
fn gutter_region_page() -> Vec<u8> {
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
    pdf(&[Page::text(c)], "")
}

#[test]
fn unsure_region_keeps_paragraphs_with_note() {
    // The text of the region of `gutter_region_page` stays paragraphs, with a note where the
    // region stands and a loss.
    let out = read(&gutter_region_page());
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

/// A ruled grid of 20 columns and 3 rows with text in every cell, and no caption.
fn wide_ruled_grid() -> Vec<u8> {
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
    pdf(&[Page::text(c)], "")
}

#[test]
fn wide_ruled_grid_says_so() {
    let out = read(&wide_ruled_grid());
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

/// Task 17 D9: a sentence printed over the last row of a ruled table, its word gap on a column
/// rule, is no text of the table's cells: it is read as a paragraph next to the table, with a
/// note that says why. The rows above keep their cells.
#[test]
fn a_sentence_over_a_ruled_row_is_a_paragraph_next_to_the_table() {
    let xs = [50.0f32, 150.0, 250.0, 350.0];
    let mut c = String::from("0.5 w ");
    for k in 0..5 {
        let y = 700.0 - 20.0 * k as f32;
        c += &format!("{} {y} m {} {y} l S ", xs[0], xs[3]);
    }
    for x in xs {
        c += &format!("{x} 620 m {x} 700 l S ");
    }
    let rows = [
        ["Teil", "Menge", "Lager"],
        ["Muster A", "2", "Nord"],
        ["Muster B", "1", "West"],
    ];
    for (r, row) in rows.iter().enumerate() {
        let y = 686.0 - 20.0 * r as f32;
        for (k, t) in row.iter().enumerate() {
            c += &show("F1", 8.0, xs[k] + 4.0, y, t);
        }
    }
    // The sentence: its first words end right before the rule at 150, the rest starts a word
    // gap after it and runs over the rule at 250.
    let first = "die Ware wird nach dem";
    let x0 = 149.0 - helvetica_width(first, 8.0);
    c += &show("F1", 8.0, x0, 626.0, first);
    c += &show(
        "F1",
        8.0,
        151.5,
        626.0,
        "Erhalt der Rechnung geliefert und bezahlt",
    );
    let out = read(&pdf(&[Page::text(c)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].1,
        [
            ["Teil", "Menge", "Lager"],
            ["Muster A", "2", "Nord"],
            ["Muster B", "1", "West"],
        ],
        "{found:?}"
    );
    let paras = paragraphs(page(&out, 1));
    assert!(
        paras
            .iter()
            .any(|p| p == "die Ware wird nach dem Erhalt der Rechnung geliefert und bezahlt"),
        "{paras:?}"
    );
    let notes: Vec<&str> = out
        .document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .collect();
    assert!(
        notes.contains(
            &"text over the table on page 1 that starts with \"die Ware wird nach dem \
              Erhalt\" reads as a sentence across the columns of its row: it is read as a \
              paragraph next to the table"
        ),
        "{notes:?}"
    );
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

/// Task 19b (carried from Task 18): a ruled parts list whose drawing column holds "Abb. 3"
/// cells is a table read by its rules. Such a cell points to a figure, it captions none; only
/// a caption alone on its line in a grid speaks for a chart.
#[test]
fn a_ruled_table_with_figure_reference_cells_is_a_table() {
    let xs = [50.0f32, 150.0, 250.0, 350.0];
    let rows = [
        ["Teil", "Menge", "Zeichnung"],
        ["Muster A", "2", "Abb. 3"],
        ["Muster B", "1", "Abb. 4"],
        ["Muster C", "4", "Abbildung 5"],
    ];
    let mut c = String::from("0.5 w ");
    for k in 0..5 {
        let y = 700.0 - 20.0 * k as f32;
        c += &format!("{} {y} m {} {y} l S ", xs[0], xs[3]);
    }
    for x in xs {
        c += &format!("{x} 620 m {x} 700 l S ");
    }
    for (r, row) in rows.iter().enumerate() {
        let y = 686.0 - 20.0 * r as f32;
        for (k, t) in row.iter().enumerate() {
            c += &show("F1", 8.0, xs[k] + 4.0, y, t);
        }
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].1, rows, "{found:?}");
    // The grid's own rules read it, not a reader without rules after a refusal.
    let note = out
        .document
        .regions
        .iter()
        .find(|r| r.anchor.0 == "p1:t1")
        .expect("the table's region");
    assert!(
        note.evidence
            .iter()
            .any(|e| matches!(e, rezzer_model::Evidence::Rule { id, .. } if id == "pdf.ruled")),
        "{note:#?}"
    );
}

/// Fix round 1 (F1): the drawing of a grid next to a figure caption is a figure with no media
/// block of its own and no text (the grid's text is its own region): it wrote nothing on the
/// page, so its region stands at the page. A figure or code region with text and no media block
/// keeps the block of its text (lib.rs); no such region occurs in the three dump sets.
#[test]
fn a_figure_without_a_media_block_or_text_stands_at_the_page() {
    let mut c = String::from("0.5 w ");
    c += &ruled_cells(&[50.0, 130.0, 210.0, 290.0], "f");
    c += &show("F1", 9.0, 50.0, 630.0, "Figure 3: A chart of the results");
    let out = read(&pdf(&[Page::text(c)], ""));
    let figures: Vec<&rezzer_model::RegionNote> = out
        .document
        .regions
        .iter()
        .filter(|r| r.kind == rezzer_model::RegionKind::Figure)
        .collect();
    assert_eq!(figures.len(), 1, "{:#?}", out.document.regions);
    assert_eq!(figures[0].anchor.0, "p1", "{figures:#?}");
    assert!(
        figures[0].evidence.iter().any(|e| matches!(
            e,
            rezzer_model::Evidence::Rule { id, .. } if id == "figure.drawn_caption"
        )),
        "{figures:#?}"
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
| 2030-01-02 | 1000001 | Sample ticket AAA /<br>BBB / CCC | 1 | 0.00 | 0.00 | 10.00 |
| 2030-01-02 | 1000002 | Seat Linked to<br>document 1000001 | 1 | 0.00 | 0.00 | 2.50 |
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

/// A page with a header band of four filled cells `Art | Basis | Satz | Betrag` over lines
/// without rules, each line a list of (column, text): the columns start at 40, 160, 280 and
/// 400, the lines 14 points apart from the top.
fn tax_band_page(lines: &[&[(usize, &str)]]) -> String {
    let xs = [40.0, 160.0, 280.0, 400.0];
    let mut c = String::from("0.9 g ");
    for x in xs {
        c += &format!("{} 666 120 14 re f ", x - 4.0);
    }
    c += "0 g ";
    for (x, t) in xs.iter().zip(["Art", "Basis", "Satz", "Betrag"]) {
        c += &show("F2", 8.0, *x, 670.0, t);
    }
    for (k, line) in lines.iter().enumerate() {
        let y = 650.0 - 14.0 * k as f32;
        for (col, t) in *line {
            c += &show("F1", 8.0, xs[*col], y, t);
        }
    }
    c += &show("F1", 9.0, 40.0, 500.0, "Thank you.");
    c
}

/// The rows of a table as written, for comparing.
fn owned(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|c| c.to_string()).collect())
        .collect()
}

#[test]
fn sparse_row_keeps_its_columns() {
    // A line with only an amount is a row of its own: a number never joins the row above,
    // and its empty cells stay empty.
    let page_text = tax_band_page(&[
        &[(0, "Steuer A"), (1, "1.000,00"), (2, "10 %"), (3, "100,00")],
        &[(3, "20,00")],
        &[(0, "Steuer C"), (1, "500,00"), (2, "5 %"), (3, "25,00")],
        &[(3, "5,00")],
    ]);
    let out = read(&pdf(&[Page::text(page_text)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    let (head, rows) = &found[0];
    assert_eq!(*head, 1, "{rows:?}");
    assert_eq!(
        rows,
        &owned(&[
            &["Art", "Basis", "Satz", "Betrag"],
            &["Steuer A", "1.000,00", "10 %", "100,00"],
            &["", "", "", "20,00"],
            &["Steuer C", "500,00", "5 %", "25,00"],
            &["", "", "", "5,00"],
        ])
    );
}

/// Guard: the header band of `sparse_row_keeps_its_columns` (a row of drawn cells, each
/// with only its title) is no row of empty fields: it heads its table, and no note says
/// label and value pairs were found.
#[test]
fn header_band_is_not_empty_fields() {
    let page_text = tax_band_page(&[
        &[(0, "Steuer A"), (1, "1.000,00"), (2, "10 %"), (3, "100,00")],
        &[(3, "20,00")],
        &[(0, "Steuer C"), (1, "500,00"), (2, "5 %"), (3, "25,00")],
        &[(3, "5,00")],
    ]);
    let out = read(&pdf(&[Page::text(page_text)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(found[0].0, 1, "{found:?}");
    assert_eq!(
        found[0].1[0],
        ["Art", "Basis", "Satz", "Betrag"],
        "{found:?}"
    );
    let text = common::all_text(&out);
    assert!(!text.contains("Found by: label and value"), "{text}");
}

#[test]
fn wrapped_cell_still_joins() {
    // Guard: a second line of words in a column that has text above goes on that cell.
    let page_text = tax_band_page(&[
        &[(0, "Steuer A"), (1, "1.000,00"), (2, "10 %"), (3, "100,00")],
        &[(0, "mit Zuschlag")],
        &[(0, "Steuer C"), (1, "500,00"), (2, "5 %"), (3, "25,00")],
        &[(0, "Steuer D"), (1, "200,00"), (2, "5 %"), (3, "10,00")],
    ]);
    let out = read(&pdf(&[Page::text(page_text)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    let rows = &found[0].1;
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert_eq!(rows[1][0], "Steuer A mit Zuschlag", "{rows:?}");
    assert_eq!(rows[2][0], "Steuer C", "{rows:?}");
}

#[test]
fn line_in_an_empty_column_starts_a_row() {
    // A line of words in a column that is empty in the row above starts a row of its own.
    let page_text = tax_band_page(&[
        &[(0, "Steuer A"), (1, "1.000,00"), (3, "100,00")],
        &[(2, "frei")],
        &[(0, "Steuer C"), (1, "500,00"), (2, "5 %"), (3, "25,00")],
    ]);
    let out = read(&pdf(&[Page::text(page_text)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    let rows = &found[0].1;
    assert_eq!(
        rows,
        &owned(&[
            &["Art", "Basis", "Satz", "Betrag"],
            &["Steuer A", "1.000,00", "", "100,00"],
            &["", "", "frei", ""],
            &["Steuer C", "500,00", "5 %", "25,00"],
        ])
    );
}

#[test]
fn a_first_cell_that_wraps_upward_joins_its_row() {
    // The first line of a label alone above the line with the values set at the middle of
    // the row: it goes on that row, not a row of its own.
    let page_text = tax_band_page(&[
        &[(0, "Steuer A")],
        &[
            (0, "mit Zuschlag"),
            (1, "1.000,00"),
            (2, "10 %"),
            (3, "100,00"),
        ],
        &[(0, "Steuer C"), (1, "500,00"), (2, "5 %"), (3, "25,00")],
        &[(0, "Steuer D"), (1, "200,00"), (2, "5 %"), (3, "10,00")],
    ]);
    let out = read(&pdf(&[Page::text(page_text)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    let rows = &found[0].1;
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert_eq!(rows[1][0], "Steuer A mit Zuschlag", "{rows:?}");
}

#[test]
fn word_rows_under_a_band_stay_rows() {
    // Guard: lines of words only that each fill at least half of the columns are each a row,
    // as before the rule for sparse lines.
    let page_text = tax_band_page(&[
        &[(0, "Steuer A"), (1, "frei"), (3, "offen")],
        &[(0, "Steuer B"), (1, "frei"), (2, "keiner")],
        &[(0, "Steuer C"), (2, "keiner"), (3, "offen")],
    ]);
    let out = read(&pdf(&[Page::text(page_text)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(
        found[0].1,
        owned(&[
            &["Art", "Basis", "Satz", "Betrag"],
            &["Steuer A", "frei", "", "offen"],
            &["Steuer B", "frei", "keiner", ""],
            &["Steuer C", "", "keiner", "offen"],
        ])
    );
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

#[test]
fn senate_expenditures_table_is_rebuilt() {
    // A page turned for display whose table has column rules in its header only and no rules
    // between the records: one table with two header rows; each record is a row, the second
    // line of a record in its description.
    let out = read_corpus("senate-expenditures.pdf");
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    let (head, rows) = &found[0];
    assert_eq!(*head, 2);
    assert_eq!(rows.len(), 34);
    let md = markdown(&out);
    for expected in [
        "| **DOCUMENT NO.** | **DATE**<br>**POSTED** | **PAYEE NAME** | \
         **OBLIGATION/SERVICE**<br>**DATES** **START** | **OBLIGATION/SERVICE**<br>**DATES** **END** | \
         **DESCRIPTION** | **AMOUNT ($)** |\n| --- | --- | --- | --- | --- | --- | --- |\n\
         |  |  | BAIN, J MATTHEW |  |  | DISTRICT DIRECTOR | 37,499.96 |\n",
        "| DHAW20190001 | 05/03/2019 | CITIBANK - TRAVEL CBA CARD | 03/04/2019 | 03/06/2019 | \
         STAFF TRANSPORTATION<br>AIRFARE FOR E JOHNSON KANSAS CITY TO WASHINGTON DC AND RETURN | \
         920.68 |\n",
        "| DHAW20190070 | 09/09/2019 | CITIBANK - TRAVEL CBA CARD | 08/12/2019 | 08/14/2019 | \
         STAFF TRANSPORTATION<br>AIRFARE FOR C WEIHS WASHINGTON DC TO SPRINGFIELD AND RETURN | \
         411.49 |\n",
    ] {
        assert!(md.contains(expected), "{expected} in {md}");
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
fn nics_checks_grid_is_noted() {
    // A ruled grid of about 25 columns over most of a landscape page, full of numbers: no
    // table, its text stays, and one note says a table may stand there.
    let out = read_corpus("nics-background-checks-2015-11.pdf");
    let blocks = page(&out, 1);
    assert!(tables(blocks).is_empty(), "{:?}", tables(blocks));
    let note = "Possible table on page 1 not rebuilt (reason: it has more than 15 columns)";
    let paras = paragraphs(blocks);
    assert_eq!(paras.iter().filter(|p| *p == note).count(), 1, "{paras:?}");
    assert!(
        paras.iter().any(|p| p.contains("Alabama 18,870")),
        "{paras:?}"
    );
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
fn lists_that_overlap_in_part_are_no_table() {
    // A label and value list on the right whose last three lines share their baselines with a
    // short list on the left that starts lower: the lines read across, yet no table mixes the
    // two lists.
    let mut c = String::new();
    let right = [
        ("Room No.", "101"),
        ("Arrival", "2030-01-02"),
        ("Departure", "2030-01-05"),
        ("Cashier", "77"),
        ("Folio No.", "1000"),
    ];
    for (k, (l, v)) in right.iter().enumerate() {
        let y = 700.0 - 12.0 * k as f32;
        c += &show("F1", 8.0, 400.0, y, l);
        c += &show("F1", 8.0, 470.0, y, v);
    }
    let left = [
        ("Confirmation", "4711"),
        ("Group", "Sample"),
        ("Booking", "1234"),
    ];
    for (k, (l, v)) in left.iter().enumerate() {
        let y = 676.0 - 12.0 * k as f32;
        c += &show("F1", 8.0, 40.0, y, l);
        c += &show("F1", 8.0, 120.0, y, v);
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    for (_, rows) in tables(page(&out, 1)) {
        let cells: Vec<&String> = rows.iter().flatten().collect();
        let has = |words: &[(&str, &str)]| {
            cells
                .iter()
                .any(|c| words.iter().any(|(l, _)| c.as_str() == *l))
        };
        assert!(
            !(has(&left) && has(&right)),
            "a table mixes both lists: {rows:?}"
        );
    }
}

#[test]
fn a_note_row_across_a_ruled_table_stays_one_cell() {
    // A fully ruled table of three columns with a row whose one cell spans the table and
    // holds a note of two lines, each set in several runs: the note stays one cell.
    let mut c = String::from("0.5 w ");
    let ys = [700.0, 680.0, 660.0, 620.0, 600.0];
    for y in ys {
        c += &format!("50 {y} m 410 {y} l S ");
    }
    for x in [50.0, 410.0] {
        c += &format!("{x} 600 m {x} 700 l S ");
    }
    for x in [170.0, 290.0] {
        c += &format!("{x} 660 m {x} 700 l S {x} 600 m {x} 620 l S ");
    }
    for (r, row) in [["Fruit", "Count", "Price"], ["Apples", "3", "1.20"]]
        .iter()
        .enumerate()
    {
        for (k, t) in row.iter().enumerate() {
            c += &show(
                "F1",
                9.0,
                56.0 + 120.0 * k as f32,
                686.0 - 20.0 * r as f32,
                t,
            );
        }
    }
    for (k, y) in [646.0, 634.0].iter().enumerate() {
        c += &format!(
            "BT /F1 9 Tf 56 {y} Td [(Note {k}: the prices) -1500 (of the season) -1500 \
             (change from week to week)] TJ ET\n"
        );
    }
    for (k, t) in ["Pears", "12", "0.80"].iter().enumerate() {
        c += &show("F1", 9.0, 56.0 + 120.0 * k as f32, 606.0, t);
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    let note: Vec<&Vec<String>> = found[0].1.iter().filter(|r| r.len() == 1).collect();
    assert_eq!(note.len(), 1, "{found:?}");
    assert!(
        note[0][0].starts_with("Note 0:") && note[0][0].contains("Note 1:"),
        "{found:?}"
    );
}

#[test]
fn invoice_table_turned_down_or_upside_down() {
    // The invoice table drawn downward on a page turned the other way, and upside down on a
    // landscape page: read in the turned frame, the same table.
    let down = format!("q 0 -1 1 0 0 842 cm\n{}Q\n", invoice_table());
    assert_invoice_table(&read(&sized_page(&down, 595.0, 842.0, "/Rotate 270")));
    let half = format!("q -1 0 0 -1 842 595 cm\n{}Q\n", invoice_table());
    assert_invoice_table(&read(&sized_page(&half, 842.0, 595.0, "/Rotate 180")));
}

#[test]
fn a_cell_over_three_rows_makes_no_header() {
    // A ruled week plan without a header row whose first cell spans three rows: the records
    // stay records, the cells under the span say "same as above".
    let mut c = String::from("0.5 w ");
    let ys = [700.0, 680.0, 660.0, 640.0, 620.0, 600.0];
    for (k, y) in ys.iter().enumerate() {
        let x0 = if [1, 2, 4].contains(&k) { 150.0 } else { 50.0 };
        c += &format!("{x0} {y} m 400 {y} l S ");
    }
    for x in [50.0, 150.0, 250.0, 400.0] {
        c += &format!("{x} 600 m {x} 700 l S ");
    }
    let cells = [
        (Some("Monday"), "08:00", "Math"),
        (None, "09:00", "Art"),
        (None, "10:00", "Music"),
        (Some("Tuesday"), "08:00", "Sport"),
        (None, "09:00", "Latin"),
    ];
    for (r, (a, b, d)) in cells.iter().enumerate() {
        let y = ys[r] - 14.0;
        if let Some(a) = a {
            c += &show("F1", 9.0, 54.0, y, a);
        }
        c += &show("F1", 9.0, 154.0, y, b);
        c += &show("F1", 9.0, 254.0, y, d);
    }
    let out = read(&pdf(&[Page::text(c)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].0 <= 1, "{found:?}");
    let md = markdown(&out);
    assert!(
        md.contains("| \u{3003} | 09:00 | Art |\n| \u{3003} | 10:00 | Music |\n"),
        "{md}"
    );
    assert!(md.contains("| Monday | 08:00 | Math |"), "{md}");
}

/// Statement lines whose date is printed only on the first entry of a day, under a header of
/// three filled cells (`ruled` false) or in a table of column rules without row rules.
fn blank_dates(ruled: bool) -> String {
    let xs = [50.0, 150.0, 400.0, 500.0];
    let mut c = String::new();
    if ruled {
        c += "0.5 w 50 700 m 500 700 l S 50 686 m 500 686 l S 50 590 m 500 590 l S ";
        for x in xs {
            c += &format!("{x} 590 m {x} 700 l S ");
        }
    } else {
        c += "0.9 g ";
        for w in xs.windows(2) {
            c += &format!("{} 686 {} 14 re f ", w[0], w[1] - w[0]);
        }
        c += "0 g 0.5 w 50 700 m 500 700 l S 50 686 m 500 686 l S 50 600 m 500 600 l S ";
    }
    for (x, t) in xs.iter().zip(["Date", "Text", "Amount"]) {
        c += &show("F1", 8.0, x + 4.0, 690.0, t);
    }
    let rows = [
        ("2030-01-02", "Coffee", "3.00"),
        ("", "Bread", "2.00"),
        ("", "Cheese", "7.50"),
        ("2030-01-03", "Milk", "1.00"),
        ("", "Apples", "4.20"),
        ("2030-01-04", "Tea", "2.80"),
    ];
    for (k, (d, t, a)) in rows.iter().enumerate() {
        let y = 672.0 - 12.0 * k as f32;
        if !d.is_empty() {
            c += &show("F1", 8.0, 54.0, y, d);
        }
        c += &show("F1", 8.0, 154.0, y, t);
        c += &show("F1", 8.0, 470.0, y, a);
    }
    c
}

#[test]
fn lines_with_a_blank_date_are_rows_of_their_own() {
    for ruled in [false, true] {
        let out = read(&pdf(&[Page::text(blank_dates(ruled))], ""));
        let found = tables(page(&out, 1));
        assert_eq!(found.len(), 1, "ruled {ruled}: {found:?}");
        let rows = &found[0].1;
        assert_eq!(rows.len(), 7, "ruled {ruled}: {rows:?}");
        assert_eq!(rows[1], ["2030-01-02", "Coffee", "3.00"], "ruled {ruled}");
        assert_eq!(rows[2], ["", "Bread", "2.00"], "ruled {ruled}");
        assert_eq!(rows[3], ["", "Cheese", "7.50"], "ruled {ruled}");
        assert_eq!(rows[5], ["", "Apples", "4.20"], "ruled {ruled}");
    }
}

#[test]
fn a_ruled_row_with_wrapped_cells_stays_one_row() {
    // A fully ruled table of three columns whose second record wraps in two of its three
    // cells: still one row (the rules part the records).
    let mut c = String::from("0.5 w ");
    for y in [700.0, 680.0, 650.0, 630.0] {
        c += &format!("50 {y} m 410 {y} l S ");
    }
    for x in [50.0, 170.0, 290.0, 410.0] {
        c += &format!("{x} 630 m {x} 700 l S ");
    }
    c += &show("F1", 9.0, 56.0, 686.0, "Apples");
    c += &show("F1", 9.0, 176.0, 686.0, "red and sweet");
    c += &show("F1", 9.0, 296.0, 686.0, "from the north");
    c += &show("F1", 9.0, 56.0, 668.0, "Pears");
    c += &show("F1", 9.0, 176.0, 668.0, "green and");
    c += &show("F1", 9.0, 296.0, 668.0, "from the");
    c += &show("F1", 9.0, 176.0, 656.0, "juicy");
    c += &show("F1", 9.0, 296.0, 656.0, "south");
    c += &show("F1", 9.0, 56.0, 636.0, "Plums");
    c += &show("F1", 9.0, 176.0, 636.0, "blue");
    c += &show("F1", 9.0, 296.0, 636.0, "from the east");
    let out = read(&pdf(&[Page::text(c)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    let rows = &found[0].1;
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(
        rows[1],
        ["Pears", "green and juicy", "from the south"],
        "{rows:?}"
    );
}

/// One section of a ruled grid: rows `h` points high from `top` down between the column
/// rules `xs`, every rule drawn, and in each cell the text `cell(row, column)` in `font` at
/// 8 points (none when it is empty).
fn grid_section(
    xs: &[f32],
    top: f32,
    h: f32,
    rows: usize,
    font: &dyn Fn(usize) -> &'static str,
    cell: &dyn Fn(usize, usize) -> String,
) -> String {
    let (x0, x1) = (xs[0], xs[xs.len() - 1]);
    let bottom = top - h * rows as f32;
    let mut c = String::from("0.5 w ");
    for k in 0..=rows {
        let y = top - h * k as f32;
        c += &format!("{x0} {y} m {x1} {y} l S ");
    }
    for x in xs {
        c += &format!("{x} {bottom} m {x} {top} l S ");
    }
    for r in 0..rows {
        for (k, x) in xs[..xs.len() - 1].iter().enumerate() {
            let text = cell(r, k);
            if !text.is_empty() {
                c += &show(font(r), 8.0, x + 3.0, top - h * r as f32 - h + 7.0, &text);
            }
        }
    }
    c
}

/// `n` columns of equal width from 72 to 552.
fn columns(n: usize) -> Vec<f32> {
    (0..=n)
        .map(|k| 72.0 + 480.0 * k as f32 / n as f32)
        .collect()
}

/// The one table of page 1 with the given number of columns in every row.
fn table_of_width(out: &ReadOutcome, cols: usize) -> Vec<Vec<String>> {
    let found = tables(page(out, 1));
    let hits: Vec<_> = found
        .iter()
        .filter(|(_, rows)| rows.iter().all(|r| r.len() == cols))
        .collect();
    assert_eq!(hits.len(), 1, "{found:?}");
    hits[0].1.clone()
}

#[test]
fn two_sections_two_tables() {
    // One connected grid: a section of 4 columns and 3 rows over one of 9 columns and 4
    // rows, the rule between them shared: 13 column edges in all. Each section is a table
    // of its own.
    let bold = |r: usize| if r == 0 { "F2" } else { "F1" };
    let mut c = grid_section(&columns(4), 700.0, 20.0, 3, &bold, &|r, k| {
        format!("{}{r}{k}", ['K', 'L', 'M'][r])
    });
    c += &grid_section(&columns(9), 640.0, 20.0, 4, &bold, &|r, k| {
        format!("w{r}{k}")
    });
    let out = read(&pdf(&[Page::text(c)], ""));
    assert_eq!(
        tables(page(&out, 1)).len(),
        2,
        "{:?}",
        tables(page(&out, 1))
    );
    let upper = table_of_width(&out, 4);
    assert_eq!(upper[0], ["K00", "K01", "K02", "K03"]);
    assert_eq!(upper.len(), 3, "{upper:?}");
    let lower = table_of_width(&out, 9);
    assert_eq!(lower.len(), 4, "{lower:?}");
    assert_eq!(lower[3][8], "w38");
    assert!(out.document.losses.is_empty(), "{:?}", out.document.losses);
}

/// Guard: a header row of two cells, each over two columns of the body, stays with its
/// body: one table, its first row the header.
#[test]
fn spanned_header_is_not_a_section() {
    let xs = columns(4);
    let bold = |r: usize| if r == 0 { "F2" } else { "F1" };
    let mut c = grid_section(&[xs[0], xs[2], xs[4]], 700.0, 20.0, 1, &bold, &|_, k| {
        ["Erika Mustermann", "Muster GmbH"][k].to_string()
    });
    c += &grid_section(&xs, 680.0, 20.0, 3, &|_| "F1", &|r, k| format!("v{r}{k}"));
    let out = read(&pdf(&[Page::text(c)], ""));
    let found = tables(page(&out, 1));
    assert_eq!(found.len(), 1, "{found:?}");
    let (header, rows) = &found[0];
    assert_eq!(*header, 1, "{found:?}");
    assert_eq!(rows[0], ["Erika Mustermann", "Muster GmbH"], "{found:?}");
    assert_eq!(rows.len(), 4, "{found:?}");
    assert_eq!(rows[3], ["v20", "v21", "v22", "v23"], "{found:?}");
    assert!(out.document.losses.is_empty(), "{:?}", out.document.losses);
}

#[test]
fn calendar_grid_is_refused_with_a_loss() {
    // A month of 31 day columns: the days in the first row, two rows of names with a mark
    // on some days. One section of 31 columns is no table, and its refusal is a loss.
    let xs: Vec<f32> = (0..=31).map(|k| 40.0 + 16.0 * k as f32).collect();
    let c = grid_section(&xs, 700.0, 16.0, 3, &|_| "F1", &|r, k| match r {
        0 => format!("{}", k + 1),
        _ if k % 7 == r => "x".into(),
        _ => String::new(),
    });
    let out = read(&pdf(&[Page::text(c)], ""));
    assert!(tables(page(&out, 1)).is_empty());
    let losses = &out.document.losses;
    assert_eq!(losses.len(), 1, "{losses:?}");
    assert_eq!(losses[0].why, LossReason::Uncertain, "{losses:?}");
    assert_eq!(losses[0].severity, Severity::Info, "{losses:?}");
    assert!(
        losses[0].what.contains("more than 15 columns"),
        "{losses:?}"
    );
}

/// A ruled table of a header row and three rows whose column rules run on past its first or
/// its last horizontal rule, with no rule there (an open top or bottom), and a free rule over
/// it (a title's underline) or under it: the open row is the table's.
fn open_ruled_page(open_top: bool) -> Vec<u8> {
    let rows = [
        ["Monat", "Umsatz", "Kosten"],
        ["Januar", "1.234,56", "234,57"],
        ["Februar", "2.345,67", "345,68"],
        ["Maerz", "3.456,78", "456,79"],
    ];
    let xs = [40.0, 200.0, 380.0, 540.0];
    let mut c = show("F1", 10.0, 40.0, 760.0, "Muster GmbH Bericht");
    c += "0.5 w 40 754 m 540 754 l S ";
    // Row edges from the top; the open one has no rule.
    let ys = [740.0, 720.0, 700.0, 680.0, 660.0];
    for (k, y) in ys.iter().enumerate() {
        let open = if open_top { k == 0 } else { k == ys.len() - 1 };
        if !open {
            c += &format!("40 {y} m 540 {y} l S ");
        }
    }
    for x in xs {
        c += &format!("{x} 660 m {x} 740 l S ");
    }
    if !open_top {
        c += "40 640 m 540 640 l S ";
    }
    for (r, row) in rows.iter().enumerate() {
        for (k, text) in row.iter().enumerate() {
            c += &show("F1", 10.0, xs[k] + 5.0, ys[r] - 15.0, text);
        }
    }
    c += &show("F1", 10.0, 40.0, 600.0, "A paragraph after the table.");
    pdf(&[Page::text(c)], "")
}

/// Guard: a free rule over an open top or under an open bottom of a ruled table does not cut
/// the open row off: it holds column titles or values, no row of fields.
#[test]
fn an_open_row_of_a_ruled_table_stays_its_own() {
    for open_top in [true, false] {
        let out = read(&open_ruled_page(open_top));
        let found = tables(page(&out, 1));
        assert_eq!(found.len(), 1, "{open_top}: {found:?}");
        assert_eq!(found[0].1.len(), 4, "{open_top}: {found:?}");
        assert_eq!(found[0].1[0], ["Monat", "Umsatz", "Kosten"], "{open_top}");
        assert_eq!(found[0].1[3], ["Maerz", "3.456,78", "456,79"], "{open_top}");
    }
}

/// A grid, a header band and a region without rules refused with a note keep their text as
/// paragraphs, and the regions of those paragraphs say why the text is no table.
#[test]
fn noted_refusals_keep_their_reason_in_their_paragraphs() {
    let grid = wide_ruled_grid();
    common::assert_every_run_in_one_region(&grid);
    common::assert_kept_with_reason(&grid, "b7", "it has more than 15 columns");
    let band = pdf(&[Page::text(header_band_page(5, true))], "");
    common::assert_every_run_in_one_region(&band);
    // The band's titles: the prose under it is no text of the band, which found no body.
    for text in ["Date", "Description", "Credits"] {
        common::assert_kept_with_reason(
            &band,
            text,
            "the text below its header does not stand in its columns",
        );
    }
    let region = gutter_region_page();
    common::assert_every_run_in_one_region(&region);
    for text in ["Site B", "late"] {
        common::assert_kept_with_reason(
            &region,
            text,
            "some of its text stands between its columns",
        );
    }
}

/// One cell of [`tagged_timetable`]: its lines, each its text (empty: no text on that line).
type TaggedCell<'a> = &'a [&'a str];

/// Private shape S1 (Task 29 fix round 1, vision class A on the given path): a tagged
/// timetable, its table stated by the tag tree, each row a departure over an arrival in cells
/// of two lines: the stops, a date and a time on each line, a platform on one line, a product
/// on the first line, and a note that wraps to the table's right end. `size` is the text size
/// (the columns scale with it), `rows` the rows of the body.
fn tagged_timetable(size: f32, rows: &[[TaggedCell<'_>; 6]]) -> Vec<u8> {
    let f = size / 8.33;
    let xs = [42.52, 170.08, 209.76, 249.45, 283.46, 340.16].map(|x| x * f);
    let titles = ["Halt", "Datum", "Zeit", "Gleis", "Produkt", "Hinweis"];
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let kid = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let bold = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>");
    let root = b.reserve();
    let table = b.reserve();
    let mut content = String::new();
    let mut mcid = 0u32;
    // Each element: its role, its row and its marked content ids.
    let mut cells: Vec<(&str, usize, Vec<u32>)> = Vec::new();
    let top = 700.0;
    let pitch = 1.2 * size;
    let mut show = |font: &str, x: f32, y: f32, text: &str, role: &str, mcid: &mut u32| {
        let t = text.replace('(', "\\(").replace(')', "\\)");
        content.push_str(&format!(
            "/{role} <</MCID {m}>> BDC BT /{font} {size} Tf 1 0 0 1 {x} {y} Tm ({t}) Tj ET EMC\n",
            m = *mcid
        ));
        *mcid += 1;
        *mcid - 1
    };
    let mut header = Vec::new();
    for (x, t) in xs.iter().zip(titles) {
        header.push(("TH", 0, vec![show("F2", *x, top, t, "TH", &mut mcid)]));
    }
    cells.extend(header);
    for (r, row) in rows.iter().enumerate() {
        let first = top - 1.6 * size - 2.0 * pitch * r as f32 - 0.5 * size * r as f32;
        for (x, lines) in xs.iter().zip(row.iter()) {
            let ids: Vec<u32> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| !l.is_empty())
                .map(|(n, l)| show("F1", *x, first - pitch * n as f32, l, "TD", &mut mcid))
                .collect();
            cells.push(("TD", r + 1, ids));
        }
    }
    let cs = b.stream("", content.as_bytes());
    let rows_n = rows.len() + 1;
    let trs: Vec<u32> = (0..rows_n).map(|_| b.reserve()).collect();
    let mut by_mcid: Vec<(u32, u32)> = Vec::new();
    let mut tr_kids: Vec<Vec<u32>> = vec![Vec::new(); rows_n];
    for (role, r, ids) in &cells {
        let k = match ids.as_slice() {
            [one] => format!("{one}"),
            many => format!(
                "[{}]",
                many.iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        };
        let el = b.add(format!(
            "<< /Type /StructElem /S /{role} /P {} 0 R /Pg {kid} 0 R /K {k} >>",
            trs[*r]
        ));
        tr_kids[*r].push(el);
        by_mcid.extend(ids.iter().map(|&m| (m, el)));
    }
    for (r, kids) in tr_kids.iter().enumerate() {
        b.set(
            trs[r],
            format!(
                "<< /Type /StructElem /S /TR /P {table} 0 R /K [{}] >>",
                kids.iter()
                    .map(|k| format!("{k} 0 R"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
    }
    b.set(
        table,
        format!(
            "<< /Type /StructElem /S /Table /P {root} 0 R /K [{}] >>",
            trs.iter()
                .map(|k| format!("{k} 0 R"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );
    by_mcid.sort_unstable();
    b.set(
        root,
        format!(
            "<< /Type /StructTreeRoot /K {table} 0 R /ParentTree << /Nums [0 [{}]] >> >>",
            by_mcid
                .iter()
                .map(|(_, e)| format!("{e} 0 R"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );
    b.set(
        kid,
        format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /StructParents 0 \
             /Resources << /Font << /F1 {font} 0 R /F2 {bold} 0 R >> >> /Contents {cs} 0 R >>"
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

/// Private shape S1, across sizes and row counts: each cell of the tagged table keeps its lines,
/// as in a table read from rules or from lines (one shared rule): each stop on the line of its
/// date and time, the platform after an empty first line where it stands on the arrival's line.
/// The times stand in a column too narrow for the next word (`an` after `ab 08:15`) and open
/// with a small letter: they stay lines because the stops beside them do. So does the note that
/// wraps on those lines (a line break in its sentence, every word in its cell); the note whose
/// second line is a long number keeps it on a line of its own.
#[test]
fn a_tagged_row_keeps_its_cell_lines() {
    let note = [
        "Bitte beachten Sie den Hinweis zur Fahrradmitnahme im Zug",
        "und am Bahnsteig",
    ];
    // Fix round 2: a note whose first line ends in a label and whose second line is its value,
    // a long number that would not have fit after the label: one note, never a pair with the
    // arrival's line.
    let numbered = [
        "Bitte die Nummer fuer Rueckfragen bereithalten, Ref.-Nr.",
        "123456789012",
    ];
    let all: [[TaggedCell<'_>; 6]; 3] = [
        [
            &["Musterstadt Hbf, Nordseite", "Beispielheim Bahnhof, Ost"],
            &["01.10.", "01.10."],
            &["ab 08:15", "an 09:40"],
            &["", "4"],
            &["ICE 571"],
            &note,
        ],
        [
            &["Beispielheim Bahnhof, Ost", "Probedorf Mitte"],
            &["01.10.", "01.10."],
            &["ab 09:52", "an 10:30"],
            &["2", "5"],
            &["RE 4711"],
            &note,
        ],
        [
            &["Probedorf Mitte", "Musterstadt Hbf, Nordseite"],
            &["02.10.", "02.10."],
            &["ab 17:05", "an 18:20"],
            &["1", "3"],
            &["RB 4712"],
            &numbered,
        ],
    ];
    for size in [7.5f32, 8.33] {
        for n in [2usize, 3] {
            let out = read(&tagged_timetable(size, &all[..n]));
            let blocks = page(&out, 1);
            let rows: Vec<&Vec<Cell>> = blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Table { rows, .. } => Some(rows),
                    _ => None,
                })
                .flatten()
                .collect();
            assert_eq!(rows.len(), n + 1, "{size} {n}");
            // Each cell's lines, parted at its line breaks.
            let lines = |c: &Cell| -> Vec<String> {
                c.inlines
                    .split(|i| matches!(i, Inline::LineBreak))
                    .map(|p| {
                        plain_text(p)
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect()
            };
            for (r, row) in all[..n].iter().enumerate() {
                let have: Vec<Vec<String>> = rows[r + 1].iter().map(lines).collect();
                let want: Vec<Vec<String>> = row
                    .iter()
                    .map(|cell| {
                        let mut l: Vec<String> = cell.iter().map(|s| s.to_string()).collect();
                        while l.last().is_some_and(String::is_empty) {
                            l.pop();
                        }
                        l
                    })
                    .collect();
                assert_eq!(have, want, "{size} {n} row {r}");
            }
        }
    }
}

/// Task 29 fix round 4, the other direction of [`a_tagged_row_keeps_its_cell_lines`]: in a row
/// whose other cells hold one line each, a note whose first line is full joins its second line
/// when the text goes on, in a small letter (with a space) or after a hyphen in a word (with
/// nothing between); a full line before a capital stays a line.
#[test]
fn a_tagged_cell_joins_a_full_line_that_goes_on() {
    let notes: [([&str; 2], &[&str]); 3] = [
        (
            [
                "Bitte beachten Sie den Hinweis zur Fahrradmitnahme im Zug",
                "und am Bahnsteig",
            ],
            &["Bitte beachten Sie den Hinweis zur Fahrradmitnahme im Zug und am Bahnsteig"],
        ),
        (
            [
                "Bitte beachten Sie den Hinweis zur Mitnahme der Fahr-",
                "raeder im Zug",
            ],
            &["Bitte beachten Sie den Hinweis zur Mitnahme der Fahr-raeder im Zug"],
        ),
        (
            [
                "Bitte beachten Sie den Hinweis zur Fahrradmitnahme im",
                "Zug nach Beispielheim",
            ],
            &[
                "Bitte beachten Sie den Hinweis zur Fahrradmitnahme im",
                "Zug nach Beispielheim",
            ],
        ),
    ];
    for (note, want) in notes {
        let row: [TaggedCell<'_>; 6] = [
            &["Musterstadt Hbf"],
            &["01.10."],
            &["ab 08:15"],
            &["4"],
            &["ICE 571"],
            &note,
        ];
        let out = read(&tagged_timetable(8.33, &[row]));
        let blocks = page(&out, 1);
        let rows: Vec<&Vec<Cell>> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Table { rows, .. } => Some(rows),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(rows.len(), 2, "{note:?}");
        let have: Vec<String> = rows[1][5]
            .inlines
            .split(|i| matches!(i, Inline::LineBreak))
            .map(|p| plain_text(p).trim().to_string())
            .collect();
        assert_eq!(have, want, "{note:?}");
        // The cells of one line take no empty line before them.
        assert_eq!(plain_text(&rows[1][3].inlines), "4");
        assert!(!matches!(rows[1][3].inlines[0], Inline::LineBreak));
    }
}
