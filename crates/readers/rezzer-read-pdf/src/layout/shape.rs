// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shape of lines, for headings and lists of an untagged page: the text sizes of a page,
//! bold that leads a line, amounts and data, addresses, and marks set into form fields.

use std::collections::{HashMap, HashSet};

use super::{Line, Run, ink, list_marker, size_key};
use crate::scan;

/// Whether a line opens with a `*`, `-` or U+2013 run whose padding a piece gap parts from
/// the text after it: padding written as one space (see `order::line_runs`), no bullet.
pub(super) fn padded_mark(line: &Line<'_>) -> bool {
    let mut inked = line.runs.iter().filter(|r| ink(&r.text) > 0);
    let (Some(mark), Some(next)) = (inked.next(), inked.next()) else {
        return false;
    };
    // `line_runs` writes padding across a piece gap as one space after the run before it; a
    // bullet set at a tab stop has its space before the text instead.
    matches!(mark.text.trim(), "*" | "-" | "\u{2013}")
        && mark.text.ends_with(char::is_whitespace)
        && next.x - (mark.x + mark.w) > crate::order::PIECE_GAP * mark.size.max(next.size)
}

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

/// Whether a line is bold where it matters for a heading: all its ink is bold, or it opens
/// with bold ink and goes on in plain words that are no data (`SECTION 1 (TO BE COMPLETED BY
/// PAYEE)`). A bold label that ends in a colon before plain text is a label and its value
/// (`Kundennummer: 00000`), and so is a bold label before a plain number or code.
pub(super) fn bold_led(line: &Line<'_>) -> bool {
    let inked: Vec<&Run> = line.runs.iter().filter(|r| ink(&r.text) > 0).collect();
    let lead = inked.iter().take_while(|r| r.bold).count();
    if lead == 0 {
        return false;
    }
    if lead == inked.len() {
        return true;
    }
    let label: String = inked[..lead].iter().map(|r| r.text.as_str()).collect();
    let rest: String = inked[lead..].iter().map(|r| r.text.as_str()).collect();
    !label.trim_end().ends_with(':') && !data_only(&rest)
}

/// Currency words and signs that may stand after an amount.
const CURRENCIES: [&str; 7] = ["EUR", "USD", "CHF", "GBP", "\u{20ac}", "$", "\u{a3}"];

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

/// Whether a line holds only numbers, codes, dates and amounts: each of its words holds a
/// digit or is a currency word or sign, and one holds a digit (`1234567 8901 2 30`,
/// `00123 45B`). A title holds a word (`Bericht 2025`).
pub(super) fn data_only(text: &str) -> bool {
    let mut digit = false;
    for w in text.split_whitespace() {
        if w.chars().any(|c| c.is_ascii_digit()) {
            digit = true;
        } else if !CURRENCIES.contains(&w) {
            return false;
        }
    }
    digit
}

/// Lines of an address or a list of labels hold at most this many words; lines of prose in a
/// narrow column hold more.
const ADDRESS_WORDS: usize = 4;

/// Whether lines have the shape of an address or a list: each starts with a list marker, or
/// none holds more than [`ADDRESS_WORDS`] words and words in small letters are rare (at most
/// one for every two lines, name particles aside). Prose, also German prose whose nouns
/// start with capitals, holds many words in small letters.
pub(super) fn listed(lines: &[&str]) -> bool {
    if lines.iter().all(|l| list_marker(l).is_some()) {
        return true;
    }
    let total: usize = lines.iter().map(|l| small_words(l)).sum();
    lines.iter().all(|l| word_count(l) <= ADDRESS_WORDS) && 2 * total <= lines.len()
}

/// The words of a line, a run of words that each hold a digit counted as one: an IBAN or a
/// phone number written in groups is one word (`IBAN DE00 0000 0000` is two).
fn word_count(text: &str) -> usize {
    let mut n = 0;
    let mut in_number = false;
    for w in text.split_whitespace() {
        let number = w.chars().any(|c| c.is_ascii_digit());
        if !(number && in_number) {
            n += 1;
        }
        in_number = number;
    }
    n
}

