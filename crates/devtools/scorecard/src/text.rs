// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! How texts are compared: white space is normalised, `<br>` is a line break, a text is found
//! in another only at word bounds, and amounts are compared by value, not by how they are
//! printed.

/// The line break a table cell is written with in Markdown.
const BREAK: &str = "<br>";

/// `text` with every run of white space as one space, `<br>` as white space, and none at the
/// ends.
pub fn norm(text: &str) -> String {
    let text = text.replace(BREAK, " ");
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// The lines of `text`, each normalised: a line ends at a line feed or at `<br>`. Empty lines
/// are kept, so that the n-th line stays the n-th.
pub fn lines(text: &str) -> Vec<String> {
    text.replace(BREAK, "\n").split('\n').map(norm).collect()
}

/// Whether a character is part of a word, for the bounds of [`find_words`].
fn wordy(c: char) -> bool {
    c.is_alphanumeric()
}

/// Where `needle` stands in `hay` at or after byte `from`, at word bounds: an end of the
/// needle that is a letter or a digit has no letter or digit next to it in `hay`. Both texts
/// are normalised by the caller. An empty needle is never found.
pub fn find_words(hay: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() || from > hay.len() {
        return None;
    }
    let first = needle.chars().next().is_some_and(wordy);
    let last = needle.chars().next_back().is_some_and(wordy);
    let mut start = from;
    while let Some(at) = hay.get(start..).and_then(|rest| rest.find(needle)) {
        let at = start + at;
        let end = at + needle.len();
        let before = hay[..at].chars().next_back().is_some_and(wordy);
        let after = hay[end..].chars().next().is_some_and(wordy);
        if !(first && before) && !(last && after) {
            return Some(at);
        }
        // The next place to look: one character on.
        start = at + hay[at..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// Whether `needle` stands in `hay` at word bounds (see [`find_words`]).
pub fn has_words(hay: &str, needle: &str) -> bool {
    find_words(hay, needle, 0).is_some()
}

/// How often `needle` stands in `hay` at word bounds, places that overlap counted each.
pub fn count_words(hay: &str, needle: &str) -> usize {
    let mut n = 0;
    let mut from = 0;
    while let Some(at) = find_words(hay, needle, from) {
        n += 1;
        from = at + hay[at..].chars().next().map_or(1, char::len_utf8);
    }
    n
}

/// An amount: its sign, the digits before the decimal point without leading zeros (`0` for
/// none) and the digits after it without trailing zeros. `1.50`, `1,5` and `01.500` are the
/// same amount; `-1.5` is another. Zero has no sign.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Number {
    negative: bool,
    whole: String,
    fraction: String,
}

impl Number {
    fn new(negative: bool, whole: &str, fraction: &str) -> Number {
        let whole = whole.trim_start_matches('0');
        let whole = if whole.is_empty() { "0" } else { whole };
        let fraction = fraction.trim_end_matches('0');
        Number {
            negative: negative && !(whole == "0" && fraction.is_empty()),
            whole: whole.to_string(),
            fraction: fraction.to_string(),
        }
    }

    /// The amount of a plain decimal as a file's data writes it: an optional sign, digits,
    /// then at most one point and digits (`160.00`, `-4.5`, `12`). `None` for anything else.
    pub fn parse(text: &str) -> Option<Number> {
        let text = text.trim();
        let (negative, text) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
            return None;
        }
        Some(Number::new(negative, whole, fraction))
    }

    /// Whether the amount is zero.
    pub fn is_zero(&self) -> bool {
        self.whole == "0" && self.fraction.is_empty()
    }
}

impl std::fmt::Display for Number {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.negative {
            write!(f, "-")?;
        }
        if self.fraction.is_empty() {
            write!(f, "{}", self.whole)
        } else {
            write!(f, "{}.{}", self.whole, self.fraction)
        }
    }
}

/// The mark a document parts decimals with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Decimal {
    Comma,
    Point,
    /// The document does not show it, or shows both as often.
    #[default]
    Unknown,
}

/// What a printed token of digits, points and commas says.
enum Reading {
    /// One amount, and the mark that parts its decimals when the token shows it.
    Sure(String, String, Option<char>),
    /// `1.234` or `1,234`: one mark, three digits after it, one to three before. The mark may
    /// part thousands or decimals; the document decides (see [`numbers_in`]).
    Open(String, String, char),
    /// No amount: a date, a version, a number of an article (`21.11.2020`, `1.2.3`).
    None,
}

