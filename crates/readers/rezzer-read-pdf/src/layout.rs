// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! From positioned text runs to blocks: running headers and footers, headings, paragraphs,
//! lists, and the tables `tables.rs` finds, placed in reading order.
//!
//! A tagged file (`/MarkInfo /Marked true`) gets its roles from the structure tree. An
//! untagged file gets headings from the font size and lists from their markers. Lines come in
//! reading order; a paragraph ends at a vertical gap over 1.5 times the line height, at a
//! change of font size and where the text moves up (the next column).

use std::cell::Cell as Counter;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::rc::Rc;

use pdfparts::StructTree;
use rezzer_limits::{Budget, LimitHit};
use rezzer_model::{Anchor, Block, Cell, Inline, LossReason, SectionKind, Severity, StatusColor};

use crate::tables::{self, Planned, PlannedCell, Rule, Tables, Unsure};

mod footnotes;
mod head;
mod shape;

pub(crate) use head::{Dates, PageDate, TITLE_FROM, document_date, page_dates};
pub(crate) use shape::{
    ColumnLine, Columns, address_block, address_columns, address_lines, address_shaped,
    page_text_sizes, page_text_sizes_of,
};
use shape::{bold_led, data_only, ends_in_amount, listed, padded_mark};

/// A piece of text in one style on one line. The text parser and the fallback parser fill it,
/// in the order and with the line numbers and word spaces [`crate::text::PageItems`] states.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Run {
    pub page: u32,
    /// The text. A run that starts a new word inside its line begins with a space.
    pub text: String,
    /// Left edge and baseline in points in the visible page box, `y` growing upward. `w` is
    /// the advance, `h` the rendered font size.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    /// The family of the font of its first glyph (see [`crate::text::family_of`]), 0 when not
    /// known: a data font told apart from a title font of the same size and weight.
    pub family: u32,
    /// The marked content id the run is shown in, for the structure tree, kept one up (see
    /// [`Run::mcid`]): with no room taken for its absence the run stays 80 bytes.
    pub mcid: Option<NonZeroU32>,
    /// The target of a URI link whose box holds the middle of the run, shared by all the
    /// runs of the link.
    pub uri: Option<Rc<UriTarget>>,
    /// The number of the run's line on its page, in reading order. The runs of one line are
    /// next to each other.
    pub line: u32,
    /// The run reads in another direction than the page's text (margin notes, stamps,
    /// vertical writing); its `x` and `y` are where it starts.
    pub off_axis: bool,
    /// The run is a reference to a footnote of its page: its text is the footnote's label.
    pub note: bool,
    /// Where the white space between its words lies, from the glyphs: what a column rule
    /// that crosses the run may cut it at. Empty when the run has one word, reads in another
    /// direction or from right to left, or its text was not laid out from glyphs. They are
    /// kept up to date until tables are built; text cut off later (a list marker) leaves them
    /// as they were.
    pub gaps: Gaps,
}

/// The gaps of a run behind one pointer, none for a run without them: a run is no larger for
/// them than a pointer.
// A box of a list is one pointer where a boxed slice is two: the run stays 80 bytes.
#[allow(clippy::box_collection)]
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Gaps(Option<Box<Vec<Gap>>>);

impl Gaps {
    pub(crate) fn new(list: Vec<Gap>) -> Gaps {
        Gaps((!list.is_empty()).then(|| Box::new(list)))
    }

    /// The memory the list takes besides the run.
    pub(crate) fn bytes(&self) -> usize {
        self.0.as_ref().map_or(0, |l| {
            std::mem::size_of::<Vec<Gap>>() + l.capacity() * std::mem::size_of::<Gap>()
        })
    }
}

impl std::ops::Deref for Gaps {
    type Target = [Gap];
    fn deref(&self) -> &[Gap] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }
}

impl std::ops::DerefMut for Gaps {
    fn deref_mut(&mut self) -> &mut [Gap] {
        self.0.as_deref_mut().map_or(&mut [], Vec::as_mut_slice)
    }
}

/// The white space between two words of a run: after its first `words` words (as
/// `split_whitespace` counts them, which separators put in later do not change), from the
/// end of the ink before it to the start of the ink after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Gap {
    pub words: u32,
    pub from: f32,
    pub to: f32,
}

// The kept text of a file is bounded by the size of its runs: a field more costs every page.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<Run>() == 80);

/// A marked content id as a run keeps it (see [`Run::mcid`]); an id of `u32::MAX` is none.
pub(crate) fn kept_mcid(mcid: Option<u32>) -> Option<NonZeroU32> {
    mcid.and_then(|m| NonZeroU32::new(m.wrapping_add(1)))
}

impl Run {
    /// The marked content id the run is shown in.
    pub(crate) fn mcid(&self) -> Option<u32> {
        self.mcid.map(|m| m.get() - 1)
    }

    /// The run cut at its gap `k`: the words before it, and the words after it as a run that
    /// starts a new word, each with its own extent and gaps. `None` when the text no longer
    /// has the words the gap was taken after.
    pub(crate) fn cut_at(&self, k: usize) -> Option<(Run, Run)> {
        let gap = *self.gaps.get(k)?;
        let mut seen = 0u32;
        let mut in_word = false;
        let mut at = None;
        for (i, c) in self.text.char_indices() {
            if c.is_whitespace() {
                in_word = false;
            } else if !in_word {
                if seen == gap.words {
                    at = Some(i);
                    break;
                }
                seen += 1;
                in_word = true;
            }
        }
        let at = at.filter(|_| gap.words > 0)?;
        let end = self.x + self.w;
        let mut before = self.clone();
        before.text = self.text[..at].trim_end().to_string();
        before.w = (gap.from - self.x).max(0.0);
        before.gaps = Gaps::new(self.gaps[..k].to_vec());
        let mut after = self.clone();
        after.text = format!(" {}", &self.text[at..]);
        after.x = gap.to;
        after.w = (end - gap.to).max(0.0);
        after.gaps = Gaps::new(
            self.gaps[k + 1..]
                .iter()
                .map(|g| Gap {
                    words: g.words.saturating_sub(gap.words),
                    ..*g
                })
                .collect(),
        );
        Some((before, after))
    }
}

/// The target of a URI link, with the most bytes its copy can be written as. The size is
/// computed once, when the link is read, and shared with every run of the link.
#[derive(Debug, PartialEq)]
pub(crate) struct UriTarget {
    pub uri: Box<str>,
    pub written: u64,
}

impl UriTarget {
    pub(crate) fn new(uri: &str) -> Rc<UriTarget> {
        Rc::new(UriTarget {
            uri: uri.into(),
            written: rezzer_model::link_target_written_len(uri),
        })
    }
}

/// Share of the page height at the top and the bottom where running headers and footers sit.
/// pdflatex sets page numbers about 11 % above the bottom edge of an A4 page.
const BAND: f32 = 0.18;

/// Pages a header or footer line must repeat on. In a file of two pages, a footer line repeats
/// on both (see [`Style::learn`]).
const REPEATS: usize = 3;

/// A line at least this many times the body size is a heading in an untagged file.
const HEADING_RATIO: f32 = 1.2;

/// A line smaller than this many times the body size is a heading only when it is led by
/// bold ink (see [`bold_led`]): data lines and labels set a little larger than the body are
/// no headings.
const HEADING_PLAIN_RATIO: f32 = 1.5;

/// A heading line is shorter than this many characters.
const HEADING_CHARS: usize = 120;

/// Deepest heading level the font size rule gives.
const SIZE_LEVELS: usize = 3;

/// A new paragraph starts at a gap over this many times the line height.
const GAP_RATIO: f32 = 1.5;

/// Font sizes closer than this are the same size.
const SIZE_SLACK: f32 = 0.5;

/// Memory the mid-page line texts the style pass keeps may take, to tell running headers from
/// content: the text and about 64 bytes of set overhead per line. Past it, no more are kept:
/// a header text that also stands mid-page later in a huge file may then keep its header
/// role.
#[cfg(not(test))]
const BODY_KEY_BYTES: usize = 32 << 20;
#[cfg(test)]
const BODY_KEY_BYTES: usize = 1_000;

/// Set overhead of one kept line text.
const BODY_KEY_COST: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Band {
    Top,
    Bottom,
}

/// One page as the style pass sees it.
pub(crate) struct PageText<'a> {
    pub page: u32,
    /// The height of the page in the frame of its runs; `None` when that frame was turned.
    pub height: Option<f32>,
    pub runs: &'a [Run],
}

/// What the layout learns from the whole file before it lays out a page.
#[derive(Debug, Default)]
pub(crate) struct Style {
    /// The most common font size, by ink character count, of the text outside running
    /// headers and footers.
    pub body_size: f32,
    /// Distinct heading sizes of an untagged file, largest first.
    heading_sizes: Vec<f32>,
    /// Lines that repeat at the top or bottom of at least [`REPEATS`] pages at the same
    /// height, or at the bottom of both pages of a file of two, with digits replaced.
    furniture: HashMap<Furniture, Vec<(f32, f32)>>,
    /// The footer lines among them that are furniture by the two-page rule only. They are
    /// running footers by their role, but their ink still counts for the body size and the
    /// text sizes of a page, as it did before the rule: an invoice of two pages keeps the body
    /// size and so the headings it had.
    two_page: HashSet<Furniture>,
    /// The footnote legend of each untagged page that has one, by the page's runs.
    legends: HashMap<u32, footnotes::Found>,
}

/// One visual line.
struct Line<'a> {
    runs: &'a [Run],
    /// The text without leading and trailing white space.
    text: String,
    size: f32,
    x: f32,
    y: f32,
    h: f32,
}

impl Line<'_> {
    /// The vertical middle of the line.
    fn middle(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

/// The lines of `runs`, in order.
fn lines(runs: &[Run]) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < runs.len() {
        let mut end = start + 1;
        while end < runs.len()
            && runs[end].line == runs[start].line
            && runs[end].page == runs[start].page
        {
            end += 1;
        }
        out.push(line(&runs[start..end]));
        start = end;
    }
    out
}

fn line(runs: &[Run]) -> Line<'_> {
    // A run printed over the run before it, or touching it in another size or weight, is
    // another text (see [`apart`]).
    let mut text = String::new();
    let mut ink_end = f32::MIN;
    let mut before: Option<&Run> = None;
    for r in runs {
        if apart(before, r, ink_end)
            && !r.text.starts_with(char::is_whitespace)
            && !text.ends_with(char::is_whitespace)
        {
            text.push(' ');
        }
        text.push_str(&r.text);
        if !r.text.trim().is_empty() {
            ink_end = ink_end.max(r.x + r.w);
            before = Some(r);
        }
    }
    // The run with the most characters sets the size and the baseline.
    let main = runs
        .iter()
        .max_by_key(|r| r.text.chars().count())
        .unwrap_or(&runs[0]);
    Line {
        runs,
        text: text.trim().to_string(),
        size: main.size,
        x: runs[0].x,
        y: main.y,
        h: runs.iter().map(|r| r.h).fold(0.0, f32::max),
    }
}