/// Whether lines have the shape of an address: three lines or more that are [`listed`], each
/// with a letter, and a digit on a line after the first (a street number, a postcode). Their
/// left edge is the caller's to check.
pub(crate) fn address_shaped(lines: &[&str]) -> bool {
    lines.len() >= 3
        && listed(lines)
        && lines.iter().all(|l| l.chars().any(char::is_alphabetic))
        && lines[1..]
            .iter()
            .any(|l| l.chars().any(|c| c.is_ascii_digit()))
}

/// Whether two lines one under the other are an address: each a few words (at most
/// [`ADDRESS_WORDS`]) with letters and a number, the number in another place on each (see
/// [`word_shape`]): a street and its number over a postcode and its town. A label over its
/// value has a number on one line at most (`Nr` over `4711`, `Strasse` over `Musterweg 1`), or
/// in the same place on both (`Zimmer 2` over `Raum 101`). Their left edge is the caller's to
/// check.
pub(crate) fn address_lines(lines: &[&str]) -> bool {
    let line = |l: &&str| {
        word_count(l) <= ADDRESS_WORDS
            && l.chars().any(char::is_alphabetic)
            && l.chars().any(|c| c.is_ascii_digit())
    };
    matches!(lines, [a, b] if line(a) && line(b) && word_shape(a) != word_shape(b))
}

/// Whether lines are a block of an address, a bank or a register entry: [`address_shaped`],
/// and not all of one shape (see [`word_shape`]), as a name, a street and a postcode with its
/// town are not. The labels or values of one form in a column of a table (`Feld 1`,
/// `Feld 2`, or `Musterweg 1`, `Musterweg 2`) have one shape.
pub(crate) fn address_block(lines: &[&str]) -> bool {
    let Some(first) = lines.first() else {
        return false;
    };
    let shape = word_shape(first);
    lines.iter().any(|l| word_shape(l) != shape) && address_shaped(lines)
}

/// A line of a block in [`address_columns`]: its text, whether all of it is bold, and its
/// size.
#[derive(Debug, Clone)]
pub(crate) struct ColumnLine {
    pub text: String,
    pub bold: bool,
    pub size: f32,
}

/// What [`address_columns`] makes of blocks of lines side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Columns {
    /// Rows of one block, a table or labels and values: read as they are.
    Rows,
    /// Blocks of their own, read one after the other. Rows were possible too (their lines
    /// share baselines), so that is said.
    Blocks,
    /// Blocks of their own under a header row of their headings (`Anschrift`,
    /// `Bankverbindung`): read as the table they make, with the doubt said.
    Headed,
    /// Rows that hold a postal address in a column but are no blocks by their evidence: read
    /// as rows, with the doubt said (they may be blocks side by side).
    Doubt,
}

/// An address block holds at most this many lines: a name, a line more (`c/o`, a
/// department), a street, a postcode and town, a country.
const ADDRESS_LINES: usize = 5;

/// How well the lines of a column are a block of facts of its own (see [`fact_block`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fact {
    /// No block of facts: too long, an amount, a line without a letter, no digit.
    No,
    /// A block of facts whose lines are of one kind by their words or shape (two `Telefon`
    /// lines, two `IBAN` lines, two lines that end with the town): a block beside an address,
    /// and a column of a table as well.
    Kinds,
    /// A block of facts whose lines are each of another kind.
    Facts,
}

