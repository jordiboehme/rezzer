// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The layout classes of the region engine, each a shape
//! of its own on a business page, named by its class. All text and values are invented.

use crate::axes::{Axes, Look, Scale, Shape, Sheet, Variant};
use crate::common::{font_width, logo, show};
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

/// An item table between two rules, one above the header and one under
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
/// and the band rule of the older ruled reader cuts it in two rows (the platform in the
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
    // The first drawn row is cut in two rows by the band rule of the older ruled reader
    // (both its lines fill at least half of the band's cells), which the class keeps;
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

/// A timetable in a grid of drawn rules, each connection a drawn row whose stop
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

/// The timetable of [`paired_lines_in_a_grid`] with a date cell drawn over
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

/// A ruled item table whose rows wrap: a label of two lines beside a start
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

/// An item over its article number beside a unit price and a total,
/// each a net over a gross amount under a title of two lines: one record a drawn row, each cell
/// its two lines. Every line fills the band, so the band rule of the older ruled reader
/// cuts each drawn row in two rows (the article number with the gross amounts): a known class
/// (`known.rs`, kept after 0.1.0), not the wanted reading.
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

/// A private shape: a ruled table one of whose drawn cells
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

/// The grid of [`columns_bounded_by_rule_breaks`] with a last row
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

/// An item table without rules whose rows are a line of values over a
/// detail line, and a note of one piece under the last row, where its detail line would stand,
/// in the same face. The note is no part of the last row. Nothing on the page tells it from a
/// detail line, so it reads as one today: a known class (`known.rs`, kept after 0.1.0).
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

/// An item table without a header row whose names are bold over plain
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

/// A stamp drawn over the rows of an item table without rules, over
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

/// The stamp over the rows of a table in a grid of drawn rules.
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

/// A flat picture in each row of an item table without rules, across
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

/// A picture from the last row of an item table down to a table of
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

/// A private shape: a paragraph of three lines, a blank line, then the header
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

/// The item rows of [`picture_in_the_description`] on the page of a
/// letter. Over them stand an address on the left and a block of five short lines on the
/// right, on baselines of their own, so the page has a gutter between the descriptions and
/// the counts with blocks on both sides that do not line up; the lines of the paragraph run
/// across it. The header (not bold, in the paragraph's face and pitch) stands over a rule
/// drawn as touching pieces with joints at the column bounds; the rows of two lines end at a
/// closing rule, with a total flush right under it. The rows between the two rules are rows
/// of one table: each count and price reads in the row of its item, never after all items,
/// and the header is the table's header row, each title a cell of its own.
fn item_rows_under_an_info_block(v: &Variant) -> Sheet {
    item_rows_on_a_letter_page(v, Over::Titles)
}

/// The page of [`item_rows_under_an_info_block`] as a second sheet of its list.
/// Over the rule in pieces stands no row of titles but a row carried over from the sheet
/// before: a label and an amount. The joints of the rule and the rows under it are the same.
/// The rows are still rows: each count and price is read with its item, never after all
/// items.
fn item_rows_under_a_carried_row(v: &Variant) -> Sheet {
    item_rows_on_a_letter_page(v, Over::Carried)
}

/// What stands right over the rule in pieces of [`item_rows_on_a_letter_page`].
#[derive(Clone, Copy, PartialEq)]
enum Over {
    /// The titles of the rows.
    Titles,
    /// A row carried over: a label over the names and its amount over the prices, in the
    /// face and size of the rows.
    Carried,
    /// A heading in bold, two points larger than the rows, with a date in the rows' face
    /// flush right at the edge of the prices. The heading starts at the page margin, where
    /// the position numbers of the rows then start too (`true`), or over the names.
    Heading(bool),
}

/// The page of [`item_rows_under_a_carried_row`]
/// with a heading where the row carried over stood: a bold line and, flush right at the edge
/// of the prices, a date, on one baseline right over the rule in pieces. Each
/// stands at an edge that the rows under the rule share, and one reads as a value: all that
/// a row carried over shows. But a heading is set in another face than the rows, and a date
/// is no value of a column of prices. The line is no row of the items: the heading stands
/// in no block with an item, and the date in none with a price.
fn item_rows_under_a_heading(v: &Variant, over: Over) -> Sheet {
    item_rows_on_a_letter_page(v, over)
}

