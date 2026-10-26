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
//! ranges (`split_rows`, its `max_tokens` halved up to 8 times while a range is over). One
//! block still over the limit is cut as text by [`crate::paging::cut`]. Units of one level
//! next to each other are packed while they fit.
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

use rezzer_model::{Block, Completeness, Document, FormatId, Inline, SectionKind, split_rows};

use crate::blocks::{self, Ctx, MAX_DEPTH, Notes};
use crate::describe::{Measure, body_tokens};
use crate::{OutputSection, RenderOptions, Selection, chosen_sheets, facets, frontmatter, paging};

/// One unit of a split document: what a part file holds besides its frontmatter, title and
/// relations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// The part's file name without `.md` (decision 9): lowercase `[a-z0-9-]`, at most 60
    /// bytes, unique in the document, never `index`, `log`, `overview`, `not-read` or
    /// `contents`.
    pub file: String,
    /// The unit's own title (at most [`TITLE_CAP`] characters and `, part <n>`), else its
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
/// writer `overview`, `not-read` and `contents`.
const RESERVED: [&str; 5] = ["index", "log", "overview", "not-read", "contents"];

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
/// stays under the 255 bytes of common file systems and [`part_budget`] never falls to its
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

/// A name from a title: lowercase ASCII letters and digits, any other run as one `-`, no
/// `-` at either end. Letters outside ASCII are left out (`Über` gives `ber`).
pub(crate) fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii() {
            gap = true;
        }
    }
    out
}

/// `slug` cut to at most `max` bytes, at its last `-` that keeps it within, else at `max`.
fn cut_slug(slug: &str, max: usize) -> &str {
    if slug.len() <= max {
        return slug;
    }
    let head = &slug[..max];
    match head.rfind('-') {
        Some(at) if at > 0 => &head[..at],
        _ => head.trim_end_matches('-'),
    }
}

/// The part names given so far (decision 9): each unique, none reserved, in document order.
#[derive(Debug)]
pub(crate) struct Names {
    taken: HashSet<String>,
    /// The next number to try for a base and suffix, so many pieces of one base do not try
    /// every number again.
    next: HashMap<(String, String), usize>,
}

impl Names {
    pub(crate) fn new() -> Names {
        Names {
            taken: RESERVED.iter().map(|r| (*r).to_string()).collect(),
            next: HashMap::new(),
        }
    }

    /// The names a split gave its units, so a later name never takes one of them.
    pub(crate) fn taken<'n>(names: impl IntoIterator<Item = &'n str>) -> Names {
        let mut out = Names::new();
        out.taken.extend(names.into_iter().map(str::to_string));
        out
    }

    /// A free name from `base` (slugged) and `suffix` (already a slug part, such as
    /// `-rows-2-300`): `-2`, `-3` and so on when it is taken; `part-<order>` when both are
    /// empty. At most 60 bytes: the base is cut at a hyphen to make room, and a name still
    /// longer is cut at a hyphen and ends with `-<8 hex>` of the whole name, so it stays
    /// unique.
    pub(crate) fn take(&mut self, base: &str, suffix: &str, order: usize) -> String {
        let base = slug(base);
        let (base, suffix) = match (base.is_empty(), suffix.trim_start_matches('-')) {
            (true, "") => (format!("part-{order}"), String::new()),
            (true, rest) => (rest.to_string(), String::new()),
            (false, _) => (base, suffix.to_string()),
        };
        let key = (base.clone(), suffix.clone());
        let start = self.next.get(&key).copied().unwrap_or(1);
        for n in start.. {
            let tail = if n == 1 {
                suffix.clone()
            } else {
                format!("{suffix}-{n}")
            };
            let room = NAME_CAP.saturating_sub(tail.len());
            let name = format!("{}{tail}", cut_slug(&base, room));
            let mut name = name.trim_matches('-').to_string();
            if name.len() > NAME_CAP {
                name = hashed(&name);
            }
            if !name.is_empty() && self.taken.insert(name.clone()) {
                self.next.insert(key, n + 1);
                return name;
            }
        }
        unreachable!("some suffix is always free")
    }
}

/// `name` cut at a hyphen to make room for `-<8 hex>`, the FNV-1a hash of the whole name.
fn hashed(name: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let head = cut_slug(name, NAME_CAP - 9).trim_end_matches('-');
    format!("{head}-{hash:08x}")
}

/// A section around a unit's blocks: the unit keeps its heading and context.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Wrap<'a> {
    kind: SectionKind,
    title: Option<&'a str>,
    anchor: Option<&'a str>,
}

/// What a unit holds: a run of blocks of one list, or blocks made by `split_rows`.
#[derive(Debug, Clone)]
enum Content<'a> {
    Slice {
        list: &'a [Block],
        from: usize,
        to: usize,
    },
    Owned(Vec<Block>),
}

impl<'a> Content<'a> {
    fn blocks(&self) -> &[Block] {
        match self {
            Content::Slice { list, from, to } => &list[*from..*to],
            Content::Owned(blocks) => blocks,
        }
    }
}

/// How a unit over the limit is cut further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expand {
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
struct Item<'a> {
    base: String,
    title: String,
    location: String,
    /// The units cut from one titled unit share it; the second on get `, part <n>`.
    family: Option<usize>,
    /// The pages it holds (first and last).
    pages: Option<(u32, u32)>,
    /// Packed, it is named `pages-<a>-<b>` (pages without an outline).
    page_names: bool,
    wraps: Vec<Wrap<'a>>,
    content: Content<'a>,
    expand: Expand,
    /// It holds the last block of every section around it: a page's own notes close it.
    last: bool,
    /// How many blocks at its start are headings of a part around it (a chapter heading
    /// that went with its first section): they stay with its first piece.
    lead: usize,
    /// Only a heading: it goes with the item after it when it cannot be packed with it.
    heading_only: bool,
    /// Rezzer made it and titled it itself (`Start`, `Footnotes`): its title never changes
    /// for a clash.
    rezzer: bool,
}

/// A unit as planned: the item, and how many pieces its text is cut into (more than one only
/// for a block cut as text).
#[derive(Debug)]
struct Spec<'a> {
    item: Item<'a>,
    /// A name part after the base, for row ranges.
    suffix: String,
    pieces: usize,
    /// The blocks whose anchors lead to this unit.
    anchors: Option<&'a [Block]>,
}

/// The names of a family's members: what the `n`-th of them (from 1) is titled.
pub(crate) fn family_title(title: &str, n: usize) -> String {
    if n <= 1 {
        title.to_string()
    } else {
        format!("{title}, part {n}")
    }
}

/// The title of Rezzer's own parts that list the other parts (ruling P5-AF), so a source
/// chapter named `Contents` keeps its name.
pub(crate) const LIST_TITLE: &str = "List of parts";
/// The whole overview when the main document cuts it.
pub(crate) const OVERVIEW_TITLE: &str = "Overview";
/// The whole `## Not read` list when the main document cuts it.
pub(crate) const NOT_READ_TITLE: &str = "Not read";
/// The blocks before the first border.
pub(crate) const START_TITLE: &str = "Start";
/// The footnote definitions no unit cites.
pub(crate) const FOOTNOTES_TITLE: &str = "Footnotes";
pub(crate) const PROPERTIES_TITLE: &str = "Properties";
pub(crate) const LOGIC_TITLE: &str = "Logic";
pub(crate) const ANNOTATIONS_TITLE: &str = "Annotations";
pub(crate) const CHANGES_TITLE: &str = "Changes";
pub(crate) const EMBEDDED_TITLE: &str = "Embedded";
pub(crate) const OUTLINE_TITLE: &str = "Outline";

/// Every title Rezzer gives its own parts, the constants the writers use.
#[cfg(test)]
pub(crate) const OWN_TITLES: [&str; 11] = [
    LIST_TITLE,
    OVERVIEW_TITLE,
    NOT_READ_TITLE,
    START_TITLE,
    FOOTNOTES_TITLE,
    PROPERTIES_TITLE,
    LOGIC_TITLE,
    ANNOTATIONS_TITLE,
    CHANGES_TITLE,
    EMBEDDED_TITLE,
    OUTLINE_TITLE,
];

/// How a unit's title is made, so the part writer can make it again once it knows which of
/// its own parts the document gets ([`finish_titles`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Naming {
    /// Rezzer made and titled it (`Start`, `Footnotes`, the trailing sections): its title
    /// never changes for a clash.
    own: bool,
    /// The unit's title before its piece number, unique among the units of the source.
    base: String,
    /// Where the unit (its first piece) lies.
    location: String,
    /// Its piece number from 2, else 0 (a unit in one piece, or its first piece).
    piece: usize,
}

