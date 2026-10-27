// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

mod cells;

use std::collections::HashMap;

use crate::types::Dialect;
pub use cells::{CellClass, analyze, analyze_until, cell_class};
use cells::{class, detect_header, is_date, number};

const CANDIDATES: &[u8; 4] = b",;\t|";
/// Tie-break order for equally good delimiters (index = priority, lower wins).
const PREFERENCE: &[u8; 4] = b"\t;|,";
const SAMPLE_RECORDS: usize = 200;
const MIN_CONSISTENCY: f32 = 0.8;
const MIN_RAGGED_CONSISTENCY: f32 = 0.6;
const MIN_RECORDS: usize = 3;
const MIN_RAGGED_RECORDS: usize = 10;

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn reader<R: std::io::Read>(input: R, delimiter: u8, quote: u8) -> csv::Reader<R> {
    csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .quote(quote)
        .has_headers(false)
        .flexible(true)
        .from_reader(input)
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
    // G-code: comment lines starting with `;`, or lines of G/M/T/N words
    // (`G1 X10 Y5 ; move`) without any delimiter outside a trailing comment.
    let gcode = if delimited_columns(&lines) {
        0
    } else {
        lines.iter().filter(|l| is_gcode_line(l)).count()
    };
    half(code) || half(gcode)
}

/// Lines that look like a `;` table, not G-code: the first line neither starts
/// with `;` nor with a G/M/T/N word, and all lines have the same number (at
/// least one) of `;`.
fn delimited_columns(lines: &[&str]) -> bool {
    let first = lines[0];
    let n = first.matches(';').count();
    n >= 1
        && !first.starts_with(';')
        && !starts_with_gcode_word(first)
        && lines.iter().all(|l| l.matches(';').count() == n)
}

fn starts_with_gcode_word(line: &str) -> bool {
    let mut chars = line.chars();
    matches!(chars.next(), Some('G' | 'M' | 'T' | 'N'))
        && chars.next().is_some_and(|c| c.is_ascii_digit())
}

fn is_gcode_line(line: &str) -> bool {
    // The comment starts at the first `;`; a second `;` means separators.
    let (code, comment) = line.split_once(';').unwrap_or((line, ""));
    if comment.contains(';') {
        return false;
    }
    if code.is_empty() {
        return true;
    }
    if code.contains([',', '\t', '|']) {
        return false;
    }
    let mut chars = code.chars();
    let word = matches!(chars.next(), Some('G' | 'M' | 'T' | 'N'));
    let rest = chars.as_str();
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    word && digits > 0
        && rest[digits..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
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

/// For each field of the record that starts at the beginning of `raw`: was it quoted?
fn quoted_flags(raw: &str, delimiter: u8, quote: u8) -> Vec<bool> {
    let b = raw.as_bytes();
    let (mut i, mut start, mut open) = (0, true, false);
    let mut flags = Vec::new();
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
        } else if c == b'\n' || c == b'\r' {
            break;
        } else if c == quote && start {
            open = true;
            start = false;
            flags.push(true);
        } else if c == delimiter {
            if start {
                flags.push(false);
            }
            start = true;
        } else if start {
            flags.push(false);
            start = false;
        }
        i += 1;
    }
    if start {
        flags.push(false);
    }
    flags
}

