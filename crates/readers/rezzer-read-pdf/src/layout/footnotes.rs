// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Footnotes of an untagged page: the legend at the bottom of the page, and the references to
//! it in the text above (a star after a word, a raised number after a label).
//!
//! Each footnote becomes a footnote section `fn:p<page>-<marker>`, so its label is unique in
//! the file, and each reference whose marker the legend holds becomes a footnote reference
//! to it. A reference whose marker the legend does not hold stays text, set apart from the
//! word before it.

use std::collections::{HashMap, HashSet};

use rezzer_model::{Block, Inline};

use super::{EDGE, Flow, GAP_RATIO, Line, Run, SIZE_SLACK, ink, lines};

/// A run of digits opens a legend line raised when it is this many times the size of the
/// text after it or smaller, or its baseline stands this many times that size higher.
const RAISED_SIZE: f32 = 0.8;
const RAISED_RISE: f32 = 0.25;

/// A reference number stands at least this many times the size of the word before it above
/// that word's baseline, or this share when it is also set smaller ([`REF_SMALLER`]).
const REF_RISE: f32 = 0.2;
const REF_RISE_SMALL: f32 = 0.1;
const REF_SMALLER: f32 = 0.9;

/// A reference number starts at most this many sizes of the word before it after that word.
const REF_GAP: f32 = 1.0;

/// Legend entries whose markers stand this many legend sizes apart or more are in other
/// columns.
const COLUMN_GAP: f32 = 2.0;

/// The superscript digits 0 to 9.
const SUPERSCRIPTS: [char; 10] = [
    '\u{2070}', '\u{b9}', '\u{b2}', '\u{b3}', '\u{2074}', '\u{2075}', '\u{2076}', '\u{2077}',
    '\u{2078}', '\u{2079}',
];

/// A note marker: one to three stars, a number with `)`, or a raised number (a superscript
/// digit, or digits set smaller or higher than the text beside them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum NoteMark {
    Stars(u8),
    Paren(u8),
    Raised(u8),
}

impl NoteMark {
    /// The part of a footnote label that names the marker.
    fn name(self) -> String {
        match self {
            NoteMark::Raised(n) | NoteMark::Paren(n) => n.to_string(),
            NoteMark::Stars(1) => "star".into(),
            NoteMark::Stars(c) => format!("star{c}"),
        }
    }
}

/// The legend of a page taken out of its flow.
#[derive(Default)]
pub(super) struct Legend {
    /// Each footnote, in reading order (column by column, top down): its label and runs.
    pub notes: Vec<(String, Vec<Run>)>,
    /// The label of each marker the legend holds.
    labels: HashMap<NoteMark, String>,
}

/// One footnote while the legend is read.
struct Entry {
    mark: NoteMark,
    /// The left edge of its marker, and the top and the last baseline of its lines.
    x: f32,
    top: f32,
    last: f32,
    h: f32,
    ids: Vec<usize>,
}

/// What [`find`] reads: the entries, and the raised reference runs above them with their
/// markers.
struct Found {
    entries: Vec<Entry>,
    raised: Vec<(usize, NoteMark)>,
}

