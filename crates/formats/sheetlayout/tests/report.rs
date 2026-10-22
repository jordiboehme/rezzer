// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Report rules and row kinds: group rows, notes, hierarchy, subtotals, section headings,
//! label and value blocks, text blocks, header-less tables, continuation, notes and sides.

mod common;

use std::cell::Cell;

use common::{Sheet, every_cell_once, list_object, no_overlap, ranges, rect};
use sheetlayout::{
    Certainty, Evidence, FactKind, IssueKind, Kind, Layout, Limits, Region, RowKind, RowNote,
    detect,
};

fn never() -> bool {
    false
}

fn only(layout: &Layout) -> &Region {
    assert_eq!(layout.regions.len(), 1, "{:?}", layout.regions);
    &layout.regions[0]
}

/// 1-based rows.
fn ones(rows: &[u32]) -> Vec<u32> {
    rows.iter().map(|&x| x + 1).collect()
}

fn has_rule(r: &Region, id: &str) -> bool {
    r.evidence
        .iter()
        .any(|e| matches!(e, Evidence::Rule { id: i, .. } if *i == id))
}

fn rule_detail(r: &Region, id: &str) -> String {
    r.evidence
        .iter()
        .find_map(|e| match e {
            Evidence::Rule { id: i, detail } if *i == id => Some(detail.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The 1-based rows of a kind.
fn rows_of(r: &Region, kind: RowKind) -> Vec<u32> {
    r.rows
        .iter()
        .filter(|n| n.kind == kind)
        .map(|n| n.row + 1)
        .collect()
}

/// The note of a 1-based row.
fn note(r: &Region, row: u32) -> Option<&RowNote> {
    r.rows.iter().find(|n| n.row + 1 == row)
}

fn no_unit(_: &[&str]) -> bool {
    false
}

fn any_label(_: &str) -> bool {
    true
}

/// A table A1:C3 with a header row.
fn small_table() -> Sheet {
    Sheet::new().rows("A1", &["Name|Size|Mass", "a|1|2", "b|3|4"])
}

#[test]
fn notes_below_attach() {
    let sheet = small_table()
        .rows("A5", &["Source: survey", "Values in kg"])
        .rows("E1", &["side note", "more"])
        .cell("A10", "far away");
    let layout = sheet.detect();
    assert_eq!(ranges(&layout), ["A1:C6", "E1:E2", "A10"]);
    let table = &layout.regions[0];
    assert_eq!(table.kind, Kind::Table);
    assert_eq!(ones(&table.notes), [5, 6]);
    assert_eq!(table.data, Some(rect("A2:C3")));
    assert_eq!(rule_detail(table, "N"), "notes in rows 5 to 6");
    // Rule S: the text beside the table stays its own region.
    assert_eq!(layout.regions[1].kind, Kind::Text);
    // Three empty rows below is too far.
    assert_eq!(layout.regions[2].kind, Kind::Text);
    // Two empty rows below still attach.
    let layout = small_table().cell("A6", "Source: survey").detect();
    assert_eq!(ranges(&layout), ["A1:C6"]);
    assert_eq!(ones(&layout.regions[0].notes), [6]);
    // A note that starts left of the table does not attach.
    let layout = Sheet::new()
        .rows("B1", &["Name|Size", "a|1", "b|3"])
        .cell("A5", "Source: survey")
        .detect();
    assert_eq!(ranges(&layout), ["B1:C3", "A5"]);
    // A note wider than the table does not attach: the joined range would reach the region
    // beside the table.
    let layout = small_table()
        .rows("E1", &["x|y", "1|2"])
        .cell("A5", "Source: survey")
        .merge("A5:F5")
        .detect();
    assert_eq!(ranges(&layout), ["A1:C3", "E1:F2", "A5:F5"]);
    assert!(layout.regions[0].notes.is_empty());
    // A note whose joined range would reach a region beside it does not attach.
    let layout = Sheet::new()
        .rows("A1", &["Name|Size|Mass|Unit", "a|1|2|kg", "b|3|4|kg"])
        .cell("A5", "Source: survey")
        .rows("C5", &["x|y"])
        .detect();
    assert_eq!(ranges(&layout), ["A1:D3", "A5", "C5:D5"]);
    assert!(layout.regions[0].notes.is_empty());
}

#[test]
fn trailing_rows_of_one_text_are_notes() {
    let layout = small_table()
        .rows("A4", &["|Note 1", "Note 2"])
        .merge("A5:C5")
        .detect();
    let r = only(&layout);
    assert_eq!(ones(&r.notes), [4, 5]);
    assert_eq!(r.data, Some(rect("A2:C3")));
    assert!(r.rows.iter().all(|n| n.kind != RowKind::Group));
    // A label alone in the first column, with no data row after it, is a data row with
    // empty values, not a note.
    let layout = small_table()
        .rows("A4", &["Note 1", "Note 2"])
        .merge("A5:C5")
        .detect();
    let r = only(&layout);
    assert_eq!(ones(&r.notes), [5]);
    assert_eq!(r.data, Some(rect("A2:C4")));
}

#[test]
fn label_only_rows_stay_data() {
    // Sparse data: "b" is followed by a data row and reads as a group row; "d" at the end
    // is a data row with empty values, not a note.
    let layout = Sheet::new()
        .rows("A1", &["Name|Q1|Q2", "a|1|2", "b||", "c|3|4", "d||"])
        .detect();
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Group), [3]);
    assert!(r.notes.is_empty());
    assert_eq!(r.data, Some(rect("A2:C5")));
}

#[test]
fn group_rows_need_data_rows_after_them() {
    // Group rows in a row are nested groups: both are followed by data rows.
    let layout = Sheet::new()
        .rows(
            "A1",
            &["Item|Value", "Annual Totals", "Year 2024", "a|1", "b|2"],
        )
        .detect();
    assert_eq!(rows_of(only(&layout), RowKind::Group), [2, 3]);
    // A label followed by a unit row is no group row: no data row follows it.
    let layout = Sheet::new()
        .rows("A1", &["Item|V|W", "a|1|2", "Fruit||", "|kg|", "b|3|4"])
        .merge("A3:B3")
        .detect();
    let r = only(&layout);
    assert!(rows_of(r, RowKind::Group).is_empty(), "{:?}", r.rows);
    assert!(r.notes.is_empty());
}

#[test]
fn continuation_needs_same_columns() {
    let joined = small_table().rows("A5", &["c|5|6"]).detect();
    let r = only(&joined);
    assert_eq!(r.rect, rect("A1:C5"));
    assert_eq!(r.data, Some(rect("A2:C5")));
    assert_eq!(rows_of(r, RowKind::Blank), [4]);
    assert!(has_rule(r, "C"));
    // Other columns: no join.
    let layout = small_table().rows("A5", &["c|5"]).detect();
    assert_eq!(ranges(&layout), ["A1:C3", "A5:B5"]);
    // Two empty rows: no join.
    let layout = small_table().rows("A6", &["c|5|6"]).detect();
    assert_eq!(ranges(&layout), ["A1:C3", "A6:C6"]);
    // A region with its own header band is a table of its own.
    let layout = small_table().rows("A5", &["X|Y|Z", "7|8|9"]).detect();
    assert_eq!(ranges(&layout), ["A1:C3", "A5:C6"]);
    assert!(layout.regions.iter().all(|r| !has_rule(r, "C")));
    // A header-less table above is a table too: it is continued.
    let layout = Sheet::new()
        .rows("A1", &["1|2|3", "4|5|6"])
        .rows("A4", &["7|8|9"])
        .detect();
    assert_eq!(ranges(&layout), ["A1:C4"]);
    assert_eq!(rows_of(only(&layout), RowKind::Blank), [3]);
    // An Excel table keeps its exact range.
    let layout = small_table()
        .fact("A1:C3", list_object("T"))
        .rows("A5", &["c|5|6"])
        .detect();
    assert_eq!(ranges(&layout), ["A1:C3", "A5:C5"]);
    // A region beside the empty row does not block the join.
    let layout = small_table()
        .rows("A5", &["c|5|6"])
        .rows("E4", &["side"])
        .detect();
    assert!(layout.regions.iter().any(|r| r.rect == rect("A1:C5")));
}

/// The Census NST-EST2023-POP shape: title rows 1 and 2, a two-row header, data to row 60,
/// an empty row 61, Puerto Rico in row 62 and notes in rows 63 to 68.
fn census(with_name: bool) -> Sheet {
    let mut s = Sheet::styled()
        .cell("A1", "^Accessibility note")
        .cell("A2", "*^Annual Estimates of the Resident Population")
        .rows(
            "A3",
            &[
                "*^Geographic Area|*^April 1, 2020 Base|*^Population Estimate (as of July 1)",
                "||*^2020|*^2021|*^2022|*^2023",
            ],
        )
        .merge("A3:A4")
        .merge("B3:B4")
        .merge("C3:F3");
    s = s.rows("A5", &["*~United States|1|2|3|4|5"]);
    for (k, name) in ["Northeast", "Midwest", "South", "West"].iter().enumerate() {
        s = s.rows(&format!("A{}", 6 + k), &[&format!("~{name}|1|2|3|4|5")]);
    }
    for row in 10..=60 {
        s = s.rows(&format!("A{row}"), &[&format!(".State {row}|1|2|3|4|5")]);
    }
    s = s.rows("A62", &["Puerto Rico|1|2|3|4|5"]);
    for row in 63..=68 {
        s = s
            .cell(&format!("A{row}"), &format!("Note {row}"))
            .merge(&format!("A{row}:F{row}"));
    }
    if with_name {
        s = s.name("_NST01", "A4:F62");
    }
    s
}

fn census_checks(layout: &Layout) {
    let r = only(layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(r.rect, rect("A1:F68"));
    assert_eq!(ones(&r.header_rows), [3, 4]);
    assert_eq!(r.data, Some(rect("A5:F62")));
    assert_eq!(ones(&r.notes), [63, 64, 65, 66, 67, 68]);
    // No row has a parent, so no level is shown: only the empty row of rule C is noted.
    assert!(
        r.rows
            .iter()
            .all(|n| n.kind == RowKind::Blank && n.level.is_none() && n.parent.is_none()),
        "{:?}",
        r.rows
    );
    assert_eq!(r.label_cols, 1);
}

#[test]
fn continuation_runs_after_procedure() {
    let layout = census(false).detect();
    census_checks(&layout);
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Blank), [61]);
    assert!(has_rule(r, "C"));
    // With the name the join is a fact, and rule C has nothing to do.
    let layout = census(true).detect();
    census_checks(&layout);
    assert!(!has_rule(only(&layout), "C"));
}

#[test]
fn is_label_callback_is_used() {
    let sheet = Sheet::new().rows("A1", &["Name|Ann", "City|Rome", "Job|Cook"]);
    let r = sheet.detect_with(&any_label, &no_unit).regions[0].clone();
    assert_eq!(r.kind, Kind::KeyValue);
    let layout = sheet.detect_with(&|_| false, &no_unit);
    let r = only(&layout);
    assert_ne!(r.kind, Kind::KeyValue);
    assert_eq!(r.kind, Kind::Grid);
    // The callback sees the label texts.
    let seen = std::cell::RefCell::new(Vec::new());
    let label = |t: &str| {
        seen.borrow_mut().push(t.to_string());
        true
    };
    sheet.detect_with(&label, &no_unit);
    for t in ["Name", "City", "Job"] {
        assert!(seen.borrow().iter().any(|s| s == t), "{t}");
    }
    // One refused label is enough.
    let layout = sheet.detect_with(&|t| t != "City", &no_unit);
    assert_ne!(only(&layout).kind, Kind::KeyValue);
}

/// The EIA table 1.1 shape: captions in rows 1 and 2, a band in rows 3 and 4, group rows
/// merged over the width and a note in row 57.
fn eia() -> Sheet {
    let mut s = Sheet::new()
        .rows(
            "A1",
            &[
                "Table 1.1. Net Generation by Energy Source",
                "(Thousand Megawatthours)",
                "|Generation at Utility Scale Facilities||Total",
                "Period|Coal|Natural Gas|",
            ],
        )
        .merge("B3:C3")
        .merge("D3:D4");
    let groups = [5, 16, 29, 42, 50, 54];
    for row in 5..=56 {
        if groups.contains(&row) {
            s = s
                .cell(&format!("A{row}"), &format!("Group {row}"))
                .merge(&format!("A{row}:D{row}"));
        } else {
            s = s.rows(&format!("A{row}"), &[&format!("{}|1|2|3", 1950 + row)]);
        }
    }
    s.cell("A57", "Note: values are rounded").merge("A57:D57")
}

#[test]
fn group_rows_inside_table() {
    let layout = eia().detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(ones(&r.captions), [1, 2]);
    assert_eq!(ones(&r.header_rows), [3, 4]);
    assert_eq!(rows_of(r, RowKind::Group), [5, 16, 29, 42, 50, 54]);
    assert_eq!(ones(&r.notes), [57]);
    assert_eq!(r.data, Some(rect("A5:D56")));
    assert_eq!(note(r, 5).unwrap().why, "one merged area across the table");
    // A group row of one text in the first column, without a merge.
    let layout = Sheet::new()
        .rows(
            "A1",
            &["Item|Value|Unit", "Fruit||", "apple|1|kg", "pear|2|kg"],
        )
        .detect();
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Group), [2]);
    assert_eq!(note(r, 2).unwrap().why, "one text in the first column");
    // A merge over 80 % of the width is across the table; over 75 % it is not, and a text
    // that is not in the first column is no group row then. The text is in the merge.
    let head: Vec<String> = (0..20).map(|k| format!("h{k}")).collect();
    let one: Vec<String> = (0..20).map(|k| k.to_string()).collect();
    let (head, one) = (head.join("|"), one.join("|"));
    let base = Sheet::new()
        .rows("A1", &[&head, &one, "", &one])
        .cell("E3", "Fruit");
    let r = base.clone().merge("E3:T3").detect().regions.remove(0);
    assert_eq!(rows_of(&r, RowKind::Group), [3]);
    let r = base.merge("E3:S3").detect().regions.remove(0);
    assert!(rows_of(&r, RowKind::Group).is_empty(), "{:?}", r.rows);
}

