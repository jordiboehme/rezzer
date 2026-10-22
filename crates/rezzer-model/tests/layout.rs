// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Table keys and the row-range split (spec 9.3, 9.4).

use rezzer_model::{
    Anchor, Block, Cell, Certainty, Evidence, Inline, LayoutKind, RowKind, RowNote, RowRange,
    TableLayout, split_rows, table_key,
};

fn name(n: &str) -> String {
    n.to_string()
}

#[test]
fn table_key_from_list_object_and_top_left() {
    let list = Evidence::ListObject {
        name: name("Sales_Alice"),
    };
    let pivot = Evidence::Pivot {
        name: name("PivotTable1"),
    };
    assert_eq!(
        table_key("Sheet3", &[Evidence::Autofilter, list.clone()], "B2"),
        "table:Sales_Alice"
    );
    // The fact kind decides, not the place in the list.
    assert_eq!(
        table_key("Sheet3", &[pivot.clone(), list], "B2"),
        "table:Sales_Alice"
    );
    assert_eq!(
        table_key(
            "Data",
            &[Evidence::DatabaseRange {
                name: name("Stock")
            }],
            "A1"
        ),
        "dbrange:Stock"
    );
    // An anonymous database range has no name of its own: the range key.
    assert_eq!(
        table_key(
            "'NST-EST2023-POP'",
            &[Evidence::DatabaseRange {
                name: name("__Anonymous_Sheet_DB__0")
            }],
            "A1"
        ),
        "'NST-EST2023-POP'!A1"
    );
    assert_eq!(
        table_key("Data", std::slice::from_ref(&pivot), "C4"),
        "pivot:Data!PivotTable1"
    );
    // The pivot's sheet is quoted like every other key (P4-H).
    assert_eq!(
        table_key("'My Sheet'", &[Evidence::Pivot { name: name("PT1") }], "A1"),
        "pivot:'My Sheet'!PT1"
    );
    assert_eq!(
        table_key(
            "'NST-EST2023-POP'",
            &[
                Evidence::Rule {
                    id: name("H1"),
                    detail: name("header by type change"),
                },
                Evidence::DefinedName {
                    name: name("_NST01"),
                },
            ],
            "A1"
        ),
        "'NST-EST2023-POP'!A1"
    );
}

// ---------------------------------------------------------------------------------------
// The row-range split. Sheet `S`, columns A and B, header paths `Name` and `Value`.
//
// Tokens by the row formula: a header row `Name | Value` is ceil((10 + 14 + 16) / 4) = 10,
// a data row `a | 1` is ceil((10 + 11 + 12) / 4) = 9, a group row `G` (column A only) is
// ceil((10 + 11) / 4) = 6, a lone `1` in column B is ceil((10 + 12) / 4) = 6. The lead
// heading `Title` is ceil(5 / 4) = 2.

fn txt(s: &str) -> Inline {
    Inline::plain(s)
}

fn cell(text: &str, at: &str) -> Cell {
    let mut c = Cell::new(vec![txt(text)]);
    c.anchor = Some(Anchor(format!("S!{at}")));
    c
}

/// A data row `a | 1` at sheet row `row`.
fn data_row(row: u32) -> Vec<Cell> {
    vec![cell("a", &format!("A{row}")), cell("1", &format!("B{row}"))]
}

fn layout(header_rows: Vec<u32>, first: u32, last: u32, rows: Vec<RowNote>) -> TableLayout {
    TableLayout {
        kind: LayoutKind::Table,
        key: "S!A1".into(),
        range: format!("A1:B{last}"),
        data: Some(format!("A{first}:B{last}")),
        header_rows,
        header_paths: vec!["Name".into(), "Value".into()],
        label_cols: 1,
        certainty: Certainty::Rule,
        found_by: vec![Evidence::Rule {
            id: "H1".into(),
            detail: "header by type change".into(),
        }],
        unsure: Vec::new(),
        rows,
        hidden_header: false,
    }
}

/// Header row 1, data rows 2 to `last` as `a | 1`.
fn table(last: u32) -> Block {
    let mut rows = vec![vec![cell("Name", "A1"), cell("Value", "B1")]];
    rows.extend((2..=last).map(data_row));
    Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: Some(Anchor(format!("S!A1:B{last}"))),
        layout: Some(Box::new(layout(vec![1], 2, last, Vec::new()))),
    }
}

