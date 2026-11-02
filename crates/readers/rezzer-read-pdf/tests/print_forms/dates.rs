// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The document title and the document date of a form's head.

use super::*;

/// Lines of plain `size` pt prose in Helvetica from `top` down, at least 600 ink characters,
/// so that `size` is the body size.
pub(crate) fn prose(size: f32, top: f32) -> String {
    const LINE: &str = "die Angaben werden von der Muster GmbH vertraulich behandelt und";
    let mut out = String::new();
    let (mut ink, mut y) = (0, top);
    while ink < 600 {
        out.push_str(&common::show("FL", size, 40.0, y, LINE));
        ink += LINE.chars().filter(|c| !c.is_whitespace()).count();
        y -= size * 1.3;
    }
    out
}

/// A one-page form whose template holds `head` over prose in `size` pt from y 450 down, drawn
/// `scale` times as large, with the Info entries `info`.
pub(crate) fn headed(head: &str, size: f32, scale: f32, info: &str) -> Vec<u8> {
    let page = FormPage {
        template: format!("{head}{}", prose(size, 450.0)),
        scale,
        ..FormPage::default()
    };
    print_form(&[page], info, "")
}

/// The value of the frontmatter key `key` of the Markdown of `out`, if it is there.
pub(crate) fn front(out: &ReadOutcome, key: &str) -> Option<String> {
    let md = common::markdown(out);
    let head = md.split("\n---\n").next().unwrap_or("").to_string();
    head.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(": "))
        .map(str::to_string)
}

/// The loss of a head with one date that nothing marks as the document date.
pub(crate) const ONE_DATE: &str =
    "1 date stands in the head of page 1; it is not marked as the document date";

/// The loss of a head with two dates.
pub(crate) const TWO_DATES: &str =
    "2 dates stand in the head of page 1; none is marked as the document date";

/// The words of the losses of `out` that are `Uncertain`.
pub(crate) fn unsure(out: &ReadOutcome) -> Vec<&str> {
    out.document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Uncertain)
        .map(|l| l.what.as_str())
        .collect()
}

#[test]
fn title_from_largest_heading() {
    let head = common::show("FB", 14.0, 40.0, 780.0, "Entgeltabrechnung Muster GmbH")
        + &common::show("FB", 11.0, 40.0, 740.0, "Abrechnungszeitraum");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    let p = &out.document.properties;
    assert_eq!(p.title.as_deref(), Some("Entgeltabrechnung Muster GmbH"));
    assert_eq!(
        p.title_from.as_deref(),
        Some("largest heading in the top third of page 1")
    );
    assert_eq!(
        front(&out, "title").as_deref(),
        Some("\"Entgeltabrechnung Muster GmbH\"")
    );
    assert_eq!(
        front(&out, "title_from").as_deref(),
        Some("\"largest heading in the top third of page 1\"")
    );
}

#[test]
fn title_from_largest_heading_in_any_size_and_font() {
    // The body size, the title's style and font and the page size vary; the title is always
    // larger than the body by the same share.
    for body in [7.0f32, 9.5, 12.0] {
        for (font, ratio) in [("FB", 1.3f32), ("FL", 1.6), ("FTB", 1.3), ("FT", 1.6)] {
            for scale in [0.8f32, 1.0, 1.25] {
                let head = common::show(font, body * ratio, 40.0, 780.0, "Musterbescheid 2026")
                    + &common::show("FL", body, 40.0, 600.0, "Muster GmbH");
                let out = read(&headed(&head, body, scale, ""));
                assert_eq!(
                    out.document.properties.title.as_deref(),
                    Some("Musterbescheid 2026"),
                    "body {body}, font {font}, ratio {ratio}, scale {scale}"
                );
            }
        }
    }
}

#[test]
fn heading_below_the_top_third_is_no_title() {
    // The only heading stands at the middle of the page.
    let head = common::show("FB", 14.0, 40.0, 500.0, "Musterbescheid");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    assert_eq!(out.document.properties.title, None);
    assert_eq!(out.document.properties.title_from, None);
    assert_eq!(front(&out, "title").as_deref(), Some("\"file.pdf\""));
}

#[test]
fn equal_headings_go_by_bold_then_height() {
    // Two headings of one size: the bold one wins over the higher plain one.
    let head = common::show("FL", 14.0, 40.0, 790.0, "Musterkopf")
        + &common::show("FB", 14.0, 40.0, 740.0, "Musterbescheid");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    assert_eq!(
        out.document.properties.title.as_deref(),
        Some("Musterbescheid")
    );
    // Both bold: the higher one.
    let head = common::show("FB", 14.0, 40.0, 790.0, "Musterkopf")
        + &common::show("FB", 14.0, 40.0, 740.0, "Musterbescheid");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    assert_eq!(out.document.properties.title.as_deref(), Some("Musterkopf"));
}

#[test]
fn info_title_wins() {
    let head = common::show("FB", 14.0, 40.0, 780.0, "Entgeltabrechnung Muster GmbH");
    let out = read(&headed(&head, 9.0, 1.0, "/Title (Lohn Muster)"));
    let p = &out.document.properties;
    assert_eq!(p.title.as_deref(), Some("Lohn Muster"));
    assert_eq!(p.title_from, None);
    assert_eq!(front(&out, "title_from"), None);
}

#[test]
fn labelled_date_is_document_date() {
    let head = common::show("FB", 14.0, 40.0, 780.0, "Musterbescheid")
        + &common::show("FL", 9.0, 400.0, 760.0, "Datum 01.02.2026")
        + &common::show("FL", 9.0, 40.0, 700.0, "Zeitraum 01.01.2026 31.01.2026");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    let found = out
        .document
        .properties
        .document_date
        .as_ref()
        .expect("date");
    assert_eq!(found.date, "2026-02-01");
    assert_eq!(found.from, "date label \"Datum\" on page 1");
    assert_eq!(front(&out, "source_date").as_deref(), Some("2026-02-01"));
    assert_eq!(
        front(&out, "temporal_confidence").as_deref(),
        Some("explicit")
    );
    assert_eq!(
        front(&out, "document_date").as_deref(),
        Some("\"2026-02-01\"")
    );
    assert_eq!(
        front(&out, "document_date_from").as_deref(),
        Some("\"date label \\\"Datum\\\" on page 1\"")
    );
    assert!(unsure(&out).is_empty(), "{:?}", unsure(&out));
}

#[test]
fn labelled_dates_of_every_form() {
    // The label on the date's line, with or without a colon, in any case, or over its value
    // (one line below it); sizes, fonts and the page size vary.
    // Each line: its text, x, y and how many lines below y it stands.
    type Lines<'a> = &'a [(&'a str, f32, f32, f32)];
    let cases: [(Lines<'_>, Option<&str>); 9] = [
        (
            &[("Datum: 01.02.2026", 40.0, 760.0, 0.0)],
            Some("2026-02-01"),
        ),
        (&[("DATE 2026-02-01", 40.0, 760.0, 0.0)], Some("2026-02-01")),
        (
            &[("Erstellt am 1.2.2026", 40.0, 760.0, 0.0)],
            Some("2026-02-01"),
        ),
        (
            &[("Fecha: 13/02/2026", 40.0, 760.0, 0.0)],
            Some("2026-02-13"),
        ),
        (
            &[("Dated 02/13/2026", 40.0, 760.0, 0.0)],
            Some("2026-02-13"),
        ),
        // Both parts 12 or less: not read.
        (&[("Date 02/03/2026", 40.0, 760.0, 0.0)], None),
        // No such day.
        (&[("Datum 31.04.2026", 40.0, 760.0, 0.0)], None),
        // The label over its value, and the label beside it.
        (
            &[
                ("Datum", 400.0, 200.0, 0.0),
                ("01.02.2026", 400.0, 200.0, 1.0),
            ],
            Some("2026-02-01"),
        ),
        (
            &[
                ("Ausgestellt am:", 40.0, 200.0, 0.0),
                ("01.02.2026", 150.0, 200.0, 0.0),
            ],
            Some("2026-02-01"),
        ),
    ];
    for (lines, want) in cases {
        for (size, font, scale) in [(7.0f32, "FL", 0.8f32), (9.5, "FB", 1.0), (12.0, "FT", 1.25)] {
            let head: String = lines
                .iter()
                .map(|(t, x, y, below)| {
                    // x as set in 10 pt, moved with the size from the left margin.
                    let x = 40.0 + (x - 40.0) * size / 10.0;
                    common::show(font, size, x, y - below * size * 1.2, t)
                })
                .collect();
            let out = read(&headed(&head, size, scale, ""));
            let got = out.document.properties.document_date.as_ref();
            assert_eq!(
                got.map(|d| d.date.as_str()),
                want,
                "{lines:?} in {size} pt {font}, scale {scale}"
            );
        }
    }
}

#[test]
fn one_unlabelled_date_in_the_head_is_not_guessed() {
    // A date in the top third that nothing marks as the document date is not chosen; the
    // loss says that it stands there.
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let head = common::show(font, size, 400.0, 760.0, "Musterstadt, 01.02.2026");
            let out = read(&headed(&head, size, scale, ""));
            let case = format!("{size} pt {font}");
            assert_eq!(out.document.properties.document_date, None, "{case}");
            assert_eq!(front(&out, "document_date"), None, "{case}");
            assert_eq!(front(&out, "source_date"), None, "{case}");
            assert_eq!(unsure(&out), [ONE_DATE], "{case}");
            assert_eq!(out.completeness, Completeness::Complete, "{case}");
            // The same date twice is still one date.
            let head = head + &common::show(font, size, 40.0, 700.0, "Stand 01.02.2026");
            let out = read(&headed(&head, size, scale, ""));
            assert_eq!(out.document.properties.document_date, None, "{case}");
            assert_eq!(unsure(&out), [ONE_DATE], "{case}");
        }
    }
}

#[test]
fn two_unlabelled_dates_are_not_guessed() {
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        let head = common::show("FL", size, 400.0, 760.0, "Musterstadt, 01.02.2026")
            + &common::show(
                "FL",
                size,
                40.0,
                700.0,
                "Zeitraum 01.01.2026 bis 31.01.2026",
            );
        let out = read(&headed(&head, size, scale, ""));
        assert_eq!(out.document.properties.document_date, None);
        assert_eq!(front(&out, "document_date"), None);
        assert_eq!(front(&out, "source_date"), None);
        assert_eq!(
            unsure(&out),
            ["3 dates stand in the head of page 1; none is marked as the document date"],
            "{size} pt, scale {scale}"
        );
        assert_eq!(out.completeness, Completeness::Complete);
    }
}