/// Finds the legend of the untagged page `page`, marks the raised references to it in `flow`
/// and takes the legend's runs out of `flow`.
pub(super) fn take(page: u32, flow: &mut Flow) -> Legend {
    let Some(found) = find(&flow.runs) else {
        return Legend::default();
    };
    let mut entries = found.entries;
    // Reading order: column by column, top down.
    let size = entries.first().map_or(1.0, |e| e.h.max(1.0));
    let mut xs: Vec<f32> = entries.iter().map(|e| e.x).collect();
    xs.sort_by(f32::total_cmp);
    let mut columns: Vec<f32> = Vec::new();
    for x in xs {
        if columns.last().is_none_or(|c| x - c >= COLUMN_GAP * size) {
            columns.push(x);
        }
    }
    let column = |x: f32| columns.iter().rposition(|c| *c <= x + EDGE).unwrap_or(0);
    entries.sort_by(|a, b| column(a.x).cmp(&column(b.x)).then(b.top.total_cmp(&a.top)));
    let mut labels: HashMap<NoteMark, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut named = Vec::with_capacity(entries.len());
    for e in &entries {
        let base = format!("p{page}-{}", e.mark.name());
        let mut label = base.clone();
        let mut k = 2;
        while !used.insert(label.clone()) {
            label = format!("{base}-{k}");
            k += 1;
        }
        labels.entry(e.mark).or_insert_with(|| label.clone());
        named.push(label);
    }
    // A raised reference to a footnote of the legend becomes a reference run; one to no
    // footnote stays text, set apart from its word.
    for &(i, mark) in &found.raised {
        let r = &mut flow.runs[i];
        match labels.get(&mark) {
            Some(label) => {
                r.text.clone_from(label);
                r.note = true;
            }
            None if !r.text.starts_with(char::is_whitespace) => r.text.insert(0, ' '),
            None => {}
        }
    }
    let mut owner: Vec<Option<usize>> = vec![None; flow.runs.len()];
    for (n, e) in entries.iter().enumerate() {
        for &i in &e.ids {
            owner[i] = Some(n);
        }
    }
    let mut runs: Vec<Vec<Run>> = vec![Vec::new(); entries.len()];
    let taken = flow.take(|i, _| owner[i].is_some());
    for ((r, _, _), n) in taken.into_iter().zip(owner.into_iter().flatten()) {
        runs[n].push(r);
    }
    Legend {
        notes: named.into_iter().zip(runs).collect(),
        labels,
    }
}

/// The `found-by` note of a page with `n` footnotes written.
pub(super) fn note(page: u32, n: usize) -> Option<String> {
    let what = match n {
        0 => return None,
        1 => format!(
            "1 footnote at the bottom of page {page} is read from its legend: it is set in the \
             legend's size below all other text and opens with a note marker that also stands \
             after a word above, where it is linked"
        ),
        _ => format!(
            "{n} footnotes at the bottom of page {page} are read from their legend: they are \
             set in one size below all other text, each opens with a note marker, the first \
             marker also stands after a word above, and the markers above are linked to them"
        ),
    };
    Some(what)
}

/// Whether `run` holds one or two digits raised against `base`, the text after it.
fn raised_digits(run: &Run, base: &Run) -> Option<u8> {
    let raised = run.size < RAISED_SIZE * base.size || run.y > base.y + RAISED_RISE * base.size;
    digits(run).filter(|_| raised)
}

/// The value of a run of one or two digits.
fn digits(run: &Run) -> Option<u8> {
    let t = run.text.trim();
    ((1..=2).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit()))
        .then(|| t.parse().ok())
        .flatten()
}

/// Whether `run` is a reference number raised right after `word`: close after it, and its
/// baseline higher than the word's.
fn raised_ref(run: &Run, word: &Run) -> Option<u8> {
    let rise = run.y - word.y;
    let close = run.x - (word.x + word.w) <= REF_GAP * word.size;
    let raised = rise >= REF_RISE * word.size
        || (run.size <= REF_SMALLER * word.size && rise >= REF_RISE_SMALL * word.size);
    let after_word = word.text.trim_end().ends_with(char::is_alphanumeric);
    digits(run).filter(|_| close && raised && after_word)
}

/// The value of one or two superscript digits.
fn superscript_value(digits: &str) -> Option<u8> {
    let mut n: u8 = 0;
    let mut count = 0;
    for c in digits.chars() {
        let d = SUPERSCRIPTS.iter().position(|s| *s == c)?;
        n = n * 10 + d as u8;
        count += 1;
    }
    (1..=2).contains(&count).then_some(n)
}