fn heading() -> Block {
    Block::Heading {
        level: 3,
        inlines: vec![txt("Title")],
        anchor: Some(Anchor("S!A1".into())),
    }
}

fn note() -> Block {
    Block::Paragraph {
        inlines: vec![txt("Source: none")],
        role: Some("note".into()),
        anchor: Some(Anchor("S!A99".into())),
    }
}

fn borders(ranges: &[RowRange]) -> Vec<(u32, u32)> {
    ranges.iter().map(|r| (r.first, r.last)).collect()
}

fn table_of(range: &RowRange) -> (&Vec<Vec<Cell>>, usize, &TableLayout) {
    let t = range
        .blocks
        .iter()
        .find(|b| matches!(b, Block::Table { .. }))
        .expect("a table in every range");
    let Block::Table {
        rows,
        header_rows,
        layout: Some(layout),
        ..
    } = t
    else {
        panic!("the table keeps its layout");
    };
    (rows, *header_rows, layout)
}

#[test]
fn row_ranges_cut_at_row_borders() {
    // Fixed part 2 + 10 = 12, three data rows 27: 39 holds three rows, 38 two.
    let t = table(11);
    let ranges = split_rows(&[heading()], &t, &[], 39);
    assert_eq!(borders(&ranges), vec![(2, 4), (5, 7), (8, 10), (11, 11)]);
    let ranges = split_rows(&[heading()], &t, &[], 38);
    assert_eq!(
        borders(&ranges),
        vec![(2, 3), (4, 5), (6, 7), (8, 9), (10, 11)]
    );
    // Each table holds exactly its own rows, whole, and names its own data range.
    for range in &ranges {
        let (rows, _, layout) = table_of(range);
        let shown: Vec<u32> = (range.first..=range.last).collect();
        assert_eq!(rows.len(), 1 + shown.len());
        for (row, n) in rows[1..].iter().zip(&shown) {
            assert_eq!(row, &data_row(*n));
        }
        assert_eq!(
            layout.data.as_deref(),
            Some(format!("A{}:B{}", range.first, range.last).as_str())
        );
    }
}

#[test]
fn row_ranges_repeat_header_band() {
    // Two header rows (sheet rows 1 and 2), data rows 3 to 12.
    let mut rows = vec![
        vec![cell("Name", "A1"), cell("Value", "B1")],
        vec![cell("Name", "A2"), cell("Value", "B2")],
    ];
    rows.extend((3..=12).map(data_row));
    let band: Vec<Vec<Cell>> = rows[..2].to_vec();
    let t = Block::Table {
        caption: Some("Sales".into()),
        header_rows: 2,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: Some(Anchor("S!A1:B12".into())),
        layout: Some(Box::new(layout(vec![1, 2], 3, 12, Vec::new()))),
    };
    let caption = Block::Paragraph {
        inlines: vec![txt("Sales")],
        role: Some("caption".into()),
        anchor: Some(Anchor("S!A1".into())),
    };
    let lead = vec![heading(), caption];
    let tail = vec![note()];
    // Fixed part: lead 2 + 2, band 10 + 10; 24 + 27 = 51 holds three data rows.
    let ranges = split_rows(&lead, &t, &tail, 51);
    assert_eq!(borders(&ranges), vec![(3, 5), (6, 8), (9, 11), (12, 12)]);
    let n = ranges.len();
    for (i, range) in ranges.iter().enumerate() {
        assert_eq!(&range.blocks[..2], &lead[..], "lead in range {i}");
        let (rows, header_rows, layout) = table_of(range);
        assert_eq!(header_rows, 2);
        assert_eq!(&rows[..2], &band[..], "band in range {i}");
        assert_eq!(layout.header_rows, vec![1, 2]);
        let last = i + 1 == n;
        assert_eq!(range.blocks.len(), if last { 4 } else { 3 }, "range {i}");
        assert_eq!(range.blocks.last() == Some(&tail[0]), last, "range {i}");
    }
}

