// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Print-run forms: a form template with a data layer of padded fixed-pitch lines, built in
//! code with invented values only.

mod common;

use std::time::{Duration, Instant};

use common::{DataLine, FormPage, all_text, boxed, data_line, in_order, label, mark, print_form};
use rezzer_model::{Block, Completeness, Inline, LossReason, ReadOutcome, Severity, plain_text};

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

/// The `Datum` of the header field grid, a label over its value in a row of fields, is the
/// document date: the pairs of the grid say it.
fn assert_grid_date(out: &ReadOutcome) {
    let found = out.document.properties.document_date.as_ref();
    assert_eq!(
        found.map(|d| (d.date.as_str(), d.from.as_str())),
        Some(("2026-10-01", "date label \"Datum\" on page 1"))
    );
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

/// One data line drawn twice 0.3 points apart (a fake bold) whose copies have their middles
/// on the two sides of a box's edge is read once, in the box of the first copy (Task 4
/// carry): the box ends between the middles of the two copies.
#[test]
fn fake_bold_across_a_box_edge_reads_once() {
    let line = "Auszahlung";
    // The middles of the two copies lie about 65.8 points from the left of the page.
    let mut failed = Vec::new();
    for edge in [65.6f32, 65.7, 65.8, 65.9, 66.0] {
        let page = FormPage {
            template: format!(
                "{}{}{}{}",
                boxed(30.0, 690.0, edge - 30.0, 30.0),
                label(32.0, 712.0, 6.0, "Betrag"),
                data_line(40.0, 700.0, 9.5, line),
                data_line(40.3, 700.0, 9.5, line)
            ),
            ..FormPage::default()
        };
        let out = read(&print_form(&[page], "", ""));
        let text = all_text(&out);
        if text.matches(line).count() != 1 {
            failed.push(format!("edge {edge}: {text:?}"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
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

/// The data column of the pay slip's data font (Courier 9.5 from x 42.52) at `x`.
fn column_at(x: f32) -> usize {
    ((x - 42.52) / PITCH).round() as usize
}

/// A field of a grid row: the x of its left edge, its label and its value.
type GridField = (f32, &'static str, &'static str);

/// The rows of the pay slip's header field grid: the height of each row's top rule, and for
/// each field of its left part the x of its left edge, its label and its value (empty for a
/// field left empty). A field `Steuerklasse` carries a raised footnote marker.
const HEADER_GRID: [(f32, [GridField; 4]); 3] = [
    (
        805.19,
        [
            (42.40, "Personalnummer", "00000"),
            (116.56, "Eintritt", "01.01.2020"),
            (190.00, "Steuerklasse", "1"),
            (264.00, "Konfession", ""),
        ],
    ),
    (
        782.39,
        [
            (42.40, "Geburtsdatum", "01011970"),
            (100.00, "Austritt", ""),
            (175.00, "Kinder", "0,5"),
            (250.00, "Kostenstelle", "4711"),
        ],
    ),
    (
        759.59,
        [
            (42.40, "Krankenkasse", "Muster BKK"),
            (130.00, "Beitragsgruppe", "1111"),
            (200.00, "Abteilung", "Muster"),
            (290.00, "Freibetrag", "1.234,56"),
        ],
    ),
];

/// The header field grid of the pay slip, as the print run draws it: rules across its left
/// part (42.40 to 390.16) and its right part (401.44 to 538.24), one row a pitch of 22.8;
/// column dividers that hang 11.53 points from the rule above (no divider reaches the rule
/// below), at other places in each row; a label of 6.6 points 8.17 under each row's rule and
/// its value in the data font 21.17 under it. The birth date is printed without its points,
/// which are drawn as marks rising 2 points from the rule below. The right part holds a date
/// label with its date line. The real form has three rows and the heights of three rules
/// only in the shape it was measured from; the top rule here is one row pitch above them.
/// All text and values are invented.
fn header_field_grid() -> (String, Vec<DataLine>) {
    let mut t = String::new();
    let mut lines = Vec::new();
    for y in [805.19, 782.39, 759.59, 736.78] {
        t += &bar_h(42.40, 390.16, y, 0.24);
        t += &bar_h(401.44, 538.24, y, 0.24);
    }
    for (top, fields) in HEADER_GRID {
        let mut parts: Vec<(usize, String)> = Vec::new();
        for (k, (x, l, v)) in fields.iter().enumerate() {
            if k > 0 {
                t += &bar_v(*x, top - 11.53, top, 0.24);
            }
            t += &label(x + 1.5, top - 8.17, 6.6, l);
            if *l == "Steuerklasse" {
                t += &label(x + 1.5 + 36.0, top - 6.17, 4.0, "1");
            }
            if !v.is_empty() {
                parts.push((column_at(x + 1.5), v.to_string()));
            }
        }
        let parts: Vec<(usize, &str)> = parts.iter().map(|(k, v)| (*k, v.as_str())).collect();
        lines.push(columns_line(top - 21.17, &parts));
    }
    // The points of the birth date, after its second and fourth digit.
    for k in [2.0, 4.0] {
        t += &mark_b(42.52 + k * PITCH, 782.39 - 22.8 + 0.24, 2.0);
    }
    // The right part: the date of the slip in its first row.
    t += &label(402.94, 805.19 - 8.17, 6.6, "Datum");
    lines.push(columns_line(
        805.19 - 21.17,
        &[(column_at(402.94), "01.10.2026")],
    ));
    (t, lines)
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
    let (grid, header_lines) = header_field_grid();
    t += &grid;
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
    let mut lines = header_lines;
    lines.extend([
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
        columns_line(
            499.02,
            &[
                (2, "045 Zulage fuer Muster Bezug"),
                (67, "X Y Z"),
                (81, "11,11"),
            ],
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
    ]);
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

/// The words of the paragraphs of a read that look like amounts or dates (digits with a
/// point or a comma), besides notes, captions and page markers: values without a label.
fn values_alone(out: &ReadOutcome) -> Vec<String> {
    let mut found = Vec::new();
    for b in &out.document.body {
        let Block::Section { blocks, .. } = b else {
            continue;
        };
        for inner in blocks {
            if let Block::Paragraph { inlines, role, .. } = inner
                && !matches!(role.as_deref(), Some("found-by" | "caption" | "page"))
            {
                found.extend(
                    plain_text(inlines)
                        .split_whitespace()
                        .filter(|w| w.chars().any(|c| c.is_ascii_digit()) && w.contains([',', '.']))
                        .map(str::to_string),
                );
            }
        }
    }
    found
}

/// The tables of the pay slip: its header field grid as label and value pairs (ruling P6-H);
/// its sections E, T, S, the right middle and the bottom of L, each a header over its lines;
/// no section is lost and none gives a loss. Each value of a value line stands in the column
/// of its label, also where only one or two spaces part it from the next value across a
/// column rule, and an empty field is an empty cell. The column of totals on the right joins
/// each section beside it (its amounts in the section's rows), and each total label in it
/// pairs with the amount under it (ruling P6-O). The block of labels with their values left
/// in L and the bank row under it are label and value pairs, an empty field marked empty.
/// No amount stands alone.
fn assert_pay_slip(out: &ReadOutcome) {
    let found = read_tables(out);
    assert_eq!(found.len(), 10, "{found:#?} {:?}", out.document.losses);
    let header = |t: &Vec<Vec<String>>| t[0].join("|");
    let body = |t: &Vec<Vec<String>>| t[1..].iter().map(|r| r.join("|")).collect::<Vec<_>>();
    let pairs = |t: &Vec<Vec<String>>| t.iter().map(|r| r.join(": ")).collect::<Vec<_>>();
    // Each section's label row is its table's header row; the pairs have none.
    assert_eq!(
        read_header_rows(out),
        [0, 1, 0, 1, 1, 0, 1, 0, 0, 1],
        "{found:#?}"
    );
    // The header field grid, row by row, with the date of its right part.
    assert_eq!(found[0].len(), 13, "{found:#?}");
    assert_eq!(
        pairs(&found[0])[..6],
        [
            "Personalnummer: 00000",
            "Eintritt: 01.01.2020",
            "Steuerklasse 1: 1",
            "Konfession: empty",
            "Datum: 01.10.2026",
            "Geburtsdatum: 01.01.1970"
        ],
        "{found:#?}"
    );
    // E: 8 columns and the amounts of the column on the right, a row for each earning; the
    // label over two columns and the three flags one character apart are cut at the column
    // rules.
    assert_eq!(
        header(&found[1]),
        "Ea|Eb|Ec Ed|Ee|Ef|Eg|Eh|Ei|Ej",
        "{found:#?}"
    );
    assert_eq!(
        body(&found[1]),
        [
            "001|Gehalt||||X|X|Y|1.234,56",
            "002|Zulage Muster||||X|X|Y|12,34",
            "003|Beispiel Bezug||||Z|X|N|56,78",
            "045|Zulage fuer Muster Bezug||||X|Y|Z|11,11"
        ]
    );
    // The total under E's amounts, with its label.
    assert_eq!(found[2], [["Ek", "1.302,90"]], "{found:#?}");
    // T and S: their headers and one row of values, the marks read as separators.
    assert_eq!(header(&found[3]), "Ta|Tb|Tc|Td|Te||Tf", "{found:#?}");
    assert_eq!(
        body(&found[3]),
        ["X|12.345,67|1.234,00||||1.234,00"],
        "{found:#?}"
    );
    assert_eq!(
        header(&found[4]),
        "Sa|Sb|Sc|Sd|Se|Sf|Sg|Sh|Si|Sj",
        "{found:#?}"
    );
    assert_eq!(
        body(&found[4]),
        ["X||1.111,00|2.222,00|||333,44|444,55|7|555,66"],
        "{found:#?}"
    );
    // The total in the band of L's titles, with its label.
    assert_eq!(found[5], [["Ln", "8.765,43"]], "{found:#?}");
    // L: the right middle Le | Lf | Lg over four rows.
    assert_eq!(header(&found[6]), "Le|Lf|Lg", "{found:#?}");
    assert_eq!(
        body(&found[6]),
        [
            "9001|Muster A|11,11",
            "9002|Muster B|22,22-",
            "9003|Muster C|33,33",
            "9004|Muster D|4.444,44-"
        ]
    );
    // L's left block: its two columns of labels, each with the value on its line, column by
    // column; the fields with only their marks are empty.
    let left = pairs(&found[7]);
    assert_eq!(left.len(), 16, "{found:#?}");
    assert_eq!(
        left[..4],
        [
            "Lc1: 99.999,99",
            "Lc2: 99.999,99",
            "Lc3: 7.777,00",
            "Lc4: empty"
        ],
        "{left:?}"
    );
    assert_eq!(
        left[9..12],
        ["Ld1: 88.888,00", "Ld2: empty", "Ld3: 6.666,00"],
        "{left:?}"
    );
    // The bank row: its labels with the bank and the account, and the fields beside it as a
    // header over their values.
    assert_eq!(
        pairs(&found[8]),
        ["Lh: Musterbank", "Li: DE00 0000 0000 0000 0000 00"],
        "{found:#?}"
    );
    assert_eq!(header(&found[9]), "Lj|Lk|Ll|Lm", "{found:#?}");
    assert_eq!(body(&found[9]), ["1.234,56|||9.876,54"], "{found:#?}");
    // The title right above a section's top rule is that table's caption, never a row of
    // the table above (the rows of E and S above show none): T's in E's band, L's in the
    // band under S, and E's over the grid, as the grid is cut (ruling P6-Q).
    let mut captions = vec![None; 10];
    captions[1] = Some("Bezuege A".to_string());
    captions[3] = Some("Abzuege B".to_string());
    captions[6] = Some("La Lb".to_string());
    assert_eq!(read_captions(out), captions);
    // Each rule that pairs is said once, before the first table it found.
    let notes = found_by(out);
    for rule in ["column of amounts", "rows of fields"] {
        assert_eq!(
            notes.iter().filter(|n| n.contains(rule)).count(),
            1,
            "{notes:?}"
        );
    }
    assert_eq!(values_alone(out), Vec::<String>::new());
    assert_no_loss(out);
    assert_grid_date(out);
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

/// The pay slip printed smaller or larger as a whole (ruling P6-Y: no bound of this task is a
/// number of points measured on one form). At 0.7 it reads the same. At 1.5 the header field
/// grid, the column of totals beside E (its amounts joined, its totals paired) and the bank
/// row read the same; the drawn marks are then 3.24 points long, longer than a mark may be
/// (`MARK_LONG`, 3 points, older than this task), so they are read as rules and the left block
/// and the sections T and S are not read as at 1.0 (reported, not tuned away).
#[test]
fn pay_slip_printed_smaller_or_larger_reads_the_same() {
    let scaled = |scale: f32| {
        let page = FormPage {
            scale,
            ..pay_slip(true)
        };
        read(&print_form(&[page], "", ""))
    };
    assert_pay_slip(&scaled(0.7));
    let base = read_tables(&read(&print_form(&[pay_slip(true)], "", "")));
    let large = read_tables(&scaled(1.5));
    assert_eq!(large.first(), base.first(), "the header field grid");
    let e = |t: &[Vec<Vec<String>>]| {
        t.iter()
            .find(|t| t[0][0] == "Ea")
            .map(|t| t.iter().map(|r| r[r.len() - 1].clone()).collect::<Vec<_>>())
    };
    assert_eq!(e(&large), e(&base), "the column of totals beside E");
    let rows: Vec<String> = large.iter().flatten().map(|r| r.join(": ")).collect();
    for want in [
        "Ek: 1.302,90",
        "Ln: 8.765,43",
        "Lh: Musterbank",
        "Li: DE00 0000 0000 0000 0000 00",
    ] {
        assert!(rows.iter().any(|r| r == want), "{want} in {large:#?}");
    }
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

/// A section of five fields with a column of totals 6 points to its right at x 406 to 480:
/// a strip at the section's rules titled "Af" on the section's label baseline over the
/// amount of the value line, and a strip below with the total label "Ag" over its amount.
/// `rules` are the heights of the column's rules, `sides` draws vertical rules at its ends.
fn section_with_totals(rules: &[f32], sides: bool) -> FormPage {
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
fn side_noted(out: &ReadOutcome) -> bool {
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

/// Template labels of 6 pt, at least `ink` ink characters, from `top` down at 40.
fn label_mass(ink: usize, top: f32) -> String {
    const LABEL: &str = "Angaben zur Person und zum Arbeitgeber laut Vertrag";
    let mut out = String::new();
    let (mut n, mut y) = (0, top);
    while n < ink {
        out.push_str(&label(40.0, y, 6.0, LABEL));
        n += LABEL.chars().filter(|c| !c.is_whitespace()).count();
        y -= 8.0;
    }
    out
}

/// Data lines of the print layer, each padded into its place, about 300 ink characters from
/// `top` down. No two start at one x, so they form no columns.
fn padded_data_lines(top: f32) -> Vec<DataLine> {
    (0..7)
        .map(|k| DataLine {
            y: top - 12.0 * k as f32,
            text: format!(
                "{}Muster{k} Kostenstelle Abteilung Musterort Nord",
                " ".repeat(4 * k)
            ),
        })
        .collect()
}

fn headings_of(out: &ReadOutcome) -> Vec<(u8, String)> {
    fn walk(blocks: &[Block], found: &mut Vec<(u8, String)>) {
        for b in blocks {
            match b {
                Block::Heading { level, inlines, .. } => {
                    found.push((*level, plain_text(inlines)));
                }
                Block::Section { blocks, .. } => walk(blocks, found),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    walk(&out.document.body, &mut found);
    found
}

/// The top-level blocks of the first page section.
fn page_blocks(out: &ReadOutcome) -> &[Block] {
    out.document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section { blocks, .. } => Some(blocks.as_slice()),
            _ => None,
        })
        .expect("a page section")
}

#[test]
fn data_lines_are_not_headings() {
    let page = FormPage {
        template: label_mass(800, 400.0),
        lines: padded_data_lines(700.0),
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(headings_of(&out).is_empty(), "{:?}", headings_of(&out));
    assert!(all_text(&out).contains("Muster5"), "{}", all_text(&out));
}

#[test]
fn address_block_keeps_its_lines() {
    let mut lines = vec![
        DataLine {
            y: 760.0,
            text: "Erika Mustermann".into(),
        },
        DataLine {
            y: 748.0,
            text: "Musterweg 1".into(),
        },
        DataLine {
            y: 736.0,
            text: "12345 Musterstadt".into(),
        },
        DataLine {
            y: 724.0,
            text: "Deutschland".into(),
        },
    ];
    lines.extend(padded_data_lines(640.0));
    let page = FormPage {
        template: label_mass(800, 400.0),
        lines,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(headings_of(&out).is_empty(), "{:?}", headings_of(&out));
    let address = page_blocks(&out)
        .iter()
        .find_map(|b| match b {
            Block::Paragraph { inlines, .. }
                if plain_text(inlines).starts_with("Erika Mustermann") =>
            {
                Some(inlines)
            }
            _ => None,
        })
        .expect("the address paragraph");
    assert_eq!(
        plain_text(address),
        "Erika Mustermann Musterweg 1 12345 Musterstadt Deutschland"
    );
    let breaks = address
        .iter()
        .filter(|i| matches!(i, rezzer_model::Inline::LineBreak))
        .count();
    assert_eq!(breaks, 3, "{address:?}");
}

#[test]
fn star_with_padding_is_no_bullet() {
    let page = FormPage {
        template: label(40.0, 760.0, 6.0, "Muster GmbH Abrechnung"),
        lines: vec![
            DataLine {
                y: 700.0,
                text: "*     Betrag".into(),
            },
            DataLine {
                y: 640.0,
                text: "die Ware wird wie vereinbart an die Muster GmbH geliefert".into(),
            },
        ],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let lists = page_blocks(&out)
        .iter()
        .filter(|b| matches!(b, Block::List { .. }))
        .count();
    assert_eq!(lists, 0, "{:#?}", page_blocks(&out));
    assert!(all_text(&out).contains("Betrag"), "{}", all_text(&out));
}

#[test]
fn footnotes_form_a_legend() {
    let template = common::show("FL", 14.0, 40.0, 780.0, "Muster GmbH")
        + &common::show("FL", 9.0, 40.0, 700.0, "Betrag* 1.234,56")
        + &common::show(
            "FL",
            9.0,
            40.0,
            688.0,
            "Die Ware wird wie vereinbart geliefert.",
        )
        + &common::show("FL", 6.0, 40.0, 60.0, "* steuerfrei");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let blocks = page_blocks(&out);
    assert_eq!(
        footnote_texts(&out),
        [("p1-star".to_string(), "steuerfrei".to_string())],
        "{blocks:#?}"
    );
    assert!(
        !blocks.iter().any(|b| matches!(b, Block::List { .. })),
        "{blocks:#?}"
    );
    // The legend closes the page, and the star after `Betrag` refers to it.
    assert!(
        matches!(blocks.last(), Some(Block::Section { anchor: Some(a), .. }) if a.0 == "fn:p1-star"),
        "{blocks:#?}"
    );
    assert_eq!(references(&out), ["p1-star"], "{blocks:#?}");
    assert!(
        all_text(&out).contains("Betrag 1.234,56"),
        "{}",
        all_text(&out)
    );
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    assert!(md.contains("Betrag[^p1-star] 1.234,56"), "{md}");
    assert!(md.contains("[^p1-star]: steuerfrei"), "{md}");
    assert_eq!(found_by(&out).len(), 1, "{:?}", found_by(&out));
}

/// A legend on pages with a running header and a running footer (three pages, so both
/// repeat): the furniture lines are taken out of the page's text before the legend's lines
/// are found again, and each reference still links to its footnote (Task 7 carry: the place
/// of each run is mapped across the lines taken out).
#[test]
fn legend_under_running_header_and_footer_is_linked() {
    let notes = ["steuerfrei", "beitragsfrei", "pauschal versteuert"];
    let page = |n: usize| {
        let template = common::show("FL", 8.0, 40.0, 815.0, "Muster GmbH Abrechnung")
            + &common::show("FL", 14.0, 40.0, 600.0, "Erika Mustermann")
            + &common::show("FL", 9.0, 40.0, 500.0, "Betrag* 1.234,56")
            + &common::show(
                "FL",
                9.0,
                40.0,
                488.0,
                "Die Ware wird wie vereinbart geliefert.",
            )
            + &common::show("FL", 6.0, 40.0, 60.0, &format!("* {}", notes[n - 1]))
            + &common::show("FL", 8.0, 40.0, 25.0, &format!("Seite {n} von 3"));
        FormPage {
            template,
            ..FormPage::default()
        }
    };
    let out = read(&print_form(&[page(1), page(2), page(3)], "", ""));
    let furniture: Vec<String> = out
        .document
        .body
        .iter()
        .flat_map(|b| match b {
            Block::Section { blocks, .. } => blocks.as_slice(),
            _ => &[],
        })
        .filter_map(|b| match b {
            Block::Paragraph {
                role: Some(r),
                inlines,
                ..
            } if r.starts_with("page-") => Some(format!("{r}: {}", plain_text(inlines))),
            _ => None,
        })
        .collect();
    let mut expected = Vec::new();
    for n in 1..=3 {
        expected.push("page-header: Muster GmbH Abrechnung".to_string());
        expected.push(format!("page-footer: Seite {n} von 3"));
    }
    assert_eq!(furniture, expected, "{:#?}", out.document.body);
    let linked: Vec<(String, String)> = (1..=3)
        .map(|n| (format!("p{n}-star"), notes[n - 1].to_string()))
        .collect();
    // The footnotes of every page, by their anchors.
    let mut found: Vec<(String, String)> = Vec::new();
    for b in &out.document.body {
        let Block::Section { blocks, .. } = b else {
            continue;
        };
        for inner in blocks {
            if let Block::Section {
                anchor: Some(a),
                blocks,
                ..
            } = inner
                && let Some(label) = a.0.strip_prefix("fn:")
            {
                let text: Vec<String> = blocks
                    .iter()
                    .map(|b| match b {
                        Block::Paragraph { inlines, .. } => plain_text(inlines),
                        _ => String::new(),
                    })
                    .collect();
                found.push((label.to_string(), text.join(" ")));
            }
        }
    }
    assert_eq!(found, linked, "{:#?}", out.document.body);
    assert_eq!(references(&out), ["p1-star", "p2-star", "p3-star"]);
    assert_no_loss(&out);
}

/// Guard: a title at 1.5 times the body size or more stays a level-1 heading, also when it is
/// not bold (Task 8 takes the document title from it).
#[test]
fn form_title_stays_a_heading() {
    let template = common::show("FL", 14.0, 40.0, 780.0, "Musterformular")
        + &common::show(
            "FL",
            9.0,
            40.0,
            700.0,
            "die Angaben dieses Formulars werden von der Muster GmbH vertraulich behandelt",
        )
        + &common::show(
            "FL",
            9.0,
            40.0,
            688.0,
            "und nur zu dem Zweck verwendet, zu dem sie erhoben wurden, und nicht weiter",
        );
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert_eq!(headings_of(&out), [(1, "Musterformular".to_string())]);
}

/// The footnote paragraphs of the first page section.
fn footnote_texts(out: &ReadOutcome) -> Vec<(String, String)> {
    page_blocks(out)
        .iter()
        .filter_map(|b| match b {
            Block::Section {
                anchor: Some(a),
                blocks,
                ..
            } => {
                let label = a.0.strip_prefix("fn:")?;
                match blocks.as_slice() {
                    [
                        Block::Paragraph {
                            inlines,
                            role: Some(r),
                            ..
                        },
                    ] if r == "footnote" => Some((label.to_string(), plain_text(inlines))),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// The labels of the footnote references of the read, in order, also in table cells.
fn references(out: &ReadOutcome) -> Vec<String> {
    fn inlines(found: &mut Vec<String>, inlines: &[rezzer_model::Inline]) {
        for i in inlines {
            if let rezzer_model::Inline::FootnoteRef { id } = i {
                found.push(id.clone());
            }
        }
    }
    fn walk(blocks: &[Block], found: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Paragraph { inlines: i, .. } | Block::Heading { inlines: i, .. } => {
                    inlines(found, i);
                }
                Block::Table { rows, .. } => {
                    rows.iter()
                        .flatten()
                        .for_each(|c| inlines(found, &c.inlines));
                }
                Block::Section { blocks, .. } => walk(blocks, found),
                Block::List { items, .. } => items.iter().for_each(|i| walk(i, found)),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    walk(&out.document.body, &mut found);
    found
}

/// Footnote numbers raised after labels above, and a legend of two columns on one line: each
/// footnote is its own paragraph, the columns do not run together.
#[test]
fn raised_numbers_and_a_legend_in_columns() {
    let template = common::show("FL", 9.0, 40.0, 720.0, "Einheit")
        + &common::show("FL", 5.0, 72.0, 724.0, "2")
        + &common::show("FL", 9.0, 200.0, 720.0, "Beitrag")
        + &common::show("FL", 5.0, 232.0, 724.0, "3")
        + &common::show(
            "FL",
            9.0,
            40.0,
            690.0,
            "die Werte werden von der Muster GmbH monatlich abgerechnet und gemeldet",
        )
        + &common::show("FL", 6.0, 40.0, 70.0, "2 Std = Stunden")
        + &common::show("FL", 6.0, 300.0, 70.0, "3 Gegebenenfalls anteilig")
        + &common::show("FL", 6.0, 300.0, 62.0, "nach Vertrag");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert_eq!(
        footnote_texts(&out),
        [
            ("p1-2".to_string(), "Std = Stunden".to_string()),
            (
                "p1-3".to_string(),
                "Gegebenenfalls anteilig nach Vertrag".to_string()
            )
        ],
        "{:#?}",
        page_blocks(&out)
    );
    assert_eq!(references(&out), ["p1-2", "p1-3"]);
    assert_eq!(found_by(&out).len(), 1, "{:?}", found_by(&out));
}

/// Guard: a numbered line at the bottom of a page is no footnote when no number above is
/// raised as a reference.
#[test]
fn numbered_bottom_line_without_reference_is_no_footnote() {
    let template = common::show("FL", 9.0, 40.0, 720.0, "Einheit 2 Stunden")
        + &common::show(
            "FL",
            9.0,
            40.0,
            690.0,
            "die Werte werden von der Muster GmbH monatlich abgerechnet und gemeldet",
        )
        + &common::show("FL", 6.0, 40.0, 70.0, "2 Std = Stunden");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    assert!(footnote_texts(&out).is_empty(), "{:#?}", page_blocks(&out));
    assert!(found_by(&out).is_empty(), "{:?}", found_by(&out));
}

/// A header label with a reference number raised right after it, nearly in its size.
fn referenced(x: f32, y: f32, text: &str, number: &str) -> String {
    let w = common::helvetica_width(text, 8.0);
    common::show("FL", 8.0, x, y, text) + &common::show("FL", 7.0, x + w + 0.5, y + 2.0, number)
}

/// The legend of a pay slip: three columns side by side at the bottom (entries 1 to 3, 4,
/// and 5 to 8), entries that wrap onto a second line, a form number between the columns,
/// and the references raised after labels in the header cells of a drawn table and in a
/// header field line.
#[test]
fn legend_of_three_columns_links_its_references() {
    let xs = [30.0, 130.0, 230.0, 330.0, 430.0, 565.0];
    let mut template = grid(&xs, &[600.0, 620.0, 640.0, 660.0, 680.0]);
    let header = [
        ("Einheit", "2"),
        ("Menge", "3"),
        ("St", "4"),
        ("SV", "4"),
        ("GB", "5"),
    ];
    for (c, (text, number)) in header.iter().enumerate() {
        template += &referenced(xs[c] + 6.0, 666.0, text, number);
    }
    for (r, y) in [646.0, 626.0, 606.0].iter().enumerate() {
        for (c, x) in xs[..5].iter().enumerate() {
            template += &common::show("FL", 8.0, x + 6.0, *y, &format!("{}0,{r}0", c + 1));
        }
    }
    for (k, (text, number)) in [
        ("Name", "1"),
        ("Datum", "1"),
        ("Abzug", "6"),
        ("Zuschlag", "7"),
        ("Rest", "8"),
    ]
    .iter()
    .enumerate()
    {
        template += &referenced(40.0 + 100.0 * k as f32, 720.0, text, number);
    }
    template += &common::show(
        "FL",
        9.0,
        40.0,
        560.0,
        "die Werte werden von der Muster GmbH monatlich abgerechnet und gemeldet",
    );
    let legend = [
        (40.0, 90.0, "1 A = Muster, B = Beispiel"),
        (40.0, 82.0, "2 Std = Stunden, Min = Minuten,"),
        (46.0, 74.0, "Tg = Tage"),
        (40.0, 66.0, "3 Gegebenenfalls anteilig"),
        (220.0, 90.0, "4 SV = Sozialversicherung, KV ="),
        (226.0, 82.0, "Krankenversicherung"),
        (400.0, 90.0, "5 GB = Grundbetrag"),
        (400.0, 82.0, "6 Abz = Abzug, Zu = Zuschlag"),
        (400.0, 74.0, "7 Rest = Restbetrag nach"),
        (406.0, 66.0, "Abzug aller Posten"),
        (400.0, 58.0, "8 Mio = Million Euro"),
        (300.0, 58.0, "Muster-Nr. 0000 00"),
    ];
    for (x, y, text) in legend {
        template += &common::show("FL", 6.0, x, y, text);
    }
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let blocks = page_blocks(&out);
    let label = |n: u32| format!("p1-{n}");
    let want: Vec<(String, String)> = [
        (1, "A = Muster, B = Beispiel"),
        (2, "Std = Stunden, Min = Minuten, Tg = Tage"),
        (3, "Gegebenenfalls anteilig"),
        (4, "SV = Sozialversicherung, KV = Krankenversicherung"),
        (5, "GB = Grundbetrag"),
        (6, "Abz = Abzug, Zu = Zuschlag"),
        (7, "Rest = Restbetrag nach Abzug aller Posten"),
        (8, "Mio = Million Euro"),
    ]
    .iter()
    .map(|(n, t)| (label(*n), t.to_string()))
    .collect();
    assert_eq!(footnote_texts(&out), want, "{blocks:#?}");
    // Every reference has its footnote; the header cells hold theirs.
    let refs = references(&out);
    for n in 1..=8 {
        assert!(refs.contains(&label(n)), "{n}: {refs:?}");
    }
    let tables = read_tables(&out);
    assert_eq!(tables.len(), 1, "{blocks:#?}");
    assert_eq!(
        tables[0][0],
        ["Einheit", "Menge", "St", "SV", "GB"],
        "{tables:?}"
    );
    assert!(
        all_text(&out).contains("Muster-Nr. 0000 00"),
        "{}",
        all_text(&out)
    );
    assert_eq!(found_by(&out).len(), 1, "{:?}", found_by(&out));
}

/// A label in 6.6 pt with a 4.7 pt marker raised 1.92 after it, `gap` points after its end.
fn label_marked(x: f32, y: f32, text: &str, number: &str, gap: f32) -> String {
    let w = common::helvetica_width(text, 6.6);
    common::show("FL", 6.6, x, y, text) + &common::show("FL", 4.7, x + w + gap, y + 1.92, number)
}

/// The legend as the real print template draws it: markers in runs of their own, the same
/// size as the text, raised; columns drawn one after the other; the form number drawn first;
/// a larger data value drawn after the template above the legend.
#[test]
fn legend_of_raised_markers_in_columns_links_its_references() {
    let xs = [30.0, 130.0, 230.0, 330.0, 430.0, 565.0];
    let mut template = String::new();
    // Form number first, as the template draws it.
    template += &common::show("FL", 4.7, 42.40, 58.31, "Muster-Nr. 0000 00");
    template += &grid(&xs, &[600.0, 620.0, 640.0, 660.0, 680.0]);
    // Header row: markers after labels, one inside a two-label run.
    template += &label_marked(xs[0] + 6.0, 666.0, "Einheit", "2", 4.4);
    template += &label_marked(xs[1] + 6.0, 666.0, "Menge", "3", 0.7);
    template += &label_marked(xs[2] + 6.0, 666.0, "St", "4", 1.3);
    let sv = xs[3] + 6.0;
    template += &common::show("FL", 6.6, sv, 666.0, "SV GB");
    let w_sv = common::helvetica_width("SV", 6.6);
    let w_all = common::helvetica_width("SV GB", 6.6);
    template += &common::show("FL", 4.7, sv + w_sv + 0.5, 667.92, "4");
    template += &common::show("FL", 4.7, sv + w_all + 1.1, 667.92, "5");
    template += &label_marked(xs[4] + 6.0, 666.0, "Betrag", "6", 0.6);
    for (r, y) in [646.0, 626.0, 606.0].iter().enumerate() {
        for (c, x) in xs[..5].iter().enumerate() {
            template += &common::show("FL", 8.0, x + 6.0, *y, &format!("{}0,{r}0", c + 1));
        }
    }
    // Header field line: a label ending in '.', one ending in '%', one plain.
    template += &label_marked(40.0, 720.0, "Musterwert mtl.", "1", 6.3);
    template += &label_marked(160.0, 720.0, "Satz %", "8", 3.4);
    template += &label_marked(260.0, 720.0, "Kennz", "7", 1.8);
    template += &common::show(
        "FL",
        9.0,
        40.0,
        560.0,
        "die Werte werden von der Muster GmbH monatlich abgerechnet und gemeldet",
    );
    let show = |x: f32, y: f32, t: &str| common::show("FL", 4.7, x, y, t);
    // Column 1: markers, then texts, then two empty lines.
    for (y, m) in [(93.58, "1"), (88.06, "2"), (76.54, "3")] {
        template += &show(42.40, y, m);
    }
    template += &show(42.40, 70.78, " ");
    for (y, t) in [
        (91.90, "A = Musterbetrag nach Vertrag"),
        (86.38, "Abc = Abcdef, Gh = Ghijkl, Mn = Mnopqr, Uv = Uvwxyz"),
        (80.62, "Ab = Abend, Bc = Becher, Cd = Cedern, De = Delle"),
        (74.86, "Muster anteilig nach Tagen im Monat"),
        (69.10, " "),
    ] {
        template += &show(46.72, y, t);
    }
    // Column 2 texts; its marker comes last.
    for (y, t) in [
        (91.90, "Q = Quelle, R = Rest, U = Umlage, X = Extra,"),
        (86.38, "Y = Ypsilon, K = Kappe, D = Delta, O = Oktav,"),
        (80.62, "H = Halde, I = Insel, C = Chor, G = Garten"),
        (74.86, "V = Vorlage, W = Wiese"),
        (69.10, " "),
    ] {
        template += &show(190.72, y, t);
    }
    // Column 3: markers, then texts.
    for (y, m) in [
        (93.34, "5"),
        (87.82, "6"),
        (76.30, "7"),
        (70.54, "8 "),
        (65.02, " "),
    ] {
        template += &show(327.52, y, m);
    }
    for (y, t) in [
        (91.90, "J = Teil der Mustersumme im Monat"),
        (86.38, "Z = Zuschlag zur Musterkasse fuer alle Gruppen,"),
        (80.62, "E = Eintrag geprueft, 1-5 = Anzahl der Muster"),
        (74.86, "KZ = Kennzeichen fuer den Monat"),
        (69.10, "Muster Satz zur Musterkasse mit allen Teilen"),
        (63.58, " "),
    ] {
        template += &show(331.60, y, t);
    }
    template += &show(186.40, 93.58, "4");
    template += &show(186.40, 70.78, " ");
    template += &show(177.52, 58.31, "- Dies ist ein Musterblatt fuer Tests -");
    // The total box at the right, its value printed after the template above the legend.
    template += &common::boxed(464.32, 98.62, 73.68, 22.81);
    template += &common::show("FB", 6.66, 480.0, 113.26, "Musterbetrag");
    let page = FormPage {
        template,
        lines: vec![DataLine {
            y: 100.02,
            text: format!("{}9.999,99", " ".repeat(79)),
        }],
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let blocks = page_blocks(&out);
    let label = |n: u32| format!("p1-{n}");
    let want: Vec<(String, String)> = [
        (1, "A = Musterbetrag nach Vertrag"),
        (
            2,
            "Abc = Abcdef, Gh = Ghijkl, Mn = Mnopqr, Uv = Uvwxyz Ab = Abend, Bc = Becher, \
             Cd = Cedern, De = Delle",
        ),
        (3, "Muster anteilig nach Tagen im Monat"),
        (
            4,
            "Q = Quelle, R = Rest, U = Umlage, X = Extra, Y = Ypsilon, K = Kappe, D = Delta, \
             O = Oktav, H = Halde, I = Insel, C = Chor, G = Garten V = Vorlage, W = Wiese",
        ),
        (5, "J = Teil der Mustersumme im Monat"),
        (
            6,
            "Z = Zuschlag zur Musterkasse fuer alle Gruppen, E = Eintrag geprueft, 1-5 = \
             Anzahl der Muster",
        ),
        (7, "KZ = Kennzeichen fuer den Monat"),
        (8, "Muster Satz zur Musterkasse mit allen Teilen"),
    ]
    .iter()
    .map(|(n, t)| (label(*n), t.to_string()))
    .collect();
    assert_eq!(footnote_texts(&out), want, "{blocks:#?}");
    // The legend sets no body size: the sentence and the bold amount label are no headings.
    assert!(headings_of(&out).is_empty(), "{:?}", headings_of(&out));
    let refs = references(&out);
    for n in 1..=8 {
        assert!(refs.contains(&label(n)), "{n}: {refs:?}");
    }
    // Both markers of the run of two labels stand after their own label.
    let header = header_cells(&out);
    assert_eq!(
        header[3],
        [
            Inline::plain("SV"),
            Inline::FootnoteRef { id: label(4) },
            Inline::plain(" GB"),
            Inline::FootnoteRef { id: label(5) },
        ],
        "{header:?}"
    );
}

/// The inlines of the cells of the first row of the first table.
fn header_cells(out: &ReadOutcome) -> Vec<Vec<Inline>> {
    page_blocks(out)
        .iter()
        .find_map(|b| match b {
            Block::Table { rows, .. } => Some(rows[0].iter().map(|c| c.inlines.clone()).collect()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Guard: every footnote reference has its footnote, whatever limit stops the page. (The
/// limits of this small page stop it before its text or not at all; the cut legend itself is
/// tested in `layout::footnotes`.)
#[test]
fn no_limit_leaves_a_reference_without_its_footnote() {
    let template = common::show("FL", 9.0, 40.0, 700.0, "Betrag* 1.234,56")
        + &common::show("FL", 9.0, 40.0, 688.0, "Summe** 2.345,67")
        + &common::show(
            "FL",
            9.0,
            40.0,
            676.0,
            "Die Ware wird wie vereinbart geliefert.",
        )
        + &common::show("FL", 6.0, 40.0, 60.0, "* steuerfrei")
        + &common::show("FL", 6.0, 40.0, 52.0, "** ohne Abzug");
    let bytes = print_form(
        &[FormPage {
            template,
            ..FormPage::default()
        }],
        "",
        "",
    );
    let limits = (100..140)
        .map(|max_elements| rezzer_limits::Limits {
            max_elements,
            ..rezzer_limits::Limits::default()
        })
        .chain((50..100).map(|max_text_bytes| rezzer_limits::Limits {
            max_text_bytes,
            ..rezzer_limits::Limits::default()
        }));
    let mut any = false;
    for limits in limits {
        let out = common::read_in(
            &bytes,
            &rezzer_model::ReadOptions::default(),
            &rezzer_limits::Budget::new(limits),
        );
        let notes: Vec<String> = out
            .document
            .body
            .iter()
            .filter_map(|b| match b {
                Block::Section { blocks, .. } => Some(blocks),
                _ => None,
            })
            .flatten()
            .filter_map(|b| match b {
                Block::Section {
                    anchor: Some(a), ..
                } => a.0.strip_prefix("fn:").map(str::to_string),
                _ => None,
            })
            .collect();
        let refs = references(&out);
        any |= !refs.is_empty();
        for r in refs {
            assert!(notes.contains(&r), "{r} without its footnote: {notes:?}");
        }
    }
    assert!(any, "no limit kept a reference");
}

/// A legend entry no marker above points to stays a paragraph with the role `footnote` and
/// its marker: a Markdown viewer would hide a footnote definition without a reference.
#[test]
fn footnote_without_reference_is_a_paragraph() {
    let template = common::show("FL", 9.0, 40.0, 700.0, "Betrag* 1.234,56")
        + &common::show(
            "FL",
            9.0,
            40.0,
            688.0,
            "Die Ware wird wie vereinbart geliefert.",
        )
        + &common::show("FL", 6.0, 40.0, 60.0, "* steuerfrei")
        + &common::show("FL", 6.0, 40.0, 52.0, "** ohne Verweis");
    let page = FormPage {
        template,
        ..FormPage::default()
    };
    let out = read(&print_form(&[page], "", ""));
    let blocks = page_blocks(&out);
    assert_eq!(
        footnote_texts(&out),
        [("p1-star".to_string(), "steuerfrei".to_string())],
        "{blocks:#?}"
    );
    let plain: Vec<String> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph {
                inlines,
                role: Some(r),
                ..
            } if r == "footnote" => Some(plain_text(inlines)),
            _ => None,
        })
        .collect();
    assert_eq!(plain, ["** ohne Verweis"], "{blocks:#?}");
    let note = found_by(&out);
    assert!(note[0].contains("1 of them are linked"), "{note:?}");
}

// ---------------------------------------------------------------------------------------
// Labels and values of drawn boxes and of rows of fields

/// The note before the first table of boxed fields of a page.
const BOX_NOTE: &str = "Found by: label and value pairs from drawn boxes; the label is the \
     smaller text at the top of each box.";

/// Three boxes in a row (touching, 120 by 24 points from x 40, 160 and 280 at y 700) with a
/// label of 6 points at the top of each, and `values` on one data line padded into the boxes
/// (an empty value leaves its box with only its label). Three lines of an address in the
/// data font elsewhere make the data font a text size of the page. `third` replaces the
/// third box's label when given (template content).
fn boxed_fields(values: [&str; 3], third: Option<String>) -> FormPage {
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
    assert_no_loss(&out);
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
fn field_rows() -> FormPage {
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
    let losses: Vec<&str> = out
        .document
        .losses
        .iter()
        .filter(|l| l.why == LossReason::Uncertain && l.severity == Severity::Info)
        .map(|l| l.what.as_str())
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
    assert_no_loss(&out);
}

/// A framed box of a pay slip with a bold title of 6.66 points at its top left and two lines
/// of the data font in it (a date and an amount); `title_box` draws the title in a box of its
/// own inside the frame's top, so the frame holds a box with text and is no region itself:
/// its lines are read later, in the page's order. Beside it, boxed fields and text elsewhere
/// on the page.
fn titled_box(title_box: bool) -> FormPage {
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

/// Lines of plain `size` pt prose in Helvetica from `top` down, at least 600 ink characters,
/// so that `size` is the body size.
fn prose(size: f32, top: f32) -> String {
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
fn headed(head: &str, size: f32, scale: f32, info: &str) -> Vec<u8> {
    let page = FormPage {
        template: format!("{head}{}", prose(size, 450.0)),
        scale,
        ..FormPage::default()
    };
    print_form(&[page], info, "")
}

/// The value of the frontmatter key `key` of the Markdown of `out`, if it is there.
fn front(out: &ReadOutcome, key: &str) -> Option<String> {
    let md = common::markdown(out);
    let head = md.split("\n---\n").next().unwrap_or("").to_string();
    head.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(": "))
        .map(str::to_string)
}

/// The words of the losses of `out` that are `Uncertain`.
fn unsure(out: &ReadOutcome) -> Vec<&str> {
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
fn only_date_in_the_head_is_document_date() {
    let head = common::show("FL", 9.0, 400.0, 760.0, "Musterstadt, 01.02.2026");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    let found = out
        .document
        .properties
        .document_date
        .as_ref()
        .expect("date");
    assert_eq!(found.date, "2026-02-01");
    assert_eq!(found.from, "only date in the top third of page 1");
    // The same date twice is still one date.
    let head = head + &common::show("FL", 9.0, 40.0, 700.0, "Stand 01.02.2026");
    let out = read(&headed(&head, 9.0, 1.0, ""));
    assert_eq!(
        out.document
            .properties
            .document_date
            .as_ref()
            .map(|d| d.date.as_str()),
        Some("2026-02-01")
    );
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
    // `Due date` is no date label: the only date in the head is the document date.
    for label in ["Due date", "Delivery date", "Start Date:"] {
        let head = common::show("FL", 9.0, 400.0, 760.0, "Musterstadt, 01.02.2026")
            + &common::show("FL", 9.0, 40.0, 200.0, &format!("{label} 15.03.2026"));
        let out = read(&headed(&head, 9.0, 1.0, ""));
        assert_eq!(
            out.document
                .properties
                .document_date
                .as_ref()
                .map(|d| d.date.as_str()),
            Some("2026-02-01"),
            "{label}"
        );
    }
}

#[test]
fn other_named_dates_are_no_document_date() {
    // A date with a label of its own that names another date: never the document date, also
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
