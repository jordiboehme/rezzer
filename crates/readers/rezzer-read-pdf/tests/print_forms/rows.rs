// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Notes, titles and rows around cut grids, columns of totals and cuts of padded lines.

use super::*;

/// A table of 4 columns, a band of one cell with a line of text, and a table of 3 other
/// columns, one grid. With the line near the band's bottom, right above the cut, it is the
/// caption of the table below; near the band's top it is a note, the last row of the table
/// above.
pub(crate) fn note_or_title(near_bottom: bool) -> ReadOutcome {
    let mut template = form_section(
        &form_columns(4),
        760.0,
        &["Nr.", "Art", "Tage", "Betrag"],
        &[
            &["1001", "Gehalt", "20", "1.234,56"],
            &["1002", "Zulage", "8", "12,34"],
        ],
        BY_CELL,
    );
    template += &grid(&[40.0, 555.0], &[682.0, 712.0]);
    let y = if near_bottom { 685.0 } else { 703.0 };
    template += &bold(44.0, y, 7.0, "Muster GmbH Abzuege");
    template += &form_section(
        &[40.0, 200.0, 420.0, 555.0],
        682.0,
        &["Kasse", "Art", "Betrag"],
        &[&["A", "Steuer", "123,45"], &["B", "Beitrag", "67,89"]],
        BY_CELL,
    );
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    read(&print_form(&[page], "", ""))
}

#[test]
fn a_line_right_above_a_cut_titles_the_table_below() {
    let out = note_or_title(true);
    let found = read_tables(&out);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].len(), 3, "{found:?}");
    assert_eq!(
        read_captions(&out),
        [None, Some("Muster GmbH Abzuege".to_string())]
    );
    assert_no_loss(&out);
}

#[test]
fn a_note_near_the_top_of_its_band_stays_above() {
    let out = note_or_title(false);
    let found = read_tables(&out);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].len(), 4, "{found:?}");
    assert_eq!(found[0][3].join("|"), "Muster GmbH Abzuege", "{found:?}");
    assert_eq!(read_captions(&out), [None, None]);
    assert_no_loss(&out);
}

/// Guard: the last row of a ruled section printed 2 points over its bottom rule, closer to
/// the cut than to the row's top, stays a row: its column rules cross it.
#[test]
fn a_last_row_printed_low_stays_a_row() {
    let low = Print {
        one_line: false,
        rise: 2.0,
    };
    let template = personal_section(low) + &earnings_section(712.0, "1.234,56", BY_CELL);
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].len(), 3, "{found:?}");
    assert_eq!(found[0][2][1], "Beispiel", "{found:?}");
    assert_eq!(read_captions(&out), [None, None]);
    assert_no_loss(&out);
}

/// Guard: a bold total row in ruled cells right over a cut stays the last row of its table.
#[test]
fn a_bold_total_row_stays_a_row() {
    let xs = form_columns(4);
    let mut template = form_section(
        &xs,
        760.0,
        &["Nr.", "Art", "Tage", "Betrag"],
        &[
            &["1001", "Gehalt", "20", "1.234,56"],
            &["1002", "Zulage", "8", "12,34"],
        ],
        BY_CELL,
    );
    template += &grid(&xs, &[696.0, 712.0]);
    template += &bold(xs[0] + 3.0, 698.0, 9.0, "Summe");
    template += &bold(xs[3] + 3.0, 698.0, 9.0, "1.246,90");
    template += &form_section(
        &[40.0, 200.0, 420.0, 555.0],
        696.0,
        &["Kasse", "Art", "Betrag"],
        &[&["A", "Steuer", "123,45"], &["B", "Beitrag", "67,89"]],
        BY_CELL,
    );
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].len(), 4, "{found:?}");
    assert_eq!(found[0][3][0], "Summe", "{found:?}");
    assert_eq!(found[0][3][3], "1.246,90", "{found:?}");
    assert_eq!(read_captions(&out), [None, None]);
    assert_no_loss(&out);
}