#[test]
fn units_row_under_header_stays_data() {
    // Limit: a units row with one unit is a single-text row. It ends the band; it is not in
    // the first column, so it is no group row, and it is not trailing, so it is no note.
    let layout = Sheet::new()
        .rows("A1", &["Name|Length|Mass", "|cm|", "a|1|2", "b|3|4"])
        .detect();
    let r = only(&layout);
    assert_eq!(ones(&r.header_rows), [1]);
    assert_eq!(r.header_paths, ["Name", "Length", "Mass"]);
    assert_eq!(r.data, Some(rect("A2:C4")));
    assert!(r.rows.is_empty(), "{:?}", r.rows);
    assert!(r.notes.is_empty());
}

#[test]
fn hierarchy_by_outline_indent_then_text() {
    // Outline levels: rows outside a group are level 0 when the table has groups.
    let layout = Sheet::new()
        .rows(
            "A1",
            &["Item|Value", "Total|10", "Part A|4", "Part B|6", "Other|1"],
        )
        .group(3, 4, 1)
        .detect();
    let r = only(&layout);
    assert_eq!(r.label_cols, 1);
    assert_eq!(note(r, 2).unwrap().level, Some(0));
    assert_eq!(note(r, 2).unwrap().why, "outline level 0");
    let a = note(r, 3).unwrap();
    assert_eq!((a.level, a.parent), (Some(1), Some(1)));
    assert_eq!(a.why, "outline level 1");
    assert_eq!(note(r, 4).unwrap().parent, Some(1));
    assert_eq!(note(r, 5).unwrap().parent, None);
    // Indent: a row without indent is the base level.
    let layout = Sheet::styled()
        .rows(
            "A1",
            &[
                "*Item|*Value",
                "Assets|10",
                "~Cash|4",
                "~~Petty|1",
                "~Bank|3",
            ],
        )
        .detect();
    let r = only(&layout);
    assert_eq!(note(r, 3).unwrap().parent, Some(1));
    assert_eq!(note(r, 3).unwrap().why, "indent 1");
    assert_eq!(note(r, 4).unwrap().parent, Some(2));
    assert_eq!(note(r, 5).unwrap().parent, Some(1));
    // Leading dots, ranked by length.
    let layout = Sheet::new()
        .rows("A1", &["Item|Value", "Assets|10", ".Cash|4", "...Petty|1"])
        .detect();
    let r = only(&layout);
    assert_eq!(note(r, 3).unwrap().level, Some(1));
    assert_eq!(note(r, 4).unwrap().level, Some(2));
    assert_eq!(note(r, 4).unwrap().parent, Some(2));
    assert_eq!(note(r, 4).unwrap().why, "leading dots or spaces, rank 2");
    // Each row takes the first source it has: indent before text, outline before indent.
    let layout = Sheet::styled()
        .rows("A1", &["*Item|*Value", "Top|1", "~A|2", "~...C|3", "..D|4"])
        .detect();
    let r = only(&layout);
    assert_eq!(note(r, 4).unwrap().level, Some(1));
    assert_eq!(
        note(r, 4).unwrap().parent,
        Some(1),
        "C by indent, under Top"
    );
    assert_eq!(note(r, 5).unwrap().parent, Some(1), "D by dots, rank 1");
    let layout = Sheet::styled()
        .rows("A1", &["*Item|*Value", "Top|1", "~~A|2", "~B|3"])
        .group(3, 4, 1)
        .detect();
    let r = only(&layout);
    assert_eq!(note(r, 3).unwrap().level, Some(1));
    assert_eq!(note(r, 4).unwrap().level, Some(1));
    assert_eq!(note(r, 4).unwrap().parent, Some(1));
    // Overlapping outline groups: the deepest counts.
    let layout = Sheet::new()
        .rows("A1", &["Item|Value", "Top|1", "A|2", "B|3"])
        .group(3, 4, 1)
        .group(4, 4, 2)
        .detect();
    let r = only(&layout);
    assert_eq!(note(r, 4).unwrap().level, Some(2));
    assert_eq!(note(r, 4).unwrap().parent, Some(2));
    // Also past level 63.
    let layout = Sheet::new()
        .rows("A1", &["Item|Value", "Top|1", "A|2", "B|3"])
        .group(3, 4, 1)
        .group(4, 4, 70)
        .detect();
    assert_eq!(note(only(&layout), 4).unwrap().level, Some(70));
}

