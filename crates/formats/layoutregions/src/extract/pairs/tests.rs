// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::blocks::{
    ColumnLine, Columns, KeyValues, Under, address_block, address_columns, address_lines,
    blocks_of, key_values, label_row_over_values, labels_with_values,
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

/// A label and its value of one size that differ in slant only are not clear by themselves.
/// Beside a field that is clear by its sizes and has evidence of its own (its label ends with
/// a colon, or its value is shaped as a number, a date or a code), the style of that label
/// marks the label, and the box is read again as a field. A label alone in an empty field
/// seeds nothing (its role is told by its size alone), nor does a clear field without such
/// evidence: only pairs with independent evidence seed a style (the typers'
/// `seeds` hold the same rule).
#[test]
fn box_pairs_weight_only_is_read_again_with_a_known_label_style() {
    let italic = |mut r: TextRun| {
        r.italic = true;
        r
    };
    // Box 0: a lone label. Box 1: label and value of one size, the label slanted.
    // Box 2: a slanted label of that size over a larger value.
    let page = |label: &str, value: &str| {
        [
            italic(run("Sheet", 4.0, 120.0, 8.0)),
            italic(run("Date", 104.0, 120.0, 8.0)),
            run_on("01.03.2026", 104.0, 108.0, 8.0, 1),
            italic(run(label, 204.0, 120.0, 8.0)),
            run_on(value, 204.0, 106.0, 10.0, 1),
        ]
    };
    let runs = page("Depot", "4711");
    // Alone, which line is the label is not clear.
    let (fields, unsure) = box_fields(&runs, &[vec![1, 2]], &[false], &[10.0]);
    assert!(fields[0].is_none());
    assert_eq!(unsure, [(0, WEIGHT_ONLY)]);
    // Beside the empty field alone it stays so: the lone label is a label by its size only.
    let (fields, unsure) = box_fields(&runs, &[vec![0], vec![1, 2]], &[false, false], &[10.0]);
    let empty = fields[0].as_ref().expect("an empty field");
    assert_eq!(texts(&runs, &empty.label), ["Sheet"]);
    assert!(empty.value.is_empty());
    assert!(fields[1].is_none());
    assert_eq!(unsure, [(1, WEIGHT_ONLY)]);
    // Beside a clear field with a value in the shape of a number, or with a colon label, the
    // slant of its label marks the label.
    for (label, value) in [("Depot", "4711"), ("Depot:", "Tall Crate")] {
        let runs = page(label, value);
        let boxes = [vec![0], vec![1, 2], vec![3, 4]];
        let (fields, unsure) = box_fields(&runs, &boxes, &[false; 3], &[10.0]);
        let field = fields[1].as_ref().expect("a field");
        assert_eq!(texts(&runs, &field.label), ["Date"], "{label}");
        assert_eq!(texts(&runs, &field.value), ["01.03.2026"], "{label}");
        assert!(unsure.is_empty(), "{label}");
    }
    // A clear field with neither a colon nor a shaped value seeds nothing.
    let runs = page("Depot", "Tall Crate");
    let boxes = [vec![0], vec![1, 2], vec![3, 4]];
    let (fields, unsure) = box_fields(&runs, &boxes, &[false; 3], &[10.0]);
    assert!(fields[1].is_none() && fields[2].is_some());
    assert_eq!(unsure, [(1, WEIGHT_ONLY)]);
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

/// A strip of four fields between `x0` 40 and `x1` 220 in text of `size`: small labels over
/// one value line whose last value is one character, with a divider `inside` points right of
/// the start of the second value and of that character. The dividers, and the runs.
fn one_character_strip(size: f32, inside: f32) -> ([f32; 3], Vec<TextRun>) {
    let ch = 0.6 * size;
    let x = 42.0;
    let dividers = [x + 6.0 * ch + inside, x + 15.0 * ch + inside, x + 22.0 * ch];
    let label = 0.7 * size;
    let runs = vec![
        run("Number", x + 1.0, 120.0, label),
        run("Born", dividers[0] + 1.0, 120.0, label),
        run("Class", dividers[1] + 1.0, 120.0, label),
        run("Factor", dividers[2] + 1.0, 120.0, label),
        run_on("00123 01.02.70 3", x, 120.0 - 1.4 * size, size, 1),
    ];
    (dividers, runs)
}

/// A drawn divider in a word gap of a run is drawn evidence of a
/// field edge. The run is cut there even when less than a character of it stands past the
/// divider (a last value of one character), with the divider a hair inside the character's
/// box. A divider half a character or more inside a word parts no words, at any text size,
/// and a divider further than the reach of drawn lines from the gap parts none either.
#[test]
fn a_run_is_cut_at_a_divider_in_its_word_gap_whatever_stands_past_it() {
    use super::strips::{DIVIDER_IN_WORD, across, divider_gap};
    use crate::segment::drawn::SNAP;
    let cuts = |size: f32, inside: f32| {
        let (dividers, runs) = one_character_strip(size, inside);
        let frame = FieldFrame {
            x0: 40.0,
            x1: 220.0,
            dividers: &dividers,
        };
        let n = runs.len();
        // The character after a divider at its start or in its box does not stand across it.
        assert!(inside <= 0.0 || !across(&runs[4], dividers[1]));
        field_cuts(&runs, &[frame], &mut |_| (0..n).collect())
    };
    let ch = 0.6 * 9.5;
    for inside in [-0.06, 0.0, 0.08 * ch, 0.3 * ch] {
        assert_eq!(cuts(9.5, inside), [(4, vec![0, 1])], "{inside} inside");
    }
    // Half a character inside the word, or past the reach of drawn lines: in the word.
    assert!(0.45 * ch > SNAP);
    for inside in [0.45 * ch, DIVIDER_IN_WORD * ch + 0.1, 0.9 * ch] {
        assert_eq!(cuts(9.5, inside), [(4, vec![0])], "{inside} inside");
    }
    // In small text half a character is less than the reach of drawn lines: the share of
    // the character bounds it.
    let small = 0.6 * 4.0;
    assert!(DIVIDER_IN_WORD * small < SNAP);
    assert_eq!(cuts(4.0, 0.4 * small), [(4, vec![0, 1])]);
    assert_eq!(cuts(4.0, 0.6 * small), [(4, vec![0])]);
    // The gap a divider stands in, and none for a divider in a word.
    let (dividers, runs) = one_character_strip(9.5, 0.0);
    assert_eq!(divider_gap(&runs[4], dividers[1]), Some(1));
    assert_eq!(divider_gap(&runs[4], dividers[1] - 4.0 * ch), None);
}

/// A gap between the digit groups of a number is no field edge: one
/// to three digits, the gap, exactly three digits, then the end, a decimal mark or another
/// group. Both ways: a group of another length, a word that is no group and a date are none.
#[test]
fn a_gap_between_digit_groups_is_told_by_its_digits() {
    use super::strips::group_gap;
    for (text, words) in [
        ("1 234,56", 1),
        ("12 345", 1),
        ("123 456.70", 1),
        ("1 234 567", 1),
        ("1 234 567", 2),
        ("-7 500", 1),
        ("Sum 1\u{2009}234,56 net", 2),
        // A sign, an opening bracket, a currency sign or a currency code may lead the first
        // group, and a closing bracket may end the last one.
        ("+7 500", 1),
        ("\u{2212}1 234,56", 1),
        ("(1 234,56)", 1),
        ("(1 234)", 1),
        ("[12 345]", 1),
        ("\u{20ac}1 234,56", 1),
        ("$12 345.00", 1),
        ("\u{a3}1 234", 1),
        ("EUR1 234,56", 1),
        ("-\u{20ac}1 234", 1),
        ("(EUR1 234)", 1),
        ("CHF-1 234", 1),
        ("\u{20ac} 1 234,56", 2),
        ("EUR 1 234,56", 2),
        ("AB 1 234", 2),
    ] {
        assert!(group_gap(text, words), "{text} after {words}");
    }
    for (text, words) in [
        ("01.02.70 3", 1),
        ("70 30", 1),
        ("7 5000", 1),
        ("1234 567", 1),
        ("00123 456", 1),
        ("1 234a", 1),
        ("01.02.70 300", 1),
        ("Sum 1 234,56 net", 1),
        ("Sum 1 234,56 net", 3),
        ("1 234", 0),
        ("1 234", 2),
        // Letters that are no currency code, a mark that is no sign, and the gap after a
        // currency sign or code that stands alone.
        ("AB1 234", 1),
        ("ABCD1 234", 1),
        ("eur1 234", 1),
        ("Nr1 234", 1),
        ("#1 234", 1),
        ("/1 234", 1),
        ("EUR 1 234", 1),
        ("\u{20ac} 1 234", 1),
        ("AB 1 234", 1),
        ("- 234", 1),
        ("EUR 234", 1),
    ] {
        assert!(!group_gap(text, words), "{text} after {words}");
    }
}

/// A number led by a sign (`-`, `+`, U+2212), an opening bracket,
/// a currency sign or a currency code is a number in digit groups too: a drawn divider in
/// its group gap leaves the fields not decided and cuts nothing. The other way: letters
/// before the first digit that are no currency code make a code, which is cut at the
/// divider as before.
#[test]
fn a_divider_in_the_group_gap_of_a_number_with_a_sign_decides_no_fields() {
    use super::strips::undecided;
    // The value run starts at 130 with one gap of 1.6 after its first `chars` characters,
    // the divider 0.8 into that gap.
    let read = |text: &str, chars: usize| {
        let from = 130.0 + 5.7 * chars as f32;
        let before: String = text.chars().take(chars).collect();
        let mut value = run_on(text, 130.0, 100.0, 9.5, 1);
        value.w -= 4.1;
        value.gaps = vec![GapIn {
            words: before.split_whitespace().count() as u32,
            from,
            to: from + 1.6,
        }];
        let divider = from + 0.8;
        let dividers = [divider];
        let runs = [
            run("Count", 44.0, 120.0, 6.6),
            run("Amount", divider + 12.0, 120.0, 6.6),
            value,
        ];
        let frame = FieldFrame {
            x0: 40.0,
            x1: 220.0,
            dividers: &dividers,
        };
        (
            field_cuts(&runs, &[frame], &mut |_| vec![0, 1, 2]),
            undecided(&runs, &frame, &[0, 1, 2]),
        )
    };
    for (text, chars) in [
        ("\u{2212}1 234,56", 2),
        ("(1 234,56)", 2),
        ("[1 234]", 2),
        ("\u{20ac}1 234,56", 2),
        ("$12 345.00", 3),
        ("EUR1 234,56", 4),
        ("-\u{20ac}1 234,56", 3),
        ("\u{20ac} 1 234,56", 3),
        ("EUR 1 234,56", 5),
        ("AB 1 234", 4),
    ] {
        assert_eq!(read(text, chars), (vec![], true), "{text}");
    }
    for (text, chars) in [("AB1 234,56", 3), ("Nr1 234", 3), ("#1 234,56", 2)] {
        assert_eq!(read(text, chars), (vec![(2, vec![0])], false), "{text}");
    }
}

/// A field edge from a label's start that falls in the digit group
/// gap of a number or inside the number leaves the fields of the frame not decided, on the
/// one function both paths ask: no run is cut, and the edge is not moved to the number's end.
/// The other way: with no group after the gap the run is cut at the label's start.
#[test]
fn a_label_start_in_a_number_in_digit_groups_decides_no_fields() {
    use super::strips::undecided;
    // Two small labels on one line and no divider; one value run from 44 with one gap after
    // its first word, its second word under the second label.
    let read = |labels: [&str; 2], second: f32, text: &str, gap: f32| {
        let first = text.chars().take_while(|c| *c != ' ').count() as f32;
        let mut value = run_on(text, 44.0, 100.0, 9.5, 1);
        value.w += gap - 5.7;
        value.gaps = vec![GapIn {
            words: 1,
            from: 44.0 + 5.7 * first,
            to: 44.0 + 5.7 * first + gap,
        }];
        let runs = [
            run(labels[0], 44.0, 120.0, 6.6),
            run(labels[1], second, 120.0, 6.6),
            value,
        ];
        let frame = FieldFrame {
            x0: 40.0,
            x1: 220.0,
            dividers: &[],
        };
        (
            field_cuts(&runs, &[frame], &mut |_| vec![0, 1, 2]),
            undecided(&runs, &frame, &[0, 1, 2]),
        )
    };
    for (labels, second, text, gap) in [
        (["A", "B"], 58.0, "12 345", 2.6),
        (["A", "B"], 56.0, "7 500", 6.3),
        (["A", "B"], 56.0, "7 500,00", 6.3),
        (["No", "Qty"], 60.0, "12 345", 4.6),
        (["No", "Qty"], 62.0, "12 345", 6.6),
        // The label start over the middle of "345", away from the group gap.
        (["A", "B"], 68.0, "12 345", 2.6),
        (["No", "Qty"], 68.0, "12 345", 4.6),
    ] {
        assert_eq!(
            read(labels, second, text, gap),
            (vec![], true),
            "{text} under {second}"
        );
    }
    for (labels, second, text, gap) in [
        (["A", "B"], 58.0, "12 3456", 2.6),
        (["No", "Qty"], 60.0, "12 34,5", 4.6),
    ] {
        assert_eq!(
            read(labels, second, text, gap),
            (vec![(2, vec![0])], false),
            "{text} under {second}"
        );
    }
}

/// A run is never cut at a gap between digit
/// groups, by a divider or by a field edge the run stands across. A drawn divider in that
/// gap, within its reach of it (see `divider_gap`) or inside the number with more than a
/// character of it on each side leaves the fields of the frame not decided: the digits say
/// one number and the divider says two fields, so no run of the frame is cut and the number
/// is not read into one field either. A divider less than a character into the number, out
/// of reach of the gap, leaves the amount whole in the field of its middle (it does not
/// stand across the divider), and gaps that are no group gaps are cut as before.
#[test]
fn a_divider_in_a_gap_between_digit_groups_decides_no_fields() {
    use super::strips::{across, divider_gap, edge_in_number, undecided};
    let amount = |text: &str| {
        let mut r = run_on(text, 130.0, 100.0, 9.5, 1);
        r.w = 41.5;
        r.gaps = vec![GapIn {
            words: 1,
            from: 135.7,
            to: 137.3,
        }];
        r
    };
    // The cuts of the frame, and whether its fields are not decided.
    let read = |value: &TextRun, divider: f32| {
        let dividers = [divider];
        let runs = [
            run("Count", 44.0, 120.0, 6.6),
            run("Amount", divider + 12.0, 120.0, 6.6),
            value.clone(),
        ];
        let frame = FieldFrame {
            x0: 40.0,
            x1: 220.0,
            dividers: &dividers,
        };
        (
            field_cuts(&runs, &[frame], &mut |_| vec![0, 1, 2]),
            undecided(&runs, &frame, &[0, 1, 2]),
        )
    };
    let grouped = amount("1\u{2009}234,56");
    // In the digit before the gap, in the gap and a hair past it: not decided, no cut.
    for divider in [135.0, 136.0, 137.5] {
        assert_eq!(divider_gap(&grouped, divider), None, "{divider}");
        assert_eq!(read(&grouped, divider), (vec![], true), "{divider}");
    }
    // Less than a character into the leading digit, out of reach of the gap: the amount
    // does not stand across the divider and is one value on the side of its middle.
    assert!(!edge_in_number(
        std::slice::from_ref(&grouped),
        &[0],
        &[133.0]
    ));
    assert!(!across(&grouped, 133.0));
    assert_eq!(read(&grouped, 133.0), (vec![], false));
    // Inside the second group, more than a character from both ends of the number: not
    // decided, the edge is not moved to the number's end.
    assert!(across(&grouped, 140.0));
    assert_eq!(read(&grouped, 140.0), (vec![], true));
    assert!(across(&grouped, 136.0));
    // Two groups of three, which may be two values of three digits or one number.
    let mut threes = run_on("123\u{2009}456", 130.0, 100.0, 9.5, 1);
    threes.w = 39.9;
    threes.gaps = vec![GapIn {
        words: 1,
        from: 147.1,
        to: 148.7,
    }];
    assert_eq!(read(&threes, 148.0), (vec![], true));
    assert_eq!(read(&threes, 144.0), (vec![], true));
    // No group after the gap: two values, cut as before, and decided.
    let two = amount("1\u{2009}23456,7");
    for divider in [135.0, 136.0] {
        assert_eq!(divider_gap(&two, divider), Some(0), "{divider}");
        assert_eq!(
            read(&two, divider),
            (vec![(2, vec![0])], false),
            "{divider}"
        );
    }
    assert!(across(&two, 136.0));
    // A word before the amount in the field before it: cut at the word gap the divider
    // stands in. With the divider at the group gap nothing is cut, the word neither.
    let mut named = run_on("Net 1\u{2009}234,56", 107.2, 100.0, 9.5, 1);
    named.w = 64.3;
    named.gaps = vec![
        GapIn {
            words: 1,
            from: 124.3,
            to: 130.0,
        },
        GapIn {
            words: 2,
            from: 135.7,
            to: 137.3,
        },
    ];
    assert_eq!(read(&named, 128.0), (vec![(2, vec![0])], false));
    for divider in [135.0, 136.0] {
        assert_eq!(read(&named, divider), (vec![], true), "{divider}");
    }
    // A value line of a code, a date and one character holds no group gap: decided, and cut
    // at its dividers (see `a_run_is_cut_at_a_divider_in_its_word_gap_whatever_stands_past_it`).
    let (dividers, runs) = one_character_strip(9.5, 0.0);
    let frame = FieldFrame {
        x0: 40.0,
        x1: 220.0,
        dividers: &dividers,
    };
    let ids: Vec<usize> = (0..runs.len()).collect();
    assert!(!undecided(&runs, &frame, &ids));
    assert_eq!(
        field_cuts(&runs, &[frame], &mut |_| ids.clone()),
        [(4, vec![0, 1])]
    );
    // Prose with a number in it holds no fields: nothing to decide.
    let prose = run_on(
        "The parcel of 1 234 small crates was sent on to the far depot by rail and came there \
         late in the week after the fair",
        44.0,
        100.0,
        9.5,
        1,
    );
    let runs = [run("Remark", 44.0, 120.0, 6.6), prose];
    let frame = FieldFrame {
        x0: 40.0,
        x1: 720.0,
        dividers: &[130.0],
    };
    assert!(edge_in_number(&runs, &[0, 1], &[130.0]));
    assert!(!undecided(&runs, &frame, &[0, 1]));
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

/// Lines set two text sizes
/// apart are at the bound of the pitch of a block. Where the lines stand on the page, the
/// step between two baselines is a hair over or under two sizes by the rounding of the
/// coordinates alone (448.0 and 435.4 at 6.3 points are 12.600006 apart), and the block must
/// read the same at every such place: the bound has a slack for the rounding
/// (`PITCH_ROUNDING`), moved both ways here.
#[test]
fn key_values_two_sizes_apart_hold_at_every_place_on_the_page() {
    let block = |top: f32, size: f32, pitch: f32| {
        let rows = [
            ("Date:", "12 May"),
            ("Order:", "po_123"),
            ("Total:", "127.50"),
        ];
        let mut runs = Vec::new();
        for (k, (label, value)) in rows.iter().enumerate() {
            let y = top - pitch * k as f32;
            runs.push(run_on(label, 28.0, y, size, k as u32));
            runs.push(run_on(value, 140.0, y, size, k as u32));
        }
        let lines = run_ranges(&runs);
        let pieces: Vec<_> = lines
            .iter()
            .map(|l| pieces_of(&runs, l.start, l.end))
            .collect();
        let off = vec![false; lines.len()];
        matches!(
            key_values(&runs, &lines, 0, 3, &pieces, &off),
            KeyValues::Rows(rows) if rows.len() == 3
        )
    };
    // The traced page, and the same block at other heights and sizes.
    assert!(block(448.0, 6.3, 12.6));
    for (top, size) in [
        (640.0, 10.0),
        (448.0, 7.0),
        (701.3, 6.3),
        (99.7, 4.1),
        (812.9, 11.7),
    ] {
        assert!(block(top, size, 2.0 * size), "{top} {size}");
        // Half the slack over the bound is at it, twice the slack is past it.
        assert!(block(top, size, (2.0 + 1.0 / 512.0) * size), "{top} {size}");
        assert!(
            !block(top, size, (2.0 + 1.0 / 128.0) * size),
            "{top} {size}"
        );
        assert!(!block(top, size, 2.2 * size), "{top} {size}");
    }
}

/// The colon lines after a block follow it at the same
/// bound with the same slack (`PITCH_ROUNDING`). Two lines of a colon label and its value
/// are no block alone; a line in one run with a short colon label two sizes under them makes
/// three, also where the f32 places land a hair over two sizes (448.0, 435.4 and 422.8 at
/// 6.3 points).
#[test]
fn a_colon_line_two_sizes_after_a_block_holds_at_every_place_on_the_page() {
    let block = |top: f32, size: f32, pitch: f32| {
        let mut runs = Vec::new();
        for (k, (label, value)) in [("Date:", "12 May"), ("Order:", "po_123")]
            .iter()
            .enumerate()
        {
            let y = top - 2.0 * size * k as f32;
            runs.push(run_on(label, 28.0, y, size, k as u32));
            runs.push(run_on(value, 140.0, y, size, k as u32));
        }
        runs.push(run_on(
            "Total: 127.50",
            28.0,
            top - 2.0 * size - pitch,
            size,
            2,
        ));
        let lines = run_ranges(&runs);
        let pieces: Vec<_> = lines
            .iter()
            .map(|l| pieces_of(&runs, l.start, l.end))
            .collect();
        let off = vec![false; lines.len()];
        matches!(
            key_values(&runs, &lines, 0, 2, &pieces[..2], &off),
            KeyValues::Rows(rows) if rows.len() == 3
        )
    };
    assert!(block(448.0, 6.3, 12.6));
    for (top, size) in [(640.0, 10.0), (701.3, 6.3), (99.7, 4.1), (812.9, 11.7)] {
        assert!(block(top, size, 2.0 * size), "{top} {size}");
        // Half the slack over the bound is at it, twice the slack is past it.
        assert!(block(top, size, (2.0 + 1.0 / 512.0) * size), "{top} {size}");
        assert!(
            !block(top, size, (2.0 + 1.0 / 128.0) * size),
            "{top} {size}"
        );
    }
}

/// A line whose label ends a word space before the value
/// column is one piece. It is a line of the block when a word of it starts at the left edge
/// the values of the other lines share and reads as a value: the row says where its run is
/// cut. A word that starts beside that edge, or text that is no value, is no line of a block.
#[test]
fn key_values_cut_a_glued_line_at_the_value_column() {
    // Characters are 6 points wide: `Customer reference ` ends 114 points after its start.
    let block = |glued: &str, column: f32| {
        let runs = vec![
            run_on("Order", 50.0, 700.0, 10.0, 0),
            run_on("4711", column, 700.0, 10.0, 0),
            run_on(glued, 50.0, 688.0, 10.0, 1),
            run_on("Depot", 50.0, 676.0, 10.0, 2),
            run_on("D-17", column, 676.0, 10.0, 2),
            run_on("Parcels", 50.0, 664.0, 10.0, 3),
            run_on("12", column, 664.0, 10.0, 3),
        ];
        let lines = run_ranges(&runs);
        let pieces: Vec<_> = lines
            .iter()
            .map(|l| pieces_of(&runs, l.start, l.end))
            .collect();
        assert_eq!(pieces[1].len(), 1, "the glued line is one piece");
        let off = vec![false; lines.len()];
        match key_values(&runs, &lines, 0, lines.len(), &pieces, &off) {
            KeyValues::Rows(rows) => Some(
                rows.iter()
                    .map(|r| (r.label, r.value, r.cut))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        }
    };
    assert_eq!(
        block("Customer reference 20481", 164.0),
        Some(vec![
            ((0, 1), Some((1, 2)), None),
            ((2, 3), None, Some((2, 19))),
            ((3, 4), Some((4, 5)), None),
            ((5, 6), Some((6, 7)), None),
        ])
    );
    // The value column a character further right: no word of the line starts at its edge.
    assert_eq!(block("Customer reference 20481", 170.0), None);
    // A word at the edge that is no value after a label without a colon: a line of text.
    assert_eq!(block("Customer reference desks", 164.0), None);
    // After a colon any short text is the value.
    assert!(block("Customer reference: desks", 170.0).is_some());
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

/// A numbered label is words with letters and no digit, then a short index: one rule for the
/// address columns and for the label columns of a cell.
#[test]
fn numbered_labels_are_words_and_a_short_index() {
    assert!(blocks::indexed("Kundennr. 1"));
    assert!(blocks::indexed("Tarif A2"));
    assert!(!blocks::indexed("Feld"));
    assert!(!blocks::indexed("Konto 12345"));
    // A head word without a letter is no word of a label.
    assert!(!blocks::indexed("Str. / 1"));
}

/// The labels of a row of labels over their values, each with the value
/// its ink stands under. A value padded with blanks as far as the next label is under its
/// own label only; a label with nothing under it has no value; a value under two labels,
/// or a run on a third baseline, pairs nothing.
#[test]
fn labels_with_values_pair_by_the_ink_under_each_label() {
    let pair = |label: &[usize], value: &[usize]| Under {
        label: label.to_vec(),
        value: value.to_vec(),
    };
    // Labels at 8 points, values at 10 (6 points a character), in the order of their cells.
    let runs = vec![
        run("Member code", 40.0, 700.0, 8.0),
        // `20481` and blanks up to 202, past the start of the second label at 200.
        run("20481                      ", 40.0, 689.0, 10.0),
        run("Issued on", 200.0, 700.0, 8.0),
        run("03.04.2026", 203.0, 689.0, 10.0),
        run("Earlier name", 360.0, 700.0, 8.0),
    ];
    assert_eq!(
        labels_with_values(&runs),
        Some(vec![pair(&[0], &[1]), pair(&[2], &[3]), pair(&[4], &[])])
    );
    // Labels alone: each with an empty value.
    let alone = vec![runs[0].clone(), runs[2].clone()];
    assert_eq!(
        labels_with_values(&alone),
        Some(vec![pair(&[0], &[]), pair(&[1], &[])])
    );
    // A value whose ink stands under two labels is no one's.
    let mut wide = runs.clone();
    wide[1] = run("20481 and more text under both", 40.0, 689.0, 10.0);
    assert_eq!(labels_with_values(&wide), None);
    // A run on a third baseline: no row of labels over their values.
    let mut third = runs.clone();
    third.push(run("more", 40.0, 670.0, 10.0));
    assert_eq!(labels_with_values(&third), None);
}

/// What the reader of tables without rules leaves to the cut. A row of two small
/// labels over two larger values is one; rows in one size, a label row that holds an amount,
/// a value row that ends with a colon and a value under no label are none.
#[test]
fn a_row_of_small_labels_over_larger_values_is_told_from_table_rows() {
    let line = |texts: [(&str, f32); 2], y: f32, size: f32| -> Vec<TextRun> {
        texts.iter().map(|&(t, x)| run(t, x, y, size)).collect()
    };
    let check = |upper: Vec<TextRun>, lower: Vec<TextRun>| {
        let mut runs = upper;
        let n = runs.len();
        runs.extend(lower);
        let (a, b) = (pieces_of(&runs, 0, n), pieces_of(&runs, n, runs.len()));
        label_row_over_values(&runs, &a, &b)
    };
    let labels = || line([("Member code", 40.0), ("Issued on", 200.0)], 700.0, 8.0);
    let values = |size: f32| line([("20481", 43.0), ("03.04.2026", 203.0)], 689.0, size);
    assert!(check(labels(), values(10.0)));
    // No size step between the rows (1.08 times): 8 against 8.5.
    assert!(!check(labels(), values(8.5)));
    assert!(check(labels(), values(8.7)));
    // An amount among the labels, a colon at the end of a value.
    assert!(!check(
        line([("Member code", 40.0), ("120,00", 200.0)], 700.0, 8.0),
        values(10.0)
    ));
    assert!(!check(
        labels(),
        line([("20481", 43.0), ("Total:", 203.0)], 689.0, 10.0)
    ));
    // A value under no label.
    assert!(!check(
        labels(),
        line([("20481", 43.0), ("03.04.2026", 420.0)], 689.0, 10.0)
    ));
}

/// The runs of a footer of three columns at 10 points for [`paragraph_columns`]: the starts
/// 50, 250 and 450; a title line, then in the middle column a paragraph of names over three
/// lines whose first line ends 8 points before the name of the third column and whose second
/// reaches half a point past the start of that column, where it is empty. Each run has the
/// width given.
fn footer_columns() -> Vec<TextRun> {
    let cell = |text: &str, x: f32, w: f32, line: u32| {
        let mut r = run_on(text, x, 700.0 - 12.0 * line as f32, 10.0, line);
        r.w = w;
        r.gaps.clear();
        r
    };
    vec![
        cell("Sample Works", 50.0, 72.0, 0),
        cell("Managers", 250.0, 48.0, 0),
        cell("Board", 450.0, 30.0, 0),
        cell("Mill Lane 4", 50.0, 66.0, 1),
        cell("Dr. Robin Sample (Chair), Ann Specimen", 250.0, 192.0, 1),
        cell("Morgan Sample", 450.0, 78.0, 1),
        cell("20481 Oldtown", 50.0, 78.0, 2),
        cell(
            "(Sales), Alex Other (Staff), Max Sampleton",
            250.0,
            200.5,
            2,
        ),
        cell("Northland", 50.0, 54.0, 3),
        cell("(Works)", 250.0, 42.0, 3),
    ]
}

/// The columns [`paragraph_columns`] reads `runs` as: the texts of each column, top down.
fn read_columns(runs: &[TextRun]) -> Option<Vec<Vec<String>>> {
    let lines: Vec<RunRange> = run_ranges(runs);
    let pieces: Vec<_> = lines
        .iter()
        .map(|l| pieces_of(runs, l.start, l.end))
        .collect();
    let cells = blocks::paragraph_columns(runs, &pieces)?;
    Some(
        cells
            .iter()
            .map(|column| {
                column
                    .iter()
                    .filter(|c| !c.is_empty())
                    .map(|c| texts(runs, c).join(" "))
                    .collect()
            })
            .collect(),
    )
}

/// Three columns at one pitch under a title line, the middle one
/// a paragraph wrapped over every line under its title, are blocks side by side: read column
/// by column, the name of the third column with its title and not after the names of the
/// second. Each piece of the evidence is asked both ways.
#[test]
fn columns_with_a_wrapped_paragraph_read_column_by_column() {
    let base = footer_columns();
    // Read by the gaps alone, the name of the third column is a word of the second.
    let lines: Vec<RunRange> = run_ranges(&base);
    assert_eq!(pieces_of(&base, lines[1].start, lines[1].end).len(), 2);
    let columns = read_columns(&base).expect("columns");
    assert_eq!(
        columns,
        [
            vec!["Sample Works", "Mill Lane 4", "20481 Oldtown", "Northland"],
            vec![
                "Managers",
                "Dr. Robin Sample (Chair), Ann Specimen",
                "(Sales), Alex Other (Staff), Max Sampleton",
                "(Works)"
            ],
            vec!["Board", "Morgan Sample"],
        ]
    );
    // The same at half and at twice the size.
    for k in [0.5f32, 2.0] {
        let mut runs = base.clone();
        for r in &mut runs {
            (r.x, r.y, r.w, r.h, r.size) = (r.x * k, r.y * k, r.w * k, r.h * k, r.size * k);
        }
        assert_eq!(read_columns(&runs).as_ref(), Some(&columns), "{k}");
    }
    let with = |change: &dyn Fn(&mut Vec<TextRun>)| {
        let mut runs = base.clone();
        change(&mut runs);
        read_columns(&runs).is_some()
    };
    // Two columns have a distance and no pitch (`PITCH_COLUMNS`, three): nothing is shown.
    // The paragraph of names beside the board alone has all the other evidence (it is the
    // first of the two columns, wrapped to the last row, the board ends after two lines)
    // and would read as columns with two starts. The company beside the names is none
    // either way: its paragraph would be the last column.
    assert!(!with(&|r| r.retain(|r| r.x > 240.0)));
    assert!(!with(&|r| r.retain(|r| r.x < 440.0)));
    // The pitch is even within an edge (0.1 sizes, a point): the third column a point
    // farther holds, a point and a half does not.
    let third = |dx: f32| {
        with(&|r| {
            for r in r.iter_mut().filter(|r| r.x >= 440.0) {
                r.x += dx;
            }
        })
    };
    assert!(third(1.0) && !third(1.5));
    // A line reaches an edge past the start of the next column at most.
    assert!(with(&|r| r[7].w = 201.0) && !with(&|r| r[7].w = 201.5));
    // The name of the third column opens its cell more than the white space of a joint
    // (half a size, 5 points) after the line of the second: nearer, it is a word of that
    // line, which then stands across the column start.
    assert!(with(&|r| r[4].w = 194.5) && !with(&|r| r[4].w = 195.5));
    // A later line shares the start of the third column: with the title alone there, the
    // start is one line's.
    assert!(!with(&|r| {
        r.remove(5);
    }));
    // The lines of the paragraph are full: the first word of the next line does not fit
    // before the next column. `(Sales),` is 8 of 42 characters of 200.5 points, 38.19 wide.
    // By a ramp over the white space of a joint (5 points) and a count over the lines
    // (`FULL_LACK`, half a line): with 2.3 points of room after that word the first line
    // lacks 0.46 of its proof and the read holds, with 2.6 points it lacks 0.52.
    let room = |room: f32| with(&|r| r[4].w = 450.0 - 38.19 - room - 250.0);
    assert!(room(-0.3) && room(1.0) && room(2.3) && !room(2.6));
    // The lines count together: two lines that lack 0.2 each hold, two that lack 0.3 each
    // do not, though each of them alone would. (The second line of names 157 points wide:
    // `(Sales),` is then 29.9 wide, and `(Works)`, 42 wide, has `second` points of room.)
    let rooms = |first: f32, second: f32| {
        with(&|r| {
            r[7].w = 450.0 - 42.0 - second - 250.0;
            r[4].w = 450.0 - r[7].w * 8.0 / 42.0 - first - 250.0;
        })
    };
    assert!(rooms(1.0, 1.0) && !rooms(1.5, 1.5) && rooms(1.5, 0.0) && rooms(0.0, 1.5));
    // And the next line goes on: it opens with a bracket before a letter or a small letter,
    // or the line over it ends in a hyphen after a letter. A line that opens with a capital
    // or with an amount in brackets is an item of its own.
    let opens = |text: &'static str| with(&|r| r[7].text = text.into());
    assert!(opens("and Alex Other (Staff), Max Sampleton too"));
    assert!(!opens("Sales: Alex Other (Staff), Max Sampleton"));
    assert!(!opens("(12,50) Alex Other (Staff), Max Sampleton"));
    assert!(with(&|r| {
        r[4].text = "Dr. Robin Sample (Chair), Ann Speci-".into();
        r[7].text = "Men: Alex Other (Staff), Max Sampleton too".into();
    }));
    // Each column is one block from the first row: a row left empty in a column is none.
    assert!(!with(&|r| {
        r.remove(6);
    }));
    // The paragraph holds every row under the first: a column that goes on under it leaves
    // rows the paragraph does not show to be one text.
    assert!(!with(&|r| {
        let mut more = r[8].clone();
        (more.y, more.line) = (more.y - 12.0, 4);
        r.push(more);
    }));
    // A line that starts at no column start.
    assert!(!with(&|r| r[3].x = 60.0));
}

/// White space before the first ink of a line stands with
/// the cell that ink opens, not with the first column: a line that starts in the second
/// column, after a run of blanks, gives the first column no line on its row.
#[test]
fn white_space_before_a_line_goes_to_the_column_of_its_ink() {
    let mut runs = footer_columns();
    // The first column ends after its second line; the third and fourth lines start in
    // the second column, each after a run of blanks at the left edge. The third column
    // goes on to the last row (one other column must reach it), beside a second line of
    // names that ends before it.
    runs.retain(|r| r.x > 240.0 || r.line < 2);
    runs[6].w = 192.0;
    for (text, line) in [("Robin Other", 2u32), ("Kim Example", 3)] {
        let mut more = runs[5].clone();
        (more.text, more.y, more.line) = (text.into(), 700.0 - 12.0 * line as f32, line);
        let at = runs.iter().rposition(|r| r.line == line).unwrap();
        runs.insert(at + 1, more);
    }
    for line in [2u32, 3] {
        let at = runs.iter().position(|r| r.line == line).unwrap();
        let mut blank = runs[at].clone();
        (blank.text, blank.x, blank.w) = ("   ".into(), 50.0, 9.0);
        runs.insert(at, blank);
    }
    let lines: Vec<RunRange> = run_ranges(&runs);
    let pieces: Vec<_> = lines
        .iter()
        .map(|l| pieces_of(&runs, l.start, l.end))
        .collect();
    let cells = blocks::paragraph_columns(&runs, &pieces).expect("columns");
    let cell = |c: usize, row: usize| texts(&runs, &cells[c][row]);
    assert!(
        cells[0][2].is_empty() && cells[0][3].is_empty(),
        "{cells:?}"
    );
    assert_eq!(
        cell(1, 2),
        ["   ", "(Sales), Alex Other (Staff), Max Sampleton"]
    );
    assert_eq!(cell(1, 3), ["   ", "(Works)"]);
    // Every run stands in one cell.
    let mut all: Vec<usize> = cells.iter().flatten().flatten().copied().collect();
    all.sort_unstable();
    assert!(all.iter().copied().eq(0..runs.len()));
}

/// The runs of a table without rules at 10 points for [`paragraph_columns`]: three columns
/// that start at 50, 200 and 350, a header over rows of an item, its description (as wide
/// as given) and a third cell. A row with an empty first and third cell is the wrapped end
/// of the description over it.
fn item_table(rows: &[(&str, &str, f32, &str)]) -> Vec<TextRun> {
    let cell = |text: &str, x: f32, w: f32, line: u32| {
        let mut r = run_on(text, x, 700.0 - 12.0 * line as f32, 10.0, line);
        r.w = w;
        r.gaps.clear();
        r
    };
    let mut out = Vec::new();
    for (k, (a, b, wb, c)) in rows.iter().enumerate() {
        let line = k as u32;
        for (text, x, w) in [
            (a, 50.0, 6.0 * a.len() as f32),
            (b, 200.0, *wb),
            (c, 350.0, 6.0 * c.len() as f32),
        ] {
            if !text.is_empty() {
                out.push(cell(text, x, w, line));
            }
        }
    }
    out
}

/// The rows of a table without rules are no blocks side by
/// side, though its columns start at one pitch, its descriptions are full lines that open
/// with a small letter and the last of them wraps. Each row is an item of its own: read
/// column by column, the descriptions of all items would be one text and no price would
/// stand with its item. The counter-evidence is the rows: no other column reaches the last
/// row of the "paragraph" (the cells of the other columns end with
/// the last item, whatever cell is left empty), or every other column has a line on each of
/// its rows but the last, or a column holds nothing but numbers under its title.
#[test]
fn rows_of_a_table_with_a_wrapped_cell_are_no_columns() {
    let rows = |third: [&'static str; 4]| {
        item_table(&[
            ("Item", "Description", 66.0, third[0]),
            (
                "Bolt",
                "steel bolt for the frame, zinc plated",
                142.0,
                third[1],
            ),
            (
                "Nut",
                "nut for the bolt, zinc plated, size M8",
                141.0,
                third[2],
            ),
            (
                "Washer",
                "washer for the nut and the bolt, set",
                140.0,
                third[3],
            ),
            ("", "twelve pieces", 60.0, ""),
        ])
    };
    // A column of prices and a column of words.
    assert_eq!(read_columns(&rows(["Price", "1.20", "0.40", "0.10"])), None);
    assert_eq!(
        read_columns(&rows(["Source", "Mill", "Forge", "Yard"])),
        None
    );
    // One row whose description wraps once, in English and in German.
    assert_eq!(
        read_columns(&item_table(&[
            ("Item", "Description", 66.0, "Source"),
            (
                "Bolt",
                "steel bolt for the frame, zinc plated",
                142.0,
                "Mill"
            ),
            ("", "and painted grey", 70.0, ""),
        ])),
        None
    );
    assert_eq!(
        read_columns(&item_table(&[
            ("Artikel", "Beschreibung", 66.0, "Preis"),
            (
                "Bolzen",
                "Stahlbolzen fuer den Rahmen, verzinkt",
                142.0,
                "1,20"
            ),
            ("", "und grau lackiert", 70.0, ""),
        ])),
        None
    );
    // The description of the last item wrapped over two lines: no other column reaches the
    // last row.
    assert_eq!(
        read_columns(&item_table(&[
            ("Item", "Description", 66.0, "Source"),
            (
                "Bolt",
                "steel bolt for the frame, zinc plated",
                142.0,
                "Mill"
            ),
            (
                "Nut",
                "nut for the bolt, zinc plated, size M8",
                141.0,
                "Forge"
            ),
            ("", "and a washer for the nut and the bolt,", 140.0, ""),
            ("", "twelve pieces", 60.0, ""),
        ])),
        None
    );
    // A cell left empty at the end of a column of words. The
    // other columns then end on different rows, as blocks of their own lengths do, but
    // none of them reaches the last row of the description: rows: the last item
    // has no source, two items, a note on the first item only and the
    // last description wrapped over two lines, each line under a full one).
    let items = |sources: [&'static str; 3]| {
        item_table(&[
            ("Item", "Description", 66.0, "Source"),
            (
                "Bolt",
                "steel bolt for the frame, zinc plated",
                142.0,
                sources[0],
            ),
            (
                "Nut",
                "nut for the bolt, zinc plated, size M8",
                141.0,
                sources[1],
            ),
            (
                "Washer",
                "washer for the nut and the bolt, set",
                140.0,
                sources[2],
            ),
            ("", "twelve pieces", 60.0, ""),
        ])
    };
    assert_eq!(read_columns(&items(["Mill", "Forge", ""])), None);
    assert_eq!(read_columns(&items(["Mill", "", ""])), None);
    assert_eq!(
        read_columns(&item_table(&[
            ("Item", "Description", 66.0, "Source"),
            (
                "Bolt",
                "steel bolt for the frame, zinc plated",
                142.0,
                "Mill"
            ),
            ("Nut", "nut for the bolt, zinc plated, size M8", 141.0, ""),
            ("", "twelve pieces", 60.0, ""),
        ])),
        None
    );
    assert_eq!(
        read_columns(&item_table(&[
            ("Item", "Description", 66.0, "Source"),
            (
                "Bolt",
                "steel bolt for the frame, zinc plated",
                142.0,
                "Mill"
            ),
            (
                "Nut",
                "nut for the bolt, zinc plated, size M8",
                141.0,
                "Forge"
            ),
            ("Washer", "washer for the nut and the bolt, set", 142.0, ""),
            ("", "twelve pieces, zinc plated, in a paper", 142.0, ""),
            ("", "bag", 18.0, ""),
        ])),
        None
    );

    // The same counter-evidence on the footer, each piece both ways.
    let base = footer_columns();
    let with = |change: &dyn Fn(&mut Vec<TextRun>)| {
        let mut runs = base.clone();
        change(&mut runs);
        read_columns(&runs).is_some()
    };
    // The third column ends after its second line, where the paragraph goes on for two
    // more: columns. With a third line it has a line on every row but the paragraph's
    // last, as the first column has: rows. (The second line of names ends before the
    // third column here, so that a line of that column can stand beside it.)
    assert!(with(&|r| r[7].w = 192.0));
    assert!(!with(&|r| {
        r[7].w = 192.0;
        let mut more = r[5].clone();
        (more.text, more.y, more.line) = ("Robin Other".into(), r[7].y, 2);
        r.insert(8, more);
    }));
    // A footer of five rows: the paragraph of the middle column over four lines, the first
    // and the third column with the lines given under their titles.
    let five = |first: &[&str], third: &[&str]| {
        let cell = |text: &str, x: f32, w: f32, line: usize| {
            let mut r = run_on(text, x, 700.0 - 12.0 * line as f32, 10.0, line as u32);
            r.w = w;
            r.gaps.clear();
            r
        };
        let names = [
            "Managers",
            "Dr. Robin Sample (Chair), Ann Specimen",
            "(Sales), Alex Other (Staff), Max Sampleton",
            "(Plant), Kim Sample (Stock), Robin Otherton",
            "(Works)",
        ];
        let mut runs = Vec::new();
        for (line, name) in names.iter().enumerate() {
            let side = |lines: &[&str], title: &'static str| match line {
                0 => Some(title.to_string()),
                n => lines.get(n - 1).map(|t| t.to_string()),
            };
            if let Some(t) = side(first, "Sample Works") {
                runs.push(cell(&t, 50.0, 6.0 * t.len() as f32, line));
            }
            let w = if (1..4).contains(&line) { 192.0 } else { 48.0 };
            runs.push(cell(name, 250.0, w, line));
            if let Some(t) = side(third, "Board") {
                runs.push(cell(&t, 450.0, 6.0 * t.len() as f32, line));
            }
        }
        read_columns(&runs).is_some()
    };
    // Blocks side by side read as columns
    // only when one other column reaches the last row of the paragraph. A column that ends
    // over it looks as the item column of a table does over the wrapped end of its last
    // description, so a footer whose blocks are all shorter than its paragraph reads row
    // by row, as it did before the columns were read. The first three cases were columns
    // before the columns were read as one.
    let (company, board) = (
        ["Mill Lane 4", "20481 Oldtown", "Northland", "Open daily"],
        ["Morgan Sample", "Robin Other", "Kim Example", "Ann Other"],
    );
    assert!(!five(&company[..1], &board[..1]));
    assert!(!five(&company[..2], &board[..1]));
    assert!(!five(&company[..3], &board[..2]));
    assert!(!five(&company[..2], &board[..2]));
    assert!(!five(&company[..3], &board[..3]));
    // One block beside the paragraph down to its last row, the other shorter: columns,
    // whichever side the long block stands on.
    assert!(five(&company, &board[..1]));
    assert!(five(&company, &board[..2]));
    assert!(five(&company[..1], &board));
    assert!(five(&company[..2], &board));
    // And with a line on each row of the paragraph but its last in every other column:
    // rows that open a line in every column are rows.
    assert!(!five(&company, &board[..3]));
    assert!(!five(&company, &board));
    // A column of nothing but numbers under its title is a column of a table.
    assert!(with(&|r| r[5].text = "Morgan 12".into()));
    assert!(!with(&|r| r[5].text = "1.250,00".into()));
}
