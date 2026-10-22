// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Header bands, header paths and label columns.

mod common;

use std::cell::Cell;

use common::{Sheet, every_cell_once, list_object, no_overlap, rect};
use sheetlayout::{
    Certainty, Evidence, FactKind, IssueKind, Kind, Layout, Limits, Pos, Rect, Region, detect,
};

fn never() -> bool {
    false
}

fn only(layout: &Layout) -> &Region {
    assert_eq!(layout.regions.len(), 1, "{:?}", layout.regions);
    &layout.regions[0]
}

/// 1-based header rows.
fn header(r: &Region) -> Vec<u32> {
    r.header_rows.iter().map(|&x| x + 1).collect()
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

fn no_header(r: &Region) {
    assert_eq!(r.kind, Kind::Grid);
    assert_eq!(r.certainty, Certainty::Unsure);
    assert!(r.header_rows.is_empty());
    assert!(r.header_paths.is_empty());
    assert_eq!(r.unsure, ["no header found"]);
}

/// The EIA table 1.1 shape: captions in rows 1 and 2, a band in rows 3 and 4, a group row 5
/// and numbers from row 6.
fn eia(styled: bool) -> Sheet {
    let mark = if styled { "*^" } else { "" };
    let rows: Vec<String> = [
        "Table 1.1. Net Generation by Energy Source",
        "(Thousand Megawatthours)",
        "Period|Generation at Utility Scale Facilities|||Total",
        "|Coal|Natural Gas|Nuclear|",
    ]
    .iter()
    .map(|line| {
        line.split('|')
            .map(|t| {
                if t.is_empty() {
                    String::new()
                } else {
                    format!("{mark}{t}")
                }
            })
            .collect::<Vec<_>>()
            .join("|")
    })
    .collect();
    let mut all: Vec<&str> = rows.iter().map(String::as_str).collect();
    all.extend([
        "Annual Totals",
        "2013|1581115|1124836|789016|4065964",
        "2014|1581710|1126609|797166|4093564",
        "2015|1352398|1333482|797178|4077601",
    ]);
    let base = if styled {
        Sheet::styled()
    } else {
        Sheet::new()
    };
    base.rows("A1", &all)
        .merge("A3:A4")
        .merge("B3:D3")
        .merge("E3:E4")
}

#[test]
fn header_by_type_change() {
    let layout = eia(false).detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(r.certainty, Certainty::Rule);
    assert_eq!(r.captions, [0, 1]);
    assert_eq!(header(r), [3, 4]);
    assert_eq!(r.data, Some(rect("A5:E8")));
    assert!(r.unsure.is_empty());
    assert!(has_rule(r, "H1"));
    assert!(!has_rule(r, "H2"), "no styles, no H2");
    // With styles, H2 agrees: rows 3 and 4 are bold, row 5 is skipped, row 6 is plain.
    let layout = eia(true).detect();
    let r = only(&layout);
    assert_eq!(r.certainty, Certainty::Rule);
    assert_eq!(r.captions, [0, 1]);
    assert_eq!(header(r), [3, 4]);
    assert!(has_rule(r, "H1"));
    assert!(has_rule(r, "H2"));
}

#[test]
fn no_data_row_after_the_band_is_no_band() {
    // Five rows of text: more than 4 header rows, and no type change.
    let layout = Sheet::new()
        .rows("A1", &["a|b", "c|d", "e|f", "g|h", "i|j", "1|2"])
        .detect();
    no_header(only(&layout));
    // A bool is a type change.
    let layout = Sheet::new().rows("A1", &["a|b", "c|TRUE", "1|2"]).detect();
    assert_eq!(header(only(&layout)), [1]);
    // A text row with a merge that does not hold its text is no single-text row: it belongs
    // to the band.
    let layout = Sheet::new()
        .rows("A1", &["a|b|c", "t||", "1|2|3"])
        .merge("B2:C2")
        .detect();
    assert_eq!(header(only(&layout)), [1, 2]);
    // A formula row counts as a type change.
    let layout = Sheet::new()
        .rows("A1", &["Name|Label", "x|=y", "z|=w"])
        .detect();
    assert_eq!(header(only(&layout)), [1]);
}

/// The Census NST-EST2023-POP shape: captions rows 1 and 2, a band with years in row 4.
fn census(merged: bool) -> Sheet {
    let sheet = Sheet::styled().rows(
        "A1",
        &[
            "^Annotation: the table is laid out for screen readers",
            "*^Annual Estimates of the Resident Population",
            "*^Geographic Area|*^April 1, 2020 Estimates Base|*^Population Estimate (as of July 1)|||",
            "||*^2020|*^2021|*^2022|*^2023",
            "*United States|331449520|331511512|332031554|333287557|334914895",
            "Alabama|5024356|5031362|5049846|5074296|5108468",
            "Alaska|733395|732923|734182|733583|733406",
        ],
    );
    if merged {
        sheet.merge("A3:A4").merge("B3:B4").merge("C3:F3")
    } else {
        sheet
    }
}

#[test]
fn year_row_is_header() {
    let layout = census(true).detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(r.certainty, Certainty::Rule);
    assert_eq!(r.captions, [0, 1]);
    assert_eq!(r.caption_heading, Some(1));
    assert_eq!(header(r), [3, 4]);
    assert_eq!(r.header_paths[0], "Geographic Area");
    assert_eq!(r.header_paths[1], "April 1, 2020 Estimates Base");
    assert_eq!(
        r.header_paths[3],
        "Population Estimate (as of July 1) / 2021"
    );
    assert!(has_rule(r, "H2"));
    // The years alone make row 4 a header row, without the merges.
    let layout = census(false).detect();
    let r = only(&layout);
    assert_eq!(header(r), [3, 4]);
    assert_eq!(r.header_paths[3], "2021");
    assert_eq!(
        r.header_paths[2],
        "Population Estimate (as of July 1) / 2020"
    );
}

#[test]
fn year_in_data_column_is_not_header() {
    let layout = Sheet::new()
        .rows(
            "A1",
            &["Name|Start|End|Cost", "a|2021|2022|17.5", "b|2020|2023|3"],
        )
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1]);
    assert_eq!(r.data, Some(rect("A2:D3")));
    // 1899 and 2101 are no years: a row with one of them is data.
    for row in ["a|1899", "a|2101"] {
        let layout = Sheet::new().rows("A1", &["From|To", row, "1|2"]).detect();
        assert_eq!(header(only(&layout)), [1], "{row}");
    }
    // 1900 and 2100 are years: the row is a header row.
    let layout = Sheet::new()
        .rows("A1", &["From|To", "1900|2100", "1|2"])
        .detect();
    assert_eq!(header(only(&layout)), [1, 2]);
    // A year with a fraction is no year.
    let layout = Sheet::new()
        .rows("A1", &["From|To", "2000.5|2001", "1|2"])
        .detect();
    assert_eq!(header(only(&layout)), [1]);
}

