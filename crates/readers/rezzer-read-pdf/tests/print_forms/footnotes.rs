// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Footnotes and legends of forms, and their references.

use super::*;

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
pub(crate) fn footnote_texts(out: &ReadOutcome) -> Vec<(String, String)> {
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
pub(crate) fn references(out: &ReadOutcome) -> Vec<String> {
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
pub(crate) fn referenced(x: f32, y: f32, text: &str, number: &str) -> String {
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
pub(crate) fn label_marked(x: f32, y: f32, text: &str, number: &str, gap: f32) -> String {
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
pub(crate) fn header_cells(out: &ReadOutcome) -> Vec<Vec<Inline>> {
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