/// The page of [`item_rows_under_an_info_block`] with `over` right over the rule in pieces
/// in the place of the titles (see [`item_rows_under_a_carried_row`] and
/// [`item_rows_under_a_heading`]).
fn item_rows_on_a_letter_page(v: &Variant, over: Over) -> Sheet {
    let carried = over != Over::Titles;
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
    let kind = over;
    let over = match kind {
        Over::Heading(_) => ["Order items", "12.05.2026"],
        _ => ["Carried over from sheet 1", "88,20"],
    };
    let margin = kind == Over::Heading(true);
    if carried {
        let (face, at, x) = match kind {
            Over::Heading(margin) => (bold, 13.0 * k, if margin { 30.0 } else { x_desc }),
            _ => (plain, size, x_desc),
        };
        t += &show(face, at, x, head, over[0]);
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
        if margin {
            t += &show(plain, size, 30.0, y, pos);
        } else {
            t += &right(plain, size, x_pos, y, pos);
        }
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
    if matches!(kind, Over::Heading(_)) {
        // The heading before the items and in no block with one; its date in no block with
        // a price. The rows in order, each item before its own price.
        let mut rows = vec![over[0].to_string()];
        for item in LISTED_ITEMS {
            s.want
                .apart
                .push((over[0].to_string(), item[1].to_string()));
            s.want
                .apart
                .push((over[1].to_string(), item[4].to_string()));
            rows.extend([item[1].to_string(), item[4].to_string()]);
        }
        s.want.orders.push(rows);
        s.want
            .order
            .extend(["Saucer".to_string(), "Total".into(), "55,36".into()]);
        return s;
    }
    if carried {
        // The rows in order, each item before its own price and before the next item: a
        // table is not asked for (no titles), a read by columns is refused. The row carried
        // over stands before them, its amount right after its label (the row over the rule is read whole: the
        // two stand on the two sides of the page's gutter, and the block on the right stood
        // between them).
        s.want.together.push(over.map(String::from).to_vec());
        let mut rows: Vec<String> = over.map(String::from).to_vec();
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

/// A totals block under one item row. The header and the item row
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

/// The rows of [`labels_with_a_shared_value_column`]: a label and its value.
const VALUE_COLUMN: [(&str, &str); 5] = [
    ("Order", "4711"),
    ("Customer reference", "20481"),
    ("Depot", "D-17"),
    ("Ticket", "QX-2026-0042"),
    ("Parcels", "12"),
];

/// A1: five lines of a label and its value, the values at one left edge. One label
/// is wider than the others and ends a word space before the value column, so its line has no
/// gap that parts it: the value column's shared left edge parts it, never the gap. Each label
/// reads with its own value.
fn labels_with_a_shared_value_column(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let pitch = 1.3 * size;
    let (x0, top) = (40.0, 640.0);
    let widest = VALUE_COLUMN
        .iter()
        .map(|(l, _)| font_width(plain, l, size))
        .fold(0.0f32, f32::max);
    let x_value = x0 + widest + font_width(plain, " ", size);
    let mut t = String::new();
    for (n, (label, value)) in VALUE_COLUMN.iter().enumerate() {
        let y = top - pitch * n as f32;
        t += &show(plain, size, x0, y, label);
        t += &show(plain, size, x_value, y, value);
    }
    let bottom = top - pitch * (VALUE_COLUMN.len() - 1) as f32;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    for (label, value) in VALUE_COLUMN {
        s.built.fields.push((label, Ok(Some(value))));
    }
    // The lines are a block of labels and values: each row a label and its value, the line
    // of the widest label cut at the left edge the values share.
    s.want.kinds.push(("Order".into(), "label_value"));
    s.want.kinds.push(("Parcels".into(), "label_value"));
    s.want.tables.push(
        VALUE_COLUMN
            .iter()
            .map(|(l, v)| vec![l.to_string(), v.to_string()])
            .collect(),
    );
    s
}

/// A2: two lines of small labels, each over the line of its values, with no rules.
/// The values start under their labels, a little indented; the last label has no value under
/// it. A value belongs to the label it stands under (their widths overlap), and the label
/// with nothing under it keeps an empty value: no value moves to it.
fn two_labels_over_their_values(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let (small, size) = (7.5 * k, 9.0 * k);
    let (x0, x1, top) = (40.0, 200.0, 640.0);
    let indent = 0.3 * size;
    // The baselines: a value 1.2 sizes under its labels, the next labels 1.7 sizes lower.
    let ys = [top, top - 1.2 * size, top - 2.9 * size, top - 4.1 * size];
    let t = show(plain, small, x0, ys[0], "Member code")
        + &show(plain, small, x1, ys[0], "Issued on / at")
        + &show(plain, size, x0 + indent, ys[1], "20481")
        + &show(plain, size, x1 + indent, ys[1], "03.04.2026 / 10:30")
        + &show(plain, small, x0, ys[2], "Policy number")
        + &show(plain, small, x1, ys[2], "Earlier name")
        + &show(plain, size, x0 + indent, ys[3], "12345678A012");
    let bottom = ys[3];
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    for (label, value) in [
        ("Member code", Some("20481")),
        ("Issued on / at", Some("03.04.2026 / 10:30")),
        ("Policy number", Some("12345678A012")),
        ("Earlier name", None),
    ] {
        s.built.fields.push((label, Ok(value)));
    }
    s.want.kinds.push(("Member code".into(), "label_value"));
    s.want.kinds.push(("Earlier name".into(), "label_value"));
    s
}

/// H1: a column of field rows in the right half of a letter page, with no
/// rules. Small labels in a proportional face stand over values in a larger monospaced face,
/// the values indented a little. The first value is padded with blanks as far as the second
/// value, so its piece reaches under the second label, but its ink does not. The second row
/// has one value under two labels, three long labels follow with no value under them, and one
/// more label with its value stands alone further down the column.
fn padded_values_under_labels_in_a_column(v: &Variant) -> Sheet {
    /// The two values of the first row as a print run sets them: one string, the first value
    /// padded with blanks to the place of the second.
    const PADDED_ROW: &str = "20481         03.04.2026 / 10:30";
    let k = v.k();
    let (plain, _) = v.faces();
    let (small, size) = (8.6 * k, 9.7 * k);
    // The monospaced face: every glyph 0.6 sizes wide.
    let glyph = 0.6 * size;
    let (x0, top) = (324.6, 700.0);
    let indent = 0.32 * size;
    // A row of labels every 2.7 sizes, its values 1.12 sizes under it.
    let (row, drop) = (2.7 * size, 1.12 * size);
    let y = |n: usize| top - row * n as f32;
    // The second value starts 14 glyphs right of the first, the second label a little left of
    // it; the first value is five digits and nine blanks.
    let x1 = x0 + indent + 14.0 * glyph;
    let t = show(plain, small, x0, y(0), "Staff-No.")
        + &show(
            plain,
            small,
            x1 - indent,
            y(0),
            "Department-/Registration number",
        )
        + &show("FD", size, x0 + indent, y(0) - drop, PADDED_ROW)
        + &show(plain, small, x0, y(1), "Membership number")
        + &show(plain, small, x0 + 14.4 * size, y(1), "Earlier name")
        + &show("FD", size, x0 + indent, y(1) - drop, "12345678A012")
        + &show(
            plain,
            small,
            x0,
            y(2),
            "Member of a pension scheme [see note 4]",
        )
        + &show(
            plain,
            small,
            x0,
            y(3),
            "Paid by another office [see note 5]",
        )
        + &show(plain, small, x0, y(4), "Leave without pay [see note 6]")
        + &show(plain, small, x0, y(7), "Contribution group")
        + &show("FD", size, x0 + indent, y(7) - drop, "11110000");
    let bottom = y(7) - drop;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    // The padded line is drawn as one string and read as its two values.
    s.once.retain(|t| t != PADDED_ROW);
    s.once.extend(
        PADDED_ROW
            .split("  ")
            .filter(|t| !t.is_empty())
            .map(|t| t.trim().to_string()),
    );
    for (label, value) in [
        ("Staff-No.", Some("20481")),
        (
            "Department-/Registration number",
            Some("03.04.2026 / 10:30"),
        ),
        ("Membership number", Some("12345678A012")),
        ("Earlier name", None),
        ("Contribution group", Some("11110000")),
    ] {
        s.built.fields.push((label, Ok(value)));
    }
    s.want.kinds.push(("Staff-No.".into(), "label_value"));
    s.want.kinds.push(("Earlier name".into(), "label_value"));
    s.want
        .kinds
        .push(("Contribution group".into(), "label_value"));
    // No long label takes the value of another row.
    for long in ["Member of a pension scheme", "Leave without pay"] {
        for value in ["12345678A012", "11110000"] {
            s.want.no_pairs.push((long.into(), value.into()));
        }
    }
    s
}

/// H2: the head block at the top right of a letter page, three lines set
/// close under each other (0.9 sizes apart) and flush right. The first is one padded string
/// in the monospaced face (a reference, blanks, a number), kept as a line: the number is
/// another cell, not more of the reference. The second and the third are a
/// small label that ends in a colon with its value in the monospaced face to the right of it;
/// the last value is one digit at the right edge, far from its label. A column of field rows
/// (as in [`padded_values_under_labels_in_a_column`]) starts under the block.
fn colon_labels_in_a_head_block(v: &Variant) -> Sheet {
    const HEAD: &str = "REF: ABC/XYZ   20481/55012/77310";
    let k = v.k();
    let (plain, _) = v.faces();
    let (small, size) = (8.6 * k, 9.7 * k);
    let glyph = 0.6 * size;
    // The right edge of the block, and the left edge of its two labels: where the number of
    // the first line starts, less a third of a size.
    let end = 548.9;
    let labels = end - 17.0 * glyph - 0.32 * size;
    let top = 760.0;
    let line = 0.9 * size;
    let mono = |text: &str, y: f32| show("FD", size, end - glyph * text.len() as f32, y, text);
    let (x0, indent) = (324.6, 0.32 * size);
    // The field rows under the block: 3.4 sizes under its last line.
    let row = top - 2.0 * line - 3.4 * size;
    let drop = 1.12 * size;
    let t = mono(HEAD, top)
        + &show(plain, small, labels, top - line, "Sheet:")
        + &mono("03.04.2026", top - line)
        + &show(plain, small, labels, top - 2.0 * line, "Pages:")
        + &mono("4", top - 2.0 * line)
        + &show(plain, small, x0, row, "Staff-No.")
        + &show(plain, small, x0 + 14.0 * glyph, row, "Department number")
        + &show("FD", size, x0 + indent, row - drop, "66120")
        + &show(
            "FD",
            size,
            x0 + indent + 14.0 * glyph,
            row - drop,
            "17.08.2026",
        )
        + &show(plain, small, x0, row - 2.7 * size, "Membership number")
        + &show(
            "FD",
            size,
            x0 + indent,
            row - 2.7 * size - drop,
            "12345678A012",
        );
    let bottom = row - 2.7 * size - drop;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    s.once.retain(|t| t != HEAD);
    s.once
        .extend(["REF:", "ABC/XYZ", "20481/55012/77310"].map(String::from));
    for (label, value) in [
        ("Staff-No.", "66120"),
        ("Department number", "17.08.2026"),
        ("Membership number", "12345678A012"),
    ] {
        s.built.fields.push((label, Ok(Some(value))));
    }
    // The two lines of a label and its value are one list of pairs, each label with the
    // value of its own line. The first line is kept as a line: its first piece is a label
    // with a value already, and the number after the gap is another cell, which is no more
    // of that value and no value of a label under it.
    s.want.table(
        rows_of(&[["Sheet:", "03.04.2026"], ["Pages:", "4"]]),
        "label_value",
    );
    for label in ["REF:", "Sheet:", "Pages:"] {
        s.want
            .no_pairs
            .push((label.into(), "20481/55012/77310".into()));
    }
    s.want
        .no_pairs
        .push(("REF:".into(), "ABC/XYZ 20481/55012/77310".into()));
    s.want.no_pairs.push(("Sheet:".into(), "4".into()));
    s.want.no_pairs.push(("Pages:".into(), "03.04.2026".into()));
    s.want.no_pairs.push(("Pages:".into(), "66120".into()));
    s
}

/// H3: one line of a print run in the monospaced face: a date, a wide
/// blank, then a label that ends with a colon and its value two blanks after it. The label
/// reads with its value; the date is no label of anything and stands for itself.
fn a_date_beside_a_colon_label_on_one_line(v: &Variant) -> Sheet {
    const LINE: &str = "03.04.2026                         Sheet:  4";
    let size = 9.7 * v.k();
    let top = 640.0;
    let t = show("FD", size, 40.0, top, LINE);
    // The line stands alone: what follows it starts four sizes lower. (Right over a table
    // without rules, 2.4 sizes from its titles, the line is taken for a row of it and the
    // table is lost to paragraphs, with and without the rule of this shape: another class.)
    let bottom = top - 4.0 * size;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    s.once.retain(|t| t != LINE);
    s.once
        .extend(["03.04.2026", "Sheet:", "4"].map(String::from));
    s.built.fields.push(("Sheet:", Ok(Some("4"))));
    s.want.kinds.push(("Sheet:".into(), "label_value"));
    s.want.no_pairs.push(("03.04.2026".into(), "Sheet:".into()));
    s.want.no_pairs.push(("03.04.2026".into(), "4".into()));
    s
}

/// Lines of a listing in the monospaced
/// face, each a label, blanks up to a column, a colon and the value. The writer sets a line
/// whose label is padded in two runs, the label with one blank and then the colon with the
/// value (`Currency ` and `: EUR`), and a line whose label fills the column in one run
/// (`Profile in use : basic-2017`, `Global number: 4000001123452 (GLN)`). The colon that
/// opens the second run says the first is a label as a colon at its end does when the line
/// next to it has its colon in the same column, as the lines of a block of the listing have
/// every line of the two blocks reads as a label and its value. The last
/// line is alone, with no line to share its column of colons: it is a pair by its value, a
/// date.
fn fixed_width_colon_lines(v: &Variant) -> Sheet {
    let size = 9.75 * v.k();
    let pitch = 11.25 * v.k();
    // The advance of a character of the monospaced face.
    let step = 0.6 * size;
    let top = 640.0;
    let mut t = String::new();
    // Each line: its label, the column of its colon in characters when the label is padded,
    // and the value.
    let blocks: [&[(&str, Option<usize>, &str)]; 3] = [
        &[
            ("Profile in use :", None, "basic-2017"),
            ("Currency", Some(15), "EUR"),
        ],
        &[
            ("Number", Some(13), "549910"),
            ("Global number:", None, "4000001123452 (GLN)"),
        ],
        &[("Delivery date", Some(30), "05.03.2018")],
    ];
    let mut y = top;
    let mut table: Vec<Vec<String>> = Vec::new();
    for block in blocks {
        for &(label, column, value) in block {
            if let Some(column) = column {
                let x = 40.0 + step * column as f32;
                t += &show("FD", size, 40.0, y, &format!("{label} "));
                t += &show("FD", size, x, y, &format!(": {value}"));
                table.push(vec![format!("{label} :"), value.to_string()]);
            } else {
                t += &show("FD", size, 40.0, y, &format!("{label} {value}"));
                table.push(vec![label.to_string(), value.to_string()]);
            }
            y -= pitch;
        }
        y -= pitch;
    }
    let bottom = y - 2.0 * size;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    // One list of pairs: the blocks stand a blank line apart at one left edge.
    s.want.table(table, "label_value");
    s
}

/// Shows each of `parts` as a text object of its own, one after the other from `(x, y)`: the
/// touching runs a form template sets a label in, word by word.
fn show_parts(font: &str, size: f32, x: f32, y: f32, parts: &[&str]) -> String {
    let mut out = String::new();
    let mut at = x;
    for part in parts {
        out += &show(font, size, at, y, part);
        at += font_width(font, part, size);
    }
    out
}

/// The words of `text`, each with the blank after it: the parts [`show_parts`] sets a label in.
fn word_parts(text: &str) -> Vec<&str> {
    text.split_inclusive(' ').collect()
}

/// H1 as a print run draws it: the field rows of
/// [`padded_values_under_labels_in_a_column`], but every line of values is one bold monospaced
/// run that starts at the left margin of the page with blanks as far as the column, each label
/// is set word by word in runs that touch, and an address block in the monospaced face stands
/// left of the column under a small sender line. Four long labels with no value stand between
/// the rows and the last field, which stands alone in the next row.
///
/// With `larger` the values are set as the owner's form sets them, in 9.7 points bold over a
/// body of 7: the bold value line is then a heading to the
/// levels of the page.
fn value_lines_from_the_page_margin(v: &Variant, larger: bool) -> Sheet {
    /// The blanks from the first value to the second one.
    const SECOND: usize = 14;
    // The blanks from the margin to the column of values: the column starts at the same
    // place at both sizes (49 blanks of 9.7 points, as on the owner's form, 67 of 7).
    let lead: usize = if larger { 49 } else { 67 };
    const LABELS: [&str; 9] = [
        "Staff-No.",
        "Department-/Registration number",
        "Membership number",
        "Earlier name",
        "Member of a pension scheme [see note 4]",
        "Paid by another office [see note 5]",
        "Leave without pay [see note 6]",
        "Second job at another place [see note 7]",
        "Rates Category Of Fund",
    ];
    let (plain, _) = v.faces();
    // The values in the size of the letter's body (7 points at 1), the labels 8.6 to their
    // 9.7. Set larger than the body (`larger`: 9.7 over 7), a bold value is a heading to the
    // levels of the page and the title of a page with no other: another class, pinned.
    let size = if larger { 9.7 } else { 7.0 } * v.k();
    let small = size * 8.6 / 9.7;
    // The monospaced face: every glyph 0.6 sizes wide.
    let glyph = 0.6 * size;
    let (margin, top) = (42.52, 744.9);
    // The values stand 0.32 sizes right of their labels.
    let ink = margin + lead as f32 * glyph;
    let x0 = ink - 0.32 * size;
    let x1 = ink + SECOND as f32 * glyph - 0.32 * size;
    // A row of labels every 2.7 sizes, its values 1.12 sizes under it.
    let (row, drop) = (2.7 * size, 1.12 * size);
    let y = |n: usize| top - row * n as f32;
    let label = |x: f32, n: usize, text: &str| show_parts(plain, small, x, y(n), &word_parts(text));
    let line = |values: &[(usize, &str)]| {
        let mut text = String::new();
        for (at, value) in values {
            text += &" ".repeat(lead + at - text.chars().count());
            text += value;
        }
        text
    };
    let rows = [
        line(&[(0, "20481"), (SECOND, "03.04.2026 / 10:30")]),
        line(&[(0, "12345678A012")]),
        line(&[(0, "11110000")]),
    ];
    let mut t = label(x0, 0, LABELS[0])
        + &label(x1, 0, LABELS[1])
        + &show("FDB", size, margin, y(0) - drop, &rows[0])
        + &label(x0, 1, LABELS[2])
        + &label(x0 + 14.4 * size, 1, LABELS[3])
        + &show("FDB", size, margin, y(1) - drop, &rows[1]);
    for n in 2..6 {
        t += &label(x0, n, LABELS[n + 2]);
    }
    t += &label(x0, 6, LABELS[8]);
    t += &show("FDB", size, margin, y(6) - drop, &rows[2]);
    // Left of the column: a sender line in a very small face on the baseline of the first long
    // label, and under it the address, its first line smaller than the others.
    let (tiny, lesser, larger) = (0.5 * size, 0.8 * size, 1.2 * size);
    let sender = "Oakfield Fund, Mill Lane 3, 12345 Sampletown";
    let address = ["Mr", "Robin Sample", "Harbour Road 12", "12345 Sampletown"];
    t += &show("FD", tiny, 60.0, y(2), sender);
    let first = y(2) - 1.9 * size;
    t += &show("FDB", lesser, 60.0, first, address[0]);
    for (n, text) in address.iter().enumerate().skip(1) {
        t += &show("FDB", larger, 60.0, first - 1.1 * larger * n as f32, text);
    }
    let bottom = y(6) - drop;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    // The labels are drawn word by word and the lines of values with their blanks: each is
    // read as its label and its values.
    s.once = LABELS.iter().map(|l| (*l).to_string()).collect();
    s.once
        .extend(["20481", "03.04.2026 / 10:30", "12345678A012", "11110000"].map(String::from));
    s.once.push(sender.to_string());
    s.once.extend(address.map(String::from));
    for (label, value) in [
        (LABELS[0], Some("20481")),
        (LABELS[1], Some("03.04.2026 / 10:30")),
        (LABELS[2], Some("12345678A012")),
        (LABELS[3], None),
        (LABELS[8], Some("11110000")),
    ] {
        s.built.fields.push((label, Ok(value)));
    }
    for at in [0, 3, 8] {
        s.want.kinds.push((LABELS[at].into(), "label_value"));
    }
    // No long label takes the value of another row, and no line of the address a value.
    for long in &LABELS[4..8] {
        for value in ["12345678A012", "11110000"] {
            s.want.no_pairs.push(((*long).into(), value.into()));
        }
    }
    for line in address {
        s.want.no_pairs.push((line.into(), "11110000".into()));
    }
    s
}

/// Two rows of small labels over their
/// values, the second one far under the first and in another column, with no box around
/// either. They are two blocks of the page: each reads as its own pairs, never as one list.
fn fields_in_two_blocks(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let (small, size) = (7.5 * k, 9.0 * k);
    let top = 640.0;
    let indent = 0.3 * size;
    // The second block: 82 points lower at scale 1 (over nine text sizes), from another left
    // edge.
    let low = top - 1.2 * size - 8.0 * size;
    let blocks = [
        (
            40.0,
            top,
            [("Member code", "20481"), ("Issued on", "03.04.2026")],
        ),
        (
            300.0,
            low,
            [("Carrier", "Northwind"), ("Parcel no", "55012")],
        ),
    ];
    let mut t = String::new();
    for (x, y, fields) in &blocks {
        for (n, (label, value)) in fields.iter().enumerate() {
            let x = x + 130.0 * n as f32;
            t += &show(plain, small, x, *y, label);
            t += &show(plain, size, x + indent, y - 1.2 * size, value);
        }
    }
    let bottom = low - 1.2 * size;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    for (_, _, fields) in &blocks {
        for (label, value) in fields {
            s.built.fields.push((label, Ok(Some(value))));
        }
        s.want.tables.push(
            fields
                .iter()
                .map(|(l, v)| vec![l.to_string(), v.to_string()])
                .collect(),
        );
    }
    s.want.kinds.push(("Member code".into(), "label_value"));
    s.want.kinds.push(("Carrier".into(), "label_value"));
    s
}

/// The lines of [`a_run_of_pair_lines`].
const PAIR_LINES: [(&str, &str); 5] = [
    ("Invoice no:", "20481"),
    ("Issued on:", "03.04.2026"),
    ("Carrier:", "Northwind"),
    ("Parcels:", "3"),
    ("Weight:", "12,5 kg"),
];

/// Lines of a label and its
/// value set far apart (2.2 text sizes, wider than the lines of a block of text), the labels
/// from one left edge and each value after its label. Each line is a region of its own to the engine; they stand at
/// one pitch from one edge with nothing between them, so they read as one list of pairs.
fn a_run_of_pair_lines(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let pitch = 2.2 * size;
    let (x0, top) = (40.0, 640.0);
    let mut t = String::new();
    for (n, (label, value)) in PAIR_LINES.iter().enumerate() {
        let y = top - pitch * n as f32;
        // Each value a word space after its own label: the values share no left edge.
        t += &show(plain, size, x0, y, &format!("{label} {value}"));
    }
    let bottom = top - pitch * (PAIR_LINES.len() - 1) as f32;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    for (label, value) in PAIR_LINES {
        s.built.fields.push((label, Ok(Some(value))));
    }
    s.want.tables.push(
        PAIR_LINES
            .iter()
            .map(|(l, v)| vec![l.to_string(), v.to_string()])
            .collect(),
    );
    s.want.kinds.push(("Invoice no:".into(), "label_value"));
    s.want.kinds.push(("Weight:".into(), "label_value"));
    s
}

/// Which rules [`ruled_pair_lines`] draws between its lines.
#[derive(Clone, Copy, PartialEq)]
enum RowRules {
    /// The same rule between every two lines.
    Every,
    /// One rule, between the second and the third line.
    Lone,
}

/// The lines of [`a_run_of_pair_lines`]
/// at the pitch of a block of fields, with drawn rules of one extent half way between the
/// baselines. The same rule between every two lines is the block's own ruling and parts
/// nothing: one list of five pairs. One rule alone is a border: the lines over it and the
/// lines under it are two lists.
fn ruled_pair_lines(v: &Variant, rules: RowRules) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let size = 9.0 * k;
    let pitch = 2.2 * size;
    let (x0, x1, top) = (40.0, 300.0, 640.0);
    let mut t = String::new();
    for (n, (label, value)) in PAIR_LINES.iter().enumerate() {
        let y = top - pitch * n as f32;
        t += &show(plain, size, x0, y, &format!("{label} {value}"));
        let last = n + 1 == PAIR_LINES.len();
        if !last && (rules == RowRules::Every || n == 1) {
            t += &rule(v, x0 - 2.0, x1, y - 0.6 * pitch, size);
        }
    }
    let bottom = top - pitch * (PAIR_LINES.len() - 1) as f32;
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    for (label, value) in PAIR_LINES {
        s.built.fields.push((label, Ok(Some(value))));
    }
    let rows = |lines: &[(&str, &str)]| -> Vec<Vec<String>> {
        lines
            .iter()
            .map(|(l, v)| vec![l.to_string(), v.to_string()])
            .collect()
    };
    match rules {
        RowRules::Every => s.want.tables.push(rows(&PAIR_LINES)),
        RowRules::Lone => {
            s.want.tables.push(rows(&PAIR_LINES[..2]));
            s.want.tables.push(rows(&PAIR_LINES[2..]));
        }
    }
    s
}

/// A line of small labels over the line of their values, where the
/// first value reaches under two labels (`Oakfield Bank` under `Bank` and `Account`), so no
/// value can be given to one label. The line of the values reads as words and a date; it is
/// no label and its value (`Oakfield Bank | 30.10.2026` is a wrong pair): the rows stay as
/// laid out.
fn a_value_under_two_labels_beside_a_date(v: &Variant) -> Sheet {
    let k = v.k();
    let (plain, _) = v.faces();
    let (small, size) = (7.5 * k, 9.0 * k);
    let (x0, top) = (40.0, 640.0);
    let indent = 0.3 * size;
    // The second label starts half way under the first value, the third far right of it.
    let wide = font_width(plain, "Oakfield Bank", size);
    let (x1, x2) = (x0 + 0.5 * wide, x0 + wide + 12.0 * size);
    let bottom = top - 1.2 * size;
    let t = show(plain, small, x0, top, "Bank")
        + &show(plain, small, x1, top, "Account")
        + &show(plain, small, x2, top, "Paid")
        + &show(plain, size, x0 + indent, bottom, "Oakfield Bank")
        + &show(plain, size, x2 + indent, bottom, "30.10.2026");
    let mut s = sheet(v, t, bottom - 70.0, Some(Span { top, bottom }));
    s.want
        .no_pairs
        .push(("Oakfield Bank".into(), "30.10.2026".into()));
    s.want.order = vec!["Bank".into(), "Oakfield Bank".into(), "30.10.2026".into()];
    s
}

/// The shapes of the layout classes.
pub(crate) fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            name: "labels with a shared value column",
            axes: DOC_NEXT,
            draw: |v| (labels_with_a_shared_value_column(v), None),
        },
        Shape {
            name: "two labels side by side over their values",
            axes: DOC_NEXT,
            draw: |v| (two_labels_over_their_values(v), None),
        },
        Shape {
            name: "padded values under labels in a column of field rows",
            axes: DOC_NEXT,
            draw: |v| (padded_values_under_labels_in_a_column(v), None),
        },
        Shape {
            name: "lines of values from the page margin in a column of field rows",
            axes: DOC_NEXT,
            draw: |v| (value_lines_from_the_page_margin(v, false), None),
        },
        Shape {
            name: "lines of bold values larger than the body in a column of field rows",
            axes: DOC_NEXT,
            draw: |v| (value_lines_from_the_page_margin(v, true), None),
        },
        Shape {
            name: "colon labels in a head block over field rows",
            axes: DOC_NEXT,
            draw: |v| (colon_labels_in_a_head_block(v), None),
        },
        Shape {
            name: "fixed-width lines of a label, a colon column and a value",
            axes: DOC_NEXT,
            draw: |v| (fixed_width_colon_lines(v), None),
        },
        Shape {
            name: "a date beside a colon label on one line",
            axes: DOC_NEXT,
            draw: |v| (a_date_beside_a_colon_label_on_one_line(v), None),
        },
        Shape {
            name: "rows of fields in two blocks",
            axes: DOC_NEXT,
            draw: |v| (fields_in_two_blocks(v), None),
        },
        Shape {
            name: "a run of lines of a label and its value",
            axes: DOC_NEXT,
            draw: |v| (a_run_of_pair_lines(v), None),
        },
        Shape {
            name: "a ruled run of lines of a label and its value",
            axes: RULED,
            draw: |v| (ruled_pair_lines(v, RowRules::Every), None),
        },
        Shape {
            name: "a run of lines of a label and its value with one rule",
            axes: RULED,
            draw: |v| (ruled_pair_lines(v, RowRules::Lone), None),
        },
        Shape {
            name: "a value under two labels beside a date",
            axes: DOC_NEXT,
            draw: |v| (a_value_under_two_labels_beside_a_date(v), None),
        },
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
        Shape {
            name: "a heading from the margin with a date right over the rule in pieces of item rows",
            axes: LISTED,
            draw: |v| (item_rows_under_a_heading(v, Over::Heading(true)), None),
        },
        Shape {
            name: "a heading over the names with a date right over the rule in pieces of item rows",
            axes: LISTED,
            draw: |v| (item_rows_under_a_heading(v, Over::Heading(false)), None),
        },
    ]
}
