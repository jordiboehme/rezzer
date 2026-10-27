// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the head of page 1 says about the document: its title and its date. Both are found
//! by general rules; a title or date the rules are not sure of is left out (a date with an
//! `Uncertain` loss that says why).

use rezzer_model::FoundDate;

use super::{
    EDGE, GAP_RATIO, HEADING_PLAIN_RATIO, Line, Run, SIZE_SLACK, address_shaped, heading_block,
    ink, line, lines, size_key,
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
/// (`body_size`) that opens an address is skipped (see [`opens_address`]), and so is the
/// title of a table of contents or an index (see [`CONTENTS_TITLES`]). At most
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
        let words: Vec<&str> = block
            .iter()
            .flat_map(|l| l.text.split_whitespace())
            .collect();
        if is_heading
            && top >= height * HEAD_SHARE
            && !(small && opens_address(lines, i, j))
            && !contents_title(&words)
        {
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

/// Titles of a table of contents or an index, accent folded (see [`fold`]), a list per
/// language as [`DATE_LABELS`]: such a heading names a part of the document, not the
/// document.
const CONTENTS_TITLES: [&str; 10] = [
    "contents",
    "table of contents",
    "inhalt",
    "inhaltsverzeichnis",
    "sommaire",
    "table des matieres",
    "indice",
    "contenido",
    "contenuto",
    "index",
];

/// Whether the words of a heading are a title of [`CONTENTS_TITLES`], a colon after it or
/// not.
fn contents_title(words: &[&str]) -> bool {
    let text = fold(&words.join(" "));
    let text = text.strip_suffix(':').unwrap_or(&text).trim_end();
    CONTENTS_TITLES.contains(&text)
}

/// `text` in small letters with its accents folded: an umlaut as its two letters (`ä` as
/// `ae`), `ß` as `ss`, another letter with an accent as the letter (`è` as `e`).
fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            '\u{e4}' => out.push_str("ae"),
            '\u{f6}' => out.push_str("oe"),
            '\u{fc}' => out.push_str("ue"),
            '\u{df}' => out.push_str("ss"),
            '\u{e0}'..='\u{e5}' => out.push('a'),
            '\u{e7}' => out.push('c'),
            '\u{e8}'..='\u{eb}' => out.push('e'),
            '\u{ec}'..='\u{ef}' => out.push('i'),
            '\u{f1}' => out.push('n'),
            '\u{f2}'..='\u{f5}' | '\u{f8}' => out.push('o'),
            '\u{f9}'..='\u{fb}' => out.push('u'),
            '\u{fd}' | '\u{ff}' => out.push('y'),
            c => out.push(c),
        }
    }
    out
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
    /// No date, or none Rezzer can read for sure, and nothing to say about it.
    None,
    Found(FoundDate),
    /// No date, with the words of an Info `Uncertain` loss: several dates and no way to tell
    /// which one is the document's, or a labelled date that cannot be read for sure.
    Unsure(String),
}

/// Words that make the label of a date name another date than the document's (a due date, a
/// date of birth, of delivery, of validity), accent folded: a label that holds one and is not
/// a whole [`DATE_LABELS`] entry (`Due date`, `Geburtsdatum`, `Faellig am`, `Lieferung`).
const OTHER_DATE_WORDS: [&str; 12] = [
    "datum", "date", "data", "fecha", "geburt", "birth", "due", "faellig", "liefer", "delivery",
    "valid", "gueltig",
];

/// A date as written: its separator and its parts.
type Written = (char, [u32; 3]);

/// Where a date stands on page 1: the number of its line in reading order (see
/// [`Run::line`]), and the date as written.
type At = (u32, Written);

/// A label and its value as a pair table of page 1 reads them (see
/// `tables::Planned::pairs`): the value's line, if it has text, and the top of the table.
#[derive(Debug, Clone)]
pub(crate) struct Pair {
    pub label: String,
    pub value: String,
    pub line: Option<u32>,
    pub top: f32,
}