/// Guard: a row of drawn cells with bold labels low in their cells, right over a section of
/// other columns, keeps its labels: no table below takes them as its caption.
#[test]
fn header_band_labels_stay_in_their_band() {
    let xs = form_columns(4);
    let mut template = grid(&xs, &[700.0, 716.0]);
    for (k, t) in ["Nr.", "Art", "Tage", "Betrag"].iter().enumerate() {
        template += &bold(xs[k] + 3.0, 702.0, 7.0, t);
    }
    template += &form_section(
        &[40.0, 200.0, 420.0, 555.0],
        700.0,
        &["Kasse", "Art", "Betrag"],
        &[&["A", "Steuer", "123,45"], &["B", "Beitrag", "67,89"]],
        BY_CELL,
    );
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(
        read_captions(&out).iter().all(Option::is_none),
        "{:?}",
        read_captions(&out)
    );
    let text = all_text(&out);
    assert!(text.contains("Tage"), "{text}");
}

/// A title of two lines in a band of one cell right over a cut is one caption of the
/// table below.
#[test]
fn a_two_line_title_is_one_caption() {
    let mut template = form_section(
        &form_columns(4),
        760.0,
        &["Nr.", "Art", "Tage", "Betrag"],
        &[
            &["1001", "Gehalt", "20", "1.234,56"],
            &["1002", "Zulage", "8", "12,34"],
        ],
        BY_CELL,
    );
    template += &grid(&[40.0, 555.0], &[682.0, 712.0]);
    template += &bold(44.0, 696.0, 7.0, "Muster GmbH");
    template += &bold(44.0, 685.0, 7.0, "Abzuege und Beitraege");
    template += &form_section(
        &[40.0, 200.0, 420.0, 555.0],
        682.0,
        &["Kasse", "Art", "Betrag"],
        &[&["A", "Steuer", "123,45"], &["B", "Beitrag", "67,89"]],
        BY_CELL,
    );
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].len(), 3, "{found:?}");
    assert_eq!(
        read_captions(&out),
        [None, Some("Muster GmbH Abzuege und Beitraege".to_string())]
    );
    assert_no_loss(&out);
}

/// Guard: a bold line right over a grid that is not cut stays a paragraph (ruling P6-Q is
/// for cut grids only).
#[test]
fn a_line_over_a_grid_that_is_not_cut_stays_a_paragraph() {
    let mut template = bold(44.0, 763.0, 7.0, "Muster GmbH Bezuege");
    template += &form_section(
        &form_columns(4),
        760.0,
        &["Nr.", "Art", "Tage", "Betrag"],
        &[
            &["1001", "Gehalt", "20", "1.234,56"],
            &["1002", "Zulage", "8", "12,34"],
        ],
        BY_CELL,
    );
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert_eq!(read_captions(&out), [None]);
    assert!(all_text(&out).contains("Muster GmbH Bezuege"));
}

/// A section of one header row of hanging column rules over value lines without rules, as a
/// print-run form draws it: rules across at 700 and 660 and borders at 40 and 400 the full
/// height, column rules hanging 12 points from the top at `dividers`, a label at the left of
/// each column and the value `lines` as data lines (Courier 9.5 from 40).
pub(crate) fn hanging_section(dividers: &[f32], labels: &[&str], lines: Vec<DataLine>) -> FormPage {
    let mut t = String::new();
    t += &bar_h(40.0, 400.0, 700.0, 0.48);
    t += &bar_h(40.0, 400.0, 660.0, 0.48);
    t += &bar_v(40.0, 660.0, 700.0, 0.48);
    t += &bar_v(400.0, 660.0, 700.0, 0.48);
    for &x in dividers {
        t += &bar_v(x, 688.0, 700.0, 0.24);
    }
    let lefts = std::iter::once(40.0).chain(dividers.iter().copied());
    for (x, l) in lefts.zip(labels) {
        t += &label(x + 2.0, 691.0, 6.6, l);
    }
    t += &label(40.0, 760.0, 11.4, "Erika Mustermann");
    FormPage {
        template: t,
        lines,
        ..FormPage::default()
    }
}

