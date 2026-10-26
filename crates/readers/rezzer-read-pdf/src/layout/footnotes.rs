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

use super::{EDGE, Flow, GAP_RATIO, Line, Run, SIZE_SLACK, ink};

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
    /// Each footnote, in reading order (column by column, top down): its label, its runs
    /// and the bytes its marker takes of their trimmed text.
    pub notes: Vec<(String, Vec<Run>, usize)>,
    /// The label of each marker the legend holds.
    labels: HashMap<NoteMark, String>,
    /// Why a legend the style pass found is not used, as a loss.
    pub refused: Option<String>,
}

/// One footnote while the legend is read.
#[derive(Debug, Clone)]
struct Entry {
    mark: NoteMark,
    /// The bytes of its first piece's trimmed text the marker takes.
    cut: usize,
    /// The left edge of its marker, and the top and the last baseline of its lines.
    x: f32,
    top: f32,
    last: f32,
    h: f32,
    ids: Vec<usize>,
}

/// Where a raised reference number goes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Place {
    /// After the run before it on its line.
    End,
    /// Into a white space of the run `word` of its line, after the word before it.
    Inside { word: usize, split: Split },
    /// After the run `word`, the last of the line above: the number opens its own line
    /// though it stands raised at the end of that word.
    Above { word: usize },
}

/// A white space between two words of a run: where the next word starts in the text, and
/// the end of the ink before it and the start of the ink after it. From the glyphs (`gap`
/// names the run's gap) or, for a run without them, by the share of characters.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Split {
    byte: usize,
    from: f32,
    to: f32,
    gap: Option<usize>,
}

