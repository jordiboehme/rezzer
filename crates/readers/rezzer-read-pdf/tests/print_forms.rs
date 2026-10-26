// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Print-run forms: a form template with a data layer of padded fixed-pitch lines, built in
//! code with invented values only.

mod common;

use std::time::{Duration, Instant};

use common::{DataLine, FormPage, all_text, boxed, data_line, label, mark, print_form};
use rezzer_model::{Block, Completeness, LossReason, ReadOutcome, Severity, plain_text};

/// The allocator of the Rezzer binary, so the reads run as they run in it.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

/// `bytes` read with the default options and limits. Every read of this file also checks
/// that no placeholder of Rezzer reaches the Markdown or the JSON.
fn read(bytes: &[u8]) -> ReadOutcome {
    let out = common::read(bytes);
    assert_no_placeholder(&out);
    out
}

/// Asserts that neither the rendered Markdown nor the JSON of the model of `out` holds a
/// noncharacter of U+FDD0 to U+FDEF: those are Rezzer's own placeholders while a page is read.
fn assert_no_placeholder(out: &ReadOutcome) {
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    // JSON writes the noncharacters as they are, not escaped.
    let json = serde_json::to_string(&out.document).expect("json");
    let placeholder = |c: char| ('\u{FDD0}'..='\u{FDEF}').contains(&c);
    assert!(
        !md.contains(placeholder),
        "a placeholder in the Markdown: {md}"
    );
    assert!(!json.contains(placeholder), "a placeholder in the JSON");
}

/// The read is complete and names no loss.
fn assert_no_loss(out: &ReadOutcome) {
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
    assert!(out.document.losses.is_empty(), "{:?}", out.document.losses);
}

