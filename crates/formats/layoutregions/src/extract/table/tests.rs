// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use super::grid::{Piece, Shape, grid_cells, pieces_of, title_cut};
use super::ruled::{GridPart, RuledIn, column_cuts, ruled_cells};
use super::*;
use crate::input::{FontKind, GapIn, Shift, TextRun};
use crate::segment::drawn::Line;

fn seg(at: f32, from: f32, to: f32) -> Line {
    Line { at, from, to }
}

/// A run of `text` at `x` on the baseline `y` in the size 10, 6 points a character, on line
/// `line`, with a gap at each stretch of spaces.
fn run_at(text: &str, x: f32, y: f32, line: u32) -> TextRun {
    let mut gaps = Vec::new();
    let mut words = 0u32;
    let mut prev = ' ';
    for (k, c) in text.char_indices() {
        if c != ' ' && prev == ' ' && k > 0 {
            gaps.push(GapIn {
                words,
                from: x + 6.0 * (text[..k].trim_end().chars().count()) as f32,
                to: x + 6.0 * k as f32,
            });
        }
        if c != ' ' && prev == ' ' {
            words += 1;
        }
        prev = c;
    }
    TextRun {
        text: text.into(),
        x,
        y,
        w: 6.0 * text.chars().count() as f32,
        h: 10.0,
        size: 10.0,
        font: 0,
        font_kind: FontKind::Unknown,
        bold: false,
        italic: false,
        color: None,
        shift: Shift::Base,
        line,
        off_axis: false,
        gaps,
    }
}

/// A run of `text` on the baseline 100 at `x`, on line 0.
fn spaced(text: &str, x: f32) -> TextRun {
    run_at(text, x, 100.0, 0)
}

/// A grid of one row from 90 to 120 over the column edges `xs`, its rules drawn the full
/// height.
fn one_row(xs: &[f32]) -> (Vec<Line>, Vec<Line>) {
    let (left, right) = (xs[0], xs[xs.len() - 1]);
    (
        vec![seg(90.0, left, right), seg(120.0, left, right)],
        xs.iter().map(|&x| seg(x, 90.0, 120.0)).collect(),
    )
}

/// The cuts [`column_cuts`] finds in `runs` for the grid of one row over `xs`, within
/// `budget` steps.
fn cuts_of(runs: &[TextRun], xs: &[f32], budget: usize) -> (Vec<(usize, Vec<usize>)>, bool) {
    let (h, v) = one_row(xs);
    let parts = [GridPart {
        h: &h,
        v: &v,
        cut: false,
    }];
    column_cuts(runs, &parts, &[], budget)
}

#[test]
fn a_run_is_alone_on_its_line_wherever_the_others_stand() {
    // The runs of line 0 stand apart in the list, with a run of line 1 between them.
    let mut runs = vec![spaced("AA", 0.0), spaced("BB", 50.0), spaced("CC", 100.0)];
    runs[1].line = 1;
    let per_line = texted_per_line(&runs);
    assert!(!alone_on_line(&per_line, &runs[0]));
    assert!(alone_on_line(&per_line, &runs[1]));
    assert!(!alone_on_line(&per_line, &runs[2]));
}

#[test]
fn a_column_rule_cuts_a_run_between_two_words() {
    // "AA  BB CC" from 0: AA ends at 12, BB starts at 24; the rule at 20 lies in the gap,
    // the first gap of the run.
    let (cuts, done) = cuts_of(&[spaced("AA  BB CC", 0.0)], &[-10.0, 20.0, 100.0], 100);
    assert!(done);
    assert_eq!(cuts, [(0, vec![0])]);
}

#[test]
fn a_column_rule_inside_a_word_cuts_nothing() {
    let (cuts, done) = cuts_of(&[spaced("AAAA BB", 0.0)], &[-10.0, 9.0, 100.0], 100);
    assert!(done);
    assert!(cuts.is_empty());
}

