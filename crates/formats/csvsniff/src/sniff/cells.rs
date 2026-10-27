// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Cell classes, the header rule and the full analysis of a dialect.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use super::{modal, reader, strip_bom};
use crate::derived::{self, MAX_NUMERIC_COLUMNS, NumCol};
use crate::stop::{STOP_CHECK_BYTES, StopRead};
use crate::types::{Analysis, Column, ColumnKind, Dialect, QuoteProblem, RaggedRow};

const KEY_TEXT_MAX_LEN: usize = 40;
const MAX_RAGGED_LISTED: usize = 1000;
const MAX_UNIQUE_TRACKED: usize = 200_000;
/// Total number of value hashes kept for uniqueness over all columns of a file.
const UNIQUE_BUDGET: usize = 2_000_000;

/// The coarse class of one cell value, as the header rule sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellClass {
    /// Nothing but whitespace.
    Empty,
    /// Anything that is no number, date or bool.
    Text,
    /// A plain number: optional sign, digits, optional fraction with a point.
    Number,
    /// A date, also with a time (see [`crate::ColumnKind::Date`]).
    Date,
    /// `true`, `false`, `yes` or `no` in any case.
    Bool,
}

/// The class of one cell value, judged on its trimmed text. The header rule of
/// [`sniff`](super::sniff) uses the same classes and counts a bool cell as text.
pub fn cell_class(cell: &str) -> CellClass {
    let t = cell.trim();
    if t.is_empty() {
        CellClass::Empty
    } else if number(t).is_some() {
        CellClass::Number
    } else if is_date(t) {
        CellClass::Date
    } else if is_bool(t) {
        CellClass::Bool
    } else {
        CellClass::Text
    }
}

/// The class of a cell for header detection: a bool cell counts as text.
pub(super) fn class(cell: &str) -> CellClass {
    match cell_class(cell) {
        CellClass::Bool => CellClass::Text,
        c => c,
    }
}

/// A first row such as `Country,2019,2020`: text cells plus two or more
/// 4-digit years that rise or fall strictly in column order and differ from
/// the cells right below them.
fn year_header(first: &[String], next: &[String]) -> bool {
    let mut years = Vec::new();
    for (j, c) in first.iter().enumerate() {
        match class(c) {
            CellClass::Text | CellClass::Bool | CellClass::Empty => {}
            CellClass::Number => match c.trim().parse::<u32>() {
                Ok(y)
                    if (1000..=2999).contains(&y)
                        && next.get(j).is_none_or(|n| n.trim() != c.trim()) =>
                {
                    years.push(y)
                }
                _ => return false,
            },
            CellClass::Date => return false,
        }
    }
    years.len() >= 2
        && years.len() < first.len()
        && (years.windows(2).all(|w| w[0] < w[1]) || years.windows(2).all(|w| w[0] > w[1]))
}

pub(super) fn detect_header(rows: &[Vec<String>]) -> bool {
    let Some((first, rest)) = rows.split_first() else {
        return false;
    };
    if rest.is_empty() || first.iter().all(|c| c.trim().is_empty()) {
        return false;
    }
    if year_header(first, &rest[0]) {
        return true;
    }
    let first_classes: Vec<CellClass> = first.iter().map(|c| class(c)).collect();
    if first_classes
        .iter()
        .any(|c| matches!(c, CellClass::Number | CellClass::Date))
    {
        return false;
    }
    // Rule A: some column is numeric or dates below the first row.
    for j in 0..first.len() {
        let mut seen = 0;
        let mut all = true;
        for row in rest {
            match row.get(j).map_or(CellClass::Empty, |c| class(c)) {
                CellClass::Empty => {}
                CellClass::Number | CellClass::Date => seen += 1,
                CellClass::Text | CellClass::Bool => {
                    all = false;
                    break;
                }
            }
        }
        if all && seen > 0 {
            return true;
        }
    }
    // Rule B: first-row cells are unique while later rows repeat values.
    let cells: HashSet<&str> = first.iter().map(|c| c.trim()).collect();
    if first.iter().any(|c| c.trim().is_empty()) || cells.len() != first.len() || rest.len() < 2 {
        return false;
    }
    (0..first.len()).any(|j| {
        let mut seen = HashSet::new();
        rest.iter()
            .filter_map(|r| r.get(j))
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .any(|c| !seen.insert(c))
    })
}

