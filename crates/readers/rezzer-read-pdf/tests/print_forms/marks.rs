// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Separator marks drawn between the digits of boxed amounts and dates.

use super::*;

/// The pitch of the data font: Courier at 9.5 points is 5.7 points a character.
pub(crate) const PITCH: f32 = 5.7;

/// A boxed number field of `digits` cells from `x` with the data baseline `y`: one stroked
/// box a digit, and a drawn mark before each cell of `marks_before` (0-based), on the
/// baseline in the gap between two cells, as a form template prints its separators.
pub(crate) fn boxed_field(x: f32, y: f32, digits: usize, marks_before: &[usize]) -> String {
    let mut out = String::new();
    for k in 0..digits {
        out += &boxed(x + k as f32 * PITCH, y - 2.0, PITCH, 11.0);
    }
    for &k in marks_before {
        out += &mark(x + k as f32 * PITCH - 0.12, y - 0.5);
    }
    out
}

/// A page with the boxed field at x 154 (20 characters from the data margin) on the
/// baseline 700, its data line, and `lines` of text elsewhere on the page.
pub(crate) fn boxed_page(digits: &str, marks_before: &[usize], lines: &[&str]) -> FormPage {
    let x = 40.0 + 20.0 * PITCH;
    let mut template = label(40.0, 712.0, 6.0, "Betrag");
    template += &boxed_field(x, 700.0, digits.len(), marks_before);
    let mut data = vec![DataLine {
        y: 700.0,
        text: padded(20, digits),
    }];
    for (k, l) in lines.iter().enumerate() {
        data.push(DataLine {
            y: 600.0 - 14.0 * k as f32,
            text: (*l).into(),
        });
    }
    FormPage {
        template,
        lines: data,
        ..FormPage::default()
    }
}

/// The text of the `found-by` paragraphs of the read.
pub(crate) fn found_by(out: &ReadOutcome) -> Vec<String> {
    let mut found = Vec::new();
    for b in &out.document.body {
        if let Block::Section { blocks, .. } = b {
            for inner in blocks {
                if let Block::Paragraph {
                    inlines,
                    role: Some(r),
                    ..
                } = inner
                    && r == "found-by"
                {
                    found.push(plain_text(inlines));
                }
            }
        }
    }
    found
}

/// The losses that say a drawn separator is not read.
pub(crate) fn separator_losses(out: &ReadOutcome) -> Vec<&rezzer_model::Loss> {
    out.document
        .losses
        .iter()
        .filter(|l| l.what.contains("separator drawn as a mark"))
        .collect()
}

/// The found-by note stands right after the page marker of its page.
pub(crate) fn note_after_marker(out: &ReadOutcome) -> bool {
    out.document.body.iter().any(|b| match b {
        Block::Section { blocks, .. } => matches!(
            blocks.get(1),
            Some(Block::Paragraph { role: Some(r), .. }) if r == "found-by"
        ),
        _ => false,
    })
}

#[test]
fn boxed_amount_gets_its_decimal_comma() {
    let page = boxed_page(
        "123456",
        &[4],
        &["Summe 9.876,54", "Netto 1.234,56", "Datum 01.02.2026"],
    );
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(text.contains("1234,56"), "{text}");
    assert!(!text.contains("123456"), "{text}");
    assert_eq!(
        found_by(&out),
        [
            "Found by: 1 decimal separator on this page is a drawn mark, not text; it is \
          written as \",\" as in 2 amounts written as text in this file."
        ]
    );
    assert!(note_after_marker(&out), "{:#?}", out.document.body);
    assert!(
        separator_losses(&out).is_empty(),
        "{:?}",
        out.document.losses
    );
    assert_eq!(out.completeness, Completeness::Complete);
}

#[test]
fn boxed_date_gets_its_points() {
    let page = boxed_page(
        "01022026",
        &[2, 4],
        &["Eintritt 01.01.2026", "Austritt 15.03.2026"],
    );
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(text.contains("01.02.2026"), "{text}");
    assert_eq!(
        found_by(&out),
        [
            "Found by: 2 date separators on this page are drawn marks, not text; they are \
          written as \".\" as in 2 dates written as text in this file."
        ]
    );
    assert!(
        separator_losses(&out).is_empty(),
        "{:?}",
        out.document.losses
    );
}