#[test]
fn a_column_rule_cuts_only_at_its_heights() {
    let mut run = spaced("AA  BB", 0.0);
    run.y = 200.0;
    let (cuts, done) = cuts_of(&[run], &[-10.0, 20.0, 100.0], 100);
    assert!(done);
    assert!(cuts.is_empty());
}

/// A row of three drawn cells cuts the lines under it at its inner edges, as far as its
/// body reaches: a line more than [`BODY_GAP`] line heights under the one over it is not
/// its body.
#[test]
fn a_band_cuts_only_the_lines_of_its_body() {
    for (y, cut) in [(80.0, true), (20.0, false)] {
        let mut run = spaced("AA  BB", 0.0);
        run.y = y;
        let (cuts, done) = cuts_of(&[run], &[-10.0, 20.0, 60.0, 100.0], 100);
        assert!(done);
        assert_eq!(!cuts.is_empty(), cut, "{y}");
    }
}

#[test]
fn cuts_past_the_budget_are_left_out() {
    let runs: Vec<TextRun> = (0..3).map(|_| spaced("AA  BB", 0.0)).collect();
    let (cuts, done) = cuts_of(&runs, &[-10.0, 20.0, 100.0], 2);
    assert!(!done);
    // The first two runs are cut, the third stays whole.
    assert_eq!(cuts, [(0, vec![0]), (1, vec![0])]);
}

#[test]
fn column_rules_across_a_title_are_looked_for_within_a_budget() {
    let vs = [seg(100.0, 0.0, 50.0), seg(200.0, 60.0, 90.0)];
    let mut work = 0;
    assert!(column_across(&vs, (50.0, 300.0), (10.0, 20.0), &mut work));
    assert!(!column_across(&vs, (50.0, 300.0), (52.0, 58.0), &mut work));
    assert!(!column_across(&vs, (150.0, 300.0), (10.0, 20.0), &mut work));
    // Out of steps: the answer is yes, so no line is taken as a title.
    let mut work = TITLE_WORK;
    assert!(column_across(&vs, (50.0, 300.0), (52.0, 58.0), &mut work));
    assert!(work > TITLE_WORK);
}

/// A drawn grid of three rows and four columns: column edges 0, 100, 200, 300, 400, row edges
/// 300, 280, 260, 240 from the top. The header row draws no rule at 200, so its second cell
/// spans two columns. The rules come with the grid's runs: a header, two rows of items and
/// an empty last cell in the first row of items.
struct Grid {
    h: Vec<Line>,
    v: Vec<Line>,
    xs: Vec<f32>,
    ys: Vec<f32>,
    runs: Vec<TextRun>,
}

fn grid(rows: Vec<Vec<(&str, f32)>>) -> Grid {
    let xs = vec![0.0, 100.0, 200.0, 300.0, 400.0];
    let ys = vec![300.0, 280.0, 260.0, 240.0];
    let h = ys.iter().map(|&y| seg(y, 0.0, 400.0)).collect();
    let mut v: Vec<Line> = xs
        .iter()
        .filter(|&&x| x != 200.0)
        .map(|&x| seg(x, 240.0, 300.0))
        .collect();
    v.push(seg(200.0, 240.0, 280.0));
    let mut runs = Vec::new();
    for (r, row) in rows.into_iter().enumerate() {
        let y = ys[r] - 14.0;
        for (text, x) in row {
            runs.push(run_at(text, x, y, r as u32));
        }
    }
    Grid { h, v, xs, ys, runs }
}

/// The header and the two rows of items of the grid, the second item's name at `name_x`
/// with the text `name`.
fn items(name: &'static str) -> Grid {
    grid(vec![
        vec![("Name", 5.0), ("Amounts", 105.0), ("Note", 305.0)],
        vec![("Muster GmbH", 5.0), ("1.234,56", 105.0), ("12,50", 205.0)],
        vec![
            (name, 5.0),
            ("2.000,00", 140.0),
            ("3,00", 205.0),
            ("ok", 305.0),
        ],
    ])
}