/// A title as Crystalline compares it (L003): trimmed, in lower case.
fn title_key(title: &str) -> String {
    title.trim().to_lowercase()
}

/// `title` with ` <suffix>` added, within [`TITLE_CAP`]: the title is cut to make room.
fn with_suffix(title: &str, suffix: &str) -> String {
    let room = TITLE_CAP.saturating_sub(suffix.chars().count() + 1);
    format!("{} {suffix}", capped(title, room))
}

/// The titles of the units of one document (title, location, own), made unique before their
/// pieces are numbered (rulings P5-AE and P5-AF), as Crystalline compares titles (L003).
/// Units of the source that share a title get their location in words,
/// `Abstract (pages 3 to 5)`; units that share title and location get ` (2)`, ` (3)` after
/// the first. Rezzer's own units keep theirs; a clash with them is [`finish_titles`]' job.
/// Each title stays within [`TITLE_CAP`], so the pieces' `, part <n>` fits the overhead.
pub(crate) fn unit_titles(units: &[(&str, &str, bool)]) -> Vec<String> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for (title, _, own) in units {
        if !own {
            *count.entry(title_key(title)).or_insert(0) += 1;
        }
    }
    let mut taken: HashSet<String> = HashSet::new();
    units
        .iter()
        .map(|&(title, location, own)| {
            if own {
                return capped(title, TITLE_CAP);
            }
            let key = title_key(title);
            let location = location.trim();
            let located = if count.get(&key).is_some_and(|&n| n > 1)
                && !location.is_empty()
                && title_key(location) != key
            {
                with_suffix(title, &format!("({})", capped(location, 60)))
            } else {
                capped(title, TITLE_CAP)
            };
            let mut unique = located.clone();
            let mut n = 1;
            while !taken.insert(title_key(&unique)) {
                n += 1;
                unique = with_suffix(&located, &format!("({n})"));
            }
            unique
        })
        .collect()
}

/// Whether `key` is `own` or a piece of it (`logic, part 2`, `logic, sheet1`).
fn is_title_or_piece(key: &str, own: &str) -> bool {
    key == own
        || key
            .strip_prefix(own)
            .is_some_and(|rest| rest.starts_with(", "))
}

