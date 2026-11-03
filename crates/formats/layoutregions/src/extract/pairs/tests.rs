// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::blocks::{
    ColumnLine, Columns, KeyValues, address_block, address_columns, address_lines, blocks_of,
    key_values,
};
use super::strips::{FieldFrame, field_cuts, first_line_labels, parts_of, strip_cells};
use super::*;
use crate::extract::table::{RunRange, grid::pieces_of, run_ranges};
use crate::input::{FontKind, GapIn, Shift, TextRun};

/// A run of `text` at `x` on the baseline `y` in `size`, 0.6 of the size a character, with a
/// gap at each space, on line 0.
fn run(text: &str, x: f32, y: f32, size: f32) -> TextRun {
    let ch = 0.6 * size;
    let mut gaps = Vec::new();
    let mut words = 0u32;
    for (k, c) in text.char_indices() {
        if c == ' ' {
            words += 1;
            gaps.push(GapIn {
                words,
                from: x + ch * k as f32,
                to: x + ch * (k + 1) as f32,
            });
        }
    }
    TextRun {
        text: text.into(),
        x,
        y,
        w: ch * text.chars().count() as f32,
        h: size,
        size,
        font: 0,
        font_kind: FontKind::Unknown,
        bold: false,
        italic: false,
        color: None,
        shift: Shift::Base,
        line: 0,
        off_axis: false,
        gaps,
    }
}

/// [`run`] on line `line`.
fn run_on(text: &str, x: f32, y: f32, size: f32, line: u32) -> TextRun {
    let mut r = run(text, x, y, size);
    r.line = line;
    r
}

fn texts(runs: &[TextRun], ids: &[usize]) -> Vec<String> {
    ids.iter().map(|&i| runs[i].text.clone()).collect()
}

/// A frame of fields from 0 to 700 with no dividers.
fn open_frame() -> FieldFrame<'static> {
    FieldFrame {
        x0: 0.0,
        x1: 700.0,
        dividers: &[],
    }
}

/// The cuts [`field_cuts`] finds in `runs` for one frame without dividers that holds all of
/// them.
fn cuts_in_open_frame(runs: &[TextRun]) -> Vec<(usize, Vec<usize>)> {
    let n = runs.len();
    field_cuts(runs, &[open_frame()], &mut |_| (0..n).collect())
}

/// A drawn box whose small label stands over its value is one field: the label's runs, then
/// the value's.
#[test]
fn box_pairs_label_over_value() {
    let runs = [
        run("Firma", 4.0, 120.0, 6.0),
        run_on("Muster GmbH", 4.0, 106.0, 10.0, 1),
    ];
    let (fields, unsure) = box_fields(&runs, &[vec![0, 1]], &[false], &[10.0]);
    let field = fields[0].as_ref().expect("a field");
    assert_eq!(texts(&runs, &field.label), ["Firma"]);
    assert_eq!(texts(&runs, &field.value), ["Muster GmbH"]);
    assert!(unsure.is_empty());
}

/// A drawn box that holds only a label smaller than the page's text is an empty field: no
/// value is made up for it.
#[test]
fn box_pairs_empty_field_stays_empty() {
    let runs = [run("Telefon", 4.0, 120.0, 6.0)];
    let (fields, unsure) = box_fields(&runs, &[vec![0]], &[false], &[10.0]);
    let field = fields[0].as_ref().expect("a field");
    assert_eq!(texts(&runs, &field.label), ["Telefon"]);
    assert!(field.value.is_empty());
    assert!(unsure.is_empty());
    // A box whose label stands below its value is not clear, and is said.
    let runs = [
        run("Muster GmbH", 4.0, 120.0, 10.0),
        run_on("Firma", 4.0, 106.0, 6.0, 1),
    ];
    let (fields, unsure) = box_fields(&runs, &[vec![0, 1]], &[false], &[10.0]);
    assert!(fields[0].is_none());
    assert_eq!(unsure, [(0, LABEL_BELOW)]);
}

