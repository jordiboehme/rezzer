// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the head of page 1 says about the document: its title and its date. Both are found
//! by general rules. A date is chosen only when a whole date label marks it as the document
//! date: before it on its line, over it, or in the cell left of it in a table, and the label
//! governs that one date (a label over a list of dates marks none). A date nothing marks is
//! never chosen, also when it is the only date of the head: the loss says that it stands
//! there.

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
    /// No date, and nothing to say about it.
    None,
    Found(FoundDate),
    /// No date, with the words of an Info `Uncertain` loss: dates in the head that nothing
    /// marks, marked dates that differ, or a marked date that cannot be read for sure.
    Unsure(String),
}

/// A date as written: its separator and its parts.
type Written = (char, [u32; 3]);

/// A table the layout made on page 1, its rows as the text of their cells.
#[derive(Debug, Clone)]
pub(crate) struct Table {
    /// The field reader made it from drawn boxes or rows of fields: a label and its value in
    /// each row, also where column titles stand over one row of items.
    pub pairs: bool,
    /// The top of the table, in the frame of the runs.
    pub top: f32,
    pub rows: Vec<Vec<String>>,
}

/// A date of page 1 that a date label marks as the document date.
#[derive(Debug, Clone)]
struct Marked {
    written: Written,
    /// The date as it stands in the text.
    text: String,
    /// The label as written, white space collapsed.
    label: String,
}

/// What page 1 holds of dates, gathered from its lines before the layout and from its laid
/// out text and pairs after it, decided once by [`document_date`].
#[derive(Debug, Default)]
pub(crate) struct Dates {
    /// Dates a date label marks as the document date.
    marked: Vec<Marked>,
    /// Every full date in the top third of the page, marked or not.
    head: Vec<Written>,
}