/// Header row 1, data rows 2 to 11, with sheet row 4 a group row `G`.
fn table_with_group() -> Block {
    let mut rows = vec![vec![cell("Name", "A1"), cell("Value", "B1")]];
    for r in 2..=11 {
        if r == 4 {
            rows.push(vec![cell("G", "A4")]);
        } else {
            rows.push(data_row(r));
        }
    }
    let notes = vec![RowNote {
        row: 4,
        kind: RowKind::Group,
        level: None,
        parent: None,
        why: "label before data".into(),
    }];
    Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: Some(Anchor("S!A1:B11".into())),
        layout: Some(Box::new(layout(vec![1], 2, 11, notes))),
    }
}

#[test]
fn row_ranges_never_split_group_from_first_row() {
    // Rows 2, 3 and the group row 4 fit 39 (12 + 9 + 9 + 6 = 36), but the group row goes
    // with row 5 into the next range.
    let ranges = split_rows(&[heading()], &table_with_group(), &[], 39);
    assert_eq!(borders(&ranges)[..2], [(2, 3), (4, 6)]);
    assert!(ranges.iter().all(|r| r.last != 4), "{:?}", borders(&ranges));
    // The group's note goes with its range.
    let (_, _, layout) = table_of(&ranges[1]);
    assert_eq!(layout.rows.len(), 1);
    assert_eq!(layout.rows[0].row, 4);
    assert!(table_of(&ranges[0]).2.rows.is_empty());
}

#[test]
fn row_ranges_keep_vertical_merges_whole() {
    // Column A of sheet rows 3 to 5 is one merged cell.
    let mut rows = vec![vec![cell("Name", "A1"), cell("Value", "B1")]];
    for r in 2..=11 {
        match r {
            3 => {
                let mut merged = cell("a", "A3");
                merged.row_span = 3;
                rows.push(vec![merged, cell("1", "B3")]);
            }
            4 | 5 => rows.push(vec![cell("1", &format!("B{r}"))]),
            _ => rows.push(data_row(r)),
        }
    }
    let t = Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: Some(Anchor("S!A1:B11".into())),
        layout: Some(Box::new(layout(vec![1], 2, 11, Vec::new()))),
    };
    // Rows 3 to 5 are 9 + 6 + 6 = 21: with row 2 (9) and the fixed 12 that is 42 > 39.
    let ranges = split_rows(&[heading()], &t, &[], 39);
    let b = borders(&ranges);
    assert_eq!(b[..2], [(2, 2), (3, 5)], "{b:?}");
    for (first, last) in &b {
        assert!(!(4..=5).contains(first), "{b:?}");
        assert!(!(3..=4).contains(last), "{b:?}");
    }
    // A span past the end of the table is clipped, never a panic.
    let Block::Table { mut rows, .. } = t.clone() else {
        unreachable!()
    };
    rows[10][0].row_span = 1_000_000;
    let hostile = Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: Some(Box::new(layout(vec![1], 2, 11, Vec::new()))),
    };
    assert_eq!(split_rows(&[], &hostile, &[], 39).last().unwrap().last, 11);
}

#[test]
fn row_ranges_same_in_both_styles() {
    // The borders depend only on the model: the function takes no style, and the same call
    // gives the same ranges. Task 10 extends this to two real renderings (P4-I).
    let _no_style: fn(&[Block], &Block, &[Block], usize) -> Vec<RowRange> = split_rows;
    let t = table_with_group();
    let a = split_rows(&[heading()], &t, &[note()], 39);
    let b = split_rows(&[heading()], &t, &[note()], 39);
    assert_eq!(a, b);
    assert!(a.len() > 1);
}

#[test]
fn small_table_is_one_range() {
    let t = table(11);
    let ranges = split_rows(&[heading()], &t, &[note()], 20_000);
    assert_eq!(ranges.len(), 1);
    assert_eq!((ranges[0].first, ranges[0].last), (2, 11));
    assert_eq!(ranges[0].blocks, vec![heading(), t.clone(), note()]);
    // A table without layout is one range too, whatever the size.
    let Block::Table { rows, .. } = t else {
        unreachable!()
    };
    let plain = Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: None,
    };
    let ranges = split_rows(&[heading()], &plain, &[note()], 1);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].blocks, vec![heading(), plain, note()]);
    // A part size of 0 means no limit.
    assert_eq!(split_rows(&[], &table(11), &[], 0).len(), 1);
}

