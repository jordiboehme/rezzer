// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Regions, fact ranges and captions.

mod common;

use std::cell::Cell;

use common::{Sheet, every_cell_once, list_object, no_overlap, pos, ranges, rect, thread_cpu};
use sheetlayout::{
    Certainty, Evidence, FactKind, IssueKind, Kind, Limits, NameIn, Pos, Rect, segment,
};

fn never() -> bool {
    false
}

#[test]
fn a1_names_cells_and_ranges() {
    assert_eq!(rect("B3:D5").a1(), "B3:D5");
    assert_eq!(rect("B3").a1(), "B3");
    assert_eq!(rect("Z1:AA2").a1(), "Z1:AA2");
    let far = Rect {
        top: 1_048_575,
        left: 16_383,
        bottom: 1_048_575,
        right: 16_383,
    };
    assert_eq!(far.a1(), "XFD1048576");
    assert_eq!(rect("AZ7:BA9").a1(), "AZ7:BA9");
}

#[test]
fn gap_rows_split_regions() {
    let layout = Sheet::new()
        .rows("A1", &["a|b|c", "1|2|3", "4|5|6"])
        .rows("A5", &["d|e|f", "7|8|9", "1|2|3"])
        .segment();
    assert_eq!(ranges(&layout), ["A1:C3", "A5:C7"]);
    assert!(layout.issues.is_empty());
    for r in &layout.regions {
        assert_eq!(r.kind, Kind::Grid);
        assert_eq!(r.certainty, Certainty::Unsure);
        assert_eq!(r.unsure, ["not decided"]);
        assert!(
            r.evidence
                .iter()
                .any(|e| matches!(e, Evidence::Rule { id: "G", .. }))
        );
    }
}

#[test]
fn gap_columns_split_regions() {
    let layout = Sheet::new()
        .rows("A1", &["a|b", "1|2", "3|4"])
        .rows("D1", &["c|d", "5|6", "7|8"])
        .segment();
    assert_eq!(ranges(&layout), ["A1:B3", "D1:E3"]);
}

#[test]
fn gaps_inside_bands_cut_again() {
    // Two blocks side by side over a third: rows first, then columns in the top band.
    let layout = Sheet::new()
        .rows("A1", &["a|b||x|y", "1|2||3|4"])
        .rows("A4", &["c|d|e|f|g", "5|6|7|8|9"])
        .segment();
    assert_eq!(ranges(&layout), ["A1:B2", "D1:E2", "A4:E5"]);
}

#[test]
fn merge_occupies_span() {
    // Without the merge, A1 and D2:F4 would be cut apart by the empty columns B and C.
    let base = Sheet::new()
        .cell("A1", "Title")
        .rows("D2", &["a|b|c", "1|2|3", "4|5|6"]);
    assert_eq!(ranges(&base.clone().segment()), ["A1", "D2:F4"]);
    let layout = base.merge("A1:F1").segment();
    assert_eq!(ranges(&layout), ["A1:F4"]);
}

#[test]
fn styled_empty_cells_are_not_content() {
    // The temperature-in-middle shape: the styled empty cells around B4:C6 are not in the
    // input. A cell with an empty text and no formula is empty too.
    let sheet = Sheet::styled().rows("B4", &["*label|*value", "a|1", "b|2"]);
    let mut input = sheet.input();
    input.cells.insert(
        0,
        sheetlayout::CellIn {
            pos: pos("A1"),
            class: sheetlayout::Class::Text,
            text: String::new(),
            number: None,
            style: sheetlayout::Style::default(),
            formula: None,
        },
    );
    let layout = segment(&input, &Limits::default(), &never);
    every_cell_once(&input, &layout);
    assert_eq!(ranges(&layout), ["B4:C6"]);
}

#[test]
fn empty_formula_result_is_occupied() {
    let layout = Sheet::new()
        .rows("A1", &["a|b", "1|2"])
        .cell("C2", "=")
        .rows("D1", &["c", "3"])
        .segment();
    assert_eq!(ranges(&layout), ["A1:D2"]);
}

#[test]
fn corner_touch_is_two_regions() {
    let layout = Sheet::new()
        .rows("A1", &["a|b", "1|2"])
        .rows("C3", &["c|d", "3|4"])
        .segment();
    assert_eq!(ranges(&layout), ["A1:B2", "C3:D4"]);
}

#[test]
fn corner_parts_inside_each_others_bounds_stay_one() {
    // An L shape and a lone cell in its corner: the parts touch nowhere, but a region for
    // the lone cell would lie inside the region of the L.
    let layout = Sheet::new().rows("A1", &["a||x", "b", "c|d|e"]).segment();
    assert_eq!(ranges(&layout), ["A1:C3"]);
}

