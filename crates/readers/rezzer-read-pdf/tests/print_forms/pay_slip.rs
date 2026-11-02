// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The pay slip shape: header field grid, sections, totals and refused sections.

use super::*;

/// A rule drawn as a filled bar of thickness `t` from `x0` to `x1`, its top at `y`.
pub(crate) fn bar_h(x0: f32, x1: f32, y: f32, t: f32) -> String {
    format!("{x0} {} {} {t} re f\n", y - t, x1 - x0)
}

/// A rule drawn as a filled bar of thickness `t` from `y0` to `y1`, centred on `x`.
pub(crate) fn bar_v(x: f32, y0: f32, y1: f32, t: f32) -> String {
    format!("{} {y0} {t} {} re f\n", x - t / 2.0, y1 - y0)
}

/// A separator mark of `h` points from `y0`, centred on `x`, filled and stroked with a
/// hairline as the print run draws it.
pub(crate) fn mark_b(x: f32, y0: f32, h: f32) -> String {
    let (l, r, top) = (x - 0.12, x + 0.12, y0 + h);
    format!("q 0 w {r} {top} m {l} {top} l {l} {y0} l {r} {y0} l h B* Q\n")
}

/// A bold template label in Helvetica-Bold (`/FB`).
pub(crate) fn bold(x: f32, y: f32, size: f32, text: &str) -> String {
    format!("BT /FB {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// A data line of the pay slip: each text placed at its character column.
pub(crate) fn columns_line(y: f32, parts: &[(usize, &str)]) -> DataLine {
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
pub(crate) fn column_at(x: f32) -> usize {
    ((x - 42.52) / PITCH).round() as usize
}

/// A field of a grid row: the x of its left edge, its label and its value.
pub(crate) type GridField = (f32, &'static str, &'static str);

/// The rows of the pay slip's header field grid: the height of each row's top rule, and for
/// each field of its left part the x of its left edge, its label and its value (empty for a
/// field left empty). A field `Steuerklasse` carries a raised footnote marker.
pub(crate) const HEADER_GRID: [(f32, [GridField; 4]); 3] = [
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
pub(crate) fn header_field_grid() -> (String, Vec<DataLine>) {
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
pub(crate) fn pay_slip(stroked_marks: bool) -> FormPage {
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
pub(crate) fn read_header_rows(out: &ReadOutcome) -> Vec<usize> {
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
pub(crate) fn values_alone(out: &ReadOutcome) -> Vec<String> {
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
pub(crate) fn assert_pay_slip(out: &ReadOutcome) {
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
/// number of points measured on one form) reads the same. At 1.5 the drawn marks are 3.24
/// points long: a mark is measured by the size of the digits it stands between, so they are
/// still marks.
#[test]
fn pay_slip_printed_smaller_or_larger_reads_the_same() {
    for scale in [0.7, 1.5] {
        let page = FormPage {
            scale,
            ..pay_slip(true)
        };
        assert_pay_slip(&read(&print_form(&[page], "", "")));
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