/// The text of a line with every digit replaced, white space collapsed.
fn furniture_key(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect();
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn band(line: &Line<'_>, height: Option<f32>) -> Option<Band> {
    let height = height.filter(|h| *h > 0.0)?;
    let middle = line.middle();
    if middle >= height * (1.0 - BAND) {
        Some(Band::Top)
    } else if middle <= height * BAND {
        Some(Band::Bottom)
    } else {
        None
    }
}

/// A running header or footer line: its band and its text with digits replaced. The style
/// keeps, for each, the heights (lowest and highest middle) it repeats at.
type Furniture = (Band, String);

/// Lines of one header or footer sit at the same height on every page, within this many
/// points.
const Y_STEP: f32 = 4.0;

fn furniture_key_of(line: &Line<'_>, height: Option<f32>) -> Option<Furniture> {
    let b = band(line, height)?;
    let key = furniture_key(&line.text);
    (!key.is_empty()).then_some((b, key))
}

/// The ink characters of `text`: white space is not counted.
fn ink(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

/// Rounds a font size to half points.
fn size_key(size: f32) -> i32 {
    (size * 2.0).round() as i32
}

impl Style {
    /// [`Style::learn`] for an untagged file.
    #[cfg(test)]
    pub(crate) fn new(pages: &[PageText<'_>]) -> Style {
        Style::learn(pages, false)
    }

    /// Learns body size, heading sizes, running headers and footers and, for an untagged
    /// file (`tagged` false), the footnote legends from every page. A line at the foot of both
    /// pages of a file of two is a footer: a letter or an invoice of two pages has no third
    /// page to repeat it on. A letterhead at the top of both is content: the title and fields
    /// of an invoice repeat on its second page.
    pub(crate) fn learn(pages: &[PageText<'_>], tagged: bool) -> Style {
        let mut seen: HashMap<Furniture, Vec<(f32, u32)>> = HashMap::new();
        // Texts that stand in the middle of some page are content, wherever else they stand.
        // A bare page number is not held against: numbers fill tables too.
        let mut body: HashSet<String> = HashSet::new();
        let mut body_bytes = 0usize;
        for p in pages {
            for l in lines(p.runs) {
                if let Some(key) = furniture_key_of(&l, p.height) {
                    seen.entry(key).or_default().push((l.middle(), p.page));
                } else if p.height.is_some() {
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
        let mut furniture: HashMap<Furniture, Vec<(f32, f32)>> = HashMap::new();
        let mut two_page: HashSet<Furniture> = HashSet::new();
        for (key, mut at) in seen {
            if key.1.chars().any(char::is_alphabetic) && body.contains(&key.1) {
                continue;
            }
            at.sort_by(|a, b| a.0.total_cmp(&b.0));
            // Heights within Y_STEP of the lowest one of a group are one height.
            let mut i = 0;
            while i < at.len() {
                let low = at[i].0;
                let mut j = i;
                let mut pages_at = HashSet::new();
                while j < at.len() && at[j].0 - low <= Y_STEP {
                    pages_at.insert(at[j].1);
                    j += 1;
                }
                let both = pages.len() == 2 && key.0 == Band::Bottom && pages_at.len() == 2;
                if both && pages_at.len() < REPEATS {
                    two_page.insert(key.clone());
                }
                if pages_at.len() >= REPEATS || both {
                    furniture
                        .entry(key.clone())
                        .or_default()
                        .push((low, at[j - 1].0));
                }
                i = j;
            }
        }
        let mut style = Style {
            furniture,
            two_page,
            ..Style::default()
        };
        // Ink characters by size (half points), and the sum of their exact sizes.
        let mut chars: HashMap<i32, (usize, f64)> = HashMap::new();
        let count = |chars: &mut HashMap<i32, (usize, f64)>, r: &Run| {
            let n = ink(&r.text);
            let e = chars.entry(size_key(r.size)).or_default();
            e.0 += n;
            e.1 += n as f64 * f64::from(r.size);
        };
        for p in pages {
            let all = lines(p.runs);
            let bands = style.furniture(&all, p.height, &[]);
            for (l, b) in all.iter().zip(&bands) {
                let counted = b.is_some()
                    && furniture_key_of(l, p.height).is_some_and(|k| style.two_page.contains(&k));
                if counted {
                    for r in l.runs {
                        count(&mut chars, r);
                    }
                }
            }
            // The lines without running headers and footers, as the layout of the page sees
            // them, and where each starts among the page's runs.
            let mut body = Vec::with_capacity(all.len());
            let mut starts = Vec::with_capacity(all.len());
            let mut at = 0;
            for (l, b) in all.into_iter().zip(&bands) {
                if b.is_none() {
                    starts.push(at);
                }
                at += l.runs.len();
                if b.is_none() {
                    body.push(l);
                }
            }
            // A legend of footnotes is no body text: a dense legend in a tiny size would make
            // every line a heading. A tagged file says its notes.
            let legend = if tagged {
                None
            } else {
                footnotes::find_lines(&body, &starts)
            };
            let in_legend: HashSet<usize> = legend
                .as_ref()
                .map(|f| f.legend_runs().collect())
                .unwrap_or_default();
            for (l, first) in body.iter().zip(&starts) {
                for (k, r) in l.runs.iter().enumerate() {
                    if !in_legend.contains(&(first + k)) {
                        count(&mut chars, r);
                    }
                }
            }
            if let Some(found) = legend {
                style.legends.insert(p.page, found);
            }
        }
        // The most ink characters wins; on a tie the smaller size. Padding is no text: a data
        // line padded into its place would count its spaces. The body size is the mean exact
        // size of the winning half point, not the half point: the ratios to it then hold for a
        // page printed at any scale.
        style.body_size = chars
            .iter()
            .max_by_key(|(size, (n, _))| (*n, -**size))
            .map(|(size, (n, sum))| match *n {
                0 => *size as f32 / 2.0,
                n => (sum / n as f64) as f32,
            })
            .unwrap_or(0.0);
        let mut sizes: Vec<i32> = Vec::new();
        for p in pages {
            let all = lines(p.runs);
            let bands = style.furniture(&all, p.height, &[]);
            for (l, b) in all.iter().zip(&bands) {
                if b.is_none() && style.may_head(l) {
                    sizes.push(size_key(l.size));
                }
            }
        }
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        sizes.dedup();
        style.heading_sizes = sizes.into_iter().map(|s| s as f32 / 2.0).collect();
        style
    }

    /// The band of each line that is a running header or footer on its page. A header stands
    /// above all text of the page and a footer below it: a repeated line inside the text
    /// stays text. Lines of white space only are no text of the page.
    /// A line marked in `content` is never a header or footer; an empty `content` marks none.
    fn furniture(
        &self,
        lines: &[Line<'_>],
        height: Option<f32>,
        content: &[bool],
    ) -> Vec<Option<Band>> {
        let found: Vec<Option<Band>> = lines
            .iter()
            .enumerate()
            .map(|(k, l)| {
                if content.get(k).copied().unwrap_or(false) {
                    return None;
                }
                let key = furniture_key_of(l, height)?;
                let m = l.middle();
                self.furniture
                    .get(&key)?
                    .iter()
                    .any(|(low, high)| m >= *low && m <= *high)
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

    fn is_heading_size(&self, line: &Line<'_>) -> bool {
        self.body_size > 0.0
            && line.size >= self.body_size * HEADING_RATIO
            && line.text.chars().count() < HEADING_CHARS
            && line.text.chars().any(char::is_alphanumeric)
    }

    /// Whether an untagged line may be a heading by its own text and style: large enough,
    /// short, not ending in an amount, and, when it is smaller than [`HEADING_PLAIN_RATIO`]
    /// times the body size, led by bold ink (see [`bold_led`]) and not only data.
    fn may_head(&self, line: &Line<'_>) -> bool {
        // A number set large (a form or file number) stays a heading.
        let large = line.size >= HEADING_PLAIN_RATIO * self.body_size;
        self.is_heading_size(line)
            && !ends_in_amount(&line.text)
            && (large || (!data_only(&line.text) && bold_led(line)))
    }

    /// The heading level of an untagged line, `None` for body text.
    fn heading_level(&self, line: &Line<'_>) -> Option<u8> {
        if !self.may_head(line) {
            return None;
        }
        // A size the style pass did not see as a heading size (its line there held a smaller
        // cell beside it, which a table took later) takes its place among the sizes.
        let key = size_key(line.size);
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

/// What the layout of one page needs besides its runs.
pub(crate) struct Context<'a> {
    pub style: &'a Style,
    /// The structure tree, only for a tagged file.
    pub tree: Option<&'a StructTree>,
    /// The element that holds a marked content id of the page.
    pub roles: &'a dyn Fn(u32) -> Option<usize>,
    pub budget: &'a Budget,
    /// Tagged tables already read, shared by all pages of the file.
    pub tables: &'a Tables,
    /// A limit cut the page's text, so cells without text may only be unread.
    pub in_part: bool,
    /// The bytes the link targets in the model may take, shared by all pages of the file.
    pub links: &'a LinkRoom,
}

/// The bytes the copies of link targets in the model may still take. Each link in the model
/// holds its own copy of its target, and the renderer and the JSON output write each copy
/// again, so a copy is charged by the most bytes it can be written as. A copy that does not
/// fit is left out: its text stays, without the link, and a shorter target later may still
/// fit.
#[derive(Debug)]
pub(crate) struct LinkRoom {
    room: u64,
    left: Counter<u64>,
    /// What laid-out pages gave back of what their runs were charged: only the cells of
    /// tables draw on it, so a table costs its text once, like a paragraph.
    freed: Counter<u64>,
    /// Links written without their target, and the first and last page with one.
    cut: Counter<u64>,
    pages: Counter<Option<(u32, u32)>>,
}

/// The room counts at one point.
struct LinkMark {
    left: u64,
    freed: u64,
    cut: u64,
    pages: Option<(u32, u32)>,
}

/// What a copy of a link target costs besides its bytes.
const LINK_COST: u64 = 64;

/// What a table costs in the model beyond the same text as paragraphs: per cell the cell
/// itself (56 bytes, rounded up), per row its list of cells, per table its block. The text
/// was charged once while the page was read.
const CELL_COST: u64 = 64;
const ROW_COST: u64 = 32;
const TABLE_COST: u64 = 128;

impl LinkRoom {
    pub(crate) fn new(bytes: u64) -> LinkRoom {
        LinkRoom {
            room: bytes,
            left: Counter::new(bytes),
            freed: Counter::new(0),
            cut: Counter::new(0),
            pages: Counter::new(None),
        }
    }

    /// Takes room for one copy of `target` on `page`; `false` when it does not fit.
    fn take(&self, target: &UriTarget, page: u32) -> bool {
        let cost = target.written + LINK_COST;
        if cost <= self.left.get() {
            self.left.set(self.left.get() - cost);
            return true;
        }
        self.cut.set(self.cut.get() + 1);
        let (first, _) = self.pages.get().unwrap_or((page, page));
        self.pages.set(Some((first, page)));
        false
    }

    /// The bytes [`LinkRoom::take_bytes`] can still take.
    pub(crate) fn available(&self) -> u64 {
        self.freed.get().saturating_add(self.left.get())
    }

    /// Gives back what a page's runs were charged while the file was read, once the page is
    /// laid out: its text now sits in the model, charged once.
    pub(crate) fn free(&self, bytes: u64) {
        self.freed.set(self.freed.get().saturating_add(bytes));
    }

    /// Takes `bytes` of overhead for the cells of a table: first from what laid-out pages
    /// gave back, then from the room the link targets share.
    pub(crate) fn take_bytes(&self, bytes: u64) -> Result<(), LimitHit> {
        let (freed, left) = (self.freed.get(), self.left.get());
        if bytes > freed.saturating_add(left) {
            return Err(LimitHit {
                limit: "kept text",
                value: bytes,
                max: freed.saturating_add(left),
                unit: "bytes",
            });
        }
        let from_freed = bytes.min(freed);
        self.freed.set(freed - from_freed);
        self.left.set(left - (bytes - from_freed));
        Ok(())
    }

    /// Takes `bytes` for a copy the object layer puts in the model (titles, outline items,
    /// graph nodes, events) from the room the link targets share; `false` when they do not
    /// fit, and nothing is taken. What laid-out pages gave back is not drawn on: it stays for
    /// tables.
    pub(crate) fn take_kept(&self, bytes: u64) -> bool {
        let left = self.left.get();
        if bytes > left {
            return false;
        }
        self.left.set(left - bytes);
        true
    }

    /// The bytes the room still holds for copies.
    pub(crate) fn left(&self) -> u64 {
        self.left.get()
    }

    /// The counts now, to go back to if the block they were taken for is dropped.
    fn mark(&self) -> LinkMark {
        LinkMark {
            left: self.left.get(),
            freed: self.freed.get(),
            cut: self.cut.get(),
            pages: self.pages.get(),
        }
    }

    fn restore(&self, mark: LinkMark) {
        self.left.set(mark.left);
        self.freed.set(mark.freed);
        self.cut.set(mark.cut);
        self.pages.set(mark.pages);
    }

    /// The bytes the copies could take at the start.
    pub(crate) fn room(&self) -> u64 {
        self.room
    }

    /// The links written without their target and the pages they stand on, if any.
    pub(crate) fn cut(&self) -> Option<(u64, u32, u32)> {
        let (first, last) = self.pages.get()?;
        Some((self.cut.get(), first, last))
    }
}

/// The blocks of a page, and the limit that stopped them early. The blocks are the ones
/// built before the stop, in reading order.
pub(crate) struct Laid {
    pub blocks: Vec<Block>,
    pub stop: Option<LimitHit>,
    /// What the stop cut first.
    pub cut: Option<Cut>,
    /// Rows for tagged tables begun on an earlier page: the table and its rows, in order.
    pub continued: Vec<(usize, Vec<Vec<Cell>>)>,
    /// For each table in `blocks`, in order: the tagged table it begins, if any.
    pub tagged: Vec<Option<usize>>,
    /// Text for cells of tagged table rows begun on earlier pages: the table, the row's place
    /// in it, the cell's place in the row and the inlines.
    pub added: Vec<(usize, usize, usize, Vec<Inline>)>,
    /// What the table detectors left undecided or found broken: words, reason, severity.
    pub notes: Vec<(String, LossReason, Severity)>,
    /// For each point asked about, the anchor of the block whose runs hold it.
    pub probed: Vec<Option<Anchor>>,
    /// The page has too many points near its runs to place them all; the rest have no block.
    pub probes_cut: bool,
    /// What the layout found by a rule of its own and says at the top of the page.
    pub found_by: Option<String>,
    /// The title the page's head gives (see [`head::page_title`]), on page 1 only.
    pub title: Option<String>,
    /// The dates the laid out text and the pairs of page 1 give as values of labels (see
    /// [`head::laid_dates`]), on page 1 only.
    pub dates: Option<Dates>,
}

/// Where a limit cut a page: the first part not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cut {
    /// The page's block anchored `p<N>:b<K>` and all after it.
    Block(usize),
    /// The page's table anchored `p<N>:t<K>` and all after it.
    Table(usize),
    /// Item `item` (from 1) of the list anchored `p<N>:b<block>` and all after it.
    Item { block: usize, item: usize },
    /// Row `row` (from 1) of the rows this page adds to a table, and all after it.
    Row { table: TableRef, row: usize },
    /// The text this page adds to the cells of row `row` (from 0) of tagged table `table`,
    /// begun on an earlier page, and all after it.
    Added { table: usize, row: usize },
}

/// A table a cut names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableRef {
    /// The page's table anchored `p<N>:t<K>`.
    Here(usize),
    /// The tagged table (an element of the tag tree) begun on an earlier page.
    Earlier(usize),
}

/// The runs of a page in reading order, with their slots and their places in the page's
/// order, which stay as runs are taken out.
#[derive(Default)]
pub(crate) struct Flow {
    pub runs: Vec<Run>,
    pub slots: Vec<Slot>,
    pub at: Vec<usize>,
}

impl Flow {
    fn new(runs: Vec<Run>, slots: Vec<Slot>) -> Flow {
        let at = (0..runs.len()).collect();
        Flow { runs, slots, at }
    }

    /// Hands each run with its index to `take`, which keeps it (returns `None`) or gives it
    /// back to stay in the flow. Nothing is copied on the way; `stay` is how many runs stay.
    pub(crate) fn take_each(
        &mut self,
        stay: usize,
        mut take: impl FnMut(usize, Run) -> Option<Run>,
    ) {
        let all = std::mem::take(self);
        let mut kept = Flow {
            runs: Vec::with_capacity(stay),
            slots: Vec::with_capacity(stay),
            at: Vec::with_capacity(stay),
        };
        for (i, ((r, s), a)) in all.runs.into_iter().zip(all.slots).zip(all.at).enumerate() {
            if let Some(r) = take(i, r) {
                kept.runs.push(r);
                kept.slots.push(s);
                kept.at.push(a);
            }
        }
        *self = kept;
    }

    /// Takes out the runs `pick` chooses by index and slot, in order, with their slots and
    /// places. Both parts are sized exactly.
    pub(crate) fn take(&mut self, pick: impl Fn(usize, Slot) -> bool) -> Vec<(Run, Slot, usize)> {
        let n = (0..self.runs.len())
            .filter(|&i| pick(i, self.slots[i]))
            .count();
        let mut taken = Vec::with_capacity(n);
        let all = std::mem::take(self);
        let stay = all.runs.len() - n;
        let mut kept = Flow {
            runs: Vec::with_capacity(stay),
            slots: Vec::with_capacity(stay),
            at: Vec::with_capacity(stay),
        };
        for (i, ((r, s), a)) in all.runs.into_iter().zip(all.slots).zip(all.at).enumerate() {
            if pick(i, s) {
                taken.push((r, s, a));
            } else {
                kept.runs.push(r);
                kept.slots.push(s);
                kept.at.push(a);
            }
        }
        *self = kept;
        taken
    }
}

/// What a page draws besides text, for tables without tags, and its size.
pub(crate) struct Drawn<'a> {
    pub rules: &'a [Rule],
    pub images: &'a [[f32; 4]],
    /// Boxes of the form fields on the page.
    pub widgets: &'a [[f32; 4]],
    /// The drawn boxes the page's runs are read in as reading regions.
    pub boxes: &'a [[f32; 4]],
    /// The text pass found the page's lines too many to search for grids, and said so.
    pub lines_unsearched: bool,
    pub width: f32,
    /// The most common font size of the file's text.
    pub body_size: f32,
}

/// [`page_blocks_at`] without points to find.
#[cfg(test)]
pub(crate) fn page_blocks(
    page: u32,
    height: Option<f32>,
    runs: Vec<Run>,
    drawn: &Drawn<'_>,
    cx: &Context<'_>,
) -> Laid {
    page_blocks_at(page, height, runs, drawn, cx, &[])
}

/// Lays out the runs of one page: running headers first, then the text with its tables in
/// reading order, then running footers. Blocks are anchored `p<N>:b<K>` and tables
/// `p<N>:t<K>`, K counting each from 1. For each of `points` (x, y in the frame of the runs)
/// [`Laid::probed`] names the block whose runs hold it: the first block built with such a
/// run.
pub(crate) fn page_blocks_at(
    page: u32,
    height: Option<f32>,
    runs: Vec<Run>,
    drawn: &Drawn<'_>,
    cx: &Context<'_>,
    points: &[(f32, f32)],
) -> Laid {
    let mut probes: Vec<(f32, f32, usize)> = points
        .iter()
        .enumerate()
        .filter(|(_, (x, y))| x.is_finite() && y.is_finite())
        .map(|(i, &(x, y))| (y, x, i))
        .collect();
    probes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut built = Built {
        probes,
        probe_block: vec![None; points.len()],
        probe_seen: vec![0; points.len()],
        probe_round: 1,
        probe_work: PROBE_WORK,
        ..Built::default()
    };
    if let Err(hit) = cx
        .budget
        .add_elements(u64::try_from(runs.len()).unwrap_or(u64::MAX))
    {
        built.stop_at(hit, cx);
        return built.laid(Vec::new());
    }
    let mut flow = match cx.tree {
        Some(tree) => {
            let (runs, slots) = classified(runs, tree, cx);
            Flow::new(runs, slots)
        }
        None => {
            let n = runs.len();
            Flow::new(runs, vec![Slot::Free; n])
        }
    };
    let mut notes = Vec::new();
    let mut placed: Vec<Placed> = Vec::new();
    // Tagged tables are taken out before running headers and footers are looked for: the
    // column titles a long table repeats at the top of each page are table text.
    let geometry = match cx.tree {
        Some(tree) => {
            let (found, fallbacks) =
                tables::tagged(page, &mut flow, tree, cx.tables, cx.in_part, &|| {
                    time_over(cx.budget).is_err()
                });
            placed.extend(found.into_iter().map(Placed::Table));
            for what in cx.tables.take_issues() {
                notes.push((what, LossReason::Damaged, Severity::Warning));
            }
            let fell_back = !fallbacks.is_empty();
            notes.extend(
                fallbacks
                    .into_iter()
                    .map(|w| (w, LossReason::Uncertain, Severity::Info)),
            );
            fell_back
        }
        None => true,
    };
    let bands = {
        let all = lines(&flow.runs);
        // Text the tag tree places is content, wherever it stands.
        let content: Vec<bool> = if cx.tree.is_some() {
            let mut at = 0;
            all.iter()
                .map(|l| {
                    let s = &flow.slots[at..at + l.runs.len()];
                    at += l.runs.len();
                    s.iter().any(|s| *s != Slot::Free)
                })
                .collect()
        } else {
            vec![false; all.len()]
        };
        let by_line = cx.style.furniture(&all, height, &content);
        let mut bands = Vec::with_capacity(flow.runs.len());
        for (l, b) in all.iter().zip(by_line) {
            bands.extend(std::iter::repeat_n(b, l.runs.len()));
        }
        bands
    };
    let edges = flow.take(|i, _| bands[i].is_some());
    let mut headers: Vec<Run> = Vec::new();
    let mut footers: Vec<Run> = Vec::new();
    for ((r, _, _), b) in edges.into_iter().zip(bands.iter().flatten()) {
        match b {
            Band::Top => headers.push(r),
            Band::Bottom => footers.push(r),
        }
    }
    // One list of blocks for the whole page, each charged as it is built: a stop keeps the
    // blocks before it.
    for l in lines(&headers) {
        built.paragraph(l.runs.iter().collect(), Some("page-header"), cx);
    }
    // Footnotes are found before tables and lists: a legend set in columns is no table, and
    // a star that opens a footnote is no bullet. A tagged file says its notes.
    let mut legend = if cx.tree.is_none() {
        footnotes::take(page, &mut flow, cx.style.legends.get(&page))
    } else {
        footnotes::Legend::default()
    };
    if let Some(why) = legend.refused.take() {
        notes.push((why, LossReason::Uncertain, Severity::Info));
    }
    // The text sizes are counted over all text of the page but its legend, also what tables
    // take later (the values of a form are its data lines), and a footer of a file of two pages
    // (see [`Style::two_page`]).
    let two_page_lines = lines(&footers);
    let two_page = two_page_lines
        .iter()
        .filter(|l| furniture_key_of(l, height).is_some_and(|k| cx.style.two_page.contains(&k)))
        .flat_map(|l| l.runs.iter());
    let text_sizes = page_text_sizes_of(flow.runs.iter().chain(two_page));
    if geometry && let Some(height) = height {
        // Drawn boxes of a label over its value are taken first: a row of them is no header
        // band. A page with form fields is the form's (see `tables::untagged`).
        if drawn.widgets.is_empty() {
            // A title over a bar is a form's: a page of prose keeps its headings and
            // paragraphs.
            if tables::form_drawn(drawn.rules, drawn.boxes) {
                let titles = tables::underlined_titles(&mut flow, drawn.rules);
                placed.extend(titles.into_iter().map(Placed::Title));
            }
            let (found, titles, unsure) =
                tables::box_pairs(page, &mut flow, drawn.boxes, drawn.rules, &text_sizes);
            placed.extend(found.into_iter().map(Placed::Table));
            placed.extend(titles.into_iter().map(Placed::Title));
            notes.extend(
                unsure
                    .into_iter()
                    .map(|w| (w, LossReason::Uncertain, Severity::Info)),
            );
        }
        let stop = || time_over(cx.budget).is_err();
        let found = tables::untagged(page, &mut flow, drawn, height, &stop);
        placed.extend(found.tables.into_iter().map(Placed::Table));
        placed.extend(found.unsure.into_iter().map(Placed::Unsure));
        notes.extend(
            found
                .skipped
                .into_iter()
                .map(|w| (w, LossReason::Limit, Severity::Info)),
        );
        notes.extend(
            found
                .uncertain
                .into_iter()
                .map(|w| (w, LossReason::Uncertain, Severity::Info)),
        );
        notes.extend(
            refused_quietly(page, found.quiet)
                .into_iter()
                .map(|w| (w, LossReason::Uncertain, Severity::Info)),
        );
    }
    placed.sort_by(|a, b| {
        let (la, ta) = a.at();
        let (lb, tb) = b.at();
        la.cmp(&lb).then(tb.total_cmp(&ta))
    });
    // The title and the labelled dates of the document: the lines of page 1 as the layout
    // lays them out as text, and the pairs it read.
    let (title, dates) = match page {
        1 => {
            let text = lines(&flow.runs);
            let title = height.and_then(|height| {
                head::page_title(
                    &text,
                    &heading_of(cx.style, &text_sizes),
                    height,
                    cx.style.body_size,
                )
            });
            // Every table of the page, its rows as the text of their cells.
            let tables: Vec<head::Table> = placed
                .iter()
                .filter_map(|p| match p {
                    Placed::Table(t) => Some(head::Table {
                        pairs: t.is_pairs(),
                        top: t.top,
                        rows: t
                            .rows
                            .iter()
                            .map(|row| {
                                row.iter()
                                    .map(|c| head::Cell {
                                        text: c.runs.iter().map(|r| r.text.as_str()).collect(),
                                        spot: c.runs.first().map(|first| head::Spot {
                                            y: first.y,
                                            x0: c.runs.iter().map(|r| r.x).fold(first.x, f32::min),
                                            x1: c
                                                .runs
                                                .iter()
                                                .map(|r| r.x + r.w)
                                                .fold(first.x, f32::max),
                                            size: first.size,
                                        }),
                                    })
                                    .collect()
                            })
                            .collect(),
                    }),
                    _ => None,
                })
                .collect();
            (title, Some(head::laid_dates(&flow.runs, &tables, height)))
        }
        _ => (None, None),
    };
    let lay = |runs: &[Run], slots: &[Slot], built: &mut Built| match cx.tree {
        Some(_) => tagged(runs, slots, cx, built),
        None => untagged(runs, slots, &text_sizes, cx, built),
    };
    let mut start = 0;
    for p in placed {
        let (at, _) = p.at();
        let end = start + flow.at[start..].partition_point(|a| *a < at);
        lay(&flow.runs[start..end], &flow.slots[start..end], &mut built);
        start = end;
        match p {
            Placed::Table(t) => built.table(t, cx),
            Placed::Title(t) => {
                let mut runs: Vec<&Run> = t.runs.iter().collect();
                runs.sort_by(|a, b| a.line.cmp(&b.line).then(a.x.total_cmp(&b.x)));
                built.heading(SIZE_LEVELS as u8, runs, cx);
                // The lines of its box or bar follow it, each a paragraph: they are lines of
                // a form (a dash that opens one is no list).
                let mut text = t.text;
                text.sort_by(|a, b| a.line.cmp(&b.line).then(a.x.total_cmp(&b.x)));
                for l in lines(&text) {
                    built.paragraph(l.runs.iter().collect(), None, cx);
                }
            }
            Placed::Unsure(u) => {
                let what = u.text.unwrap_or_else(|| {
                    format!(
                        "Possible table on page {page} not rebuilt (reason: {})",
                        u.reason
                    )
                });
                built.note(&what, cx);
                notes.push((what, LossReason::Uncertain, Severity::Info));
            }
        }
    }
    lay(&flow.runs[start..], &flow.slots[start..], &mut built);
    // Markers that end a word above refer to the footnotes too. Only the footnotes written
    // are said: a stop writes none.
    footnotes::link_words(&mut built.blocks, &legend);
    // A footnote no reference points to is a paragraph of its own: a Markdown viewer hides a
    // footnote definition without a reference.
    let linked = footnotes::referenced(&mut built.blocks);
    let before = built.blocks.len();
    let mut linked_written = 0;
    for (label, runs, cut) in &legend.notes {
        let runs: Vec<&Run> = runs.iter().collect();
        if linked.contains(label) {
            // The label says the marker: the footnote's text starts after it.
            let text = without_marker(&runs, *cut);
            let at = built.blocks.len();
            built.footnote(label, text.iter().collect(), cx);
            linked_written += built.blocks.len() - at;
        } else {
            built.paragraph(runs, Some("footnote"), cx);
        }
    }
    let footnotes_written = built.blocks.len() - before;
    // References to footnotes a stop did not write go back to text.
    let unwritten: Vec<&str> = legend.notes[footnotes_written.min(legend.notes.len())..]
        .iter()
        .map(|n| n.0.as_str())
        .collect();
    footnotes::unlink(&mut built.blocks, &legend, &unwritten);
    // The running footer, block by block (see [`footer_blocks`]).
    let footer_lines = lines(&footers);
    for block in footer_blocks(&footer_lines) {
        let runs: Vec<&Run> = block.iter().flat_map(|l| l.runs.iter()).collect();
        let mut breaks: Vec<(u32, u32)> = block
            .windows(2)
            .filter(|w| !footer_wrap(&w[0], &w[1]))
            .map(|w| (w[1].runs[0].page, w[1].runs[0].line))
            .collect();
        breaks.sort_unstable();
        built.broken_paragraph(runs, &breaks, Some("page-footer"), cx);
    }
    if built.tables_as_text > 0 {
        let k = built.tables_as_text;
        let (s, them) = if k == 1 { ("", "its") } else { ("s", "their") };
        let mut what = format!(
            "{k} table{s} on page {page} kept as paragraphs: the room for the cells of tables \
             is spent; {them} text is kept"
        );
        match built.tagged_as_text.as_slice() {
            [] => {}
            [b] => what.push_str(&format!(
                "; the tagged table from p{page}:b{b} on stays paragraphs on its later pages \
                 too"
            )),
            [b, ..] => what.push_str(&format!(
                "; the {} tagged tables among them, the first from p{page}:b{b} on, stay \
                 paragraphs on their later pages too",
                built.tagged_as_text.len()
            )),
        }
        notes.push((what, LossReason::Limit, Severity::Info));
    }
    let probe_block = std::mem::take(&mut built.probe_block);
    let probes_cut = built.probes_cut;
    let mut laid = built.laid(notes);
    laid.probes_cut = probes_cut;
    laid.found_by = footnotes::note(page, footnotes_written, linked_written);
    laid.title = title;
    laid.dates = dates;
    let (mut b, mut t) = (0, 0);
    for block in &mut laid.blocks {
        let anchor = if matches!(block, Block::Table { .. }) {
            t += 1;
            format!("p{page}:t{t}")
        } else {
            b += 1;
            format!("p{page}:b{b}")
        };
        set_anchor(block, Anchor(anchor));
    }
    laid.probed = probe_block
        .into_iter()
        .map(|k| k.and_then(|k| anchor_of(laid.blocks.get(k)?)))
        .collect();
    laid
}

fn anchor_of(block: &Block) -> Option<Anchor> {
    match block {
        Block::Heading { anchor, .. }
        | Block::Paragraph { anchor, .. }
        | Block::List { anchor, .. }
        | Block::Table { anchor, .. } => anchor.clone(),
        _ => None,
    }
}

/// A table or an undecided region, placed in reading order among the text.
enum Placed {
    Table(Planned),
    Unsure(Unsure),
    /// The title of a box beside boxed fields, a heading of the lowest level.
    Title(tables::BoxTitle),
}

impl Placed {
    /// The place in reading order it stands at, and its top.
    fn at(&self) -> (usize, f32) {
        match self {
            Placed::Table(t) => (t.at, t.top),
            Placed::Unsure(u) => (u.at, u.top),
            Placed::Title(t) => (t.at, t.top),
        }
    }
}

fn set_anchor(block: &mut Block, to: Anchor) {
    match block {
        Block::Heading { anchor, .. }
        | Block::Paragraph { anchor, .. }
        | Block::List { anchor, .. }
        | Block::Table { anchor, .. } => *anchor = Some(to),
        _ => {}
    }
}

/// Run and point pairs one page may test to find the blocks of annotations.
const PROBE_WORK: usize = 16_000_000;

/// How many runs or pieces of one block go by between two looks at the clock.
const CHECK_EVERY: usize = 1024;

/// Fails once the time limit is over. A cancel is not looked at here: the walk answers it, and
/// the pages it read are still laid out.
fn time_over(budget: &Budget) -> Result<(), LimitHit> {
    match budget.check_time() {
        Err(hit) if hit.limit == "time" => Err(hit),
        _ => Ok(()),
    }
}

/// Blocks being built, and the limit that stopped them.
#[derive(Default)]
struct Built {
    blocks: Vec<Block>,
    stop: Option<LimitHit>,
    /// What is being built now, which a stop cuts.
    pending: Pending,
    cut: Option<Cut>,
    /// Rows for tagged tables begun on earlier pages.
    continued: Vec<(usize, Vec<Vec<Cell>>)>,
    /// The tagged table each table block begins, if any.
    tagged: Vec<Option<usize>>,
    /// Text for cells of rows begun on earlier pages.
    added: Vec<(usize, usize, usize, Vec<Inline>)>,
    /// Tables whose overhead did not fit and that were laid out as paragraphs.
    tables_as_text: usize,
    /// Of those, the tagged ones by the place of their first paragraph among the page's
    /// blocks (`p<N>:b<K>`).
    tagged_as_text: Vec<usize>,
    /// The link room before the block (or the table row) now being built, to go back to if
    /// it is dropped.
    mark: Option<LinkMark>,
    /// Points to find the blocks of: y, x and their place in the list asked about, by y.
    probes: Vec<(f32, f32, usize)>,
    /// The block each point lies in, by its place in `blocks`.
    probe_block: Vec<Option<usize>>,
    /// The points the runs of the block now being built hold, each once.
    probe_pending: Vec<usize>,
    /// For each point, the round of `probe_pending` it was last put in, so a point that many
    /// runs of one block hold is listed once: the list never holds more than the points.
    probe_seen: Vec<u32>,
    /// The round of `probe_pending`, from 1; it moves on each time the list is emptied.
    probe_round: u32,
    /// Run and point pairs the page may still test.
    probe_work: usize,
    probes_cut: bool,
    /// The `found-by` notes of tables said on the page so far.
    found_by: Vec<&'static str>,
}

/// What a page's layout is building.
#[derive(Debug, Clone, Copy, Default)]
enum Pending {
    /// A new block.
    #[default]
    Block,
    /// A new table.
    Table,
    /// An item that joins the list built last.
    Item,
    /// A row of a table.
    Row(TableRef, usize),
    /// Text for a row of a tagged table begun on an earlier page.
    Added(usize, usize),
}

impl Built {
    /// Adds a block, charged as one element. Past the limit the block is dropped and the page
    /// stops.
    fn push(&mut self, block: Block, cx: &Context<'_>) {
        if self.stop.is_some() {
            return;
        }
        match time_over(cx.budget).and_then(|()| cx.budget.add_elements(1)) {
            Ok(()) => {
                self.mark = None;
                self.blocks.push(block);
                self.probes_found();
            }
            Err(hit) => self.stop_at(hit, cx),
        }
    }

    /// Stops the page at `hit`: the only place that sets `stop`. The link counts taken for the
    /// block being built go back, so no loss says the text of a link is kept when the block
    /// is not, whichever check stopped it and however many of its cells were built.
    fn stop_at(&mut self, hit: LimitHit, cx: &Context<'_>) {
        if let Some(mark) = self.mark.take() {
            cx.links.restore(mark);
        }
        self.probe_pending.clear();
        self.probe_round += 1;
        let tables = self
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Table { .. }))
            .count();
        let others = self.blocks.len() - tables;
        self.cut = Some(match self.pending {
            Pending::Block => Cut::Block(others + 1),
            Pending::Table => Cut::Table(tables + 1),
            Pending::Item => match self.blocks.last() {
                Some(Block::List { items, .. }) => Cut::Item {
                    block: others,
                    item: items.len() + 1,
                },
                _ => Cut::Block(others + 1),
            },
            // Nothing of a table of this page is kept before its first row.
            Pending::Row(TableRef::Here(k), 1) => Cut::Table(k),
            Pending::Row(table, row) => Cut::Row { table, row },
            Pending::Added(table, row) => Cut::Added { table, row },
        });
        self.stop = Some(hit);
    }

    /// What was built, with the table detectors' `notes`.
    fn laid(self, notes: Vec<(String, LossReason, Severity)>) -> Laid {
        Laid {
            blocks: self.blocks,
            stop: self.stop,
            cut: self.cut,
            continued: self.continued,
            tagged: self.tagged,
            added: self.added,
            notes,
            probed: Vec::new(),
            probes_cut: false,
            found_by: None,
            title: None,
            dates: None,
        }
    }

    /// Notes the points the boxes of `runs` hold, for the block now being built. A run's box
    /// reaches from its left edge over its advance, and from a third of its size below the
    /// baseline to its size above it.
    fn probe(&mut self, runs: &[&Run]) {
        if self.probes.is_empty() {
            return;
        }
        for r in runs {
            let (y0, y1) = (r.y - r.h / 3.0, r.y + r.h);
            let (x0, x1) = (r.x.min(r.x + r.w), r.x.max(r.x + r.w));
            let from = self.probes.partition_point(|p| p.0 < y0);
            for &(y, x, i) in &self.probes[from..] {
                if y > y1 {
                    break;
                }
                if self.probe_work == 0 {
                    // Many points at the height of many runs: the rest stay without a block.
                    self.probes_cut = true;
                    self.probes.clear();
                    return;
                }
                self.probe_work -= 1;
                if x >= x0
                    && x <= x1
                    && self.probe_block[i].is_none()
                    && self.probe_seen[i] != self.probe_round
                {
                    self.probe_seen[i] = self.probe_round;
                    self.probe_pending.push(i);
                }
            }
        }
    }

    /// The points the runs of the block just added hold lie in it.
    fn probes_found(&mut self) {
        let k = self.blocks.len().saturating_sub(1);
        for i in self.probe_pending.drain(..) {
            self.probe_block[i].get_or_insert(k);
        }
        self.probe_round += 1;
    }

    /// A paragraph that says something about the page, not its text.
    fn note(&mut self, text: &str, cx: &Context<'_>) {
        if self.stop.is_some() {
            return;
        }
        self.pending = Pending::Block;
        if let Err(hit) = time_over(cx.budget).and_then(|()| cx.budget.charge_text(text.len())) {
            self.stop_at(hit, cx);
            return;
        }
        self.push(Block::paragraph(vec![Inline::plain(text)]), cx);
    }

    /// Builds a planned table: a new table on this page, or rows and cell text for a tagged
    /// table begun on an earlier page. The table, each row and each cell are charged as
    /// elements, and each cell also by its fixed cost and its text against the room the model
    /// may keep; a stop inside a table keeps the rows built before.
    fn table(&mut self, mut plan: Planned, cx: &Context<'_>) {
        if self.stop.is_some() {
            return;
        }
        let earlier = plan.tagged.filter(|_| plan.continues);
        // Text for rows an earlier page began has no overhead (no new cells). It is built
        // first, so the overhead set aside below stays with the rows it is for.
        if let Some(t) = earlier {
            for add in std::mem::take(&mut plan.additions) {
                self.pending = Pending::Added(t, add.row);
                self.mark = Some(cx.links.mark());
                let Some(inlines) = self.cell_inlines(&add.runs, cx) else {
                    return;
                };
                self.mark = None;
                self.added.push((t, add.row, add.cell, inlines));
            }
        }
        // The rule that found the table is said once on the page, before its first table.
        if let Some(note) = plan.found_by.filter(|n| !self.found_by.contains(n)) {
            self.found_by.push(note);
            self.pending = Pending::Block;
            if let Err(hit) = time_over(cx.budget).and_then(|()| cx.budget.charge_text(note.len()))
            {
                self.stop_at(hit, cx);
                return;
            }
            self.push(
                Block::Paragraph {
                    inlines: vec![Inline::plain(note)],
                    role: Some("found-by".into()),
                    anchor: None,
                },
                cx,
            );
            if self.stop.is_some() {
                return;
            }
        }
        // The title line or lines, left to right, are built first, as they come before the
        // table: charged like any text, links kept.
        let mut title = std::mem::take(&mut plan.caption);
        title.sort_by(|a, b| a.line.cmp(&b.line).then(a.x.total_cmp(&b.x)));
        let title: Option<Vec<Inline>> = if title.is_empty() {
            None
        } else {
            self.pending = Pending::Block;
            let refs: Vec<&Run> = title.iter().collect();
            // The title's mark stays: a stop before the title is pushed or the table is
            // kept gives its link counts back with the table's.
            let built = self.inlines(&refs, cx);
            if built.is_none() {
                return;
            }
            built
        };
        // A title is the table's caption when a table is built and the title is plain text;
        // otherwise it is a paragraph with role `caption` right before what it titles.
        let title_paragraph = |built: &mut Self, title: Option<Vec<Inline>>| {
            if let Some(inlines) = title {
                built.pending = Pending::Block;
                built.push(
                    Block::Paragraph {
                        inlines,
                        role: Some("caption".into()),
                        anchor: None,
                    },
                    cx,
                );
            }
        };
        // A title with a link goes first, before the table is set aside, as a caption is
        // plain text.
        let title = match title {
            Some(t) if t.iter().any(|i| matches!(i, Inline::Link { .. })) => {
                title_paragraph(self, Some(t));
                if self.stop.is_some() {
                    return;
                }
                None
            }
            other => other,
        };
        // What the table's cells and rows cost beyond their text is set aside at once. When it
        // does not fit, the table's text is laid out as paragraphs: the overhead of a table
        // never stops the text.
        let cells: usize = plan.rows.iter().map(Vec::len).sum();
        let overhead = if earlier.is_some() { 0 } else { TABLE_COST }
            + ROW_COST * plan.rows.len() as u64
            + CELL_COST * cells as u64;
        if !plan.rows.is_empty() {
            // The overhead alone goes back when it does not fit: the title is still placed
            // (as a paragraph), so what its links took, cut links too, stays counted. A later
            // stop drops the title with the table and gives back from the title's mark.
            let title_mark = self.mark.take();
            let mark = cx.links.mark();
            if cx.links.take_bytes(overhead).is_err() {
                cx.links.restore(mark);
                let rows = std::mem::take(&mut plan.rows);
                title_paragraph(self, title);
                self.as_text(rows, plan.tagged, cx);
                return;
            }
            // The overhead goes back with the table's first row if that is dropped.
            self.mark = Some(title_mark.unwrap_or(mark));
        }
        let table = match earlier {
            Some(t) => TableRef::Earlier(t),
            None => {
                self.pending = Pending::Table;
                if self.mark.is_none() {
                    self.mark = Some(cx.links.mark());
                }
                if let Err(hit) = time_over(cx.budget).and_then(|()| cx.budget.add_elements(1)) {
                    self.stop_at(hit, cx);
                    return;
                }
                let here = self
                    .blocks
                    .iter()
                    .filter(|b| matches!(b, Block::Table { .. }))
                    .count();
                TableRef::Here(here + 1)
            }
        };
        let mut rows: Vec<Vec<Cell>> = Vec::with_capacity(plan.rows.len());
        'rows: for (r, row) in plan.rows.into_iter().enumerate() {
            self.pending = Pending::Row(table, r + 1);
            if self.mark.is_none() {
                self.mark = Some(cx.links.mark());
            }
            if let Err(hit) = cx.budget.add_elements(1) {
                self.stop_at(hit, cx);
                break;
            }
            let mut cells = Vec::with_capacity(row.len());
            for c in row {
                let Some(mut inlines) = self.cell_inlines(&c.runs, cx) else {
                    break 'rows;
                };
                if let (true, Some(text)) = (inlines.is_empty(), c.status) {
                    inlines.push(Inline::Status {
                        text: text.into(),
                        color: StatusColor::Grey,
                    });
                }
                cells.push(Cell {
                    inlines,
                    row_span: c.row_span,
                    col_span: c.col_span,
                    anchor: None,
                });
            }
            self.mark = None;
            if let (Some(t), Some(tr)) = (plan.tagged, plan.trs.get(r)) {
                cx.tables.row_built(t, *tr);
            }
            rows.push(cells);
        }
        self.mark = None;
        if rows.is_empty() {
            self.probe_pending.clear();
            self.probe_round += 1;
            title_paragraph(self, title);
            return;
        }
        match earlier {
            Some(t) => {
                // The rows join a table of an earlier page, which no point here can name.
                self.probe_pending.clear();
                self.probe_round += 1;
                self.continued.push((t, rows))
            }
            None => {
                self.tagged.push(plan.tagged);
                let caption = title
                    .map(|t| rezzer_model::plain_text(&t).trim().to_string())
                    .filter(|t| !t.is_empty());
                self.blocks.push(Block::Table {
                    caption,
                    header_rows: plan.header_rows.min(rows.len()),
                    role: None,
                    rows,
                    alignments: Vec::new(),
                    anchor: None,
                    layout: None,
                });
                self.probes_found();
            }
        }
    }

    /// The text of a table whose overhead does not fit, laid out as paragraphs in reading
    /// order. The page counts it for one loss. A tagged table (`table`) stays paragraphs on
    /// its later pages too, and the loss names it by its first paragraph.
    fn as_text(&mut self, rows: Vec<Vec<PlannedCell>>, table: Option<usize>, cx: &Context<'_>) {
        let mut runs: Vec<Run> = rows.into_iter().flatten().flat_map(|c| c.runs).collect();
        if runs.is_empty() {
            return;
        }
        runs.sort_by(|a, b| a.line.cmp(&b.line).then(a.x.total_cmp(&b.x)));
        self.tables_as_text += 1;
        let first = self
            .blocks
            .iter()
            .filter(|b| !matches!(b, Block::Table { .. }))
            .count()
            + 1;
        let before = self.blocks.len();
        let free = vec![Slot::Free; runs.len()];
        tagged(&runs, &free, cx, self);
        if let Some(t) = table {
            cx.tables.keep_as_text(t);
            if self.blocks.len() > before {
                self.tagged_as_text.push(first);
            }
        }
    }

    /// The inlines of one table cell, charged as an element (its text was charged when the
    /// page was read, its overhead with its table); `None` once a limit stopped the page.
    fn cell_inlines(&mut self, runs: &[Run], cx: &Context<'_>) -> Option<Vec<Inline>> {
        let charged = time_over(cx.budget).and_then(|()| cx.budget.add_elements(1));
        if let Err(hit) = charged {
            self.stop_at(hit, cx);
            return None;
        }
        if runs.is_empty() {
            return Some(Vec::new());
        }
        let refs: Vec<&Run> = runs.iter().collect();
        self.inlines(&refs, cx)
    }

    /// The inlines of `runs`, or `None` once a limit stopped the page. Each stretch of runs
    /// under one link becomes one link with its own copy of the target, charged to the link
    /// room.
    fn inlines(&mut self, runs: &[&Run], cx: &Context<'_>) -> Option<Vec<Inline>> {
        self.broken_inlines(runs, &[], cx)
    }

    /// [`Built::inlines`] with a line break before each line `breaks` names by page and line,
    /// sorted.
    fn broken_inlines(
        &mut self,
        runs: &[&Run],
        breaks: &[(u32, u32)],
        cx: &Context<'_>,
    ) -> Option<Vec<Inline>> {
        if self.stop.is_some() {
            return None;
        }
        if self.mark.is_none() {
            self.mark = Some(cx.links.mark());
        }
        let bytes: usize = runs.iter().map(|r| r.text.len() + 1).sum();
        if let Err(hit) = time_over(cx.budget).and_then(|()| cx.budget.charge_text(bytes)) {
            self.stop_at(hit, cx);
            return None;
        }
        self.probe(runs);
        match inlines(runs, breaks, cx.links, cx.budget) {
            Ok(mut built) => {
                // Most blocks hold one or two inlines: the list keeps no spare room.
                built.shrink_to_fit();
                Some(built)
            }
            Err(hit) => {
                self.stop_at(hit, cx);
                None
            }
        }
    }

    fn paragraph(&mut self, runs: Vec<&Run>, role: Option<&str>, cx: &Context<'_>) {
        self.broken_paragraph(runs, &[], role, cx);
    }

    /// A paragraph with a line break before each line `breaks` names (page, line), sorted.
    fn broken_paragraph(
        &mut self,
        runs: Vec<&Run>,
        breaks: &[(u32, u32)],
        role: Option<&str>,
        cx: &Context<'_>,
    ) {
        self.pending = Pending::Block;
        if let Some(inlines) = self.broken_inlines(&runs, breaks, cx) {
            self.push(
                Block::Paragraph {
                    inlines,
                    role: role.map(str::to_string),
                    anchor: None,
                },
                cx,
            );
        }
    }

    /// A footnote: a footnote section labelled `label` that holds its text as a paragraph
    /// with the role `footnote`.
    fn footnote(&mut self, label: &str, runs: Vec<&Run>, cx: &Context<'_>) {
        self.pending = Pending::Block;
        if self.stop.is_some() {
            return;
        }
        // The paragraph inside the section is an element too.
        if let Err(hit) = cx.budget.add_elements(1) {
            self.stop_at(hit, cx);
            return;
        }
        if let Some(inlines) = self.inlines(&runs, cx) {
            self.push(
                Block::Section {
                    kind: SectionKind::Other,
                    title: None,
                    anchor: Some(Anchor(format!("fn:{label}"))),
                    blocks: vec![Block::Paragraph {
                        inlines,
                        role: Some("footnote".into()),
                        anchor: None,
                    }],
                },
                cx,
            );
        }
    }

    fn heading(&mut self, level: u8, runs: Vec<&Run>, cx: &Context<'_>) {
        self.pending = Pending::Block;
        if let Some(mut inlines) = self.inlines(&runs, cx) {
            // Heading fonts are mostly bold; the heading says that already.
            for i in &mut inlines {
                match i {
                    Inline::Text { strong, .. } => *strong = false,
                    Inline::Link { inlines, .. } => inlines.iter_mut().for_each(|i| {
                        if let Inline::Text { strong, .. } = i {
                            *strong = false;
                        }
                    }),
                    _ => {}
                }
            }
            self.push(
                Block::Heading {
                    level,
                    inlines,
                    anchor: None,
                },
                cx,
            );
        }
    }

    /// Adds a list item; it joins the list just built when `list` names the same list.
    fn item(
        &mut self,
        list: Option<usize>,
        ordered: bool,
        runs: Vec<&Run>,
        cx: &Context<'_>,
        open: &mut Option<Option<usize>>,
    ) {
        let joins = *open == Some(list) && matches!(self.blocks.last(), Some(Block::List { .. }));
        self.pending = if joins { Pending::Item } else { Pending::Block };
        let Some(inlines) = self.inlines(&runs, cx) else {
            return;
        };
        let para = Block::paragraph(inlines);
        if *open == Some(list)
            && let Some(Block::List { items, .. }) = self.blocks.last_mut()
        {
            items.push(vec![para]);
            // The item joins a list that is kept already.
            self.mark = None;
            self.probes_found();
            return;
        }
        self.push(
            Block::List {
                ordered,
                items: vec![vec![para]],
                anchor: None,
            },
            cx,
        );
        *open = Some(list);
    }
}

/// Inlines from runs in order. Runs on a new line are joined with a space, or without one
/// after a hyphen that ends a word (not after a dash with white space before it); a line `breaks` names (page, line; sorted) starts after a line break.
/// Bold, italic and URI links are kept while `links` has room for their targets; past it a
/// link's text stays without the link.
fn inlines(
    runs: &[&Run],
    breaks: &[(u32, u32)],
    links: &LinkRoom,
    budget: &Budget,
) -> Result<Vec<Inline>, LimitHit> {
    struct Piece {
        text: String,
        bold: bool,
        italic: bool,
        uri: Option<Rc<UriTarget>>,
        page: u32,
        /// A line break stands before it.
        broken: bool,
        /// A footnote reference, its text the label.
        note: bool,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut last: Option<(u32, u32)> = None;
    // Where the ink of the last run with text ends, on its line, and that run.
    let mut ink_end = f32::MIN;
    let mut before: Option<&Run> = None;
    for (k, r) in runs.iter().enumerate() {
        if k % CHECK_EVERY == 0 {
            time_over(budget)?;
        }
        let new_line = last.is_some_and(|l| l != (r.page, r.line));
        let mut text = if last.is_none() || new_line {
            r.text.trim_start().to_string()
        } else {
            r.text.clone()
        };
        // A run that is only white space does not take the line change: the first run with
        // text on the new line does.
        if text.is_empty() {
            continue;
        }
        last = Some((r.page, r.line));
        // A run that starts well inside the ink of the run before it on its line is printed
        // over it (a value over a label), and one that touches it in another size or weight
        // is set against it: another text, never one word with it (see [`apart`]).
        let over = !new_line
            && k > 0
            && apart(before, r, ink_end)
            && !text.starts_with(char::is_whitespace)
            && pieces
                .last()
                .is_some_and(|p| !p.text.ends_with(char::is_whitespace));
        if over {
            text.insert(0, ' ');
        }
        if !r.text.trim().is_empty() {
            ink_end = if new_line {
                r.x + r.w
            } else {
                ink_end.max(r.x + r.w)
            };
            before = Some(r);
        }
        let broken = new_line && breaks.binary_search(&(r.page, r.line)).is_ok();
        // A hyphen at the end of a line joins the word that wraps; one with white space before
        // it is a dash (`01.10.2026 -` over `31.10.2026`), and the space after it stays, as it
        // does after the minus of an amount (`10,00-` over `80,00`).
        let glued = pieces.last().is_none_or(|p| {
            p.text.ends_with(' ')
                || (!signed_amount(&p.text)
                    && p.text.strip_suffix('-').is_some_and(|rest| {
                        !rest.is_empty() && !rest.ends_with(char::is_whitespace)
                    }))
        });
        if new_line && !glued && !broken {
            text.insert(0, ' ');
        }
        if broken && let Some(p) = pieces.last_mut() {
            // White space at the end of a line before a break is dropped.
            let end = p.text.trim_end().len();
            p.text.truncate(end);
        }
        if r.note {
            // A reference stands right after its word.
            text = r.text.trim().to_string();
        }
        match pieces.last_mut() {
            Some(p)
                if !broken
                    && !r.note
                    && !p.note
                    && p.bold == r.bold
                    && p.italic == r.italic
                    && p.uri == r.uri =>
            {
                p.text.push_str(&text)
            }
            _ => pieces.push(Piece {
                text,
                bold: r.bold,
                italic: r.italic,
                uri: r.uri.clone(),
                page: r.page,
                broken,
                note: r.note,
            }),
        }
    }
    if let Some(p) = pieces.last_mut() {
        let end = p.text.trim_end().len();
        p.text.truncate(end);
    }
    let mut out = Vec::new();
    // The target of the link written last without its target: a link that goes on over a
    // change of style is one link cut, not two.
    let mut refused: Option<Rc<UriTarget>> = None;
    // White space after the text of the link written last: inside the link when the link
    // goes on, else after it.
    let mut trail = String::new();
    for (k, p) in pieces.into_iter().enumerate() {
        if k % CHECK_EVERY == 0 {
            time_over(budget)?;
        }
        if p.text.is_empty() {
            continue;
        }
        if p.broken && !out.is_empty() {
            // White space after a link before the break ends the line: it is dropped.
            trail.clear();
            refused = None;
            out.push(Inline::LineBreak);
        }
        if p.note {
            let rest = std::mem::take(&mut trail);
            if !rest.is_empty() {
                out.push(Inline::plain(rest));
            }
            // The white space before a reference goes: it stands at its word.
            if let Some(Inline::Text { text, .. }) = out.last_mut() {
                let end = text.trim_end().len();
                text.truncate(end);
            }
            refused = None;
            out.push(Inline::FootnoteRef { id: p.text });
            continue;
        }
        // White space at the edge of a link belongs outside it.
        let (inner, lead) = match p.uri {
            Some(_) => {
                let body = p.text.trim_end();
                let lead = body.len() - body.trim_start().len();
                let inner = body[lead..].to_string();
                let rest = std::mem::replace(&mut trail, p.text[body.len()..].to_string());
                (inner, rest + &p.text[..lead])
            }
            None => {
                let rest = std::mem::take(&mut trail);
                if !rest.is_empty() {
                    out.push(Inline::plain(rest));
                }
                (p.text.clone(), String::new())
            }
        };
        if inner.is_empty() {
            // Only white space in a link: it goes on as the white space after it.
            trail.insert_str(0, &lead);
            continue;
        }
        let text = Inline::Text {
            text: inner,
            emphasis: p.italic,
            strong: p.bold,
            strike: false,
        };
        match p.uri {
            Some(target) => {
                // A link goes on over a change of style: one link, one copy of its target.
                if let Some(Inline::Link {
                    target: before,
                    inlines,
                    ..
                }) = out.last_mut()
                    && before.as_str() == &*target.uri
                {
                    if !lead.is_empty() {
                        inlines.push(Inline::plain(lead));
                    }
                    inlines.push(text);
                    continue;
                }
                if !lead.is_empty() {
                    out.push(Inline::plain(lead));
                }
                let goes_on = refused.as_ref().is_some_and(|r| Rc::ptr_eq(r, &target));
                if goes_on || !links.take(&target, p.page) {
                    out.push(text);
                    refused = Some(target);
                    continue;
                }
                refused = None;
                out.push(Inline::Link {
                    target: target.uri.to_string(),
                    title: None,
                    internal: false,
                    wiki: false,
                    inlines: vec![text],
                });
            }
            None => {
                refused = None;
                out.push(text);
            }
        }
    }
    // White space after the last link ends the text: it is dropped, as the end of the text
    // is.
    Ok(out)
}

/// Whether the text of a run may be printed over another: two letters or digits or more. A
/// lone mark (an accent drawn as a run of its own over its letter) belongs to its word, as
/// the reading order keeps it there.
fn overprints(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphanumeric()).count() >= 2
}

/// Whether the run `r` is another text than the run `before` it on its line, whose ink ends
/// at `ink_end`: it is printed over it (it starts [`OVERPRINT`] sizes or more left of that
/// end, with text of its own, see [`overprints`]), or it touches it (it starts less than
/// [`TOUCH`] sizes right of that end, or over it) in a larger size by a label step or in
/// another weight, the words that meet each of two letters or digits or more and one with a
/// letter: a label and its value set against it (`Name` and `Hinterhaus`). A smaller run after
/// a word is a raised part of it (`21st`), a word in pieces of one style is one word, and a
/// number in pieces (a change of font inside `1234,56`) is one number.
fn apart(before: Option<&Run>, r: &Run, ink_end: f32) -> bool {
    if r.x + OVERPRINT * r.size < ink_end && overprints(&r.text) {
        return true;
    }
    let Some(b) = before else {
        return false;
    };
    let (last, first) = (
        b.text.split_whitespace().next_back(),
        r.text.split_whitespace().next(),
    );
    let letters = |w: Option<&str>| w.is_some_and(|w| w.chars().any(char::is_alphabetic));
    r.x < ink_end + TOUCH * r.size.max(b.size)
        && (r.size >= crate::tables::LABEL_STEP * b.size || r.bold != b.bold)
        && last.is_some_and(overprints)
        && first.is_some_and(overprints)
        && (letters(last) || letters(first))
}

/// A run that starts less than this many sizes right of where the ink of the run before it
/// ends touches it (see [`apart`]): less than a space.
const TOUCH: f32 = 0.2;

/// A run that starts this many of its sizes or more left of where the ink of the run before it
/// on its line ends is printed over that run (see [`inlines`]): far more than a kerning pair
/// takes back.
const OVERPRINT: f32 = 0.5;

/// Whether `cur` goes on in the same paragraph as `prev`: below it, near enough and in the
/// same size, and not wholly right of it: a line that starts [`BESIDE_GAP`] sizes or more
/// after the ink of the line over it ends, and is not much wider than it, stands in another
/// column (an amount alone under a table in another column than the one over it), as text that
/// wraps goes on at the left.
fn continues(prev: &Line<'_>, cur: &Line<'_>) -> bool {
    let gap = prev.y - cur.y;
    let height = prev.h.max(cur.h).max(1.0);
    let ink = |l: &Line<'_>| {
        l.runs
            .iter()
            .filter(|r| !r.text.trim().is_empty())
            .fold((f32::MAX, f32::MIN), |(a, b), r| {
                (a.min(r.x), b.max(r.x + r.w.max(0.0)))
            })
    };
    let ((p0, end), (c0, c1)) = (ink(prev), ink(cur));
    // A line that is much wider than a short line over it is text going on after a label or
    // a marker (an indented continuation), not a cell of another column.
    // A word cut at the end of a line goes on at the left of the next, never right of where
    // the cut ends (`Page-` over a cell of the next row).
    let narrow = c1 - c0 <= BESIDE_WIDTH * (end - p0).max(prev.size) || word_cut(&prev.text);
    let beside = narrow && c0 >= end + BESIDE_GAP * prev.size.max(cur.size).max(1.0);
    gap > 0.0 && gap <= GAP_RATIO * height && (prev.size - cur.size).abs() <= SIZE_SLACK && !beside
}

/// A line that starts this many sizes or more right of where the line over it ends goes on
/// no paragraph of it (see [`continues`]): the white space of a cell gap, far over a space.
const BESIDE_GAP: f32 = 1.0;

/// A line beside the line over it (see [`BESIDE_GAP`]) is a cell of another column only when
/// it is at most this many times as wide as that line.
const BESIDE_WIDTH: f32 = 2.0;

/// The blocks of a running footer, in reading order. Footer lines that go on one another at
/// one left edge are one block. A footer set in columns (an address, a bank, a register entry,
/// a contact) is read column by column (ruling P6-X5: on furniture lines only): the ink of its
/// lines falls into columns apart by white space, and the parts of the lines in each column
/// are blocks of their own, when every block of three lines or more is an [`address_block`]
/// and one is. A line of its own in one piece (a page number centred over or under the
/// columns) takes no part in finding the columns: it would bridge their gutter. It is read whole, before the
/// columns when it stands before them in the reading order, else after them. Otherwise the
/// lines stay whole, as the reading order gives them. Each line of
/// a block is a line of its own: a footer lists facts (`USt-IdNr. DE000000000`,
/// `www.muster.example`), it is no prose to join; only a sentence that wraps goes on (see
/// [`footer_wrap`]).
fn footer_blocks<'a>(all: &[Line<'a>]) -> Vec<Vec<Line<'a>>> {
    let whole = || stacked(all.iter().map(|l| line(l.runs)).collect());
    let ink = |l: &Line<'a>| -> Vec<(f32, f32)> {
        l.runs
            .iter()
            .filter(|r| !r.text.trim().is_empty())
            .map(|r| (r.x, r.x + r.w))
            .collect()
    };
    // The lines of their own: the blocks of one line whose ink is one piece. A row of the
    // columns that stands alone (columns centred on one another) has gaps in it.
    let one_piece = |l: &Line<'a>| {
        let size = l.size.max(1.0);
        ink(l)
            .windows(2)
            .all(|w| w[1].0 - w[0].1 <= crate::order::PIECE_GAP * size)
    };
    let mut single = vec![false; all.len()];
    let mut k = 0;
    for b in stacked(all.iter().map(|l| line(l.runs)).collect()) {
        single[k] = b.len() == 1 && one_piece(&all[k]);
        k += b.len();
    }
    // The columns: the extents of the ink of the lines in blocks, merged where they overlap.
    let columns_of = |single: &[bool]| -> Vec<(f32, f32)> {
        let mut spans: Vec<(f32, f32)> = (0..all.len())
            .filter(|&i| !single[i])
            .flat_map(|i| ink(&all[i]))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut columns: Vec<(f32, f32)> = Vec::new();
        for (x0, x1) in spans {
            match columns.last_mut() {
                Some(c) if x0 < c.1 => c.1 = c.1.max(x1),
                _ => columns.push((x0, x1)),
            }
        }
        columns
    };
    // A line of its own over the gutter between two of those columns (a page number centred
    // over or under them) is read on its own. Any other is a line of the columns: the first
    // line of a column centred lower than its neighbours, or the lines of a short column.
    let found = columns_of(&single);
    for (i, l) in all.iter().enumerate() {
        if single[i] {
            let touched = found
                .iter()
                .filter(|c| ink(l).iter().any(|r| r.0 < c.1 && r.1 > c.0))
                .count();
            single[i] = touched >= 2;
        }
    }
    let columns = columns_of(&single);
    if columns.len() < 2 {
        return whole();
    }
    let Some(first) = single.iter().position(|s| !s) else {
        return whole();
    };
    let lines: Vec<&Line<'a>> = (0..all.len())
        .filter(|&i| !single[i])
        .map(|i| &all[i])
        .collect();
    let alone = |range: std::ops::Range<usize>| -> Vec<Vec<Line<'a>>> {
        range
            .filter(|&i| single[i])
            .map(|i| vec![line(all[i].runs)])
            .collect()
    };
    let column_of = |r: &Run| columns.partition_point(|c| c.1 <= r.x);
    // The part of each line in each column; a line whose parts of one column are not next to
    // each other keeps the lines whole.
    let mut parts: Vec<Vec<Line<'a>>> = (0..columns.len()).map(|_| Vec::new()).collect();
    for l in &lines {
        let mut start = 0;
        let mut seen: Vec<usize> = Vec::new();
        while start < l.runs.len() {
            let c = column_of(&l.runs[start]);
            let mut end = start + 1;
            while end < l.runs.len() && column_of(&l.runs[end]) == c {
                end += 1;
            }
            if seen.contains(&c) {
                return whole();
            }
            seen.push(c);
            let part = line(&l.runs[start..end]);
            if !part.text.is_empty() {
                parts[c.min(columns.len() - 1)].push(part);
            }
            start = end;
        }
    }
    let blocks: Vec<Vec<Line<'a>>> = parts.into_iter().flat_map(stacked).collect();
    let tall: Vec<&Vec<Line<'a>>> = blocks.iter().filter(|b| b.len() >= 3).collect();
    let shaped =
        |b: &Vec<Line<'a>>| address_block(&b.iter().map(|l| l.text.as_str()).collect::<Vec<_>>());
    if tall.is_empty() || !tall.iter().all(|b| shaped(b)) {
        return whole();
    }
    let mut out = alone(0..first);
    out.extend(blocks);
    out.extend(alone(first..all.len()));
    out
}