/// Reads one token: digits with points, commas and apostrophes between them. An apostrophe
/// parts thousands. With a point and a comma, the one that comes last parts the decimals and
/// the other parts groups of three. One mark more than once parts groups of three.
fn reading(token: &str) -> Reading {
    let token: String = token.chars().filter(|c| *c != '\'').collect();
    let points = token.matches('.').count();
    let commas = token.matches(',').count();
    // Groups of three digits after a first group of one to three.
    let grouped = |text: &str, mark: char| {
        let mut groups = text.split(mark);
        groups.next().is_some_and(|g| (1..=3).contains(&g.len())) && groups.all(|g| g.len() == 3)
    };
    let other = |mark: char| if mark == '.' { ',' } else { '.' };
    match (points, commas) {
        (0, 0) => Reading::Sure(token, String::new(), None),
        (p, c) if p > 0 && c > 0 => {
            let decimal = if token.rfind('.') > token.rfind(',') {
                '.'
            } else {
                ','
            };
            let (whole, fraction) = token.rsplit_once(decimal).unwrap_or((&token, ""));
            if whole.contains(decimal) || !grouped(whole, other(decimal)) {
                return Reading::None;
            }
            Reading::Sure(
                whole.replace(other(decimal), ""),
                fraction.to_string(),
                Some(decimal),
            )
        }
        _ => {
            let (mark, count) = if commas == 0 {
                ('.', points)
            } else {
                (',', commas)
            };
            if count > 1 {
                if !grouped(&token, mark) {
                    return Reading::None;
                }
                return Reading::Sure(token.replace(mark, ""), String::new(), Some(other(mark)));
            }
            let (whole, fraction) = token.split_once(mark).unwrap_or((&token, ""));
            let open =
                fraction.len() == 3 && (1..=3).contains(&whole.len()) && !whole.starts_with('0');
            if open {
                Reading::Open(whole.to_string(), fraction.to_string(), mark)
            } else {
                Reading::Sure(whole.to_string(), fraction.to_string(), Some(mark))
            }
        }
    }
}

/// A minus sign as documents print it.
fn minus(c: char) -> bool {
    matches!(c, '-' | '\u{2212}')
}

/// The printed tokens of `text` that may be amounts, each with whether it carries a minus:
/// runs of digits, points, commas and apostrophes that start and end with a digit. A minus
/// right before the token counts when no letter or digit stands before it (`-5,00`, not
/// `RE-2026` or `1-2`); a minus right after it counts when none stands after it (`5,00-`).
fn tokens(text: &str) -> Vec<(bool, &str)> {
    let part = |c: char| c.is_ascii_digit() || matches!(c, '.' | ',' | '\'');
    let mut out = Vec::new();
    let mut rest = 0;
    while let Some(found) = text[rest..].find(|c: char| c.is_ascii_digit()) {
        let start = rest + found;
        let run = text[start..]
            .find(|c: char| !part(c))
            .map_or(text.len(), |n| start + n);
        let token = text[start..run].trim_end_matches(|c: char| !c.is_ascii_digit());
        let end = start + token.len();
        let mut before = text[..start].chars().rev();
        let lead = before.next().is_some_and(minus) && !before.next().is_some_and(wordy);
        let mut after = text[end..].chars();
        let trail = after.next().is_some_and(minus) && !after.next().is_some_and(wordy);
        out.push((lead || trail, token));
        rest = run.max(end);
    }
    out
}

/// The mark the amounts of `text` part their decimals with: the one more tokens show (`1,50`
/// and `1.234,56` show the comma, `1.50` and `1,234.56` the point).
pub fn decimal_of(text: &str) -> Decimal {
    let (mut comma, mut point) = (0usize, 0usize);
    for (_, token) in tokens(text) {
        match reading(token) {
            Reading::Sure(_, _, Some(',')) => comma += 1,
            Reading::Sure(_, _, Some('.')) => point += 1,
            _ => {}
        }
    }
    match comma.cmp(&point) {
        std::cmp::Ordering::Greater => Decimal::Comma,
        std::cmp::Ordering::Less => Decimal::Point,
        std::cmp::Ordering::Equal => Decimal::Unknown,
    }
}

/// The amount of one token in a document that parts decimals with `decimal`. A token that
/// may be read two ways (`1.234`) is read as its document reads: its mark parts decimals when
/// it is the document's decimal mark, and thousands when it is not or the document shows
/// none.
fn amount(negative: bool, token: &str, decimal: Decimal) -> Option<Number> {
    match reading(token) {
        Reading::Sure(whole, fraction, _) => Some(Number::new(negative, &whole, &fraction)),
        Reading::Open(whole, fraction, mark) => {
            let parts_decimals = matches!(
                (decimal, mark),
                (Decimal::Comma, ',') | (Decimal::Point, '.')
            );
            Some(if parts_decimals {
                Number::new(negative, &whole, &fraction)
            } else {
                Number::new(negative, &format!("{whole}{fraction}"), "")
            })
        }
        Reading::None => None,
    }
}

