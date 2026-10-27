// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The evidence at both ends of a grid of fields, cell by cell (ruling P6-Z2): for every pair
//! of ends a grid of labels over values and a grid of labels under values that have them,
//! with one and with two fields a row, the first or the last row empty or full, in every look.
//! Each read holds the invariants and the reading the table of ends gives: the fields pair,
//! or they stay text with a loss that names them.

use crate::check::{Broken, Inv, check_fields, read_pages, unexpected};
use crate::shapes::{Built, FieldSpec, Geo, REAL, ROWS, Rows, rows_page};
use crate::{draw, looks};

/// What stands over a grid's top rule: nothing, values at some of its first fields' starts
/// (or a run at no field start), or a value at every one.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Top {
    None,
    Partial,
    Complete,
}

/// What stands under a grid's last rule: nothing, labels that head some of its lowest values
/// only (or none), a label for every lowest value beyond the grid, or a row of labels only of
/// the grid itself (an empty last row, or labels over a rule drawn under them).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Bottom {
    None,
    Partial,
    Free,
    Own,
}

/// How the grid is read.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Outcome {
    /// Labels over values.
    Over,
    /// Labels under the rule their values stand on.
    Under,
    /// Neither: a loss names the fields.
    Unclear,
}

/// The table of ends (ruling P6-Z2), as the reader keeps it: two fields a row or more read
/// labels under values with both ends complete, are not clear with one complete end (an own
/// row of labels counts only with a complete top), and read labels over values with none;
/// one field a row never reads labels under values, and is not clear with any complete end.
/// Evidence in part at both ends from loose text, as every layout of the matrix has it, keeps
/// labels over values (it gives a loss only when other field groups joined by rules make it).
fn table(single: bool, top: Top, bottom: Bottom) -> Outcome {
    match (single, top, bottom) {
        (true, Top::Complete, _) | (true, _, Bottom::Free) => Outcome::Unclear,
        (true, _, _) => Outcome::Over,
        (false, Top::Complete, Bottom::Free) => Outcome::Under,
        (false, Top::Complete, _) | (false, _, Bottom::Free) => Outcome::Unclear,
        (false, _, _) => Outcome::Over,
    }
}

/// Rows of two fields and of one field (each row a list of fields), with the first or the
/// last row empty.
fn rows(single: bool, first_empty: bool, last_empty: bool) -> Vec<Vec<FieldSpec>> {
    let full: [[FieldSpec; 2]; 3] = [
        [
            (0, 30, "Name", "Erika Mustermann"),
            (30, 20, "Ort", "Musterstadt"),
        ],
        [(0, 30, "Strasse", "Musterweg 1"), (30, 20, "Hausnr", "12")],
        [(0, 30, "PLZ", "12345"), (30, 20, "Land", "DE")],
    ];
    full.iter()
        .enumerate()
        .map(|(k, row)| {
            let empty = (k == 0 && first_empty) || (k == 2 && last_empty);
            let fields: Vec<FieldSpec> = if single {
                vec![(0, 50, row[0].2, row[0].3)]
            } else {
                row.to_vec()
            };
            fields
                .into_iter()
                .map(|f| if empty { (f.0, f.1, f.2, "") } else { f })
                .collect()
        })
        .collect()
}

/// A layout of the matrix: the grid's fields, which way its labels stand, its empty rows and
/// what stands beyond it, with the ends the reader sees.
#[derive(Debug, Clone, Copy)]
struct Layout {
    single: bool,
    under: bool,
    first_empty: bool,
    last_empty: bool,
    /// Labels under values: the first row's first value only is empty.
    half_first: bool,
    /// Over the top rule (labels over values; labels under values with an empty first row:
    /// a run inside the first field, not at its start).
    over: Top,
    /// Under the last rule: column titles (labels over values), or a rule under the last
    /// labels (labels under values, `Bottom::Own`).
    beyond: Bottom,
    /// Labels under values: the value of the second row's first field has two lines, so its
    /// strip is no line of labels over one line of values. (Over labels in a row of one pitch
    /// a second value line has no room: the variant is drawn for labels under values only.)
    two_line: bool,
}

impl Layout {
    /// The ends the reader sees.
    fn ends(&self) -> (Top, Bottom) {
        if self.under {
            let top = if self.first_empty {
                self.over
            } else if self.half_first {
                Top::Partial
            } else {
                Top::Complete
            };
            // An empty last row leaves a row of labels only over the last labels.
            let bottom = if self.last_empty {
                Bottom::Own
            } else {
                self.beyond
            };
            (top, bottom)
        } else {
            let bottom = if self.last_empty {
                Bottom::Own
            } else {
                self.beyond
            };
            (self.over, bottom)
        }
    }

