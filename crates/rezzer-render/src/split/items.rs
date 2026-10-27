// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The items a document is cut into and the specs that carry them.

use super::*;

/// A section around a unit's blocks: the unit keeps its heading and context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Wrap<'a> {
    pub(super) kind: SectionKind,
    pub(super) title: Option<&'a str>,
    pub(super) anchor: Option<&'a str>,
}

/// What a unit holds: a run of blocks of one list, or blocks made by `split_rows`.
#[derive(Debug, Clone)]
pub(super) enum Content<'a> {
    Slice {
        list: &'a [Block],
        from: usize,
        to: usize,
    },
    Owned(Vec<Block>),
}

impl<'a> Content<'a> {
    pub(super) fn blocks(&self) -> &[Block] {
        match self {
            Content::Slice { list, from, to } => &list[*from..*to],
            Content::Owned(blocks) => blocks,
        }
    }
}

/// How a unit over the limit is cut further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Expand {
    /// Each block on its own: the pages of an outline chapter.
    Group,
    /// Into the one section it is: its blocks inside its heading.
    Section,
    /// At its headings of the next level, else into blocks.
    Chunk,
    /// One block with what stays with it: a table into row ranges, anything else as text.
    Atom,
    /// Row ranges and other made blocks: only as text.
    Text,
}

/// A candidate unit.
#[derive(Debug, Clone)]
pub(super) struct Item<'a> {
    pub(super) base: String,
    pub(super) title: String,
    pub(super) location: String,
    /// The units cut from one titled unit share it; the second on get `, part <n>`.
    pub(super) family: Option<usize>,
    /// The pages it holds (first and last).
    pub(super) pages: Option<(u32, u32)>,
    /// Packed, it is named `pages-<a>-<b>` (pages without an outline).
    pub(super) page_names: bool,
    pub(super) wraps: Vec<Wrap<'a>>,
    pub(super) content: Content<'a>,
    pub(super) expand: Expand,
    /// It holds the last block of every section around it: a page's own notes close it.
    pub(super) last: bool,
    /// How many blocks at its start are headings of a part around it (a chapter heading
    /// that went with its first section): they stay with its first piece.
    pub(super) lead: usize,
    /// Only a heading: it goes with the item after it when it cannot be packed with it.
    pub(super) heading_only: bool,
    /// Rezzer made it and titled it itself (`Start`, `Footnotes`): its title never changes
    /// for a clash.
    pub(super) rezzer: bool,
    /// The unit on the top level it lies in: the final packing never joins two of them.
    pub(super) top: usize,
    /// The paragraphs it holds, counted in its part around it from 1: they make its
    /// location more exact ([`Item::shown_location`]).
    pub(super) paragraphs: Option<(usize, usize)>,
    /// Rows of a table: the title of the unit that holds the table. Its lead and tail pieces
    /// carry that title, so a part that packs them with the rows is named by the rows.
    pub(super) table: Option<String>,
    /// The units it holds when packed in the final pass, by title (ruling P5-AH); empty for
    /// one unit.
    pub(super) held: Vec<String>,
}

impl Item<'_> {
    /// Where it lies, with its paragraphs: `Start, paragraphs 12 to 40`, `Intro, paragraph 3`.
    pub(super) fn shown_location(&self) -> String {
        match self.paragraphs {
            Some((a, b)) if a == b => {
                capped(&format!("{}, paragraph {a}", self.location), TITLE_CAP)
            }
            Some((a, b)) => capped(
                &format!("{}, paragraphs {a} to {b}", self.location),
                TITLE_CAP,
            ),
            None => self.location.clone(),
        }
    }

    /// The pages it holds: its own, else those of the page it lies in.
    pub(super) fn on_pages(&self) -> Option<(u32, u32)> {
        self.pages.or_else(|| {
            let page = self
                .wraps
                .iter()
                .rev()
                .find(|w| w.kind == SectionKind::Page)?;
            let n = page.anchor?.strip_prefix('p')?.parse().ok()?;
            Some((n, n))
        })
    }

    /// The titles of the units it holds.
    pub(super) fn held(&self) -> Vec<String> {
        if self.held.is_empty() {
            vec![self.title.clone()]
        } else {
            self.held.clone()
        }
    }
}