/// A label and its value of one size that differ in slant only are not clear by themselves;
/// beside an empty field whose lone label has the slant, the slanted text is the label, and the
/// box is read again as a field.
#[test]
fn box_pairs_weight_only_is_read_again_with_a_known_label_style() {
    let italic = |mut r: TextRun| {
        r.italic = true;
        r
    };
    let runs = [
        italic(run("Kundennr.", 4.0, 120.0, 8.0)),
        italic(run("Datum", 104.0, 120.0, 8.0)),
        run_on("01.03.2026", 104.0, 108.0, 8.0, 1),
    ];
    // Alone, which line is the label is not clear.
    let (fields, unsure) = box_fields(&runs, &[vec![1, 2]], &[false], &[10.0]);
    assert!(fields[0].is_none());
    assert_eq!(unsure, [(0, WEIGHT_ONLY)]);
    // Beside the empty field, its label's slant marks the label.
    let (fields, unsure) = box_fields(&runs, &[vec![0], vec![1, 2]], &[false, false], &[10.0]);
    let empty = fields[0].as_ref().expect("an empty field");
    assert_eq!(texts(&runs, &empty.label), ["Kundennr."]);
    assert!(empty.value.is_empty());
    let field = fields[1].as_ref().expect("a field");
    assert_eq!(texts(&runs, &field.label), ["Datum"]);
    assert_eq!(texts(&runs, &field.value), ["01.03.2026"]);
    assert!(unsure.is_empty());
}

/// A box with no runs and a box a framed title took are read as no field.
#[test]
fn box_pairs_empty_or_consumed_box_is_no_field() {
    let runs = [
        run("Firma", 4.0, 120.0, 6.0),
        run_on("Muster GmbH", 4.0, 106.0, 10.0, 1),
    ];
    let (fields, unsure) = box_fields(&runs, &[vec![], vec![0, 1]], &[false, true], &[10.0]);
    assert!(fields.iter().all(Option::is_none));
    assert!(unsure.is_empty());
}

/// A value line that runs across a divider leaves the cells it stands in out of the row with
/// [`ACROSS`], named by their labels; the other cells of the row still pair.
#[test]
fn strip_cells_leave_out_the_cells_a_run_stands_across() {
    let runs = [
        run("Name", 10.0, 120.0, 6.0),
        run_on("Erika Mustermann Musterstadt", 10.0, 100.0, 10.0, 1),
        run("Ort", 110.0, 120.0, 6.0),
        run("Telefon", 210.0, 120.0, 6.0),
        run_on("0123 4567", 210.0, 100.0, 10.0, 1),
    ];
    let edges = [0.0, 100.0, 200.0, 300.0];
    let cells = [vec![0, 1], vec![2], vec![3, 4]];
    let how = Reading {
        one_field: false,
        smaller: true,
        small: small_size(&[10.0]).saturating_sub(1),
        labels: &[],
    };
    let read = strip_cells(&runs, &edges, &cells, how);
    assert!(read.fits && !read.unread);
    assert_eq!(
        read.left_out,
        [(strips::ACROSS, vec!["Name".to_string(), "Ort".to_string()])]
    );
    let pairs: Vec<(Vec<String>, Vec<String>)> = read
        .fields
        .iter()
        .map(|f| (texts(&runs, &f.label), texts(&runs, &f.value)))
        .collect();
    assert_eq!(
        pairs,
        [(vec!["Telefon".to_string()], vec!["0123 4567".to_string()])]
    );
}

/// A row of fields between two rules with no dividers: the value line under two labels is cut
/// where the second label starts, and each piece pairs with the label it stands under.
#[test]
fn strip_without_dividers_pairs_by_field_start() {
    let runs = [
        run("Name", 10.0, 120.0, 6.0),
        run("Ort", 120.0, 120.0, 6.0),
        run_on("Erika Mustermann Musterstadt", 10.0, 100.0, 10.0, 1),
    ];
    assert_eq!(cuts_in_open_frame(&runs), [(2, vec![1])]);
    // The value line as the reader cuts it at that gap.
    let runs = [
        run("Name", 10.0, 120.0, 6.0),
        run("Ort", 120.0, 120.0, 6.0),
        run_on("Erika Mustermann", 10.0, 100.0, 10.0, 1),
        run_on(" Musterstadt", 106.0, 100.0, 10.0, 1),
    ];
    let ids: Vec<usize> = (0..runs.len()).collect();
    let edges = [0.0, 700.0];
    let how = Reading {
        one_field: false,
        smaller: true,
        small: small_size(&[10.0]).saturating_sub(1),
        labels: &[],
    };
    let read = strip_cells(&runs, &edges, &[ids], how);
    assert!(read.fits && !read.unread && read.left_out.is_empty());
    let pairs: Vec<(Vec<String>, Vec<String>)> = read
        .fields
        .iter()
        .map(|f| (texts(&runs, &f.label), texts(&runs, &f.value)))
        .collect();
    assert_eq!(
        pairs,
        [
            (
                vec!["Name".to_string()],
                vec!["Erika Mustermann".to_string()]
            ),
            (vec!["Ort".to_string()], vec![" Musterstadt".to_string()]),
        ]
    );
}