/// Whether the lines of a column are a block of facts of its own (an address, a bank, a
/// register entry, a contact). A first line that ends with a colon over two lines or more is
/// its heading (`Bankverbindung:`), and the block is the lines under it. The block holds two
/// to [`ADDRESS_LINES`] lines, each short (at most [`ADDRESS_WORDS`] words) with a letter, no
/// colon at its end (a label's) and no amount alone (see [`data_only`]), and a digit on some
/// line (a street number, a postcode, an account, a phone number): else [`Fact::No`]. Each line
/// a fact of another kind (no two lines start or end with the same word, and not all have one
/// shape, see [`word_shape`]) is [`Fact::Facts`]; else, when two lines start or end with the
/// same word but the lines under its first do not all start with one word (`Musterweg 1`,
/// `Musterweg 2` are records), [`Fact::Kinds`]. The
/// lines of a column of a table are records of one kind: they share words (`Halle 2`, `Halle
/// 1`; `12 Stueck`, `8 Stueck`) or a shape (`3 Kartons`, `12 Paletten`).
fn fact_block(block: &[&ColumnLine]) -> Fact {
    let headed = block.len() >= 3
        && block[0].text.ends_with(':')
        && !block[0].text.chars().any(|c| c.is_ascii_digit());
    let block = if headed { &block[1..] } else { block };
    if !(2..=ADDRESS_LINES).contains(&block.len()) {
        return Fact::No;
    }
    let short = |l: &&ColumnLine| {
        word_count(&l.text) <= ADDRESS_WORDS
            && l.text.chars().any(char::is_alphabetic)
            && !l.text.ends_with(':')
            && !data_only(&l.text)
    };
    if !block.iter().all(short)
        || !block
            .iter()
            .any(|l| l.text.chars().any(|c| c.is_ascii_digit()))
    {
        return Fact::No;
    }
    let distinct = |word: &dyn Fn(&str) -> Option<String>| {
        let mut seen: Vec<String> = block.iter().filter_map(|l| word(&l.text)).collect();
        let n = seen.len();
        seen.sort_unstable();
        seen.dedup();
        seen.len() == n
    };
    let first = |t: &str| t.split_whitespace().next().map(str::to_lowercase);
    let last = |t: &str| t.split_whitespace().last().map(str::to_lowercase);
    let shapes = block
        .iter()
        .any(|l| word_shape(&l.text) != word_shape(&block[0].text));
    if distinct(&first) && distinct(&last) && shapes {
        return Fact::Facts;
    }
    // Lines of one kind repeat a word; lines of one shape alone (`Kundennr. 1`, `Vertrag 7`)
    // are labels or records, no block.
    let rest: Vec<Option<String>> = block[1..].iter().map(|l| first(&l.text)).collect();
    let repeated = !distinct(&first) || !distinct(&last);
    if !repeated || rest.windows(2).all(|w| w[0] == w[1]) {
        Fact::No
    } else {
        Fact::Kinds
    }
}

/// Whether lines are a postal address: a line of a postcode (a number of four digits or more)
/// and a town, and a line of a street or a PO box with its number (words, then a number of
/// at most four characters: `Musterweg 1`, `Hauptstr. 2a`, `Postfach 12 34`).
fn postal(lines: &[&str]) -> bool {
    let town = lines.iter().any(|l| {
        let mut w = l.split_whitespace();
        w.next()
            .is_some_and(|n| n.len() >= 4 && n.chars().all(|c| c.is_ascii_digit()))
            && w.next().is_some_and(|t| t.chars().any(char::is_alphabetic))
    });
    let street = lines.iter().any(|l| {
        let w: Vec<&str> = l.split_whitespace().collect();
        w.len() >= 2
            && w[0]
                .chars()
                .all(|c| c.is_alphabetic() || c == '.' || c == '-')
            && w.last().is_some_and(|n| {
                n.len() <= 4 && n.chars().next().is_some_and(|c| c.is_ascii_digit())
            })
    });
    town && street
}

