// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sheet layout detector on an input built from the bytes: cells, merges, layout facts
//! and defined names, each from one record of 8 bytes. Nothing may panic, and every cell with
//! content lies in exactly one region (in a caption, header, note or heading row or in the
//! data area), apart from overlapping facts that the detector states. The detector's and the
//! reader's rule for which region holds a cell agree.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sheetlayout::{
    CellIn, Class, Evidence, Fact, FactKind, FormulaIn, Input, IssueKind, Layout, Limits, NameIn,
    Pos, Rect, Region, Style, detect,
};

/// Bytes of one record.
const RECORD: usize = 8;

/// Texts the cells take: labels, a year, a unit, words that look like totals, Markdown
/// marks and the path separator.
const TEXTS: [&str; 16] = [
    "Region",
    "Total",
    "2021",
    "2022",
    "m",
    "kg",
    "Year 2024",
    "a / b",
    "|",
    "- item",
    "[x]",
    "Notes",
    "Gesamtergebnis",
    "Name",
    "Value",
    "",
];

/// A row: mostly near the top, sometimes at the far end of the sheet.
fn row_of(b: u8) -> u32 {
    if b >= 0xF8 {
        1_048_575 - u32::from(b - 0xF8)
    } else {
        u32::from(b % 48)
    }
}

/// A column: mostly near the left, sometimes at the far end of the sheet.
fn col_of(b: u8) -> u32 {
    if b >= 0xF8 {
        16_383 - u32::from(b - 0xF8)
    } else {
        u32::from(b % 12)
    }
}

/// A rectangle from four bytes, corners in order.
fn rect_of(b: &[u8]) -> Rect {
    Rect {
        top: row_of(b[0]),
        left: col_of(b[1]),
        bottom: row_of(b[2]),
        right: col_of(b[3]),
    }
    .normalized()
}

/// An unknown, false or true flag.
fn flag(b: u8) -> Option<bool> {
    match b % 3 {
        0 => None,
        1 => Some(false),
        _ => Some(true),
    }
}

fn cell_of(r: &[u8]) -> CellIn {
    let class = match r[3] % 5 {
        0 => Class::Text,
        1 => Class::Number,
        2 => Class::Date,
        3 => Class::Bool,
        _ => Class::Error,
    };
    let number = matches!(class, Class::Number | Class::Date).then(|| f64::from(r[4]));
    let text = match class {
        Class::Text => TEXTS[usize::from(r[4]) % TEXTS.len()].to_string(),
        Class::Number | Class::Date => r[4].to_string(),
        Class::Bool => if r[4] % 2 == 0 { "TRUE" } else { "FALSE" }.to_string(),
        Class::Error => "#REF!".to_string(),
    };
    let style = Style {
        bold: flag(r[5]),
        filled: flag(r[5] / 3),
        indent: (r[5] / 9 % 3 == 2).then_some(r[5] % 4),
        center_across: flag(r[5] / 27),
        rule_above: flag(r[6]),
        rule_below: flag(r[6] / 3),
    };
    let formula = (r[6] / 9 % 3 == 2).then(|| FormulaIn {
        functions: vec![
            match r[7] % 4 {
                0 => "SUM",
                1 => "SUBTOTAL",
                2 => "AGGREGATE",
                _ => "IF",
            }
            .to_string(),
        ],
        reads: vec![
            Rect {
                top: row_of(r[7] / 4),
                left: col_of(r[2]),
                bottom: row_of(r[1]),
                right: col_of(r[2]),
            }
            .normalized(),
        ],
    });
    CellIn {
        pos: Pos {
            row: row_of(r[1]),
            col: col_of(r[2]),
        },
        class,
        text,
        number,
        style,
        formula,
    }
}

fn fact_of(r: &[u8]) -> Fact {
    let rect = rect_of(&r[1..5]);
    let name = format!("T{}", r[5]);
    let kind = match r[6] % 4 {
        0 => FactKind::ListObject {
            name,
            header_rows: u32::from(r[7] % 3),
            totals_rows: u32::from(r[7] / 3 % 3),
            columns: (0..r[7] % 5).map(|i| format!("C{i}")).collect(),
        },
        1 => FactKind::DatabaseRange {
            name,
            header: r[7] % 2 == 0,
            footer: r[7] / 2 % 2 == 0,
        },
        2 => FactKind::Pivot {
            name,
            first_header_row: u32::from(r[7] % 4),
            first_data_row: u32::from(r[7] / 4 % 4),
            first_data_col: u32::from(r[7] / 16 % 4),
            total_row: r[7] / 64 % 2 == 0,
            total_col: r[7] / 128 == 0,
        },
        _ => FactKind::AutoFilter,
    };
    Fact { rect, kind }
}

fn name_of(r: &[u8]) -> NameIn {
    NameIn {
        name: format!("N{}", r[5]),
        rect: rect_of(&r[1..5]),
        hidden: r[6] % 2 == 1,
        builtin: r[6] / 2 % 2 == 1,
    }
}