#[test]
fn diagonal_steps_are_separate_regions() {
    let layout = Sheet::new()
        .rows("A1", &["a|b", "c|d"])
        .rows("C3", &["e|f", "g|h"])
        .rows("E5", &["i"])
        .segment();
    assert_eq!(ranges(&layout), ["A1:B2", "C3:D4", "E5"]);
    assert!(
        layout.regions[1]
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::Rule { id: "G", detail } if detail.contains("corner")))
    );
}

#[test]
fn fact_region_wins_and_rest_is_recut() {
    // One block A1:C5. An Excel table over rows 2 and 3 takes them; the rest is cut again
    // and falls apart into the rows above and below it.
    let layout = Sheet::new()
        .rows("A1", &["t|u|v", "a|b|c", "1|2|3", "x|y|z", "4|5|6"])
        .fact("A2:C3", list_object("Sales"))
        .segment();
    assert_eq!(ranges(&layout), ["A1:C1", "A2:C3", "A4:C5"]);
    let table = &layout.regions[1];
    assert_eq!(table.kind, Kind::Table);
    assert_eq!(table.certainty, Certainty::Exact);
    assert_eq!(table.header_rows, [1]);
    assert_eq!(table.data, Some(rect("A3:C3")));
    assert!(table.unsure.is_empty());
    assert_eq!(
        table.evidence,
        [Evidence::ListObject {
            name: "Sales".into()
        }]
    );
    assert_eq!(layout.regions[0].kind, Kind::Grid);
    assert_eq!(layout.regions[2].kind, Kind::Grid);
}

#[test]
fn rule_region_never_reaches_around_a_fact() {
    // A merged title over A1:E1 and a notes column E2:E10 around an Excel table A2:D10: the
    // free cells form an L that no empty row or column cuts.
    let mut sheet = Sheet::new().cell("A1", "Wide title").merge("A1:E1");
    for row in 2..=10 {
        sheet = sheet
            .rows(&format!("A{row}"), &["a|b|c|d"])
            .cell(&format!("E{row}"), &format!("note {row}"));
    }
    let layout = sheet.fact("A2:D10", list_object("T")).segment();
    assert_eq!(ranges(&layout), ["A1:E1", "A2:D10", "E2:E10"]);
    assert!(layout.regions[2].evidence.iter().any(
        |e| matches!(e, Evidence::Rule { id: "G", detail } if detail.contains("table range"))
    ));
}

#[test]
fn ring_around_a_fact_is_cut_around_it() {
    // A U shape: columns A and E and row 6 around a table in B2:D5.
    let mut sheet = Sheet::new();
    for row in 1..=6 {
        sheet = sheet
            .cell(&format!("A{row}"), "l")
            .cell(&format!("E{row}"), "r");
    }
    sheet = sheet
        .rows("B6", &["x|y|z"])
        .rows("B2", &["h|i|j", "1|2|3", "4|5|6", "7|8|9"]);
    let layout = sheet.fact("B2:D5", list_object("Inner")).segment();
    assert!(ranges(&layout).contains(&"B2:D5".to_string()));
    assert_eq!(layout.regions.len(), 6, "{:?}", ranges(&layout));
}

#[test]
fn fact_takes_a_side_of_a_block() {
    let layout = Sheet::new()
        .rows("A1", &["a|b|c|d", "1|2|3|4", "5|6|7|8"])
        .fact("A1:B3", list_object("Left"))
        .segment();
    assert_eq!(ranges(&layout), ["A1:B3", "C1:D3"]);
}

#[test]
fn autofilter_on_a_table_is_evidence_of_the_table() {
    let layout = Sheet::new()
        .rows("B2", &["a|b", "1|2", "3|4"])
        .fact("B2:C4", list_object("T1"))
        .fact("B2:C4", FactKind::AutoFilter)
        .segment();
    assert_eq!(ranges(&layout), ["B2:C4"]);
    assert_eq!(
        layout.regions[0].evidence,
        [
            Evidence::ListObject { name: "T1".into() },
            Evidence::AutoFilter
        ]
    );
    assert!(layout.issues.is_empty());
}

#[test]
fn fact_headers_follow_the_fact() {
    let layout = Sheet::new()
        .rows("A1", &["p|q", "a|b", "1|2"])
        .rows("E1", &["c|d", "3|4"])
        .rows("H1", &["e|f", "5|6"])
        .fact(
            "A1:B3",
            FactKind::Pivot {
                name: "P1".into(),
                first_header_row: 1,
                first_data_row: 2,
                first_data_col: 1,
                total_row: false,
                total_col: false,
            },
        )
        .fact(
            "E1:F2",
            FactKind::DatabaseRange {
                name: "db".into(),
                header: false,
                footer: false,
            },
        )
        .fact("H1:I2", FactKind::AutoFilter)
        .segment();
    let r = &layout.regions;
    assert_eq!(ranges(&layout), ["A1:B3", "E1:F2", "H1:I2"]);
    assert_eq!(r[0].header_rows, [0, 1]);
    assert_eq!(r[0].data, Some(rect("A3:B3")));
    assert_eq!(r[0].label_cols, 1);
    assert!(r[1].header_rows.is_empty());
    assert_eq!(r[1].data, Some(rect("E1:F2")));
    assert_eq!(r[2].header_rows, [0]);
}