/// Whether blocks of lines side by side on shared baselines are blocks of their own (an
/// address, a bank, a register entry, a contact beside each other) and not the columns of a
/// table or of labels and values. `columns` holds each column's lines by row, `None` where
/// the column has no line on a row. They are blocks when all of these hold:
/// - two columns or more and three rows or more;
/// - each column a block: its lines one under the other, with no row left empty between
///   them (a block may start a row lower or end higher than the others);
/// - each column a block of facts of its own (see [`fact_block`]): the evidence is each
///   block's, never one column's. One column whose lines are of one kind ([`Fact::Kinds`]:
///   two phone numbers, two banks) counts too, when a column of the region is a [`postal`]
///   address;
/// - one column an [`address_block`].
///
/// Their lines share baselines, so rows were possible too: the caller says the doubt. A first
/// row that is a header (bold over lines that are not, or a size step larger) over such blocks
/// may be their headings or a table's titles: [`Columns::Headed`]. Lines that are no such
/// blocks but where a column holds a postal address (and no column of labels without a digit
/// stands left of it, and no column of amounts beside it) are read as rows with the doubt
/// said: [`Columns::Doubt`].
pub(crate) fn address_columns(columns: &[Vec<Option<ColumnLine>>]) -> Columns {
    let rows = columns.iter().map(Vec::len).max().unwrap_or(0);
    if columns.len() < 2 || rows < 3 {
        return Columns::Rows;
    }
    let text_of = |c: &[Option<ColumnLine>]| -> Vec<String> {
        c.iter().flatten().map(|l| l.text.clone()).collect()
    };
    let postal_at = columns
        .iter()
        .position(|c| postal(&text_of(c).iter().map(String::as_str).collect::<Vec<_>>()));
    let has_postal = postal_at.is_some();
    // A column without a digit left of the address is its labels (`Name`, `Strasse`, `Ort`):
    // rows, no doubt.
    let labels = postal_at.is_some_and(|p| {
        columns[..p].iter().any(|c| {
            !text_of(c)
                .iter()
                .any(|t| t.chars().any(|ch| ch.is_ascii_digit()))
        })
    });
    // A column of amounts beside it makes rows of a table: no doubt either.
    let amounts = columns
        .iter()
        .any(|c| c.iter().flatten().all(|l| data_only(&l.text)));
    let rows_or_doubt = if has_postal && !labels && !amounts {
        Columns::Doubt
    } else {
        Columns::Rows
    };
    // Each column's lines, one under the other without a gap (a block may start a row
    // lower than the others, or end higher).
    let mut blocks: Vec<Vec<&ColumnLine>> = Vec::with_capacity(columns.len());
    let mut kinds = 0;
    for c in columns {
        let start = c.iter().take_while(|l| l.is_none()).count();
        let block: Vec<&ColumnLine> = c[start..].iter().map_while(Option::as_ref).collect();
        if c[start + block.len()..].iter().any(Option::is_some) {
            return rows_or_doubt;
        }
        match fact_block(&block) {
            Fact::Facts => {}
            Fact::Kinds => kinds += 1,
            Fact::No => return rows_or_doubt,
        }
        blocks.push(block);
    }
    if kinds > 1 || (kinds == 1 && !has_postal) {
        return rows_or_doubt;
    }
    let addressed = blocks
        .iter()
        .any(|b| address_block(&b.iter().map(|l| l.text.as_str()).collect::<Vec<_>>()));
    if !addressed {
        return rows_or_doubt;
    }
    // A header: the first row, a line in every column, bold over lines that are not, or a size
    // step larger.
    let Some(first) = columns
        .iter()
        .map(|c| c.first().and_then(Option::as_ref))
        .collect::<Option<Vec<&ColumnLine>>>()
    else {
        return Columns::Blocks;
    };
    let rest: Vec<&ColumnLine> = blocks.iter().flat_map(|b| b[1..].iter().copied()).collect();
    let bold_head = first.iter().all(|l| l.bold) && rest.iter().any(|l| !l.bold);
    let smallest_head = first.iter().map(|l| l.size).fold(f32::MAX, f32::min);
    let largest_rest = rest.iter().map(|l| l.size).fold(0.0f32, f32::max);
    if bold_head || smallest_head >= crate::order::SIZE_STEP * largest_rest {
        Columns::Headed
    } else {
        Columns::Blocks
    }
}

/// The shape of a line by its words: for each run of words with a digit and each run of words
/// without one, whether it holds a digit. `Musterweg 1` and `Feld 2` have one shape, `Muster
/// GmbH` and `12345 Musterstadt` each another.
fn word_shape(text: &str) -> Vec<bool> {
    let mut out: Vec<bool> = Vec::new();
    for w in text.split_whitespace() {
        let digit = w.chars().any(|c| c.is_ascii_digit());
        if out.last() != Some(&digit) {
            out.push(digit);
        }
    }
    out
}

