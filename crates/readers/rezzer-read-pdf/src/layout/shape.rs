// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shape of lines, for headings and lists of an untagged page: lines that end in an
//! amount, by the reader's own scanner of amounts and dates. The text sizes of a page and the
//! rules of addresses are the engine's (see [`layoutregions::file::text_sizes`] and
//! [`layoutregions::extract::pairs::blocks`]).

use layoutregions::file::CURRENCIES;
use layoutregions::segment::cut;

use crate::scan;

/// Year widths of the dates a line is masked by before its amounts are read.
const YEARS: std::ops::RangeInclusive<usize> = 2..=4;

/// Whether `text` ends in an amount, a currency word or sign after it allowed
/// (`Gesamtbetrag 1.234,56 EUR`). Dates are masked first, so `01.03.2026` is no amount. A
/// number without decimals is an amount when its currency stands with it, after it or
/// before it (`Gesamt 1.428 EUR`, `Total CHF 80`): alone it may be a year or a count
/// (`Bericht 2025`).
pub(super) fn ends_in_amount(text: &str) -> bool {
    let mut t = text.trim();
    let mut currency = false;
    for c in CURRENCIES {
        if let Some(rest) = t.strip_suffix(c) {
            // A currency word is a word of its own: `TEUR` is no `EUR`.
            let word = c.chars().all(char::is_alphabetic);
            if !word || rest.ends_with(char::is_whitespace) {
                t = rest.trim_end();
                currency = true;
            }
            break;
        }
    }
    let skip: Vec<(usize, usize)> = scan::dates(t, YEARS)
        .iter()
        .map(|d| (d.start, d.end))
        .collect();
    if scan::amounts(t, &skip)
        .last()
        .is_some_and(|a| a.end == t.len())
    {
        return true;
    }
    // The last word in figures, with its currency: after it (taken off above) or as the
    // word before it.
    let mut words = t.rsplit(char::is_whitespace).filter(|w| !w.is_empty());
    let figures = words
        .next()
        .is_some_and(|w| cut::reads_as_value(w) && !skip.iter().any(|&(_, end)| end == t.len()));
    figures && (currency || words.next().is_some_and(|w| CURRENCIES.contains(&w)))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // A number without decimals with its currency (Q6 of Task 31b, Task 26 part 3c-1).
        assert!(ends_in_amount("Gesamt 1.428 EUR"));
        assert!(ends_in_amount("zzgl. USt. 19 % 228 EUR"));
        assert!(ends_in_amount("Total 80 $"));
        assert!(ends_in_amount("Total CHF 80"));
        assert!(ends_in_amount("Total $ 80"));
        assert!(!ends_in_amount("Gesamt 1.428"));
        assert!(!ends_in_amount("Gesamt 1.428 TEUR"));
        assert!(!ends_in_amount("Total CHF"));
        assert!(!ends_in_amount("Kurs EUR USD"));
        assert!(!ends_in_amount("Stand 01.03.2026 EUR"));
        assert!(!ends_in_amount("Box 12a EUR"));
    }
}