/// A section of five fields with a column of totals 6 points to its right at x 406 to 480:
/// a strip at the section's rules titled "Af" on the section's label baseline over the
/// amount of the value line, and a strip below with the total label "Ag" over its amount.
/// `rules` are the heights of the column's rules, `sides` draws vertical rules at its ends.
pub(crate) fn section_with_totals(rules: &[f32], sides: bool) -> FormPage {
    let mut page = hanging_section(
        &[120.0, 200.0, 280.0, 340.0],
        &["Aa", "Ab", "Ac", "Ad", "Ae"],
        vec![
            columns_line(675.0, &[(1, "Muster"), (16, "1234"), (66, "1.234,56")]),
            columns_line(636.0, &[(68, "99,99")]),
        ],
    );
    for &y in rules {
        page.template += &bar_h(406.0, 480.0, y, 0.48);
    }
    if sides {
        for x in [406.0, 480.0] {
            page.template += &bar_v(x, 630.0, 700.0, 0.48);
        }
    }
    page.template += &bold(410.0, 691.0, 6.66, "Af");
    page.template += &bold(410.0, 650.0, 6.66, "Ag");
    page
}

/// A column drawn with horizontal rules only whose rules line up with a section's rules
/// joins the section: its title heads one more column and its amount stands in the row; a
/// total label in it pairs with the amount under it (ruling P6-O). One note says so.
#[test]
fn a_column_of_totals_joins_its_section() {
    let page = section_with_totals(&[700.0, 660.0, 630.0], false);
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 2, "{found:#?} {:?}", out.document.losses);
    assert_eq!(
        found[0][0],
        ["Aa", "Ab", "Ac", "Ad", "Ae", "Af"],
        "{found:#?}"
    );
    assert_eq!(
        found[0][1],
        ["Muster", "1234", "", "", "", "1.234,56"],
        "{found:#?}"
    );
    assert_eq!(found[1], [["Ag", "99,99"]], "{found:#?}");
    assert_eq!(read_header_rows(&out), [1, 0]);
    let notes = found_by(&out);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("column of amounts beside a table"),
        "{notes:?}"
    );
    assert_no_loss(&out);
}

/// Guard: a column with sides of its own is a box, not a column of the section.
#[test]
fn a_boxed_column_beside_a_section_stays_apart() {
    let page = section_with_totals(&[700.0, 660.0, 630.0], true);
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found[0][0], ["Aa", "Ab", "Ac", "Ad", "Ae"], "{found:#?}");
    assert!(!side_noted(&out), "{:?}", found_by(&out));
}

/// Whether the read says a column of totals joined a table.
pub(crate) fn side_noted(out: &ReadOutcome) -> bool {
    found_by(out)
        .iter()
        .any(|n| n.contains("column of amounts"))
}

/// Guard: a column whose rules do not line up with the section's rules stays apart.
#[test]
fn a_column_off_the_rules_of_a_section_stays_apart() {
    let page = section_with_totals(&[695.0, 650.0, 625.0], false);
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found[0][0], ["Aa", "Ab", "Ac", "Ad", "Ae"], "{found:#?}");
    assert!(!side_noted(&out), "{:?}", found_by(&out));
}

/// A rule inside a word cuts nothing: the run stays whole in the column of its middle. A rule
/// in the one space between two values cuts there, and each value goes into its column.
#[test]
fn a_column_rule_cuts_a_run_only_between_words() {
    let page = hanging_section(
        &[120.0, 200.0, 280.0, 340.0],
        &["Aa", "Ab", "Ac", "Ad", "Ae"],
        vec![columns_line(
            675.0,
            &[(1, "Muster"), (12, "ABCD EFGH"), (40, "11 22")],
        )],
    );
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(found[0][0], ["Aa", "Ab", "Ac", "Ad", "Ae"], "{found:#?}");
    assert_eq!(
        found[0][1],
        ["Muster", "ABCD EFGH", "11", "22", ""],
        "{found:#?}"
    );
    assert_eq!(read_header_rows(&out), [1]);
    assert_no_loss(&out);
}

