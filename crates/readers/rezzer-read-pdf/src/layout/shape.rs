// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shape of lines, for headings and lists of an untagged page: the text sizes of a page and
//! lines that end in an amount. The rules of addresses are the engine's (see
//! [`layoutregions::extract::pairs::blocks`]).

use std::collections::{HashMap, HashSet};

use layoutregions::file::CURRENCIES;

use super::{Run, ink, size_key};
use crate::scan;

/// A size that sets at least this share of a page's ink characters is a text size there.
const TEXT_SHARE: f32 = 0.25;

/// A size that sets at least this many ink characters of a page is a text size there.
const PAGE_BODY_CHARS: usize = 400;

/// A size is a text size by its share only when it sets ink on at least this many lines: the
/// title of a cover page with little else on it stays a heading.
const SHARE_LINES: usize = 3;

/// The text sizes of a page, smallest first: every size that sets at least [`TEXT_SHARE`] of
/// its ink characters on [`SHARE_LINES`] lines or more, or at least [`PAGE_BODY_CHARS`] of
/// them. A line in one of them is body
/// text on that page, never a heading: the data lines of a form set beside many small
/// labels, or the back of a form printed in larger type, are text.
pub(crate) fn page_text_sizes(runs: &[Run]) -> Vec<f32> {
    page_text_sizes_of(runs.iter())
}

/// [`page_text_sizes`] of the runs `runs`, which may come from several lists.
pub(crate) fn page_text_sizes_of<'a>(runs: impl Iterator<Item = &'a Run>) -> Vec<f32> {
    // For each size: its ink characters and the lines it sets ink on.
    let mut chars: HashMap<i32, (usize, HashSet<u32>)> = HashMap::new();
    for r in runs {
        let n = ink(&r.text);
        if n > 0 {
            let e = chars.entry(size_key(r.size)).or_default();
            e.0 += n;
            e.1.insert(r.line);
        }
    }
    let total: usize = chars.values().map(|c| c.0).sum();
    let mut sizes: Vec<i32> = chars
        .into_iter()
        .filter(|(_, (n, at))| {
            (at.len() >= SHARE_LINES && *n as f32 >= TEXT_SHARE * total as f32)
                || *n >= PAGE_BODY_CHARS
        })
        .map(|(size, _)| size)
        .collect();
    sizes.sort_unstable();
    sizes.into_iter().map(|s| s as f32 / 2.0).collect()
}

/// Year widths of the dates a line is masked by before its amounts are read.
const YEARS: std::ops::RangeInclusive<usize> = 2..=4;

/// Whether `text` ends in an amount, a currency word or sign after it allowed
/// (`Gesamtbetrag 1.234,56 EUR`). Dates are masked first, so `01.03.2026` is no amount.
pub(super) fn ends_in_amount(text: &str) -> bool {
    let mut t = text.trim();
    for c in CURRENCIES {
        if let Some(rest) = t.strip_suffix(c) {
            // A currency word is a word of its own: `TEUR` is no `EUR`.
            let word = c.chars().all(char::is_alphabetic);
            if !word || rest.ends_with(char::is_whitespace) {
                t = rest.trim_end();
            }
            break;
        }
    }
    let skip: Vec<(usize, usize)> = scan::dates(t, YEARS)
        .iter()
        .map(|d| (d.start, d.end))
        .collect();
    scan::amounts(t, &skip)
        .last()
        .is_some_and(|a| a.end == t.len())
}

#[cfg(test)]
mod tests {
    use super::super::Gaps;
    use super::*;

    fn run(text: &str, line: u32, y: f32, size: f32) -> Run {
        Run {
            page: 1,
            text: text.into(),
            x: 72.0,
            y,
            w: 100.0,
            h: size,
            size,
            bold: false,
            italic: false,
            family: 0,
            mcid: None,
            uri: None,
            line,
            off_axis: false,
            note: false,
            gaps: Gaps::default(),
        }
    }

    #[test]
    fn text_sizes_of_a_page() {
        // 800 ink characters of labels, 300 of data lines with padding on 3 lines, 20 of a
        // title.
        let data = |line: u32| {
            let text = format!("{}{}", "b".repeat(100), " ".repeat(300));
            run(&text, line, 690.0 - 10.0 * line as f32, 9.5)
        };
        let runs = [
            run(&"a".repeat(800), 0, 700.0, 6.0),
            data(1),
            data(2),
            data(3),
            run(&"c".repeat(20), 4, 640.0, 14.0),
        ];
        assert_eq!(page_text_sizes(&runs), [6.0, 9.5]);
        // 400 ink characters make a text size also under a quarter of the page.
        let runs = [
            run(&"a".repeat(2000), 0, 700.0, 6.0),
            run(&"b".repeat(400), 1, 680.0, 9.5),
        ];
        assert_eq!(page_text_sizes(&runs), [6.0, 9.5]);
        // A cover page: a title of one line sets half of the little text there is, and is no
        // text size.
        let runs = [
            run(&"a".repeat(30), 0, 700.0, 24.0),
            run(&"b".repeat(10), 1, 600.0, 10.0),
            run(&"b".repeat(10), 2, 590.0, 10.0),
            run(&"b".repeat(10), 3, 580.0, 10.0),
        ];
        assert_eq!(page_text_sizes(&runs), [10.0]);
    }

    #[test]
    fn amounts_at_the_end() {
        assert!(ends_in_amount("Gesamtbetrag 1.234,56 EUR"));
        assert!(ends_in_amount("Summe 9,99"));
        assert!(ends_in_amount("Total $ 1,234.56"));
        assert!(ends_in_amount("Netto 12,50 \u{20ac}"));
        assert!(ends_in_amount("1.234,56 50,00 123,45"));
        assert!(!ends_in_amount("Rechnung vom 01.03.2026"));
        assert!(!ends_in_amount("Bericht 2025"));
        assert!(!ends_in_amount("Betrag 3,50 TEUR"));
        assert!(!ends_in_amount("1,50 und mehr"));
    }
}