/// A unit as planned: the item, and how many pieces its text is cut into (more than one only
/// for a block cut as text).
#[derive(Debug)]
pub(super) struct Spec<'a> {
    pub(super) item: Item<'a>,
    /// A name part after the base, for row ranges.
    pub(super) suffix: String,
    pub(super) pieces: usize,
    /// The runs of blocks whose anchors lead to this unit; none for its own slice.
    pub(super) anchors: Vec<(&'a [Block], usize, usize)>,
    /// The anchors of sections it holds besides its wraps (a packed unit's inner sections).
    pub(super) wrap_anchors: Vec<&'a str>,
    /// Its size by [`Measure`], when the plan measured it.
    pub(super) cost: Option<usize>,
}

/// A spec for an item that fits.
pub(super) fn spec(item: Item<'_>) -> Spec<'_> {
    Spec {
        item,
        suffix: String::new(),
        pieces: 1,
        anchors: Vec::new(),
        wrap_anchors: Vec::new(),
        cost: None,
    }
}

/// A spec for an item that fits, measured at `cost`.
pub(super) fn measured(item: Item<'_>, cost: usize) -> Spec<'_> {
    Spec {
        cost: Some(cost),
        ..spec(item)
    }
}

/// A spec for an item cut as text: as many pieces as its trial render gives.
pub(super) fn cut_spec<'a>(mut item: Item<'a>, splitter: &mut Splitter<'a>) -> Spec<'a> {
    let mark = splitter.trial.checkpoint();
    let text = splitter.render(&item.wraps, &item.content, item.last, &splitter.trial, None);
    splitter.trial.rollback(mark);
    let pieces = splitter.pieces(&text).len().max(1);
    if pieces > 1 && item.family.is_none() {
        item.family = splitter.family(&item);
    }
    Spec {
        item,
        suffix: String::new(),
        pieces,
        anchors: Vec::new(),
        wrap_anchors: Vec::new(),
        cost: None,
    }
}

/// A row range cut again into ranges of at most `max` tokens, with the same lead, header
/// band and notes. A table without layout counts its own rows, so the new ranges' numbers
/// are moved to the rows of the whole table.
pub(super) fn resplit(range: &rezzer_model::RowRange, max: usize) -> Vec<rezzer_model::RowRange> {
    let Some(t) = range.table else {
        return Vec::new();
    };
    let blocks = &range.blocks;
    let mut sub = split_rows(&blocks[..t], &blocks[t], &blocks[t + 1..], max);
    if let Block::Table {
        layout: None,
        header_rows,
        ..
    } = &blocks[t]
    {
        let offset = range.first.saturating_sub(*header_rows as u32 + 1);
        for r in &mut sub {
            if r.first > 0 {
                r.first += offset;
                r.last += offset;
            }
        }
    }
    sub
}

/// Whether `next` can be packed after `prev`: both runs of one block list, next to each
/// other, inside the same sections.
pub(super) fn packs(prev: &Item<'_>, next: &Item<'_>) -> bool {
    match (&prev.content, &next.content) {
        (Content::Slice { list: a, to, .. }, Content::Slice { list: b, from, .. }) => {
            std::ptr::eq(*a, *b) && to == from && prev.wraps == next.wraps
        }
        _ => false,
    }
}

/// Packed items as one: the first one's name (`pages-<a>-<b>` for pages without an
/// outline), `<first> to <last>` as location where they differ, and every item in the
/// title ([`listed`]).
pub(super) fn merge<'a>(group: &[Item<'a>]) -> Item<'a> {
    let first = &group[0];
    let last = &group[group.len() - 1];
    let mut item = first.clone();
    if let (Content::Slice { to, .. }, Content::Slice { to: end, .. }) =
        (&mut item.content, &last.content)
    {
        *to = *end;
    }
    item.last = last.last;
    let pages = match (first.pages, last.pages) {
        (Some((a, _)), Some((_, b))) if group.iter().all(|g| g.pages.is_some()) => Some((a, b)),
        _ => None,
    };
    item.pages = pages;
    item.rezzer = group.iter().all(|g| g.rezzer);
    let paragraphs = match (first.paragraphs, last.paragraphs) {
        (Some((a, _)), Some((_, b)))
            if group
                .iter()
                .all(|g| g.paragraphs.is_some() && g.location == first.location) =>
        {
            Some((a, b))
        }
        _ => None,
    };
    item.paragraphs = None;
    item.location = match (pages, paragraphs) {
        (Some(p), _) => pages_location(p),
        (None, Some(_)) => {
            item.paragraphs = paragraphs;
            first.location.clone()
        }
        (None, None) => joined_location(&first.shown_location(), &last.shown_location()),
    };
    if first.page_names && pages.is_some() {
        let (a, b) = pages.unwrap_or_default();
        item.base = if a == b {
            format!("page-{a}")
        } else {
            format!("pages-{a}-{b}")
        };
        item.title = item.location.clone();
    } else if group
        .iter()
        .all(|g| g.family.is_some() && g.family == first.family)
    {
        item.title = first.title.clone();
    } else {
        // Every packed item by name (ruling P5-AH).
        let titles: Vec<String> = group.iter().map(|g| g.title.clone()).collect();
        item.title = capped(&listed(&titles), TITLE_CAP);
    }
    item
}