/// Labels on a first line over values: each value goes to the one label its ink overlaps; a
/// value under two labels has no certain label, so the cell is not parted.
#[test]
fn labels_over_values_pair_by_overlap() {
    let runs = [
        run("Name", 10.0, 120.0, 6.0),
        run("Ort", 120.0, 120.0, 6.0),
        run_on("Erika Mustermann", 10.0, 100.0, 10.0, 1),
        run_on("Musterstadt", 120.0, 100.0, 10.0, 1),
    ];
    let parts = parts_of(&runs, &[0, 1, 2, 3]).expect("parts");
    assert_eq!(parts, [vec![0, 2], vec![1, 3]]);
    // A value whose ink reaches under both labels.
    let runs = [
        run("Name", 10.0, 120.0, 6.0),
        run("Ort", 120.0, 120.0, 6.0),
        run_on("Erika Mustermann aus Musterstadt", 10.0, 100.0, 10.0, 1),
    ];
    assert_eq!(parts_of(&runs, &[0, 1, 2]), None);
}

/// Blocks of facts side by side (an address beside a bank) are read block by block: the runs
/// of each block top down, one block after the other, never line by line across them.
#[test]
fn side_blocks_read_block_by_block() {
    let rows = [
        ("Muster GmbH", "Musterbank"),
        ("Musterweg 1", "IBAN DE00 0000 0000 0000 0000 00"),
        ("12345 Musterstadt", "BIC MUSTDEXXXXX"),
    ];
    let mut runs = Vec::new();
    for (k, (left, right)) in rows.iter().enumerate() {
        let y = 700.0 - 12.0 * k as f32;
        runs.push(run_on(left, 50.0, y, 10.0, k as u32));
        runs.push(run_on(right, 300.0, y, 10.0, k as u32));
    }
    let lines: Vec<RunRange> = run_ranges(&runs);
    let pieces: Vec<_> = lines
        .iter()
        .map(|l| pieces_of(&runs, l.start, l.end))
        .collect();
    let (columns, order) = blocks_of(&runs, &pieces);
    assert_eq!(order, [0, 2, 4, 1, 3, 5]);
    assert_eq!(address_columns(&columns), Columns::Blocks);
}

/// Three lines of a short label and its value, labels flush left, values flush left: a block
/// of labels and values, each line one row.
#[test]
fn key_values_are_rows_of_a_label_and_its_value() {
    let rows = [
        ("Kundennr.:", "4711"),
        ("Datum:", "01.03.2026"),
        ("Betrag:", "1.234,56"),
    ];
    let mut runs = Vec::new();
    for (k, (label, value)) in rows.iter().enumerate() {
        let y = 700.0 - 12.0 * k as f32;
        runs.push(run_on(label, 50.0, y, 10.0, k as u32));
        runs.push(run_on(value, 200.0, y, 10.0, k as u32));
    }
    let lines = run_ranges(&runs);
    let pieces: Vec<_> = lines
        .iter()
        .map(|l| pieces_of(&runs, l.start, l.end))
        .collect();
    let off = vec![false; lines.len()];
    let KeyValues::Rows(got) = key_values(&runs, &lines, 0, 3, &pieces, &off) else {
        panic!("no rows");
    };
    let lines: Vec<usize> = got.iter().map(|r| r.line).collect();
    let ranges: Vec<_> = got.iter().map(|r| (r.label, r.value)).collect();
    assert_eq!(lines, [0, 1, 2]);
    assert_eq!(
        ranges,
        [
            ((0, 1), Some((1, 2))),
            ((2, 3), Some((3, 4))),
            ((4, 5), Some((5, 6)))
        ]
    );
}