/// The note marker a word ends with, after an alphanumeric character (`Betrag*`,
/// `Betrag1)`, `Betrag` and a superscript digit), and its length in bytes.
fn word_mark(word: &str) -> Option<(NoteMark, usize)> {
    let w = word.trim_end_matches([':', ',', '.', ';']);
    let stars = w.len() - w.trim_end_matches('*').len();
    if stars > 0 {
        let before = w[..w.len() - stars].chars().next_back()?;
        return (stars <= 3 && before.is_alphanumeric())
            .then_some((NoteMark::Stars(stars as u8), stars));
    }
    if let Some(open) = w.strip_suffix(')') {
        let n = open.len() - open.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let before = open[..open.len() - n].chars().next_back()?;
        let value: u8 = open[open.len() - n..].parse().ok()?;
        return ((1..=2).contains(&n) && before.is_alphabetic())
            .then_some((NoteMark::Paren(value), n + 1));
    }
    let sup = w.len() - w.trim_end_matches(|c| SUPERSCRIPTS.contains(&c)).len();
    let before = w[..w.len() - sup].chars().next_back()?;
    let value = superscript_value(&w[w.len() - sup..])?;
    before
        .is_alphanumeric()
        .then_some((NoteMark::Raised(value), sup))
}

/// The marker a piece of a legend line opens with: stars, a number with `)` or a superscript
/// number before its text, a run of digits raised against the run after it, or plain digits
/// and a space.
fn opening_mark(runs: &[Run]) -> Option<NoteMark> {
    let mut inked = runs.iter().filter(|r| ink(&r.text) > 0);
    let first = inked.next()?;
    if let Some(next) = inked.next()
        && let Some(n) = raised_digits(first, next)
    {
        return Some(NoteMark::Raised(n));
    }
    // The marker and the start of the text after it stand in the first two runs with ink.
    let head = runs
        .iter()
        .enumerate()
        .filter(|(_, r)| ink(&r.text) > 0)
        .nth(1)
        .map_or(runs.len(), |(i, _)| i + 1);
    let text: String = runs[..head].iter().map(|r| r.text.as_str()).collect();
    let text = text.trim_start();
    let stars = text.len() - text.trim_start_matches('*').len();
    if stars > 0 {
        let rest = &text[stars..];
        return (stars <= 3
            && rest
                .trim()
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric))
        .then_some(NoteMark::Stars(stars as u8));
    }
    let n = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if (1..=2).contains(&n) {
        let value: u8 = text[..n].parse().ok()?;
        let rest = &text[n..];
        if let Some(after) = rest.strip_prefix(')') {
            return after
                .starts_with(char::is_whitespace)
                .then_some(NoteMark::Paren(value));
        }
        return rest
            .starts_with(char::is_whitespace)
            .then_some(NoteMark::Raised(value));
    }
    let sup: String = text
        .chars()
        .take_while(|c| SUPERSCRIPTS.contains(c))
        .collect();
    let value = superscript_value(&sup)?;
    text[sup.len()..]
        .trim_start()
        .chars()
        .next()
        .is_some_and(char::is_alphanumeric)
        .then_some(NoteMark::Raised(value))
}

/// The references of a line: markers that end a word, and numbers raised right after a
/// word (`PV-Beitrag` and a raised `6`) with the index of their run, from `start`.
fn references(l: &Line<'_>, start: usize, marks: &mut HashSet<NoteMark>) -> Vec<(usize, u8)> {
    marks.extend(l.text.split_whitespace().filter_map(word_mark).map(|m| m.0));
    let mut raised = Vec::new();
    let mut word: Option<&Run> = None;
    for (i, r) in l.runs.iter().enumerate() {
        if ink(&r.text) == 0 {
            continue;
        }
        if let Some(w) = word
            && let Some(n) = raised_ref(r, w)
        {
            marks.insert(NoteMark::Raised(n));
            raised.push((start + i, n));
        }
        word = Some(r);
    }
    raised
}