#[test]
fn list_object_without_header_row_is_hidden_header() {
    let layout = Sheet::new()
        .rows("A1", &["1|2", "3|4"])
        .fact(
            "A1:B2",
            FactKind::ListObject {
                name: "T".into(),
                header_rows: 0,
                totals_rows: 0,
                columns: vec!["x".into(), "y".into()],
            },
        )
        .segment();
    let t = &layout.regions[0];
    assert!(t.hidden_header);
    assert!(t.header_rows.is_empty());
    assert_eq!(t.data, Some(rect("A1:B2")));
}

#[test]
fn equal_facts_overlap_is_an_issue() {
    let layout = Sheet::new()
        .rows("A1", &["a|b|c|", "1|2|3|", "4|5|6|7", "|8|9|0"])
        .fact("A1:C3", list_object("One"))
        .fact("B3:D4", list_object("Two"))
        .segment_unchecked();
    assert_eq!(ranges(&layout), ["A1:C3", "B3:D4"]);
    let overlaps: Vec<_> = layout
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Overlap)
        .collect();
    assert_eq!(overlaps.len(), 1);
    assert_eq!(overlaps[0].rect, Some(rect("B3:C3")));
    assert!(overlaps[0].what.contains("One") && overlaps[0].what.contains("Two"));
}

#[test]
fn name_joins_regions_across_gap() {
    // Census shape: A1:F60 and A62:F68 with the name _NST01 over A4:F62.
    let layout = census(true).segment();
    assert_eq!(ranges(&layout), ["A1:F68"]);
    let r = &layout.regions[0];
    assert!(r.evidence.contains(&Evidence::DefinedName {
        name: "_NST01".into()
    }));
    assert_eq!(r.captions, [0, 1]);
    assert_eq!(r.caption_heading, Some(1));
}

#[test]
fn name_without_both_regions_inside_does_not_join() {
    let layout = census(false).name("Part", "A4:F70").segment();
    assert_eq!(ranges(&layout), ["A1:F60", "A62:F68"]);
}

#[test]
fn builtin_names_never_join() {
    let mut sheet = census(false);
    sheet.names.push(NameIn {
        name: "_xlnm.Print_Area".into(),
        rect: rect("A4:F62"),
        hidden: true,
        builtin: true,
    });
    assert_eq!(ranges(&sheet.segment()), ["A1:F60", "A62:F68"]);
}

#[test]
fn name_never_cuts() {
    let layout = Sheet::new()
        .rows("A1", &["a|b|c|d", "1|2|3|4", "5|6|7|8", "9|0|1|2"])
        .name("Inner", "A1:B2")
        .name("Small", "C3:C4")
        .segment();
    assert_eq!(ranges(&layout), ["A1:D4"]);
}

#[test]
fn title_within_two_rows_attaches() {
    let layout = Sheet::styled()
        .cell("A1", "*Sales by region")
        .rows("A4", &["region|q1|q2", "north|1|2", "south|3|4"])
        .segment();
    assert_eq!(ranges(&layout), ["A1:C6"]);
    let r = &layout.regions[0];
    assert_eq!(r.captions, [0]);
    assert_eq!(r.caption_heading, Some(0));
    assert_eq!(r.data, Some(rect("A4:C6")));
    assert!(
        r.evidence
            .iter()
            .any(|e| matches!(e, Evidence::Rule { id: "T2", .. }))
    );
}

#[test]
fn title_attaches_to_a_fact_table() {
    let layout = Sheet::new()
        .cell("B1", "Bob")
        .rows("A3", &["a|b", "1|2"])
        .fact("A3:B4", list_object("Sales_Bob"))
        .segment();
    assert_eq!(ranges(&layout), ["A1:B4"]);
    let r = &layout.regions[0];
    assert_eq!(r.kind, Kind::Table);
    assert_eq!(r.captions, [0]);
    assert_eq!(r.header_rows, [2]);
}

#[test]
fn title_above_a_captioned_region_is_a_title() {
    let layout = Sheet::styled()
        .cell("A1", "*Report")
        .rows("A3", &["*Section", "a|1", "b|2"])
        .segment();
    assert_eq!(ranges(&layout), ["A1", "A3:B5"]);
    let title = &layout.regions[0];
    assert_eq!(title.kind, Kind::Title);
    assert_eq!(title.certainty, Certainty::Rule);
    assert_eq!(title.captions, [0]);
    assert_eq!(title.data, None);
    assert_eq!(layout.regions[1].captions, [2]);
}