/// Lines in reading order cut into blocks of lines that go on one another at one left edge.
fn stacked(lines: Vec<Line<'_>>) -> Vec<Vec<Line<'_>>> {
    let mut out: Vec<Vec<Line<'_>>> = Vec::new();
    for l in lines {
        match out.last_mut() {
            Some(b) if continues(&b[b.len() - 1], &l) && (l.x - b[0].x).abs() <= EDGE => {
                b.push(l);
            }
            _ => out.push(vec![l]),
        }
    }
    out
}

/// Whether the footer line `next` goes on the sentence of `line` above it at its edge: `line`
/// ends with no sentence end, comma or colon, and `next` starts with a word in small letters
/// that is no web or mail address (`www.muster.example`, `info@muster.example` are items of
/// their own).
fn footer_wrap(line: &Line<'_>, next: &Line<'_>) -> bool {
    let open = !line.text.ends_with(['.', '!', '?', ',', ':', ';']);
    let first = next.text.split_whitespace().next().unwrap_or("");
    let address = first.contains('@')
        || first.contains('/')
        || first.trim_end_matches(['.', ',']).contains('.');
    open && first.chars().next().is_some_and(char::is_lowercase) && !address
}

/// A bold line under plain lines stands this many times farther from them than their pitch
/// when it is a title of its own.
const TITLE_STEP: f32 = 1.1;

/// Line heights one line of text steps down by, for a block of one line that has no pitch of
/// its own.
const LEADING: f32 = 1.2;

/// Whether the last of `lines` is a title under the block of the others: all its ink is bold,
/// none of theirs is, and it stands farther under them than [`TITLE_STEP`] times their pitch
/// (or than the leading of one line, see [`LEADING`], under a block of one line). A bold
/// title of the size of the sender lines under them (`Rechnung` under the sender of a letter)
/// is no line of the sender's block; a bold line at the block's own pitch is.
fn bold_title_after(lines: &[Line<'_>]) -> bool {
    let [above @ .., prev, cur] = lines else {
        return false;
    };
    let inked = |l: &Line<'_>| {
        l.runs
            .iter()
            .filter(|r| !r.text.trim().is_empty())
            .map(|r| r.bold)
            .collect::<Vec<bool>>()
    };
    let (bold, plain) = (inked(cur), inked(prev));
    if bold.is_empty() || !bold.iter().all(|b| *b) || plain.iter().any(|b| *b) {
        return false;
    }
    if above.iter().any(|l| inked(l).iter().any(|b| *b)) {
        return false;
    }
    let pitch = match above.last() {
        Some(a) => a.y - prev.y,
        None => LEADING * prev.size,
    };
    pitch > 0.0 && prev.y - cur.y > TITLE_STEP * pitch
}

/// The marker of a list line: `(ordered, bytes of marker and the space after it)`.
fn list_marker(text: &str) -> Option<(bool, usize)> {
    let first = text.chars().next()?;
    if first == '\u{2022}' {
        let rest = &text[first.len_utf8()..];
        return Some((false, text.len() - rest.trim_start().len()));
    }
    if matches!(first, '-' | '*' | '\u{2013}') {
        let rest = &text[first.len_utf8()..];
        // Two padding characters or more after it: a mark set into a field of a form (a
        // starred amount, a dash for none), no bullet.
        let pad = rest.chars().take_while(|c| c.is_whitespace()).count();
        // A star that a lone star later on the line closes encloses an item (`* A123456B * x/y`,
        // a code set between stars): no bullet.
        let closed = first == '*' && rest.split_whitespace().any(|w| w == "*");
        if pad == 1 && !closed {
            return Some((false, text.len() - rest.trim_start().len()));
        }
        return None;
    }
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = &text[digits..];
    let after = rest.strip_prefix(['.', ')'])?;
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    Some((true, text.len() - after.trim_start().len()))
}

/// Runs without the list marker that takes the first `cut` bytes of their trimmed text.
fn without_marker(runs: &[&Run], cut: usize) -> Vec<Run> {
    let mut runs: Vec<Run> = runs.iter().map(|r| (*r).clone()).collect();
    let mut left = cut;
    for (k, r) in runs.iter_mut().enumerate() {
        // White space at the start of a run is dropped; after the first run it is part of
        // the cut (the space between marker and text).
        let lead = r.text.len() - r.text.trim_start().len();
        r.text.drain(..lead);
        if k > 0 {
            left = left.saturating_sub(lead);
        }
        let take = (0..=left.min(r.text.len()))
            .rev()
            .find(|t| r.text.is_char_boundary(*t))
            .unwrap_or(0);
        r.text.drain(..take);
        left -= take;
        if left == 0 || !r.text.is_empty() {
            break;
        }
    }
    runs.retain(|r| !r.text.is_empty());
    runs
}

/// The losses for a page's grids, header bands and blocks in columns refused without a note:
/// one for each kind, naming why, by reason in reading order.
fn refused_quietly(
    page: u32,
    mut quiet: Vec<(usize, &'static str, tables::Refused)>,
) -> Vec<String> {
    quiet.sort_by_key(|q| q.0);
    let mut out = Vec::new();
    for (kind, one, many, tail, tails) in [
        (
            tables::Refused::Grid,
            "a grid of drawn rules",
            "grids of drawn rules",
            "its text is read without the grid",
            "their text is read without the grids",
        ),
        (
            tables::Refused::Band,
            "a header band",
            "header bands",
            "its text is read without the band",
            "their text is read without the bands",
        ),
        (
            tables::Refused::Region,
            "a block of text in columns",
            "blocks of text in columns",
            "its text stays paragraphs",
            "their text stays paragraphs",
        ),
    ] {
        let why: Vec<&str> = quiet.iter().filter(|q| q.2 == kind).map(|q| q.1).collect();
        match why.as_slice() {
            [] => {}
            [why] => out.push(format!(
                "{one} on page {page} is not read as a table: {why}; {tail}"
            )),
            _ => {
                // One reason for all is said once, without a count.
                let reasons = if why.iter().all(|w| *w == why[0]) {
                    tables::plural(why[0]).to_string()
                } else {
                    tables::counted(why.iter().copied())
                };
                out.push(format!(
                    "{} {many} on page {page} are not read as tables: {reasons}; {tails}",
                    why.len()
                ));
            }
        }
    }
    out
}

/// Lays out an untagged page: headings by size, lists by marker, paragraphs by gaps. A line
/// in one of the page's `text_sizes`, or less than [`HEADING_RATIO`] times the largest of
/// them, is no heading.
fn untagged(flow: &[Run], slots: &[Slot], text_sizes: &[f32], cx: &Context<'_>, out: &mut Built) {
    let heading_level = heading_of(cx.style, text_sizes);
    let all = lines(flow);
    // The block of its own a line stands in, if the table pass read one (see
    // [`Slot::Block`]): a line goes on the line before it only in the same block. A line's
    // runs are a slice of `flow`, so its first run's place is where the slice starts.
    let block_of = |l: &Line<'_>| -> Option<usize> {
        let at = (l.runs.as_ptr() as usize - flow.as_ptr() as usize) / std::mem::size_of::<Run>();
        match slots.get(at) {
            Some(Slot::Block(b)) => Some(*b),
            _ => None,
        }
    };
    let block: Vec<Option<usize>> = all.iter().map(block_of).collect();
    let continues =
        |prev: &Line<'_>, cur: &Line<'_>| continues(prev, cur) && block_of(prev) == block_of(cur);
    // A number of three digits or more that opens a line in the middle of a sentence is part
    // of the sentence ("31 CFR 209 and/or / 210. The information"), not a list item.
    let marker = |k: usize| {
        if padded_mark(&all[k]) {
            return None;
        }
        let found = list_marker(&all[k].text)?;
        let digits = all[k].text.chars().take_while(char::is_ascii_digit).count();
        let mid_sentence = k > 0
            && continues(&all[k - 1], &all[k])
            && !all[k - 1].text.ends_with(['.', ':', ';', '!', '?']);
        (!(found.0 && digits >= 3 && mid_sentence)).then_some(found)
    };
    let mut i = 0;
    let mut open_list: Option<Option<usize>> = None;
    while i < all.len() && out.stop.is_none() {
        let l = &all[i];
        if l.text.is_empty() {
            i += 1;
            continue;
        }
        if let Some(level) = heading_level(l) {
            let (j, heading) = heading_block(&all, i, level, &heading_level, cx.style.body_size);
            let j = (i + 1..j).find(|&k| block[k] != block[i]).unwrap_or(j);
            let block = &all[i..j];
            let runs: Vec<&Run> = block.iter().flat_map(|l| l.runs.iter()).collect();
            if heading {
                out.heading(level, runs, cx);
            } else {
                out.broken_paragraph(runs, &line_breaks(&all, i, j), None, cx);
            }
            open_list = None;
            i = j;
            continue;
        }
        if let Some((ordered, cut)) = marker(i) {
            let owned = without_marker(&l.runs.iter().collect::<Vec<_>>(), cut);
            let mut extra: Vec<&Run> = Vec::new();
            let mut j = i + 1;
            while j < all.len()
                && heading_level(&all[j]).is_none()
                && marker(j).is_none()
                && continues(&all[j - 1], &all[j])
                && all[j].x > l.x + SIZE_SLACK
            {
                extra.extend(all[j].runs.iter());
                j += 1;
            }
            let mut runs: Vec<&Run> = owned.iter().collect();
            runs.extend(extra);
            // An item follows the item before it when the gap is not over twice the usual.
            let joins = i > 0 && open_list.is_some() && list_gap_ok(&all[i - 1], l);
            if !joins {
                open_list = None;
            }
            out.item(None, ordered, runs, cx, &mut open_list);
            i = j;
            continue;
        }
        open_list = None;
        let mut runs: Vec<&Run> = l.runs.iter().collect();
        let mut j = i + 1;
        while j < all.len()
            && heading_level(&all[j]).is_none()
            && marker(j).is_none()
            && continues(&all[j - 1], &all[j])
            && !bold_title_after(&all[i..=j])
        {
            runs.extend(all[j].runs.iter());
            j += 1;
        }
        let breaks = line_breaks(&all, i, j);
        out.broken_paragraph(runs, &breaks, None, cx);
        i = j;
    }
}

/// The heading level of an untagged line on a page whose text is set in `text_sizes`: none
/// for a line in one of them, or less than [`HEADING_RATIO`] times the largest of them.
fn heading_of<'s>(
    style: &'s Style,
    text_sizes: &'s [f32],
) -> impl Fn(&Line<'_>) -> Option<u8> + 's {
    let largest = text_sizes.last().copied();
    move |l: &Line<'_>| {
        style.heading_level(l).filter(|_| {
            !text_sizes.iter().any(|s| size_key(*s) == size_key(l.size))
                && largest.is_none_or(|b| l.size >= b * HEADING_RATIO)
        })
    }
}

