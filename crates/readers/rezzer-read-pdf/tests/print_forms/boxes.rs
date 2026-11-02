// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Boxed fields: label and value pairs from drawn boxes, box titles and field rows.

use super::*;

/// The note before the first table of boxed fields of a page.
pub(crate) const BOX_NOTE: &str = "Found by: label and value pairs from drawn boxes; the label is the \
     smaller text at the top of each box.";

/// Three boxes in a row (touching, 120 by 24 points from x 40, 160 and 280 at y 700) with a
/// label of 6 points at the top of each, and `values` on one data line padded into the boxes
/// (an empty value leaves its box with only its label). Three lines of an address in the
/// data font elsewhere make the data font a text size of the page. `third` replaces the
/// third box's label when given (template content).
pub(crate) fn boxed_fields(values: [&str; 3], third: Option<String>) -> FormPage {
    let labels = ["Personalnummer", "Eintritt", "Steuerklasse"];
    let mut t = String::new();
    let mut line = String::new();
    for (k, x) in [40.0f32, 160.0, 280.0].into_iter().enumerate() {
        t += &boxed(x, 700.0, 120.0, 24.0);
        if k < 2 || third.is_none() {
            t += &label(x + 3.0, 716.0, 6.0, labels[k]);
        }
        if !values[k].is_empty() {
            line = format!("{line:<0$}{1}", pad_to(x + 5.0), values[k]);
        }
    }
    if let Some(third) = third {
        t += &third;
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
        ..FormPage::default()
    }
}

/// Three boxed fields in a row are one table of label and value pairs, the field without a
/// value marked empty, after one note.
#[test]
fn boxed_fields_become_pairs() {
    let out = read(&print_form(
        &[boxed_fields(["00000", "01.01.2020", ""], None)],
        "",
        "",
    ));
    let found = read_tables(&out);
    assert_eq!(
        found,
        [[
            ["Personalnummer", "00000"],
            ["Eintritt", "01.01.2020"],
            ["Steuerklasse", "empty"]
        ]],
        "{found:#?} {:?}",
        out.document.losses
    );
    assert_eq!(read_header_rows(&out), [0]);
    assert_eq!(found_by(&out), [BOX_NOTE]);
    // The empty value is a grey status, not text.
    let empty = page_blocks(&out).iter().any(|b| match b {
        Block::Table { rows, .. } => matches!(
            rows[2][1].inlines.as_slice(),
            [Inline::Status { text, color: rezzer_model::StatusColor::Grey }] if text == "empty"
        ),
        _ => false,
    });
    assert!(empty, "{:#?}", page_blocks(&out));
    assert_no_loss_but_the_head_date(&out);
}

/// A box of three styles beside boxed fields is not paired: its text stays as it is, and a
/// loss says why; the fields beside it are still pairs.
#[test]
fn box_of_three_styles_is_not_paired() {
    let third = label(283.0, 716.0, 6.0, "Steuerklasse") + &bold(283.0, 708.0, 8.0, "Hinweis");
    let out = read(&print_form(
        &[boxed_fields(["00000", "01.01.2020", "1"], Some(third))],
        "",
        "",
    ));
    let found = read_tables(&out);
    assert_eq!(
        found,
        [[["Personalnummer", "00000"], ["Eintritt", "01.01.2020"]]],
        "{found:#?}"
    );
    assert!(all_text(&out).contains("Hinweis"), "{}", all_text(&out));
    let losses: Vec<&rezzer_model::Loss> = out
        .document
        .losses
        .iter()
        .filter(|l| l.what.contains("three styles"))
        .collect();
    assert_eq!(losses.len(), 1, "{:?}", out.document.losses);
    assert_eq!(
        (losses[0].why, losses[0].severity),
        (LossReason::Uncertain, Severity::Info)
    );
    assert!(losses[0].what.contains("page 1"), "{:?}", losses[0]);
}