/// Tokens of a row by the formula of spec 9.3 (R13), for cells in column order.
fn row_tokens(cells: &[(&str, &str)]) -> usize {
    let chars: usize = cells
        .iter()
        .filter(|(_, text)| !text.is_empty())
        .map(|(path, text)| path.chars().count() + text.chars().count() + 6)
        .sum();
    (10 + chars).div_ceil(4)
}

#[test]
fn row_ranges_follow_part_size() {
    // 2,000 data rows of `äääääääääá | 1234567890` (characters, not bytes, are counted)
    // and one row whose text alone is larger than 2,500 tokens.
    let long = "ä".repeat(9) + "á";
    let huge = "x".repeat(12_000);
    let big_row = 1_000;
    let mut rows = vec![vec![cell("Name", "A1"), cell("Value", "B1")]];
    for r in 2..=2001 {
        let text = if r == big_row {
            huge.as_str()
        } else {
            long.as_str()
        };
        rows.push(vec![
            cell(text, &format!("A{r}")),
            cell("1234567890", &format!("B{r}")),
        ]);
    }
    let t = Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: Some(Box::new(layout(vec![1], 2, 2001, Vec::new()))),
    };
    let fixed = 2 + row_tokens(&[("Name", "Name"), ("Value", "Value")]);
    let small = row_tokens(&[("Name", &long), ("Value", "1234567890")]);
    let large = row_tokens(&[("Name", &huge), ("Value", "1234567890")]);
    assert_eq!(small, 13);
    assert!(large > 2_500);
    let wide = split_rows(&[heading()], &t, &[], 20_000);
    let narrow = split_rows(&[heading()], &t, &[], 2_500);
    assert_ne!(borders(&wide), borders(&narrow));
    for (ranges, max) in [(&wide, 20_000), (&narrow, 2_500)] {
        // Consecutive, starting at the first and ending at the last data row.
        assert_eq!(ranges[0].first, 2);
        assert_eq!(ranges.last().unwrap().last, 2001);
        for w in ranges.windows(2) {
            assert_eq!(w[1].first, w[0].last + 1);
        }
        for r in ranges.iter() {
            let rows = (r.first..=r.last).map(|n| if n == big_row { large } else { small });
            let size = fixed + rows.sum::<usize>();
            assert!(
                size <= max || r.first == r.last,
                "{:?} is {size} over {max}",
                (r.first, r.last)
            );
        }
        // Greedy: a range and the first row of the next would not fit.
        for w in ranges.windows(2) {
            let next = if w[1].first == big_row { large } else { small };
            let rows: usize = (w[0].first..=w[0].last)
                .map(|n| if n == big_row { large } else { small })
                .sum();
            assert!(fixed + rows + next > max, "{:?}", (w[0].first, w[0].last));
        }
    }
    // The large row is a range of its own in the narrow split.
    assert!(
        narrow
            .iter()
            .any(|r| (r.first, r.last) == (big_row, big_row))
    );
    assert_eq!(wide.len(), 2, "{:?}", borders(&wide));
}

#[test]
fn row_size_uses_the_column_of_each_cell() {
    // Column A has a long header path. Row 4 holds only a cell in column B, because A3
    // covers A4: it is sized with B's path (ceil((10 + 1 + 1 + 6) / 4) = 5), not A's (15).
    let long = "x".repeat(40);
    let mut merged = cell("a", "A3");
    merged.row_span = 2;
    let rows = vec![
        vec![cell("N", "A1"), cell("V", "B1")],
        data_row(2),
        vec![merged, cell("1", "B3")],
        vec![cell("1", "B4")],
        data_row(5),
    ];
    let mut l = layout(vec![1], 2, 5, Vec::new());
    l.header_paths = vec![long, "V".into()];
    let t = Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: Some(Box::new(l)),
    };
    // Header 17, row 2 17, rows 3 and 4 17 + 5: 56 holds rows 2 to 4.
    let ranges = split_rows(&[], &t, &[], 56);
    assert_eq!(borders(&ranges), vec![(2, 4), (5, 5)]);
}
