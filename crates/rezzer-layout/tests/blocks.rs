// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The mapping of detected regions into model blocks, on small sheets built in memory.

use rezzer_layout::{Caps, CellText, ColumnNames, Mapped, blocks, clean_merges, covered};
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Block, Certainty, Evidence, LayoutKind, LossReason, RowKind, Severity, TableLayout, plain_text,
};
use sheetlayout::{CellIn, Class, Fact, FactKind, Input, Pos, Rect, Style, detect};

/// A sheet: cells by A1 position. A text starting with `*` is bold; a text that parses as a
/// number is a number.
struct Sheet {
    cells: Vec<(u32, u32, String)>,
    merges: Vec<Rect>,
    facts: Vec<Fact>,
    styled: bool,
}

fn rect(a1: &str) -> Rect {
    let (a, b) = a1.split_once(':').unwrap_or((a1, a1));
    let (r0, c0) = rezzer_model::a1_position(a).unwrap();
    let (r1, c1) = rezzer_model::a1_position(b).unwrap();
    Rect {
        top: r0,
        left: c0,
        bottom: r1,
        right: c1,
    }
}

impl Sheet {
    fn new() -> Sheet {
        Sheet {
            cells: Vec::new(),
            merges: Vec::new(),
            facts: Vec::new(),
            styled: false,
        }
    }

    fn styled(mut self) -> Sheet {
        self.styled = true;
        self
    }

    /// Rows of `|`-separated cells from `at` down; an empty part is an empty cell.
    fn rows(mut self, at: &str, rows: &[&str]) -> Sheet {
        let (r0, c0) = rezzer_model::a1_position(at).unwrap();
        for (i, row) in rows.iter().enumerate() {
            for (j, text) in row.split('|').enumerate() {
                if !text.is_empty() {
                    self.cells
                        .push((r0 + i as u32, c0 + j as u32, text.to_string()));
                }
            }
        }
        self
    }

    fn merge(mut self, a1: &str) -> Sheet {
        self.merges.push(rect(a1));
        self
    }

    fn fact(mut self, a1: &str, kind: FactKind) -> Sheet {
        self.facts.push(Fact {
            rect: rect(a1),
            kind,
        });
        self
    }

    fn texts(&self) -> Vec<CellText> {
        let mut out: Vec<CellText> = self
            .cells
            .iter()
            .map(|(row, col, t)| CellText {
                row: *row,
                col: *col,
                text: t.trim_start_matches('*').to_string(),
            })
            .collect();
        out.sort_by_key(|c| (c.row, c.col));
        out
    }

    fn input(&self) -> Input<'static> {
        let mut cells: Vec<CellIn> = self
            .cells
            .iter()
            .map(|(row, col, t)| {
                let bold = t.starts_with('*');
                let text = t.trim_start_matches('*').to_string();
                let number = text.parse::<f64>().ok();
                let style = if self.styled {
                    Style {
                        bold: Some(bold),
                        filled: Some(false),
                        indent: Some(0),
                        center_across: Some(false),
                        rule_above: Some(false),
                        rule_below: Some(false),
                    }
                } else {
                    Style::default()
                };
                CellIn {
                    pos: Pos {
                        row: *row,
                        col: *col,
                    },
                    class: if number.is_some() {
                        Class::Number
                    } else {
                        Class::Text
                    },
                    text,
                    number,
                    style,
                    formula: None,
                }
            })
            .collect();
        cells.sort_by_key(|c| c.pos);
        Input {
            cells,
            merges: self.merges.clone(),
            facts: self.facts.clone(),
            names: Vec::new(),
            outline: Vec::new(),
            frozen: None,
            print_titles: None,
            header_guess: None,
            styles_read: self.styled,
            is_label: &|t| !t.trim().is_empty(),
            is_unit_column: &|_| false,
        }
    }

    fn map_with(&self, names: ColumnNames, caps: &mut Caps, shown: &[String]) -> Mapped {
        let layout = detect(&self.input(), &sheetlayout::Limits::default(), &|| false);
        let budget = Budget::new(Limits::default());
        blocks(
            "S",
            &layout,
            &self.texts(),
            &self.merges,
            names,
            shown,
            caps,
            &budget,
        )
    }

    fn map(&self) -> Mapped {
        self.map_with(ColumnNames::Letters, &mut Caps::default(), &[])
    }
}

fn tables(m: &Mapped) -> Vec<(&Vec<Vec<rezzer_model::Cell>>, usize, &TableLayout)> {
    m.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Table {
                rows,
                header_rows,
                layout: Some(l),
                ..
            } => Some((rows, *header_rows, &**l)),
            _ => None,
        })
        .collect()
}

