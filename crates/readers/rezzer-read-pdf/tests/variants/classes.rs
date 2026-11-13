// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The layout classes of step 5 of plan 7 (appendix A of the region engine spec), each a shape
//! of its own on a business page, named by its class. All text and values are invented.

use crate::axes::{Axes, Look, Scale, Shape, Sheet, Variant};
use crate::common::{logo, show};
use crate::docs::{DOC_NEXT, RULED, Span, right, rule, sheet};

/// The looks of [`RULED`] whose rules show where their segments break: a dashed rule is all
/// breaks, so it draws no column bound by them.
const SOLID: Axes = Axes {
    look: &[Look::Real, Look::Dotted, Look::Thick],
    ..RULED
};

/// [`RULED`] at the scales that keep the pictures of a real item row (about 1.4 by 1 cm) over a
/// square centimetre: a smaller picture is not listed (an absolute bound of the listing of
/// pictures, not of the table), so it cannot stand in a cell.
const LISTED: Axes = Axes {
    scale: &[Scale::S100, Scale::S150],
    ..SOLID
};

/// The rows of a table as owned texts.
fn rows_of<const N: usize>(rows: &[[&str; N]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|c| c.to_string()).collect())
        .collect()
}

/// The items of [`picture_items`]: the name, its detail line, the quantity and the price.
const PICTURE_ITEMS: [[&str; 4]; 3] = [
    ["Kaffeebecher", "Art.-Nr. 1001", "2", "12,50"],
    ["Teekanne", "Art.-Nr. 1002", "1", "34,56"],
    ["Untersetzer", "Art.-Nr. 1003", "4", "7,80"],
];

/// B1 and vision class C: an item table between two rules, one above the header and one under
/// the rows, none between them. Each row holds a picture at the left, the item's name over a
/// smaller detail line, and the quantity and the price flush right on the name's line. Each row
/// is a table row (its name and detail one cell, its values in their cells), the picture is in
/// its row's first cell.
fn picture_items(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let top = 600.0;
    let (x_name, x_qty, x_price) = (100.0, 420.0, 540.0);
    let mut t = rule(v, 40.0, 540.0, top + 1.2 * size, size)
        + &show(bold, size, 40.0, top, "Bild")
        + &show(bold, size, x_name, top, "Artikel")
        + &right(bold, size, x_qty, top, "Menge")
        + &right(bold, size, x_price, top, "Preis");
    // Pictures of 48 points (about 1.7 cm, listed at every scale), from the top of the name's
    // capitals down past the detail line, rows 8 points apart.
    let pic = 48.0;
    let pitch = pic + 8.0;
    let mut bottom = top;
    for (n, [name, detail, qty, price]) in PICTURE_ITEMS.iter().enumerate() {
        let y = top - 2.0 * size - pitch * n as f32;
        t += &logo(40.0, y + 0.75 * size - pic, pic, pic);
        t += &show(plain, size, x_name, y, name);
        t += &show(plain, 0.85 * size, x_name, y - 1.2 * size, detail);
        t += &right(plain, size, x_qty, y, qty);
        t += &right(plain, size, x_price, y, price);
        bottom = y + 0.75 * size - pic - 0.6 * size;
    }
    t += &rule(v, 40.0, 540.0, bottom, size);
    // The body starts under the table and the table of the neighbour under it.
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    let mut table = vec![vec![
        "Bild".to_string(),
        "Artikel".into(),
        "Menge".into(),
        "Preis".into(),
    ]];
    for [name, detail, qty, price] in PICTURE_ITEMS {
        table.push(vec![
            String::new(),
            format!("{name} {detail}"),
            qty.into(),
            price.into(),
        ]);
        s.want.row_media.push((name.to_string(), 0));
    }
    s.want.table(table, "table");
    // Every picture stands in its row's cell, none as a media block of its own.
    s.want.media_count = Some(0);
    // The last row's first text is its name, apart from its detail line by the values.
    s.want.kinds.retain(|(t, _)| !t.starts_with("Untersetzer"));
    s.want.kinds.push(("Untersetzer".into(), "table"));
    s
}

/// B2: line items under a rule under the header, where the description of the first item wraps
/// to four lines and its other cells stand on the description's second line; the second item
/// has one line. The description's lines are one cell (a sentence that wraps, read with spaces),
/// and the lines under the values belong to the first row, never a row of their own.
fn wrapped_description(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let top = 600.0;
    let lead = 1.2 * size;
    let mut t = show(bold, size, 40.0, top, "Pos")
        + &show(bold, size, 70.0, top, "Beschreibung")
        + &right(bold, size, 380.0, top, "Menge")
        + &right(bold, size, 460.0, top, "Preis")
        + &right(bold, size, 540.0, top, "Betrag")
        + &rule(v, 40.0, 540.0, top - 0.5 * size, size);
    let lines = [
        "Wartung der Heizungsanlage",
        "im Haus Musterweg 1",
        "einschliesslich Anfahrt",
        "und Kleinmaterial",
    ];
    let first = top - 1.8 * size;
    for (n, l) in lines.iter().enumerate() {
        t += &show(plain, size, 70.0, first - lead * n as f32, l);
    }
    let values = first - lead;
    t += &show(plain, size, 40.0, values, "1");
    t += &right(plain, size, 380.0, values, "1");
    t += &right(plain, size, 460.0, values, "250,00");
    t += &right(plain, size, 540.0, values, "250,00");
    let second = first - lead * 4.0 - 0.6 * size;
    t += &show(plain, size, 40.0, second, "2");
    t += &show(plain, size, 70.0, second, "Ersatzteil Pumpe");
    t += &right(plain, size, 380.0, second, "1");
    t += &right(plain, size, 460.0, second, "120,00");
    t += &right(plain, size, 540.0, second, "120,00");
    let bottom = second - 0.6 * size;
    t += &rule(v, 40.0, 540.0, bottom, size);
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want.table(
        rows_of(&[
            ["Pos", "Beschreibung", "Menge", "Preis", "Betrag"],
            ["1", &lines.join(" "), "1", "250,00", "250,00"],
            ["2", "Ersatzteil Pumpe", "1", "120,00", "120,00"],
        ]),
        "table",
    );
    s
}

/// A vertical rule from `y0` to `y1` at `x`.
fn vrule(x: f32, y0: f32, y1: f32) -> String {
    format!("0 0 0 RG 0.5 w {x} {y0} m {x} {y1} l S\n")
}

