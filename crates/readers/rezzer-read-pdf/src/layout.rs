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
use std::rc::Rc;

use pdfparts::StructTree;
use rezzer_limits::{Budget, LimitHit};
use rezzer_model::{Anchor, Block, Cell, Inline, LossReason, Severity};

use crate::tables::{self, Planned, Rule, Tables, Unsure};

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
    /// The marked content id the run is shown in, for the structure tree.
    pub mcid: Option<u32>,
    /// The target of a URI link whose box holds the middle of the run, shared by all the
    /// runs of the link.
    pub uri: Option<Rc<UriTarget>>,
    /// The number of the run's line on its page, in reading order. The runs of one line are
    /// next to each other.
    pub line: u32,
    /// The run reads in another direction than the page's text (margin notes, stamps,
    /// vertical writing); its `x` and `y` are where it starts.
    pub off_axis: bool,
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

/// Pages a header or footer line must repeat on.
const REPEATS: usize = 3;

/// A line at least this many times the body size is a heading in an untagged file.
const HEADING_RATIO: f32 = 1.2;

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
    /// The most common font size, by character count, of the text outside running headers
    /// and footers.
    pub body_size: f32,
    /// Distinct heading sizes of an untagged file, largest first.
    heading_sizes: Vec<f32>,
    /// Lines that repeat at the top or bottom of at least [`REPEATS`] pages at the same
    /// height, with digits replaced.
    furniture: HashMap<Furniture, Vec<(f32, f32)>>,
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
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
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

/// Rounds a font size to half points.
fn size_key(size: f32) -> i32 {
    (size * 2.0).round() as i32
}