/// The cells of the grid `g`, every run of it in the section.
fn read(g: &Grid) -> Cells {
    let all: Vec<usize> = (0..g.runs.len()).collect();
    let per_line = texted_per_line(&g.runs);
    ruled_cells(
        &g.runs,
        &RuledIn {
            h: &g.h,
            v: &g.v,
            xs: &g.xs,
            ys: &g.ys,
            open: &[],
            mine: &all,
            within: &all,
            per_line: &per_line,
        },
        &|| false,
        &|| None,
        &mut |_| false,
    )
}

/// Each run of `table` by the cell it is in: (row, cell).
fn placed(table: &TableOut) -> HashMap<usize, (usize, usize)> {
    let mut out = HashMap::new();
    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            for &i in &cell.runs {
                assert!(out.insert(i, (r, c)).is_none(), "run {i} twice");
            }
        }
    }
    out
}

#[test]
fn every_prim_of_a_table_once() {
    let g = items("Erika Mustermann");
    let Cells::Table(table) = read(&g) else {
        panic!("a table");
    };
    let at = placed(&table);
    // Every run once, in the cell it stands in, none left out.
    assert!(table.unplaced.is_empty());
    assert_eq!(at.len(), g.runs.len());
    let spans: Vec<Vec<(u32, u32)>> = table
        .rows
        .iter()
        .map(|r| r.iter().map(|c| (c.row_span, c.col_span)).collect())
        .collect();
    assert_eq!(
        spans,
        [
            vec![(1, 1), (1, 2), (1, 1)],
            vec![(1, 1), (1, 1), (1, 1), (1, 1)],
            vec![(1, 1), (1, 1), (1, 1), (1, 1)],
        ]
    );
    let texts: Vec<Vec<String>> = table
        .rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| c.runs.iter().map(|&i| g.runs[i].text.as_str()).collect())
                .collect()
        })
        .collect();
    assert_eq!(
        texts,
        [
            vec!["Name", "Amounts", "Note"],
            vec!["Muster GmbH", "1.234,56", "12,50", ""],
            vec!["Erika Mustermann", "2.000,00", "3,00", "ok"],
        ]
    );
    // The header row is judged by its text once the cells are filled: no rule makes it one.
    assert_eq!(table.header_rows, 0);
    assert!(table.icons.is_empty());
}

#[test]
fn a_run_over_its_cell_side_into_white_space_goes_by_its_middle() {
    // The name runs from 5 to 119, over the rule at 100 inside a word (no cut there), into
    // white space: the amount of its row starts at 140.
    let g = items("Erika Mustermannsta");
    let Cells::Table(table) = read(&g) else {
        panic!("a table");
    };
    assert!(table.unplaced.is_empty());
    assert_eq!(placed(&table)[&6], (2, 0));
}

#[test]
fn unplaced_run_is_reported() {
    // Ruled: the name runs from 5 to 155, over the rule at 100 inside a word (where no rule
    // cuts it) and onto the amount at 140 on its line. It is in no cell, and the amount keeps
    // its own.
    let g = items("Erika Mustermannstadtteil");
    let part = GridPart {
        h: &g.h,
        v: &g.v,
        cut: false,
    };
    let (cuts, _) = column_cuts(&g.runs, &[part], &[], 1000);
    assert!(
        cuts.iter().all(|c| c.0 != 6),
        "no rule cuts the name: {cuts:?}"
    );
    let Cells::Table(table) = read(&g) else {
        panic!("a table");
    };
    assert_eq!(table.unplaced, [(6, RUNS_INTO_CELL)]);
    let at = placed(&table);
    assert!(!at.contains_key(&6));
    assert_eq!(at[&7], (2, 1));
    assert_eq!(at.len() + 1, g.runs.len());

    // Without rules: a line between two rows that runs over the gutter between two columns
    // is in no cell.
    let mut runs = Vec::new();
    let rows = [
        ["Name", "Qty", "Amount"],
        ["Muster GmbH", "2", "1.234,56"],
        ["Erika Mustermann", "1", "12,50"],
        ["Musterstadt", "3", "3,00"],
        ["Muster AG", "4", "2.000,00"],
    ];
    let mut line = 0u32;
    for (k, row) in rows.iter().enumerate() {
        let y = 700.0 - 20.0 * k as f32;
        for (text, x) in row.iter().zip([40.0, 200.0, 300.0]) {
            runs.push(run_at(text, x, y, line));
        }
        line += 1;
        if k == 2 {
            // A note under the second item, from the first column into the second.
            runs.push(run_at(
                "see the note below for the terms",
                40.0,
                y - 10.0,
                line,
            ));
            line += 1;
        }
    }
    let note = runs.iter().position(|r| r.text.starts_with("see")).unwrap();
    let mut pieces: Vec<Vec<Piece>> = Vec::new();
    let mut start = 0;
    while start < runs.len() {
        let end = start
            + runs[start..]
                .iter()
                .take_while(|r| r.line == runs[start].line)
                .count();
        pieces.push(pieces_of(&runs, start, end));
        start = end;
    }
    let table = grid_cells(&runs, &pieces, 3, false).expect("a table");
    assert_eq!(table.unplaced, [(note, RUNS_OVER_COLUMNS)]);
    let at = placed(&table);
    assert_eq!(at.len() + 1, runs.len());
    assert!(!at.contains_key(&note));
}