#[test]
fn vertical_merge_extends_band() {
    // Row 2 holds nothing; the merges from row 1 extend the band over it.
    let layout = Sheet::new()
        .rows("A1", &["Region|Total", "", "North|5", "South|7"])
        .merge("A1:A2")
        .merge("B1:B2")
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1, 2]);
    assert_eq!(r.data, Some(rect("A3:B4")));
    assert_eq!(r.header_paths, ["Region", "Total"]);
    // A merge that would make the band longer than 4 rows gives no band.
    let layout = Sheet::new()
        .rows(
            "A1",
            &["Region|Total", "", "", "", "", "North|5", "South|7"],
        )
        .merge("A1:A5")
        .detect();
    no_header(only(&layout));
}

#[test]
fn header_and_style_disagree_is_unsure() {
    let layout = Sheet::styled()
        .rows("A1", &["*a|*b", "*c|*d", "*e|*f", "g|h", "1|2", "3|4"])
        .detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Grid);
    assert_eq!(r.certainty, Certainty::Unsure);
    assert!(r.header_rows.is_empty());
    assert_eq!(
        r.unsure,
        ["values change type at row 5 but bold ends at row 3"]
    );
    // The same with fill.
    let layout = Sheet::styled()
        .rows("A1", &["^a|^b", "c|d", "1|2"])
        .detect();
    assert_eq!(
        only(&layout).unsure,
        ["values change type at row 3 but fill ends at row 1"]
    );
}

