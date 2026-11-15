// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a file says as a whole, learned from every page before any page is laid out: the
//! body size, the heading sizes, the running headers and footers, the footer of a file of two
//! pages and the footnote legend of each page.
//!
//! [`learn`] reads the pages three times over (the running lines first, then the body size
//! without them and without the legends, then the heading sizes against that body size). It
//! borrows the runs of each page as the reader keeps them (see [`FilePage`], [`LineRun`]):
//! nothing of the file's text is copied.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::input::{PageIn, TextRun};
use crate::line::{
    LineRun, TextLine, in_place_order, ink, place_order, placed_text, size_key, text_lines,
};

mod legend;

pub use legend::{Legend, LegendEntry, NoteMark, RaisedRef, RefPlace, Split, word_marks};

/// Share of the page height at the top and the bottom where running headers and footers sit.
/// pdflatex sets page numbers about 11 % above the bottom edge of an A4 page.
pub(crate) const BAND: f32 = 0.18;

/// Pages a header or footer line must repeat on. In a file of two pages, a footer line repeats
/// on both (see [`learn`]).
pub(crate) const REPEATS: usize = 3;

/// Lines of one header or footer sit at the same height on every page, within this share of
/// their text size: 4 points at 10 points. A share of the size of the line itself, so that a
/// file printed smaller or larger keeps its running lines.
pub(crate) const Y_STEP: f32 = 0.4;

/// A line at least this many times the body size is a heading in an untagged file.
pub const HEADING_RATIO: f32 = 1.2;

/// A line smaller than this many times the body size is a heading only when it is led by
/// bold ink (see [`bold_led`]): data lines and labels set a little larger than the body are
/// no headings.
pub const HEADING_PLAIN_RATIO: f32 = 1.5;

/// A heading line is shorter than this many characters.
pub(crate) const HEADING_CHARS: usize = 120;

/// Currency words and signs that may stand after an amount.
pub const CURRENCIES: [&str; 7] = ["EUR", "USD", "CHF", "GBP", "\u{20ac}", "$", "\u{a3}"];

/// Memory the mid-page line texts the file pass keeps may take, to tell running headers from
/// content: the text and about 64 bytes of set overhead per line. Past it, no more are kept:
/// a header text that also stands mid-page later in a huge file may then keep its header
/// role.
#[cfg(not(test))]
const BODY_KEY_BYTES: usize = 32 << 20;
#[cfg(test)]
const BODY_KEY_BYTES: usize = 1_000;

/// Set overhead of one kept line text.
const BODY_KEY_COST: usize = 64;

#[cfg(test)]
thread_local! {
    /// The mid-page line texts the last file pass kept.
    static BODY_KEPT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The band of the page a running line stands in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Band {
    Top,
    Bottom,
}

/// A running header or footer: a line text (see [`furniture_key`]) in a band of the page that
/// repeats on at least [`REPEATS`] pages at one height (or on both pages of a file of two, at
/// the bottom). One group of such heights is one `Running`; a text at two heights is two.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub key: String,
    pub band: Band,
    /// The pages of the group, in order.
    pub pages: Vec<u32>,
    /// The lowest and the highest middle of its lines (see [`TextLine::middle`]).
    pub at: [f32; 2],
    /// Whether its lines count the pages (see [`counts_pages`]): the number of each is the
    /// number of its page plus one offset, and the count of the pages after it, when the lines
    /// have one, is one count. A line that only repeats (`Total 120` on every page), or whose
    /// number changes in another way (`Total 120`, `Total 140`), does not.
    pub counts: bool,
}

/// The numbers of a line that may count the pages: its one number, or its two (the page and
/// the count of the pages). `None` for a line without a number, with more than two, or with
/// one too long to hold.
fn page_numbers(text: &str) -> Option<(u32, Option<u32>)> {
    let mut groups = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|g| !g.is_empty());
    let first = groups.next()?.parse().ok()?;
    let count = match groups.next() {
        Some(g) => Some(g.parse().ok()?),
        None => None,
    };
    groups.next().is_none().then_some((first, count))
}

/// Whether the lines of one running line, each with its page and its numbers (see
/// [`page_numbers`]), count the pages: every line has a number; the number goes up by one
/// from page to page, so it is the page's own number plus one offset for all of them (a file
/// may start at another number than its first page); a second number is on every line or on
/// none, the same on all, and not below the number of any; and the lines stand on two pages
/// or more, as one page alone shows no step.
///
/// This is the evidence that a number at the foot of a page is its page number and no value:
/// an amount that repeats, or that changes from page to page, does not follow the pages.
fn counts_pages(lines: impl IntoIterator<Item = (u32, Option<(u32, Option<u32>)>)>) -> bool {
    let mut offset: Option<(i64, Option<u32>)> = None;
    let mut pages: BTreeSet<u32> = BTreeSet::new();
    for (page, numbers) in lines {
        let Some((number, count)) = numbers else {
            return false;
        };
        if count.is_some_and(|c| c < number) {
            return false;
        }
        let this = (i64::from(number) - i64::from(page), count);
        if *offset.get_or_insert(this) != this {
            return false;
        }
        pages.insert(page);
    }
    pages.len() >= 2
}

/// What the layout learns from the whole file before it lays out a page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileFacts {
    /// The most common font size, by ink character count, of the text outside running
    /// headers and footers and outside the footnote legends of an untagged file: the mean
    /// exact size of the winning half point.
    pub body_size: f32,
    /// The distinct sizes of the lines that may head (see [`may_head`]), largest first: the
    /// place of a size is its heading level.
    pub heading_sizes: Vec<f32>,
    /// The running headers and footers, by band, key and height.
    pub running: Vec<Running>,
    /// The keys of the running footers that are running by the two-page rule only, sorted.
    /// They stand in the bottom band, as the rule asks. They are running footers by their role,
    /// but their ink still counts for the body size and the text sizes of a page: an invoice
    /// of two pages keeps the body size and so the headings it had.
    pub two_page: Vec<String>,
    /// The footnote legend of each page of an untagged file that has one.
    pub legends: BTreeMap<u32, Legend>,
}