/// The heading that starts at line `i` of `all`, a line of `level`: the end of its lines
/// (each after the first of `level` and going on the one before), and whether they are a
/// heading. Bold lines a little larger than the body (`body_size`) that read as an address
/// or as sentences are a paragraph: the address of an invoice, its closing words.
fn heading_block(
    all: &[Line<'_>],
    i: usize,
    level: u8,
    heading_level: &dyn Fn(&Line<'_>) -> Option<u8>,
    body_size: f32,
) -> (usize, bool) {
    let mut j = i + 1;
    while j < all.len() && heading_level(&all[j]) == Some(level) && continues(&all[j - 1], &all[j])
    {
        j += 1;
    }
    let first = &all[i];
    let small = first.size < HEADING_PLAIN_RATIO * body_size;
    let block = &all[i..j];
    // Lines that keep a break between them (items, fields) are no heading but a paragraph with
    // its breaks, when they are a little larger than the body or three lines or more (a column
    // of items, also where a box cuts it). Two large short lines are a title over two lines.
    // The lines of the same column over the block (a box may have cut it off them), two at
    // most: enough to make it three lines.
    let mut over = i;
    while over > 0
        && i - over < 2
        && heading_level(&all[over - 1]) == Some(level)
        && continues(&all[over - 1], &all[over])
    {
        over -= 1;
    }
    let column = &all[over..j];
    let items = block_listed(column);
    // Lines that each stand in pieces apart by a cell gap (cells of columns) are no title; lines
    // set full (justified: words equal gaps apart, both outer edges shared) are no cells.
    let full = {
        let edges = ink_edges(block);
        let tol = 0.5
            * block
                .iter()
                .map(|l| l.size)
                .fold(f32::MAX, f32::min)
                .max(1.0);
        let even = |l: &Line<'_>| {
            let mut end: Option<f32> = None;
            let gaps: Vec<f32> = l
                .runs
                .iter()
                .filter(|r| !r.text.trim().is_empty())
                .filter_map(|r| {
                    let gap = end.map(|e| r.x - e);
                    end = Some(r.x + r.w);
                    gap
                })
                .collect();
            let (lo, hi) = gaps
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), &g| (a.min(g), b.max(g)));
            hi - lo <= tol
        };
        block.iter().all(even)
            && edges
                .iter()
                .all(|e| (e.0 - edges[0].0).abs() <= tol && (e.1 - edges[0].1).abs() <= tol)
    };
    // A sentence that goes on over its lines (a line opening with a small letter, or a last
    // line ending with a full stop) is no row of cells either.
    let prose = block
        .iter()
        .skip(1)
        .any(|l| l.text.starts_with(char::is_lowercase))
        || block.last().is_some_and(|l| l.text.ends_with('.'));
    let pieced = |l: &Line<'_>| !full && !prose && cell_gaps(l) > 0;
    let apart = (1..block.len()).any(|k| lines_apart(block, k, items))
        || wraps(column, &ink_edges(column)).contains(&false)
        || (block.len() >= 2 && block.iter().all(pieced));
    if (small || column.len() >= 3 || block.iter().all(pieced)) && apart {
        return (j, false);
    }
    let texts: Vec<&str> = block.iter().map(|l| l.text.as_str()).collect();
    let one_edge = block.iter().all(|b| (b.x - first.x).abs() <= EDGE);
    let paragraph = small && ((address_shaped(&texts) && one_edge) || sentences(block));
    (j, !paragraph)
}