/// Writes the final title of every unit from its [`Naming`] (ruling P5-AF): a unit of the
/// source titled like a part Rezzer really writes for this document (one of its own units,
/// or one of `extra`, the parts the main document writer adds: `Overview`, `Not read`,
/// `List of parts`) gets its location in words, else the word `(section)`; then its piece
/// number. A title still used twice gets ` (2)`, ` (3)`, own units first.
pub(crate) fn finish_titles(units: &mut [Unit], extra: &[&str]) {
    let present: Vec<String> = units
        .iter()
        .filter(|u| u.naming.own)
        .map(|u| title_key(&u.naming.base))
        .chain(extra.iter().map(|t| title_key(t)))
        .collect();
    for unit in units.iter_mut() {
        let n = &unit.naming;
        if n.base.is_empty() {
            continue;
        }
        let mut base = n.base.clone();
        let key = title_key(&base);
        if !n.own
            && present
                .iter()
                .any(|own| is_title_or_piece(&key, own) || is_title_or_piece(own, &key))
        {
            let location = n.location.trim();
            base = if !location.is_empty() && title_key(location) != key {
                with_suffix(&base, &format!("({})", capped(location, 60)))
            } else {
                with_suffix(&base, "(section)")
            };
        }
        unit.title = family_title(&base, n.piece);
    }
    let mut taken: HashSet<String> = HashSet::new();
    for own in [true, false] {
        for unit in units.iter_mut().filter(|u| u.naming.own == own) {
            let base = unit.title.clone();
            let mut n = 1;
            while !taken.insert(title_key(&unit.title)) {
                n += 1;
                unit.title = with_suffix(&capped(&base, TITLE_CAP), &format!("({n})"));
            }
        }
    }
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

    /// The units on the top level of the content.
    fn top_items(&mut self) -> Vec<Item<'a>> {
        let doc = self.doc;
        let body = &doc.body;
        let title = frontmatter::shown_title(doc, self.opts).trim();
        let skip_first = body.len() - content_body(doc, title).len();
        // The blocks that are no unit's: the title, unchosen sheets, the PDF outline (a
        // trailing unit) and the cited footnote definitions.
        // A `Footnotes` section keeps only the definitions no unit cites; with none left it
        // is no unit's either.
        let left_out = |i: usize, b: &Block| {
            i < skip_first
                || self.selection.as_ref().is_some_and(|s| !s.keeps_block(b))
                || is_pdf_outline(doc, b)
                || footnote_label(b).is_some_and(|l| self.footnotes.contains_key(l))
                || footnotes_section(b).is_some()
        };
        let mut runs: Vec<Run> = Vec::new();
        let mut from = None;
        for (i, b) in body.iter().enumerate() {
            match (left_out(i, b), from) {
                (true, Some(f)) => {
                    runs.push(Run::Blocks(f, i));
                    from = None;
                }
                (false, None) => from = Some(i),
                _ => {}
            }
            if let Some(list) = footnotes_section(b)
                && i >= skip_first
            {
                let rest: Vec<Block> = list
                    .iter()
                    .filter(|d| footnote_label(d).is_none_or(|l| !self.footnotes.contains_key(l)))
                    .cloned()
                    .collect();
                if !rest.is_empty() {
                    let Block::Section {
                        kind,
                        title,
                        anchor,
                        ..
                    } = b
                    else {
                        continue;
                    };
                    runs.push(Run::Owned(Block::Section {
                        kind: *kind,
                        title: title.clone(),
                        anchor: anchor.clone(),
                        blocks: rest,
                    }));
                }
            }
        }
        if let Some(f) = from {
            runs.push(Run::Blocks(f, body.len()));
        }
        let start = Item {
            base: "start".into(),
            title: START_TITLE.into(),
            location: START_TITLE.into(),
            family: None,
            pages: None,
            page_names: false,
            wraps: Vec::new(),
            content: Content::Slice {
                list: body,
                from: 0,
                to: 0,
            },
            expand: Expand::Chunk,
            lead: 0,
            heading_only: false,
            rezzer: true,
            last: true,
        };
        let pages = body.iter().any(|b| page_number(b).is_some());
        let chapters = if pages { chapters(doc) } else { Vec::new() };
        let mut items = Vec::new();
        for run in runs {
            let (from, to) = match run {
                Run::Blocks(from, to) => (from, to),
                Run::Owned(section) => {
                    items.push(Item {
                        base: "footnotes".into(),
                        title: FOOTNOTES_TITLE.into(),
                        location: FOOTNOTES_TITLE.into(),
                        content: Content::Owned(vec![section]),
                        expand: Expand::Text,
                        ..start.clone()
                    });
                    continue;
                }
            };
            if !chapters.is_empty() {
                items.extend(self.chapter_items(body, from, to, &chapters, &start));
            } else if pages {
                items.extend(self.page_items(body, from, to, &start));
            } else {
                items.extend(self.items_of(body, from, to, &[], &start, true));
            }
        }
        items
    }

    /// Pages grouped into outline chapters: each run of blocks from a chapter's first page to
    /// the next chapter's.
    fn chapter_items(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        chapters: &[(u32, &'a str)],
        start: &Item<'a>,
    ) -> Vec<Item<'a>> {
        // The chapter of each block: the last one whose first page is at or before the
        // block's page; blocks between pages go with the page before.
        let mut out: Vec<Item<'a>> = Vec::new();
        let mut current: Option<usize> = None;
        let mut group_from = from;
        let mut pages: Option<(u32, u32)> = None;
        let chapter_of = |n: u32| chapters.iter().rposition(|(p, _)| *p <= n);
        let close = |out: &mut Vec<Item<'a>>,
                     chapter: Option<usize>,
                     a: usize,
                     b: usize,
                     pages: Option<(u32, u32)>| {
            if a >= b {
                return;
            }
            let (base, title) = match chapter {
                Some(k) => (slug(chapters[k].1), capped(chapters[k].1, TITLE_CAP)),
                None => (start.base.clone(), start.title.clone()),
            };
            let location = pages.map_or_else(|| title.clone(), pages_location);
            out.push(Item {
                base,
                title,
                location,
                family: None,
                pages,
                page_names: false,
                wraps: Vec::new(),
                content: Content::Slice {
                    list,
                    from: a,
                    to: b,
                },
                expand: Expand::Group,
                lead: 0,
                heading_only: false,
                rezzer: chapter.is_none(),
                last: true,
            });
        };
        for (i, b) in list.iter().enumerate().take(to).skip(from) {
            if let Some(n) = page_number(b) {
                let chapter = chapter_of(n);
                if i > group_from && chapter != current {
                    close(&mut out, current, group_from, i, pages);
                    group_from = i;
                    pages = None;
                }
                current = chapter;
                pages = Some(pages.map_or((n, n), |(a, _)| (a, n)));
            }
        }
        close(&mut out, current, group_from, to, pages);
        out
    }

    /// Pages without an outline: each page a unit `page-<n>`, packed into `pages-<a>-<b>`.
    fn page_items(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        start: &Item<'a>,
    ) -> Vec<Item<'a>> {
        let mut out = Vec::new();
        let mut i = from;
        while i < to {
            if let Some(n) = page_number(&list[i]) {
                out.push(self.page_item(list, i, n, None, &[], true));
                i += 1;
                continue;
            }
            let mut j = i;
            while j < to && page_number(&list[j]).is_none() {
                j += 1;
            }
            out.extend(self.items_of(list, i, j, &[], start, true));
            i = j;
        }
        out
    }

    /// A page section as a unit: `page-<n>` on the top level, else a piece of `parent`.
    fn page_item(
        &mut self,
        list: &'a [Block],
        i: usize,
        n: u32,
        parent: Option<&Item<'a>>,
        wraps: &[Wrap<'a>],
        last: bool,
    ) -> Item<'a> {
        let location = pages_location((n, n));
        // Only pages on the top level, without an outline, are named by their numbers.
        let page_names = parent.is_none();
        let (base, title, family) = match parent {
            Some(p) => {
                let family = self.family(p);
                (p.base.clone(), p.title.clone(), family)
            }
            None => (format!("page-{n}"), location.clone(), None),
        };
        Item {
            base,
            title,
            location,
            family,
            pages: Some((n, n)),
            page_names,
            wraps: wraps.to_vec(),
            content: Content::Slice {
                list,
                from: i,
                to: i + 1,
            },
            expand: Expand::Section,
            lead: 0,
            heading_only: false,
            rezzer: parent.is_some_and(|p| p.rezzer),
            last,
        }
    }

    /// The units of `list[from..to]` inside `wraps`: each section on its own, the runs of
    /// other blocks at their headings. Untitled units are pieces of `parent`.
    fn items_of(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        last: bool,
    ) -> Vec<Item<'a>> {
        let mut out = Vec::new();
        let mut i = from;
        while i < to {
            if is_section(&list[i]) {
                let at_end = last && i + 1 == to;
                out.push(self.section_item(list, i, wraps, parent, at_end));
                i += 1;
                continue;
            }
            let mut j = i;
            while j < to && !is_section(&list[j]) {
                j += 1;
            }
            out.extend(self.chunks(list, i, j, wraps, parent, last && j == to));
            i = j;
        }
        out
    }

    fn section_item(
        &mut self,
        list: &'a [Block],
        i: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        last: bool,
    ) -> Item<'a> {
        if let Some(n) = page_number(&list[i]) {
            return self.page_item(list, i, n, Some(parent), wraps, last);
        }
        let title = match &list[i] {
            Block::Section { title, .. } => title.as_deref().filter(|t| !t.trim().is_empty()),
            _ => None,
        };
        self.titled_item(title, list, i, i + 1, wraps, parent, Expand::Section, last)
    }

    /// An item titled `title`, else a piece of `parent`.
    #[allow(clippy::too_many_arguments)]
    fn titled_item(
        &mut self,
        title: Option<&str>,
        list: &'a [Block],
        from: usize,
        to: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        expand: Expand,
        last: bool,
    ) -> Item<'a> {
        // A piece of a unit Rezzer titled itself keeps that title.
        let rezzer = title.is_none() && parent.rezzer;
        let (base, title, location, family) = match title {
            Some(t) => {
                let t = capped(t, TITLE_CAP);
                (slug(&t), t.clone(), t, None)
            }
            None => {
                let family = self.family(parent);
                (
                    parent.base.clone(),
                    parent.title.clone(),
                    parent.location.clone(),
                    family,
                )
            }
        };
        Item {
            base,
            title,
            location,
            family,
            // A piece holds pages of its parent; a titled part says where it is by its title.
            pages: parent.pages.filter(|_| family.is_some()),
            page_names: false,
            wraps: wraps.to_vec(),
            content: Content::Slice { list, from, to },
            expand,
            lead: 0,
            heading_only: false,
            rezzer,
            last,
        }
    }

    /// `list[from..to]` (no sections) at its headings of the smallest level; the blocks
    /// before the first such heading are a piece of `parent`. Without headings, blocks.
    fn chunks(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        last: bool,
    ) -> Vec<Item<'a>> {
        let Some(min) = list[from..to].iter().filter_map(heading_level).min() else {
            return self.atoms(list, from, to, wraps, parent, last);
        };
        let mut borders: Vec<usize> = (from..to)
            .filter(|&i| heading_level(&list[i]) == Some(min))
            .collect();
        let mut out = Vec::new();
        if borders.first() != Some(&from) {
            borders.insert(0, from);
        }
        for (k, &a) in borders.iter().enumerate() {
            let b = borders.get(k + 1).copied().unwrap_or(to);
            let title = match &list[a] {
                Block::Heading { inlines, .. } if heading_level(&list[a]) == Some(min) => {
                    Some(rezzer_model::plain_text(inlines))
                }
                _ => None,
            };
            let title = title.filter(|t| !t.trim().is_empty());
            out.push(self.titled_item(
                title.as_deref(),
                list,
                a,
                b,
                wraps,
                parent,
                Expand::Chunk,
                last && b == to,
            ));
        }
        out
    }

    /// `list[from..to]` as blocks: a heading or caption with the block after it, a table with
    /// the notes after it. Each is a piece of `parent`.
    fn atoms(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        last: bool,
    ) -> Vec<Item<'a>> {
        let mut out = Vec::new();
        let mut i = from;
        while i < to {
            let mut j = i;
            while j + 1 < to && leads(&list[j]) {
                j += 1;
            }
            j += 1;
            if matches!(list[j - 1], Block::Table { .. }) {
                while j < to && is_note(&list[j]) {
                    j += 1;
                }
            }
            out.push(self.titled_item(
                None,
                list,
                i,
                j,
                wraps,
                parent,
                Expand::Atom,
                last && j == to,
            ));
            i = j;
        }
        out
    }

    // -----------------------------------------------------------------------------------
    // Planning

    /// Packs `items` (one level, in order) into specs, cutting those over the limit further.
    fn plan(&mut self, items: Vec<Item<'a>>, out: &mut Vec<Spec<'a>>) {
        let mut costs: Vec<usize> = items.iter().map(|item| self.cost(item)).collect();
        let mut items: Vec<Option<Item<'a>>> = items.into_iter().map(Some).collect();
        let mut i = 0;
        while i < items.len() {
            let Some(item) = items[i].take() else {
                i += 1;
                continue;
            };
            if costs[i] > self.budget {
                self.expand(item, out);
                i += 1;
                continue;
            }
            // A heading alone that cannot be packed with the item after it goes into that
            // item, so no part holds only a heading.
            if item.heading_only
                && let Some(Some(next)) = items.get_mut(i + 1)
                && next.expand == Expand::Chunk
                && packs(&item, next)
                && costs[i] + 2 + costs[i + 1] > self.budget
                && let (
                    Content::Slice { from, to, .. },
                    Content::Slice {
                        from: next_from, ..
                    },
                ) = (&item.content, &mut next.content)
            {
                next.lead += to - from;
                *next_from = *from;
                costs[i + 1] = self.cost(next);
                i += 1;
                continue;
            }
            let mut j = i + 1;
            let mut sum = costs[i];
            let mut prev = &item;
            while j < items.len() {
                let Some(next) = items[j].as_ref() else {
                    break;
                };
                if costs[j] > self.budget || !packs(prev, next) || sum + 2 + costs[j] > self.budget
                {
                    break;
                }
                sum += 2 + costs[j];
                prev = next;
                j += 1;
            }
            if j == i + 1 {
                out.push(spec(item));
                i += 1;
                continue;
            }
            let group: Vec<Item<'a>> = std::iter::once(item)
                .chain(items[i + 1..j].iter_mut().filter_map(Option::take))
                .collect();
            let packed = merge(&group);
            if self.cost(&packed) <= self.budget {
                out.push(spec(packed));
            } else {
                out.extend(group.into_iter().map(spec));
            }
            i = j;
        }
    }

    fn expand(&mut self, item: Item<'a>, out: &mut Vec<Spec<'a>>) {
        let Content::Slice { list, from, to } = item.content else {
            out.push(cut_spec(item, self));
            return;
        };
        let mut children = match item.expand {
            Expand::Group => {
                let mut children = Vec::new();
                for i in from..to {
                    let at_end = item.last && i + 1 == to;
                    match page_number(&list[i]) {
                        Some(n) => children.push(self.page_item(
                            list,
                            i,
                            n,
                            Some(&item),
                            &item.wraps,
                            at_end,
                        )),
                        None => children.extend(self.items_of(
                            list,
                            i,
                            i + 1,
                            &item.wraps,
                            &item,
                            at_end,
                        )),
                    }
                }
                children
            }
            Expand::Section => match &list[from] {
                Block::Section {
                    kind,
                    title,
                    anchor,
                    blocks: inner,
                } if item.wraps.len() < MAX_DEPTH => {
                    let mut wraps = item.wraps.clone();
                    wraps.push(Wrap {
                        kind: *kind,
                        title: title.as_deref(),
                        anchor: anchor.as_ref().map(|a| a.0.as_str()),
                    });
                    let mut parent = item.clone();
                    parent.family = self.family(&item);
                    self.items_of(inner, 0, inner.len(), &wraps, &parent, item.last)
                }
                _ => Vec::new(),
            },
            Expand::Chunk => self.chunk_children(&item, list, from, to),
            Expand::Atom => {
                self.rows(item, list, from, to, out);
                return;
            }
            Expand::Text => Vec::new(),
        };
        // A section that holds only a section: go down to it without measuring again.
        if item.expand == Expand::Section
            && children.len() == 1
            && children[0].expand == Expand::Section
            && let Some(child) = children.pop()
        {
            self.expand(child, out);
            return;
        }
        // A border that gives nothing finer is cut as text.
        if children.is_empty() || (children.len() == 1 && children[0].expand == item.expand) {
            out.push(cut_spec(item, self));
            return;
        }
        self.plan(children, out);
    }

    /// The parts of a heading chunk: its blocks at the next headings, the heading with the
    /// first; without more headings, its blocks.
    fn chunk_children(
        &mut self,
        item: &Item<'a>,
        list: &'a [Block],
        from: usize,
        to: usize,
    ) -> Vec<Item<'a>> {
        let mut parent = item.clone();
        parent.family = self.family(item);
        parent.lead = 0;
        // The headings it took from a part around it stay in front of its own.
        let own = (from + item.lead).min(to);
        let head = own < to && heading_level(&list[own]).is_some();
        let rest = if head { own + 1 } else { own };
        let deeper = list[rest..to].iter().any(|b| heading_level(b).is_some());
        if !deeper {
            return self.atoms(list, from, to, &item.wraps, &parent, item.last);
        }
        let mut children = self.chunks(list, rest, to, &item.wraps, &parent, item.last);
        if rest > from {
            match children.first_mut() {
                // The blocks before the first heading are a piece already: the headings join
                // them.
                Some(first) if first.family == parent.family => {
                    if let Content::Slice { from: f, .. } = &mut first.content {
                        *f = from;
                    }
                }
                _ => {
                    let mut heading = self.titled_item(
                        None,
                        list,
                        from,
                        rest,
                        &item.wraps,
                        &parent,
                        Expand::Atom,
                        false,
                    );
                    heading.heading_only = true;
                    children.insert(0, heading);
                }
            }
        }
        children
    }

    /// One block over the limit: a table into row ranges (retried with half the size up to 8
    /// times while a range is over), anything else cut as text.
    fn rows(
        &mut self,
        item: Item<'a>,
        list: &'a [Block],
        from: usize,
        to: usize,
        out: &mut Vec<Spec<'a>>,
    ) {
        let Some(t) = (from..to).find(|&i| matches!(list[i], Block::Table { .. })) else {
            out.push(cut_spec(item, self));
            return;
        };
        let (lead, table, tail) = (&list[from..t], &list[t], &list[t + 1..to]);
        let mut max = (self.budget / 4).max(1);
        let costs_of = |ranges: &[rezzer_model::RowRange]| -> Vec<usize> {
            ranges
                .iter()
                .map(|r| {
                    let mut probe = item.clone();
                    probe.content = Content::Owned(r.blocks.clone());
                    self.cost(&probe)
                })
                .collect()
        };
        let fit = |costs: &[usize]| costs.iter().all(|c| *c <= self.budget);
        let mut ranges = split_rows(lead, table, tail, max);
        let mut costs = costs_of(&ranges);
        if fit(&costs) && ranges.len() > 1 {
            // The row estimate is careful: when every range fits, ranges of twice the size
            // are taken if they fit too, so the parts are fuller.
            let wider = split_rows(lead, table, tail, max.saturating_mul(2));
            let wider_costs = costs_of(&wider);
            if fit(&wider_costs) {
                (ranges, costs) = (wider, wider_costs);
            }
        }
        // Only a range over the limit is cut again, with half the size, up to 8 times; the
        // others keep their size. A range of one row over the limit is cut as text.
        let mut tries = 0;
        while !fit(&costs) && tries < ROW_RETRIES && max > 1 {
            tries += 1;
            max = (max / 2).max(1);
            let mut next: Vec<(rezzer_model::RowRange, usize)> = Vec::new();
            let mut changed = false;
            for (range, cost) in ranges.into_iter().zip(costs) {
                if cost <= self.budget || range.first >= range.last {
                    next.push((range, cost));
                    continue;
                }
                let sub = resplit(&range, max);
                if sub.len() <= 1 {
                    next.push((range, cost));
                    continue;
                }
                changed = true;
                let sub_costs = costs_of(&sub);
                next.extend(sub.into_iter().zip(sub_costs));
            }
            (ranges, costs) = next.into_iter().unzip();
            if !changed {
                break;
            }
        }
        if ranges.len() == 1 {
            out.push(cut_spec(item, self));
            return;
        }
        let key = table_key(list, table);
        let sheet = item
            .wraps
            .last()
            .filter(|w| w.kind == SectionKind::Sheet)
            .and_then(|w| w.title);
        let count = ranges.len();
        for (k, (range, cost)) in ranges.into_iter().zip(costs).enumerate() {
            let data = range.table.and_then(|i| match &range.blocks[i] {
                Block::Table {
                    layout: Some(layout),
                    ..
                } => layout.data.clone(),
                _ => None,
            });
            let location = match (sheet, data) {
                (Some(sheet), Some(data)) => format!("{sheet}!{data}"),
                _ => format!("{}, rows {} to {}", item.location, range.first, range.last),
            };
            let location = capped(&location, TITLE_CAP);
            let suffix = format!("{key}-rows-{}-{}", range.first, range.last);
            let row_item = Item {
                base: item.base.clone(),
                title: location.clone(),
                location,
                family: None,
                pages: item.pages,
                page_names: false,
                wraps: item.wraps.clone(),
                content: Content::Owned(range.blocks),
                expand: Expand::Text,
                lead: 0,
                heading_only: false,
                rezzer: false,
                last: item.last && k + 1 == count,
            };
            let mut s = if cost > self.budget {
                cut_spec(row_item, self)
            } else {
                spec(row_item)
            };
            s.suffix = suffix;
            if k == 0 {
                s.anchors = Some(&list[from..to]);
            }
            out.push(s);
        }
    }

    // -----------------------------------------------------------------------------------
    // Writing

    /// Names the units, renders them with their links and adds the trailing sections.
    fn write(&self, specs: Vec<Spec<'a>>) -> Vec<Unit> {
        let mut names = Names::new();
        // Every piece the plan expects is named up front, in document order (P5-T): the
        // anchors lead to a unit's first piece, and the pieces of one unit are numbered in
        // order. A piece more than planned gets the next free name when it is written; a
        // piece fewer leaves its name unused.
        let mut order = 0usize;
        let mut files: Vec<String> = Vec::with_capacity(specs.len());
        let mut further: Vec<Vec<String>> = Vec::with_capacity(specs.len());
        for s in &specs {
            files.push(names.take(&s.item.base, &s.suffix, order + 1));
            further.push(
                (1..s.pieces.max(1))
                    .map(|p| names.take(&s.item.base, &s.suffix, order + p + 1))
                    .collect(),
            );
            order += s.pieces.max(1);
        }
        // The unit of each anchor: the first unit that holds it. A section cut into units
        // leads to the first of them.
        let mut at: HashMap<&'a str, usize> = HashMap::new();
        for (k, s) in specs.iter().enumerate() {
            for anchor in s.item.wraps.iter().filter_map(|w| w.anchor) {
                at.entry(anchor).or_insert(k);
            }
            if let Some(list) = anchor_source(s) {
                collect_anchors(list, 0, &mut |anchor| {
                    at.entry(anchor).or_insert(k);
                });
            }
        }
        let frags = fragments(&specs, &at);

        let notes = Notes::new(&self.doc.facets.annotations);
        // One title per unit before its pieces are numbered: a family (a section cut into
        // several units) is one unit, named by its first member.
        let family_of = |k: usize| specs[k].item.family.unwrap_or(usize::MAX - k);
        let mut seen: HashSet<usize> = HashSet::new();
        let firsts: Vec<(usize, usize)> = (0..specs.len())
            .filter(|&k| seen.insert(family_of(k)))
            .map(|k| (family_of(k), k))
            .collect();
        let pairs: Vec<(&str, &str, bool)> = firsts
            .iter()
            .map(|&(_, k)| {
                (
                    specs[k].item.title.as_str(),
                    specs[k].item.location.as_str(),
                    specs[k].item.rezzer,
                )
            })
            .collect();
        let titles: HashMap<usize, String> = firsts
            .iter()
            .map(|&(f, _)| f)
            .zip(unit_titles(&pairs))
            .collect();
        let first_location: HashMap<usize, String> = firsts
            .iter()
            .map(|&(f, k)| (f, specs[k].item.location.clone()))
            .collect();
        let mut units: Vec<Unit> = Vec::new();
        let mut families: HashMap<usize, usize> = HashMap::new();
        for (k, s) in specs.iter().enumerate() {
            let links = Links {
                at: &at,
                files: &files,
                frags: &frags,
                current: k,
            };
            let text = self.render(
                &s.item.wraps,
                &s.item.content,
                s.item.last,
                &notes,
                Some(&links),
            );
            let pieces = self.pieces(&text);
            let lone = pieces.len() == 1 && s.item.family.is_none();
            let family = family_of(k);
            let unit_title = &titles[&family];
            for (p, piece) in pieces.into_iter().enumerate() {
                let file = match p {
                    0 => files[k].clone(),
                    _ => match further[k].get(p - 1) {
                        Some(name) => name.clone(),
                        None => names.take(&s.item.base, &s.suffix, units.len() + 1),
                    },
                };
                let n = families.entry(family).or_insert(0);
                *n += 1;
                let piece_number = if lone { 0 } else { *n };
                units.push(Unit {
                    file,
                    title: family_title(unit_title, piece_number),
                    location: s.item.location.clone(),
                    markdown: piece,
                    naming: Naming {
                        own: s.item.rezzer,
                        base: unit_title.clone(),
                        location: first_location[&family].clone(),
                        piece: piece_number,
                    },
                });
            }
        }
        // The trailing sections are in no unit of the content: every anchor link from them
        // leads to another file.
        let links = Links {
            at: &at,
            files: &files,
            frags: &frags,
            current: usize::MAX,
        };
        for (base, location, text) in self.trailing(&notes, &links) {
            self.push_trailing(&mut units, &mut names, base, location, &text);
        }
        finish_titles(&mut units, &[]);
        units
    }

    /// `text` cut to the budget, each piece without blank lines at its ends.
    fn pieces(&self, text: &str) -> Vec<String> {
        pieces(text, self.budget, self.measure)
    }

    /// The trailing sections in order, each with its name and location: properties, logic,
    /// annotations (those the content did not show), changes, embedded and the PDF outline.
    fn trailing(
        &self,
        notes: &Notes<'a>,
        links: &Links<'_>,
    ) -> Vec<(&'static str, &'static str, String)> {
        let selection = self.selection.as_ref();
        let mut out = Vec::new();
        if self.want(OutputSection::Properties)
            && let Some(text) = facets::properties(self.doc)
        {
            out.push(("properties", PROPERTIES_TITLE, text));
        }
        if self.want(OutputSection::Logic)
            && let Some(text) = facets::logic(self.doc, self.opts.math, selection)
        {
            out.push(("logic", LOGIC_TITLE, text));
        }
        if self.want(OutputSection::Annotations) {
            let mut ctx = self.ctx(notes);
            ctx.heading_offset = 0;
            ctx.notes = None;
            let list = &self.doc.facets.annotations;
            let list: Cow<'_, [rezzer_model::Annotation]> =
                if list.iter().any(|a| annotation_needs_rewrite(a, links, 0)) {
                    let mut owned = list.clone();
                    for a in &mut owned {
                        rewrite_annotation(a, links, 0);
                    }
                    Cow::Owned(owned)
                } else {
                    Cow::Borrowed(list)
                };
            if let Some(text) = facets::annotations(&list, &ctx, notes, selection) {
                out.push(("annotations", ANNOTATIONS_TITLE, text));
            }
        }
        if self.want(OutputSection::Changes)
            && let Some(text) = facets::changes(self.doc)
        {
            out.push(("changes", CHANGES_TITLE, text));
        }
        if self.want(OutputSection::Embedded)
            && let Some(text) = facets::embedded(self.doc, "../")
        {
            out.push(("embedded", EMBEDDED_TITLE, text));
        }
        if self.want(OutputSection::Content)
            && let Some(outline) = self.doc.body.iter().find(|b| is_pdf_outline(self.doc, b))
        {
            let ctx = self.ctx(notes);
            let text = blocks::blocks(std::slice::from_ref(outline), &ctx, 0, 2, false);
            if !text.is_empty() {
                out.push(("outline", OUTLINE_TITLE, text));
            }
        }
        out
    }

    /// A trailing section as units. Logic is named per `###` part it starts in
    /// (`logic-<sheet>`), when it does not fit whole.
    fn push_trailing(
        &self,
        units: &mut Vec<Unit>,
        names: &mut Names,
        base: &str,
        location: &str,
        text: &str,
    ) {
        let pieces = self.pieces(text);
        let whole = pieces.len() == 1;
        // The part the previous piece is in, and how many pieces it has had.
        let mut prev: Option<(String, String)> = None;
        let mut n = 0usize;
        for piece in pieces {
            let part = if whole || base != "logic" {
                None
            } else {
                logic_part(&piece)
            };
            // Packed `###` parts are located `<first> to <last>`.
            let last = last_logic_part(&piece);
            let here = |title: &str| {
                let shown = match &last {
                    Some(l) if l != title => format!("{title} to {l}"),
                    _ => title.to_string(),
                };
                (
                    format!("{base}-{}", slug(title)),
                    capped(&format!("{location}, {shown}"), TITLE_CAP),
                )
            };
            let (piece_base, piece_location) = match (&part, &prev) {
                (Some(PartStart::Begins(title) | PartStart::Continues(title)), _) => here(title),
                // A piece without a heading of its own goes on in the part before it.
                (None, Some(before)) => before.clone(),
                (None, None) => (base.to_string(), location.to_string()),
            };
            let goes_on = !matches!(part, Some(PartStart::Begins(_)))
                && prev.as_ref().is_some_and(|(b, _)| *b == piece_base);
            n = if goes_on { n + 1 } else { 1 };
            prev = Some((piece_base.clone(), piece_location.clone()));
            let order = units.len() + 1;
            let title = family_title(&piece_location, n);
            units.push(Unit {
                file: names.take(&piece_base, "", order),
                naming: Naming {
                    own: true,
                    base: title.clone(),
                    location: piece_location.clone(),
                    piece: 0,
                },
                title,
                location: piece_location,
                markdown: piece,
            });
        }
    }
}