/// A value line that fills two of nine fields is still the section's row: its empty fields
/// are empty cells, not a reason to refuse the table.
#[test]
fn a_sparse_value_line_keeps_its_empty_fields() {
    let page = hanging_section(
        &[80.0, 120.0, 160.0, 200.0, 240.0, 280.0, 320.0, 360.0],
        &["Sa", "Sb", "Sc", "Sd", "Se", "Sf", "Sg", "Sh", "Si"],
        vec![columns_line(675.0, &[(1, "X"), (43, "7")])],
    );
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(found[0][1].join("|"), "X||||||7||", "{found:#?}");
    assert_no_loss(&out);
}

/// Values printed as one data line padded into the cells of a grid ruled all the way down,
/// two spaces apart across a column rule: each value goes into its own cell.
#[test]
fn padded_values_are_cut_at_the_rules_of_their_grid() {
    let xs = form_columns(9);
    let rows: [[&str; 9]; 3] = [
        [
            "1001", "Gehalt", "20", "160", "12,34", "1.974,40", "1.234,56", "123,45", "876,54",
        ],
        [
            "1002", "Zulage", "20", "8", "5,00", "40,00", "40,00", "4,00", "36,00",
        ],
        [
            "1003", "Bonus", "1", "0", "0,00", "100,00", "100,00", "10,00", "90,00",
        ],
    ];
    let empty: [&[&str]; 3] = [&[""; 9], &[""; 9], &[""; 9]];
    let titles = [
        "Nr.", "Art", "Tage", "Std.", "Satz", "Basis", "Brutto", "Steuer", "Netto",
    ];
    let lines = rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let parts: Vec<(usize, &str)> = row
                .iter()
                .enumerate()
                .map(|(k, v)| (pad_to(xs[k] + 3.0), *v))
                .collect();
            columns_line(712.0 - 16.0 * (r + 2) as f32 + 5.0, &parts)
        })
        .collect();
    let page = FormPage {
        template: form_section(&xs, 712.0, &titles, &empty, BY_CELL),
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    for (got, want) in found[0][1..].iter().zip(rows) {
        assert_eq!(got, &want, "{found:#?}");
    }
    assert_no_loss(&out);
}

/// Guard: values that run on into the next field of an open row (no rule is drawn there) go
/// by their middle; two of nine runs doing so do not refuse the section.
#[test]
fn values_running_into_the_next_field_keep_the_section() {
    let page = hanging_section(
        &[120.0, 200.0, 280.0, 340.0],
        &["Aa", "Ab", "Ac", "Ad", "Ae"],
        vec![columns_line(
            675.0,
            &[(1, "Muster"), (12, "ABCDEFGH"), (40, "IJKLMNOP"), (58, "Z")],
        )],
    );
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(
        found[0][1],
        ["Muster", "ABCDEFGH", "", "IJKLMNOP", "Z"],
        "{found:#?}"
    );
    assert_no_loss(&out);
}

/// A note of prose across the columns under a header of hanging rules is not cut at them: it
/// is one cell across its row, and the value line above it keeps its columns.
#[test]
fn a_note_across_an_open_row_stays_whole() {
    let note = "Bitte die Hinweise auf der Rueckseite beachten";
    let page = hanging_section(
        &[120.0, 200.0, 280.0, 340.0],
        &["Aa", "Ab", "Ac", "Ad", "Ae"],
        vec![
            columns_line(675.0, &[(1, "Muster"), (22, "ABCD"), (40, "11")]),
            columns_line(665.0, &[(1, note)]),
        ],
    );
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(found[0].last(), Some(&vec![note.to_string()]), "{found:#?}");
    // The value line of the same row keeps its columns.
    assert_eq!(found[0][1], ["Muster", "ABCD", "11", "", ""], "{found:#?}");
    assert_no_loss(&out);
}

