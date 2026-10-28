// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Cells of a row of fields: addresses, names over addresses, titles and strips without dividers.

use super::*;

/// The lines of three cells for [`address_cells`]: two address lines each, or a label over
/// its value.
pub(crate) const ADDRESS_CELLS: [(&str, &str); 3] = [
    ("Musterweg 1", "12345 Musterstadt"),
    ("Hauptstr. 2", "54321 Beispielort"),
    ("Am Hang 3", "11111 Neudorf"),
];
pub(crate) const LABEL_CELLS: [(&str, &str); 3] =
    [("Name", "Muster"), ("Ort", "Musterstadt"), ("Nr", "4711")];

/// A row of three fields between rules (task 15b part 2 re-review, its probe): in each cell a
/// first line of `size` over a second line `step` times as large, in `font`.
pub(crate) fn address_cells(
    cells: [(&str, &str); 3],
    size: f32,
    step: f32,
    font: &str,
) -> ReadOutcome {
    let value = size * step;
    let mut c = String::from("0 0 0 RG 0.5 w\n");
    let k = size / 8.0;
    let (x0, x1, top) = (60.0, 60.0 + 360.0 * k, 760.0);
    let bottom = top - 2.6 * value;
    for y in [top, bottom] {
        c += &format!("{x0} {y} m {x1} {y} l S\n");
    }
    for x in [x0, 60.0 + 120.0 * k, 60.0 + 240.0 * k, x1] {
        c += &format!("{x} {bottom} m {x} {top} l S\n");
    }
    for (n, (l, v)) in cells.iter().enumerate() {
        let x = 64.0 + 120.0 * k * n as f32;
        c += &shown(font, size, x, top - 1.1 * size, l);
        c += &shown(font, value, x, bottom + 0.4 * value, v);
    }
    read(&three_font_page(&c))
}