/// A date found on page 1 that a label marks as the document date, or that may be the only
/// date of the head.
#[derive(Debug, Clone)]
struct Seen {
    written: Written,
    /// The number of its line on the page.
    line: u32,
    /// The date as it stands in the text.
    text: String,
    /// The label as written, white space collapsed, when one marks it.
    label: Option<String>,
    /// A label of its own that is no document-date label stands beside it or over it (see
    /// [`page_dates`]): it counts as a date of the head but is never the document date.
    named: bool,
}

/// What page 1 holds of dates, gathered from its lines before the layout and from its laid
/// out text and pairs after it, decided once by [`document_date`].
#[derive(Debug, Default)]
pub(crate) struct Dates {
    /// Dates a document-date label marks.
    labelled: Vec<Seen>,
    /// Dates in the top third of the page without a document-date label, plain or with a label
    /// of their own (see [`Seen::named`]).
    head: Vec<Seen>,
    /// Dates a label names as another date, where each stands: never the document date, though
    /// one in the head still counts as a date of the head. Only the date at that place is
    /// meant, not another with the same value.
    other: Vec<At>,
}

impl Dates {
    /// Adds what `more` found.
    pub(crate) fn merge(&mut self, more: Dates) {
        self.labelled.extend(more.labelled);
        self.head.extend(more.head);
        self.other.extend(more.other);
    }
}

/// The pieces of the lines of `runs`, in order: each line cut where the white space between
/// two of its runs is wider than [`BESIDE_SIZES`] times the line's size. A piece is the text
/// of one column of the page on that line: a label and its date stand in one piece, and a
/// label never reaches across a column gap.
fn pieces(runs: &[Run]) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    for l in lines(runs) {
        let mut start = 0;
        for k in 1..l.runs.len() {
            let (a, b) = (&l.runs[k - 1], &l.runs[k]);
            if b.x - (a.x + a.w) > BESIDE_SIZES * l.size {
                out.push(line(&l.runs[start..k]));
                start = k;
            }
        }
        out.push(line(&l.runs[start..]));
    }
    out
}

/// The dates of the lines of page 1 (its runs as read, tables and furniture still in), read
/// piece by piece (see [`pieces`]), with `height` the height of the page: a date right after a
/// document-date label in its piece; a date in the top third, plain or with a label of its
/// own; a date whose label names another date (see [`OTHER_DATE_WORDS`]). Only full dates
/// count, with a year of four digits.
///
/// A date has a label of its own when text stands beside it or over it (see [`labels_of`]):
/// beside, the other text of its piece (from the date before it to the date after it) or a
/// text piece that ends to its left on its baseline, unless that text is a place followed by
/// a comma (`Musterstadt, 01.02.2026`, see [`place_and_comma`]); over, a piece on the line
/// right above it whose x span overlaps the date's, with any alignment (a label over an
/// indented, right aligned or centred value, a heading). Such a date is never the document
/// date (see [`document_date`]): a label Rezzer does not know may name any date.
pub(crate) fn page_dates(runs: &[Run], height: Option<f32>) -> Dates {
    let mut out = Dates::default();
    let in_head = |l: &Line<'_>| height.is_some_and(|h| h > 0.0 && l.y + l.h >= h * HEAD_SHARE);
    let all = pieces(runs);
    for l in &all {
        let line = l.runs.first().map_or(0, |r| r.line);
        let found = scan::dates(&l.text, 4..=4);
        for (i, d) in found.iter().enumerate() {
            let own = &l.text[found.get(i.wrapping_sub(1)).map_or(0, |p| p.end)..d.start];
            let after = &l.text[d.end..found.get(i + 1).map_or(l.text.len(), |n| n.start)];
            // A label stands right before its date: at most [`BESIDE_SIZES`] sizes apart.
            let label = label_before(own).filter(|_| {
                let label_end = d.start - (own.len() - own.trim_end().len());
                ink_gap(l, label_end, d.start) <= BESIDE_SIZES * l.size
            });
            if let Some(label) = label {
                out.labelled.push(Seen {
                    written: (d.sep, d.parts),
                    line,
                    text: l.text[d.start..d.end].to_string(),
                    label: Some(label),
                    named: false,
                });
                continue;
            }
            let labels = labels_of(&all, l, own, after, d.start, d.end);
            let seen = Seen {
                written: (d.sep, d.parts),
                line,
                text: l.text[d.start..d.end].to_string(),
                label: None,
                named: !labels.is_empty(),
            };
            if labels.iter().any(|t| names_other_date(t)) {
                out.other.push((line, seen.written));
            }
            if in_head(l) {
                out.head.push(seen);
            }
        }
    }
    out
}

