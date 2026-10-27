// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Labels and unit columns next to values.

use super::*;

/// Labels longer than this are text, not labels.
pub(super) const MAX_LABEL: usize = 200;

/// Empty cells a label or header may be away from its value.
const LABEL_GAP: u32 = 3;

/// The texts around a value: the label to its left, the text right beside it (where a unit
/// sits in `Diameter | 10.0 | m`) when its column reads as a unit column, the header above
/// it.
#[derive(Clone, Copy, Default)]
pub(super) struct Around<'a> {
    pub(super) left: Option<&'a str>,
    /// The text on the left is a value of the row (a category like `blue`), not the name of
    /// this value: it occurs more than once in its column, or its column is a column of
    /// categories. A header above wins over it.
    pub(super) left_is_value: bool,
    pub(super) beside: Option<&'a str>,
    pub(super) header: Option<&'a str>,
}

/// What the sweep knows about one column.
#[derive(Clone, Copy)]
struct Column<'a> {
    /// The last row with a cell, and whether that cell holds a value or formula.
    last: u32,
    last_data: bool,
    /// The last text that heads the cells below it, with its row.
    head: Option<(&'a str, u32)>,
    /// Every row from the head down to `last` holds a value or formula: an unbroken run.
    solid: bool,
    /// The header at the top of the current run of values.
    block: Option<&'a str>,
}

/// Words a column header uses for a column of units.
const UNIT_HEADERS: &[&str] = &["unit", "units", "uom", "einheit", "einheiten", "dimension"];

/// The texts around each wanted cell, in one sweep over the sorted cells.
/// - The label to the left is at most [`LABEL_GAP`] empty cells away; a value or formula in
///   between stops the search. Labels name one value each and categories repeat: a text
///   that occurs more than once in its column, or that stands in a column where at least
///   half of the texts repeat an earlier one (four texts or more), is marked as a value of
///   the row.
/// - The header is the first text above the value when an unbroken run of values or
///   formulas joins them (any length, so every value under `t (m)` finds it). Across empty
///   cells it is at most [`LABEL_GAP`] rows above. A text right under values of the same
///   column (`see note` inside a number column) is a value, not a header.
pub(super) fn labels_of<'a>(
    cells: &'a [RawCell],
    wanted: &HashSet<(u32, u32)>,
) -> HashMap<(u32, u32), Around<'a>> {
    let label = |t: &'a str| is_label(t).then_some(t);
    let mut columns: HashMap<u32, Column<'a>> = HashMap::new();
    let mut out = HashMap::new();
    // How often each text occurs in its column, and how many texts and distinct texts
    // each column has.
    let mut in_column: HashMap<(u32, &'a str), u32> = HashMap::new();
    let mut texts: HashMap<u32, (u32, u32)> = HashMap::new();
    for c in cells {
        if let Some(t) = text_of(c) {
            let n = in_column.entry((c.col, t.trim())).or_default();
            *n += 1;
            let column = texts.entry(c.col).or_default();
            column.0 += 1;
            if *n == 1 {
                column.1 += 1;
            }
        }
    }
    let categories = |col: u32| {
        texts
            .get(&col)
            .is_some_and(|&(all, distinct)| all >= 4 && distinct * 2 <= all)
    };
    // The verdict on each text run beside values, kept for every cell of the run.
    let mut unit_columns: HashMap<usize, UnitColumn> = HashMap::new();
    for (i, c) in cells.iter().enumerate() {
        let near = columns
            .get(&c.col)
            .copied()
            .filter(|s| c.row - s.last - 1 <= LABEL_GAP);
        let text = text_of(c);
        let data = text.is_none() && !(c.val == Val::Empty && c.formula.is_none());
        if wanted.contains(&(c.row, c.col)) {
            let left_cell = i
                .checked_sub(1)
                .and_then(|j| cells.get(j))
                .filter(|p| p.row == c.row && c.col - p.col - 1 <= LABEL_GAP);
            let left = left_cell.and_then(text_of).and_then(label);
            let left_is_value = left_cell.zip(left).is_some_and(|(p, t)| {
                in_column.get(&(p.col, t.trim())).copied().unwrap_or(0) > 1 || categories(p.col)
            });
            let unbroken = near.is_some_and(|s| s.solid && c.row == s.last + 1);
            let header = near
                .and_then(|s| s.head)
                .filter(|(_, row)| unbroken || c.row - row <= LABEL_GAP)
                .map(|(t, _)| t);
            // The value heads no table column of its own: `label | value | unit` rows.
            let in_table = near.and_then(|s| s.block).is_some();
            let beside = cells
                .get(i + 1)
                .filter(|n| n.row == c.row && n.col == c.col + 1)
                .and_then(|n| {
                    let t = text_of(n)?;
                    let column = match unit_columns.get(&(i + 1)) {
                        Some(v) => *v,
                        None => {
                            let run = text_run(cells, i + 1);
                            let v = reads_as_unit_column(cells, &run);
                            unit_columns.extend(run.into_iter().map(|j| (j, v)));
                            v
                        }
                    };
                    let ok = match column {
                        UnitColumn::Declared => true,
                        UnitColumn::Likely => left.is_some() && !in_table,
                        UnitColumn::No => false,
                    };
                    ok.then_some(t)
                });
            out.insert(
                (c.row, c.col),
                Around {
                    left,
                    left_is_value,
                    beside,
                    header,
                },
            );
        }
        let state = match text {
            Some(t) => {
                // A text right under values is one more value of the column.
                let heads = !near.is_some_and(|s| s.last_data);
                let head = label(t).filter(|_| heads);
                Column {
                    last: c.row,
                    last_data: false,
                    head: head.map(|h| (h, c.row)),
                    solid: head.is_some(),
                    block: head,
                }
            }
            None => Column {
                last: c.row,
                last_data: data,
                head: near.and_then(|s| s.head),
                solid: data && near.is_some_and(|s| s.solid && c.row == s.last + 1),
                block: near.and_then(|s| s.block),
            },
        };
        columns.insert(c.col, state);
    }
    out
}