/// Where a piece of `## Logic` starts: at a `###` part, or inside one (its heading repeated
/// as continued).
enum PartStart {
    Begins(String),
    Continues(String),
}

/// The `###` part a piece of `## Logic` begins in, its title without escapes; `None` when
/// it begins with other text.
fn logic_part(piece: &str) -> Option<PartStart> {
    let mut lines = piece.lines().filter(|l| !l.trim().is_empty());
    let mut first = lines.next()?;
    if first == "## Logic" || first == "## Logic (continued)" {
        first = lines.next()?;
    }
    let title = first.strip_prefix("### ")?;
    Some(match title.strip_suffix(" (continued)") {
        Some(t) => PartStart::Continues(unescape(t)),
        None => PartStart::Begins(unescape(title)),
    })
}

/// The title of the last `###` part that begins in a piece of `## Logic`, without escapes.
fn last_logic_part(piece: &str) -> Option<String> {
    piece
        .lines()
        .filter_map(|l| l.strip_prefix("### "))
        .rfind(|t| !t.ends_with(" (continued)"))
        .map(unescape)
}

/// Rendered line text back as plain text: a backslash before ASCII punctuation goes.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(next) = chars.peek().copied()
            && next.is_ascii_punctuation()
        {
            out.push(next);
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// A spec for an item that fits.
fn spec(item: Item<'_>) -> Spec<'_> {
    Spec {
        item,
        suffix: String::new(),
        pieces: 1,
        anchors: None,
    }
}

/// A spec for an item cut as text: as many pieces as its trial render gives.
fn cut_spec<'a>(mut item: Item<'a>, splitter: &mut Splitter<'a>) -> Spec<'a> {
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
        anchors: None,
    }
}

/// A row range cut again into ranges of at most `max` tokens, with the same lead, header
/// band and notes. A table without layout counts its own rows, so the new ranges' numbers
/// are moved to the rows of the whole table.
fn resplit(range: &rezzer_model::RowRange, max: usize) -> Vec<rezzer_model::RowRange> {
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
fn packs(prev: &Item<'_>, next: &Item<'_>) -> bool {
    match (&prev.content, &next.content) {
        (Content::Slice { list: a, to, .. }, Content::Slice { list: b, from, .. }) => {
            std::ptr::eq(*a, *b) && to == from && prev.wraps == next.wraps
        }
        _ => false,
    }
}

/// Packed items as one: the first one's name (`pages-<a>-<b>` for pages without an
/// outline), `<first> to <last>` as title and location where they differ.
fn merge<'a>(group: &[Item<'a>]) -> Item<'a> {
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
    let joined = |a: &str, b: &str| {
        if a == b {
            a.to_string()
        } else {
            capped(&format!("{a} to {b}"), TITLE_CAP)
        }
    };
    item.location = match pages {
        Some(p) => pages_location(p),
        None => joined(&first.location, &last.location),
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
        item.title = joined(&first.title, &last.title);
    }
    item
}