/// The pieces of a line: its runs cut where a piece gap parts two runs with ink.
fn pieces<'a>(l: &Line<'a>) -> Vec<(usize, &'a [Run])> {
    let mut cuts = vec![0];
    let mut end: Option<f32> = None;
    for (i, r) in l.runs.iter().enumerate() {
        if ink(&r.text) == 0 {
            continue;
        }
        if let Some(e) = end
            && r.x - e > crate::order::PIECE_GAP * r.size
        {
            cuts.push(i);
        }
        end = Some(r.x + r.w);
    }
    cuts.push(l.runs.len());
    cuts.windows(2)
        .map(|w| (w[0], &l.runs[w[0]..w[1]]))
        .filter(|(_, p)| p.iter().any(|r| ink(&r.text) > 0))
        .collect()
}

/// The legend of a page. It opens at the first line that is the lowest line so far, with
/// nothing below it, in a size no larger than every line above, and that opens with a note
/// marker which also stands as a reference above. Its lines are the lines in that size from
/// there on; lines in other sizes stay text. Each piece of a legend line (cut at piece gaps,
/// so columns side by side stay apart) opens a footnote with a marker or goes on the
/// footnote of its column on the line above. Pieces that go on nothing stay text; when they
/// outnumber the footnotes there is no legend.
fn find(runs: &[Run]) -> Option<Found> {
    let all = lines(runs);
    let n = all.len();
    let inked = |l: &Line<'_>| !l.text.is_empty();
    let mut starts = Vec::with_capacity(n);
    let mut at = 0;
    for l in &all {
        starts.push(at);
        at += l.runs.len();
    }
    // The highest line after each line.
    let mut highest_after = vec![f32::MIN; n + 1];
    for k in (0..n).rev() {
        let y = if inked(&all[k]) { all[k].y } else { f32::MIN };
        highest_after[k] = highest_after[k + 1].max(y);
    }
    let mut marks: HashSet<NoteMark> = HashSet::new();
    let mut raised: Vec<(usize, NoteMark)> = Vec::new();
    let (mut lowest, mut smallest) = (f32::MAX, f32::MAX);
    let mut first = None;
    for k in 0..n {
        let l = &all[k];
        if !inked(l) {
            continue;
        }
        let opens = pieces(l)
            .first()
            .and_then(|p| opening_mark(p.1))
            .is_some_and(|m| marks.contains(&m));
        if k > 0
            && opens
            && l.size <= smallest + SIZE_SLACK
            && l.y <= lowest + EDGE
            && highest_after[k + 1] <= l.y + EDGE
        {
            first = Some(k);
            break;
        }
        raised.extend(
            references(l, starts[k], &mut marks)
                .into_iter()
                .map(|(i, v)| (i, NoteMark::Raised(v))),
        );
        lowest = lowest.min(l.y);
        smallest = smallest.min(l.size);
    }
    let first = first?;
    let size = all[first].size;
    let mut entries: Vec<Entry> = Vec::new();
    let mut loose = 0usize;
    for k in first..n {
        let l = &all[k];
        if !inked(l) || (l.size - size).abs() > SIZE_SLACK {
            continue;
        }
        for (from, piece) in pieces(l) {
            let Some(x) = piece.iter().find(|r| ink(&r.text) > 0).map(|r| r.x) else {
                continue;
            };
            let ids = (starts[k] + from)..(starts[k] + from + piece.len());
            if let Some(mark) = opening_mark(piece) {
                entries.push(Entry {
                    mark,
                    x,
                    top: l.y,
                    last: l.y,
                    h: l.h,
                    ids: ids.collect(),
                });
                continue;
            }
            // The footnote of its column whose last line stands right above.
            let column = entries
                .iter_mut()
                .filter(|e| e.x <= x + EDGE && e.last > l.y)
                .max_by(|a, b| a.x.total_cmp(&b.x).then(b.last.total_cmp(&a.last)));
            match column {
                Some(e) if e.last - l.y <= GAP_RATIO * e.h.max(l.h).max(1.0) => {
                    e.ids.extend(ids);
                    e.last = l.y;
                }
                _ => loose += 1,
            }
        }
    }
    if loose > entries.len() {
        return None;
    }
    Some(Found { entries, raised })
}