/// Task 17 D9: a sentence printed over a ruled row, in runs parted where its word gap falls on
/// a column rule (the rule at 100 between "wird" and "nach"), reads as a sentence across the
/// columns (`sentence_across`): its runs are in no cell, so the reader keeps the line as a
/// paragraph next to the table. The values of the rows above keep their cells.
#[test]
fn a_sentence_over_a_ruled_row_is_in_no_cell() {
    let g = grid(vec![
        vec![("Name", 5.0), ("Amounts", 105.0), ("Note", 305.0)],
        vec![("Muster GmbH", 5.0), ("1.234,56", 105.0), ("12,50", 205.0)],
        vec![
            ("die Ware wird", 22.0),
            ("nach dem Erhalt geliefert", 103.0),
        ],
    ]);
    let Cells::Table(table) = read(&g) else {
        panic!("a table");
    };
    assert_eq!(
        table.unplaced,
        [(6, SENTENCE_ACROSS), (7, SENTENCE_ACROSS)],
        "{table:#?}"
    );
    let at = placed(&table);
    assert_eq!(at.len() + 2, g.runs.len());
    assert_eq!(at[&3], (1, 0));
    assert_eq!(at[&4], (1, 1));
    // Its row held nothing else: it is no row of the table.
    assert_eq!(table.rows.len(), 2, "{table:#?}");
    // Two short titles in two cells of a row of more text, close enough to read as one piece:
    // the cells' own text, not a sentence printed over the row.
    let mut g = grid(vec![
        vec![("Name", 5.0), ("Amounts", 105.0), ("Note", 305.0)],
        vec![("Gezahlte Beträge", 4.0), ("Muster", 103.0)],
        vec![("Muster AG", 5.0), ("2.000,00", 105.0), ("3,00", 205.0)],
    ]);
    g.runs.push(run_at("im Jahr", 5.0, 256.0 + 14.0, 9));
    let Cells::Table(table) = read(&g) else {
        panic!("a table");
    };
    assert!(table.unplaced.is_empty(), "{table:#?}");
    // Values in the cells of a row are no sentence, wherever a run stands.
    let g = grid(vec![
        vec![("Name", 5.0), ("Amounts", 105.0), ("Note", 305.0)],
        vec![("Muster GmbH", 5.0), ("1.234,56", 105.0), ("12,50", 205.0)],
        vec![("Muster AG", 5.0), ("2.000,00", 105.0), ("3,00", 205.0)],
    ]);
    let Cells::Table(table) = read(&g) else {
        panic!("a table");
    };
    assert!(table.unplaced.is_empty(), "{table:#?}");
}