/// A box whose label stands below its value is not paired, with a loss that says why.
#[test]
fn box_with_its_label_below_is_not_paired() {
    let third = label(283.0, 701.0, 6.0, "Steuerklasse");
    let mut page = boxed_fields(["00000", "01.01.2020", ""], Some(third));
    page.lines.push(DataLine {
        y: 712.0,
        text: padded(pad_to(285.0), "1"),
    });
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(
        found,
        [[["Personalnummer", "00000"], ["Eintritt", "01.01.2020"]]],
        "{found:#?}"
    );
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what.contains("its label stands below its value")),
        "{:?}",
        out.document.losses
    );
}

/// Boxes whose bold value stands over a regular label of the same size: weight alone does not
/// tell which line is the label, so no box pairs (the value is never read as the label of the
/// label, ruling P6-X), and a loss says why.
#[test]
fn bold_value_over_a_regular_label_of_its_size_is_not_paired() {
    let mut t = String::new();
    for (k, (l, v)) in [("Name", "Erika"), ("Ort", "Musterstadt"), ("Nr", "12")]
        .iter()
        .enumerate()
    {
        let x = 40.0 + 120.0 * k as f32;
        t += &boxed(x, 700.0, 120.0, 24.0);
        t += &bold(x + 3.0, 712.0, 8.0, v);
        t += &common::show("FL", 8.0, x + 3.0, 703.0, l);
    }
    let lines = ["Muster GmbH", "Musterweg 1", "12345 Musterstadt"]
        .iter()
        .enumerate()
        .map(|(k, l)| DataLine {
            y: 600.0 - 12.0 * k as f32,
            text: (*l).into(),
        })
        .collect();
    let page = FormPage {
        template: t,
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert!(found.is_empty(), "{found:#?}");
    assert!(
        out.document
            .losses
            .iter()
            .any(|l| l.what.contains("weight") && l.what.contains("page 1")),
        "{:?}",
        out.document.losses
    );
}

/// Boxed fields one under the other (a column of boxes) are one table too.
#[test]
fn boxed_fields_in_a_column_become_pairs() {
    let mut t = String::new();
    let mut lines = Vec::new();
    for (k, (l, v)) in [("Name", "Erika Mustermann"), ("Ort", "Musterstadt")]
        .iter()
        .enumerate()
    {
        let y = 700.0 - 24.0 * k as f32;
        t += &boxed(40.0, y, 200.0, 24.0);
        t += &label(43.0, y + 16.0, 6.0, l);
        lines.push(DataLine {
            y: y + 3.0,
            text: padded(1, v),
        });
    }
    for (k, l) in ["Muster GmbH", "Musterweg 1", "12345 Musterstadt"]
        .iter()
        .enumerate()
    {
        lines.push(DataLine {
            y: 600.0 - 12.0 * k as f32,
            text: (*l).into(),
        });
    }
    let page = FormPage {
        template: t,
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(
        found,
        [[["Name", "Erika Mustermann"], ["Ort", "Musterstadt"]]],
        "{found:#?}"
    );
    assert_eq!(found_by(&out), [BOX_NOTE]);
}

/// Guard: a box that holds a form field is the form's, not a field pair (it stays as today).
#[test]
fn widget_box_is_not_paired() {
    let mut page = boxed_fields(["00000", "01.01.2020", ""], None);
    page.annots
        .push("<< /Type /Annot /Subtype /Widget /FT /Tx /T (pnr) /Rect [42 701 158 712] >>".into());
    let out = read(&print_form(
        &[page],
        "",
        "/AcroForm << /Fields [{annots}] >>",
    ));
    assert!(read_tables(&out).is_empty(), "{:#?}", read_tables(&out));
    assert!(found_by(&out).is_empty(), "{:?}", found_by(&out));
    assert!(
        all_text(&out).contains("Personalnummer"),
        "{}",
        all_text(&out)
    );
}

/// The header field grid of a pay slip (label rows over value rows, dividers that hang from
/// the rule above, a raised footnote marker in a label, a date line in a part of its own to
/// the right) reads as label and value pairs, row by row; an empty field is marked empty
/// (ruling P6-H).
#[test]
fn header_field_grid_reads_as_pairs() {
    let (template, lines) = header_field_grid();
    let page = FormPage {
        template,
        left: 42.52,
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:#?}");
    let rows: Vec<String> = found[0].iter().map(|r| r.join(": ")).collect();
    assert_eq!(
        rows,
        [
            "Personalnummer: 00000",
            "Eintritt: 01.01.2020",
            "Steuerklasse 1: 1",
            "Konfession: empty",
            "Datum: 01.10.2026",
            "Geburtsdatum: 01.01.1970",
            "Austritt: empty",
            "Kinder: 0,5",
            "Kostenstelle: 4711",
            "Krankenkasse: Muster BKK",
            "Beitragsgruppe: 1111",
            "Abteilung: Muster",
            "Freibetrag: 1.234,56"
        ],
        "{found:#?}"
    );
    let notes = found_by(&out);
    assert_eq!(
        notes
            .iter()
            .filter(|n| n.contains("rows of fields"))
            .count(),
        1,
        "{notes:?}"
    );
    assert_no_loss(&out);
    assert_grid_date(&out);
}

/// Rows of fields as a print run fills them (ruling P6-X): each row a rule over 6.6 point
/// labels and one data line, dividers that hang 11.53 points from the rule above and end over
/// the value line. Row 1: three short values one or two spaces apart, each in its own field;
/// row 2: an ID and a name in two fields; row 3: a value that runs over the divider without a
/// word gap there; row 4: two labels without a divider, a date under the first; row 5: an ID
/// and a two-word name whose word gap lies left of the divider; row 6: two fields with a
/// divider, a date in the first and the second empty. All values are invented.
pub(crate) fn field_rows() -> FormPage {
    // Each row: the fields' left edges (the first is the frame's) with their labels, the
    // data line as (column, text) parts, and whether its fields have dividers.
    type Row<'a> = (&'a [(f32, &'a str)], &'a [(usize, &'a str)], bool);
    let rows: [Row<'_>; 6] = [
        (
            &[(42.40, "Pers.-Nr."), (76.72, "Eintritt"), (133.0, "Stkl")],
            &[(0, "00123"), (7, "01.01.80"), (16, "1")],
            true,
        ),
        (
            &[(42.40, "Kasse-Nr."), (115.0, "Krankenkasse")],
            &[(0, "00000000X000"), (13, "Muster Kasse")],
            true,
        ),
        (
            &[(42.40, "Betrag"), (90.0, "Zuschlag")],
            &[(0, "123456789012")],
            true,
        ),
        (
            &[(42.40, "Eintritt"), (120.0, "Austritt")],
            &[(0, "01.04.2020")],
            false,
        ),
        (
            &[(42.40, "SV-Nummer"), (119.5, "Krankenkasse")],
            &[(0, "00000000X000"), (13, "Muster Kasse")],
            true,
        ),
        (
            &[(42.40, "Eintritt"), (110.0, "Austritt")],
            &[(0, "01.04.2020")],
            true,
        ),
    ];
    let mut t = String::new();
    let mut lines = Vec::new();
    let top0 = 760.0f32;
    for k in 0..=rows.len() {
        t += &bar_h(42.40, 200.0, top0 - 22.8 * k as f32, 0.24);
    }
    for (k, (fields, values, dividers)) in rows.iter().enumerate() {
        let top = top0 - 22.8 * k as f32;
        for (n, (x, l)) in fields.iter().enumerate() {
            if n > 0 && *dividers {
                t += &bar_v(*x, top - 11.53, top, 0.24);
            }
            t += &label(x + 1.5, top - 8.17, 6.6, l);
        }
        lines.push(columns_line(top - 21.17, values));
    }
    // Text in the data font elsewhere, so it is a text size of the page.
    for (k, l) in ["Muster GmbH", "Musterweg 1", "12345 Musterstadt"]
        .iter()
        .enumerate()
    {
        lines.push(columns_line(600.0 - 12.0 * k as f32, &[(0, l)]));
    }
    FormPage {
        template: t,
        left: 42.52,
        lines,
        ..FormPage::default()
    }
}

/// A value line over several fields is cut at the word gap nearest each divider, each value
/// paired with the field it stands in; a row whose value runs over a divider is not paired,
/// and a loss names it; labels alone part the fields of a row without dividers (ruling P6-X:
/// never a wrong pair).
#[test]
fn values_over_several_fields_pair_with_their_own_fields() {
    let out = read(&print_form(&[field_rows()], "", ""));
    let found = read_tables(&out);
    let pairs: Vec<Vec<String>> = found
        .iter()
        .map(|t| t.iter().map(|r| r.join(": ")).collect())
        .collect();
    assert_eq!(
        pairs,
        [
            vec![
                "Pers.-Nr.: 00123",
                "Eintritt: 01.01.80",
                "Stkl: 1",
                "Kasse-Nr.: 00000000X000",
                "Krankenkasse: Muster Kasse"
            ],
            vec![
                "Eintritt: 01.04.2020",
                "Austritt: empty",
                "SV-Nummer: 00000000X000",
                "Krankenkasse: Muster Kasse",
                "Eintritt: 01.04.2020",
                "Austritt: empty"
            ]
        ],
        "{found:#?}"
    );
    let text = all_text(&out);
    assert!(text.contains("123456789012"), "{text}");
    // The `Eintritt` date in the head gives its own loss.
    let losses: Vec<&str> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Uncertain && l.severity == Severity::Info)
        .map(|l| l.what.as_str())
        .filter(|w| *w != ONE_DATE)
        .collect();
    assert_eq!(losses.len(), 1, "{:?}", out.document.losses);
    assert!(
        losses[0].contains("Betrag, Zuschlag") && losses[0].contains("page 1"),
        "{losses:?}"
    );
}

/// A bold title alone in a box beside boxed fields is the box's title, not the label of an
/// empty field: it is no pair but a heading, and the text under it follows it.
#[test]
fn bold_box_title_is_no_empty_field() {
    let third = bold(283.0, 712.0, 6.0, "Hinweise zur Abrechnung");
    let mut page = boxed_fields(["00000", "01.01.2020", ""], Some(third));
    page.template += &label(283.0, 690.0, 6.0, "Bitte pruefen Sie alle Angaben.");
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(
        found,
        [[["Personalnummer", "00000"], ["Eintritt", "01.01.2020"]]],
        "{found:#?}"
    );
    let text = all_text(&out);
    assert!(
        in_order(
            &text,
            &["Hinweise zur Abrechnung", "Bitte pruefen Sie alle Angaben."]
        )
        .is_ok(),
        "{text}"
    );
    let title = page_blocks(&out).iter().position(|b| {
        matches!(b, Block::Heading { inlines, .. } if plain_text(inlines) == "Hinweise zur Abrechnung")
    });
    let text_at = page_blocks(&out).iter().position(|b| {
        matches!(b, Block::Paragraph { inlines, .. } if plain_text(inlines) == "Bitte pruefen Sie alle Angaben.")
    });
    assert!(
        title.zip(text_at).is_some_and(|(h, p)| h < p),
        "{:#?}",
        page_blocks(&out)
    );
    assert_no_loss_but_the_head_date(&out);
}

/// A framed box of a pay slip with a bold title of 6.66 points at its top left and two lines
/// of the data font in it (a date and an amount); `title_box` draws the title in a box of its
/// own inside the frame's top, so the frame holds a box with text and is no region itself:
/// its lines are read later, in the page's order. Beside it, boxed fields and text elsewhere
/// on the page.
pub(crate) fn titled_box(title_box: bool) -> FormPage {
    let mut page = boxed_fields(["00000", "01.01.2020", "1"], None);
    page.template += &boxed(40.0, 420.0, 360.0, 60.0);
    if title_box {
        page.template += &boxed(43.5, 463.5, 120.0, 13.0);
    }
    page.template += &bold(46.0, 468.0, 6.66, "Hinweise zur Abrechnung");
    for (k, l) in ["Abschlag vom 01.10.2026 ueberwiesen", "Restbetrag 1.234,56"]
        .iter()
        .enumerate()
    {
        page.lines.push(DataLine {
            y: 450.0 - 12.0 * k as f32,
            text: padded(1, l),
        });
    }
    page
}

/// A framed box with a bold title at its top and lines of text under it in the frame reads
/// as the title as a heading and then its lines, found by the frame, not by the order the
/// text comes in; it is no label with a value.
#[test]
fn framed_box_title_heads_its_lines() {
    for title_box in [false, true] {
        let out = read(&print_form(&[titled_box(title_box)], "", ""));
        let blocks = page_blocks(&out);
        let at = blocks.iter().position(|b| {
            matches!(b, Block::Heading { inlines, .. } if plain_text(inlines) == "Hinweise zur Abrechnung")
        });
        let text: Vec<String> = at
            .map(|k| {
                blocks[k + 1..]
                    .iter()
                    .take(2)
                    .map(|b| match b {
                        Block::Paragraph { inlines, .. } => plain_text(inlines),
                        _ => String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            at.is_some()
                && text
                    .join(" ")
                    .contains("Abschlag vom 01.10.2026 ueberwiesen")
                && text.join(" ").contains("Restbetrag 1.234,56"),
            "title box {title_box}: {blocks:#?}"
        );
        assert!(
            read_tables(&out)
                .iter()
                .flatten()
                .all(|r| !r.iter().any(|c| c.contains("Hinweise"))),
            "{:#?}",
            read_tables(&out)
        );
    }
}

/// A header band of four drawn cells, and under it value lines padded into its columns with
/// one or two spaces between two fields, each line one run of the data font: each line is cut
/// at the band's column edges where a word gap lies there, and the lines are the band's rows.
#[test]
fn padded_value_lines_under_a_band_are_cut_at_its_columns() {
    // Courier at 9.5 pt is 5.7 points a character; the band's edges fall in the spaces after
    // characters 4, 13 and 17 of the lines.
    let ch = 0.6 * 9.5;
    let edges = [
        36.0,
        40.0 + 4.5 * ch,
        40.0 + 13.5 * ch,
        40.0 + 18.0 * ch,
        40.0 + 28.0 * ch,
    ];
    let mut template = String::new();
    for w in edges.windows(2) {
        template += &boxed(w[0], 700.0, w[1] - w[0], 12.0);
    }
    for (x, t) in edges.iter().zip(["Nr", "Text", "Mg", "Betrag"]) {
        template += &label(x + 2.0, 703.0, 6.0, t);
    }
    let lines = [
        "1000 Gehalt     2  1.111,11",
        "2000 Zulage     1     22,22",
        "3000 Bonus      1    333,33",
    ];
    let page = FormPage {
        template,
        lines: lines
            .iter()
            .enumerate()
            .map(|(k, t)| DataLine {
                y: 688.0 - 11.4 * k as f32,
                text: (*t).to_string(),
            })
            .collect(),
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let want: Vec<Vec<String>> = [
        ["Nr", "Text", "Mg", "Betrag"],
        ["1000", "Gehalt", "2", "1.111,11"],
        ["2000", "Zulage", "1", "22,22"],
        ["3000", "Bonus", "1", "333,33"],
    ]
    .iter()
    .map(|r| r.iter().map(|c| (*c).to_string()).collect())
    .collect();
    assert_eq!(
        read_tables(&out),
        [want],
        "{}\n{:?}",
        all_text(&out),
        out.document.losses
    );
}