/// Turns the markers that end a word in the text of `blocks` into references to the
/// footnotes of `legend`: `Betrag*` becomes `Betrag` and a reference.
pub(super) fn link_words(blocks: &mut [Block], legend: &Legend) {
    if legend.labels.is_empty() {
        return;
    }
    for b in blocks {
        link_block(b, legend);
    }
}

fn link_block(block: &mut Block, legend: &Legend) {
    match block {
        Block::Heading { inlines, .. } | Block::Paragraph { inlines, .. } => {
            link_inlines(inlines, legend);
        }
        Block::List { items, .. } => items
            .iter_mut()
            .flatten()
            .for_each(|b| link_block(b, legend)),
        Block::Table { rows, .. } => rows
            .iter_mut()
            .flatten()
            .for_each(|c| link_inlines(&mut c.inlines, legend)),
        Block::Section { blocks, .. }
        | Block::Quote { blocks, .. }
        | Block::Callout { blocks, .. } => blocks.iter_mut().for_each(|b| link_block(b, legend)),
        _ => {}
    }
}

/// [`link_words`] for one list of inlines. Text inside links stays as it is.
fn link_inlines(inlines: &mut Vec<Inline>, legend: &Legend) {
    if !inlines.iter().any(|i| matches!(i, Inline::Text { .. })) {
        return;
    }
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines.drain(..) {
        let Inline::Text {
            text,
            emphasis,
            strong,
            strike,
        } = inline
        else {
            out.push(inline);
            continue;
        };
        let piece = |t: &str| Inline::Text {
            text: t.to_string(),
            emphasis,
            strong,
            strike,
        };
        let mut from = 0;
        for (start, word) in words(&text) {
            let Some((mark, len)) = word_mark(word) else {
                continue;
            };
            let Some(label) = legend.labels.get(&mark) else {
                continue;
            };
            let kept = word.trim_end_matches([':', ',', '.', ';']).len();
            let end = start + kept;
            if end - len > from {
                out.push(piece(&text[from..end - len]));
            }
            out.push(Inline::FootnoteRef { id: label.clone() });
            from = end;
        }
        if from == 0 {
            out.push(piece(&text));
        } else if from < text.len() {
            out.push(piece(&text[from..]));
        }
    }
    *inlines = out;
}