/// `page 12` or `pages 12 to 20` (decision 10).
fn pages_location((a, b): (u32, u32)) -> String {
    if a == b {
        format!("page {a}")
    } else {
        format!("pages {a} to {b}")
    }
}

/// The body without the leading level-1 heading that is the title (as the content writes
/// it).
fn content_body<'d>(doc: &'d Document, title: &str) -> &'d [Block] {
    match doc.body.first() {
        Some(Block::Heading {
            level: 1, inlines, ..
        }) if rezzer_model::plain_text(inlines).trim() == title => &doc.body[1..],
        _ => &doc.body,
    }
}

/// The page number of a page section, from its anchor `p<n>`.
fn page_number(b: &Block) -> Option<u32> {
    match b {
        Block::Section {
            kind: SectionKind::Page,
            anchor,
            ..
        } => anchor
            .as_ref()
            .and_then(|a| a.0.strip_prefix('p'))
            .and_then(|n| n.parse().ok()),
        _ => None,
    }
}

/// The outline's top-level chapters with a page, in page order, one per page (the first
/// title wins).
fn chapters(doc: &Document) -> Vec<(u32, &str)> {
    let mut list: Vec<(u32, &str)> = doc
        .facets
        .outline
        .iter()
        .filter(|e| e.level == 1)
        .filter_map(|e| e.page.map(|p| (p, e.title.as_str())))
        .collect();
    list.sort_by_key(|(p, _)| *p);
    list.dedup_by_key(|(p, _)| *p);
    list
}