#[test]
fn title_that_would_overlap_a_side_region_does_not_attach() {
    let layout = Sheet::new()
        .cell("A1", "Wide title")
        .merge("A1:F1")
        .rows("A3", &["a|b", "1|2"])
        .rows("E3", &["c|d", "3|4"])
        .segment();
    assert_eq!(ranges(&layout), ["A1:F1", "A3:B4", "E3:F4"]);
    assert_eq!(layout.regions[0].kind, Kind::Title);
}

#[test]
fn title_far_away_stays_text() {
    // header-row.xlsx shape: a note in C3, the table in A9:D10.
    let layout = Sheet::new()
        .cell("C3", "Note 1")
        .rows("A9", &["a|b|c|d", "1|2|3|4"])
        .segment();
    assert_eq!(ranges(&layout), ["C3", "A9:D10"]);
    assert_ne!(layout.regions[0].kind, Kind::Title);
    assert!(layout.regions[1].captions.is_empty());
}

#[test]
fn caption_rows_stop_at_less_marked_row() {
    // Tunnel rows 18 to 25: row 18 bold and filled, the rest plain merged text rows.
    let mut tunnel = Sheet::styled()
        .cell("A18", "*^PROJECT DESCRIPTION")
        .merge("A18:C18");
    for row in 19..=25 {
        tunnel = tunnel
            .cell(&format!("A{row}"), &format!("line {row}"))
            .merge(&format!("A{row}:C{row}"));
    }
    let layout = tunnel.segment();
    assert_eq!(ranges(&layout), ["A18:C25"]);
    assert_eq!(layout.regions[0].captions, [17]);
    assert_eq!(layout.regions[0].caption_heading, Some(17));
    assert_eq!(layout.regions[0].data, Some(rect("A19:C25")));

    // Census rows 1 and 2: filled, then bold and filled. The heading is the bold row 2.
    let layout = census(false).segment();
    assert_eq!(layout.regions[0].captions, [0, 1]);
    assert_eq!(layout.regions[0].caption_heading, Some(1));
}

#[test]
fn caption_rows_without_styles_stop_at_three() {
    let mut sheet = Sheet::new();
    for row in 1..=6 {
        sheet = sheet.cell(&format!("A{row}"), &format!("line {row}"));
    }
    let layout = sheet.segment();
    assert_eq!(layout.regions[0].captions, [0, 1, 2]);
    assert_eq!(layout.regions[0].caption_heading, Some(0));
    assert_eq!(layout.regions[0].data, Some(rect("A4:A6")));
}

#[test]
fn caption_rows_need_one_text_cell() {
    let layout = Sheet::new().rows("A1", &["2024", "a|b", "1|2"]).segment();
    assert!(layout.regions[0].captions.is_empty());
}

#[test]
fn print_titles_and_panes_are_matched() {
    let mut sheet = Sheet::new()
        .rows("A1", &["a|b", "1|2", "3|4"])
        .rows("A6", &["c|d", "5|6"]);
    sheet.print_titles = Some((0, 0));
    sheet.frozen = Some((1, 0));
    let layout = sheet.segment();
    assert!(layout.regions[0].evidence.contains(&Evidence::PrintTitles));
    assert!(layout.regions[0].evidence.contains(&Evidence::FrozenPanes));
    assert!(!layout.regions[1].evidence.contains(&Evidence::PrintTitles));
    assert!(!layout.regions[1].evidence.contains(&Evidence::FrozenPanes));
}

/// A real checkerboard: a cell wherever row plus column is even. No row or column is empty, so
/// only the corner split separates the cells.
fn checkerboard(n: u32) -> Sheet {
    let mut sheet = Sheet::new();
    for row in 0..n {
        for col in (row % 2..n).step_by(2) {
            sheet.put(Pos { row, col }, "x");
        }
    }
    sheet
}

/// Segments with no stop and gives the layout and the thread CPU time it took.
fn timed(input: &sheetlayout::Input) -> (sheetlayout::Layout, std::time::Duration) {
    let start = thread_cpu();
    let layout = segment(input, &Limits::default(), &never);
    (layout, thread_cpu() - start)
}

const BOUND: std::time::Duration = std::time::Duration::from_secs(2);

#[test]
fn checkerboard_hits_region_limit() {
    let input = checkerboard(300).input();
    let (layout, used) = timed(&input);
    eprintln!(
        "checkerboard 300: {} regions, {used:?} CPU",
        layout.regions.len()
    );
    assert!(used < BOUND, "{used:?}");
    assert!(layout.regions.len() <= 10_000);
    let limits: Vec<_> = layout
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Limit)
        .collect();
    assert_eq!(limits.len(), 1);
    let rest: Vec<_> = layout.regions.iter().filter(|r| r.rest).collect();
    assert_eq!(rest.len(), 1);
    assert!(rest[0].unsure[0].contains("region limit"));
    assert_eq!(rest[0].kind, Kind::Grid);
    assert_eq!(limits[0].rect, Some(rest[0].rect));
    every_cell_once(&input, &layout);
    no_overlap(&layout);
}