#[test]
fn no_false_parent_when_levels_equal() {
    // Regions with indent 1, states with one leading dot: every row has level 1.
    let layout = Sheet::styled()
        .rows(
            "A1",
            &[
                "*Area|*Count",
                "~United States|1",
                "~Northeast|2",
                "~West|3",
                ".Alabama|4",
                ".Alaska|5",
            ],
        )
        .detect();
    let r = only(&layout);
    assert_eq!(r.label_cols, 1);
    assert!(r.rows.iter().all(|n| n.parent.is_none()), "{:?}", r.rows);
    assert!(r.rows.iter().all(|n| n.level.is_none()), "{:?}", r.rows);
    // No label column: no hierarchy, even with indent.
    let layout = Sheet::styled()
        .rows("A1", &["*Year|*Count", "2001|1", "~2002|2"])
        .detect();
    assert!(only(&layout).rows.is_empty());
}

#[test]
fn subtotal_by_formula_not_by_word() {
    let layout = Sheet::new()
        .rows(
            "A1",
            &["Item|Value", "a|1", "b|2", "Total|3", "Gesamtergebnis|3"],
        )
        .detect();
    let r = only(&layout);
    assert!(rows_of(r, RowKind::Subtotal).is_empty());
    assert!(rows_of(r, RowKind::Total).is_empty());
    assert!(r.rows.is_empty(), "{:?}", r.rows);
}