/// Whether the line `k` of `block` (a paragraph or a heading) starts after a line break
/// whatever the width of the block, as no wrapped text is made of it and the line over it:
/// the block's lines are short items (an address, a column of labels, of values, or of fields
/// of one face; see [`listed`]: at most a few words each and few small words) and the two
/// lines share their left edge, or both lines are a label and its value in the same two faces
/// (see [`field_line`]) and not long lines of prose.
/// `items` is whether the block is [`listed`] (see [`block_listed`]).
fn lines_apart(block: &[Line<'_>], k: usize, items: bool) -> bool {
    let (a, b) = (&block[k - 1], &block[k]);
    let left = |l: &Line<'_>| {
        l.runs
            .iter()
            .filter(|r| !r.text.trim().is_empty())
            .map(|r| r.x)
            .fold(f32::MAX, f32::min)
    };
    let items = items && (left(a) - left(b)).abs() <= EDGE;
    let words = |l: &Line<'_>| l.text.split_whitespace().count();
    let fields = field_line(a).is_some_and(|f| {
        field_line(b).is_some_and(|g| same_face(f[0], g[0]) && same_face(f[1], g[1]))
    }) && words(a).max(words(b)) <= FIELD_WORDS;
    items || fields
}

/// Whether the lines of `block` are [`listed`]: short items.
fn block_listed(block: &[Line<'_>]) -> bool {
    listed(&block.iter().map(|l| l.text.as_str()).collect::<Vec<_>>())
}

/// A line of more words than this is a line of prose, though it starts plain and ends bold (see
/// [`lines_apart`]).
const FIELD_WORDS: usize = 8;

/// A line is short when the next first word fits after it with this share of the widest
/// line to spare.
const SHORT_ROOM: f32 = 0.15;

/// Line ends this close, in points, are one edge.
const EDGE: f32 = 1.0;

/// The lines of a paragraph (by page and line, sorted) that start after a line break: the
/// first word of the line would have fit at the end of the line before within the widest
/// line with [`SHORT_ROOM`] of it to spare (and that line does not end in a hyphen), or the
/// paragraph is set flush right or centred and the two lines share a right edge or a centre
/// but not a left edge, or the
/// paragraph is a block of short lines each of which after the first starts with a capital or
/// a digit, none ends in a hyphen or a comma, the lines do not read as sentences, share their
/// left edge and have the shape of an address or a list (list markers, or at most
/// [`ADDRESS_WORDS`] words a line), or the two lines are each a label and its value in the
/// same two faces (see [`field_line`]). How wide the block is does not matter: a long name in an
/// address is still a line of it. Prose that fills its measure keeps joining, also where
/// every line starts with a capital.
fn line_breaks(all: &[Line<'_>], i: usize, j: usize) -> Vec<(u32, u32)> {
    let lines = &all[i..j];
    if lines.len() < 2 {
        return Vec::new();
    }
    // The column the block stands in: up to two lines over and under it that go on it but
    // for a box's edge (see [`Slot::Block`]), which a box may have cut off it. Where the
    // column shows that its lines are lines of their own, so are the block's.
    let (mut lo, mut hi) = (i, j);
    while lo > 0 && i - lo < 2 && continues(&all[lo - 1], &all[lo]) {
        lo -= 1;
    }
    while hi < all.len() && hi - j < 2 && continues(&all[hi - 1], &all[hi]) {
        hi += 1;
    }
    let column = &all[lo..hi];
    let around = wraps(column, &ink_edges(column));
    let edges = ink_edges(lines);
    let right = edges.iter().map(|e| e.1).fold(f32::MIN, f32::max);
    let left = edges.iter().map(|e| e.0).fold(f32::MAX, f32::min);
    let short = lines[1..].iter().all(|l| {
        l.text
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
    }) && lines
        .iter()
        .all(|l| !word_cut(&l.text) && !l.text.ends_with(','))
        && !sentences(lines)
        && edges.iter().all(|e| (e.0 - left).abs() <= EDGE)
        && listed(&lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>());
    // Lines set flush right or centred: most pairs of lines in a row, and at least two,
    // share a right edge or a centre but not a left edge. One indented first line of
    // justified prose is not that.
    let same_left = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() <= EDGE;
    let aligned_pair = |a: (f32, f32), b: (f32, f32)| {
        ((a.1 - b.1).abs() <= EDGE || ((a.0 + a.1) / 2.0 - (b.0 + b.1) / 2.0).abs() <= EDGE)
            && !same_left(a, b)
    };
    let pairs = edges
        .windows(2)
        .filter(|w| aligned_pair(w[0], w[1]))
        .count();
    // Two lines centred on one another (their right edges apart) are a centred block too: an
    // indented first line of prose shares the right edge, not the centre.
    let centred_pair = edges.len() == 2 && {
        let (a, b) = (edges[0], edges[1]);
        ((a.0 + a.1) / 2.0 - (b.0 + b.1) / 2.0).abs() <= EDGE && (a.1 - b.1).abs() > EDGE
    };
    let set_aligned = (pairs >= 2 && 2 * pairs >= edges.len() - 1) || centred_pair;
    let items = block_listed(lines);
    let wraps = wraps(lines, &edges);
    // Whether a line break stands before each line.
    let mut broken = vec![false; lines.len()];
    for k in 0..lines.len() - 1 {
        let (a, b) = (edges[k], edges[k + 1]);
        let next = &lines[k + 1];
        let word = first_word(next);
        // A short line: it ends well before the widest line, so far that the next line's
        // first word and a good share of the measure more would still fit. Ragged prose
        // misses a word by a little. A word cut at the line end goes on in the next line.
        let room = SHORT_ROOM * (right - left);
        let fits = a.1 + 0.25 * next.size + word + room <= right && !word_cut(&lines[k].text);
        let aligned = set_aligned && aligned_pair(a, b);
        let apart = !wraps[k] || !around[i - lo + k];
        broken[k + 1] = fits || aligned || short || apart || lines_apart(lines, k + 1, items);
    }
    // A line of words after the end of a sentence that ends no sentence and stands on a line of
    // its own (a break after it, before a line of the block) is no text the sentence before goes
    // on in: a field line under prose (`Schaeden sofort.` over `Kunde Muster GmbH`). A line of
    // punctuation alone (a dash that ends a verse) still ends the line before.
    let stop = |l: &Line<'_>| l.text.ends_with(['.', '!', '?']);
    let words = |l: &Line<'_>| l.text.chars().any(char::is_alphanumeric);
    for k in 1..lines.len().saturating_sub(1) {
        if stop(&lines[k - 1]) && !stop(&lines[k]) && words(&lines[k]) && broken[k + 1] {
            broken[k] = true;
        }
    }
    let mut out: Vec<(u32, u32)> = (1..lines.len())
        .filter(|&k| broken[k])
        .map(|k| (lines[k].runs[0].page, lines[k].runs[0].line))
        .collect();
    out.sort_unstable();
    out
}

/// The ends of each line's ink.
fn ink_edges(lines: &[Line<'_>]) -> Vec<(f32, f32)> {
    lines
        .iter()
        .map(|l| {
            let inked = l.runs.iter().filter(|r| !r.text.trim().is_empty());
            let x0 = inked.clone().map(|r| r.x).fold(f32::MAX, f32::min);
            let x1 = inked.map(|r| r.x + r.w).fold(f32::MIN, f32::max);
            (x0, x1)
        })
        .collect()
}

/// The gaps of a cell between the runs with ink of `line` (see [`crate::order::PIECE_GAP`]): the
/// white space between cells of columns, far over a word space.
fn cell_gaps(line: &Line<'_>) -> usize {
    let mut end: Option<f32> = None;
    line.runs
        .iter()
        .filter(|r| !r.text.trim().is_empty())
        .filter(|r| {
            let gap = end.is_some_and(|e| r.x - e > crate::order::PIECE_GAP * r.size);
            end = Some(r.x + r.w);
            gap
        })
        .count()
}

/// Whether the cell gaps of `line` (see [`cell_gaps`]) are all of one width, within half a
/// size: the word spaces of a line set full (justified), no gutters of cells.
fn set_full(line: &Line<'_>) -> bool {
    let mut end: Option<f32> = None;
    let gaps: Vec<f32> = line
        .runs
        .iter()
        .filter(|r| !r.text.trim().is_empty())
        .filter_map(|r| {
            let gap = end
                .map(|e| r.x - e)
                .filter(|g| *g > crate::order::PIECE_GAP * r.size);
            end = Some(r.x + r.w);
            gap
        })
        .collect();
    let (lo, hi) = gaps
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &g| (a.min(g), b.max(g)));
    !gaps.is_empty() && hi - lo <= 0.5 * line.size.max(1.0)
}

/// Whether the last run with ink of `line` stands after a cell gap (see [`cell_gaps`]): it
/// is a cell of its own.
fn last_cell(line: &Line<'_>) -> bool {
    let mut inked = line.runs.iter().rev().filter(|r| !r.text.trim().is_empty());
    match (inked.next(), inked.next()) {
        (Some(last), Some(before)) => {
            last.x - (before.x + before.w) > crate::order::PIECE_GAP * last.size
        }
        _ => false,
    }
}

/// The width of the first word of `line`, by its share of the first run with text.
fn first_word(line: &Line<'_>) -> f32 {
    line.runs
        .iter()
        .find(|r| !r.text.trim().is_empty())
        .map_or(f32::MAX, |r| {
            let text = r.text.trim_start();
            let chars = text.chars().count().max(1) as f32;
            let first = text.split_whitespace().next().unwrap_or("");
            r.w * first.chars().count() as f32 / chars
        })
}

/// For each line of `lines` but the last (with the ends `edges` of their ink), whether it may
/// go on in the next line as wrapped text: the positive evidence that a line was broken where
/// it is only because its measure was full (Task 15b part 2e-2, ruling P6-X: a line of its own
/// read as wrapped text mixes two fields). A line that ends in a hyphen or a comma, or whose
/// next line starts with a small letter or holds punctuation alone, goes on. Otherwise no line goes on when
/// - the lines each end with an exclamation or a question mark: sentences of their own (a
///   slogan, a greeting), or each opens with a star: starred items;
/// - the lines read as no sentences and one that ends no sentence, or the first line, would
///   have taken the next line's first word in its own face within the widest line, with
///   [`SPARE`] sizes to spare:
///   the lines were not filled to a measure, they are lines of their own (fields, items,
///   address lines) of any length;
/// - the lines read as no sentences and one ends in a number (an amount, a date, a customer
///   number) over a line that opens with a capital, or in an amount over a line of an amount
///   alone: a column of values.
///
/// In lines that read as sentences, only a row of cells that ends in a number over a line
/// without cells ends there. Prose goes on, whatever its last word (a number over a noun).
fn wraps(lines: &[Line<'_>], edges: &[(f32, f32)]) -> Vec<bool> {
    let n = lines.len();
    if n < 2 {
        return Vec::new();
    }
    let ends_sentence = |l: &Line<'_>| l.text.ends_with(['.', '!', '?', ':']);
    let cut = |l: &Line<'_>| word_cut(&l.text) || l.text.ends_with(',');
    let lower = |l: &Line<'_>| l.text.chars().next().is_some_and(char::is_lowercase);
    // A line of punctuation alone (a dash that ends a verse, set on a line of its own) is no
    // line of its own: it ends the line before.
    let marks = |l: &Line<'_>| !l.text.chars().any(char::is_alphanumeric);
    let goes_on: Vec<bool> = (0..n - 1)
        .map(|k| cut(&lines[k]) || lower(&lines[k + 1]) || marks(&lines[k + 1]))
        .collect();
    let exclaims = lines.iter().all(|l| l.text.ends_with(['!', '?']));
    let starred = lines.iter().all(|l| l.text.starts_with('*'));
    let right = edges.iter().map(|e| e.1).fold(f32::MIN, f32::max);
    // A title line in bold over plain text is no line of that text.
    let one_face = |a: &Line<'_>, b: &Line<'_>| {
        let last = a.runs.iter().rev().find(|r| !r.text.trim().is_empty());
        let first = b.runs.iter().find(|r| !r.text.trim().is_empty());
        match (last, first) {
            (Some(x), Some(y)) => same_face(x, y),
            (x, y) => x.is_none() && y.is_none(),
        }
    };
    let unfilled = !sentences(lines)
        && (0..n - 1).any(|k| {
            let l = &lines[k];
            // A short line that ends a sentence ends a paragraph in prose, after a line that
            // wrapped into it; as the first line of the block it is a line of its own.
            (!ends_sentence(l) || k == 0)
                && !goes_on[k]
                && one_face(l, &lines[k + 1])
                && edges[k].1 + (0.25 + SPARE) * lines[k + 1].size + first_word(&lines[k + 1])
                    <= right
        });
    // A line that ends in a number (an amount, a date, a customer number), and no sentence,
    // over a line that opens with a capital ends a value: the next line is a field of its own,
    // and so is every line of a block that holds such a line and no sentence. So does a line
    // that ends in an amount over a line of an amount alone (a total under it).
    let value_end = |k: usize| {
        let l = &lines[k];
        let next = &lines[k + 1].text;
        let total = || {
            data_only(next)
                && ends_in_amount(next)
                && (ends_in_amount(&l.text) || signed_amount(&l.text))
        };
        !ends_sentence(l)
            && !goes_on[k]
            && (next.chars().next().is_some_and(char::is_uppercase) || total())
            && l.text.split_whitespace().next_back().is_some_and(|w| {
                w.chars().any(|c| c.is_ascii_digit())
                    && w.chars().all(|c| c.is_ascii_digit() || ".,/-".contains(c))
            })
    };
    let values = !sentences(lines) && (0..n - 1).any(value_end);
    if exclaims || starred || unfilled || values {
        goes_on
    } else {
        // In prose a line that ends in a number goes on (`mit 2` over `Prozent Skonto`), but a
        // line that ends in a row of cells (two cell gaps or more of other widths than the word
        // spaces of a line set full, the number a cell of its own after one) ends with that row
        // (`79-84 70-85` over `Preparation work`).
        (0..n - 1)
            .map(|k| {
                !(value_end(k)
                    && cell_gaps(&lines[k]) >= 2
                    && last_cell(&lines[k])
                    && !set_full(&lines[k]))
            })
            .collect()
    }
}

/// Whether `text` ends in a hyphen that cuts a word: not the minus after an amount with
/// cents (`10,00-`, `1.234,56-`, the way a deduction or a credit is written), which ends the
/// amount, not a word.
fn word_cut(text: &str) -> bool {
    text.ends_with('-') && !signed_amount(text)
}

/// Whether the last word of `text` is an amount with a decimal part of two digits and a minus
/// after it (`10,00-`, `1.234,56-`).
pub(crate) fn signed_amount(text: &str) -> bool {
    let Some(word) = text.split_whitespace().next_back() else {
        return false;
    };
    let Some(number) = word.strip_suffix('-') else {
        return false;
    };
    // Digits and marks only (all ASCII, so the byte offsets below are char bounds), a digit
    // first, and a mark before two digits last.
    let b = number.as_bytes();
    b.len() >= 4
        && b.iter()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b','))
        && b[0].is_ascii_digit()
        && matches!(b[b.len() - 3], b'.' | b',')
        && b[b.len() - 2..].iter().all(u8::is_ascii_digit)
}

/// A line has room for the next line's first word when this many sizes of the widest line
/// are left over after it and a space (see [`wraps`]): more than the error of a word's
/// width taken from its share of a run.
const SPARE: f32 = 1.0;

/// The faces of a line that is a label and its value: the face its ink starts in and the other
/// face it ends in (a plain label, then a bold value), each as weight, slant, family and size.
/// Two lines one under the other that are each a label and its value in the same two faces are
/// two fields, never one line of text going on (see [`line_breaks`]).
fn field_line<'a>(l: &Line<'a>) -> Option<[&'a Run; 2]> {
    let mut inked = l.runs.iter().filter(|r| !r.text.trim().is_empty());
    let first = inked.next()?;
    let last = inked.next_back()?;
    (!same_face(first, last)).then_some([first, last])
}