#[test]
fn two_labelled_dates_that_differ_are_not_guessed() {
    let head = common::show("FL", 9.0, 400.0, 760.0, "Datum 01.02.2026")
        + &common::show("FL", 9.0, 40.0, 200.0, "Date 15.02.2026");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    assert_eq!(out.document.properties.document_date, None);
    assert_eq!(
        unsure(&out),
        [
            "2 different dates stand after a date label on page 1; none is taken as the document \
          date"
        ]
    );
}

#[test]
fn compound_date_label_is_not_the_document_date() {
    // Below the top third, so only a label could make it the document date.
    let head = common::show("FL", 9.0, 40.0, 200.0, "Eintrittsdatum 01.01.2020");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    assert_eq!(out.document.properties.document_date, None);
    assert_eq!(front(&out, "document_date"), None);
    assert!(unsure(&out).is_empty(), "{:?}", unsure(&out));
}

#[test]
fn compound_english_date_label_is_no_label() {
    // `Due date` is no date label: its date is not chosen, and the plain date in the head is
    // not chosen either; the loss names the head date.
    for label in ["Due date", "Delivery date", "Start Date:"] {
        let head = common::show("FL", 9.0, 400.0, 760.0, "Musterstadt, 01.02.2026")
            + &common::show("FL", 9.0, 40.0, 200.0, &format!("{label} 15.03.2026"));
        let out = read(&headed(&head, 9.0, 1.0, ""));
        assert_eq!(out.document.properties.document_date, None, "{label}");
        assert_eq!(unsure(&out), [ONE_DATE], "{label}");
    }
}

#[test]
fn other_named_dates_are_no_document_date() {
    // A date with a label of its own that is no date label: never the document date, also
    // when it is the only date in the head.
    for line in [
        "Due date 15.03.2026",
        "Geburtsdatum 01.01.1970",
        "Eintrittsdatum 01.01.2020",
        "Rechnungsdatum 01.02.2026",
        "F\u{e4}llig am 15.02.2026",
        "Lieferung: 15.02.2026",
    ] {
        for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
            for font in ["FL", "FB"] {
                let head = common::show("FB", size * 1.6, 40.0, 790.0, "Musterbescheid")
                    + &common::show(font, size, 40.0, 760.0, line);
                let out = read(&headed(&head, size, scale, ""));
                assert_eq!(
                    out.document.properties.document_date, None,
                    "{line} in {size} pt {font}"
                );
                assert_eq!(front(&out, "source_date"), None, "{line}");
                assert_eq!(unsure(&out), [ONE_DATE], "{line} in {size} pt {font}");
            }
        }
    }
}

#[test]
fn labelled_date_wins_over_a_due_date() {
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let head = common::show(font, size, 400.0, 760.0, "Datum 01.02.2026")
                + &common::show(font, size, 40.0, 700.0, "F\u{e4}llig am 15.02.2026");
            let out = read(&headed(&head, size, scale, ""));
            assert_eq!(
                out.document
                    .properties
                    .document_date
                    .as_ref()
                    .map(|d| d.date.as_str()),
                Some("2026-02-01"),
                "{size} pt {font}"
            );
            assert!(unsure(&out).is_empty(), "{:?}", unsure(&out));
        }
    }
}

#[test]
fn date_column_of_a_table_gives_no_date() {
    // A grid of three columns below the top third: `Datum` is a column title over one row,
    // no label of a pair.
    let mut head = String::new();
    for x in [40.0f32, 140.0, 280.0, 380.0] {
        head += &format!("0.5 w {x} 150 m {x} 210 l S\n");
    }
    for y in [150.0f32, 180.0, 210.0] {
        head += &common::hrule(40.0, 380.0, y);
    }
    for (x, t) in [(45.0f32, "Pos"), (145.0, "Datum"), (285.0, "Betrag")] {
        head += &common::show("FL", 9.0, x, 190.0, t);
    }
    for (x, t) in [(45.0f32, "1"), (145.0, "01.02.2026"), (285.0, "9,99")] {
        head += &common::show("FL", 9.0, x, 160.0, t);
    }
    let out = read(&headed(&head, 9.0, 1.0, ""));
    assert_eq!(out.document.properties.document_date, None);
    let tables = out
        .document
        .body
        .iter()
        .flat_map(|b| match b {
            Block::Section { blocks, .. } => blocks.as_slice(),
            _ => std::slice::from_ref(b),
        })
        .filter(|b| matches!(b, Block::Table { .. }))
        .count();
    assert_eq!(tables, 1, "the grid is read as a table");
    assert!(unsure(&out).is_empty(), "{:?}", unsure(&out));
}

#[test]
fn labelled_date_not_read_for_sure_says_so() {
    for (line, want) in [
        (
            "Date 02/03/2026",
            "the date 02/03/2026 after the label \"Date\" on page 1 can be read two ways",
        ),
        (
            "Datum 31.04.2026",
            "the date 31.04.2026 after the label \"Datum\" on page 1 is no day of the calendar",
        ),
    ] {
        let head = common::show("FL", 9.0, 400.0, 760.0, line)
            + &common::show("FL", 9.0, 40.0, 700.0, "Zeitraum 01.01.2026 bis 31.01.2026");
        let out = read(&headed(&head, 9.0, 1.0, ""));
        assert_eq!(out.document.properties.document_date, None);
        assert_eq!(unsure(&out), [want]);
    }
}

#[test]
fn two_word_label_with_wide_space() {
    let head = common::show("FL", 9.0, 400.0, 760.0, "Erstellt   am 01.02.2026");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    let found = out
        .document
        .properties
        .document_date
        .as_ref()
        .expect("date");
    assert_eq!(found.date, "2026-02-01");
    assert_eq!(found.from, "date label \"Erstellt am\" on page 1");
}

#[test]
fn far_value_on_the_label_baseline_is_not_its_value() {
    // Below the top third, so only a label could make it the document date.
    let head = common::show("FL", 9.0, 40.0, 200.0, "Datum:")
        + &common::show("FL", 9.0, 480.0, 200.0, "01.02.2026");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    assert_eq!(out.document.properties.document_date, None);
}

#[test]
fn contents_heading_is_no_title() {
    for contents in [
        "Contents",
        "Table of Contents",
        "INHALTSVERZEICHNIS",
        "Inhalt",
        "Table des mati\u{e8}res",
        "\u{cd}ndice",
        "Index:",
    ] {
        let head = common::show("FB", 14.0, 40.0, 790.0, contents)
            + &common::show("FB", 12.0, 40.0, 740.0, "Musterbericht");
        let out = read(&headed(&head, 9.0, 1.0, ""));
        assert_eq!(
            out.document.properties.title.as_deref(),
            Some("Musterbericht"),
            "{contents}"
        );
    }
}

#[test]
fn equal_value_of_a_lower_date_changes_nothing() {
    // The delivery date below the head has the value of a head date: the head still holds two
    // dates and none is taken.
    let want = [TWO_DATES];
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let head = common::show("FB", size * 1.6, 40.0, 790.0, "Musterbescheid")
                + &common::show(font, size, 400.0, 760.0, "Musterstadt, 01.02.2026")
                + &common::show(font, size, 40.0, 740.0, "Kunde seit 01.01.2020");
            // The other date's label on the date's line, or alone over it.
            for other in [
                common::show(font, size, 40.0, 200.0, "Lieferdatum 01.02.2026"),
                common::show(font, size, 40.0, 200.0, "Lieferdatum")
                    + &common::show(font, size, 40.0, 200.0 - size * 1.2, "01.02.2026"),
            ] {
                let out = read(&headed(&format!("{head}{other}"), size, scale, ""));
                assert_eq!(
                    out.document.properties.document_date, None,
                    "{size} pt {font}: {other}"
                );
                assert_eq!(unsure(&out), want, "{size} pt {font}: {other}");
            }
        }
    }
}