#[test]
fn formula_flags_feed_subtotals() {
    let layout = Sheet::new()
        .rows(
            "A1",
            &[
                "Item|Value",
                "a|1",
                "b|2",
                "Sub|=3",
                "c|4",
                "d|5",
                "Sum|=9",
                "All|=15",
                "x|1",
                "y|=3",
                "z|=7",
            ],
        )
        .formula("B4", &["SUBTOTAL"], &["B2:B3"])
        .formula("B7", &["SUM"], &["B5:B6"])
        .formula("B8", &["SUM"], &["B2:B7"])
        .formula("B10", &["SUM"], &["B2:B3"])
        .formula("B11", &["SUM"], &["A2:A10"])
        .detect();
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Subtotal), [4, 7]);
    assert_eq!(rows_of(r, RowKind::Total), [8]);
    assert_eq!(note(r, 4).unwrap().why, "SUBTOTAL formula in B4");
    assert_eq!(note(r, 7).unwrap().why, "SUM over B5:B6 in B7");
    assert_eq!(note(r, 8).unwrap().why, "SUM over B2:B7 in B8");
    // A SUM that does not reach the row, or reads another column, marks nothing.
    assert!(note(r, 10).is_none());
    assert!(note(r, 11).is_none());
    // AGGREGATE counts like SUBTOTAL; a SUM from above the table does not count.
    let layout = Sheet::new()
        .rows("A1", &["Item|Value", "a|1", "b|2", "S|=3", "T|=6"])
        .formula("B4", &["AGGREGATE"], &["B2:B3"])
        .formula("B5", &["SUM"], &["B1:B4"])
        .detect();
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Subtotal), [4]);
    assert_eq!(note(r, 4).unwrap().why, "AGGREGATE formula in B4");
    assert!(note(r, 5).is_none());
    // The first data row is the first row that is not a group row.
    let layout = Sheet::new()
        .rows("A1", &["Item|Value", "Fruit|", "a|1", "b|2", "All|=3"])
        .formula("B5", &["SUM"], &["B3:B4"])
        .detect();
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Group), [2]);
    assert_eq!(rows_of(r, RowKind::Total), [5]);
}

#[test]
fn fact_totals_rows_are_totals() {
    let base = Sheet::new().rows("A1", &["Item|Value", "a|1", "b|2", "Sum|3"]);
    let layout = base
        .clone()
        .fact(
            "A1:B4",
            FactKind::ListObject {
                name: "T".into(),
                header_rows: 1,
                totals_rows: 1,
                columns: Vec::new(),
            },
        )
        .detect();
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Total), [4]);
    assert_eq!(note(r, 4).unwrap().why, "Excel table totals row");
    let layout = base
        .clone()
        .fact(
            "A1:B4",
            FactKind::DatabaseRange {
                name: "D".into(),
                header: true,
                footer: true,
            },
        )
        .detect();
    assert_eq!(note(only(&layout), 4).unwrap().why, "database range footer");
    let layout = base
        .clone()
        .fact(
            "A1:B4",
            FactKind::Pivot {
                name: "P".into(),
                first_header_row: 0,
                first_data_row: 1,
                first_data_col: 1,
                total_row: true,
                total_col: false,
            },
        )
        .detect();
    assert_eq!(note(only(&layout), 4).unwrap().why, "pivot total row");
    // Fact regions go through step 6c too: group rows and trailing notes.
    let layout = Sheet::new()
        .rows("A1", &["Item|Value", "a|1", "Fruit|", "b|2", "Note|"])
        .merge("A5:B5")
        .fact("A1:B5", list_object("T"))
        .detect();
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Group), [3]);
    assert_eq!(ones(&r.notes), [5]);
    assert_eq!(r.data, Some(rect("A2:B4")));
    // A stated totals row that holds only a label stays a total, not a note.
    let layout = Sheet::new()
        .rows("A1", &["Item|Value", "a|1", "b|2", "Sum|"])
        .fact(
            "A1:B4",
            FactKind::ListObject {
                name: "T".into(),
                header_rows: 1,
                totals_rows: 1,
                columns: Vec::new(),
            },
        )
        .detect();
    let r = only(&layout);
    assert!(r.notes.is_empty());
    assert_eq!(rows_of(r, RowKind::Total), [4]);
}