/// True for the Outline section of a PDF: a trailing unit of its own.
fn is_pdf_outline(doc: &Document, b: &Block) -> bool {
    doc.source.format == FormatId::Pdf
        && matches!(b, Block::Section {
            kind: SectionKind::Other,
            title: Some(title),
            ..
        } if title == "Outline")
}

/// A run of the body's top level for the units: blocks of the body, or a section made anew
/// (the footnote definitions no unit cites).
enum Run {
    Blocks(usize, usize),
    Owned(Block),
}

/// The definitions of the Markdown reader's `Footnotes` section: a titled section that holds
/// only footnote definitions.
fn footnotes_section(b: &Block) -> Option<&[Block]> {
    match b {
        Block::Section {
            title: Some(title),
            blocks,
            ..
        } if title == "Footnotes"
            && !blocks.is_empty()
            && blocks.iter().all(|d| footnote_label(d).is_some()) =>
        {
            Some(blocks)
        }
        _ => None,
    }
}

/// The label of a footnote definition (`fn:<label>`, no title).
fn footnote_label(b: &Block) -> Option<&str> {
    match b {
        Block::Section {
            title: None,
            anchor: Some(anchor),
            ..
        } => anchor.0.strip_prefix("fn:"),
        _ => None,
    }
}

/// A section that is a unit of its own: any section but a footnote definition.
fn is_section(b: &Block) -> bool {
    matches!(b, Block::Section { .. }) && footnote_label(b).is_none()
}

fn heading_level(b: &Block) -> Option<u8> {
    match b {
        Block::Heading { level, .. } => Some(*level),
        _ => None,
    }
}

/// A block that stays with the block after it: a heading or a caption.
fn leads(b: &Block) -> bool {
    match b {
        Block::Heading { .. } => true,
        Block::Paragraph {
            role: Some(role), ..
        } => role == "caption",
        _ => false,
    }
}

fn is_note(b: &Block) -> bool {
    matches!(b, Block::Paragraph { role: Some(role), .. } if role == "note")
}

/// The part of a row range name that tells the table, when its list holds more than one
/// table: `-<key slug>` from the layout key without its sheet (`table:Sales` gives
/// `-sales`, `Data!B5` gives `-b5`), else `-table-<n>`.
fn table_key(list: &[Block], table: &Block) -> String {
    let tables: Vec<&Block> = list
        .iter()
        .filter(|b| matches!(b, Block::Table { .. }))
        .collect();
    if tables.len() < 2 {
        return String::new();
    }
    if let Block::Table {
        layout: Some(layout),
        ..
    } = table
    {
        let key = layout.key.rsplit(['!', ':']).next().unwrap_or(&layout.key);
        let key = slug(key);
        let key = cut_slug(&key, KEY_CAP);
        if !key.is_empty() {
            return format!("-{key}");
        }
    }
    let n = tables
        .iter()
        .position(|t| std::ptr::eq(*t, table))
        .map_or(1, |i| i + 1);
    format!("-table-{n}")
}

