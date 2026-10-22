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

/// No header band. Steps 6d to 6h of the report rules then decide the kind; `id` names the
/// rule that did (`K`, `X`, `H0`), or is empty for the grid of step 6h.
#[track_caller]
fn no_header(r: &Region, kind: Kind, id: &str) {
    assert_eq!(r.kind, kind);
    assert!(r.header_rows.is_empty());
    assert!(r.header_paths.is_empty());
    assert!(r.body.is_none());
    for band in ["H1", "H2", "H3"] {
        assert!(!has_rule(r, band), "{band} gave no band");
    }
    if kind == Kind::Grid {
        assert_eq!(r.certainty, Certainty::Unsure);
        assert_eq!(r.unsure, ["no header found"]);
    } else {
        assert_eq!(r.certainty, Certainty::Rule);
        assert!(r.unsure.is_empty());
        assert!(has_rule(r, id), "{id}");
    }
}

/// The EIA table 1.1 shape: captions in rows 1 and 2, a band in rows 3 and 4 (A3 empty,
/// `Period` in A4), a group row 5 merged over the width (filled, as in the real file) and
/// numbers from row 6.
fn eia(styled: bool) -> Sheet {
    let mark = if styled { "*^" } else { "" };
    let rows: Vec<String> = [
        "Table 1.1. Net Generation by Energy Source",
        "(Thousand Megawatthours)",
        "|Generation at Utility Scale Facilities|||Total",
        "Period|Coal|Natural Gas|Nuclear|",
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
    let group = if styled {
        "^Annual Totals"
    } else {
        "Annual Totals"
    };
    all.push(group);
    all.extend([
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
        .merge("B3:D3")
        .merge("E3:E4")
        .merge("A5:E5")
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
    no_header(only(&layout), Kind::Table, "H0");
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
    no_header(only(&layout), Kind::KeyValue, "K");
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
    no_header(
        only(&Sheet::new().rows("A1", &rows).detect()),
        Kind::KeyValue,
        "K",
    );
    // With styles but no mark: no band.
    no_header(
        only(&Sheet::styled().rows("A1", &rows).detect()),
        Kind::KeyValue,
        "K",
    );
    // A header guess for the first row is a positive signal.
    let mut sheet = Sheet::new().rows("A1", &rows);
    sheet.header_guess = Some((0, true));
    assert_eq!(header(only(&sheet.detect())), [1]);
    sheet.header_guess = Some((0, false));
    no_header(only(&sheet.detect()), Kind::KeyValue, "K");
    // A header text that repeats below in its column: no band.
    let layout = Sheet::styled()
        .rows("A1", &["*Name|*City", "Ann|Rome", "Bob|City"])
        .detect();
    no_header(only(&layout), Kind::KeyValue, "K");
    // Two equal header texts: no band.
    let layout = Sheet::styled()
        .rows("A1", &["*Name|*Name", "Ann|Rome", "Bob|Oslo"])
        .detect();
    no_header(only(&layout), Kind::KeyValue, "K");
    // An empty header cell: no band.
    let layout = Sheet::styled()
        .rows("A1", &["*Name||*City", "Ann|x|Rome", "Bob|y|Oslo"])
        .detect();
    no_header(only(&layout), Kind::Table, "K");
    // H2 marks two rows: not exactly row 1, no band.
    let layout = Sheet::styled()
        .rows("A1", &["*Name|*City", "*Ann|*Rome", "Bob|Oslo"])
        .detect();
    no_header(only(&layout), Kind::KeyValue, "K");
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
    no_header(&r, Kind::Table, "H0");
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
    assert_eq!(header(r), [2]);
    assert_eq!(r.above_header, [0]);
    assert_eq!(r.header_paths, ["Rows", "East", "West"]);
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
    let mut sheet = Sheet::new()
        .rows("A1", &["Head|||||||Tail", "1|2|3|4|5|6|7|8"])
        .merge("A1:G1");
    sheet.frozen = Some((1, 0));
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
    // A grid shows no header evidence.
    assert!(
        !r.evidence.contains(&Evidence::FrozenPanes),
        "{:?}",
        r.evidence
    );
    assert!(!has_rule(r, "H1"));
    assert!(r.body.is_none());
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
                    FactKind::Pivot {
                        name: "P".into(),
                        first_header_row: g.next(3),
                        first_data_row: g.next(4),
                        first_data_col: g.next(3),
                        total_row: false,
                        total_col: false,
                    },
                ][g.next(4) as usize]
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
            if let Some(body) = r.body {
                assert_eq!(r.header_paths.len() as u64, body.width(), "case {case}");
            }
            if r.kind != Kind::Table {
                assert!(r.above_header.is_empty(), "case {case}");
                assert!(r.body.is_none(), "case {case}");
            }
            if r.kind == Kind::Table && !r.header_rows.is_empty() {
                let cols = r.data.map_or(r.rect.width(), |d| d.width());
                assert_eq!(r.header_paths.len() as u64, cols, "case {case}");
                let band_end = *r.header_rows.last().unwrap();
                assert!(r.data.is_none_or(|d| d.top > band_end), "case {case}");
            }
            if r.certainty == Certainty::Rule && r.kind == Kind::Table {
                assert!(r.header_rows.len() <= 4, "case {case}");
            }
            // Rule L leaves at least one column; a pivot's own first data column may not.
            let pivot = r
                .evidence
                .iter()
                .any(|e| matches!(e, Evidence::Pivot { .. }));
            if pivot {
                assert!(u64::from(r.label_cols) <= r.rect.width(), "case {case}");
            } else {
                assert!(
                    u64::from(r.label_cols) < r.rect.width().max(1) || r.label_cols == 0,
                    "case {case}"
                );
            }
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

// Fix round 1.

#[test]
fn tall_title_merge_leaves_the_header() {
    // A title merged over A1:D2: row 2 holds no cell, the header is row 3.
    let rows = ["Region|Q1|Q2|Q3", "North|1|2|3", "South|4|5|6"];
    for styled in [false, true] {
        let base = if styled {
            Sheet::styled().cell("A1", "*Sales")
        } else {
            Sheet::new().cell("A1", "Sales")
        };
        let header_row = if styled {
            "*Region|*Q1|*Q2|*Q3"
        } else {
            rows[0]
        };
        let layout = base
            .merge("A1:D2")
            .rows("A3", &[header_row, rows[1], rows[2]])
            .detect();
        let r = only(&layout);
        assert_eq!(r.captions, [0], "styled {styled}");
        assert_eq!(r.kind, Kind::Table, "styled {styled}");
        assert_eq!(header(r), [3], "styled {styled}");
        assert_eq!(r.header_paths, ["Region", "Q1", "Q2", "Q3"]);
        assert_eq!(r.data, Some(rect("A4:D5")));
        // Row 2 holds only the caption's own merge: it belongs to the caption line (ruling
        // P4-AQ), so the body starts at the header row.
        assert_eq!(r.body, Some(rect("A3:D5")));
    }
    // Print titles on row 3 set the band.
    let mut sheet = Sheet::new()
        .cell("A1", "Sales")
        .merge("A1:D2")
        .rows("A3", &rows);
    sheet.print_titles = Some((2, 2));
    let layout = sheet.detect();
    let r = only(&layout);
    assert_eq!(r.certainty, Certainty::Exact);
    assert_eq!(header(r), [3]);
}

#[test]
fn band_row_covered_by_a_merge_from_above() {
    // Row 2 holds nothing but the merge from row 1: it belongs to the band, and row 3 goes on.
    let layout = Sheet::new()
        .rows("A1", &["a|b", "", "c|d", "1|2"])
        .merge("A1:A2")
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1, 2, 3]);
    assert_eq!(r.header_paths, ["a / c", "b / d"]);
    // A truly empty row (here inside a region joined by a name) ends the band, so row 3 is a
    // text row after the band: no band.
    let layout = Sheet::new()
        .rows("A1", &["a|b", "", "c|d", "1|2"])
        .name("All", "A1:B4")
        .detect();
    let r = only(&layout);
    assert_eq!(r.rect, rect("A1:B4"));
    no_header(r, Kind::Table, "H0");
}

/// Pivot cells, the location triple, the rows above the band, the band (1-based) and the
/// paths.
type PivotCase = (
    &'static [&'static str],
    (u32, u32, u32),
    Vec<u32>,
    Vec<u32>,
    Vec<&'static str>,
);

