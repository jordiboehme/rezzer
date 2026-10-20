// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! From positioned text runs to blocks: running headers and footers, headings, paragraphs,
//! lists and tagged tables.
//!
//! A tagged file (`/MarkInfo /Marked true`) gets its roles from the structure tree. An
//! untagged file gets headings from the font size and lists from their markers. Lines come in
//! reading order; a paragraph ends at a vertical gap over 1.5 times the line height, at a
//! change of font size and where the text moves up (the next column).

use std::cell::{Cell as Counter, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pdfparts::StructTree;
use rezzer_limits::{Budget, LimitHit};
use rezzer_model::{Anchor, Block, Cell, Inline};

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
    pub uri: Option<Rc<str>>,
    /// The number of the run's line on its page, in reading order. The runs of one line are
    /// next to each other.
    pub line: u32,
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
            let bands = style.furniture(&all, p.height);
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
            let bands = style.furniture(&all, p.height);
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
    fn furniture(&self, lines: &[Line<'_>], height: Option<f32>) -> Vec<Option<Band>> {
        let found: Vec<Option<Band>> = lines
            .iter()
            .map(|l| {
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
/// holds its own copy of its target, and the renderer writes each copy again, so they are
/// charged by their bytes. Once a copy does not fit, no later link keeps its target: the
/// text stays, without the link.
#[derive(Debug)]
pub(crate) struct LinkRoom {
    room: u64,
    left: Counter<u64>,
    spent: Counter<bool>,
    /// Links written without their target, and the first and last page with one.
    cut: Counter<u64>,
    pages: Counter<Option<(u32, u32)>>,
}

/// What a copy of a link target costs besides its bytes.
const LINK_COST: u64 = 64;

impl LinkRoom {
    pub(crate) fn new(bytes: u64) -> LinkRoom {
        LinkRoom {
            room: bytes,
            left: Counter::new(bytes),
            spent: Counter::new(false),
            cut: Counter::new(0),
            pages: Counter::new(None),
        }
    }

    /// Takes room for one copy of `target` on `page`; `false` once the room is spent.
    fn take(&self, target: &str, page: u32) -> bool {
        let cost = target.len() as u64 + LINK_COST;
        if !self.spent.get() && cost <= self.left.get() {
            self.left.set(self.left.get() - cost);
            return true;
        }
        self.spent.set(true);
        self.cut.set(self.cut.get() + 1);
        let (first, _) = self.pages.get().unwrap_or((page, page));
        self.pages.set(Some((first, page)));
        false
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
}

/// Lays out the runs of one page: running headers first, then the text, then running
/// footers. Blocks are anchored `p<N>:b<K>`, K counting from 1.
pub(crate) fn page_blocks(
    page: u32,
    height: Option<f32>,
    runs: Vec<Run>,
    cx: &Context<'_>,
) -> Laid {
    if let Err(hit) = cx
        .budget
        .add_elements(u64::try_from(runs.len()).unwrap_or(u64::MAX))
    {
        return Laid {
            blocks: Vec::new(),
            stop: Some(hit),
        };
    }
    let mut built = Built::default();
    let mut flow: Vec<Run> = Vec::with_capacity(runs.len());
    let mut headers = Vec::new();
    let mut footers = Vec::new();
    let all = lines(&runs);
    let bands = cx.style.furniture(&all, height);
    for (l, b) in all.iter().zip(bands) {
        match b {
            Some(Band::Top) => headers.push(l.runs),
            Some(Band::Bottom) => footers.push(l.runs),
            None => flow.extend_from_slice(l.runs),
        }
    }
    // One list of blocks for the whole page, each charged as it is built: a stop keeps the
    // blocks before it.
    for runs in headers {
        built.paragraph(runs.iter().collect(), Some("page-header"), cx);
    }
    let tables = claimed_tables(page, &mut flow);
    match cx.tree {
        Some(tree) => tagged(&flow, tree, cx, &mut built),
        None => untagged(&flow, cx, &mut built),
    }
    for (_, table) in tables {
        built.push(table, cx.budget);
    }
    for runs in footers {
        built.paragraph(runs.iter().collect(), Some("page-footer"), cx);
    }
    let mut blocks = built.blocks;
    for (k, block) in blocks.iter_mut().enumerate() {
        set_anchor(block, Anchor(format!("p{page}:b{}", k + 1)));
    }
    Laid {
        blocks,
        stop: built.stop,
    }
}

/// Tables found without tags, each with the y of its top. The runs a table holds are taken
/// out of `flow` before paragraphs are built, so no paragraph repeats them. Untagged tables
/// are not rebuilt yet: this finds none.
fn claimed_tables(_page: u32, _flow: &mut Vec<Run>) -> Vec<(f32, Block)> {
    Vec::new()
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

/// Blocks being built, and the limit that stopped them.
#[derive(Default)]
struct Built {
    blocks: Vec<Block>,
    stop: Option<LimitHit>,
}

impl Built {
    /// Adds a block, charged as one element. Past the limit the block is dropped and the page
    /// stops.
    fn push(&mut self, block: Block, budget: &Budget) {
        if self.stop.is_some() {
            return;
        }
        match budget.add_elements(1) {
            Ok(()) => self.blocks.push(block),
            Err(hit) => self.stop = Some(hit),
        }
    }

    /// The inlines of `runs`, or `None` once a limit stopped the page. Each stretch of runs
    /// under one link becomes one link with its own copy of the target, charged to the link
    /// room.
    fn inlines(&mut self, runs: &[&Run], cx: &Context<'_>) -> Option<Vec<Inline>> {
        if self.stop.is_some() {
            return None;
        }
        let bytes: usize = runs.iter().map(|r| r.text.len() + 1).sum();
        if let Err(hit) = cx.budget.charge_text(bytes) {
            self.stop = Some(hit);
            return None;
        }
        Some(inlines(runs, cx.links))
    }

    fn paragraph(&mut self, runs: Vec<&Run>, role: Option<&str>, cx: &Context<'_>) {
        if let Some(inlines) = self.inlines(&runs, cx) {
            self.push(
                Block::Paragraph {
                    inlines,
                    role: role.map(str::to_string),
                    anchor: None,
                },
                cx.budget,
            );
        }
    }

    fn heading(&mut self, level: u8, runs: Vec<&Run>, cx: &Context<'_>) {
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
                cx.budget,
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
        let Some(inlines) = self.inlines(&runs, cx) else {
            return;
        };
        let para = Block::paragraph(inlines);
        if *open == Some(list)
            && let Some(Block::List { items, .. }) = self.blocks.last_mut()
        {
            items.push(vec![para]);
            return;
        }
        self.push(
            Block::List {
                ordered,
                items: vec![vec![para]],
                anchor: None,
            },
            cx.budget,
        );
        *open = Some(list);
    }
}

/// Inlines from runs in order. Runs on a new line are joined with a space, or without one
/// after a hyphen. Bold, italic and URI links are kept while `links` has room for their
/// targets; past it a link's text stays without the link.
fn inlines(runs: &[&Run], links: &LinkRoom) -> Vec<Inline> {
    struct Piece {
        text: String,
        bold: bool,
        italic: bool,
        uri: Option<Rc<str>>,
        page: u32,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut last: Option<(u32, u32)> = None;
    for r in runs {
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
    let mut refused: Option<Rc<str>> = None;
    for p in pieces {
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
                    && before.as_str() == &*target
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
                    target: target.to_string(),
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
    out
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
enum Slot {
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

/// Lays out a tagged page: runs of one element form one block, in reading order.
fn tagged(flow: &[Run], tree: &StructTree, cx: &Context<'_>, out: &mut Built) {
    let mut slots = Vec::with_capacity(flow.len());
    // ActualText of an element replaces the text of all its runs: the first run carries it.
    let mut replaced: HashSet<usize> = HashSet::new();
    let mut runs: Vec<Option<Run>> = Vec::with_capacity(flow.len());
    for r in flow {
        let (slot, actual) = match r.mcid.and_then(|m| (cx.roles)(m)) {
            Some(e) if e < tree.elems.len() => classify(tree, e),
            _ => (Slot::Free, None),
        };
        slots.push(slot);
        match actual {
            Some(a) if !replaced.insert(a) => runs.push(None),
            Some(a) => {
                let mut r = r.clone();
                let space = if r.text.starts_with(' ') { " " } else { "" };
                r.text = format!(
                    "{space}{}",
                    tree.elems[a].actual_text.as_deref().unwrap_or("")
                );
                runs.push(Some(r));
            }
            None => runs.push(Some(r.clone())),
        }
    }
    // The runs of each table cell on this page.
    let mut cells: HashMap<usize, Vec<&Run>> = HashMap::new();
    for (slot, r) in slots.iter().zip(&runs) {
        if let (Slot::Cell { cell, .. }, Some(r)) = (slot, r) {
            cells.entry(*cell).or_default().push(r);
        }
    }
    let mut tables_done: HashSet<usize> = HashSet::new();
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
                Slot::Free => {
                    *open = None;
                    out.paragraph(taken, None, cx);
                }
                Slot::Cell { .. } => {}
            }
        };
    for (i, slot) in slots.iter().enumerate() {
        if out.stop.is_some() {
            return;
        }
        let Some(r) = runs[i].as_ref() else { continue };
        if let Slot::Cell { table, .. } = slot {
            flush(out, &mut current, current_slot, &mut open_list);
            current_slot = Slot::Free;
            if tables_done.insert(*table) {
                open_list = None;
                if let Some(t) = tagged_table(tree, *table, &cells, cx, out) {
                    out.push(t, cx.budget);
                }
            }
            continue;
        }
        let same = *slot == current_slot
            && match slot {
                Slot::Free => {
                    let (a, b) = (line_of[current_last], line_of[i]);
                    current.is_empty() || a == b || continues(&all_lines[a], &all_lines[b])
                }
                _ => true,
            };
        if !same {
            flush(out, &mut current, current_slot, &mut open_list);
            current_slot = *slot;
        }
        current.push(r);
        current_last = i;
    }
    flush(out, &mut current, current_slot, &mut open_list);
}

/// The rows of the tagged tables met so far, each table read once for all its pages.
#[derive(Default)]
pub(crate) struct Tables(RefCell<HashMap<usize, Rc<TableRows>>>);

/// The rows of one tagged table: the `TD` and `TH` cells of each row, and the row of each
/// cell.
struct TableRows {
    rows: Vec<Vec<usize>>,
    row_of: HashMap<usize, usize>,
}

impl Tables {
    fn rows(&self, tree: &StructTree, table: usize) -> Option<Rc<TableRows>> {
        if let Some(found) = self.0.borrow().get(&table) {
            return Some(found.clone());
        }
        let elems = &tree.elems;
        let mut trs: Vec<usize> = Vec::new();
        for &c in &elems.get(table)?.children {
            match elems.get(c).map(|e| e.role.as_str()) {
                Some("TR") => trs.push(c),
                Some("THead" | "TBody" | "TFoot") => trs.extend(
                    elems[c]
                        .children
                        .iter()
                        .copied()
                        .filter(|&r| elems.get(r).is_some_and(|e| e.role == "TR")),
                ),
                _ => {}
            }
        }
        let mut rows = Vec::with_capacity(trs.len());
        let mut row_of = HashMap::new();
        for (i, tr) in trs.into_iter().enumerate() {
            let cells: Vec<usize> = elems[tr]
                .children
                .iter()
                .copied()
                .filter(|&c| {
                    elems
                        .get(c)
                        .is_some_and(|e| matches!(e.role.as_str(), "TD" | "TH"))
                })
                .collect();
            for &c in &cells {
                row_of.entry(c).or_insert(i);
            }
            rows.push(cells);
        }
        let found = Rc::new(TableRows { rows, row_of });
        self.0.borrow_mut().insert(table, found.clone());
        Some(found)
    }
}

/// A table from the tag tree, with the rows that have text on this page. Header rows are the
/// leading rows made only of `TH` cells. The work is bounded by the cells on the page.
fn tagged_table(
    tree: &StructTree,
    table: usize,
    cells: &HashMap<usize, Vec<&Run>>,
    cx: &Context<'_>,
    out: &mut Built,
) -> Option<Block> {
    let elems = &tree.elems;
    let index = cx.tables.rows(tree, table)?;
    let mut touched: Vec<usize> = cells
        .keys()
        .filter_map(|c| index.row_of.get(c).copied())
        .collect();
    touched.sort_unstable();
    touched.dedup();
    let mut built: Vec<Vec<Cell>> = Vec::new();
    let mut header_rows = 0;
    let mut in_header = true;
    for row in touched {
        let row_cells = &index.rows[row];
        let mut cells_out = Vec::with_capacity(row_cells.len());
        for c in row_cells {
            let e = &elems[*c];
            let runs = cells.get(c).cloned().unwrap_or_default();
            let inlines = if runs.is_empty() {
                Vec::new()
            } else {
                out.inlines(&runs, cx)?
            };
            cells_out.push(Cell {
                inlines,
                row_span: e.row_span.max(1),
                col_span: e.col_span.max(1),
                anchor: None,
            });
        }
        if in_header && row_cells.iter().all(|c| elems[*c].role == "TH") {
            header_rows += 1;
        } else {
            in_header = false;
        }
        built.push(cells_out);
    }
    if built.is_empty() {
        return None;
    }
    Some(Block::Table {
        caption: None,
        header_rows,
        role: None,
        rows: built,
        alignments: Vec::new(),
        anchor: None,
    })
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
        let uri: Rc<str> = Rc::from("https://example.org/long");
        let runs: Vec<Run> = (0..6)
            .map(|k| Run {
                bold: k % 2 == 0,
                uri: Some(uri.clone()),
                ..run(if k == 0 { "a" } else { " b" }, 0, 700.0, 10.0)
            })
            .collect();
        let refs: Vec<&Run> = runs.iter().collect();
        let built = inlines(&refs, &LinkRoom::new(u64::MAX));
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
        let a: Rc<str> = Rc::from("https://example.org/a");
        let b: Rc<str> = Rc::from("https://example.org/a-longer-target");
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
        let room = LinkRoom::new(2 * (a.len() as u64 + LINK_COST));
        let built = inlines(&refs, &room);
        assert!(
            matches!(&built[0], Inline::Link { target, .. } if target == "https://example.org/a"),
            "{built:?}"
        );
        assert!(
            built[1..].iter().all(|i| !matches!(i, Inline::Link { .. })),
            "{built:?}"
        );
        assert_eq!(rezzer_model::plain_text(&built), "one two three four");
        // The link over the change of style is one link cut; the last link is cut too, though
        // it would fit, as the room is spent.
        assert_eq!(room.cut(), Some((2, 1, 1)));
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
        let laid = page_blocks(1, Some(842.0), runs, &cx);
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
        let laid = page_blocks(1, Some(842.0), runs, &cx);
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
            let laid = page_blocks(page as u32 + 1, None, runs, &cx);
            match laid.blocks.as_slice() {
                [Block::Table { rows: r, .. }] => rows += r.len(),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(rows, ROWS);
        assert!(start.elapsed().as_secs() < 3, "{:?}", start.elapsed());
    }
}