/// A prose line between two rules under a smaller caption line of two parts is no row of
/// fields: it is too long for values, so it is not cut where the caption's second part
/// starts.
#[test]
fn a_prose_line_under_a_caption_is_not_cut() {
    let caption = run("Abb. 1", 10.0, 120.0, 6.0);
    let source = run("Quelle: Muster", 200.0, 120.0, 6.0);
    let prose = run_on(
        "Die Werte steigen im Lauf des Jahres stark an und fallen erst im Herbst wieder auf \
         den Stand des Vorjahres zurueck",
        10.0,
        100.0,
        10.0,
        1,
    );
    assert!(cuts_in_open_frame(&[caption, source, prose]).is_empty());
}

/// Guard: a value line of two fields under their labels is cut where the second label
/// starts.
#[test]
fn a_value_line_under_two_labels_is_cut() {
    let runs = [
        run("Name", 10.0, 120.0, 6.0),
        run("Ort", 100.0, 120.0, 6.0),
        run_on("Erika Mustermann Musterstadt", 10.0, 100.0, 10.0, 1),
    ];
    assert_eq!(cuts_in_open_frame(&runs), [(2, vec![1])]);
}

/// Labels a size step under their values are labels at every scale: printed at 0.4, labels
/// of 4.8 points over values of 5.2 differ by less than half a point; labels and values of
/// one size are no labels over values.
#[test]
fn labels_over_values_by_a_size_step_at_any_scale() {
    let cell = |label: f32, value: f32| {
        vec![
            run("Name", 4.0, 100.0, label),
            run("Ort", 4.0 + 10.0 * label, 100.0, label),
            run_on(
                "Erika Mustermann Musterstadt",
                4.0,
                100.0 - 2.0 * value,
                value,
                1,
            ),
        ]
    };
    for (label, value, labels) in [(12.0, 13.0, true), (4.8, 5.2, true), (5.0, 5.0, false)] {
        let runs = cell(label, value);
        let lines = lines_of(&runs, &[0, 1, 2]);
        assert_eq!(
            first_line_labels(&runs, &lines).map(|l| l.len()),
            labels.then_some(2),
            "{label} {value}"
        );
    }
}

/// At the sizes of a form, labels half a point smaller than their values (8 over 8.5,
/// 9 over 9.5, 10 over 10.5) are labels, as they were when the step was half a point; a
/// first line that mixes 8 and 8.5 points is no line of labels (its 8.5 point run may be
/// a value).
#[test]
fn labels_half_a_point_under_their_values_at_8_to_10_points() {
    let cell = |sizes: [f32; 2], value: f32| {
        vec![
            run("Name", 4.0, 100.0, sizes[0]),
            run("Ort", 120.0, 100.0, sizes[1]),
            run_on("Erika Mustermann Musterstadt", 4.0, 70.0, value, 1),
        ]
    };
    for size in [8.0, 9.0, 10.0] {
        let runs = cell([size, size], size + 0.5);
        let lines = lines_of(&runs, &[0, 1, 2]);
        assert_eq!(
            first_line_labels(&runs, &lines).map(|l| l.len()),
            Some(2),
            "{size}"
        );
    }
    let runs = cell([8.0, 8.5], 10.0);
    let lines = lines_of(&runs, &[0, 1, 2]);
    assert_eq!(first_line_labels(&runs, &lines).map(|l| l.len()), None);
}

/// Labels of one size with a smaller name between them head their cells, and so does a name
/// at the values' size, over lines of its size or over nothing, right of a label with a value
/// under it; right of a label with nothing under it, the name is no head (it may be the value
/// beside the label).
#[test]
fn heads_of_one_size_may_hold_a_smaller_or_a_value_sized_head() {
    let cell = |middle: f32, under: bool, left: bool| {
        let mut runs = vec![
            run("Name", 4.0, 100.0, 8.0),
            run("Muster GmbH", 60.0, 100.0, middle),
            run("Nr", 180.0, 100.0, 8.0),
        ];
        let mut low = |text: &str, x: f32, y: f32| runs.push(run_on(text, x, y, 8.5, 1));
        if left {
            low("Beispiel", 4.0, 80.0);
        }
        if under {
            low("Musterweg 1", 60.0, 90.0);
            low("12345 Musterstadt", 60.0, 80.0);
        }
        low("4711", 180.0, 80.0);
        runs
    };
    for (middle, under, left, heads) in [
        (7.2, true, true, Some(3)),
        (8.5, true, true, Some(3)),
        (8.5, false, true, Some(3)),
        (8.5, true, false, None),
    ] {
        let runs = cell(middle, under, left);
        let ids: Vec<usize> = (0..runs.len()).collect();
        let lines = lines_of(&runs, &ids);
        assert_eq!(
            first_line_labels(&runs, &lines).map(|l| l.len()),
            heads,
            "{middle} {under} {left}"
        );
    }
}