#[test]
fn pivot_bands_from_the_location() {
    // The corpus triples (first header row, first data row, first data column).
    let cases: [PivotCase; 5] = [
        // pivots.xlsx pT1
        (
            &["Row Labels|Sum of Size", "a|1", "b|2"],
            (0, 1, 1),
            vec![],
            vec![1],
            vec!["Row Labels", "Sum of Size"],
        ),
        // pivots.xlsx pT2 and ExcelPivotTableSample pT1
        (
            &[
                "Sum of Size|Column Labels||",
                "Row Labels|blue|yellow|Grand Total",
                "a|1|2|3",
            ],
            (1, 2, 1),
            vec![0],
            vec![2],
            vec!["Row Labels", "blue", "yellow", "Grand Total"],
        ),
        (
            &[
                "Summe von Field2|Spaltenbeschriftungen||",
                "Zeilenbeschriftungen|44562|44564|Gesamtergebnis",
                "a|1|2|3",
            ],
            (1, 2, 1),
            vec![0],
            vec![2],
            vec!["Zeilenbeschriftungen", "44562", "44564", "Gesamtergebnis"],
        ),
        // pivots.xlsx pT3, pT4, pT5 and ExcelPivotTableSample pT2
        (
            &["Row Labels|Count of Value2", "a|1", "b|2"],
            (1, 1, 1),
            vec![],
            vec![1],
            vec!["Row Labels", "Count of Value2"],
        ),
        (
            &["Zeilenbeschriftungen|Summe von Field2", "a|1"],
            (1, 1, 1),
            vec![],
            vec![1],
            vec!["Zeilenbeschriftungen", "Summe von Field2"],
        ),
    ];
    for (rows, (h, d, c), above, band, paths) in cases {
        let width = rows[0].split('|').count() as u32;
        let range = Rect {
            top: 0,
            left: 0,
            bottom: rows.len() as u32 - 1,
            right: width - 1,
        };
        let layout = Sheet::new()
            .rows("A1", rows)
            .fact(
                &range.a1(),
                FactKind::Pivot {
                    name: "P".into(),
                    first_header_row: h,
                    first_data_row: d,
                    first_data_col: c,
                    total_row: false,
                    total_col: false,
                },
            )
            .detect();
        let r = only(&layout);
        assert_eq!(r.above_header, above, "{rows:?}");
        assert_eq!(header(r), band, "{rows:?}");
        assert_eq!(r.header_paths, paths, "{rows:?}");
        assert_eq!(r.label_cols, c);
    }
    // First data row 0: no band.
    let layout = Sheet::new()
        .rows("A1", &["a|1", "b|2"])
        .fact(
            "A1:B2",
            FactKind::Pivot {
                name: "P".into(),
                first_header_row: 0,
                first_data_row: 0,
                first_data_col: 1,
                total_row: false,
                total_col: false,
            },
        )
        .detect();
    let r = only(&layout);
    assert!(r.header_rows.is_empty());
    assert!(r.above_header.is_empty());
    assert_eq!(r.data, Some(rect("A1:B2")));
}

