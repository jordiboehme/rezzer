// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The footnote legend of a page, read by position: its lines at the bottom of the page, each
//! footnote opening with a note marker, and the references to them in the text above (a star
//! after a word, a raised number after a label).

use std::collections::HashSet;

#[cfg(test)]
use crate::line::text_lines;
use crate::line::{EDGE, GAP_RATIO, LineRun, PIECE_GAP, SIZE_SLACK, TextLine, ink};

/// A run of digits opens a legend line raised when it is this many times the size of the
/// text after it or smaller, or its baseline stands this many times that size higher.
const RAISED_SIZE: f32 = 0.8;
const RAISED_RISE: f32 = 0.25;

/// A reference number stands at least this many times the size of the word before it above
/// that word's baseline, or this share when it is also set smaller ([`REF_SMALLER`]).
const REF_RISE: f32 = 0.2;
const REF_RISE_SMALL: f32 = 0.1;
const REF_SMALLER: f32 = 0.9;

/// A reference number starts at most this many sizes of the word before it before the end of
/// that word's glyphs: it stands after the word, not over it.
const REF_LEAD: f32 = 0.3;

/// A reference number starts at most this many sizes of the word before it after that word.
const REF_GAP: f32 = 1.0;

/// The superscript digits 0 to 9.
const SUPERSCRIPTS: [char; 10] = [
    '\u{2070}', '\u{b9}', '\u{b2}', '\u{b3}', '\u{2074}', '\u{2075}', '\u{2076}', '\u{2077}',
    '\u{2078}', '\u{2079}',
];

/// A note marker: one to three stars, a number with `)`, or a raised number (a superscript
/// digit, or digits set smaller or higher than the text beside them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NoteMark {
    Stars(u8),
    Paren(u8),
    Raised(u8),
}

impl NoteMark {
    /// The part of a footnote label that names the marker.
    pub fn name(self) -> String {
        match self {
            NoteMark::Raised(n) | NoteMark::Paren(n) => n.to_string(),
            NoteMark::Stars(1) => "star".into(),
            NoteMark::Stars(c) => format!("star{c}"),
        }
    }
}

/// The footnote legend of a page and the raised references above it. Run numbers are indices
/// into the page's [`crate::PageIn::texts`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Legend {
    /// The line numbers of the legend's lines, in order.
    pub lines: Vec<u32>,
    /// Each footnote, as the page lists its lines.
    pub entries: Vec<LegendEntry>,
    /// The raised reference numbers above the legend.
    pub raised: Vec<RaisedRef>,
}

/// One footnote of a legend.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendEntry {
    pub mark: NoteMark,
    /// The bytes of its first piece's trimmed text the marker takes.
    pub cut: usize,
    /// The left edge of its marker, the baseline of its first and of its last line, and the
    /// height of its first line.
    pub x: f32,
    pub top: f32,
    pub last: f32,
    pub h: f32,
    /// Its runs.
    pub runs: Vec<u32>,
}

/// A reference number raised after a word: its run, its marker and where it goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RaisedRef {
    pub run: u32,
    pub mark: NoteMark,
    pub place: RefPlace,
}

/// Where a raised reference number goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RefPlace {
    /// After the run before it on its line.
    End,
    /// Into a white space of the run `word` of its line, after the word before it.
    Inside { word: u32, split: Split },
    /// After the run `word`, the last of the line above: the number opens its own line
    /// though it stands raised at the end of that word.
    Above { word: u32 },
}

/// A white space between two words of a run: where the next word starts in the text, and
/// the end of the ink before it and the start of the ink after it. From the glyphs (`gap`
/// names the run's gap) or, for a run without them, by the share of characters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Split {
    pub byte: usize,
    pub from: f32,
    pub to: f32,
    pub gap: Option<usize>,
}