#[test]
fn address_blocks_have_lines_of_more_than_one_shape() {
    assert!(address_block(&[
        "Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt"
    ]));
    assert!(address_block(&[
        "Musterbank",
        "IBAN DE00 0000 0000 0000 0000 00",
        "BIC MUSTDEXXXXX"
    ]));
    // The labels or values of one form in a column of a table.
    assert!(!address_block(&["Label 0", "Label 1", "Label 2"]));
    assert!(!address_block(&[
        "Musterweg 1",
        "Musterweg 2",
        "Musterweg 3"
    ]));
    assert!(!address_block(&[]));
}

/// Columns of `rows`, each row the column texts left to right (`""` for none), plain and
/// of one size; the first row bold when `bold_head`.
fn columns_of(rows: &[&[&str]], bold_head: bool) -> Vec<Vec<Option<ColumnLine>>> {
    let n = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    (0..n)
        .map(|c| {
            rows.iter()
                .enumerate()
                .map(|(k, r)| {
                    r.get(c).filter(|t| !t.is_empty()).map(|t| ColumnLine {
                        text: (*t).to_string(),
                        bold: bold_head && k == 0,
                        size: 8.0,
                    })
                })
                .collect()
        })
        .collect()
}

#[test]
fn address_columns_are_blocks_and_tables_are_rows() {
    let blocks: [&[&str]; 4] = [
        &["Muster GmbH", "Musterbank", "Amtsgericht Musterstadt"],
        &[
            "Musterweg 1",
            "IBAN DE00 0000 0000 0000 0000 00",
            "HRB 00000",
        ],
        &[
            "12345 Musterstadt",
            "BIC MUSTDEXXXXX",
            "USt-IdNr. DE000000000",
        ],
        &["Deutschland", "", "Erika Mustermann"],
    ];
    assert_eq!(
        address_columns(&columns_of(&blocks, false)),
        Columns::Blocks
    );
    let two: Vec<&[&str]> = blocks[..3].iter().map(|r| &r[..2]).collect();
    assert_eq!(address_columns(&columns_of(&two, false)), Columns::Blocks);
    // Under a bold row of their headings: the table they make, with the doubt said.
    let headed: [&[&str]; 4] = [
        &["Anschrift", "Bankverbindung", "Register"],
        blocks[0],
        blocks[1],
        blocks[2],
    ];
    assert_eq!(address_columns(&columns_of(&headed, true)), Columns::Headed);
    // A table with a bold header, or whose columns list records under their titles.
    let table: [&[&str]; 4] = [
        &["Strasse", "Ort"],
        &["Musterweg 1", "12345 Musterstadt"],
        &["Musterweg 2", "23456 Musterdorf"],
        &["Musterweg 3", "34567 Musterhausen"],
    ];
    assert_eq!(address_columns(&columns_of(&table, true)), Columns::Rows);
    assert_eq!(address_columns(&columns_of(&table, false)), Columns::Rows);
    // Labels without a digit beside an address.
    let labelled: [&[&str]; 3] = [
        &["Name", "Erika Mustermann"],
        &["Strasse", "Musterweg 1"],
        &["Ort", "12345 Musterstadt"],
    ];
    assert_eq!(
        address_columns(&columns_of(&labelled, false)),
        Columns::Rows
    );
    // Labels and values with a value left empty between others.
    let notice: [&[&str]; 4] = [
        &["Name", "Erika Mustermann"],
        &["Zimmer 2", "Raum 101"],
        &["Steuerjahr 2025", ""],
        &["Bearbeitung", "Max Mustermann"],
    ];
    assert_eq!(address_columns(&columns_of(&notice, false)), Columns::Rows);
    // Amounts beside an address are rows of a table.
    let amounts: [&[&str]; 3] = [
        &["Muster GmbH", "100,00"],
        &["Musterweg 1", "20,00"],
        &["12345 Musterstadt", "5,00"],
    ];
    assert_eq!(address_columns(&columns_of(&amounts, false)), Columns::Rows);
    // A column of more lines than an address is no block; its address makes the rows
    // doubtful, which is said.
    let long: Vec<&[&str]> = (0..6).map(|k| blocks[k % 3]).collect();
    assert_eq!(address_columns(&columns_of(&long, false)), Columns::Doubt);
    // Small tables whose columns list records of one kind, with a plain header or none, and
    // line items with amounts.
    let items: [&[&str]; 5] = [
        &["Artikel", "Lager", "Menge"],
        &["Schraube M4", "Halle 2", "12 Stueck"],
        &["Mutter M5", "Regal 3B", "8 Stueck"],
        &["Scheibe", "Halle 1", "20 Stueck"],
        &["Duebel 6", "Halle 3", "5 Stueck"],
    ];
    for n in [4, 5] {
        assert_eq!(
            address_columns(&columns_of(&items[..n], false)),
            Columns::Rows
        );
    }
    let shipments: [&[&str]; 3] = [
        &["Muster GmbH", "Lieferung 12", "3 Kartons"],
        &["Beispiel AG", "Lieferung 13", "12 Paletten"],
        &["Muster KG 2", "Ruecksendung", "1 Karton"],
    ];
    assert_eq!(
        address_columns(&columns_of(&shipments, false)),
        Columns::Rows
    );
    let lines: [&[&str]; 3] = [
        &["Beratung 2 Std", "1.000,00 EUR"],
        &["Material", "200,00 EUR"],
        &["Versand 3", "34,56 EUR"],
    ];
    assert_eq!(address_columns(&columns_of(&lines, false)), Columns::Rows);
    // Beside a postal address, a block of lines of one kind (two banks) or under a colon
    // heading is a block; labels beside an address are rows, said.
    let banks: [&[&str]; 4] = [
        &["Muster GmbH", "Musterbank"],
        &["Musterweg 1", "IBAN DE00 0000 0000 0000 0000 00"],
        &["12345 Musterstadt", "Beispielbank"],
        &["Deutschland", "IBAN DE11 1111 1111 1111 1111 11"],
    ];
    assert_eq!(address_columns(&columns_of(&banks, false)), Columns::Blocks);
    let colon: [&[&str]; 4] = [
        &["Muster GmbH", "Bankverbindung:"],
        &["Musterweg 1", "Musterbank"],
        &["12345 Musterstadt", "IBAN DE00 0000 0000 0000 0000 00"],
        &["Deutschland", ""],
    ];
    assert_eq!(address_columns(&columns_of(&colon, false)), Columns::Blocks);
    let labelled: [&[&str]; 3] = [
        &["Kundennr. 1", "Muster GmbH"],
        &["Vertrag 7", "Musterweg 1"],
        &["Tarif A2", "12345 Musterstadt"],
    ];
    // Numbered labels beside an address are rows, with no doubt; phone lines beside it may be
    // a block of their own, which stays a doubt.
    assert_eq!(
        address_columns(&columns_of(&labelled, false)),
        Columns::Rows
    );
    let phones: [&[&str]; 3] = [
        &["Telefon 0123 4567", "Muster GmbH"],
        &["Telefax 0123 4568", "Musterweg 1"],
        &["Mobil 0170 1234", "12345 Musterstadt"],
    ];
    assert_eq!(address_columns(&columns_of(&phones, false)), Columns::Doubt);
}

#[test]
fn two_address_lines_have_numbers_in_another_order() {
    assert!(address_lines(&["Musterweg 1", "12345 Musterstadt"]));
    assert!(address_lines(&["Am Hang 3", "11111 Neudorf"]));
    // Three lines are no two (see `address_shaped`).
    assert!(!address_lines(&[
        "Muster GmbH",
        "Musterweg 1",
        "12345 Musterstadt"
    ]));
    // A label over its value: a number on one line at most, or in the same order.
    assert!(!address_lines(&["Strasse", "Musterweg 1"]));
    assert!(!address_lines(&["Nr", "4711"]));
    assert!(!address_lines(&["Zimmer 2", "Raum 101"]));
    assert!(!address_lines(&["Name", "Erika Mustermann"]));
}
