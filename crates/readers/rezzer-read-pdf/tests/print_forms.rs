// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Print-run forms: a form template with a data layer of padded fixed-pitch lines, built in
//! code with invented values only.

mod common;

use common::{DataLine, FormPage, all_text, data_line, label, print_form, read};
use rezzer_model::{Completeness, LossReason, ReadOutcome, Severity};

/// The allocator of the Rezzer binary, so the reads run as they run in it.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

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

/// A header band whose rows are set in text much smaller than the page's body text: refused
/// without a note, and that refusal is a loss.
#[test]
fn band_refused_for_tiny_text_is_a_loss() {
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
        "2 grids of drawn rules on page 1 are not read as tables: 2 (most of their cells are \
         empty); their text is read without the grids"
    );
    let text = all_text(&out);
    assert!(text.contains("Erika Mustermann"), "{text}");
}