#[test]
fn one_row_item_grid_with_small_titles_gives_no_date() {
    // Column titles smaller than their values, one row, below the top third: the pairs layer
    // may read `Datum` over its value, but a pair table there gives no document date.
    for (size, scale) in [(7.0f32, 0.8f32), (9.0, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let title = size * 6.6 / 9.0;
            let mut head = String::new();
            for x in [40.0f32, 140.0, 280.0, 380.0] {
                head += &format!("0.5 w {x} 150 m {x} 210 l S\n");
            }
            for y in [150.0f32, 210.0] {
                head += &common::hrule(40.0, 380.0, y);
            }
            for (x, t) in [(45.0f32, "Pos"), (145.0, "Datum"), (285.0, "Betrag")] {
                head += &common::show("FL", title, x, 200.0, t);
            }
            for (x, t) in [(45.0f32, "1"), (145.0, "01.02.2026"), (285.0, "9,99")] {
                head += &common::show(font, size, x, 160.0, t);
            }
            let out = read(&headed(&head, size, scale, ""));
            assert_eq!(
                out.document.properties.document_date, None,
                "{size} pt {font}, scale {scale}"
            );
        }
    }
}

#[test]
fn other_date_label_across_a_column_gap_stays_its_own() {
    // An other date's label over its value in one column, a head date on the label's baseline
    // in another column: nothing marks either, so the head holds two dates.
    let want = [TWO_DATES];
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            for other in ["Geburtsdatum", "Lieferdatum"] {
                for (left, right) in [(40.0f32, 300.0f32), (300.0, 40.0)] {
                    let head = common::show("FB", size * 1.6, 40.0, 790.0, "Musterbescheid")
                        + &common::show(font, size, left, 760.0, other)
                        + &common::show(font, size, left, 760.0 - size * 1.2, "01.01.1970")
                        + &common::show(font, size, right, 760.0, "Kunde seit 01.01.2020");
                    let out = read(&headed(&head, size, scale, ""));
                    let case = format!("{size} pt {font}, {other} at x {left}");
                    assert_eq!(out.document.properties.document_date, None, "{case}");
                    assert_eq!(unsure(&out), want, "{case}");
                }
            }
        }
    }
}

#[test]
fn other_label_over_an_offset_value_gives_no_date() {
    // A label that names another date over its value, the value indented, right aligned or
    // centred under it: no date label marks the value, so it is not chosen.
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            for label in ["Geburtsdatum", "Kunde seit", "Lieferdatum"] {
                let value = "01.01.1970";
                let slack =
                    common::font_width(font, label, size) - common::font_width(font, value, size);
                for (how, x) in [
                    ("indented", 40.0 + size * 1.5),
                    ("right aligned", 40.0 + slack),
                    ("centred", 40.0 + slack / 2.0),
                ] {
                    let head = common::show("FB", size * 1.6, 40.0, 790.0, "Musterbescheid")
                        + &common::show(font, size, 40.0, 760.0, label)
                        + &common::show(font, size, x, 760.0 - size * 1.2, value);
                    let out = read(&headed(&head, size, scale, ""));
                    let case = format!("{label} over the value {how} in {size} pt {font}");
                    assert_eq!(out.document.properties.document_date, None, "{case}");
                    assert_eq!(front(&out, "source_date"), None, "{case}");
                    assert_eq!(unsure(&out), [ONE_DATE], "{case}");
                }
            }
        }
    }
}

#[test]
fn place_and_comma_before_the_head_date_is_no_marker() {
    // `Musterstadt, 01.02.2026` without `den`: a name with a comma looks the same, so the date
    // is not chosen.
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let head = common::show("FB", size * 1.6, 40.0, 790.0, "Musterbescheid")
                + &common::show(font, size, 400.0, 760.0, "Musterstadt, 01.02.2026");
            let out = read(&headed(&head, size, scale, ""));
            let case = format!("{size} pt {font}");
            assert_eq!(out.document.properties.document_date, None, "{case}");
            assert_eq!(front(&out, "source_date"), None, "{case}");
            assert_eq!(unsure(&out), [ONE_DATE], "{case}");
        }
    }
}

#[test]
fn plain_date_under_or_beside_a_heading_is_not_guessed() {
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let heading =
                |x: f32| common::show("FB", size * 1.6, x, 760.0 + size * 1.3, "Rechnung");
            let case = format!("{size} pt {font}");
            // The heading at the left margin, the date in the right column, and the heading
            // right over the date: nothing marks the date.
            for x in [40.0, 400.0] {
                let head = heading(x) + &common::show(font, size, 400.0, 760.0, "01.02.2026");
                let out = read(&headed(&head, size, scale, ""));
                assert_eq!(out.document.properties.document_date, None, "{case}");
                assert_eq!(unsure(&out), [ONE_DATE], "{case}");
            }
        }
    }
}

/// What a layout gives: a date with what `document_date_from` says, no date and no loss about
/// a date (the layout may note a table), or no date with a loss about dates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum HeadWant {
    Date(&'static str, &'static str),
    Nothing,
    Lost(&'static str),
    /// No date, with a loss about dates whose words vary with the size.
    NoDate,
}

/// Asserts that `out` gives `want`, naming `case` when it does not.
pub(crate) fn assert_head(out: &ReadOutcome, want: HeadWant, case: &str) {
    let found = out.document.properties.document_date.as_ref();
    match want {
        HeadWant::Date(date, from) => {
            assert_eq!(found.map(|d| d.date.as_str()), Some(date), "{case}");
            assert_eq!(found.map(|d| d.from.as_str()), Some(from), "{case}");
            assert!(unsure(out).is_empty(), "{case}: {:?}", unsure(out));
        }
        HeadWant::Nothing => {
            assert_eq!(found, None, "{case}");
            let about_dates: Vec<&str> = unsure(out)
                .into_iter()
                .filter(|w| w.contains("of page 1") && w.contains("date"))
                .collect();
            assert!(about_dates.is_empty(), "{case}: {about_dates:?}");
        }
        HeadWant::Lost(loss) => {
            assert_eq!(found, None, "{case}");
            assert_eq!(unsure(out), [loss], "{case}");
        }
        HeadWant::NoDate => {
            assert_eq!(found, None, "{case}");
            let losses = unsure(out);
            assert!(
                losses.len() == 1 && losses[0].contains("date"),
                "{case}: {losses:?}"
            );
        }
    }
}