/// Whether two runs are set in one face: weight, slant and family, and sizes less than
/// [`crate::tables::LABEL_STEP`] apart (relative to the size, ruling P6-Y).
fn same_face(a: &Run, b: &Run) -> bool {
    let (lo, hi) = (a.size.min(b.size), a.size.max(b.size));
    (a.bold, a.italic, a.family) == (b.bold, b.italic, b.family)
        && hi < crate::tables::LABEL_STEP * lo.max(f32::MIN_POSITIVE)
}

/// Whether the lines read as sentences: one holds a sentence end with more text after it, or
/// the last ends a sentence after a word in small letters. One pass over each line.
fn sentences(lines: &[Line<'_>]) -> bool {
    // A stop after a word of three letters or more, then a space and a capital: "ul. 18" or
    // "Nr. 5" are no sentence ends.
    let inner = lines.iter().any(|l| {
        let chars: Vec<char> = l.text.chars().collect();
        let mut letters = 0usize;
        for (k, &c) in chars.iter().enumerate() {
            if c == ' ' {
                letters = 0;
            } else if matches!(c, '.' | '!' | '?') {
                if letters >= 3
                    && chars.get(k + 1) == Some(&' ')
                    && chars.get(k + 2).is_some_and(|n| n.is_uppercase())
                {
                    return true;
                }
            } else if c.is_alphabetic() {
                letters += 1;
            }
        }
        false
    });
    let last = lines.last().map_or("", |l| l.text.as_str());
    let mut tail = last.chars().rev();
    let ends =
        matches!(tail.next(), Some('.' | '!' | '?')) && tail.next().is_some_and(char::is_lowercase);
    inner || ends
}

fn list_gap_ok(prev: &Line<'_>, cur: &Line<'_>) -> bool {
    let gap = prev.y - cur.y;
    gap > 0.0 && gap <= 2.0 * GAP_RATIO * prev.h.max(cur.h).max(1.0)
}

/// Where a run belongs in a tagged file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// Not tagged, or no block role above it.
    Free,
    Heading(u8, usize),
    /// A paragraph-like element, with a role for notes and captions.
    Para(usize, Option<&'static str>),
    Item {
        list: usize,
        item: usize,
    },
    Cell {
        table: usize,
        cell: usize,
    },
    /// Untagged text the table pass read as a block of its own (a cell of a row of fields read
    /// cell by cell): its lines go on no line of another block.
    Block(usize),
}

/// Roles that only group blocks or mark text inside a block.
fn is_wrapper(role: &str) -> bool {
    matches!(
        role,
        "Document"
            | "DocumentFragment"
            | "Part"
            | "Art"
            | "Sect"
            | "Div"
            | "Aside"
            | "NonStruct"
            | "Private"
            | "Span"
            | "Link"
            | "Annot"
            | "Form"
            | "Reference"
            | "BibEntry"
            | "Quote"
            | "Em"
            | "Strong"
            | "Sub"
            | "Ruby"
            | "RB"
            | "RT"
            | "RP"
            | "Warichu"
            | "WT"
            | "WP"
            | "TOC"
            | "Index"
            | "L"
            | "Table"
            | "THead"
            | "TBody"
            | "TFoot"
            | "TR"
    )
}

fn heading_level(role: &str) -> Option<u8> {
    match role {
        "H" | "Title" => Some(1),
        _ => {
            let n: u8 = role.strip_prefix('H')?.parse().ok()?;
            (1..=6).contains(&n).then_some(n)
        }
    }
}

/// What the parents of element `start` say about a run: its slot and the element whose
/// ActualText replaces it, if any.
fn classify(tree: &StructTree, start: usize) -> (Slot, Option<usize>) {
    let elems = &tree.elems;
    let mut block = None;
    let mut item = None;
    let mut list = None;
    let mut cell = None;
    let mut table = None;
    let mut actual = None;
    let mut at = Some(start);
    // Parents come before children, so a walk is at most as long as the tree.
    for _ in 0..elems.len() {
        let Some(i) = at else { break };
        let Some(e) = elems.get(i) else { break };
        if e.actual_text.is_some() {
            actual = Some(i);
        }
        match e.role.as_str() {
            "LI" if item.is_none() => item = Some(i),
            "L" if list.is_none() => list = Some(i),
            "TD" | "TH" if cell.is_none() => cell = Some(i),
            "Table" if table.is_none() => table = Some(i),
            r if block.is_none() && !is_wrapper(r) => block = Some(i),
            _ => {}
        }
        at = e.parent;
    }
    let slot = if let (Some(cell), Some(table)) = (cell, table) {
        Slot::Cell { table, cell }
    } else if let Some(item) = item {
        Slot::Item {
            list: list.unwrap_or(item),
            item,
        }
    } else if let Some(b) = block {
        let role = elems[b].role.as_str();
        match heading_level(role) {
            Some(level) => Slot::Heading(level, b),
            None => Slot::Para(
                b,
                match role {
                    "Note" => Some("footnote"),
                    "Caption" => Some("caption"),
                    _ => None,
                },
            ),
        }
    } else {
        Slot::Free
    };
    (slot, actual)
}

/// The slot of every run of a tagged page, with the ActualText of an element written into the
/// first of its runs (the others are dropped). The runs are moved, not copied.
fn classified(runs: Vec<Run>, tree: &StructTree, cx: &Context<'_>) -> (Vec<Run>, Vec<Slot>) {
    let mut slots = Vec::with_capacity(runs.len());
    let mut kept = Vec::with_capacity(runs.len());
    // ActualText of an element replaces the text of all its runs: the first run carries it.
    let mut replaced: HashSet<usize> = HashSet::new();
    for mut r in runs {
        let (slot, actual) = match r.mcid().and_then(|m| (cx.roles)(m)) {
            Some(e) if e < tree.elems.len() => classify(tree, e),
            _ => (Slot::Free, None),
        };
        match actual {
            Some(a) if !replaced.insert(a) => continue,
            Some(a) => {
                let space = if r.text.starts_with(' ') { " " } else { "" };
                r.text = format!(
                    "{space}{}",
                    tree.elems[a].actual_text.as_deref().unwrap_or("")
                );
            }
            None => {}
        }
        slots.push(slot);
        kept.push(r);
    }
    (kept, slots)
}

/// Lays out runs of a tagged page with their slots: runs of one element form one block, in
/// reading order. Table cells are laid out as tables before; a cell run left here is text.
fn tagged(flow: &[Run], slots: &[Slot], cx: &Context<'_>, out: &mut Built) {
    let mut open_list: Option<Option<usize>> = None;
    let mut current: Vec<&Run> = Vec::new();
    let mut current_slot = Slot::Free;
    // The flow index of the last run in `current`.
    let mut current_last = 0;
    let all_lines = lines(flow);
    // The line of each run, to test gaps between free runs.
    let mut line_of = Vec::with_capacity(flow.len());
    for (n, l) in all_lines.iter().enumerate() {
        line_of.extend(std::iter::repeat_n(n, l.runs.len()));
    }
    let flush =
        |out: &mut Built, current: &mut Vec<&Run>, slot: Slot, open: &mut Option<Option<usize>>| {
            if current.is_empty() {
                return;
            }
            let taken = std::mem::take(current);
            match slot {
                Slot::Heading(level, _) => {
                    *open = None;
                    out.heading(level, taken, cx);
                }
                Slot::Item { list, .. } => {
                    // The label of an item (`Lbl`) is often the bullet or number the list shows
                    // itself.
                    let text: String = taken.iter().map(|r| r.text.as_str()).collect();
                    match list_marker(text.trim_start()) {
                        Some((ordered, cut)) => {
                            let owned = without_marker(&taken, cut);
                            out.item(Some(list), ordered, owned.iter().collect(), cx, open);
                        }
                        None => out.item(Some(list), false, taken, cx, open),
                    }
                }
                Slot::Para(_, role) => {
                    *open = None;
                    out.paragraph(taken, role, cx);
                }
                Slot::Free | Slot::Cell { .. } | Slot::Block(_) => {
                    *open = None;
                    out.paragraph(taken, None, cx);
                }
            }
        };
    for (i, (r, slot)) in flow.iter().zip(slots).enumerate() {
        if out.stop.is_some() {
            return;
        }
        // A cell run left to the text reads like a free run.
        let slot = match slot {
            Slot::Cell { .. } | Slot::Block(_) => Slot::Free,
            s => *s,
        };
        let same = slot == current_slot
            && match slot {
                Slot::Free => {
                    let (a, b) = (line_of[current_last], line_of[i]);
                    current.is_empty() || a == b || continues(&all_lines[a], &all_lines[b])
                }
                _ => true,
            };
        if !same {
            flush(out, &mut current, current_slot, &mut open_list);
            current_slot = slot;
        }
        current.push(r);
        current_last = i;
    }
    flush(out, &mut current, current_slot, &mut open_list);
}

#[cfg(test)]
thread_local! {
    /// The mid-page line texts the last style pass kept.
    static BODY_KEPT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use pdfparts::StructElem;

    use super::*;

    const NOTHING_DRAWN: Drawn<'static> = Drawn {
        rules: &[],
        images: &[],
        widgets: &[],
        boxes: &[],
        lines_unsearched: false,
        width: 595.0,
        body_size: 10.0,
    };

    /// The minus after an amount with cents ends the amount; a hyphen after a word or a whole
    /// number cuts a word or a range (part 2e-4, item 3). Text of other scripts is read char by
    /// char, never cut inside a char.
    #[test]
    fn a_minus_after_an_amount_is_no_word_cut() {
        for signed in ["10,00-", "Rabatt 1 10,00-", "1.234,56-", "12.50-"] {
            assert!(signed_amount(signed), "{signed}");
            assert!(!word_cut(signed), "{signed}");
        }
        for cut in [
            "Page-",
            "Rechnungs-",
            "2025-",
            "1,5-",
            "\u{201c}ab-",
            "\u{20ac}1,00-",
            "-",
        ] {
            assert!(!signed_amount(cut), "{cut}");
            assert!(word_cut(cut), "{cut}");
        }
    }

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
    fn a_cut_at_words_the_text_no_longer_has_is_none() {
        let mut r = run("AA BB", 0, 700.0, 10.0);
        r.gaps = Gaps::new(vec![Gap {
            words: 1,
            from: 82.0,
            to: 86.0,
        }]);
        let (before, after) = r.cut_at(0).expect("a cut");
        assert_eq!((before.text.as_str(), after.text.as_str()), ("AA", " BB"));
        r.text = "AABB".into();
        assert!(r.cut_at(0).is_none());
        assert!(r.cut_at(1).is_none());
    }

    #[test]
    fn kept_body_lines_are_bounded() {
        // Distinct mid-page lines of 13 bytes each cost 77 bytes with the set: 1,000 bytes
        // keep 12 of them.
        let letter = |k: u32| char::from(b'a' + (k % 26) as u8);
        let runs: Vec<Run> = (0..200)
            .map(|k| {
                let word: String = [letter(k / 676), letter(k / 26), letter(k)]
                    .iter()
                    .collect();
                run(&format!("body line {word}"), k, 500.0, 10.0)
            })
            .collect();
        Style::new(&[PageText {
            page: 1,
            height: Some(842.0),
            runs: &runs,
        }]);
        assert_eq!(BODY_KEPT.with(|k| k.get()), 1_000 / (13 + BODY_KEY_COST));
    }

    #[test]
    fn one_link_over_a_change_of_style_is_one_link() {
        let uri = UriTarget::new("https://example.org/long");
        let runs: Vec<Run> = (0..6)
            .map(|k| Run {
                bold: k % 2 == 0,
                uri: Some(uri.clone()),
                ..run(if k == 0 { "a" } else { " b" }, 0, 700.0, 10.0)
            })
            .collect();
        let refs: Vec<&Run> = runs.iter().collect();
        let built = inlines(
            &refs,
            &[],
            &LinkRoom::new(u64::MAX),
            &Budget::new(rezzer_limits::Limits::default()),
        )
        .unwrap();
        assert_eq!(built.len(), 1, "{built:?}");
        match &built[0] {
            Inline::Link {
                target, inlines, ..
            } => {
                assert_eq!(target, "https://example.org/long");
                assert_eq!(rezzer_model::plain_text(inlines), "a b b b b b");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn space_only_first_run() {
        // The second line starts with a run that is only a space: the line change still
        // puts a space between the words.
        let runs = [
            run("A", 0, 700.0, 10.0),
            run(" ", 1, 688.0, 10.0),
            run("B", 1, 688.0, 10.0),
        ];
        let refs: Vec<&Run> = runs.iter().collect();
        let built = inlines(
            &refs,
            &[],
            &LinkRoom::new(u64::MAX),
            &Budget::new(rezzer_limits::Limits::default()),
        )
        .unwrap();
        assert_eq!(rezzer_model::plain_text(&built), "A B");
    }

    #[test]
    fn white_space_at_a_link_end_stands_after_it() {
        let uri = UriTarget::new("https://example.org/a");
        let runs: Vec<Run> = [("see ", None), ("example.org ", Some(&uri)), ("now", None)]
            .into_iter()
            .map(|(t, u)| Run {
                uri: u.cloned(),
                ..run(t, 0, 700.0, 10.0)
            })
            .collect();
        let refs: Vec<&Run> = runs.iter().collect();
        let built = inlines(
            &refs,
            &[],
            &LinkRoom::new(u64::MAX),
            &Budget::new(rezzer_limits::Limits::default()),
        )
        .unwrap();
        assert_eq!(built.len(), 4, "{built:?}");
        match &built[1] {
            Inline::Link { inlines, .. } => {
                assert_eq!(rezzer_model::plain_text(inlines), "example.org")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(rezzer_model::plain_text(&built), "see example.org now");
    }

    #[test]
    fn links_past_their_room_keep_their_text() {
        let a = UriTarget::new("https://example.org/a");
        let b = UriTarget::new("https://example.org/a-longer-target");
        let runs: Vec<Run> = [("one", &a), (" two", &b), (" three", &b), (" four", &a)]
            .into_iter()
            .enumerate()
            .map(|(k, (t, u))| Run {
                bold: k == 2,
                uri: Some(u.clone()),
                ..run(t, 0, 700.0, 10.0)
            })
            .collect();
        let refs: Vec<&Run> = runs.iter().collect();
        // Room for two copies of `a`, not for `a` and `b`.
        let room = LinkRoom::new(2 * (a.written + LINK_COST));
        let built = inlines(
            &refs,
            &[],
            &room,
            &Budget::new(rezzer_limits::Limits::default()),
        )
        .unwrap();
        assert!(
            matches!(&built[0], Inline::Link { target, .. } if target == "https://example.org/a"),
            "{built:?}"
        );
        let last = built.len() - 1;
        assert!(
            built[1..last]
                .iter()
                .all(|i| !matches!(i, Inline::Link { .. })),
            "{built:?}"
        );
        assert!(matches!(&built[last], Inline::Link { .. }), "{built:?}");
        assert_eq!(rezzer_model::plain_text(&built), "one two three four");
        // The link over the change of style is one link cut; the last link fits again, as the
        // copy of `a` is shorter than the room that is left.
        assert_eq!(room.cut(), Some((1, 1, 1)));
    }

    #[test]
    fn link_targets_are_charged_by_what_is_written() {
        // Three bytes for a space in Markdown, six for a control character in JSON.
        let room = LinkRoom::new(1_000);
        assert!(room.take(&UriTarget::new("a b"), 1));
        assert_eq!(room.left.get(), 1_000 - (1 + 3 + 1 + LINK_COST));
        assert!(room.take(&UriTarget::new("\u{1}"), 1));
        assert_eq!(room.left.get(), 1_000 - (5 + LINK_COST) - (6 + LINK_COST));
    }

    /// Fix round 2 of Task 13, item 2: a heading whose size the style pass did not see as a
    /// heading size (its line there held a smaller cell beside it) gets the level of its size
    /// among the heading sizes, not the deepest level: a title larger than all but one heading
    /// size is level 2.
    #[test]
    fn a_heading_size_not_seen_takes_its_place_among_the_sizes() {
        let style = Style {
            body_size: 7.0,
            heading_sizes: vec![20.0, 12.0],
            ..Style::default()
        };
        let runs = [run("Rechnung", 0, 690.0, 14.0)];
        assert_eq!(style.heading_level(&line(&runs)), Some(2));
        let runs = [run("Rechnung", 0, 690.0, 24.0)];
        assert_eq!(style.heading_level(&line(&runs)), Some(1));
        // A size seen keeps its place; one smaller than all is the deepest level.
        let runs = [run("Rechnung", 0, 690.0, 12.0)];
        assert_eq!(style.heading_level(&line(&runs)), Some(2));
        let runs = [run("Rechnung", 0, 690.0, 11.0)];
        assert_eq!(style.heading_level(&line(&runs)), Some(3));
    }

    #[test]
    fn a_dropped_block_gives_back_its_link_counts() {
        let a = UriTarget::new("https://example.org/a");
        let runs = vec![Run {
            uri: Some(a),
            ..run("link", 0, 700.0, 10.0)
        }];
        let style = Style::new(&[PageText {
            page: 1,
            height: Some(842.0),
            runs: &runs,
        }]);
        // No room for the target, and no element for the block: the block is dropped, and
        // the cut link with it.
        let budget = Budget::new(rezzer_limits::Limits {
            max_elements: 0,
            ..rezzer_limits::Limits::default()
        });
        let room = LinkRoom::new(0);
        let cx = Context {
            style: &style,
            tree: None,
            roles: &|_| None,
            budget: &budget,
            tables: &Tables::default(),
            in_part: false,
            links: &room,
        };
        let mut built = Built::default();
        built.paragraph(runs.iter().collect(), None, &cx);
        assert!(built.blocks.is_empty());
        assert!(built.stop.is_some());
        assert_eq!(room.cut(), None);
    }

    /// A ruled table of one row holding "cell", with the title "Muster Titel" (with a link
    /// when `uri`), laid out with `room` bytes of link and table room.
    fn titled_table(rows: usize, uri: bool, room: u64) -> Vec<Block> {
        let style = Style::new(&[PageText {
            page: 1,
            height: Some(842.0),
            runs: &[],
        }]);
        let budget = Budget::new(rezzer_limits::Limits::default());
        let links = LinkRoom::new(room);
        let tables = Tables::default();
        let cx = Context {
            style: &style,
            tree: None,
            roles: &|_| None,
            budget: &budget,
            tables: &tables,
            in_part: false,
            links: &links,
        };
        let title = Run {
            uri: uri.then(|| UriTarget::new("https://example.org/titel")),
            ..run("Muster Titel", 0, 700.0, 10.0)
        };
        let plan = Planned {
            at: 0,
            top: 710.0,
            rows: (0..rows)
                .map(|_| {
                    vec![PlannedCell {
                        runs: vec![run("cell", 1, 680.0, 10.0)],
                        row_span: 1,
                        col_span: 1,
                        status: None,
                    }]
                })
                .collect(),
            trs: Vec::new(),
            additions: Vec::new(),
            header_rows: 0,
            tagged: None,
            continues: false,
            caption: vec![title],
            found_by: None,
        };
        let mut built = Built::default();
        built.table(plan, &cx);
        built.blocks
    }

    /// The kind and text of each block.
    fn kinds(blocks: &[Block]) -> Vec<(String, String)> {
        blocks
            .iter()
            .map(|b| match b {
                Block::Table { caption, rows, .. } => (
                    format!("table {caption:?}"),
                    rows.iter()
                        .flatten()
                        .map(|c| rezzer_model::plain_text(&c.inlines))
                        .collect(),
                ),
                Block::Paragraph { inlines, role, .. } => (
                    format!("paragraph {role:?}"),
                    rezzer_model::plain_text(inlines),
                ),
                other => (format!("{other:?}"), String::new()),
            })
            .collect()
    }

    #[test]
    fn a_title_is_the_caption_of_its_table() {
        let blocks = titled_table(1, false, 10_000);
        assert_eq!(
            kinds(&blocks),
            [(
                "table Some(\"Muster Titel\")".to_string(),
                "cell".to_string()
            )]
        );
    }

    #[test]
    fn a_title_with_a_link_stands_before_its_table() {
        let blocks = titled_table(1, true, 10_000);
        assert_eq!(kinds(&blocks)[0].0, "paragraph Some(\"caption\")");
        assert!(matches!(
            &blocks[0],
            Block::Paragraph { inlines, .. } if matches!(inlines[0], Inline::Link { .. })
        ));
        assert_eq!(kinds(&blocks)[1].0, "table None");
    }

    #[test]
    fn a_title_of_a_table_kept_as_paragraphs_comes_first() {
        // No room for the table's overhead: its text is paragraphs, the title before it.
        let blocks = titled_table(1, false, 0);
        let k = kinds(&blocks);
        assert_eq!(
            k[0],
            (
                "paragraph Some(\"caption\")".to_string(),
                "Muster Titel".to_string()
            )
        );
        assert!(k[1..].iter().any(|(_, t)| t == "cell"), "{k:?}");
        assert!(
            !k.iter().any(|(kind, _)| kind.starts_with("table")),
            "{k:?}"
        );
    }

    #[test]
    fn a_stop_at_a_linked_title_names_it_and_gives_its_links_back() {
        // No room for any element: the caption paragraph of the linked title is the block the
        // stop names, nothing of it is kept, and the link counts it took go back.
        let style = Style::new(&[PageText {
            page: 1,
            height: Some(842.0),
            runs: &[],
        }]);
        let budget = Budget::new(rezzer_limits::Limits {
            max_elements: 0,
            ..rezzer_limits::Limits::default()
        });
        let links = LinkRoom::new(10_000);
        let tables = Tables::default();
        let cx = Context {
            style: &style,
            tree: None,
            roles: &|_| None,
            budget: &budget,
            tables: &tables,
            in_part: false,
            links: &links,
        };
        let before = (links.left.get(), links.freed.get());
        let plan = Planned {
            at: 0,
            top: 710.0,
            rows: vec![vec![PlannedCell {
                runs: vec![run("cell", 1, 680.0, 10.0)],
                row_span: 1,
                col_span: 1,
                status: None,
            }]],
            trs: Vec::new(),
            additions: Vec::new(),
            header_rows: 0,
            tagged: None,
            continues: false,
            caption: vec![Run {
                uri: Some(UriTarget::new("https://example.org/titel")),
                ..run("Muster Titel", 0, 700.0, 10.0)
            }],
            found_by: None,
        };
        let mut built = Built::default();
        built.table(plan, &cx);
        assert!(built.stop.is_some());
        assert_eq!(built.cut, Some(Cut::Block(1)));
        assert!(built.blocks.is_empty(), "{:?}", built.blocks);
        assert_eq!((links.left.get(), links.freed.get()), before);
    }

    #[test]
    fn a_cut_title_link_stays_counted_when_its_table_is_kept_as_text() {
        // No link room: the title's link is cut (its text stays plain, so the title is a
        // plain caption), and the table's overhead does not fit either. The table is text,
        // the title a caption paragraph before it, and the cut link is still counted for
        // its loss.
        let style = Style::new(&[PageText {
            page: 1,
            height: Some(842.0),
            runs: &[],
        }]);
        let budget = Budget::new(rezzer_limits::Limits::default());
        let links = LinkRoom::new(0);
        let tables = Tables::default();
        let cx = Context {
            style: &style,
            tree: None,
            roles: &|_| None,
            budget: &budget,
            tables: &tables,
            in_part: false,
            links: &links,
        };
        let plan = Planned {
            at: 0,
            top: 710.0,
            rows: vec![vec![PlannedCell {
                runs: vec![run("cell", 1, 680.0, 10.0)],
                row_span: 1,
                col_span: 1,
                status: None,
            }]],
            trs: Vec::new(),
            additions: Vec::new(),
            header_rows: 0,
            tagged: None,
            continues: false,
            caption: vec![Run {
                uri: Some(UriTarget::new("https://example.org/titel")),
                ..run("Muster Titel", 0, 700.0, 10.0)
            }],
            found_by: None,
        };
        let mut built = Built::default();
        built.table(plan, &cx);
        let k = kinds(&built.blocks);
        assert_eq!(
            k[0],
            (
                "paragraph Some(\"caption\")".to_string(),
                "Muster Titel".to_string()
            )
        );
        assert!(
            !k.iter().any(|(kind, _)| kind.starts_with("table")),
            "{k:?}"
        );
        assert_eq!(links.cut(), Some((1, 1, 1)));
    }

    #[test]
    fn a_title_of_a_table_without_rows_stays() {
        let blocks = titled_table(0, false, 10_000);
        assert_eq!(
            kinds(&blocks),
            [(
                "paragraph Some(\"caption\")".to_string(),
                "Muster Titel".to_string()
            )]
        );
    }

    /// A block built in two calls, as a table is, one cell per call: the first cell cuts its
    /// link; the second call is stopped at its start by `limits`. The cut goes back.
    fn second_cell_stops(limits: rezzer_limits::Limits) {
        let second = run("a second cell with more text", 1, 600.0, 10.0);
        let style = Style::new(&[PageText {
            page: 1,
            height: Some(842.0),
            runs: &[],
        }]);
        let budget = Budget::new(limits);
        let room = LinkRoom::new(0);
        let cx = Context {
            style: &style,
            tree: None,
            roles: &|_| None,
            budget: &budget,
            tables: &Tables::default(),
            in_part: false,
            links: &room,
        };
        // The first cell, as the cell before a stop is built: its cut is counted.
        let mut built = Built {
            mark: Some(room.mark()),
            ..Built::default()
        };
        assert!(!room.take(&UriTarget::new("https://example.org/a"), 1));
        assert_eq!(room.cut(), Some((1, 1, 1)));
        assert!(built.inlines(&[&second], &cx).is_none());
        assert!(built.stop.is_some());
        assert_eq!(room.cut(), None, "the dropped block keeps a link cut");
    }

    #[test]
    fn a_text_stop_at_a_later_cell_gives_back_the_earlier_link_cuts() {
        second_cell_stops(rezzer_limits::Limits {
            max_text_bytes: 10,
            ..rezzer_limits::Limits::default()
        });
    }

    #[test]
    fn a_time_stop_at_a_later_cell_gives_back_the_earlier_link_cuts() {
        second_cell_stops(rezzer_limits::Limits {
            max_time: std::time::Duration::ZERO,
            ..rezzer_limits::Limits::default()
        });
    }

    #[test]
    fn markers() {
        assert_eq!(list_marker("\u{2022} one"), Some((false, 4)));
        assert_eq!(list_marker("- two"), Some((false, 2)));
        assert_eq!(list_marker("\u{2013} three"), Some((false, 4)));
        assert_eq!(list_marker("12) four"), Some((true, 4)));
        assert_eq!(list_marker("3. five"), Some((true, 3)));
        assert_eq!(list_marker("-5 degrees"), None);
        assert_eq!(list_marker("2024 was"), None);
        assert_eq!(list_marker("1.5 litres"), None);
        // Padding after a star or a dash: a mark set into a form field, no bullet.
        assert_eq!(list_marker("*     Betrag"), None);
        assert_eq!(list_marker("-  keine"), None);
        // A star closed by a lone star on its line encloses an item, no bullet (part 2e-4,
        // item 6); a star bullet over text with a star in a word still is one.
        assert_eq!(list_marker("* A123456B * x/y"), None);
        assert_eq!(list_marker("* Erste Position"), Some((false, 2)));
        assert_eq!(list_marker("* Rabatt 5* auf alles"), Some((false, 2)));
        assert_eq!(list_marker("\u{2013}\t\tkeine"), None);
        assert_eq!(list_marker("* Punkt"), Some((false, 2)));
    }

    #[test]
    fn untagged_list_and_paragraphs() {
        let runs = vec![
            run("Shopping", 0, 700.0, 10.0),
            run("- apples", 1, 688.0, 10.0),
            run("- pears and", 2, 676.0, 10.0),
            Run {
                x: 80.0,
                ..run("plums", 3, 664.0, 10.0)
            },
            run("After the list.", 4, 630.0, 10.0),
        ];
        let style = Style::new(&[PageText {
            page: 1,
            height: Some(842.0),
            runs: &runs,
        }]);
        let budget = Budget::new(rezzer_limits::Limits::default());
        let cx = Context {
            style: &style,
            tree: None,
            roles: &|_| None,
            budget: &budget,
            tables: &Tables::default(),
            in_part: false,
            links: &LinkRoom::new(u64::MAX),
        };
        let laid = page_blocks(1, Some(842.0), runs, &NOTHING_DRAWN, &cx);
        let texts: Vec<String> = laid
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph { inlines, .. } => rezzer_model::plain_text(inlines),
                Block::List { items, .. } => items
                    .iter()
                    .map(|i| match &i[0] {
                        Block::Paragraph { inlines, .. } => {
                            format!("[{}]", rezzer_model::plain_text(inlines))
                        }
                        other => panic!("{other:?}"),
                    })
                    .collect(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            texts,
            ["Shopping", "[apples][pears and plums]", "After the list."]
        );
    }

    fn elem(role: &str, parent: Option<usize>, children: &[usize]) -> StructElem {
        StructElem {
            role: role.into(),
            parent,
            children: children.to_vec(),
            col_span: 1,
            row_span: 1,
            ..StructElem::default()
        }
    }

    /// A table of `rows` rows of cells, each `(role, text)`, below a Document element, and
    /// the cell elements of each row.
    fn table_tree(rows: &[&[&str]]) -> (StructTree, Vec<Vec<usize>>) {
        let mut elems = vec![elem("Document", None, &[1]), elem("Table", Some(0), &[])];
        let mut cells = Vec::new();
        for row in rows {
            let tr = elems.len();
            elems[1].children.push(tr);
            elems.push(elem("TR", Some(1), &[]));
            let mut ids = Vec::new();
            for role in row.iter() {
                let c = elems.len();
                elems[tr].children.push(c);
                elems.push(elem(role, Some(tr), &[]));
                ids.push(c);
            }
            cells.push(ids);
        }
        (
            StructTree {
                elems,
                roots: vec![0],
            },
            cells,
        )
    }

    /// Runs one line each from the top of a page, MCID k on line k.
    fn tagged_runs(texts: &[&str], top: f32) -> Vec<Run> {
        texts
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let mut r = run(t, k as u32, top - 14.0 * k as f32, 10.0);
                r.mcid = kept_mcid(Some(k as u32));
                r
            })
            .collect()
    }

    fn cell_texts(rows: &[Vec<Cell>]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| {
                r.iter()
                    .map(|c| rezzer_model::plain_text(&c.inlines))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_tagged_table_goes_on_over_pages_without_its_repeated_header() {
        let (tree, cells) =
            table_tree(&[&["TH", "TH"], &["TD", "TD"], &["TH", "TH"], &["TD", "TD"]]);
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits::default());
        let tables = Tables::default();
        let links = LinkRoom::new(u64::MAX);
        // Page 1 holds rows 1 and 2, page 2 the header again and row 4.
        let on = |rows: [usize; 2]| -> Vec<usize> {
            rows.iter().flat_map(|&r| cells[r].clone()).collect()
        };
        let pages = [
            (on([0, 1]), ["Name", "Age", "Ann", "31"]),
            (on([2, 3]), ["Name", "Age", "Bob", "42"]),
        ];
        let mut laid = Vec::new();
        for (k, (owners, texts)) in pages.iter().enumerate() {
            let roles = |m: u32| owners.get(m as usize).copied();
            let cx = Context {
                style: &style,
                tree: Some(&tree),
                roles: &roles,
                budget: &budget,
                tables: &tables,
                in_part: false,
                links: &links,
            };
            laid.push(page_blocks(
                k as u32 + 1,
                Some(842.0),
                tagged_runs(texts, 700.0),
                &NOTHING_DRAWN,
                &cx,
            ));
        }
        match laid[0].blocks.as_slice() {
            [
                Block::Table {
                    header_rows: 1,
                    rows,
                    anchor: Some(a),
                    ..
                },
            ] => {
                assert_eq!(cell_texts(rows), [["Name", "Age"], ["Ann", "31"]]);
                assert_eq!(a.0, "p1:t1");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(laid[0].tagged, [Some(1)]);
        assert!(laid[1].blocks.is_empty(), "{:?}", laid[1].blocks);
        match laid[1].continued.as_slice() {
            [(1, rows)] => assert_eq!(cell_texts(rows), [["Bob", "42"]]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tagged_column_titles_are_not_running_headers() {
        // Each page has an untagged running header and a tagged table whose column titles
        // stand at the same height below it.
        let (tree, cells) = table_tree(&[&["TD"], &["TD"], &["TD"], &["TD"], &["TD"], &["TD"]]);
        let page_runs = |n: usize| -> Vec<Run> {
            let mut runs = vec![run("ACME Catalog", 0, 815.0, 9.0)];
            let mut table = tagged_runs(&["CONTROL NAME", &format!("Row {n}")], 790.0);
            for r in &mut table {
                r.line += 1;
            }
            runs.extend(table);
            runs
        };
        let all: Vec<Vec<Run>> = (0..3).map(page_runs).collect();
        let texts: Vec<PageText<'_>> = all
            .iter()
            .enumerate()
            .map(|(k, runs)| PageText {
                page: k as u32 + 1,
                height: Some(842.0),
                runs,
            })
            .collect();
        let style = Style::new(&texts);
        let budget = Budget::new(rezzer_limits::Limits::default());
        let tables = Tables::default();
        let links = LinkRoom::new(u64::MAX);
        for (k, runs) in all.iter().enumerate() {
            let owners = [cells[2 * k][0], cells[2 * k + 1][0]];
            let roles = |m: u32| {
                m.checked_sub(0)
                    .and_then(|m| owners.get(m as usize).copied())
            };
            let cx = Context {
                style: &style,
                tree: Some(&tree),
                roles: &roles,
                budget: &budget,
                tables: &tables,
                in_part: false,
                links: &links,
            };
            let laid = page_blocks(k as u32 + 1, Some(842.0), runs.clone(), &NOTHING_DRAWN, &cx);
            let header = laid.blocks.iter().any(|b| {
                matches!(b, Block::Paragraph { role: Some(r), inlines, .. }
                    if r == "page-header" && rezzer_model::plain_text(inlines) == "ACME Catalog")
            });
            assert!(header, "page {}: {:?}", k + 1, laid.blocks);
            let titles_as_header = laid.blocks.iter().any(|b| {
                matches!(b, Block::Paragraph { inlines, .. }
                    if rezzer_model::plain_text(inlines) == "CONTROL NAME")
            });
            assert!(!titles_as_header, "page {}: {:?}", k + 1, laid.blocks);
        }
    }

    #[test]
    fn a_stop_in_the_text_added_to_an_earlier_row_is_named() {
        // Two rows, each with a cell on page 1 and one on page 2. Page 2 adds to both rows; the
        // text limit stops the second addition.
        let (tree, cells) = table_tree(&[&["TD", "TD"], &["TD", "TD"]]);
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits {
            max_text_bytes: 4,
            ..rezzer_limits::Limits::default()
        });
        let tables = Tables::default();
        let links = LinkRoom::new(u64::MAX);
        let one = [cells[0][0], cells[1][0]];
        let roles = |m: u32| one.get(m as usize).copied();
        let cx = Context {
            style: &style,
            tree: Some(&tree),
            roles: &roles,
            budget: &budget,
            tables: &tables,
            in_part: false,
            links: &links,
        };
        let first = page_blocks(
            1,
            Some(842.0),
            tagged_runs(&["a", "b"], 700.0),
            &NOTHING_DRAWN,
            &cx,
        );
        assert_eq!(first.blocks.len(), 1);
        let two = [cells[0][1], cells[1][1]];
        let roles = |m: u32| two.get(m as usize).copied();
        let cx = Context {
            roles: &roles,
            ..cx
        };
        let second = page_blocks(
            2,
            Some(842.0),
            tagged_runs(&["c", "a longer cell"], 700.0),
            &NOTHING_DRAWN,
            &cx,
        );
        assert_eq!(second.added.len(), 1, "{:?}", second.added);
        assert_eq!(second.cut, Some(Cut::Added { table: 1, row: 1 }));
    }

    #[test]
    fn a_stop_at_the_first_row_added_to_an_earlier_table_gives_back_its_overhead() {
        // Page 1 begins the table with row 1. Page 2 adds text to row 1 and begins row 2,
        // whose text is over the text limit: the stop drops row 2, and the overhead set aside
        // for it goes back.
        let (tree, cells) = table_tree(&[&["TD", "TD"], &["TD", "TD"]]);
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits {
            max_text_bytes: 4,
            ..rezzer_limits::Limits::default()
        });
        let tables = Tables::default();
        let links = LinkRoom::new(10_000);
        let one = [cells[0][0]];
        let roles = |m: u32| one.get(m as usize).copied();
        let cx = Context {
            style: &style,
            tree: Some(&tree),
            roles: &roles,
            budget: &budget,
            tables: &tables,
            in_part: false,
            links: &links,
        };
        let first = page_blocks(
            1,
            Some(842.0),
            tagged_runs(&["a"], 700.0),
            &NOTHING_DRAWN,
            &cx,
        );
        assert_eq!(first.blocks.len(), 1);
        let before = (links.left.get(), links.freed.get());
        let two = [cells[0][1], cells[1][0]];
        let roles = |m: u32| two.get(m as usize).copied();
        let cx = Context {
            roles: &roles,
            ..cx
        };
        let second = page_blocks(
            2,
            Some(842.0),
            tagged_runs(&["c", "a longer cell"], 700.0),
            &NOTHING_DRAWN,
            &cx,
        );
        assert_eq!(second.added.len(), 1, "{:?}", second.added);
        assert_eq!(
            second.cut,
            Some(Cut::Row {
                table: TableRef::Earlier(1),
                row: 1
            })
        );
        assert_eq!((links.left.get(), links.freed.get()), before);
    }

    #[test]
    fn a_tagged_table_kept_as_paragraphs_stays_text_on_later_pages() {
        // Room for the overhead of a row, not of a table (TABLE_COST + ROW_COST + CELL_COST):
        // the table's text is a paragraph on page 1, and its row on page 2, which would fit,
        // is a paragraph too, not a row for a table that is not kept.
        let (tree, cells) = table_tree(&[&["TD"], &["TD"]]);
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits::default());
        let tables = Tables::default();
        let links = LinkRoom::new(200);
        let mut laid = Vec::new();
        for (k, word) in ["first", "second"].into_iter().enumerate() {
            let owner = [cells[k][0]];
            let roles = |m: u32| owner.get(m as usize).copied();
            let cx = Context {
                style: &style,
                tree: Some(&tree),
                roles: &roles,
                budget: &budget,
                tables: &tables,
                in_part: false,
                links: &links,
            };
            laid.push(page_blocks(
                k as u32 + 1,
                Some(842.0),
                tagged_runs(&[word], 700.0),
                &NOTHING_DRAWN,
                &cx,
            ));
        }
        for (k, word) in ["first", "second"].into_iter().enumerate() {
            match laid[k].blocks.as_slice() {
                [Block::Paragraph { inlines, .. }] => {
                    assert_eq!(rezzer_model::plain_text(inlines), word);
                }
                other => panic!("page {}: {other:?}", k + 1),
            }
            assert!(laid[k].continued.is_empty() && laid[k].added.is_empty());
            assert!(laid[k].stop.is_none());
        }
        match laid[0].notes.as_slice() {
            [(what, LossReason::Limit, Severity::Info)] => {
                assert!(what.contains("kept as paragraphs"), "{what}");
                assert!(
                    what.contains("p1:b1") && what.contains("later pages"),
                    "{what}"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(laid[1].notes.is_empty(), "{:?}", laid[1].notes);
    }

    #[test]
    fn a_cell_in_two_rows_is_kept_once_and_said() {
        let (mut tree, cells) = table_tree(&[&["TD"], &["TD"]]);
        // The second row also names the cell of the first.
        let (first, second) = (cells[0][0], cells[1][0]);
        let tr = tree.elems[second].parent.expect("a row");
        tree.elems[tr].children.insert(0, first);
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits::default());
        let owners = [first, second];
        let roles = |m: u32| owners.get(m as usize).copied();
        let cx = Context {
            style: &style,
            tree: Some(&tree),
            roles: &roles,
            budget: &budget,
            tables: &Tables::default(),
            in_part: false,
            links: &LinkRoom::new(u64::MAX),
        };
        let laid = page_blocks(
            1,
            Some(842.0),
            tagged_runs(&["one", "two"], 700.0),
            &NOTHING_DRAWN,
            &cx,
        );
        match laid.blocks.as_slice() {
            [Block::Table { rows, .. }] => assert_eq!(cell_texts(rows), [["one"], ["two"]]),
            other => panic!("{other:?}"),
        }
        assert!(
            laid.notes
                .iter()
                .any(|(w, why, _)| *why == LossReason::Damaged
                    && w == "1 cell of a tagged table on page 1 stands in more than one row of the tag \
                     tree; it is kept in its first row"),
            "{:?}",
            laid.notes
        );
    }

    #[test]
    fn a_cell_listed_twice_in_one_row_is_kept_once_and_said() {
        let (mut tree, cells) = table_tree(&[&["TD", "TD"]]);
        let first = cells[0][0];
        let tr = tree.elems[first].parent.expect("a row");
        tree.elems[tr].children.push(first);
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits::default());
        let owners = [first, cells[0][1]];
        let roles = |m: u32| owners.get(m as usize).copied();
        let cx = Context {
            style: &style,
            tree: Some(&tree),
            roles: &roles,
            budget: &budget,
            tables: &Tables::default(),
            in_part: false,
            links: &LinkRoom::new(u64::MAX),
        };
        let laid = page_blocks(
            1,
            Some(842.0),
            tagged_runs(&["one", "two"], 700.0),
            &NOTHING_DRAWN,
            &cx,
        );
        match laid.blocks.as_slice() {
            [Block::Table { rows, .. }] => assert_eq!(cell_texts(rows), [["one", "two"]]),
            other => panic!("{other:?}"),
        }
        assert!(
            laid.notes
                .iter()
                .any(|(w, why, _)| *why == LossReason::Damaged
                    && w == "1 cell of a tagged table on page 1 listed twice in the same row of \
                         the tag tree; it is kept once"),
            "{:?}",
            laid.notes
        );
    }

    #[test]
    fn a_tagged_table_mostly_without_text_is_left_as_text() {
        let (mut tree, cells) = table_tree(&[&["TD", "TD"], &["TD", "TD"]]);
        // All four cells have marked content on page 1, but only one has text there.
        for row in &cells {
            for &c in row {
                tree.elems[c].mcids = vec![0];
                tree.elems[c].page = Some(1);
            }
        }
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits::default());
        let owners = [cells[0][0]];
        let roles = |m: u32| owners.get(m as usize).copied();
        let cx = Context {
            style: &style,
            tree: Some(&tree),
            roles: &roles,
            budget: &budget,
            tables: &Tables::default(),
            in_part: false,
            links: &LinkRoom::new(u64::MAX),
        };
        let laid = page_blocks(
            1,
            Some(842.0),
            tagged_runs(&["lonely"], 700.0),
            &NOTHING_DRAWN,
            &cx,
        );
        match laid.blocks.as_slice() {
            [Block::Paragraph { inlines, .. }] => {
                assert_eq!(rezzer_model::plain_text(inlines), "lonely")
            }
            other => panic!("{other:?}"),
        }
        assert!(
            laid.notes
                .iter()
                .any(|(w, why, _)| *why == LossReason::Uncertain
                    && w.contains("only 1 of its 4 cells")),
            "{:?}",
            laid.notes
        );
    }

    #[test]
    fn a_stop_inside_a_kept_list_names_the_item() {
        let runs = vec![
            run("- apples", 0, 700.0, 10.0),
            run("- pears", 1, 688.0, 10.0),
            run("- a much longer third item", 2, 676.0, 10.0),
            run("After the list.", 3, 640.0, 10.0),
        ];
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits {
            max_text_bytes: 10,
            ..rezzer_limits::Limits::default()
        });
        let cx = Context {
            style: &style,
            tree: None,
            roles: &|_| None,
            budget: &budget,
            tables: &Tables::default(),
            in_part: false,
            links: &LinkRoom::new(u64::MAX),
        };
        let laid = page_blocks(1, Some(842.0), runs, &NOTHING_DRAWN, &cx);
        match laid.blocks.as_slice() {
            [Block::List { items, .. }] => assert_eq!(items.len(), 2),
            other => panic!("{other:?}"),
        }
        assert!(laid.stop.is_some());
        assert_eq!(laid.cut, Some(Cut::Item { block: 1, item: 3 }));
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
        let pages = [PageText {
            page: 1,
            height: Some(842.0),
            runs: &runs,
        }];
        let untagged = Style::learn(&pages, false);
        assert_eq!(untagged.body_size, 9.0);
        assert!(untagged.legends.contains_key(&1));
        let tagged = Style::learn(&pages, true);
        assert_eq!(tagged.body_size, 6.0);
        assert!(tagged.legends.is_empty());
    }

    /// The body size is the exact size of the body text, not its half point: a letter printed
    /// at 0.675 sets its body of 7 points at 4.725, which a half point would make 4.5, and a
    /// name of 6.75 points (10 at full size, under 1.5 times the body there) would then be
    /// 1.5 times the body.
    #[test]
    fn the_body_size_is_exact() {
        let runs: Vec<Run> = (0..4)
            .map(|k| {
                run(
                    "die Firma liefert die bestellte Ware",
                    k,
                    700.0 - 6.0 * k as f32,
                    4.725,
                )
            })
            .collect();
        let pages = [PageText {
            page: 1,
            height: Some(842.0),
            runs: &runs,
        }];
        let style = Style::learn(&pages, false);
        assert!(
            (style.body_size - 4.725).abs() < 1e-4,
            "{}",
            style.body_size
        );
        assert!(6.75 < HEADING_PLAIN_RATIO * style.body_size);
    }

    /// A stop inside the legend: the references to the footnotes it cut are text again, so
    /// no reference is left without its footnote, whatever the limit.
    #[test]
    fn a_stop_inside_the_legend_unlinks_what_it_cut() {
        let mut runs: Vec<Run> = (1..=6)
            .map(|k| {
                run(
                    &format!("Wert{k}) {k},00"),
                    k - 1,
                    700.0 - 12.0 * k as f32,
                    9.0,
                )
            })
            .collect();
        runs.extend((1..=6).map(|k| {
            run(
                &format!("{k}) Muster {k}"),
                k + 5,
                140.0 - 8.0 * k as f32,
                6.0,
            )
        }));
        // The style pass finds the legend the layout takes.
        let style = Style::new(&[PageText {
            page: 1,
            height: Some(842.0),
            runs: &runs,
        }]);
        let mut cut = 0;
        for max_elements in 1..60 {
            let budget = Budget::new(rezzer_limits::Limits {
                max_elements,
                ..rezzer_limits::Limits::default()
            });
            let cx = Context {
                style: &style,
                tree: None,
                roles: &|_| None,
                budget: &budget,
                tables: &Tables::default(),
                in_part: false,
                links: &LinkRoom::new(u64::MAX),
            };
            let mut laid = page_blocks(1, Some(842.0), runs.clone(), &NOTHING_DRAWN, &cx);
            let notes: Vec<String> = laid
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Section {
                        anchor: Some(a), ..
                    } => a.0.strip_prefix("fn:").map(str::to_string),
                    _ => None,
                })
                .collect();
            let refs = footnotes::referenced(&mut laid.blocks);
            for r in &refs {
                assert!(notes.contains(r), "{max_elements}: {r} without {notes:?}");
            }
            if (1..6).contains(&notes.len()) {
                cut += 1;
                // The cut ones are their markers as text again.
                assert_eq!(refs.len(), notes.len(), "{max_elements}");
            }
        }
        assert!(cut > 0, "no limit stopped inside the legend");
    }

    #[test]
    fn a_stop_inside_a_table_keeps_its_rows_before() {
        let (tree, cells) = table_tree(&[&["TD"], &["TD"], &["TD"]]);
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits {
            max_text_bytes: 10,
            ..rezzer_limits::Limits::default()
        });
        let owners: Vec<usize> = cells.iter().map(|r| r[0]).collect();
        let roles = |m: u32| owners.get(m as usize).copied();
        let cx = Context {
            style: &style,
            tree: Some(&tree),
            roles: &roles,
            budget: &budget,
            tables: &Tables::default(),
            in_part: false,
            links: &LinkRoom::new(u64::MAX),
        };
        let laid = page_blocks(
            1,
            Some(842.0),
            tagged_runs(&["one", "two", "a much longer third"], 700.0),
            &NOTHING_DRAWN,
            &cx,
        );
        match laid.blocks.as_slice() {
            [Block::Table { rows, .. }] => assert_eq!(cell_texts(rows), [["one"], ["two"]]),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            laid.cut,
            Some(Cut::Row {
                table: TableRef::Here(1),
                row: 3
            })
        );
    }

    #[test]
    fn tagged_roles_lists_tables_and_actual_text() {
        let mut elems = vec![
            elem("Document", None, &[1, 3, 4, 10, 17]),
            elem("H1", Some(0), &[2]),
            elem("NonStruct", Some(1), &[]),
            elem("P", Some(0), &[]),
            elem("L", Some(0), &[5, 8]),
            elem("LI", Some(4), &[6, 7]),
            elem("Lbl", Some(5), &[]),
            elem("LBody", Some(5), &[]),
            elem("LI", Some(4), &[9]),
            elem("LBody", Some(8), &[]),
            elem("Table", Some(0), &[11, 14]),
            elem("TR", Some(10), &[12, 13]),
            elem("TH", Some(11), &[]),
            elem("TH", Some(11), &[]),
            elem("TR", Some(10), &[15, 16]),
            elem("TD", Some(14), &[]),
            elem("TD", Some(14), &[]),
            elem("Note", Some(0), &[]),
        ];
        elems[3].actual_text = Some("Replaced text".into());
        let tree = StructTree {
            elems,
            roots: vec![0],
        };
        // mcid to element.
        let owner = [2usize, 3, 3, 6, 7, 9, 12, 13, 15, 16, 17];
        let texts = [
            "Title", "Gl", "yphs", "\u{2022}", " first", "second", "Name", "Age", "Ann", "31",
            "A note",
        ];
        let mut runs = Vec::new();
        let mut y = 800.0;
        for (i, t) in texts.iter().enumerate() {
            // Glyph runs of the paragraph and the label and body of an item share a line.
            let same_line = matches!(i, 2 | 4 | 7 | 9);
            if !same_line {
                y -= 20.0;
            }
            let mut r = run(t, 0, y, 10.0);
            r.line = (800.0 - y) as u32;
            r.mcid = kept_mcid(Some(i as u32));
            runs.push(r);
        }
        let style = Style::new(&[PageText {
            page: 1,
            height: Some(842.0),
            runs: &runs,
        }]);
        let budget = Budget::new(rezzer_limits::Limits::default());
        let tables = Tables::default();
        let roles = |m: u32| owner.get(m as usize).copied();
        let cx = Context {
            style: &style,
            tree: Some(&tree),
            roles: &roles,
            budget: &budget,
            tables: &tables,
            in_part: false,
            links: &LinkRoom::new(u64::MAX),
        };
        let laid = page_blocks(1, Some(842.0), runs, &NOTHING_DRAWN, &cx);
        let text = |inlines: &[Inline]| rezzer_model::plain_text(inlines);
        match laid.blocks.as_slice() {
            [
                Block::Heading {
                    level: 1,
                    inlines: h,
                    ..
                },
                Block::Paragraph {
                    inlines: p,
                    role: None,
                    ..
                },
                Block::List {
                    ordered: false,
                    items,
                    ..
                },
                Block::Table {
                    header_rows: 1,
                    rows,
                    ..
                },
                Block::Paragraph {
                    inlines: note,
                    role: Some(r),
                    ..
                },
            ] => {
                assert_eq!(text(h), "Title");
                assert_eq!(text(p), "Replaced text");
                let items: Vec<String> = items
                    .iter()
                    .map(|i| match &i[0] {
                        Block::Paragraph { inlines, .. } => text(inlines),
                        other => panic!("{other:?}"),
                    })
                    .collect();
                assert_eq!(items, ["first", "second"]);
                let cells: Vec<Vec<String>> = rows
                    .iter()
                    .map(|r| r.iter().map(|c| text(&c.inlines)).collect())
                    .collect();
                assert_eq!(cells, [["Name", "Age"], ["Ann", "31"]]);
                assert_eq!(r, "footnote");
                assert_eq!(text(note), "A note");
            }
            other => panic!("{other:#?}"),
        }
        assert!(laid.stop.is_none());
    }

    #[test]
    fn a_table_over_many_pages_is_walked_once() {
        const ROWS: usize = 200_000;
        const PER_PAGE: usize = 100;
        let mut elems = vec![elem("Document", None, &[1]), elem("Table", Some(0), &[])];
        for _ in 0..ROWS {
            let tr = elems.len();
            elems[1].children.push(tr);
            elems.push(elem("TR", Some(1), &[tr + 1]));
            elems.push(elem("TD", Some(tr), &[]));
        }
        let tree = StructTree {
            elems,
            roots: vec![0],
        };
        let style = Style::default();
        let budget = Budget::new(rezzer_limits::Limits::default());
        let tables = Tables::default();
        let start = std::time::Instant::now();
        let mut rows = 0;
        for page in 0..ROWS / PER_PAGE {
            let first = page * PER_PAGE;
            let runs: Vec<Run> = (0..PER_PAGE)
                .map(|k| {
                    let mut r = run("cell", k as u32, 800.0 - k as f32 * 7.0, 6.0);
                    r.mcid = kept_mcid(Some(k as u32));
                    r
                })
                .collect();
            // Marked content k of this page sits in the cell of row first + k.
            let roles = |m: u32| Some(3 + 2 * (first + m as usize));
            let cx = Context {
                style: &style,
                tree: Some(&tree),
                roles: &roles,
                budget: &budget,
                tables: &tables,
                in_part: false,
                links: &LinkRoom::new(u64::MAX),
            };
            let laid = page_blocks(page as u32 + 1, None, runs, &NOTHING_DRAWN, &cx);
            // The table starts on page 1; later pages add rows to it.
            match (laid.blocks.as_slice(), laid.continued.as_slice()) {
                ([Block::Table { rows: r, .. }], []) if page == 0 => rows += r.len(),
                ([], [(1, r)]) if page > 0 => rows += r.len(),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(rows, ROWS);
        assert!(start.elapsed().as_secs() < 3, "{:?}", start.elapsed());
    }
}