impl Dates {
    /// Adds what `more` found.
    pub(crate) fn merge(&mut self, more: Dates) {
        self.marked.extend(more.marked);
        self.head.extend(more.head);
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
/// piece by piece (see [`pieces`]), with `height` the height of the page. Only full dates
/// count, with a year of four digits. A date is marked when the text of its piece before it
/// is a whole [`DATE_LABELS`] entry and nothing else (see [`whole_label`]: `Datum 01.02.2026`,
/// `DATE: 2026-02-01`; not `Eintritt: Datum 01.01.2020` or `Kundennr. 4711 Datum 01.02.2026`),
/// at most [`BESIDE_SIZES`] sizes away, the piece holds no other date (`Datum 01.01.2026 bis
/// 31.01.2026` marks nothing), and no date stands under the date in the next [`VALUE_LINES`]
/// pieces (see [`date_under`]: `Datum 15.01.2026` over `16.01.2026` heads a list). Every date
/// in the top third of the page is a date of the head, marked or not.
pub(crate) fn page_dates(runs: &[Run], height: Option<f32>) -> Dates {
    let mut out = Dates::default();
    let in_head = |l: &Line<'_>| height.is_some_and(|h| h > 0.0 && l.y + l.h >= h * HEAD_SHARE);
    let all = pieces(runs);
    for (k, l) in all.iter().enumerate() {
        let found = scan::dates(&l.text, 4..=4);
        for d in &found {
            let written = (d.sep, d.parts);
            if in_head(l) {
                out.head.push(written);
            }
            let before = &l.text[..d.start];
            let label = whole_label(before).filter(|_| {
                let label_end = before.trim_end().len();
                let (x0, x1) = span_of(l, d.start, d.end);
                found.len() == 1
                    && ink_gap(l, label_end, d.start) <= BESIDE_SIZES * l.size
                    && !date_under(all.iter().skip(k + 1).take(VALUE_LINES), l, x0, x1)
            });
            if let Some(label) = label {
                out.marked.push(Marked {
                    written,
                    text: l.text[d.start..d.end].to_string(),
                    label,
                });
            }
        }
    }
    out
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

/// The right end of the ink of `l`.
fn right_of(l: &Line<'_>) -> f32 {
    l.runs.iter().map(|r| r.x + r.w).fold(l.x, f32::max)
}

/// Whether `l` starts with a full date.
fn starts_with_date(l: &Line<'_>) -> bool {
    scan::dates(&l.text, 4..=4)
        .first()
        .is_some_and(|d| d.start == 0)
}

/// Whether one of `pieces`, below `over` on the page, starts with a date and stands in the
/// column `x0..x1`: its x span, widened by [`BESIDE_SIZES`] sizes on each side (an indented
/// entry of a list), overlaps the column. Such a date makes the date in that column above it
/// one entry of a list.
fn date_under<'a>(
    pieces: impl Iterator<Item = &'a Line<'a>>,
    over: &Line<'_>,
    x0: f32,
    x1: f32,
) -> bool {
    pieces.into_iter().any(|w| {
        let slack = BESIDE_SIZES * w.size;
        w.y < over.y
            && !std::ptr::eq(w, over)
            && w.x - slack <= x1
            && right_of(w) + slack >= x0
            && starts_with_date(w)
    })
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

/// The dates the laid out text of page 1 gives as values of date labels: `runs` are the runs
/// the layout lays out as text (tables taken out), read piece by piece (see [`pieces`]), and
/// `tables` every table the layout made on the page, with `height` the height of the page. A
/// piece that is a whole [`DATE_LABELS`] entry marks the value beside it or right below it
/// (see [`value_of`]). In a table of exactly two columns (labels and values) whose top lies in
/// the top half of the page, a cell that is a whole entry marks the cell right of it when that
/// cell is a full date and nothing else (`Datum: | 26.10.2026`), it is the only such label
/// cell with a date in the table, and no row has a date under an empty label cell (a list
/// goes on in the value column). A table of more columns marks nothing: a row of items may
/// end in `Datum | 15.01.2026` or list dates after a title (`Datum | 15.01.2026 |
/// 16.01.2026`, `Datum | 15.01.2026 | Stunden | 8`), and a column title `Datum` has another
/// title beside it. A table of label and value pairs
/// that the field reader made counts only when its top lies in the top third of the page,
/// where the header fields of a form stand: lower down it may be one row of items under small
/// column titles, each title read as the label of the value below it.
pub(crate) fn laid_dates(runs: &[Run], tables: &[Table], height: Option<f32>) -> Dates {
    let mut out = Dates::default();
    let lines = pieces(runs);
    let add = |label: String, value: &str, out: &mut Dates| {
        let Some(d) = scan::dates(value, 4..=4)
            .into_iter()
            .next()
            .filter(|d| d.start == 0)
        else {
            return;
        };
        out.marked.push(Marked {
            written: (d.sep, d.parts),
            text: value[d.start..d.end].to_string(),
            label,
        });
    };
    for (k, l) in lines.iter().enumerate() {
        if let Some(label) = whole_label(&l.text)
            && let Some(v) = value_of(&lines, k, height)
        {
            add(label, &v.text, &mut out);
        }
    }
    let above = |top: f32, share: f32| height.is_some_and(|h| h > 0.0 && top >= h * share);
    let is_date = |cell: &str| {
        let cell = cell.trim();
        scan::dates(cell, 4..=4)
            .first()
            .is_some_and(|d| d.start == 0 && d.end == cell.len())
    };
    for t in tables
        .iter()
        .filter(|t| above(t.top, if t.pairs { HEAD_SHARE } else { HALF }))
        .filter(|t| t.rows.iter().all(|row| row.len() == 2))
    {
        // A date under an empty label cell continues a list in the value column.
        let continued = t
            .rows
            .iter()
            .any(|row| row[0].trim().is_empty() && is_date(&row[1]));
        let mut labelled = t
            .rows
            .iter()
            .filter(|row| is_date(&row[1]))
            .filter_map(|row| whole_label(&row[0]).map(|label| (label, &row[1])));
        if let Some((label, value)) = labelled.next()
            && labelled.next().is_none()
            && !continued
        {
            add(label, value.trim(), &mut out);
        }
    }
    out
}

/// A table whose top lies in the upper half of the page may hold the document date.
const HALF: f32 = 0.5;

/// The document date of page 1 from what `dates` holds: a date that a date label marks.
/// Marked dates that differ give no date, and a marked date that cannot be read for sure
/// gives none either; both say so. A date nothing marks is never chosen: when the
/// top third of the page holds dates, the loss says how many stand there. A dotted date
/// reads day first, a dashed one year first, a slashed one only when one part is over 12;
/// only a day the calendar has.
pub(crate) fn document_date(dates: &Dates) -> PageDate {
    if let Some(first) = dates.marked.first() {
        let kinds = distinct(dates.marked.iter().map(|s| s.written));
        if kinds > 1 {
            return PageDate::Unsure(format!(
                "{kinds} different dates stand after a date label on page 1; none is taken as \
                 the document date"
            ));
        }
        return match iso(first.written) {
            Ok(date) => PageDate::Found(FoundDate {
                date,
                from: format!("date label \"{}\" on page 1", first.label),
            }),
            Err(why) => PageDate::Unsure(format!(
                "the date {} after the label \"{}\" on page 1 {why}",
                first.text, first.label
            )),
        };
    }
    match distinct(dates.head.iter().copied()) {
        0 => PageDate::None,
        1 => PageDate::Unsure(
            "1 date stands in the head of page 1; it is not marked as the document date".into(),
        ),
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

/// The date label `text` is in whole (white space around it and a colon after it aside), white
/// space inside it collapsed: a [`DATE_LABELS`] entry, case folded (`DATE:`, `Erstellt   am`).
/// A word more is no label (`Due date`, `Eintrittsdatum`, `Ort, Datum`, `Liefer-Datum`).
fn whole_label(text: &str) -> Option<String> {
    let text = text.trim();
    let bare = text.strip_suffix(':').unwrap_or(text).trim_end();
    let words: Vec<&str> = bare.split_whitespace().collect();
    let label = words.join(" ");
    DATE_LABELS
        .contains(&fold(&label).as_str())
        .then_some(label)
}

/// A label's value stands beside it at most this many label sizes after its end.
const BESIDE_SIZES: f32 = 6.0;

/// The value of the label piece `all[k]`, with `height` the height of the page: among the
/// next [`VALUE_LINES`] pieces, the nearest that starts with a date and stands beside it on its
/// baseline (to its right, at most [`BESIDE_SIZES`] label sizes after its end), else the one
/// right below it at its left edge (within the label's size) that is a date alone on its line
/// (see [`date_alone`]). The label governs one date only: no other date may stand under the
/// value in the next [`VALUE_LINES`] pieces (a value beside the label), or anywhere below the
/// label in the label's or the value's column (see [`date_under`]) down to the middle of the
/// page or the next [`VALUE_LINES`] pieces, whichever reaches lower (a value below the label).
/// A row of items under column titles has more cells on the line, and a column of dates under
/// a title is a list, however spaced or indented: neither is the value of one label.
fn value_of<'a, 'r>(all: &'a [Line<'r>], k: usize, height: Option<f32>) -> Option<&'a Line<'r>> {
    let label = &all[k];
    let end = right_of(label);
    let after = || all.iter().skip(k + 1).take(VALUE_LINES);
    let at = |v: &Line<'_>| all.iter().position(|p| std::ptr::eq(p, v)).unwrap_or(k);
    let beside = after()
        .filter(|v| {
            (v.y - label.y).abs() <= label.size / 2.0
                && v.x > label.x
                && v.x - end <= BESIDE_SIZES * label.size
        })
        .filter(|v| starts_with_date(v))
        .min_by(|a, b| a.x.total_cmp(&b.x))
        .filter(|v| {
            let under = all.iter().skip(at(v) + 1).take(VALUE_LINES);
            !date_under(under, v, v.x, right_of(v))
        });
    beside.or_else(|| {
        let half = height.map_or(0.0, |h| h * HALF);
        after()
            .filter(|v| right_below(label, v) && (v.x - label.x).abs() <= label.size)
            .filter(|v| date_alone(all, v))
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .filter(|v| {
                let reach = after().next_back().map_or(label.y, |l| l.y).min(half);
                let region = all.iter().filter(|w| w.y >= reach && !std::ptr::eq(*w, *v));
                let (x0, x1) = (label.x.min(v.x), end.max(right_of(v)));
                !date_under(region, label, x0, x1)
            })
    })
}

/// Whether the piece `v` among the pieces `all` is a date and nothing else, with no other
/// piece on its baseline (within half its size).
fn date_alone(all: &[Line<'_>], v: &Line<'_>) -> bool {
    let text = v.text.trim();
    let is_date = scan::dates(text, 4..=4)
        .first()
        .is_some_and(|d| d.start == 0 && d.end == text.len());
    is_date
        && !all
            .iter()
            .any(|p| !std::ptr::eq(p, v) && (p.y - v.y).abs() <= v.size / 2.0)
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
        let label = |t: &str| whole_label(t);
        assert_eq!(label("Datum ").as_deref(), Some("Datum"));
        assert_eq!(label("DATE:").as_deref(), Some("DATE"));
        assert_eq!(label("Erstellt   am").as_deref(), Some("Erstellt am"));
        assert_eq!(
            label(" Ausgestellt am: ").as_deref(),
            Some("Ausgestellt am")
        );
        assert_eq!(label("Ort, Datum:"), None);
        assert_eq!(label("Kundennr. 4711 Datum"), None);
        assert_eq!(label("Eintritt: Datum"), None);
        assert_eq!(label("Liefer-Datum:"), None);
        assert_eq!(label("Due date"), None);
        assert_eq!(label("Eintrittsdatum"), None);
        assert_eq!(label("Update"), None);
        assert_eq!(label("Datum der Zahlung"), None);
        assert_eq!(label(""), None);
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
    fn accents_fold() {
        assert_eq!(fold("Table des Mati\u{e8}res"), "table des matieres");
        assert_eq!(fold("G\u{fc}ltig bis"), "gueltig bis");
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