/// A text cell's text, for cells that hold a text and no formula.
fn text_of(c: &RawCell) -> Option<&str> {
    match &c.val {
        Val::Text(t) if c.formula.is_none() && !c.shared_unknown && !c.text_dropped => {
            Some(t.as_str())
        }
        _ => None,
    }
}

/// How a column of texts beside values reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnitColumn {
    /// Its header says it holds units (`Unit`, `Einheit`).
    Declared,
    /// It reads as units: at least two known units of two or more characters, most of its
    /// texts known units, and at least one that no code looks like (`kN/m3`, `m2`, `°C`).
    Likely,
    No,
}

/// The cells above and below `i` in its column that hold text, with no gap, top first.
fn text_run(cells: &[RawCell], i: usize) -> Vec<usize> {
    let col = cells[i].col;
    let find = |row: u32| {
        cells
            .binary_search_by_key(&(row, col), |c| (c.row, c.col))
            .ok()
            .filter(|&j| text_of(&cells[j]).is_some())
    };
    // The whole run: its verdict is kept for every cell of it, so each run is walked once.
    let mut up = Vec::new();
    let mut row = cells[i].row;
    while let Some(r) = row.checked_sub(1)
        && let Some(j) = find(r)
    {
        up.push(j);
        row = r;
    }
    up.reverse();
    up.push(i);
    let mut row = cells[i].row;
    while let Some(j) = row.checked_add(1).and_then(find) {
        up.push(j);
        row += 1;
    }
    up
}

/// Whether a text reads as a label: not empty after trimming and at most [`MAX_LABEL`]
/// bytes. The rule of [`labels_of`], also handed to the layout detector.
pub(crate) fn is_label(text: &str) -> bool {
    text.len() <= MAX_LABEL && !text.trim().is_empty()
}

/// Whether the texts of a column, top first, read as units: declared by its first text or
/// likely (see [`UnitColumn`]). The rule of [`reads_as_unit_column`], also handed to the
/// layout detector.
pub(crate) fn is_unit_column_texts(texts: &[&str]) -> bool {
    let texts: Vec<&str> = texts.iter().map(|t| t.trim()).collect();
    unit_column_of(texts) != UnitColumn::No
}

