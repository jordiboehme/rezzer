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
/// that neither the rendered Markdown nor the JSON of the model holds a noncharacter of
/// U+FDD0 to U+FDEF: those are Rezzer's own placeholders while a page is read.
fn read(bytes: &[u8]) -> ReadOutcome {
    let out = common::read(bytes);
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    let json = serde_json::to_string(&out.document).expect("json");
    let placeholder = |c: char| ('\u{FDD0}'..='\u{FDEF}').contains(&c);
    assert!(
        !md.contains(placeholder),
        "a placeholder in the Markdown: {md}"
    );
    // JSON writes the noncharacters as they are, not escaped.
    assert!(!json.contains(placeholder), "a placeholder in the JSON");
    out
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
    // The place and the digit count, never the value.
    assert!(losses[0].what.contains("(6 digits)"), "{}", losses[0].what);
    assert!(!losses[0].what.contains("1234"), "{}", losses[0].what);
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
    assert!(losses[0].what.contains("(6 digits)"), "{}", losses[0].what);
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
    // 100,001 marks in rows of 500 below the data line, none near a digit.
    for k in 0..100_001usize {
        let (col, row) = (k % 500, k / 500);
        template += &format!(
            "{:.2} {:.2} 0.24 2 re f\n",
            20.0 + col as f32 * 1.1,
            20.0 + row as f32 * 2.5
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