/// The white spaces between the words of `run`, in order.
fn splits<R: LineRun>(run: &R) -> Vec<Split> {
    // Where each word after the first starts, and how many words stand before it.
    let mut starts = Vec::new();
    let mut words = 0;
    let mut in_word = false;
    for (i, c) in run.text().char_indices() {
        if c.is_whitespace() {
            in_word = false;
        } else if !in_word {
            if words > 0 {
                starts.push((i, words));
            }
            words += 1;
            in_word = true;
        }
    }
    let mut gaps = run.gaps().peekable();
    if gaps.peek().is_some() {
        return gaps
            .enumerate()
            .filter_map(|(k, g)| {
                let &(byte, _) = starts.iter().find(|s| s.1 == g.words as usize)?;
                Some(Split {
                    byte,
                    from: g.from,
                    to: g.to,
                    gap: Some(k),
                })
            })
            .collect();
    }
    // By the share of characters of the text from its first ink on.
    let lead = run.text().len() - run.text().trim_start().len();
    let text = run.text().trim();
    let total = text.chars().count().max(1) as f32;
    let at = |byte: usize| {
        let chars = run.text()[lead..byte.max(lead)].chars().count() as f32;
        run.x() + run.w() * chars / total
    };
    starts
        .iter()
        .map(|&(byte, _)| {
            let end = run.text()[..byte].trim_end().len();
            Split {
                byte,
                from: at(end),
                to: at(byte),
                gap: None,
            }
        })
        .collect()
}

/// Whether `run` holds one or two digits raised against `base`, the text after it.
fn raised_digits<R: LineRun>(run: &R, base: &R) -> Option<u8> {
    let raised =
        run.size() < RAISED_SIZE * base.size() || run.y() > base.y() + RAISED_RISE * base.size();
    digits(run).filter(|_| raised)
}

/// The value of a run of one or two digits.
fn digits<R: LineRun>(run: &R) -> Option<u8> {
    let t = run.text().trim();
    ((1..=2).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit()))
        .then(|| t.parse().ok())
        .flatten()
}

/// Whether `run` is a reference number raised right after a word of `word`: its baseline
/// higher than the word's, and it starts close after the end of the run's glyphs (`None`)
/// or in one of the run's white spaces right after a word (`Some` of that space). The word
/// before it does not end in a digit: `mtl.` or `%` take references, a number does not.
fn raised_ref<R: LineRun>(run: &R, word: &R) -> Option<(u8, Option<Split>)> {
    let n = digits(run)?;
    let size = word.size();
    let rise = run.y() - word.y();
    let raised = rise >= REF_RISE * size
        || (run.size() <= REF_SMALLER * size && rise >= REF_RISE_SMALL * size);
    if !raised {
        return None;
    }
    let no_digit = |w: &str| w.chars().next_back().is_some_and(|c| !c.is_ascii_digit());
    let lead = REF_LEAD * size;
    let end = word.x() + word.w();
    // The word end nearest to the number among those it may follow.
    let mut best: Option<(f32, Option<Split>)> = None;
    let mut offer = |dist: f32, at: Option<Split>| {
        if best.is_none_or(|b| dist < b.0) {
            best = Some((dist, at));
        }
    };
    if run.x() >= end - lead && run.x() - end <= REF_GAP * size && no_digit(word.text().trim_end())
    {
        offer((run.x() - end).abs(), None);
    }
    let spaces = splits(word);
    for (k, g) in spaces.iter().enumerate() {
        if !no_digit(word.text()[..g.byte].trim_end()) {
            continue;
        }
        let (low, high) = match g.gap {
            Some(_) => (g.from - lead, g.to),
            // A space by the share of characters may sit off by some letters either way: the
            // number may stand from the start of the word before to a little before the end
            // of the word after, and goes after the word end nearest to it.
            None => {
                let start = k.checked_sub(1).map_or(word.x(), |p| spaces[p].to);
                let next_end = spaces.get(k + 1).map_or(end, |s| s.from);
                (start.min(g.from - lead), (next_end - lead).max(g.to))
            }
        };
        if run.x() >= low && run.x() <= high {
            offer((run.x() - g.from).abs(), Some(*g));
        }
    }
    best.map(|b| (n, b.1))
}