/// The texts that label the date at bytes `start..end` of the piece `l` among the pieces
/// `all` of the page without being a document-date label (see [`page_dates`]): `own`, the
/// text before it in its piece, and `after`, the text after it, when either is text and not a
/// place followed by a comma; a text piece that ends to the left of the date on its baseline,
/// at any distance, unless it is such a place; and a piece on the line right above it (see
/// [`right_below`]) whose x span overlaps the date's within [`SPAN_SLACK`] sizes.
fn labels_of<'a>(
    all: &'a [Line<'_>],
    l: &'a Line<'_>,
    own: &'a str,
    after: &'a str,
    start: usize,
    end: usize,
) -> Vec<&'a str> {
    let beside_text = |t: &str| is_text(t) && !place_and_comma(t);
    let mut out: Vec<&str> = Vec::new();
    if beside_text(own) {
        out.push(own);
    }
    if beside_text(after) {
        out.push(after);
    }
    let (x0, x1) = span_of(l, start, end);
    let slack = SPAN_SLACK * l.size;
    for p in all.iter().filter(|p| !std::ptr::eq(*p, l)) {
        let right = p.runs.iter().map(|r| r.x + r.w).fold(p.x, f32::max);
        let beside =
            (p.y - l.y).abs() <= l.size / 2.0 && right <= x0 + slack && beside_text(&p.text);
        let over =
            right_below(p, l) && p.x <= x1 + slack && right >= x0 - slack && is_text(&p.text);
        if beside || over {
            out.push(&p.text);
        }
    }
    out
}

/// The x spans of a date and of a piece over it may miss each other by this many sizes and
/// still overlap: the place of a date inside a run is taken by its share of the run's
/// characters, which is a little off in a proportional font.
const SPAN_SLACK: f32 = 0.5;

