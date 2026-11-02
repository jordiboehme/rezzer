// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dates and amounts written as text, found by a hand-written scanner (no regular
//! expressions): what the separators drawn as marks are read by, and what a page's own date
//! is found by.

use std::ops::RangeInclusive;

/// A full date written as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Date {
    /// Where it stands in the text, in bytes.
    pub start: usize,
    pub end: usize,
    /// The character between its parts: `.`, `/` or `-`.
    pub sep: char,
    /// The parts as written: day, month, year for `.`; first, second, year for `/`; year,
    /// month, day for `-`.
    pub parts: [u32; 3],
}

/// An amount with two decimals written as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Amount {
    pub start: usize,
    pub end: usize,
    /// The character before the last two digits.
    pub decimal: char,
    /// The character between groups of three digits, when the amount has groups.
    pub group: Option<char>,
}

/// The full dates of `text`: day and month of one or two digits and a year of `years`
/// digits, joined by `.` or by `/`, or `yyyy-mm-dd`. A date stands alone: no digit stands
/// right before or after it, nor its separator with a digit (`1.01.02.2026` holds none).
/// Day and month must be possible: a dotted date reads day first; a slashed one needs a part
/// of at most 12 and both of at most 31.
pub(crate) fn dates(text: &str, years: RangeInclusive<usize>) -> Vec<Date> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() || !starts_alone(b, i) {
            i += 1;
            continue;
        }
        match date_at(b, i, &years) {
            Some(d) => {
                i = d.end;
                out.push(d);
            }
            None => i += digits(b, i),
        }
    }
    out
}

/// The date that starts at the digit `i` of `b`, if one does.
fn date_at(b: &[u8], i: usize, years: &RangeInclusive<usize>) -> Option<Date> {
    let n1 = digits(b, i);
    let sep = *b.get(i + n1)?;
    if !matches!(sep, b'.' | b'/' | b'-') {
        return None;
    }
    let j = i + n1 + 1;
    let n2 = digits(b, j);
    if b.get(j + n2) != Some(&sep) {
        return None;
    }
    let k = j + n2 + 1;
    let n3 = digits(b, k);
    let end = k + n3;
    // Nothing goes on after the date: no digit, nor its separator with a digit.
    if n3 == 0 || (b.get(end) == Some(&sep) && b.get(end + 1).is_some_and(u8::is_ascii_digit)) {
        return None;
    }
    let parts = [value(b, i, n1), value(b, j, n2), value(b, k, n3)];
    let day = |d: u32| (1..=31).contains(&d);
    let month = |m: u32| (1..=12).contains(&m);
    let fits = match sep {
        b'-' => n1 == 4 && n2 == 2 && n3 == 2 && month(parts[1]) && day(parts[2]),
        _ if !(1..=2).contains(&n1) || !(1..=2).contains(&n2) || !years.contains(&n3) => false,
        b'.' => day(parts[0]) && month(parts[1]),
        _ => day(parts[0]) && day(parts[1]) && (month(parts[0]) || month(parts[1])),
    };
    fits.then_some(Date {
        start: i,
        end,
        sep: char::from(sep),
        parts,
    })
}

/// Whether a number may start at `i`: no digit stands before it, nor a separator with a
/// digit before that.
fn starts_alone(b: &[u8], i: usize) -> bool {
    match i.checked_sub(1).map(|p| b[p]) {
        None => true,
        Some(c) if c.is_ascii_digit() => false,
        Some(b'.' | b'/' | b'-' | b',') => !(i >= 2 && b[i - 2].is_ascii_digit()),
        Some(_) => true,
    }
}

/// The number of ASCII digits from `i` on.
fn digits(b: &[u8], i: usize) -> usize {
    b.get(i..).map_or(0, |rest| {
        rest.iter().take_while(|c| c.is_ascii_digit()).count()
    })
}

/// The value of the `n` digits from `i` on, saturated.
fn value(b: &[u8], i: usize, n: usize) -> u32 {
    b[i..i + n].iter().fold(0u32, |v, c| {
        v.saturating_mul(10).saturating_add(u32::from(c - b'0'))
    })
}