/// The words of `text` with their byte offsets.
fn words(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                out.push((s, &text[s..i]));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &text[s..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::Gaps;
    use super::*;

    fn run(text: &str, line: u32, x: f32, y: f32, size: f32) -> Run {
        Run {
            page: 1,
            text: text.into(),
            x,
            y,
            w: text.len() as f32 * size * 0.5,
            h: size,
            size,
            bold: false,
            italic: false,
            mcid: None,
            uri: None,
            line,
            off_axis: false,
            note: false,
            gaps: Gaps::default(),
        }
    }

    #[test]
    fn note_markers() {
        assert_eq!(word_mark("Betrag*"), Some((NoteMark::Stars(1), 1)));
        assert_eq!(word_mark("Betrag**:"), Some((NoteMark::Stars(2), 2)));
        assert_eq!(word_mark("Betrag1)"), Some((NoteMark::Paren(1), 2)));
        assert_eq!(word_mark("Beitrag\u{b2}"), Some((NoteMark::Raised(2), 2)));
        assert_eq!(word_mark("*"), None);
        assert_eq!(word_mark("12)"), None);
        assert_eq!(word_mark("Betrag"), None);
        let opens = |text: &str| opening_mark(&[run(text, 0, 40.0, 60.0, 6.0)]);
        assert_eq!(opens("* steuerfrei"), Some(NoteMark::Stars(1)));
        assert_eq!(opens("*** ohne"), Some(NoteMark::Stars(3)));
        assert_eq!(opens("1) laut Vertrag"), Some(NoteMark::Paren(1)));
        assert_eq!(opens("\u{b3} anteilig"), Some(NoteMark::Raised(3)));
        assert_eq!(opens("2 Std = Stunden"), Some(NoteMark::Raised(2)));
        assert_eq!(opens("2025 war"), None);
        assert_eq!(opens("*"), None);
        // A digit run raised against the text after it.
        let raised = [
            run("4", 0, 40.0, 62.0, 4.0),
            run(" anteilig", 0, 44.0, 60.0, 6.0),
        ];
        assert_eq!(opening_mark(&raised), Some(NoteMark::Raised(4)));
    }

    #[test]
    fn raised_references() {
        let word = run("Einheit", 0, 40.0, 720.0, 9.0);
        // Nearly the size of the word and raised, as header cells set them.
        let close = run("2", 0, 72.0, 722.5, 8.0);
        assert_eq!(raised_ref(&close, &word), Some(2));
        // On the baseline, or far away: no reference.
        assert_eq!(raised_ref(&run("2", 0, 72.0, 720.0, 8.0), &word), None);
        assert_eq!(raised_ref(&run("2", 0, 140.0, 724.0, 6.0), &word), None);
    }

    /// Runs of a page with references raised after `Einheit` and `Beitrag`, a line of body
    /// text, and the legend pieces `legend` (text, x, line, y) in 6 pt at the bottom.
    fn legend_page(legend: &[(&str, f32, u32, f32)]) -> Vec<Run> {
        let mut runs = vec![
            run("Einheit", 0, 40.0, 720.0, 9.0),
            run(" 2", 0, 72.0, 724.0, 6.0),
            run(" Beitrag", 0, 200.0, 720.0, 9.0),
            run(" 3", 0, 236.0, 724.0, 6.0),
            run("die Werte werden monatlich gemeldet", 1, 40.0, 690.0, 9.0),
        ];
        for &(text, x, line, y) in legend {
            runs.push(run(text, line, x, y, 6.0));
        }
        runs
    }

    fn legend_texts(runs: &[Run]) -> Vec<String> {
        find(runs)
            .map(|f| {
                f.entries
                    .iter()
                    .map(|e| e.ids.iter().map(|&i| runs[i].text.as_str()).collect())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_legend_line_of_two_columns_is_cut() {
        let runs = legend_page(&[
            ("2 Std = Stunden", 40.0, 2, 70.0),
            (" 3 Gegebenenfalls anteilig", 300.0, 2, 70.0),
        ]);
        assert_eq!(
            legend_texts(&runs),
            ["2 Std = Stunden", " 3 Gegebenenfalls anteilig"]
        );
    }

    #[test]
    fn a_marker_without_its_reference_opens_its_own_footnote() {
        let runs = legend_page(&[
            ("2 Std = Stunden", 40.0, 2, 70.0),
            ("5 Muster ohne Verweis", 40.0, 3, 62.0),
        ]);
        assert_eq!(
            legend_texts(&runs),
            ["2 Std = Stunden", "5 Muster ohne Verweis"]
        );
    }

    #[test]
    fn wrapped_entries_of_columns_read_line_by_line() {
        // Two columns joined line by line: the second line goes on both entries.
        let runs = legend_page(&[
            ("2 Std = Stunden,", 40.0, 2, 70.0),
            (" 3 Gegebenenfalls", 300.0, 2, 70.0),
            ("Min = Minuten", 46.0, 3, 63.0),
            (" anteilig", 306.0, 3, 63.0),
        ]);
        assert_eq!(
            legend_texts(&runs),
            [
                "2 Std = Stunden,Min = Minuten",
                " 3 Gegebenenfalls anteilig"
            ]
        );
    }

    #[test]
    fn markers_in_words_become_references() {
        let mut legend = Legend::default();
        legend.labels.insert(NoteMark::Stars(1), "p1-star".into());
        let mut inlines = vec![Inline::plain("Betrag* 1.234,56 und Summe**")];
        link_inlines(&mut inlines, &legend);
        assert_eq!(
            inlines,
            [
                Inline::plain("Betrag"),
                Inline::FootnoteRef {
                    id: "p1-star".into()
                },
                Inline::plain(" 1.234,56 und Summe**"),
            ]
        );
    }
}