impl Style {
    /// Learns body size, heading sizes and running headers and footers from every page.
    pub(crate) fn new(pages: &[PageText<'_>]) -> Style {
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
                if pages_at.len() >= REPEATS {
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
            ..Style::default()
        };
        let mut chars: HashMap<i32, usize> = HashMap::new();
        for p in pages {
            let all = lines(p.runs);
            let bands = style.furniture(&all, p.height, &[]);
            for (l, b) in all.iter().zip(&bands) {
                if b.is_some() {
                    continue;
                }
                for r in l.runs {
                    *chars.entry(size_key(r.size)).or_default() += r.text.chars().count();
                }
            }
        }
        // The most characters wins; on a tie the smaller size.
        style.body_size = chars
            .iter()
            .max_by_key(|(size, n)| (**n, -**size))
            .map(|(size, _)| *size as f32 / 2.0)
            .unwrap_or(0.0);
        let mut sizes: Vec<i32> = Vec::new();
        for p in pages {
            let all = lines(p.runs);
            let bands = style.furniture(&all, p.height, &[]);
            for (l, b) in all.iter().zip(&bands) {
                if b.is_none() && style.is_heading_size(l) {
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
    /// stays text.
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
        let body = || lines.iter().zip(&found).filter(|(_, b)| b.is_none());
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

    /// The heading level of an untagged line, `None` for body text.
    fn heading_level(&self, line: &Line<'_>) -> Option<u8> {
        if !self.is_heading_size(line) {
            return None;
        }
        let key = size_key(line.size);
        let index = self
            .heading_sizes
            .iter()
            .position(|s| size_key(*s) == key)
            .unwrap_or(SIZE_LEVELS - 1);
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
    /// Links written without their target, and the first and last page with one.
    cut: Counter<u64>,
    pages: Counter<Option<(u32, u32)>>,
}

/// The room counts at one point.
struct LinkMark {
    left: u64,
    cut: u64,
    pages: Option<(u32, u32)>,
}

/// What a copy of a link target costs besides its bytes.
const LINK_COST: u64 = 64;

impl LinkRoom {
    pub(crate) fn new(bytes: u64) -> LinkRoom {
        LinkRoom {
            room: bytes,
            left: Counter::new(bytes),
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

    /// The counts now, to go back to if the block they were taken for is dropped.
    fn mark(&self) -> LinkMark {
        LinkMark {
            left: self.left.get(),
            cut: self.cut.get(),
            pages: self.pages.get(),
        }
    }

    fn restore(&self, mark: LinkMark) {
        self.left.set(mark.left);
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
    /// What the table detectors left undecided or found broken: words, reason, severity.
    pub notes: Vec<(String, LossReason, Severity)>,
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

    /// Takes out the runs `pick` chooses, in order, with their slots and places.
    pub(crate) fn take(
        &mut self,
        mut pick: impl FnMut(&Run, Slot) -> bool,
    ) -> Vec<(Run, Slot, usize)> {
        let mut taken = Vec::new();
        let mut kept = Flow::default();
        let all = std::mem::take(self);
        for ((r, s), a) in all.runs.into_iter().zip(all.slots).zip(all.at) {
            if pick(&r, s) {
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
    pub width: f32,
    /// The most common font size of the file's text.
    pub body_size: f32,
}

/// Lays out the runs of one page: running headers first, then the text with its tables in
/// reading order, then running footers. Blocks are anchored `p<N>:b<K>` and tables
/// `p<N>:t<K>`, K counting each from 1.
pub(crate) fn page_blocks(
    page: u32,
    height: Option<f32>,
    runs: Vec<Run>,
    drawn: &Drawn<'_>,
    cx: &Context<'_>,
) -> Laid {
    let mut built = Built::default();
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
            let (found, fallbacks) = tables::tagged(page, &mut flow, tree, cx.tables);
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
    let mut k = 0;
    let edges = flow.take(|_, _| {
        k += 1;
        bands[k - 1].is_some()
    });
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
    if geometry && let Some(height) = height {
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
    }
    placed.sort_by(|a, b| {
        let (la, ta) = a.at();
        let (lb, tb) = b.at();
        la.cmp(&lb).then(tb.total_cmp(&ta))
    });
    let lay = |runs: &[Run], slots: &[Slot], built: &mut Built| match cx.tree {
        Some(_) => tagged(runs, slots, cx, built),
        None => untagged(runs, cx, built),
    };
    let mut start = 0;
    for p in placed {
        let (at, _) = p.at();
        let end = start + flow.at[start..].partition_point(|a| *a < at);
        lay(&flow.runs[start..end], &flow.slots[start..end], &mut built);
        start = end;
        match p {
            Placed::Table(t) => built.table(t, cx),
            Placed::Unsure(u) => {
                let what = format!(
                    "Possible table on page {page} not rebuilt (reason: {})",
                    u.reason
                );
                built.note(&what, cx);
                notes.push((what, LossReason::Uncertain, Severity::Info));
            }
        }
    }
    lay(&flow.runs[start..], &flow.slots[start..], &mut built);
    for l in lines(&footers) {
        built.paragraph(l.runs.iter().collect(), Some("page-footer"), cx);
    }
    let mut laid = built.laid(notes);
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
    laid
}

/// A table or an undecided region, placed in reading order among the text.
enum Placed {
    Table(Planned),
    Unsure(Unsure),
}

impl Placed {
    /// The place in reading order it stands at, and its top.
    fn at(&self) -> (usize, f32) {
        match self {
            Placed::Table(t) => (t.at, t.top),
            Placed::Unsure(u) => (u.at, u.top),
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
    /// The link room before the block (or the table row) now being built, to go back to if
    /// it is dropped.
    mark: Option<LinkMark>,
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
            notes,
        }
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

    /// Builds a planned table: a new table on this page, or rows for a tagged table begun on an
    /// earlier page. The table is one element, charged first; a stop inside it keeps the rows
    /// built before.
    fn table(&mut self, plan: Planned, cx: &Context<'_>) {
        if self.stop.is_some() {
            return;
        }
        let earlier = plan.tagged.filter(|_| plan.continues);
        let table = match earlier {
            Some(t) => TableRef::Earlier(t),
            None => {
                self.pending = Pending::Table;
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
            self.mark = Some(cx.links.mark());
            let mut cells = Vec::with_capacity(row.len());
            for c in row {
                let inlines = if c.runs.is_empty() {
                    Vec::new()
                } else {
                    let refs: Vec<&Run> = c.runs.iter().collect();
                    match self.inlines(&refs, cx) {
                        Some(i) => i,
                        None => break 'rows,
                    }
                };
                cells.push(Cell {
                    inlines,
                    row_span: c.row_span,
                    col_span: c.col_span,
                    anchor: None,
                });
            }
            self.mark = None;
            rows.push(cells);
        }
        self.mark = None;
        if rows.is_empty() {
            return;
        }
        match earlier {
            Some(t) => self.continued.push((t, rows)),
            None => {
                self.tagged.push(plan.tagged);
                self.blocks.push(Block::Table {
                    caption: None,
                    header_rows: plan.header_rows.min(rows.len()),
                    role: None,
                    rows,
                    alignments: Vec::new(),
                    anchor: None,
                })
            }
        }
    }

    /// The inlines of `runs`, or `None` once a limit stopped the page. Each stretch of runs
    /// under one link becomes one link with its own copy of the target, charged to the link
    /// room.
    fn inlines(&mut self, runs: &[&Run], cx: &Context<'_>) -> Option<Vec<Inline>> {
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
        match inlines(runs, cx.links, cx.budget) {
            Ok(built) => Some(built),
            Err(hit) => {
                self.stop_at(hit, cx);
                None
            }
        }
    }

    fn paragraph(&mut self, runs: Vec<&Run>, role: Option<&str>, cx: &Context<'_>) {
        self.pending = Pending::Block;
        if let Some(inlines) = self.inlines(&runs, cx) {
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
/// after a hyphen. Bold, italic and URI links are kept while `links` has room for their
/// targets; past it a link's text stays without the link.
fn inlines(runs: &[&Run], links: &LinkRoom, budget: &Budget) -> Result<Vec<Inline>, LimitHit> {
    struct Piece {
        text: String,
        bold: bool,
        italic: bool,
        uri: Option<Rc<UriTarget>>,
        page: u32,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut last: Option<(u32, u32)> = None;
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
        last = Some((r.page, r.line));
        if text.is_empty() {
            continue;
        }
        let glued = pieces.last().is_none_or(|p| p.text.ends_with(['-', ' ']));
        if new_line && !glued {
            text.insert(0, ' ');
        }
        match pieces.last_mut() {
            Some(p) if p.bold == r.bold && p.italic == r.italic && p.uri == r.uri => {
                p.text.push_str(&text)
            }
            _ => pieces.push(Piece {
                text,
                bold: r.bold,
                italic: r.italic,
                uri: r.uri.clone(),
                page: r.page,
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
    for (k, p) in pieces.into_iter().enumerate() {
        if k % CHECK_EVERY == 0 {
            time_over(budget)?;
        }
        if p.text.is_empty() {
            continue;
        }
        // White space at the edge of a link belongs outside it.
        let (inner, lead) = match p.uri {
            Some(_) => {
                let lead = p.text.len() - p.text.trim_start().len();
                (p.text[lead..].to_string(), &p.text[..lead])
            }
            None => (p.text.clone(), ""),
        };
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
    Ok(out)
}

/// Whether `cur` goes on in the same paragraph as `prev`: below it, near enough and in the
/// same size.
fn continues(prev: &Line<'_>, cur: &Line<'_>) -> bool {
    let gap = prev.y - cur.y;
    let height = prev.h.max(cur.h).max(1.0);
    gap > 0.0 && gap <= GAP_RATIO * height && (prev.size - cur.size).abs() <= SIZE_SLACK
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
        if rest.starts_with(char::is_whitespace) {
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

/// Lays out an untagged page: headings by size, lists by marker, paragraphs by gaps.
fn untagged(flow: &[Run], cx: &Context<'_>, out: &mut Built) {
    let all = lines(flow);
    let mut i = 0;
    let mut open_list: Option<Option<usize>> = None;
    while i < all.len() && out.stop.is_none() {
        let l = &all[i];
        if l.text.is_empty() {
            i += 1;
            continue;
        }
        if let Some(level) = cx.style.heading_level(l) {
            let mut runs: Vec<&Run> = l.runs.iter().collect();
            let mut j = i + 1;
            while j < all.len()
                && cx.style.heading_level(&all[j]) == Some(level)
                && continues(&all[j - 1], &all[j])
            {
                runs.extend(all[j].runs.iter());
                j += 1;
            }
            out.heading(level, runs, cx);
            open_list = None;
            i = j;
            continue;
        }
        if let Some((ordered, cut)) = list_marker(&l.text) {
            let owned = without_marker(&l.runs.iter().collect::<Vec<_>>(), cut);
            let mut extra: Vec<&Run> = Vec::new();
            let mut j = i + 1;
            while j < all.len()
                && cx.style.heading_level(&all[j]).is_none()
                && list_marker(&all[j].text).is_none()
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
            && cx.style.heading_level(&all[j]).is_none()
            && list_marker(&all[j].text).is_none()
            && continues(&all[j - 1], &all[j])
        {
            runs.extend(all[j].runs.iter());
            j += 1;
        }
        out.paragraph(runs, None, cx);
        i = j;
    }
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
        let (slot, actual) = match r.mcid.and_then(|m| (cx.roles)(m)) {
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
                Slot::Free | Slot::Cell { .. } => {
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
            Slot::Cell { .. } => Slot::Free,
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
        width: 595.0,
        body_size: 10.0,
    };

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
            mcid: None,
            uri: None,
            line,
            off_axis: false,
        }
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
        let built = inlines(&refs, &room, &Budget::new(rezzer_limits::Limits::default())).unwrap();
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
            links: &room,
        };
        let mut built = Built::default();
        built.paragraph(runs.iter().collect(), None, &cx);
        assert!(built.blocks.is_empty());
        assert!(built.stop.is_some());
        assert_eq!(room.cut(), None);
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
                r.mcid = Some(k as u32);
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
            r.mcid = Some(i as u32);
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
                    r.mcid = Some(k as u32);
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