/// `wraps` around the text `body` makes, outermost first; a page's own notes only where the
/// unit reaches the page's end (`last`).
fn wrapped(
    wraps: &[Wrap<'_>],
    last: bool,
    ctx: &Ctx,
    depth: usize,
    level: u8,
    body: &dyn Fn(usize, u8) -> String,
) -> String {
    match wraps.split_first() {
        None => body(depth, level),
        Some((w, rest)) => blocks::section(
            w.kind,
            w.title,
            w.anchor.filter(|_| last),
            ctx,
            depth,
            level,
            &|d, l| wrapped(rest, last, ctx, d, l, body),
        ),
    }
}

/// A node of the model a walk visits.
#[derive(Clone, Copy)]
enum Node<'x> {
    Block(&'x Block),
    Inline(&'x Inline),
}

/// Visits every block and inline of `list`, nested ones included (cells too), down to
/// [`MAX_DEPTH`].
fn walk<'x>(list: &'x [Block], depth: usize, f: &mut dyn FnMut(Node<'x>)) {
    if depth > MAX_DEPTH {
        return;
    }
    for b in list {
        f(Node::Block(b));
        match b {
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => walk(blocks, depth + 1, f),
            Block::List { items, .. } => {
                for item in items {
                    walk(item, depth + 1, f);
                }
            }
            Block::Paragraph { inlines, .. }
            | Block::Heading { inlines, .. }
            | Block::Task { inlines, .. } => walk_inlines(inlines, depth + 1, f),
            Block::Table { rows, .. } => {
                for cell in rows.iter().flatten() {
                    walk_inlines(&cell.inlines, depth + 1, f);
                }
            }
            Block::Code { .. } | Block::Math { .. } | Block::Media { .. } | Block::Rule { .. } => {}
        }
    }
}

fn walk_inlines<'x>(list: &'x [Inline], depth: usize, f: &mut dyn FnMut(Node<'x>)) {
    if depth > MAX_DEPTH {
        return;
    }
    for i in list {
        f(Node::Inline(i));
        if let Inline::Link { inlines, .. } = i {
            walk_inlines(inlines, depth + 1, f);
        }
    }
}

/// The footnote labels `list` cites.
fn cited_footnotes(list: &[Block], depth: usize, out: &mut HashSet<String>) {
    walk(list, depth, &mut |node| {
        if let Node::Inline(Inline::FootnoteRef { id }) = node
            && !out.contains(id.as_str())
        {
            out.insert(id.clone());
        }
    });
}

/// Every anchor `list` holds: of its blocks and sections and its inline anchor marks.
fn collect_anchors<'x>(list: &'x [Block], depth: usize, f: &mut dyn FnMut(&'x str)) {
    walk(list, depth, &mut |node| match node {
        Node::Block(b) => {
            if let Some(anchor) = block_anchor(b) {
                f(anchor);
            }
        }
        Node::Inline(Inline::AnchorMark { anchor }) => f(&anchor.0),
        Node::Inline(_) => {}
    });
}

fn block_anchor(b: &Block) -> Option<&str> {
    match b {
        Block::Section { anchor, .. }
        | Block::Heading { anchor, .. }
        | Block::Paragraph { anchor, .. }
        | Block::List { anchor, .. }
        | Block::Table { anchor, .. }
        | Block::Code { anchor, .. }
        | Block::Quote { anchor, .. }
        | Block::Math { anchor, .. }
        | Block::Media { anchor, .. }
        | Block::Callout { anchor, .. }
        | Block::Rule { anchor, .. }
        | Block::Task { anchor, .. } => anchor.as_ref().map(|a| a.0.as_str()),
    }
}

/// Where the units' anchors are, for the links of unit `current`.
struct Links<'x> {
    at: &'x HashMap<&'x str, usize>,
    /// Each unit's first file name.
    files: &'x [String],
    /// The fragment a heading anchor has in the part file that holds it.
    frags: &'x HashMap<&'x str, String>,
    current: usize,
}

impl Links<'_> {
    /// The new target of an anchor link `#frag` whose anchor lies in another unit:
    /// `<file>.md#<fragment>`, the fragment the heading has in that file (a second
    /// `Summary` is `summary` in a part where it comes first). The anchor is the fragment
    /// itself, `h:<fragment>`, or the heading slug of the decoded fragment.
    fn anchor_target(&self, frag: &str) -> Option<String> {
        let decoded = heading_slug(&percent_decode(frag));
        let candidates = [
            frag.to_string(),
            format!("h:{frag}"),
            format!("h:{decoded}"),
        ];
        for c in &candidates {
            if let Some(&k) = self.at.get(c.as_str()) {
                if k == self.current {
                    return None;
                }
                let file = self.files.get(k)?;
                let frag = match self.frags.get(c.as_str()) {
                    Some(f) => f.as_str(),
                    None => c.strip_prefix("h:").unwrap_or(c),
                };
                return Some(format!("{file}.md#{frag}"));
            }
        }
        None
    }

    /// The target an inline gets in its unit, when it changes.
    fn target(&self, inline: &Inline) -> Option<String> {
        match inline {
            Inline::Link {
                target,
                internal: true,
                wiki: false,
                ..
            } => match target.strip_prefix('#') {
                Some(frag) => self.anchor_target(frag),
                None => relative_path(target).then(|| format!("../{target}")),
            },
            Inline::Image { target, .. } => relative_path(target).then(|| format!("../{target}")),
            _ => None,
        }
    }
}

/// The blocks whose anchors lead to a spec's unit.
fn anchor_source<'a>(s: &Spec<'a>) -> Option<&'a [Block]> {
    match (&s.anchors, &s.item.content) {
        (Some(list), _) => Some(*list),
        (None, Content::Slice { list, from, to }) => Some(&list[*from..*to]),
        (None, Content::Owned(_)) => None,
    }
}

/// The fragment each heading anchor has in the part file of its unit: the headings of the
/// unit in order (its sections' titles first), slugged as the Markdown reader does, a
/// repeated slug numbered `-1`, `-2` within the file.
fn fragments<'a>(specs: &[Spec<'a>], at: &HashMap<&'a str, usize>) -> HashMap<&'a str, String> {
    let mut out: HashMap<&'a str, String> = HashMap::new();
    for (k, s) in specs.iter().enumerate() {
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut unique = |text: &str| {
            let slug = heading_slug(text);
            let n = seen.entry(slug.clone()).or_insert(0);
            let frag = if *n == 0 { slug } else { format!("{slug}-{n}") };
            *n += 1;
            frag
        };
        let mut take = |anchor: Option<&'a str>, frag: String| {
            if let Some(a) = anchor
                && at.get(a) == Some(&k)
            {
                out.entry(a).or_insert(frag);
            }
        };
        for w in &s.item.wraps {
            if let Some(title) = w.title.filter(|t| !t.trim().is_empty()) {
                let frag = unique(title);
                take(w.anchor, frag);
            }
        }
        if let Some(list) = anchor_source(s) {
            walk(list, 0, &mut |node| match node {
                Node::Block(Block::Heading {
                    inlines, anchor, ..
                }) => {
                    let frag = unique(&rezzer_model::plain_text(inlines));
                    take(anchor.as_ref().map(|a| a.0.as_str()), frag);
                }
                Node::Block(Block::Section {
                    title: Some(title),
                    anchor,
                    ..
                }) if !title.trim().is_empty() => {
                    let frag = unique(title);
                    take(anchor.as_ref().map(|a| a.0.as_str()), frag);
                }
                _ => {}
            });
        }
    }
    out
}

fn annotation_needs_rewrite(a: &rezzer_model::Annotation, links: &Links<'_>, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let mut found = false;
    walk_inlines(&a.body, depth, &mut |node| {
        if !found && let Node::Inline(inline) = node {
            found = links.target(inline).is_some();
        }
    });
    found
        || a.replies
            .iter()
            .any(|r| annotation_needs_rewrite(r, links, depth + 1))
}

fn rewrite_annotation(a: &mut rezzer_model::Annotation, links: &Links<'_>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    rewrite_inlines(&mut a.body, links, depth);
    for r in &mut a.replies {
        rewrite_annotation(r, links, depth + 1);
    }
}

/// A target that is a relative path: one folder down it needs `../`.
fn relative_path(target: &str) -> bool {
    if target.is_empty() || target.starts_with(['#', '/', '\\', '?']) {
        return false;
    }
    let before = target.split(['/', '?', '#']).next().unwrap_or("");
    !before.contains(':')
}

fn block_needs_rewrite(b: &Block, links: &Links<'_>, depth: usize) -> bool {
    let mut found = false;
    walk(std::slice::from_ref(b), depth, &mut |node| {
        if !found && let Node::Inline(inline) = node {
            found = links.target(inline).is_some();
        }
    });
    found
}

fn rewrite_block(b: &mut Block, links: &Links<'_>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match b {
        Block::Section { blocks, .. }
        | Block::Quote { blocks, .. }
        | Block::Callout { blocks, .. } => {
            for inner in blocks {
                rewrite_block(inner, links, depth + 1);
            }
        }
        Block::List { items, .. } => {
            for inner in items.iter_mut().flatten() {
                rewrite_block(inner, links, depth + 1);
            }
        }
        Block::Paragraph { inlines, .. }
        | Block::Heading { inlines, .. }
        | Block::Task { inlines, .. } => rewrite_inlines(inlines, links, depth + 1),
        Block::Table { rows, .. } => {
            for cell in rows.iter_mut().flatten() {
                rewrite_inlines(&mut cell.inlines, links, depth + 1);
            }
        }
        Block::Code { .. } | Block::Math { .. } | Block::Media { .. } | Block::Rule { .. } => {}
    }
}

fn rewrite_inlines(list: &mut [Inline], links: &Links<'_>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for inline in list {
        if let Some(new) = links.target(inline) {
            match inline {
                Inline::Link { target, .. } | Inline::Image { target, .. } => *target = new,
                _ => {}
            }
        }
        if let Inline::Link { inlines, .. } = inline {
            rewrite_inlines(inlines, links, depth + 1);
        }
    }
}