#[test]
fn head_layouts_with_one_other_date() {
    // Layouts of the head with one date that has a label of its own which is no date label,
    // and one plain date or none. The rule: a date is chosen only when a date label marks it,
    // beside it, over it or in the cell left of it; the dates of the head give a loss. Never a
    // wrong date: a loss or no date is fine.
    use HeadWant::*;
    let one = Lost(ONE_DATE);
    let two = Lost(TWO_DATES);
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, y: f32, t: &str| common::show(font, size, x, y, t);
            let w = |t: &str| common::font_width(font, t, size);
            let below = |n: f32| 760.0 - n * size * 1.2;
            // An other date below the head, so that it says nothing about the head.
            let low = s(40.0, 200.0, "Lieferdatum 15.02.2026");
            let title = common::show("FB", size * 1.6, 40.0, 790.0, "Musterbescheid");
            let cases: Vec<(&str, String, HeadWant)> = vec![
                (
                    "A: other label beside its value as its own run",
                    title.clone()
                        + &s(40.0, 760.0, "Geburtsdatum:")
                        + &s(40.0 + size * 8.0, 760.0, "01.01.1970"),
                    one,
                ),
                (
                    "B: other label far left of its value on one baseline",
                    title.clone() + &s(40.0, 760.0, "Kunde seit") + &s(300.0, 760.0, "01.01.2020"),
                    one,
                ),
                (
                    "C: two rows of a label column and a value column",
                    title.clone()
                        + &s(40.0, 760.0, "Name")
                        + &s(200.0, 760.0, "Erika Mustermann")
                        + &s(40.0, below(1.0), "Geburtsdatum")
                        + &s(200.0, below(1.0), "01.01.1970"),
                    one,
                ),
                (
                    "D: letter with the place and date on the baseline of the first address line",
                    title.clone()
                        + &s(40.0, 760.0, "Erika Mustermann")
                        + &s(400.0, 760.0, "Musterstadt, 01.02.2026")
                        + &s(40.0, below(1.0), "Musterweg 1")
                        + &s(40.0, below(2.0), "12345 Musterstadt")
                        + &low,
                    one,
                ),
                (
                    "E: letter with the place and date alone on its line below the address",
                    title.clone()
                        + &s(40.0, 760.0, "Erika Mustermann")
                        + &s(40.0, below(1.0), "Musterweg 1")
                        + &s(40.0, below(2.0), "12345 Musterstadt")
                        + &s(400.0, 700.0, "Musterstadt, 01.02.2026")
                        + &low,
                    one,
                ),
                (
                    "F: other label over a centred value, a labelled document date beside",
                    title.clone()
                        + &s(40.0, 760.0, "Geburtsdatum")
                        + &s(
                            40.0 + (w("Geburtsdatum") - w("01.01.1970")) / 2.0,
                            below(1.0),
                            "01.01.1970",
                        )
                        + &s(400.0, 760.0, "Datum 01.02.2026"),
                    Date("2026-02-01", "date label \"Datum\" on page 1"),
                ),
                (
                    "G: the place on the line over the date",
                    title.clone()
                        + &s(400.0, 760.0, "Musterstadt,")
                        + &s(400.0, below(1.0), "01.02.2026")
                        + &low,
                    one,
                ),
                (
                    "H: place, comma and the article before the date",
                    title.clone() + &s(400.0, 760.0, "Musterstadt, den 01.02.2026") + &low,
                    one,
                ),
                (
                    "I: a heading right over the date",
                    common::show("FB", size * 1.6, 40.0, 760.0 + size * 1.3, "Rechnung")
                        + &s(40.0, 760.0, "01.02.2026")
                        + &low,
                    one,
                ),
                (
                    "J: a heading over the line of the date, in another column",
                    common::show("FB", size * 1.6, 40.0, 760.0 + size * 1.3, "Rechnung")
                        + &s(400.0, 760.0, "01.02.2026")
                        + &low,
                    one,
                ),
                (
                    "K: the other label after its value",
                    title.clone() + &s(40.0, 760.0, "01.01.1970 (Geburtsdatum)"),
                    one,
                ),
                (
                    "L: a name with its birth date, the plain date alone lower in the head",
                    title.clone()
                        + &s(40.0, 760.0, "Erika Mustermann, geb. 01.01.1970")
                        + &s(400.0, 700.0, "Musterstadt, 01.02.2026"),
                    two,
                ),
                (
                    "M: other label over its value, the plain date alone lower in the head",
                    title.clone()
                        + &s(40.0, 760.0, "Geburtsdatum")
                        + &s(40.0, below(1.0), "01.01.1970")
                        + &s(400.0, 700.0, "Musterstadt, 01.02.2026"),
                    two,
                ),
                (
                    "N: validity label over a right aligned value",
                    title.clone()
                        + &s(40.0, 760.0, "G\u{fc}ltig bis")
                        + &s(
                            40.0 + w("Gultig bis") - w("31.12.2026"),
                            below(1.0),
                            "31.12.2026",
                        )
                        + &low,
                    one,
                ),
                (
                    "O: company name far left of the date on one baseline",
                    s(40.0, 800.0, "Muster GmbH") + &s(450.0, 800.0, "01.02.2026") + &low,
                    one,
                ),
                (
                    "P: a period of two dates, the second after a word",
                    title.clone() + &s(40.0, 760.0, "Zeitraum 01.01.2026 bis 31.01.2026") + &low,
                    two,
                ),
                (
                    "Q: other label over the value in the right column, plain date left on the value's baseline",
                    title.clone()
                        + &s(400.0, 760.0, "Geburtsdatum")
                        + &s(
                            400.0 + w("Geburtsdatum") - w("01.01.1970"),
                            below(1.0),
                            "01.01.1970",
                        )
                        + &s(40.0, below(1.0), "Musterstadt, 01.02.2026"),
                    two,
                ),
                (
                    "R: a date and the other date two lines below it, same column",
                    title.clone()
                        + &s(400.0, 760.0, "Musterstadt, 01.02.2026")
                        + &s(400.0, below(2.0), "Kunde seit 01.01.2020"),
                    two,
                ),
            ];
            for (name, head, want) in cases {
                let out = read(&headed(&head, size, scale, ""));
                assert_head(&out, want, &format!("{name} in {size} pt {font}"));
            }
        }
    }
}

#[test]
fn rereview_probes_give_no_wrong_date() {
    // The probes of the round 3 re-review as a test: the labelled date, or a loss.
    use HeadWant::*;
    let one = Lost(ONE_DATE);
    let two = Lost(TWO_DATES);
    let by_label = Date("2026-02-01", "date label \"Datum\" on page 1");
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, y: f32, t: &str| common::show(font, size, x, y, t);
            let below = 760.0 - size * 1.2;
            let title = common::show("FB", size * 1.6, 40.0, 790.0, "Musterbescheid");
            let cases: Vec<(&str, String, HeadWant)> = vec![
                (
                    "P1: labels over values, the other date left",
                    title.clone()
                        + &s(40.0, 760.0, "Geburtsdatum")
                        + &s(40.0, below, "01.01.1970")
                        + &s(300.0, 760.0, "Datum")
                        + &s(300.0, below, "01.02.2026"),
                    two,
                ),
                (
                    "P2: labels over values, the other date right",
                    title.clone()
                        + &s(300.0, 760.0, "Geburtsdatum")
                        + &s(300.0, below, "01.01.1970")
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "01.02.2026"),
                    two,
                ),
                (
                    "P3: labels beside their values in two columns",
                    title.clone()
                        + &s(40.0, 760.0, "Geburtsdatum 01.01.1970")
                        + &s(300.0, 760.0, "Datum 01.02.2026"),
                    by_label,
                ),
                (
                    "P3b: the same, swapped",
                    title.clone()
                        + &s(300.0, 760.0, "Geburtsdatum 01.01.1970")
                        + &s(40.0, 760.0, "Datum 01.02.2026"),
                    by_label,
                ),
                (
                    "P4: other date beside its label, place and date in the other column",
                    title.clone()
                        + &s(40.0, 760.0, "Geburtsdatum 01.01.1970")
                        + &s(300.0, 760.0, "Musterstadt, 01.02.2026"),
                    two,
                ),
                (
                    "P5: label piece, the value beside it as its own run",
                    title.clone()
                        + &s(40.0, 760.0, "Lieferdatum:")
                        + &s(40.0 + size * 8.0, 760.0, "15.02.2026"),
                    one,
                ),
                (
                    "P6: label over the value indented 1.5 sizes",
                    title.clone()
                        + &s(40.0, 760.0, "Geburtsdatum")
                        + &s(40.0 + size * 1.5, below, "01.01.1970"),
                    one,
                ),
                (
                    "P7: labels over values in close columns",
                    title.clone()
                        + &s(40.0, 760.0, "Geburtsdatum")
                        + &s(40.0, below, "01.01.1970")
                        + &s(40.0 + size * 10.0, 760.0, "Datum")
                        + &s(40.0 + size * 10.0, below, "01.02.2026"),
                    two,
                ),
                (
                    "P8: Datum alone far left of an other label over its value",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(300.0, 760.0, "Lieferdatum")
                        + &s(300.0, below, "15.02.2026"),
                    one,
                ),
            ];
            for (name, head, want) in cases {
                let out = read(&headed(&head, size, scale, ""));
                assert_head(&out, want, &format!("{name} in {size} pt {font}"));
            }
        }
    }
}

#[test]
fn date_label_in_a_far_cell_is_lost() {
    // A row `Datum: | 26.10.2026`: the label and its value in two cells with a wide gap between
    // them, in the top third, drawn as two boxes or plain. The two boxes are no pair table, and
    // the label stands too far from the date to mark it: the date is lost, with the loss. (Close
    // to its value the label marks it, see `labelled_dates_of_every_form`.)
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let text = common::show(font, size, 44.0, 760.0, "Datum:")
                + &common::show(font, size, 304.0, 760.0, "26.10.2026");
            let cells = common::boxed(40.0, 756.0, 120.0, size * 1.6)
                + &common::boxed(300.0, 756.0, 120.0, size * 1.6);
            for (how, head) in [("plain", text.clone()), ("boxed", cells + &text)] {
                let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
                let out = read(&headed(&format!("{title}{head}"), size, scale, ""));
                let case = format!("{how} cells in {size} pt {font}");
                assert_head(&out, HeadWant::Lost(ONE_DATE), &case);
                assert_eq!(front(&out, "source_date"), None, "{case}");
            }
        }
    }
}

#[test]
fn head_date_with_text_after_it_is_not_guessed() {
    // A head line `22.10.2026 1` with the next field's label on the same line, a time after
    // the date, a number alone, or an other date's name: nothing marks the date, so it is not
    // chosen and the loss names it.
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, t: &str| common::show(font, size, x, 760.0, t);
            let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
            let heads = [
                (
                    "next field",
                    s(40.0, "22.10.2026 1")
                        + &s(40.0 + size * 10.0, "Seite:")
                        + &s(40.0 + size * 14.0, "1"),
                ),
                ("time", s(40.0, "22.10.2026 10:30")),
                ("number", s(40.0, "22.10.2026 1")),
                ("in parentheses", s(40.0, "01.01.1970 (Geburtsdatum)")),
                ("born", s(40.0, "01.01.1970 geboren")),
                ("due", s(40.0, "15.02.2026 (f\u{e4}llig)")),
            ];
            for (how, head) in heads {
                let out = read(&headed(&format!("{title}{head}"), size, scale, ""));
                let case = format!("{how} in {size} pt {font}");
                assert_head(&out, HeadWant::Lost(ONE_DATE), &case);
                assert_eq!(front(&out, "source_date"), None, "{case}");
            }
        }
    }
}

#[test]
fn place_and_den_before_the_date_mark_nothing() {
    // `Musterstadt, den 01.02.2026`: a place with `den` cannot be told from a weekday, an
    // appointment or a birth place (`Montag, den 15.02.2026`), so it marks no date. In the
    // head the date gives the loss; lower on the page nothing is said.
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            for line in [
                "Musterstadt, den 01.02.2026",
                "MUSTERSTADT, DEN 01.02.2026",
                "Frankfurt am Main, den 01.02.2026",
            ] {
                for (y, want) in [
                    (760.0, HeadWant::Lost(ONE_DATE)),
                    (300.0, HeadWant::Nothing),
                ] {
                    let head = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid")
                        + &common::show(font, size, 400.0, y, line);
                    let out = read(&headed(&head, size, scale, ""));
                    let case = format!("{line} at y {y} in {size} pt {font}");
                    assert_head(&out, want, &case);
                    assert_eq!(front(&out, "source_date"), None, "{case}");
                }
            }
        }
    }
}