fn headings(m: &Mapped) -> Vec<(u8, String, Option<String>)> {
    m.blocks
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

fn paragraphs(m: &Mapped, role: &str) -> Vec<(String, String)> {
    m.blocks
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

/// Every non-empty (anchor, text) the blocks hold, sorted.
fn cells_of(m: &Mapped) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for b in &m.blocks {
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
            Block::Table { rows, .. } => {
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

fn expected_cells(s: &Sheet) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = s
        .texts()
        .into_iter()
        .map(|c| {
            let a = Rect {
                top: c.row,
                left: c.col,
                bottom: c.row,
                right: c.col,
            }
            .a1();
            (format!("S!{a}"), c.text)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn single_table_off_a1_has_no_region_heading() {
    let s = Sheet::new().rows("B3", &["Name|Age", "Ann|31", "Bob|42"]);
    let m = s.map();
    assert!(headings(&m).is_empty(), "{:?}", headings(&m));
    let t = tables(&m);
    assert_eq!(t.len(), 1);
    let (rows, header_rows, l) = t[0];
    assert_eq!(header_rows, 1);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.len() == 2), "no padding column A");
    assert_eq!(rows[0][0].anchor.as_ref().unwrap().0, "S!B3");
    assert_eq!(rows[2][1].anchor.as_ref().unwrap().0, "S!C5");
    assert_eq!(l.kind, LayoutKind::Table);
    assert_eq!(l.key, "S!B3");
    assert_eq!(l.range, "B3:C5");
    assert_eq!(l.data.as_deref(), Some("B4:C5"));
    assert_eq!(l.header_rows, [3]);
    assert_eq!(l.header_paths, ["Name", "Age"]);
    assert_eq!(l.certainty, Certainty::Rule);
    assert!(m.losses.is_empty(), "{:?}", m.losses);
    assert_eq!(cells_of(&m), expected_cells(&s));
}

#[test]
fn regions_get_headings_captions_and_notes() {
    let s = Sheet::new()
        .styled()
        .rows(
            "A1",
            &[
                "*Sales report",
                "*in dollars",
                "*Item|*2024|*2025",
                "Pens|1|2",
                "Ink|3|4",
            ],
        )
        .rows("A8", &["*x|*y", "1|2"])
        .rows("A6", &["Source: own count"]);
    let m = s.map();
    let h = headings(&m);
    assert_eq!(h[0], (3, "Sales report".into(), Some("S!A1".into())));
    assert!(h.iter().any(|x| x.1.starts_with("Table A8")), "{h:?}");
    assert_eq!(
        paragraphs(&m, "caption"),
        [("in dollars".into(), "S!A2".into())]
    );
    assert_eq!(
        paragraphs(&m, "note"),
        [("Source: own count".into(), "S!A6".into())]
    );
    let t = tables(&m);
    let (rows, header_rows, l) = t[0];
    assert_eq!(header_rows, 1);
    assert_eq!(rows.len(), 3, "caption and note rows are not table rows");
    assert_eq!(l.header_paths, ["Item", "2024", "2025"]);
    assert_eq!(cells_of(&m), expected_cells(&s));
    // The caption joins into Table.caption.
    let caption = m.blocks.iter().find_map(|b| match b {
        Block::Table { caption, .. } => caption.clone(),
        _ => None,
    });
    assert_eq!(caption.as_deref(), Some("Sales report in dollars"));
}

#[test]
fn headerless_table_takes_column_names() {
    let s = Sheet::new().rows("C2", &["1|2", "3|4"]);
    let m = s.map();
    let (_, header_rows, l) = tables(&m)[0];
    assert_eq!(header_rows, 0);
    assert_eq!(l.kind, LayoutKind::Table);
    assert_eq!(l.header_paths, ["C", "D"]);
    assert!(m.losses.is_empty(), "{:?}", m.losses);
    let m = s.map_with(ColumnNames::Numbers, &mut Caps::default(), &[]);
    assert_eq!(tables(&m)[0].2.header_paths, ["3", "4"]);
}

#[test]
fn merge_spans_and_covered_cell_is_a_loss() {
    let s = Sheet::new()
        .rows("A1", &["Name|Group|", "Ann|1|9", "Bob|2|3"])
        .merge("B1:C1");
    let mut s = s;
    // A value under the merge, as calamine returns ODS covered cells.
    s.cells.push((0, 2, "hidden".into()));
    let m = s.map();
    let (rows, _, l) = tables(&m)[0];
    assert_eq!(rows[0].len(), 2);
    assert_eq!(rows[0][1].col_span, 2);
    assert_eq!(l.header_paths, ["Name", "Group", "Group"]);
    let loss = m
        .losses
        .iter()
        .find(|l| l.location == "S!C1")
        .expect("a loss for the covered cell");
    assert_eq!(loss.why, LossReason::Unsupported);
    assert_eq!(loss.severity, Severity::Info);
    assert!(
        loss.what.contains("hidden") && loss.what.contains("B1:C1"),
        "{}",
        loss.what
    );
    assert!(!cells_of(&m).iter().any(|c| c.1 == "hidden"));
}

#[test]
fn caps_cut_rows_and_columns_with_a_loss() {
    let s = Sheet::new().rows("A1", &["a|b|c", "1|2|3", "4|5|6", "7|8|9"]);
    let mut caps = Caps {
        cells_left: 6,
        max_cols: 2,
    };
    let m = s.map_with(ColumnNames::Letters, &mut caps, &[]);
    let (rows, _, l) = tables(&m)[0];
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.len() == 2));
    assert_eq!(l.header_paths.len(), 2);
    assert_eq!(caps.cells_left, 0);
    let what: Vec<&str> = m.losses.iter().map(|l| l.what.as_str()).collect();
    assert_eq!(
        what,
        [
            "column C of Table A1:C4 not shown (at most 1000 columns per region); their formulas are still listed",
            "row 4 of Table A1:C4 not shown (at most 200000 cells per sheet); their formulas are still listed",
        ]
    );
    assert!(
        m.losses
            .iter()
            .all(|l| l.why == LossReason::Limit && l.severity == Severity::Warning)
    );
    assert!(m.partial);
}

#[test]
fn excel_table_key_and_exact_certainty() {
    let s = Sheet::new()
        .rows("A1", &["Name|Sales", "Ann|1", "Bob|2"])
        .rows("A6", &["Name|Sales", "Cy|3"])
        .fact(
            "A1:B3",
            FactKind::ListObject {
                name: "Sales_Alice".into(),
                header_rows: 1,
                totals_rows: 0,
                columns: vec!["Name".into(), "Sales".into()],
            },
        );
    let m = s.map();
    let t = tables(&m);
    assert_eq!(t[0].2.key, "table:Sales_Alice");
    assert_eq!(t[0].2.certainty, Certainty::Exact);
    assert!(matches!(&t[0].2.found_by[0], Evidence::ListObject { name } if name == "Sales_Alice"));
    assert_eq!(t[1].2.key, "S!A6");
    assert_eq!(cells_of(&m), expected_cells(&s));
}

#[test]
fn totals_row_is_a_row_note() {
    let s = Sheet::new()
        .rows("A1", &["Name|Sales", "Ann|1", "Bob|2", "Total|3"])
        .fact(
            "A1:B4",
            FactKind::ListObject {
                name: "T".into(),
                header_rows: 1,
                totals_rows: 1,
                columns: vec!["Name".into(), "Sales".into()],
            },
        );
    let m = s.map();
    let l = tables(&m)[0].2;
    let total = l.rows.iter().find(|n| n.kind == RowKind::Total).unwrap();
    assert_eq!(total.row, 4, "rows are 1-based");
}

#[test]
fn name_caption_only_when_shown() {
    let s = Sheet::new()
        .rows("A1", &["a|b", "1|2"])
        .rows("A5", &["c|d", "3|4"]);
    let mut input = s.input();
    input.names.push(sheetlayout::NameIn {
        name: "Prices".into(),
        rect: rect("A5:B6"),
        hidden: false,
        builtin: false,
    });
    let layout = detect(&input, &sheetlayout::Limits::default(), &|| false);
    let budget = Budget::new(Limits::default());
    let shown = ["Prices".to_string()];
    let m = blocks(
        "S",
        &layout,
        &s.texts(),
        &[],
        ColumnNames::Letters,
        &shown,
        &mut Caps::default(),
        &budget,
    );
    let h = headings(&m);
    assert!(h.contains(&(3, "Prices".into(), None)), "{h:?}");
    let m = blocks(
        "S",
        &layout,
        &s.texts(),
        &[],
        ColumnNames::Letters,
        &[],
        &mut Caps::default(),
        &budget,
    );
    assert!(!headings(&m).iter().any(|h| h.1 == "Prices"));
}

#[test]
fn stop_gives_a_limit_loss_and_partial() {
    let s = Sheet::new()
        .rows("A1", &["a|b", "1|2"])
        .rows("A5", &["c|d", "3|4"]);
    let layout = detect(&s.input(), &sheetlayout::Limits::default(), &|| true);
    let budget = Budget::new(Limits::default());
    let m = blocks(
        "S",
        &layout,
        &s.texts(),
        &[],
        ColumnNames::Letters,
        &[],
        &mut Caps::default(),
        &budget,
    );
    assert!(m.partial);
    assert!(
        m.losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("time limit")),
        "{:?}",
        m.losses
    );
    assert_eq!(cells_of(&m), expected_cells(&s));
}

#[test]
fn unsure_region_is_an_uncertain_loss() {
    // A bold first row over a plain second header row: the rules disagree.
    let s = Sheet::new()
        .styled()
        .rows("A1", &["*Name|*Price", "first|second", "Ann|1"]);
    let m = s.map();
    let l = tables(&m)[0].2;
    assert_eq!(l.certainty, Certainty::Unsure);
    assert_eq!(l.kind, LayoutKind::Grid);
    let loss = m
        .losses
        .iter()
        .find(|x| x.why == LossReason::Uncertain)
        .expect("an uncertain loss");
    assert_eq!(loss.location, format!("S!{}", l.range));
    assert_eq!(loss.severity, Severity::Info);
    assert!(!l.unsure.is_empty());
    assert_eq!(loss.what, l.unsure.join("; "));
    assert_eq!(cells_of(&m), expected_cells(&s));
}

#[test]
fn overlapping_merges_first_wins() {
    let (kept, ignored) = clean_merges(&[rect("A1:B2"), rect("B2:C3"), rect("D1"), rect("D4:E4")]);
    assert_eq!(kept, [rect("A1:B2"), rect("D4:E4")]);
    assert_eq!(ignored, 1);
    let at = [(0, 0), (0, 1), (1, 1), (2, 2), (3, 4)];
    assert_eq!(covered(&at, &kept), [false, true, true, false, true]);
}

#[test]
fn sheetlayout_a1_agrees_with_the_model() {
    // Column letters across the one-, two- and three-letter borders, rows up to the last.
    let cols = [0u32, 25, 26, 51, 52, 701, 702, 703, 16_383];
    let rows = [0u32, 9, 99, 1_048_575];
    for &col in &cols {
        for &row in &rows {
            let a1 = Rect::cell(Pos { row, col }).a1();
            assert_eq!(rezzer_model::a1_position(&a1), Some((row, col)), "{a1}");
            assert_eq!(
                rezzer_model::column_letter(col as usize),
                sheetlayout::column_letters(col)
            );
        }
    }
}

#[test]
fn title_region_has_no_rows_and_section_headings_are_level_four() {
    // A title two rows above a table it does not reach (another table has its own caption).
    let s = Sheet::new()
        .styled()
        .rows("A1", &["*Overview"])
        .rows("A4", &["*Part one", "*a|*b", "1|2"]);
    let m = s.map();
    assert_eq!(cells_of(&m), expected_cells(&s));
    let kinds: Vec<LayoutKind> = tables(&m).iter().map(|t| t.2.kind).collect();
    assert_eq!(kinds, [LayoutKind::Title, LayoutKind::Table]);
    assert!(tables(&m)[0].0.is_empty());
    assert_eq!(headings(&m)[0], (3, "Overview".into(), Some("S!A1".into())));
    let empty_title_caption = m.blocks.iter().any(|b| {
        matches!(b, Block::Table { caption: Some(_), layout: Some(l), .. } if l.kind == LayoutKind::Title)
    });
    assert!(!empty_title_caption);
    // Section rows with no empty row between them (R1).
    let s = Sheet::new().styled().rows(
        "A1",
        &[
            "*First",
            "Name|Ann",
            "City|Rome",
            "*Second",
            "Name|Bob",
            "City|Oslo",
        ],
    );
    let m = s.map();
    assert_eq!(cells_of(&m), expected_cells(&s));
    let h = headings(&m);
    assert!(
        h.iter()
            .any(|x| x.0 == 4 && x.1 == "Second" && x.2.as_deref() == Some("S!A4")),
        "{h:?}"
    );
}

#[test]
fn region_without_a_value_is_left_out() {
    // Merged areas with no value: occupied for the detector, but nothing to show.
    let s = Sheet::new()
        .rows("A1", &["a|b", "1|2"])
        .merge("A5:C6")
        .merge("A8:B9");
    let m = s.map();
    let ranges: Vec<&str> = tables(&m).iter().map(|t| t.2.range.as_str()).collect();
    assert_eq!(ranges, ["A1:B2"]);
    // The one region left is the sheet's only table: no generated heading.
    assert!(headings(&m).is_empty(), "{:?}", headings(&m));
    assert!(m.losses.is_empty(), "{:?}", m.losses);
    let only_merges = Sheet::new().merge("A1:C50");
    let m = only_merges.map();
    assert!(m.blocks.is_empty());
    assert!(m.losses.is_empty(), "{:?}", m.losses);
}
