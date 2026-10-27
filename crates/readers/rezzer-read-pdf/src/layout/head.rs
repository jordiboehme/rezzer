// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the head of page 1 says about the document: its title and its date. Both are found
//! by general rules; a title or date the rules are not sure of is left out (a date with an
//! `Uncertain` loss that says why).

use rezzer_model::FoundDate;

use super::{
    EDGE, GAP_RATIO, HEADING_PLAIN_RATIO, Line, Run, SIZE_SLACK, address_shaped, heading_block,
    ink, lines, size_key,
};
use crate::scan;

/// What `title_from` says of a title [`page_title`] found.
pub(crate) const TITLE_FROM: &str = "largest heading in the top third of page 1";

/// A title holds at most this many characters.
const TITLE_CHARS: usize = 120;

/// The head of a page: the top third of its height.
const HEAD_SHARE: f32 = 2.0 / 3.0;

/// The words that mark a date as the document's date, a list per language (German,
/// English, Italian or Portuguese, Spanish). A label matches as a whole, case folded, with a
/// colon after it or not: `Eintrittsdatum` is no date label.
pub(crate) const DATE_LABELS: [&str; 8] = [
    "datum",
    "date",
    "data",
    "fecha",
    "dated",
    "issued",
    "erstellt am",
    "ausgestellt am",
];

/// A label line looks for its value among this many lines after it in reading order.
const VALUE_LINES: usize = 8;

/// A heading that may be the title: its size key, whether all its ink is bold, its top and
/// its text.
struct Candidate {
    size: i32,
    bold: bool,
    top: f32,
    text: String,
}

impl Candidate {
    /// Whether it wins over `other`: larger, else bold where `other` is not, else higher.
    fn beats(&self, other: &Candidate) -> bool {
        (self.size, self.bold, self.top) > (other.size, other.bold, other.top)
    }
}