#[test]
fn rereview_r4_probes_give_no_wrong_date() {
    // The probes W1 to W5 of the round 4 re-review: each gave a wrong date once. Now nothing
    // marks their dates, so each gives the loss for the dates of the head.
    use HeadWant::*;
    let three = "3 dates stand in the head of page 1; none is marked as the document date";
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, y: f32, t: &str| common::show(font, size, x, y, t);
            let pitch = size * 1.2;
            let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
            let low = s(40.0, 200.0, "Lieferdatum 15.02.2026");
            let mut cases: Vec<(String, String, HeadWant)> = Vec::new();
            // W4: an item table in the head with a `Datum` column, ruled or plain, 1 or 3 rows.
            for ruled in [true, false] {
                for rows in [1usize, 3] {
                    let mut head = title.clone();
                    let y0 = 740.0;
                    if ruled {
                        let bottom = y0 - pitch * (rows as f32 + 1.0) - 4.0;
                        for x in [40.0f32, 120.0, 260.0, 380.0] {
                            head += &format!("0.5 w {x} {bottom} m {x} {} l S\n", y0 + pitch);
                        }
                        head += &common::hrule(40.0, 380.0, y0 + pitch);
                        head += &common::hrule(40.0, 380.0, y0 - 3.0);
                        head += &common::hrule(40.0, 380.0, bottom);
                    }
                    for (x, t) in [(45.0f32, "Pos"), (125.0, "Datum"), (265.0, "Betrag")] {
                        head += &s(x, y0, t);
                    }
                    for r in 0..rows {
                        let y = y0 - pitch * (r as f32 + 1.0);
                        head += &s(45.0, y, &format!("{}", r + 1));
                        head += &s(125.0, y, &format!("1{}.01.2026", r + 5));
                        head += &s(265.0, y, "9,99");
                    }
                    let how = if ruled { "ruled" } else { "plain" };
                    let want = if rows == 1 {
                        Lost(ONE_DATE)
                    } else {
                        Lost(three)
                    };
                    cases.push((
                        format!("W4: {how} item table with {rows} row(s)"),
                        head,
                        want,
                    ));
                }
            }
            cases.push((
                "W1: the label under its value".to_string(),
                title.clone()
                    + &s(40.0, 760.0, "01.01.1970")
                    + &s(40.0, 760.0 - pitch, "Geburtsdatum")
                    + &low,
                Lost(ONE_DATE),
            ));
            cases.push((
                "W2: the label far right of its value".to_string(),
                title.clone()
                    + &s(40.0, 760.0, "01.01.1970")
                    + &s(300.0, 760.0, "Geburtsdatum")
                    + &low,
                Lost(ONE_DATE),
            ));
            cases.push((
                "W3: a due word after the date".to_string(),
                title.clone() + &s(40.0, 760.0, "15.03.2026 Zahlungsziel") + &low,
                Lost(ONE_DATE),
            ));
            cases.push((
                "W5: a name with a comma before the date".to_string(),
                title.clone() + &s(40.0, 760.0, "Erika Mustermann, 01.01.1970") + &low,
                Lost(ONE_DATE),
            ));
            for (name, head, want) in cases {
                let out = read(&headed(&head, size, scale, ""));
                let case = format!("{name} in {size} pt {font}");
                assert_head(&out, want, &case);
                assert_eq!(front(&out, "source_date"), None, "{case}");
            }
        }
    }
}

/// A two-column table of label and value rows in the head of a `headed` page, `Datum:` with
/// its date among them: `ruled` draws the grid, else the columns stand in white space alone.
pub(crate) fn label_value_rows(font: &str, size: f32, ruled: bool) -> String {
    let rows = [
        ("Rechnungsnummer:", "0000-0001"),
        ("Kundennummer:", "00000"),
        ("Datum:", "26.10.2026"),
        ("Sachbearbeiter:", "Erika Mustermann"),
        ("Seite:", "1 von 2"),
    ];
    let pitch = size * 2.0;
    let (top, xs) = (770.0, [40.0f32, 220.0, 400.0]);
    let bottom = top - pitch * rows.len() as f32;
    let mut head = String::new();
    if ruled {
        for x in xs {
            head += &format!("0.5 w {x} {bottom} m {x} {top} l S\n");
        }
        for k in 0..=rows.len() {
            head += &common::hrule(xs[0], xs[2], top - pitch * k as f32);
        }
    }
    for (k, (label, value)) in rows.iter().enumerate() {
        let y = top - pitch * (k as f32 + 0.7);
        head += &common::show(font, size, xs[0] + 5.0, y, label);
        head += &common::show(font, size, xs[1] + 5.0, y, value);
    }
    head
}

/// Three boxed fields in a row in the head, `Datum` the second: the label of each in its box
/// over its value, the values one padded data line (as `boxed_fields`).
pub(crate) fn boxed_date_fields(scale: f32, bold_data: bool) -> FormPage {
    let labels = ["Personalnummer", "Datum", "Steuerklasse"];
    let values = ["00000", "26.10.2026", "1"];
    let mut t = String::new();
    let mut line = String::new();
    for (k, x) in [40.0f32, 160.0, 280.0].into_iter().enumerate() {
        t += &boxed(x, 700.0, 120.0, 24.0);
        t += &label(x + 3.0, 716.0, 6.0, labels[k]);
        line = format!("{line:<0$}{1}", pad_to(x + 5.0), values[k]);
    }
    let mut lines = vec![DataLine {
        y: 703.0,
        text: line,
    }];
    for (k, l) in ["Muster GmbH", "Musterweg 1", "12345 Musterstadt"]
        .iter()
        .enumerate()
    {
        lines.push(DataLine {
            y: 600.0 - 12.0 * k as f32,
            text: (*l).into(),
        });
    }
    FormPage {
        template: t,
        lines,
        scale,
        bold_data,
        ..FormPage::default()
    }
}

#[test]
fn date_label_cell_beside_its_date_cell_marks_it() {
    // A table row `Datum: | 26.10.2026` among other rows: as a ruled table, as a borderless
    // table of two columns, and as a row of a pair table of boxed fields. The label cell with
    // the date as its right neighbour marks the date.
    let want = HeadWant::Date("2026-10-26", "date label \"Datum\" on page 1");
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            for ruled in [true, false] {
                let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
                let head = title + &label_value_rows(font, size, ruled);
                let out = read(&headed(&head, size, scale, ""));
                let how = if ruled { "ruled" } else { "borderless" };
                let case = format!("{how} rows in {size} pt {font}");
                let found = out.document.properties.document_date.as_ref();
                assert_eq!(found.map(|d| d.date.as_str()), Some("2026-10-26"), "{case}");
                assert_eq!(
                    found.map(|d| d.from.as_str()),
                    Some("date label \"Datum\" on page 1"),
                    "{case}"
                );
                // The borderless table may note the line under it; no loss speaks of the date.
                assert!(
                    !unsure(&out)
                        .iter()
                        .any(|w| w.contains("in the head of page 1")),
                    "{case}: {:?}",
                    unsure(&out)
                );
                assert_eq!(
                    front(&out, "source_date").as_deref(),
                    Some("2026-10-26"),
                    "{case}"
                );
            }
        }
    }
    for scale in [0.8f32, 1.0, 1.25] {
        for bold in [false, true] {
            let out = read(&print_form(&[boxed_date_fields(scale, bold)], "", ""));
            let case = format!("boxed fields at scale {scale}, bold {bold}");
            assert_head(&out, want, &case);
        }
    }
}

#[test]
fn date_label_cell_with_a_cell_between_marks_nothing() {
    // A table row `Datum | Name | 01.02.2026`: the date is not the label's right neighbour,
    // so it is not marked. Ruled, and as plain pieces on one baseline.
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            for ruled in [true, false] {
                let rows = [
                    ["Datum", "Name", "01.02.2026"],
                    ["Kundennummer", "00000", "4711"],
                ];
                let pitch = size * 2.0;
                let (top, xs) = (770.0, [40.0f32, 160.0, 300.0, 440.0]);
                let bottom = top - pitch * rows.len() as f32;
                let mut head = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
                if ruled {
                    for x in xs {
                        head += &format!("0.5 w {x} {bottom} m {x} {top} l S\n");
                    }
                    for k in 0..=rows.len() {
                        head += &common::hrule(xs[0], xs[3], top - pitch * k as f32);
                    }
                }
                for (k, row) in rows.iter().enumerate() {
                    for (x, text) in xs.iter().zip(row) {
                        let y = top - pitch * (k as f32 + 0.7);
                        head += &common::show(font, size, x + 5.0, y, text);
                    }
                }
                let out = read(&headed(&head, size, scale, ""));
                let how = if ruled { "ruled" } else { "plain" };
                let case = format!("{how} in {size} pt {font}");
                assert_head(&out, HeadWant::Lost(ONE_DATE), &case);
                assert_eq!(front(&out, "source_date"), None, "{case}");
            }
        }
    }
}

