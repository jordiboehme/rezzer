// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Labels and values of forms as print runs fill them, in the shapes of a real pay slip
//! (measured by structure only; all text and values invented), and generated variants of each
//! shape: shifted, scaled as a whole, other label and data sizes, other data fonts, other rule
//! thicknesses and strokes, dividers hanging or rising, labels over or under their values and
//! set left, in the middle or right, values left or right, with and without drawn decimal
//! marks, on two pages and turned. Each variant is held to the same invariants (see
//! [`check`]): no wrong pair ever, every value once, a pair where the geometry is clear, a
//! named loss where it is not.

#[path = "../common/mod.rs"]
mod common;

mod check;
mod draw;
mod ends;
mod shapes;

use check::{
    Broken, Inv, Known, assert_fields, blocks, check_fields, check_page, check_title, pairs_of,
    read, read_pages, unexpected,
};
use common::{all_text, label};
use draw::{Stroke, bar_h, bar_v, mark_b};
use rezzer_model::{Block, plain_text};
use shapes::{
    Align, BLOCK_A, Built, DataFont, FieldSpec, Geo, GridAt, Neighbour, REAL, ROWS, Rows,
    THREE_ROWS, TitleBlock, boxed_row, grids_side_by_side, header_grid, label_block, lone_field,
    neighbour, overflow_rows, page, rows_page, shape_page, short_value_rows, title_grid_labels,
    title_grid_page, title_over, total_column,
};

#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

#[test]
fn rate_over_its_divider_pairs_by_its_mark() {
    let b = page(&REAL, true, true);
    let out = read_pages(&[&b]);
    let broken = check_fields(&out, &b);
    assert!(broken.is_empty(), "{broken:#?}\n{:#?}", blocks(&out));
}

#[test]
fn rate_over_its_divider_without_a_mark_names_only_its_fields() {
    let g = Geo {
        marks: false,
        ..REAL
    };
    let b = page(&g, true, true);
    let out = read_pages(&[&b]);
    let broken = check_fields(&out, &b);
    assert!(broken.is_empty(), "{broken:#?}\n{:#?}", blocks(&out));
    let named: Vec<&str> = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.contains("row of fields") || l.what.contains("fields "))
        .map(|l| l.what.as_str())
        .collect();
    assert!(
        named
            .iter()
            .all(|w| !w.contains("Feld A") && !w.contains("Gr D")),
        "{named:?}"
    );
}

#[test]
fn row_under_a_longer_rule_pairs() {
    let b = page(&REAL, true, true);
    let out = read_pages(&[&b]);
    let pairs = pairs_of(blocks(&out));
    for want in [("Eintritt", "01.01.20"), ("Austritt", "empty")] {
        assert!(
            pairs.iter().any(|(l, v)| l == want.0 && v == want.1),
            "{want:?} in {pairs:?}"
        );
    }
}

#[test]
fn title_over_a_bar_heads_its_lines() {
    let b = page(&REAL, true, true);
    let out = read_pages(&[&b]);
    let broken = check_title(&out);
    assert!(broken.is_empty(), "{broken:#?}\n{:#?}", blocks(&out));
}

/// A page of prose: a small bold heading over a drawn rule, then two paragraphs of regular
/// text half a point larger, of three lines each, with no field or grid on the page; with and
/// without a framed figure under the text. The heading and the rule are no form's title: the
/// lines keep their paragraphs.
#[test]
fn a_heading_over_a_rule_on_a_prose_page_keeps_its_paragraphs() {
    for figure in ["", "q 0.5 w 72 300 300 200 re S Q\n"] {
        prose_page_keeps_its_paragraphs(figure);
    }
}

fn prose_page_keeps_its_paragraphs(figure: &str) {
    let mut c = String::from(figure);
    c += "BT /F2 9 Tf 72 700 Td (Ergebnisse der Messung) Tj ET\n";
    c += "q 0.5 w 72 697.5 m 400 697.5 l S Q\n";
    let lines = [
        "Die Messung zeigt einen klaren Anstieg der Werte im",
        "zweiten Abschnitt, der sich bis zum Ende der Reihe",
        "fortsetzt und im letzten Monat am hoechsten ist.",
        "Im Vergleich mit dem Vorjahr liegt der Wert damit",
        "deutlich hoeher, obwohl die Zahl der Proben gleich",
        "geblieben ist und die Methode nicht geaendert wurde.",
    ];
    for (k, l) in lines.iter().enumerate() {
        let y = 682.0 - 11.4 * k as f32 - if k >= 3 { 8.0 } else { 0.0 };
        c += &format!("BT /F1 9.5 Tf 72 {y} Td ({l}) Tj ET\n");
    }
    let out = read(&common::pdf(&[common::Page::text(c)], ""));
    let paragraphs: Vec<String> = blocks(&out)
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph { inlines, .. } => Some(plain_text(inlines)),
            _ => None,
        })
        .collect();
    for (a, b) in [(0, 2), (3, 5)] {
        assert!(
            paragraphs
                .iter()
                .any(|p| p.contains(lines[a]) && p.contains(lines[b])),
            "{figure:?}: {paragraphs:#?}"
        );
    }
}

/// A row of fields like F1 (labels over a value line, dividers hanging from the upper rule
/// at data columns 13, 43, 47 and 51) whose rate field (columns 43 to 47) holds `digits`
/// from column 43, a drawn mark after its first two digits, and `next` (column, text) on the
/// value line after it; with `frame` false only the divider at column 47 is drawn and no
/// rule over or under the row. Two amounts elsewhere set the page's style of amounts.
fn mark_row(digits: &str, next: Option<(usize, &str)>, frame: bool) -> Built {
    let g = REAL;
    let mut b = Built::new(&g);
    let top = 760.0;
    if frame {
        b.page.template += &bar_h(g.col(0) - 0.12, g.col(61), top, g.rule);
        b.page.template += &bar_h(g.col(0) - 0.12, g.col(61), top - 2.4 * g.data, g.rule);
    }
    for k in [13, 43, 47, 51] {
        if frame || k == 47 {
            b.page.template += &bar_v(g.col(k) - 0.06, top - 1.21 * g.data, top, g.rule);
        }
    }
    let base = top - 0.86 * g.data;
    for (k, l) in [
        (0, "Feld A"),
        (13, "Feld B"),
        (43, "Satz"),
        (47, "Gr C"),
        (51, "Gr D"),
    ] {
        b.page.template += &label(g.col(k) + 0.18 * g.label, base, g.label, l);
    }
    let y = top - 2.23 * g.data;
    let mut parts = vec![(0, "12345678X012"), (13, "Muster Kasse"), (43, digits)];
    parts.extend(next);
    parts.push((51, "8901"));
    g.values(&mut b, y, &parts);
    b.page.template += &mark_b(
        g.col(43) + 2.0 * g.pitch(),
        y - 0.087 * g.data,
        0.2 * g.data,
    );
    for (k, l) in ["Summe 9.876,54", "Netto 1.234,56"].iter().enumerate() {
        g.values(&mut b, 120.0 - 1.2 * g.data * k as f32, &[(0, l)]);
    }
    b
}

/// The text of a read and the losses that keep a number without its separators.
fn kept_whole(b: &Built) -> (String, Vec<String>) {
    let out = read_pages(&[b]);
    let kept = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.contains("is kept without"))
        .map(|l| l.what.clone())
        .collect();
    (all_text(&out), kept)
}

/// A number under a vertical rule that divides no row of fields (no rule over or under the
/// row) is never cut at it: it stays whole, with the loss that its separator is not read.
#[test]
fn a_number_under_a_lone_rule_stays_whole() {
    let (text, kept) = kept_whole(&mark_row("123456", None, false));
    assert!(text.contains("123456") && !text.contains("12,34"), "{text}");
    assert!(kept.iter().any(|w| w.contains("123456")), "{kept:?}");
}