/// Task 15b part 2 re-review (ruling P6-X): a field cell of two address lines, the second a
/// little larger than the first (less than a size step), is an address, not a label over its
/// value: no pair, both lines read in order. A label over its value at the same sizes pairs
/// (but at 6 points 1.04 times, which the half-point style of a run does not tell apart).
/// Sizes 6 to 14, both fonts.
#[test]
fn a_cell_of_two_address_lines_is_no_pair() {
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for step in [1.04f32, 1.0625] {
            for font in ["F1", "F3"] {
                let at = format!("{size} pt x {step}, {font}");
                let out = address_cells(ADDRESS_CELLS, size, step, font);
                let rows: Vec<String> = read_tables(&out)
                    .iter()
                    .flatten()
                    .map(|r| r.join(": "))
                    .collect();
                let text = all_text(&out);
                // At 6 points 1.04 times is one style of run (part 2d): no pair, nothing to say.
                let one_style = step < 1.05 && size < 8.0;
                if !one_style
                    && !out
                        .document
                        .losses
                        .iter()
                        .any(|l| l.what.contains("read as an address"))
                {
                    wrong.push(format!("{at}: no loss"));
                }
                for (l, v) in ADDRESS_CELLS {
                    if rows.iter().any(|r| r.contains(l) || r.contains(v)) {
                        wrong.push(format!("{at}: {l} in a table {rows:?}"));
                    }
                    if let Err(e) = common::in_order(&text, &[l, v]) {
                        wrong.push(format!("{at}: {e}"));
                    }
                }
                // At 6 points 1.04 times is under the half point a style tells (part 2d).
                if step < 1.05 && size < 8.0 {
                    continue;
                }
                let out = address_cells(LABEL_CELLS, size, step, font);
                let rows: Vec<String> = read_tables(&out)
                    .iter()
                    .flatten()
                    .map(|r| r.join(": "))
                    .collect();
                for (l, v) in LABEL_CELLS {
                    if !rows.iter().any(|r| *r == format!("{l}: {v}")) {
                        wrong.push(format!("{at}: no pair {l}: {v} in {rows:?}"));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// A row of three fields between rules, five lines high: `Name` over `Muster`, `first` over the
/// lines `rest`, `Nr` over `4711`; the first lines of `size`, the others `step` times as large,
/// in `font`.
pub(crate) fn cell_between(
    first: &str,
    rest: &[&str],
    size: f32,
    step: f32,
    font: &str,
) -> ReadOutcome {
    cell_between_in(first, rest, size, step, font, 1.0)
}

/// [`cell_between`] with the first line of the middle cell `head` times the size of the other
/// labels.
pub(crate) fn cell_between_in(
    first: &str,
    rest: &[&str],
    size: f32,
    step: f32,
    font: &str,
    head: f32,
) -> ReadOutcome {
    let value = size * step;
    let k = size / 8.0;
    let mut c = String::from("0 0 0 RG 0.5 w\n");
    let (x0, x1, top) = (60.0, 60.0 + 360.0 * k, 760.0);
    let bottom = top - 5.0 * value;
    for y in [top, bottom] {
        c += &format!("{x0} {y} m {x1} {y} l S\n");
    }
    for x in [x0, 60.0 + 120.0 * k, 60.0 + 240.0 * k, x1] {
        c += &format!("{x} {bottom} m {x} {top} l S\n");
    }
    let low = bottom + 0.4 * value;
    let cells: [(&str, &[&str]); 3] = [("Name", &["Muster"]), (first, rest), ("Nr", &["4711"])];
    for (n, (label, values)) in cells.iter().enumerate() {
        let x = 64.0 + 120.0 * k * n as f32;
        let label_size = if n == 1 { head * size } else { size };
        c += &shown(font, label_size, x, top - 1.1 * size, label);
        for (m, v) in values.iter().rev().enumerate() {
            c += &shown(font, value, x, low + 1.2 * value * m as f32, v);
        }
    }
    read(&three_font_page(&c))
}

/// Task 15b part 2c concern 3 ruling: a first line without a number over address lines pairs
/// as a label only on evidence: the label style of the fields beside it, or a colon at its
/// end. Smaller than the labels beside it (`Muster GmbH`, `Erika Mustermann` over c/o and
/// address, `Muster GmbH` over a PO box), it is no pair, and a loss says so; `Anschrift:` pairs.
/// The fields beside it pair. Steps 1.0625 to 1.25, sizes 6 to 14, both fonts.
#[test]
fn a_name_over_an_address_pairs_only_on_evidence() {
    let cases: [(&str, &[&str], bool); 5] = [
        ("Muster GmbH", &["Musterweg 1", "12345 Musterstadt"], false),
        (
            "Muster GmbH",
            &["Postfach 12 34", "12345 Musterstadt"],
            false,
        ),
        (
            "Erika Mustermann",
            &["c/o Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
            false,
        ),
        (
            "Kunde",
            &["Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
            false,
        ),
        ("Anschrift:", &["Musterweg 1", "12345 Musterstadt"], true),
    ];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for step in [1.0625f32, 1.125, 1.25] {
            for font in ["F1", "F3"] {
                for (first, rest, pairs) in cases {
                    let at = format!("{first:?} {size} pt x {step}, {font}");
                    let out = cell_between_in(first, rest, size, step, font, 0.9);
                    let rows: Vec<String> = read_tables(&out)
                        .iter()
                        .flatten()
                        .map(|r| r.join(": "))
                        .collect();
                    for want in ["Name: Muster", "Nr: 4711"] {
                        if !rows.iter().any(|r| r == want) {
                            wrong.push(format!("{at}: no {want} in {rows:?}"));
                        }
                    }
                    let paired = rows.iter().any(|r| r.contains(first));
                    let said = out
                        .document
                        .losses
                        .iter()
                        .any(|l| l.what.contains("read as an address"));
                    if pairs && !paired {
                        wrong.push(format!("{at}: no pair: {rows:?}"));
                    }
                    if !pairs && (paired || !said) {
                        wrong.push(format!("{at}: paired {paired}, said {said}: {rows:?}"));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Task 15b part 2c review (Important 2 and 3, concern 4; ruling P6-X): a field cell whose lines
/// are an address, between `Name` over `Muster` and `Nr` over `4711`, is left out of its row and
/// a loss says so; the two other fields still pair. An address whose first line holds a number
/// (a street, a PO box, a postcode over the country, `c/o`) is no pair at any size step; a first
/// line without one over address lines (a name, a company, the label `Anschrift`) pairs as a
/// label in the label style of the fields beside it (concern 3 ruling). Steps 1.0625 to 1.25,
/// sizes 6 to 14, both fonts.
#[test]
fn an_address_cell_is_left_out_of_its_row_and_said() {
    // The first line, the lines under it, and whether it is an address by its number.
    let cases: [(&str, &[&str], bool); 12] = [
        ("Musterweg 1", &["12345 Musterstadt"], true),
        ("Musterweg 1", &["12345 Musterstadt", "Deutschland"], true),
        ("Hauptstr. 2a", &["12345 Musterstadt"], true),
        ("Postfach 12 34", &["12345 Musterstadt"], true),
        ("12345 Musterstadt", &["Deutschland"], true),
        ("c/o Erika Muster", &["Musterweg 1"], true),
        ("Feld 12", &["12345 Musterstadt"], true),
        ("Konto 1", &["DE00 1234 5678"], true),
        ("Muster GmbH", &["Musterweg 1", "12345 Musterstadt"], false),
        (
            "Erika Mustermann",
            &["c/o Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
            false,
        ),
        ("Anschrift", &["Musterweg 1", "12345 Musterstadt"], false),
        (
            "Name und Anschrift",
            &["Musterweg 1", "12345 Musterstadt"],
            false,
        ),
    ];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for step in [1.0625f32, 1.08, 1.125, 1.25] {
            for font in ["F1", "F3"] {
                for (first, rest, numbered) in cases {
                    let at = format!("{first:?} {size} pt x {step}, {font}");
                    let out = cell_between(first, rest, size, step, font);
                    let rows: Vec<String> = read_tables(&out)
                        .iter()
                        .flatten()
                        .map(|r| r.join(": "))
                        .collect();
                    for want in ["Name: Muster", "Nr: 4711"] {
                        if !rows.iter().any(|r| r == want) {
                            wrong.push(format!("{at}: no {want} in {rows:?}"));
                        }
                    }
                    let address = numbered;
                    let paired = rows.iter().any(|r| r.contains(first));
                    let said = out
                        .document
                        .losses
                        .iter()
                        .any(|l| l.what.contains("read as an address"));
                    if address && (paired || !said) {
                        wrong.push(format!("{at}: paired {paired}, said {said}: {rows:?}"));
                    }
                    if !address
                        && !rows
                            .iter()
                            .any(|r| *r == format!("{first}: {}", rest.join(" ")))
                    {
                        wrong.push(format!("{at}: the label does not pair: {rows:?}"));
                    }
                    let mut order = vec![first];
                    order.extend(rest.iter().copied());
                    if let Err(e) = common::in_order(&all_text(&out), &order) {
                        wrong.push(format!("{at}: {e}"));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The middle cell of [`three_cells`]: its first lines (`head`, in `font` at `size` times the
/// labels' size; F2 is bold) and the lines under them at the value size in the data font.
#[derive(Clone, Copy)]
pub(crate) struct Middle<'a> {
    head: &'a [&'a str],
    size: f32,
    font: Option<&'a str>,
    lines: &'a [&'a str],
}

/// How the three cells of [`three_cells`] are drawn: boxes in a row, boxes in a column, or
/// a strip between two rules with no rule between its cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cells {
    Row,
    Column,
    Strip,
}

/// Three cells, `Name` over `Beispiel`, the cell `m`, and `Nr` over `4711`, the labels in
/// `font` at `size`, the values a `step` larger: drawn as `how` says.
pub(crate) fn three_cells(m: Middle, size: f32, step: f32, font: &str, how: Cells) -> ReadOutcome {
    let value = size * step;
    let k = size / 8.0;
    let mut c = String::from("0 0 0 RG 0.5 w\n");
    let h = 5.0 * value;
    let (x0, top) = (60.0, 760.0);
    let width = 120.0 * k;
    let cells: [(&[&str], f32, &str, &[&str]); 3] = [
        (&["Name"], size, font, &["Beispiel"]),
        (m.head, m.size * size, m.font.unwrap_or(font), m.lines),
        (&["Nr"], size, font, &["4711"]),
    ];
    // Each cell's left edge and top.
    let at = |n: usize| match how {
        Cells::Column => (x0, top - h * n as f32),
        _ => (x0 + width * n as f32, top),
    };
    match how {
        Cells::Strip => {
            for y in [top, top - h] {
                c += &format!("{x0} {y} m {} {y} l S\n", x0 + 3.0 * width);
            }
        }
        _ => {
            for n in 0..3 {
                let (x, y) = at(n);
                c += &format!("{x} {} {width} {h} re S\n", y - h);
            }
        }
    }
    for (n, (head, hs, hf, values)) in cells.iter().enumerate() {
        let (x, y) = at(n);
        for (l, t) in head.iter().enumerate() {
            c += &shown(
                hf,
                *hs,
                x + 4.0 * k,
                y - 1.1 * size - 1.15 * hs * l as f32,
                t,
            );
        }
        let low = y - h + 0.4 * value;
        for (l, v) in values.iter().rev().enumerate() {
            c += &shown(font, value, x + 4.0 * k, low + 1.2 * value * l as f32, v);
        }
    }
    read(&three_font_page(&c))
}

/// The texts of the blocks of a read that are no table, sections opened.
pub(crate) fn text_blocks(blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        match b {
            Block::Section { blocks, .. } => text_blocks(blocks, out),
            Block::Table { .. } => {}
            b => {
                let mut s = String::new();
                common::block_text(b, &mut s);
                out.push(s.trim().replace('\n', " / "));
            }
        }
    }
}

/// What the middle cell of [`three_cells`] must read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Want {
    /// Its first line the label, the lines under it the value.
    Pair,
    /// In no pair, and a loss names its first line.
    Said,
    /// In no pair; no loss is needed (a title).
    Alone,
}

/// What is wrong with the read `out` of [`three_cells`] around `m`: `Name: Beispiel` and
/// `Nr: 4711` pair; the middle cell reads as `want` says; no block that is no table holds
/// words of two cells (words of one cell only, the words they share aside).
pub(crate) fn three_cells_wrong(out: &ReadOutcome, m: Middle, want: Want) -> Vec<String> {
    let mut wrong = Vec::new();
    let rows: Vec<String> = read_tables(out)
        .iter()
        .flatten()
        .map(|r| r.join(": "))
        .collect();
    for want in ["Name: Beispiel", "Nr: 4711"] {
        if !rows.iter().any(|r| r == want) {
            wrong.push(format!("no {want} in {rows:?}"));
        }
    }
    let head = m.head.join(" ");
    let paired = rows
        .iter()
        .any(|r| m.head.iter().chain(m.lines).any(|t| r.contains(t)));
    if want == Want::Pair {
        let want = format!("{head}: {}", m.lines.join(" "));
        if !rows.contains(&want) {
            wrong.push(format!("no {want} in {rows:?}"));
        }
    } else {
        let said = want == Want::Alone
            || out
                .document
                .losses
                .iter()
                .any(|l| l.what.contains(m.head[0]));
        if paired || !said {
            let losses: Vec<&str> = out
                .document
                .losses
                .iter()
                .map(|l| l.what.as_str())
                .collect();
            wrong.push(format!("paired {paired}, said {said}: {rows:?} {losses:?}"));
        }
    }
    let words = |ts: &[&str]| -> Vec<String> {
        ts.iter()
            .flat_map(|t| t.split_whitespace())
            .map(str::to_string)
            .collect()
    };
    let mut middle = m.head.to_vec();
    middle.extend(m.lines);
    let cells = [
        words(&["Name", "Beispiel"]),
        words(&middle),
        words(&["Nr", "4711"]),
    ];
    let own = |c: usize, w: &str| {
        cells[c].iter().any(|x| x == w)
            && (0..3).all(|o| o == c || !cells[o].iter().any(|x| x == w))
    };
    let mut blocks = Vec::new();
    text_blocks(&out.document.body, &mut blocks);
    for b in blocks {
        let of: Vec<usize> = (0..3)
            .filter(|&c| b.split_whitespace().any(|w| own(c, w)))
            .collect();
        if of.len() > 1 {
            wrong.push(format!("a block mixes cells {of:?}: {b:?}"));
        }
    }
    wrong
}

/// The middle cells of the matrix tests below: a name or a label over an address, its first
/// line at `size` times the labels' size in `font` (none: the labels' font), and how it reads
/// (it pairs when a colon ends it or it has the labels' style and font).
pub(crate) type Case = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    f32,
    Option<bool>,
    Want,
);

pub(crate) const ADDR: &[&str] = &["Musterweg 1", "12345 Musterstadt"];

/// Runs the cases over sizes 6 to 14, steps 1.0625 to 1.25, both data fonts and the ways
/// `hows`; a case's font `Some(true)` is the other data font, `Some(false)` the bold one.
pub(crate) fn middle_matrix(cases: &[Case], hows: &[Cells]) -> Vec<String> {
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for step in [1.0625f32, 1.125, 1.25] {
            for font in ["F1", "F3"] {
                let other = if font == "F1" { "F3" } else { "F1" };
                for &how in hows {
                    for &(name, head, lines, hs, hf, want) in cases {
                        let m = Middle {
                            head,
                            size: if hs == 0.0 { step } else { hs },
                            font: hf.map(|o| if o { other } else { "F2" }),
                            lines,
                        };
                        let at = format!("{name} {how:?} {size} pt x {step}, {font}");
                        let out = three_cells(m, size, step, font, how);
                        wrong.extend(
                            three_cells_wrong(&out, m, want)
                                .into_iter()
                                .map(|e| format!("{at}: {e}")),
                        );
                    }
                }
            }
        }
    }
    wrong
}

/// Task 15b part 2d item 4 (ruling P6-X): the font family is part of a label's style. A name
/// over an address at the labels' size in another family than the labels beside it is no
/// label: it pairs with nothing and a loss names it; in the labels' own family it pairs. Boxes in a row and in a column, and a strip between two rules.
#[test]
fn a_name_in_another_font_family_is_no_label() {
    let cases: [Case; 3] = [
        (
            "name in another family",
            &["Muster GmbH"],
            ADDR,
            1.0,
            Some(true),
            Want::Said,
        ),
        (
            "person in another family",
            &["Erika Mustermann"],
            &["c/o Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
            1.0,
            Some(true),
            Want::Said,
        ),
        (
            "name in the labels' family",
            &["Muster GmbH"],
            ADDR,
            1.0,
            None,
            Want::Pair,
        ),
    ];
    let wrong = middle_matrix(&cases, &[Cells::Row, Cells::Column, Cells::Strip]);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Task 15b part 2d item 1 (ruling P6-X): a bold title in a drawn box between boxed fields, of
/// one line or two at the labels' size, the values' size or larger, in a row of boxes or a
/// column of them: the title is a heading of its own, and the fields on both sides keep their
/// pairs. Sizes 6 to 14, steps 1.0625 to 1.25, both data fonts.
#[test]
fn a_bold_title_in_a_box_between_fields_is_a_heading_of_its_own() {
    let titles: [&[&str]; 2] = [&["Angaben zum Betrieb"], &["Angaben zum", "Betrieb"]];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for step in [1.0625f32, 1.125, 1.25] {
            for font in ["F1", "F3"] {
                for how in [Cells::Row, Cells::Column] {
                    for title in titles {
                        for big in [1.0, step, 1.2 * step] {
                            let m = Middle {
                                head: title,
                                size: big,
                                font: Some("F2"),
                                lines: &[],
                            };
                            let at = format!("{title:?} x{big} {how:?} {size} pt x {step}, {font}");
                            let out = three_cells(m, size, step, font, how);
                            let mut w = three_cells_wrong(&out, m, Want::Alone);
                            let heading = out.document.body.iter().any(|b| {
                                let mut found = false;
                                let mut stack = vec![b];
                                while let Some(b) = stack.pop() {
                                    match b {
                                        Block::Section { blocks, .. } => stack.extend(blocks),
                                        Block::Heading { inlines, .. } => {
                                            found |= plain_text(inlines) == title.join(" ");
                                        }
                                        _ => {}
                                    }
                                }
                                found
                            });
                            if !heading {
                                w.push(format!("no heading {:?}", title.join(" ")));
                            }
                            wrong.extend(w.into_iter().map(|e| format!("{at}: {e}")));
                        }
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Task 15b part 2d item 2 (ruling P6-X): a boxed cell between boxed fields whose first line
/// has the values' size (a name over an address, a value with no label, two lines of one
/// style) is left out of its row and a loss names it; `Name: Beispiel` and `Nr: 4711` pair, in
/// a row of boxes and in a column. (A size 0.0 below is the values' size.)
#[test]
fn a_box_at_the_value_size_is_left_out_of_its_row_and_said() {
    let cases: [Case; 4] = [
        (
            "name at value size",
            &["Muster GmbH"],
            ADDR,
            0.0,
            None,
            Want::Said,
        ),
        ("value alone", &["Musterwert"], &[], 0.0, None, Want::Said),
        (
            "two lines of one style",
            &["Musterwert"],
            &["Zweite Zeile"],
            0.0,
            None,
            Want::Said,
        ),
        (
            "name over a PO box",
            &["Muster GmbH"],
            &["Postfach 12 34", "12345 Musterstadt"],
            0.0,
            None,
            Want::Said,
        ),
    ];
    let wrong = middle_matrix(&cases, &[Cells::Row, Cells::Column]);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Task 15b part 2d item 3 (ruling P6-X): a strip of three fields between two rules with no
/// rule between them, its middle cell's first line a little smaller than the labels beside
/// it, at the values' size, or a bold title: the cells are read apart, never line by line
/// across them; `Name: Beispiel` and `Nr: 4711` pair; the middle cell pairs only on a colon or
/// the labels' style, a title is in no pair, and else a loss names it.
#[test]
fn a_strip_without_dividers_reads_its_cells_apart() {
    let cases: [Case; 11] = [
        (
            "bold title",
            &["Angaben"],
            &[],
            1.0,
            Some(false),
            Want::Alone,
        ),
        (
            "bold title at value size",
            &["Angaben"],
            &[],
            0.0,
            Some(false),
            Want::Alone,
        ),
        (
            "bold title of two lines",
            &["Angaben zum", "Betrieb"],
            &[],
            1.0,
            Some(false),
            Want::Alone,
        ),
        ("name 0.9", &["Muster GmbH"], ADDR, 0.9, None, Want::Said),
        (
            "person over c/o 0.9",
            &["Erika Mustermann"],
            &["c/o Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
            0.9,
            None,
            Want::Said,
        ),
        (
            "firm over a PO box 0.9",
            &["Muster GmbH"],
            &["Postfach 12 34", "12345 Musterstadt"],
            0.9,
            None,
            Want::Said,
        ),
        (
            "Anschrift: 0.9",
            &["Anschrift:"],
            ADDR,
            0.9,
            None,
            Want::Pair,
        ),
        (
            "Kunde 0.9",
            &["Kunde"],
            &["Muster GmbH", "Musterweg 1", "12345 Musterstadt"],
            0.9,
            None,
            Want::Said,
        ),
        (
            "Anschrift in the labels' style",
            &["Anschrift"],
            ADDR,
            1.0,
            None,
            Want::Pair,
        ),
        (
            "name at value size",
            &["Muster GmbH"],
            ADDR,
            0.0,
            None,
            Want::Said,
        ),
        (
            "label 0.9 over a value",
            &["Kundennummer"],
            &["00000"],
            0.9,
            None,
            Want::Pair,
        ),
    ];
    let wrong = middle_matrix(&cases, &[Cells::Strip]);
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// A cell of [`open_strip_of`]: its first line, that line's size as a share of the labels'
/// size, and the lines under it.
pub(crate) type StripCell = (&'static str, f32, &'static [&'static str]);

/// A strip between two rules with no rule between its cells: each cell its first line (a
/// label at `head` times the labels' size `size`) and the lines under it at the values' size
/// (`size` times `step`), in `font`.
pub(crate) fn open_strip_of(cells: &[StripCell], size: f32, step: f32, font: &str) -> ReadOutcome {
    let value = size * step;
    let k = size / 8.0;
    let width = 120.0 * k;
    let (x0, top) = (60.0, 760.0);
    let h = 5.0 * value;
    let mut c = String::from("0 0 0 RG 0.5 w\n");
    for y in [top, top - h] {
        c += &format!("{x0} {y} m {} {y} l S\n", x0 + width * cells.len() as f32);
    }
    for (n, (head, hs, lines)) in cells.iter().enumerate() {
        let x = x0 + width * n as f32 + 4.0 * k;
        c += &shown(font, hs * size, x, top - 1.1 * size, head);
        let low = top - h + 0.4 * value;
        for (l, v) in lines.iter().rev().enumerate() {
            c += &shown(font, value, x, low + 1.2 * value * l as f32, v);
        }
    }
    read(&three_font_page(&c))
}

/// Task 15b part 2d item 3 (ruling P6-X), beyond one odd cell between two labels: strips of
/// two to four cells with a name a little smaller than the labels over an address first, last,
/// in the middle or twice, beside labels over their values. Each cell pairs right (its label
/// over its lines) or a loss names it; the cells marked to pair do; no block that is no table
/// holds words of two cells. Fix round 1 (rulings on concerns 1 and 3): one field beside the
/// name pairs only when a colon ends its label, else it is refused, and the refused strip reads
/// cell by cell. Sizes 6 to 14, steps 1.0625 to 1.25, both data fonts.
#[test]
fn strips_of_two_to_four_cells_read_their_cells_apart() {
    const NAME: StripCell = ("Muster GmbH", 0.9, &["Musterweg 1", "12345 Musterstadt"]);
    const PERSON: StripCell = (
        "Erika Mustermann",
        0.9,
        &["c/o Muster GmbH", "Beispielweg 2"],
    );
    const NR: StripCell = ("Nr", 1.0, &["4711"]);
    const KEY: StripCell = ("Name", 1.0, &["Beispiel"]);
    const DATE: StripCell = ("Datum", 1.0, &["01.01.2020"]);
    const NR_COLON: StripCell = ("Nr:", 1.0, &["4711"]);
    // The cells, the heads that must pair, and the heads that must not (a lone field with no
    // sign of a label).
    type Layout = (
        &'static [StripCell],
        &'static [&'static str],
        &'static [&'static str],
    );
    let layouts: [Layout; 7] = [
        (&[NAME, NR_COLON], &["Nr:"], &[]),
        (&[NAME, NR], &[], &["Nr"]),
        (&[KEY, NAME], &[], &["Name"]),
        (&[NAME, KEY, NR], &["Name", "Nr"], &[]),
        (&[KEY, NR, NAME], &["Name", "Nr"], &[]),
        (&[KEY, NAME, NR, DATE], &["Name", "Nr", "Datum"], &[]),
        (&[NAME, PERSON, NR, KEY], &["Nr", "Name"], &[]),
    ];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for step in [1.0625f32, 1.125, 1.25] {
            for font in ["F1", "F3"] {
                for (cells, pairs, lone) in layouts {
                    let heads: Vec<&str> = cells.iter().map(|c| c.0).collect();
                    let at = format!("{heads:?} {size} pt x {step}, {font}");
                    let out = open_strip_of(cells, size, step, font);
                    let rows: Vec<String> = read_tables(&out)
                        .iter()
                        .flatten()
                        .map(|r| r.join(": "))
                        .collect();
                    for (head, _, lines) in cells.iter() {
                        let want = format!("{head}: {}", lines.join(" "));
                        let paired = rows.iter().any(|r| r.starts_with(&format!("{head}:")));
                        if paired && !rows.contains(&want) {
                            wrong.push(format!("{at}: wrong pair for {head}: {rows:?}"));
                        }
                        let said = out.document.losses.iter().any(|l| l.what.contains(head));
                        if !paired && !said {
                            wrong.push(format!("{at}: {head} neither pairs nor is said: {rows:?}"));
                        }
                        if pairs.contains(head) && !rows.contains(&want) {
                            wrong.push(format!("{at}: no {want} in {rows:?}"));
                        }
                        if lone.contains(head) && paired {
                            wrong.push(format!("{at}: the lone {head} pairs: {rows:?}"));
                        }
                    }
                    let words: Vec<Vec<&str>> = cells
                        .iter()
                        .map(|(h, _, l)| {
                            h.split_whitespace()
                                .chain(l.iter().flat_map(|t| t.split_whitespace()))
                                .collect()
                        })
                        .collect();
                    let own = |c: usize, w: &str| {
                        words[c].contains(&w)
                            && (0..words.len()).all(|o| o == c || !words[o].contains(&w))
                    };
                    let mut blocks = Vec::new();
                    text_blocks(&out.document.body, &mut blocks);
                    for b in blocks {
                        let of: Vec<usize> = (0..cells.len())
                            .filter(|&c| b.split_whitespace().any(|w| own(c, w)))
                            .collect();
                        if of.len() > 1 {
                            wrong.push(format!("{at}: a block mixes cells {of:?}: {b:?}"));
                        }
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Task 15b part 2d item 3 (ruling P6-X: never silent): a strip whose cells are not clear (the
/// address under a smaller name runs on under the next label) and whose text is in three
/// styles is not read as fields, and a loss names its heads; no pair is made. Fix round 1
/// (ruling on concern 1): its text is read cell by cell, never line by line across them.
#[test]
fn a_strip_in_three_styles_not_clear_is_said() {
    let long: &[&str] = &["Musterweg 1", "12345 Musterstadt-Beispielhausen"];
    let mut wrong = Vec::new();
    for size in [6.0f32, 8.0, 10.0, 12.0, 14.0] {
        for step in [1.0625f32, 1.125, 1.25] {
            for font in ["F1", "F3"] {
                let at = format!("{size} pt x {step}, {font}");
                let cells: [StripCell; 3] = [
                    ("Name", 1.0, &["Beispiel"]),
                    ("Muster GmbH", 0.9, long),
                    ("Nr", 1.0, &["4711"]),
                ];
                let out = open_strip_of(&cells, size, step, font);
                let rows: Vec<String> = read_tables(&out)
                    .iter()
                    .flatten()
                    .map(|r| r.join(": "))
                    .collect();
                if !rows.is_empty() {
                    wrong.push(format!("{at}: pairs {rows:?}"));
                }
                let said = out
                    .document
                    .losses
                    .iter()
                    .any(|l| l.what.contains("Name, Muster GmbH, Nr"));
                if !said {
                    let losses: Vec<&str> = out
                        .document
                        .losses
                        .iter()
                        .map(|l| l.what.as_str())
                        .collect();
                    wrong.push(format!("{at}: not said: {losses:?}"));
                }
                let mut blocks = Vec::new();
                text_blocks(&out.document.body, &mut blocks);
                for b in blocks {
                    let of: Vec<&str> = ["Beispiel", "Musterweg", "4711"]
                        .into_iter()
                        .filter(|w| b.split_whitespace().any(|x| x == *w))
                        .collect();
                    if of.len() > 1 {
                        wrong.push(format!("{at}: a block mixes cells: {b:?}"));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