/// A ruled table of `rows` in `font` and `size`: its columns start at `xs` (the last one is
/// 120 points wide), its top at `top`; an empty cell draws no text.
pub(crate) fn ruled_rows(rows: &[&[&str]], xs: &[f32], top: f32, size: f32, font: &str) -> String {
    let pitch = size * 1.8;
    let bottom = top - pitch * rows.len() as f32;
    let right = xs[xs.len() - 1] + 120.0;
    let mut out = String::new();
    for x in xs.iter().chain(std::iter::once(&right)) {
        out += &format!("0.5 w {x} {bottom} m {x} {top} l S\n");
    }
    for r in 0..=rows.len() {
        out += &common::hrule(xs[0], right, top - pitch * r as f32);
    }
    for (r, row) in rows.iter().enumerate() {
        let y = top - pitch * (r as f32 + 1.0) + size * 0.5;
        for (x, text) in xs.iter().zip(row.iter()) {
            if !text.is_empty() {
                out += &common::show(font, size, x + 4.0, y, text);
            }
        }
    }
    out
}

#[test]
fn rereview_r6_probes_give_no_wrong_date() {
    // The probes of the round 6 re-review (X1 to X4 and the checks beside them) as a test.
    // Tables at y 750 stand in the head, at y 250 in the body under the prose. Never a wrong
    // date.
    use HeadWant::*;
    let one = Lost(ONE_DATE);
    let two = Lost(TWO_DATES);
    let three = Lost("3 dates stand in the head of page 1; none is marked as the document date");
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, y: f32, t: &str| common::show(font, size, x, y, t);
            let ruled =
                |rows: &[&[&str]], xs: &[f32], top: f32| ruled_rows(rows, xs, top, size, font);
            let below = 760.0 - size * 1.2;
            let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
            let transposed: &[&[&str]] = &[
                &["Datum", "15.01.2026", "16.01.2026", "17.01.2026"],
                &["Betrag", "9,99", "19,99", "29,99"],
            ];
            let transposed_3: &[&[&str]] = &[
                &["Pos", "1", "2"],
                &["Datum", "15.01.2026", "16.01.2026"],
                &["Betrag", "9,99", "19,99"],
            ];
            let timesheet: &[&[&str]] = &[
                &["Datum", "15.01.2026", "Stunden", "8"],
                &["Datum", "16.01.2026", "Stunden", "7"],
            ];
            let appointment: &[&[&str]] = &[
                &["Termin", "Raum 1"],
                &["Datum", "15.01.2026"],
                &["Uhrzeit", "10:00"],
            ];
            let items: &[&[&str]] = &[
                &["Datum", "Text", "Betrag"],
                &["15.01.2026", "Muster", "9,99"],
                &["16.01.2026", "Muster", "9,99"],
            ];
            let two_titles: &[&[&str]] = &[
                &["Datum", "Datum", "Betrag"],
                &["15.01.2026", "16.01.2026", "9,99"],
            ];
            let gap: &[&[&str]] = &[
                &["Datum", "", "01.02.2026"],
                &["Name", "Erika", "Mustermann"],
            ];
            let key_value: &[&[&str]] = &[&["Datum", "15.01.2026"], &["Lieferdatum", "16.01.2026"]];
            let wide = [40.0f32, 140.0, 260.0, 380.0];
            let three_cols = [40.0f32, 160.0, 300.0];
            let two_cols = [40.0f32, 160.0];
            let cases: Vec<(&str, String, HeadWant)> = vec![
                (
                    "F: Datum: far left of the date on one baseline",
                    title.clone() + &s(40.0, 760.0, "Datum:") + &s(450.0, 760.0, "01.02.2026"),
                    one,
                ),
                (
                    "O: the value over Datum",
                    title.clone() + &s(40.0, 760.0, "01.02.2026") + &s(40.0, below, "Datum"),
                    one,
                ),
                (
                    "I: Datum over Lieferdatum over the date",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "Lieferdatum")
                        + &s(40.0, below - size * 1.2, "15.02.2026"),
                    one,
                ),
                (
                    "T1: transposed table in the head",
                    title.clone() + &ruled(transposed, &wide, 750.0),
                    three,
                ),
                (
                    "T2: transposed table in the body",
                    title.clone() + &ruled(transposed, &wide, 250.0),
                    Nothing,
                ),
                (
                    "T3: transposed table of three rows in the body",
                    title.clone() + &ruled(transposed_3, &three_cols, 250.0),
                    Nothing,
                ),
                (
                    "T4: timesheet rows in the body",
                    title.clone() + &ruled(timesheet, &wide, 250.0),
                    Nothing,
                ),
                (
                    // At 12 pt each row is one piece `Datum 15.01.2026 Stunden 8`, so both
                    // rows are marked and differ; at smaller sizes the dates are far from
                    // their labels and the head holds two plain dates.
                    "T4h: timesheet rows in the head",
                    title.clone() + &ruled(timesheet, &wide, 750.0),
                    NoDate,
                ),
                (
                    "T5: appointment table in the body",
                    title.clone() + &ruled(appointment, &two_cols, 250.0),
                    Nothing,
                ),
                (
                    "T6: item table with Datum as its first column",
                    title.clone() + &ruled(items, &three_cols, 750.0),
                    two,
                ),
                (
                    "T7: two Datum titles",
                    title.clone() + &ruled(two_titles, &three_cols, 750.0),
                    two,
                ),
                (
                    "T8: an empty cell between Datum and the date",
                    title.clone() + &ruled(gap, &three_cols, 750.0),
                    one,
                ),
                (
                    // Two dates in the value column: the Datum date is lost.
                    "T9: key and value table with Datum and Lieferdatum",
                    title.clone() + &ruled(key_value, &two_cols, 750.0),
                    two,
                ),
                (
                    "D1: weekday and den in the body",
                    title.clone() + &s(40.0, 250.0, "Bitte zahlen Sie bis Montag, den 15.02.2026"),
                    Nothing,
                ),
                (
                    "D2: weekday and den in the head",
                    title.clone() + &s(40.0, 760.0, "Zahlbar bis Montag, den 15.02.2026"),
                    one,
                ),
                (
                    "D3: an appointment with den in the body",
                    title.clone()
                        + &s(
                            40.0,
                            250.0,
                            "Ihr Termin ist am Montag, den 15.02.2026 um 10 Uhr",
                        ),
                    Nothing,
                ),
                (
                    "D4: a birth place with den",
                    title.clone()
                        + &s(
                            40.0,
                            760.0,
                            "Erika Mustermann geboren in Musterstadt, den 01.01.1970",
                        ),
                    one,
                ),
                (
                    "D5: a place with den",
                    title.clone() + &s(400.0, 760.0, "Musterstadt, den 01.02.2026"),
                    one,
                ),
                (
                    "L1: a number before the label",
                    title.clone() + &s(40.0, 760.0, "Kundennr. 4711 Datum 01.02.2026"),
                    one,
                ),
                (
                    "L2: another field's name before the label",
                    title.clone() + &s(40.0, 760.0, "Eintritt: Datum 01.01.2020"),
                    one,
                ),
                (
                    "L3: Datum over a date with text beside it",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "01.02.2026")
                        + &s(40.0 + size * 7.0, below, "Muster"),
                    one,
                ),
                (
                    "L4: a period after the label",
                    title.clone() + &s(40.0, 760.0, "Datum 01.01.2026 bis 31.01.2026"),
                    two,
                ),
            ];
            for (name, head, want) in cases {
                let out = read(&headed(&head, size, scale, ""));
                let case = format!("{name} in {size} pt {font}");
                assert_head(&out, want, &case);
                assert_eq!(
                    front(&out, "source_date").as_deref(),
                    matches!(want, Date(..)).then_some("2026-01-15"),
                    "{case}"
                );
            }
        }
    }
}

