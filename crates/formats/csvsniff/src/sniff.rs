// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::derived::{self, MAX_NUMERIC_COLUMNS, NumCol};
use crate::types::{Analysis, Column, ColumnKind, Dialect, RaggedRow};

const CANDIDATES: &[u8; 4] = b",;\t|";
/// Tie-break order for equally good delimiters (index = priority, lower wins).
const PREFERENCE: &[u8; 4] = b"\t;|,";
const SAMPLE_RECORDS: usize = 200;
const MIN_CONSISTENCY: f32 = 0.8;
const MIN_RAGGED_CONSISTENCY: f32 = 0.6;
const MIN_RECORDS: usize = 3;
const MIN_RAGGED_RECORDS: usize = 10;
const KEY_TEXT_MAX_LEN: usize = 40;
const MAX_RAGGED_LISTED: usize = 1000;
const MAX_UNIQUE_TRACKED: usize = 200_000;

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn reader(text: &str, delimiter: u8, quote: u8) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .quote(quote)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes())
}

/// Modal field count of a histogram: the most frequent one, the larger on ties.
/// Returns (count, frequency).
fn modal(hist: &HashMap<usize, usize>) -> Option<(usize, usize)> {
    hist.iter()
        .map(|(&count, &freq)| (freq, count))
        .max()
        .map(|(freq, count)| (count, freq))
}

fn histogram(counts: &[usize]) -> HashMap<usize, usize> {
    let mut hist = HashMap::new();
    for &c in counts {
        *hist.entry(c).or_default() += 1;
    }
    hist
}

/// Text that is plainly not delimited data: JSON, source code or G-code.
fn looks_like_other_format(text: &str) -> bool {
    if text.trim_start().starts_with(['{', '[']) {
        return true;
    }
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(SAMPLE_RECORDS)
        .collect();
    if lines.is_empty() {
        return false;
    }
    let half = |n: usize| n * 2 > lines.len();
    // Code: lines ending in `{` or `}` or in a lone `;` (a CSV with a trailing
    // delimiter has several `;` per line).
    let code = lines
        .iter()
        .filter(|l| l.ends_with(['{', '}']) || (l.ends_with(';') && l.matches(';').count() == 1))
        .count();
    // G-code: comment lines starting with `;` or G/M words.
    let gcode = lines
        .iter()
        .filter(|l| {
            l.starts_with(';')
                || (l.starts_with(['G', 'M']) && l[1..].starts_with(|c: char| c.is_ascii_digit()))
        })
        .count();
    half(code) || half(gcode)
}

/// A cell that looks like a number or date, also with a decimal comma.
fn numeric_looking(cell: &str) -> bool {
    let t = cell.trim();
    if number(t).is_some() || is_date(t) {
        return true;
    }
    match t.split_once(',') {
        Some((i, f)) => {
            let digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
            digits(i.trim_start_matches(['+', '-'])) && digits(f)
        }
        None => false,
    }
}

/// A delimiter candidate that passed the checks, with its ranking keys.
struct Pick {
    share: f32,
    count: usize,
    numeric: f32,
    rank: usize,
    delimiter: u8,
    rows: Vec<Vec<String>>,
}

impl Pick {
    /// Higher share, then field count, then numeric-looking cells, then preference order.
    fn beats(&self, other: &Pick) -> bool {
        let a = (self.share, self.count, self.numeric);
        let b = (other.share, other.count, other.numeric);
        a > b || (a == b && self.rank < other.rank)
    }
}