/// Whether `text` holds a word: a letter or a digit. Signs alone (`-`, `(`, `|`) label
/// nothing.
fn is_text(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

/// Whether `text` is a place followed by a comma, as a letter writes it before its date
/// (`Musterstadt,`, `Frankfurt am Main,`, `St. Gallen,`): words in letters, with a hyphen, a
/// period, an apostrophe or a slash inside, that name no other date (see
/// [`names_other_date`]), and a comma after them.
fn place_and_comma(text: &str) -> bool {
    let Some(place) = text.trim_end().strip_suffix(',') else {
        return false;
    };
    let place = place.trim();
    place.chars().any(char::is_alphabetic)
        && place
            .chars()
            .all(|c| c.is_alphabetic() || c.is_whitespace() || matches!(c, '-' | '.' | '\'' | '/'))
        && !names_other_date(place)
}

/// The x span of the bytes `start..end` of the text of `l`: from the runs that hold them, a
/// place inside a run taken by its share of the run's characters.
fn span_of(l: &Line<'_>, start: usize, end: usize) -> (f32, f32) {
    let joined: String = l.runs.iter().map(|r| r.text.as_str()).collect();
    let lead = joined.len() - joined.trim_start().len();
    // The x of byte `b` of the joined text; a byte at the end of a run is the run's end when
    // `closing`, else the start of the next run.
    let x_at = |b: usize, closing: bool| {
        let b = b + lead;
        let mut at = 0;
        for (k, r) in l.runs.iter().enumerate() {
            let next = at + r.text.len();
            if b < next || (closing && b == next) || k + 1 == l.runs.len() {
                let cut = b.saturating_sub(at).min(r.text.len());
                let chars = r.text.chars().count().max(1) as f32;
                let before = r
                    .text
                    .get(..cut)
                    .map_or(chars, |t| t.chars().count() as f32);
                return r.x + r.w * (before / chars);
            }
            at = next;
        }
        l.x
    };
    (x_at(start, false), x_at(end, true))
}

/// The white space in points between the ink that ends at byte `end` of the text of `l` and
/// the ink that starts at byte `start`: the gap between their runs, 0 within one run.
fn ink_gap(l: &Line<'_>, end: usize, start: usize) -> f32 {
    let joined: String = l.runs.iter().map(|r| r.text.as_str()).collect();
    let lead = joined.len() - joined.trim_start().len();
    // The run that holds byte `b` of the line's text.
    let run_at = |b: usize| {
        let mut at = 0;
        l.runs.iter().find(|r| {
            at += r.text.len();
            b + lead < at
        })
    };
    match (end.checked_sub(1).and_then(run_at), run_at(start)) {
        (Some(a), Some(b)) if !std::ptr::eq(a, b) => (b.x - (a.x + a.w)).max(0.0),
        _ => 0.0,
    }
}

/// The dates the laid out text of page 1 gives as values of labels: `runs` are the runs the
/// layout lays out as text (tables taken out), read piece by piece (see [`pieces`]), `pairs`
/// the pairs that the layout read from drawn boxes and rows of fields, with `height` the
/// height of the page. A label alone in its piece marks the value beside it or right below it
/// (see [`value_of`]); a pair marks its
/// value when its table's top lies in the top third of the page, where the header fields of a
/// form stand (a pair table lower down may be one row of items under small column titles). A
/// table of columns gives no pairs: its column title is no label of one value.
pub(crate) fn laid_dates(runs: &[Run], pairs: &[Pair], height: Option<f32>) -> Dates {
    let mut out = Dates::default();
    let lines = pieces(runs);
    let lines = lines.as_slice();
    let add = |label: &str, value: &str, line: u32, out: &mut Dates| {
        let Some(d) = scan::dates(value, 4..=4)
            .into_iter()
            .next()
            .filter(|d| d.start == 0)
        else {
            return;
        };
        let written = (d.sep, d.parts);
        if let Some(label) = whole_label(label) {
            out.labelled.push(Seen {
                written,
                line,
                text: value[d.start..d.end].to_string(),
                label: Some(label),
                named: false,
            });
        } else if names_other_date(label) {
            out.other.push((line, written));
        }
    };
    for (k, l) in lines.iter().enumerate() {
        if scan::dates(&l.text, 4..=4).is_empty()
            && (whole_label(&l.text).is_some() || names_other_date(&l.text))
            && let Some(v) = value_of(lines, k)
        {
            let line = v.runs.first().map_or(0, |r| r.line);
            add(&l.text, &v.text, line, &mut out);
        }
    }
    let head = |top: f32| height.is_some_and(|h| h > 0.0 && top >= h * HEAD_SHARE);
    for p in pairs.iter().filter(|p| head(p.top)) {
        if let Some(line) = p.line {
            add(&p.label, p.value.trim(), line, &mut out);
        }
    }
    out
}

/// The document date of page 1 from what `dates` holds. A date a document-date label marks
/// wins; labelled dates that differ give no date, and a labelled date that cannot be read for
/// sure gives none either; both say so. Else the only date of the head is the document's,
/// when it stands there once without a label of its own (see [`Seen::named`]) and no label
/// names it as another date; several dates there (dates with a label of their own counted)
/// give no date and say so. A dotted date reads day first, a dashed one year first, a slashed
/// one only when one part is over 12; only a day the calendar has.
pub(crate) fn document_date(dates: &Dates) -> PageDate {
    if let Some(first) = dates.labelled.first() {
        let kinds = distinct(dates.labelled.iter().map(|s| s.written));
        if kinds > 1 {
            return PageDate::Unsure(format!(
                "{kinds} different dates stand after a date label on page 1; none is taken as \
                 the document date"
            ));
        }
        let label = first.label.as_deref().unwrap_or_default();
        return match iso(first.written) {
            Ok(date) => PageDate::Found(FoundDate {
                date,
                from: format!("date label \"{label}\" on page 1"),
            }),
            Err(why) => PageDate::Unsure(format!(
                "the date {} after the label \"{label}\" on page 1 {why}",
                first.text
            )),
        };
    }
    // Every date of the head counts, one with a label of its own too: it is a date the head
    // holds, only it can never be the document's. One value that a label names as another
    // date anywhere in the head is that other date wherever it stands; one value with a label
    // of its own at one place and plain at another is the plain date.
    let other = |s: &Seen| dates.other.contains(&(s.line, s.written));
    match distinct(dates.head.iter().map(|s| s.written)) {
        0 => PageDate::None,
        1 if dates.head.iter().any(other) => PageDate::None,
        1 => match dates.head.iter().find(|s| !s.named).map(|s| iso(s.written)) {
            Some(Ok(date)) => PageDate::Found(FoundDate {
                date,
                from: "only date in the top third of page 1".to_string(),
            }),
            _ => PageDate::None,
        },
        n => PageDate::Unsure(format!(
            "{n} dates stand in the head of page 1; none is marked as the document date"
        )),
    }
}

/// The number of different dates among `written`.
fn distinct(written: impl Iterator<Item = Written>) -> usize {
    let mut kinds: Vec<Written> = written.collect();
    kinds.sort_unstable();
    kinds.dedup();
    kinds.len()
}

/// Whether the label `text` of a date names another date: it holds one of
/// [`OTHER_DATE_WORDS`], accent folded, and is no whole [`DATE_LABELS`] entry.
fn names_other_date(text: &str) -> bool {
    let folded = fold(text);
    whole_label(text).is_none() && OTHER_DATE_WORDS.iter().any(|w| folded.contains(w))
}

/// The date label `before` ends with (white space and a colon after it aside), white space
/// inside it collapsed, when it stands as a whole: its words are the last words of `before`
/// (`Liefer-Datum` is one word, no label), and no word in letters stands right before them
/// (`Due date`, `Invoice date`: a word group is none of the list). A number or a sign before
/// it may stand (`Kundennr. 4711 Datum`, `Ort, Datum`).
fn label_before(before: &str) -> Option<String> {
    let before = before.trim_end();
    let before = before.strip_suffix(':').unwrap_or(before).trim_end();
    let words: Vec<&str> = before.split_whitespace().collect();
    DATE_LABELS.iter().find_map(|label| {
        let n = label.split(' ').count();
        let at = words.len().checked_sub(n)?;
        let tail = words[at..].join(" ");
        let alone = at == 0
            || words[at - 1]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphabetic());
        (alone && fold(&tail) == *label).then_some(tail)
    })
}