#[test]
fn rereview_r8_probes_give_no_wrong_date() {
    // The probes of the round 8 re-review (Y1, Y2 and the checks beside them) as a test. A
    // label over a column of lone dates marks none of them; a label cell marks the date cell
    // beside it only in a table of two columns. Never a wrong date.
    use HeadWant::*;
    let one = Lost(ONE_DATE);
    let three = Lost("3 dates stand in the head of page 1; none is marked as the document date");
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, y: f32, t: &str| common::show(font, size, x, y, t);
            let ruled =
                |rows: &[&[&str]], xs: &[f32], top: f32| ruled_rows(rows, xs, top, size, font);
            let pitch = size * 1.2;
            let below = 760.0 - pitch;
            let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
            let one_row: &[&[&str]] = &[&["Stunden", "8", "Datum", "15.01.2026"]];
            let row_ends_with_label: &[&[&str]] = &[
                &["Pos", "Text", "Datum", "Betrag"],
                &["1", "Muster", "Datum", "15.01.2026"],
            ];
            let with_header: &[&[&str]] = &[&["Ereignis", "Datum"], &["Datum", "01.02.2026"]];
            let repeated: &[&[&str]] = &[&["Datum", "15.01.2026"], &["Datum", "16.01.2026"]];
            let item_titles: &[&[&str]] = &[&["Leistung", "Datum"], &["Muster", "15.01.2026"]];
            let wide = [40.0f32, 140.0, 260.0, 380.0];
            let two_cols = [40.0f32, 160.0];
            // The one-row grid with small column titles of `one_row_item_grid_with_small_titles_gives_no_date`,
            // in the top half but below the top third.
            let small_titles = {
                let title_size = size * 6.6 / 9.0;
                let mut h = String::new();
                for x in [40.0f32, 140.0, 280.0, 380.0] {
                    h += &format!("0.5 w {x} 450 m {x} 510 l S\n");
                }
                for y in [450.0f32, 510.0] {
                    h += &common::hrule(40.0, 380.0, y);
                }
                for (x, t) in [(45.0f32, "Pos"), (145.0, "Datum"), (285.0, "Betrag")] {
                    h += &common::show("FL", title_size, x, 500.0, t);
                }
                for (x, t) in [(45.0f32, "1"), (145.0, "01.02.2026"), (285.0, "9,99")] {
                    h += &s(x, 460.0, t);
                }
                h
            };
            let cases: Vec<(&str, String, HeadWant)> = vec![
                (
                    "Z1: Datum over a column of lone dates in the head",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "15.01.2026")
                        + &s(40.0, below - pitch, "16.01.2026")
                        + &s(40.0, below - 2.0 * pitch, "17.01.2026"),
                    three,
                ),
                (
                    "Z1b: Datum over a column of lone dates in the body",
                    title.clone()
                        + &s(40.0, 250.0, "Datum")
                        + &s(40.0, 250.0 - pitch, "15.01.2026")
                        + &s(40.0, 250.0 - 2.0 * pitch, "16.01.2026"),
                    Nothing,
                ),
                (
                    "Z2: one ruled row Stunden | 8 | Datum | date",
                    title.clone() + &ruled(one_row, &wide, 750.0),
                    one,
                ),
                (
                    "Z3: an item row that ends with Datum | date",
                    title.clone() + &ruled(row_ends_with_label, &wide, 750.0),
                    one,
                ),
                (
                    "Z4: a two-column table with a header and a Datum row",
                    title.clone() + &ruled(with_header, &two_cols, 750.0),
                    Date("2026-02-01", "date label \"Datum\" on page 1"),
                ),
                (
                    "Z5: two Datum rows that differ",
                    title.clone() + &ruled(repeated, &two_cols, 750.0),
                    NoDate,
                ),
                (
                    "Z6: item titles Leistung | Datum over one row",
                    title.clone() + &ruled(item_titles, &two_cols, 750.0),
                    one,
                ),
                (
                    "Z7: Datum: before the date, then the page number",
                    title.clone() + &s(400.0, 760.0, "Datum: 01.02.2026 Seite 1"),
                    Date("2026-02-01", "date label \"Datum\" on page 1"),
                ),
                (
                    "Z8: Datum over one lone date, another column's cells on other baselines",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "15.01.2026")
                        + &s(300.0, 760.0, "Betrag")
                        + &s(300.0, below - pitch * 3.0, "9,99"),
                    Date("2026-01-15", "date label \"Datum\" on page 1"),
                ),
                (
                    "N2b: a one-row grid with small titles below the top third",
                    title.clone() + &small_titles,
                    Nothing,
                ),
            ];
            for (name, head, want) in cases {
                let out = read(&headed(&head, size, scale, ""));
                let case = format!("{name} in {size} pt {font}");
                assert_head(&out, want, &case);
                let date = match want {
                    Date(date, _) => Some(date),
                    _ => None,
                };
                assert_eq!(front(&out, "source_date").as_deref(), date, "{case}");
            }
        }
    }
}

#[test]
fn rereview_r9_probes_give_no_wrong_date() {
    // The probes of the round 9 re-review (Y1b with V1 to V3, Y3 with V12, and the checks
    // beside them) as a test, with lists of dates beside a label too. A label marks a date
    // only when it governs one date: no other date below the label in its column, no
    // continuation rows in a two-column table, no dates under a date beside its label. Never
    // a wrong date.
    use HeadWant::*;
    let one = Lost(ONE_DATE);
    let two = Lost(TWO_DATES);
    let three = Lost("3 dates stand in the head of page 1; none is marked as the document date");
    let by_label = Date("2026-01-15", "date label \"Datum\" on page 1");
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, y: f32, t: &str| common::show(font, size, x, y, t);
            let ruled =
                |rows: &[&[&str]], xs: &[f32], top: f32| ruled_rows(rows, xs, top, size, font);
            let pitch = size * 1.2;
            let below = 760.0 - pitch;
            let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
            let then_pos: &[&[&str]] = &[&["Datum", "15.01.2026"], &["Pos", "1"]];
            let item_table: &[&[&str]] = &[&["Datum", "Betrag"], &["15.01.2026", "9,99"]];
            let continued: &[&[&str]] = &[
                &["Datum", "15.01.2026"],
                &["", "16.01.2026"],
                &["", "17.01.2026"],
            ];
            let two_cols = [40.0f32, 160.0];
            let cases: Vec<(&str, String, HeadWant)> = vec![
                (
                    "V1: a list of entries of two lines each under Datum",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "15.01.2026")
                        + &s(40.0, below - pitch, "Muster Text")
                        + &s(40.0, below - 2.0 * pitch, "16.01.2026")
                        + &s(40.0, below - 3.0 * pitch, "Muster Text"),
                    two,
                ),
                (
                    "V2: a loosely spaced list under Datum",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "15.01.2026")
                        + &s(40.0, below - 2.6 * size, "16.01.2026")
                        + &s(40.0, below - 5.2 * size, "17.01.2026"),
                    three,
                ),
                (
                    "V3: the second date of the list indented",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "15.01.2026")
                        + &s(40.0 + size * 6.0, below - pitch, "16.01.2026"),
                    two,
                ),
                (
                    "V4: list entries of a date and text",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "15.01.2026")
                        + &s(40.0, below - pitch, "16.01.2026 Muster"),
                    two,
                ),
                (
                    "V5: two lists side by side",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "15.01.2026")
                        + &s(40.0, below - pitch, "16.01.2026")
                        + &s(300.0, 760.0, "Datum")
                        + &s(300.0, below, "15.01.2026"),
                    two,
                ),
                (
                    "V6: Datum over its date, Lieferdatum over its own below",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "01.02.2026")
                        + &s(40.0, below - pitch, "Lieferdatum")
                        + &s(40.0, below - 2.0 * pitch, "15.02.2026"),
                    two,
                ),
                (
                    "V7: Datum over its date, a date in another column one line lower",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "01.02.2026")
                        + &s(300.0, below - pitch, "15.02.2026"),
                    Date("2026-02-01", "date label \"Datum\" on page 1"),
                ),
                (
                    "V8: Datum over its date, a word below",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "15.01.2026")
                        + &s(40.0, below - pitch, "Summe"),
                    by_label,
                ),
                (
                    "V9: a column of dates offset half a size under Datum",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0 + size * 0.5, below, "15.01.2026")
                        + &s(40.0 + size * 0.5, below - pitch, "16.01.2026"),
                    two,
                ),
                (
                    "V10: a two-column table, Datum row then Pos row",
                    title.clone() + &ruled(then_pos, &two_cols, 750.0),
                    by_label,
                ),
                (
                    "V11: a two-column item table Datum | Betrag",
                    title.clone() + &ruled(item_table, &two_cols, 750.0),
                    one,
                ),
                (
                    "V12: a Datum row with continuation rows",
                    title.clone() + &ruled(continued, &two_cols, 750.0),
                    three,
                ),
                (
                    "B1: a list of dates under Datum 15.01.2026",
                    title.clone()
                        + &s(40.0, 760.0, "Datum 15.01.2026")
                        + &s(40.0 + size * 4.0, below, "16.01.2026")
                        + &s(40.0 + size * 4.0, below - pitch, "17.01.2026"),
                    three,
                ),
                (
                    "B2: Datum: and its value as pieces, a list of dates under the value",
                    title.clone()
                        + &s(40.0, 760.0, "Datum:")
                        + &s(40.0 + size * 8.0, 760.0, "15.01.2026")
                        + &s(40.0 + size * 8.0, below, "16.01.2026")
                        + &s(40.0 + size * 8.0, below - pitch, "17.01.2026"),
                    three,
                ),
            ];
            for (name, head, want) in cases {
                let out = read(&headed(&head, size, scale, ""));
                let case = format!("{name} in {size} pt {font}");
                assert_head(&out, want, &case);
                let date = match want {
                    Date(date, _) => Some(date),
                    _ => None,
                };
                assert_eq!(front(&out, "source_date").as_deref(), date, "{case}");
            }
        }
    }
}