/// The text of a line with every digit replaced, white space collapsed: the same running line
/// on every page, whatever page number it holds.
pub(crate) fn furniture_key(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect();
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The band of a page of `height` a line stands in, if any.
pub(crate) fn band_of<R>(line: &TextLine<'_, R>, height: f32) -> Option<Band> {
    if height <= 0.0 || height.is_nan() {
        return None;
    }
    let middle = line.middle();
    if middle >= height * (1.0 - BAND) {
        Some(Band::Top)
    } else if middle <= height * BAND {
        Some(Band::Bottom)
    } else {
        None
    }
}

/// The band and the key of a line that may be a running line on a page of `height`: it stands
/// in a band and has text.
///
/// The key is taken from the text of the line as its runs read in the order of their places
/// ([`placed_text`]): a line is found again by its key whatever order its runs come in.
pub(crate) fn running_key<R: LineRun>(
    line: &TextLine<'_, R>,
    height: f32,
) -> Option<(Band, String)> {
    let b = band_of(line, height)?;
    let key = furniture_key(&placed_text(line));
    (!key.is_empty()).then_some((b, key))
}

/// Whether a line on a page of `height` is a footer of the two-page rule: in the bottom band,
/// with a key of `two_page`.
pub fn two_page_line<R: LineRun>(
    line: &TextLine<'_, R>,
    height: f32,
    two_page: &HashSet<String>,
) -> bool {
    running_key(line, height).is_some_and(|(b, k)| b == Band::Bottom && two_page.contains(&k))
}

/// The heights of one running line (the lowest and the highest middle of its lines), and
/// whether its lines count the pages ([`Running::counts`]).
type RunningAt = ([f32; 2], bool);

/// The running lines by band and key, for the check of a page's lines.
#[derive(Debug, Clone, Default)]
pub struct RunningIndex {
    at: HashMap<(Band, String), Vec<RunningAt>>,
}

impl RunningIndex {
    pub fn new(running: &[Running]) -> RunningIndex {
        let mut at: HashMap<(Band, String), Vec<RunningAt>> = HashMap::new();
        for r in running {
            at.entry((r.band, r.key.clone()))
                .or_default()
                .push((r.at, r.counts));
        }
        RunningIndex { at }
    }

    /// Whether `line`, on a page of `height`, is a line of a running header or footer of the
    /// file whose lines count the pages ([`Running::counts`]).
    pub fn counts_pages<R: LineRun>(&self, line: &TextLine<'_, R>, height: f32) -> bool {
        let Some(key) = running_key(line, height) else {
            return false;
        };
        let m = line.middle();
        self.at.get(&key).is_some_and(|at| {
            at.iter()
                .any(|([low, high], counts)| *counts && m >= *low && m <= *high)
        })
    }

    /// The band of each of `lines`, a page of `height`, that is a running header or footer on
    /// it. A header stands above all text of the page and a footer below it: a repeated line
    /// inside the text stays text. Lines of white space only are no text of the page.
    /// A line marked in `content` is never a header or footer; an empty `content` marks none.
    pub fn bands<R: LineRun>(
        &self,
        lines: &[TextLine<'_, R>],
        height: f32,
        content: &[bool],
    ) -> Vec<Option<Band>> {
        let found: Vec<Option<Band>> = lines
            .iter()
            .enumerate()
            .map(|(k, l)| {
                if content.get(k).copied().unwrap_or(false) {
                    return None;
                }
                let key = running_key(l, height)?;
                let m = l.middle();
                self.at
                    .get(&key)?
                    .iter()
                    .any(|([low, high], _)| m >= *low && m <= *high)
                    .then_some(key.0)
            })
            .collect();
        // Lines of white space only are no text to stand above or below: a print run blanks
        // the empty lines of a footer column with spaces.
        let body = || {
            lines
                .iter()
                .zip(&found)
                .filter(|(l, b)| b.is_none() && !l.text.is_empty())
        };
        let top = body().map(|(l, _)| l.middle()).fold(f32::MIN, f32::max);
        let bottom = body().map(|(l, _)| l.middle()).fold(f32::MAX, f32::min);
        lines
            .iter()
            .zip(found)
            .map(|(l, b)| match b {
                Some(Band::Top) if l.middle() > top => Some(Band::Top),
                Some(Band::Bottom) if l.middle() < bottom => Some(Band::Bottom),
                _ => None,
            })
            .collect()
    }
}

/// Whether a line is large enough and short enough to be a heading against `body_size`: at
/// least [`HEADING_RATIO`] times it, under [`HEADING_CHARS`] characters, with a letter or digit.
pub(crate) fn is_heading_size<R>(line: &TextLine<'_, R>, body_size: f32) -> bool {
    body_size > 0.0
        && line.size >= body_size * HEADING_RATIO
        && line.text.chars().count() < HEADING_CHARS
        && line.text.chars().any(char::is_alphanumeric)
}

/// Whether a line of `size` is set in one of `sizes`: the same half point (see [`size_key`]).
/// A line in a text size of its page is body text there (see [`text_sizes`]).
pub fn in_text_size(size: f32, sizes: &[f32]) -> bool {
    let key = size_key(size);
    sizes.iter().any(|s| size_key(*s) == key)
}

/// Whether a line of `size` is under [`HEADING_PLAIN_RATIO`] times `body_size`: a heading of
/// this size needs bold ink to lead it, and lines of it that read as an address or as
/// sentences are a paragraph. The product form gives a plain answer for a body size of 0.
pub fn plain_size(size: f32, body_size: f32) -> bool {
    size < HEADING_PLAIN_RATIO * body_size
}

/// Whether an untagged line may be a heading by its own text and style: large enough, short,
/// not ending in an amount (`ends_in_amount`, the reader's own reading of amounts and dates),
/// and, when it is smaller than [`HEADING_PLAIN_RATIO`] times the body size, led by bold ink
/// (see [`bold_led`]) and not only data.
pub fn may_head<R: LineRun>(
    line: &TextLine<'_, R>,
    body_size: f32,
    ends_in_amount: &dyn Fn(&str) -> bool,
) -> bool {
    // A number set large (a form or file number) stays a heading.
    let large = !plain_size(line.size, body_size);
    is_heading_size(line, body_size)
        && !ends_in_amount(&line.text)
        && (large || (!data_only(&line.text) && bold_led(line)))
}

/// Whether a line is bold where it matters for a heading: all its ink is bold, or it opens
/// with bold ink and goes on in plain words that are no data (`SECTION 1 (TO BE COMPLETED BY
/// PAYEE)`). A bold label that ends in a colon before plain text is a label and its value
/// (`Kundennummer: 00000`), and so is a bold label before a plain number or code.
pub(crate) fn bold_led<R: LineRun>(line: &TextLine<'_, R>) -> bool {
    let inked: Vec<&R> = line.runs.iter().filter(|r| ink(r.text()) > 0).collect();
    let lead = inked.iter().take_while(|r| r.bold()).count();
    if lead == 0 {
        return false;
    }
    if lead == inked.len() {
        return true;
    }
    let label: String = inked[..lead].iter().map(|r| r.text()).collect();
    let rest: String = inked[lead..].iter().map(|r| r.text()).collect();
    !label.trim_end().ends_with(':') && !data_only(&rest)
}

/// Whether a line holds only numbers, codes, dates and amounts: each of its words holds a
/// digit or is a currency word or sign, and one holds a digit (`1234567 8901 2 30`,
/// `00123 45B`). A title holds a word (`Bericht 2025`).
pub fn data_only(text: &str) -> bool {
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

/// Deepest heading level the font size rule gives.
pub const SIZE_LEVELS: usize = 3;

/// A size that sets at least this share of a page's ink characters is a text size there.
const TEXT_SHARE: f32 = 0.25;

/// A size that sets at least this many ink characters of a page is a text size there.
const PAGE_BODY_CHARS: usize = 400;

/// A size is a text size by its share only when it sets ink on at least this many lines: the
/// title of a cover page with little else on it stays a heading.
const SHARE_LINES: usize = 3;

/// The text sizes of a page, smallest first: every size that sets at least [`TEXT_SHARE`] of
/// its ink characters on [`SHARE_LINES`] lines or more, or at least [`PAGE_BODY_CHARS`] of
/// them. A line in one of them is body text on that page, never a heading: the data lines of
/// a form set beside many small labels, or the back of a form printed in larger type, are
/// text. The caller picks the runs: a page's text without its running lines and its legend,
/// with the footers of the two-page rule.
pub fn text_sizes<'r, R: LineRun + 'r>(runs: impl IntoIterator<Item = &'r R>) -> Vec<f32> {
    // For each size: its ink characters and the lines it sets ink on.
    let mut chars: HashMap<i32, (usize, HashSet<u32>)> = HashMap::new();
    for r in runs {
        let n = ink(r.text());
        if n > 0 {
            let e = chars.entry(size_key(r.size())).or_default();
            e.0 += n;
            e.1.insert(r.line());
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

/// What the heading level of an untagged line depends on: the file's body and heading sizes
/// (see [`FileFacts`]), the text sizes of its page (see [`text_sizes`]) and the reader's
/// reading of amounts.
#[derive(Clone, Copy)]
pub struct Levels<'a> {
    pub body_size: f32,
    /// The heading sizes of the file, largest first (see [`FileFacts::heading_sizes`]).
    pub heading_sizes: &'a [f32],
    /// The text sizes of the page, smallest first; empty where no page is looked at.
    pub text_sizes: &'a [f32],
    /// Whether a text ends in an amount (`Gesamtbetrag 1.234,56 EUR`), the reader's own
    /// reading of amounts and dates.
    pub ends_in_amount: &'a dyn Fn(&str) -> bool,
}

impl Levels<'_> {
    /// The heading level of an untagged line, `None` for body text: a line that may head (see
    /// [`may_head`]) takes the place of its size among the heading sizes, at most
    /// [`SIZE_LEVELS`]. It is none on a page whose text is set in its size, or when it is less
    /// than [`HEADING_RATIO`] times the largest text size of the page.
    pub fn level<R: LineRun>(&self, line: &TextLine<'_, R>) -> Option<u8> {
        if !may_head(line, self.body_size, self.ends_in_amount) {
            return None;
        }
        let key = size_key(line.size);
        let largest = self.text_sizes.last().copied();
        let page_text = in_text_size(line.size, self.text_sizes);
        if page_text || !largest.is_none_or(|b| line.size >= b * HEADING_RATIO) {
            return None;
        }
        // A size the file pass did not see as a heading size (its line there held a smaller
        // cell beside it, which a table took later) takes its place among the sizes.
        let index = self
            .heading_sizes
            .iter()
            .position(|s| size_key(*s) == key)
            .unwrap_or_else(|| {
                self.heading_sizes
                    .iter()
                    .filter(|s| size_key(**s) > key)
                    .count()
            });
        Some(index.min(SIZE_LEVELS - 1) as u8 + 1)
    }
}

/// One page as [`learn`] reads it: its number, the height of the frame its runs are in, and
/// its runs in line order, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct FilePage<'a, R> {
    pub page: u32,
    pub height: f32,
    pub runs: &'a [R],
}

impl<'a> FilePage<'a, TextRun> {
    /// The text of an engine page.
    pub fn of(page: &'a PageIn) -> FilePage<'a, TextRun> {
        FilePage {
            page: page.page,
            height: page.size[1],
            runs: &page.texts,
        }
    }
}

/// A line seen in a band of a page: its middle, its page, its text size and its numbers (see
/// [`page_numbers`]).
type Seen = (f32, u32, f32, Option<(u32, Option<u32>)>);

/// The runs of a page in the order of their places ([`place_order`]). [`learn`] reads the
/// lines of every page in this order, so what it learns from them is the same in every order
/// the runs are handed in: the runs of a line stand together, and its text reads from left
/// to right. The legend is not read in this order, and the body size leaves out the legend's
/// runs (see [`learn`]).
fn placed<R: LineRun>(runs: &[R]) -> Vec<&R> {
    placed_with_ids(runs).0
}

/// [`placed`] with the number each run has among `runs`.
fn placed_with_ids<R: LineRun>(runs: &[R]) -> (Vec<&R>, Vec<u32>) {
    let mut ids: Vec<u32> = (0..runs.len() as u32).collect();
    if !in_place_order(runs) {
        // A stable sort: runs equal by their places keep the order they came in.
        ids.sort_by(|&a, &b| place_order(&runs[a as usize], &runs[b as usize]));
    }
    (ids.iter().map(|&i| &runs[i as usize]).collect(), ids)
}

/// Learns body size, heading sizes, running headers and footers and, for an untagged file
/// (`tagged` false), the footnote legends from every page of `pages`, in order. The lines of
/// a page are read with their runs in the order of their places (see [`placed`]), whatever
/// order they are handed in: the running lines and the keys of the body lines are the same
/// in every order.
///
/// The legend of a page is the exception, a known limit with its owner after 0.1.0: it is
/// read from the runs in the order handed in, which must be the reader's. The cause:
/// [`RefPlace::Above`] needs the reference number to open the next line, and only the
/// order of the runs says that it opens it (see the comment at the legend's search). The
/// rule that would lift the limit asks for no order (the number goes to the word whose
/// baseline is nearest over it); it changes the legend rules and needs its own private
/// check. The body size, and with it the heading sizes, leaves out the ink of the legend's
/// runs, so it holds in every order only as far as the legend does: where another order
/// loses the legend, its runs count as body ink.
///
/// `ends_in_amount` tells a line that ends in an amount, which is no heading.
///
/// A line at the foot of both pages of a file of two is a footer: a letter or an invoice of two
/// pages has no third page to repeat it on. A letterhead at the top of both is content: the
/// title and fields of an invoice repeat on its second page.
pub fn learn<R: LineRun>(
    pages: &[FilePage<'_, R>],
    tagged: bool,
    ends_in_amount: &dyn Fn(&str) -> bool,
) -> FileFacts {
    learn_until(pages, tagged, ends_in_amount, &|| false).0
}

/// [`learn`] with `stop` asked before each page of each of its three passes over the file
/// (the running lines, the body size and the legends, the heading sizes). Once it says
/// stop, no further page is read: the passes left learn nothing more, and the facts are
/// those of the pages read by then (running lines of the pages of the first pass, a body
/// size from the ink counted, heading sizes of the pages of the last pass). The second
/// value tells that it stopped: the facts are then no facts of the whole file, and the
/// caller says so or does not use them.
pub fn learn_until<R: LineRun>(
    pages: &[FilePage<'_, R>],
    tagged: bool,
    ends_in_amount: &dyn Fn(&str) -> bool,
    stop: &dyn Fn() -> bool,
) -> (FileFacts, bool) {
    let stopped = std::cell::Cell::new(false);
    // Asked before each page; once it said stop it is not asked again.
    let stop = || {
        if !stopped.get() && stop() {
            stopped.set(true);
        }
        stopped.get()
    };
    let mut seen: HashMap<(Band, String), Vec<Seen>> = HashMap::new();
    // Texts that stand in the middle of some page are content, wherever else they stand.
    // A bare page number is not held against: numbers fill tables too.
    let mut body: HashSet<String> = HashSet::new();
    let mut body_bytes = 0usize;
    for p in pages {
        if stop() {
            break;
        }
        let runs = placed(p.runs);
        for l in text_lines(&runs) {
            if let Some(key) = running_key(&l, p.height) {
                seen.entry(key).or_default().push((
                    l.middle(),
                    p.page,
                    l.size,
                    page_numbers(&l.text),
                ));
            } else {
                let key = furniture_key(&l.text);
                let cost = key.len() + BODY_KEY_COST;
                if body_bytes + cost <= BODY_KEY_BYTES && body.insert(key) {
                    body_bytes += cost;
                }
            }
        }
    }
    #[cfg(test)]
    BODY_KEPT.with(|k| k.set(body.len()));
    let mut running: Vec<Running> = Vec::new();
    let mut two_page: BTreeSet<String> = BTreeSet::new();
    for ((band, key), mut at) in seen {
        if key.chars().any(char::is_alphabetic) && body.contains(&key) {
            continue;
        }
        at.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Heights within Y_STEP of the lowest one of a group are one height.
        let mut i = 0;
        while i < at.len() {
            let low = at[i].0;
            let mut j = i;
            let mut pages_at = BTreeSet::new();
            while j < at.len() && at[j].0 - low <= Y_STEP * at[i].2.max(at[j].2) {
                pages_at.insert(at[j].1);
                j += 1;
            }
            let both = pages.len() == 2 && band == Band::Bottom && pages_at.len() == 2;
            if both && pages_at.len() < REPEATS {
                two_page.insert(key.clone());
            }
            if pages_at.len() >= REPEATS || both {
                running.push(Running {
                    key: key.clone(),
                    band,
                    pages: pages_at.into_iter().collect(),
                    at: [low, at[j - 1].0],
                    counts: counts_pages(at[i..j].iter().map(|s| (s.1, s.3))),
                });
            }
            i = j;
        }
    }
    running.sort_by(|a, b| {
        (a.band, &a.key)
            .cmp(&(b.band, &b.key))
            .then(a.at[0].total_cmp(&b.at[0]))
    });
    let index = RunningIndex::new(&running);
    let two_page_keys: HashSet<String> = two_page.iter().cloned().collect();
    let mut legends: BTreeMap<u32, Legend> = BTreeMap::new();
    // Ink characters by size (half points), and the sum of their exact sizes.
    let mut chars: HashMap<i32, (usize, f64)> = HashMap::new();
    let count_run = |chars: &mut HashMap<i32, (usize, f64)>, text: &str, size: f32| {
        let n = ink(text);
        let e = chars.entry(size_key(size)).or_default();
        e.0 += n;
        e.1 += n as f64 * f64::from(size);
    };
    for p in pages {
        if stop() {
            break;
        }
        let height = p.height;
        // A legend of footnotes is no body text: a dense legend in a tiny size would make
        // every line a heading. A tagged file says its notes.
        //
        // The legend is read from the runs in the order handed in: a reference number that
        // opens its line belongs to the last word of the line before (`RefPlace::Above`),
        // and only the reader's order says that it opens the line. By its place alone it
        // would follow the word left of it on its own line: a wrong pair. A known limit,
        // owner after 0.1.0 (see the doc of `learn` for the rule that would lift it).
        let legend = if tagged {
            None
        } else {
            let all = text_lines(p.runs);
            let bands = index.bands(&all, height, &[]);
            // The lines without running headers and footers, as the layout of the page sees
            // them, and where each starts among the page's runs.
            let mut lines = Vec::with_capacity(all.len());
            let mut starts = Vec::with_capacity(all.len());
            let mut at = 0;
            for (l, b) in all.into_iter().zip(&bands) {
                if b.is_none() {
                    starts.push(at);
                    at += l.runs.len();
                    lines.push(l);
                } else {
                    at += l.runs.len();
                }
            }
            legend::find_lines(&lines, &starts)
        };
        let in_legend: HashSet<u32> = legend
            .as_ref()
            .map(|f| {
                f.entries
                    .iter()
                    .flat_map(|e| e.runs.iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        // The ink of the page by size, from its lines in place order: a running line is
        // counted only as a footer of the two-page rule, a run of the legend not at all.
        let (runs, ids) = placed_with_ids(p.runs);
        let all = text_lines(&runs);
        let bands = index.bands(&all, height, &[]);
        let mut at = 0;
        for (l, b) in all.iter().zip(&bands) {
            let counted = match b {
                Some(_) => two_page_line(l, height, &two_page_keys),
                None => true,
            };
            for (k, r) in l.runs.iter().enumerate() {
                if counted && (b.is_some() || !in_legend.contains(&ids[at + k])) {
                    count_run(&mut chars, r.text(), r.size());
                }
            }
            at += l.runs.len();
        }
        if let Some(found) = legend {
            legends.insert(p.page, found);
        }
    }
    // The most ink characters wins; on a tie the smaller size. Padding is no text: a data
    // line padded into its place would count its spaces. The body size is the mean exact
    // size of the winning half point, not the half point: the ratios to it then hold for a
    // page printed at any scale.
    let body_size = chars
        .iter()
        .max_by_key(|(size, (n, _))| (*n, -**size))
        .map(|(size, (n, sum))| match *n {
            0 => *size as f32 / 2.0,
            n => (sum / n as f64) as f32,
        })
        .unwrap_or(0.0);
    let mut sizes: Vec<i32> = Vec::new();
    for p in pages {
        if stop() {
            break;
        }
        let runs = placed(p.runs);
        let all = text_lines(&runs);
        let bands = index.bands(&all, p.height, &[]);
        for (l, b) in all.iter().zip(&bands) {
            if b.is_none() && may_head(l, body_size, ends_in_amount) {
                sizes.push(size_key(l.size));
            }
        }
    }
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes.dedup();
    let facts = FileFacts {
        body_size,
        heading_sizes: sizes.into_iter().map(|s| s as f32 / 2.0).collect(),
        running,
        two_page: two_page.into_iter().collect(),
        legends,
    };
    (facts, stopped.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers of a line that may count the pages, and the lines that do: each condition
    /// moved both ways.
    #[test]
    fn a_running_line_counts_the_pages_when_its_number_follows_them() {
        assert_eq!(page_numbers("Page 7"), Some((7, None)));
        assert_eq!(page_numbers("- 7 -"), Some((7, None)));
        assert_eq!(page_numbers("Page 7 of 12"), Some((7, Some(12))));
        assert_eq!(page_numbers("7/12"), Some((7, Some(12))));
        assert_eq!(page_numbers("Page"), None);
        assert_eq!(page_numbers("7 / 12 / 3"), None);
        assert_eq!(page_numbers("Page 99999999999"), None);
        let counts =
            |lines: &[(u32, &str)]| counts_pages(lines.iter().map(|(p, t)| (*p, page_numbers(t))));
        // One step from page to page, from 1 or from another number, with pages left out.
        assert!(counts(&[(1, "Page 1"), (2, "Page 2"), (3, "Page 3")]));
        assert!(counts(&[(1, "Page 5"), (2, "Page 6")]));
        assert!(counts(&[(2, "Page 2"), (5, "Page 5"), (6, "Page 6")]));
        assert!(counts(&[(3, "Page 1"), (4, "Page 2")]));
        // One page shows no step, also with two lines on it.
        assert!(!counts(&[(1, "Page 1")]));
        assert!(!counts(&[(1, "Page 1"), (1, "Page 1")]));
        assert!(!counts(&[]));
        // The same number, a step of two, a step down, a page left out that the number does
        // not leave out.
        assert!(!counts(&[(1, "Total 120"), (2, "Total 120")]));
        assert!(!counts(&[
            (1, "Total 120"),
            (2, "Total 140"),
            (3, "Total 160")
        ]));
        assert!(!counts(&[(1, "Lot 2"), (2, "Lot 4")]));
        assert!(!counts(&[(1, "Step 8"), (2, "Step 7")]));
        assert!(!counts(&[(1, "Page 1"), (3, "Page 2")]));
        assert!(!counts(&[(1, "Page 1"), (2, "Page 2"), (3, "Page 4")]));
        // A count: the same on every line, on every line or on none, not below the number.
        assert!(counts(&[(1, "1 / 3"), (2, "2 / 3"), (3, "3 / 3")]));
        assert!(counts(&[(1, "7 / 12"), (2, "8 / 12")]));
        assert!(!counts(&[(1, "1 / 3"), (2, "2 / 4")]));
        assert!(!counts(&[(1, "1 / 3"), (2, "2")]));
        assert!(!counts(&[(1, "3 / 3"), (2, "4 / 3")]));
        assert!(!counts(&[(1, "7 / 12"), (2, "7 / 12")]));
        // A line without a number, or with three.
        assert!(!counts(&[(1, "Page 1"), (2, "Page")]));
        assert!(!counts(&[(1, "1 / 3 / 9"), (2, "2 / 3 / 9")]));
    }
    use crate::input::{FontKind, Shift, TextRun};
    use crate::line::text_line;

    fn run(text: &str, line: u32, y: f32, size: f32) -> TextRun {
        TextRun {
            text: text.into(),
            x: 72.0,
            y,
            w: 100.0,
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

    fn page(page: u32, texts: Vec<TextRun>) -> PageIn {
        PageIn {
            page,
            size: [595.0, 842.0],
            turn: 0,
            texts,
            rules: Vec::new(),
            boxes: Vec::new(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given: Vec::new(),
        }
    }

    fn no_amount(_: &str) -> bool {
        false
    }

    fn learned(pages: &[PageIn], tagged: bool) -> FileFacts {
        let pages: Vec<FilePage<'_, TextRun>> = pages.iter().map(FilePage::of).collect();
        learn(&pages, tagged, &no_amount)
    }

    /// The learner asks `stop` before each page of each of its three passes, and
    /// never again once it said stop. With no stop the facts are those of [`learn`]; a stop
    /// in the first pass leaves the running lines of the pages read by then, and the second
    /// value says that it stopped.
    #[test]
    fn learning_asks_to_stop_before_each_page_of_each_pass() {
        let pages: Vec<PageIn> = (1..=4)
            .map(|n| {
                page(
                    n,
                    vec![
                        run("Muster GmbH", 0, 800.0, 8.0),
                        run("Erika Mustermann wohnt in Musterstadt", 1, 500.0, 10.0),
                    ],
                )
            })
            .collect();
        let file: Vec<FilePage<'_, TextRun>> = pages.iter().map(FilePage::of).collect();
        let asked = std::cell::Cell::new(0u32);
        let never = || {
            asked.set(asked.get() + 1);
            false
        };
        let (facts, stopped) = learn_until(&file, false, &no_amount, &never);
        assert!(!stopped);
        assert_eq!(asked.get(), 12, "four pages in three passes");
        assert_eq!(facts, learn(&file, false, &no_amount));
        assert_eq!(facts.running.len(), 1, "{:?}", facts.running);
        assert_eq!(facts.running[0].pages, [1, 2, 3, 4]);
        // Stop before the fourth page: the head line runs on three pages, and no page is
        // asked for after the stop.
        asked.set(0);
        let at_four = || {
            asked.set(asked.get() + 1);
            asked.get() == 4
        };
        let (facts, stopped) = learn_until(&file, false, &no_amount, &at_four);
        assert!(stopped);
        assert_eq!(asked.get(), 4);
        assert_eq!(facts.running[0].pages, [1, 2, 3]);
        // Stop before the third page: two pages are too few for a running line.
        asked.set(0);
        let at_three = || {
            asked.set(asked.get() + 1);
            asked.get() == 3
        };
        let (facts, stopped) = learn_until(&file, false, &no_amount, &at_three);
        assert!(stopped);
        assert!(facts.running.is_empty(), "{:?}", facts.running);
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
    fn bold_leads_a_heading_line() {
        let bold = |text: &str| TextRun {
            bold: true,
            ..run(text, 0, 700.0, 9.0)
        };
        let plain = |text: &str| run(text, 0, 700.0, 9.0);
        let all = [bold("Rechnung")];
        assert!(bold_led(&text_line(&all)));
        let section = [bold("SECTION 1"), plain(" (TO BE COMPLETED BY PAYEE)")];
        assert!(bold_led(&text_line(&section)));
        let label = [bold("Kundennummer:"), plain(" 00000")];
        assert!(!bold_led(&text_line(&label)));
        let value = [bold("Rechnungsnummer"), plain(" RE-2026-001")];
        assert!(!bold_led(&text_line(&value)));
        let plain_first = [plain("Seite"), bold(" 1")];
        assert!(!bold_led(&text_line(&plain_first)));
    }

    #[test]
    fn an_empty_file_has_no_facts() {
        assert_eq!(learned(&[], false), FileFacts::default());
        assert_eq!(learned(&[], true), FileFacts::default());
        assert_eq!(learned(&[page(1, Vec::new())], false), FileFacts::default());
    }

    /// A tagged file learns its body and heading sizes as an untagged one does; only the
    /// legends are left to its tags.
    #[test]
    fn a_tagged_file_has_heading_sizes() {
        let mut runs = vec![run("Kapitel Eins", 0, 650.0, 18.0)];
        runs.extend((1..=4).map(|k| {
            run(
                "die Firma liefert die bestellte Ware",
                k,
                600.0 - 14.0 * k as f32,
                10.0,
            )
        }));
        let facts = learned(&[page(1, runs)], true);
        assert!((facts.body_size - 10.0).abs() < 1e-4, "{}", facts.body_size);
        assert_eq!(facts.heading_sizes, [18.0]);
        assert!(facts.legends.is_empty());
    }

    #[test]
    fn kept_body_lines_are_bounded() {
        // Distinct mid-page lines of 13 bytes each cost 77 bytes with the set: 1,000 bytes
        // keep 12 of them.
        let letter = |k: u32| char::from(b'a' + (k % 26) as u8);
        let runs: Vec<TextRun> = (0..200)
            .map(|k| {
                let word: String = [letter(k / 676), letter(k / 26), letter(k)]
                    .iter()
                    .collect();
                run(&format!("body line {word}"), k, 500.0, 10.0)
            })
            .collect();
        learned(&[page(1, runs)], false);
        assert_eq!(BODY_KEPT.with(|k| k.get()), 1_000 / (13 + BODY_KEY_COST));
    }

    /// The legend leaves the body size only in an untagged file, where the layout takes it
    /// too; a tagged file counts it, as the text sizes of its pages do.
    #[test]
    fn only_an_untagged_legend_leaves_the_body_size() {
        let mut runs = vec![run("Wert1) und Wert2) gemeldet", 0, 700.0, 9.0)];
        runs.extend((1..=2).map(|k| {
            run(
                &format!("{k}) Muster der Erklaerung zu diesem Wert mit vielen Worten"),
                k,
                140.0 - 8.0 * k as f32,
                6.0,
            )
        }));
        let pages = [page(1, runs)];
        let untagged = learned(&pages, false);
        assert_eq!(untagged.body_size, 9.0);
        assert_eq!(
            untagged.legends.get(&1).map(|l| l.lines.clone()),
            Some(vec![1, 2])
        );
        let tagged = learned(&pages, true);
        assert_eq!(tagged.body_size, 6.0);
        assert!(tagged.legends.is_empty());
    }

    /// The body size is the exact size of the body text, not its half point: a letter printed
    /// at 0.675 sets its body of 7 points at 4.725, which a half point would make 4.5, and a
    /// name of 6.75 points (10 at full size, under 1.5 times the body there) would then be
    /// 1.5 times the body.
    #[test]
    fn the_body_size_is_exact() {
        let runs: Vec<TextRun> = (0..4)
            .map(|k| {
                run(
                    "die Firma liefert die bestellte Ware",
                    k,
                    700.0 - 6.0 * k as f32,
                    4.725,
                )
            })
            .collect();
        let facts = learned(&[page(1, runs)], false);
        assert!(
            (facts.body_size - 4.725).abs() < 1e-4,
            "{}",
            facts.body_size
        );
        assert!(6.75 < HEADING_PLAIN_RATIO * facts.body_size);
    }

    /// A page of a file of `n` pages at `scale`: a running header, a page number in the
    /// footer, a heading in 18 pt and one in 16 pt, body lines in 10 pt with a raised
    /// reference, and a legend of one footnote in 7 pt.
    fn scaled_page(k: u32, scale: f32) -> PageIn {
        let s = |v: f32| v * scale;
        let mut texts = Vec::new();
        let mut at = |text: &str, x: f32, y: f32, size: f32| {
            let line = texts.len() as u32;
            texts.push(TextRun {
                x: s(x),
                w: s(text.len() as f32 * size * 0.5),
                ..run(text, line, s(y), s(size))
            });
        };
        at("Muster GmbH Jahresbericht", 72.0, 800.0, 9.0);
        at("Kapitel Eins", 72.0, 650.0, 18.0);
        at("Abschnitt Zwei", 72.0, 620.0, 16.0);
        for i in 0..6 {
            at(
                "die Firma liefert die bestellte Ware an Erika Mustermann",
                72.0,
                590.0 - 14.0 * i as f32,
                10.0,
            );
        }
        // A raised reference after `Mustermann`, on the line of the last body line.
        let last = texts.len() as u32 - 1;
        texts.push(TextRun {
            x: s(72.0 + 56.0 * 5.0),
            w: s(3.0),
            line: last,
            ..run("1", last, s(520.0 + 3.0), s(6.0))
        });
        let mut at = |text: &str, x: f32, y: f32, size: f32| {
            let line = texts.len() as u32;
            texts.push(TextRun {
                x: s(x),
                w: s(text.len() as f32 * size * 0.5),
                ..run(text, line, s(y), s(size))
            });
        };
        at("1 Muster der Erklaerung", 72.0, 200.0, 7.0);
        at(&format!("Seite {k}"), 280.0, 40.0, 9.0);
        PageIn {
            size: [s(595.0), s(842.0)],
            ..page(k, texts)
        }
    }

    /// The same file at 0.75x and 1.5x: the facts are equal but for sizes and places, which
    /// scale.
    #[test]
    fn facts_are_the_same_at_every_scale() {
        let at = |scale: f32| {
            let pages: Vec<PageIn> = (1..=3).map(|k| scaled_page(k, scale)).collect();
            learned(&pages, false)
        };
        let one = at(1.0);
        assert!((one.body_size - 10.0).abs() < 1e-4, "{}", one.body_size);
        assert_eq!(one.heading_sizes, [18.0, 16.0]);
        let keys: Vec<(&str, Band, &[u32])> = one
            .running
            .iter()
            .map(|r| (r.key.as_str(), r.band, r.pages.as_slice()))
            .collect();
        assert_eq!(
            keys,
            [
                ("Muster GmbH Jahresbericht", Band::Top, &[1, 2, 3][..]),
                ("Seite #", Band::Bottom, &[1, 2, 3][..]),
            ]
        );
        assert!(one.two_page.is_empty());
        assert_eq!(one.legends.keys().copied().collect::<Vec<_>>(), [1, 2, 3]);
        for scale in [0.75, 1.5] {
            let other = at(scale);
            let close = |a: f32, b: f32| (a * scale - b).abs() < 1e-3 * scale.max(1.0);
            assert!(close(one.body_size, other.body_size), "{scale}");
            assert_eq!(one.heading_sizes.len(), other.heading_sizes.len());
            for (a, b) in one.heading_sizes.iter().zip(&other.heading_sizes) {
                assert!(close(*a, *b), "{scale}: {a} {b}");
            }
            assert_eq!(one.running.len(), other.running.len());
            for (a, b) in one.running.iter().zip(&other.running) {
                assert_eq!((&a.key, a.band, &a.pages), (&b.key, b.band, &b.pages));
                assert!(
                    close(a.at[0], b.at[0]) && close(a.at[1], b.at[1]),
                    "{scale}"
                );
            }
            assert_eq!(one.two_page, other.two_page);
            assert_eq!(one.legends.len(), other.legends.len());
            for (p, a) in &one.legends {
                let b = &other.legends[p];
                assert_eq!(a.lines, b.lines);
                assert_eq!(a.raised, b.raised);
                assert_eq!(a.entries.len(), b.entries.len());
                for (x, y) in a.entries.iter().zip(&b.entries) {
                    assert_eq!((x.mark, x.cut, &x.runs), (y.mark, y.cut, &y.runs));
                    assert!(close(x.x, y.x) && close(x.top, y.top) && close(x.h, y.h));
                }
            }
        }
    }

    /// A line at the foot of both pages of a file of two is a footer, a letterhead at the top
    /// of both is content.
    #[test]
    fn a_footer_on_both_pages_of_two_is_running() {
        let pages: Vec<PageIn> = (1..=2)
            .map(|k| {
                page(
                    k,
                    vec![
                        run("Muster GmbH Rechnung", 0, 800.0, 13.0),
                        run("die Ware", 1, 600.0, 10.0),
                        run("Bank DE00 0000 0000 0000 0000 00", 2, 40.0, 8.0),
                    ],
                )
            })
            .collect();
        let facts = learned(&pages, false);
        assert_eq!(facts.two_page, ["Bank DE## #### #### #### #### ##"]);
        let keys: Vec<(&str, Band)> = facts
            .running
            .iter()
            .map(|r| (r.key.as_str(), r.band))
            .collect();
        assert_eq!(keys, [("Bank DE## #### #### #### #### ##", Band::Bottom)]);
        // Its ink still counts for the body size (8 pt, not 10 pt): the plain 13 pt title is
        // a heading, as it is 1.5 times that body.
        assert!((facts.body_size - 8.0).abs() < 1e-4, "{}", facts.body_size);
        assert_eq!(facts.heading_sizes, [13.0]);
    }

    /// The lines of a file are learned with their runs
    /// in the order of their places. A footer handed over in two runs, the right one first
    /// (as a reader hands over two pieces of one row), has the key its places give, with
    /// its runs in every order and with the runs of the page shuffled; and a line with its
    /// runs out of that order is found again by that key.
    #[test]
    fn a_running_line_is_learned_and_found_by_the_places_of_its_runs() {
        let at = |text: &str, line: u32, x: f32, y: f32| TextRun {
            x,
            w: 60.0,
            ..run(text, line, y, 10.0)
        };
        let texts = |order: [usize; 4]| -> Vec<TextRun> {
            let all = [
                at("the goods of the north works", 0, 72.0, 600.0),
                at("and of the south works", 1, 72.0, 588.0),
                at("Sample works", 2, 72.0, 40.0),
                at(" yearly report", 2, 140.0, 40.0),
            ];
            order.iter().map(|&i| all[i].clone()).collect()
        };
        let file =
            |order: [usize; 4]| -> Vec<PageIn> { (1..=3).map(|k| page(k, texts(order))).collect() };
        let in_place = learned(&file([0, 1, 2, 3]), false);
        let keys: Vec<&str> = in_place.running.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["Sample works yearly report"]);
        // The right run first, and the runs of the page in an order that parts the line.
        for order in [[0, 1, 3, 2], [3, 0, 2, 1]] {
            assert_eq!(learned(&file(order), false), in_place, "{order:?}");
        }
        // The footer with its runs the other way round is the running line of the page.
        let flow = texts([0, 1, 3, 2]);
        let lines = text_lines(&flow);
        assert_eq!(lines[2].text, "yearly report Sample works");
        let bands = RunningIndex::new(&in_place.running).bands(&lines, 842.0, &[]);
        assert_eq!(bands, [None, None, Some(Band::Bottom)]);
    }

    /// A header repeated at two heights is two groups, each with its own range.
    #[test]
    fn running_lines_keep_the_height_of_each_group() {
        let pages: Vec<PageIn> = (1..=6)
            .map(|k| {
                let y = if k <= 3 { 800.0 } else { 780.0 };
                page(
                    k,
                    vec![
                        run("Muster GmbH", 0, y + (k % 2) as f32, 9.0),
                        run("die Firma liefert die bestellte Ware", 1, 600.0, 10.0),
                    ],
                )
            })
            .collect();
        let facts = learned(&pages, false);
        let groups: Vec<([f32; 2], &[u32])> = facts
            .running
            .iter()
            .map(|r| (r.at, r.pages.as_slice()))
            .collect();
        assert_eq!(
            groups,
            [
                ([784.5, 785.5], &[4, 5, 6][..]),
                ([804.5, 805.5], &[1, 2, 3][..]),
            ]
        );
        // A line of the key between the two ranges is no running header.
        let index = RunningIndex::new(&facts.running);
        let between = [run("Muster GmbH", 0, 790.0, 9.0)];
        assert_eq!(index.bands(&text_lines(&between), 842.0, &[]), [None]);
        let inside = [run("Muster GmbH", 0, 800.0, 9.0)];
        assert_eq!(
            index.bands(&text_lines(&inside), 842.0, &[]),
            [Some(Band::Top)]
        );
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
        assert_eq!(text_sizes(&runs), [6.0, 9.5]);
        // 400 ink characters make a text size also under a quarter of the page.
        let runs = [
            run(&"a".repeat(2000), 0, 700.0, 6.0),
            run(&"b".repeat(400), 1, 680.0, 9.5),
        ];
        assert_eq!(text_sizes(&runs), [6.0, 9.5]);
        // A cover page: a title of one line sets half of the little text there is, and is no
        // text size.
        let runs = [
            run(&"a".repeat(30), 0, 700.0, 24.0),
            run(&"b".repeat(10), 1, 600.0, 10.0),
            run(&"b".repeat(10), 2, 590.0, 10.0),
            run(&"b".repeat(10), 3, 580.0, 10.0),
        ];
        assert_eq!(text_sizes(&runs), [10.0]);
    }

    /// A heading whose size the file pass did not see as a
    /// heading size (its line there held a smaller cell beside it) gets the level of its size
    /// among the heading sizes, not the deepest level: a title larger than all but one heading
    /// size is level 2.
    #[test]
    fn a_heading_size_not_seen_takes_its_place_among_the_sizes() {
        let levels = Levels {
            body_size: 7.0,
            heading_sizes: &[20.0, 12.0],
            text_sizes: &[],
            ends_in_amount: &no_amount,
        };
        let level = |size: f32| levels.level(&text_line(&[run("Rechnung", 0, 690.0, size)]));
        assert_eq!(level(14.0), Some(2));
        assert_eq!(level(24.0), Some(1));
        // A size seen keeps its place; one smaller than all is the deepest level.
        assert_eq!(level(12.0), Some(2));
        assert_eq!(level(11.0), Some(3));
    }

    /// A line set in a text size of its page, or under a heading step over the largest of
    /// them, is no heading there.
    #[test]
    fn page_text_sizes_keep_a_line_body_text() {
        let levels = |text_sizes: &'static [f32]| Levels {
            body_size: 7.0,
            heading_sizes: &[20.0, 12.0],
            text_sizes,
            ends_in_amount: &no_amount,
        };
        let line = [run("Rechnung", 0, 690.0, 12.0)];
        let line = text_line(&line);
        assert_eq!(levels(&[7.0]).level(&line), Some(2));
        assert_eq!(levels(&[7.0, 12.0]).level(&line), None);
        assert_eq!(levels(&[7.0, 10.5]).level(&line), None);
        assert_eq!(levels(&[7.0, 10.0]).level(&line), Some(2));
    }
}