/// The date label a line is in whole (with a colon after it or not), white space collapsed.
fn whole_label(text: &str) -> Option<String> {
    let text = text.trim();
    let bare = text.strip_suffix(':').unwrap_or(text).trim_end();
    let label = label_before(bare)?;
    (label.split(' ').count() == bare.split_whitespace().count()).then_some(label)
}

/// A label's value stands beside it at most this many label sizes after its end.
const BESIDE_SIZES: f32 = 6.0;

/// The value of the label piece `all[k]`: among the next [`VALUE_LINES`] pieces, the nearest
/// that starts with a date and stands beside it on its baseline (to its right, at most
/// [`BESIDE_SIZES`] label sizes after its end) or right below it at its left edge (within its
/// size).
fn value_of<'a, 'r>(all: &'a [Line<'r>], k: usize) -> Option<&'a Line<'r>> {
    let label = &all[k];
    let end = label.runs.iter().map(|r| r.x + r.w).fold(label.x, f32::max);
    let starts_with_date = |l: &Line<'_>| {
        scan::dates(&l.text, 4..=4)
            .first()
            .is_some_and(|d| d.start == 0)
    };
    let after = all.iter().skip(k + 1).take(VALUE_LINES);
    let beside = after
        .clone()
        .filter(|v| {
            (v.y - label.y).abs() <= label.size / 2.0
                && v.x > label.x
                && v.x - end <= BESIDE_SIZES * label.size
        })
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
/// day); and only a day the calendar has. Else why not, in words.
fn iso((sep, [a, b, c]): Written) -> Result<String, &'static str> {
    let (year, month, day) = match sep {
        '.' => (c, b, a),
        '-' => (a, b, c),
        _ if a > 12 => (c, b, a),
        _ if b > 12 => (c, a, b),
        _ => return Err("can be read two ways"),
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if year >= 1 && (1..=days).contains(&day) {
        Ok(format!("{year:04}-{month:02}-{day:02}"))
    } else {
        Err("is no day of the calendar")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_stand_whole() {
        let label = |t: &str| label_before(t);
        assert_eq!(label("Datum ").as_deref(), Some("Datum"));
        assert_eq!(label("Erstellt   am").as_deref(), Some("Erstellt am"));
        assert_eq!(label("Ort, Datum:").as_deref(), Some("Datum"));
        assert_eq!(label("Liefer-Datum:"), None);
        assert_eq!(label("Due date"), None);
        assert_eq!(label("Kundennr. 4711 Datum").as_deref(), Some("Datum"));
        assert_eq!(label("Erstellt am").as_deref(), Some("Erstellt am"));
        assert_eq!(label("Eintrittsdatum"), None);
        assert_eq!(label("Update"), None);
        assert_eq!(label(""), None);
        assert_eq!(whole_label("DATE:").as_deref(), Some("DATE"));
        assert_eq!(whole_label("Datum der Zahlung"), None);
    }

    #[test]
    fn dates_read_for_sure_only() {
        let one = |t: &str| {
            let d = &scan::dates(t, 4..=4)[0];
            iso((d.sep, d.parts)).ok()
        };
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
    fn other_dates_by_their_labels() {
        assert!(names_other_date("Due date"));
        assert!(names_other_date("F\u{e4}llig am"));
        assert!(names_other_date("Geburtsdatum"));
        assert!(!names_other_date("Datum"));
        assert!(!names_other_date("Musterstadt, "));
        assert_eq!(fold("Table des Mati\u{e8}res"), "table des matieres");
        assert_eq!(fold("G\u{fc}ltig bis"), "gueltig bis");
    }

    #[test]
    fn a_place_with_a_comma_labels_nothing() {
        assert!(place_and_comma("Musterstadt, "));
        assert!(place_and_comma("Frankfurt am Main,"));
        assert!(place_and_comma("St. Gallen,"));
        assert!(place_and_comma("Baden-Baden, "));
        // The article after the comma, a postcode, a colon, a number, an other date's label.
        assert!(!place_and_comma("Musterstadt, den "));
        assert!(!place_and_comma("12345 Musterstadt, "));
        assert!(!place_and_comma("Datum: Musterstadt, "));
        assert!(!place_and_comma("Nr. 4711, "));
        assert!(!place_and_comma("Geburtsort, "));
        assert!(!place_and_comma(""));
        assert!(!place_and_comma(", "));
        assert!(is_text("Kunde seit "));
        assert!(!is_text(" - "));
        assert!(!is_text(""));
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