#[test]
fn large_checkerboards_are_bounded() {
    for n in [600, 1000] {
        let input = checkerboard(n).input();
        let (layout, used) = timed(&input);
        eprintln!(
            "checkerboard {n}: {} regions, {used:?} CPU",
            layout.regions.len()
        );
        assert!(used < BOUND, "{n}: {used:?}");
        assert_eq!(layout.regions.iter().filter(|r| r.rest).count(), 1);
    }
}

#[test]
fn diagonal_staircase_is_bounded() {
    let mut sheet = Sheet::new();
    for k in 0..20_000u32 {
        sheet.put(Pos { row: k, col: k }, "x");
    }
    let input = sheet.input();
    let (layout, used) = timed(&input);
    eprintln!("staircase: {} regions, {used:?} CPU", layout.regions.len());
    assert!(used < BOUND, "{used:?}");
    assert_eq!(layout.regions.len(), 10_000);
    assert_eq!(layout.regions.iter().filter(|r| r.rest).count(), 1);
    every_cell_once(&input, &layout);
    no_overlap(&layout);
}

#[test]
fn cut_depth_limit_gives_rest() {
    // The depth limit holds per branch: only the pieces that need a deeper cut are rest.
    let sheet = Sheet::new()
        .rows("A1", &["a|b||c"])
        .rows("A3", &["d"])
        .rows("A5", &["e|f||g"]);
    let input = sheet.input();
    let limits = Limits {
        max_cut_depth: 1,
        ..Limits::default()
    };
    let layout = segment(&input, &limits, &never);
    every_cell_once(&input, &layout);
    assert_eq!(ranges(&layout), ["A1:D1", "A3", "A5:D5"]);
    let rest: Vec<bool> = layout.regions.iter().map(|r| r.rest).collect();
    assert_eq!(rest, [true, false, true]);
    assert_eq!(layout.issues.len(), 2);
    for i in &layout.issues {
        assert_eq!(i.kind, IssueKind::Limit);
        assert!(i.what.contains("cut depth"));
    }
}

#[test]
fn region_limit_is_exact() {
    let mut sheet = Sheet::new();
    for k in 0..5 {
        sheet = sheet.cell(&format!("A{}", 1 + 2 * k), "1");
    }
    let input = sheet.input();
    for max in 1..=6 {
        let limits = Limits {
            max_regions: max,
            ..Limits::default()
        };
        let layout = segment(&input, &limits, &never);
        every_cell_once(&input, &layout);
        assert_eq!(layout.regions.len(), max.min(5), "max_regions {max}");
    }
}

#[test]
fn stop_callback_ends_early() {
    // 100 blocks of 2 by 2 cells with gaps between them.
    let mut sheet = Sheet::new();
    for i in 0..10u32 {
        for j in 0..10u32 {
            for (dr, dc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                sheet.put(
                    Pos {
                        row: 3 * i + dr,
                        col: 3 * j + dc,
                    },
                    "1",
                );
            }
        }
    }
    let input = sheet.input();
    let calls = Cell::new(0u32);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 20
    };
    let layout = segment(&input, &Limits::default(), &stop);
    every_cell_once(&input, &layout);
    no_overlap(&layout);
    assert!(calls.get() < 50, "{} calls", calls.get());
    assert!(layout.regions.len() > 1 && layout.regions.len() < 100);
    let stopped: Vec<_> = layout
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Stopped)
        .collect();
    assert_eq!(stopped.len(), 1);
    let rest: Vec<_> = layout
        .regions
        .iter()
        .filter(|r| r.unsure.iter().any(|u| u.contains("time limit")))
        .collect();
    assert_eq!(rest.len(), 1);
    assert!(rest[0].rest);
    assert_eq!(stopped[0].rect, Some(rest[0].rect));

    // A stop before any work: the whole sheet is one grid region.
    let layout = segment(&input, &Limits::default(), &|| true);
    every_cell_once(&input, &layout);
    assert_eq!(ranges(&layout), ["A1:AC29"]);
    assert_eq!(layout.issues[0].kind, IssueKind::Stopped);
}