#[test]
fn fill_everywhere_gives_h2_no_answer() {
    // ex051: every cell is filled. H2 has no answer, which is no disagreement.
    let layout = Sheet::styled()
        .rows("A1", &["^Name|^Value", "^a|^1", "^b|^2"])
        .detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(r.certainty, Certainty::Rule);
    assert_eq!(header(r), [1]);
    assert!(!has_rule(r, "H2"));
    // No row marked: no answer either.
    let layout = Sheet::styled()
        .rows("A1", &["Name|Value", "a|1", "b|2"])
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1]);
    assert!(!has_rule(r, "H2"));
    // More than 4 marked rows: no answer.
    let layout = Sheet::styled()
        .rows("A1", &["*a|*b", "*c|*1", "*d|*2", "*e|*3", "*f|*4", "g|5"])
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1]);
    assert_eq!(r.certainty, Certainty::Rule);
}

#[test]
fn unknown_style_gives_h2_no_answer() {
    // Styles are read, but one header cell has no known bold or fill.
    let mut sheet = Sheet::styled().rows("A1", &["*a|*b", "*c|*d", "1|2"]);
    sheet.cells[1].style.bold = None;
    sheet.cells[1].style.filled = None;
    let layout = sheet.detect();
    let r = only(&layout);
    assert_eq!(header(r), [1, 2]);
    assert_eq!(r.certainty, Certainty::Rule);
    // An unknown flag on a data row: H2 has no answer, so it cannot disagree.
    let mut sheet = Sheet::styled().rows("A1", &["*a|*b", "1|2", "3|4"]);
    for c in &mut sheet.cells[2..4] {
        c.style.bold = None;
        c.style.filled = None;
    }
    let layout = sheet.detect();
    let r = only(&layout);
    assert_eq!(header(r), [1]);
    assert_eq!(r.certainty, Certainty::Rule);
    assert!(!has_rule(r, "H2"));
}

#[test]
fn all_text_region_header_rule() {
    let rows = ["Name|City", "Ann|Rome", "Bob|Oslo"];
    let layout = Sheet::styled()
        .rows("A1", &["*Name|*City", "Ann|Rome", "Bob|Oslo"])
        .detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(r.certainty, Certainty::Rule);
    assert_eq!(header(r), [1]);
    assert_eq!(r.header_paths, ["Name", "City"]);
    assert!(has_rule(r, "H3"));
    // Without styles there is no positive signal: no band.
    no_header(only(&Sheet::new().rows("A1", &rows).detect()));
    // With styles but no mark: no band.
    no_header(only(&Sheet::styled().rows("A1", &rows).detect()));
    // A header guess for the first row is a positive signal.
    let mut sheet = Sheet::new().rows("A1", &rows);
    sheet.header_guess = Some((0, true));
    assert_eq!(header(only(&sheet.detect())), [1]);
    sheet.header_guess = Some((0, false));
    no_header(only(&sheet.detect()));
    // A header text that repeats below in its column: no band.
    let layout = Sheet::styled()
        .rows("A1", &["*Name|*City", "Ann|Rome", "Bob|City"])
        .detect();
    no_header(only(&layout));
    // Two equal header texts: no band.
    let layout = Sheet::styled()
        .rows("A1", &["*Name|*Name", "Ann|Rome", "Bob|Oslo"])
        .detect();
    no_header(only(&layout));
    // An empty header cell: no band.
    let layout = Sheet::styled()
        .rows("A1", &["*Name||*City", "Ann|x|Rome", "Bob|y|Oslo"])
        .detect();
    no_header(only(&layout));
    // H2 marks two rows: not exactly row 1, no band.
    let layout = Sheet::styled()
        .rows("A1", &["*Name|*City", "*Ann|*Rome", "Bob|Oslo"])
        .detect();
    no_header(only(&layout));
}

#[test]
fn unmerged_group_label_is_not_spread() {
    let layout = Sheet::new()
        .rows("A1", &["Item|Size|", "|Small|Large", "x|1|2"])
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1, 2]);
    assert_eq!(r.header_paths, ["Item", "Size / Small", "Large"]);
}

#[test]
fn merged_group_label_spreads() {
    let layout = eia(false).detect();
    let r = only(&layout);
    assert_eq!(
        r.header_paths,
        [
            "Period",
            "Generation at Utility Scale Facilities / Coal",
            "Generation at Utility Scale Facilities / Natural Gas",
            "Generation at Utility Scale Facilities / Nuclear",
            "Total",
        ]
    );
}

