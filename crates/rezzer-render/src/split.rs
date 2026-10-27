// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The splitter (spec 5, decisions 9, 10 and 12): a document whose body is over the part size
//! is cut into units at its own borders, each within the limit.
//!
//! Borders, coarse to fine: the top-level sections (sheet, chapter, slide, part, message);
//! PDF pages grouped into the chapters of the outline's top level, else pages; then the
//! headings of the smallest level; then blocks, a heading kept with the block after it; a
//! table with its heading and caption before it and its notes after it is cut into row
//! ranges (`split_rows` at a quarter of the part, a range over the limit cut again with half
//! the size up to 8 times), which are joined again while they fit; a CSV column profile goes
//! with the first rows of its table. One block still over the limit is cut as text by
//! [`crate::paging::cut`]. Units of one level next to each other are packed while they fit,
//! and a last pass packs neighbours inside one top-level unit whatever level made them: a
//! heading with the rows after it, the last rows with the notes and pages after them.
//!
//! Each unit is measured by [`Measure`] against the part size less the worst case a part
//! file adds around it (title line, description, `## Relations`, F11). The measure counts
//! the worst case for what the place of the unit changes, so the final render, which knows
//! the file of every unit, can only be smaller:
//! - an anchor link (`#x`) to a unit in another file becomes `<file>.md#x`;
//! - a relative link and a picture get `../` (the part lies one folder down);
//! - a link to a file counts at its import length (decision 12).
//!
//! The trailing sections follow the content, each a unit or cut like one: `properties`,
//! `logic` (cut per sheet at its `###` parts), `annotations` (those not shown at their
//! passage), `changes`, `embedded` and, for a PDF, the full `outline`. Overview and Not read
//! belong to the main document. Time and memory stay linear in the size of the document:
//! each level renders what it measures once, and a retry is capped.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use rezzer_model::text::{heading_slug, percent_decode};
use rezzer_model::{Block, Completeness, Document, FormatId, Inline, SectionKind, split_rows};

use crate::blocks::{self, Ctx, MAX_DEPTH, Notes};
use crate::describe::{Measure, body_tokens};
use crate::{OutputSection, RenderOptions, Selection, chosen_sheets, facets, frontmatter, paging};

mod items;
mod links;
mod names;
mod pack;
mod plan;
mod walk;
mod write;

use items::*;
use links::*;
pub(crate) use names::*;
use walk::*;
use write::*;

/// One unit of a split document: what a part file holds besides its frontmatter, title and
/// relations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// The part's file name without `.md` (decision 9): lowercase `[a-z0-9-]`, at most 60
    /// bytes, unique in the document, never `index`, `log`, `overview`, `not-read` or
    /// `contents`.
    pub file: String,
    /// The unit's own title (at most `TITLE_CAP` characters and `, part <n>`), else its
    /// location.
    pub title: String,
    /// Where the unit lies in the document (decision 10): `pages 12 to 20`, `Data!A1:F400`,
    /// a sheet or heading title, `<first> to <last>`, or a section name such as `Logic`.
    pub location: String,
    /// The body only: no frontmatter, no title, no relations.
    pub markdown: String,
    /// How `title` is made.
    pub(crate) naming: Naming,
}

/// Unit titles and locations, and the document title in a part's title, are cut to this many
/// characters ([`capped`]). The part overhead (F11) counts with it, so the part writer must
/// cap the document title the same way.
pub(crate) const TITLE_CAP: usize = 120;

/// The longest file name of a part, without `.md`.
const NAME_CAP: usize = 60;

/// The longest table key in a row range name.
const KEY_CAP: usize = 24;

/// The names a part never takes: Crystalline reserves `index` and `log`, the main document
/// writer `overview`, `not-read` and `contents`, and Windows the device names (`con.md`
/// opens the console there). A part name is a slug (lowercase ASCII letters, digits and
/// `-`, no dot or space), so these are the only forms a device name can take in it; they
/// are reserved on every OS, so a parts folder is legal on Windows wherever it was made.
const RESERVED: [&str; 27] = [
    "index", "log", "overview", "not-read", "contents", "con", "prn", "aux", "nul", "com1", "com2",
    "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5",
    "lpt6", "lpt7", "lpt8", "lpt9",
];