    /// How the table reads it: a strip of another shape reads labels under values never, and
    /// labels over values only without evidence at either end.
    fn outcome(&self) -> Outcome {
        let (top, bottom) = self.ends();
        match table(self.single, top, bottom) {
            _ if self.two_line && top == Top::None && bottom == Bottom::None => Outcome::Over,
            _ if self.two_line => Outcome::Unclear,
            o => o,
        }
    }

    fn name(&self) -> String {
        format!(
            "{}{}, labels {}, first {}, last {}, over {:?}, beyond {:?}",
            if self.single {
                "one field"
            } else {
                "two fields"
            },
            if self.two_line {
                ", a value of two lines"
            } else {
                ""
            },
            if self.under { "under" } else { "over" },
            if self.half_first {
                "half empty"
            } else if self.first_empty {
                "empty"
            } else {
                "full"
            },
            if self.last_empty { "empty" } else { "full" },
            self.over,
            self.beyond,
        )
    }

    /// The rows of fields drawn.
    fn rows(&self) -> Vec<Vec<FieldSpec>> {
        let mut rows = rows(self.single, self.first_empty, self.last_empty);
        if self.half_first {
            rows[0][0].3 = "";
        }
        rows
    }

    /// The page in geometry `g`, with what to expect of each field when the grid reads
    /// `outcome`.
    fn page(&self, g: &Geo, outcome: Outcome) -> Built {
        let rows = self.rows();
        let specs: Vec<&[FieldSpec]> = rows.iter().map(|r| r.as_slice()).collect();
        let s = Rows {
            under: self.under,
            dividers: !self.single,
            ..ROWS
        };
        let all: Vec<&str> = rows.iter().flatten().map(|f| f.2).collect();
        let unclear: &[&str] = if outcome == Outcome::Unclear {
            &all
        } else {
            &[]
        };
        let mut b = rows_page(g, &s, &specs, unclear);
        let (d, h) = (g.data, 2.4 * g.data);
        let top = 760.0 + g.dy;
        let last = top - h * if self.under { 2.0 } else { 3.0 };
        let starts: Vec<usize> = rows[0].iter().map(|f| f.0).collect();
        let text = |b: &mut Built, k: f32, y: f32, size: f32, t: &'static str, value: bool| {
            b.page.template += &draw::courier(g.col(0) + k * g.pitch(), y, size, t);
            if value {
                b.values.push(t.to_string());
            }
        };
        let words = ["Lohn", "Okt"];
        match self.over {
            Top::None => {}
            Top::Complete => {
                for (&k, w) in starts.iter().zip(words) {
                    text(&mut b, k as f32 + 0.3, top + 0.17 * d, d, w, true);
                }
            }
            // At the first field's start of several, or inside the only field.
            Top::Partial if self.single || self.under => {
                text(&mut b, 10.3, top + 0.17 * d, d, words[0], true)
            }
            Top::Partial => text(&mut b, 0.3, top + 0.17 * d, d, words[0], true),
        }
        let label = 0.18 * g.label / g.pitch();
        match self.beyond {
            Bottom::None => {}
            Bottom::Free if !self.under => {
                for (&k, t) in starts.iter().zip(["Lohnart", "Betrag"]) {
                    text(&mut b, k as f32 + label, last - 0.86 * d, g.label, t, false);
                }
            }
            // At no value's offset with one field, at the first field's start of two.
            Bottom::Partial => {
                let k = if self.single { 8.0 } else { 0.0 };
                text(
                    &mut b,
                    k + label,
                    last - 0.86 * d,
                    g.label,
                    "Lohnart",
                    false,
                );
            }
            // One row pitch under the last rule and as long as the grid's rules: the page of
            // a grid of labels over values with an empty last row (see [`Layout::twin`]).
            Bottom::Own => {
                b.page.template += &g.hrule(g.col(0) - 0.12, g.col(50), last - h);
            }
            Bottom::Free => {}
        }
        if self.two_line {
            // Over the second row's first value, which stands on the second rule.
            let y = top - h + 0.17 * d + 0.75 * d;
            text(&mut b, 0.3, y, d, "Hinterhaus", true);
        }
        b
    }

    /// For a grid of labels under values with a rule under its last labels: the same page
    /// drawn as a grid of labels over values whose last row is empty, its first row's values
    /// a line over its top rule. None for the other layouts.
    fn twin(&self, g: &Geo) -> Option<Built> {
        if !(self.under && self.beyond == Bottom::Own && !self.last_empty) {
            return None;
        }
        let rows = self.rows();
        let shifted: Vec<Vec<FieldSpec>> = (0..3)
            .map(|k| {
                rows[k]
                    .iter()
                    .enumerate()
                    .map(|(n, f)| {
                        let value = rows.get(k + 1).map_or("", |r| r[n].3);
                        (f.0, f.1, f.2, value)
                    })
                    .collect()
            })
            .collect();
        let specs: Vec<&[FieldSpec]> = shifted.iter().map(|r| r.as_slice()).collect();
        let s = Rows {
            dividers: !self.single,
            ..ROWS
        };
        let mut b = rows_page(g, &s, &specs, &[]);
        let (d, h) = (g.data, 2.4 * g.data);
        let top = 760.0 + g.dy;
        for f in rows[0].iter().filter(|f| !f.3.is_empty()) {
            b.page.template += &draw::courier(g.col(f.0) + 0.3 * g.pitch(), top + 0.17 * d, d, f.3);
        }
        if self.over == Top::Partial && self.first_empty {
            b.page.template +=
                &draw::courier(g.col(0) + 10.3 * g.pitch(), top + 0.17 * d, d, "Lohn");
        }
        if self.two_line {
            let y = top - h + 0.17 * d + 0.75 * d;
            b.page.template += &draw::courier(g.col(0) + 0.3 * g.pitch(), y, d, "Hinterhaus");
        }
        Some(b)
    }
}