#[test]
fn center_across_spreads() {
    // B1 is centered across B1:C1; D1 is empty without the alignment, so it stops there.
    let layout = Sheet::styled()
        .rows(
            "A1",
            &["Item|>Size|>||", "|Small|Large|Other|Rest", "x|1|2|3|4"],
        )
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1, 2]);
    assert_eq!(
        r.header_paths,
        ["Item", "Size / Small", "Size / Large", "Other", "Rest"]
    );
    // A centered text never spreads over a cell with content.
    let layout = Sheet::styled()
        .rows("A1", &["Item|>Size|Other", "|Small|Large", "x|1|2"])
        .detect();
    assert_eq!(
        only(&layout).header_paths,
        ["Item", "Size / Small", "Other / Large"]
    );
}

#[test]
fn vertical_merge_counts_once_in_path() {
    // The merged_range shape: C1:D2 holds "C1".
    let layout = Sheet::new()
        .rows("A1", &["A|B|C1|", "a|b||", "1|2|3|4"])
        .merge("C1:D2")
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1, 2]);
    assert_eq!(r.header_paths, ["A / a", "B / b", "C1", "C1"]);
    // Two equal neighbours in a path count once; a part that holds " / " is kept.
    let layout = Sheet::new()
        .rows("A1", &["Area|km / h", "Area|x", "1|2"])
        .detect();
    assert_eq!(only(&layout).header_paths, ["Area", "km / h / x"]);
}

#[test]
fn empty_path_stays_empty() {
    let layout = Sheet::new()
        .rows("A1", &["Name||Value", "a|b|1", "c|d|2"])
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1]);
    assert_eq!(r.header_paths, ["Name", "", "Value"]);
    // A merge anchored below the band gives no header text.
    let layout = Sheet::new()
        .rows("A1", &["Name||Value", "a|b|1", "c|d|2", "n||"])
        .merge("A4:C4")
        .detect();
    assert_eq!(only(&layout).header_paths, ["Name", "", "Value"]);
}

#[test]
fn print_titles_win_and_are_exact() {
    let mut sheet = Sheet::new().rows("A1", &["Name|Value", "a|1", "b|2", "c|3"]);
    sheet.print_titles = Some((0, 1));
    let layout = sheet.detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(r.certainty, Certainty::Exact);
    assert_eq!(header(r), [1, 2]);
    assert_eq!(r.header_paths, ["Name / a", "Value / 1"]);
    assert!(r.evidence.contains(&Evidence::PrintTitles));
    assert_eq!(
        rule_detail(r, "H1"),
        "header row 1 by type change; the print titles name rows 1 to 2 and win"
    );
    // Print titles that agree: exact, no disagreement.
    sheet.print_titles = Some((0, 0));
    let layout = sheet.detect();
    let r = only(&layout);
    assert_eq!(r.certainty, Certainty::Exact);
    assert_eq!(header(r), [1]);
    assert!(!rule_detail(r, "H1").contains("win"));
    // They win over an unsure header too.
    let mut sheet = Sheet::styled().rows("A1", &["*a|*b", "c|d", "1|2", "3|4"]);
    sheet.print_titles = Some((0, 1));
    let layout = sheet.detect();
    let r = only(&layout);
    assert_eq!(r.certainty, Certainty::Exact);
    assert_eq!(header(r), [1, 2]);
    assert!(r.unsure.is_empty());
    // Print titles that do not start at the region's first row after its captions are not
    // used, and are no evidence.
    let mut sheet = Sheet::new().rows("A1", &["Name|Value", "a|1", "b|2", "c|3"]);
    sheet.print_titles = Some((1, 2));
    let layout = sheet.detect();
    let r = only(&layout);
    assert_eq!(r.certainty, Certainty::Rule);
    assert_eq!(header(r), [1]);
    assert!(!r.evidence.contains(&Evidence::PrintTitles));
}

#[test]
fn print_titles_leave_data_and_hold_at_most_10_rows() {
    // Print titles over every row of the region would leave no data: not used.
    let mut sheet = Sheet::new().rows("A1", &["Name|Value", "a|1", "b|2", "c|3"]);
    sheet.print_titles = Some((0, 3));
    let r = sheet.detect().regions.remove(0);
    assert_eq!(r.certainty, Certainty::Rule);
    assert_eq!(header(&r), [1]);
    assert!(!r.evidence.contains(&Evidence::PrintTitles));
    // 10 rows are a band; 11 are not.
    let rows: Vec<String> = (0..12).map(|k| format!("t{k}|u{k}")).collect();
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    let mut sheet = Sheet::new().rows("A1", &rows).rows("A13", &["1|2"]);
    sheet.print_titles = Some((0, 9));
    let r = sheet.detect().regions.remove(0);
    assert_eq!(r.certainty, Certainty::Exact);
    assert_eq!(r.header_rows.len(), 10);
    sheet.print_titles = Some((0, 10));
    let r = sheet.detect().regions.remove(0);
    assert_eq!(r.kind, Kind::Grid);
    assert_eq!(r.unsure, ["no header found"]);
}