/// Words that start with a small letter, not counting name particles ("von der Heide",
/// "Maria de la Cruz") before a capitalised word. German articles on their own are no
/// particles: they precede nouns in prose.
fn small_words(text: &str) -> usize {
    const PARTICLES: [&str; 17] = [
        "von", "van", "de", "del", "della", "di", "da", "du", "la", "le", "ten", "ter", "zu",
        "zum", "zur", "bin", "al",
    ];
    const ARTICLES: [&str; 4] = ["der", "den", "dem", "des"];
    const LINKS: [&str; 3] = ["von", "van", "zu"];
    let words: Vec<&str> = text.split_whitespace().collect();
    let capital =
        |w: Option<&&str>| w.is_some_and(|w| w.chars().next().is_some_and(char::is_uppercase));
    let mut count = 0;
    for (k, w) in words.iter().enumerate() {
        if !w.chars().next().is_some_and(char::is_lowercase) {
            continue;
        }
        let next = words.get(k + 1);
        let exempt = if PARTICLES.contains(w) {
            capital(next)
                || (LINKS.contains(w)
                    && next.is_some_and(|n| ARTICLES.contains(n))
                    && capital(words.get(k + 2)))
        } else {
            ARTICLES.contains(w) && k > 0 && LINKS.contains(&words[k - 1]) && capital(next)
        };
        if !exempt {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::super::{Gaps, line};
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
    fn a_star_parted_by_a_piece_gap_is_no_bullet() {
        let mark = Run {
            w: 6.0,
            ..run("* ", 0, 700.0, 10.0)
        };
        let far = Run {
            x: 120.0,
            ..run("Betrag", 0, 700.0, 10.0)
        };
        let near = Run {
            x: 80.0,
            ..run("Punkt", 0, 700.0, 10.0)
        };
        let padded = [mark.clone(), far];
        assert!(padded_mark(&line(&padded)));
        let bullet = [mark, near];
        assert!(!padded_mark(&line(&bullet)));
        // A dash bullet at a tab stop: its space stands before the text.
        let dash = Run {
            w: 6.0,
            ..run("-", 0, 700.0, 10.0)
        };
        let tabbed = Run {
            x: 120.0,
            ..run(" Punkt", 0, 700.0, 10.0)
        };
        let tab_stop = [dash, tabbed];
        assert!(!padded_mark(&line(&tab_stop)));
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

    #[test]
    fn lines_of_data_only() {
        assert!(data_only("1234567 8901 2 30"));
        assert!(data_only("01.03.2026"));
        assert!(data_only("00000000000"));
        assert!(data_only("1.234,56 EUR"));
        assert!(!data_only("Bericht 2025"));
        assert!(!data_only("EUR"));
        assert!(!data_only("Rechnung"));
    }

    #[test]
    fn address_blocks_have_lines_of_more_than_one_shape() {
        assert!(address_block(&[
            "Muster GmbH",
            "Musterweg 1",
            "12345 Musterstadt"
        ]));
        assert!(address_block(&[
            "Musterbank",
            "IBAN DE00 0000 0000 0000 0000 00",
            "BIC MUSTDEXXXXX"
        ]));
        // The labels or values of one form in a column of a table.
        assert!(!address_block(&["Label 0", "Label 1", "Label 2"]));
        assert!(!address_block(&[
            "Musterweg 1",
            "Musterweg 2",
            "Musterweg 3"
        ]));
        assert!(!address_block(&[]));
    }

    /// Columns of `rows`, each row the column texts left to right (`""` for none), plain and
    /// of one size; the first row bold when `bold_head`.
    fn columns_of(rows: &[&[&str]], bold_head: bool) -> Vec<Vec<Option<ColumnLine>>> {
        let n = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        (0..n)
            .map(|c| {
                rows.iter()
                    .enumerate()
                    .map(|(k, r)| {
                        r.get(c).filter(|t| !t.is_empty()).map(|t| ColumnLine {
                            text: (*t).to_string(),
                            bold: bold_head && k == 0,
                            size: 8.0,
                        })
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn address_columns_are_blocks_and_tables_are_rows() {
        let blocks: [&[&str]; 4] = [
            &["Muster GmbH", "Musterbank", "Amtsgericht Musterstadt"],
            &[
                "Musterweg 1",
                "IBAN DE00 0000 0000 0000 0000 00",
                "HRB 00000",
            ],
            &[
                "12345 Musterstadt",
                "BIC MUSTDEXXXXX",
                "USt-IdNr. DE000000000",
            ],
            &["Deutschland", "", "Erika Mustermann"],
        ];
        assert_eq!(
            address_columns(&columns_of(&blocks, false)),
            Columns::Blocks
        );
        let two: Vec<&[&str]> = blocks[..3].iter().map(|r| &r[..2]).collect();
        assert_eq!(address_columns(&columns_of(&two, false)), Columns::Blocks);
        // Under a bold row of their headings: the table they make, with the doubt said.
        let headed: [&[&str]; 4] = [
            &["Anschrift", "Bankverbindung", "Register"],
            blocks[0],
            blocks[1],
            blocks[2],
        ];
        assert_eq!(address_columns(&columns_of(&headed, true)), Columns::Headed);
        // A table with a bold header, or whose columns list records under their titles.
        let table: [&[&str]; 4] = [
            &["Strasse", "Ort"],
            &["Musterweg 1", "12345 Musterstadt"],
            &["Musterweg 2", "23456 Musterdorf"],
            &["Musterweg 3", "34567 Musterhausen"],
        ];
        assert_eq!(address_columns(&columns_of(&table, true)), Columns::Rows);
        assert_eq!(address_columns(&columns_of(&table, false)), Columns::Rows);
        // Labels without a digit beside an address.
        let labelled: [&[&str]; 3] = [
            &["Name", "Erika Mustermann"],
            &["Strasse", "Musterweg 1"],
            &["Ort", "12345 Musterstadt"],
        ];
        assert_eq!(
            address_columns(&columns_of(&labelled, false)),
            Columns::Rows
        );
        // Labels and values with a value left empty between others.
        let notice: [&[&str]; 4] = [
            &["Name", "Erika Mustermann"],
            &["Zimmer 2", "Raum 101"],
            &["Steuerjahr 2025", ""],
            &["Bearbeitung", "Max Mustermann"],
        ];
        assert_eq!(address_columns(&columns_of(&notice, false)), Columns::Rows);
        // Amounts beside an address are rows of a table.
        let amounts: [&[&str]; 3] = [
            &["Muster GmbH", "100,00"],
            &["Musterweg 1", "20,00"],
            &["12345 Musterstadt", "5,00"],
        ];
        assert_eq!(address_columns(&columns_of(&amounts, false)), Columns::Rows);
        // A column of more lines than an address is no block; its address makes the rows
        // doubtful, which is said.
        let long: Vec<&[&str]> = (0..6).map(|k| blocks[k % 3]).collect();
        assert_eq!(address_columns(&columns_of(&long, false)), Columns::Doubt);
        // Small tables whose columns list records of one kind (part 2c review, Critical 1),
        // with a plain header or none, and line items with amounts.
        let items: [&[&str]; 5] = [
            &["Artikel", "Lager", "Menge"],
            &["Schraube M4", "Halle 2", "12 Stueck"],
            &["Mutter M5", "Regal 3B", "8 Stueck"],
            &["Scheibe", "Halle 1", "20 Stueck"],
            &["Duebel 6", "Halle 3", "5 Stueck"],
        ];
        for n in [4, 5] {
            assert_eq!(
                address_columns(&columns_of(&items[..n], false)),
                Columns::Rows
            );
        }
        let shipments: [&[&str]; 3] = [
            &["Muster GmbH", "Lieferung 12", "3 Kartons"],
            &["Beispiel AG", "Lieferung 13", "12 Paletten"],
            &["Muster KG 2", "Ruecksendung", "1 Karton"],
        ];
        assert_eq!(
            address_columns(&columns_of(&shipments, false)),
            Columns::Rows
        );
        let lines: [&[&str]; 3] = [
            &["Beratung 2 Std", "1.000,00 EUR"],
            &["Material", "200,00 EUR"],
            &["Versand 3", "34,56 EUR"],
        ];
        assert_eq!(address_columns(&columns_of(&lines, false)), Columns::Rows);
        // Re-review r1 (Important 1): beside a postal address, a block of lines of one kind (two
        // banks) or under a colon heading is a block; labels beside an address are rows, said.
        let banks: [&[&str]; 4] = [
            &["Muster GmbH", "Musterbank"],
            &["Musterweg 1", "IBAN DE00 0000 0000 0000 0000 00"],
            &["12345 Musterstadt", "Beispielbank"],
            &["Deutschland", "IBAN DE11 1111 1111 1111 1111 11"],
        ];
        assert_eq!(address_columns(&columns_of(&banks, false)), Columns::Blocks);
        let colon: [&[&str]; 4] = [
            &["Muster GmbH", "Bankverbindung:"],
            &["Musterweg 1", "Musterbank"],
            &["12345 Musterstadt", "IBAN DE00 0000 0000 0000 0000 00"],
            &["Deutschland", ""],
        ];
        assert_eq!(address_columns(&columns_of(&colon, false)), Columns::Blocks);
        let labelled: [&[&str]; 3] = [
            &["Kundennr. 1", "Muster GmbH"],
            &["Vertrag 7", "Musterweg 1"],
            &["Tarif A2", "12345 Musterstadt"],
        ];
        assert_eq!(
            address_columns(&columns_of(&labelled, false)),
            Columns::Doubt
        );
    }

    #[test]
    fn two_address_lines_have_numbers_in_another_order() {
        assert!(address_lines(&["Musterweg 1", "12345 Musterstadt"]));
        assert!(address_lines(&["Am Hang 3", "11111 Neudorf"]));
        // Three lines are no two (see `address_shaped`).
        assert!(!address_lines(&[
            "Muster GmbH",
            "Musterweg 1",
            "12345 Musterstadt"
        ]));
        // A label over its value: a number on one line at most, or in the same order.
        assert!(!address_lines(&["Strasse", "Musterweg 1"]));
        assert!(!address_lines(&["Nr", "4711"]));
        assert!(!address_lines(&["Zimmer 2", "Raum 101"]));
        assert!(!address_lines(&["Name", "Erika Mustermann"]));
    }

    #[test]
    fn address_shapes() {
        assert!(address_shaped(&[
            "Erika Mustermann",
            "Musterweg 1",
            "12345 Musterstadt"
        ]));
        // An IBAN in groups is one word.
        assert!(address_shaped(&[
            "Muster GmbH",
            "IBAN DE00 0000 0000 0000 0000 00",
            "BIC MUSTDEXXXXX"
        ]));
        assert_eq!(word_count("IBAN DE00 0000 0000 0000 0000 00"), 2);
        // Two lines, no digit after the first, prose.
        assert!(!address_shaped(&["Erika Mustermann", "Musterweg 1"]));
        assert!(!address_shaped(&[
            "Muster GmbH",
            "Abteilung Einkauf",
            "Musterstadt"
        ]));
        assert!(!address_shaped(&[
            "die Ware wird",
            "am 1. Mai geliefert",
            "und bezahlt"
        ]));
        assert!(!address_shaped(&["Muster GmbH", "12345", "00000"]));
    }

    #[test]
    fn bold_leads_a_heading_line() {
        let bold = |text: &str| Run {
            bold: true,
            ..run(text, 0, 700.0, 9.0)
        };
        let plain = |text: &str| run(text, 0, 700.0, 9.0);
        let all = [bold("Rechnung")];
        assert!(bold_led(&line(&all)));
        let section = [bold("SECTION 1"), plain(" (TO BE COMPLETED BY PAYEE)")];
        assert!(bold_led(&line(&section)));
        let label = [bold("Kundennummer:"), plain(" 00000")];
        assert!(!bold_led(&line(&label)));
        let value = [bold("Rechnungsnummer"), plain(" RE-2026-001")];
        assert!(!bold_led(&line(&value)));
        let plain_first = [plain("Seite"), bold(" 1")];
        assert!(!bold_led(&line(&plain_first)));
    }
}