/// A delimiter candidate that passed the checks, with its ranking keys.
struct Pick {
    share: f32,
    count: usize,
    numeric: f32,
    rank: usize,
    delimiter: u8,
    /// The records before the table that were left out (title lines).
    lead: usize,
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

/// Leading records with fewer fields than the modal count, at most this many, may be title
/// lines above the table. They are left out only when the records as a whole fail the checks.
const MAX_LEAD_RECORDS: usize = 10;

/// The share, the modal field count and the share of numeric-looking cells of the sampled
/// records of one delimiter, when they pass the checks (see [`sniff`]).
fn judge(rows: &[Vec<String>], quoted: &[Vec<bool>]) -> Option<(f32, usize, f32)> {
    let counts: Vec<usize> = rows.iter().map(Vec::len).collect();
    let n = counts.len();
    if n < 2 {
        return None;
    }
    let (count, freq) = modal(&histogram(&counts))?;
    if count < 2 || (n < MIN_RECORDS && !(n == 2 && counts.iter().all(|&c| c >= 3))) {
        return None;
    }
    let share = freq as f32 / n as f32;
    // A ragged file is accepted when the header line and the first data
    // line both have the modal field count.
    let ragged_ok = share >= MIN_RAGGED_CONSISTENCY
        && n >= MIN_RAGGED_RECORDS
        && counts[0] == count
        && counts[1] == count;
    if share < MIN_CONSISTENCY && !ragged_ok {
        return None;
    }
    let cells: Vec<&str> = rows.iter().flatten().map(String::as_str).collect();
    let filled: Vec<&&str> = cells.iter().filter(|c| !c.trim().is_empty()).collect();
    if filled.is_empty() {
        return None;
    }
    // Prose: most unquoted fields are long free text (quoted text is
    // normal in CSV: translations, questions and answers).
    let unquoted: Vec<&str> = rows
        .iter()
        .zip(quoted)
        .flat_map(|(r, q)| {
            r.iter()
                .zip(q.iter().chain(std::iter::repeat(&false)))
                .filter(|(c, q)| !**q && !c.trim().is_empty())
                .map(|(c, _)| c.as_str())
        })
        .collect();
    let long = unquoted
        .iter()
        .filter(|c| c.split_whitespace().count() > 4)
        .count();
    if long * 2 > unquoted.len() {
        return None;
    }
    let numeric = filled.iter().filter(|c| numeric_looking(c)).count() as f32 / filled.len() as f32;
    Some((share, count, numeric))
}

/// Whether the records after title lines show a table beyond a consistent field count, so
/// prose or a list with commas under a title stays text: a column of numbers or dates below
/// the first record, and also at least three fields, at least [`MIN_RAGGED_RECORDS`]
/// records or a header by the header rule.
fn table_evidence(rows: &[Vec<String>], (_, count, _): (f32, usize, f32)) -> bool {
    numeric_column(rows) && (count >= 3 || rows.len() >= MIN_RAGGED_RECORDS || detect_header(rows))
}

/// Whether some column holds only numbers or dates (at least one) below the first record.
fn numeric_column(rows: &[Vec<String>]) -> bool {
    let Some((first, rest)) = rows.split_first() else {
        return false;
    };
    let width = rest
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(first.len());
    (0..width).any(|j| {
        let mut seen = false;
        for row in rest {
            match row.get(j).map_or(CellClass::Empty, |c| class(c)) {
                CellClass::Empty => {}
                CellClass::Number | CellClass::Date => seen = true,
                CellClass::Text | CellClass::Bool => return false,
            }
        }
        seen
    })
}

/// The number of leading records with fewer fields than the modal count, when there are
/// 1 to [`MAX_LEAD_RECORDS`] of them.
fn lead_records(rows: &[Vec<String>]) -> Option<usize> {
    let counts: Vec<usize> = rows.iter().map(Vec::len).collect();
    let (count, _) = modal(&histogram(&counts))?;
    let lead = counts.iter().take_while(|&&c| c < count).count();
    (1..=MAX_LEAD_RECORDS).contains(&lead).then_some(lead)
}

/// Sniffing looks at most at this many bytes from the start (and at most [`SAMPLE_RECORDS`]
/// records), so one huge record cannot make it slow.
const SNIFF_MAX_BYTES: usize = 1024 * 1024;

/// The first `max` bytes of `text`, cut after the last line break within them so the sample
/// ends with a complete record (a cut at a character boundary when there is no line break).
fn head(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    match text[..end].rfind('\n') {
        Some(nl) => &text[..=nl],
        None => &text[..end],
    }
}

/// Detects the CSV dialect of already-decoded text.
///
/// Tries tab, semicolon, pipe and comma on the first 200 records (within the first 1 MiB)
/// and picks
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
/// share). When the records fail these checks only because of 1 to 10 short
/// records at the start (title lines above the table), the checks run on the
/// records after them and [`Dialect::lead_records`] counts them. Those records
/// must then also hold a column of numbers or dates below their first record,
/// and have at least three fields, a header or at least ten records. The header
/// flag is a heuristic: see [`Dialect::has_header`].
pub fn sniff(text: &str) -> Option<Dialect> {
    let text = head(strip_bom(text), SNIFF_MAX_BYTES);
    crate::work::add(text.len() as u64);
    if looks_like_other_format(text) {
        return None;
    }
    let quote = b'"';
    let mut best: Option<Pick> = None;
    for &delimiter in CANDIDATES {
        crate::work::add(text.len() as u64);
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut quoted: Vec<Vec<bool>> = Vec::new();
        for rec in reader(text.as_bytes(), delimiter, quote)
            .records()
            .take(SAMPLE_RECORDS)
            .map_while(Result::ok)
        {
            let start = rec.position().map_or(0, |p| p.byte() as usize);
            quoted.push(quoted_flags(
                &text[start.min(text.len())..],
                delimiter,
                quote,
            ));
            rows.push(rec.iter().map(str::to_owned).collect());
        }
        // The records as they are; else without title lines above the table.
        let (lead, (share, count, numeric)) = match judge(&rows, &quoted) {
            Some(found) => (0, found),
            None => match lead_records(&rows).and_then(|lead| {
                let found = judge(&rows[lead..], &quoted[lead..])?;
                table_evidence(&rows[lead..], found).then_some((lead, found))
            }) {
                Some(found) => found,
                None => continue,
            },
        };
        rows.drain(..lead);
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
            lead,
            rows,
        };
        if best.as_ref().is_none_or(|b| pick.beats(b)) {
            best = Some(pick);
        }
    }
    let Pick {
        share,
        delimiter,
        lead,
        rows,
        ..
    } = best?;
    Some(Dialect {
        delimiter,
        quote,
        lead_records: lead,
        has_header: detect_header(&rows),
        consistent_rows: share,
    })
}