/// Detects the CSV dialect of already-decoded text.
///
/// Tries tab, semicolon, pipe and comma on the first 200 records and picks
/// the delimiter whose most common field count (at least 2) covers the
/// largest share of records; on equal share and count, the delimiter whose
/// cells more often look numeric wins, then tab, `;`, `|`, `,` in that order.
///
/// Returns `None` for text that is not delimited data: JSON, code lines,
/// G-code, prose (most fields longer than four words), fewer than three
/// records (two are enough only when both have at least three fields), or a
/// best share below 0.8. A share between 0.6 and 0.8 is accepted when the
/// first two records both have the modal field count and at least ten
/// records exist (a ragged or broken file; `consistent_rows` keeps the real
/// share). The header flag is a heuristic: see [`Dialect::has_header`].
pub fn sniff(text: &str) -> Option<Dialect> {
    let text = strip_bom(text);
    if looks_like_other_format(text) {
        return None;
    }
    let quote = b'"';
    let mut best: Option<Pick> = None;
    for &delimiter in CANDIDATES {
        let rows: Vec<Vec<String>> = reader(text, delimiter, quote)
            .records()
            .take(SAMPLE_RECORDS)
            .map_while(Result::ok)
            .map(|r| r.iter().map(str::to_owned).collect())
            .collect();
        let counts: Vec<usize> = rows.iter().map(Vec::len).collect();
        let n = counts.len();
        if n < 2 {
            continue;
        }
        let Some((count, freq)) = modal(&histogram(&counts)) else {
            continue;
        };
        if count < 2 || (n < MIN_RECORDS && !(n == 2 && counts.iter().all(|&c| c >= 3))) {
            continue;
        }
        let share = freq as f32 / n as f32;
        // A ragged file is accepted when the header line and the first data
        // line both have the modal field count.
        let ragged_ok = share >= MIN_RAGGED_CONSISTENCY
            && n >= MIN_RAGGED_RECORDS
            && counts[0] == count
            && counts[1] == count;
        if share < MIN_CONSISTENCY && !ragged_ok {
            continue;
        }
        let cells: Vec<&str> = rows.iter().flatten().map(String::as_str).collect();
        let filled: Vec<&&str> = cells.iter().filter(|c| !c.trim().is_empty()).collect();
        if filled.is_empty() {
            continue;
        }
        // Prose: most fields are long free text.
        let long = filled
            .iter()
            .filter(|c| c.split_whitespace().count() > 4)
            .count();
        if long * 2 > filled.len() {
            continue;
        }
        let numeric =
            filled.iter().filter(|c| numeric_looking(c)).count() as f32 / filled.len() as f32;
        let rank = PREFERENCE
            .iter()
            .position(|&d| d == delimiter)
            .unwrap_or(usize::MAX);
        let pick = Pick {
            share,
            count,
            numeric,
            rank,
            delimiter,
            rows,
        };
        if best.as_ref().is_none_or(|b| pick.beats(b)) {
            best = Some(pick);
        }
    }
    let Pick {
        share,
        delimiter,
        rows,
        ..
    } = best?;
    Some(Dialect {
        delimiter,
        quote,
        has_header: detect_header(&rows),
        consistent_rows: share,
    })
}

/// Coarse class of one cell, for header detection.
#[derive(PartialEq)]
enum Class {
    Empty,
    Numeric,
    Date,
    Other,
}

fn class(cell: &str) -> Class {
    let t = cell.trim();
    if t.is_empty() {
        Class::Empty
    } else if number(t).is_some() {
        Class::Numeric
    } else if is_date(t) {
        Class::Date
    } else {
        Class::Other
    }
}

/// A first row such as `Country,2019,2020`: text cells plus two or more
/// 4-digit years that rise or fall strictly in column order and differ from
/// the cells right below them.
fn year_header(first: &[String], next: &[String]) -> bool {
    let mut years = Vec::new();
    for (j, c) in first.iter().enumerate() {
        match class(c) {
            Class::Other | Class::Empty => {}
            Class::Numeric => match c.trim().parse::<u32>() {
                Ok(y)
                    if (1000..=2999).contains(&y)
                        && next.get(j).is_none_or(|n| n.trim() != c.trim()) =>
                {
                    years.push(y)
                }
                _ => return false,
            },
            Class::Date => return false,
        }
    }
    years.len() >= 2
        && years.len() < first.len()
        && (years.windows(2).all(|w| w[0] < w[1]) || years.windows(2).all(|w| w[0] > w[1]))
}