/// Every layout of the matrix that a read can be held to. Left out, with why:
/// - labels over values with a complete top and a free complete bottom, two fields a row: its
///   page is a grid of labels under values with both ends, which the table reads so;
/// - labels under values with an own bottom and no complete top, two fields a row: its page is
///   a grid of labels over values with an empty last row, which the table reads so;
/// - labels under values, one field a row, with no complete end: the same page as a column of
///   labels over values with an empty last row.
///
/// Cells no layout gives: a partial top over labels under values with a full first row (its
/// first row fills the top), and a partial bottom under labels under values (its last labels
/// are there or the grid ends otherwise).
fn candidates() -> Vec<Layout> {
    let mut out = Vec::new();
    for single in [false, true] {
        for first_empty in [false, true] {
            for last_empty in [false, true] {
                for over in [Top::None, Top::Partial, Top::Complete] {
                    let beyonds: &[Bottom] = if last_empty {
                        &[Bottom::None]
                    } else {
                        &[Bottom::None, Bottom::Partial, Bottom::Free]
                    };
                    for &beyond in beyonds {
                        out.push(Layout {
                            single,
                            under: false,
                            first_empty,
                            last_empty,
                            half_first: false,
                            over,
                            beyond,
                            two_line: false,
                        });
                    }
                }
            }
        }
        // Labels under values: the first row full, half empty (two fields) or empty (with
        // nothing over it, or a run inside its first field); the last row full with nothing
        // under it or a rule under the last labels, or empty.
        let firsts: Vec<(bool, bool, Top)> = if single {
            vec![
                (false, false, Top::None),
                (true, false, Top::None),
                (true, false, Top::Partial),
            ]
        } else {
            vec![
                (false, false, Top::None),
                (false, true, Top::None),
                (true, false, Top::None),
                (true, false, Top::Partial),
            ]
        };
        for (first_empty, half_first, over) in firsts {
            for (last_empty, beyond) in [
                (false, Bottom::Free),
                (false, Bottom::Own),
                (true, Bottom::None),
            ] {
                for two_line in [false, true] {
                    out.push(Layout {
                        single,
                        under: true,
                        first_empty,
                        last_empty,
                        half_first,
                        over,
                        beyond,
                        two_line,
                    });
                }
            }
        }
    }
    out
}

/// The layouts of [`candidates`] a read can be held to, each with the reading the table gives.
fn layouts() -> Vec<(Layout, Outcome)> {
    candidates()
        .into_iter()
        .filter_map(|l| {
            let outcome = l.outcome();
            let truth = if l.under {
                Outcome::Under
            } else {
                Outcome::Over
            };
            // A reading of the other way is the page of a grid of the other way: left out.
            (outcome == Outcome::Unclear || outcome == truth).then_some((l, outcome))
        })
        .collect()
}