/// The input of `data`: a head of 8 bytes for the signals and limits, then records.
fn input_of(data: &[u8]) -> Option<(Input<'static>, Limits)> {
    let (head, rest) = data.split_at_checked(RECORD)?;
    let mut cells = Vec::new();
    let mut merges = Vec::new();
    let mut facts = Vec::new();
    let mut names = Vec::new();
    for r in rest.chunks_exact(RECORD) {
        match r[0] % 8 {
            0..=3 => cells.push(cell_of(r)),
            4 | 5 => merges.push(rect_of(&r[1..5])),
            6 => facts.push(fact_of(r)),
            _ => names.push(name_of(r)),
        }
    }
    cells.sort_by_key(|c| c.pos);
    cells.dedup_by_key(|c| c.pos);
    let key = |r: &Rect| (r.top, r.left, r.bottom, r.right);
    merges.sort_by_key(key);
    merges.dedup();
    facts.sort_by_key(|f| key(&f.rect));
    facts.dedup();
    names.sort_by(|a, b| key(&a.rect).cmp(&key(&b.rect)).then(a.name.cmp(&b.name)));
    names.dedup();
    let outline = if head[1] % 2 == 0 {
        Vec::new()
    } else {
        vec![(
            row_of(head[1]),
            row_of(head[2]).max(row_of(head[1])),
            head[2] % 4,
        )]
    };
    let input = Input {
        cells,
        merges,
        facts,
        names,
        outline,
        frozen: (head[3] % 3 == 0).then(|| (u32::from(head[3] % 4), u32::from(head[4] % 3))),
        print_titles: (head[4] % 3 == 0).then(|| (row_of(head[4]), row_of(head[4]) + 1)),
        header_guess: (head[5] % 4 == 0).then(|| (u32::from(head[5] % 6), head[5] % 8 == 0)),
        styles_read: head[0] % 2 == 0,
        is_label: &|t: &str| !t.trim().is_empty() && t.len() <= 64,
        is_unit_column: &|t: &[&str]| t.iter().all(|t| matches!(*t, "m" | "kg" | "-")),
    };
    // Small limits now and then, so the region, depth and path limits are reached.
    let limits = if head[6] % 4 == 0 {
        Limits {
            max_cut_depth: u32::from(head[6] % 8),
            max_regions: usize::from(head[7] % 16),
            max_section_depth: u32::from(head[7] % 3),
            max_path_bytes: usize::from(head[7]) * 4,
        }
    } else {
        Limits::default()
    };
    Some((input, limits))
}

/// Whether region `r` holds the cell at `p`, as the reader writes it.
fn holds(r: &Region, p: Pos) -> bool {
    if !r.rect.contains(p) {
        return false;
    }
    r.captions.contains(&p.row)
        || r.above_header.contains(&p.row)
        || r.header_rows.contains(&p.row)
        || r.notes.contains(&p.row)
        || r.heading.map(|h| h.0) == Some(p.row)
        || r.data.as_ref().is_some_and(|d| d.contains(p))
}

/// Whether the region comes from a fact of the file (a table, database range, pivot or
/// autofilter), also when a limit left it a grid.
fn from_fact(r: &Region) -> bool {
    r.evidence.iter().any(|e| {
        matches!(
            e,
            Evidence::ListObject { .. }
                | Evidence::DatabaseRange { .. }
                | Evidence::Pivot { .. }
                | Evidence::AutoFilter
        )
    })
}

/// Every cell with content lies in exactly one region. The one stated exception: facts of the
/// same level that overlap (a damaged file) all stay, and `Overlap` issues say so (the first
/// common range of each, a count of the others), so a cell there may lie in several fact
/// regions.
fn check(input: &Input, layout: &Layout) {
    let overlap = layout.issues.iter().any(|i| i.kind == IssueKind::Overlap);
    for c in input.cells.iter().filter(|c| c.occupied()) {
        let holding: Vec<&Region> = layout.regions.iter().filter(|r| holds(r, c.pos)).collect();
        let n = holding.len();
        let stated = n > 1 && overlap && holding.iter().all(|r| from_fact(r));
        assert!(
            n == 1 || stated,
            "cell {} is in {n} regions\ncells {:?}\nmerges {:?}\nfacts {:?}\nnames {:?}\nregions {:#?}\nissues {:?}",
            Rect::cell(c.pos).a1(),
            input.cells,
            input.merges,
            input.facts,
            input.names,
            layout.regions,
            layout.issues
        );
    }
}

/// The detector's rule for which region holds a cell (`sheetlayout::locate`) and the
/// reader's (`rezzer_model::locate_cells`) agree on the regions found.
fn same_owner(input: &Input, layout: &Layout) {
    let rects: Vec<Rect> = layout.regions.iter().map(|r| r.rect).collect();
    let areas: Vec<rezzer_model::CellArea> = rects
        .iter()
        .map(|r| rezzer_model::CellArea {
            top: r.top,
            left: r.left,
            bottom: r.bottom,
            right: r.right,
        })
        .collect();
    let pos: Vec<Pos> = input.cells.iter().map(|c| c.pos).collect();
    let points: Vec<(u32, u32)> = pos.iter().map(|p| (p.row, p.col)).collect();
    assert_eq!(
        sheetlayout::locate(&rects, &pos),
        rezzer_model::locate_cells(&areas, &points)
    );
}

fuzz_target!(|data: &[u8]| {
    // At most 512 records: the check is quadratic in cells and regions.
    if data.len() > RECORD * 513 {
        return;
    }
    let Some((input, limits)) = input_of(data) else {
        return;
    };
    let layout = detect(&input, &limits, &|| false);
    check(&input, &layout);
    same_owner(&input, &layout);
});