#[test]
fn hostile_rectangles_do_not_panic() {
    let mut sheet = Sheet::new().rows("A1", &["a|b", "1|2"]);
    let max = Rect {
        top: u32::MAX,
        left: u32::MAX,
        bottom: u32::MAX,
        right: u32::MAX,
    };
    let inverted = Rect {
        top: 5,
        left: 5,
        bottom: 2,
        right: 1,
    };
    sheet.merges.push(max);
    sheet.merges.push(inverted);
    sheet.names.push(NameIn {
        name: "n".into(),
        rect: max,
        hidden: false,
        builtin: false,
    });
    sheet.facts.push(sheetlayout::Fact {
        rect: inverted,
        kind: FactKind::AutoFilter,
    });
    sheet.cells.push(sheetlayout::CellIn {
        pos: Pos {
            row: u32::MAX,
            col: u32::MAX,
        },
        class: sheetlayout::Class::Text,
        text: "far".into(),
        number: None,
        style: Default::default(),
        formula: None,
    });
    let _ = sheet.segment();
}

#[test]
fn unsorted_and_repeated_cells_are_tolerated() {
    let sheet = Sheet::new().rows("A1", &["a|b", "1|2"]).rows("A5", &["c"]);
    let mut input = sheet.input();
    input.cells.reverse();
    let dup = input.cells[0].clone();
    input.cells.push(dup);
    let layout = segment(&input, &Limits::default(), &never);
    assert_eq!(ranges(&layout), ["A1:B2", "A5"]);
}

#[test]
fn title_three_rows_away_does_not_attach() {
    let layout = Sheet::new()
        .cell("A1", "Sales")
        .rows("A5", &["region|q1", "north|1"])
        .segment();
    assert_eq!(ranges(&layout), ["A1", "A5:B6"]);
    assert!(layout.regions[1].captions.is_empty());
}

#[test]
fn caption_row_needs_its_merge_to_hold_the_text() {
    // A text in A1 and an empty merge B1:C1 beside it: not a single-text row.
    let layout = Sheet::new()
        .cell("A1", "Title")
        .merge("B1:C1")
        .rows("A2", &["a|b|c", "1|2|3"])
        .segment();
    assert!(layout.regions[0].captions.is_empty());
    // A text in a merge and a second merge in the row: not a single-text row either.
    let layout = Sheet::new()
        .cell("A1", "Title")
        .merge("A1:B1")
        .merge("C1:D1")
        .rows("A2", &["a|b|c|d", "1|2|3|4"])
        .segment();
    assert!(layout.regions[0].captions.is_empty());
    // One merge that holds the text is one.
    let layout = Sheet::new()
        .cell("A1", "Title")
        .merge("A1:D1")
        .rows("A2", &["a|b|c|d", "1|2|3|4"])
        .segment();
    assert_eq!(layout.regions[0].captions, [0]);
}

#[test]
fn one_row_name_never_joins() {
    let layout = Sheet::new()
        .rows("A1", &["a|b||d|e"])
        .name("Row", "A1:E1")
        .segment();
    assert_eq!(ranges(&layout), ["A1:B1", "D1:E1"]);
}

#[test]
fn name_join_that_would_reach_another_region_is_skipped() {
    let layout = Sheet::new()
        .rows("A1", &["a|b|c"])
        .cell("C3", "7")
        .rows("A5", &["d|e|f"])
        .name("Both", "A1:B5")
        .segment();
    assert_eq!(ranges(&layout), ["A1:C1", "C3", "A5:C5"]);
}

#[test]
fn name_on_an_exact_range_is_evidence() {
    let layout = Sheet::new()
        .rows("B2", &["a|b", "1|2"])
        .name("Block", "B2:C3")
        .segment();
    assert!(layout.regions[0].evidence.contains(&Evidence::DefinedName {
        name: "Block".into()
    }));
    let layout = Sheet::new()
        .rows("B2", &["a|b", "1|2"])
        .name("Bigger", "B2:C4")
        .segment();
    assert!(
        !layout.regions[0]
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::DefinedName { .. }))
    );
}

#[test]
fn stop_during_fact_assignment_leaves_no_fact_region() {
    let sheet = Sheet::new()
        .rows("A1", &["a|b", "1|2"])
        .rows("B10", &["c|d", "3|4"])
        .fact("B10:C11", list_object("Far"));
    let input = sheet.input();
    let layout = segment(&input, &Limits::default(), &|| true);
    every_cell_once(&input, &layout);
    assert_eq!(ranges(&layout), ["A1:C11"]);
    assert!(layout.regions[0].rest);
}

#[test]
fn pane_and_print_title_boundaries() {
    // The split row (3) is the region's last row: the pane covers it.
    let mut sheet = Sheet::new().rows("A1", &["a|b", "1|2", "3|4", "5|6"]);
    sheet.frozen = Some((3, 0));
    let layout = sheet.segment();
    assert!(layout.regions[0].evidence.contains(&Evidence::FrozenPanes));
    // The split is below the region: not matched.
    let mut sheet = Sheet::new().rows("A1", &["a|b", "1|2", "3|4"]);
    sheet.frozen = Some((3, 0));
    assert!(
        !sheet.segment().regions[0]
            .evidence
            .contains(&Evidence::FrozenPanes)
    );
    // Print titles must lie inside the region, not only touch it.
    let mut sheet = Sheet::new().rows("A2", &["a|b", "1|2", "3|4"]);
    sheet.print_titles = Some((0, 1));
    assert!(
        !sheet.segment().regions[0]
            .evidence
            .contains(&Evidence::PrintTitles)
    );
    sheet.print_titles = Some((1, 2));
    assert!(
        sheet.segment().regions[0]
            .evidence
            .contains(&Evidence::PrintTitles)
    );
}