/// `%XX` codes decoded; other text as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A heading slug the way the Markdown reader builds its anchors: lowercase letters and
/// digits, a space or `-` as `-`, `_` kept, other characters left out.
fn heading_slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c == ' ' || c == '-' {
            out.push('-');
        } else if c == '_' {
            out.push('_');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        Names, Naming, OWN_TITLES, Unit, capped, finish_titles, part_budget, part_measure, slug,
        split, title_key,
    };
    use crate::RenderOptions;
    use crate::describe::body_tokens;
    use rezzer_model::{
        Anchor, Asset, Block, Confidence, Detection, Document, FormatId, Inline, SectionKind,
        SourceInfo,
    };

    fn doc() -> Document {
        Document::new(SourceInfo {
            path: None,
            name: "a report with a long name (final).md".into(),
            sha256: "00".into(),
            size: 1,
            format: FormatId::Markdown,
            detection: Detection {
                confidence: Confidence::Certain,
                evidence: "test".into(),
            },
            modified: None,
            created: None,
            downloaded_from: None,
            file_tags: Vec::new(),
            encoding: None,
            reader: None,
        })
    }

    /// A part file as the part writer will make it at worst: the longest title, the
    /// description and both relations around the unit's body.
    /// The main name is percent-encoded as the renderer writes a destination.
    fn part_file(title: &str, doc_title: &str, main: &str, body: &str) -> String {
        let long = "y".repeat(super::TITLE_CAP);
        let main = main
            .replace(' ', "%20")
            .replace('(', "%28")
            .replace(')', "%29");
        format!(
            "---\ntype: source\n---\n\n# {doc_title}: {title}, part 9999\n\nPart 9999 of 9999 of \"{doc_title}\": {long}.\n\n{body}\n\n## Relations\n\n- part_of [{doc_title}](../{main})\n- follows [{doc_title}: {long}, part 9999]({}.md)\n",
            "z".repeat(60)
        )
    }

    #[test]
    fn units_with_their_part_overhead_stay_within_the_part_size() {
        // Pictures (import length) and anchor links to other units (file names) in every
        // chapter: the finished part file is within the size by Crystalline's measure.
        let mut d = doc();
        for i in 0..40 {
            let id = format!("p{i}");
            d.assets.push(Asset {
                id: id.clone(),
                path: Some(format!("b.assets/{id}.png")),
                sha256: "00".into(),
                mime: "image/png".into(),
                caption: None,
                alt: Some("p".into()),
                needs_vision: false,
                codes: Vec::new(),
                code_unread: false,
            });
        }
        for c in 0..8 {
            d.body.push(Block::Heading {
                level: 2,
                inlines: vec![Inline::plain(format!("Chapter {c}"))],
                anchor: Some(Anchor(format!("h:chapter-{c}"))),
            });
            for k in 0..60 {
                let mut inlines = vec![Inline::plain("see ")];
                inlines.push(Inline::Link {
                    target: format!("#chapter-{}", (c + 3) % 8),
                    title: None,
                    internal: true,
                    wiki: false,
                    inlines: vec![Inline::plain("there")],
                });
                inlines.push(Inline::plain(
                    " and some words to fill the line up a bit more",
                ));
                d.body.push(Block::Paragraph {
                    inlines,
                    role: None,
                    anchor: None,
                });
                if k % 10 == 0 {
                    d.body.push(Block::Media {
                        asset: format!("p{}", (c * 6 + k / 10) % 40),
                        alt: None,
                        anchor: None,
                    });
                }
            }
        }
        d.body.push(Block::Section {
            kind: SectionKind::Other,
            title: Some("Tail".into()),
            anchor: None,
            blocks: vec![],
        });
        let opts = RenderOptions::default();
        let bundle = "a-report-with-a-long-name-final-0123abcd";
        let list = split(&d, &opts, 2_500, bundle).expect("over the size");
        assert!(list.len() > 2);
        let budget = part_budget(&d, &opts, 2_500);
        let main = "a report with a long name (final).md-0123abcd.md";
        for u in &list {
            assert!(
                part_measure(bundle).chars(&u.markdown) <= budget,
                "{}",
                u.file
            );
            let file = part_file(
                &u.title,
                "a report with a long name (final).md",
                main,
                &u.markdown,
            );
            let note = bundle.len() + 1 + u.file.len();
            assert!(
                body_tokens(&file, note) <= 2_500,
                "{}: {}",
                u.file,
                body_tokens(&file, note)
            );
        }
    }

    #[test]
    fn slugs_and_names() {
        assert_eq!(slug("Über alles!"), "ber-alles");
        assert_eq!(slug("  --A_b--  "), "a-b");
        let mut names = Names::new();
        assert_eq!(names.take("Index", "", 1), "index-2");
        assert_eq!(names.take("Contents", "", 2), "contents-2");
        assert_eq!(names.take("!!!", "", 3), "part-3");
        assert_eq!(names.take("Big", "-rows-2-10", 4), "big-rows-2-10");
        assert_eq!(names.take("Big", "-rows-2-10", 5), "big-rows-2-10-2");
        let long = "x".repeat(80);
        assert_eq!(names.take(&long, "-rows-1-2", 6).len(), 60);
    }

    #[test]
    fn capped_cuts_at_a_space() {
        assert_eq!(capped("short", 10), "short");
        assert_eq!(capped("one two three four", 12), "one two...");
        assert_eq!(capped(&"x".repeat(20), 10), "xxxxxxx...");
    }

    #[test]
    fn logic_parts_read_their_heading() {
        use super::{PartStart, logic_part};
        let begins = |p: &str| match logic_part(p) {
            Some(PartStart::Begins(t)) => Some(t),
            _ => None,
        };
        assert_eq!(
            begins("## Logic\n\n### Q1 \\*\n\ntext").as_deref(),
            Some("Q1 *")
        );
        assert!(matches!(
            logic_part("### Data (continued)\n\nrows"),
            Some(PartStart::Continues(t)) if t == "Data"
        ));
        assert!(logic_part("## Logic\n\n> [!WARNING] Old\n\n### Data").is_none());
    }

    fn unit(title: &str, location: &str, own: bool, piece: usize) -> Unit {
        Unit {
            file: String::new(),
            title: String::new(),
            location: location.into(),
            markdown: String::new(),
            naming: Naming {
                own,
                base: title.into(),
                location: location.into(),
                piece,
            },
        }
    }

    fn titled(units: &mut [Unit], extra: &[&str]) -> Vec<String> {
        finish_titles(units, extra);
        units.iter().map(|u| u.title.clone()).collect()
    }

    /// Every title Rezzer gives its own parts (ruling P5-AF): a source unit titled like it,
    /// like a piece of it, or in another letter case gets its location, else `(section)`,
    /// but only when the document really has that part (as one of its units or as a part
    /// the main document adds). Rezzer's own unit keeps its title.
    #[test]
    fn every_own_title_is_checked() {
        for own in OWN_TITLES {
            let lower = own.to_lowercase();
            let piece = format!("{own}, part 2");
            for title in [own, lower.as_str(), piece.as_str()] {
                // The part is one of the units.
                assert_eq!(
                    titled(
                        &mut [unit(own, own, true, 0), unit(title, title, false, 0)],
                        &[]
                    ),
                    [own.to_string(), format!("{title} (section)")]
                );
                assert_eq!(
                    titled(
                        &mut [
                            unit(own, own, true, 0),
                            unit(title, "pages 1 to 2", false, 0)
                        ],
                        &[]
                    ),
                    [own.to_string(), format!("{title} (pages 1 to 2)")]
                );
                // The main document writer adds the part.
                assert_eq!(
                    titled(&mut [unit(title, title, false, 0)], &[own]),
                    [format!("{title} (section)")]
                );
                // No such part: the title stays.
                assert_eq!(
                    titled(&mut [unit(title, title, false, 0)], &[]),
                    [title.to_string()]
                );
            }
        }
        // The pieces of a renamed unit keep one name.
        assert_eq!(
            titled(
                &mut [
                    unit("Logic", "Logic", false, 1),
                    unit("Logic", "Logic", false, 2),
                    unit("Logic", "Logic", true, 0),
                ],
                &[]
            ),
            ["Logic (section)", "Logic (section), part 2", "Logic"]
        );
        assert_eq!(title_key(" Start "), "start");
        assert_eq!(
            titled(&mut [unit("Startup", "Startup", false, 0)], &["Start"]),
            ["Startup"]
        );
    }
}