#[test]
fn year_formulas_count_as_years() {
    let layout = Sheet::new()
        .rows(
            "A1",
            &["Area|Population||", "|2020|=2021|=2022", "US|1|2|3"],
        )
        .detect();
    let r = only(&layout);
    assert_eq!(header(r), [1, 2]);
    assert_eq!(
        r.header_paths,
        ["Area", "Population / 2020", "2021", "2022"]
    );
    // A text formula column in the data keeps the header.
    let layout = Sheet::new()
        .rows("A1", &["Name|Label", "x|=X", "z|=Z"])
        .detect();
    assert_eq!(header(only(&layout)), [1]);
    // A row of only empty results is an empty row: it ends the band like one.
    let layout = Sheet::new()
        .rows("A1", &["a|b", "=|=", "c|d", "1|2"])
        .detect();
    no_header(only(&layout), Kind::Table, "H0");
    // A formula with an empty result in a band row counts as empty.
    let layout = Sheet::new()
        .rows("A1", &["a|b|e", "c|=|d", "1|2|3"])
        .detect();
    assert_eq!(header(only(&layout)), [1, 2]);
}

#[test]
fn body_names_the_path_columns() {
    // A title merged over B1:H1 above an Excel table B3:D3 with only its header row.
    let layout = Sheet::new()
        .cell("B1", "Sales by region")
        .merge("B1:H1")
        .rows("B3", &["Name|Age|City"])
        .fact("B3:D3", list_object("T"))
        .detect();
    let r = only(&layout);
    assert_eq!(r.rect, rect("B1:H3"));
    assert_eq!(r.data, None);
    assert_eq!(r.body, Some(rect("B3:D3")));
    assert_eq!(r.header_paths, ["Name", "Age", "City"]);
    // No band: no body.
    let layout = Sheet::new().rows("A1", &["a|b", "c|d"]).detect();
    assert_eq!(only(&layout).body, None);
}