/// The value of one or two superscript digits. More digits are no marker (a power, `10³⁰⁰`).
fn superscript_value(digits: &str) -> Option<u8> {
    let mut n: u8 = 0;
    let mut count = 0;
    for c in digits.chars() {
        let d = SUPERSCRIPTS.iter().position(|s| *s == c)?;
        count += 1;
        if count > 2 {
            return None;
        }
        n = n * 10 + d as u8;
    }
    (count >= 1).then_some(n)
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

/// The words of `text` that end in a note marker (see [`word_mark`]), each with its byte
/// offset, its marker and the marker's length. Stars that close text a star opens on the same
/// line (`*Abc.-Nr. 12345*`, a starred item) are no marker: they are the end of the item, and a
/// star the same size and on the line of the text is no evidence of a reference.
pub fn word_marks(text: &str) -> Vec<(usize, &str, NoteMark, usize)> {
    let mut out = Vec::new();
    let mut open = false;
    for (start, word) in word_spans(text) {
        let w = word.trim_end_matches([':', ',', '.', ';']);
        let opens = starred(w);
        let mark = word_mark(word);
        let closes = mark.is_some_and(|m| matches!(m.0, NoteMark::Stars(_)))
            && (open || opens && w.trim_matches('*').chars().any(char::is_alphanumeric));
        if opens {
            open = true;
        }
        match mark {
            Some(_) if closes => open = false,
            Some((mark, len)) => out.push((start, word, mark, len)),
            None => {}
        }
    }
    out
}

/// Whether `word` opens with stars right before a letter or digit (`*Abc`): the start of a
/// starred item.
fn starred(word: &str) -> bool {
    let rest = word.trim_start_matches('*');
    rest.len() < word.len() && rest.starts_with(char::is_alphanumeric)
}

/// The words of `text` with their byte offsets.
fn word_spans(text: &str) -> Vec<(usize, &str)> {
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

/// The marker a piece of a legend line opens with: stars, a number with `)` or a superscript
/// number before its text, a run of digits raised against the run after it, or plain digits
/// and a space.
fn opening_mark<R: LineRun>(runs: &[R]) -> Option<NoteMark> {
    opening(runs).map(|o| o.0)
}

/// [`opening_mark`] with the bytes the marker and the white space after it take of the
/// trimmed text of the piece: what the footnote's text leaves out.
fn opening<R: LineRun>(runs: &[R]) -> Option<(NoteMark, usize)> {
    let mut inked = runs.iter().filter(|r| ink(r.text()) > 0);
    let first = inked.next()?;
    if let Some(next) = inked.next()
        && let Some(n) = raised_digits(first, next)
    {
        return Some((NoteMark::Raised(n), first.text().trim().len()));
    }
    // The marker and the start of the text after it stand in the first two runs with ink.
    let head = runs
        .iter()
        .enumerate()
        .filter(|(_, r)| ink(r.text()) > 0)
        .nth(1)
        .map_or(runs.len(), |(i, _)| i + 1);
    let text: String = runs[..head].iter().map(|r| r.text()).collect();
    let text = text.trim_start();
    // The bytes up to the text after the marker.
    let cut = |after: &str| text.len() - after.trim_start().len();
    let opens_text = |after: &str| after.trim_start().starts_with(char::is_alphanumeric);
    let stars = text.len() - text.trim_start_matches('*').len();
    if stars > 0 {
        let rest = &text[stars..];
        // A piece that also ends in stars after its text is a starred item (`*Abc.-Nr. ab*`),
        // not a footnote opened by its marker.
        let whole: String = runs.iter().map(|r| r.text()).collect();
        let whole = whole.trim().trim_end_matches([':', ',', '.', ';']);
        let item = whole.len() > stars
            && whole.ends_with('*')
            && whole.trim_end_matches('*').ends_with(char::is_alphanumeric);
        return (stars <= 3 && opens_text(rest) && !item)
            .then(|| (NoteMark::Stars(stars as u8), cut(rest)));
    }
    let n = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if (1..=2).contains(&n) {
        let value: u8 = text[..n].parse().ok()?;
        let rest = &text[n..];
        if let Some(after) = rest.strip_prefix(')') {
            return after
                .starts_with(char::is_whitespace)
                .then(|| (NoteMark::Paren(value), cut(after)));
        }
        return rest
            .starts_with(char::is_whitespace)
            .then(|| (NoteMark::Raised(value), cut(rest)));
    }
    let sup: String = text
        .chars()
        .take_while(|c| SUPERSCRIPTS.contains(c))
        .collect();
    let value = superscript_value(&sup)?;
    let rest = &text[sup.len()..];
    opens_text(rest).then(|| (NoteMark::Raised(value), cut(rest)))
}

/// The references of line `k` of `all`, whose first run is run `starts[k]` of the page:
/// markers that end a word go to `marks`, numbers raised right after a word are returned. A
/// number that opens the line belongs to the last word of the line before when it stands
/// raised, smaller and close at that word's end.
fn references<R: LineRun>(
    all: &[TextLine<'_, R>],
    starts: &[usize],
    k: usize,
    marks: &mut HashSet<NoteMark>,
) -> Vec<RaisedRef> {
    let l = &all[k];
    let start = starts[k];
    marks.extend(word_marks(&l.text).into_iter().map(|m| m.2));
    let mut raised = Vec::new();
    let mut word: Option<(usize, &R)> = None;
    for (i, r) in l.runs.iter().enumerate() {
        if ink(r.text()) == 0 {
            continue;
        }
        let found = match word {
            Some((w, run)) => raised_ref(r, run).map(|(n, gap)| {
                let place = match gap {
                    None => RefPlace::End,
                    Some(split) => RefPlace::Inside {
                        word: (start + w) as u32,
                        split,
                    },
                };
                (n, place)
            }),
            None => above(all, starts, k, r),
        };
        match found {
            Some((n, place)) => {
                marks.insert(NoteMark::Raised(n));
                raised.push(RaisedRef {
                    run: (start + i) as u32,
                    mark: NoteMark::Raised(n),
                    place,
                });
            }
            // A reference does not end the word it follows: the next one may follow it too.
            None => word = Some((i, r)),
        }
    }
    raised
}

/// The reference `r`, the first run of line `k`, makes at the end of the line before, with
/// the index of that line's last run with ink.
fn above<R: LineRun>(
    all: &[TextLine<'_, R>],
    starts: &[usize],
    k: usize,
    r: &R,
) -> Option<(u8, RefPlace)> {
    let p = k.checked_sub(1)?;
    let prev = &all[p];
    let (i, word) = prev
        .runs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, w)| ink(w.text()) > 0)?;
    if r.size() >= word.size() {
        return None;
    }
    let (n, gap) = raised_ref(r, word)?;
    gap.is_none().then_some((
        n,
        RefPlace::Above {
            word: (starts[p] + i) as u32,
        },
    ))
}

/// The pieces of a line: its runs cut where a piece gap parts two runs with ink.
fn pieces<'a, R: LineRun>(l: &TextLine<'a, R>) -> Vec<(usize, &'a [R])> {
    let mut cuts = vec![0];
    let mut end: Option<f32> = None;
    for (i, r) in l.runs.iter().enumerate() {
        if ink(r.text()) == 0 {
            continue;
        }
        if let Some(e) = end
            && r.x() - e > PIECE_GAP * r.size()
        {
            cuts.push(i);
        }
        end = Some(r.x() + r.w());
    }
    cuts.push(l.runs.len());
    cuts.windows(2)
        .map(|w| (w[0], &l.runs[w[0]..w[1]]))
        .filter(|(_, p)| p.iter().any(|r| ink(r.text()) > 0))
        .collect()
}

/// [`find_lines`] over all lines of the page of the runs `texts`.
#[cfg(test)]
fn find_legend<R: LineRun>(texts: &[R]) -> Option<Legend> {
    let all = text_lines(texts);
    let mut starts = Vec::with_capacity(all.len());
    let mut at = 0;
    for l in &all {
        starts.push(at);
        at += l.runs.len();
    }
    find_lines(&all, &starts)
}

/// The legend of a page, read by position: its lines are taken top down by
/// baseline. It opens at the first line that opens with a note marker which also stands as a
/// reference above, in a size no larger than every line above and with no larger line below.
/// Its lines are the lines in that size from there down; lines in other sizes stay text. Each
/// piece of a legend line (cut at piece gaps, so columns side by side stay apart) opens a
/// footnote with a marker or goes on the footnote of its column on the line above. Pieces that
/// go on nothing stay text; when they outnumber the footnotes there is no legend.
///
/// `all` are the lines of the page without its running headers and footers, line `k` starting
/// at run `starts[k]` of the page.
pub(super) fn find_lines<R: LineRun>(all: &[TextLine<'_, R>], starts: &[usize]) -> Option<Legend> {
    // Top down by baseline; lines on one baseline keep their order.
    let mut order: Vec<usize> = (0..all.len())
        .filter(|&k| !all[k].text.is_empty())
        .collect();
    order.sort_by(|&a, &b| all[b].y.total_cmp(&all[a].y));
    let mut largest_below = vec![f32::MIN; order.len() + 1];
    for p in (0..order.len()).rev() {
        largest_below[p] = largest_below[p + 1].max(all[order[p]].size);
    }
    let mut marks: HashSet<NoteMark> = HashSet::new();
    let mut raised: Vec<(f32, RaisedRef)> = Vec::new();
    let mut smallest = f32::MAX;
    let mut top = None;
    for (p, &k) in order.iter().enumerate() {
        let l = &all[k];
        let opens = pieces(l)
            .first()
            .and_then(|piece| opening_mark(piece.1))
            .is_some_and(|m| marks.contains(&m));
        if p > 0
            && opens
            && l.size <= smallest + SIZE_SLACK
            && largest_below[p + 1] <= l.size + SIZE_SLACK
        {
            top = Some((l.y, l.size));
            break;
        }
        raised.extend(
            references(all, starts, k, &mut marks)
                .into_iter()
                .map(|r| (l.y, r)),
        );
        smallest = smallest.min(l.size);
    }
    let (top, size) = top?;
    // Only references that stand above the legend count.
    let raised: Vec<RaisedRef> = raised
        .into_iter()
        .filter(|(y, _)| *y > top + EDGE)
        .map(|(_, r)| r)
        .collect();
    let mut entries: Vec<LegendEntry> = Vec::new();
    let mut lines: Vec<u32> = Vec::new();
    let mut loose = 0usize;
    for &k in &order {
        let l = &all[k];
        if l.y > top + EDGE || (l.size - size).abs() > SIZE_SLACK {
            continue;
        }
        for (from, piece) in pieces(l) {
            let Some(x) = piece.iter().find(|r| ink(r.text()) > 0).map(|r| r.x()) else {
                continue;
            };
            let first = starts[k] + from;
            let runs = (first..first + piece.len()).map(|i| i as u32);
            if let Some((mark, cut)) = opening(piece) {
                entries.push(LegendEntry {
                    mark,
                    cut,
                    x,
                    top: l.y,
                    last: l.y,
                    h: l.h,
                    runs: runs.collect(),
                });
                lines.push(piece[0].line());
                continue;
            }
            // The footnote of its column whose last line stands right above.
            let column = entries
                .iter_mut()
                .filter(|e| e.x <= x + EDGE && e.last > l.y)
                .max_by(|a, b| a.x.total_cmp(&b.x).then(b.last.total_cmp(&a.last)));
            match column {
                Some(e) if e.last - l.y <= GAP_RATIO * e.h.max(l.h).max(1.0) => {
                    e.runs.extend(runs);
                    e.last = l.y;
                    lines.push(piece[0].line());
                }
                _ => loose += 1,
            }
        }
    }
    if loose > entries.len() {
        return None;
    }
    lines.sort_unstable();
    lines.dedup();
    Some(Legend {
        lines,
        entries,
        raised,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{FontKind, GapIn, Shift, TextRun};

    fn run(text: &str, line: u32, x: f32, y: f32, size: f32) -> TextRun {
        TextRun {
            text: text.into(),
            x,
            y,
            w: text.len() as f32 * size * 0.5,
            h: size,
            size,
            font: 0,
            font_kind: FontKind::Unknown,
            bold: false,
            italic: false,
            color: None,
            shift: Shift::Base,
            line,
            off_axis: false,
            gaps: Vec::new(),
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
        // Three superscript digits or more are a power, no marker, and never overflow.
        assert_eq!(word_mark("10\u{b3}\u{2070}\u{2070}"), None);
        assert_eq!(word_mark("x\u{2079}\u{2079}\u{2079}"), None);
        assert_eq!(superscript_value("\u{b3}\u{2070}\u{2070}"), None);
        assert_eq!(superscript_value("\u{2079}\u{2079}"), Some(99));
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
        assert_eq!(raised_ref(&close, &word), Some((2, None)));
        // On the baseline, far away, or over the word's glyphs: no reference.
        assert_eq!(raised_ref(&run("2", 0, 72.0, 720.0, 8.0), &word), None);
        assert_eq!(raised_ref(&run("2", 0, 140.0, 724.0, 6.0), &word), None);
        assert_eq!(raised_ref(&run("2", 0, 50.0, 722.5, 8.0), &word), None);
        // After a label that ends in a stop or a sign, not after a number.
        let short = run("mtl.", 0, 40.0, 720.0, 9.0);
        assert_eq!(
            raised_ref(&run("1", 0, 59.0, 722.5, 8.0), &short),
            Some((1, None))
        );
        let number = run("1234", 0, 40.0, 720.0, 9.0);
        assert_eq!(raised_ref(&run("1", 0, 59.0, 722.5, 8.0), &number), None);
    }

    /// Runs of a page with references raised after `Einheit` and `Beitrag`, a line of body
    /// text, and the legend pieces `legend` (text, x, line, y) in 6 pt at the bottom.
    fn legend_page(legend: &[(&str, f32, u32, f32)]) -> Vec<TextRun> {
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

    fn legend_texts(runs: &[TextRun]) -> Vec<String> {
        find_legend(runs)
            .map(|f| {
                f.entries
                    .iter()
                    .map(|e| {
                        e.runs
                            .iter()
                            .map(|&i| runs[i as usize].text.as_str())
                            .collect()
                    })
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
        // The legend names its line, and the two references above it.
        let found = find_legend(&runs).expect("a legend");
        assert_eq!(found.lines, [2]);
        let raised: Vec<(u32, NoteMark)> = found.raised.iter().map(|r| (r.run, r.mark)).collect();
        assert_eq!(raised, [(1, NoteMark::Raised(2)), (3, NoteMark::Raised(3))]);
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
        assert_eq!(find_legend(&runs).expect("a legend").lines, [2, 3]);
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
    fn a_marker_after_wide_letters_is_found() {
        // Without glyph gaps the space after `MW` is estimated by characters and ends early:
        // the marker right after the real glyph end still goes after `MW`.
        // Helvetica 9 pt: `MW` ends at 56, the estimate says 49 and the next word at 53.5.
        let mut word = run("MW ii", 0, 40.0, 720.0, 9.0);
        word.w = 22.5;
        assert_eq!(
            raised_ref(&run("4", 0, 56.3, 722.5, 6.0), &word).map(|r| r.0),
            Some(4)
        );
        assert!(matches!(
            raised_ref(&run("4", 0, 56.3, 722.5, 6.0), &word),
            Some((4, Some(_)))
        ));
        // At the end of `ii` it follows the whole run.
        assert_eq!(
            raised_ref(&run("4", 0, 61.0, 722.5, 6.0), &word),
            Some((4, None))
        );
    }

    #[test]
    fn a_marker_after_a_narrow_word_goes_after_it() {
        // `Xa ii Yb` by the share of characters: `Xa` ends at 50, `ii` at 65, the run at 80.
        // The number at 61 stands after the narrow `ii`, nearer its end than the end of `Xa`.
        let mut word = run("Xa ii Yb", 0, 40.0, 720.0, 9.0);
        word.w = 40.0;
        let found = raised_ref(&run("4", 0, 61.0, 722.5, 6.0), &word);
        assert_eq!(found.and_then(|f| f.1).map(|s| s.byte), Some(6));
        let first = raised_ref(&run("4", 0, 51.0, 722.5, 6.0), &word);
        assert_eq!(first.and_then(|f| f.1).map(|s| s.byte), Some(3));
    }

    /// A glyph gap names the words before it: the gap after the second word of `Xa ii Yb`
    /// splits at the byte of `Yb`, whatever the gap's place in the run's list.
    #[test]
    fn a_glyph_gap_splits_after_its_words() {
        let mut word = run("Xa ii Yb", 0, 40.0, 720.0, 9.0);
        word.w = 40.0;
        word.gaps = vec![GapIn {
            words: 2,
            from: 64.0,
            to: 67.0,
        }];
        let s = splits(&word);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].byte, s[0].gap), (6, Some(0)));
        let found = raised_ref(&run("4", 0, 64.5, 722.5, 6.0), &word);
        assert_eq!(found.and_then(|f| f.1).map(|s| s.byte), Some(6));
    }
}