#[test]
fn rereview_r10_probes_give_no_wrong_date() {
    // The probes of the round 10 re-review (G1 to G5, U1 to U10) as a test, and the shape of a
    // header field table over an item table whose date column lies under the header's value
    // column. A marked date must be the only date in its column on page 1, above or below it.
    // Never a wrong date. Body cases stand at y 250, under the `headed` prose.
    use HeadWant::*;
    let one = Lost(ONE_DATE);
    let two = Lost(TWO_DATES);
    let three = Lost("3 dates stand in the head of page 1; none is marked as the document date");
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, y: f32, t: &str| common::show(font, size, x, y, t);
            let ruled =
                |rows: &[&[&str]], xs: &[f32], top: f32| ruled_rows(rows, xs, top, size, font);
            let pitch = size * 1.2;
            let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
            let two_cols = [40.0f32, 160.0];
            let dashes: &[&[&str]] = &[
                &["Datum", "15.01.2026"],
                &["-", "16.01.2026"],
                &["-", "17.01.2026"],
            ];
            let one_row: &[&[&str]] = &[&["Datum", "15.01.2026"]];
            let twice: &[&[&str]] = &[
                &["Datum", "15.01.2026"],
                &["Text", "Muster"],
                &["Datum", "15.01.2026"],
            ];
            let period: &[&[&str]] = &[&["Datum", "15.01.2026"], &["Bis", "16.01.2026"]];
            let items: &[&[&str]] = &[
                &["Pos", "Datum", "Betrag"],
                &["1", "15.01.2026", "9,99"],
                &["2", "16.01.2026", "19,99"],
            ];
            // A long list entry: the label (over or beside its date), ten text lines, the next
            // date, from `top` down.
            let long_entry = |top: f32, beside: bool| {
                let mut h = title.clone();
                let mut y = top;
                if beside {
                    h += &s(40.0, y, "Datum 15.01.2026");
                } else {
                    h += &s(40.0, y, "Datum");
                    y -= pitch;
                    h += &s(40.0, y, "15.01.2026");
                }
                for _ in 0..10 {
                    y -= pitch;
                    h += &s(40.0, y, "Muster Text der Zeile");
                }
                h + &s(40.0, y - pitch, "16.01.2026")
            };
            let cases: Vec<(&str, String, HeadWant)> = vec![
                (
                    "U1: continuation rows with a dash as their label",
                    title.clone() + &ruled(dashes, &two_cols, 750.0),
                    three,
                ),
                (
                    "U2: a one-row table, lone dates under its value column",
                    title.clone()
                        + &ruled(one_row, &two_cols, 750.0)
                        + &s(164.0, 700.0, "16.01.2026")
                        + &s(164.0, 700.0 - pitch, "17.01.2026"),
                    three,
                ),
                (
                    "U3: the same Datum row twice",
                    title.clone() + &ruled(twice, &two_cols, 750.0),
                    one,
                ),
                (
                    "U4: label over its date, a long entry, the next date, in the body",
                    long_entry(250.0, false),
                    Nothing,
                ),
                (
                    "U5: label beside its date, a long entry, the next date, in the head",
                    long_entry(760.0, true),
                    two,
                ),
                (
                    "U6: label beside its date, a long entry, the next date, in the body",
                    long_entry(250.0, true),
                    Nothing,
                ),
                (
                    "U7: a date on the line over Datum 15.01.2026",
                    title.clone()
                        + &s(40.0, 760.0, "16.01.2026")
                        + &s(40.0, 760.0 - pitch, "Datum 15.01.2026"),
                    two,
                ),
                (
                    "U8: a date right aligned under the date beside its label",
                    title.clone()
                        + &s(40.0, 760.0, "Datum 15.01.2026")
                        + &s(40.0 + size * 14.0, 760.0 - pitch, "16.01.2026"),
                    two,
                ),
                (
                    "U9: a date in the next column one line under the lone date",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, 760.0 - pitch, "15.01.2026")
                        + &s(40.0 + size * 9.0, 760.0 - 2.0 * pitch, "16.01.2026"),
                    two,
                ),
                (
                    "U10: a Datum row over a Bis row",
                    title.clone() + &ruled(period, &two_cols, 750.0),
                    two,
                ),
                (
                    "H1: a header field table over an item table with a date column under its values",
                    title.clone()
                        + &label_value_rows(font, size, true)
                        + &ruled(items, &[40.0, 200.0, 320.0], 500.0),
                    one,
                ),
            ];
            for (name, head, want) in cases {
                let out = read(&headed(&head, size, scale, ""));
                let case = format!("{name} in {size} pt {font}");
                assert_head(&out, want, &case);
                assert_eq!(front(&out, "source_date"), None, "{case}");
            }
        }
    }
}

#[test]
fn rereview_r11_probes_give_no_wrong_date() {
    // The probes of the round 11 re-review (finding S, S1 to S7) as a test: a list whose other
    // entries are dates in a short or long form (a two-digit year, a month name, no year, a
    // Polish form) still makes the first entry one of a list; the CJK forms are unit tests, as
    // the test fonts hold no CJK glyphs. Never a wrong date.
    use HeadWant::*;
    let by_label = Date("2026-01-15", "date label \"Datum\" on page 1");
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, y: f32, t: &str| common::show(font, size, x, y, t);
            let ruled =
                |rows: &[&[&str]], xs: &[f32], top: f32| ruled_rows(rows, xs, top, size, font);
            let pitch = size * 1.2;
            let below = 760.0 - pitch;
            let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
            let two_cols = [40.0f32, 160.0];
            let short_years: &[&[&str]] = &[
                &["Datum", "15.01.2026"],
                &["", "16.01.26"],
                &["", "17.01.26"],
            ];
            let list = |second: &str, third: &str| {
                title.clone()
                    + &s(40.0, 760.0, "Datum")
                    + &s(40.0, below, "15.01.2026")
                    + &s(40.0, below - pitch, second)
                    + &s(40.0, below - 2.0 * pitch, third)
            };
            let cases: Vec<(&str, String, HeadWant)> = vec![
                (
                    "S1: the rest of the list with two-digit years",
                    list("16.01.26", "17.01.26"),
                    Lost(ONE_DATE),
                ),
                (
                    "S2: the rest of the list written out",
                    list("16. Januar 2026", "17. Januar 2026"),
                    Lost(ONE_DATE),
                ),
                (
                    "S3: the rest of the list without a year",
                    list("16.01.", "17.01."),
                    Lost(ONE_DATE),
                ),
                (
                    "S4: continuation rows with two-digit years",
                    title.clone() + &ruled(short_years, &two_cols, 750.0),
                    Lost(ONE_DATE),
                ),
                (
                    "S5: a date beside its label, ISO dates offset below",
                    title.clone()
                        + &s(40.0, 760.0, "Datum 15.01.2026")
                        + &s(40.0 + size * 3.0, below, "2026-01-16")
                        + &s(40.0 + size * 3.0, below - pitch, "2026-01-17"),
                    Lost(
                        "3 dates stand in the head of page 1; none is marked as the document date",
                    ),
                ),
                (
                    "S7: a lone date far below, offset 0.4 sizes",
                    title.clone()
                        + &s(40.0, 760.0, "Datum")
                        + &s(40.0, below, "15.01.2026")
                        + &s(40.0 + size * 0.4, 300.0, "16.01.2026"),
                    Lost(ONE_DATE),
                ),
                (
                    "S8: English and French forms",
                    list("January 16, 2026", "17 janvier 2026"),
                    Lost(ONE_DATE),
                ),
                (
                    "S9: Polish genitive and a slashed form without a year",
                    list("16 stycznia 2026", "17/1"),
                    Lost(ONE_DATE),
                ),
                (
                    "S11: words that are no dates below the lone date",
                    list("Muster 16", "Summe 2026"),
                    by_label,
                ),
                (
                    "S12: an amount and a time below the lone date",
                    list("1.234,56", "16:30"),
                    by_label,
                ),
            ];
            for (name, head, want) in cases {
                let out = read(&headed(&head, size, scale, ""));
                let case = format!("{name} in {size} pt {font}");
                assert_head(&out, want, &case);
                let date = match want {
                    Date(date, _) => Some(date),
                    _ => None,
                };
                assert_eq!(front(&out, "source_date").as_deref(), date, "{case}");
            }
        }
    }
}

#[test]
fn rereview_r12_probe_gives_no_wrong_date() {
    // The probe of the round 12 re-review (R4): the rest of the list writes its dates as a
    // day, a month name and a year joined by a sign (`16-Jan-2026`). The first entry is one of a
    // list, not the document date. Never a wrong date.
    for (size, scale) in [(7.0f32, 0.8f32), (9.5, 1.0), (12.0, 1.25)] {
        for font in ["FL", "FB"] {
            let s = |x: f32, y: f32, t: &str| common::show(font, size, x, y, t);
            let pitch = size * 1.2;
            let below = 760.0 - pitch;
            let title = common::show("FB", size * 1.6, 40.0, 800.0, "Musterbescheid");
            for (how, second, third) in [
                ("dashes", "16-Jan-2026", "17-Jan-2026"),
                ("slashes", "16/Jan/2026", "17/Jan/2026"),
                ("dots", "16.Jan.2026", "17.Jan.2026"),
                ("month first", "Jan-16-2026", "Jan-17-26"),
            ] {
                let head = title.clone()
                    + &s(40.0, 760.0, "Datum")
                    + &s(40.0, below, "15.01.2026")
                    + &s(40.0, below - pitch, second)
                    + &s(40.0, below - 2.0 * pitch, third);
                let out = read(&headed(&head, size, scale, ""));
                let case = format!("R4 {how} in {size} pt {font}");
                assert_head(&out, HeadWant::Lost(ONE_DATE), &case);
                assert_eq!(front(&out, "source_date"), None, "{case}");
            }
        }
    }
}