#[test]
fn covered_cell_text_gives_way_to_the_merge() {
    // C1 is covered by the merge B1:D1 and still holds a stale text.
    let layout = Sheet::new()
        .rows("A1", &["Item|Group|stale|", "|x|y|z", "1|2|3|4"])
        .merge("B1:D1")
        .detect();
    assert_eq!(
        only(&layout).header_paths,
        ["Item", "Group / x", "Group / y", "Group / z"]
    );
}

#[test]
fn overlapping_spreads_take_the_widest() {
    // A damaged file: A1:E1 and B1:C1 are both merged.
    let layout = Sheet::new()
        .rows("A1", &["Wide|Mid|||", "a|b|c|d|e", "1|2|3|4|5"])
        .merge("A1:E1")
        .merge("B1:C1")
        .detect();
    let r = only(&layout);
    assert_eq!(r.header_paths[3], "Wide / d");
    assert_eq!(r.header_paths[4], "Wide / e");
    // B1 anchors its own merge: it keeps its text although A1:E1 covers it. C takes the
    // outer spread.
    assert_eq!(r.header_paths[0], "Wide / a");
    assert_eq!(r.header_paths[1], "Mid / b");
    assert_eq!(r.header_paths[2], "Wide / c");
}

#[test]
fn failed_paths_charge_nothing() {
    // The first region's paths pass the limit; the second still fits.
    let sheet = Sheet::new()
        .rows("A1", &["Head|||||||Tail", "1|2|3|4|5|6|7|8"])
        .merge("A1:G1")
        .rows("A4", &["x|y", "1|2"]);
    let input = sheet.input();
    let limits = Limits {
        max_path_bytes: 200,
        ..Limits::default()
    };
    let layout = detect(&input, &limits, &never);
    assert!(layout.regions[0].rest);
    assert_eq!(layout.regions[1].header_paths, ["x", "y"]);
}

#[test]
fn surviving_mutants_of_the_review() {
    // M2: 4 marked rows are an H2 band.
    let r = Sheet::styled()
        .rows("A1", &["*a|*b", "*c|*d", "*e|*f", "*g|*h", "1|2"])
        .detect()
        .regions
        .remove(0);
    assert_eq!(header(&r), [1, 2, 3, 4]);
    assert!(has_rule(&r, "H2"));
    // M4: a row of text and an error after the band ends H1.
    let mut sheet = Sheet::new().rows("A1", &["a|b", "x|#N/A", "1|2"]);
    for c in &mut sheet.cells {
        if c.text == "#N/A" {
            c.class = sheetlayout::Class::Error;
        }
    }
    no_header(only(&sheet.detect()), Kind::Table, "H0");
    // M6: H3 needs a row below.
    let mut sheet = Sheet::new().rows("A1", &["Name|City"]);
    sheet.header_guess = Some((0, true));
    no_header(only(&sheet.detect()), Kind::Grid, "");
    // M11: 75 % text is no label column.
    let r = Sheet::new()
        .rows("A1", &["Name|V", "a|1", "b|2", "c|3", "4|4"])
        .detect()
        .regions
        .remove(0);
    assert_eq!(header(&r), [1]);
    assert_eq!(r.label_cols, 0);
    // M12: an extended band row that holds data gives no band.
    let layout = Sheet::new()
        .rows("A1", &["a|b", "|2", "3|4"])
        .merge("A1:A2")
        .detect();
    no_header(only(&layout), Kind::Table, "H0");
    // M18: a header guess for another row is no signal for the first.
    let mut sheet = Sheet::new().rows("A1", &["Name|City", "Ann|Rome", "Bob|Oslo"]);
    sheet.header_guess = Some((1, true));
    no_header(only(&sheet.detect()), Kind::KeyValue, "K");
    // The guess names a sheet row: a list from row 3 needs the guess for row 3.
    let mut sheet = Sheet::new().rows("A3", &["Name|City", "Ann|Rome", "Bob|Oslo"]);
    sheet.header_guess = Some((2, true));
    assert_eq!(header(only(&sheet.detect())), [3]);
    // M20: rule L does not run on a pivot.
    let layout = Sheet::new()
        .rows("A1", &["R|S|V", "a|1|5"])
        .fact(
            "A1:C2",
            FactKind::Pivot {
                name: "P".into(),
                first_header_row: 0,
                first_data_row: 1,
                first_data_col: 2,
                total_row: false,
                total_col: false,
            },
        )
        .detect();
    assert_eq!(only(&layout).label_cols, 2);
    // M23: the H2 detail says when the print titles win.
    let mut sheet = Sheet::styled().rows("A1", &["*a|*b", "c|d", "1|2", "3|4"]);
    sheet.print_titles = Some((0, 1));
    let r = sheet.detect().regions.remove(0);
    assert!(rule_detail(&r, "H2").contains("win"), "{:?}", r.evidence);
}