/// The shape where every name hits almost every region and a join is always blocked.
fn names_sheet(names: usize) -> Sheet {
    let mut sheet = Sheet::new();
    for row in 0..400u32 {
        sheet.put(Pos { row, col: 0 }, "t");
    }
    for i in 0..200u32 {
        for j in 0..49u32 {
            sheet.put(
                Pos {
                    row: 2 * i,
                    col: 2 + 2 * j,
                },
                "1",
            );
        }
    }
    sheet.put(Pos { row: 399, col: 99 }, "end");
    let mut k = 0;
    'outer: for c in (2..=98u32).rev().step_by(2) {
        for r in (100..398u32).rev() {
            if k == names {
                break 'outer;
            }
            sheet.names.push(NameIn {
                name: format!("n{k}"),
                rect: Rect {
                    top: 0,
                    left: 0,
                    bottom: r,
                    right: c,
                },
                hidden: false,
                builtin: false,
            });
            k += 1;
        }
    }
    sheet
}

#[test]
fn many_names_over_many_regions_are_bounded() {
    let input = names_sheet(3_000).input();
    let (layout, used) = timed(&input);
    eprintln!("names: {} regions, {used:?} CPU", layout.regions.len());
    assert!(used < BOUND, "{used:?}");
    assert!(layout.regions.len() > 9_000);
    every_cell_once(&input, &layout);
}

#[test]
fn stop_during_the_names_step_is_said() {
    let input = names_sheet(3_000).input();
    let calls = Cell::new(0u64);
    let count = || {
        calls.set(calls.get() + 1);
        false
    };
    let _ = segment(&input, &Limits::default(), &count);
    let total = calls.get();
    // The names step asks last: stop at its first question.
    calls.set(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > total - 3_000 / 64
    };
    let layout = segment(&input, &Limits::default(), &stop);
    every_cell_once(&input, &layout);
    assert!(layout.regions.iter().all(|r| !r.rest));
    let stopped: Vec<_> = layout
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Stopped)
        .collect();
    assert_eq!(stopped.len(), 1);
    assert!(stopped[0].what.contains("names"));
}

/// A small random generator for the property test.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u32) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % u64::from(n)) as u32
    }
}

#[test]
fn random_sheets_keep_every_cell_once() {
    let mut g = Lcg(7);
    for case in 0..3_000 {
        let mut sheet = if g.next(2) == 0 {
            Sheet::new()
        } else {
            Sheet::styled()
        };
        for _ in 0..g.next(40) {
            let p = Pos {
                row: g.next(12),
                col: g.next(12),
            };
            let token = ["x", "1", "*t", "^u", "=", "*^v"][g.next(6) as usize];
            sheet.put(p, token);
        }
        let rect_at = |g: &mut Lcg| {
            let (top, left) = (g.next(12), g.next(12));
            Rect {
                top,
                left,
                bottom: top + g.next(4),
                right: left + g.next(4),
            }
        };
        for _ in 0..g.next(5) {
            sheet.merges.push(rect_at(&mut g));
        }
        // Facts of different levels only, so no two facts of one level overlap.
        let kinds = [
            list_object("L"),
            FactKind::DatabaseRange {
                name: "D".into(),
                header: true,
                footer: false,
            },
            FactKind::AutoFilter,
        ];
        for kind in kinds {
            if g.next(3) == 0 {
                sheet.facts.push(sheetlayout::Fact {
                    rect: rect_at(&mut g),
                    kind,
                });
            }
        }
        for k in 0..g.next(3) {
            sheet.names.push(NameIn {
                name: format!("n{k}"),
                rect: rect_at(&mut g),
                hidden: false,
                builtin: false,
            });
        }
        let input = sheet.input();
        let limits = Limits {
            max_cut_depth: g.next(5),
            max_regions: 1 + g.next(8) as usize,
            ..Limits::default()
        };
        let after = g.next(12);
        let calls = Cell::new(0u32);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > after
        };
        let layout = segment(&input, &limits, &stop);
        every_cell_once(&input, &layout);
        no_overlap(&layout);
        assert!(
            layout.regions.len() <= limits.max_regions,
            "case {case}: {} regions over {}",
            layout.regions.len(),
            limits.max_regions
        );
        assert!(
            layout
                .issues
                .iter()
                .filter(|i| i.kind == IssueKind::Stopped)
                .count()
                <= 1
        );
    }
}