/// Task 19b: a figure's name and number alone in a cell beside other cells names a figure, it
/// captions none; a caption alone on its line, or with caption words in the runs that go on
/// from it, is one.
#[test]
fn a_figure_reference_in_a_cell_is_no_caption_inside() {
    use super::caption::caption_inside;
    let reference = [
        run_at("Abb. 3", 5.0, 100.0, 0),
        run_at("Muster", 105.0, 100.0, 0),
    ];
    assert!(!caption_inside(&reference));
    let alone = [
        run_at("Abb. 3", 5.0, 100.0, 0),
        run_at("Muster", 105.0, 80.0, 1),
    ];
    assert!(caption_inside(&alone));
    // "Abb. 3:" then its words in a run of their own, a word gap after it, then a cell.
    let split = [
        run_at("Abb. 3:", 5.0, 100.0, 0),
        run_at("Muster Teile", 50.0, 100.0, 0),
        run_at("4", 205.0, 100.0, 0),
    ];
    assert!(caption_inside(&split));
}

#[test]
fn a_header_row_is_judged_by_its_text() {
    let cell = |text: &str, bold: bool| {
        let mut r = spaced(text, 0.0);
        r.bold = bold;
        vec![r]
    };
    // A word over numbers.
    let rows = [
        vec![cell("Qty", false), cell("Name", false)],
        vec![cell("12", false), cell("Muster GmbH", false)],
    ];
    let view: Vec<Vec<&[TextRun]>> = rows
        .iter()
        .map(|r| r.iter().map(Vec::as_slice).collect())
        .collect();
    assert_eq!(header_of(&view), 1);
    // Words over words, the first row bold and the rest not.
    let rows = [
        vec![cell("Name", true), cell("Town", true)],
        vec![cell("Muster GmbH", false), cell("Musterstadt", false)],
    ];
    let view: Vec<Vec<&[TextRun]>> = rows
        .iter()
        .map(|r| r.iter().map(Vec::as_slice).collect())
        .collect();
    assert_eq!(header_of(&view), 1);
    // Words over words in one weight: no header.
    let rows = [
        vec![cell("Name", false), cell("Town", false)],
        vec![cell("Muster GmbH", false), cell("Musterstadt", false)],
    ];
    let view: Vec<Vec<&[TextRun]>> = rows
        .iter()
        .map(|r| r.iter().map(Vec::as_slice).collect())
        .collect();
    assert_eq!(header_of(&view), 0);
}

#[test]
fn close_titles_are_cut_between_runs_and_by_characters() {
    // Runs of size 8, 5 points a character, without glyph gaps.
    let run = |text: &str, x: f32| {
        let mut r = run_at(text, x, 600.0, 0);
        r.w = 5.0 * text.chars().count() as f32;
        (r.h, r.size, r.gaps) = (8.0, 8.0, Vec::new());
        r
    };
    let (left, right) = ((300.0, 305.0), (330.0, 345.0));
    let piece = |runs: &[TextRun]| Piece {
        x0: runs[0].x,
        x1: runs.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max),
        start: 0,
        end: runs.len(),
    };
    // Two runs: between them, from the end of the first to the start of the second.
    let two = [run("Menge", 300.0), run("Einheit", 328.0)];
    assert_eq!(
        title_cut(&two, &piece(&two), (left, right), true),
        Some((1, 0))
    );
    // One run without glyph gaps: its 13 characters share its width of 65, so the space
    // lies from 325 to 330, over the gutter.
    let one = [run("Menge Einheit", 300.0)];
    assert_eq!(
        title_cut(&one, &piece(&one), (left, right), true),
        Some((0, 6))
    );
    // A space that lies over the left column (the words after it start in it) is none.
    assert_eq!(
        title_cut(&one, &piece(&one), ((300.0, 331.0), (335.0, 345.0)), true),
        None
    );
    // Columns of one shape allow no cut.
    assert_eq!(title_cut(&one, &piece(&one), (left, right), false), None);
}