fn detect_header(rows: &[Vec<String>]) -> bool {
    let Some((first, rest)) = rows.split_first() else {
        return false;
    };
    if rest.is_empty() || first.iter().all(|c| c.trim().is_empty()) {
        return false;
    }
    if year_header(first, &rest[0]) {
        return true;
    }
    let first_classes: Vec<Class> = first.iter().map(|c| class(c)).collect();
    if first_classes
        .iter()
        .any(|c| matches!(c, Class::Numeric | Class::Date))
    {
        return false;
    }
    // Rule A: some column is numeric or dates below the first row.
    for j in 0..first.len() {
        let mut seen = 0;
        let mut all = true;
        for row in rest {
            match row.get(j).map_or(Class::Empty, |c| class(c)) {
                Class::Empty => {}
                Class::Numeric | Class::Date => seen += 1,
                Class::Other => {
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
fn number(s: &str) -> Option<(u8, f64)> {
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

fn is_date(s: &str) -> bool {
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

#[derive(Default)]
struct Acc {
    non_empty: usize,
    missing: usize,
    int: usize,
    dec: usize,
    places: u8,
    date: usize,
    boolean: usize,
    max_len: usize,
    /// Hashes of the values seen so far while the column is still unique.
    seen: HashSet<u64>,
    /// A duplicate was seen, or more than `MAX_UNIQUE_TRACKED` distinct values.
    not_unique: bool,
}

impl Acc {
    fn add(&mut self, raw: Option<&str>) {
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
            if !self.seen.insert(h.finish()) || self.seen.len() > MAX_UNIQUE_TRACKED {
                self.not_unique = true;
                self.seen = HashSet::new();
            }
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

/// Integer values that step by exactly 1 once sorted (row ids); needs no gaps in the data.
fn consecutive(values: &[Option<f64>]) -> bool {
    let mut v: Vec<f64> = values.iter().flatten().copied().collect();
    if v.len() != values.len() || v.len() < 2 {
        return false;
    }
    v.sort_by(f64::total_cmp);
    v.windows(2).all(|w| w[1] - w[0] == 1.0)
}

/// Unique Integer column with every value in 1000..=2999.
fn year_like(c: &Column, values: &[Option<f64>]) -> bool {
    c.unique
        && c.kind == ColumnKind::Integer
        && values
            .iter()
            .flatten()
            .all(|v| (1000.0..=2999.0).contains(v))
}

/// True when a quoted field is still open at the end of `text`.
/// Mirrors the csv crate: a quote only opens a field at its start.
fn has_unterminated_quote(text: &str, delimiter: u8, quote: u8) -> bool {
    let b = text.as_bytes();
    let (mut i, mut start, mut open) = (0, true, false);
    while i < b.len() {
        let c = b[i];
        if open {
            if c == quote {
                if b.get(i + 1) == Some(&quote) {
                    i += 1;
                } else {
                    open = false;
                }
            }
        } else if c == quote && start {
            open = true;
            start = false;
        } else {
            start = c == delimiter || c == b'\n' || c == b'\r';
        }
        i += 1;
    }
    open
}

/// Analyzes the whole text with a known dialect.
///
/// Streams the records (ragged rows are reported, not errors) and returns
/// column names, types, units, missing counts and key candidates, ragged
/// rows, an unterminated-quote flag and observed sum relations. The number
/// of columns is the modal field count; extra cells in longer records only
/// show up as ragged rows. Memory does not grow with the number of rows,
/// except for a uniqueness set of at most 200,000 value hashes per column
/// (above that a column counts as not unique).
///
/// `max_rows` is the number of rows used for the derived-column check (at
/// most the first `max_rows` data rows, and the check needs at least 10);
/// all rows are still counted and typed. The first 1,000 ragged rows are
/// listed, `ragged_total` counts them all.
pub fn analyze(text: &str, dialect: &Dialect, max_rows: usize) -> Analysis {
    let text = strip_bom(text);
    let (delimiter, quote) = (dialect.delimiter, dialect.quote);

    // Pass 1: field count histogram (no per-cell allocation).
    let mut hist: HashMap<usize, usize> = HashMap::new();
    let mut rdr = reader(text, delimiter, quote);
    let mut byte_rec = csv::ByteRecord::new();
    while rdr.read_byte_record(&mut byte_rec).unwrap_or(false) {
        *hist.entry(byte_rec.len()).or_default() += 1;
    }
    let expected = modal(&hist).map_or(0, |(c, _)| c);

    // Pass 2: column statistics and ragged rows.
    let mut accs: Vec<Acc> = (0..expected).map(|_| Acc::default()).collect();
    let mut header: Option<Vec<String>> = None;
    let mut ragged_rows = Vec::new();
    let mut ragged_total = 0;
    let mut rows = 0;
    let mut rdr = reader(text, delimiter, quote);
    let mut rec = csv::StringRecord::new();
    let mut first = true;
    while rdr.read_record(&mut rec).unwrap_or(false) {
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
                acc.add(rec.get(j));
            }
        }
        first = false;
    }
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
                key_like,
            }
        })
        .collect();

    // Pass 3: numeric values of the first `max_rows` rows, for the derived check.
    let checked_rows = rows.min(max_rows);
    let derived = if checked_rows >= 10 {
        let eligible: Vec<(usize, u8)> = columns
            .iter()
            .enumerate()
            .filter(|(_, c)| !id_like_name(&c.name))
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
            let mut rdr = reader(text, delimiter, quote);
            let skip = usize::from(dialect.has_header);
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
                .filter(|((j, _), v)| {
                    let c = &columns[*j];
                    !(year_like(c, v) || (c.kind == ColumnKind::Integer && consecutive(v)))
                })
                .map(|(&(index, places), values)| NumCol {
                    index,
                    places,
                    values,
                })
                .collect();
            derived::find(&nums, checked_rows)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    Analysis {
        columns,
        rows,
        ragged_rows,
        ragged_total,
        unterminated_quote: has_unterminated_quote(text, delimiter, quote),
        derived,
    }
}