/// The title of a page, from its lines in reading order: among its headings (each with the
/// lines `heading` gives its level and that go on one another, as the layout joins them)
/// whose top lies in the top third of the page (`height`), the one of the largest size
/// wins, whatever its level; on a tie the one whose ink is all bold, then the highest. A
/// heading the layout writes as a paragraph is none. A heading a little larger than the body
/// (`body_size`) that opens an address is skipped (see [`opens_address`]). At most
/// [`TITLE_CHARS`] characters, cut at a word.
pub(crate) fn page_title(
    lines: &[Line<'_>],
    heading: &dyn Fn(&Line<'_>) -> Option<u8>,
    height: f32,
    body_size: f32,
) -> Option<String> {
    if height.is_nan() || height <= 0.0 {
        return None;
    }
    let mut best: Option<Candidate> = None;
    let mut i = 0;
    while i < lines.len() {
        let first = &lines[i];
        let level = (!first.text.is_empty()).then(|| heading(first)).flatten();
        let Some(level) = level else {
            i += 1;
            continue;
        };
        let (j, is_heading) = heading_block(lines, i, level, heading, body_size);
        let block = &lines[i..j];
        let top = first.y + first.h;
        let small = first.size < HEADING_PLAIN_RATIO * body_size;
        if is_heading && top >= height * HEAD_SHARE && !(small && opens_address(lines, i, j)) {
            let words: Vec<&str> = block
                .iter()
                .flat_map(|l| l.text.split_whitespace())
                .collect();
            let candidate = Candidate {
                size: size_key(first.size),
                bold: block
                    .iter()
                    .flat_map(|l| l.runs.iter())
                    .filter(|r| ink(&r.text) > 0)
                    .all(|r| r.bold),
                top,
                text: words.join(" "),
            };
            if best.as_ref().is_none_or(|b| candidate.beats(b)) {
                best = Some(candidate);
            }
        }
        i = j;
    }
    best.map(|b| capped(&b.text)).filter(|t| !t.is_empty())
}

/// `text` cut to at most [`TITLE_CHARS`] characters, at the last space within them when there
/// is one.
fn capped(text: &str) -> String {
    if text.chars().count() <= TITLE_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(TITLE_CHARS + 1).collect();
    match cut.rfind(' ') {
        Some(at) if at > 0 => cut[..at].trim_end().to_string(),
        _ => cut.chars().take(TITLE_CHARS).collect(),
    }
}

/// An address block holds at most this many lines.
const ADDRESS_LINES: usize = 8;

/// Whether the heading of `lines[i..j]` opens an address: its lines share one left edge and,
/// with some of the lines right below it at that edge (each right below the one before, see
/// [`right_below`], and not larger than the heading), have the shape of an address (see
/// [`address_shaped`]): the heading and the first lines below, three or more in all and at
/// most [`ADDRESS_LINES`]. The lines below may be smaller than the heading: a name in bold
/// over its street and town; a longer line after the town does not hide the address.
fn opens_address(lines: &[Line<'_>], i: usize, j: usize) -> bool {
    let first = &lines[i];
    let at_edge = |l: &Line<'_>| (l.x - first.x).abs() <= EDGE;
    if !lines[i..j].iter().all(at_edge) {
        return false;
    }
    let mut k = j;
    while k < lines.len()
        && k - i < ADDRESS_LINES
        && at_edge(&lines[k])
        && right_below(&lines[k - 1], &lines[k])
        && lines[k].size <= first.size + SIZE_SLACK
    {
        k += 1;
    }
    let texts: Vec<&str> = lines[i..k].iter().map(|l| l.text.as_str()).collect();
    (3..=texts.len()).any(|n| address_shaped(&texts[..n]))
}

/// Whether `cur` stands right below `prev`: under it, with a gap of at most [`GAP_RATIO`]
/// line heights, whatever their sizes.
fn right_below(prev: &Line<'_>, cur: &Line<'_>) -> bool {
    let gap = prev.y - cur.y;
    gap > 0.0 && gap <= GAP_RATIO * prev.h.max(cur.h).max(1.0)
}

/// What page 1 says its date is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageDate {
    /// No date, or none Rezzer can read for sure.
    None,
    Found(FoundDate),
    /// Several dates and no way to tell which one is the document's: the words of the loss.
    Unsure(String),
}

/// A date of the page: as written (to tell dates apart), as `YYYY-MM-DD` when it reads for
/// sure, and the label it stands after, if one.
struct Seen<'t> {
    written: (char, [u32; 3]),
    iso: Option<String>,
    label: Option<&'t str>,
}

/// The document date page 1 gives, from its runs, with `height` the height of the page. A
/// date right after a date label of [`DATE_LABELS`], on the label's line or as its value (the
/// line beside it or right below it), wins; dates after labels that differ give no date.
/// Else the only date in the top third of the page is the document's; several there give no
/// date. Only full dates count, with a year of four digits: `dd.mm.yyyy` and `yyyy-mm-dd`
/// read as written, `a/b/yyyy` only when one part is over 12, and only a day the calendar
/// has.
pub(crate) fn page_document_date(runs: &[Run], height: Option<f32>) -> PageDate {
    let all = lines(runs);
    let mut labelled: Vec<Seen<'_>> = Vec::new();
    let mut head: Vec<Seen<'_>> = Vec::new();
    let in_head = |l: &Line<'_>| height.is_some_and(|h| h > 0.0 && l.y + l.h >= h * HEAD_SHARE);
    for (k, l) in all.iter().enumerate() {
        for d in scan::dates(&l.text, 4..=4) {
            let seen = Seen {
                written: (d.sep, d.parts),
                iso: iso(&d),
                label: label_before(&l.text[..d.start]),
            };
            if seen.label.is_some() {
                labelled.push(seen);
            } else if in_head(l) {
                head.push(seen);
            }
        }
        // A label alone on its line: its value is the line beside it or right below it.
        if let Some(label) = whole_label(&l.text)
            && let Some(v) = value_of(&all, k)
            && let Some(d) = scan::dates(&v.text, 4..=4).into_iter().next()
        {
            labelled.push(Seen {
                written: (d.sep, d.parts),
                iso: iso(&d),
                label: Some(label),
            });
        }
    }
    if let Some(first) = labelled.first() {
        let kinds = distinct(&labelled);
        if kinds > 1 {
            return PageDate::Unsure(format!(
                "{kinds} different dates stand after a date label on page 1; none is taken as \
                 the document date"
            ));
        }
        return match (&first.iso, first.label) {
            (Some(date), Some(label)) => PageDate::Found(FoundDate {
                date: date.clone(),
                from: format!("date label {label} on page 1"),
            }),
            _ => PageDate::None,
        };
    }
    match distinct(&head) {
        0 => PageDate::None,
        1 => match &head[0].iso {
            Some(date) => PageDate::Found(FoundDate {
                date: date.clone(),
                from: "only date in the top third of page 1".to_string(),
            }),
            None => PageDate::None,
        },
        n => PageDate::Unsure(format!(
            "{n} dates stand in the head of page 1; none is marked as the document date"
        )),
    }
}

/// The number of dates among `seen` that are written differently.
fn distinct(seen: &[Seen<'_>]) -> usize {
    let mut kinds: Vec<(char, [u32; 3])> = seen.iter().map(|s| s.written).collect();
    kinds.sort_unstable();
    kinds.dedup();
    kinds.len()
}

/// The date label `before` ends with (white space and a colon after it aside), as written,
/// when it stands as a whole: no letter, digit or hyphen right before it (`Liefer-Datum` is
/// a word of its own), and no word right before it either (`Due date`, `Invoice date`: a
/// label of a word group is none of the list). A digit or a sign before it may stand
/// (`Kundennr. 4711 Datum`, `Ort, Datum`).
fn label_before(before: &str) -> Option<&str> {
    let before = before.trim_end();
    let before = before.strip_suffix(':').unwrap_or(before).trim_end();
    DATE_LABELS.iter().find_map(|label| {
        let n = label.chars().count();
        let start = before.char_indices().rev().nth(n - 1)?.0;
        let tail = &before[start..];
        let rest = &before[..start];
        let alone = rest
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '-')
            && rest
                .trim_end()
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphabetic());
        (alone && tail.to_lowercase() == *label).then_some(tail)
    })
}