/// `k` long one-column merges crossed by `k` merges over the whole width, and a 2 by 2 block
/// that touches the big block only at its lower right corner.
fn crossed_merges(k: u32) -> Sheet {
    let mut sheet = Sheet::new();
    for c in 0..k {
        sheet.merges.push(Rect {
            top: 0,
            left: c,
            bottom: 2 * k,
            right: c,
        });
    }
    for r in 0..k {
        sheet.merges.push(Rect {
            top: 2 * r,
            left: 0,
            bottom: 2 * r,
            right: k - 1,
        });
    }
    sheet
        .cell("A1", "x")
        .rows(&rect_name(2 * k + 1, k), &["a|b", "c|d"])
}

fn rect_name(row: u32, col: u32) -> String {
    Rect {
        top: row,
        left: col,
        bottom: row,
        right: col,
    }
    .a1()
}

#[test]
fn corner_split_past_its_work_budget_says_so() {
    let layout = crossed_merges(1_000).segment();
    assert_eq!(layout.regions.len(), 1);
    let coarse: Vec<_> = layout
        .issues
        .iter()
        .filter(|i| i.kind == IssueKind::Coarse)
        .collect();
    assert_eq!(coarse.len(), 1);
    assert_eq!(coarse[0].rect, Some(layout.regions[0].rect));
    assert!(!layout.regions[0].rest);
    // A small shape of the same kind is split and needs no issue.
    let layout = crossed_merges(20).segment();
    assert_eq!(ranges(&layout), ["A1:T41", "U42:V43"]);
    assert!(layout.issues.is_empty());
}

#[test]
fn cuts_around_facts_do_not_count_toward_the_depth_limit() {
    // 70 facts of three rows in B:C, each with a full connecting row below it. Cutting the
    // free cells around them takes 70 cuts in a row; the cells left of each fact then need
    // one more cut (rows 1 and 3 of the fact, with row 2 empty in column A).
    let mut sheet = Sheet::new();
    for k in 0..70u32 {
        let top = 4 * k;
        sheet.put(Pos { row: top, col: 0 }, "l");
        sheet.put(
            Pos {
                row: top + 2,
                col: 0,
            },
            "l",
        );
        for row in top..top + 3 {
            sheet.put(Pos { row, col: 1 }, "1");
            sheet.put(Pos { row, col: 2 }, "2");
            sheet.put(Pos { row, col: 3 }, "r");
        }
        for col in 0..4 {
            sheet.put(Pos { row: top + 3, col }, "c");
        }
        sheet.facts.push(sheetlayout::Fact {
            rect: Rect {
                top,
                left: 1,
                bottom: top + 2,
                right: 2,
            },
            kind: FactKind::AutoFilter,
        });
    }
    let layout = sheet.segment();
    assert_eq!(
        layout
            .regions
            .iter()
            .filter(|r| r.kind == Kind::Table)
            .count(),
        70
    );
    assert!(layout.regions.iter().all(|r| !r.rest));
    assert!(layout.issues.is_empty(), "{:?}", layout.issues);
}

#[test]
fn depth_rest_folded_into_the_region_rest_leaves_no_stale_issue() {
    // Band 1: A1 and A3 (they need a third cut), C2 and E1. Band 2: row 5. With depth 2 and
    // room for 3 regions, A1:A3 stays uncut, C2 is placed, and E1 trips the region limit;
    // the rest A1:I5 then holds the uncut piece too.
    let sheet = Sheet::new()
        .cell("A1", "1")
        .cell("A3", "2")
        .cell("C2", "3")
        .cell("E1", "4")
        .rows("A5", &["5|6|7|8|9|1|2|3|4"]);
    let input = sheet.input();
    let limits = Limits {
        max_cut_depth: 2,
        max_regions: 3,
        ..Limits::default()
    };
    let layout = segment(&input, &limits, &never);
    every_cell_once(&input, &layout);
    assert_eq!(ranges(&layout), ["A1:I5"]);
    assert_eq!(layout.issues.len(), 1, "{:?}", layout.issues);
    assert!(layout.issues[0].what.contains("region limit"));
}

/// The Census NST-EST2023-POP shape: title rows 1 and 2, a two-row header, data to row 60,
/// an empty row 61 and Puerto Rico plus notes in rows 62 to 68.
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
    for row in 5..=60 {
        s = s.rows(&format!("A{row}"), &[&format!("*.State {row}|1|2|3|4|5")]);
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

trait Unchecked {
    fn segment_unchecked(&self) -> sheetlayout::Layout;
}

impl Unchecked for Sheet {
    /// Segments without the every-cell check: overlapping facts hold some cells twice.
    fn segment_unchecked(&self) -> sheetlayout::Layout {
        segment(&self.input(), &Limits::default(), &never)
    }
}