/// A number that reads as a whole number in groups of three (a thousands value, or a value of
/// three decimals) is never cut at a divider, even when the part before it would read as an
/// amount: it stays whole with its loss.
#[test]
fn a_number_in_groups_of_three_stays_whole_at_a_divider() {
    for digits in ["12500", "12345"] {
        let (text, kept) = kept_whole(&mark_row(digits, None, true));
        assert!(text.contains(digits), "{digits}: {text}");
        assert!(
            kept.iter().any(|w| w.contains(digits)),
            "{digits}: {kept:?}"
        );
    }
}

/// A number is cut at a divider only when the part after it is the whole value of the next
/// field: with another value in that field it stays whole with its loss.
#[test]
fn a_number_is_not_cut_into_a_field_with_a_value() {
    let (text, kept) = kept_whole(&mark_row("123456", Some((50, "7")), true));
    assert!(text.contains("123456") && !text.contains("12,34"), "{text}");
    assert!(kept.iter().any(|w| w.contains("123456")), "{kept:?}");
}

/// A number in boxes drawn around each digit, standing on the rule under its row: the sides
/// of the boxes reach the row's lower rule like dividers rising from it, but more than one of
/// them stands in the number, so it is never cut at one; it stays whole with its loss.
#[test]
fn a_number_in_boxes_of_its_digits_stays_whole() {
    let mut b = mark_row("123456", None, true);
    let g = REAL;
    let bottom = 760.0 - 2.4 * g.data;
    for k in 0..6 {
        let x = g.col(43 + k) - 0.06;
        b.page.template += &format!("{x} {bottom} {} {} re S\n", g.pitch(), 1.21 * g.data);
    }
    let (text, kept) = kept_whole(&b);
    assert!(text.contains("123456") && !text.contains("12,34"), "{text}");
    assert!(kept.iter().any(|w| w.contains("123456")), "{kept:?}");
}

/// A label with a raised footnote marker after it, on a page with the legend of that marker:
/// the label pairs as its own text, and the marker is a reference to its footnote after it.
#[test]
fn a_marked_label_pairs_as_its_text_with_its_reference() {
    let mut b = page(&REAL, true, true);
    b.page.template += &draw::plain(42.52, 60.0, 6.0, "8 Satz in Prozent des Bruttos");
    let out = read_pages(&[&b]);
    let label = blocks(&out).iter().find_map(|bl| match bl {
        Block::Table { rows, .. } => rows
            .iter()
            .find(|r| plain_text(&r[0].inlines).starts_with("Satz"))
            .map(|r| r[0].inlines.clone()),
        _ => None,
    });
    assert!(
        matches!(label.as_deref(), Some([rezzer_model::Inline::Text { text, .. }, rezzer_model::Inline::FootnoteRef { .. }]) if text == "Satz %"),
        "{label:?}"
    );
}

/// Labels printed under the rule their value stands on (a signature line) pair with the
/// value over the same rule, never with the value of the next row, which stands under them
/// between the same two rules (ruling P6-X).
#[test]
fn labels_under_their_values_pair_across_the_rule() {
    for dividers in [true, false] {
        let s = Rows {
            under: true,
            dividers,
            ..ROWS
        };
        assert_fields(&rows_page(&REAL, &s, &THREE_ROWS, &[]));
    }
}

/// A row of fields whose labels stand over their values in one field and under them in the
/// next is not read as labels and values: its text stays, and a loss names its fields.
#[test]
fn mixed_directions_in_a_row_give_a_loss() {
    let mut b = rows_page(
        &REAL,
        &ROWS,
        &[&[(0, 30, "Name", "Erika Mustermann"), (30, 20, "Ort", "")]],
        &["Name", "Ort"],
    );
    // The second field: its value at the top, its label at the bottom.
    let (top, d) = (760.0, REAL.data);
    let x = 42.52 + 30.0 * 0.6 * d + 0.3 * 0.6 * d;
    b.page.template += &format!(
        "BT /FD {d} Tf {x} {} Td (Musterstadt) Tj ET\n",
        top - 0.86 * d - 0.3 * d
    );
    b.values.push("Musterstadt".into());
    // Move the label "Ort" to the bottom of its field.
    b.page.template = b.page.template.replacen(
        &format!("{} Td (Ort)", top - 0.86 * d),
        &format!("{} Td (Ort)", top - 2.23 * d),
        1,
    );
    assert_fields(&b);
}

/// A grid of fields beside the header grid on the same top rule, with rows of another pitch:
/// the rules of the two grids interleave, yet each grid is one table whose fields all pair
/// right, with one field or two in each row of the grid beside it.
#[test]
fn a_grid_beside_another_of_another_row_pitch_pairs_both() {
    for two in [true, false] {
        for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
            let b = grids_side_by_side(&g, two);
            let out = read_pages(&[&b]);
            let broken = check_fields(&out, &b);
            assert!(
                broken.is_empty(),
                "{look}, two {two}: {broken:#?}\n{:#?}",
                blocks(&out)
            );
        }
    }
}

/// One field alone between two rules is not paired (a caption over a line between rules reads
/// the same), and a loss names it: it is never left out without a word.
#[test]
fn a_lone_field_between_rules_is_named() {
    assert_fields(&shape_page(&REAL, lone_field));
}

/// The shape file's variants a to d of text over the top rule of a header grid, by name.
fn title_blocks() -> Vec<(&'static str, TitleBlock)> {
    vec![
        ("a", BLOCK_A),
        (
            "b",
            TitleBlock {
                blatt: None,
                ..BLOCK_A
            },
        ),
        (
            "c",
            TitleBlock {
                title: None,
                period: false,
                date: false,
                blatt: None,
                code: false,
                ..BLOCK_A
            },
        ),
        (
            "d, title at the values' depth",
            TitleBlock {
                title: Some(0.17),
                ..BLOCK_A
            },
        ),
        (
            "d, label over the title",
            TitleBlock {
                blatt: Some(0.40),
                ..BLOCK_A
            },
        ),
    ]
}

/// The invariants of a grid whose fields must all pair as one table: those of
/// [`check_fields`], no loss that names one of its `labels`, and one table that holds them all.
fn check_one_table(out: &rezzer_model::ReadOutcome, b: &Built, labels: &[&str]) -> Vec<Broken> {
    let mut broken = check_fields(out, b);
    for l in &out.document.losses {
        if let Some(name) = labels.iter().find(|n| l.what.contains(&format!(" {n}"))) {
            broken.push(Broken(
                Inv::ClearPairs,
                format!("a loss names {name}: {}", l.what),
            ));
        }
    }
    let tables = blocks(out)
        .iter()
        .filter(|bl| match bl {
            Block::Table { rows, .. } => rows
                .iter()
                .any(|r| labels.contains(&plain_text(&r[0].inlines).trim())),
            _ => false,
        })
        .count();
    if tables != 1 {
        broken.push(Broken(
            Inv::ClearPairs,
            format!("the grid is {tables} tables"),
        ));
    }
    broken
}