/// The amounts of groups of three digits parted by spaces that start at word `start` of
/// `words` (`1 234,56`): one for each place the number may end.
fn grouped(words: &[&str], start: usize) -> Vec<Number> {
    // One to three digits, with a minus before them or not.
    fn head(w: &str) -> Option<(bool, &str)> {
        let (negative, digits) = match w.strip_prefix(minus) {
            Some(rest) => (true, rest),
            None => (false, w),
        };
        ((1..=3).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()))
            .then_some((negative, digits))
    }
    let all_digits = |part: &[u8]| part.iter().all(u8::is_ascii_digit);
    // Three digits, then nothing or a mark and digits. By bytes: a word may hold any text.
    let tail = |w: &str| {
        let b = w.as_bytes();
        b.len() >= 3
            && all_digits(&b[..3])
            && (b.len() == 3 || (matches!(b[3], b'.' | b',') && b.len() > 4 && all_digits(&b[4..])))
    };
    let mut out = Vec::new();
    let Some((negative, first)) = words.get(start).and_then(|w| head(w)) else {
        return out;
    };
    let mut joined = first.to_string();
    for w in &words[start + 1..] {
        if !tail(w) {
            break;
        }
        let (group, rest) = w.split_at(3);
        joined.push_str(group);
        let fraction = rest.get(1..).unwrap_or("");
        out.push(Number::new(negative, &joined, fraction));
        if !rest.is_empty() {
            break;
        }
    }
    out
}

/// Every amount `text` holds, in a document that parts decimals with `decimal`, and the
/// amounts of groups of three digits parted by spaces (`1 234,56`).
pub fn numbers_in(text: &str, decimal: Decimal) -> Vec<Number> {
    let mut out: Vec<Number> = tokens(text)
        .into_iter()
        .filter_map(|(negative, token)| amount(negative, token, decimal))
        .collect();
    let words: Vec<&str> = text.split(' ').collect();
    for start in 0..words.len() {
        out.extend(grouped(&words, start));
    }
    out
}

/// What the first amount of `text` may be: the first printed number, and, when groups of
/// three digits follow it after spaces, the amounts of those groups with it. Empty when the
/// text holds no amount.
pub fn first_numbers(text: &str, decimal: Decimal) -> Vec<Number> {
    let words: Vec<&str> = text.split(' ').collect();
    let Some(at) = words
        .iter()
        .position(|w| w.chars().any(|c| c.is_ascii_digit()))
    else {
        return Vec::new();
    };
    let mut out: Vec<Number> = tokens(words[at])
        .into_iter()
        .take(1)
        .filter_map(|(negative, token)| amount(negative, token, decimal))
        .collect();
    out.extend(grouped(&words, at));
    out
}

/// Whether `text` starts with `needle`, ending at a word bound.
pub fn starts_with_words(text: &str, needle: &str) -> bool {
    !needle.is_empty() && text.starts_with(needle) && find_words(text, needle, 0) == Some(0)
}