#[test]
fn section_rows_split_region() {
    let layout = Sheet::styled()
        .rows(
            "A1",
            &[
                "*^Design report",
                "*Inputs",
                "Length|5",
                "Width|3",
                "*Outputs",
                "Area|15",
                "Cost|20",
            ],
        )
        .detect();
    assert_eq!(ranges(&layout), ["A1:B1", "A2:B4", "A5:B7"]);
    let title = &layout.regions[0];
    assert_eq!(title.kind, Kind::Title);
    assert_eq!(title.captions, [0]);
    let a = &layout.regions[1];
    assert_eq!(a.heading, Some((1, 1)));
    assert_eq!(a.kind, Kind::KeyValue);
    assert_eq!(a.data, Some(rect("A3:B4")));
    assert!(has_rule(a, "R1"));
    let b = &layout.regions[2];
    assert_eq!(b.heading, Some((4, 1)));
    assert_eq!(b.kind, Kind::KeyValue);
    // A section with a table of its own.
    let layout = Sheet::styled()
        .rows(
            "A1",
            &[
                "Plain line|x",
                "*Prices",
                "*Item|*Price",
                "apple|1",
                "*Notes",
                "one|two",
                "three|four",
            ],
        )
        .detect();
    assert_eq!(ranges(&layout), ["A1:B1", "A2:B4", "A5:B7"]);
    let t = &layout.regions[1];
    assert_eq!(t.heading, Some((1, 1)));
    assert_eq!(t.kind, Kind::Table);
    assert_eq!(ones(&t.header_rows), [3]);
    // A heading needs a non-empty row after it, and must stand in the first column.
    let layout = Sheet::styled()
        .rows(
            "A1",
            &[
                "one|two",
                "x|*Mid",
                "|*Off",
                "three|four",
                "five|six",
                "*End",
            ],
        )
        .detect();
    assert!(layout.regions.iter().all(|r| r.heading.is_none()));
}

#[test]
fn section_needs_merge_without_styles() {
    let rows = [
        "Project|Tunnel",
        "Inputs",
        "Length|long",
        "Width|narrow",
        "Outputs",
        "Area|large",
        "Cost|high",
    ];
    let layout = Sheet::new()
        .rows("A1", &rows)
        .merge("A2:B2")
        .merge("A5:B5")
        .detect();
    assert_eq!(ranges(&layout), ["A1:B1", "A2:B4", "A5:B7"]);
    // One row is too few for rule K.
    assert_eq!(layout.regions[0].kind, Kind::Grid);
    assert_eq!(layout.regions[1].heading, Some((1, 1)));
    assert_eq!(layout.regions[2].heading, Some((4, 1)));
    assert_eq!(layout.regions[1].kind, Kind::KeyValue);
    let layout = Sheet::new().rows("A1", &rows).detect();
    let r = only(&layout);
    assert_eq!(r.heading, None);
    assert_eq!(r.kind, Kind::KeyValue);
}

#[test]
fn sections_respect_depth_and_region_limits() {
    let sheet = Sheet::new()
        .rows(
            "A1",
            &["Project|Tunnel", "Inputs", "Length|long", "Width|narrow"],
        )
        .merge("A2:B2");
    let input = sheet.input();
    let limits = Limits {
        max_section_depth: 0,
        ..Limits::default()
    };
    let layout = detect(&input, &limits, &never);
    every_cell_once(&input, &layout);
    assert!(layout.regions.iter().all(|r| r.heading.is_none()));
    let limits = Limits {
        max_section_depth: 1,
        ..Limits::default()
    };
    let layout = detect(&input, &limits, &never);
    assert!(layout.regions.iter().any(|r| r.heading.is_some()));
    let limits = Limits {
        max_regions: 1,
        ..Limits::default()
    };
    let layout = detect(&input, &limits, &never);
    every_cell_once(&input, &layout);
    assert_eq!(layout.regions.len(), 1);
    let issue = layout
        .issues
        .iter()
        .find(|i| i.kind == IssueKind::Limit)
        .expect("a limit issue");
    assert!(issue.what.contains("not split"), "{}", issue.what);
    assert_eq!(issue.rect, None);
}

#[test]
fn all_text_two_columns_without_signal_is_key_value() {
    let layout = Sheet::new()
        .rows("A1", &["Name|City", "Ann|Rome", "Bob|Oslo"])
        .detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::KeyValue);
    assert_eq!(r.certainty, Certainty::Rule);
    assert!(r.unsure.is_empty());
    assert!(r.header_rows.is_empty());
    assert_eq!(r.data, Some(rect("A1:B3")));
    assert_eq!(rule_detail(r, "K"), "label and value pairs");
    // Labels must all differ.
    let layout = Sheet::new()
        .rows("A1", &["Name|City", "Ann|Rome", "Ann|Oslo"])
        .detect();
    assert_ne!(only(&layout).kind, Kind::KeyValue);
    // At least one value: formulas with an empty result are none.
    let layout = Sheet::new()
        .rows("A1", &["Name|=", "Ann|=", "Bob|="])
        .detect();
    let r = only(&layout);
    assert_eq!(r.rect, rect("A1:B3"));
    assert_ne!(r.kind, Kind::KeyValue);
    let layout = Sheet::new()
        .rows("A1", &["Name|=", "Ann|x", "Bob|="])
        .detect();
    assert_eq!(only(&layout).kind, Kind::KeyValue);
}

#[test]
fn all_text_with_bold_first_row_is_table() {
    let layout = Sheet::styled()
        .rows("A1", &["*Name|*City", "Ann|Rome", "Bob|Oslo"])
        .detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(ones(&r.header_rows), [1]);
    assert!(has_rule(r, "H3"));
}

#[test]
fn key_value_with_a_number_reads_as_table() {
    // Limit: H1 decides before rule K, so a number in the value column makes the first row
    // a header.
    let layout = Sheet::new().rows("A1", &["Name:|Ann", "Age:|31"]).detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(ones(&r.header_rows), [1]);
}