#[test]
fn frozen_panes_confirm_only() {
    let base = Sheet::new().rows("A1", &["Name|Value", "a|1", "b|2"]);
    let mut sheet = base.clone();
    sheet.frozen = Some((1, 0));
    let r = sheet.detect().regions.remove(0);
    assert_eq!(r.certainty, Certainty::Rule);
    assert!(r.evidence.contains(&Evidence::FrozenPanes));
    // A split that is not at the band's end confirms nothing.
    let mut sheet = base.clone();
    sheet.frozen = Some((2, 0));
    let r = sheet.detect().regions.remove(0);
    assert_eq!(header(&r), [1]);
    assert!(!r.evidence.contains(&Evidence::FrozenPanes));
    // Panes never decide an unsure header.
    let mut sheet = Sheet::styled().rows("A1", &["*a|*b", "c|d", "1|2"]);
    sheet.frozen = Some((1, 0));
    let r = sheet.detect().regions.remove(0);
    assert_eq!(r.certainty, Certainty::Unsure);
}

#[test]
fn label_columns_by_text_share() {
    // B holds text in 4 of 5 data rows: 80 %.
    let rows = [
        "Region|State|2020|2021",
        "North|Maine|1|2",
        "North||3|4",
        "South|Texas|5|6",
        "South|Ohio|7|8",
        "West|Utah|9|10",
    ];
    let layout = Sheet::new().rows("A1", &rows).detect();
    let r = only(&layout);
    assert_eq!(header(r), [1]);
    assert_eq!(r.label_cols, 2);
    assert_eq!(rule_detail(r, "L"), "label columns A to B");
    // B holds text in 3 of 5 rows: only A.
    let mut fewer = rows;
    fewer[3] = "South||5|6";
    let r = Sheet::new().rows("A1", &fewer).detect().regions.remove(0);
    assert_eq!(r.label_cols, 1);
    assert_eq!(rule_detail(&r, "L"), "label column A");
    // A frozen column split at the label columns confirms them.
    let mut sheet = Sheet::new().rows("A1", &rows);
    sheet.frozen = Some((0, 2));
    let r = sheet.detect().regions.remove(0);
    assert!(r.evidence.contains(&Evidence::FrozenPanes));
    // A number header cell: no label column.
    let r = Sheet::new()
        .rows("A1", &["2020|Value", "a|1", "b|2"])
        .detect()
        .regions
        .remove(0);
    assert_eq!(header(&r), [1]);
    assert_eq!(r.label_cols, 0);
    // At most 3, and never every column.
    let r = Sheet::new()
        .rows("A1", &["a|b|c|d|e", "x|y|z|w|1", "x|y|z|w|2"])
        .detect()
        .regions
        .remove(0);
    assert_eq!(r.label_cols, 3);
    let r = Sheet::styled()
        .rows("A1", &["*a|*b", "x|y", "z|w"])
        .detect()
        .regions
        .remove(0);
    assert_eq!(header(&r), [1]);
    assert_eq!(r.label_cols, 1);
}

#[test]
fn list_object_without_header_row() {
    let sheet = Sheet::new()
        .rows("B2", &["Ann|31|Rome", "Bob|42|Oslo"])
        .fact(
            "B2:D3",
            FactKind::ListObject {
                name: "People".into(),
                header_rows: 0,
                totals_rows: 0,
                columns: vec!["Name".into(), "Age".into(), "City".into()],
            },
        );
    let layout = sheet.detect();
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(r.certainty, Certainty::Exact);
    assert!(r.hidden_header);
    assert!(r.header_rows.is_empty());
    assert_eq!(r.header_paths, ["Name", "Age", "City"]);
    assert_eq!(r.label_cols, 1);
    // Fewer names than columns: the rest stay empty.
    let sheet = Sheet::new().rows("B2", &["Ann|31|Rome"]).fact(
        "B2:D2",
        FactKind::ListObject {
            name: "People".into(),
            header_rows: 0,
            totals_rows: 0,
            columns: vec!["Name".into()],
        },
    );
    assert_eq!(only(&sheet.detect()).header_paths, ["Name", "", ""]);
}