/// Parses a plain number: optional sign, digits, optional fraction.
/// Returns the number of decimal places and the value.
pub(super) fn number(s: &str) -> Option<(u8, f64)> {
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let digits = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    if !digits(int)
        || frac.is_some_and(|f| f.is_empty() || !digits(f))
        || (int.is_empty() && frac.is_none())
    {
        return None;
    }
    if int.is_empty() && frac.is_none_or(str::is_empty) {
        return None;
    }
    let places = frac.map_or(0, |f| f.len().min(255) as u8);
    s.parse::<f64>().ok().map(|v| (places, v))
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

pub(super) fn is_date(s: &str) -> bool {
    if date_part(s) {
        return true;
    }
    // Date followed by a time: "2024-01-31T10:00:00Z", "2024-01-31 10:00".
    let Some((date, time)) = s.split_once(['T', ' ']) else {
        return false;
    };
    let time = time.trim();
    date_part(date)
        && time.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && time.contains(':')
        && time
            .bytes()
            .all(|b| b.is_ascii_digit() || b":.Z+-".contains(&b))
}

fn date_part(s: &str) -> bool {
    let parts: Vec<&str> = s
        .split(['-', '.', '/', ' '])
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return false;
    }
    let num = |p: &str| {
        p.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| p.parse::<u32>().ok())
            .flatten()
    };
    let month_name = |p: &str| {
        p.len() >= 3
            && MONTHS.iter().any(|m| p.to_ascii_lowercase().starts_with(m))
            && p.bytes().all(|b| b.is_ascii_alphabetic())
    };
    let (a, b, c) = (parts[0], parts[1], parts[2]);
    // d-Mon-yy, d-Mon-yyyy
    if month_name(b) {
        return num(a).is_some_and(|d| (1..=31).contains(&d))
            && num(c).is_some()
            && (c.len() == 2 || c.len() == 4);
    }
    let (Some(x), Some(y), Some(z)) = (num(a), num(b), num(c)) else {
        return false;
    };
    let valid = |m: u32, d: u32| (1..=12).contains(&m) && (1..=31).contains(&d);
    if a.len() == 4 && b.len() <= 2 && c.len() <= 2 {
        return valid(y, z);
    }
    if c.len() == 4 && a.len() <= 2 && b.len() <= 2 {
        return valid(y, x) || valid(x, y);
    }
    false
}

fn is_bool(s: &str) -> bool {
    ["true", "false", "yes", "no"]
        .iter()
        .any(|b| s.eq_ignore_ascii_case(b))
}

const UNITS: &[&str] = &[
    "%", "‰", "kg", "g", "t", "mg", "m", "cm", "mm", "km", "s", "ms", "h", "min", "°C", "°F", "K",
    "l", "ml", "W", "kW", "MW", "kWh", "MWh", "V", "A", "Hz", "Pa", "kPa", "bar", "EUR", "USD",
    "GBP", "CHF", "Mt", "kt",
];

/// Pulls a unit out of a header: `(kg)`, `[m]` or a trailing `in %` / `in kg`
/// (only for known unit tokens, so "Born in Rome" has no unit).
fn unit_of(header: &str) -> Option<String> {
    for (open, close) in [('(', ')'), ('[', ']')] {
        if let (Some(s), Some(e)) = (header.rfind(open), header.rfind(close))
            && s < e
        {
            let u = header[s + 1..e].trim();
            if !u.is_empty() {
                return Some(u.to_owned());
            }
        }
    }
    let (_, tail) = header.rsplit_once(" in ")?;
    let tail = tail.trim();
    UNITS.contains(&tail).then(|| tail.to_owned())
}

struct Acc {
    non_empty: usize,
    missing: usize,
    int: usize,
    dec: usize,
    places: u8,
    date: usize,
    boolean: usize,
    max_len: usize,
    /// Smallest and largest numeric value (valid when `int + dec > 0`).
    min: f64,
    max: f64,
    /// Numeric values so far rise strictly in file order; `prev` is the last one.
    increasing: bool,
    prev: f64,
    /// Hashes of the values seen so far while the column is still unique.
    seen: HashSet<u64>,
    /// A duplicate was seen, or more than `MAX_UNIQUE_TRACKED` distinct values.
    not_unique: bool,
    /// `not_unique` because a limit was hit, not because of a duplicate.
    exhausted: bool,
}