/// Every layout of the matrix in the real look and each of [`looks`] holds the invariants and
/// reads as the table of ends says.
#[test]
fn every_cell_of_the_ends_reads_as_the_table_says() {
    let mut reads = Vec::new();
    for (l, outcome) in layouts() {
        for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
            let b = l.page(&g, outcome);
            let out = read_pages(&[&b]);
            let broken: Vec<Broken> = check_fields(&out, &b);
            reads.push((format!("{}, {look}", l.name()), broken));
        }
    }
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The lone upright rule far from the fields that [`rows_page`] draws on a page of grids of one
/// field a row (they have no dividers), so that the page has a rule of each direction.
fn lone_vertical(g: &Geo) -> String {
    g.vrule(30.0 + g.dx, 200.0 + g.dy, 300.0 + g.dy)
}

/// Every layout of the matrix of one field a row, drawn without the lone upright rule: a page
/// of horizontal rules alone. It holds the invariants and reads as the table of ends says, as
/// it does with that rule.
#[test]
fn every_cell_of_one_field_a_row_reads_so_without_an_upright_rule() {
    let mut reads = Vec::new();
    let mut count = 0;
    for (l, outcome) in layouts().into_iter().filter(|(l, _)| l.single) {
        for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
            let mut b = l.page(&g, outcome);
            let lone = lone_vertical(&g);
            assert!(b.page.template.contains(&lone), "{}", l.name());
            b.page.template = b.page.template.replace(&lone, "");
            let out = read_pages(&[&b]);
            reads.push((format!("{}, {look}", l.name()), check_fields(&out, &b)));
            count += 1;
        }
    }
    assert_eq!(count, 38 * 6);
    let wrong = unexpected(reads, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Reads `layouts` of the matrix with `extra` drawn on each page (in geometry `g`, from its
/// top rule), in every look: no field pairs wrongly, every value stands once, and each field
/// with a value pairs right or a loss names it. An empty field left as text loses no value:
/// it need not be named.
fn labels_under_with(
    what: &str,
    layouts: Vec<Layout>,
    extra: fn(&Geo, f32) -> String,
) -> Vec<String> {
    let mut reads = Vec::new();
    for l in layouts {
        for (look, g) in std::iter::once(("real", REAL)).chain(looks()) {
            let mut b = l.page(&g, Outcome::Under);
            b.page.template += &extra(&g, 760.0 + g.dy);
            let out = read_pages(&[&b]);
            // As `check_fields` looks for a name in a loss.
            let named = |n: &str| {
                out.document
                    .losses
                    .iter()
                    .any(|l| l.what.contains(&format!(" {n}")))
            };
            let empty = |n: &str| b.fields.iter().any(|f| f.0 == n && f.1 == Ok(None));
            let broken: Vec<Broken> = check_fields(&out, &b)
                .into_iter()
                .filter(|x| {
                    let field = x.1.strip_suffix(" does not pair");
                    !(x.0 == Inv::ClearPairs && field.is_some_and(|n| named(n) || empty(n)))
                })
                .collect();
            reads.push((format!("{}, {what}, {look}", l.name()), broken));
        }
    }
    unexpected(reads, &[])
}

/// Every labels-under layout of the matrix split by a double rule: a second rule a third of
/// the data size under its second rule, farther than the rules of one line are apart, so the
/// grid falls into two groups whose ends lie in each other. The runs one group holds are
/// evidence for the other one only toward a loss.
#[test]
fn a_labels_under_grid_split_by_a_double_rule_never_pairs_shifted() {
    let layouts: Vec<Layout> = candidates().into_iter().filter(|l| l.under).collect();
    assert_eq!(layouts.len(), 42);
    let wrong = labels_under_with("split", layouts, |g, top| {
        g.hrule(
            g.col(0) - 0.12,
            g.col(50),
            top - 2.4 * g.data - g.data / 3.0,
        )
    });
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The labels-under layouts the matrix holds under a section rule one row pitch over their
/// top rule, that frame boxed into two cells by upright lines whose divider is not in line with
/// the grid's: the box's grid takes the top rule, so the grid's first row is text between two
/// rules, and what the box holds is a table's. Runs a table holds, and a row between two
/// rules, count toward a loss only. A left-out layout is the page of a grid of the other way,
/// and the box adds nothing to it. The grid's first row is still a row of fields under the
/// box's lowest rule, so a grid whose last row is labels only is read too (and not clear).
#[test]
fn a_labels_under_grid_under_a_boxed_section_never_pairs_shifted() {
    let layouts: Vec<Layout> = layouts()
        .into_iter()
        .map(|l| l.0)
        .filter(|l| l.under)
        .collect();
    assert_eq!(layouts.len(), 32);
    let wrong = labels_under_with("boxed", layouts, |g, top| {
        let (h, x0, x1) = (2.4 * g.data, g.col(0) - 0.12, g.col(50));
        let mut c = g.hrule(x0, x1, top + h);
        for x in [x0, g.col(20), x1] {
            c += &g.vrule(x, top, top + h);
        }
        c
    });
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A grid of labels under values with a rule under its last labels is the page of a grid of
/// labels over values with an empty last row (see [`Layout::twin`]): both read the same, in
/// every look with hanging dividers, so the table cannot tell them apart and reads that page
/// one way.
#[test]
fn a_rule_under_the_last_labels_reads_as_its_twin() {
    let mut differ = Vec::new();
    // Dividers that rise from the lower rule rise from the other rule of a row in the two
    // drawings, so those pages are not the same; every other look is.
    let same = looks().into_iter().filter(|(_, g)| !g.rising);
    for l in candidates() {
        for (look, g) in std::iter::once(("real", REAL)).chain(same.clone()) {
            let Some(twin) = l.twin(&g) else {
                continue;
            };
            let page = l.page(&g, l.outcome());
            let (a, b) = (read_pages(&[&page]), read_pages(&[&twin]));
            let read = |o: &rezzer_model::ReadOutcome| {
                let losses: Vec<String> =
                    o.document.losses.iter().map(|l| l.what.clone()).collect();
                (crate::common::text(crate::check::blocks(o)), losses)
            };
            if read(&a) != read(&b) {
                differ.push(format!(
                    "{}, {look}:\n{:#?}\n{:#?}",
                    l.name(),
                    read(&a),
                    read(&b)
                ));
            }
        }
    }
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

/// The layouts left out of the matrix (see [`candidates`]), by name.
const LEFT_OUT: [&str; 12] = [
    "one field, labels under, first empty, last empty, over None, beyond None",
    "one field, labels under, first empty, last empty, over Partial, beyond None",
    "one field, labels under, first empty, last full, over None, beyond Own",
    "one field, labels under, first empty, last full, over Partial, beyond Own",
    "two fields, labels over, first empty, last full, over Complete, beyond Free",
    "two fields, labels over, first full, last full, over Complete, beyond Free",
    "two fields, labels under, first empty, last empty, over None, beyond None",
    "two fields, labels under, first empty, last empty, over Partial, beyond None",
    "two fields, labels under, first empty, last full, over None, beyond Own",
    "two fields, labels under, first empty, last full, over Partial, beyond Own",
    "two fields, labels under, first half empty, last empty, over None, beyond None",
    "two fields, labels under, first half empty, last full, over None, beyond Own",
];

/// The matrix is pinned: its size, its layouts per outcome, the cells it reaches and the
/// layouts it leaves out, so that no change drops a layout without a failing test. With
/// `--no-capture` it prints them for the report.
#[test]
fn the_matrix_is_pinned() {
    let all = layouts();
    let count = |o: Outcome| all.iter().filter(|(_, x)| *x == o).count();
    let mut cells: Vec<(bool, Top, Bottom)> = Vec::new();
    for (l, _) in &all {
        let (t, b) = l.ends();
        if !cells.contains(&(l.single, t, b)) {
            cells.push((l.single, t, b));
        }
    }
    for c in &cells {
        eprintln!("ENDS cell {c:?}: {:?}", table(c.0, c.1, c.2));
    }
    let mut left: Vec<String> = candidates()
        .iter()
        .map(Layout::name)
        .filter(|n| !all.iter().any(|(k, _)| k.name() == *n))
        .collect();
    left.sort();
    eprintln!(
        "ENDS {} layouts of {} candidates, {} cells: Over {}, Under {}, Unclear {}",
        all.len(),
        candidates().len(),
        cells.len(),
        count(Outcome::Over),
        count(Outcome::Under),
        count(Outcome::Unclear)
    );
    assert_eq!(
        (
            candidates().len(),
            all.len(),
            count(Outcome::Over),
            count(Outcome::Under),
            count(Outcome::Unclear),
            cells.len()
        ),
        (90, 78, 24, 1, 53, 24)
    );
    assert_eq!(left, LEFT_OUT);
}