/// The amounts with two decimals of `text`, outside the byte ranges `skip` (dates found
/// first): digits, then `.` or `,` and two digits, no letter, digit or `_` after them. An
/// amount with groups has one to three digits and then groups of three after one other
/// character. A number whose separators fit neither (`12.34.56`) is no amount.
pub(crate) fn amounts(text: &str, skip: &[(usize, usize)]) -> Vec<Amount> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut skip = skip.iter().peekable();
    let mut i = 0;
    while i < b.len() {
        while skip.peek().is_some_and(|s| s.1 <= i) {
            skip.next();
        }
        if let Some(&&(start, end)) = skip.peek()
            && start <= i
        {
            i = end;
            continue;
        }
        if !b[i].is_ascii_digit() || (i > 0 && b[i - 1].is_ascii_digit()) {
            i += 1;
            continue;
        }
        // The number: digit runs, each after one `.` or `,`.
        let mut runs = vec![(i, digits(b, i))];
        let mut seps = Vec::new();
        let mut end = i + runs[0].1;
        while let Some(&c) = b.get(end)
            && matches!(c, b'.' | b',')
            && b.get(end + 1).is_some_and(u8::is_ascii_digit)
        {
            seps.push(c);
            let n = digits(b, end + 1);
            runs.push((end + 1, n));
            end += 1 + n;
        }
        // A letter, a digit or `_` right after it: the number goes on.
        let alone = text[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if alone && let Some(a) = amount(i, end, &runs, &seps) {
            out.push(a);
        }
        i = end;
    }
    out
}

/// The amount a number of digit `runs` with `seps` between them is, if it is one.
fn amount(start: usize, end: usize, runs: &[(usize, usize)], seps: &[u8]) -> Option<Amount> {
    let (&decimal, groups) = seps.split_last()?;
    if runs.last()?.1 != 2 {
        return None;
    }
    let group = match groups.first() {
        None => None,
        Some(&g) => {
            let fits = g != decimal
                && groups.iter().all(|&c| c == g)
                && (1..=3).contains(&runs[0].1)
                && runs[1..runs.len() - 1].iter().all(|r| r.1 == 3);
            if !fits {
                return None;
            }
            Some(char::from(g))
        }
    };
    Some(Amount {
        start,
        end,
        decimal: char::from(decimal),
        group,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seps(text: &str) -> Vec<(char, [u32; 3])> {
        dates(text, 2..=4)
            .iter()
            .map(|d| (d.sep, d.parts))
            .collect()
    }

    #[test]
    fn dates_of_three_styles() {
        assert_eq!(
            seps("Datum 01.02.2026, am 1/13/26 und 2026-02-01."),
            [('.', [1, 2, 2026]), ('/', [1, 13, 26]), ('-', [2026, 2, 1])]
        );
    }

    #[test]
    fn a_date_stands_alone() {
        assert!(seps("1.01.02.2026").is_empty());
        assert!(seps("01.02.20261").is_empty());
        assert!(seps("101.02.2026").is_empty());
        assert!(seps("01.02.2026.5").is_empty());
        // Not a possible day or month.
        assert!(seps("32.01.2026 01.13.2026 13/13/2026").is_empty());
        // Year widths are the caller's.
        assert!(dates("01.02.26", 4..=4).is_empty());
        assert_eq!(dates("01.02.26", 2..=4).len(), 1);
    }

    fn found(text: &str) -> Vec<(char, Option<char>)> {
        amounts(text, &[])
            .iter()
            .map(|a| (a.decimal, a.group))
            .collect()
    }

    #[test]
    fn amounts_with_and_without_groups() {
        assert_eq!(
            found("9,99 und 1.234,56 or 1,234,567.89 EUR 12.30"),
            [(',', None), (',', Some('.')), ('.', Some(',')), ('.', None)]
        );
    }

    #[test]
    fn what_is_no_amount() {
        // Three digits after the separator, a letter after, groups that are not three.
        assert!(found("1.234 1,234 3,50EUR 12.34.56 1,2").is_empty());
        // A sentence may end right after an amount.
        assert_eq!(found("Summe 9,99."), [(',', None)]);
    }

    #[test]
    fn skipped_ranges_are_not_read() {
        let text = "01.02.2026 1,50";
        let d = dates(text, 2..=4);
        let skip: Vec<(usize, usize)> = d.iter().map(|d| (d.start, d.end)).collect();
        assert_eq!(skip, [(0, 10)]);
        let a = amounts(text, &skip);
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].start, a[0].end), (11, 15));
    }
}