impl Acc {
    fn new() -> Self {
        Acc {
            non_empty: 0,
            missing: 0,
            int: 0,
            dec: 0,
            places: 0,
            date: 0,
            boolean: 0,
            max_len: 0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            increasing: true,
            prev: f64::NEG_INFINITY,
            exhausted: false,
            seen: HashSet::new(),
            not_unique: false,
        }
    }

    /// `budget` is the number of uniqueness hashes the whole file may still keep.
    fn add(&mut self, raw: Option<&str>, budget: &mut usize) {
        let v = raw.map_or("", str::trim);
        if v.is_empty() {
            self.missing += 1;
            return;
        }
        self.non_empty += 1;
        self.max_len = self.max_len.max(v.chars().count());
        if !self.not_unique {
            let mut h = DefaultHasher::new();
            v.hash(&mut h);
            if !self.seen.insert(h.finish()) {
                self.give_up(budget);
            } else if *budget == 0 || self.seen.len() > MAX_UNIQUE_TRACKED {
                self.exhausted = true;
                self.give_up(budget);
            } else {
                *budget -= 1;
            }
        }
        if let Some((_, x)) = number(v) {
            self.min = self.min.min(x);
            self.max = self.max.max(x);
            self.increasing &= x > self.prev;
            self.prev = x;
        }
        match number(v) {
            Some((0, _)) => self.int += 1,
            Some((p, _)) => {
                self.dec += 1;
                self.places = self.places.max(p);
            }
            None if is_date(v) => self.date += 1,
            None if is_bool(v) => self.boolean += 1,
            None => {}
        }
    }

    /// Marks the column as not unique and returns its hashes to the budget.
    fn give_up(&mut self, budget: &mut usize) {
        self.not_unique = true;
        *budget += self.seen.len();
        self.seen = HashSet::new();
    }

    fn kind(&self) -> ColumnKind {
        let n = self.non_empty;
        if n == 0 {
            ColumnKind::Empty
        } else if self.int == n {
            ColumnKind::Integer
        } else if self.int + self.dec == n {
            ColumnKind::Decimal {
                places: self.places,
            }
        } else if self.date == n {
            ColumnKind::Date
        } else if self.boolean == n {
            ColumnKind::Bool
        } else {
            ColumnKind::Text
        }
    }
}

/// Headers that mark identifiers, never measures: a word `year`, `code` or
/// `date`, or a last word `number`, `no`, `nr` or `id` ("Case number", but
/// not "Number of cases").
fn id_like_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().any(|w| matches!(*w, "year" | "code" | "date"))
        || words
            .last()
            .is_some_and(|w| matches!(*w, "number" | "no" | "nr" | "id"))
}

/// Integer columns that are identifiers, judged over all rows: unique values
/// in 1000..=2999 (years), or values without gaps that are unique or strictly
/// rising (row ids).
fn identifier_values(acc: &Acc, c: &Column, rows: usize) -> bool {
    if c.kind != ColumnKind::Integer || acc.missing > 0 || rows == 0 {
        return false;
    }
    let years = c.unique && acc.min >= 1000.0 && acc.max <= 2999.0;
    // Past the uniqueness budget, strictly rising values prove uniqueness.
    let gapless = rows >= 2
        && (c.unique || acc.increasing)
        && acc.max - acc.min + 1.0 == acc.non_empty as f64;
    years || gapless
}