/// A header grid with the title block of a pay slip over its top rule (the shape file's
/// variants a to d): a bold proportional title of the data size, a period line, a date and a
/// page number in the data font that cross the dividers under them or start at no field, a
/// label, a code line higher up, and an address window under part of its bottom rule. Text on
/// one side of a grid only is no evidence that its labels stand under their values: every
/// field pairs, labels over values, as one table, with no loss.
#[test]
fn a_title_block_over_a_header_grid_leaves_it_whole() {
    let labels = title_grid_labels();
    let mut reads = Vec::new();
    for (variant, over) in title_blocks() {
        for (geo, g) in geometries().into_iter().chain(looks()) {
            let b = title_grid_page(&g, over);
            let out = read_pages(&[&b]);
            reads.push((
                format!("{variant}, {geo}"),
                check_one_table(&out, &b, &labels),
            ));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Variant e: rows of fields with their labels under the rule their values stand on, and the
/// document title over the top rule across a divider: the title is in no value's font, the
/// first row's values stand over the top rule and the last row's labels under the bottom
/// rule, so the rows still pair labels under values.
#[test]
fn labels_under_values_with_a_title_over_them_pair_across_the_rule() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let mut reads = Vec::new();
    for (geo, g) in geometries().into_iter().chain(looks()) {
        let mut b = rows_page(&g, &s, &THREE_ROWS, &[]);
        title_over(&g, &mut b);
        let out = read_pages(&[&b]);
        reads.push((geo.to_string(), check_fields(&out, &b)));
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Three rows of two fields, the first row's values empty.
const EMPTY_FIRST: [&[FieldSpec]; 3] = [
    &[(0, 30, "Name", ""), (30, 20, "Ort", "")],
    &[(0, 30, "Strasse", "Musterweg 1"), (30, 20, "Hausnr", "12")],
    &[(0, 30, "PLZ", "12345"), (30, 20, "Land", "DE")],
];

/// Variant f: rows of fields with their labels under the rule their values stand on, the
/// first row's values empty, and a period line in the values' font over the top rule inside
/// the first field, but not at its start. The labels under the bottom rule show labels under
/// values, the period line stands where no value of a field starts: neither reading is clear,
/// so no field pairs (never the first label with the period line, never a label with the
/// value of the row under it), and a loss names them all.
#[test]
fn labels_under_values_with_a_line_over_an_empty_first_row_give_a_loss() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let all = ["Name", "Ort", "Strasse", "Hausnr", "PLZ", "Land"];
    let mut reads = Vec::new();
    for (geo, g) in geometries().into_iter().chain(looks()) {
        let mut b = rows_page(&g, &s, &EMPTY_FIRST, &all);
        let t = "fuer Muster 2026";
        b.page.template += &draw::courier(g.col(8), 760.0 + g.dy + 0.17 * g.data, g.data, t);
        b.values.push(t.to_string());
        let out = read_pages(&[&b]);
        reads.push((geo.to_string(), check_fields(&out, &b)));
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Rows of fields with their labels under the rule their values stand on, the first row's
/// values empty and nothing over the top rule: the labels under the bottom rule show labels
/// under values, but the same page is a grid of labels over values with the column titles of
/// a table under it. Neither reading is clear: no field pairs (never a label with the value of
/// the row under it), and a loss names them all.
#[test]
fn labels_under_values_with_an_empty_first_row_give_a_loss() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let all = ["Name", "Ort", "Strasse", "Hausnr", "PLZ", "Land"];
    let mut reads = Vec::new();
    for (geo, g) in geometries().into_iter().chain(looks()) {
        let b = rows_page(&g, &s, &EMPTY_FIRST, &all);
        let out = read_pages(&[&b]);
        reads.push((geo.to_string(), check_fields(&out, &b)));
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Three rows of two fields, the last row's values empty.
const EMPTY_LAST: [&[FieldSpec]; 3] = [
    &[
        (0, 30, "Name", "Erika Mustermann"),
        (30, 20, "Ort", "Musterstadt"),
    ],
    &[(0, 30, "Strasse", "Musterweg 1"), (30, 20, "Hausnr", "12")],
    &[(0, 30, "PLZ", ""), (30, 20, "Land", "")],
];

/// Three rows of one field each (no dividers), the last value empty or not.
const ONE_FIELD_ROWS: [&[FieldSpec]; 3] = [
    &[(0, 50, "Name", "Erika Mustermann")],
    &[(0, 50, "Strasse", "Musterweg 1")],
    &[(0, 50, "Ort", "")],
];

const ONE_FIELD_ROWS_FULL: [&[FieldSpec]; 3] = [
    &[(0, 50, "Name", "Erika Mustermann")],
    &[(0, 50, "Strasse", "Musterweg 1")],
    &[(0, 50, "Ort", "Musterstadt")],
];

/// Grids of labels over values whose last row is empty or which have one field a row, with a
/// line of data over the top rule, at every field start, and with or without the column title
/// of a next table under the last rule, in the real look and each of [`looks`]. A last row of
/// labels only looks like the labels under the last rule of a grid of labels under values,
/// and one field a row is too little evidence for that reading: the fields never pair shifted.
/// With a full line over them they stay text with a loss; with nothing over them they pair.
#[test]
fn grids_with_an_empty_last_row_or_one_field_a_row_never_pair_shifted() {
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        let top = 760.0 + g.dy;
        let line = |b: &mut Built, parts: &[(usize, &'static str)]| {
            for &(k, t) in parts {
                b.page.template +=
                    &draw::courier(g.col(k) + 0.3 * g.pitch(), top + 0.17 * g.data, g.data, t);
                b.values.push(t.into());
            }
        };
        let unclear = |b: &mut Built| {
            for f in &mut b.fields {
                f.1 = Err(());
            }
        };
        // An empty last row and a line with a value at both field starts.
        let mut b = rows_page(&g, &ROWS, &EMPTY_LAST, &[]);
        line(&mut b, &[(0, "Abrechnung"), (30, "Oktober")]);
        unclear(&mut b);
        reads.push((format!("empty last row, a full line over it, {look}"), b));
        // The same with nothing over it: it pairs.
        let b = rows_page(&g, &ROWS, &EMPTY_LAST, &[]);
        reads.push((format!("empty last row, {look}"), b));
        // One field a row, its last value empty, a heading line over it.
        let one = Rows {
            dividers: false,
            ..ROWS
        };
        let mut b = rows_page(&g, &one, &ONE_FIELD_ROWS, &[]);
        line(&mut b, &[(0, "Abrechnung Oktober 2026")]);
        unclear(&mut b);
        reads.push((format!("one field a row, a line over it, {look}"), b));
        // One field a row, all filled, a heading line over it and a column title under it.
        let mut b = rows_page(&g, &one, &ONE_FIELD_ROWS_FULL, &[]);
        line(&mut b, &[(0, "Abrechnung Oktober 2026")]);
        let bottom = top - 3.0 * 2.4 * g.data;
        b.page.template += &draw::courier(
            g.col(0) + 0.18 * g.label,
            bottom - 0.86 * g.data,
            g.label,
            "Lohnart",
        );
        unclear(&mut b);
        reads.push((
            format!("one field a row, a line over it and a title under it, {look}"),
            b,
        ));
    }
    let reads = reads
        .into_iter()
        .map(|(name, b)| {
            let out = read_pages(&[&b]);
            let mut broken = check_fields(&out, &b);
            // A loss names each label once.
            for l in &out.document.losses {
                let Some(names) = l
                    .what
                    .strip_prefix("the fields ")
                    .and_then(|w| w.split(" of a row").next())
                else {
                    continue;
                };
                let list: Vec<&str> = names.split(", ").collect();
                if (1..list.len()).any(|k| list[..k].contains(&list[k])) {
                    broken.push(Broken(Inv::UnclearSaid, format!("named twice: {}", l.what)));
                }
            }
            (name, broken)
        })
        .collect();
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Labels under the rule their values stand on, one field a row, the first value empty and
/// nothing over the top rule (the reviewer's probe E): the last label under the bottom rule is
/// one complete end, which a column of labels over values with a column title under it has as
/// well. Neither reading is clear: no field pairs (never a label with the value of the row
/// under it), and a loss names them all.
#[test]
fn a_column_of_labels_under_values_with_an_empty_first_value_gives_a_loss() {
    let s = Rows {
        under: true,
        dividers: false,
        ..ROWS
    };
    let rows: [&[FieldSpec]; 3] = [
        &[(0, 50, "Name", "")],
        &[(0, 50, "Strasse", "Musterweg 1")],
        &[(0, 50, "Ort", "Musterstadt")],
    ];
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        let b = rows_page(&g, &s, &rows, &["Name", "Strasse", "Ort"]);
        let out = read_pages(&[&b]);
        reads.push((look.to_string(), check_fields(&out, &b)));
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Labels under the rule their values stand on, two fields a row, every row full, one value
/// of two lines (the reviewer's probe P1): both ends are complete, but the strip of the
/// two-line value is no line of labels over one line of values, which is how labels under
/// values are paired. So no field pairs (never a label with the value of the row under it),
/// and a loss names them all.
#[test]
fn labels_under_values_with_a_value_of_two_lines_give_a_loss() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let all = ["Name", "Ort", "Strasse", "Hausnr", "PLZ", "Land"];
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        let mut b = rows_page(&g, &s, &THREE_ROWS, &all);
        let rule = 760.0 + g.dy - 2.4 * g.data;
        let t = "Hinterhaus";
        let y = rule + 0.17 * g.data + 0.95 * g.data;
        b.page.template += &draw::courier(g.col(0) + 0.3 * g.pitch(), y, g.data, t);
        let out = read_pages(&[&b]);
        reads.push((look.to_string(), check_fields(&out, &b)));
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Labels under the rule their values stand on, every row full, under a section rule one row
/// pitch over the top rule with a bold title in the frame it makes over the first row's values
/// (the reviewer's probe P2), with and without a rule one pitch under the last labels. The
/// title makes that frame a row refused with a loss; its values are still the top end. With
/// the rule under the last labels (an own bottom) one end is complete: no field pairs and a
/// loss names them all. Without it both ends are complete: the fields pair labels under values.
#[test]
fn labels_under_values_under_a_titled_section_rule_never_pair_shifted() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let all = ["Name", "Ort", "Strasse", "Hausnr", "PLZ", "Land"];
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        for (size, own) in [
            (1.25f32, true),
            (1.25, false),
            (1.003, true),
            (1.003, false),
        ] {
            let mut b = rows_page(&g, &s, &THREE_ROWS, if own { &all } else { &[] });
            let (d, h) = (g.data, 2.4 * g.data);
            let top = 760.0 + g.dy;
            b.page.template += &g.hrule(g.col(0) - 0.12, g.col(50), top + h);
            if own {
                b.page.template += &g.hrule(g.col(0) - 0.12, g.col(50), top - 3.0 * h);
            }
            let t = "Anschrift";
            b.page.template += &draw::bold(g.col(0), top + h - 1.2 * d * size, d * size, t);
            b.values.push(t.into());
            let out = read_pages(&[&b]);
            let name = format!("title {size}, {}, {look}", if own { "own" } else { "free" });
            reads.push((name, check_fields(&out, &b)));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Labels under the rule their values stand on, every row full, with a second rule 3 or 4
/// points under the second rule (a double rule, farther apart than one rule drawn in pieces)
/// and with or without a rule one pitch under the last labels (the reviewer's probe P3). The
/// double rule parts the grid into two groups; the values over the lower group's top rule are
/// held by the upper group. Neither group may pair shifted: each gives a loss that names its
/// fields.
#[test]
fn labels_under_values_split_by_a_double_rule_give_a_loss() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let all = ["Name", "Ort", "Strasse", "Hausnr", "PLZ", "Land"];
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        for (gap, own) in [(3.0f32, true), (3.0, false), (4.0, true)] {
            let mut b = rows_page(&g, &s, &THREE_ROWS, &all);
            let (h, top) = (2.4 * g.data, 760.0 + g.dy);
            b.page.template += &g.hrule(g.col(0) - 0.12, g.col(50), top - h - gap);
            if own {
                b.page.template += &g.hrule(g.col(0) - 0.12, g.col(50), top - 3.0 * h);
            }
            let out = read_pages(&[&b]);
            let name = format!("gap {gap}, {}, {look}", if own { "own" } else { "free" });
            reads.push((name, check_fields(&out, &b)));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Labels under the rule their values stand on, every row full, under a section rule one row
/// pitch over the top rule, with upright lines boxing that frame into two cells whose divider
/// is not in line with the grid's (the reviewer's probe P5), with and without a rule one pitch
/// under the last labels. The box holds the first row's values, and the grid's first row is a
/// row of fields under the box's lowest rule. With the rule under the last labels (an own
/// bottom) one end is complete: no field pairs and a loss names them all. Without it both ends
/// are complete: the fields pair labels under values. The grid never pairs shifted.
#[test]
fn labels_under_values_under_a_boxed_section_never_pair_shifted() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let all = ["Name", "Ort", "Strasse", "Hausnr", "PLZ", "Land"];
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        for (mid, own) in [(20usize, true), (20, false), (45, true), (45, false)] {
            let mut b = rows_page(&g, &s, &THREE_ROWS, if own { &all } else { &[] });
            let (h, top) = (2.4 * g.data, 760.0 + g.dy);
            let (x0, x1) = (g.col(0) - 0.12, g.col(50));
            b.page.template += &g.hrule(x0, x1, top + h);
            for x in [x0, g.col(mid), x1] {
                b.page.template += &g.vrule(x, top, top + h);
            }
            if own {
                b.page.template += &g.hrule(x0, x1, top - 3.0 * h);
            }
            let out = read_pages(&[&b]);
            let name = format!("box at {mid}, {}, {look}", if own { "own" } else { "free" });
            reads.push((name, check_fields(&out, &b)));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The reviewer's probe P4: as [`labels_under_values_under_a_boxed_section_never_pair_shifted`],
/// with the box's divider in line with the grid's, so that the box and the grid's first row
/// make one grid of drawn rules, its cells holding the first row's values over its labels.
/// The box's grid ends at its lowest rule: the grid's first row is a row of fields under it,
/// and the box's divider hangs into it. With a rule one pitch under the last labels (an own
/// bottom) no field pairs and a loss names them all; without it the fields pair labels under
/// values. No table sets a value over the wrong label.
#[test]
fn labels_under_values_under_a_box_in_line_with_their_divider_never_pair_shifted() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let all = ["Name", "Ort", "Strasse", "Hausnr", "PLZ", "Land"];
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        for own in [true, false] {
            let mut b = rows_page(&g, &s, &THREE_ROWS, if own { &all } else { &[] });
            let (h, top) = (2.4 * g.data, 760.0 + g.dy);
            let (x0, x1) = (g.col(0) - 0.12, g.col(50));
            b.page.template += &g.hrule(x0, x1, top + h);
            for x in [x0, g.col(30) - 0.06, x1] {
                b.page.template += &g.vrule(x, top, top + h);
            }
            if own {
                b.page.template += &g.hrule(x0, x1, top - 3.0 * h);
            }
            let out = read_pages(&[&b]);
            let name = format!("{}, {look}", if own { "own" } else { "free" });
            reads.push((name, check_fields(&out, &b)));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Labels under the rule their values stand on, four rows of two fields, parted into three
/// groups by two double rules (a second rule 3 points under the second and the third rule),
/// the middle group holding a value of two lines (`Hinterhof` over `12345`), with some values
/// empty (the reviewer's probe P7). The middle group's ends are partial and held by the
/// groups beside it: it is not clear, and no group pairs shifted. Every value stands once,
/// and each field with a value pairs right or a loss names it.
#[test]
fn labels_under_values_in_three_groups_with_a_value_of_two_lines_never_pair_shifted() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        for (hausnr, land) in [("", ""), ("", "DE"), ("12", "")] {
            let rows: [&[FieldSpec]; 4] = [
                &[
                    (0, 30, "Name", "Erika Mustermann"),
                    (30, 20, "Ort", "Musterstadt"),
                ],
                &[
                    (0, 30, "Strasse", "Musterweg 1"),
                    (30, 20, "Hausnr", hausnr),
                ],
                &[(0, 30, "PLZ", "12345"), (30, 20, "Land", land)],
                &[(0, 30, "Tel", "0000"), (30, 20, "Fax", "1111")],
            ];
            let mut b = rows_page(&g, &s, &rows, &[]);
            let (h, top) = (2.4 * g.data, 760.0 + g.dy);
            let (x0, x1) = (g.col(0) - 0.12, g.col(50));
            b.page.template += &g.hrule(x0, x1, top - h - 3.0);
            b.page.template += &g.hrule(x0, x1, top - 2.0 * h - 3.0);
            let t = "Hinterhof";
            let y = top - 2.0 * h + 0.17 * g.data + 0.75 * g.data;
            b.page.template += &draw::courier(g.col(0) + 0.3 * g.pitch(), y, g.data, t);
            b.values.push(t.into());
            let out = read_pages(&[&b]);
            // As `check_fields` looks for a name in a loss; an empty field left as text loses
            // no value.
            let named = |n: &str| {
                out.document
                    .losses
                    .iter()
                    .any(|l| l.what.contains(&format!(" {n}")))
            };
            let empty = |n: &str| b.fields.iter().any(|f| f.0 == n && f.1 == Ok(None));
            let broken: Vec<Broken> = check_fields(&out, &b)
                .into_iter()
                .filter(|x| {
                    let field = x.1.strip_suffix(" does not pair");
                    !(x.0 == Inv::ClearPairs && field.is_some_and(|n| named(n) || empty(n)))
                })
                .collect();
            reads.push((format!("Hausnr {hausnr:?}, Land {land:?}, {look}"), broken));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The reviewer's probe P7 without its value of two lines: labels under the rule their values
/// stand on, four rows of two fields, parted into three groups by two double rules, with
/// some values empty. The middle group's ends lie in the groups beside it: with `Hausnr` and
/// `Land` empty both are partial, which with the runs those groups hold is not clear. No
/// group pairs shifted; each field with a value pairs right or a loss names it.
#[test]
fn labels_under_values_in_three_groups_never_pair_shifted() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        for (hausnr, land) in [("", ""), ("", "DE"), ("12", "")] {
            let rows: [&[FieldSpec]; 4] = [
                &[
                    (0, 30, "Name", "Erika Mustermann"),
                    (30, 20, "Ort", "Musterstadt"),
                ],
                &[
                    (0, 30, "Strasse", "Musterweg 1"),
                    (30, 20, "Hausnr", hausnr),
                ],
                &[(0, 30, "PLZ", "12345"), (30, 20, "Land", land)],
                &[(0, 30, "Tel", "0000"), (30, 20, "Fax", "1111")],
            ];
            let mut b = rows_page(&g, &s, &rows, &[]);
            let (h, top) = (2.4 * g.data, 760.0 + g.dy);
            let (x0, x1) = (g.col(0) - 0.12, g.col(50));
            b.page.template += &g.hrule(x0, x1, top - h - 3.0);
            b.page.template += &g.hrule(x0, x1, top - 2.0 * h - 3.0);
            let out = read_pages(&[&b]);
            let named = |n: &str| {
                out.document
                    .losses
                    .iter()
                    .any(|l| l.what.contains(&format!(" {n}")))
            };
            let empty = |n: &str| b.fields.iter().any(|f| f.0 == n && f.1 == Ok(None));
            let broken: Vec<Broken> = check_fields(&out, &b)
                .into_iter()
                .filter(|x| {
                    let field = x.1.strip_suffix(" does not pair");
                    !(x.0 == Inv::ClearPairs && field.is_some_and(|n| named(n) || empty(n)))
                })
                .collect();
            reads.push((format!("Hausnr {hausnr:?}, Land {land:?}, {look}"), broken));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Where the end of a grid of labels under values lies in [`labels_under_values_with_a_value_of_two_lines_between_tables_give_a_loss`]:
/// free, in a box of drawn rules over (or under) it whose divider is at the data column
/// `mid`, in a ruled table of two rows (its lower row over the grid, its upper row under
/// it), or (under it) the line right over a ruled grid under its last labels.
#[derive(Debug, Clone, Copy)]
enum EndIn {
    Free,
    Box(usize),
    Table,
    Caption,
}

/// The reviewer's probe for the ruled `tabled` exclusion: labels under the rule their values
/// stand on, every row full, the second row's first value of two lines (`Hinterhof` over
/// `Musterweg 1`), its ends in boxes of drawn rules (their divider in line with the grid's, or
/// not), in ruled tables of two rows, one end free, or the last labels right over a ruled grid
/// under the grid. A grid
/// with a strip of another shape never reads labels under values: no field pairs with the
/// value of another row, and each field with a value pairs right or a loss names it.
#[test]
fn labels_under_values_with_a_value_of_two_lines_between_tables_give_a_loss() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        for (over, under) in [
            (EndIn::Table, EndIn::Table),
            (EndIn::Table, EndIn::Free),
            (EndIn::Free, EndIn::Table),
            (EndIn::Box(30), EndIn::Box(30)),
            (EndIn::Box(20), EndIn::Box(20)),
            (EndIn::Box(30), EndIn::Free),
            (EndIn::Free, EndIn::Box(30)),
            (EndIn::Free, EndIn::Caption),
            (EndIn::Box(30), EndIn::Caption),
        ] {
            let mut b = rows_page(&g, &s, &THREE_ROWS, &[]);
            let (d, h, top) = (g.data, 2.4 * g.data, 760.0 + g.dy);
            let (x0, x1) = (g.col(0) - 0.12, g.col(50));
            let last = top - 2.0 * h;
            let t = "Hinterhof";
            let y = top - h + 0.17 * d + 0.75 * d;
            b.page.template += &draw::courier(g.col(0) + 0.3 * g.pitch(), y, d, t);
            b.values.push(t.into());
            let boxed = |b: &mut Built, mid: usize, lo: f32, hi: f32| {
                b.page.template += &g.hrule(x0, x1, if lo < last { lo } else { hi });
                let x = if mid == 30 {
                    g.col(30) - 0.06
                } else {
                    g.col(mid)
                };
                for x in [x0, x, x1] {
                    b.page.template += &g.vrule(x, lo, hi);
                }
            };
            // A ruled table of two rows from `hi` down, its divider in line with the grid's, the
            // text `row` in the row away from the grid.
            let table = |b: &mut Built, hi: f32, row: f32, text: [&'static str; 2]| {
                for k in 0..3 {
                    b.page.template += &g.hrule(x0, x1, hi - h * k as f32);
                }
                for x in [x0, g.col(30) - 0.06, x1] {
                    b.page.template += &g.vrule(x, hi - 2.0 * h, hi);
                }
                for (k, t) in [0, 30].into_iter().zip(text) {
                    let x = g.col(k) + 0.3 * g.pitch();
                    b.page.template += &draw::courier(x, row + 0.6 * d, d, t);
                    b.values.push(t.to_string());
                }
            };
            match over {
                EndIn::Box(mid) => boxed(&mut b, mid, top, top + h),
                EndIn::Table => table(&mut b, top + 2.0 * h, top + h, ["Anschrift", "Kontakt"]),
                _ => {}
            }
            match under {
                EndIn::Free => {}
                EndIn::Box(mid) => boxed(&mut b, mid, last - h, last),
                EndIn::Table => table(&mut b, last, last - 2.0 * h, ["1000", "1.111,11"]),
                EndIn::Caption => {
                    // A ruled grid right under the last labels, cut in two sections of other
                    // columns by a rule across, as a section titled by the line over it is.
                    // (The last labels stand between the grid's last rule and this grid's top:
                    // the grid search takes them as a band of their own, not as a title.)
                    let t0 = last - 0.86 * d - 0.25 * d;
                    // Each section: its top, its column edges and its two rows of cells.
                    type Section<'a> = (f32, &'a [f32], [[&'a str; 3]; 2]);
                    let sections: [Section; 2] = [
                        (
                            t0,
                            &[x0, g.col(15), g.col(35), x1],
                            [
                                ["Lohnart", "Bezeichnung", "Betrag"],
                                ["1000", "Gehalt", "1.111,11"],
                            ],
                        ),
                        (
                            t0 - 2.0 * h,
                            &[x0, g.col(25), x1],
                            [["Summe", "2.222,22", ""], ["Netto", "1.000,00", ""]],
                        ),
                    ];
                    for k in 0..5 {
                        b.page.template += &g.hrule(x0, x1, t0 - h * k as f32);
                    }
                    for (hi, xs, cells) in sections {
                        for &x in xs {
                            b.page.template += &g.vrule(x, hi - 2.0 * h, hi);
                        }
                        for (k, row) in cells.iter().enumerate() {
                            for (c, text) in row.iter().enumerate().filter(|t| !t.1.is_empty()) {
                                let x = xs[c] + 0.3 * g.pitch();
                                let y = hi - h * (k as f32 + 1.0) + 0.6 * d;
                                b.page.template += &draw::courier(x, y, d, text);
                                b.values.push(text.to_string());
                            }
                        }
                    }
                }
            }
            let out = read_pages(&[&b]);
            let named = |n: &str| {
                out.document
                    .losses
                    .iter()
                    .any(|l| l.what.contains(&format!(" {n}")))
            };
            let broken: Vec<Broken> = check_fields(&out, &b)
                .into_iter()
                .filter(|x| {
                    let field = x.1.strip_suffix(" does not pair");
                    !(x.0 == Inv::ClearPairs && field.is_some_and(named))
                })
                .collect();
            reads.push((format!("over {over:?}, under {under:?}, {look}"), broken));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Rows of two fields between horizontal rules alone, with no upright rule on the page: the
/// fields of a row are parted by their labels, labels over or under their values. Each reads
/// as it does with a lone upright rule far from the grid, and pairs.
#[test]
fn rows_of_fields_between_horizontal_rules_alone_pair() {
    let mut reads = Vec::new();
    for under in [false, true] {
        let s = Rows {
            under,
            dividers: false,
            ..ROWS
        };
        for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
            let with = rows_page(&g, &s, &THREE_ROWS, &[]);
            let lone = g.vrule(30.0 + g.dx, 200.0 + g.dy, 300.0 + g.dy);
            let mut b = rows_page(&g, &s, &THREE_ROWS, &[]);
            b.page.template = b.page.template.replace(&lone, "");
            assert_ne!(b.page.template, with.page.template);
            let (out, twin) = (read_pages(&[&b]), read_pages(&[&with]));
            let mut broken = check_fields(&out, &b);
            let read = |o: &rezzer_model::ReadOutcome| {
                let losses: Vec<String> =
                    o.document.losses.iter().map(|l| l.what.clone()).collect();
                (common::text(blocks(o)), losses)
            };
            if read(&out) != read(&twin) {
                broken.push(Broken(
                    Inv::ClearPairs,
                    format!("reads otherwise with the upright rule: {:?}", read(&twin)),
                ));
            }
            let name = format!("labels {}, {look}", if under { "under" } else { "over" });
            reads.push((name, broken));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// As [`labels_under_values_with_a_value_of_two_lines_between_tables_give_a_loss`] with both ends
/// in ruled tables of two rows, but the strip of another shape is a mix: its first row holds,
/// beside the labels under the values over them, a third field whose label stands beside its
/// value (`Kst 4711`). No field pairs with the value of another row, and each field with a
/// value pairs right or a loss names it.
#[test]
fn labels_under_values_beside_a_label_and_value_between_tables_never_pair_shifted() {
    let s = Rows {
        under: true,
        ..ROWS
    };
    let mut reads = Vec::new();
    for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
        let mut b = rows_page(&g, &s, &THREE_ROWS, &[]);
        let (d, h, top) = (g.data, 2.4 * g.data, 760.0 + g.dy);
        let (x0, x1) = (g.col(0) - 0.12, g.col(50));
        // The rules run on to column 70; a divider at column 50 parts the third field.
        for k in 0..3 {
            b.page.template += &g.hrule(x1, g.col(70), top - h * k as f32);
        }
        let span = 1.21 * d;
        b.page.template += &if g.rising {
            g.vrule(x1 - 0.06, top - h, top - h + span)
        } else {
            g.vrule(x1 - 0.06, top - span, top)
        };
        let y = top - 0.86 * d;
        b.page.template += &draw::courier(g.col(51), y, g.label, "Kst");
        b.page.template += &draw::courier(g.col(56), y, d, "4711");
        b.values.push("4711".into());
        b.fields.push(("Kst", Ok(Some("4711"))));
        // Ruled tables of two rows over the grid and under it, their divider in line with the
        // grid's, each with the text of its row away from the grid.
        let table = |b: &mut Built, hi: f32, row: f32, text: [&'static str; 2]| {
            for k in 0..3 {
                b.page.template += &g.hrule(x0, x1, hi - h * k as f32);
            }
            for x in [x0, g.col(30) - 0.06, x1] {
                b.page.template += &g.vrule(x, hi - 2.0 * h, hi);
            }
            for (k, t) in [0, 30].into_iter().zip(text) {
                let x = g.col(k) + 0.3 * g.pitch();
                b.page.template += &draw::courier(x, row + 0.6 * d, d, t);
                b.values.push(t.to_string());
            }
        };
        table(&mut b, top + 2.0 * h, top + h, ["Anschrift", "Kontakt"]);
        let last = top - 2.0 * h;
        table(&mut b, last, last - 2.0 * h, ["1000", "1.111,11"]);
        let out = read_pages(&[&b]);
        let named = |n: &str| {
            out.document
                .losses
                .iter()
                .any(|l| l.what.contains(&format!(" {n}")))
        };
        let broken: Vec<Broken> = check_fields(&out, &b)
            .into_iter()
            .filter(|x| {
                let field = x.1.strip_suffix(" does not pair");
                !(x.0 == Inv::ClearPairs && field.is_some_and(named))
            })
            .collect();
        reads.push((look.to_string(), broken));
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A row between two rules, parted by a divider, whose every cell holds text in three styles
/// (a caption, a line of text and a note, as on a page of prose between rules): it never
/// looked like a row of fields, so it gives no loss.
#[test]
fn a_row_of_three_styles_between_rules_gives_no_loss() {
    let mut b = rows_page(
        &REAL,
        &ROWS,
        &[&[(0, 30, "Abb", ""), (30, 20, "Tab", "")]],
        &[],
    );
    let (top, d) = (760.0, REAL.data);
    for k in [0usize, 30] {
        let x = REAL.col(k) + 0.3 * REAL.pitch();
        b.page.template += &draw::courier(x, top - 1.5 * d, d, "Muster");
        b.page.template += &draw::courier(x, top - 2.2 * d, 0.8 * d, "Beispiel");
    }
    b.fields.clear();
    let out = read_pages(&[&b]);
    let row: Vec<&str> = out
        .document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .filter(|w| w.contains("row of fields"))
        .collect();
    assert!(row.is_empty(), "{row:#?}");
}

/// A bold heading over lines of smaller prose between two rules (a picture of a page): its
/// first style met is the larger one, but its text is longer than a field's, so it never looked
/// like a row of fields and gives no loss.
#[test]
fn a_heading_over_prose_between_rules_gives_no_loss() {
    // Without dividers a vertical rule stands far from the row: the page has both directions.
    let s = Rows {
        dividers: false,
        ..ROWS
    };
    let mut b = rows_page(&REAL, &s, &[&[(0, 60, "Abb", "")]], &[]);
    let (top, d) = (760.0, REAL.data);
    b.page.template = b.page.template.replacen(
        &format!("{} Td (Abb)", top - 0.86 * d),
        &format!("{} Td ()", top - 0.86 * d),
        1,
    );
    b.page.template += &draw::bold(REAL.col(0), top - 0.7 * d, 0.7 * d, "MUSTER");
    for (k, l) in [
        "Das Muster ist ein Beispiel fuer einen Text ueber",
        "mehrere Zeilen, der in einem Rahmen steht und",
    ]
    .iter()
    .enumerate()
    {
        b.page.template +=
            &draw::plain(REAL.col(0), top - 1.3 * d - 0.5 * d * k as f32, 0.4 * d, l);
    }
    b.fields.clear();
    let out = read_pages(&[&b]);
    let row: Vec<&str> = out
        .document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .filter(|w| w.contains("row of fields"))
        .collect();
    assert!(row.is_empty(), "{row:#?}");
}

/// A row of fields between two rules, parted by a divider, whose every field has its label
/// under its value: no field is read as labels and values, and a loss says that the row's text
/// is read as it stands, though it names no label (it cannot tell which text is one).
#[test]
fn a_row_of_fields_none_of_which_is_clear_gives_a_loss() {
    let mut b = rows_page(
        &REAL,
        &ROWS,
        &[&[(0, 30, "Name", ""), (30, 20, "Ort", "")]],
        &[],
    );
    let (top, d) = (760.0, REAL.data);
    for (k, v, l) in [
        (0usize, "Erika Mustermann", "Name"),
        (30, "Musterstadt", "Ort"),
    ] {
        let x = REAL.col(k) + 0.3 * REAL.pitch();
        b.page.template += &draw::courier(x, top - 0.86 * d - 0.3 * d, d, v);
        b.values.push(v.into());
        // The label at the bottom of its field.
        b.page.template = b.page.template.replacen(
            &format!("{} Td ({l})", top - 0.86 * d),
            &format!("{} Td ({l})", top - 2.23 * d),
            1,
        );
    }
    b.fields.clear();
    let out = read_pages(&[&b]);
    let broken = check_fields(&out, &b);
    assert!(broken.is_empty(), "{broken:#?}");
    assert!(pairs_of(blocks(&out)).is_empty(), "{:#?}", blocks(&out));
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what.contains("a row of fields on page 1")),
        "{:#?}",
        out.document.losses
    );
}

/// Two rows of two fields each without dividers, the second field narrow.
const NARROW: [&[FieldSpec]; 2] = [
    &[(0, 30, "Name", "Erika Mustermann"), (30, 12, "Nr", "30")],
    &[(0, 30, "Ort", "Musterstadt"), (30, 12, "Tag", "X")],
];

/// Labels set in the middle of their fields or at their right end, over values at the
/// fields' left ends, without dividers: a value under no label has no certain field, so the
/// fields it may belong to stay text with a loss (ruling P6-X); a value under its label
/// pairs.
#[test]
fn labels_away_from_their_values_never_pair_wrongly() {
    for labels in [Align::Center, Align::Right] {
        let s = Rows {
            labels,
            dividers: false,
            ..ROWS
        };
        assert_fields(&rows_page(
            &REAL,
            &s,
            &NARROW,
            &["Name", "Nr", "Ort", "Tag"],
        ));
    }
}

/// Values set at the right end of their fields under labels at the left end, without
/// dividers: they stand under no label, so their fields stay text with a loss.
#[test]
fn right_aligned_values_without_dividers_never_pair_wrongly() {
    let s = Rows {
        values: Align::Right,
        dividers: false,
        ..ROWS
    };
    assert_fields(&rows_page(
        &REAL,
        &s,
        &NARROW,
        &["Name", "Nr", "Ort", "Tag"],
    ));
}

/// The generated geometries of F1 to F3 besides the looks, by name.
fn geometries() -> Vec<(&'static str, Geo)> {
    vec![
        ("real", REAL),
        (
            "shifted",
            Geo {
                dx: -17.3,
                dy: -41.0,
                ..REAL
            },
        ),
        ("larger data", Geo { data: 10.5, ..REAL }),
        ("smaller data", Geo { data: 8.5, ..REAL }),
        ("larger labels", Geo { label: 7.5, ..REAL }),
        ("smaller labels", Geo { label: 5.6, ..REAL }),
        (
            "courier",
            Geo {
                font: DataFont::Courier,
                ..REAL
            },
        ),
        (
            "helvetica",
            Geo {
                font: DataFont::Helvetica,
                ..REAL
            },
        ),
        (
            "thick rules",
            Geo {
                rule: 0.6,
                bar: 1.2,
                ..REAL
            },
        ),
        (
            "hairlines",
            Geo {
                rule: 0.1,
                bar: 0.3,
                ..REAL
            },
        ),
        (
            "stroked bar",
            Geo {
                stroked_bar: true,
                ..REAL
            },
        ),
        (
            "no marks",
            Geo {
                marks: false,
                ..REAL
            },
        ),
        ("wider field", Geo { wide: 5, ..REAL }),
        (
            "one field less",
            Geo {
                fewer: true,
                ..REAL
            },
        ),
        (
            "wider and fewer, courier",
            Geo {
                wide: 3,
                fewer: true,
                font: DataFont::Courier,
                ..REAL
            },
        ),
        (
            "all changed",
            Geo {
                dx: 11.0,
                dy: 23.0,
                data: 10.0,
                label: 7.0,
                font: DataFont::Courier,
                rule: 0.4,
                bar: 1.0,
                stroked_bar: true,
                ..REAL
            },
        ),
    ]
}

/// The generated variants of F1 to F3, by name, each with whether its rate overflows and
/// whether its last two labels are one run.
fn variants() -> Vec<(String, Geo, bool, bool)> {
    let mut out = Vec::new();
    for (name, g) in geometries().into_iter().chain(looks()) {
        for overflow in [true, false] {
            let label = format!("{name}{}", if overflow { ", overflow" } else { "" });
            out.push((label, g, overflow, true));
        }
    }
    // The two short labels as two text objects, each in its field, close together.
    for (name, g) in [
        ("separate labels", REAL),
        ("separate labels, larger data", Geo { data: 10.5, ..REAL }),
        (
            "separate labels, smaller labels",
            Geo { label: 5.6, ..REAL },
        ),
    ] {
        out.push((name.to_string(), g, true, false));
    }
    out
}

/// The looks every shape is drawn in besides the real one: printed smaller and larger as a
/// whole, its thin rules dashed or dotted, its dividers rising from the rule under the row.
fn looks() -> Vec<(&'static str, Geo)> {
    vec![
        ("scale 0.7", Geo { scale: 0.7, ..REAL }),
        ("scale 1.5", Geo { scale: 1.5, ..REAL }),
        (
            "dashed rules",
            Geo {
                stroke: Stroke::Dashed,
                ..REAL
            },
        ),
        (
            "dotted rules",
            Geo {
                stroke: Stroke::Dotted,
                ..REAL
            },
        ),
        (
            "rising dividers",
            Geo {
                rising: true,
                ..REAL
            },
        ),
    ]
}

/// Variants of F1 to F3 that break an invariant today: the variant's name, the one invariant
/// it breaks, and why (reported, not tuned away). Every other invariant is still held for it,
/// and the test fails when a known variant starts to hold or breaks another one. No variant
/// may be known to break [`Inv::RightPair`] or [`Inv::ValueOnce`].
const KNOWN: &[Known] = &[(
    "separate labels",
    Inv::ClearPairs,
    "the two short labels are two text objects less than half a label size apart, so the \
     text pass reads them as one word without a space; that word stands across the divider \
     between them, so both fields stay text with a loss that names them",
)];

/// The check of known variants bites: a known variant that breaks another invariant than
/// its own, or that holds them all, is reported; so is any other variant that breaks one.
#[test]
fn known_variants_are_held_to_every_other_invariant() {
    let known: &[Known] = &[("k", Inv::ClearPairs, "a reason")];
    let read = |name: &str, broken: Vec<Broken>| vec![(name.to_string(), broken)];
    let b = |inv: Inv| Broken(inv, String::new());
    assert!(unexpected(read("k", vec![b(Inv::ClearPairs)]), known).is_empty());
    for other in [
        Inv::RightPair,
        Inv::ValueOnce,
        Inv::UnclearSaid,
        Inv::Title,
        Inv::NoteOnce,
    ] {
        let both = vec![b(Inv::ClearPairs), b(other)];
        assert_eq!(unexpected(read("k", both), known).len(), 1, "{other:?}");
    }
    assert_eq!(unexpected(read("k", Vec::new()), known).len(), 1);
    assert_eq!(
        unexpected(read("other", vec![b(Inv::Title)]), known).len(),
        1
    );
}

#[test]
fn generated_variants_hold_the_invariants() {
    let mut reads = Vec::new();
    for (name, g, overflow, one_run) in variants() {
        let b = page(&g, overflow, one_run);
        let out = read_pages(&[&b]);
        let mut broken = check_fields(&out, &b);
        broken.extend(check_title(&out));
        reads.push((name, broken));
    }
    let wrong = unexpected(reads, KNOWN);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A shape drawn on a page of its own in a geometry.
type Shape = fn(&Geo) -> Built;

/// The other shapes of the pay slip, each on a page of its own, by name.
fn shapes() -> Vec<(&'static str, Shape)> {
    vec![
        ("header grid", |g| shape_page(g, header_grid)),
        ("column of totals", |g| shape_page(g, total_column)),
        ("boxed fields", |g| shape_page(g, boxed_row)),
        ("label block and bank row", |g| shape_page(g, label_block)),
        ("overflow into an empty field", |g| {
            shape_page(g, overflow_rows)
        }),
        ("a short value after a full field", |g| {
            shape_page(g, short_value_rows)
        }),
        ("rows", |g| rows_page(g, &ROWS, &THREE_ROWS, &[])),
        ("labels under values", |g| {
            let s = Rows {
                under: true,
                ..ROWS
            };
            rows_page(g, &s, &THREE_ROWS, &[])
        }),
        ("centered labels", |g| {
            let s = Rows {
                labels: Align::Center,
                ..ROWS
            };
            rows_page(g, &s, &THREE_ROWS, &[])
        }),
        ("right-aligned values", |g| {
            let s = Rows {
                values: Align::Right,
                ..ROWS
            };
            rows_page(g, &s, &THREE_ROWS, &[])
        }),
    ]
}

/// A grid of fields drawn on a page of its own in a geometry, with where it stands.
type Grid = fn(&Geo) -> (Built, GridAt);

/// The grids of fields of the pay slip that neighbours are drawn beside, by name.
fn grids() -> Vec<(&'static str, Grid)> {
    vec![
        ("header grid", |g| {
            let mut b = Built::new(g);
            let top = 760.0 + g.dy;
            let bottom = header_grid(g, &mut b, top);
            b.address(g);
            let at = GridAt {
                top,
                bottom,
                starts: &[0, 10, 30],
                under: false,
                courier: false,
                whole_bottom: false,
            };
            (b, at)
        }),
        ("rows", |g| {
            let top = 760.0 + g.dy;
            let at = GridAt {
                top,
                bottom: top - 3.0 * 2.4 * g.data,
                starts: &[0, 30],
                under: false,
                courier: true,
                whole_bottom: true,
            };
            (rows_page(g, &ROWS, &THREE_ROWS, &[]), at)
        }),
        ("rows with an empty last row", |g| {
            let top = 760.0 + g.dy;
            let at = GridAt {
                top,
                bottom: top - 3.0 * 2.4 * g.data,
                starts: &[0, 30],
                under: false,
                courier: true,
                whole_bottom: false,
            };
            (rows_page(g, &ROWS, &EMPTY_LAST, &[]), at)
        }),
        ("labels under values", |g| {
            let s = Rows {
                under: true,
                ..ROWS
            };
            let top = 760.0 + g.dy;
            let at = GridAt {
                top,
                bottom: top - 2.0 * 2.4 * g.data,
                starts: &[0, 30],
                under: true,
                courier: true,
                whole_bottom: true,
            };
            (rows_page(g, &s, &THREE_ROWS, &[]), at)
        }),
    ]
}

/// The neighbours drawn beside each grid, by name. Text over the top rule at the values'
/// depth is drawn only beside grids whose labels stand over their values: over the others
/// their own values stand there.
fn neighbours(under: bool) -> Vec<(&'static str, Neighbour)> {
    let mut out = vec![
        (
            "column titles with their table",
            Neighbour::ColumnTitles { body: true },
        ),
        (
            "column titles alone",
            Neighbour::ColumnTitles { body: false },
        ),
        ("a grid beside it of another pitch", Neighbour::Beside),
        ("a rule over it", Neighbour::RuleOver),
        ("a rule under it", Neighbour::RuleUnder),
    ];
    if !under {
        out.extend([
            ("a data line over it", Neighbour::DataLine),
            ("a title at the values' depth over it", Neighbour::Title),
            ("the title block of a pay slip", Neighbour::Cluster),
            (
                "a short line over it and column titles alone under it",
                Neighbour::LineAndTitles,
            ),
            (
                "a line with a value at every field start over it",
                Neighbour::FullLine,
            ),
        ]);
    }
    out
}

/// Every grid of fields with each of its neighbours, in the real look and in each of
/// [`looks`], holds the invariants: text just over or under a grid on one side of it, or a grid
/// beside it with rows of another pitch, never makes its labels read under their values nor
/// refuses it, and the grid beside pairs too. Text at both ends that a grid of labels under
/// their values could have as well leaves the grid as text with a loss, never pairs it.
#[test]
fn grids_with_neighbours_hold_the_invariants() {
    let mut reads = Vec::new();
    for (grid, draw) in grids() {
        for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
            let (_, at) = draw(&g);
            for (name, n) in neighbours(at.under) {
                let (mut b, at) = draw(&g);
                neighbour(&g, &mut b, at, n);
                let out = read_pages(&[&b]);
                reads.push((format!("{grid}, {name}, {look}"), check_fields(&out, &b)));
            }
        }
    }
    let wrong = unexpected(reads, KNOWN_NEIGHBOURS);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Grids with neighbours that break an invariant today (see [`KNOWN`]).
const KNOWN_NEIGHBOURS: &[Known] = &[];

/// Shapes in looks that break an invariant today (see [`KNOWN`]).
const KNOWN_SHAPES: &[Known] = &[];

/// Every other shape of the pay slip in the real look and in each of [`looks`] holds the
/// invariants.
#[test]
fn generated_shapes_hold_the_invariants() {
    let mut reads = Vec::new();
    for (shape, draw) in shapes() {
        for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
            let b = draw(&g);
            let out = read_pages(&[&b]);
            reads.push((format!("{shape}, {look}"), check_fields(&out, &b)));
        }
    }
    let wrong = unexpected(reads, KNOWN_SHAPES);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A form of two pages, each of several shapes: every page holds the invariants on its own,
/// each note of a kind once on each page (ruling P6-W).
#[test]
fn a_form_of_two_pages_holds_the_invariants_on_each() {
    let first = page(&REAL, true, true);
    let mut second = Built::new(&REAL);
    let below = header_grid(&REAL, &mut second, 760.0);
    let below = total_column(&REAL, &mut second, below - 3.0 * REAL.data);
    label_block(&REAL, &mut second, below - 3.0 * REAL.data);
    second.address(&REAL);
    let out = read_pages(&[&first, &second]);
    let mut broken = check_page(&out, 0, &first);
    broken.extend(check_page(&out, 1, &second));
    assert!(broken.is_empty(), "{broken:#?}\n{:#?}", out.document.body);
}

/// Every shape drawn a quarter turn across the page never gives a wrong pair, and keeps
/// every value once. F1 to F3 are drawn without marks there: a separator drawn between digits
/// that do not read in the direction of the page's text is not read, by design (a loss says
/// so), so the values written with them would not stand as expected.
#[test]
fn turned_forms_never_pair_wrongly() {
    let g = Geo { turn: true, ..REAL };
    let plain = Geo { marks: false, ..g };
    let mut all: Vec<(String, Built)> = vec![("F1 to F3".to_string(), page(&plain, true, true))];
    all.extend(
        shapes()
            .into_iter()
            .map(|(n, draw)| (n.to_string(), draw(&g))),
    );
    let mut reads: Vec<(String, Vec<Broken>)> = Vec::new();
    for (name, b) in all {
        let out = read_pages(&[&b]);
        let broken: Vec<Broken> = check_fields(&out, &b)
            .into_iter()
            .filter(|b| matches!(b.0, Inv::RightPair | Inv::ValueOnce))
            .collect();
        reads.push((format!("{name}, turned"), broken));
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