/// How the text run `run` reads (see [`UnitColumn`]).
fn reads_as_unit_column(cells: &[RawCell], run: &[usize]) -> UnitColumn {
    let texts: Vec<&str> = run
        .iter()
        .filter_map(|&j| text_of(&cells[j]))
        .map(str::trim)
        .collect();
    unit_column_of(texts)
}

/// Unit symbols that read as units on their own, also without a digit or a sign: SI base
/// and derived units with the usual prefixes and common engineering units. A column of
/// these (`m`, `kPa`, `kN`) is a unit column even though no text in it looks like `kN/m3`.
const UNIT_SYMBOLS: &[&str] = &[
    "mm", "cm", "dm", "m", "km", "mm2", "cm2", "m2", "km2", "mm3", "cm3", "m3", "mg", "g", "kg",
    "t", "ms", "s", "min", "h", "hr", "d", "N", "kN", "MN", "Nm", "kNm", "Pa", "kPa", "MPa", "GPa",
    "bar", "mbar", "%", "deg", "°", "°C", "K", "mW", "W", "kW", "MW", "GW", "Wh", "kWh", "MWh",
    "J", "kJ", "MJ", "ml", "mL", "l", "L", "mV", "V", "kV", "mA", "A", "kA", "Hz", "kHz", "MHz",
    "rad", "ha", "ppm",
];

/// Words that count things (`nos` for numbers, `pcs` for pieces). They read as a unit only
/// in a column that also holds a unit symbol of two or more characters: a column of `no`
/// alone is a yes/no column, and `N`, `no` a flag column.
const COUNT_WORDS: &[&str] = &["nos", "no", "pcs", "ea"];

/// Whether `t` is one of [`UNIT_SYMBOLS`] or a quotient of two of them, like `m/s` or
/// `kN/m3`. A quotient of two upper case letters is a code (`N/A`, `A/C`), never a unit.
fn is_unit_symbol(t: &str) -> bool {
    let known = |p: &str| UNIT_SYMBOLS.contains(&p);
    match t.split_once('/') {
        Some((a, b)) => {
            let code = |p: &str| p.len() == 1 && p.chars().all(|c| c.is_ascii_uppercase());
            known(a) && known(b) && !(code(a) && code(b))
        }
        None => known(t),
    }
}

/// How trimmed texts of a column, top first, read (see [`UnitColumn`]).
fn unit_column_of(texts: Vec<&str>) -> UnitColumn {
    if texts
        .first()
        .is_some_and(|t| UNIT_HEADERS.iter().any(|h| t.eq_ignore_ascii_case(h)))
    {
        return UnitColumn::Declared;
    }
    let counted: Vec<&str> = texts.into_iter().filter(|t| *t != "-").collect();
    // Every text is a unit symbol or a count word: units, also when none is marked by a
    // digit or a sign. One-letter symbols (`A`, `N`, `m` and `d`) are as often classes, flags
    // or codes, also beside a count word (`N`, `no`), so the column needs a symbol of two or
    // more characters (`kPa`, or a quotient like `m/s`).
    let count = |t: &str| COUNT_WORDS.contains(&t);
    if counted.len() >= 2
        && counted.iter().all(|t| is_unit_symbol(t) || count(t))
        && counted.iter().any(|t| !count(t) && t.chars().count() >= 2)
    {
        return UnitColumn::Likely;
    }
    let known: Vec<&str> = counted
        .iter()
        .copied()
        .filter(|t| units::is_unit(t, true) || is_unit_symbol(t))
        .collect();
    let long = known.iter().filter(|t| t.chars().count() > 1).count();
    let marked = known.iter().any(|t| {
        t.chars()
            .any(|c| c.is_ascii_digit() || "/^²³°%·".contains(c))
    });
    if long >= 2 && known.len() * 2 >= counted.len() && marked {
        UnitColumn::Likely
    } else {
        UnitColumn::No
    }
}