/// Scans the quoting like the csv crate does (a quote only opens a field at its start) and
/// returns the problems: the first mid-file one and, independently, a field still open at the
/// end. Each is a quoted field that is not closed properly: the byte offset of its opening
/// quote and whether it is still open at the end of `text`. A field that runs over a line break
/// and then ends in a quote that is not followed by a delimiter or a line break (the csv crate
/// reads on without an error) counts as well, with `at_end` false.
fn find_quote_problems(
    text: &str,
    delimiter: u8,
    quote: u8,
    stop: &dyn Fn() -> bool,
) -> Option<Vec<QuoteProblem>> {
    let b = text.as_bytes();
    let (mut i, mut start, mut open) = (0, true, false);
    let (mut opened_at, mut multiline) = (0, false);
    let mut problems: Vec<QuoteProblem> = Vec::new();
    let mut next_check = STOP_CHECK_BYTES;
    while i < b.len() {
        if i >= next_check {
            next_check = i + STOP_CHECK_BYTES;
            if stop() {
                return None;
            }
        }
        let c = b[i];
        if open {
            if c == b'\n' {
                multiline = true;
            }
            if c == quote {
                if b.get(i + 1) == Some(&quote) {
                    i += 1;
                } else {
                    open = false;
                    let ends_field = match b.get(i + 1) {
                        None => true,
                        Some(&n) => n == delimiter || n == b'\n' || n == b'\r',
                    };
                    if multiline && !ends_field && problems.is_empty() {
                        problems.push(QuoteProblem {
                            offset: opened_at,
                            at_end: false,
                        });
                    }
                }
            }
        } else if c == quote && start {
            open = true;
            multiline = false;
            opened_at = i;
            start = false;
        } else {
            start = c == delimiter || c == b'\n' || c == b'\r';
        }
        i += 1;
    }
    if open {
        problems.push(QuoteProblem {
            offset: opened_at,
            at_end: true,
        });
    }
    Some(problems)
}

/// Analyzes the whole text with a known dialect.
///
/// Streams the records (ragged rows are reported, not errors) and returns
/// column names, types, units, missing counts and key candidates, ragged
/// rows, an unterminated-quote flag and observed sum relations. The number
/// of columns is the modal field count; extra cells in longer records only
/// show up as ragged rows. The first [`Dialect::lead_records`] records (title
/// lines) are left out of all of it. Memory does not grow with the number of rows,
/// except for a uniqueness set of at most 200,000 value hashes per column
/// (above that a column counts as not unique).
///
/// `max_rows` is the number of rows used for the derived-column check (at
/// most the first `max_rows` data rows, and the check needs at least 10);
/// all rows are still counted and typed. The first 1,000 ragged rows are
/// listed, `ragged_total` counts them all.
pub fn analyze(text: &str, dialect: &Dialect, max_rows: usize) -> Analysis {
    analyze_until(text, dialect, max_rows, &|| false)
}

/// Records between two calls of the stop callback.
const STOP_CHECK_EVERY: usize = 4096;