// Fix round 2.

#[test]
fn merge_anchored_outside_a_fact_keeps_its_header() {
    // A1 is merged over A1:A5 and reaches into the Excel table A3:C5.
    let layout = Sheet::new()
        .cell("A1", "t")
        .merge("A1:A5")
        .rows("A3", &["Name|Age|City", "Ann|31|Rome", "Bob|42|Oslo"])
        .fact("A3:C5", list_object("T"))
        .detect();
    let table = layout
        .regions
        .iter()
        .find(|r| r.kind == Kind::Table)
        .expect("table");
    assert_eq!(table.header_paths, ["Name", "Age", "City"]);
}

#[test]
fn caption_merge_does_not_cover_a_band_row() {
    // The caption A1 is merged over A1:A4; rows 2 and 4 hold header texts, row 3 nothing. A
    // row only the caption merge covers ends the run, so row 4 is a text row after it.
    let layout = Sheet::new()
        .cell("A1", "Caption")
        .merge("A1:A4")
        .rows("B2", &["a|b"])
        .rows("B4", &["c|d", "1|2"])
        .detect();
    let r = only(&layout);
    assert_eq!(r.captions, [0]);
    no_header(r, Kind::Table, "H0");
}

#[test]
fn h2_skips_rows_of_empty_results() {
    let layout = Sheet::styled()
        .rows("A1", &["*a|*b", "=|=", "1|2"])
        .detect();
    let r = only(&layout);
    assert_eq!(r.certainty, Certainty::Rule);
    assert_eq!(header(r), [1]);
    assert!(has_rule(r, "H2"));
}

#[test]
fn stopped_pivot_keeps_every_cell_once() {
    let sheet = Sheet::new()
        .rows(
            "A1",
            &["Sum of Size|Column Labels|", "Row Labels|blue|red", "a|1|2"],
        )
        .fact(
            "A1:C3",
            FactKind::Pivot {
                name: "P".into(),
                first_header_row: 1,
                first_data_row: 2,
                first_data_col: 1,
                total_row: false,
                total_col: false,
            },
        );
    let input = sheet.input();
    let quiet = segment_calls(&input);
    let calls = Cell::new(0u32);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > quiet
    };
    let layout = detect(&input, &Limits::default(), &stop);
    every_cell_once(&input, &layout);
    let r = only(&layout);
    assert_eq!(r.kind, Kind::Grid);
    assert!(r.above_header.is_empty());
    assert_eq!(r.data, Some(rect("A1:C3")));
}

// Fix round 2: what stops a caption line that a merge carries down (ruling P4-AQ).

/// The region that holds A1.
fn region_at_a1(layout: &Layout) -> &Region {
    layout
        .regions
        .iter()
        .find(|r| r.rect.top == 0 && r.rect.left == 0)
        .unwrap_or_else(|| panic!("no region at A1: {:?}", common::ranges(layout)))
}

#[test]
fn formula_beside_the_caption_merge_stops_the_caption_line() {
    // C2 holds only a formula with an empty result, inside the rows of the caption merge
    // A1:B3: row 2 is not part of the caption line.
    let layout = Sheet::new()
        .cell("A1", "Sales")
        .merge("A1:B3")
        .cell("C2", "=")
        .rows("A4", &["Region|Q1|Q2", "North|1|2", "South|3|4"])
        .detect();
    let r = region_at_a1(&layout);
    assert_eq!(r.captions, [0], "{r:?}");
    // The rows after the caption line start at row 2 (the body when a header is found,
    // else the data).
    assert_eq!(r.body.or(r.data).map(|b| b.top), Some(1), "{r:?}");
}

#[test]
fn second_merge_inside_the_caption_rows_stops_the_caption_line() {
    // D2:E3 starts in row 2, inside the rows of the caption merge A1:C3: the caption line
    // ends at row 1.
    let layout = Sheet::new()
        .cell("A1", "Sales")
        .merge("A1:C3")
        .merge("D2:E3")
        .rows(
            "A4",
            &["Region|Q1|Q2|Q3|Q4", "North|1|2|3|4", "South|5|6|7|8"],
        )
        .detect();
    let r = region_at_a1(&layout);
    assert_eq!(r.captions, [0], "{r:?}");
    // The rows after the caption line start at row 2 (the body when a header is found,
    // else the data).
    assert_eq!(r.body.or(r.data).map(|b| b.top), Some(1), "{r:?}");
}