/// A name of four words that runs on into the next field inside a word, on a line with its
/// code and flags, is a value: it is no note, goes by its middle, and the code and flags keep
/// their columns.
#[test]
fn a_long_name_beside_its_code_is_no_note() {
    let page = hanging_section(
        &[66.0, 150.0, 250.0, 300.0, 340.0],
        &["Aa", "Ab", "Ac", "Ad", "Ae", "Af"],
        vec![columns_line(
            675.0,
            &[
                (1, "045 Zulage fuer Muster Bezug"),
                (40, "Z"),
                (46, "X"),
                (56, "Y"),
            ],
        )],
    );
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(
        found[0][1],
        ["045", "Zulage fuer Muster Bezug", "", "Z", "X", "Y"],
        "{found:#?}"
    );
    assert_no_loss(&out);
}

/// A sentence whose every crossed column rule falls in a gap between its words is cut there:
/// only text that runs over a rule inside a word can be a note, as a value line padded into
/// its fields has a space at each rule.
#[test]
fn a_sentence_with_a_gap_at_each_rule_is_cut() {
    let page = hanging_section(
        &[120.0, 200.0, 280.0, 340.0],
        &["Aa", "Ab", "Ac", "Ad", "Ae"],
        vec![columns_line(
            675.0,
            &[(
                1,
                "bitte nur die Hinweise beim Ausfuellen am Ende eine Zeile",
            )],
        )],
    );
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(
        found[0][1],
        [
            "bitte nur die",
            "Hinweise beim",
            "Ausfuellen am",
            "Ende eine",
            "Zeile"
        ],
        "{found:#?}"
    );
}

/// A note of two lines across an open row is one cell: its second line, alone on its line in
/// its style one line below, goes on it.
#[test]
fn a_note_of_two_lines_is_one_cell() {
    let page = hanging_section(
        &[120.0, 200.0, 280.0, 340.0],
        &["Aa", "Ab", "Ac", "Ad", "Ae"],
        vec![
            columns_line(682.0, &[(1, "Muster"), (22, "ABCD"), (40, "11")]),
            columns_line(
                673.0,
                &[(1, "Bitte die Hinweise auf der Rueckseite beachten")],
            ),
            columns_line(664.0, &[(1, "und die Frist einhalten")]),
        ],
    );
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(found[0].len(), 3, "{found:#?}");
    assert_eq!(found[0][1], ["Muster", "ABCD", "11", "", ""], "{found:#?}");
    assert_eq!(
        found[0][2],
        ["Bitte die Hinweise auf der Rueckseite beachten und die Frist einhalten"],
        "{found:#?}"
    );
    assert_no_loss(&out);
}

/// A value alone on the line below a note does not go on the note: it is no sentence, and
/// stays a row of its own in its column.
#[test]
fn a_value_below_a_note_stays_its_own_row() {
    let page = hanging_section(
        &[120.0, 200.0, 280.0, 340.0],
        &["Aa", "Ab", "Ac", "Ad", "Ae"],
        vec![
            columns_line(682.0, &[(1, "Muster"), (22, "ABCD"), (40, "11")]),
            columns_line(
                673.0,
                &[(1, "Bitte die Hinweise auf der Rueckseite beachten")],
            ),
            columns_line(664.0, &[(1, "Muster A")]),
        ],
    );
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?} {:?}", out.document.losses);
    assert_eq!(found[0].len(), 4, "{found:#?}");
    assert_eq!(
        found[0][2],
        ["Bitte die Hinweise auf der Rueckseite beachten"],
        "{found:#?}"
    );
    assert_eq!(found[0][3], ["Muster A", "", "", "", ""], "{found:#?}");
    assert_no_loss(&out);
}
