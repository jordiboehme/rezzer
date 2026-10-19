// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{HashMap, HashSet};

use crate::derived::{self, NumCol};
use crate::types::{Analysis, Column, ColumnKind, Dialect, RaggedRow};

const CANDIDATES: &[u8; 4] = b",;\t|";
const SAMPLE_RECORDS: usize = 200;
const MIN_CONSISTENCY: f32 = 0.8;
const MIN_RAGGED_CONSISTENCY: f32 = 0.6;
const KEY_TEXT_MAX_LEN: usize = 40;

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

/// Modal field count of a list of counts: the most frequent one, the larger on ties.
fn modal(counts: &[usize]) -> Option<(usize, usize)> {
    let mut hist: HashMap<usize, usize> = HashMap::new();
    for &c in counts {
        *hist.entry(c).or_default() += 1;
    }
    hist.into_iter()
        .map(|(count, freq)| (freq, count))
        .max()
        .map(|(freq, count)| (count, freq))
}

/// Detects the CSV dialect of already-decoded text.
///
/// Tries comma, semicolon, tab and pipe on the first 200 records and picks
/// the delimiter whose most common field count (at least 2) covers the
/// largest share of records. Returns `None` when fewer than two records
/// exist or no candidate gives two fields with a share of at least 0.8, so
/// ordinary prose is not mistaken for CSV. A share between 0.6 and 0.8 is
/// accepted when the first two records both have the modal field count
/// (a ragged or broken file; `consistent_rows` keeps the real share). The header flag is a heuristic:
/// see [`Dialect::has_header`].
pub fn sniff(text: &str) -> Option<Dialect> {
    let text = strip_bom(text);
    let quote = b'"';
    let mut best: Option<(f32, usize, u8)> = None;
    for &delimiter in CANDIDATES {
        let counts: Vec<usize> = reader(text, delimiter, quote)
            .records()
            .take(SAMPLE_RECORDS)
            .map_while(Result::ok)
            .map(|r| r.len())
            .collect();
        if counts.len() < 2 {
            continue;
        }
        let Some((count, freq)) = modal(&counts) else {
            continue;
        };
        if count < 2 {
            continue;
        }
        let share = freq as f32 / counts.len() as f32;
        // A ragged file is accepted when the header line and the first data
        // line both have the modal field count.
        let ragged_ok = share >= MIN_RAGGED_CONSISTENCY && counts[0] == count && counts[1] == count;
        if share < MIN_CONSISTENCY && !ragged_ok {
            continue;
        }
        if best.is_none_or(|(s, c, _)| share > s || (share == s && count > c)) {
            best = Some((share, count, delimiter));
        }
    }
    let (share, _, delimiter) = best?;
    let rows: Vec<Vec<String>> = reader(text, delimiter, quote)
        .records()
        .take(SAMPLE_RECORDS)
        .map_while(Result::ok)
        .map(|r| r.iter().map(str::to_owned).collect())
        .collect();
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

fn detect_header(rows: &[Vec<String>]) -> bool {
    let Some((first, rest)) = rows.split_first() else {
        return false;
    };
    if rest.is_empty() || first.iter().all(|c| c.trim().is_empty()) {
        return false;
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

/// Pulls a unit out of a header: `(kg)`, `[m]` or a trailing `in %`.
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
    (!tail.is_empty() && tail.chars().count() <= 4 && !tail.contains(' ')).then(|| tail.to_owned())
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
    seen: HashSet<String>,
    duplicate: bool,
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
        if !self.duplicate && !self.seen.insert(v.to_owned()) {
            self.duplicate = true;
            self.seen = HashSet::new();
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

/// Header words that mark identifiers, never measures: year, id, code, number, date.
fn id_like_name(name: &str) -> bool {
    name.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| matches!(w, "year" | "id" | "code" | "number" | "date"))
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
/// Reads every record (ragged rows are reported, not errors) and returns
/// column names, types, units, missing counts and key candidates, ragged
/// rows, an unterminated-quote flag and observed sum relations. The
/// relation check (see [`Derived`]) needs at least 10 rows and is skipped
/// when there are more than `max_rows` data rows; everything else always
/// covers all rows.
pub fn analyze(text: &str, dialect: &Dialect, max_rows: usize) -> Analysis {
    let text = strip_bom(text);
    let mut rdr = reader(text, dialect.delimiter, dialect.quote);
    let mut records: Vec<(usize, Vec<String>)> = Vec::new();
    let mut counts = Vec::new();
    for rec in rdr.records().map_while(Result::ok) {
        let line = rec.position().map_or(0, |p| p.line() as usize);
        counts.push(rec.len());
        records.push((line, rec.iter().map(str::to_owned).collect()));
    }
    let expected = modal(&counts).map_or(0, |(c, _)| c);
    let ragged_rows = records
        .iter()
        .filter(|(_, r)| r.len() != expected)
        .map(|(line, r)| RaggedRow {
            line: *line,
            fields: r.len(),
            expected,
        })
        .collect();
    let width = counts.iter().copied().max().unwrap_or(0).max(expected);
    let (header, data) = match records.split_first() {
        Some((h, rest)) if dialect.has_header => (Some(&h.1), rest),
        _ => (None, &records[..]),
    };
    let rows = data.len();
    let mut accs: Vec<Acc> = (0..width).map(|_| Acc::default()).collect();
    for (_, r) in data {
        for (j, acc) in accs.iter_mut().enumerate() {
            acc.add(r.get(j).map(String::as_str));
        }
    }
    let columns: Vec<Column> = accs
        .iter()
        .enumerate()
        .map(|(j, a)| {
            let head = header
                .and_then(|h| h.get(j))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let kind = a.kind();
            let unique = !a.duplicate;
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
    let derived = if (10..=max_rows).contains(&rows) {
        let nums: Vec<NumCol> = columns
            .iter()
            .enumerate()
            .filter_map(|(j, c)| {
                if c.key_like || id_like_name(&c.name) {
                    return None;
                }
                let places = match c.kind {
                    ColumnKind::Integer => 0,
                    ColumnKind::Decimal { places } => places,
                    _ => return None,
                };
                let values = data
                    .iter()
                    .map(|(_, r)| r.get(j).and_then(|v| number(v.trim())).map(|n| n.1))
                    .collect::<Vec<_>>();
                if year_like(c, &values) {
                    return None;
                }
                Some(NumCol {
                    index: j,
                    places,
                    values,
                })
            })
            .take(12)
            .collect();
        derived::find(&nums, rows)
    } else {
        Vec::new()
    };
    Analysis {
        columns,
        rows,
        ragged_rows,
        unterminated_quote: has_unterminated_quote(text, dialect.delimiter, dialect.quote),
        derived,
    }
}