/// Like [`analyze`], but asks `stop` every few thousand records and every 64 KiB of text in
/// each pass over it (also inside one long record) and ends the analysis early when it
/// returns true (a time limit, a cancel). The result then has `truncated` set: the
/// row count and column facts cover only the records read so far, and the derived-column and
/// quote checks are skipped.
pub fn analyze_until(
    text: &str,
    dialect: &Dialect,
    max_rows: usize,
    stop: &dyn Fn() -> bool,
) -> Analysis {
    let text = strip_bom(text);
    let mut truncated = false;
    let (delimiter, quote) = (dialect.delimiter, dialect.quote);

    // Pass 1: field count histogram (no per-cell allocation).
    let mut hist: HashMap<usize, usize> = HashMap::new();
    let stopped = Cell::new(false);
    let mut rdr = reader(StopRead::new(text, stop, &stopped), delimiter, quote);
    let mut byte_rec = csv::ByteRecord::new();
    let mut seen = 0usize;
    // Title lines above the table are not part of it.
    let lead = dialect.lead_records;
    while rdr.read_byte_record(&mut byte_rec).unwrap_or(false) {
        seen += 1;
        if seen > lead {
            *hist.entry(byte_rec.len()).or_default() += 1;
        }
        if seen.is_multiple_of(STOP_CHECK_EVERY) && stop() {
            truncated = true;
            break;
        }
    }
    truncated |= stopped.get();
    let expected = modal(&hist).map_or(0, |(c, _)| c);

    // Pass 2: column statistics and ragged rows.
    let mut accs: Vec<Acc> = (0..expected).map(|_| Acc::new()).collect();
    let mut header: Option<Vec<String>> = None;
    let mut ragged_rows = Vec::new();
    let mut ragged_total = 0;
    let mut rows = 0;
    let mut budget = UNIQUE_BUDGET;
    let stopped = Cell::new(false);
    let mut rdr = reader(StopRead::new(text, stop, &stopped), delimiter, quote);
    let mut rec = csv::StringRecord::new();
    let mut first = true;
    let mut seen = 0usize;
    while rdr.read_record(&mut rec).unwrap_or(false) {
        seen += 1;
        if seen.is_multiple_of(STOP_CHECK_EVERY) && stop() {
            truncated = true;
            break;
        }
        if seen <= lead {
            continue;
        }
        if rec.len() != expected {
            ragged_total += 1;
            if ragged_rows.len() < MAX_RAGGED_LISTED {
                ragged_rows.push(RaggedRow {
                    line: rec.position().map_or(0, |p| p.line() as usize),
                    fields: rec.len(),
                    expected,
                });
            }
        }
        if first && dialect.has_header {
            header = Some(rec.iter().take(expected).map(str::to_owned).collect());
        } else {
            rows += 1;
            for (j, acc) in accs.iter_mut().enumerate() {
                acc.add(rec.get(j), &mut budget);
            }
        }
        first = false;
    }
    truncated |= stopped.get();
    let columns: Vec<Column> = accs
        .iter()
        .enumerate()
        .map(|(j, a)| {
            let head = header
                .as_ref()
                .and_then(|h| h.get(j))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let kind = a.kind();
            let unique = !a.not_unique;
            let key_like = unique
                && a.missing == 0
                && rows >= 2
                && match kind {
                    ColumnKind::Integer => true,
                    ColumnKind::Text => a.max_len <= KEY_TEXT_MAX_LEN,
                    _ => false,
                };
            Column {
                name: head.map_or_else(|| format!("Column {}", j + 1), str::to_owned),
                kind,
                unit: head.and_then(unit_of),
                missing: a.missing,
                unique,
                unique_exhausted: a.exhausted,
                key_like,
            }
        })
        .collect();

    // Pass 3: numeric values of the first `max_rows` rows, for the derived check.
    let checked_rows = rows.min(max_rows);
    let mut derived = if checked_rows >= 10 && !truncated {
        let eligible: Vec<(usize, u8)> = columns
            .iter()
            .enumerate()
            .filter(|(j, c)| !id_like_name(&c.name) && !identifier_values(&accs[*j], c, rows))
            .filter_map(|(j, c)| match c.kind {
                ColumnKind::Integer => Some((j, 0)),
                ColumnKind::Decimal { places } => Some((j, places)),
                _ => None,
            })
            .take(MAX_NUMERIC_COLUMNS)
            .collect();
        if eligible.len() >= 3 {
            let mut values: Vec<Vec<Option<f64>>> =
                vec![Vec::with_capacity(checked_rows); eligible.len()];
            let stopped = Cell::new(false);
            let mut rdr = reader(StopRead::new(text, stop, &stopped), delimiter, quote);
            let skip = lead + usize::from(dialect.has_header);
            for rec in rdr
                .records()
                .map_while(Result::ok)
                .skip(skip)
                .take(checked_rows)
            {
                for (v, &(j, _)) in values.iter_mut().zip(&eligible) {
                    v.push(rec.get(j).and_then(|c| number(c.trim())).map(|n| n.1));
                }
            }
            let nums: Vec<NumCol> = eligible
                .iter()
                .zip(values)
                .map(|(&(index, places), values)| NumCol {
                    index,
                    places,
                    values,
                })
                .collect();
            truncated = stopped.get();
            if truncated {
                Vec::new()
            } else {
                derived::find(&nums, checked_rows)
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let found = (!truncated)
        .then(|| find_quote_problems(text, delimiter, quote, stop))
        .flatten();
    truncated |= found.is_none();
    if truncated {
        derived.clear();
    }
    let quote_problems = found.unwrap_or_default();
    Analysis {
        truncated,
        columns,
        rows,
        ragged_rows,
        ragged_total,
        unterminated_quote: quote_problems.iter().any(|p| p.at_end),
        quote_problems,
        derived,
    }
}