#[test]
fn unit_column_keeps_key_value() {
    let sheet = Sheet::new().rows("A1", &["Length|5|m", "Mass|3|kg", "Speed|7|m/s"]);
    let seen = std::cell::RefCell::new(Vec::new());
    let units = |t: &[&str]| {
        seen.borrow_mut().extend(t.iter().map(|s| s.to_string()));
        true
    };
    let layout = sheet.detect_with(&any_label, &units);
    let r = only(&layout);
    assert_eq!(r.kind, Kind::KeyValue);
    assert_eq!(r.data, Some(rect("A1:C3")));
    assert_eq!(rule_detail(r, "K"), "label, value and unit columns");
    assert_eq!(*seen.borrow(), ["m", "kg", "m/s"]);
    let layout = sheet.detect_with(&any_label, &no_unit);
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert!(r.header_rows.is_empty());
    assert_eq!(rule_detail(r, "K"), "column C is not a unit column");
}

#[test]
fn one_column_text_is_text_block() {
    let layout = Sheet::styled()
        .rows("A1", &["*References", "IS 4880", "IS 456", "IRC 6"])
        .detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Text);
    assert_eq!(r.captions, [0]);
    assert_eq!(r.data, Some(rect("A2:A4")));
    assert_eq!(rule_detail(r, "X"), "one column of text");
    // Rows of one merged area each.
    let layout = Sheet::styled()
        .rows("A1", &["*^Codes", "first line", "second line"])
        .merge("A1:C1")
        .merge("A2:C2")
        .merge("A3:C3")
        .detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Text);
    assert_eq!(r.data, Some(rect("A2:A3")));
    // A lone line is text, not a title.
    let layout = Sheet::new().cell("C3", "Note 1").detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Text);
    assert!(r.captions.is_empty());
    assert_eq!(r.data, Some(rect("C3")));
}

#[test]
fn headerless_numbers_are_plain_table() {
    let layout = Sheet::new().rows("B2", &["1|2|3", "4|5|6"]).detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(r.certainty, Certainty::Rule);
    assert!(r.header_rows.is_empty());
    assert!(r.header_paths.is_empty());
    assert!(r.unsure.is_empty());
    assert_eq!(r.data, Some(rect("B2:D3")));
    assert_eq!(rule_detail(r, "H0"), "no header found");
}

#[test]
fn all_text_without_rule_is_grid() {
    let layout = Sheet::new().rows("A1", &["a|b|c|d", "e|f|g|h"]).detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Grid);
    assert_eq!(r.certainty, Certainty::Unsure);
    assert_eq!(r.unsure, ["no header found"]);
}

/// The shape of tunnel_design_calculations.xlsx, sheet Design_Basis.
fn tunnel() -> Sheet {
    let mut s = Sheet::styled()
        .rows(
            "A1",
            &[
                "*UNDERGROUND TUNNEL STRUCTURAL ANALYSIS",
                "Conceptual design calculations - structural stability",
            ],
        )
        .cell("A4", "*^PROJECT DESCRIPTION")
        .merge("A4:C4")
        .rows(
            "A5",
            &[
                "*Project type|Two-lane road tunnel",
                "*Prepared by|Alok",
                "*Location|Hills",
                "*Standard|IS 4880",
                "*Status|Conceptual",
            ],
        )
        .cell("A11", "*^KEY GEOMETRIC PARAMETERS (shared across sheets)")
        .merge("A11:C11")
        .rows(
            "A12",
            &[
                "*Shape|horseshoe",
                "*Lining|concrete",
                "*Drainage|side drains",
                "*Ventilation|jet fans",
                "*Lighting|LED",
            ],
        )
        .cell(
            "A18",
            "*^DESIGN CODES / REFERENCES USED (conceptual guidance)",
        )
        .merge("A18:C18");
    for row in 19..=25 {
        s = s
            .cell(
                &format!("A{row}"),
                &format!("- Code {row} - Code of Practice"),
            )
            .merge(&format!("A{row}:C{row}"));
    }
    s.cell("A27", "*^SUMMARY")
        .merge("A27:C27")
        .rows("A28", &["*Cover|2 m", "*Span|11 m"])
}

#[test]
fn tunnel_design_basis_walkthrough() {
    let layout = tunnel().detect();
    assert_eq!(
        ranges(&layout),
        ["A1:A2", "A4:C9", "A11:C16", "A18:C25", "A27:C29"]
    );
    let head = &layout.regions[0];
    assert_eq!(head.kind, Kind::Text);
    assert_eq!(head.captions, [0]);
    assert_eq!(head.data, Some(rect("A2")));
    let kv = &layout.regions[1];
    assert_eq!(kv.kind, Kind::KeyValue);
    assert_eq!(kv.captions, [3]);
    assert_eq!(kv.data, Some(rect("A5:B9")));
    assert_eq!(layout.regions[2].kind, Kind::KeyValue);
    let text = &layout.regions[3];
    assert_eq!(text.kind, Kind::Text);
    assert_eq!(text.captions, [17]);
    assert_eq!(text.data, Some(rect("A19:A25")));
    assert!(text.notes.is_empty());
    assert_eq!(layout.regions[4].kind, Kind::KeyValue);
}

#[test]
fn note_between_tables_titles_the_next() {
    // Limit: rule T2 (step 4) runs before rule N (step 8). A note that stands within 2 rows
    // above another table becomes that table's title.
    let layout = small_table()
        .cell("A5", "Source: survey")
        .rows("A7", &["Name|Size|Mass", "c|5|6"])
        .detect();
    assert_eq!(ranges(&layout), ["A1:C3", "A5:C8"]);
    assert!(layout.regions[0].notes.is_empty());
    assert_eq!(ones(&layout.regions[1].captions), [5]);
    // When the next table has a title of its own, the line stays a title region, and rule N
    // makes it a note of the table above.
    let layout = small_table()
        .cell("A5", "Source: survey")
        .rows("A7", &["Second table", "Name|Size|Mass", "c|5|6"])
        .detect();
    assert_eq!(ranges(&layout), ["A1:C5", "A7:C9"]);
    assert_eq!(ones(&layout.regions[0].notes), [5]);
}