#[test]
fn fact_headers_give_paths() {
    // An Excel table with its header row.
    let layout = Sheet::new()
        .rows("A1", &["Name|Age", "Ann|31", "Bob|42"])
        .fact("A1:B3", list_object("T"))
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1]);
    assert_eq!(r.header_paths, ["Name", "Age"]);
    assert_eq!(r.label_cols, 1);
    assert!(!r.hidden_header);
    // Print titles on the same row stay evidence; on other rows they do not.
    let mut sheet = Sheet::new()
        .rows("A1", &["Name|Age", "Ann|31", "Bob|42"])
        .fact("A1:B3", list_object("T"));
    sheet.print_titles = Some((0, 0));
    assert!(
        only(&sheet.detect())
            .evidence
            .contains(&Evidence::PrintTitles)
    );
    sheet.print_titles = Some((0, 1));
    assert!(
        !only(&sheet.detect())
            .evidence
            .contains(&Evidence::PrintTitles)
    );
    // A database range without a header: no paths.
    let layout = Sheet::new()
        .rows("A1", &["Ann|31", "Bob|42"])
        .fact(
            "A1:B2",
            FactKind::DatabaseRange {
                name: "D".into(),
                header: false,
                footer: false,
            },
        )
        .detect();
    let r = only(&layout);
    assert!(r.header_rows.is_empty());
    assert!(r.header_paths.is_empty());
    // A pivot: rows above the first data row are the band, label columns from the fact.
    let layout = Sheet::new()
        .rows(
            "A1",
            &["Sum of Sales|Columns|", "Rows|East|West", "a|1|2", "b|3|4"],
        )
        .merge("B1:C1")
        .fact(
            "A1:C4",
            FactKind::Pivot {
                name: "P".into(),
                first_header_row: 1,
                first_data_row: 2,
                first_data_col: 1,
                total_row: false,
                total_col: false,
            },
        )
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1, 2]);
    assert_eq!(
        r.header_paths,
        ["Sum of Sales / Rows", "Columns / East", "Columns / West"]
    );
    assert_eq!(r.label_cols, 1);
    // An autofilter: the first row.
    let layout = Sheet::new()
        .rows("A1", &["Name|Age", "Ann|Rome"])
        .fact("A1:B2", FactKind::AutoFilter)
        .detect();
    assert_eq!(only(&layout).header_paths, ["Name", "Age"]);
}

#[test]
fn path_text_is_bounded() {
    let sheet = Sheet::new()
        .rows("A1", &["Head|||||||Tail", "1|2|3|4|5|6|7|8"])
        .merge("A1:G1");
    let input = sheet.input();
    let limits = Limits {
        max_path_bytes: 100,
        ..Limits::default()
    };
    let layout = detect(&input, &limits, &never);
    every_cell_once(&input, &layout);
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Grid);
    assert!(r.rest);
    assert!(r.header_paths.is_empty());
    assert_eq!(layout.issues.len(), 1);
    assert_eq!(layout.issues[0].kind, IssueKind::Limit);
    assert_eq!(layout.issues[0].rect, Some(r.rect));
    assert!(layout.issues[0].what.contains("header path limit"));
    // Enough room: the paths are there.
    let limits = Limits {
        max_path_bytes: 1_000,
        ..Limits::default()
    };
    let layout = detect(&input, &limits, &never);
    let mut want = vec!["Head"; 7];
    want.push("Tail");
    assert_eq!(only(&layout).header_paths, want);
}

#[test]
fn rest_regions_are_not_decided() {
    let sheet = Sheet::new()
        .rows("A1", &["a|b", "1|2"])
        .rows("A4", &["c|d", "3|4"]);
    let input = sheet.input();
    let limits = Limits {
        max_regions: 1,
        ..Limits::default()
    };
    let layout = detect(&input, &limits, &never);
    let r = only(&layout);
    assert!(r.rest);
    assert_eq!(r.kind, Kind::Grid);
    assert!(r.header_rows.is_empty());
}