/// The white spaces between the words of `run`, in order.
fn splits(run: &Run) -> Vec<Split> {
    // Where each word after the first starts, and how many words stand before it.
    let mut starts = Vec::new();
    let mut words = 0;
    let mut in_word = false;
    for (i, c) in run.text.char_indices() {
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
    if !run.gaps.is_empty() {
        return run
            .gaps
            .iter()
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
    let lead = run.text.len() - run.text.trim_start().len();
    let text = run.text.trim();
    let total = text.chars().count().max(1) as f32;
    let at = |byte: usize| {
        let chars = run.text[lead..byte.max(lead)].chars().count() as f32;
        run.x + run.w * chars / total
    };
    starts
        .iter()
        .map(|&(byte, _)| {
            let end = run.text[..byte].trim_end().len();
            Split {
                byte,
                from: at(end),
                to: at(byte),
                gap: None,
            }
        })
        .collect()
}

/// `run` cut at `split`: the words before it and the words after it as a run that starts a
/// new word.
fn cut(run: &Run, split: &Split) -> Option<(Run, Run)> {
    if let Some(k) = split.gap {
        return run.cut_at(k);
    }
    let end = run.x + run.w;
    let mut before = run.clone();
    before.text = run.text.get(..split.byte)?.trim_end().to_string();
    before.w = (split.from - run.x).max(0.0);
    let mut after = run.clone();
    after.text = format!(" {}", run.text.get(split.byte..)?);
    after.x = split.to;
    after.w = (end - split.to).max(0.0);
    Some((before, after))
}

/// A raised reference number: its run, its marker and where it goes.
#[derive(Debug, Clone, Copy)]
struct Raised {
    run: usize,
    mark: NoteMark,
    place: Place,
}

/// What [`find`] reads: the entries, and the raised reference numbers above them. Run
/// indices are of the page's runs as the style pass sees them.
#[derive(Debug, Clone)]
pub(super) struct Found {
    entries: Vec<Entry>,
    raised: Vec<Raised>,
}

impl Found {
    /// The runs of the legend.
    pub(super) fn legend_runs(&self) -> impl Iterator<Item = usize> + '_ {
        self.entries.iter().flat_map(|e| e.ids.iter().copied())
    }

    /// `self` with every run index moved to where `place` puts it; `None` when a run is not
    /// there.
    fn placed(&self, place: impl Fn(usize) -> Option<usize>) -> Option<Found> {
        let mut found = self.clone();
        for e in &mut found.entries {
            for i in &mut e.ids {
                *i = place(*i)?;
            }
        }
        for r in &mut found.raised {
            r.run = place(r.run)?;
            match &mut r.place {
                Place::End => {}
                Place::Inside { word, .. } | Place::Above { word } => *word = place(*word)?,
            }
        }
        Some(found)
    }
}

/// Marks the raised references of the untagged page `page` to the legend `found` (by the
/// style pass, which saw the page's runs `flow` keeps by their place) in `flow`, and takes
/// the legend's runs out of `flow`.
pub(super) fn take(page: u32, flow: &mut Flow, found: Option<&Found>) -> Legend {
    let at = std::mem::take(&mut flow.at);
    let placed = found.and_then(|f| f.placed(|i| at.binary_search(&i).ok()));
    flow.at = at;
    let Some(found) = placed else {
        // A legend whose lines are no longer all in the page's text is not used, and the
        // read says so.
        return Legend {
            refused: found.map(|_| {
                format!(
                    "footnotes on page {page} are not linked to their markers: the lines of \
                     their legend are not all in the page's text; the legend stays text"
                )
            }),
            ..Legend::default()
        };
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
    // A raised reference to a footnote of the legend becomes a reference run after its word;
    // one to no footnote stays text where it is, set apart from its word.
    let mut inside = Vec::new();
    for r in &found.raised {
        let Some(label) = labels.get(&r.mark) else {
            let run = &mut flow.runs[r.run];
            if !run.text.starts_with(char::is_whitespace) {
                run.text.insert(0, ' ');
            }
            continue;
        };
        match r.place {
            Place::End => {}
            Place::Above { word } => {
                // The number goes to the end of the line above, which its runs follow.
                let (line, y) = (flow.runs[word].line, flow.runs[word].y);
                let run = &mut flow.runs[r.run];
                run.line = line;
                run.y = y;
            }
            Place::Inside { word, split } => inside.push((word, split, r.run)),
        }
        let run = &mut flow.runs[r.run];
        run.text.clone_from(label);
        run.note = true;
    }
    let moved = place_inside(flow, inside);
    let mut owner: Vec<Option<usize>> = vec![None; flow.runs.len()];
    for (n, e) in entries.iter().enumerate() {
        for &i in &e.ids {
            owner[moved[i]] = Some(n);
        }
    }
    let mut runs: Vec<Vec<Run>> = vec![Vec::new(); entries.len()];
    let taken = flow.take(|i, _| owner[i].is_some());
    for ((r, _, _), n) in taken.into_iter().zip(owner.into_iter().flatten()) {
        runs[n].push(r);
    }
    Legend {
        notes: named
            .into_iter()
            .zip(runs)
            .zip(&entries)
            .map(|((label, runs), e)| (label, runs, e.cut))
            .collect(),
        labels,
        refused: None,
    }
}

/// Puts each reference run of `inside` (word run, gap, reference run) into the white space
/// of its word run: the word run is cut at the gap and the reference stands between the two
/// parts. Returns where each run of the flow went. A word run that no longer has the gap
/// keeps the reference after it.
fn place_inside(flow: &mut Flow, mut inside: Vec<(usize, Split, usize)>) -> Vec<usize> {
    let n = flow.runs.len();
    if inside.is_empty() {
        return (0..n).collect();
    }
    inside.sort_by_key(|c| (c.0, c.1.byte));
    let refs: HashSet<usize> = inside.iter().map(|c| c.2).collect();
    let all = std::mem::take(flow);
    let mut runs: Vec<Option<Run>> = all.runs.into_iter().map(Some).collect();
    let mut moved = vec![0; n];
    let mut out = Flow::default();
    let push = |out: &mut Flow, run: Run, slot, at| {
        out.runs.push(run);
        out.slots.push(slot);
        out.at.push(at);
        out.runs.len() - 1
    };
    let mut next = 0;
    for i in 0..n {
        if refs.contains(&i) {
            continue;
        }
        let (slot, at) = (all.slots[i], all.at[i]);
        let Some(run) = runs[i].take() else {
            continue;
        };
        moved[i] = out.runs.len();
        // The cuts of this run, in order, each in the part after the cut before: its gaps
        // and bytes count from that cut.
        let mut rest = Some(run);
        let mut last: Option<Split> = None;
        while next < inside.len() && inside[next].0 == i {
            let (_, split, r) = inside[next];
            next += 1;
            let reference = runs[r].take().expect("a reference is placed once");
            // The part after a cut starts with one space before its first word, and its gaps
            // count from the cut.
            let here = match last {
                // Two references after one word: the second follows the first.
                Some(l) if l.byte == split.byte => {
                    moved[r] = push(&mut out, reference, slot, at);
                    continue;
                }
                None => Some(split),
                Some(l) => (split.byte + 1).checked_sub(l.byte).map(|byte| Split {
                    byte,
                    gap: split.gap.zip(l.gap).and_then(|(g, k)| g.checked_sub(k + 1)),
                    ..split
                }),
            };
            let parts = rest.as_ref().zip(here).and_then(|(x, here)| cut(x, &here));
            match parts {
                Some((before, after)) => {
                    push(&mut out, before, slot, at);
                    rest = Some(after);
                    last = Some(split);
                }
                // Without the gap the reference goes after what is left of the run.
                None => {
                    if let Some(x) = rest.take() {
                        push(&mut out, x, slot, at);
                    }
                }
            }
            moved[r] = push(&mut out, reference, slot, at);
        }
        if let Some(x) = rest {
            push(&mut out, x, slot, at);
        }
    }
    *flow = out;
    moved
}

/// Writes the references of `blocks` to the footnotes `unwritten` (by label) as the text of
/// their markers: a stop that cut the legend leaves no reference without its footnote.
pub(super) fn unlink(blocks: &mut [Block], legend: &Legend, unwritten: &[&str]) {
    if unwritten.is_empty() {
        return;
    }
    let marks: HashMap<&str, NoteMark> = legend
        .labels
        .iter()
        .map(|(m, l)| (l.as_str(), *m))
        .collect();
    let text = |id: &str| match marks.get(id) {
        Some(NoteMark::Stars(c)) => "*".repeat(usize::from(*c)),
        Some(NoteMark::Paren(n)) => format!("{n})"),
        Some(NoteMark::Raised(n)) => format!(" {n}"),
        None => String::new(),
    };
    let fix = |inlines: &mut Vec<Inline>| {
        for i in inlines.iter_mut() {
            if let Inline::FootnoteRef { id } = i
                && unwritten.contains(&id.as_str())
            {
                *i = Inline::plain(text(id));
            }
        }
    };
    walk_inlines(blocks, &mut |i| fix(i));
}

/// Calls `f` with every list of inlines of `blocks` outside links: paragraphs, headings, list
/// items, table cells and the blocks inside sections, quotes and callouts.
fn walk_inlines(blocks: &mut [Block], f: &mut dyn FnMut(&mut Vec<Inline>)) {
    for b in blocks {
        match b {
            Block::Heading { inlines, .. } | Block::Paragraph { inlines, .. } => f(inlines),
            Block::List { items, .. } => items.iter_mut().for_each(|i| walk_inlines(i, f)),
            Block::Table { rows, .. } => rows.iter_mut().flatten().for_each(|c| f(&mut c.inlines)),
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => walk_inlines(blocks, f),
            _ => {}
        }
    }
}

/// The `found-by` note of a page with `n` footnotes written, `linked` of them with their
/// references.
pub(super) fn note(page: u32, n: usize, linked: usize) -> Option<String> {
    let how = "set in one size below all other text, each opening with a note marker, the \
               first of which also stands after a word above";
    let what = match n {
        0 => return None,
        1 => format!("1 footnote at the bottom of page {page} is read from its legend, {how}"),
        _ => {
            format!("{n} footnotes at the bottom of page {page} are read from their legend, {how}")
        }
    };
    let refs = match (n, linked) {
        (1, 1) => "; the marker above is linked to it".to_string(),
        (1, 0) => "; no marker above is linked to it".to_string(),
        (_, 0) => "; no marker above is linked to them".to_string(),
        (n, l) if n == l => "; the markers above are linked to them".to_string(),
        (_, l) => format!("; {l} of them are linked to their markers above"),
    };
    Some(what + &refs)
}

/// The labels of the footnote references of `blocks`.
pub(super) fn referenced(blocks: &mut [Block]) -> HashSet<String> {
    let mut found = HashSet::new();
    walk_inlines(blocks, &mut |inlines| {
        for i in inlines.iter() {
            if let Inline::FootnoteRef { id } = i {
                found.insert(id.clone());
            }
        }
    });
    found
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

/// Whether `run` is a reference number raised right after a word of `word`: its baseline
/// higher than the word's, and it starts close after the end of the run's glyphs (`None`)
/// or in one of the run's white spaces right after a word (`Some` of that space). The word
/// before it does not end in a digit: `mtl.` or `%` take references, a number does not.
fn raised_ref(run: &Run, word: &Run) -> Option<(u8, Option<Split>)> {
    let n = digits(run)?;
    let size = word.size;
    let rise = run.y - word.y;
    let raised = rise >= REF_RISE * size
        || (run.size <= REF_SMALLER * size && rise >= REF_RISE_SMALL * size);
    if !raised {
        return None;
    }
    let no_digit = |w: &str| w.chars().next_back().is_some_and(|c| !c.is_ascii_digit());
    let lead = REF_LEAD * size;
    let end = word.x + word.w;
    // The word end nearest to the number among those it may follow.
    let mut best: Option<(f32, Option<Split>)> = None;
    let mut offer = |dist: f32, at: Option<Split>| {
        if best.is_none_or(|b| dist < b.0) {
            best = Some((dist, at));
        }
    };
    if run.x >= end - lead && run.x - end <= REF_GAP * size && no_digit(word.text.trim_end()) {
        offer((run.x - end).abs(), None);
    }
    let spaces = splits(word);
    for (k, g) in spaces.iter().enumerate() {
        if !no_digit(word.text[..g.byte].trim_end()) {
            continue;
        }
        let (low, high) = match g.gap {
            Some(_) => (g.from - lead, g.to),
            // A space by the share of characters may sit off by some letters either way: the
            // number may stand from the start of the word before to a little before the end
            // of the word after, and goes after the word end nearest to it.
            None => {
                let start = k.checked_sub(1).map_or(word.x, |p| spaces[p].to);
                let next_end = spaces.get(k + 1).map_or(end, |s| s.from);
                (start.min(g.from - lead), (next_end - lead).max(g.to))
            }
        };
        if run.x >= low && run.x <= high {
            offer((run.x - g.from).abs(), Some(*g));
        }
    }
    best.map(|b| (n, b.1))
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
    opening(runs).map(|o| o.0)
}

/// [`opening_mark`] with the bytes the marker and the white space after it take of the
/// trimmed text of the piece: what the footnote's text leaves out.
fn opening(runs: &[Run]) -> Option<(NoteMark, usize)> {
    let mut inked = runs.iter().filter(|r| ink(&r.text) > 0);
    let first = inked.next()?;
    if let Some(next) = inked.next()
        && let Some(n) = raised_digits(first, next)
    {
        return Some((NoteMark::Raised(n), first.text.trim().len()));
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
    // The bytes up to the text after the marker.
    let cut = |after: &str| text.len() - after.trim_start().len();
    let opens_text = |after: &str| after.trim_start().starts_with(char::is_alphanumeric);
    let stars = text.len() - text.trim_start_matches('*').len();
    if stars > 0 {
        let rest = &text[stars..];
        return (stars <= 3 && opens_text(rest)).then(|| (NoteMark::Stars(stars as u8), cut(rest)));
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
fn references(
    all: &[Line<'_>],
    starts: &[usize],
    k: usize,
    marks: &mut HashSet<NoteMark>,
) -> Vec<Raised> {
    let l = &all[k];
    let start = starts[k];
    marks.extend(l.text.split_whitespace().filter_map(word_mark).map(|m| m.0));
    let mut raised = Vec::new();
    let mut word: Option<(usize, &Run)> = None;
    for (i, r) in l.runs.iter().enumerate() {
        if ink(&r.text) == 0 {
            continue;
        }
        let found = match word {
            Some((w, run)) => raised_ref(r, run).map(|(n, gap)| {
                let place = match gap {
                    None => Place::End,
                    Some(split) => Place::Inside {
                        word: start + w,
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
                raised.push(Raised {
                    run: start + i,
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
fn above(all: &[Line<'_>], starts: &[usize], k: usize, r: &Run) -> Option<(u8, Place)> {
    let p = k.checked_sub(1)?;
    let prev = &all[p];
    let (i, word) = prev
        .runs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, w)| ink(&w.text) > 0)?;
    if r.size >= word.size {
        return None;
    }
    let (n, gap) = raised_ref(r, word)?;
    gap.is_none().then_some((
        n,
        Place::Above {
            word: starts[p] + i,
        },
    ))
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

/// The legend of a page, read by position: its lines are taken top down by baseline. It
/// opens at the first line that opens with a note marker which also stands as a reference
/// above, in a size no larger than every line above and with no larger line below. Its lines
/// are the lines in that size from there down; lines in other sizes stay text. Each piece
/// of a legend line (cut at piece gaps, so columns side by side stay apart) opens a footnote
/// with a marker or goes on the footnote of its column on the line above. Pieces that go on
/// nothing stay text; when they outnumber the footnotes there is no legend.
#[cfg(test)]
fn find(runs: &[Run]) -> Option<Found> {
    let all = super::lines(runs);
    let mut starts = Vec::with_capacity(all.len());
    let mut at = 0;
    for l in &all {
        starts.push(at);
        at += l.runs.len();
    }
    find_lines(&all, &starts)
}

/// [`find`] over the lines `all` of a page, line `k` starting at run `starts[k]`: the lines
/// of the page without its running headers and footers.
pub(super) fn find_lines(all: &[Line<'_>], starts: &[usize]) -> Option<Found> {
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
    let mut raised: Vec<(f32, Raised)> = Vec::new();
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
    let raised: Vec<Raised> = raised
        .into_iter()
        .filter(|(y, _)| *y > top + EDGE)
        .map(|(_, r)| r)
        .collect();
    let mut entries: Vec<Entry> = Vec::new();
    let mut loose = 0usize;
    for &k in &order {
        let l = &all[k];
        if l.y > top + EDGE || (l.size - size).abs() > SIZE_SLACK {
            continue;
        }
        for (from, piece) in pieces(l) {
            let Some(x) = piece.iter().find(|r| ink(&r.text) > 0).map(|r| r.x) else {
                continue;
            };
            let ids = (starts[k] + from)..(starts[k] + from + piece.len());
            if let Some((mark, cut)) = opening(piece) {
                entries.push(Entry {
                    mark,
                    cut,
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
    walk_inlines(blocks, &mut |i| link_inlines(i, legend));
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
        for (start, word) in word_spans(&text) {
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

    /// A legend the style pass found whose runs the page's text no longer holds is not used,
    /// and says so (Task 7 carry: no silent refusal).
    #[test]
    fn a_legend_not_found_again_is_refused_with_a_note() {
        let runs = vec![
            run("Betrag*", 0, 40.0, 700.0, 9.0),
            run("die Werte werden monatlich gemeldet", 1, 40.0, 690.0, 9.0),
            run("* steuerfrei", 2, 40.0, 60.0, 6.0),
        ];
        let found = find(&runs);
        assert!(found.is_some());
        // The page's text without its last line: the legend's run is gone.
        let mut flow = Flow::new(runs[..2].to_vec(), vec![super::super::Slot::Free; 2]);
        let legend = take(1, &mut flow, found.as_ref());
        assert!(legend.notes.is_empty());
        let why = legend.refused.expect("a note");
        assert!(why.contains("page 1"), "{why}");
        assert_eq!(flow.runs.len(), 2);
        // Without a legend found, nothing is said.
        let mut flow = Flow::new(runs[..2].to_vec(), vec![super::super::Slot::Free; 2]);
        assert!(take(1, &mut flow, None).refused.is_none());
    }

    #[test]
    fn a_marker_put_on_the_next_line_goes_to_its_word() {
        // The raised 1 after `Musterwert` opens the next line in the line order; the 2 after
        // the run of two labels `Xa Xb` stands after `Xa`, the 3 after `Xb`.
        let mut xa = run(" Xa Xb", 2, 40.0, 700.0, 6.6);
        xa.w = 15.0;
        let runs = vec![
            run("Musterwert", 0, 40.0, 720.0, 6.6),
            run("1", 1, 73.5, 721.9, 4.7),
            run(" Kl Muster", 1, 40.0, 712.0, 6.6),
            xa,
            run("2", 2, 47.0, 701.9, 4.7),
            run("3", 2, 55.5, 701.9, 4.7),
            run("die Werte werden monatlich gemeldet", 3, 40.0, 690.0, 9.0),
            run("1 A = Muster", 4, 40.0, 70.0, 6.0),
            run("2 B = Beispiel", 5, 40.0, 62.0, 6.0),
            run("3 C = Chor", 6, 40.0, 54.0, 6.0),
        ];
        let n = runs.len();
        let found = find(&runs);
        let mut flow = Flow::new(runs, vec![super::super::Slot::Free; n]);
        let legend = take(1, &mut flow, found.as_ref());
        assert_eq!(legend.notes.len(), 3);
        let texts: Vec<(&str, u32, bool)> = flow
            .runs
            .iter()
            .map(|r| (r.text.as_str(), r.line, r.note))
            .collect();
        assert_eq!(
            texts[..6],
            [
                ("Musterwert", 0, false),
                ("p1-1", 0, true),
                (" Kl Muster", 1, false),
                (" Xa", 2, false),
                ("p1-2", 2, true),
                (" Xb", 2, false),
            ]
        );
        assert_eq!(texts[6], ("p1-3", 2, true));
    }

    #[test]
    fn references_to_unwritten_footnotes_become_text() {
        let mut legend = Legend::default();
        legend.labels.insert(NoteMark::Stars(1), "p1-star".into());
        legend.labels.insert(NoteMark::Raised(2), "p1-2".into());
        let reference = |id: &str| Inline::FootnoteRef { id: id.into() };
        let mut blocks = vec![Block::Paragraph {
            inlines: vec![
                Inline::plain("Betrag"),
                reference("p1-star"),
                Inline::plain(" Einheit"),
                reference("p1-2"),
            ],
            role: None,
            anchor: None,
        }];
        // A stop wrote the first footnote only.
        unlink(&mut blocks, &legend, &["p1-2"]);
        let Block::Paragraph { inlines, .. } = &blocks[0] else {
            unreachable!()
        };
        assert_eq!(
            inlines,
            &[
                Inline::plain("Betrag"),
                reference("p1-star"),
                Inline::plain(" Einheit"),
                Inline::plain(" 2"),
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

    #[test]
    fn two_markers_after_one_word_follow_each_other() {
        let mut word = run(" Xa Xb", 0, 40.0, 700.0, 6.6);
        word.w = 15.0;
        let runs = vec![
            word,
            run("2", 0, 47.0, 701.9, 4.7),
            run("3", 0, 49.0, 701.9, 4.7),
            run("die Werte werden monatlich gemeldet", 1, 40.0, 690.0, 9.0),
            run("2 B = Beispiel", 2, 40.0, 62.0, 6.0),
            run("3 C = Chor", 3, 40.0, 54.0, 6.0),
        ];
        let n = runs.len();
        let found = find(&runs);
        let mut flow = Flow::new(runs, vec![super::super::Slot::Free; n]);
        take(1, &mut flow, found.as_ref());
        let texts: Vec<&str> = flow.runs.iter().map(|r| r.text.as_str()).take(4).collect();
        assert_eq!(texts, [" Xa", "p1-2", "p1-3", " Xb"]);
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