#[test]
fn stop_during_report_rules_keeps_every_cell() {
    for sheet in [census(false), eia(), tunnel()] {
        let input = sheet.input();
        for after in 0..40 {
            let calls = Cell::new(0u32);
            let stop = || {
                calls.set(calls.get() + 1);
                calls.get() > after
            };
            let layout = detect(&input, &Limits::default(), &stop);
            every_cell_once(&input, &layout);
            no_overlap(&layout);
            let stops = layout
                .issues
                .iter()
                .filter(|i| i.kind == IssueKind::Stopped)
                .count();
            assert!(stops <= 1);
        }
    }
}

fn fixtures() -> Vec<Sheet> {
    vec![
        small_table()
            .rows("A5", &["Source: survey", "Values in kg"])
            .rows("E1", &["side note", "more"])
            .cell("A10", "far away"),
        small_table().rows("A5", &["c|5|6"]),
        small_table().rows("A5", &["c|5"]),
        census(false),
        census(true),
        eia(),
        tunnel(),
        Sheet::new().rows("A1", &["Name|Ann", "City|Rome", "Job|Cook"]),
        Sheet::styled().rows(
            "A1",
            &[
                "*^Design report",
                "*Inputs",
                "Length|5",
                "Width|3",
                "*Outputs",
                "Area|15",
            ],
        ),
        Sheet::new()
            .rows(
                "A1",
                &[
                    "Project|Tunnel",
                    "Inputs",
                    "Length|long",
                    "Outputs",
                    "Area|large",
                ],
            )
            .merge("A2:B2")
            .merge("A4:B4"),
        Sheet::new().rows("A1", &["Length|5|m", "Mass|3|kg"]),
        Sheet::new().rows("B2", &["1|2|3", "4|5|6"]),
        Sheet::new().rows("A1", &["Name|Length|Mass", "|cm|", "a|1|2"]),
        Sheet::styled().rows("A1", &["*Item|*Value", "Assets|10", "~Cash|4", "..Petty|1"]),
    ]
}

#[test]
fn every_cell_in_exactly_one_region() {
    for (i, sheet) in fixtures().iter().enumerate() {
        let input = sheet.input();
        let layout = detect(&input, &Limits::default(), &never);
        every_cell_once(&input, &layout);
        no_overlap(&layout);
        // Header rows count: a cell in a header row is held by that row.
        for r in &layout.regions {
            for h in &r.header_rows {
                assert!(r.rect.top <= *h && *h <= r.rect.bottom, "fixture {i}");
                assert!(r.data.is_none_or(|d| !(d.top <= *h && *h <= d.bottom)));
            }
        }
    }
}

// Fix round 1.

/// A two-group statement: group rows, indented items, SUM totals per group and a SUM over
/// the two group totals.
fn statement() -> Sheet {
    Sheet::styled()
        .rows(
            "A1",
            &[
                "*Statement",
                "*Item|*2024|*2025",
                "Revenue",
                "~Sales|10|11",
                "~Fees|2|3",
                "Total revenue|=12|=14",
                "Expenses",
                "~Wages|5|6",
                "~Rent|1|1",
                "Total expenses|=6|=7",
                "Net income|=6|=7",
            ],
        )
        .formula("B6", &["SUM"], &["B4:B5"])
        .formula("C6", &["SUM"], &["C4:C5"])
        .formula("B10", &["SUM"], &["B8:B9"])
        .formula("C10", &["SUM"], &["C8:C9"])
        .formula("B11", &["SUM"], &["B6", "B10"])
        .formula("C11", &["SUM"], &["C6", "C10"])
}

#[test]
fn group_rows_are_parents() {
    let layout = statement().detect();
    let r = only(&layout);
    assert_eq!(ones(&r.header_rows), [2]);
    assert_eq!(rows_of(r, RowKind::Group), [3, 7]);
    for (row, parent) in [(4, 3), (5, 3), (8, 7), (9, 7)] {
        assert_eq!(note(r, row).unwrap().parent, Some(parent - 1), "row {row}");
    }
    // Group rows get no level of their own.
    assert_eq!(note(r, 3).unwrap().level, None);
    assert_eq!(note(r, 6).unwrap().parent, None);
}

#[test]
fn totals_across_groups() {
    let layout = statement().detect();
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Subtotal), [6, 10]);
    assert_eq!(rows_of(r, RowKind::Total), [11]);
    // SUBTOTAL in one column and a total SUM in another: the row is a total.
    let layout = Sheet::new()
        .rows("A1", &["Item|V|W", "a|1|2", "b|3|4", "S|=4|=6"])
        .formula("B4", &["SUBTOTAL"], &["B2:B3"])
        .formula("C4", &["SUM"], &["C2:C3"])
        .detect();
    assert_eq!(rows_of(only(&layout), RowKind::Total), [4]);
}

#[test]
fn running_balance_is_not_a_total() {
    let layout = Sheet::new()
        .rows(
            "A1",
            &["Item|In|Balance", "a|5|5", "b|3|=8", "c|2|=10", "d|1|=11"],
        )
        .formula("C3", &["SUM"], &["C2", "B3"])
        .formula("C4", &["SUM"], &["C3", "B4"])
        .formula("C5", &["SUM"], &["C4:C4", "B5:B5"])
        .detect();
    let r = only(&layout);
    assert!(r.rows.is_empty(), "{:?}", r.rows);
    // A SUM over one cell in the middle of a block is no total either.
    let layout = Sheet::new()
        .rows("A1", &["Item|V", "a|1", "b|2", "c|=2"])
        .formula("B4", &["SUM"], &["B3"])
        .detect();
    assert!(only(&layout).rows.is_empty());
}