/// The words of a heading for a part's title and file name: its plain text, with inline HTML
/// tags left out of its text when `html` (only the Markdown reader keeps inline HTML, as text;
/// the model has no inline HTML of its own). Code keeps its text whole, so `<div>` in code
/// and `Vec<T>` from a DOCX heading stay.
pub(super) fn heading_words(list: &[Inline], html: bool) -> String {
    let mut out = String::new();
    for inline in list {
        match inline {
            Inline::Text { text, .. } if html => {
                out.push_str(&rezzer_model::text::strip_html_tags(text));
            }
            Inline::Link { inlines, .. } => out.push_str(&heading_words(inlines, html)),
            other => out.push_str(&rezzer_model::plain_text(std::slice::from_ref(other))),
        }
    }
    out
}

/// `<first> to <last>` as the location of packed units, or the one location they share.
pub(super) fn joined_location(a: &str, b: &str) -> String {
    if a == b {
        a.to_string()
    } else {
        capped(&format!("{a} to {b}"), TITLE_CAP)
    }
}

/// The column profile the CSV reader writes for `table` (`list[t]`): a table of role
/// `summary` after it in `list`, captioned `Columns <range>` with the table's range, or
/// `Columns` when `list` holds no other table.
pub(super) fn column_profile<'x>(list: &'x [Block], t: usize, table: &Block) -> Option<&'x Block> {
    let Block::Table { layout, .. } = table else {
        return None;
    };
    let range = layout.as_ref().map(|l| l.range.as_str());
    let summary = |b: &Block| matches!(b, Block::Table { role: Some(r), .. } if r == "summary");
    let tables = list
        .iter()
        .filter(|b| matches!(b, Block::Table { .. }) && !summary(b))
        .count();
    list[t + 1..].iter().find(|b| match b {
        Block::Table {
            caption: Some(caption),
            ..
        } if summary(b) => match caption.strip_prefix("Columns") {
            Some("") => tables == 1,
            Some(rest) => rest.strip_prefix(' ').is_some_and(|r| Some(r) == range),
            None => false,
        },
        _ => false,
    })
}

/// The blocks of `range`, with `profile` right after its table.
pub(super) fn with_profile(range: &rezzer_model::RowRange, profile: Option<&Block>) -> Vec<Block> {
    let mut blocks = range.blocks.clone();
    if let (Some(p), Some(t)) = (profile, range.table) {
        blocks.insert(t + 1, p.clone());
    }
    blocks
}

/// The lead of `range` and its table's header band, without data rows: what every range
/// of the table repeats.
pub(super) fn header_only(range: &rezzer_model::RowRange) -> Vec<Block> {
    let Some(t) = range.table else {
        return range.blocks.clone();
    };
    let mut blocks = range.blocks[..=t].to_vec();
    if let Some(Block::Table {
        rows, header_rows, ..
    }) = blocks.last_mut()
    {
        rows.truncate(*header_rows);
    }
    blocks
}

/// `page 12` or `pages 12 to 20` (decision 10).
pub(super) fn pages_location((a, b): (u32, u32)) -> String {
    if a == b {
        format!("page {a}")
    } else {
        format!("pages {a} to {b}")
    }
}