#[test]
fn values_have_shapes() {
    use Shape::{Date, Number, Percent, Text};
    for (text, shape) in [
        // Numbers: whole, decimal, amounts, thousands marks, signs and currencies.
        ("2", Some(Number)),
        ("-12", Some(Number)),
        ("+5", Some(Number)),
        ("-6,98", Some(Number)),
        ("3.50", Some(Number)),
        ("1.234,56", Some(Number)),
        ("1,234.56", Some(Number)),
        ("1.200", Some(Number)),
        ("1,200", Some(Number)),
        ("1 200", Some(Number)),
        ("1.234.567", Some(Number)),
        ("12,50 EUR", Some(Number)),
        ("12,50 \u{20ac}", Some(Number)),
        ("\u{20ac} 12,50", Some(Number)),
        ("EUR 12,50", Some(Number)),
        ("$3.50", Some(Number)),
        // Percentages.
        ("19%", Some(Percent)),
        ("19 %", Some(Percent)),
        // Dates: a year of four digits, or of two after a day and a month of two.
        ("01.01.2026", Some(Date)),
        ("31.01.26", Some(Date)),
        ("2026-01-01", Some(Date)),
        ("1/31/2026", Some(Date)),
        ("31/12/2026", Some(Date)),
        // Text: letters and no digits.
        ("Stk", Some(Text)),
        ("Std", Some(Text)),
        // No shape: digits in a form no shape knows, or neither digits nor letters.
        ("12,-", None),
        ("6,98-", None),
        ("(6,98)", None),
        ("8:00", None),
        ("50 ct", None),
        ("1.2.24", None),
        ("32.01.2026", None),
        ("01.13.2026", None),
        ("1..2", None),
        ("12.34.56", None),
        ("Jan 26", None),
        ("-", None),
        ("", None),
        // Currencies are stripped once each, in the order of `file::CURRENCIES` (codes before
        // signs): a sign before a code at the end is stripped, a sign before a code at the
        // start is not.
        ("12 \u{20ac} EUR", Some(Number)),
        ("\u{20ac} EUR 12", None),
    ] {
        assert_eq!(Shape::of(text), shape, "{text:?}");
    }
    let (n, t) = (Some(Number), Some(Text));
    assert_eq!(Shape::most(&[n, t, n]), Some(Number));
    assert_eq!(Shape::most(&[n, t, n, t]), None, "a tie");
    assert_eq!(Shape::most(&[n, t, Some(Percent)]), None, "no majority");
    assert_eq!(Shape::most(&[None, None, n]), None, "most have no shape");
    assert_eq!(Shape::most(&[]), None);
}

/// The pieces and lines of `runs` (one line each by line number, in order).
fn lined(runs: &[TextRun]) -> (Vec<RunRange>, Vec<Vec<Piece>>) {
    let lines = run_ranges(runs);
    let pieces = lines
        .iter()
        .map(|l| pieces_of(runs, l.start, l.end))
        .collect();
    (lines, pieces)
}

#[test]
fn captions_name_tables_and_figures() {
    use super::caption::caption;
    assert_eq!(caption("Table 3: Muster"), Some(true));
    assert_eq!(caption("Figure 2 Muster"), Some(false));
    assert_eq!(caption("Fig. A. Muster"), Some(false));
    assert_eq!(caption("Abb. 3: Muster"), Some(false));
    assert_eq!(caption("Abbildung 2 Muster"), Some(false));
    assert_eq!(caption("Abbildungen der Muster GmbH"), None);
    assert_eq!(caption("Abb Muster"), None);
    assert_eq!(caption("Tables of Muster"), None);
    assert_eq!(caption("Muster"), None);
}

#[test]
fn titles_over_values_need_titles_over_a_row_with_a_digit() {
    use super::grid::titles_over_values;
    let runs = vec![
        run_at("Pos", 100.0, 200.0, 0),
        run_at("Datum", 200.0, 200.0, 0),
        run_at("Betrag", 320.0, 200.0, 0),
        run_at("1", 100.0, 186.0, 1),
        run_at("15.01.2026", 200.0, 186.0, 1),
        run_at("9,99", 320.0, 186.0, 1),
    ];
    let (lines, pieces) = lined(&runs);
    assert!(titles_over_values(&runs, &lines, &pieces));
    // Words over words are no titles over values.
    let words = vec![
        run_at("Pos", 100.0, 200.0, 0),
        run_at("Datum", 200.0, 200.0, 0),
        run_at("Betrag", 320.0, 200.0, 0),
        run_at("Eins", 100.0, 186.0, 1),
        run_at("Zwei", 200.0, 186.0, 1),
        run_at("Drei", 320.0, 186.0, 1),
    ];
    let (lines, pieces) = lined(&words);
    assert!(!titles_over_values(&words, &lines, &pieces));
}

