// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The evidence at both ends of a grid of fields, cell by cell (ruling P6-Z2): for every pair
//! of ends a grid of labels over values and a grid of labels under values that have them,
//! with one and with two fields a row, the first or the last row empty or full, in every look.
//! Each read holds the invariants and the reading the table of ends gives: the fields pair,
//! or they stay text with a loss that names them.

use crate::check::{Broken, check_fields, read_pages, unexpected};
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

    fn name(&self) -> String {
        format!(
            "{}, labels {}, first {}, last {}, over {:?}, beyond {:?}",
            if self.single {
                "one field"
            } else {
                "two fields"
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

    /// The page in geometry `g`, with what to expect of each field when the grid reads
    /// `outcome`.
    fn page(&self, g: &Geo, outcome: Outcome) -> Built {
        let mut rows = rows(self.single, self.first_empty, self.last_empty);
        if self.half_first {
            rows[0][0].3 = "";
        }
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
            Bottom::Own => {
                b.page.template += &g.hrule(g.col(0) - 0.6 * d, g.col(70), last - 1.5 * h);
            }
            Bottom::Free => {}
        }
        b
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
                out.push(Layout {
                    single,
                    under: true,
                    first_empty,
                    last_empty,
                    half_first,
                    over,
                    beyond,
                });
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
            let (top, bottom) = l.ends();
            let outcome = table(l.single, top, bottom);
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

/// The size of the matrix and its cells per outcome, for the report.
#[test]
fn the_matrix_covers_every_outcome() {
    let all = layouts();
    for o in [Outcome::Over, Outcome::Under, Outcome::Unclear] {
        let n = all.iter().filter(|(_, x)| *x == o).count();
        assert!(n > 0, "{o:?}");
        eprintln!("ENDS {o:?}: {n} layouts");
    }
    let mut cells: Vec<(bool, Top, Bottom)> = Vec::new();
    for (l, _) in &all {
        let (t, b) = l.ends();
        if !cells.contains(&(l.single, t, b)) {
            cells.push((l.single, t, b));
        }
    }
    eprintln!(
        "ENDS all: {} layouts of {} candidates, {} cells",
        all.len(),
        candidates().len(),
        cells.len()
    );
    for c in &cells {
        eprintln!("ENDS cell {c:?}: {:?}", table(c.0, c.1, c.2));
    }
}