/// `split_rows` gets its `max_tokens` halved at most this often.
const ROW_RETRIES: usize = 8;

/// Cuts `doc` into units of at most `part_size` tokens each (decisions 9, 10 and 12).
/// `None` when the whole rendered body fits, and always for `part_size` 0. `bundle` is the
/// folder the parts are written to, next to the main document.
pub fn split(
    doc: &Document,
    opts: &RenderOptions,
    part_size: usize,
    bundle: &str,
) -> Option<Vec<Unit>> {
    if part_size == 0 {
        return None;
    }
    let whole = crate::render(doc, Completeness::Complete, opts);
    if body_tokens(&whole, main_note_len(doc)) <= part_size {
        return None;
    }
    Some(Splitter::new(doc, opts, part_size, bundle).run())
}

/// The longest a main document's name without `.md` may be, in bytes (decision 8). The
/// writer cuts a longer name and adds the name hash, so the file name with `.md` or `.json`
/// stays under the 255 bytes of common file systems and `part_budget` never falls to its
/// floor.
pub const MAIN_NAME_MAX: usize = 200;

/// The longest the main document's path without `.md` can be (decision 8): the file name's
/// last component, plus `-<8 hex>` when it gets a hash, at most [`MAIN_NAME_MAX`].
fn main_note_len(doc: &Document) -> usize {
    let name = doc.source.name.rsplit(['/', '\\']).next().unwrap_or("");
    (name.chars().count().max(4) + 9).min(MAIN_NAME_MAX)
}

/// The most characters a part file adds around its unit (F11): the title line, a description
/// paragraph (Q001), and `## Relations` with `part_of` and `follows`. Titles are escaped, so
/// they count twice; the main document's name is percent-encoded, so it counts three times.
fn part_overhead(doc_title: usize, main_name: usize) -> usize {
    let part_title = doc_title + 2 + TITLE_CAP + 12;
    let title_line = 4 + 2 * part_title;
    let description = 50 + 2 * doc_title + 2 * TITLE_CAP;
    let part_of = 32 + 2 * doc_title + 3 * main_name;
    let follows = 16 + 2 * part_title + NAME_CAP + 8;
    title_line + description + part_of + follows + 32
}

/// The characters a unit may take for `part_size` tokens: the limit less the part overhead.
/// The main document writer cuts its `not-read` and `contents` parts with it (F12), through
/// [`paging::cut`] and [`part_measure`].
pub(crate) fn part_budget(doc: &Document, opts: &RenderOptions, part_size: usize) -> usize {
    let limit = part_size.saturating_mul(4).saturating_add(3);
    let doc_title = frontmatter::shown_title(doc, opts)
        .trim()
        .chars()
        .count()
        .min(TITLE_CAP);
    let overhead = part_overhead(doc_title, main_note_len(doc) + 3);
    limit.saturating_sub(overhead).max(limit / 2)
}

/// The measure of a unit written as a part into the folder `bundle`: the import path of the
/// longest part name, and room for the links that change one folder down.
pub(crate) fn part_measure(bundle: &str) -> Measure {
    Measure {
        note_path_len: bundle.chars().count() + 1 + NAME_CAP,
        moved: true,
    }
}

/// `text` cut into pieces of at most `budget` by `measure` ([`paging::cut`]), each without
/// blank lines at its ends.
pub(crate) fn pieces(text: &str, budget: usize, measure: Measure) -> Vec<String> {
    paging::cut(text, budget, &|t: &str| measure.chars(t))
        .into_iter()
        .map(|p| p.trim_matches(['\n', '\r']).to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// `text` cut to at most `max` characters: at the last space in its second half, else between
/// characters, then `...`. Unchanged when it fits.
pub(crate) fn capped(text: &str, max: usize) -> String {
    let text = text.trim();
    let Some((end, _)) = text.char_indices().nth(max.saturating_sub(3)) else {
        return text.to_string();
    };
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut = match text[..end].rfind(' ') {
        Some(space) if space > end / 2 => space,
        _ => end,
    };
    format!("{}...", text[..cut].trim_end())
}

struct Splitter<'a> {
    doc: &'a Document,
    opts: &'a RenderOptions,
    budget: usize,
    measure: Measure,
    heading_offset: u8,
    column_numbers: bool,
    selection: Option<Selection<'a>>,
    /// The footnote definitions on the top level that something cites, by label, with their
    /// place in the body: they go into every unit that cites them, not into a unit of their
    /// own.
    footnotes: HashMap<&'a str, (usize, &'a Block)>,
    /// A trial render's notes: undone after each measure.
    trial: Notes<'a>,
    next_family: usize,
    /// Blocks shown in another unit than their own (a column profile with the first rows of
    /// its table): left out where they stand.
    moved: HashSet<*const Block>,
}