/// A table in a grid of drawn rules (see [`grid_of_lines`]).
struct Grid<'a> {
    /// The start of each column, and the right end of the last.
    xs: &'a [f32],
    x_end: f32,
    titles: &'a [&'a str],
    /// Each body row's cells, each its lines (an empty line is a line without text).
    rows: &'a [&'a [&'a [&'a str]]],
    /// The rows of the table read as a row for each line of a drawn row (its lines are records
    /// of their own); else each drawn row as one row, a cell's lines joined with spaces.
    parted: bool,
    /// A column whose cell spans the first two body rows: no rule parts them there, and the
    /// second row has no cell of its own in it.
    merged: Option<usize>,
    /// Columns (by their place in `xs`) bounded by breaks in the horizontal rules only, with no
    /// vertical rule.
    breaks_only: &'a [usize],
    /// How far the joints of the rules under the second body row stand from those above (the
    /// segments of one rule touch, each starting a hair before the last ends).
    shift: f32,
    /// A last row of one cell across the table (only the frame bounds it): its text at the left
    /// and a short text flush right (none when empty).
    note: Option<(&'a str, &'a str)>,
}

/// The page of `g`: a bold header row, then one drawn row for each body row, each cell's lines
/// one under the other from the row's first line; and the table it reads as.
fn grid_of_lines(v: &Variant, g: &Grid<'_>) -> (Sheet, Vec<Vec<String>>) {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let top = 600.0;
    let lead = 1.2 * size;
    let pad = 0.4 * size;
    let mut t = String::new();
    let mut y = top;
    let mut ys = vec![y];
    // A title of several lines (parted by a line feed) stands one line under the other.
    let title_lines = g
        .titles
        .iter()
        .map(|h| h.lines().count())
        .max()
        .unwrap_or(1)
        .max(1);
    for (x, h) in g.xs.iter().zip(g.titles).filter(|(_, h)| !h.is_empty()) {
        for (n, l) in h.lines().enumerate() {
            t += &show(bold, size, x + pad, y - 1.3 * size - lead * n as f32, l);
        }
    }
    y -= 1.8 * size + lead * (title_lines - 1) as f32;
    ys.push(y);
    let mut table = vec![
        g.titles
            .iter()
            .map(|h| h.lines().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>(),
    ];
    for (r, row) in g.rows.iter().enumerate() {
        let first = y - 1.3 * size;
        let lines = row.iter().map(|c| c.len()).max().unwrap_or(1);
        for (c, (x, cell)) in g.xs.iter().zip(row.iter()).enumerate() {
            if g.merged == Some(c) && r == 1 {
                continue;
            }
            for (n, l) in cell.iter().enumerate() {
                if !l.is_empty() {
                    t += &show(plain, size, x + pad, first - lead * n as f32, l);
                }
            }
        }
        y = first - lead * (lines - 1) as f32 - 0.7 * size;
        ys.push(y);
        let cells = row
            .iter()
            .enumerate()
            .filter(|(c, _)| !(g.merged == Some(*c) && r == 1));
        if g.parted {
            for n in 0..lines {
                table.push(
                    cells
                        .clone()
                        .map(|(_, c)| c.get(n).copied().unwrap_or("").to_string())
                        .collect(),
                );
            }
        } else {
            table.push(
                cells
                    .map(|(_, c)| {
                        c.iter()
                            .filter(|l| !l.is_empty())
                            .copied()
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect(),
            );
        }
    }
    // The row of one cell across the table, under the others.
    let inner_bottom = y;
    if let Some((left, right_text)) = g.note {
        let first = y - 1.3 * size;
        t += &show(plain, size, g.xs[0] + pad, first, left);
        if !right_text.is_empty() {
            t += &right(plain, size, g.x_end - pad, first, right_text);
        }
        y = first - 0.7 * size;
        ys.push(y);
        table.push(vec![format!("{left} {right_text}").trim().to_string()]);
    }
    // The horizontal rules, as segments broken at every column bound.
    let mut bounds: Vec<f32> = g.xs.to_vec();
    bounds.push(g.x_end);
    for (n, &h) in ys.iter().enumerate() {
        let shift = if n >= 3 { g.shift } else { 0.0 };
        let last = bounds.len() - 2;
        for (c, w) in bounds.windows(2).enumerate() {
            // The rule between the first two body rows leaves out the merged column.
            if n == 2 && g.merged == Some(c) {
                continue;
            }
            let a = if c == 0 { w[0] } else { w[0] + shift - 0.01 };
            let b = if c == last { w[1] } else { w[1] + shift };
            t += &rule(v, a, b, h, size);
        }
    }
    let bottom = *ys.last().unwrap_or(&top);
    for (c, x) in bounds.iter().enumerate() {
        if !g.breaks_only.contains(&c) {
            // The inner rules stop over the row of one cell across the table.
            let frame = c == 0 || c + 1 == bounds.len();
            t += &vrule(*x, if frame { bottom } else { inner_bottom }, top);
        }
    }
    (sheet(v, t, 420.0, Some(Span { top, bottom })), table)
}

/// The kinds of a table whose cells join lines: each by the first word of its text, as the
/// region holds the words of one line together.
fn first_words(s: &mut Sheet) {
    for (t, _) in &mut s.want.kinds {
        if let Some(w) = t.split_whitespace().next() {
            *t = w.to_string();
        }
    }
}

/// B3: a timetable in a grid of drawn rules, each connection a drawn row of two lines, the
/// departure over the arrival, a time on each line, the date and the train on the first line and
/// the platform on one line only. The first drawn row's lines each fill half the band's cells,
/// and the ruled reader's band rule from before Task 29 cuts it in two rows (the platform in the
/// arrival's row); the second row's cells keep their lines (no line wraps into the next), so each
/// stop stays on the line of its time and the platform stands on the departure's line.
fn two_line_cells(v: &Variant) -> Sheet {
    let rows: [&[&[&str]]; 2] = [
        &[
            &["Musterstadt Hbf", "Beispielheim"],
            &["01.10.2026"],
            &["ab 08:15", "an 09:40"],
            &["", "4"],
            &["RE 4711"],
        ],
        &[
            &["Beispielheim", "Probedorf"],
            &["01.10.2026"],
            &["ab 09:52", "an 10:30"],
            &["2"],
            &["RB 4712"],
        ],
    ];
    let (mut s, table) = grid_of_lines(
        v,
        &Grid {
            xs: &[40.0, 160.0, 230.0, 300.0, 350.0],
            x_end: 420.0,
            titles: &["Halt", "Datum", "Zeit", "Gleis", "Zug"],
            rows: &rows,
            parted: false,
            merged: None,
            breaks_only: &[],
            shift: 0.0,
            note: None,
        },
    );
    let _ = table;
    // The first drawn row is cut in two rows by the ruled reader's band rule from before Task 29
    // (both its lines fill at least half of the band's cells), which the class A ruling keeps;
    // the second keeps its cells' lines, the platform on the departure's line.
    s.want.table(
        rows_of(&[
            ["Halt", "Datum", "Zeit", "Gleis", "Zug"],
            ["Musterstadt Hbf", "01.10.2026", "ab 08:15", "", "RE 4711"],
            ["Beispielheim", "", "an 09:40", "4", ""],
            [
                "Beispielheim Probedorf",
                "01.10.2026",
                "ab 09:52 an 10:30",
                "2",
                "RB 4712",
            ],
        ]),
        "table",
    );
    first_words(&mut s);
    s.want
        .cell_lines
        .push(vec!["Beispielheim".into(), "Probedorf".into()]);
    s.want
        .cell_lines
        .push(vec!["ab 09:52".into(), "an 10:30".into()]);
    s
}

/// Vision class A: a timetable in a grid of drawn rules, each connection a drawn row whose stop
/// cell lists its stops one under the other beside a time on the line of each stop, and a date,
/// a platform and a train on the first line. Each cell keeps its lines, so each stop stays on
/// the line of its time; the cells of one line are read as they stand.
fn paired_lines_in_a_grid(v: &Variant) -> Sheet {
    let rows: [&[&[&str]]; 2] = [
        &[
            &["Musterstadt Hbf", "Beispielheim", "Probedorf"],
            &["01.10.2026"],
            &["ab 08:15", "an 09:40", "an 10:30"],
            &["3"],
            &["RE 4711"],
        ],
        &[
            &["Probedorf", "Musterstadt Hbf"],
            &["02.10.2026"],
            &["ab 17:05", "an 18:20"],
            &["1"],
            &["RE 4712"],
        ],
    ];
    let (mut s, table) = grid_of_lines(
        v,
        &Grid {
            xs: &[40.0, 170.0, 220.0, 290.0, 340.0],
            x_end: 420.0,
            titles: &["Halt", "Datum", "Zeit", "Gleis", "Zug"],
            rows: &rows,
            parted: false,
            merged: None,
            breaks_only: &[],
            shift: 0.0,
            note: None,
        },
    );
    s.want.table(table, "table");
    first_words(&mut s);
    // Each cell keeps its lines; a value on one line stands on its line.
    for row in rows {
        let lines = row.iter().map(|c| c.len()).max().unwrap_or(1);
        for cell in row.iter() {
            let mut have: Vec<String> = cell.iter().map(|l| l.to_string()).collect();
            if have.len() == 1 && lines > 1 && have[0].is_empty() {
                continue;
            }
            // A one-line value on a later line stands after empty lines; a cell of the first
            // line alone has no break.
            while have.last().is_some_and(|l| l.is_empty()) {
                have.pop();
            }
            if !have.is_empty() {
                s.want.cell_lines.push(have);
            }
        }
    }
    s
}

/// Review Important 1: the timetable of [`paired_lines_in_a_grid`] with a date cell drawn over
/// its first two rows. The second row has no date cell of its own (the cell over it covers it):
/// parting its lines into rows would move its cells one column to the left, so both rows stay
/// one row each, their cells read as one text.
fn paired_lines_under_a_merged_cell(v: &Variant) -> Sheet {
    let rows: [&[&[&str]]; 2] = [
        &[
            &["Musterstadt Hbf", "Beispielheim"],
            &["01.10.2026"],
            &["ab 08:15", "an 09:40"],
            &["3", "4"],
            &["RE 4711"],
        ],
        &[
            &["Beispielheim", "Probedorf"],
            &[""],
            &["ab 09:52", "an 10:30"],
            &["2", "5"],
            &["RB 4712"],
        ],
    ];
    let (mut s, table) = grid_of_lines(
        v,
        &Grid {
            xs: &[40.0, 160.0, 230.0, 300.0, 350.0],
            x_end: 420.0,
            titles: &["Halt", "Datum", "Zeit", "Gleis", "Zug"],
            rows: &rows,
            parted: false,
            merged: Some(1),
            breaks_only: &[],
            shift: 0.0,
            note: None,
        },
    );
    s.want.table(table, "table");
    first_words(&mut s);
    s
}

/// Review Important 2: a ruled item table whose rows wrap: a label of two lines beside a start
/// date over an end date under a title of two lines (`Beginn` over `Ende`), and a description of
/// two lines beside a price over its discount, among cells of one line. Stacked values under a
/// stacked title are one record's attributes: each drawn row stays one row.
fn wrapped_text_beside_stacked_values(v: &Variant) -> Sheet {
    let rows: [&[&[&str]]; 2] = [
        &[
            &["1"],
            &["Beginn und Ende des", "Zeitraums"],
            &["01.10.", "31.10."],
            &["Tag"],
            &["30"],
        ],
        &[
            &["2"],
            &["Wartung der Heizungsanlage", "im Haus Musterweg 1"],
            &["12,50", "-10 %"],
            &["Stk"],
            &["2"],
        ],
    ];
    let (mut s, table) = grid_of_lines(
        v,
        &Grid {
            xs: &[40.0, 70.0, 220.0, 300.0, 350.0],
            x_end: 400.0,
            titles: &["Pos", "Leistung", "Beginn\nEnde", "Einheit", "Menge"],
            rows: &rows,
            parted: false,
            merged: None,
            breaks_only: &[],
            shift: 0.0,
            note: None,
        },
    );
    s.want.table(table, "table");
    first_words(&mut s);
    s
}

/// Re-review 1 (new Important): an item over its article number beside a unit price and a total,
/// each a net over a gross amount under a title of two lines: one record a drawn row, each cell
/// its two lines. Every line fills the band, so the ruled reader's band rule from before Task 29
/// cuts each drawn row in two rows (the article number with the gross amounts): a known class
/// (`known.rs`, Task 36a), not the wanted reading.
fn item_beside_net_over_gross(v: &Variant) -> Sheet {
    let rows: [&[&[&str]]; 2] = [
        &[
            &["Schraube M8", "Art.-Nr. 1001"],
            &["100,00", "119,00"],
            &["200,00", "238,00"],
        ],
        &[
            &["Mutter M8", "Art.-Nr. 1002"],
            &["50,00", "59,50"],
            &["150,00", "178,50"],
        ],
    ];
    let (mut s, table) = grid_of_lines(
        v,
        &Grid {
            xs: &[40.0, 220.0, 320.0],
            x_end: 420.0,
            titles: &["Artikel", "Preis netto\nbrutto", "Summe netto\nbrutto"],
            rows: &rows,
            parted: false,
            merged: None,
            breaks_only: &[],
            shift: 0.0,
            note: None,
        },
    );
    s.want.table(table, "table");
    first_words(&mut s);
    for row in rows {
        for cell in row {
            s.want
                .cell_lines
                .push(cell.iter().map(|l| l.to_string()).collect());
        }
    }
    s
}

/// Private shape S2 (vision class A on the ruled path): a ruled table one of whose drawn cells
/// holds two columns, bounded only by joints in the segments of every horizontal rule: from and
/// to, with an arrow in another font between their titles. The segments touch at the joints,
/// and the joints of the rules under the second body row stand 0.4 points off those above. Two
/// rows under them hold text across the joints (cells merged across). The joints bound the
/// columns of the rows whose text leaves a gutter there: the from and the to titles head columns
/// of their own, each with its value, the arrow a column of its own; the rows across them keep
/// one cell over the three. The arrow and the to title are one show, the joint in the white
/// space after the arrow: the show is cut there. A last row of one cell across the table, its
/// text far from the joints, stays one cell.
fn columns_bounded_by_rule_breaks(v: &Variant) -> Sheet {
    let k = v.k();
    let size = 8.0 * k;
    let across = "Hin und zurueck ueber Beispielheim nach Nord";
    let rows: [&[&[&str]]; 4] = [
        &[
            &["1"],
            &["2"],
            &["Musterstadt"],
            &[""],
            &["Beispielheim"],
            &["01.10.2026"],
            &["12,50"],
        ],
        &[
            &["2"],
            &["1"],
            &["Beispielheim"],
            &[""],
            &["Probedorf"],
            &["02.10.2026"],
            &["8,90"],
        ],
        &[
            &["3"],
            &["2"],
            &[across],
            &[""],
            &[""],
            &["03.10.2026"],
            &["20,00"],
        ],
        &[
            &["4"],
            &["1"],
            &[across],
            &[""],
            &[""],
            &["04.10.2026"],
            &["16,40"],
        ],
    ];
    let xs = [40.0, 80.0, 110.0, 210.0, 240.0, 340.0, 410.0];
    let (mut s, _) = grid_of_lines(
        v,
        &Grid {
            xs: &xs,
            x_end: 460.0,
            titles: &["Nr", "Kl", "Von", "", "", "Datum", "Preis"],
            rows: &rows,
            parted: false,
            merged: None,
            breaks_only: &[3, 4],
            shift: 0.4,
            note: Some(("Hinweis: Fahrt mit Umstieg", "Zone: 12")),
        },
    );
    // The arrow and the to title in one show, a space after the arrow and a kerned gap after
    // it holding the joint of the rules (as a writer sets them), the title over its column.
    let at = 600.0 - 1.3 * size;
    s.built.page.template += &format!(
        "BT /FB {size} Tf {} {at} Td [(\u{bb} ) -500 (Nach)] TJ ET\n",
        xs[4] - 0.95 * size
    );
    s.once.push("\u{bb}".into());
    let table = [
        rows_of(&[["Nr", "Kl", "Von", "\u{bb}", "Nach", "Datum", "Preis"]]),
        rows_of(&[[
            "1",
            "2",
            "Musterstadt",
            "",
            "Beispielheim",
            "01.10.2026",
            "12,50",
        ]]),
        rows_of(&[[
            "2",
            "1",
            "Beispielheim",
            "",
            "Probedorf",
            "02.10.2026",
            "8,90",
        ]]),
        rows_of(&[["3", "2", across, "03.10.2026", "20,00"]]),
        rows_of(&[["4", "1", across, "04.10.2026", "16,40"]]),
        rows_of(&[["Hinweis: Fahrt mit Umstieg Zone: 12"]]),
    ]
    .concat();
    s.want.table(table, "table");
    s
}

/// Re-review 3 (new Important 2): the grid of [`columns_bounded_by_rule_breaks`] with a last row
/// of one cell whose sentence runs across both joints of the rules, its words a plain word
/// space apart. A word space is no column gap: the sentence stays one cell, wherever the joints
/// fall in it.
fn a_sentence_across_rule_breaks(v: &Variant) -> Sheet {
    let sentence =
        "Hinweis: Diese Fahrt gilt nur am selben Tag und nur mit Umstieg in Beispielheim";
    let rows: [&[&[&str]]; 2] = [
        &[
            &["1"],
            &["2"],
            &["Musterstadt"],
            &["nach"],
            &["Beispielheim"],
            &["01.10.2026"],
            &["12,50"],
        ],
        &[
            &["2"],
            &["1"],
            &["Beispielheim"],
            &["nach"],
            &["Probedorf"],
            &["02.10.2026"],
            &["8,90"],
        ],
    ];
    let (mut s, mut table) = grid_of_lines(
        v,
        &Grid {
            xs: &[40.0, 80.0, 110.0, 210.0, 240.0, 340.0, 410.0],
            x_end: 460.0,
            titles: &["Nr", "Kl", "Von", "", "Nach", "Datum", "Preis"],
            rows: &rows,
            parted: false,
            merged: None,
            breaks_only: &[3, 4],
            shift: 0.0,
            note: Some((sentence, "")),
        },
    );
    if let Some(last) = table.last_mut() {
        *last = vec![sentence.to_string()];
    }
    s.want.table(table, "table");
    s
}

/// Review Minor 2, open: an item table without rules whose rows are a line of values over a
/// detail line, and a note of one piece under the last row, where its detail line would stand,
/// in the same face. The note is no part of the last row. Nothing on the page tells it from a
/// detail line, so it reads as one today: a known class (`known.rs`, Task 36a).
fn a_note_where_a_detail_line_would_stand(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 8.0 * k;
    let top = 600.0;
    let lead = 1.2 * size;
    let items = [
        ["Kaffeebecher", "Steingut, weiss", "2", "12,50"],
        ["Teekanne", "Glas, 1 Liter", "1", "34,56"],
        ["Untersetzer", "", "4", "7,80"],
    ];
    let note = "Lieferung frei Haus";
    let mut t = String::new();
    let mut table = Vec::new();
    for (n, [name, detail, qty, price]) in items.iter().enumerate() {
        let y = top - 2.0 * lead * n as f32;
        t += &show(plain, size, 40.0, y, name);
        if !detail.is_empty() {
            t += &show(plain, size, 40.0, y - lead, detail);
        }
        t += &right(plain, size, 420.0, y, qty);
        t += &right(plain, size, 540.0, y, price);
        table.push(vec![
            format!("{name} {detail}").trim().to_string(),
            qty.to_string(),
            price.to_string(),
        ]);
    }
    t += &show(plain, size, 40.0, top - 5.0 * lead, note);
    let bottom = top - 5.0 * lead;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want.table(table, "table");
    first_words(&mut s);
    s.want.paragraphs.push(note.into());
    s
}

/// Review Important 3: an item table without a header row whose names are bold over plain
/// detail lines, the rows a line apart (no blank between them): each detail line stays with the
/// name over it.
fn bold_names_over_detail_lines(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let top = 600.0;
    let lead = 1.2 * size;
    let items = [
        ["Kaffeebecher", "Steingut, weiss", "2", "12,50"],
        ["Teekanne", "Glas, 1 Liter", "1", "34,56"],
        ["Untersetzer", "Kork, rund", "4", "7,80"],
    ];
    let mut t = String::new();
    let mut table = Vec::new();
    for (n, [name, detail, qty, price]) in items.iter().enumerate() {
        let y = top - 2.0 * lead * n as f32;
        t += &show(bold, size, 40.0, y, name);
        t += &show(plain, size, 40.0, y - lead, detail);
        t += &right(plain, size, 420.0, y, qty);
        t += &right(plain, size, 540.0, y, price);
        table.push(vec![
            format!("{name} {detail}"),
            qty.to_string(),
            price.to_string(),
        ]);
    }
    let bottom = top - 6.0 * lead;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want.table(table, "table");
    s.want.kinds.retain(|(t, _)| !t.starts_with("Untersetzer"));
    s.want.kinds.push(("Untersetzer".into(), "table"));
    s.want.kinds.retain(|(t, _)| !t.starts_with("Kaffeebecher"));
    s.want.kinds.push(("Kaffeebecher".into(), "table"));
    s
}

/// The items of the stamp and picture shapes: a header and three rows of four columns without
/// rules, at `top`, a line apart. Gives the template and the table.
fn plain_items(v: &Variant, top: f32) -> (String, Vec<Vec<String>>) {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let lead = 1.25 * size;
    let rows = [
        ["Pos", "Artikel", "Menge", "Betrag"],
        ["1", "Beratung", "2", "1.000,00"],
        ["2", "Material", "1", "200,00"],
        ["3", "Versand", "1", "34,56"],
    ];
    let mut t = String::new();
    for (n, r) in rows.iter().enumerate() {
        let y = top - lead * n as f32;
        let f = if n == 0 { bold } else { plain };
        t += &show(f, size, 40.0, y, r[0]);
        t += &show(f, size, 80.0, y, r[1]);
        t += &right(f, size, 380.0, y, r[2]);
        t += &right(f, size, 540.0, y, r[3]);
    }
    (t, rows_of(&rows))
}

/// Review Important 4 (a): a stamp drawn over the rows of an item table without rules, over
/// more than one row and across its columns. It is no row's picture: the table keeps its
/// cells, and the stamp is a picture of its own.
fn a_stamp_over_an_item_table(v: &Variant) -> Sheet {
    let k = v.k();
    let top = 600.0;
    let (mut t, table) = plain_items(v, top);
    let lead = 1.25 * 8.0 * k;
    // Over rows 1 and 2, from the article column over the quantity column.
    t += &logo(150.0, top - 2.5 * lead, 260.0, 2.2 * lead);
    let bottom = top - 3.0 * lead;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want.table(table, "table");
    s.want.media_count = Some(1);
    s
}

/// Review Important 4 (a), ruled: the stamp over the rows of a table in a grid of drawn rules.
fn a_stamp_over_a_ruled_table(v: &Variant) -> Sheet {
    let rows: [&[&[&str]]; 3] = [
        &[&["1"], &["Beratung"], &["2"], &["1.000,00"]],
        &[&["2"], &["Material"], &["1"], &["200,00"]],
        &[&["3"], &["Versand"], &["1"], &["34,56"]],
    ];
    let (mut s, table) = grid_of_lines(
        v,
        &Grid {
            xs: &[40.0, 80.0, 300.0, 380.0],
            x_end: 540.0,
            titles: &["Pos", "Artikel", "Menge", "Betrag"],
            rows: &rows,
            parted: false,
            merged: None,
            breaks_only: &[],
            shift: 0.0,
            note: None,
        },
    );
    let size = 8.0 * v.k();
    // Over the first two body rows, across the article and the quantity columns.
    s.built.page.template += &logo(150.0, 600.0 - 5.0 * size, 200.0, 2.4 * size);
    s.want.table(table, "table");
    s.want.media_count = Some(1);
    s
}

/// Review Important 4 (b): a flat picture in each row of an item table without rules, across
/// the bound between the article and the quantity columns. In no one column, it is no cell's
/// picture: the table keeps its cells, the pictures stand apart.
fn pictures_across_a_column_bound(v: &Variant) -> Sheet {
    let k = v.k();
    let top = 600.0;
    let (mut t, table) = plain_items(v, top);
    let lead = 1.25 * 8.0 * k;
    for n in 1..4 {
        let y = top - lead * n as f32;
        t += &logo(120.0, y - 0.2 * lead, 260.0, 0.8 * lead);
    }
    let bottom = top - 3.0 * lead;
    let mut s = sheet(v, t, 420.0, Some(Span { top, bottom }));
    s.want.table(table, "table");
    s.want.media_count = Some(3);
    s
}

/// Review Important 4 (c): a picture from the last row of an item table down to a table of
/// other columns under it, past a gap wider than a blank row. One picture bridges no gap: both
/// tables keep their cells, and the picture stands apart.
fn a_picture_over_the_gap_to_the_next_table(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 8.0 * k;
    let top = 600.0;
    let lead = 1.25 * size;
    let (mut t, table) = plain_items(v, top);
    let last = top - 3.0 * lead;
    let next = last - 6.0 * size;
    let under = [
        ["Kennung", "Vorgang", "Wert"],
        ["7001", "Fahrgeld", "45,67"],
        ["7002", "Essensgeld", "8,90"],
    ];
    for (n, r) in under.iter().enumerate() {
        let y = next - lead * n as f32;
        let f = if n == 0 { bold } else { plain };
        t += &show(f, size, 40.0, y, r[0]);
        t += &show(f, size, 200.0, y, r[1]);
        t += &right(f, size, 470.0, y, r[2]);
    }
    // From over the last row's baseline to under the next table's header.
    t += &logo(250.0, next - 0.5 * size, 60.0, last - next + 1.5 * size);
    let bottom = next - 2.0 * lead;
    let mut s = sheet(v, t, 380.0, Some(Span { top, bottom }));
    s.want.table(table, "table");
    s.want.table(rows_of(&under), "table");
    s.want.media_count = Some(1);
    s
}

/// The items of [`picture_in_the_description`]: position, name, article code, quantity, price.
const CODED_ITEMS: [[&str; 5]; 3] = [
    ["1", "Kaffeebecher", "40211AB12C / X12Y3Z4567", "2", "12,50"],
    ["2", "Teekanne", "40212AB12C / X12Y3Z4568", "1", "34,56"],
    ["3", "Untersetzer", "40213AB12C / X12Y3Z4569", "4", "7,80"],
];

/// Private shape S3 (vision class C): a paragraph of three lines, a blank line, then the header
/// of an item table in the paragraph's face, size and pitch (not bold), a rule under the header
/// drawn as touching segments with joints at the column bounds, item rows of two lines (a name
/// over an article code) with a picture inside the description column over both lines, a
/// position flush right, a quantity and a price flush right on the first line, a closing rule,
/// and a total flush right under it. One table from the header to the closing rule, a row for
/// each item, the picture in its row's description cell; the paragraph keeps its lines and the
/// total stays after the table with its value.
fn picture_in_the_description(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 11.0 * k;
    let pitch = 12.0 * k;
    let (x_pos, x_desc, x_pic, x_qty, x_price) = (50.0, 64.0, 342.0, 506.0, 565.0);
    let para = [
        "Wir danken Ihnen fuer Ihre Bestellung und liefern Ihnen heute die folgenden Artikel aus",
        "unserem Lager in Musterstadt. Die Ware bleibt bis zur vollstaendigen Bezahlung unser",
        "Eigentum. Es gelten unsere allgemeinen Bedingungen, die Sie auf Wunsch gerne erhalten.",
    ];
    let mut t = String::new();
    let top = 640.0;
    for (n, l) in para.iter().enumerate() {
        t += &show(plain, size, 30.0, top - pitch * n as f32, l);
    }
    // A blank line drawn as a run of spaces, then the header at the paragraph's pitch.
    t += &show(plain, size, 30.0, top - pitch * 3.0, " ");
    let head = top - pitch * 4.0 - 1.0 * k;
    t += &show(plain, size, 32.0, head, "Pos.");
    t += &show(plain, size, x_desc, head, "Artikel (3)");
    t += &right(plain, size, x_qty, head, "Menge");
    t += &right(plain, size, x_price, head, "Preis");
    let under_head = head - 4.0 * k;
    for (a, b) in [
        (30.0, 62.1),
        (62.1, 468.7),
        (468.7, x_qty),
        (x_qty, x_price),
    ] {
        t += &rule(v, a, b, under_head, size);
    }
    let row_pitch = 35.0 * k;
    let mut y = under_head - 13.0 * k;
    let mut table = vec![vec![
        "Pos.".to_string(),
        "Artikel (3)".into(),
        "Menge".into(),
        "Preis".into(),
    ]];
    for [pos, name, code, qty, price] in CODED_ITEMS {
        t += &right(plain, size, x_pos, y, pos);
        t += &show(plain, size, x_desc, y, name);
        t += &show(plain, size, x_desc, y - 11.0 * k, code);
        t += &right(plain, size, x_qty, y, qty);
        t += &right(plain, size, x_price, y, price);
        // About 1.4 by 1 cm, from over the name to under the code.
        t += &logo(x_pic, y - 11.0 * k - 0.3 * size, 40.5, 27.0);
        table.push(vec![
            pos.to_string(),
            format!("{name} {code}"),
            qty.to_string(),
            price.to_string(),
        ]);
        y -= row_pitch;
    }
    let close = y + row_pitch - 11.0 * k - 11.0 * k;
    for (a, b) in [(30.0, x_qty), (x_qty, x_price)] {
        t += &rule(v, a, b, close, size);
    }
    let total = close - 13.0 * k;
    t += &right(bold, size, x_qty, total, "Summe");
    t += &right(bold, size, x_price, total, "55,36");
    t += &right(plain, size, x_price, total - 17.0 * k, "inkl. 19 % USt");
    // A neighbour under the shape stands a blank line under the total.
    let bottom = total - 1.5 * size;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    for item in CODED_ITEMS {
        s.want.row_media.push((item[1].to_string(), 1));
    }
    s.want.table(table, "table");
    s.want.media_count = Some(0);
    s.want.paragraphs.push(para.join(" "));
    s.want
        .order
        .extend(["Untersetzer".to_string(), "Summe".into(), "55,36".into()]);
    s.once.push("inkl. 19 % USt".into());
    s
}

/// The items of [`item_rows_under_an_info_block`]: position, name, article code, count, price.
const LISTED_ITEMS: [[&str; 5]; 3] = [
    ["1", "Coffee mug", "40211AB12C / X12Y3Z4567", "2", "12,50"],
    ["2", "Tea pot", "40212AB12C / X12Y3Z4568", "1", "34,56"],
    ["3", "Saucer", "40213AB12C / X12Y3Z4569", "4", "7,80"],
];

/// Task 31b, class S3: the item rows of [`picture_in_the_description`] on the page of a
/// letter. Over them stand an address on the left and a block of five short lines on the
/// right, on baselines of their own, so the page has a gutter between the descriptions and
/// the counts with blocks on both sides that do not line up; the lines of the paragraph run
/// across it. The header (not bold, in the paragraph's face and pitch) stands over a rule
/// drawn as touching pieces with joints at the column bounds; the rows of two lines end at a
/// closing rule, with a total flush right under it. The rows between the two rules are rows
/// of one table: each count and price reads in the row of its item, never after all items,
/// and the header is the table's header row, each title a cell of its own.
fn item_rows_under_an_info_block(v: &Variant) -> Sheet {
    item_rows_on_a_letter_page(v, false)
}

/// Task 31c: the page of [`item_rows_under_an_info_block`] as a second sheet of its list.
/// Over the rule in pieces stands no row of titles but a row carried over from the sheet
/// before: a label and an amount. The joints of the rule and the rows under it are the same.
/// The rows are still rows: each count and price is read with its item, never after all
/// items.
fn item_rows_under_a_carried_row(v: &Variant) -> Sheet {
    item_rows_on_a_letter_page(v, true)
}

/// The page of [`item_rows_under_an_info_block`]; with `carried`, the row over the rule in
/// pieces holds a label and an amount (see [`item_rows_under_a_carried_row`]) and no titles.
fn item_rows_on_a_letter_page(v: &Variant, carried: bool) -> Sheet {
    let k = v.k();
    let (plain, bold) = v.faces();
    let size = 11.0 * k;
    let pitch = 12.0 * k;
    let (x_pos, x_desc, x_pic, x_qty, x_price) = (50.0, 64.0, 342.0, 506.0, 565.0);
    let mut t = String::new();
    let address = ["Erika Example", "34 Mill Lane", "67890 Oakfield"];
    for (n, l) in address.iter().enumerate() {
        t += &show(plain, size, 30.0, 760.0 - 13.0 * k * n as f32, l);
    }
    let info = [
        "Order 4521",
        "Date 12 May 2026",
        "Client 7788",
        "Clerk Alex Sample",
        "Sheet 1 of 1",
    ];
    for (n, l) in info.iter().enumerate() {
        t += &right(plain, 9.0 * k, x_price, 766.0 - 10.0 * k * n as f32, l);
    }
    // The lines run across the page, under the block on the right too.
    let para = [
        "Thank you for your order. Today we send you the parts listed under these lines from our large",
        "store in Fairhaven. The parts stay ours until the whole sum has reached us, and our terms apply",
        "to this order as to every order before it. We send you a copy of them when you ask us for one.",
    ];
    let top = 640.0;
    for (n, l) in para.iter().enumerate() {
        t += &show(plain, size, 30.0, top - pitch * n as f32, l);
    }
    t += &show(plain, size, 30.0, top - pitch * 3.0, " ");
    let head = top - pitch * 4.0 - 1.0 * k;
    let titles = ["No.", "Article (3)", "Count", "Price"];
    let over = ["Carried over from sheet 1", "88,20"];
    if carried {
        t += &show(plain, size, x_desc, head, over[0]);
        t += &right(plain, size, x_price, head, over[1]);
    } else {
        t += &show(plain, size, 32.0, head, titles[0]);
        t += &show(plain, size, x_desc, head, titles[1]);
        t += &right(plain, size, x_qty, head, titles[2]);
        t += &right(plain, size, x_price, head, titles[3]);
    }
    let under_head = head - 4.0 * k;
    for (a, b) in [
        (30.0, 62.1),
        (62.1, 468.7),
        (468.7, x_qty),
        (x_qty, x_price),
    ] {
        t += &rule(v, a, b, under_head, size);
    }
    let row_pitch = 35.0 * k;
    let mut y = under_head - 13.0 * k;
    let mut table = vec![titles.iter().map(|c| c.to_string()).collect::<Vec<_>>()];
    for [pos, name, code, qty, price] in LISTED_ITEMS {
        t += &right(plain, size, x_pos, y, pos);
        t += &show(plain, size, x_desc, y, name);
        t += &show(plain, size, x_desc, y - 11.0 * k, code);
        t += &right(plain, size, x_qty, y, qty);
        t += &right(plain, size, x_price, y, price);
        t += &logo(x_pic, y - 11.0 * k - 0.3 * size, 40.5, 27.0);
        table.push(vec![
            pos.to_string(),
            format!("{name} {code}"),
            qty.to_string(),
            price.to_string(),
        ]);
        y -= row_pitch;
    }
    let close = y + row_pitch - 11.0 * k - 11.0 * k;
    for (a, b) in [(30.0, x_qty), (x_qty, x_price)] {
        t += &rule(v, a, b, close, size);
    }
    let total = close - 13.0 * k;
    t += &right(bold, size, x_qty, total, "Total");
    t += &right(bold, size, x_price, total, "55,36");
    t += &right(
        plain,
        size,
        x_price,
        total - 17.0 * k,
        "all prices in units",
    );
    // The page holds the letter only: no body text under the total, whose place beside or
    // under it would be read in another order at another text size.
    let mut built = crate::shapes::Built::new(&v.geo());
    built.page = crate::common::FormPage {
        template: t,
        ..crate::common::FormPage::default()
    };
    let mut s = Sheet::new(built);
    s.once = crate::axes::shown(&s.built.page.template);
    if carried {
        // The rows in order, each item before its own price and before the next item: a
        // table is not asked for (no titles), a read by columns is refused. The row carried
        // over is not in the chain: its label and its amount stand on the two sides of the
        // page's gutter, a label and its value apart (label/value groups, Task 26).
        let mut rows = Vec::new();
        for item in LISTED_ITEMS {
            rows.extend([item[1].to_string(), item[4].to_string()]);
        }
        s.want.orders.push(rows);
        s.want
            .order
            .extend(["Saucer".to_string(), "Total".into(), "55,36".into()]);
        return s;
    }
    for item in LISTED_ITEMS {
        s.want.row_media.push((item[1].to_string(), 1));
    }
    s.want.table(table, "table");
    s.want.media_count = Some(0);
    s.want.paragraphs.push(para.join(" "));
    s.want
        .order
        .extend(["Saucer".to_string(), "Total".into(), "55,36".into()]);
    s
}

/// Task 31b fix round 1 (C1): a totals block under one item row. The header and the item row
/// of [`item_rows_under_an_info_block`] on the same letter page, then, between two rules
/// drawn in the same three pieces, two lines of a label and its amount set flush right (the
/// labels end at a joint of the rules), and under the lower rule a small note on the left
/// beside the grand total, with a rule of one piece further down. The lines between the
/// rules are totals, no item rows: each label keeps its amount on its line, the grand total
/// keeps its amount, and no rule of the item rows takes the text under the lower rule.
fn totals_between_rules_in_pieces(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 11.0 * k;
    let (x_pos, x_desc, x_qty, x_price) = (50.0, 64.0, 506.0, 565.0);
    let mut t = String::new();
    // A block of lines alone at the head of the page, on the right, and a date line under it.
    t += &show(plain, size, 30.0, 690.0, "12 May 2026");
    for (n, l) in [
        "Order 4521",
        "Date 12 May 2026",
        "Client 7788",
        "Sheet 2 of 2",
    ]
    .iter()
    .enumerate()
    {
        t += &right(plain, 9.0 * k, x_price, 766.0 - 10.0 * k * n as f32, l);
    }
    let head = 640.0;
    let titles = ["No.", "Article (3)", "Count", "Price"];
    t += &show(plain, size, 32.0, head, titles[0]);
    t += &show(plain, size, x_desc, head, titles[1]);
    t += &right(plain, size, x_qty, head, titles[2]);
    t += &right(plain, size, x_price, head, titles[3]);
    let pieces = |t: &mut String, y: f32, ends: &[f32]| {
        for w in ends.windows(2) {
            *t += &rule(v, w[0], w[1], y, size);
        }
    };
    pieces(&mut t, head - 4.0 * k, &[30.0, 62.1, 468.7, x_qty, x_price]);
    let row = head - 17.0 * k;
    t += &right(plain, size, x_pos, row, "4");
    t += &show(plain, size, x_desc, row, "Bread box");
    t += &show(
        plain,
        size,
        x_desc,
        row - 11.0 * k,
        "40214AB12C / X12Y3Z4570",
    );
    t += &right(plain, size, x_qty, row, "1");
    t += &right(plain, size, x_price, row, "12,50");
    t += &logo(342.0, row - 11.0 * k - 0.3 * size, 40.5, 27.0);
    let three = [30.0, 265.4, x_qty, x_price];
    let upper = row - 24.0 * k;
    pieces(&mut t, upper, &three);
    let totals = [
        ("Subtotal", "12,50"),
        ("Shipping / handling of parts", "4,99"),
    ];
    for (n, (label, amount)) in totals.iter().enumerate() {
        let y = upper - 13.0 * k - 17.0 * k * n as f32;
        t += &right(plain, size, x_qty, y, label);
        t += &right(plain, size, x_price, y, amount);
    }
    let lower = upper - 34.0 * k;
    pieces(&mut t, lower, &three);
    let note = "The sum holds a share of 19% for goods: 2,79";
    t += &show(plain, 9.0 * k, 32.0, lower - 11.0 * k, note);
    t += &right(plain, size, x_qty, lower - 13.0 * k, "Grand total");
    t += &right(plain, size, x_price, lower - 13.0 * k, "17,49");
    t += &rule(v, 30.0, x_price, lower - 80.0 * k, size);
    t += &show(
        plain,
        9.0 * k,
        30.0,
        lower - 92.0 * k,
        "Harbor store Fairhaven, gate four",
    );
    let mut built = crate::shapes::Built::new(&v.geo());
    built.page = crate::common::FormPage {
        template: t,
        ..crate::common::FormPage::default()
    };
    let mut s = Sheet::new(built);
    s.once = crate::axes::shown(&s.built.page.template);
    for (label, amount) in totals.iter().chain(&[("Grand total", "17,49")]) {
        s.want
            .lines_joined
            .push(((*label).to_string(), (*amount).to_string()));
    }
    s.want.lines_apart.push(("12,50".into(), "Shipping".into()));
    s.want
        .lines_apart
        .push(("4,99".into(), "Grand total".into()));
    s.want.order.extend([
        "Subtotal".to_string(),
        "Shipping".into(),
        "Grand total".into(),
    ]);
    s.want.apart.push((note.into(), "Grand total".into()));
    s
}

/// The shapes of the step 5 classes.
pub(crate) fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            name: "item table between two rules with a picture column",
            axes: RULED,
            draw: |v| (picture_items(v), None),
        },
        Shape {
            name: "description wrapping to four lines",
            axes: DOC_NEXT,
            draw: |v| (wrapped_description(v), None),
        },
        Shape {
            name: "cells of several lines that pair up line by line in a ruled grid",
            axes: RULED,
            draw: |v| (paired_lines_in_a_grid(v), None),
        },
        Shape {
            name: "two-line cells with one value on the second line",
            axes: RULED,
            draw: |v| (two_line_cells(v), None),
        },
        Shape {
            name: "cells of several lines under a cell merged over two rows",
            axes: RULED,
            draw: |v| (paired_lines_under_a_merged_cell(v), None),
        },
        Shape {
            name: "wrapped text beside stacked values in a ruled grid",
            axes: RULED,
            draw: |v| (wrapped_text_beside_stacked_values(v), None),
        },
        Shape {
            name: "an item beside net over gross amounts under titles of two lines",
            axes: RULED,
            draw: |v| (item_beside_net_over_gross(v), None),
        },
        Shape {
            name: "columns bounded by breaks in the rules",
            axes: SOLID,
            draw: |v| (columns_bounded_by_rule_breaks(v), None),
        },
        Shape {
            name: "a sentence across the breaks in the rules",
            axes: SOLID,
            draw: |v| (a_sentence_across_rule_breaks(v), None),
        },
        Shape {
            name: "a note where the detail line of the last item would stand",
            axes: DOC_NEXT,
            draw: |v| (a_note_where_a_detail_line_would_stand(v), None),
        },
        Shape {
            name: "bold item names over plain detail lines without a header",
            axes: DOC_NEXT,
            draw: |v| (bold_names_over_detail_lines(v), None),
        },
        Shape {
            name: "a stamp over an item table without rules",
            axes: DOC_NEXT,
            draw: |v| (a_stamp_over_an_item_table(v), None),
        },
        Shape {
            name: "a stamp over a ruled item table",
            axes: RULED,
            draw: |v| (a_stamp_over_a_ruled_table(v), None),
        },
        Shape {
            name: "flat pictures across a column bound in item rows",
            axes: DOC_NEXT,
            draw: |v| (pictures_across_a_column_bound(v), None),
        },
        Shape {
            name: "a picture over the gap to the next table",
            axes: DOC_NEXT,
            draw: |v| (a_picture_over_the_gap_to_the_next_table(v), None),
        },
        Shape {
            name: "item rows with a picture in the description column",
            axes: LISTED,
            draw: |v| (picture_in_the_description(v), None),
        },
        Shape {
            name: "totals between two rules in pieces under one item row",
            axes: LISTED,
            draw: |v| (totals_between_rules_in_pieces(v), None),
        },
        Shape {
            name: "item rows under a rule in pieces on a page with a block of lines on the right",
            axes: LISTED,
            draw: |v| (item_rows_under_an_info_block(v), None),
        },
        Shape {
            name: "item rows under a rule in pieces and a row carried over, on a page with a block of lines on the right",
            axes: LISTED,
            draw: |v| (item_rows_under_a_carried_row(v), None),
        },
    ]
}