#[test]
fn stop_during_detection_is_said() {
    let mut sheet = Sheet::new();
    for k in 0..10u32 {
        sheet = sheet.rows(&format!("A{}", 1 + 3 * k), &["a|b", "1|2"]);
    }
    let input = sheet.input();
    let undecided = detect(&input, &Limits::default(), &never);
    assert!(undecided.regions.iter().all(|r| r.kind == Kind::Table));
    // Stop after the segmenting work: some regions stay undecided.
    let calls = Cell::new(0u32);
    let quiet = segment_calls(&input);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > quiet + 3
    };
    let layout = detect(&input, &Limits::default(), &stop);
    every_cell_once(&input, &layout);
    no_overlap(&layout);
    let stopped: Vec<_> = layout
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Stopped)
        .collect();
    assert_eq!(stopped.len(), 1, "{:?}", layout.issues);
    assert!(
        stopped[0].what.contains("not decided"),
        "{}",
        stopped[0].what
    );
    let tables = layout
        .regions
        .iter()
        .filter(|r| r.kind == Kind::Table)
        .count();
    assert!(0 < tables && tables < 10, "{tables} tables");
    for r in layout.regions.iter().filter(|r| r.kind != Kind::Table) {
        assert_eq!(r.certainty, Certainty::Unsure);
        assert_eq!(r.unsure, ["time limit reached before the header was found"]);
    }
}

#[test]
fn stop_part_way_keeps_every_cell() {
    // A header of 10,000 columns: the stop can come inside the path work.
    let mut sheet = Sheet::new();
    for col in 0..10_000u32 {
        sheet.put(Pos { row: 0, col }, &format!("h{col}"));
        sheet.put(Pos { row: 1, col }, "1");
    }
    let input = sheet.input();
    let quiet = segment_calls(&input);
    let mut outcomes = (0, 0);
    for after in quiet..quiet + 12 {
        let calls = Cell::new(0u32);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > after
        };
        let layout = detect(&input, &Limits::default(), &stop);
        every_cell_once(&input, &layout);
        let r = only(&layout);
        if r.kind == Kind::Table {
            outcomes.0 += 1;
            assert_eq!(r.header_paths.len(), 10_000);
        } else {
            outcomes.1 += 1;
            assert!(r.header_rows.is_empty());
            assert_eq!(r.data, Some(r.rect));
        }
    }
    assert!(outcomes.0 > 0 && outcomes.1 > 0, "{outcomes:?}");
}

/// A merged title B1:H1 over a table B3:D6: a fact table, a fact table with its header row
/// hidden, and a rule table.
fn titled(kind: Option<FactKind>) -> Sheet {
    let sheet = Sheet::new()
        .cell("B1", "Sales by region")
        .merge("B1:H1")
        .rows(
            "B3",
            &["Name|Age|City", "Ann|31|Rome", "Bob|42|Oslo", "Cid|53|Riga"],
        );
    match kind {
        Some(kind) => sheet.fact("B3:D6", kind),
        None => sheet,
    }
}

#[test]
fn titled_tables_keep_their_columns() {
    let hidden = FactKind::ListObject {
        name: "H".into(),
        header_rows: 0,
        totals_rows: 0,
        columns: vec!["N".into(), "A".into(), "C".into()],
    };
    let cases = [
        (Some(list_object("T")), vec!["Name", "Age", "City"]),
        (Some(hidden), vec!["N", "A", "C"]),
        (None, vec!["Name", "Age", "City"]),
    ];
    for (kind, want) in cases {
        let input = titled(kind.clone()).input();
        let layout = detect(&input, &Limits::default(), &never);
        every_cell_once(&input, &layout);
        let r = only(&layout);
        assert_eq!(r.rect, rect("B1:H6"), "{kind:?}");
        assert_eq!(r.captions, [0], "{kind:?}");
        assert_eq!(r.kind, Kind::Table, "{kind:?}");
        assert_eq!(r.header_paths, want, "{kind:?}");
        assert_eq!(r.label_cols, 1, "{kind:?}");
        // A stop at any point keeps every cell once.
        let quiet = segment_calls(&input);
        for after in quiet..quiet + 6 {
            let calls = Cell::new(0u32);
            let stop = || {
                calls.set(calls.get() + 1);
                calls.get() > after
            };
            let layout = detect(&input, &Limits::default(), &stop);
            every_cell_once(&input, &layout);
        }
        // So does the path limit.
        let limits = Limits {
            max_path_bytes: 10,
            ..Limits::default()
        };
        let layout = detect(&input, &limits, &never);
        every_cell_once(&input, &layout);
        assert!(only(&layout).rest, "{kind:?}");
    }
    // H3 on a titled all-text table looks at the table's columns only.
    let mut sheet = Sheet::styled()
        .cell("B1", "*People")
        .merge("B1:H1")
        .rows("B3", &["*Name|*City", "Ann|Rome", "Bob|Oslo"]);
    sheet.styles_read = true;
    let layout = sheet.detect();
    let r = only(&layout);
    assert_eq!(header(r), [3]);
    assert_eq!(r.header_paths, ["Name", "City"]);
}