#[test]
fn headerless_table_gets_row_rules() {
    let layout = Sheet::new()
        .rows("A1", &["a|1|2", "b|3|4", "S|=4|=6", "Source: x"])
        .formula("B3", &["SUBTOTAL"], &["B1:B2"])
        .merge("A4:C4")
        .detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert!(has_rule(r, "H0"));
    assert_eq!(rows_of(r, RowKind::Subtotal), [3]);
    assert_eq!(ones(&r.notes), [4]);
    assert_eq!(r.data, Some(rect("A1:C3")));
}

fn typed(sheet: Sheet, class: sheetlayout::Class, text: &str, cells: &[&str]) -> Sheet {
    let mut sheet = sheet;
    for at in cells {
        sheet.cells.push(sheetlayout::CellIn {
            pos: common::pos(at),
            class,
            text: text.to_string(),
            number: None,
            style: sheetlayout::Style::default(),
            formula: None,
        });
    }
    sheet
}

#[test]
fn values_without_header_are_tables() {
    // One column of numbers: a table, not a text block.
    let layout = Sheet::new().rows("A1", &["1", "2", "3"]).detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert!(has_rule(r, "H0"));
    // Dates only and bools only.
    let layout = typed(
        Sheet::new(),
        sheetlayout::Class::Date,
        "2024-01-01",
        &["A1", "B1", "A2", "B2"],
    )
    .detect();
    assert!(has_rule(only(&layout), "H0"));
    let layout = Sheet::new()
        .rows("A1", &["TRUE|FALSE", "FALSE|TRUE"])
        .detect();
    assert!(has_rule(only(&layout), "H0"));
}

#[test]
fn two_stated_totals_rows() {
    let layout = Sheet::new()
        .rows("A1", &["Item|Value", "a|1", "b|2", "Sum|3", "Max|2"])
        .fact(
            "A1:B5",
            FactKind::ListObject {
                name: "T".into(),
                header_rows: 1,
                totals_rows: 2,
                columns: Vec::new(),
            },
        )
        .detect();
    assert_eq!(rows_of(only(&layout), RowKind::Total), [4, 5]);
}

#[test]
fn regions_below_that_are_not_joined() {
    // A lower part with a caption of its own is not a continuation.
    let layout = small_table().rows("A5", &["Part two", "c|5|6"]).detect();
    assert_eq!(ranges(&layout), ["A1:C3", "A5:C6"]);
    // A label and value block one row below is no note.
    let layout = small_table()
        .rows("A5", &["Unit|kg", "Source|survey"])
        .detect();
    assert_eq!(ranges(&layout), ["A1:C3", "A5:B6"]);
    assert_eq!(layout.regions[1].kind, Kind::KeyValue);
    assert!(layout.regions[0].notes.is_empty());
    // Notes as wide as the table, one row below, are joined by rule C, and they are said
    // to be notes, not a continuation.
    let layout = small_table()
        .cell("A5", "Source: x")
        .merge("A5:C5")
        .detect();
    let r = only(&layout);
    assert_eq!(ones(&r.notes), [5]);
    assert!(!has_rule(r, "C"));
    assert_eq!(rule_detail(r, "N"), "notes in row 5");
    assert!(r.rows.is_empty(), "{:?}", r.rows);
}

#[test]
fn section_heading_width_inside_the_merge() {
    // The heading text sits inside the merge, not in the first column: the merge must cover
    // 80 % of the width.
    let rows = ["a|b|c|d|e", "|Inputs|||", "f|g|h|i|j", "k|l|m|n|o"];
    let layout = Sheet::new().rows("A1", &rows).merge("A2:D2").detect();
    assert!(layout.regions.iter().any(|r| r.heading == Some((1, 1))));
    let layout = Sheet::new().rows("A1", &rows).merge("A2:C2").detect();
    assert!(layout.regions.iter().all(|r| r.heading.is_none()));
}

#[test]
fn sum_kinds_by_reach() {
    // A SUM that no later total reads but that does not start at the first data row is a
    // subtotal.
    let layout = Sheet::new()
        .rows("A1", &["Item|V", "a|1", "b|2", "c|3", "d|4", "S|=7"])
        .formula("B6", &["SUM"], &["B4:B5"])
        .detect();
    assert_eq!(rows_of(only(&layout), RowKind::Subtotal), [6]);
    // A SUM over the one item after a group row closes that block.
    let layout = Sheet::new()
        .rows("A1", &["Item|V", "a|1", "Fruit|", "b|2", "S|=2"])
        .formula("B5", &["SUM"], &["B4"])
        .detect();
    assert_eq!(rows_of(only(&layout), RowKind::Subtotal), [5]);
    // A SUM over two one-cell totals reaches the first data row through the first of them.
    let layout = Sheet::new()
        .rows(
            "A1",
            &[
                "Item|V", "a|1", "b|2", "S|=3", "c|4", "d|5", "T|=9", "All|=12",
            ],
        )
        .formula("B4", &["SUM"], &["B2:B3"])
        .formula("B7", &["SUM"], &["B5:B6"])
        .formula("B8", &["SUM"], &["B7", "B4"])
        .detect();
    let r = only(&layout);
    assert_eq!(rows_of(r, RowKind::Subtotal), [4, 7]);
    assert_eq!(rows_of(r, RowKind::Total), [8]);
    // One-cell reads in the middle of a block mark nothing.
    let layout = Sheet::new()
        .rows("A1", &["Item|V", "a|1", "b|2", "c|4", "T|=6"])
        .formula("B5", &["SUM"], &["B3", "B4"])
        .detect();
    assert!(only(&layout).rows.is_empty());
}

#[test]
fn headerless_wide_table_gets_row_rules() {
    // Four columns: no label and value block, so step 6g decides.
    let layout = Sheet::new()
        .rows("A1", &["a|1|2|3", "b|3|4|5", "S|=4|=6|=8", "Source: x"])
        .formula("B3", &["SUBTOTAL"], &["B1:B2"])
        .merge("A4:D4")
        .detect();
    let r = only(&layout);
    assert_eq!(rule_detail(r, "H0"), "no header found");
    assert!(!has_rule(r, "K"));
    assert_eq!(rows_of(r, RowKind::Subtotal), [3]);
    assert_eq!(ones(&r.notes), [4]);
}