/// The date label a line is in whole (with a colon after it or not), as written.
fn whole_label(text: &str) -> Option<&str> {
    let text = text.trim();
    let bare = text.strip_suffix(':').unwrap_or(text).trim_end();
    label_before(bare).filter(|l| l.len() == bare.len())
}

/// The value of the label line `all[k]`: among the next [`VALUE_LINES`] lines, the nearest
/// that starts with a date and stands beside it on its baseline (to its right) or right below
/// it at its left edge (within its size).
fn value_of<'a, 'r>(all: &'a [Line<'r>], k: usize) -> Option<&'a Line<'r>> {
    let label = &all[k];
    let starts_with_date = |l: &Line<'_>| {
        scan::dates(&l.text, 4..=4)
            .first()
            .is_some_and(|d| d.start == 0)
    };
    let after = all.iter().skip(k + 1).take(VALUE_LINES);
    let beside = after
        .clone()
        .filter(|v| (v.y - label.y).abs() <= label.size / 2.0 && v.x > label.x)
        .filter(|v| starts_with_date(v))
        .min_by(|a, b| a.x.total_cmp(&b.x));
    beside.or_else(|| {
        after
            .filter(|v| right_below(label, v) && (v.x - label.x).abs() <= label.size)
            .filter(|v| starts_with_date(v))
            .max_by(|a, b| a.y.total_cmp(&b.y))
    })
}

/// A date as `YYYY-MM-DD`, when it reads for sure: a dotted date day first, a dashed one year
/// first, a slashed one only when one of its first two parts is over 12 (that part is the
/// day); and only a day the calendar has.
fn iso(d: &scan::Date) -> Option<String> {
    let [a, b, c] = d.parts;
    let (year, month, day) = match d.sep {
        '.' => (c, b, a),
        '-' => (a, b, c),
        _ if a > 12 => (c, b, a),
        _ if b > 12 => (c, a, b),
        _ => return None,
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (year >= 1 && (1..=days).contains(&day)).then(|| format!("{year:04}-{month:02}-{day:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_stand_whole() {
        assert_eq!(label_before("Datum "), Some("Datum"));
        assert_eq!(label_before("Ort, Datum:"), Some("Datum"));
        assert_eq!(label_before("Liefer-Datum:"), None);
        assert_eq!(label_before("Due date"), None);
        assert_eq!(label_before("Kundennr. 4711 Datum"), Some("Datum"));
        assert_eq!(label_before("Erstellt am"), Some("Erstellt am"));
        assert_eq!(label_before("Eintrittsdatum"), None);
        assert_eq!(label_before("Update"), None);
        assert_eq!(label_before(""), None);
        assert_eq!(whole_label("DATE:"), Some("DATE"));
        assert_eq!(whole_label("Datum der Zahlung"), None);
    }

    #[test]
    fn dates_read_for_sure_only() {
        let one = |t: &str| iso(&scan::dates(t, 4..=4)[0]);
        assert_eq!(one("01.02.2026").as_deref(), Some("2026-02-01"));
        assert_eq!(one("2026-02-01").as_deref(), Some("2026-02-01"));
        assert_eq!(one("13/02/2026").as_deref(), Some("2026-02-13"));
        assert_eq!(one("02/13/2026").as_deref(), Some("2026-02-13"));
        assert_eq!(one("02/03/2026"), None);
        assert_eq!(one("31.04.2026"), None);
        assert_eq!(one("29.02.2024").as_deref(), Some("2024-02-29"));
        assert_eq!(one("29.02.2026"), None);
    }

    #[test]
    fn long_titles_are_cut_at_a_word() {
        let long = "Wort ".repeat(30);
        let cut = capped(long.trim());
        assert!(cut.chars().count() <= TITLE_CHARS, "{cut}");
        assert!(cut.ends_with("Wort"), "{cut}");
        let one = "\u{e4}".repeat(130);
        assert_eq!(capped(&one).chars().count(), TITLE_CHARS);
    }
}