/// Group, decimal and date marks on one page, with amounts in groups and dates as text.
#[test]
fn group_decimal_and_date_marks_on_one_page() {
    let mut page = boxed_page(
        "123456789",
        &[1, 4, 7],
        &[
            "Summe 9.876,54",
            "Netto 1.234,56",
            "Am 01.01.2026",
            "Bis 15.03.2026",
        ],
    );
    let x = 40.0 + 20.0 * PITCH;
    page.template += &boxed_field(x, 650.0, 8, &[2, 4]);
    page.lines.push(DataLine {
        y: 650.0,
        text: padded(20, "28022026"),
    });
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(text.contains("1.234.567,89"), "{text}");
    assert!(text.contains("28.02.2026"), "{text}");
    assert_eq!(
        found_by(&out),
        [
            "Found by: 1 decimal separator, 2 group separators and 2 date separators on this \
          page are drawn marks, not text; they are written as \",\", \".\" and \".\" as in 2 \
          amounts and 2 dates written as text in this file."
        ]
    );
}

#[test]
fn no_style_in_text_means_not_read() {
    let page = boxed_page("123456", &[4], &["Muster GmbH", "Erika Mustermann"]);
    let out = read(&print_form(&[page], "", "/Lang (de-DE)"));
    let text = all_text(&out);
    assert!(text.contains("123456"), "{text}");
    let losses = separator_losses(&out);
    assert_eq!(losses.len(), 1, "{:?}", out.document.losses);
    assert_eq!(losses[0].why, LossReason::Unsupported);
    assert_eq!(losses[0].severity, Severity::Warning);
    assert_eq!(losses[0].location, "p1");
    // The number by its printed digits.
    assert_eq!(
        losses[0].what,
        "a separator drawn as a mark on page 1 is not read: the number 123456 is kept without \
         it, as the amounts written as text in this file do not show one decimal separator"
    );
    assert_eq!(out.completeness, Completeness::Partial);
    assert!(found_by(&out).is_empty());
}

/// Amounts as text in both styles: no decimal separator is written.
#[test]
fn mixed_styles_in_text_mean_not_read() {
    let page = boxed_page(
        "123456",
        &[4],
        &["Summe 9.876,54", "Netto 1.234,56", "Total 1,234.56"],
    );
    let out = read(&print_form(&[page], "", ""));
    assert!(all_text(&out).contains("123456"));
    assert_eq!(separator_losses(&out).len(), 1, "{:?}", out.document.losses);
    assert_eq!(out.completeness, Completeness::Partial);
}

/// A mark that cuts the digits in no known pattern is not read.
#[test]
fn unknown_digit_groups_are_not_read() {
    let page = boxed_page("123456", &[3], &["Summe 9.876,54", "Netto 1.234,56"]);
    let out = read(&print_form(&[page], "", ""));
    assert!(all_text(&out).contains("123456"));
    let losses = separator_losses(&out);
    assert_eq!(losses.len(), 1, "{:?}", out.document.losses);
    assert!(
        losses[0]
            .what
            .contains("the number 123456 is kept without it"),
        "{}",
        losses[0].what
    );
}