/// Whether `text` holds the amount `number`, in a document that parts decimals with
/// `decimal`.
pub fn has_number(text: &str, number: &Number, decimal: Decimal) -> bool {
    numbers_in(text, decimal).contains(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_space_and_breaks_are_normalised() {
        assert_eq!(norm("  a \u{a0} b\n\tc<br>d "), "a b c d");
        assert_eq!(lines("a  b<br><br> c\nd"), ["a b", "", "c", "d"]);
    }

    #[test]
    fn a_text_is_found_only_at_word_bounds() {
        assert!(has_words("Summe 10,00 EUR", "10,00"));
        assert!(!has_words("Summe 110,00 EUR", "10,00"));
        assert!(!has_words("Musterstadt", "Muster"));
        assert!(has_words("(Muster) GmbH", "Muster"));
        assert!(has_words("a: b", "a:"));
        assert_eq!(find_words("ab a b a", "a", 0), Some(3));
        assert_eq!(count_words("a a aa a", "a"), 3);
        assert_eq!(count_words("aaa", ""), 0);
        // Characters of more than one byte on both sides.
        assert!(has_words("für Müller, Köln", "Müller"));
        assert!(!has_words("Müllerö", "Müller"));
    }

    fn n(s: &str) -> Number {
        Number::parse(s).unwrap()
    }

    /// Whether `text`, as its own document, holds the amount.
    fn has(text: &str, number: &str) -> bool {
        has_number(text, &n(number), decimal_of(text))
    }

    #[test]
    fn amounts_compare_by_value() {
        assert_eq!(n("160.00"), n("160"));
        assert_eq!(n("007").to_string(), "7");
        assert!(Number::parse("1,5").is_none());
        assert!(Number::parse("").is_none());
        assert!(Number::parse(".").is_none());
        assert!(Number::parse("-").is_none());
        assert!(n("0.00").is_zero());
        assert!(has("160.00 €", "160"));
        assert!(has("1.234,56 EUR", "1234.56"));
        assert!(has("1,234.56", "1234.56"));
        assert!(has("1 234,56", "1234.56"));
        assert!(has("12 345 678", "12345678"));
        assert!(has("1'234.50", "1234.5"));
        assert!(has("0,025 €", "0.025"));
        assert!(has("1.234.567", "1234567"));
        assert!(has("7%", "7"));
        assert!(has("Menge: 3.", "3"));
        // Never a part of a longer number.
        assert!(!has("160.00", "60"));
        assert!(!has("2160", "160"));
        assert!(!has("12 34", "1234"));
        assert!(!has("no digits", "0"));
        // Words of characters of more than one byte beside a number.
        assert!(has("1 \u{e4}\u{e4} 12 a\u{20ac}b 5", "12"));
        assert!(!has("7 \u{20ac}\u{20ac}", "0"));
    }

    #[test]
    fn the_first_amount_is_the_first_printed_number() {
        let first = |text: &str| first_numbers(text, decimal_of(text));
        assert_eq!(first("100,00 Brutto 119,00"), [n("100")]);
        assert_eq!(first("EUR -5,50 und 7"), [n("-5.5")]);
        assert_eq!(first("1 234,56 EUR 9"), [n("1"), n("1234.56")]);
        assert!(first("kein Betrag").is_empty());
        assert!(starts_with_words("Muster GmbH", "Muster"));
        assert!(!starts_with_words("Musterstadt", "Muster"));
        assert!(!starts_with_words("Die Muster", "Muster"));
        assert!(!starts_with_words("Muster", ""));
    }

    #[test]
    fn a_date_or_a_version_is_no_amount() {
        assert!(numbers_in("21.11.2020", Decimal::Unknown).is_empty());
        assert!(numbers_in("1.2.3", Decimal::Unknown).is_empty());
        assert!(numbers_in("1.23,456.7", Decimal::Unknown).is_empty());
        assert!(numbers_in("1,2,3.4", Decimal::Unknown).is_empty());
        // Nor does it say how the document parts decimals.
        assert_eq!(
            decimal_of("vom 21.11.2020 bis 01.12.2020"),
            Decimal::Unknown
        );
    }

    #[test]
    fn an_amount_keeps_its_sign() {
        assert_eq!(n("-0.50").to_string(), "-0.5");
        assert_ne!(n("-5"), n("5"));
        // Zero has none.
        assert_eq!(n("-0.00"), n("0"));
        assert!(has("-5,00 EUR", "-5"));
        assert!(has("5,00-", "-5"));
        assert!(has("\u{2212}1.234,56", "-1234.56"));
        assert!(has("Summe -1 234,56", "-1234.56"));
        assert!(!has("-5,00", "5"));
        assert!(!has("5,00", "-5"));
        // A hyphen in a number of a document or in a range is no sign.
        assert!(has("RE-2026", "2026"));
        assert!(has("Seite 1-2", "2"));
        assert!(has("Seite 1-2", "1"));
        assert!(has("10 - 20", "20"));
        assert!(!has("RE-2026", "-2026"));
    }

    #[test]
    fn one_mark_before_three_digits_reads_as_its_document_reads() {
        // A document with comma decimals: the point parts thousands.
        let german = "Preis 12,50 Summe 1.234 Rabatt 0,5";
        assert_eq!(decimal_of(german), Decimal::Comma);
        assert!(has(german, "1234"));
        assert!(!has(german, "1.234"));
        // A document with point decimals: the point parts decimals, the comma thousands.
        let english = "price 12.50 weight 1.234 total 1,234";
        assert_eq!(decimal_of(english), Decimal::Point);
        assert!(has(english, "1.234"));
        assert!(has(english, "1234"));
        assert!(!has("price 12.50 weight 1.234", "1234"));
        // The comma in a document with comma decimals parts decimals.
        assert!(has("Preis 12,50 Gewicht 1,234", "1.234"));
        assert!(!has("Preis 12,50 Gewicht 1,234", "1234"));
        // A document that shows no decimal mark, or both as often: thousands.
        assert_eq!(decimal_of("1.234"), Decimal::Unknown);
        assert!(has("1.234", "1234"));
        assert!(!has("1.234", "1.234"));
        assert!(has("1,50 und 1.50 und 2,345", "2345"));
        // A whole part that starts with a zero is no group of thousands.
        assert!(has("0.025", "0.025"));
        assert!(has("1.50 und 1,234.5", "1234.5"));
    }
}