impl<'a> Splitter<'a> {
    fn new(
        doc: &'a Document,
        opts: &'a RenderOptions,
        part_size: usize,
        bundle: &str,
    ) -> Splitter<'a> {
        let title = frontmatter::shown_title(doc, opts).trim();
        let body = content_body(doc, title);
        let min = crate::min_heading(body, 0).unwrap_or(2).max(1);
        let selection = chosen_sheets(opts).map(|titles| Selection::of(doc, titles));
        // Every footnote definition, on the top level or in the Markdown reader's
        // `Footnotes` section, in body order.
        let mut defs: HashMap<&'a str, (usize, &'a Block)> = HashMap::new();
        for b in &doc.body {
            let inner = match footnotes_section(b) {
                Some(list) => list,
                None => std::slice::from_ref(b),
            };
            for d in inner {
                if let Some(label) = footnote_label(d) {
                    let n = defs.len();
                    defs.entry(label).or_insert((n, d));
                }
            }
        }
        // The ones a unit cites, and those they cite in turn. The definitions themselves and
        // what no unit writes (the title, unchosen sheets, the PDF outline) do not count: a
        // definition nothing in a unit cites stays where it is, so it is never lost.
        let skip_first = doc.body.len() - body.len();
        let mut cited: HashSet<String> = HashSet::new();
        for (i, b) in doc.body.iter().enumerate() {
            if i < skip_first
                || footnote_label(b).is_some()
                || footnotes_section(b).is_some()
                || is_pdf_outline(doc, b)
                || selection.as_ref().is_some_and(|s| !s.keeps_block(b))
            {
                continue;
            }
            cited_footnotes(std::slice::from_ref(b), 0, &mut cited);
        }
        let mut queue: Vec<String> = cited.iter().cloned().collect();
        while let Some(label) = queue.pop() {
            if let Some((_, def)) = defs.get(label.as_str()) {
                let mut inner = HashSet::new();
                cited_footnotes(std::slice::from_ref(*def), 0, &mut inner);
                for l in inner {
                    if cited.insert(l.clone()) {
                        queue.push(l);
                    }
                }
            }
        }
        defs.retain(|label, _| cited.contains(*label));
        let footnotes = defs;
        Splitter {
            doc,
            opts,
            budget: part_budget(doc, opts, part_size),
            measure: part_measure(bundle),
            heading_offset: 2u8.saturating_sub(min),
            column_numbers: crate::column_numbers(doc),
            selection,
            footnotes,
            trial: Notes::new(&doc.facets.annotations),
            next_family: 0,
            moved: HashSet::new(),
        }
    }

    fn want(&self, s: OutputSection) -> bool {
        self.opts.sections.as_ref().is_none_or(|l| l.contains(&s))
    }

    fn ctx<'n>(&self, notes: &'n Notes<'a>) -> Ctx<'n>
    where
        'a: 'n,
    {
        Ctx {
            assets: &self.doc.assets,
            link_base: "../",
            heading_offset: self.heading_offset,
            notes: Some(notes),
            codes_shown: Default::default(),
            in_sheet: Default::default(),
            table_style: self.opts.table_style,
            column_numbers: self.column_numbers,
        }
    }

    fn family(&mut self, parent: &Item<'a>) -> Option<usize> {
        if parent.family.is_some() {
            return parent.family;
        }
        self.next_family += 1;
        Some(self.next_family)
    }

    fn run(mut self) -> Vec<Unit> {
        let mut specs: Vec<Spec<'a>> = Vec::new();
        if self.want(OutputSection::Content) {
            let items = self.top_items();
            self.plan(items, &mut specs);
        }
        let specs = self.pack(specs);
        self.write(specs)
    }

    // -----------------------------------------------------------------------------------
    // Rendering and measuring

    /// The Markdown of `content` inside `wraps`, with the footnote definitions it cites.
    /// With `links` (once every unit has its file), links are written for the unit's place.
    fn render(
        &self,
        wraps: &[Wrap<'a>],
        content: &Content<'a>,
        last: bool,
        notes: &Notes<'a>,
        links: Option<&Links<'_>>,
    ) -> String {
        let ctx = self.ctx(notes);
        let mut list: Cow<'_, [Block]> = Cow::Borrowed(content.blocks());
        if !self.moved.is_empty()
            && list
                .iter()
                .any(|b| self.moved.contains(&std::ptr::from_ref(b)))
        {
            list = Cow::Owned(
                list.iter()
                    .filter(|b| !self.moved.contains(&std::ptr::from_ref(*b)))
                    .cloned()
                    .collect(),
            );
        }
        if let Some(links) = links
            && list.iter().any(|b| block_needs_rewrite(b, links, 0))
        {
            let mut owned = list.into_owned();
            for b in &mut owned {
                rewrite_block(b, links, 0);
            }
            list = Cow::Owned(owned);
        }
        let sheet = wraps.last().is_some_and(|w| w.kind == SectionKind::Sheet);
        let body = |d: usize, l: u8| blocks::section_body(&list, sheet, &ctx, d, l);
        let mut text = wrapped(wraps, last, &ctx, 0, 2, &body);
        let defs = self.cited_definitions(&list);
        if !defs.is_empty() {
            let defs: Vec<Block> = defs.into_iter().cloned().collect();
            let defs: Cow<'_, [Block]> = match links {
                Some(links) if defs.iter().any(|b| block_needs_rewrite(b, links, 0)) => {
                    let mut owned = defs;
                    for b in &mut owned {
                        rewrite_block(b, links, 0);
                    }
                    Cow::Owned(owned)
                }
                _ => Cow::Owned(defs),
            };
            let notes_text = blocks::blocks(&defs, &ctx, 0, 2, false);
            if !notes_text.is_empty() {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&notes_text);
            }
        }
        text
    }

    /// The size of an item by [`Measure`], from a trial render whose notes are undone.
    fn cost(&self, item: &Item<'a>) -> usize {
        let mark = self.trial.checkpoint();
        let text = self.render(&item.wraps, &item.content, item.last, &self.trial, None);
        self.trial.rollback(mark);
        self.measure.chars(&text)
    }

    /// The top-level footnote definitions `list` cites and does not hold, with the ones they
    /// cite in turn, in document order.
    fn cited_definitions(&self, list: &[Block]) -> Vec<&'a Block> {
        if self.footnotes.is_empty() {
            return Vec::new();
        }
        let mut held: HashSet<&str> = HashSet::new();
        for b in list {
            if let Some(label) = footnote_label(b) {
                held.insert(label);
            }
        }
        let mut cited: HashSet<String> = HashSet::new();
        cited_footnotes(list, 0, &mut cited);
        let mut queue: Vec<String> = cited.iter().cloned().collect();
        while let Some(label) = queue.pop() {
            if let Some((_, def)) = self.footnotes.get(label.as_str()) {
                let mut inner = HashSet::new();
                cited_footnotes(std::slice::from_ref(*def), 0, &mut inner);
                for l in inner {
                    if cited.insert(l.clone()) {
                        queue.push(l);
                    }
                }
            }
        }
        // In body order, found through the labels: no walk over the whole body per unit.
        let mut defs: Vec<(usize, &'a Block)> = cited
            .iter()
            .filter(|l| !held.contains(l.as_str()))
            .filter_map(|l| self.footnotes.get(l.as_str()).copied())
            .collect();
        defs.sort_by_key(|(i, _)| *i);
        defs.into_iter().map(|(_, b)| b).collect()
    }

    // -----------------------------------------------------------------------------------
    // Borders
}

#[cfg(test)]
mod tests;