#[test]
fn cm_placed_identical_lines_are_kept() {
    let page = FormPage {
        template: label(40.0, 760.0, 6.0, "Muster GmbH Abrechnung"),
        lines: vec![
            DataLine {
                y: 700.0,
                text: "      0,00".into(),
            },
            DataLine {
                y: 680.0,
                text: "      0,00".into(),
            },
        ],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert_eq!(text.matches("0,00").count(), 2, "{text}");
    assert_no_loss(&out);
}

/// Guard: a fill-then-stroke show (render mode 2) inside a `cm` block is still one text.
#[test]
fn fill_and_stroke_show_reads_once() {
    let page = FormPage {
        template: "q 1 0 0 1 40 700 cm BT /FB 12 Tf 2 Tr 0 0 Td (Erika Mustermann) Tj ET Q\n"
            .into(),
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert_eq!(text.matches("Erika Mustermann").count(), 1, "{text}");
    assert_no_loss(&out);
}

/// Guard: one data line drawn twice 0.3 points apart (a fake bold) is read once.
#[test]
fn fake_bold_line_reads_once() {
    let line = "Auszahlung      1.234,56";
    let page = FormPage {
        template: format!(
            "{}{}",
            data_line(40.0, 700.0, 9.5, line),
            data_line(40.3, 700.0, 9.5, line)
        ),
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert_eq!(text.matches("1.234,56").count(), 1, "{text}");
    assert_eq!(text.matches("Auszahlung").count(), 1, "{text}");
    assert_no_loss(&out);
}

/// The builder writes text in WinAnsi, so a German label reads back as written.
#[test]
fn label_with_umlauts_reads_back() {
    let words = "Bez\u{fc}ge \u{c4}rger \u{d6}l \u{dc}bung Gr\u{f6}\u{df}e Stra\u{df}e";
    let page = FormPage {
        template: label(40.0, 760.0, 8.0, words),
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(text.contains(words), "{text}");
    assert_no_loss(&out);
}

/// The rules of a grid between the column edges `xs` and the row edges `ys`.
fn grid(xs: &[f32], ys: &[f32]) -> String {
    let (x0, x1) = (xs[0], xs[xs.len() - 1]);
    let (y0, y1) = (ys[0], ys[ys.len() - 1]);
    let mut c = String::from("0.5 w ");
    for y in ys {
        c += &format!("{x0} {y} m {x1} {y} l S ");
    }
    for x in xs {
        c += &format!("{x} {y0} m {x} {y1} l S ");
    }
    c.push('\n');
    c
}

/// The read's one loss, which is an `Uncertain` Info loss, and the rendered frontmatter
/// counts it.
fn one_uncertain_loss(out: &ReadOutcome) -> String {
    let losses = &out.document.losses;
    assert_eq!(losses.len(), 1, "{losses:?}");
    assert_eq!(losses[0].why, LossReason::Uncertain, "{losses:?}");
    assert_eq!(losses[0].severity, Severity::Info, "{losses:?}");
    assert_eq!(out.completeness, Completeness::Complete, "{losses:?}");
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    assert!(md.contains("\nlosses: 1\n"), "{md}");
    losses[0].what.clone()
}

/// A grid of three columns with text in every cell and a figure caption under it: a chart,
/// and no loss says it may be a table.
#[test]
fn chart_grid_gives_no_loss() {
    let mut template = grid(&[40.0, 140.0, 240.0, 340.0], &[640.0, 660.0, 680.0, 700.0]);
    for (r, y) in [686.0, 666.0, 646.0].into_iter().enumerate() {
        for (c, x) in [44.0, 144.0, 244.0].into_iter().enumerate() {
            template += &label(x, y, 9.0, &format!("{},{}0", r + 1, c + 1));
        }
    }
    template += &label(40.0, 625.0, 9.0, "Figure 1: Umsatz der Muster GmbH");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert_no_loss(&out);
}

/// A grid of two columns by two rows over most of the page, with text in three of its
/// squares: refused as the page's frame without a note, and that refusal is a loss.
#[test]
fn page_frame_grid_refusal_is_a_loss() {
    let mut template = grid(&[30.0, 297.0, 565.0], &[60.0, 410.0, 760.0]);
    template += &label(40.0, 740.0, 10.0, "Muster GmbH");
    template += &label(307.0, 740.0, 10.0, "Erika Mustermann");
    template += &label(40.0, 390.0, 10.0, "1.234,56");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let what = one_uncertain_loss(&out);
    assert_eq!(
        what,
        "a grid of drawn rules on page 1 is not read as a table: it covers most of the page; \
         its text is read without the grid"
    );
    let text = all_text(&out);
    for t in ["Muster GmbH", "Erika Mustermann", "1.234,56"] {
        assert!(text.contains(t), "{text}");
    }
    assert!(!text.contains('|'), "{text}");
}

/// A header band of three titles at 10 pt with three rows at 4 pt below it, and 12 lines of
/// 10 pt prose that make 10 pt the body size: the band is refused for its tiny text.
fn tiny_band() -> String {
    let mut template = grid(&[40.0, 200.0, 360.0, 520.0], &[700.0, 715.0]);
    for (x, t) in [(44.0, "Name"), (204.0, "Ort"), (364.0, "Betrag")] {
        template += &label(x, 704.0, 10.0, t);
    }
    for (k, y) in [692.0, 686.0, 680.0].into_iter().enumerate() {
        template += &label(44.0, y, 4.0, &format!("Erika Mustermann {k}"));
        template += &label(204.0, y, 4.0, "Musterstadt");
        template += &label(364.0, y, 4.0, "1.234,56");
    }
    for k in 0..12 {
        let y = 500.0 - 14.0 * k as f32;
        template += &label(
            40.0,
            y,
            10.0,
            "Die Muster GmbH zahlt den Betrag zum Monatsende aus.",
        );
    }
    template
}

/// A header band whose rows are set in text much smaller than the page's body text: refused
/// without a note, and that refusal is a loss.
#[test]
fn band_refused_for_tiny_text_is_a_loss() {
    let template = tiny_band();
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let what = one_uncertain_loss(&out);
    assert_eq!(
        what,
        "a header band on page 1 is not read as a table: its text is much smaller than the \
         body text, as in a picture of a page; its text is read without the band"
    );
    let text = all_text(&out);
    assert!(text.contains("Erika Mustermann 2"), "{text}");
    assert!(!text.contains('|'), "{text}");
}

/// Guard: a grid refused with a note (many columns, text in three rows) gives its one loss,
/// not a second one for the same refusal.
#[test]
fn noted_refusal_is_not_counted_twice() {
    let xs: Vec<f32> = (0..=20).map(|k| 40.0 + 25.0 * k as f32).collect();
    let mut template = grid(&xs, &[640.0, 660.0, 680.0, 700.0]);
    for (r, y) in [686.0, 666.0, 646.0].into_iter().enumerate() {
        for k in 0..20 {
            template += &label(43.0 + 25.0 * k as f32, y, 7.0, &format!("{}{k}", r + 1));
        }
    }
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let what = one_uncertain_loss(&out);
    assert_eq!(
        what,
        "Possible table on page 1 not rebuilt (reason: it has more than 15 columns)"
    );
}

/// Guard: a table without rules inside a grid that frames the page is still read, while the
/// frame's refusal is a loss.
#[test]
fn table_inside_a_page_frame_is_read() {
    let mut template = grid(&[30.0, 297.0, 565.0], &[60.0, 410.0, 760.0]);
    let rows = [
        ["Name", "Ort", "Betrag"],
        ["Erika", "Berlin", "1.234,56"],
        ["Max", "Bonn", "234,56"],
        ["Anna", "Essen", "34,56"],
        ["Paul", "Kiel", "4,56"],
    ];
    for (r, row) in rows.iter().enumerate() {
        let y = 730.0 - 14.0 * r as f32;
        for (c, t) in row.iter().enumerate() {
            template += &label(40.0 + 85.0 * c as f32, y, 9.0, t);
        }
    }
    template += &label(307.0, 740.0, 9.0, "Muster GmbH");
    template += &label(40.0, 390.0, 9.0, "Musterstadt");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(text.contains("Erika|Berlin|1.234,56|"), "{text}");
    let what = one_uncertain_loss(&out);
    assert!(what.contains("it covers most of the page"), "{what}");
}

/// Guard: a table without rules in one box of a grid of empty boxes is still read, while the
/// grid's refusal is a loss.
#[test]
fn table_inside_a_box_grid_is_read() {
    let mut template = grid(&[30.0, 210.0, 390.0, 565.0], &[400.0, 580.0, 760.0]);
    let rows = [
        ["Name", "Ort", "Betrag"],
        ["Erika", "Kiel", "34,56"],
        ["Max", "Bonn", "4,56"],
        ["Anna", "Essen", "1,23"],
        ["Paul", "Ulm", "0,99"],
    ];
    for (r, row) in rows.iter().enumerate() {
        let y = 730.0 - 14.0 * r as f32;
        for (c, t) in row.iter().enumerate() {
            template += &label(40.0 + 55.0 * c as f32, y, 9.0, t);
        }
    }
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(text.contains("Erika|Kiel|34,56|"), "{text}");
    let what = one_uncertain_loss(&out);
    assert!(what.contains("most of its cells are empty"), "{what}");
}

/// Two grids of boxes with text in one box each: one loss for the page says how many and why.
#[test]
fn quiet_grids_of_a_page_give_one_loss() {
    let mut template = String::new();
    for (top, t) in [(760.0, "Muster GmbH"), (560.0, "Erika Mustermann")] {
        template += &grid(&[40.0, 200.0, 360.0, 520.0], &[top - 80.0, top - 40.0, top]);
        template += &label(44.0, top - 20.0, 9.0, t);
    }
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let what = one_uncertain_loss(&out);
    assert_eq!(
        what,
        "2 grids of drawn rules on page 1 are not read as tables: most of their cells are \
         empty; their text is read without the grids"
    );
    let text = all_text(&out);
    assert!(text.contains("Erika Mustermann"), "{text}");
}

/// A gray picture of `w` by `h` points with its lower left corner at `(x, y)`, drawn inline.
fn picture(x: f32, y: f32, w: f32, h: f32) -> String {
    format!(
        "q {w} 0 0 {h} {x} {y} cm BI /W 8 /H 8 /CS /G /BPC 8 ID {} EI Q\n",
        "A".repeat(64)
    )
}

/// The losses of a read that speak of tables.
fn table_losses(out: &ReadOutcome) -> Vec<&str> {
    out.document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .filter(|w| w.contains("table"))
        .collect()
}

/// A chart with twenty columns of grid lines, labels in one row and a figure caption: refused
/// for its many columns, and no loss, as the caption names a figure.
#[test]
fn wide_chart_grid_gives_no_loss() {
    let xs: Vec<f32> = (0..=20).map(|k| 40.0 + 25.0 * k as f32).collect();
    let mut template = grid(&xs, &[640.0, 660.0, 680.0, 700.0]);
    for k in 0..5 {
        template += &label(43.0 + 100.0 * k as f32, 646.0, 7.0, &format!("{}0", k + 1));
    }
    template += &label(40.0, 625.0, 9.0, "Figure 2: Stunden je Woche");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert_no_loss(&out);
}

/// A grid with text in two of its nine squares over a picture: refused for its empty cells,
/// and no loss, as a grid on a picture is a chart's.
#[test]
fn sparse_grid_on_a_picture_gives_no_loss() {
    let mut template = picture(40.0, 500.0, 300.0, 200.0);
    template += &grid(&[40.0, 140.0, 240.0, 340.0], &[520.0, 580.0, 640.0, 700.0]);
    template += &label(44.0, 680.0, 9.0, "100");
    template += &label(144.0, 680.0, 9.0, "200");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(table_losses(&out).is_empty(), "{:?}", out.document.losses);
}

/// Guard: the same grid without the picture gives its loss, so the picture is what keeps the
/// loss away.
#[test]
fn sparse_grid_without_a_picture_is_a_loss() {
    let mut template = grid(&[40.0, 140.0, 240.0, 340.0], &[520.0, 580.0, 640.0, 700.0]);
    template += &label(44.0, 680.0, 9.0, "100");
    template += &label(144.0, 680.0, 9.0, "200");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let what = one_uncertain_loss(&out);
    assert!(what.contains("most of its cells are empty"), "{what}");
}

/// A header band refused for its tiny text over a picture: no loss, as the band is a chart's.
#[test]
fn band_on_a_picture_gives_no_loss() {
    let template = picture(30.0, 660.0, 500.0, 70.0) + &tiny_band();
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(table_losses(&out).is_empty(), "{:?}", out.document.losses);
}

/// A page with a grid and a header band refused without a note: one loss for each, the
/// grid's first.
#[test]
fn quiet_grid_and_band_of_a_page_give_two_losses() {
    let mut template = tiny_band();
    template += &grid(&[40.0, 200.0, 360.0, 520.0], &[220.0, 260.0, 300.0]);
    template += &label(44.0, 280.0, 9.0, "Muster GmbH");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let losses: Vec<&str> = out
        .document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .collect();
    assert_eq!(
        losses,
        [
            "a grid of drawn rules on page 1 is not read as a table: most of its cells are \
             empty; its text is read without the grid",
            "a header band on page 1 is not read as a table: its text is much smaller than \
             the body text, as in a picture of a page; its text is read without the band",
        ],
        "{:?}",
        out.document.losses
    );
    assert!(
        out.document
            .losses
            .iter()
            .all(|l| l.why == LossReason::Uncertain && l.severity == Severity::Info)
    );
}

/// `n` spaces of the data font, then `text`: a value the print run pads into its place.
fn padded(n: usize, text: &str) -> String {
    format!("{}{text}", " ".repeat(n))
}

/// Two boxes side by side with a label each; the data lines start at the left edge of the
/// page and are padded into the boxes. Each value is read in its own box, after its label,
/// left box first.
#[test]
fn padded_values_stand_in_their_box() {
    let mut template = boxed(40.0, 620.0, 250.0, 90.0) + &boxed(300.0, 620.0, 255.0, 90.0);
    template += &label(45.0, 698.0, 6.0, "Arbeitgeber");
    template += &label(305.0, 698.0, 6.0, "Arbeitnehmer");
    // Data lines from x 40 in Courier 9.5 (5.7 points a character): 47 spaces reach x 307.9,
    // inside the right box.
    let lines = vec![
        DataLine {
            y: 680.0,
            text: format!("  Muster GmbH{}", padded(34, "Erika Mustermann")),
        },
        DataLine {
            y: 665.0,
            text: padded(2, "Musterweg 1"),
        },
        DataLine {
            y: 650.0,
            text: padded(47, "Beispielstr. 2"),
        },
        DataLine {
            y: 635.0,
            text: padded(47, "12345 Musterstadt"),
        },
    ];
    let page = FormPage {
        template,
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    let at = |t: &str| text.find(t).unwrap_or_else(|| panic!("no {t:?} in {text}"));
    let order = [
        "Arbeitgeber",
        "Muster GmbH",
        "Musterweg 1",
        "Arbeitnehmer",
        "Erika Mustermann",
        "Beispielstr. 2",
        "12345 Musterstadt",
    ];
    for w in order.windows(2) {
        assert!(
            at(w[0]) < at(w[1]),
            "{:?} before {:?} in {text}",
            w[0],
            w[1]
        );
    }
    assert!(!text.contains("  "), "{text:?}");
}

/// A template label and a data line padded onto its baseline read as one line, label first.
/// One row only, so the column search does not run: the rows of a label column are
/// `padded_values_beside_long_labels_read_row_by_row`.
#[test]
fn label_then_value_on_one_row() {
    let page = FormPage {
        template: label(60.0, 600.0, 8.0, "Auszahlungsbetrag"),
        lines: vec![DataLine {
            y: 600.0,
            text: padded(30, "1.234,56"),
        }],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(
        text.lines().any(|l| l == "Auszahlungsbetrag 1.234,56"),
        "{text:?}"
    );
}

/// Whether each of `order` stands in `text`, each after the one before it; else which one
/// does not.
fn in_order(text: &str, order: &[&str]) -> Result<(), String> {
    let mut from = 0;
    for t in order {
        let at = text[from..]
            .find(t)
            .ok_or_else(|| format!("no {t:?} after byte {from} in {text}"))?;
        from += at + t.len();
    }
    Ok(())
}

/// A column of long labels with their values on the same baselines, the values padded from
/// the left margin far to the right (as a tax notice prints them): read row by row, each
/// label with its value. Padding is no text, so the values do not read as a column of prose.
#[test]
fn padded_values_beside_long_labels_read_row_by_row() {
    let rows = [
        ("Zu versteuerndes Einkommen", "12.345"),
        ("Festgesetzte Einkommensteuer", "1.234,56"),
        ("Solidaritaetszuschlag", "0,00"),
        ("Kirchensteuer evangelisch", "123,45"),
        ("Anzurechnende Lohnsteuer", "1.000,00"),
    ];
    let order: Vec<&str> = rows.iter().flat_map(|(l, v)| [*l, *v]).collect();
    // Labels left of the data margin and right of it, and values padded at the end to a fixed
    // record width too.
    let mut failed = Vec::new();
    for (x, width) in [(30.0, 0), (45.0, 0), (30.0, 90)] {
        let mut template = String::new();
        let mut lines = Vec::new();
        for (k, (l, v)) in rows.iter().enumerate() {
            let y = 600.0 - 14.0 * k as f32;
            template += &label(x, y, 8.0, l);
            lines.push(DataLine {
                y,
                text: format!("{:<width$}", padded(50, v)),
            });
        }
        let page = FormPage {
            template,
            lines,
            ..FormPage::default()
        };
        let out = read(&print_form(&[page], "", ""));
        if let Err(e) = in_order(&all_text(&out), &order) {
            failed.push(format!("labels at x {x}, width {width}: {e}"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The fields of a header grid in the right half of the page: (x, label) for each field of
/// each grid row, with its value.
const HEADER_FIELDS: [[(f32, &str, &str); 3]; 2] = [
    [
        (320.0, "Personalnr", "00001"),
        (400.0, "Eintritt", "01.04.2020"),
        (480.0, "Kostenstelle", "4711"),
    ],
    [
        (320.0, "Geburtsdatum", "01.01.1970"),
        (400.0, "Austritt", "31.12.2030"),
        (480.0, "Abteilung", "Muster"),
    ],
];

/// The spaces of the data font from the data margin to `x`.
fn pad_to(x: f32) -> usize {
    ((x - 40.0) / 5.7).round() as usize
}

/// A header grid read row by row: the labels of a grid row, then its values, left to right;
/// `one_line` prints the values of a grid row as one data line, else one line per field.
fn header_grid(one_line: bool) {
    let mut template = String::new();
    let mut lines = Vec::new();
    for (row, y) in HEADER_FIELDS.iter().zip([700.0f32, 676.0]) {
        let mut line = String::new();
        for &(x, l, v) in row {
            template += &label(x, y + 4.0, 6.0, l);
            if one_line {
                line = format!("{line:<0$}{v}", pad_to(x));
            } else {
                lines.push(DataLine {
                    y: y - 6.0,
                    text: padded(pad_to(x), v),
                });
            }
        }
        if one_line {
            lines.push(DataLine {
                y: y - 6.0,
                text: line,
            });
        }
    }
    let page = FormPage {
        template,
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let order: Vec<&str> = HEADER_FIELDS
        .iter()
        .flat_map(|row| {
            row.iter()
                .map(|f| f.1)
                .chain(row.iter().map(|f| f.2))
                .collect::<Vec<_>>()
        })
        .collect();
    if let Err(e) = in_order(&all_text(&out), &order) {
        panic!("{e}");
    }
}

/// A header field grid whose values are padded from the left margin, one data line per field:
/// read grid row by grid row, not in column bands.
#[test]
fn header_field_grid_reads_row_by_row() {
    header_grid(false);
}

/// Guard: the same grid with the values of a grid row in one data line.
#[test]
fn header_field_grid_of_one_line_a_row_reads_row_by_row() {
    header_grid(true);
}

// ---------------------------------------------------------------------------------------
// Separators drawn as marks

/// The pitch of the data font: Courier at 9.5 points is 5.7 points a character.
const PITCH: f32 = 5.7;

/// A boxed number field of `digits` cells from `x` with the data baseline `y`: one stroked
/// box a digit, and a drawn mark before each cell of `marks_before` (0-based), on the
/// baseline in the gap between two cells, as a form template prints its separators.
fn boxed_field(x: f32, y: f32, digits: usize, marks_before: &[usize]) -> String {
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
fn boxed_page(digits: &str, marks_before: &[usize], lines: &[&str]) -> FormPage {
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
fn found_by(out: &ReadOutcome) -> Vec<String> {
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
fn separator_losses(out: &ReadOutcome) -> Vec<&rezzer_model::Loss> {
    out.document
        .losses
        .iter()
        .filter(|l| l.what.contains("separator drawn as a mark"))
        .collect()
}

/// The found-by note stands right after the page marker of its page.
fn note_after_marker(out: &ReadOutcome) -> bool {
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
fn dashes(x0: f32, y0: f32, x1: f32, y1: f32) -> String {
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
fn separator_texts(out: &ReadOutcome) -> Vec<String> {
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

/// The edges of `n` columns of equal width across the form, from 40 to 555.
fn form_columns(n: usize) -> Vec<f32> {
    (0..=n)
        .map(|k| 40.0 + 515.0 * k as f32 / n as f32)
        .collect()
}

/// How the values of a form grid are printed.
#[derive(Clone, Copy)]
struct Print {
    /// Each value row as one data line from the data margin, padded into the cells; else
    /// each value as a data line of its own at its cell.
    one_line: bool,
    /// The baseline of the values above the bottom of their row.
    rise: f32,
}

/// Values one to a cell, 5 points above the row's bottom.
const BY_CELL: Print = Print {
    one_line: false,
    rise: 5.0,
};

/// One section of a form grid: rows of 16 points from `top` down between the column edges
/// `xs`, every rule drawn; the first row holds the labels `titles`, the other rows the
/// values of `rows`, printed as `print` says, each 3 points into its cell.
fn form_section(xs: &[f32], top: f32, titles: &[&str], rows: &[&[&str]], print: Print) -> String {
    let ys: Vec<f32> = (0..=rows.len() + 1)
        .map(|k| top - 16.0 * k as f32)
        .collect();
    let mut c = grid(xs, &ys);
    for (k, t) in titles.iter().enumerate() {
        c += &label(xs[k] + 3.0, top - 11.0, 7.0, t);
    }
    for (r, row) in rows.iter().enumerate() {
        let y = top - 16.0 * (r + 2) as f32 + print.rise;
        if print.one_line {
            let mut line = String::new();
            for (k, v) in row.iter().enumerate() {
                let at = pad_to(xs[k] + 3.0);
                line += &" ".repeat(at.saturating_sub(line.chars().count()));
                line += v;
            }
            c += &data_line(40.0, y, 9.5, &line);
        } else {
            for (k, v) in row.iter().enumerate() {
                c += &data_line(xs[k] + 3.0, y, 9.5, v);
            }
        }
    }
    c
}

/// The upper section of the form: 8 columns of personal data, a label row over two value
/// rows.
fn personal_section(print: Print) -> String {
    form_section(
        &form_columns(8),
        760.0,
        &[
            "Pers.-Nr.",
            "Name",
            "Eintritt",
            "Klasse",
            "Kinder",
            "Konf.",
            "Kasse",
            "Abt.",
        ],
        &[
            &[
                "0001", "Muster", "01.01.20", "1", "0,5", "x", "Kasse A", "100",
            ],
            &[
                "0002", "Beispiel", "15.03.21", "3", "1,0", "y", "Kasse B", "200",
            ],
        ],
        print,
    )
}

/// The earnings section: 9 columns, a label row over three rows of amounts, the first of
/// them drawn by `brutto`.
fn earnings_section(top: f32, brutto: &str, print: Print) -> String {
    form_section(
        &form_columns(9),
        top,
        &[
            "Nr.", "Art", "Tage", "Std.", "Satz", "Basis", "Brutto", "Steuer", "Netto",
        ],
        &[
            &[
                "1001", "Gehalt", "20", "160", "12,34", "1.974,40", brutto, "123,45", "876,54",
            ],
            &[
                "1002", "Zulage", "20", "8", "5,00", "40,00", "40,00", "4,00", "36,00",
            ],
            &[
                "1003", "Bonus", "1", "0", "0,00", "100,00", "100,00", "10,00", "90,00",
            ],
        ],
        print,
    )
}

/// The captions of the tables of the read, in order.
fn read_captions(out: &ReadOutcome) -> Vec<Option<String>> {
    let mut found = Vec::new();
    for b in &out.document.body {
        if let Block::Section { blocks, .. } = b {
            for inner in blocks {
                if let Block::Table { caption, .. } = inner {
                    found.push(caption.clone());
                }
            }
        }
    }
    found
}

/// The tables of the read: the text of each cell.
fn read_tables(out: &ReadOutcome) -> Vec<Vec<Vec<String>>> {
    let mut found = Vec::new();
    for b in &out.document.body {
        if let Block::Section { blocks, .. } = b {
            for inner in blocks {
                if let Block::Table { rows, .. } = inner {
                    found.push(
                        rows.iter()
                            .map(|r| r.iter().map(|c| plain_text(&c.inlines)).collect())
                            .collect(),
                    );
                }
            }
        }
    }
    found
}

/// The form's grid of two sections without marks: 8 columns over 9 that share only the
/// outer edges, 17 column edges in all. Each section is read as its own table.
#[test]
fn form_sections_without_marks() {
    let template = personal_section(BY_CELL) + &earnings_section(712.0, "1.234,56", BY_CELL);
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let found = read_tables(&out);
    assert_eq!(found.len(), 2, "{found:?} {:?}", out.document.losses);
    assert_eq!(found[0].len(), 3, "{found:?}");
    assert!(found[0].iter().all(|r| r.len() == 8), "{found:?}");
    assert_eq!(found[0][1][1], "Muster", "{found:?}");
    assert_eq!(found[1].len(), 4, "{found:?}");
    assert!(found[1].iter().all(|r| r.len() == 9), "{found:?}");
    assert_eq!(found[1][1][6], "1.234,56", "{found:?}");
    assert_no_loss(&out);
}

/// The earnings section alone, its amounts printed as digits with drawn decimal marks on a
/// baseline 2 points above the row rules, filled (`re f`) or filled and stroked (`B*`) as
/// `stroked` says. The marks are no rules, so the grid keeps its 9 columns and 4 rows and is
/// read as one table with its separators.
fn marks_without_sections(stroked: bool) {
    let xs = form_columns(9);
    let top = 712.0;
    let amounts: [[&str; 4]; 3] = [
        ["197440", "123456", "12345", "87654"],
        ["4000", "4000", "400", "3600"],
        ["10000", "10000", "1000", "9000"],
    ];
    let rows: Vec<Vec<&str>> = ["1001", "1002", "1003"]
        .iter()
        .zip(["Gehalt", "Zulage", "Bonus"])
        .zip(&amounts)
        .map(|((nr, art), a)| {
            let mut row = vec![*nr, art, "20", "8", "5,00"];
            row.extend(a.iter().copied());
            row
        })
        .collect();
    let refs: Vec<&[&str]> = rows.iter().map(|r| r.as_slice()).collect();
    let titles = [
        "Nr.", "Art", "Tage", "Std.", "Satz", "Basis", "Brutto", "Steuer", "Netto",
    ];
    let print = Print {
        one_line: false,
        rise: 2.0,
    };
    let mut template = form_section(&xs, top, &titles, &refs, print);
    for (r, row) in amounts.iter().enumerate() {
        let y = top - 16.0 * (r + 2) as f32 + print.rise;
        for (k, v) in row.iter().enumerate() {
            let x = xs[5 + k] + 3.0 + (v.len() - 2) as f32 * PITCH;
            template += &if stroked {
                mark_b(x, y - 0.5, 2.0)
            } else {
                mark(x - 0.12, y - 0.5)
            };
        }
    }
    let page = FormPage {
        template,
        lines: vec![
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
    let found = read_tables(&out);
    assert_eq!(found.len(), 1, "{found:?} {:?}", out.document.losses);
    assert_eq!(found[0].len(), 4, "{found:?}");
    assert!(found[0].iter().all(|r| r.len() == 9), "{found:?}");
    assert_eq!(
        found[0][1][5..],
        ["1974,40", "1234,56", "123,45", "876,54"],
        "{found:?}"
    );
    assert_eq!(found[0][3][5..], ["100,00", "100,00", "10,00", "90,00"]);
    assert_no_loss(&out);
}

/// Guard: filled marks in one section; the form of two sections is refused only for its
/// sections (`form_sections_without_marks`).
#[test]
fn filled_marks_without_sections() {
    marks_without_sections(false);
}

/// Marks filled and stroked with a hairline: the 2 point sides of the stroke would meet the
/// row rules 1.5 points below and add 12 column edges; they are no rules either.
#[test]
fn stroked_marks_without_sections() {
    marks_without_sections(true);
}

/// Guard: a boxed amount of one row of digit boxes alone is a header band candidate that
/// holds no words: no table and no loss (this was so before Task 5 too).
#[test]
fn boxed_amount_alone_gives_no_loss() {
    let page = boxed_page("123456", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    let out = read(&print_form(&[page], "", ""));
    assert!(read_tables(&out).is_empty(), "{:?}", read_tables(&out));
    assert_no_loss(&out);
}

/// A boxed amount whose digit boxes stand under a box for its label: a boxed field, not a
/// table, and no loss.
#[test]
fn boxed_amount_under_a_label_box_gives_no_loss() {
    let x = 40.0 + 20.0 * PITCH;
    let mut page = boxed_page("123456", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    page.template += &boxed(x, 698.0, 6.0 * PITCH, 22.0);
    let out = read(&print_form(&[page], "", ""));
    assert!(all_text(&out).contains("1234,56"), "{}", all_text(&out));
    assert!(read_tables(&out).is_empty(), "{:?}", read_tables(&out));
    assert_no_loss(&out);
}

/// Two boxed fields, one right on top of the other: boxed fields, not a table, and no loss.
#[test]
fn stacked_boxed_fields_give_no_loss() {
    let x = 40.0 + 20.0 * PITCH;
    let mut page = boxed_page("123456", &[4], &["Summe 9.876,54", "Netto 1.234,56"]);
    page.template += &boxed_field(x, 711.0, 6, &[4]);
    page.lines.push(DataLine {
        y: 711.0,
        text: padded(20, "654321"),
    });
    let out = read(&print_form(&[page], "", ""));
    let text = all_text(&out);
    assert!(
        text.contains("1234,56") && text.contains("6543,21"),
        "{text}"
    );
    assert!(read_tables(&out).is_empty(), "{:?}", read_tables(&out));
    assert_no_loss(&out);
}

/// Guard: a calendar of 31 narrow equal day columns with one digit run in each row is no
/// boxed field when a row holds the days apart: it stays a refused grid with its loss.
#[test]
fn day_columns_are_not_a_boxed_field() {
    let xs: Vec<f32> = (0..=31).map(|k| 40.0 + 12.0 * k as f32).collect();
    let mut template = grid(&xs, &[660.0, 676.0, 692.0]);
    for (k, x) in xs[..31].iter().enumerate() {
        template += &label(x + 1.0, 681.0, 7.0, &format!("{}", k + 1));
    }
    template += &data_line(41.0, 665.0, 9.5, "1");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let what = one_uncertain_loss(&out);
    assert!(what.contains("more than 15 columns"), "{what}");
}

/// A chart of twenty columns with labels in two rows and a figure caption: the caption is
/// looked at before the columns are counted, so no note says it may be a table, and no loss.
#[test]
fn wide_chart_with_two_label_rows_gives_no_note() {
    let xs: Vec<f32> = (0..=20).map(|k| 40.0 + 25.0 * k as f32).collect();
    let mut template = grid(&xs, &[640.0, 660.0, 680.0, 700.0]);
    for k in 0..5 {
        template += &label(43.0 + 100.0 * k as f32, 646.0, 7.0, &format!("{}0", k + 1));
        template += &label(43.0 + 100.0 * k as f32, 686.0, 7.0, &format!("{}5", k + 1));
    }
    template += &label(40.0, 625.0, 9.0, "Figure 3: Stunden je Monat");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(
        !all_text(&out).contains("Possible table"),
        "{}",
        all_text(&out)
    );
    assert_no_loss(&out);
}

// ---------------------------------------------------------------------------------------
// A pay slip form: the geometry of a real print-run form, all text and values invented

/// A rule drawn as a filled bar of thickness `t` from `x0` to `x1`, its top at `y`.
fn bar_h(x0: f32, x1: f32, y: f32, t: f32) -> String {
    format!("{x0} {} {} {t} re f\n", y - t, x1 - x0)
}

/// A rule drawn as a filled bar of thickness `t` from `y0` to `y1`, centred on `x`.
fn bar_v(x: f32, y0: f32, y1: f32, t: f32) -> String {
    format!("{} {y0} {t} {} re f\n", x - t / 2.0, y1 - y0)
}

/// A separator mark of `h` points from `y0`, centred on `x`, filled and stroked with a
/// hairline as the print run draws it.
fn mark_b(x: f32, y0: f32, h: f32) -> String {
    let (l, r, top) = (x - 0.12, x + 0.12, y0 + h);
    format!("q 0 w {r} {top} m {l} {top} l {l} {y0} l {r} {y0} l h B* Q\n")
}

/// A bold template label in Helvetica-Bold (`/FB`).
fn bold(x: f32, y: f32, size: f32, text: &str) -> String {
    format!("BT /FB {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// A data line of the pay slip: each text placed at its character column.
fn columns_line(y: f32, parts: &[(usize, &str)]) -> DataLine {
    let mut text = String::new();
    for (k, t) in parts {
        let len = text.chars().count();
        assert!(*k >= len, "{k} overlaps {text:?}");
        text += &" ".repeat(k - len);
        text += t;
    }
    DataLine { y, text }
}

/// The pay slip page. `marks` draws the separator marks as `mark_b` (fill and stroke) when
/// true, as `re f` when false.
fn pay_slip(stroked_marks: bool) -> FormPage {
    let m = |x: f32, y0: f32| {
        if stroked_marks {
            mark_b(x, y0, 2.16)
        } else {
            format!("{} {y0} 0.24 2.16 re f\n", x - 0.12)
        }
    };
    let mut t = String::new();
    // Shared: the long border and the rules of the right column.
    t += &bar_v(458.44, 98.39, 554.62, 0.72);
    for y in [
        554.51, 394.91, 372.10, 326.51, 280.91, 258.10, 121.43, 98.62,
    ] {
        t += &bar_h(464.32, 538.00, y, 0.48);
    }
    t += &label(59.67, 632.28, 11.4, "Erika Mustermann");
    // Section E.
    t += &bar_h(42.40, 458.32, 554.51, 0.48);
    t += &bar_h(42.40, 458.32, 372.10, 0.48);
    for x in [76.72, 253.36, 327.52, 384.40, 424.24, 435.76, 447.04] {
        t += &bar_v(x, 542.86, 554.39, 0.24);
    }
    t += &bold(42.40, 557.74, 6.66, "Bezuege A");
    for (x, s) in [
        (42.40, "Ea"),
        (79.56, "Eb"),
        (256.24, "Ec"),
        (307.12, "Ed"),
        (364.96, "Ee"),
        (387.24, "Ef"),
        (427.08, "Eg"),
        (437.64, "Eh Ei"),
    ] {
        t += &label(x, 546.22, 6.6, s);
    }
    t += &bold(520.0, 546.22, 6.66, "Ej");
    t += &bold(42.40, 375.35, 6.66, "Abzuege B");
    t += &bold(500.0, 386.86, 6.66, "Ek");
    // Section T.
    t += &bar_h(42.40, 458.56, 326.39, 0.48);
    for x in [53.92, 128.08, 202.00, 270.40, 338.80] {
        t += &bar_v(x, 360.46, 371.98, 0.24);
    }
    for (x, s) in [
        (42.40, "Ta"),
        (114.0, "Tb"),
        (188.0, "Tc"),
        (256.6, "Td"),
        (324.7, "Te"),
        (470.32, "Tf"),
    ] {
        t += &bold(x, 364.07, 6.66, s);
    }
    for y0 in [349.42, 338.14, 326.62] {
        for x in [110.68, 184.84, 253.24, 321.64] {
            t += &m(x, y0);
        }
    }
    // Section S.
    t += &bar_h(42.40, 458.32, 280.91, 0.48);
    for x in [
        53.92, 105.28, 156.40, 207.76, 259.12, 310.24, 361.60, 407.20,
    ] {
        t += &bar_v(x, 315.11, 326.39, 0.24);
    }
    t += &bold(42.40, 318.46, 6.66, "Sa");
    for (end, s) in [
        (99.39, "Sb"),
        (150.75, "Sc"),
        (201.87, "Sd"),
        (253.23, "Se"),
        (304.59, "Sf"),
        (355.71, "Sg"),
        (401.31, "Sh"),
        (452.67, "Si"),
    ] {
        t += &bold(end - 8.0, 318.46, 6.66, s);
    }
    t += &bold(477.76, 318.46, 6.66, "Sj");
    for y0 in [303.83, 292.55, 281.02] {
        for x in [
            87.88, 139.24, 190.60, 241.84, 293.08, 344.44, 390.04, 441.28,
        ] {
            t += &m(x, y0);
        }
    }
    // Section L.
    t += &bar_h(281.92, 458.56, 258.10, 0.48);
    t += &bar_h(281.92, 458.56, 121.18, 0.48);
    t += &bar_h(42.40, 458.80, 98.39, 0.72);
    t += &bar_h(42.40, 270.40, 258.10, 0.48);
    t += &bar_h(42.40, 270.40, 143.99, 0.48);
    t += &bar_h(42.40, 270.40, 121.18, 0.48);
    t += &bar_v(162.16, 143.99, 257.98, 0.24);
    t += &bar_v(310.24, 246.71, 257.98, 0.24);
    for x in [333.04, 384.40] {
        t += &bar_v(x, 109.90, 121.18, 0.24);
    }
    for k in 0..10 {
        let y0 = 144.23 + 11.4 * k as f32;
        t += &m(145.00, y0);
        t += &m(253.24, y0);
    }
    for x in [315.88, 367.24, 441.28] {
        t += &m(x, 98.62);
    }
    t += &bold(42.40, 261.35, 6.66, "La");
    t += &bold(281.92, 261.35, 6.66, "Lb");
    for k in 0..9 {
        let y = 249.10 - 11.4 * k as f32;
        t += &label(42.40, y, 6.6, &format!("Lc{}", k + 1));
        if k < 7 {
            t += &label(163.60, y, 6.6, &format!("Ld{}", k + 1));
        }
    }
    t += &label(281.92, 249.82, 6.6, "Le");
    t += &label(311.68, 249.82, 6.6, "Lf");
    t += &bold(500.0, 250.07, 6.66, "Lg");
    t += &label(42.40, 112.54, 6.6, "Lh");
    t += &label(42.40, 101.03, 6.6, "Li");
    for (x, s) in [(297.76, "Lj"), (343.84, "Lk"), (421.60, "Ll")] {
        t += &label(x, 113.26, 6.6, s);
    }
    t += &bold(500.0, 113.26, 6.66, "Lm");
    t += &bold(493.36, 272.86, 6.66, "Ln");
    let lines = vec![
        columns_line(
            533.22,
            &[(2, "001 Gehalt"), (67, "X X Y"), (78, "1.234,56")],
        ),
        columns_line(
            521.82,
            &[(2, "002 Zulage Muster"), (67, "X X Y"), (81, "12,34")],
        ),
        columns_line(
            510.42,
            &[(2, "003 Beispiel Bezug"), (67, "Z X N"), (81, "56,78")],
        ),
        columns_line(373.62, &[(78, "1.302,90")]),
        columns_line(
            350.82,
            &[(0, "X"), (6, "12.34567"), (20, "1.23400"), (78, "1.234,00")],
        ),
        columns_line(
            305.22,
            &[
                (0, "X"),
                (12, "1.11100"),
                (21, "2.22200"),
                (50, "33344"),
                (58, "44455 7"),
                (80, "555,66"),
            ],
        ),
        columns_line(259.62, &[(78, "8.765,43")]),
        columns_line(248.22, &[(12, "99.99999"), (31, "88.88800")]),
        columns_line(
            236.82,
            &[(12, "99.99999"), (42, "9001 Muster A"), (81, "11,11")],
        ),
        columns_line(
            225.42,
            &[
                (13, "7.77700"),
                (32, "6.66600"),
                (42, "9002 Muster B"),
                (81, "22,22-"),
            ],
        ),
        columns_line(214.02, &[(42, "9003 Muster C"), (81, "33,33")]),
        columns_line(202.62, &[(42, "9004 Muster D"), (78, "4.444,44-")]),
        columns_line(111.42, &[(4, "Musterbank")]),
        columns_line(
            100.02,
            &[
                (4, "DE00 0000 0000 0000 0000 00"),
                (43, "1.23456"),
                (78, "9.876,54"),
            ],
        ),
    ];
    FormPage {
        template: t,
        left: 42.52,
        size: 9.5,
        lines,
        ..FormPage::default()
    }
}

/// The header rows of the tables of the read, in order.
fn read_header_rows(out: &ReadOutcome) -> Vec<usize> {
    let mut found = Vec::new();
    for b in &out.document.body {
        if let Block::Section { blocks, .. } = b {
            for inner in blocks {
                if let Block::Table { header_rows, .. } = inner {
                    found.push(*header_rows);
                }
            }
        }
    }
    found
}

/// The tables of the pay slip: its sections E, T, S, the right middle and the bottom of L,
/// each a header over its lines; no section is lost and none gives a loss. Each value of a
/// value line stands in the column of its label, also where only one or two spaces part it
/// from the next value across a column rule, and an empty field is an empty cell.
fn assert_pay_slip(out: &ReadOutcome) {
    let found = read_tables(out);
    assert_eq!(found.len(), 5, "{found:#?} {:?}", out.document.losses);
    let header = |t: &Vec<Vec<String>>| t[0].join("|");
    let body = |t: &Vec<Vec<String>>| t[1..].iter().map(|r| r.join("|")).collect::<Vec<_>>();
    // Each section's label row is its table's header row.
    assert_eq!(read_header_rows(out), [1, 1, 1, 1, 1], "{found:#?}");
    // E: 8 columns, a row for each earning; the label over two columns and the three flags
    // one character apart are cut at the column rules.
    assert_eq!(
        header(&found[0]),
        "Ea|Eb|Ec Ed|Ee|Ef|Eg|Eh|Ei",
        "{found:#?}"
    );
    assert_eq!(
        body(&found[0]),
        [
            "001|Gehalt||||X|X|Y",
            "002|Zulage Muster||||X|X|Y",
            "003|Beispiel Bezug||||Z|X|N"
        ]
    );
    // T and S: their headers and one row of values, the marks read as separators.
    assert_eq!(header(&found[1]), "Ta|Tb|Tc|Td|Te|", "{found:#?}");
    assert_eq!(body(&found[1]), ["X|12.345,67|1.234,00|||"], "{found:#?}");
    assert_eq!(
        header(&found[2]),
        "Sa|Sb|Sc|Sd|Se|Sf|Sg|Sh|Si",
        "{found:#?}"
    );
    assert_eq!(
        body(&found[2]),
        ["X||1.111,00|2.222,00|||333,44|444,55|7"],
        "{found:#?}"
    );
    // L: the right middle Le | Lf over four rows, the bottom right Lk | Ll over a value.
    assert_eq!(header(&found[3]), "Le|Lf", "{found:#?}");
    assert_eq!(
        body(&found[3]),
        [
            "9001|Muster A",
            "9002|Muster B",
            "9003|Muster C",
            "9004|Muster D"
        ]
    );
    assert!(header(&found[4]).ends_with("Lj|Lk|Ll"), "{found:#?}");
    assert_eq!(
        body(&found[4]),
        ["Li DE00 0000 0000 0000 0000 00 1.234,56||"],
        "{found:#?}"
    );
    // The title right above a section's top rule is that table's caption, never a row of
    // the table above (the rows of E and S above show none): T's in E's band, L's in the
    // band under S, and E's over the grid, as the grid is cut (ruling P6-Q).
    assert_eq!(
        read_captions(out),
        [
            Some("Bezuege A".to_string()),
            Some("Abzuege B".to_string()),
            None,
            Some("La Lb".to_string()),
            None
        ]
    );
    assert_no_loss(out);
}

/// The pay slip with its separator marks filled and stroked (`B*`), as the print run draws
/// them: the marks are no rules, so S keeps its 9 columns.
#[test]
fn pay_slip_sections_are_tables() {
    let out = read(&print_form(&[pay_slip(true)], "", ""));
    assert_pay_slip(&out);
}

/// Guard: the same pay slip with marks drawn as `re f` reads the same.
#[test]
fn pay_slip_with_filled_marks_reads_the_same() {
    let out = read(&print_form(&[pay_slip(false)], "", ""));
    assert_pay_slip(&out);
}

/// A table of 4 columns over a row of 3 drawn cells with one label: the row is a section of
/// its own and no header band (too few titles); it is not let go without a loss.
#[test]
fn a_refused_section_is_a_loss() {
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
    let xs = [40.0, 150.0, 400.0, 555.0];
    template += &grid(&xs, &[692.0, 712.0]);
    template += &label(44.0, 698.0, 7.0, "Muster GmbH");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert_eq!(read_tables(&out).len(), 1, "{:?}", read_tables(&out));
    let what = one_uncertain_loss(&out);
    assert_eq!(
        what,
        "a header band on page 1 is not read as a table: its cells hold too few column titles; \
         its text is read without the band"
    );
}

/// A table of 4 columns, a band of one cell with a line of text, and a table of 3 other
/// columns, one grid. With the line near the band's bottom, right above the cut, it is the
/// caption of the table below; near the band's top it is a note, the last row of the table
/// above.
fn note_or_title(near_bottom: bool) -> ReadOutcome {
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
fn hanging_section(dividers: &[f32], labels: &[&str], lines: Vec<DataLine>) -> FormPage {
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