/// Guard: a dotted leader of tiny squares between a label and an amount written with its
/// comma is no separator and gives no loss.
#[test]
fn leader_dots_are_not_separators() {
    let mut template = String::new();
    for k in 0..30 {
        template += &format!("{} 700 0.4 0.4 re f\n", 80.0 + k as f32 * 2.0);
        template += &format!("{} 690 0.3 1.2 re f\n", 80.0 + k as f32 * 2.0);
    }
    let page = FormPage {
        template,
        lines: vec![
            DataLine {
                y: 700.0,
                text: format!("Summe{}1.234,56", " ".repeat(20)),
            },
            DataLine {
                y: 690.0,
                text: format!("Netto{}9.876,54", " ".repeat(20)),
            },
        ],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(
        text.contains("1.234,56") && text.contains("9.876,54"),
        "{text}"
    );
    assert!(found_by(&out).is_empty());
    assert_no_loss(&out);
}

/// Guard: a decimal comma printed as text with a mark under it is written once.
#[test]
fn printed_point_is_not_doubled() {
    let x = 40.0 + 20.0 * PITCH;
    let page = FormPage {
        // The mark stands under the comma, the fifth character.
        template: mark(x + 4.5 * PITCH - 0.12, 699.5),
        lines: vec![
            DataLine {
                y: 700.0,
                text: padded(20, "1234,56"),
            },
            DataLine {
                y: 600.0,
                text: "Summe 9.876,54".into(),
            },
            DataLine {
                y: 586.0,
                text: "Netto 1.234,56".into(),
            },
        ],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(text.contains("1234,56"), "{text}");
    assert!(!text.contains("1234,,56") && !text.contains(",,"), "{text}");
    assert!(found_by(&out).is_empty());
    assert_no_loss(&out);
}

/// Guard: a mark between a letter and a digit is no separator.
#[test]
fn mark_beside_a_letter_is_ignored() {
    let x = 40.0 + 20.0 * PITCH;
    let page = FormPage {
        template: mark(x + 2.0 * PITCH - 0.12, 699.5),
        lines: vec![DataLine {
            y: 700.0,
            text: padded(20, "AB1234"),
        }],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(all_text(&out).contains("AB1234"), "{}", all_text(&out));
    assert!(found_by(&out).is_empty());
    assert_no_loss(&out);
}

/// More marks than a page keeps: a Limit note says so, and the read stays quick.
#[test]
fn many_marks_are_bounded() {
    let mut template = String::with_capacity(2_600_000);
    // 100,001 marks in rows of 1,000 below the data line, none near a digit, and too far
    // apart to be dashed lines.
    for k in 0..100_001usize {
        let (col, row) = (k % 1000, k / 1000);
        template += &format!(
            "{:.2} {:.2} 0.24 2 re f\n",
            20.0 + col as f32 * 0.55,
            20.0 + row as f32 * 5.0
        );
    }
    let mut page = boxed_page("123456", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    page.template += &template;
    let start = Instant::now();
    let out = read(&print_form(&[page], "", ""));
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    let limit: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit && l.what.contains("100000 drawn marks"))
        .collect();
    assert_eq!(limit.len(), 1, "{:?}", out.document.losses);
    // The marks are no rules and no code.
    assert!(out.document.assets.is_empty(), "{:?}", out.document.assets);
    // The field's mark is among the first ones the page draws and is still read.
    assert!(all_text(&out).contains("1234,56"));
}

/// A highlight over a boxed amount quotes it with the separator the body has.
#[test]
fn quote_over_a_boxed_amount_has_its_separator() {
    let mut page = boxed_page("123456", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    let x = 40.0 + 20.0 * PITCH;
    let (x1, y0, y1) = (x + 6.0 * PITCH, 697.0, 709.0);
    page.annots.push(format!(
        "<< /Type /Annot /Subtype /Highlight /Rect [{x} {y0} {x1} {y1}] \
         /QuadPoints [{x} {y1} {x1} {y1} {x} {y0} {x1} {y0}] >>"
    ));
    let out = read(&print_form(&[page], "", ""));
    let list = &out.document.facets.annotations;
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].quoted.as_deref(), Some("1234,56"), "{list:?}");
}

/// Two pages: the style comes from the text of the whole file, and each page with drawn
/// separators has its own note.
#[test]
fn style_from_another_page() {
    let first = boxed_page("123456", &[4], &[]);
    let second = boxed_page("654321", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    let out = read(&print_form(&[first, second], "", ""));
    let text = all_text(&out);
    assert!(
        text.contains("1234,56") && text.contains("6543,21"),
        "{text}"
    );
    assert_eq!(found_by(&out).len(), 2);
}

/// An amount with a drawn decimal mark printed twice 0.3 points apart (a fake bold) reads
/// once with one separator, in the body and in a quote over it. The field has no boxes here:
/// with per-digit boxes the two copies are read twice even without a mark (reported apart).
#[test]
fn overprinted_boxed_amount_reads_once() {
    let mut page = boxed_page("123456", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    let x = 40.0 + 20.0 * PITCH;
    page.template = label(40.0, 712.0, 6.0, "Betrag") + &mark(x + 4.0 * PITCH - 0.12, 699.5);
    // The labels of the template leave a horizontal scaling of 90 % set.
    page.template += "100 Tz\n";
    page.template += &data_line(40.3, 700.0, 9.5, &padded(20, "123456"));
    let (x1, y0, y1) = (x + 6.0 * PITCH, 697.0, 709.0);
    page.annots.push(format!(
        "<< /Type /Annot /Subtype /Highlight /Rect [{x} {y0} {x1} {y1}] \
         /QuadPoints [{x} {y1} {x1} {y1} {x} {y0} {x1} {y0}] >>"
    ));
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert_eq!(text.matches("1234,56").count(), 1, "{text}");
    assert!(!text.contains(",,") && !text.contains("123456"), "{text}");
    let list = &out.document.facets.annotations;
    assert_eq!(list[0].quoted.as_deref(), Some("1234,56"), "{list:?}");
    assert!(
        separator_losses(&out).is_empty(),
        "{:?}",
        out.document.losses
    );
    assert_eq!(
        found_by(&out),
        [
            "Found by: 1 decimal separator on this page is a drawn mark, not text; it is \
          written as \",\" as in 2 amounts written as text in this file."
        ]
    );
}

/// The border of a table drawn as filled dashes, each `w` by `h`, from (x0, y0) to (x1, y1)
/// along one axis, a dash and a gap of 1 point after the other.
pub(crate) fn dashes(x0: f32, y0: f32, x1: f32, y1: f32) -> String {
    let mut out = String::new();
    if y0 == y1 {
        let mut x = x0;
        while x < x1 {
            out += &format!("{x:.2} {:.2} 1.5 0.3 re f\n", y0 - 0.15);
            x += 2.5;
        }
    } else {
        let mut y = y0;
        while y < y1 {
            out += &format!("{:.2} {y:.2} 0.3 1.5 re f\n", x0 - 0.15);
            y += 2.5;
        }
    }
    out
}

/// Guard: a table whose rules are drawn as many small filled dashes is still a table, though
/// each dash has the shape of a mark.
#[test]
fn dashed_borders_still_make_a_table() {
    let (xs, ys) = ([100.0f32, 250.0, 400.0], [600.0f32, 630.0, 660.0]);
    let mut template = String::from("0 g\n");
    for y in ys {
        template += &dashes(xs[0], y, xs[2], y);
    }
    for x in xs {
        template += &dashes(x, ys[0], x, ys[2]);
    }
    template += &label(105.0, 640.0, 9.0, "Name");
    template += &label(255.0, 640.0, 9.0, "Betrag");
    template += &label(105.0, 610.0, 9.0, "Muster GmbH");
    template += &label(255.0, 610.0, 9.0, "1.234,56");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let tables = out
        .document
        .body
        .iter()
        .filter_map(|b| match b {
            Block::Section { blocks, .. } => Some(blocks),
            _ => None,
        })
        .flatten()
        .filter(|b| matches!(b, Block::Table { .. }))
        .count();
    assert_eq!(tables, 1, "{}\n{:?}", all_text(&out), out.document.losses);
}

/// The separator losses of a read, as their text.
pub(crate) fn separator_texts(out: &ReadOutcome) -> Vec<String> {
    separator_losses(out)
        .iter()
        .map(|l| l.what.clone())
        .collect()
}

/// One rate written with a point and one time do not stop eleven comma amounts: the note
/// names the mixed vote.
#[test]
fn a_clear_majority_of_amounts_sets_the_style() {
    let mut lines: Vec<String> = (1..=11).map(|k| format!("Posten {k} {k}.234,56")).collect();
    lines.push("Satz 1.55 %".into());
    lines.push("Beginn 12.30".into());
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let page = boxed_page("123456", &[4], &refs);
    let out = read(&print_form(&[page], "", ""));
    assert!(all_text(&out).contains("1234,56"), "{}", all_text(&out));
    assert!(
        separator_texts(&out).is_empty(),
        "{:?}",
        out.document.losses
    );
    assert_eq!(
        found_by(&out),
        [
            "Found by: 1 decimal separator on this page is a drawn mark, not text; it is \
          written as \",\" as in 11 of 12 amounts written as text in this file."
        ]
    );
}

/// Two amounts against two: no separator is written.
#[test]
fn a_tied_vote_sets_no_style() {
    let page = boxed_page(
        "123456",
        &[4],
        &[
            "Summe 9.876,54",
            "Netto 1.234,56",
            "Total 1,234.56",
            "Net 9,876.54",
        ],
    );
    let out = read(&print_form(&[page], "", ""));
    assert!(all_text(&out).contains("123456"));
    assert_eq!(separator_texts(&out).len(), 1, "{:?}", out.document.losses);
    assert_eq!(out.completeness, Completeness::Partial);
}

/// A mark one digit from the right is a decimal separator of one decimal, also after groups.
#[test]
fn one_decimal_amounts_are_read() {
    let x = 40.0 + 20.0 * PITCH;
    let mut page = boxed_page("300", &[2], &["Summe 9.876,54", "Netto 1.234,56"]);
    page.template += &boxed_field(x, 650.0, 2, &[1]);
    page.lines.push(DataLine {
        y: 650.0,
        text: padded(20, "90"),
    });
    page.template += &boxed_field(x, 630.0, 5, &[1, 4]);
    page.lines.push(DataLine {
        y: 630.0,
        text: padded(20, "12345"),
    });
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    for value in ["30,0", "9,0", "1.234,5"] {
        assert!(text.contains(value), "{value}: {text}");
    }
    assert!(
        separator_texts(&out).is_empty(),
        "{:?}",
        out.document.losses
    );
    assert_eq!(
        found_by(&out),
        [
            "Found by: 3 decimal separators and 1 group separator on this page are drawn marks, \
          not text; they are written as \",\" and \".\" as in 2 amounts written as text in \
          this file."
        ]
    );
}

/// A mark between digits that read upward on a page read left to right is not read: a loss
/// says so.
#[test]
fn a_mark_on_turned_digits_is_a_loss() {
    let mut page = boxed_page("", &[], &["Summe 9.876,54", "Netto 1.234,56"]);
    // Digits from (300, 400) upward; the mark lies across the baseline between the fourth and
    // the fifth digit.
    page.template += "100 Tz q 0 1 -1 0 300 400 cm BT /FD 9.5 Tf 0 0 Td (123456) Tj ET Q\n";
    page.template += &format!("298 {:.2} 2 0.24 re f\n", 400.0 + 4.0 * PITCH - 0.12);
    let out = read(&print_form(&[page], "", ""));
    assert!(all_text(&out).contains("123456"), "{}", all_text(&out));
    assert_eq!(
        separator_texts(&out),
        [
            "a separator drawn as a mark on page 1 is not read: the number 123456 is kept \
          without it, as its digits do not read in the direction of the page's text"
        ]
    );
    assert_eq!(out.completeness, Completeness::Partial);
}

/// Marks near many digits that are no separators (too high above the baseline) stop the
/// search with a Limit note once it has tested too many.
#[test]
fn many_marks_near_digits_end_the_search_with_a_note() {
    let mut template = String::with_capacity(2_200_000);
    for _ in 0..100_000 {
        // 3.5 points above the baseline at 700: no separator for a 9.5 point digit.
        template += "176.68 703.5 0.24 2 re f\n";
    }
    let mut page = FormPage {
        template,
        ..FormPage::default()
    };
    // 41 copies of one line, each a hundredth of a point on: 41 digit pairs at the marks.
    for k in 0..41 {
        page.template += &data_line(40.0 + 0.01 * k as f32, 700.0, 9.5, &padded(20, "123456"));
    }
    let start = Instant::now();
    let out = read(&print_form(&[page], "", ""));
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "{:?}",
        start.elapsed()
    );
    let notes: Vec<_> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Limit && l.what.contains("looked for only in part"))
        .collect();
    assert_eq!(notes.len(), 1, "{:?}", out.document.losses);
}

/// A number cut into two runs (a change of font inside it) reads with its separator in the
/// body and in a quote over it.
#[test]
fn a_number_cut_into_two_runs_reads_as_its_quote() {
    let x = 40.0 + 20.0 * PITCH;
    let mut page = boxed_page("", &[], &["Summe 9.876,54", "Netto 1.234,56"]);
    page.template += &format!(
        "100 Tz BT /FD 9.5 Tf {x} 700 Td (1234) Tj /FB 9.5 Tf (56) Tj ET\n{}",
        mark(x + 4.0 * PITCH - 0.12, 699.5)
    );
    let (x1, y0, y1) = (x + 6.0 * PITCH, 697.0, 709.0);
    page.annots.push(format!(
        "<< /Type /Annot /Subtype /Highlight /Rect [{x} {y0} {x1} {y1}] \
         /QuadPoints [{x} {y1} {x1} {y1} {x} {y0} {x1} {y0}] >>"
    ));
    let out = read(&print_form(&[page], "", ""));
    assert!(all_text(&out).contains("1234,56"), "{}", all_text(&out));
    let list = &out.document.facets.annotations;
    assert_eq!(list[0].quoted.as_deref(), Some("1234,56"), "{list:?}");
    assert!(
        separator_texts(&out).is_empty(),
        "{:?}",
        out.document.losses
    );
}

// ---------------------------------------------------------------------------------------
// Form grids of several sections