#[test]
fn a_line_set_full_in_pieces_is_prose() {
    use super::grid::set_full;
    // Three lines of four words set to one measure from 100 to 400: even gaps on each line,
    // wider on the first, words in small letters.
    let line = |words: &[&str], y: f32, line: u32| -> Vec<TextRun> {
        let width: f32 = words.iter().map(|w| 6.0 * w.chars().count() as f32).sum();
        let gap = (300.0 - width) / (words.len() - 1) as f32;
        let mut x = 100.0;
        words
            .iter()
            .map(|w| {
                let r = run_at(w, x, y, line);
                x += r.w + gap;
                r
            })
            .collect()
    };
    let mut runs = line(&["die", "Ware", "geht", "an"], 200.0, 0);
    runs.extend(line(&["den", "Kunden", "noch", "heute"], 186.0, 1));
    runs.extend(line(&["und", "wird", "dort", "bezahlt"], 172.0, 2));
    let (lines, pieces) = lined(&runs);
    assert!(set_full(&runs, &pieces, &lines));
    // Rows of a table: no line ends where the others do.
    let table = vec![
        run_at("Muster", 100.0, 200.0, 0),
        run_at("2", 200.0, 200.0, 0),
        run_at("12,50", 300.0, 200.0, 0),
        run_at("Beispiel", 100.0, 186.0, 1),
        run_at("10", 200.0, 186.0, 1),
        run_at("3,00", 306.0, 186.0, 1),
    ];
    let (lines, pieces) = lined(&table);
    assert!(!set_full(&table, &pieces, &lines));
}

/// Two runs of one line with a gap of 20 points (two sizes) between them: `first` ends at its
/// last character, `second` starts after the gap.
fn two_runs(first: &str, second: &str) -> Vec<TextRun> {
    let a = run_at(first, 0.0, 100.0, 1);
    let x = 6.0 * first.chars().count() as f32 + 20.0;
    vec![a, run_at(second, x, 100.0, 1)]
}

#[test]
fn a_wide_gap_after_a_full_stop_in_a_justified_line_parts_no_pieces() {
    // Word gaps of 12 points: the line is stretched, the 20 after the full stop goes with them.
    let runs = two_runs(
        "vitae,  felis.  Curabitur  dictum  mauris.",
        "Nam  arcu  libero,",
    );
    assert_eq!(pieces_of(&runs, 0, runs.len()).len(), 1);
    assert_eq!(grid::piece_count(&runs, 0, runs.len()), 1);
}

#[test]
fn a_wide_gap_stays_a_cell_gap_without_wide_word_gaps_or_a_sentence_end() {
    // Word gaps of 6 points: 20 is over twice them, two cells.
    let runs = two_runs("vitae, felis. Curabitur dictum mauris.", "Nam arcu libero,");
    assert_eq!(pieces_of(&runs, 0, runs.len()).len(), 2);
    // A number before the full stop is no sentence end ("Sa 1." of a calendar).
    let runs = two_runs("Sa  1.  So  2.  Mo  3.", "Di  4.  Mi  5.");
    assert_eq!(pieces_of(&runs, 0, runs.len()).len(), 2);
    // A small letter after the gap starts no sentence.
    let runs = two_runs(
        "vitae,  felis.  Curabitur  dictum  mauris.",
        "nam  arcu  libero,",
    );
    assert_eq!(grid::piece_count(&runs, 0, runs.len()), 2);
    // Too few word gaps to judge the line by.
    let runs = two_runs("dictum  mauris.", "Nam  arcu");
    assert_eq!(pieces_of(&runs, 0, runs.len()).len(), 2);
}