/// How often `segment` asks the stop callback on this input.
fn segment_calls(input: &sheetlayout::Input) -> u32 {
    let calls = Cell::new(0u32);
    let count = || {
        calls.set(calls.get() + 1);
        false
    };
    sheetlayout::segment(input, &Limits::default(), &count);
    calls.get()
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u32) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % u64::from(n.max(1))) as u32
    }
}

#[test]
fn random_sheets_detect_every_cell_once() {
    let mut g = Lcg(11);
    for case in 0..3_000 {
        let mut sheet = if g.next(2) == 0 {
            Sheet::new()
        } else {
            Sheet::styled()
        };
        for _ in 0..g.next(40) {
            let p = Pos {
                row: g.next(12),
                col: g.next(8),
            };
            let token =
                ["x", "y", "1", "2020", "*t", "^u", "=", "*^v", ">w", ">"][g.next(10) as usize];
            sheet.put(p, token);
        }
        for _ in 0..g.next(5) {
            let (top, left) = (g.next(12), g.next(8));
            sheet.merges.push(Rect {
                top,
                left,
                bottom: top + g.next(3),
                right: left + g.next(4),
            });
        }
        if g.next(3) == 0 {
            let top = g.next(8);
            sheet.facts.push(sheetlayout::Fact {
                rect: Rect {
                    top,
                    left: 0,
                    bottom: top + g.next(5),
                    right: g.next(5),
                },
                kind: [
                    list_object("L"),
                    FactKind::AutoFilter,
                    FactKind::ListObject {
                        name: "H".into(),
                        header_rows: 0,
                        totals_rows: 0,
                        columns: vec!["n".into()],
                    },
                ][g.next(3) as usize]
                    .clone(),
            });
        }
        if g.next(3) == 0 {
            let a = g.next(4);
            sheet.print_titles = Some((a, a + g.next(3)));
        }
        if g.next(3) == 0 {
            sheet.frozen = Some((g.next(4), g.next(3)));
        }
        if g.next(4) == 0 {
            sheet.header_guess = Some((g.next(3), g.next(2) == 0));
        }
        let input = sheet.input();
        let limits = Limits {
            max_regions: 1 + g.next(8) as usize,
            max_path_bytes: [20, 200, 1 << 20][g.next(3) as usize],
            ..Limits::default()
        };
        let after = g.next(30);
        let calls = Cell::new(0u32);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > after
        };
        let layout = detect(&input, &limits, &stop);
        every_cell_once(&input, &layout);
        no_overlap(&layout);
        let stops = layout
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Stopped)
            .count();
        assert!(stops <= 1, "case {case}");
        for r in &layout.regions {
            let ranged = layout.issues.iter().any(|i| {
                matches!(i.kind, IssueKind::Limit | IssueKind::Stopped) && i.rect == Some(r.rect)
            });
            assert_eq!(r.rest, ranged, "case {case}: {:?}", r);
            assert!(r.header_rows.len() <= 10, "case {case}");
            if r.kind == Kind::Table && !r.header_rows.is_empty() {
                let cols = r.data.map_or(r.rect.width(), |d| d.width());
                assert_eq!(r.header_paths.len() as u64, cols, "case {case}");
                let band_end = *r.header_rows.last().unwrap();
                assert!(r.data.is_none_or(|d| d.top > band_end), "case {case}");
            }
            if r.certainty == Certainty::Rule && r.kind == Kind::Table {
                assert!(r.header_rows.len() <= 4, "case {case}");
            }
            assert!(u64::from(r.label_cols) < r.rect.width().max(1) || r.label_cols == 0);
        }
        for i in &layout.issues {
            if matches!(i.kind, IssueKind::Limit | IssueKind::Stopped)
                && let Some(rect) = i.rect
            {
                assert!(
                    layout.regions.iter().any(|r| r.rest && r.rect == rect),
                    "case {case}: {}",
                    i.what
                );
            }
        }
    }
}
