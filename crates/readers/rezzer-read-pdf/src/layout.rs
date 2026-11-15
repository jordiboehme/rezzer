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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroU32;
use std::ops::Range;
use std::rc::Rc;

use pdfparts::StructTree;
use rezzer_limits::{Budget, LimitHit};
use rezzer_model::{
    Anchor, Block, Cell, Inline, KEPT_AS_LAID_OUT, LossReason, MARKED_IN_JSON_ONLY, SectionKind,
    Severity, StatusColor,
};

use layoutregions::GapIn;
use layoutregions::extract::spatial::SpatialFrame;
use layoutregions::extract::text::{
    Demotion, TextBlock, TextCx, continues, list_marker, next_block, signed_amount,
};
#[cfg(doc)]
use layoutregions::file::FileFacts;
use layoutregions::file::{self, Band, FilePage, Levels, RunningIndex, SIZE_LEVELS};
use layoutregions::line::{
    EDGE_SIZES, LineRun, TextLine, apart, text_line as line, text_lines as lines,
};
use layoutregions::segment::drawn::{DrawnStructure, Grids, Unsearched};
use layoutregions::{Kind, PageRegions};

use crate::engine;
use crate::found;
use crate::tables::{self, Planned, PlannedCell, Rule, Tables, Unsure};

mod decide;
mod footnotes;
mod head;
mod hosts;
mod shape;
mod today;

use decide::Class;
use hosts::Became;

pub(crate) use head::{Dates, PageDate, TITLE_FROM, document_date, page_dates};
use shape::ends_in_amount;

use layoutregions::extract::pairs::blocks::{address_block, paragraph_columns};

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

/// One page as the file pass sees it: its runs, borrowed, and the height of their frame.
pub(crate) type PageText<'a> = FilePage<'a, Run>;

/// What the layout learns from the whole file before it lays out a page (see
/// [`layoutregions::file::learn`]), kept the way the layout looks it up.
#[derive(Debug, Default)]
pub(crate) struct Style {
    /// The body size of the file (see [`FileFacts::body_size`]).
    pub body_size: f32,
    /// Distinct heading sizes of an untagged file, largest first.
    heading_sizes: Vec<f32>,
    /// The running headers and footers by band, key and height.
    running: RunningIndex,
    /// The keys of the footers of a file of two pages (see [`FileFacts::two_page`]).
    two_page: HashSet<String>,
    /// The footnote legend of each untagged page that has one, by the page's runs.
    legends: BTreeMap<u32, footnotes::Found>,
    /// The facts as learned, as the typers read them.
    facts: layoutregions::file::FileFacts,
    /// The facts as the typers look them up.
    index: layoutregions::typers::FileIndex,
}

/// One visual line.
type Line<'a> = TextLine<'a, Run>;

impl LineRun for Run {
    fn text(&self) -> &str {
        &self.text
    }
    fn x(&self) -> f32 {
        self.x
    }
    fn y(&self) -> f32 {
        self.y
    }
    fn w(&self) -> f32 {
        self.w
    }
    fn h(&self) -> f32 {
        self.h
    }
    fn size(&self) -> f32 {
        self.size
    }
    fn bold(&self) -> bool {
        self.bold
    }
    fn italic(&self) -> bool {
        self.italic
    }
    fn font(&self) -> u32 {
        self.family
    }
    fn line(&self) -> u32 {
        self.line
    }
    fn page(&self) -> u32 {
        self.page
    }
    fn off_axis(&self) -> bool {
        self.off_axis
    }
    fn note(&self) -> bool {
        self.note
    }
    fn gaps(&self) -> impl Iterator<Item = GapIn> + '_ {
        self.gaps.iter().map(|g| GapIn {
            words: g.words,
            from: g.from,
            to: g.to,
        })
    }
}

impl Style {
    /// [`Style::learn`] for an untagged file, with nothing stopping it.
    #[cfg(test)]
    pub(crate) fn new(pages: &[PageText<'_>]) -> Style {
        Style::learn(pages, false, &|| false).0
    }

    /// Learns the facts of the file from every page (see
    /// [`layoutregions::file::learn_until`]); `tagged` files say their footnotes themselves.
    /// `stop` is asked before each page of each pass; once it says stop the facts are those
    /// of the pages read by then, and the second value says so.
    pub(crate) fn learn(
        pages: &[PageText<'_>],
        tagged: bool,
        stop: &dyn Fn() -> bool,
    ) -> (Style, bool) {
        let (facts, stopped) = file::learn_until(pages, tagged, &ends_in_amount, stop);
        let index = layoutregions::typers::FileIndex::new(&facts);
        let style = Style {
            body_size: facts.body_size,
            heading_sizes: facts.heading_sizes.clone(),
            running: RunningIndex::new(&facts.running),
            two_page: facts.two_page.iter().cloned().collect(),
            legends: facts.legends.clone(),
            facts,
            index,
        };
        (style, stopped)
    }

    /// The band of each line that is a running header or footer on its page (see
    /// [`RunningIndex::bands`]). A line marked in `content` is never a header or footer; an
    /// empty `content` marks none.
    fn furniture(&self, lines: &[Line<'_>], height: f32, content: &[bool]) -> Vec<Option<Band>> {
        self.running.bands(lines, height, content)
    }

    /// Whether a line on a page of `height` is a footer by the two-page rule.
    fn two_page_line(&self, line: &Line<'_>, height: f32) -> bool {
        file::two_page_line(line, height, &self.two_page)
    }

    /// The heading levels of the untagged lines of a page whose text is set in `text_sizes`
    /// (see [`Levels::level`]).
    fn levels<'a>(&'a self, text_sizes: &'a [f32]) -> Levels<'a> {
        Levels {
            body_size: self.body_size,
            heading_sizes: &self.heading_sizes,
            text_sizes,
            ends_in_amount: &ends_in_amount,
        }
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
    /// Losses at a block of the page: its anchor, words, reason, severity (a block kept as
    /// laid out says why).
    pub located: Vec<(String, String, LossReason, Severity)>,
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
    /// The regions of the page: one per decision of the region engine, each primitive in one,
    /// each at the block it became.
    pub regions: PageRegions,
    /// The page as the region engine took it; none for a page not laid out.
    pub engine_in: Option<layoutregions::PageIn>,
    /// The regions that became rows of a tagged table begun on an earlier page: the region's
    /// place in `regions` and the table. Their host is a block of this page near them, for
    /// when the table has no anchor.
    pub earlier: Vec<(usize, usize)>,
    /// The block of `blocks` each run of the page (by its id) was written in, `u32::MAX` for
    /// a run written in none.
    pub run_blocks: Vec<u32>,
    /// The pictures and codes of the page's figure and code regions, each where its region
    /// stands in the page's reading order (see [`MediaAt`]). A picture of no such region is
    /// placed by the block of the run nearest to it.
    pub media_at: Vec<MediaAt>,
    /// The pictures that stand in a table cell (a picture in each row of an item
    /// table): the picture, the table's place in `blocks`, the row and the cell. Each is
    /// written in its cell, not as a media block of its own.
    pub in_cells: Vec<(layoutregions::PrimId, usize, usize, usize)>,
}

/// Where a picture or code of a figure or code region goes among the blocks of its page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaAt {
    pub prim: layoutregions::PrimId,
    /// The place in the page's blocks it stands before: after the blocks of the regions read
    /// before its region.
    pub at: usize,
    /// The blocks of the region read just before its region and of the one read just after
    /// it, from the first to past the last (`at` when there is none). The engine orders
    /// regions, not the blocks inside one: where the picture stands beside a region of several
    /// blocks, the block of the run nearest to it may say a place among them, after the first
    /// of them and up to past the last.
    pub near: Range<usize>,
}

/// A run of the engine's input as the file facts are learned from it: the reader's own runs
/// say which run is a footnote reference (`note`), and `page_in` writes that as a raised run.
struct NoteRun<'a>(&'a layoutregions::TextRun);

impl LineRun for NoteRun<'_> {
    fn text(&self) -> &str {
        &self.0.text
    }
    fn x(&self) -> f32 {
        self.0.x
    }
    fn y(&self) -> f32 {
        self.0.y
    }
    fn w(&self) -> f32 {
        self.0.w
    }
    fn h(&self) -> f32 {
        self.0.h
    }
    fn size(&self) -> f32 {
        self.0.size
    }
    fn bold(&self) -> bool {
        self.0.bold
    }
    fn italic(&self) -> bool {
        self.0.italic
    }
    fn font(&self) -> u32 {
        self.0.font
    }
    fn line(&self) -> u32 {
        self.0.line
    }
    fn off_axis(&self) -> bool {
        self.0.off_axis
    }
    fn note(&self) -> bool {
        self.0.shift == layoutregions::Shift::Raised
    }
    fn gaps(&self) -> impl Iterator<Item = GapIn> + '_ {
        self.0.gaps.iter().copied()
    }
}

/// The engine's decisions on each of `pages` (one file, in page order), made by the layout's
/// own [`decide::decide`] at the default engine limits: the file's facts learned from these
/// pages as the reader learns them (`tagged` as for the file, a raised run a footnote
/// reference), then per page the segments, the typers, the arbitration and the layout's steps
/// after it (a segment a bound left whole kept as laid out, pictures parted from text). Given
/// regions of a page are decided as given, with no reading of the pre-pass readers behind them.
/// Each decision is its kind, its primitives and what made its segment and the ones over it
/// (innermost first, joined by `<`), in the order of the decisions. The runs are read in the
/// order of their places, whatever order the pages hold them in. For the metamorphic tests,
/// which compare these decisions on a page and on the same page changed in a way that keeps
/// its meaning.
pub(crate) fn engine_decisions(
    pages: &[layoutregions::PageIn],
    tagged: bool,
) -> Vec<Vec<(Kind, Vec<layoutregions::PrimId>, String)>> {
    engine_decisions_why(pages, tagged)
        .into_iter()
        .map(|page| page.into_iter().map(|(k, p, m, _)| (k, p, m)).collect())
        .collect()
}

/// The facts of the file of `pages` as [`engine_decisions_why`] learns them: with `placed`
/// from the runs of each page in the order of their places ([`layoutregions::RunOrder`]),
/// else in the order they have on the page. With them, per page, the id each run of the
/// order learned in has on the page handed in (`None`: the same ids). The runs of a legend
/// are named by the order learned in. [`file::learn`] reads the lines in place order itself;
/// `placed` is kept for the legends and for the test that compares the two.
fn engine_facts(
    pages: &[layoutregions::PageIn],
    tagged: bool,
    placed: bool,
) -> (file::FileFacts, Vec<Option<Vec<u32>>>) {
    let ordered: Vec<Option<layoutregions::RunOrder>> = pages
        .iter()
        .map(|p| placed.then(|| layoutregions::RunOrder::of(p)).flatten())
        .collect();
    let runs: Vec<Vec<NoteRun<'_>>> = pages
        .iter()
        .zip(&ordered)
        .map(|(p, o)| {
            let texts = o.as_ref().map_or(&p.texts, |o| &o.page.texts);
            texts.iter().map(NoteRun).collect()
        })
        .collect();
    let file: Vec<FilePage<'_, NoteRun<'_>>> = pages
        .iter()
        .zip(&runs)
        .map(|(p, r)| FilePage {
            page: p.page,
            height: p.size[1],
            runs: r,
        })
        .collect();
    let facts = file::learn(&file, tagged, &ends_in_amount);
    drop(file);
    drop(runs);
    (
        facts,
        ordered.into_iter().map(|o| o.map(|o| o.back)).collect(),
    )
}

/// The facts of the file of `pages` learned from the runs in the order they have on each
/// page (the reader's flow) and in the order of their places (as [`engine_decisions_why`]
/// learns them), the runs of the legends named by the ids of `pages` in both: the runs of
/// each footnote, each raised reference and the word it goes after or into. For the tests
/// that compare the facts of the two orders.
///
/// The first of the two is what the read learns for an untagged file. For a tagged file the
/// read learns from the reader's runs before a run that an ActualText replaces is dropped and
/// the first one rewritten, and the engine pages hold the runs after that step: there the
/// set of runs differs, not only their order.
pub(crate) fn engine_facts_by_order(
    pages: &[layoutregions::PageIn],
    tagged: bool,
) -> [file::FileFacts; 2] {
    let given = engine_facts(pages, tagged, false).0;
    let (mut by_place, back) = engine_facts(pages, tagged, true);
    for (page, back) in pages.iter().zip(&back) {
        let (Some(back), Some(legend)) = (back, by_place.legends.get_mut(&page.page)) else {
            continue;
        };
        let old = |run: &mut u32| *run = back.get(*run as usize).copied().unwrap_or(*run);
        for e in &mut legend.entries {
            e.runs.iter_mut().for_each(old);
        }
        for r in &mut legend.raised {
            old(&mut r.run);
            match &mut r.place {
                file::RefPlace::End => {}
                file::RefPlace::Inside { word, .. } | file::RefPlace::Above { word } => old(word),
            }
        }
    }
    [given, by_place]
}

/// One decision of [`engine_decisions_why`]: its kind, its primitives, what made its segment
/// and its reasons in words.
pub(crate) type DecisionWhy = (Kind, Vec<layoutregions::PrimId>, String, String);

/// [`engine_decisions`] with the reasons of each decision in words: its certainty and score,
/// the ids of its signals, and each candidate of its segment with its score and signals (the
/// kinds that claimed it, the winner among them). For the path trace of a metamorphic pin.
pub(crate) fn engine_decisions_why(
    pages: &[layoutregions::PageIn],
    tagged: bool,
) -> Vec<Vec<DecisionWhy>> {
    // The lines of a file are learned with their runs in the order of their places,
    // whatever order they are handed in (`file::learn` sorts them), on this path and on the
    // read's (`Style::learn` in `lay_out`): the body size, the heading sizes and the running
    // lines do not change with the order of the runs. The legend of a page is the exception:
    // it is read in the order handed in, because only the reader's order says that a
    // reference number opens its line, and it names its runs by the ids of the page handed
    // in (`decide` renames them for the page in place order). For a tagged file the read
    // learns before the runs an ActualText replaces are dropped, so there the set of runs
    // differs. The metamorphic tests compare the facts of the two orders
    // (`engine_facts_by_order`), and the facts of the lines also with the runs shuffled.
    //
    // This path has no reader's order to keep: its pages may come with their runs shuffled.
    // So it reads the legends from the pages in place order, which is the reader's order on
    // nearly every page.
    let [_, facts] = engine_facts_by_order(pages, tagged);
    let index = layoutregions::typers::FileIndex::new(&facts);
    let limits = layoutregions::Limits::default();
    let no_pre: HashMap<u32, usize> = HashMap::new();
    let no_signal = |_: usize| -> (layoutregions::Signal, bool) {
        unreachable!("no run is in a part of today's reading")
    };
    pages
        .iter()
        .map(|page| {
            let grids = layoutregions::segment::drawn::grids(&page.rules);
            let decided = decide::decide(
                page,
                &grids,
                (&facts, &index),
                &ends_in_amount,
                (&no_pre, &no_signal),
                &limits,
                &|| false,
            );
            (0..decided.decisions.len())
                .map(|k| {
                    // What made the segment and each one over it, innermost first.
                    let mut made = Vec::new();
                    let mut at = Some(decided.decisions[k].seg);
                    while let Some(s) = at.and_then(|s| decided.segs.all.get(s.0 as usize)) {
                        made.push(format!("{:?}", s.source));
                        at = s.parent;
                    }
                    let mut prims = decided.prims(k).to_vec();
                    prims.sort_unstable();
                    let d = &decided.decisions[k];
                    let ids = |s: &[layoutregions::Signal]| {
                        s.iter().map(|s| s.id).collect::<Vec<_>>().join(",")
                    };
                    let claims: Vec<String> = decided
                        .typed
                        .get(&d.seg)
                        .map(|t| {
                            t.iter()
                                .map(|t| {
                                    format!("{} {} {}", t.kind.name(), t.score.0, ids(&t.signals))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let why = format!(
                        "{:?} {} [{}] claims [{}]",
                        d.certainty,
                        d.score.0,
                        ids(&d.signals),
                        claims.join(" | ")
                    );
                    (d.kind, prims, made.join("<"), why)
                })
                .collect()
        })
        .collect()
}

/// The regions of a page that was not laid out.
pub(crate) fn no_regions(page: u32) -> PageRegions {
    PageRegions {
        page,
        regions: Vec::new(),
        issues: Vec::new(),
    }
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

/// The runs of a page in reading order, with their slots, their places in the page's order,
/// which stay as runs are taken out, and their ids.
#[derive(Default)]
pub(crate) struct Flow {
    pub runs: Vec<Run>,
    pub slots: Vec<Slot>,
    /// The place of each run in the page's order: sorted, also where a detector reads runs
    /// in another order (the runs move, the places stay).
    pub at: Vec<usize>,
    /// The id of each run: its index in the page's runs as the flow began (the region
    /// engine's `PrimId::Text`, see `engine::page_in`). It moves with its run; the pieces of a
    /// cut run keep it.
    pub ids: Vec<u32>,
}

impl Flow {
    pub(crate) fn new(runs: Vec<Run>, slots: Vec<Slot>) -> Flow {
        let at = (0..runs.len()).collect();
        let ids = (0..runs.len()).map(|i| i as u32).collect();
        Flow {
            runs,
            slots,
            at,
            ids,
        }
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
            ids: Vec::with_capacity(stay),
        };
        let places = all.at.into_iter().zip(all.ids);
        for (i, ((r, s), (a, id))) in all.runs.into_iter().zip(all.slots).zip(places).enumerate() {
            if let Some(r) = take(i, r) {
                kept.runs.push(r);
                kept.slots.push(s);
                kept.at.push(a);
                kept.ids.push(id);
            }
        }
        *self = kept;
    }

    /// Takes out the runs `pick` chooses by index and slot, in order, with their slots,
    /// places and ids. Both parts are sized exactly.
    pub(crate) fn take(&mut self, pick: impl Fn(usize, Slot) -> bool) -> Vec<Taken> {
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
            ids: Vec::with_capacity(stay),
        };
        let places = all.at.into_iter().zip(all.ids);
        for (i, ((r, s), (a, id))) in all.runs.into_iter().zip(all.slots).zip(places).enumerate() {
            if pick(i, s) {
                taken.push(Taken {
                    run: r,
                    slot: s,
                    at: a,
                    id,
                });
            } else {
                kept.runs.push(r);
                kept.slots.push(s);
                kept.at.push(a);
                kept.ids.push(id);
            }
        }
        *self = kept;
        taken
    }

    /// The flow parted by the class of each run (`class`, by index): one flow for each of
    /// [`Class::ALL`], each in the flow's order. The runs move, nothing is copied.
    fn parted(self, class: &[Class]) -> [Flow; 7] {
        let mut out: [Flow; 7] = Default::default();
        let places = self.at.into_iter().zip(self.ids);
        for (i, ((r, s), (a, id))) in self
            .runs
            .into_iter()
            .zip(self.slots)
            .zip(places)
            .enumerate()
        {
            let f = &mut out[class.get(i).copied().unwrap_or(Class::Text).index()];
            f.runs.push(r);
            f.slots.push(s);
            f.at.push(a);
            f.ids.push(id);
        }
        out
    }

    /// The runs of `a` and `b` together, by their places in the page's order.
    fn merged(a: Flow, b: Flow) -> Flow {
        Flow::all_merged(vec![a, b])
    }

    /// The runs of all `parts` together, by their places in the page's order (each part is in
    /// that order; a place in two parts comes first from the earlier part). One pass over the
    /// order of all places ([`layoutregions::line::by_place`], which counts its work), so a
    /// caller collects its parts and merges them once.
    fn all_merged(parts: Vec<Flow>) -> Flow {
        let mut parts: Vec<Flow> = parts.into_iter().filter(|f| !f.runs.is_empty()).collect();
        if parts.len() <= 1 {
            return parts.pop().unwrap_or_default();
        }
        let order = {
            let places: Vec<&[usize]> = parts.iter().map(|f| f.at.as_slice()).collect();
            layoutregions::line::by_place(&places)
        };
        let n = order.len();
        let mut out = Flow {
            runs: Vec::with_capacity(n),
            slots: Vec::with_capacity(n),
            at: Vec::with_capacity(n),
            ids: Vec::with_capacity(n),
        };
        /// A run of a flow with its slot, place and id, until it is moved out.
        type Entry = Option<(Run, Slot, usize, u32)>;
        let mut parts: Vec<Vec<Entry>> = parts
            .into_iter()
            .map(|f| {
                f.runs
                    .into_iter()
                    .zip(f.slots)
                    .zip(f.at.into_iter().zip(f.ids))
                    .map(|((r, s), (a, id))| Some((r, s, a, id)))
                    .collect()
            })
            .collect();
        for (p, i) in order {
            if let Some((r, s, a, id)) = parts[p][i].take() {
                out.runs.push(r);
                out.slots.push(s);
                out.at.push(a);
                out.ids.push(id);
            }
        }
        out
    }

    /// A copy of the flow.
    pub(crate) fn copied(&self) -> Flow {
        Flow {
            runs: self.runs.clone(),
            slots: self.slots.clone(),
            at: self.at.clone(),
            ids: self.ids.clone(),
        }
    }
}

impl Flow {
    /// The flow with each run at its place in reading order (`places`), sorted by it; runs
    /// at one place keep their order.
    fn read_in(self, places: &Places) -> Flow {
        let mut order: Vec<(usize, usize)> = self
            .at
            .iter()
            .enumerate()
            .map(|(k, &at)| (places.of(at), k))
            .collect();
        order.sort_by_key(|&(at, _)| at);
        let mut runs: Vec<Option<(Run, Slot, u32)>> = self
            .runs
            .into_iter()
            .zip(self.slots)
            .zip(self.ids)
            .map(|((r, s), id)| Some((r, s, id)))
            .collect();
        let mut out = Flow {
            runs: Vec::with_capacity(order.len()),
            slots: Vec::with_capacity(order.len()),
            at: Vec::with_capacity(order.len()),
            ids: Vec::with_capacity(order.len()),
        };
        for (at, k) in order {
            if let Some((r, s, id)) = runs[k].take() {
                out.runs.push(r);
                out.slots.push(s);
                out.at.push(at);
                out.ids.push(id);
            }
        }
        out
    }
}

/// The reading order of a page, set in one place: the parts of the page's regions in the
/// engine's source order ([`layoutregions::order_by_source`]). A part is a stretch of the runs
/// of one region that the page's flow (the order the text pass gave its runs) reads without a
/// run with ink of another region between them: a region whose text stands above and below a
/// table, which the flow reads between, is two parts, and the table stands between them; so
/// is a region over two drawn boxes or cells of a row of fields. The flow is the
/// source order; the engine places what the flow has no place for (a figure or code region,
/// which stands by its height among the parts). Text set across the page's direction reads
/// last, as the text pass reads it. A run's place is the rank of the part of the run that
/// began the flow at its place, then that place. Every part of the page (text, tables, text
/// kept as laid out, the running header, pictures) stands at such a place.
struct Places {
    /// The rank of the part of each run of the page, by its id.
    rank: Vec<u32>,
    /// The rank of the first part of each decision's region.
    of_decision: Vec<u32>,
    /// The part of each rank (an index into the parts, see [`Places::new`]), and the
    /// decision of each part.
    by_rank: Vec<usize>,
    decision_of: Vec<usize>,
    /// The part of each run, by its id.
    part_of: Vec<usize>,
    /// The places of the page's flow: a place in reading order is `rank * width + place`.
    width: usize,
}

impl Places {
    /// The page's places from the engine's source order of the parts of its decisions (`box_of`:
    /// the drawn box or cell of a row of fields of each run that stands in one).
    fn new(
        page: &layoutregions::PageIn,
        decided: &decide::Decided,
        box_of: &HashMap<u32, u32>,
    ) -> Places {
        use layoutregions::PrimId;
        let n = decided.decisions.len();
        let runs = decided.of_run.len();
        // The parts: the runs of the flow (by id, which is their place in the flow as it
        // began) from one run with ink to the next run with ink of another decision, drawn box
        // or cell, or that reads across the page's direction where the other reads along it. A
        // run without ink or of no decision goes with the part before it (the first ones with
        // the first part).
        let inked = |id: usize| {
            page.texts
                .get(id)
                .is_some_and(|t| t.text.chars().any(|c| !c.is_whitespace()))
        };
        let mut part_of: Vec<usize> = vec![0; runs];
        let mut decision_of: Vec<usize> = Vec::new();
        // The parts of text set across the page's direction of reading.
        let mut across: Vec<usize> = Vec::new();
        // The decision, box and direction of the last run with ink.
        let mut last: Option<(u32, Option<u32>, bool)> = None;
        let mut current: Option<usize> = None;
        for (id, part) in part_of.iter_mut().enumerate() {
            let d = decided.of_run[id];
            if d != decide::NONE && inked(id) {
                // The drawn box or the cell of a row of fields the run stands in: a part never
                // goes on from one into another.
                let b = box_of.get(&u32::try_from(id).unwrap_or(u32::MAX)).copied();
                let off = page.texts[id].off_axis;
                let key = (d, b, off);
                if last != Some(key) {
                    decision_of.push(d as usize);
                    current = Some(decision_of.len() - 1);
                    if off {
                        across.push(decision_of.len() - 1);
                    }
                }
                last = Some(key);
            }
            *part = current.unwrap_or(0);
        }
        // The parts with runs, in the order the flow first reads them: the source order, which
        // is the page's reading order.
        let source: Vec<layoutregions::RegionId> = (0..decision_of.len())
            .map(|p| layoutregions::RegionId(u32::try_from(p).unwrap_or(u32::MAX)))
            .collect();
        if decision_of.is_empty() && runs > 0 {
            // No run with ink of a decision: the runs go with the first decision that holds
            // one of them, or stand alone.
            let d = decided.of_run.iter().find(|&&d| d != decide::NONE);
            decision_of.push(d.map_or(n, |&d| d as usize));
        }
        // The parts of each decision; a decision without runs is one part.
        let mut parts_of: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (p, &d) in decision_of.iter().enumerate() {
            if let Some(list) = parts_of.get_mut(d) {
                list.push(p);
            }
        }
        for (d, list) in parts_of.iter_mut().enumerate() {
            if list.is_empty() {
                decision_of.push(d);
                list.push(decision_of.len() - 1);
            }
        }
        // The text of each part, by part.
        let mut texts: Vec<Vec<PrimId>> = vec![Vec::new(); decision_of.len()];
        for (id, &p) in part_of.iter().enumerate() {
            texts[p].push(PrimId::Text(u32::try_from(id).unwrap_or(u32::MAX)));
        }
        let nan = layoutregions::Bounds {
            x0: f32::NAN,
            y0: f32::NAN,
            x1: f32::NAN,
            y1: f32::NAN,
        };
        // The order reads the parts by their boxes, the source order, and the runs of each
        // part (where a picture stands in a column, see `layoutregions::order_by_source`): the
        // run lists move into the regions, nothing is copied.
        let regions: Vec<layoutregions::Region> = decision_of
            .iter()
            .enumerate()
            .map(|(p, &d)| {
                let decision = decided.decisions.get(d);
                // A decision of one part stands where its region stands; a part of several,
                // where its runs stand.
                let whole = parts_of.get(d).is_some_and(|l| l.len() == 1);
                let bounds = if whole {
                    decided.bounds(d)
                } else {
                    layoutregions::segment::bounds_of(page, &texts[p])
                };
                layoutregions::Region {
                    id: layoutregions::RegionId(u32::try_from(p).unwrap_or(u32::MAX)),
                    page: page.page,
                    // A band the engine takes for the running header may hold body text beside
                    // the page's title: it reads where it stands, as the body does.
                    kind: match decision.map_or(Kind::Spatial, |d| d.kind) {
                        Kind::HeaderBand => Kind::Paragraph,
                        k => k,
                    },
                    bounds: bounds.unwrap_or(nan),
                    prims: std::mem::take(&mut texts[p]),
                    certainty: decision.map_or(layoutregions::Certainty::Unsure, |d| d.certainty),
                    score: decision.map_or(layoutregions::Score(0), |d| d.score),
                    signals: Vec::new(),
                    alternatives: Vec::new(),
                    parent: None,
                    unsure: Vec::new(),
                    host: 0,
                }
            })
            .collect();
        // Text set across the page's direction of reading (a note up the margin) reads last, as
        // the text pass reads it after the page's main text: the order takes the other parts.
        let mut order = if across.is_empty() {
            layoutregions::order_by_source(page, &regions, &source)
        } else {
            let along: Vec<layoutregions::Region> = regions
                .iter()
                .enumerate()
                .filter(|(p, _)| across.binary_search(p).is_err())
                .map(|(_, r)| r.clone())
                .collect();
            layoutregions::order_by_source(page, &along, &source)
        };
        order.extend(
            across
                .iter()
                .map(|&p| layoutregions::RegionId(u32::try_from(p).unwrap_or(u32::MAX))),
        );
        let mut rank_of_part = vec![0u32; decision_of.len()];
        for (rank, id) in order.iter().enumerate() {
            if let Some(r) = rank_of_part.get_mut(id.0 as usize) {
                *r = u32::try_from(rank).unwrap_or(u32::MAX);
            }
        }
        let of_decision = parts_of
            .iter()
            .map(|l| l.iter().map(|&p| rank_of_part[p]).min().unwrap_or(0))
            .collect();
        Places {
            width: runs + 1,
            rank: part_of.iter().map(|&p| rank_of_part[p]).collect(),
            of_decision,
            by_rank: order.iter().map(|id| id.0 as usize).collect(),
            decision_of,
            part_of,
        }
    }

    /// The place in reading order of place `at` of the page's flow: in the part of the run
    /// that began the flow there (a run's place in the flow as it began is its id). A run a
    /// reader moves keeps its place (see [`Flow::at`]), so it reads where that reader put it;
    /// a table stands at the place its reader gave it.
    fn of(&self, at: usize) -> usize {
        let place = at.min(self.rank.len().saturating_sub(1));
        self.at_rank(self.rank.get(place).copied().unwrap_or(0), at)
    }

    /// The place in reading order of `at` (a place of the page's flow) in the part of rank
    /// `rank`.
    fn at_rank(&self, rank: u32, at: usize) -> usize {
        (rank as usize)
            .saturating_mul(self.width)
            .saturating_add(at.min(self.width - 1))
    }

    /// The first place in reading order of decision `d`'s region.
    fn start_of(&self, d: usize) -> usize {
        self.at_rank(self.of_decision.get(d).copied().unwrap_or(0), 0)
    }

    /// Where each picture or code of `media` (with the block it stands before and its
    /// decision) goes, with the blocks of the parts read just before and just after the first
    /// part of its region that hold a written run (`run_blocks`, by run id).
    fn media_at(
        &self,
        media: Vec<(layoutregions::PrimId, usize, usize)>,
        run_blocks: &[u32],
        body_end: usize,
    ) -> Vec<MediaAt> {
        if media.is_empty() {
            return Vec::new();
        }
        // The written runs by their place in reading order (a run's place in the flow as it
        // began is its id), and from each on the first block any of them was written in.
        let mut keyed: Vec<(usize, usize)> = run_blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| **b != u32::MAX)
            .map(|(id, &b)| (self.of(id), b as usize))
            .collect();
        keyed.sort_unstable();
        // A page that wrote no run gives no block to stand by: its pictures keep the place of
        // their nearest run, at the end of the page without one, as before.
        if keyed.is_empty() {
            return Vec::new();
        }
        let mut first_from = vec![body_end; keyed.len() + 1];
        for k in (0..keyed.len()).rev() {
            first_from[k] = first_from[k + 1].min(keyed[k].1);
        }
        // The block a place stands before: the first block written from it on.
        let block_at = |at: usize| first_from[keyed.partition_point(|k| k.0 < at)];
        // The first and past the last block of each part's written runs.
        let mut blocks: Vec<Option<(usize, usize)>> = vec![None; self.decision_of.len()];
        for (&p, &b) in self.part_of.iter().zip(run_blocks) {
            if b == u32::MAX {
                continue;
            }
            if let Some(slot) = blocks.get_mut(p) {
                let b = b as usize;
                *slot = Some(slot.map_or((b, b + 1), |(lo, hi)| (lo.min(b), hi.max(b + 1))));
            }
        }
        let written: Vec<Option<(usize, usize)>> = self
            .by_rank
            .iter()
            .map(|&p| blocks.get(p).copied().flatten())
            .collect();
        // For each rank, the blocks of the nearest rank before it and after it that has some.
        let mut before: Vec<Option<(usize, usize)>> = vec![None; written.len()];
        let mut after: Vec<Option<(usize, usize)>> = vec![None; written.len()];
        for r in 1..written.len() {
            before[r] = written[r - 1].or(before[r - 1]);
        }
        for r in (0..written.len().saturating_sub(1)).rev() {
            after[r] = written[r + 1].or(after[r + 1]);
        }
        media
            .into_iter()
            .map(|(prim, at, d)| {
                let at = block_at(at);
                let rank = self.of_decision.get(d).copied().unwrap_or(0) as usize;
                let before = before
                    .get(rank)
                    .copied()
                    .flatten()
                    .map_or(at, |b| b.0.min(at));
                let after = after
                    .get(rank)
                    .copied()
                    .flatten()
                    .map_or(at, |b| b.1.max(at));
                MediaAt {
                    prim,
                    at,
                    near: before..after,
                }
            })
            .collect()
    }
}

/// A run taken out of a [`Flow`], with its slot, its place and its id.
pub(crate) struct Taken {
    pub run: Run,
    pub slot: Slot,
    pub at: usize,
    pub id: u32,
}

/// What a page draws besides text, for tables without tags, and its size.
pub(crate) struct Drawn<'a> {
    pub rules: &'a [Rule],
    /// The grids `rules` make, in the frame of the runs, found once when the page is laid out
    /// (see `regions::structure_in_frame`), and the boxes the runs were read in. Every
    /// detector that looks at drawn lines uses them.
    pub structure: &'a DrawnStructure,
    pub images: &'a [[f32; 4]],
    /// Boxes of the form fields on the page.
    pub widgets: &'a [[f32; 4]],
    /// The drawn boxes the page's runs are read in as reading regions.
    pub boxes: &'a [[f32; 4]],
    /// What the text pass found on the page besides its runs (taken out of it), for the
    /// region engine's input; `None` where a page is laid out from runs alone.
    pub items: Option<&'a crate::text::PageItems>,
    /// The text pass found the page's lines too many to search for grids, and said so.
    pub lines_unsearched: bool,
    pub width: f32,
    /// The most common font size of the file's text.
    pub body_size: f32,
}

impl Drawn<'_> {
    /// The grids of the page's rules, or why they were not searched.
    pub(crate) fn grids(&self) -> Result<&Grids, Unsearched> {
        self.structure.grids.as_ref().map_err(|e| *e)
    }
}

/// [`page_blocks_at`] without points to find.
#[cfg(test)]
pub(crate) fn page_blocks(
    page: u32,
    height: f32,
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
    height: f32,
    runs: Vec<Run>,
    drawn: &Drawn<'_>,
    cx: &Context<'_>,
    points: &[(f32, f32)],
) -> Laid {
    let stop = || time_over(cx.budget).is_err();
    page_blocks_until(page, height, runs, drawn, cx, points, &stop)
}

/// [`page_blocks_at`] with `stop` asked by the segmentation and the extractors: what a stop
/// leaves is said by a limit note.
///
/// The page is read in one pass of the region engine: the tables of the tag tree first (the
/// file states them), then the regions the page states (its tag tree's blocks, its form
/// fields, and the columns of totals beside ruled tables that the pre-pass rule reads),
/// then segmentation, scored typing and one arbitration (see [`decide::decide`]). Each
/// decision is read by the extractor of its kind (see [`decide::Class`]), and the parts stand
/// in the page's order of runs, each part where its first run stands, running
/// header lines too; footnotes and footers last.
fn page_blocks_until(
    page: u32,
    height: f32,
    runs: Vec<Run>,
    drawn: &Drawn<'_>,
    cx: &Context<'_>,
    points: &[(f32, f32)],
    stop: &dyn Fn() -> bool,
) -> Laid {
    // The page's text, the measure the work of its detectors is held to.
    crate::work::add(runs.len() as u64);
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
        return built.laid(Vec::new(), no_regions(page));
    }
    let mut flow = match cx.tree {
        Some(tree) => {
            let (mut runs, mut slots) = classified(runs, tree, cx);
            tagged_columns(&mut runs, &mut slots);
            Flow::new(runs, slots)
        }
        None => {
            let n = runs.len();
            Flow::new(runs, vec![Slot::Free; n])
        }
    };
    // The page as the region engine takes it: `TextRun` i is run i of the flow, whose id is i
    // (see `engine::page_in`). A page laid out from runs alone has its runs only.
    let size = [drawn.width, height];
    let mut engine_in = match drawn.items {
        Some(items) => engine::page_in(items, &flow.runs, drawn, size),
        None => {
            let mut p =
                engine::page_in(&crate::text::PageItems::default(), &flow.runs, drawn, size);
            p.page = page;
            p
        }
    };
    let slots = flow.slots.clone();
    let mut notes: Vec<(String, LossReason, Severity)> = Vec::new();
    let mut placed: Vec<Placed> = Vec::new();
    // The tables of the tag tree: the file states them. They are taken before the running
    // headers and footers are looked for: the column titles a long table repeats at the top of
    // each page are table text.
    let mut tag_tables: HashSet<usize> = HashSet::new();
    // Whether the pre-pass readers ran on the drawn structure: not on a tagged page whose tag tree
    // holds.
    let mut geometry = cx.tree.is_none();
    if let Some(tree) = cx.tree {
        let (found, fallbacks) =
            tables::tagged(page, &mut flow, tree, cx.tables, cx.in_part, &|| {
                time_over(cx.budget).is_err()
            });
        for what in cx.tables.take_issues() {
            notes.push((what, LossReason::Damaged, Severity::Warning));
        }
        for what in cx.tables.take_glued() {
            notes.push((what, LossReason::Uncertain, Severity::Info));
        }
        geometry = !fallbacks.is_empty();
        notes.extend(
            fallbacks
                .into_iter()
                .map(|w| (w, LossReason::Uncertain, Severity::Info)),
        );
        tag_tables.extend(found.iter().filter_map(|t| t.tagged));
        placed.extend(found.into_iter().map(Placed::Table));
    }
    // The drawn box each free run stands in, as the pre-pass box reader puts it (see
    // `regions::regions_of`): the text of a box is laid out on its own, so a paragraph never goes
    // on from one box into the next, nor into text outside its box.
    let mut box_of: HashMap<u32, u32> = {
        let points: Vec<(f32, f32, f32)> = flow
            .runs
            .iter()
            .map(|r| {
                if r.off_axis {
                    (f32::NAN, f32::NAN, 0.0)
                } else {
                    (r.x + r.w.max(0.0) / 2.0, r.y + 0.25 * r.size, r.size)
                }
            })
            .collect();
        match crate::regions::regions_of(&points, drawn.boxes) {
            Ok(regions) if !drawn.boxes.is_empty() => regions
                .iter()
                .zip(&flow.ids)
                .zip(&flow.slots)
                .filter_map(|((b, &id), slot)| b.filter(|_| *slot == Slot::Free).map(|b| (id, b)))
                .collect(),
            _ => HashMap::new(),
        }
    };
    // The regions the page states: the tag tree's blocks and tables, and the form fields.
    let mut given = engine::given_regions(&engine_in, &slots, &|t| tag_tables.contains(&t));
    // The pre-pass reading of the page, once (see [`today`]): the tables of the classes kept as
    // pre-passes are given regions of the reader's kind. Runs a given region already holds are
    // left to it, and a table that shares one is not given.
    let held: HashSet<u32> = given
        .iter()
        .flat_map(|g| g.prims.iter())
        .filter_map(|p| match *p {
            layoutregions::PrimId::Text(i) => Some(i),
            _ => None,
        })
        .collect();
    let reading = today::read(
        page,
        height,
        &flow,
        &engine_in,
        drawn,
        cx,
        geometry,
        &|| time_over(cx.budget).is_err(),
    );
    let today::Reading {
        tables: today_tables,
        titles: today_titles,
        left: today_left,
        notes: today_notes,
        unsure: today_unsure,
        doubts: today_doubts,
        issues: today_issues,
        cells: today_cells,
        own_lines: today_own_lines,
    } = reading;
    // The runs of blocks side by side that a wrapped paragraph shows: their lines stay lines.
    let mut own_lines: HashSet<u32> = today_own_lines.into_iter().collect();
    // Each cell of a row of fields the ruled pass read apart is a block of its own, as a drawn
    // box is (keys past the boxes' indices): no paragraph goes on from one cell into another.
    let first_cell = u32::try_from(drawn.boxes.len()).unwrap_or(u32::MAX);
    for (n, cell) in today_cells.iter().enumerate() {
        let key = first_cell.saturating_add(u32::try_from(n).unwrap_or(u32::MAX));
        for &id in cell {
            box_of.insert(id, key);
        }
    }
    // The parts of the pre-pass reading given as regions, and the part of each run: the first given
    // region that lists a run holds it.
    let mut pre_parts: Vec<today::Part> = Vec::new();
    let mut pre_of_run: HashMap<u32, usize> = HashMap::new();
    let mut give = |given: &mut Vec<layoutregions::GivenRegion>,
                    kind: Kind,
                    ids: &[u32],
                    part: today::Part| {
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let k = pre_parts.len();
        for &i in &ids {
            pre_of_run.entry(i).or_insert(k);
        }
        given.push(layoutregions::GivenRegion {
            kind,
            prims: ids.into_iter().map(layoutregions::PrimId::Text).collect(),
            order: u32::try_from(given.len()).unwrap_or(u32::MAX),
            level: None,
            role: None,
            group: None,
        });
        pre_parts.push(part);
    };
    let free = |ids: &[u32]| ids.iter().all(|i| !held.contains(i));
    // In the order the pre-pass readers made them: the titles of a form page and the box pairs come
    // before the tables without tags (`today::read`).
    for (_, t) in today_titles {
        if free(&t.ids) {
            let ids = t.ids.clone();
            give(&mut given, Kind::Heading, &ids, today::Part::Title(Some(t)));
        }
    }
    for t in today_tables {
        if free(&t.ids) {
            let ids = t.ids.clone();
            give(
                &mut given,
                t.by.region_kind(t.kind),
                &ids,
                today::Part::Table(Some(t)),
            );
        }
    }
    let mut pre_notes: Vec<(String, LossReason, Severity)> = Vec::new();
    let mut pre_unsure: Vec<Unsure> = Vec::new();
    for (_, w, why) in today_notes {
        pre_notes.push((w, why, Severity::Info));
    }
    for (_, u) in today_unsure {
        pre_unsure.push(u);
    }
    // The text a family kept refused: text where it stands, with why; no typer reads it. It is
    // given as a paragraph so that the walk writes it, and decided unsure (`decide::decide`).
    let mut pre_doubts: tables::Doubts = Vec::new();
    for (f, ids, why) in today_doubts {
        let ids: Vec<u32> = ids.into_iter().filter(|i| !held.contains(i)).collect();
        if !ids.is_empty() {
            give(
                &mut given,
                Kind::Paragraph,
                &ids,
                today::Part::Refused(f, why),
            );
            pre_doubts.push((ids, why));
        }
    }
    // The drawn box each table of the pre-pass reading was read by: its region reaches over it.
    let pre_drawn: Vec<Option<layoutregions::Bounds>> = pre_parts
        .iter()
        .map(|p| match p {
            today::Part::Table(Some(t)) => t.drawn.map(engine::bounds),
            _ => None,
        })
        .collect();
    let pre_signal = |k: usize| -> (layoutregions::Signal, bool) {
        let (id, detail, text) = match &pre_parts[k] {
            today::Part::Table(t) => (
                t.as_ref().map_or("", |t| t.by.id()),
                t.as_ref()
                    .map_or("", |t| t.found_by.unwrap_or(t.by.found_from())),
                false,
            ),
            today::Part::Title(_) => (
                "pdf.underlined_title",
                "a title over the box or the bar it names",
                false,
            ),
            today::Part::Refused(f, why) => (today::refused_id(*f), *why, true),
        };
        (
            layoutregions::Signal {
                id,
                detail: detail.to_string(),
                // A refusal weighs nothing: its region is unsure.
                weight: if text {
                    0
                } else {
                    layoutregions::typers::MAX_SCORE
                },
            },
            text,
        )
    };
    engine_in.given = given;
    let limits = rezzer_layout::pdf::engine_limits(cx.budget);
    let mut decided = decide::decide(
        &engine_in,
        &drawn.structure.grids,
        (&cx.style.facts, &cx.style.index),
        &ends_in_amount,
        (&pre_of_run, &pre_signal),
        &limits,
        stop,
    );
    // Pictures a bound left in a region of text are a figure region of their own (see
    // `decide::part_pictures`): the page says so, without making the read partial.
    for d in &decided.decisions {
        if let Some(s) = d.signals.iter().find(|s| s.id == decide::PICTURES_UNPARTED) {
            notes.push((
                format!("on page {page}, {}", s.detail),
                LossReason::Uncertain,
                Severity::Info,
            ));
        }
    }
    // A tagged table whose glued columns were cut (see `tables::tagged`) is no exact reading
    // of the tag tree: its region is the rule's, and says why.
    if cx.tree.is_some() {
        for k in 0..decided.decisions.len() {
            if decided.source(k) != layoutregions::segment::Source::Given(Kind::Table) {
                continue;
            }
            let table = decided.prims(k).iter().find_map(|p| match *p {
                layoutregions::PrimId::Text(i) => match slots.get(i as usize) {
                    Some(Slot::Cell { table, .. }) => Some(*table),
                    _ => None,
                },
                _ => None,
            });
            if table.is_some_and(|t| cx.tables.is_cut(t)) {
                let d = &mut decided.decisions[k];
                d.certainty = layoutregions::Certainty::Rule;
                d.signals.push(layoutregions::Signal {
                    id: "pdf.tagged_table.cut",
                    detail: "its tagged cells hold the text of several columns that line up \
                             over its rows: they are cut into those columns"
                        .to_string(),
                    weight: 0,
                });
            }
        }
    }
    let tagged_page = cx.tree.is_some();
    // Each run to the extractor of its decision's kind.
    // The footnote legend of an untagged page (see `footnotes::take`), as the pre-pass reader takes
    // it: from the page's whole flow, before its runs go to their extractors, so a raised
    // reference to a note is marked wherever it stands (in a table's cell too) and every line
    // of the legend is its own (a segment may cut a raised marker off its note, or a column of
    // the legend off the others). A tagged file says its notes. Lines of the legend it does not
    // read stay text.
    let legend = if tagged_page {
        // A tagged page's legend: entries the tags give as list items or paragraphs that open
        // with a raised number a cell of its tagged tables refers to.
        let cells = placed.iter_mut().flat_map(|p| match p {
            Placed::Table(t) if t.tagged.is_some() => t.rows.iter_mut().flatten().collect(),
            _ => Vec::new(),
        });
        footnotes::tagged_legend(page, &mut flow, cells)
    } else {
        footnotes::take(page, &mut flow, cx.style.legends.get(&page))
    };
    let mut legend = legend;
    if let Some(why) = legend.refused.take() {
        notes.push((why, LossReason::Uncertain, Severity::Info));
    }
    // The page's reading order, set once: the engine's order of its regions (see
    // [`Places`]). Every part found so far moves to its place in it; the parts found from
    // here on take the places of their runs. (The legend is found by the places of the
    // page's flow, before.)
    let reading = Places::new(&engine_in, &decided, &box_of);
    let flow = flow.read_in(&reading);
    for p in &mut placed {
        if let Placed::Table(t) = p {
            t.at = reading.of(t.at);
        }
    }
    for part in &mut pre_parts {
        match part {
            today::Part::Table(Some(t)) => t.at = reading.of(t.at),
            today::Part::Title(Some(t)) => t.at = reading.of(t.at),
            _ => {}
        }
    }
    for u in &mut pre_unsure {
        u.at = reading.of(u.at);
    }
    // The link and footnote label of each run of the page that has one, by its id, for text
    // kept as laid out from the page's runs (`engine_in` holds their text and places only).
    let run_marks: HashMap<u32, (Option<Rc<UriTarget>>, Option<String>)> = flow
        .runs
        .iter()
        .zip(&flow.ids)
        .filter(|(r, _)| r.uri.is_some() || r.note)
        .map(|(r, &id)| {
            (
                id,
                (r.uri.clone(), r.note.then(|| r.text.trim().to_string())),
            )
        })
        .collect();
    let markers = legend.markers();
    let classes: Vec<Class> = flow
        .ids
        .iter()
        .map(|&id| decided.class_of_run(id, tagged_page))
        .collect();
    let [
        text_flow,
        header_flow,
        footer_flow,
        legend_flow,
        mut table_flow,
        spatial_flow,
        _,
    ] = flow.parted(&classes);
    // The running headers, one paragraph a line, each placed by its first run in the page's
    // flow like every other part: a band the engine finds may hold body text
    // beside the page's title, which reads where the flow has it.
    let header_lines = lines(&header_flow.runs);
    let mut header_runs = 0;
    for (k, l) in header_lines.iter().enumerate() {
        placed.push(Placed::Header {
            line: k,
            at: header_flow.at[header_runs],
            top: l.y,
        });
        header_runs += l.runs.len();
    }
    let text_flow = Flow::merged(text_flow, legend_flow);
    // The text sizes of the page: all its text but its bands and its legend (the values of a
    // form are its data lines), and a footer of a file of two pages.
    let text_sizes = {
        let two_page_lines = lines(&footer_flow.runs);
        let two_page = two_page_lines
            .iter()
            .filter(|l| cx.style.two_page_line(l, height))
            .flat_map(|l| l.runs.iter());
        file::text_sizes(
            text_flow
                .runs
                .iter()
                .chain(&table_flow.runs)
                .chain(&spatial_flow.runs)
                .chain(two_page),
        )
    };
    // The tables, pairs and totals: each decision read on its own area (its runs alone) by the
    // reader its winning evidence names (see [`area_reader`]). A pairs reading that pairs a
    // label with a value across a line of another region is refused: its area is kept as laid
    // out.
    let mut doubts: tables::Doubts = pre_doubts;
    placed.extend(pre_unsure.into_iter().map(Placed::Unsure));
    notes.extend(pre_notes);
    // What a bound cut short in the segments the pre-pass readers read by is the page's too.
    let mut segment_issues = decided.segs.issues.clone();
    for i in today_issues {
        if !segment_issues
            .iter()
            .any(|j| j.kind == i.kind && j.what == i.what)
        {
            segment_issues.push(i);
        }
    }
    let mut refused_areas: BTreeMap<u32, (usize, f32, Vec<u32>, String)> = BTreeMap::new();
    // What the readers left of each area, merged once after the loop.
    let mut table_left: Vec<Flow> = Vec::new();
    // The runs of the areas refused by the crossing rule, whole: they are kept as laid out, and
    // the dates and facts among them are read as in any text kept so. Merged once after the
    // loop.
    let mut refused_text: Vec<Flow> = Vec::new();
    let across = Across::new(&engine_in, &decided.of_run);
    for (d, mut area) in by_decision(std::mem::take(&mut table_flow), &decided.of_run) {
        let decision = &decided.decisions[d as usize];
        let ids = area.ids.clone();
        let whole = area.copied();
        let (at, top) = (
            area.at.first().copied().unwrap_or(0),
            area.runs.iter().map(|r| r.y).fold(f32::MIN, f32::max),
        );
        let mut found: Vec<Planned> = Vec::new();
        let mut titles: Vec<tables::BoxTitle> = Vec::new();
        let mut unsure: Vec<Unsure> = Vec::new();
        let mut said: Vec<(String, LossReason)> = Vec::new();
        let pre = decided.pre[d as usize];
        match area_reader(decision) {
            // A table of the pre-pass reading, given before the page was typed (see [`today`]):
            // placed as its reader made it. The pieces its reader left of a run it cut stay
            // text, as today; the runs of the area are the table's.
            _ if pre.is_some() => {
                match pre.and_then(|(k, _)| pre_parts.get_mut(k)) {
                    Some(today::Part::Table(t)) => found.extend(t.take()),
                    Some(today::Part::Title(t)) => titles.extend(t.take()),
                    _ => {}
                }
                area = Flow::default();
            }
            AreaReader::Boxes => {
                let (t, b, (u, box_doubts), skipped) = tables::box_pairs(
                    page,
                    &mut area,
                    drawn.boxes,
                    drawn.grids(),
                    &text_sizes,
                    stop,
                );
                found.extend(t);
                titles.extend(b);
                said.extend(u.into_iter().map(|w| (w, LossReason::Uncertain)));
                doubts.extend(box_doubts);
                said.extend(skipped.map(|w| (w, LossReason::Limit)));
            }
            AreaReader::Lines => found.extend(line_pairs(&mut area).map(|mut t| {
                // A lone total read as a pair is a row of its totals region.
                t.kind = decision.kind;
                t
            })),
            AreaReader::Cell => found.extend(label_over_value(&mut area)),
            AreaReader::Tables => {
                let f = tables::untagged(page, &mut area, drawn, &decided.segs, height, stop);
                found.extend(f.tables);
                unsure.extend(f.unsure);
                doubts.extend(f.doubts);
                own_lines.extend(f.own_lines);
                // A limit of the page as a whole (the shapes it keeps) is said once, however
                // many table areas of the page say it. What a table says of itself is kept
                // for each table, also in the same words: two tables that start alike with
                // icons in the same cells have a loss each.
                for w in f.skipped {
                    if !said
                        .iter()
                        .any(|(s, why)| *why == LossReason::Limit && *s == w)
                    {
                        said.push((w, LossReason::Limit));
                    }
                }
                for (list, why) in [
                    (f.uncertain, LossReason::Uncertain),
                    (refused_quietly(page, f.quiet), LossReason::Uncertain),
                ] {
                    said.extend(list.into_iter().map(|w| (w, why)));
                }
            }
        }
        let pairs = decision.kind == Kind::LabelValue;
        if found
            .iter()
            .any(|t| (pairs || t.is_pairs()) && across.crossed(t, d))
        {
            refused_areas.insert(
                d,
                (
                    at,
                    top,
                    ids,
                    decide::unread_why(
                        decision.kind,
                        Some("its labels and values stand on either side of a line of another region"),
                    ),
                ),
            );
            refused_text.push(whole);
            continue;
        }
        placed.extend(found.into_iter().map(Placed::Table));
        placed.extend(titles.into_iter().map(Placed::Title));
        placed.extend(unsure.into_iter().map(Placed::Unsure));
        notes.extend(said.into_iter().map(|(w, why)| (w, why, Severity::Info)));
        table_left.push(area);
    }
    if across.spent.get() {
        notes.push((
            format!(
                "labels and values paired across a line of another region on page {page} are \
                 looked for only in part: the page has too many lines between them; the pairs \
                 of the tables past that are kept as read"
            ),
            LossReason::Limit,
            Severity::Info,
        ));
    }
    let refused_text = Flow::all_merged(refused_text);
    let mut table_flow = Flow::all_merged(table_left);
    // The pieces the pre-pass readers left of the runs they cut, where a pre-pass table holds the
    // run: they are text, as before.
    // The text the pre-pass readers refused is laid out from their copy of the flow: a
    // refused row of fields was put there cell by cell, each cell on lines of its own.
    let refused_run = |id: u32| {
        decided.of_run.get(id as usize).is_some_and(|&d| {
            d != decide::NONE && matches!(decided.pre[d as usize], Some((_, true)))
        })
    };
    let pre_left = {
        let mut left = today_left.read_in(&reading);
        let ids = left.ids.clone();
        let cut: HashSet<u32> = placed
            .iter()
            .filter_map(|p| match p {
                Placed::Table(t) => Some(t),
                _ => None,
            })
            .flat_map(|t| t.ids.iter().copied())
            .filter(|i| pre_of_run.contains_key(i))
            .collect();
        left.take(|i, _| !cut.contains(&ids[i]) && !refused_run(ids[i]));
        left
    };
    // The refused runs the copy holds: they are laid out from it, in place of the page's own.
    let from_copy: HashSet<u32> = pre_left
        .ids
        .iter()
        .copied()
        .filter(|&i| refused_run(i))
        .collect();
    let mut text_flow = text_flow;
    let text_ids = text_flow.ids.clone();
    text_flow.take(|i, _| from_copy.contains(&text_ids[i]));
    let text_flow = Flow::merged(text_flow, pre_left);
    // What the table readers left of a decision is kept as laid out, with their reason.
    let doubt_of: HashMap<u32, &str> = doubts
        .iter()
        .flat_map(|(ids, why)| ids.iter().map(move |&i| (i, *why)))
        .collect();
    // A decision's runs kept as laid out are one part as long as no text or other part of the
    // page stands between them in the page's flow; such a thing between them starts a new part, so each part
    // stands where its first run stands and the text reads in the flow's order.
    // A run without ink stands in the flow only, not on the page (spaces a form sets after a
    // field): it parts nothing.
    let mut placed_at: Vec<usize> = placed.iter().map(|p| p.at().0).collect();
    placed_at.sort_unstable();
    let text_at: Vec<usize> = layoutregions::line::with_ink(
        text_flow
            .runs
            .iter()
            .zip(&text_flow.at)
            .map(|(r, &a)| (r.text.as_str(), a)),
    );
    let text_between = |a: usize, b: usize| {
        [&text_at, &placed_at].into_iter().any(|at| {
            let i = at.partition_point(|x| *x <= a);
            at.get(i).is_some_and(|x| *x < b)
        })
    };
    // The runs with ink of `spatial_flow` before each of its places.
    let inked_before =
        layoutregions::line::inked_before(spatial_flow.runs.iter().map(|r| r.text.as_str()));
    let mut spatial_parts: BTreeMap<(u32, u32), SpatialPart> = BTreeMap::new();
    // By decision: its part, the place of its last run and that run's index in `spatial_flow`
    // (a run of another decision between them stands between them too).
    let mut last_of: HashMap<u32, (u32, usize, usize)> = HashMap::new();
    for (k, (&id, &at)) in spatial_flow.ids.iter().zip(&spatial_flow.at).enumerate() {
        let d = decided.of_run[id as usize];
        let seq = match last_of.get(&d) {
            Some(&(seq, last, j))
                if inked_before[k] > inked_before[j + 1] || text_between(last, at) =>
            {
                seq + 1
            }
            Some(&(seq, ..)) => seq,
            None => 0,
        };
        last_of.insert(d, (seq, at, k));
        let part = spatial_parts
            .entry((d, seq))
            .or_insert_with(|| SpatialPart {
                at,
                top: f32::MIN,
                ids: Vec::new(),
                pieces: Vec::new(),
                why: if decided.untyped {
                    "the time limit was reached before its region type was looked for".to_string()
                } else if decided.limit[d as usize] {
                    "a bound of the segmentation, or the time limit, left it whole before it \
                     was parted, so its region type was not looked for in full"
                        .to_string()
                } else {
                    decide::unsure_why(decided.candidates(d as usize), &decided.margins)
                },
                limit: decided.untyped || decided.limit[d as usize],
            });
        part.ids.push(id);
        part.top = part.top.max(spatial_flow.runs[k].y);
    }
    for (d, (at, top, ids, why)) in refused_areas {
        spatial_parts.insert(
            (d, 0),
            SpatialPart {
                at,
                top,
                ids,
                pieces: Vec::new(),
                why,
                limit: false,
            },
        );
    }
    // What a reader left of one decision is one part as long as no text and no other part of
    // the page stands between its runs in the page's flow; such a thing between them starts a
    // new part: a heading over a table and a line under it, both left by the table's reader,
    // are written each where it stands, not as one block before the table. Unlike the runs
    // kept as laid out above, runs kept as laid out of another decision part nothing here, on
    // purpose: they would cut a column of amounts a totals reader left into a block a line.
    // The parts, in the flow's order, each with its decision; by decision, the part its last
    // piece went to and that piece's place.
    let mut left_parts: Vec<(u32, Flow)> = Vec::new();
    let mut last_left: HashMap<u32, (usize, usize)> = HashMap::new();
    let table_left = std::mem::take(&mut table_flow);
    let places = table_left.at.into_iter().zip(table_left.ids);
    for ((r, s), (at, id)) in table_left
        .runs
        .into_iter()
        .zip(table_left.slots)
        .zip(places)
    {
        let d = decided.of_run[id as usize];
        let k = match last_left.get(&d) {
            Some(&(k, last)) if !text_between(last, at) => k,
            _ => {
                left_parts.push((d, Flow::default()));
                left_parts.len() - 1
            }
        };
        last_left.insert(d, (k, at));
        let f = &mut left_parts[k].1;
        f.runs.push(r);
        f.slots.push(s);
        f.at.push(at);
        f.ids.push(id);
    }
    // A part a table's reader left that is one line of whole runs (no piece of a run in a
    // table, no text the reader refused) and stands after a table of its decision in the
    // flow is a pair when the line pair rule parts it as a lone line, as the table's reader
    // ended the table over it (`pairs_end`): a line `Label: value` under a table is a pair of
    // its own. A line over the table (a heading `Step 2: Pay the fee`) stays as it is.
    let in_tables: HashSet<u32> = placed
        .iter()
        .filter_map(|p| match p {
            Placed::Table(t) => Some(t.ids.iter().copied()),
            _ => None,
        })
        .flatten()
        .collect();
    // By decision: the place of the first run of its last table.
    let mut table_at: HashMap<u32, usize> = HashMap::new();
    for p in &placed {
        if let Placed::Table(t) = p
            && let Some(d) = majority(t.ids.iter().map(|&i| decided.of_run[i as usize]))
        {
            let at = table_at.entry(d).or_insert(t.at);
            *at = (*at).max(t.at);
        }
    }
    // The runs of each such pair, for the region it is (see `decide::agree`).
    let mut left_pairs: Vec<Vec<u32>> = Vec::new();
    for (d, part) in &mut left_parts {
        let reader = area_reader(&decided.decisions[*d as usize]);
        let whole = part
            .ids
            .iter()
            .all(|i| !in_tables.contains(i) && !doubt_of.contains_key(i));
        let under = part
            .at
            .first()
            .is_some_and(|a| table_at.get(d).is_some_and(|t| a > t));
        if decided.pre[*d as usize].is_none()
            && reader == AreaReader::Tables
            && under
            && whole
            && one_line(part)
            && let Some(t) = line_pairs(part)
        {
            left_pairs.push(t.ids.clone());
            placed.push(Placed::Table(t));
        }
    }
    // Their runs as the readers left them: a run cut at a field edge or a column line may have
    // a piece in a table and a piece left, and only the piece left is kept here.
    let table_rest = Flow::all_merged(left_parts.iter().map(|(_, f)| f.copied()).collect());
    let mut first_left: HashSet<u32> = HashSet::new();
    for (d, part) in left_parts {
        if part.runs.is_empty() {
            continue;
        }
        // The first part of a decision is its part kept as laid out; a later one is a part of
        // its own.
        let seq = if first_left.insert(d) {
            0
        } else {
            spatial_parts
                .range((d, 0)..=(d, u32::MAX))
                .next_back()
                .map_or(0, |(&(_, s), _)| s + 1)
        };
        for ((r, id), at) in part.runs.into_iter().zip(part.ids).zip(part.at) {
            let sp = spatial_parts
                .entry((d, seq))
                .or_insert_with(|| SpatialPart {
                    at,
                    top: f32::MIN,
                    ids: Vec::new(),
                    pieces: Vec::new(),
                    why: decide::unread_why(
                        decided.decisions[d as usize].kind,
                        doubt_of.get(&id).copied(),
                    ),
                    limit: false,
                });
            sp.at = sp.at.min(at);
            sp.top = sp.top.max(r.y);
            // The pieces of a run the readers cut share its id: the ids are made unique below.
            sp.ids.push(id);
            sp.pieces.push(r);
        }
    }
    // Which decision each placed table stands for, for its note: the decision of most of its
    // runs. A column of totals beside a ruled section keeps its note on the section it joins.
    for p in &mut placed {
        if let Placed::Table(t) = p
            && t.tagged.is_none()
        {
            let by_decision = majority(t.ids.iter().map(|&i| decided.of_run[i as usize]));
            // A table of the pre-pass reading keeps its reader's note.
            if by_decision.is_some_and(|d| decided.pre[d as usize].is_some()) {
                continue;
            }
            let from =
                by_decision.and_then(|d| found::found_by_text(&decided.decisions[d as usize]));
            let side = t.found_by == Some(found::SIDE_FOUND_BY);
            t.found_by = from.or(side.then_some(found::SIDE_FOUND_BY));
        }
    }
    placed.extend(
        spatial_parts
            .into_iter()
            .map(|(d, mut part)| {
                part.ids.sort_unstable();
                part.ids.dedup();
                (d, part)
            })
            .map(|(_, part)| Placed::Spatial(part)),
    );
    // The pictures and codes of each figure or code region, where its region starts.
    for k in 0..decided.decisions.len() {
        if !matches!(decided.decisions[k].kind, Kind::Figure | Kind::Code) {
            continue;
        }
        let prims: Vec<layoutregions::PrimId> = decided
            .prims(k)
            .iter()
            .copied()
            .filter(|p| {
                matches!(
                    p,
                    layoutregions::PrimId::Picture(_) | layoutregions::PrimId::Code(_)
                )
            })
            .collect();
        if !prims.is_empty() {
            placed.push(Placed::Media {
                at: reading.start_of(k),
                decision: k,
                prims,
            });
        }
    }
    // The parts in reading order (a picture before the text of its own region, which starts at
    // the same place).
    let mut sorted: Vec<Placed> = placed;
    sorted.sort_by(|a, b| {
        let (la, ta) = a.at();
        let (lb, tb) = b.at();
        la.cmp(&lb).then(tb.total_cmp(&ta))
    });
    let placed = join_pair_rows(sorted, &text_flow, drawn);
    // The text the page reads as text, without the text kept as laid out: the title of a
    // page is read from text the layout is sure of (a word the engine left unsure is no
    // title, however large).
    let title_text = Flow::merged(
        text_flow.copied(),
        Flow::merged(table_rest.copied(), refused_text.copied()),
    );
    // The text the page reads as text, the text kept as laid out among it (dates there are
    // dates too).
    let read_text = Flow::merged(
        Flow::merged(text_flow.copied(), spatial_flow.copied()),
        Flow::merged(table_rest, refused_text),
    );
    // The title and the labelled dates of the document: the lines of page 1 as the layout
    // lays them out as text, and the pairs it read.
    let (title, dates) = match page {
        1 => {
            let text = lines(&title_text.runs);
            let title =
                head::page_title(&text, &cx.style.levels(&text_sizes), height).map(|t| t.text);
            // Every table of the page, its rows as the text of their cells.
            let tables: Vec<head::Table> = placed
                .iter()
                .filter_map(|p| match p {
                    Placed::Table(t) => {
                        // A table the line pairs rule read (see [`area_reader`]), or a line a
                        // table's reader left that the rule parted, is lines of text.
                        let lines = t.by == tables::Detector::LinePairs
                            || majority(t.ids.iter().map(|&i| decided.of_run[i as usize]))
                                .is_some_and(|d| {
                                    area_reader(&decided.decisions[d as usize]) == AreaReader::Lines
                                });
                        Some(head_table(t, lines))
                    }
                    _ => None,
                })
                .collect();
            (
                title,
                Some(head::laid_dates(&read_text.runs, &tables, height)),
            )
        }
        _ => (None, None),
    };
    drop(read_text);
    drop(title_text);
    // The text regions with ink on more than one line of the page's text: the blocks of
    // several lines the engine kept whole (see `layoutregions::part_lines`).
    let mut first_line: HashMap<u32, u32> = HashMap::new();
    let mut several: HashSet<u32> = HashSet::new();
    for (r, &id) in text_flow.runs.iter().zip(&text_flow.ids) {
        if r.text.trim().is_empty() {
            continue;
        }
        if let Some(k) = decided.text_region_of_run(id)
            && *first_line.entry(k).or_insert(r.line) != r.line
        {
            several.insert(k);
        }
    }
    let keys = RunKeys {
        kind_of: &|id| (decided.kind_of_run(id), box_of.get(&id).copied()),
        region_of: &|id| decided.text_region_of_run(id),
        grid_of: &|id| decided.given_of_run(id),
        several: &|k| several.contains(&k),
        own_line: &|id| own_lines.contains(&id),
        rules: &engine_in.rules,
    };
    let lay = |flow: &Flow, range: Range<usize>, built: &mut Built| {
        lay_text(flow, range, &keys, &text_sizes, cx, built);
    };
    let mut start = 0;
    // What each placed part became, and the runs it holds.
    let mut became: Vec<(Became, Vec<u32>)> = Vec::with_capacity(placed.len());
    let mut spatial_whys: Vec<(usize, String, bool)> = Vec::new();
    // Where each block kept as laid out stands on the page, by its place in the blocks.
    let mut spatial_frames: Vec<(usize, SpatialFrame)> = Vec::new();
    // Why runs were written as they were, where the writing says it (see `decide::agree`):
    // the reason of each block kept as laid out, by its runs.
    let mut whys: Vec<String> = Vec::new();
    let mut why_of: HashMap<u32, usize> = HashMap::new();
    for p in placed {
        let (at, _) = p.at();
        // A picture's place is found once the page is written (see `Places::media_at`): it
        // parts no text, so a paragraph or list beside a figure is not cut at it.
        if let Placed::Media {
            prims, decision, ..
        } = &p
        {
            built
                .media_at
                .extend(prims.iter().map(|&m| (m, at, *decision)));
            continue;
        }
        let end = start + text_flow.at[start..].partition_point(|a| *a < at);
        lay(&text_flow, start..end, &mut built);
        start = end;
        let first = built.blocks.len();
        let earlier = match &p {
            Placed::Table(t) => t.tagged.filter(|_| t.continues),
            _ => None,
        };
        let ids = match &p {
            Placed::Table(t) => t.ids.clone(),
            Placed::Title(t) => {
                let lines: HashSet<u32> = t.text_ids.iter().copied().collect();
                t.ids
                    .iter()
                    .copied()
                    .filter(|i| !lines.contains(i))
                    .collect()
            }
            Placed::Spatial(s) => s.ids.clone(),
            Placed::Unsure(_) | Placed::Header { .. } | Placed::Media { .. } => Vec::new(),
        };
        match p {
            Placed::Header { line, .. } => {
                built.paragraph(
                    header_lines[line].runs.iter().collect(),
                    Some("page-header"),
                    cx,
                );
            }
            Placed::Table(t) => built.table(t, cx),
            Placed::Title(t) => {
                let mut runs: Vec<&Run> = t.runs.iter().collect();
                runs.sort_by(|a, b| a.line.cmp(&b.line).then(a.x.total_cmp(&b.x)));
                built.heading(SIZE_LEVELS as u8, runs, cx);
                // The lines of its box or bar follow it, each a paragraph: they are lines of
                // a form (a dash that opens one is no list).
                let at = built.blocks.len();
                // The runs with their ids (`text_ids` holds the id of each run of `text`).
                let mut text: Vec<(Run, u32)> = t.text.into_iter().zip(t.text_ids).collect();
                text.sort_by(|a, b| a.0.line.cmp(&b.0.line).then(a.0.x.total_cmp(&b.0.x)));
                let (text, ids): (Vec<Run>, Vec<u32>) = text.into_iter().unzip();
                // Each line's runs became the paragraph it was written as; a line that wrote no
                // block goes with the block before it (the first one when none was before).
                let mut of_block: Vec<(usize, Vec<u32>)> = Vec::new();
                let mut from = 0;
                for l in lines(&text) {
                    let before = built.blocks.len();
                    built.paragraph(l.runs.iter().collect(), None, cx);
                    let own = &ids[from..from + l.runs.len()];
                    from += l.runs.len();
                    match of_block.last_mut() {
                        Some(last) if built.blocks.len() == before => {
                            last.1.extend_from_slice(own);
                        }
                        _ => of_block
                            .push((built.blocks.len().saturating_sub(1).max(at), own.to_vec())),
                    }
                }
                if built.blocks.len() > at && !ids.is_empty() {
                    let k = whys.len();
                    whys.push(decide::TITLE_LINES.to_string());
                    why_of.extend(ids.iter().map(|&i| (i, k)));
                    for (block, own) in of_block {
                        became.push((hosts::Became::Block(block), own));
                    }
                }
            }
            Placed::Media { .. } => {}
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
            Placed::Spatial(s) => {
                // The block's link targets, each once, and the index each target got (none
                // when the room for link targets is spent: the text stays, without its link).
                let mut targets: Vec<String> = Vec::new();
                let mut index: HashMap<*const UriTarget, Option<u32>> = HashMap::new();
                let mut link = |t: &Rc<UriTarget>| -> Option<u32> {
                    *index.entry(Rc::as_ptr(t)).or_insert_with(|| {
                        let k = u32::try_from(targets.len()).ok()?;
                        cx.links.take(t, page).then(|| {
                            targets.push(t.uri.to_string());
                            k
                        })
                    })
                };
                let frame;
                let lines = if s.pieces.is_empty() {
                    let prims: Vec<layoutregions::PrimId> = s
                        .ids
                        .iter()
                        .map(|&i| layoutregions::PrimId::Text(i))
                        .collect();
                    frame = layoutregions::extract::spatial::frame(&engine_in, &prims);
                    let marks = |id: u32| match run_marks.get(&id) {
                        Some((uri, note)) => decide::RunMarks {
                            uri: uri.as_ref(),
                            note: note.as_deref(),
                        },
                        None => decide::RunMarks::default(),
                    };
                    decide::spatial_lines(&engine_in, &prims, &marks, &mut link)
                } else {
                    // A reference's run holds its footnote's label: the text laid out is its
                    // marker as printed.
                    let texts = s
                        .pieces
                        .iter()
                        .map(|r| {
                            let mut t = engine::text_run(r);
                            if r.note
                                && let Some(m) = markers.get(r.text.trim())
                            {
                                t.text.clone_from(m);
                            }
                            t
                        })
                        .collect();
                    let marks = |i: u32| {
                        s.pieces
                            .get(i as usize)
                            .map_or_else(decide::RunMarks::default, |r| decide::RunMarks {
                                uri: r.uri.as_ref(),
                                note: r.note.then(|| r.text.trim()),
                            })
                    };
                    let own = layoutregions::PageIn {
                        texts,
                        page: engine_in.page,
                        size: engine_in.size,
                        turn: engine_in.turn,
                        given: Vec::new(),
                        rules: Vec::new(),
                        boxes: Vec::new(),
                        pictures: Vec::new(),
                        codes: Vec::new(),
                    };
                    let prims: Vec<layoutregions::PrimId> = (0..s.pieces.len())
                        .map(|i| layoutregions::PrimId::Text(i as u32))
                        .collect();
                    frame = layoutregions::extract::spatial::frame(&own, &prims);
                    decide::spatial_lines(&own, &prims, &marks, &mut link)
                };
                let k = whys.len();
                whys.push(s.why.clone());
                why_of.extend(s.ids.iter().map(|&i| (i, k)));
                if built.spatial(lines, targets, cx) {
                    let at = built.blocks.len() - 1;
                    spatial_whys.push((at, s.why, s.limit));
                    spatial_frames.extend(frame.map(|f| (at, f)));
                }
            }
        }
        became.push((hosts::became(&built.blocks, first, earlier), ids));
    }
    lay(&text_flow, start..text_flow.runs.len(), &mut built);
    let body_end = built.blocks.len();
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
    let footer_lines = lines(&footer_flow.runs);
    let (footer, own_lines) = footer_blocks(&footer_lines);
    for (n, block) in footer.iter().enumerate() {
        let runs: Vec<&Run> = block.iter().flat_map(|l| l.runs.iter()).collect();
        // A line of columns that a wrapped paragraph shows stays a line of its own.
        let own = own_lines.contains(&n);
        let mut breaks: Vec<(u32, u32)> = block
            .windows(2)
            .filter(|w| own || !footer_wrap(&w[0], &w[1]))
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
    built.own_unlaid();
    // Where the segmentation stopped, what it did not part is read by its decisions; the page
    // names the stop once.
    let stopped = segment_issues
        .iter()
        .any(|i| i.kind == layoutregions::IssueKind::Stopped);
    if stopped && built.stop.is_none() && !engine_in.texts.is_empty() {
        notes.push((
            format!(
                "the regions of page {page} are looked for only in part: the time limit was \
                 reached; the text past that is read as it stands"
            ),
            LossReason::Limit,
            Severity::Info,
        ));
    }
    segment_issues.dedup_by(|a, b| a.kind == b.kind);
    // Each decision says what the page wrote of it, then stands at the block it became.
    let demoted = std::mem::take(&mut built.demoted);
    let run_blocks = hosts::run_blocks(&built, &became, engine_in.texts.len());
    let media_at = reading.media_at(std::mem::take(&mut built.media_at), &run_blocks, body_end);
    let written: Vec<Option<&'static str>> = run_blocks
        .iter()
        .map(|&b| built.blocks.get(b as usize).map(crate::block_word))
        .collect();
    decide::agree(
        &mut decided,
        &engine_in,
        (&legend.ids, &demoted, &pre_drawn, &left_pairs),
        &written,
        &|i| why_of.get(&i).map(|&k| whys[k].as_str()),
    );
    let (hosts, earlier, unsure) = hosts::hosts(
        &decided,
        &engine_in,
        &built,
        &became,
        &run_blocks,
        &doubt_of,
        tagged_page,
    );
    let regions = decide::regions(page, decided, &hosts, &unsure, segment_issues);
    let probe_block = std::mem::take(&mut built.probe_block);
    let probes_cut = built.probes_cut;
    let mut laid = built.laid(notes, regions);
    laid.earlier = earlier;
    laid.run_blocks = run_blocks;
    laid.media_at = media_at;
    // A block kept as laid out that hosts a region of the engine gets its place on the page
    // grid; rows of one small table cut into regions are joined on one grid.
    let hosting: HashSet<usize> = laid
        .regions
        .regions
        .iter()
        .map(|r| r.host as usize)
        .collect();
    spatial_frames.retain(|(k, _)| hosting.contains(k));
    place_spatial(
        &mut laid.blocks,
        &spatial_frames,
        layoutregions::extract::spatial::page_grid(&engine_in),
    );
    laid.engine_in = Some(engine_in);
    laid.probes_cut = probes_cut;
    laid.found_by = footnotes::note(page, footnotes_written, linked_written);
    laid.title = title;
    laid.dates = dates;
    let (mut b, mut t) = (0, 0);
    let mut anchors: Vec<String> = Vec::with_capacity(laid.blocks.len());
    for block in &mut laid.blocks {
        let anchor = if matches!(block, Block::Table { .. }) {
            t += 1;
            format!("p{page}:t{t}")
        } else {
            b += 1;
            format!("p{page}:b{b}")
        };
        anchors.push(anchor.clone());
        set_anchor(block, Anchor(anchor));
    }
    // Each block kept as laid out says so, at its anchor, and how the Markdown writes it:
    // line by line, or, for blocks in the same columns that follow one another, as text in
    // its columns. That form is a fence, which shows no emphasis (the JSON marks it).
    let aligned: HashSet<usize> = rezzer_model::spatial_runs(&laid.blocks)
        .into_iter()
        .flatten()
        .collect();
    for (k, why, limit) in spatial_whys {
        // A block of one line with one piece has no layout its text does not say: it reads
        // as it stands, so the note that no type has clear evidence for it would name
        // nothing. Its region still says spatial and unsure. A note that names a kind the
        // block reads as (two kinds about equally, a kind whose reader built nothing) says
        // a doubt about what the line is, and stays. A block a limit left unread in part
        // keeps its note too, and so does a block written with others in their columns.
        let lone = match laid.blocks.get(k) {
            Some(Block::Spatial { lines, .. }) => one_piece(lines),
            _ => false,
        };
        if lone && !limit && !aligned.contains(&k) && why == decide::NO_EVIDENCE {
            continue;
        }
        if let Some(at) = anchors.get(k) {
            let how = if aligned.contains(&k) {
                "as text in its columns"
            } else {
                "line by line"
            };
            let styled = match laid.blocks.get(k) {
                Some(Block::Spatial { lines, .. }) if aligned.contains(&k) => style_clause(lines),
                _ => String::new(),
            };
            laid.located.push((
                at.clone(),
                format!("the region at {at}{KEPT_AS_LAID_OUT}{how}: {why}{styled}"),
                if limit {
                    LossReason::Limit
                } else {
                    LossReason::Uncertain
                },
                Severity::Info,
            ));
        }
    }
    laid.probed = probe_block
        .into_iter()
        .map(|k| k.and_then(|k| anchor_of(laid.blocks.get(k)?)))
        .collect();
    laid
}

/// Whether the lines of a block kept as laid out are one line of one piece: all its text is
/// one piece, so nothing stands beside or under anything. Lines and pieces without text do
/// not count.
fn one_piece(lines: &[rezzer_model::SpatialLine]) -> bool {
    let mut pieces = lines
        .iter()
        .flat_map(|l| &l.pieces)
        .filter(|p| !p.text.trim().is_empty());
    pieces.next().is_some() && pieces.next().is_none()
}

/// The reader of a table, pairs or totals decision, named by its winning evidence: drawn boxes
/// that each hold a label and its value are read by the box reader, lines that each hold a label
/// and its value by the line pairs reader, everything else (ruled grids, header bands, strips,
/// columns without rules, key values, totals) by the table readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AreaReader {
    Boxes,
    Lines,
    /// A row of labels over the row of their values, or a label with an underline of its
    /// own over its value: each label with the value under it (see [`label_over_value`]).
    Cell,
    Tables,
}

fn area_reader(d: &layoutregions::Decision) -> AreaReader {
    let has = |id: &str| d.signals.iter().any(|s| s.id == id);
    if d.kind == Kind::Totals && has(layoutregions::typers::TOTALS_LINE_PAIR) {
        return AreaReader::Lines;
    }
    if d.kind != Kind::LabelValue || has("label_value.strip_fields") || has("label_value.dividers")
    {
        return AreaReader::Tables;
    }
    if has(layoutregions::typers::pairs::LABEL_OVER_VALUE_ID)
        || has(layoutregions::typers::pairs::LABEL_UNDERLINED_ID)
    {
        AreaReader::Cell
    } else if has("label_value.line_pairs") {
        AreaReader::Lines
    } else if has("label_value.fields") || has("label_value.field_cells") {
        AreaReader::Boxes
    } else {
        AreaReader::Tables
    }
}

/// The runs of `flow` by their decision (`of_run`, by run id), each part in the flow's order,
/// the parts in the order of their first run.
fn by_decision(flow: Flow, of_run: &[u32]) -> Vec<(u32, Flow)> {
    let mut out: Vec<(u32, Flow)> = Vec::new();
    let mut at: HashMap<u32, usize> = HashMap::new();
    let places = flow.at.into_iter().zip(flow.ids);
    for ((r, s), (a, id)) in flow.runs.into_iter().zip(flow.slots).zip(places) {
        let d = of_run.get(id as usize).copied().unwrap_or(decide::NONE);
        let k = *at.entry(d).or_insert_with(|| {
            out.push((d, Flow::default()));
            out.len() - 1
        });
        let f = &mut out[k].1;
        f.runs.push(r);
        f.slots.push(s);
        f.at.push(a);
        f.ids.push(id);
    }
    out
}

/// The runs of a page by the middle of their height, with their decision, to tell whether a
/// line of another region stands between a label and its value.
struct Across {
    /// (middle, x0, x1, decision), by middle.
    runs: Vec<(f32, f32, f32, u32)>,
    /// Runs looked at so far on the page, against [`ACROSS_WORK`].
    work: std::cell::Cell<usize>,
    /// The bound was spent: the pairs of the tables checked after it were not checked.
    spent: std::cell::Cell<bool>,
}

impl Across {
    fn new(page: &layoutregions::PageIn, of_run: &[u32]) -> Across {
        let mut runs: Vec<(f32, f32, f32, u32)> = page
            .texts
            .iter()
            .enumerate()
            .filter(|(_, r)| layoutregions::line::ink(&r.text) > 0)
            .map(|(i, r)| {
                let d = of_run.get(i).copied().unwrap_or(decide::NONE);
                (r.y + r.h / 3.0, r.x.min(r.x + r.w), r.x.max(r.x + r.w), d)
            })
            .collect();
        runs.sort_by(|a, b| a.0.total_cmp(&b.0));
        Across {
            runs,
            work: std::cell::Cell::new(0),
            spent: std::cell::Cell::new(false),
        }
    }

    /// Whether a row of `t` (a table of decision `d`) has cells on two lines with a run of
    /// another decision between them, over their width: its label and value are paired across
    /// a line of another region. The runs looked at count against one bound for the page
    /// ([`ACROSS_WORK`]); past it a table is taken as not crossed and [`Across::spent`] says so.
    fn crossed(&self, t: &Planned, d: u32) -> bool {
        if self.spent.get() {
            return false;
        }
        for row in &t.rows {
            let cells: Vec<&PlannedCell> = row.iter().filter(|c| !c.runs.is_empty()).collect();
            for pair in cells.windows(2) {
                let span = |c: &PlannedCell| {
                    let ys = c.runs.iter().map(|r| r.y + r.h / 3.0);
                    let lo = ys.clone().fold(f32::MAX, f32::min);
                    let hi = ys.fold(f32::MIN, f32::max);
                    let x0 = c
                        .runs
                        .iter()
                        .map(|r| r.x.min(r.x + r.w))
                        .fold(f32::MAX, f32::min);
                    let x1 = c
                        .runs
                        .iter()
                        .map(|r| r.x.max(r.x + r.w))
                        .fold(f32::MIN, f32::max);
                    (lo, hi, x0, x1)
                };
                let (a, b) = (span(pair[0]), span(pair[1]));
                // The space between the two cells' lines, a third of a size kept off each.
                let size = pair[0].runs[0].size.max(pair[1].runs[0].size).max(1.0);
                let (lo, hi) = if a.0 > b.1 {
                    (b.1, a.0)
                } else if b.0 > a.1 {
                    (a.1, b.0)
                } else {
                    continue;
                };
                let (lo, hi) = (lo + size / 3.0, hi - size / 3.0);
                if hi <= lo {
                    continue;
                }
                let (x0, x1) = (a.2.min(b.2), a.3.max(b.3));
                let from = self.runs.partition_point(|r| r.0 <= lo);
                for r in &self.runs[from..] {
                    if r.0 >= hi {
                        break;
                    }
                    self.work.set(self.work.get() + 1);
                    if self.work.get() > ACROSS_WORK {
                        self.spent.set(true);
                        return false;
                    }
                    if r.3 != d && r.1 < x1 && r.2 > x0 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Runs one page may look at to tell labels paired across a line of another region, over all
/// its tables.
const ACROSS_WORK: usize = 4_000_000;

/// A table of the label and value of each line of a line pairs decision (see the typer's
/// `label_value.line_pairs`): each line parts where the typer's line pair rule parts it
/// ([`layoutregions::typers::pairs::line_pair_cut`]), so the pairs written are the pairs that
/// won the region. The runs of the lines it reads leave `area`; a line the rule does not part
/// stays, and the area reads no table when no line parts.
fn line_pairs(area: &mut Flow) -> Option<Planned> {
    let mut rows: Vec<Vec<PlannedCell>> = Vec::new();
    let mut ids: Vec<u32> = Vec::new();
    let mut taken = vec![false; area.runs.len()];
    let cell = |runs: Vec<Run>| PlannedCell {
        runs,
        row_span: 1,
        col_span: 1,
        status: None,
    };
    // The only line of the area with ink is lone, as the typer reads a group of one line.
    let lone = one_line(area);
    let mut i = 0;
    while i < area.runs.len() {
        let line = area.runs[i].line;
        let mut j = i;
        while j < area.runs.len() && area.runs[j].line == line {
            j += 1;
        }
        let runs = &area.runs[i..j];
        if let Some((label, value)) = part_line(runs, lone) {
            rows.push(vec![cell(label), cell(value)]);
            ids.extend_from_slice(&area.ids[i..j]);
            taken[i..j].iter_mut().for_each(|t| *t = true);
        }
        i = j;
    }
    if rows.is_empty() {
        return None;
    }
    let first = taken.iter().position(|t| *t)?;
    let at = area.at[first];
    let top = area.runs[first].y;
    area.take(|i, _| taken[i]);
    Some(Planned {
        at,
        top,
        rows,
        trs: Vec::new(),
        additions: Vec::new(),
        header_rows: 0,
        tagged: None,
        continues: false,
        caption: Vec::new(),
        found_by: None,
        kind: Kind::LabelValue,
        ids,
        by: tables::Detector::LinePairs,
        drawn: None,
        pictures: Vec::new(),
    })
}

/// A table of the labels of a row of labels over their values (the typer's
/// `label_value.label_over_value`), each a row of the label and the value under it, as the
/// typer's rule pairs them ([`layoutregions::extract::pairs::blocks::labels_with_values`]); a
/// label with nothing under it has an empty value. Every run of `area` leaves it. It reads no
/// table when the rule pairs nothing.
fn label_over_value(area: &mut Flow) -> Option<Planned> {
    let under = layoutregions::extract::pairs::blocks::labels_with_values(&area.runs)?;
    let cell = |ids: &[usize], status: Option<&'static str>| PlannedCell {
        runs: ids.iter().map(|&i| area.runs[i].clone()).collect(),
        row_span: 1,
        col_span: 1,
        status,
    };
    let rows: Vec<Vec<PlannedCell>> = under
        .iter()
        .map(|u| {
            vec![
                cell(&u.label, None),
                // A label with nothing under it is a field left empty, as an empty box is.
                cell(&u.value, u.value.is_empty().then_some("empty")),
            ]
        })
        .collect();
    let first = *under.first()?.label.first()?;
    let (at, top) = (
        area.at.iter().copied().min().unwrap_or(area.at[first]),
        area.runs[first].y,
    );
    let ids = area.ids.clone();
    area.take(|_, _| true);
    Some(Planned {
        at,
        top,
        rows,
        trs: Vec::new(),
        additions: Vec::new(),
        header_rows: 0,
        tagged: None,
        continues: false,
        caption: Vec::new(),
        found_by: None,
        kind: Kind::LabelValue,
        ids,
        by: tables::Detector::LinePairs,
        drawn: None,
        pictures: Vec::new(),
    })
}

/// Whether `t` is rows of a label and its value that the line pair rule or the cells of
/// labels over values gave (see [`line_pairs`] and [`label_over_value`]): two cells a row, no
/// header row, nothing else to it.
fn pair_rows(t: &Planned) -> bool {
    t.by == tables::Detector::LinePairs
        && t.kind == Kind::LabelValue
        && t.header_rows == 0
        && t.tagged.is_none()
        && !t.continues
        && t.caption.is_empty()
        && t.found_by.is_none()
        && t.drawn.is_none()
        && t.pictures.is_empty()
        && t.additions.is_empty()
        && t.trs.is_empty()
        && t.rows.iter().all(|r| r.len() == 2)
}

/// Where a table of pair rows (see [`pair_rows`]) stands, for [`join_pair_rows`]: its highest
/// and lowest baseline, the pitch of its rows where they stand under each other (the first and
/// the last step between the baselines of two labels), the left edge of its labels and of its
/// values, its width and its largest text size.
#[derive(Debug, Clone, Copy)]
struct PairPlace {
    top: f32,
    bottom: f32,
    first_step: Option<f32>,
    last_step: Option<f32>,
    labels: f32,
    values: Option<f32>,
    x0: f32,
    x1: f32,
    size: f32,
}

impl PairPlace {
    fn of(t: &Planned) -> Option<PairPlace> {
        let runs = || t.rows.iter().flatten().flat_map(|c| &c.runs);
        let edge = |cell: usize| {
            t.rows
                .iter()
                .filter_map(|r| r.get(cell))
                .flat_map(|c| &c.runs)
                .map(|r| r.x.min(r.x + r.w))
                .min_by(f32::total_cmp)
        };
        let size = runs().map(|r| r.size).max_by(f32::total_cmp)?;
        // The baselines of the labels, from the top down: rows under each other have one each.
        let mut ys: Vec<f32> = t
            .rows
            .iter()
            .filter_map(|r| r.first()?.runs.first())
            .map(|r| r.y)
            .collect();
        ys.sort_by(|a, b| b.total_cmp(a));
        ys.dedup_by(|a, b| (*a - *b).abs() <= layoutregions::segment::cut::BASELINE_NEAR * size);
        let step = |w: Option<&[f32]>| w.map(|w| w[0] - w[1]);
        Some(PairPlace {
            top: runs().map(|r| r.y).max_by(f32::total_cmp)?,
            bottom: runs().map(|r| r.y).min_by(f32::total_cmp)?,
            first_step: step(ys.windows(2).next()),
            last_step: step(ys.windows(2).next_back()),
            labels: edge(0)?,
            values: edge(1),
            x0: runs().map(|r| r.x.min(r.x + r.w)).min_by(f32::total_cmp)?,
            x1: runs().map(|r| r.x.max(r.x + r.w)).max_by(f32::total_cmp)?,
            size,
        })
    }
}

/// A drawn line that runs along the lines of text, for [`pair_step`]: a rule, or the upper
/// or the lower edge of a rectangle or of a drawn box. `other` is the height of the far edge
/// of what draws it (the other side of a rule's own thickness, or the far side of a box).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Edge {
    y: f32,
    x0: f32,
    x1: f32,
    other: f32,
}

/// The one drawn line between two rows of pairs (see [`pair_step`]): its height and its
/// extent.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RowRule {
    y: f32,
    x0: f32,
    x1: f32,
    /// The lowest and the highest far side of the edges of drawn boxes it is made of (an
    /// edge whose far side lies off its own height); `None` for a line of thin rules only.
    far: Option<(f32, f32)>,
}

impl RowRule {
    /// Notes the far side `other` of an edge of the line at height `y`, when it is the
    /// edge of a box (its far side farther than `near` from it).
    fn edge_of_box(&mut self, y: f32, other: f32, near: f32) {
        if (other - y).abs() > near {
            let (lo, hi) = self.far.unwrap_or((other, other));
            self.far = Some((lo.min(other), hi.max(other)));
        }
    }

    /// Whether this line and `other` are the two sides of one drawn box, each within
    /// `near` of where the box edges of the other end: every box edge of one of them has
    /// its far side at the other. The box of one row has its upper and its lower side one
    /// row apart, as the lines of a ruled block are, but it is one frame around one row
    /// and no line that comes again. Boxes of one row each that touch have, at each line
    /// between two rows, the lower side of the box over it and the upper side of the box
    /// under it: far sides both ways, so those lines do come again.
    fn sides_of_one_box(&self, other: &RowRule, near: f32) -> bool {
        let at = |a: &RowRule, b: &RowRule| {
            a.far
                .is_some_and(|(lo, hi)| (lo - b.y).abs() <= near && (hi - b.y).abs() <= near)
        };
        at(self, other) || at(other, self)
    }
}

/// How two tables of pair rows that follow each other stand to each other: `step` is how far
/// the first baseline of the lower one stands under the last baseline of the upper one, and
/// `size` the larger text size of the two.
#[derive(Debug, Clone, Copy)]
struct PairStep {
    step: f32,
    size: f32,
    /// The pitch of the rows of either table, where it has rows under each other.
    own: [Option<f32>; 2],
    /// The one drawn line between the two tables, when there is one.
    ruled: Option<RowRule>,
}

impl PairStep {
    /// Whether two steps are the same pitch: within [`BASELINE_NEAR`] of the text size, as
    /// two baselines are one line.
    ///
    /// [`BASELINE_NEAR`]: layoutregions::segment::cut::BASELINE_NEAR
    fn like(&self, other: f32) -> bool {
        (self.step - other).abs() <= layoutregions::segment::cut::BASELINE_NEAR * self.size
    }

    /// Whether the two tables stand at the pitch of one block: as near as the lines of a
    /// block of labels and values ([`ITEM_PITCH`] text sizes), at the pitch of the rows of
    /// one of them, or at the pitch the tables before or after them follow each other by
    /// (`around`: the document shows the pitch of the block twice).
    ///
    /// [`ITEM_PITCH`]: layoutregions::typers::pairs::ITEM_PITCH
    fn of_one_block(&self, around: [Option<f32>; 2]) -> bool {
        self.step <= layoutregions::typers::pairs::ITEM_PITCH * self.size
            || self
                .own
                .iter()
                .chain(&around)
                .flatten()
                .any(|&p| self.like(p))
    }

    /// Whether the drawn line of this step comes again in the step `next` right after it:
    /// the same line, of one extent (both ends within [`EDGE_SIZES`] text sizes), one row
    /// pitch lower (the lines as far apart as the rows, and both steps the same pitch).
    ///
    /// [`EDGE_SIZES`]: layoutregions::line::EDGE_SIZES
    fn comes_again(&self, next: &PairStep) -> bool {
        let (Some(line), Some(other)) = (self.ruled, next.ruled) else {
            return false;
        };
        let near = |a: f32, b: f32| (a - b).abs() <= layoutregions::line::EDGE_SIZES * self.size;
        near(line.x0, other.x0)
            && near(line.x1, other.x1)
            && self.like(next.step)
            && self.like((line.y - other.y).abs())
    }

    /// Whether the drawn lines of this step and of `next` are the two sides of one drawn
    /// box (see [`RowRule::sides_of_one_box`], within [`BASELINE_NEAR`] of the text size).
    ///
    /// [`BASELINE_NEAR`]: layoutregions::segment::cut::BASELINE_NEAR
    fn frames_a_row(&self, next: &PairStep) -> bool {
        let near = layoutregions::segment::cut::BASELINE_NEAR * self.size;
        matches!((self.ruled, next.ruled), (Some(a), Some(b)) if a.sides_of_one_box(&b, near))
    }
}

/// For each step of `steps` (the steps between the parts of a page that follow each other,
/// see [`pair_step`]), whether its drawn line parts nothing. A step with no line has none
/// that parts. A drawn line is the ruling of the rows' own block only when the same line
/// stands between every two rows of the run: the run is the steps that follow each other at
/// one pitch (within [`BASELINE_NEAR`] of the text size), and in it every step has the line,
/// each the line of the step before again ([`PairStep::comes_again`]), two steps or more, and
/// no table has rows of its own under each other (which have no line between them). Two
/// lines alone that are the upper and the lower side of one drawn box
/// ([`PairStep::frames_a_row`]) are one frame around one row, no line that comes again;
/// with a third line at the pitch (a band behind every other row) the lines do. So a
/// lone line, a line of another extent or off the pitch, the two sides of the box of one
/// row, and lines that stop while the rows go on at their pitch (a boxed row among plain
/// rows, a ruled block with plain rows after it) are borders: each parts its two rows.
///
/// [`BASELINE_NEAR`]: layoutregions::segment::cut::BASELINE_NEAR
fn own_rulings(steps: &[Option<PairStep>]) -> Vec<bool> {
    let mut own: Vec<bool> = steps
        .iter()
        .map(|s| s.is_some_and(|s| s.ruled.is_none()))
        .collect();
    crate::work::add(2 * steps.len() as u64);
    let mut from = 0;
    while from < steps.len() {
        // The run from `from`: the steps after it, each at the pitch of the one before.
        let mut to = from + 1;
        while let (Some(Some(before)), Some(Some(next))) = (steps.get(to - 1), steps.get(to))
            && before.like(next.step)
        {
            to += 1;
        }
        let run: Vec<PairStep> = steps[from..to].iter().flatten().copied().collect();
        if run.len() >= 2
            && run
                .iter()
                .all(|s| s.ruled.is_some() && s.own.iter().all(Option::is_none))
            && run.windows(2).all(|w| w[0].comes_again(&w[1]))
            && (run.len() > 2 || !run[0].frames_a_row(&run[1]))
        {
            own[from..to].iter_mut().for_each(|o| *o = true);
        }
        from = to;
    }
    own
}

/// For each step of `steps`, whether its two tables are rows of one block: its drawn line,
/// when it has one, is the block's own ruling ([`own_rulings`]), and the step is at the
/// pitch of one block ([`PairStep::of_one_block`]). The steps before and after show their
/// pitch to it only when they are steps inside a block themselves: a step whose line is a
/// border stands between two blocks, and how far those are apart is no pitch of rows. The
/// pitch is then shown by the nearest step past the border that is one inside a block (the
/// rows over a rule stand at the pitch of the rows under it), with no other part between.
/// The look goes past one border only: a second border step ends it, as a part that is no
/// row of pairs does (rows three borders away are rows of a block far off, not the next one).
fn joined_steps(steps: &[Option<PairStep>]) -> Vec<bool> {
    let own = own_rulings(steps);
    crate::work::add(steps.len() as u64);
    // For each step, the pitch of the nearest step inside a block before it and after it,
    // past one step across a border, up to a part that is no row of pairs.
    let mut before: Vec<Option<f32>> = vec![None; steps.len()];
    let mut after = before.clone();
    // The pitch seen last and the border steps passed since; the next step in the look.
    let mut seen: (Option<f32>, u32) = (None, 0);
    let pass = |k: usize, seen: &mut (Option<f32>, u32)| match steps[k] {
        Some(s) if own[k] => *seen = (Some(s.step), 0),
        Some(_) if seen.1 == 0 => seen.1 = 1,
        _ => *seen = (None, 0),
    };
    for (k, b) in before.iter_mut().enumerate() {
        *b = seen.0;
        pass(k, &mut seen);
    }
    seen = (None, 0);
    for (k, a) in after.iter_mut().enumerate().rev() {
        *a = seen.0;
        pass(k, &mut seen);
    }
    (0..steps.len())
        .map(|j| steps[j].is_some_and(|s| own[j] && s.of_one_block([before[j], after[j]])))
        .collect()
}

/// The step from table `a` to table `b` (the next part in reading order) when they may be
/// rows of one block: `b` stands under `a`, their labels or their values start at one left
/// edge (within [`EDGE_SIZES`] text sizes), and no edge of a drawn box and at most one drawn
/// line lies between them over their width (`edges`, sorted by height). One line is kept in
/// the step ([`PairStep::ruled`]; whether it parts the two is asked of the steps of its run,
/// see [`own_rulings`]): every edge between the two baselines is drawn by something
/// no higher than one row (a rule with its thickness, a hairline rectangle, the box of one
/// row of a ruled block: its far side at most the step of the two rows away, within
/// [`BASELINE_NEAR`] text sizes), and they all lie within [`BASELINE_NEAR`] text sizes of
/// one height, as two baselines are one line. An edge whose far side is farther away is the
/// side of a box around more than a row, and edges at two heights are two lines: both part
/// the tables. The extent of the line is that of
/// its pieces over the tables and of the pieces that go on from them (a rule drawn in pieces
/// is one rule): pieces no higher than one row too, by the same bound. The upper side of a
/// tall box that starts where the rule ends is no piece of the rule.
///
/// [`EDGE_SIZES`]: layoutregions::line::EDGE_SIZES
/// [`BASELINE_NEAR`]: layoutregions::segment::cut::BASELINE_NEAR
fn pair_step(a: &PairPlace, b: &PairPlace, edges: &[Edge]) -> Option<PairStep> {
    let size = a.size.max(b.size);
    let step = a.bottom - b.top;
    if step <= layoutregions::segment::cut::BASELINE_NEAR * size {
        return None;
    }
    let near = |x: f32, y: f32| (x - y).abs() <= layoutregions::line::EDGE_SIZES * size;
    let values = matches!((a.values, b.values), (Some(x), Some(y)) if near(x, y));
    if !near(a.labels, b.labels) && !values {
        return None;
    }
    let (x0, x1) = (a.x0.min(b.x0), a.x1.max(b.x1));
    let from = edges.partition_point(|e| e.y <= b.top);
    let to = from + edges[from..].partition_point(|e| e.y < a.bottom);
    let between = &edges[from..to];
    crate::work::add(between.len() as u64);
    let near_y = layoutregions::segment::cut::BASELINE_NEAR * size;
    // No higher than one row: the far side at most the step of the two rows away.
    let of_a_row = |e: &Edge| (e.other - e.y).abs() <= step + near_y;
    let mut ruled: Option<RowRule> = None;
    for e in between.iter().filter(|e| e.x0 < x1 && e.x1 > x0) {
        if !of_a_row(e) {
            return None;
        }
        let line = ruled.get_or_insert(RowRule {
            y: e.y,
            x0: e.x0,
            x1: e.x1,
            far: None,
        });
        if (e.y - line.y).abs() > near_y {
            return None;
        }
        (line.x0, line.x1) = (line.x0.min(e.x0), line.x1.max(e.x1));
        line.edge_of_box(e.y, e.other, near_y);
    }
    if let Some(line) = &mut ruled {
        // The pieces of the line that go on beside the tables, left and right, each from
        // where the one before ends.
        let mut on: Vec<&Edge> = between
            .iter()
            .filter(|e| (e.y - line.y).abs() <= near_y && of_a_row(e))
            .collect();
        on.sort_by(|p, q| p.x0.total_cmp(&q.x0));
        for e in &on {
            if e.x0 <= line.x1 + layoutregions::line::EDGE_SIZES * size && e.x1 > line.x1 {
                line.x1 = e.x1;
            }
        }
        on.sort_by(|p, q| q.x1.total_cmp(&p.x1));
        for e in &on {
            if e.x1 >= line.x0 - layoutregions::line::EDGE_SIZES * size && e.x0 < line.x0 {
                line.x0 = e.x0;
            }
        }
    }
    Some(PairStep {
        step,
        size,
        own: [a.last_step, b.first_step],
        ruled,
    })
}

/// The parts `placed` of a page (in reading order), with each run of tables of pair rows
/// (see [`pair_rows`]) of one block written as one table. Two tables that follow each other
/// are rows of one block when no other part and no text with ink of `text` (the page's text
/// in reading order) stands between them, the lower one stands under the upper one at the
/// pitch of the block, from the same left edge of the labels or of the values, and no box
/// edge of `drawn` and no drawn rule but the block's own ruling lies between them (see
/// [`pair_step`], [`own_rulings`] and [`PairStep::of_one_block`]). Lines of a label and its value under each other, and the
/// cells of a row of labels over their values, are regions of their own to the engine, one
/// pair each; to a reader the rows of one block are one list of pairs, and two blocks of the
/// page are two lists.
fn join_pair_rows(placed: Vec<Placed>, text: &Flow, drawn: &Drawn<'_>) -> Vec<Placed> {
    let inked: Vec<usize> = text
        .runs
        .iter()
        .zip(&text.at)
        .filter(|(r, _)| layoutregions::line::ink(&r.text) > 0)
        .map(|(_, &a)| a)
        .collect();
    crate::work::add((placed.len() + text.runs.len()) as u64);
    let between = |a: usize, b: usize| {
        let i = inked.partition_point(|x| *x <= a);
        inked.get(i).is_some_and(|x| *x < b)
    };
    let places: Vec<Option<(usize, PairPlace)>> = placed
        .iter()
        .map(|p| match p {
            Placed::Table(t) if pair_rows(t) => PairPlace::of(t).map(|at| (t.at, at)),
            _ => None,
        })
        .collect();
    // No table of pair rows follows another: nothing to join, and the page's rules are not
    // looked at.
    if !places.windows(2).any(|w| w[0].is_some() && w[1].is_some()) {
        return placed;
    }
    // The drawn lines that part two rows: each rule that runs along the lines, and the upper
    // and the lower edge of each rectangle and box.
    let mut edges: Vec<Edge> = Vec::new();
    let mut sides = |y0: f32, y1: f32, x0: f32, x1: f32| {
        edges.push(Edge {
            y: y0,
            x0,
            x1,
            other: y1,
        });
        edges.push(Edge {
            y: y1,
            x0,
            x1,
            other: y0,
        });
    };
    for r in drawn.rules {
        let (x0, x1) = (r.x0.min(r.x1), r.x0.max(r.x1));
        let (y0, y1) = (r.y0.min(r.y1), r.y0.max(r.y1));
        if x1 - x0 > y1 - y0 || r.kind == tables::RuleKind::Rect {
            sides(y0, y1, x0, x1);
        }
    }
    for b in drawn.boxes {
        sides(
            b[1].min(b[3]),
            b[1].max(b[3]),
            b[0].min(b[2]),
            b[0].max(b[2]),
        );
    }
    edges.sort_by(|a, b| a.y.total_cmp(&b.y));
    crate::work::add((drawn.rules.len() + drawn.boxes.len()) as u64);
    // For each part, how it stands to the part after it, when both are rows of pairs that
    // may be of one block.
    let steps: Vec<Option<PairStep>> = places
        .windows(2)
        .map(|w| {
            let ((a_at, a), (b_at, b)) = (w[0]?, w[1]?);
            if between(a_at, b_at) {
                return None;
            }
            pair_step(&a, &b, &edges)
        })
        .collect();
    let joined = joined_steps(&steps);
    let mut out: Vec<Placed> = Vec::with_capacity(placed.len());
    for (k, p) in placed.into_iter().enumerate() {
        // Whether the part before and this one are rows of one block.
        let joins = k.checked_sub(1).is_some_and(|j| joined[j]);
        match (p, out.last_mut()) {
            (Placed::Table(next), Some(Placed::Table(last))) if joins => {
                last.rows.extend(next.rows);
                last.ids.extend(next.ids);
            }
            (p, _) => out.push(p),
        }
    }
    out
}

/// Whether the runs of `area` with ink stand on one line.
fn one_line(area: &Flow) -> bool {
    let mut inked = area
        .runs
        .iter()
        .filter(|r| layoutregions::line::ink(&r.text) > 0)
        .map(|r| r.line);
    let first = inked.next();
    inked.all(|l| Some(l) == first)
}

/// The label and the value of one line where the typer's line pair rule parts it: a run the
/// cut falls in is cut in two, each piece its share of the run's width.
fn part_line(runs: &[Run], lone: bool) -> Option<(Vec<Run>, Vec<Run>)> {
    let cut = layoutregions::typers::pairs::line_pair_cut(runs, lone)?;
    if cut.byte == 0 {
        return Some((runs[..cut.run].to_vec(), runs[cut.run..].to_vec()));
    }
    let r = &runs[cut.run];
    let mut before = r.clone();
    before.text = r.text[..cut.byte].to_string();
    let mut after = r.clone();
    after.text = r.text[cut.byte..].to_string();
    let chars = r.text.chars().count().max(1) as f32;
    let share = r.text[..cut.byte].chars().count() as f32 / chars;
    before.w = r.w * share;
    after.x = r.x + r.w * share;
    after.w = r.w - before.w;
    before.gaps = Gaps::default();
    after.gaps = Gaps::default();
    let mut label = runs[..cut.run].to_vec();
    label.push(before);
    let mut value = vec![after];
    value.extend(runs[cut.run + 1..].iter().cloned());
    Some((label, value))
}

/// A table of the page as the head of page 1 reads it: its rows as the text of their cells,
/// `lines` when its pairs were read from lines of text.
fn head_table(t: &Planned, lines: bool) -> head::Table {
    head::Table {
        pairs: t.is_pairs(),
        lines,
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
                            x1: c.runs.iter().map(|r| r.x + r.w).fold(first.x, f32::max),
                            size: first.size,
                        }),
                    })
                    .collect()
            })
            .collect(),
    }
}

/// The value that stands most often in `values` (the first of them on a tie), leaving out
/// [`decide::NONE`].
fn majority(values: impl Iterator<Item = u32>) -> Option<u32> {
    let mut count: Vec<(u32, usize)> = Vec::new();
    for v in values.filter(|v| *v != decide::NONE) {
        match count.iter_mut().find(|c| c.0 == v) {
            Some(c) => c.1 += 1,
            None => count.push((v, 1)),
        }
    }
    let most = count.iter().map(|c| c.1).max()?;
    count.iter().find(|c| c.1 == most).map(|c| c.0)
}

/// What a block of the walk never goes across: the kind of the decision of a line, the drawn
/// box it stands in, and its region when the region is placed whole (see [`lay_text`]).
type WalkKey = (Option<Kind>, Option<u32>, Option<u32>);

/// What the laying of text asks about a run, by its id.
struct RunKeys<'a> {
    /// The kind of the run's decision and the drawn box it stands in.
    kind_of: &'a dyn Fn(u32) -> (Option<Kind>, Option<u32>),
    /// The region the engine itself decided the run as text (see
    /// `decide::Decided::text_region_of_run`).
    region_of: &'a dyn Fn(u32) -> Option<u32>,
    /// The drawn grid the run stands in, when a pre-pass reader gave or refused its text as
    /// one region.
    grid_of: &'a dyn Fn(u32) -> Option<u32>,
    /// Whether a region has ink on more than one line of the page's text.
    several: &'a dyn Fn(u32) -> bool,
    /// Whether the run stands in blocks side by side read as columns, whose lines stay lines
    /// of their own (see `tables::Found::own_lines`).
    own_line: &'a dyn Fn(u32) -> bool,
    /// The drawn lines of the page, in the frame of the runs.
    rules: &'a [layoutregions::RuleIn],
}

/// A drawn line of a block's own stands between two of its lines (the mirror of
/// `layoutregions::part_lines`): a line across the page's reading direction that lies at
/// or under the baseline of `over` (a run of the block's upper line) and over the baseline of
/// `under` (a run of its next line), stands under `over` with a part of its length, and ends
/// before `beside`, the text that follows `over` on its line. A rule of a grid beside the
/// block that goes on under the block does not end before the grid's text.
fn own_rule(rules: &[layoutregions::RuleIn], over: &Run, under: &Run, beside: &Run) -> bool {
    crate::work::add(rules.len() as u64);
    rules.iter().any(|r| {
        let (x0, x1) = (r.x0.min(r.x1), r.x0.max(r.x1));
        let (y0, y1) = (r.y0.min(r.y1), r.y0.max(r.y1));
        !r.rect
            && y1 - y0 < x1 - x0
            && y1 <= over.y
            && y0 > under.y
            && x0 < over.x + over.w
            && x1 > over.x
            && x1 < beside.x
    })
}

/// How a run of text is laid out: by its tag (a run a tag element holds), or by the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextKey {
    Tag,
    Free,
}

/// Lays out the runs `range` of `flow` as text: runs a tag element holds by their tags
/// ([`tagged`]), the others by one walk ([`untagged`]) over each stretch of them, with the
/// kind of their decision and the drawn box they stand in (`keys`, by run id). A block of
/// the walk never goes from one kind or box into another (text in a drawn box never joins text
/// outside it or in another box), but the walk reads the lines around it too: a line's
/// measure and the column a heading size stands in are the page's, not its decision's.
fn lay_text(
    flow: &Flow,
    range: Range<usize>,
    keys: &RunKeys<'_>,
    text_sizes: &[f32],
    cx: &Context<'_>,
    built: &mut Built,
) {
    let key = |i: usize| -> TextKey {
        if flow.slots[i] == Slot::Free {
            TextKey::Free
        } else {
            TextKey::Tag
        }
    };
    let walk_key = |id: u32| -> WalkKey {
        let (kind, boxed) = (keys.kind_of)(id);
        let kind = match kind {
            Some(Kind::Heading | Kind::TitleBlock) => Some(Kind::Heading),
            Some(k @ (Kind::Paragraph | Kind::List)) => Some(k),
            _ => None,
        };
        (kind, boxed, None)
    };
    let mut s = range.start;
    while s < range.end && built.stop.is_none() {
        let k = key(s);
        let mut e = s + 1;
        while e < range.end && key(e) == k {
            e += 1;
        }
        let (runs, slots, ids) = (&flow.runs[s..e], &flow.slots[s..e], &flow.ids[s..e]);
        match k {
            TextKey::Tag => tagged(runs, slots, Some(ids), cx, built),
            TextKey::Free => {
                // A region the engine keeps whole is placed whole: a line
                // of the text pass is parted where a block of several lines starts beside the
                // text of a drawn grid (see `layoutregions::part_lines`). The edge is the
                // grid's only: a drawn box around a word (a part number, a mark, a bracket at
                // a box's edge) parts headings and values from their labels, measured on the
                // public sets. The walk then never takes a block from such a region into
                // other text: the region is part of the key of its lines.
                // The mirror: where the block starts the line and a row of the
                // grid's cells follows it, the cells are read before the block when a drawn
                // line of the block's own stands between its two lines (`own_rule`), so the
                // label stands with its value.
                let (region, boxed): (Vec<Option<u32>>, Vec<Option<u32>>) = ids
                    .iter()
                    .map(|&id| ((keys.region_of)(id), (keys.grid_of)(id)))
                    .unzip();
                let line: Vec<u32> = runs.iter().map(|r| r.line).collect();
                let inked: Vec<bool> = runs.iter().map(|r| !r.text.trim().is_empty()).collect();
                let extent: Vec<(f32, f32, f32)> =
                    runs.iter().map(|r| (r.x, r.x + r.w, r.size)).collect();
                let parted = layoutregions::part_lines(
                    &line,
                    &region,
                    &boxed,
                    &inked,
                    &extent,
                    keys.several,
                    &|over, under, beside| {
                        own_rule(keys.rules, &runs[over], &runs[under], &runs[beside])
                    },
                );
                match parted {
                    None => {
                        let keys = (&walk_key as &dyn Fn(u32) -> WalkKey, keys.own_line);
                        untagged(runs, Some(ids), text_sizes, keys, cx, built);
                    }
                    Some(parted) => {
                        // The runs in the order the parting reads them: the cells of a grid
                        // before the block that starts their line.
                        let cut: Vec<Run> = parted
                            .order
                            .iter()
                            .zip(&parted.lines)
                            .map(|(&i, &line)| Run {
                                line,
                                ..runs[i].clone()
                            })
                            .collect();
                        let ids: Vec<u32> = parted.order.iter().map(|&i| ids[i]).collect();
                        let ids = &ids[..];
                        let key = |id: u32| -> WalkKey {
                            let (kind, boxed, _) = walk_key(id);
                            let whole = (keys.region_of)(id)
                                .filter(|r| parted.whole.binary_search(r).is_ok());
                            (kind, boxed, whole)
                        };
                        let keys = (&key as &dyn Fn(u32) -> WalkKey, keys.own_line);
                        untagged(&cut, Some(ids), text_sizes, keys, cx, built);
                    }
                }
            }
        }
        s = e;
    }
}

/// The anchor a block of a page carries, a footnote section's (`fn:<label>`) too.
pub(crate) fn block_anchor(block: &Block) -> Option<Anchor> {
    match block {
        Block::Section { anchor, .. } => anchor.clone(),
        b => anchor_of(b),
    }
}

fn anchor_of(block: &Block) -> Option<Anchor> {
    match block {
        Block::Heading { anchor, .. }
        | Block::Paragraph { anchor, .. }
        | Block::List { anchor, .. }
        | Block::Spatial { anchor, .. }
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
    /// Runs of one decision kept as laid out.
    Spatial(SpatialPart),
    /// A line of the running header (its index in the page's header lines), where its first
    /// run stands in the page's order, and its baseline.
    Header {
        line: usize,
        at: usize,
        top: f32,
    },
    /// The pictures and codes of a figure or code region, at its place in reading order: they
    /// stand before the first block written from that place on (see [`Laid::media_at`]).
    Media {
        at: usize,
        decision: usize,
        prims: Vec<layoutregions::PrimId>,
    },
}

/// Gives each block of `blocks` kept as laid out whose place on the page is known (`frames`:
/// its place in `blocks`, ascending, and its frame) its column on the grid of the page
/// (`page`, see [`layoutregions::extract::spatial::page_grid`]), and joins rows of one small table that
/// the engine cut into regions: a block joins the block kept as laid out before it (pictures
/// between them aside) when both hold a line of two pieces or more and it stands right under
/// that block in the column of the first block of the run ([`SpatialFrame::under`]). The
/// columns of the pieces of a run are then counted on one grid, from the least left edge of
/// its blocks in the least character width among them ([`SpatialFrame::column_on`]), so the
/// Markdown can write the run as one block aligned in columns without moving a piece under
/// a column it does not stand under. A block that does not join reads alone, as do blocks
/// without a frame.
///
/// The columns of a block are counted from its own left edge in its own character width
/// They say nothing about another block until both stand on one grid.
fn place_spatial(
    blocks: &mut [Block],
    frames: &[(usize, SpatialFrame)],
    page: Option<layoutregions::extract::spatial::PageGrid>,
) {
    let Some(page) = page.filter(|p| p.left.is_finite() && p.unit.is_finite() && p.unit > 0.0)
    else {
        return;
    };
    let (left, unit) = (page.left, page.unit);
    // A line of two pieces or more: the block has columns.
    let columns = |block: &Block| match block {
        Block::Spatial { lines, .. } => lines.iter().any(|l| {
            l.pieces
                .iter()
                .filter(|p| !p.text.trim().is_empty())
                .nth(1)
                .is_some()
        }),
        _ => false,
    };
    // The runs, a block alone too: the place and the frame of each block.
    let mut runs: Vec<Vec<(usize, SpatialFrame)>> = Vec::new();
    for &(k, frame) in frames {
        if !matches!(blocks.get(k), Some(Block::Spatial { .. })) {
            continue;
        }
        let joins = runs.last().is_some_and(|run| {
            let (&(first, start), &(last, above)) = (&run[0], &run[run.len() - 1]);
            // Only pictures stand between the last block of the run and this one.
            last < k
                && blocks[last + 1..k]
                    .iter()
                    .all(|b| matches!(b, Block::Media { .. }))
                && columns(&blocks[first])
                && columns(&blocks[k])
                && frame.under(&above, start.x0, &page)
        });
        match runs.last_mut() {
            Some(run) if joins => run.push((k, frame)),
            _ => runs.push(vec![(k, frame)]),
        }
    }
    for run in runs {
        let x0 = run.iter().map(|(_, f)| f.x0).fold(f32::MAX, f32::min);
        let adv = run.iter().map(|(_, f)| f.adv).fold(f32::MAX, f32::min);
        let c = ((x0 - left) / unit).round();
        let col = if c.is_finite() && c > 0.0 {
            c.min(u32::MAX as f32) as u32
        } else {
            0
        };
        let one_grid = run.len() > 1 && adv.is_finite() && adv > 0.0;
        for (n, (k, frame)) in run.into_iter().enumerate() {
            let Some(Block::Spatial { lines, grid, .. }) = blocks.get_mut(k) else {
                continue;
            };
            *grid = Some(rezzer_model::SpatialGrid {
                col,
                joins: one_grid && n > 0,
            });
            if one_grid {
                for p in lines.iter_mut().flat_map(|l| &mut l.pieces) {
                    p.col = frame.column_on(p.col, x0, adv);
                }
            }
        }
    }
}

/// What the loss of a block kept as laid out and written as text in its columns adds for its
/// style: its bold or italic text is marked in the JSON only, as a text fence shows no
/// emphasis.
fn style_clause(lines: &[rezzer_model::SpatialLine]) -> String {
    let spans = || lines.iter().flat_map(|l| &l.pieces).flat_map(|p| &p.spans);
    match (spans().any(|s| s.strong), spans().any(|s| s.emphasis)) {
        (true, true) => format!("; its bold and italic{MARKED_IN_JSON_ONLY}"),
        (true, false) => format!("; its bold{MARKED_IN_JSON_ONLY}"),
        (false, true) => format!("; its italic{MARKED_IN_JSON_ONLY}"),
        (false, false) => String::new(),
    }
}

/// The runs of one decision kept as laid out: where it stands in the page's order, its top,
/// the ids of its runs and why it is kept so.
struct SpatialPart {
    at: usize,
    top: f32,
    ids: Vec<u32>,
    /// The runs of the part as the table readers left them, pieces of cut runs too; none for
    /// a part whose runs are the page's own.
    pieces: Vec<Run>,
    why: String,
    /// Kept as laid out because a limit or a stop ended the search before its kind was
    /// looked for: its loss is a `Limit` one.
    limit: bool,
}

impl Placed {
    /// The place in reading order it stands at, and its top.
    fn at(&self) -> (usize, f32) {
        match self {
            Placed::Table(t) => (t.at, t.top),
            Placed::Unsure(u) => (u.at, u.top),
            Placed::Title(t) => (t.at, t.top),
            Placed::Spatial(s) => (s.at, s.top),
            Placed::Header { at, top, .. } => (*at, *top),
            Placed::Media { at, .. } => (*at, f32::MAX),
        }
    }
}

fn set_anchor(block: &mut Block, to: Anchor) {
    match block {
        Block::Heading { anchor, .. }
        | Block::Paragraph { anchor, .. }
        | Block::List { anchor, .. }
        | Block::Spatial { anchor, .. }
        | Block::Table { anchor, .. } => *anchor = Some(to),
        _ => {}
    }
}

/// Run and point pairs one page may test to find the blocks of annotations.
const PROBE_WORK: usize = 16_000_000;

/// How many runs or pieces of one block go by between two looks at the clock.
const CHECK_EVERY: usize = 1024;

/// Fails once the time the layout may take is over: the time limit less the time kept back
/// for writing as plain lines the pages not laid out yet (see [`Budget::keep_time`]; the
/// read sets it before each page). A cancel is not looked at here: the walk answers it, and
/// the pages it read are still laid out.
pub(crate) fn time_over(budget: &Budget) -> Result<(), LimitHit> {
    match budget.check_time_ahead() {
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
    /// For each block, by its place in `blocks`: the ids of the runs of the page's flow laid
    /// out in it (see [`Flow::ids`]); none for a block built from what a detector took.
    block_ids: Vec<Vec<u32>>,
    /// The ids of runs of empty lines passed over, for the next block laid out.
    unlaid: Vec<u32>,
    /// The ids of runs of a heading decision that the walk laid out as a paragraph for their
    /// geometry, with why (see [`untagged`]): their region is a paragraph.
    demoted: Vec<(u32, Demotion)>,
    /// The pictures and codes of the page's figure and code regions, each with its place in
    /// reading order and its decision.
    media_at: Vec<(layoutregions::PrimId, usize, usize)>,
    /// The pictures that stand in table cells (see [`Laid::in_cells`]).
    in_cells: Vec<(layoutregions::PrimId, usize, usize, usize)>,
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

    /// What was built, with the table detectors' `notes` and the page's `regions`.
    fn laid(self, notes: Vec<(String, LossReason, Severity)>, regions: PageRegions) -> Laid {
        Laid {
            blocks: self.blocks,
            stop: self.stop,
            cut: self.cut,
            continued: self.continued,
            tagged: self.tagged,
            added: self.added,
            notes,
            located: Vec::new(),
            probed: Vec::new(),
            probes_cut: false,
            found_by: None,
            title: None,
            dates: None,
            regions,
            engine_in: None,
            earlier: Vec::new(),
            run_blocks: Vec::new(),
            media_at: Vec::new(),
            in_cells: self.in_cells,
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

    /// Gives the runs `range` of the runs being laid out (`ids` their ids) and the runs passed
    /// over before them to the block built or joined last: they were laid out in it. Nothing
    /// without `ids` (the text of a table laid out as paragraphs) or once the page stopped.
    fn own(&mut self, ids: Option<&[u32]>, range: Range<usize>) {
        let Some(new) = ids else { return };
        if self.stop.is_some() {
            return;
        }
        let Some(k) = self.blocks.len().checked_sub(1) else {
            return;
        };
        if self.block_ids.len() <= k {
            self.block_ids.resize_with(k + 1, Vec::new);
        }
        let ids = &mut self.block_ids[k];
        ids.append(&mut self.unlaid);
        ids.extend_from_slice(&new[range]);
    }

    /// Notes the runs `range` of the runs being laid out (`ids` their ids), passed over as an
    /// empty line: the next block laid out takes them.
    fn pass_over(&mut self, ids: Option<&[u32]>, range: Range<usize>) {
        if let Some(ids) = ids {
            self.unlaid.extend_from_slice(&ids[range]);
        }
    }

    /// Gives the runs passed over at the end of the page to the last block laid out, if any.
    fn own_unlaid(&mut self) {
        if self.unlaid.is_empty() || self.stop.is_some() {
            return;
        }
        if let Some(ids) = self.block_ids.iter_mut().rev().find(|ids| !ids.is_empty()) {
            ids.append(&mut self.unlaid);
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

    /// A block of text kept as laid out (see [`Block::Spatial`]), its text charged; whether it
    /// was added (a block of no text is not).
    fn spatial(
        &mut self,
        lines: Vec<rezzer_model::SpatialLine>,
        links: Vec<String>,
        cx: &Context<'_>,
    ) -> bool {
        if self.stop.is_some() || lines.is_empty() {
            return false;
        }
        self.pending = Pending::Block;
        let text: usize = lines
            .iter()
            .flat_map(|l| &l.pieces)
            .map(|p| p.text.len())
            .sum();
        if let Err(hit) = time_over(cx.budget).and_then(|()| cx.budget.charge_text(text)) {
            self.stop_at(hit, cx);
            return false;
        }
        let at = self.blocks.len();
        self.push(
            Block::Spatial {
                lines,
                links,
                grid: None,
                anchor: None,
            },
            cx,
        );
        self.blocks.len() > at
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
        // Where each column ends: the cell lines' wrap test reads it. A table
        // whose cells all hold one line each has no lines to keep.
        let lined = plan
            .rows
            .iter()
            .flatten()
            .any(|c| c.runs.windows(2).any(|w| w[0].line != w[1].line));
        let ends = if lined {
            column_ends(&plan.rows, plan.drawn.map(|b| b[0].max(b[2])))
        } else {
            Vec::new()
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
            // Each cell's lines stay lines unless a line wraps.
            let lines = if !lined {
                vec![layoutregions::extract::table::lines::CellLines::default(); row.len()]
            } else {
                let runs: Vec<&[Run]> = row.iter().map(|c| c.runs.as_slice()).collect();
                let mut col = 0usize;
                let extents: Vec<f32> = row
                    .iter()
                    .map(|c| {
                        col += c.col_span.max(1) as usize;
                        ends.get(col - 1).copied().unwrap_or(f32::MAX)
                    })
                    .collect();
                layoutregions::extract::table::lines::cell_lines(&runs, &extents, &link_runs_on)
            };
            for (c, lines) in row.into_iter().zip(lines) {
                let Some(mut inlines) = self.lined_inlines(&c.runs, &lines, cx) else {
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
                    media: Vec::new(),
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
                // The pictures of its cells, in the rows that were built.
                let at = self.blocks.len();
                for &(r, c, pic) in &plan.pictures {
                    if rows.get(r).is_some_and(|row| c < row.len()) {
                        self.in_cells
                            .push((layoutregions::PrimId::Picture(pic), at, r, c));
                    }
                }
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
        tagged(&runs, &free, None, cx, self);
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

    /// The inlines of the runs of a cell, with a line break before each run `lines` names and
    /// its empty lines before the first (see
    /// [`layoutregions::extract::table::lines::cell_lines`]), or `None` once a limit stopped the
    /// page.
    fn lined_inlines(
        &mut self,
        runs: &[Run],
        lines: &layoutregions::extract::table::lines::CellLines,
        cx: &Context<'_>,
    ) -> Option<Vec<Inline>> {
        if lines.broken.is_empty() && lines.glued.is_empty() && lines.lead == 0 {
            return self.cell_inlines(runs, cx);
        }
        let charged = time_over(cx.budget).and_then(|()| cx.budget.add_elements(1));
        if let Err(hit) = charged {
            self.stop_at(hit, cx);
            return None;
        }
        let lines_at = |at: &[usize]| {
            let mut all: Vec<(u32, u32)> = at
                .iter()
                .filter_map(|&k| runs.get(k).map(|r| (r.page, r.line)))
                .collect();
            all.sort_unstable();
            all.dedup();
            all
        };
        let (breaks, glued) = (lines_at(&lines.broken), lines_at(&lines.glued));
        let refs: Vec<&Run> = runs.iter().collect();
        let mut out: Vec<Inline> = std::iter::repeat_n(Inline::LineBreak, lines.lead).collect();
        out.extend(self.joined_inlines(&refs, &breaks, &glued, cx)?);
        Some(out)
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
        self.joined_inlines(runs, breaks, &[], cx)
    }

    /// [`Built::broken_inlines`] with each line `glued` names (page and line, sorted) going on
    /// right after the line before it, with no space: an address that runs on.
    fn joined_inlines(
        &mut self,
        runs: &[&Run],
        breaks: &[(u32, u32)],
        glued: &[(u32, u32)],
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
        match inlines(runs, breaks, glued, cx.links, cx.budget) {
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

/// Where each column of `rows` ends, for the wrap test of its cells' lines: where the text of
/// the next column starts (its leftmost run over the table), and for the last column the
/// table's right edge: the right end of its drawn rules or frame (`edge`), else of its widest
/// row.
fn column_ends(rows: &[Vec<PlannedCell>], edge: Option<f32>) -> Vec<f32> {
    let mut starts: Vec<f32> = Vec::new();
    let mut right = f32::MIN;
    for row in rows {
        let mut col = 0usize;
        for c in row {
            if starts.len() <= col {
                starts.resize(col + 1, f32::MAX);
            }
            for r in &c.runs {
                if !r.text.trim().is_empty() {
                    starts[col] = starts[col].min(r.x);
                    right = right.max(r.x + r.w);
                }
            }
            col += c.col_span.max(1) as usize;
        }
    }
    let right = edge.map_or(right, |e| e.max(right));
    let n = starts.len();
    (0..n)
        .map(|c| {
            starts[c + 1..]
                .iter()
                .copied()
                .find(|x| *x < f32::MAX)
                .unwrap_or(right)
        })
        .collect()
}

/// How a link goes on from the last run `a` of a line of a cell to the first run `b` of the
/// next: both under one target. Its text is its address going on when the two words that meet
/// stand in the target one after the other (`.../muster-` over `seite.pdf`).
fn link_runs_on(a: &Run, b: &Run) -> layoutregions::extract::table::lines::RunsOn {
    use layoutregions::extract::table::lines::RunsOn;
    let (Some(ua), Some(ub)) = (&a.uri, &b.uri) else {
        return RunsOn::Apart;
    };
    if !Rc::ptr_eq(ua, ub) && ua.uri != ub.uri {
        return RunsOn::Apart;
    }
    let end = a.text.split_whitespace().next_back().unwrap_or("");
    let start = b.text.split_whitespace().next().unwrap_or("");
    if end.is_empty() || start.is_empty() {
        return RunsOn::Words;
    }
    if ua.uri.contains(&format!("{end}{start}")) {
        RunsOn::Address
    } else {
        RunsOn::Words
    }
}

/// Inlines from runs in order. Runs on a new line are joined with a space, or without one
/// after a hyphen that ends a word (not after a dash with white space before it); a line
/// `breaks` names (page, line; sorted) starts after a line break, and a line `glued` names goes
/// on right after the line before it (an address that runs on).
/// Bold, italic and URI links are kept while `links` has room for their targets; past it a
/// link's text stays without the link.
fn inlines(
    runs: &[&Run],
    breaks: &[(u32, u32)],
    glued: &[(u32, u32)],
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
        // does after the minus of an amount before a word (`12,50-` over `gutgeschrieben`). An
        // amount's hyphen before a number is a range that goes on (`08.00-` over `18.00 Uhr`).
        let range = text.starts_with(|c: char| c.is_ascii_digit());
        let joined = pieces.last().is_none_or(|p| {
            p.text.ends_with(' ')
                || ((range || !signed_amount(&p.text))
                    && p.text.strip_suffix('-').is_some_and(|rest| {
                        !rest.is_empty() && !rest.ends_with(char::is_whitespace)
                    }))
        });
        // An address that runs on has nothing between its lines.
        let address = new_line && !broken && glued.binary_search(&(r.page, r.line)).is_ok();
        if address && let Some(p) = pieces.last_mut() {
            let end = p.text.trim_end().len();
            p.text.truncate(end);
        }
        if new_line && !joined && !broken && !address {
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

/// The blocks of a running footer, in reading order. Footer lines that go on one another at
/// one left edge are one block. A footer set in columns (an address, a bank, a register entry,
/// a contact) is read column by column (on furniture lines only): the ink of its
/// lines falls into columns apart by white space, and the parts of the lines in each column
/// are blocks of their own, when every block of three lines or more is an [`address_block`]
/// and one is. A line of its own in one piece (a page number centred over or under the
/// columns) takes no part in finding the columns: it would bridge their gutter. It is read whole, before the
/// columns when it stands before them in the reading order, else after them. Otherwise the
/// lines stay whole, as the reading order gives them. Each line of
/// a block is a line of its own: a footer lists facts (`USt-IdNr. DE000000000`,
/// `www.muster.example`), it is no prose to join; only a sentence that wraps goes on (see
/// [`footer_wrap`]).
///
/// With the blocks, which of them are columns that a paragraph wrapped in one of them shows
/// (see [`paragraph_columns`]): in those no line goes on, not even a sentence that wraps.
/// What shows such columns the rows of a table can show too, and with its lines kept a
/// column of a table read as a block loses the order of the rows only, never where an item
/// ends.
fn footer_blocks<'a>(all: &[Line<'a>]) -> (Vec<Vec<Line<'a>>>, Range<usize>) {
    let whole = || (stacked(all.iter().map(|l| line(l.runs)).collect()), 0..0);
    // A footer in columns that a paragraph wrapped in one of them shows is read column by
    // column, whatever its blocks hold: the lines of that paragraph are no rows. The lines
    // before and after the columns (a page number under them) are read as they are.
    let mut at = 0;
    for b in whole().0 {
        let (from, to) = (at, at + b.len());
        at = to;
        if let Some(columns) = wrapped_columns(&all[from..to]) {
            let stack = |lines: &[Line<'a>]| stacked(lines.iter().map(|l| line(l.runs)).collect());
            let mut out = stack(&all[..from]);
            let own = out.len()..out.len() + columns.len();
            out.extend(columns);
            out.extend(stack(&all[to..]));
            return (out, own);
        }
    }
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
    (out, 0..0)
}

/// The lines of a footer column by column, each column's lines top down, when they are blocks
/// side by side that a paragraph wrapped in one of them shows (see [`paragraph_columns`]).
fn wrapped_columns<'a>(lines: &[Line<'a>]) -> Option<Vec<Vec<Line<'a>>>> {
    let mut runs: Vec<&Run> = Vec::new();
    let mut starts = Vec::with_capacity(lines.len());
    let mut pieces = Vec::with_capacity(lines.len());
    for l in lines {
        let start = runs.len();
        runs.extend(l.runs.iter());
        pieces.push(layoutregions::extract::table::grid::pieces_of(
            &runs,
            start,
            runs.len(),
        ));
        starts.push(start);
    }
    let cells = paragraph_columns(&runs, &pieces)?;
    let column = |cells: Vec<Vec<usize>>| -> Vec<Line<'a>> {
        cells
            .iter()
            .enumerate()
            .filter_map(|(row, cell)| {
                let (first, last) = (*cell.first()?, *cell.last()?);
                Some(line(
                    &lines[row].runs[first - starts[row]..last + 1 - starts[row]],
                ))
            })
            .collect()
    };
    Some(cells.into_iter().map(column).collect())
}

/// Lines in reading order cut into blocks of lines that go on one another at one left edge.
fn stacked(lines: Vec<Line<'_>>) -> Vec<Vec<Line<'_>>> {
    let mut out: Vec<Vec<Line<'_>>> = Vec::new();
    for l in lines {
        match out.last_mut() {
            Some(b)
                if continues(&b[b.len() - 1], &l)
                    && (l.x - b[0].x).abs() <= EDGE_SIZES * l.size.max(b[0].size) =>
            {
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

/// The places in `flow` of the runs of `block`, lines of `flow`.
fn span(flow: &[Run], block: &[Line<'_>]) -> Range<usize> {
    let start = |l: &Line<'_>| {
        (l.runs.as_ptr() as usize - flow.as_ptr() as usize) / std::mem::size_of::<Run>()
    };
    match (block.first(), block.last()) {
        (Some(a), Some(z)) => start(a)..start(z) + z.runs.len(),
        _ => 0..0,
    }
}

/// Lays out the runs of an untagged page (`ids` their ids, if they are the page's flow) as
/// headings, lists and paragraphs, block by block as [`next_block`] reads them: headings by
/// size (a line in one of the page's `text_sizes`, or less than [`file::HEADING_RATIO`] times the
/// largest of them, is no heading), lists by marker, paragraphs by gaps. `kind` is the kind the
/// region engine decided the runs as, when it decided one: a heading decision makes each block
/// a heading (at the level the walk reads, else the lowest), a paragraph decision makes each a
/// paragraph (an item with its marker), a list decision keeps the walk's items and makes its
/// headings paragraphs. `own_line` says whether a run stands in blocks side by side read as
/// columns: a line of such a block stays a line of its own (see `tables::Found::own_lines`).
fn untagged(
    flow: &[Run],
    ids: Option<&[u32]>,
    text_sizes: &[f32],
    (key_of, own_line): (&dyn Fn(u32) -> WalkKey, &dyn Fn(u32) -> bool),
    cx: &Context<'_>,
    out: &mut Built,
) {
    let levels = cx.style.levels(text_sizes);
    let all = lines(flow);
    // The kind of the decision and the drawn box of each line (`key_of`, by the id of the
    // line's first run; none without `ids`). A line goes on the line before it by the walk, and
    // only with the same kind and box: the text of a page never holds the cells of a table its
    // readers left (they are kept as laid out).
    let line_keys: Vec<WalkKey> = all
        .iter()
        .map(|l| {
            let at = span(flow, std::slice::from_ref(l)).start;
            ids.and_then(|ids| ids.get(at))
                .map_or((None, None, None), |&id| key_of(id))
        })
        .collect();
    let mut keys: HashMap<WalkKey, usize> = HashMap::new();
    let block: Vec<Option<usize>> = line_keys
        .iter()
        .map(|k| {
            let n = keys.len();
            Some(*keys.entry(*k).or_insert(n))
        })
        .collect();
    // The lines of blocks side by side read as columns: each stays a line of its own.
    let own: Vec<bool> = all
        .iter()
        .map(|l| {
            let at = span(flow, std::slice::from_ref(l)).start;
            ids.and_then(|ids| ids.get(at))
                .is_some_and(|&id| own_line(id))
        })
        .collect();
    let text = TextCx {
        levels: &levels,
        block: &block,
        own: &own,
    };
    let mut i = 0;
    let mut open_list: Option<Option<usize>> = None;
    while i < all.len() && out.stop.is_none() {
        let (found, j) = next_block(&all, i, open_list.is_some(), &text);
        let kind = line_keys[i].0;
        let heading = kind == Some(Kind::Heading);
        let paragraph = matches!(kind, Some(Kind::Paragraph | Kind::List));
        let runs = || -> Vec<&Run> { all[i..j].iter().flat_map(|l| l.runs.iter()).collect() };
        let lowest = || levels.level(&all[i]).unwrap_or(SIZE_LEVELS as u8);
        match found {
            TextBlock::Empty => {
                out.pass_over(ids, span(flow, &all[i..j]));
                i = j;
                continue;
            }
            TextBlock::Heading {
                level,
                demoted: None,
                ..
            } if !paragraph => out.heading(level, runs(), cx),
            // A heading decision makes a heading of lines the walk demoted for what they say
            // (an address, sentences), not of lines it demoted for their geometry: lines apart,
            // unwrapped or in cells keep their breaks, as the walk reads them with the lines
            // of their column around them (a box may have cut the decision's segment off them).
            TextBlock::Heading {
                level,
                demoted: Some(d),
                ..
            } if heading
                && !matches!(
                    d,
                    Demotion::LinesApart | Demotion::Unwrapped | Demotion::Cells
                ) =>
            {
                out.heading(level, runs(), cx);
            }
            // Lines demoted for their geometry under a heading decision: a paragraph with its
            // breaks, and its runs said, so their region is the paragraph written.
            TextBlock::Heading {
                breaks,
                demoted: Some(d),
                ..
            } if heading => {
                out.broken_paragraph(runs(), &breaks, None, cx);
                if let Some(ids) = ids
                    && out.stop.is_none()
                {
                    let span = span(flow, &all[i..j]);
                    out.demoted.extend(ids[span].iter().map(|&id| (id, d)));
                }
            }
            TextBlock::Paragraph { .. } if heading => out.heading(lowest(), runs(), cx),
            TextBlock::Heading { breaks, .. } | TextBlock::Paragraph { breaks } => {
                out.broken_paragraph(runs(), &breaks, None, cx);
            }
            TextBlock::Item { .. } if heading => out.heading(lowest(), runs(), cx),
            TextBlock::Item { .. } if kind == Some(Kind::Paragraph) => {
                out.paragraph(runs(), None, cx);
            }
            TextBlock::Item {
                ordered,
                cut,
                joins,
            } => {
                let owned = without_marker(&all[i].runs.iter().collect::<Vec<_>>(), cut);
                let mut runs: Vec<&Run> = owned.iter().collect();
                runs.extend(all[i + 1..j].iter().flat_map(|l| l.runs.iter()));
                if !joins {
                    open_list = None;
                }
                out.item(None, ordered, runs, cx, &mut open_list);
                out.own(ids, span(flow, &all[i..j]));
                i = j;
                continue;
            }
        }
        open_list = None;
        out.own(ids, span(flow, &all[i..j]));
        i = j;
    }
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

/// Places the paragraph elements of a tagged page that stand in columns the page shows in
/// column order: each column top to bottom, the columns left to right. The tags of such a
/// block (a footer of a company beside its managers beside its board) often give one
/// paragraph per line, row by row across the columns; read in that order a line of one column
/// stands between the lines of another, which breaks what each column says.
///
/// The columns are the page's own evidence, asked of the shared rule the untagged path asks
/// (see [`paragraph_columns`]: a first line whose pieces start at one pitch, later lines that
/// share those starts, each column one block, one of them a paragraph wrapped over every row
/// with another block beside it down to its last row, so the rows are no rows of a table;
/// without that the order of the tags stays). The lines asked are rows by their baselines, whatever
/// order the runs came in (the page's order may have read one of the columns apart already):
/// a first row and the rows under it at the step of the first two (within an edge, see
/// [`EDGE_SIZES`]) that start at one of its starts, among runs that follow each other and are
/// plain paragraph elements.
///
/// What the tags state stays: no element is joined or split, only the order of the elements
/// changes. A block is left as it is when an element of it holds runs of two columns, or
/// runs outside the block, or would not stay in one piece, or when other runs stand between
/// its runs in the page's order. Each cell of the block becomes a line of its own in the new
/// order, and the lines are numbered on from there, as the lines of columns are numbered on
/// an untagged page.
fn tagged_columns(runs: &mut Vec<Run>, slots: &mut Vec<Slot>) {
    let plain = |i: usize| matches!(slots[i], Slot::Para(_, None)) && !runs[i].off_axis;
    let element = |i: usize| match slots[i] {
        Slot::Para(e, _) => Some(e),
        _ => None,
    };
    // How often each paragraph element stands on the page: an element of a block must
    // stand in the block with all its runs.
    let mut count: HashMap<usize, usize> = HashMap::new();
    for e in (0..runs.len()).filter_map(element) {
        *count.entry(e).or_default() += 1;
    }
    crate::work::add(runs.len() as u64);
    // The blocks to move: where each starts in the page's order, and its runs cell by cell.
    let mut blocks: Vec<(usize, Vec<Vec<usize>>)> = Vec::new();
    let mut from = 0;
    while from < runs.len() {
        // Plain paragraph runs of one page that follow each other.
        let mut to = from;
        while to < runs.len() && plain(to) && runs[to].page == runs[from].page {
            to += 1;
        }
        if to == from {
            from += 1;
            continue;
        }
        // Their rows, top down: runs on one baseline (within an edge), left to right.
        let mut by_place: Vec<usize> = (from..to).collect();
        by_place.sort_by(|&a, &b| {
            runs[b]
                .y
                .total_cmp(&runs[a].y)
                .then(runs[a].x.total_cmp(&runs[b].x))
        });
        let mut rows: Vec<Vec<usize>> = Vec::new();
        for i in by_place {
            match rows.last_mut() {
                Some(row) if runs[row[0]].y - runs[i].y <= EDGE_SIZES * runs[i].size => row.push(i),
                _ => rows.push(vec![i]),
            }
        }
        crate::work::add((to - from) as u64);
        let mut at = 0;
        while at < rows.len() {
            let end = column_block(runs, &rows, at);
            if end - at < 3 {
                at += 1;
                continue;
            }
            let ids: Vec<usize> = rows[at..end].iter().flatten().copied().collect();
            let refs: Vec<&Run> = ids.iter().map(|&i| &runs[i]).collect();
            let mut start = 0;
            let pieces: Vec<Vec<_>> = rows[at..end]
                .iter()
                .map(|row| {
                    let (a, z) = (start, start + row.len());
                    start = z;
                    layoutregions::extract::table::grid::pieces_of(&refs, a, z)
                })
                .collect();
            let (first, last) = ids
                .iter()
                .fold((usize::MAX, 0), |(a, z), &i| (a.min(i), z.max(i)));
            let together = last + 1 - first == ids.len();
            let cells = paragraph_columns(&refs, &pieces).filter(|cells| {
                // Each element in one column, with all its runs, in one piece.
                let mut seen: HashMap<usize, (usize, usize)> = HashMap::new();
                let mut before = None;
                together
                    && cells.iter().enumerate().all(|(c, column)| {
                        column.iter().flatten().all(|&i| {
                            let Some(e) = element(ids[i]) else {
                                return false;
                            };
                            let (column_of, n) = seen.entry(e).or_insert((c, 0));
                            *n += 1;
                            let whole = *column_of == c && (*n == 1 || before == Some(e));
                            before = Some(e);
                            whole
                        })
                    })
                    && seen.iter().all(|(e, (_, n))| count.get(e) == Some(n))
            });
            if let Some(cells) = cells {
                let cells = cells
                    .into_iter()
                    .flatten()
                    .filter(|cell| !cell.is_empty())
                    .map(|cell| cell.into_iter().map(|i| ids[i]).collect())
                    .collect();
                blocks.push((first, cells));
            }
            at = end;
        }
        from = to;
    }
    if blocks.is_empty() {
        return;
    }
    blocks.sort_by_key(|b| b.0);
    // The new order: each run with a mark that is new where a line starts.
    let mut order: Vec<(usize, (u32, u32))> = Vec::with_capacity(runs.len());
    let mut blocks = blocks.into_iter().peekable();
    let mut i = 0;
    while i < runs.len() {
        match blocks.next_if(|b| b.0 == i) {
            Some((_, cells)) => {
                for (n, cell) in cells.iter().enumerate() {
                    order.extend(cell.iter().map(|&k| (k, (1, n as u32))));
                }
                i += cells.iter().map(Vec::len).sum::<usize>();
            }
            None => {
                order.push((i, (0, runs[i].line)));
                i += 1;
            }
        }
    }
    let mut old: Vec<Option<(Run, Slot)>> = std::mem::take(runs)
        .into_iter()
        .zip(std::mem::take(slots))
        .map(Some)
        .collect();
    let first = old.iter().flatten().map(|(r, _)| r.line).min().unwrap_or(0);
    let mut line = first;
    let mut last = None;
    for (i, mark) in order {
        if let Some((mut r, slot)) = old[i].take() {
            if last.is_some_and(|l| l != mark) {
                line += 1;
            }
            last = Some(mark);
            r.line = line;
            runs.push(r);
            slots.push(slot);
        }
    }
}

/// Where the block of rows that may be columns under row `at` ends (see
/// [`tagged_columns`]): the rows under it at the step of its first two (within an edge) that
/// start at the start of one of its pieces. `rows` hold the runs of each row, left to right.
fn column_block(runs: &[Run], rows: &[Vec<usize>], at: usize) -> usize {
    let head = |row: usize| &runs[rows[row][0]];
    let mut end = at + 1;
    if end >= rows.len() {
        return end;
    }
    let edge = EDGE_SIZES * head(at).size;
    let step = head(at).y - head(end).y;
    let refs: Vec<&Run> = rows[at].iter().map(|&i| &runs[i]).collect();
    let starts = layoutregions::extract::table::grid::pieces_of(&refs, 0, refs.len());
    while end < rows.len()
        && (head(end - 1).y - head(end).y - step).abs() <= edge
        && starts.iter().any(|p| (head(end).x - p.x0).abs() <= edge)
    {
        end += 1;
    }
    crate::work::add((refs.len() + end - at) as u64);
    end
}

/// Lays out runs of a tagged page with their slots (`ids` their ids, if they are the page's
/// flow): runs of one element form one block, in reading order. Table cells are laid out as
/// tables before; a cell run left here is text.
fn tagged(flow: &[Run], slots: &[Slot], ids: Option<&[u32]>, cx: &Context<'_>, out: &mut Built) {
    let mut open_list: Option<Option<usize>> = None;
    let mut current: Vec<&Run> = Vec::new();
    let mut current_slot = Slot::Free;
    // The flow indices of the first and the last run in `current`.
    let mut current_first = 0;
    let mut current_last = 0;
    let all_lines = lines(flow);
    // The line of each run, to test gaps between free runs.
    let mut line_of = Vec::with_capacity(flow.len());
    for (n, l) in all_lines.iter().enumerate() {
        line_of.extend(std::iter::repeat_n(n, l.runs.len()));
    }
    let flush = |out: &mut Built,
                 current: &mut Vec<&Run>,
                 slot: Slot,
                 open: &mut Option<Option<usize>>,
                 range: Range<usize>| {
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
        out.own(ids, range);
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
            let range = current_first..current_last + 1;
            flush(out, &mut current, current_slot, &mut open_list, range);
            current_slot = slot;
        }
        if current.is_empty() {
            current_first = i;
        }
        current.push(r);
        current_last = i;
    }
    let range = current_first..current_last + 1;
    flush(out, &mut current, current_slot, &mut open_list, range);
}

#[cfg(test)]
mod tests {
    use layoutregions::PrimId;
    use pdfparts::StructElem;

    use super::*;

    /// A page that draws nothing: no rules, so the empty structure is theirs (see
    /// `engine::tests::no_structure_is_that_of_no_rules`).
    const NOTHING_DRAWN: Drawn<'static> = Drawn {
        rules: &[],
        structure: &crate::engine::NO_STRUCTURE,
        images: &[],
        widgets: &[],
        boxes: &[],
        items: None,
        lines_unsearched: false,
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
            family: 0,
            mcid: None,
            uri: None,
            line,
            off_axis: false,
            note: false,
            gaps: Gaps::default(),
        }
    }

    /// A block of three columns at the starts 100, 200 and 300 in 10 points, the lines 12
    /// points apart: a title line over a first column of three lines, a middle column of one
    /// paragraph wrapped over three lines (each full, the next opens with a small letter) and
    /// a last column of one line. Each cell is one run; the runs stand row by row.
    fn column_runs() -> Vec<Run> {
        let rows: [&[(&str, f32, f32)]; 4] = [
            &[
                ("One", 100.0, 30.0),
                ("Two", 200.0, 30.0),
                ("Three", 300.0, 30.0),
            ],
            &[
                ("first", 100.0, 30.0),
                ("Alpha beta gamma", 200.0, 90.0),
                ("last", 300.0, 30.0),
            ],
            &[("second", 100.0, 30.0), ("delta epsilon", 200.0, 98.0)],
            &[("third", 100.0, 30.0), ("zeta", 200.0, 20.0)],
        ];
        let mut out = Vec::new();
        for (n, row) in rows.iter().enumerate() {
            for (text, x, w) in row.iter() {
                out.push(Run {
                    x: *x,
                    w: *w,
                    ..run(text, n as u32, 700.0 - 12.0 * n as f32, 10.0)
                });
            }
        }
        out
    }

    /// The texts of `runs` with their lines after [`tagged_columns`] with `slots`.
    fn column_order(mut runs: Vec<Run>, mut slots: Vec<Slot>) -> Vec<(String, u32)> {
        tagged_columns(&mut runs, &mut slots);
        assert_eq!(runs.len(), slots.len());
        runs.into_iter().map(|r| (r.text, r.line)).collect()
    }

    /// Paragraph elements in columns the page
    /// shows are placed column by column, and only then.
    #[test]
    fn tagged_paragraphs_in_columns_are_placed_column_by_column() {
        let runs = column_runs();
        let rows: Vec<(String, u32)> = runs.iter().map(|r| (r.text.clone(), r.line)).collect();
        let each: Vec<Slot> = (0..runs.len()).map(|e| Slot::Para(e, None)).collect();
        let texts = |order: &[(String, u32)]| -> Vec<String> {
            order.iter().map(|(t, _)| t.clone()).collect()
        };
        // One element per cell: column by column, each cell a line of its own.
        let moved = column_order(runs.clone(), each.clone());
        assert_eq!(
            texts(&moved),
            [
                "One",
                "first",
                "second",
                "third",
                "Two",
                "Alpha beta gamma",
                "delta epsilon",
                "zeta",
                "Three",
                "last"
            ]
        );
        let lines: Vec<u32> = moved.iter().map(|(_, l)| *l).collect();
        assert_eq!(lines, (0..10).collect::<Vec<u32>>());
        // A line after the block (at no column start) stays after it, numbered on.
        let mut more = runs.clone();
        more.push(Run {
            x: 400.0,
            w: 30.0,
            ..run("Sheet 1", 4, 652.0, 10.0)
        });
        let mut slots = each.clone();
        slots.push(Slot::Para(99, None));
        let moved = column_order(more, slots);
        assert_eq!(moved.last(), Some(&("Sheet 1".to_string(), 10)));
        assert_eq!(moved[1].0, "first");
        // The page's order read the first column apart already (its four lines, then the
        // rows of the other two): the rows are found by their baselines, the same order.
        let mut cut: Vec<Run> = [0, 3, 6, 8, 1, 2, 4, 5, 7, 9]
            .iter()
            .map(|&i| runs[i].clone())
            .collect();
        for (r, line) in cut.iter_mut().zip([0, 1, 2, 3, 4, 4, 5, 5, 6, 7]) {
            r.line = line;
        }
        let slots: Vec<Slot> = (0..cut.len()).map(|e| Slot::Para(e, None)).collect();
        assert_eq!(
            column_order(cut, slots),
            column_order(runs.clone(), each.clone())
        );
        // One element per column (its lines one paragraph): the same order, no element cut.
        let by_column = [0, 1, 2, 0, 1, 2, 0, 1, 0, 1].map(|e| Slot::Para(e, None));
        assert_eq!(
            texts(&column_order(runs.clone(), by_column.to_vec()))[1],
            "first"
        );
        // The tags state rows (one element per line across the columns): left as given.
        let by_row = [0, 0, 0, 1, 1, 1, 2, 2, 3, 3].map(|e| Slot::Para(e, None));
        assert_eq!(column_order(runs.clone(), by_row.to_vec()), rows);
        // An element of the block holds a run outside it: left as given.
        let mut over = vec![Run {
            x: 100.0,
            w: 300.0,
            ..run("A line over the block.", 0, 740.0, 10.0)
        }];
        over.extend(runs.iter().cloned().map(|r| Run {
            line: r.line + 1,
            ..r
        }));
        let mut slots = vec![Slot::Para(3, None)];
        slots.extend(each.iter().copied());
        let given: Vec<(String, u32)> = over.iter().map(|r| (r.text.clone(), r.line)).collect();
        assert_eq!(column_order(over.clone(), slots.clone()), given);
        // The same line as an element of its own: the block under it moves.
        slots[0] = Slot::Para(98, None);
        assert_eq!(column_order(over, slots)[2].0, "first");
        // A heading, a list item, a note or an untagged run in the block: left as given.
        for other in [
            Slot::Heading(2, 4),
            Slot::Item { list: 50, item: 4 },
            Slot::Para(4, Some("footnote")),
            Slot::Free,
        ] {
            let mut slots = each.clone();
            slots[4] = other;
            assert_eq!(column_order(runs.clone(), slots), rows, "{other:?}");
        }
        // No paragraph wrapped in a column (the middle lines are short): rows, left as given.
        let mut short = runs.clone();
        short[7].w = 40.0;
        assert_eq!(column_order(short, each.clone()), rows);
        // The lines under the first at another step are no lines of the block.
        let mut loose = runs.clone();
        for r in loose.iter_mut().filter(|r| r.line >= 2) {
            r.y -= 6.0;
        }
        let given: Vec<(String, u32)> = loose.iter().map(|r| (r.text.clone(), r.line)).collect();
        assert_eq!(column_order(loose, each.clone()), given);
        // A step within an edge of the first (1 point at 10 points) is the same step.
        for (drop, moves) in [(0.9, true), (1.1, false)] {
            let mut near = runs.clone();
            for r in near.iter_mut().filter(|r| r.line >= 2) {
                r.y -= drop;
            }
            let read = column_order(near, each.clone());
            assert_eq!(read[1].0 == "first", moves, "{drop}");
        }
    }

    /// A table without rules tagged as one paragraph
    /// per cell, row by row. Its columns start at one pitch (50, 200, 350), its descriptions
    /// fill their column and open with a small letter, and the last one wraps. The order of
    /// the tags is the meaning (each item with its description and its source) and stays,
    /// also with a cell left empty at the end of the word column: no other column reaches
    /// the last row of the "paragraph". A block that does reach it is the other way.
    #[test]
    fn tagged_cells_of_a_table_keep_the_order_of_their_tags() {
        let table = |rows: &[(&str, &str, f32, &str)]| {
            let mut out = Vec::new();
            for (n, (a, b, wb, c)) in rows.iter().enumerate() {
                for (text, x, w) in [
                    (*a, 50.0f32, 6.0 * a.len() as f32),
                    (*b, 200.0, *wb),
                    (*c, 350.0, 6.0 * c.len() as f32),
                ] {
                    if !text.is_empty() {
                        out.push(Run {
                            x,
                            w,
                            ..run(text, n as u32, 700.0 - 12.0 * n as f32, 10.0)
                        });
                    }
                }
            }
            out
        };
        let moved = |runs: Vec<Run>| {
            let slots: Vec<Slot> = (0..runs.len()).map(|e| Slot::Para(e, None)).collect();
            let given: Vec<String> = runs.iter().map(|r| r.text.clone()).collect();
            let read: Vec<String> = column_order(runs, slots)
                .into_iter()
                .map(|(t, _)| t)
                .collect();
            read != given
        };
        let head = ("Item", "Description", 66.0, "Source");
        let bolt = |source| {
            (
                "Bolt",
                "steel bolt for the frame, zinc plated",
                142.0,
                source,
            )
        };
        let nut = |source| {
            (
                "Nut",
                "nut for the bolt, zinc plated, size M8",
                141.0,
                source,
            )
        };
        let washer = |source| {
            (
                "Washer",
                "washer for the nut and the bolt, set",
                142.0,
                source,
            )
        };
        let end = ("", "twelve pieces", 60.0, "");
        // Every cell filled; the last source empty; two items; a source on
        // the first item only; the last description over two more lines.
        for (name, rows) in [
            (
                "full",
                vec![head, bolt("Mill"), nut("Forge"), washer("Yard"), end],
            ),
            ("A", vec![head, bolt("Mill"), nut("Forge"), washer(""), end]),
            ("C", vec![head, bolt("Mill"), nut(""), end]),
            ("D", vec![head, bolt("Mill"), nut(""), washer(""), end]),
            (
                "16",
                vec![
                    head,
                    bolt("Mill"),
                    nut("Forge"),
                    washer(""),
                    ("", "twelve pieces, zinc plated, in a paper", 142.0, ""),
                    ("", "bag", 18.0, ""),
                ],
            ),
        ] {
            assert!(!moved(table(&rows)), "{name}");
        }
        // The other way: the first column goes on beside the text to its last row, the
        // third ends early. These are blocks side by side and read column by column.
        assert!(moved(table(&[
            head,
            bolt("Mill"),
            nut(""),
            ("Washer", "washer for the nut and the bolt, set", 142.0, ""),
            ("Spring", "twelve pieces", 60.0, ""),
        ])));
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

    /// An address under one link that runs on across a line end of a cell
    /// is one link with its whole text, nothing between its parts; linked words that run on
    /// join with a space; the line after the link stays a line.
    #[test]
    fn an_address_that_runs_on_in_a_cell_is_one_link() {
        use layoutregions::extract::table::lines::{RunsOn, cell_lines};
        let target = UriTarget::new("https://example.org/muster/seite-eins.pdf");
        let cell = |first: &str, second: &str| -> Vec<Run> {
            [
                ("Siehe ", None, 0, 72.0),
                (first, Some(&target), 0, 110.0),
                (second, Some(&target), 1, 72.0),
                ("Stand 2026", None, 2, 72.0),
            ]
            .into_iter()
            .map(|(t, u, line, x)| Run {
                uri: u.cloned(),
                x,
                w: 5.0 * t.len() as f32,
                ..run(t, line, 700.0 - 12.0 * line as f32, 10.0)
            })
            .collect()
        };
        let read = |runs: &[Run]| {
            let lines = cell_lines(&[runs], &[400.0], &link_runs_on);
            let at = |k: &[usize]| -> Vec<(u32, u32)> {
                k.iter().map(|&k| (runs[k].page, runs[k].line)).collect()
            };
            let refs: Vec<&Run> = runs.iter().collect();
            inlines(
                &refs,
                &at(&lines[0].broken),
                &at(&lines[0].glued),
                &LinkRoom::new(u64::MAX),
                &Budget::new(rezzer_limits::Limits::default()),
            )
            .unwrap()
        };
        let runs = cell("https://example.org/muster/", "seite-eins.pdf");
        assert_eq!(link_runs_on(&runs[1], &runs[2]), RunsOn::Address);
        assert_eq!(link_runs_on(&runs[0], &runs[1]), RunsOn::Apart);
        let built = read(&runs);
        assert_eq!(built.len(), 4, "{built:?}");
        match &built[1] {
            Inline::Link {
                target, inlines, ..
            } => {
                assert_eq!(target, "https://example.org/muster/seite-eins.pdf");
                assert_eq!(
                    rezzer_model::plain_text(inlines),
                    "https://example.org/muster/seite-eins.pdf"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(built[2], Inline::LineBreak), "{built:?}");
        // Words under one link: one link, a space between its lines.
        let runs = cell("die Seite", "eins");
        assert_eq!(link_runs_on(&runs[1], &runs[2]), RunsOn::Words);
        let built = read(&runs);
        assert_eq!(built.len(), 4, "{built:?}");
        match &built[1] {
            Inline::Link { inlines, .. } => {
                assert_eq!(rezzer_model::plain_text(inlines), "die Seite eins")
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
        let built = inlines(
            &refs,
            &[],
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

    #[test]
    fn a_dropped_block_gives_back_its_link_counts() {
        let a = UriTarget::new("https://example.org/a");
        let runs = vec![Run {
            uri: Some(a),
            ..run("link", 0, 700.0, 10.0)
        }];
        let style = Style::new(&[PageText {
            page: 1,
            height: 842.0,
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
            height: 842.0,
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
            kind: layoutregions::Kind::Table,
            ids: Vec::new(),
            by: crate::tables::Detector::Grid,
            drawn: None,
            pictures: Vec::new(),
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
            height: 842.0,
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
            kind: layoutregions::Kind::Table,
            ids: Vec::new(),
            by: crate::tables::Detector::Grid,
            drawn: None,
            pictures: Vec::new(),
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
            height: 842.0,
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
            kind: layoutregions::Kind::Table,
            ids: Vec::new(),
            by: crate::tables::Detector::Grid,
            drawn: None,
            pictures: Vec::new(),
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
            height: 842.0,
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

    /// A note set up the margin (text across the page's direction) that a
    /// flow holds between the lines of a paragraph reads after the body, and the paragraph
    /// stays whole. The untagged text pass puts such runs after the page's main text already,
    /// so on its flows this changes nothing (no measured page changes); it holds the order of
    /// `Places` for a flow that does not.
    #[test]
    fn text_across_the_page_reads_after_the_body() {
        let runs = vec![
            run("The first line of a paragraph that goes on", 0, 700.0, 10.0),
            Run {
                x: 20.0,
                w: 10.0,
                off_axis: true,
                ..run("Note up the margin", 1, 400.0, 10.0)
            },
            run("and ends on its second line here.", 2, 688.0, 10.0),
        ];
        let laid = laid_page(runs, &NOTHING_DRAWN);
        let texts: Vec<String> = laid
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph { inlines, .. } => Some(rezzer_model::plain_text(inlines)),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts,
            [
                "The first line of a paragraph that goes on and ends on its second line here.",
                "Note up the margin"
            ]
        );
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
            height: 842.0,
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
        let laid = page_blocks(1, 842.0, runs, &NOTHING_DRAWN, &cx);
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

    /// A run of a table cell at `x`, `y`, 8 points, bold or not.
    fn cell_run(text: &str, x: f32, y: f32, line: u32, bold: bool) -> Run {
        Run {
            x,
            w: 4.5 * text.chars().count() as f32,
            bold,
            ..run(text, line, y, 8.0)
        }
    }

    /// The runs of a table of taxes: a header row (bold or not) and three rows of items,
    /// in columns at 43, 162, 282 and 402.
    fn tax_table(bold_header: bool) -> Vec<Run> {
        let xs = [43.0, 162.0, 282.0, 402.0];
        let rows = [
            ["Art", "Basis", "Satz", "Betrag"],
            ["Steuer A", "1.000,00", "10 %", "100,00"],
            ["Steuer B", "2.000,00", "7 %", "140,00"],
            ["Steuer C", "500,00", "19 %", "95,00"],
        ];
        let mut runs = Vec::new();
        for (k, row) in rows.iter().enumerate() {
            let y = 750.0 - 14.0 * k as f32;
            for (x, t) in xs.iter().zip(row) {
                runs.push(cell_run(t, *x, y, k as u32, k == 0 && bold_header));
            }
        }
        runs
    }

    /// The page laid out from `runs` and `drawn`, with nothing stopping it.
    fn laid_page(runs: Vec<Run>, drawn: &Drawn<'_>) -> Laid {
        let style = Style::new(&[PageText {
            page: 1,
            height: 842.0,
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
        page_blocks_until(1, 842.0, runs, drawn, &cx, &[], &|| false)
    }

    /// The tables of `laid`: their header rows and the text of their cells.
    fn tables_of(laid: &Laid) -> Vec<(usize, Vec<String>)> {
        laid.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Table {
                    rows, header_rows, ..
                } => Some((
                    *header_rows,
                    rows.iter()
                        .flatten()
                        .map(|c| rezzer_model::plain_text(&c.inlines))
                        .collect(),
                )),
                _ => None,
            })
            .collect()
    }

    /// Asserts that `laid` holds the tax table as one table under its header row.
    fn one_tax_table(laid: &Laid) {
        let tables = tables_of(laid);
        assert_eq!(tables.len(), 1, "{:?}", kinds(&laid.blocks));
        let (header_rows, cells) = &tables[0];
        assert_eq!(*header_rows, 1);
        assert_eq!(cells.len(), 16, "{cells:?}");
        assert_eq!(cells[..4], ["Art", "Basis", "Satz", "Betrag"]);
    }

    #[test]
    fn a_bold_header_over_its_body_is_one_table() {
        // The page's segments part the bold header from its rows (by the white space under
        // it or by its style): a hint, no wall.
        let runs = tax_table(true);
        let page = engine::page_in(
            &crate::text::PageItems::default(),
            &runs,
            &NOTHING_DRAWN,
            [595.0, 842.0],
        );
        let s = layoutregions::segment::segment_page(&page, &Default::default(), &|| false);
        let leaf = |t: u32| {
            s.all
                .iter()
                .position(|g| g.children.is_empty() && g.prims.contains(&PrimId::Text(t)))
        };
        // "Art" heads the first column, "Steuer A" is the first row under it.
        assert_ne!(
            leaf(0),
            leaf(4),
            "the segments part the header from its rows"
        );
        one_tax_table(&laid_page(runs, &NOTHING_DRAWN));
    }

    #[test]
    fn a_table_across_boxed_cells_is_one_table() {
        // Each header cell is drawn as a box of its own, the rows under them are not: the
        // boxes are segments of their own, and the table reaches across them.
        let boxes: Vec<[f32; 4]> = [39.0, 158.0, 278.0, 398.0]
            .iter()
            .map(|&x| [x, 746.0, x + 114.0, 760.0])
            .collect();
        let rules: Vec<Rule> = boxes
            .iter()
            .map(|b| Rule {
                x0: b[0],
                y0: b[1],
                x1: b[2],
                y1: b[3],
                kind: crate::tables::RuleKind::Rect,
            })
            .collect();
        let structure = crate::regions::structure_in_frame(&rules, &boxes);
        let drawn = Drawn {
            rules: &rules,
            structure: &structure,
            boxes: &boxes,
            ..NOTHING_DRAWN
        };
        let runs = tax_table(false);
        let page = engine::page_in(
            &crate::text::PageItems::default(),
            &runs,
            &drawn,
            [595.0, 842.0],
        );
        let s =
            layoutregions::segment::segment_drawn(&page, &structure, &Default::default(), &|| {
                false
            });
        let cells: Vec<_> = s
            .all
            .iter()
            .filter(|g| g.source == layoutregions::segment::Source::Box)
            .collect();
        assert_eq!(cells.len(), 4, "each header cell is a box segment");
        one_tax_table(&laid_page(runs, &drawn));
    }

    #[test]
    fn a_segment_limit_loses_no_read() {
        // 70 boxes drawn one in the other, and in the innermost a ruled grid of three rows and
        // two columns: the segments nest deeper than their bound, so the grid has no segment.
        // The ruled detector still reads it, the bound travels with the page's regions, and
        // the page says no loss, as nothing is lost.
        let boxes: Vec<[f32; 4]> = (0..70)
            .map(|k| {
                let d = 2.5 * k as f32;
                [20.0 + d, 20.0 + d, 575.0 - d, 820.0 - d]
            })
            .collect();
        let line = |x0: f32, y0: f32, x1: f32, y1: f32| Rule {
            x0,
            y0,
            x1,
            y1,
            kind: crate::tables::RuleKind::Line,
        };
        let mut rules = Vec::new();
        for y in [400.0, 420.0, 440.0, 460.0] {
            rules.push(line(210.0, y, 390.0, y));
        }
        for x in [210.0, 300.0, 390.0] {
            rules.push(line(x, 400.0, x, 460.0));
        }
        let structure = crate::regions::structure_in_frame(&rules, &boxes);
        let drawn = Drawn {
            rules: &rules,
            structure: &structure,
            boxes: &boxes,
            ..NOTHING_DRAWN
        };
        let rows = [
            ["Art", "Betrag"],
            ["Steuer A", "100,00"],
            ["Steuer B", "140,00"],
        ];
        let mut runs = Vec::new();
        for (k, row) in rows.iter().enumerate() {
            let y = 446.0 - 20.0 * k as f32;
            runs.push(cell_run(row[0], 214.0, y, k as u32, false));
            runs.push(cell_run(row[1], 304.0, y, k as u32, false));
        }
        let page = engine::page_in(
            &crate::text::PageItems::default(),
            &runs,
            &drawn,
            [595.0, 842.0],
        );
        let s =
            layoutregions::segment::segment_drawn(&page, &structure, &Default::default(), &|| {
                false
            });
        assert!(
            !s.all
                .iter()
                .any(|g| matches!(g.source, layoutregions::segment::Source::Grid { .. })),
            "the grid lies past the bound"
        );
        let laid = laid_page(runs, &drawn);
        let tables = tables_of(&laid);
        assert_eq!(tables.len(), 1, "{:?}", kinds(&laid.blocks));
        assert_eq!(
            tables[0].1,
            ["Art", "Betrag", "Steuer A", "100,00", "Steuer B", "140,00"]
        );
        assert!(
            laid.notes.iter().all(|n| n.1 != LossReason::Limit),
            "{:?}",
            laid.notes
        );
        assert!(
            laid.regions
                .issues
                .iter()
                .any(|i| i.kind == layoutregions::IssueKind::Limit
                    && i.what.contains("nest deeper than 64 levels")),
            "{:?}",
            laid.regions.issues
        );
    }

    /// A leaf the segmentation's depth bound left whole is not typed (limit
    /// leaves are kept as laid out with a Limit loss): 70 boxes drawn one in the other nest
    /// deeper than the bound of 64 levels, so the box at the bound keeps the lines of the
    /// boxes in it. Those lines read as items of a list by their dashes; they are one region
    /// kept as laid out, no other region holds them, every line is written once, and the loss
    /// names the bound.
    #[test]
    fn a_leaf_the_depth_bound_left_whole_is_kept_as_laid_out() {
        let line = |x0: f32, y0: f32, x1: f32, y1: f32| Rule {
            x0,
            y0,
            x1,
            y1,
            kind: crate::tables::RuleKind::Line,
        };
        let mut rules = Vec::new();
        for k in 0..70 {
            let d = 2.5 * k as f32;
            let (x0, y0, x1, y1) = (100.0 + d, 200.0 + d, 500.0 - d, 700.0 - d);
            rules.push(line(x0, y0, x1, y0));
            rules.push(line(x0, y1, x1, y1));
            rules.push(line(x0, y0, x0, y1));
            rules.push(line(x1, y0, x1, y1));
        }
        let structure = crate::regions::structure_in_frame(&rules, &[]);
        let drawn = Drawn {
            rules: &rules,
            structure: &structure,
            ..NOTHING_DRAWN
        };
        let items = ["- one", "- two", "- six", "- ten"];
        let runs: Vec<Run> = items
            .iter()
            .enumerate()
            .map(|(k, t)| cell_run(t, 280.0, 480.0 - 14.0 * k as f32, k as u32, false))
            .collect();
        let laid = laid_page(runs, &drawn);
        let short: Vec<_> = laid
            .regions
            .regions
            .iter()
            .filter(|r| r.signals.iter().any(|s| s.id == "pdf.limit.unparted"))
            .collect();
        assert_eq!(
            short.len(),
            1,
            "{:?} {:?}",
            laid.regions.issues,
            laid.regions.regions
        );
        assert_eq!(short[0].kind, Kind::Spatial);
        assert_eq!(short[0].alternatives[0].kind, Kind::List);
        let texts: Vec<u32> = short[0]
            .prims
            .iter()
            .filter_map(|p| match p {
                layoutregions::PrimId::Text(i) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(texts, [0, 1, 2, 3]);
        for r in &laid.regions.regions {
            if r.id != short[0].id {
                assert!(
                    !r.prims
                        .iter()
                        .any(|p| matches!(p, layoutregions::PrimId::Text(_))),
                    "{r:?}"
                );
            }
        }
        let mut text = String::new();
        for b in &laid.blocks {
            if let Block::Spatial { lines, .. } = b {
                for l in lines {
                    for p in &l.pieces {
                        text.push_str(&p.text);
                        text.push('\n');
                    }
                }
            }
        }
        for t in items {
            assert_eq!(text.matches(t).count(), 1, "{t} in {text}");
        }
        assert!(
            laid.located.iter().any(|l| l.2 == LossReason::Limit
                && l.1.contains("kept as laid out")
                && l.1.contains("bound")),
            "{:?}",
            laid.located
        );
    }

    /// A drawing with a figure caption beside it is a figure region, though it holds no text
    /// of its own: only a leaf of rules or boxes the engine left unsure is no region.
    #[test]
    fn a_drawn_figure_without_text_keeps_its_region() {
        let line = |x0: f32, y0: f32, x1: f32, y1: f32| Rule {
            x0,
            y0,
            x1,
            y1,
            kind: crate::tables::RuleKind::Line,
        };
        let rules = vec![
            line(72.0, 400.0, 372.0, 400.0),
            line(72.0, 600.0, 372.0, 600.0),
            line(72.0, 400.0, 72.0, 600.0),
            line(372.0, 400.0, 372.0, 600.0),
            line(100.0, 420.0, 340.0, 580.0),
        ];
        let structure = crate::regions::structure_in_frame(&rules, &[]);
        let drawn = Drawn {
            rules: &rules,
            structure: &structure,
            ..NOTHING_DRAWN
        };
        let runs = vec![Run {
            size: 9.2,
            ..cell_run("Fig. 4: values of a made-up series", 72.0, 385.0, 0, false)
        }];
        let laid = laid_page(runs, &drawn);
        let figures: Vec<_> = laid
            .regions
            .regions
            .iter()
            .filter(|r| r.kind == Kind::Figure)
            .collect();
        assert_eq!(figures.len(), 1, "{:?}", laid.regions.regions);
        assert!(
            figures[0]
                .signals
                .iter()
                .any(|s| s.id == "figure.drawn_caption"),
            "{:?}",
            figures[0].signals
        );
    }

    /// A stop before the first detector runs none of them: the running footer of a file of
    /// two pages, which the furniture finder takes without the stop, is still laid out as
    /// text, every run is in a region of its own block, and a limit note says the detectors
    /// looked only in part.
    #[test]
    fn a_stop_before_the_first_finder_keeps_all_text() {
        let runs = vec![
            run("Muster GmbH", 0, 700.0, 10.0),
            run("Musterweg 1", 1, 688.0, 10.0),
            run("12345 Musterstadt", 2, 676.0, 10.0),
            run("Betrag 1.234,56", 3, 640.0, 10.0),
            run("Muster GmbH, Musterweg 1, 12345 Musterstadt", 4, 30.0, 10.0),
        ];
        let second: Vec<Run> = runs
            .iter()
            .map(|r| Run {
                page: 2,
                ..r.clone()
            })
            .collect();
        let style = Style::new(&[
            PageText {
                page: 1,
                height: 842.0,
                runs: &runs,
            },
            PageText {
                page: 2,
                height: 842.0,
                runs: &second,
            },
        ]);
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
        let footer = |laid: &Laid| {
            laid.blocks.iter().find_map(|b| match b {
                Block::Paragraph { inlines, role, .. }
                    if rezzer_model::plain_text(inlines).starts_with("Muster GmbH, ") =>
                {
                    Some(role.clone())
                }
                _ => None,
            })
        };
        // Without the stop the furniture finder takes the footer.
        let found = page_blocks_until(1, 842.0, runs.clone(), &NOTHING_DRAWN, &cx, &[], &|| false);
        assert_eq!(footer(&found), Some(Some("page-footer".to_string())));
        let n = runs.len();
        let laid = page_blocks_until(1, 842.0, runs, &NOTHING_DRAWN, &cx, &[], &|| true);
        assert!(laid.stop.is_none());
        let mut text = String::new();
        // A stop before the regions are typed keeps every leaf as laid out (limit and
        // stop leaves are spatial, with a Limit loss).
        for b in &laid.blocks {
            match b {
                Block::Paragraph { inlines, .. } | Block::Heading { inlines, .. } => {
                    text.push_str(&rezzer_model::plain_text(inlines));
                    text.push('\n');
                }
                Block::Spatial { lines, .. } => {
                    for l in lines {
                        let pieces: Vec<&str> = l.pieces.iter().map(|p| p.text.as_str()).collect();
                        text.push_str(&pieces.join(" "));
                        text.push('\n');
                    }
                }
                _ => {}
            }
        }
        for t in [
            "Muster GmbH",
            "Musterweg 1",
            "12345 Musterstadt",
            "Betrag 1.234,56",
            "Muster GmbH, Musterweg 1, 12345 Musterstadt",
        ] {
            assert!(text.contains(t), "{t} in {text}");
        }
        assert_ne!(
            footer(&laid),
            Some(Some("page-footer".to_string())),
            "the footer is text"
        );
        let limits: Vec<&str> = laid
            .notes
            .iter()
            .filter(|n| n.1 == LossReason::Limit)
            .map(|n| n.0.as_str())
            .collect();
        assert_eq!(limits.len(), 1, "{:?}", laid.notes);
        assert!(
            limits[0].contains("looked for only in part"),
            "{}",
            limits[0]
        );
        let issues = &laid.regions.issues;
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, layoutregions::IssueKind::Stopped);
        let mut ids: Vec<layoutregions::PrimId> = laid
            .regions
            .regions
            .iter()
            .flat_map(|r| r.prims.iter().copied())
            .collect();
        ids.sort_unstable();
        let all: Vec<layoutregions::PrimId> =
            (0..n as u32).map(layoutregions::PrimId::Text).collect();
        assert_eq!(ids, all);
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
                842.0,
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
                height: 842.0,
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
            let laid = page_blocks(k as u32 + 1, 842.0, runs.clone(), &NOTHING_DRAWN, &cx);
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
            842.0,
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
            842.0,
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
        let first = page_blocks(1, 842.0, tagged_runs(&["a"], 700.0), &NOTHING_DRAWN, &cx);
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
            842.0,
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
                842.0,
                tagged_runs(&[word], 700.0),
                &NOTHING_DRAWN,
                &cx,
            ));
        }
        for (k, word) in ["first", "second"].into_iter().enumerate() {
            // The cells of a tag table that is not kept are free text, which the
            // engine types; a lone word has no clear evidence and may be kept as laid out (a
            // right read makes it a paragraph again). It is never a row.
            match laid[k].blocks.as_slice() {
                [Block::Paragraph { inlines, .. }] => {
                    assert_eq!(rezzer_model::plain_text(inlines), word);
                }
                [Block::Spatial { lines, .. }] => {
                    assert_eq!(lines.len(), 1);
                    assert_eq!(lines[0].pieces[0].text, word);
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
            842.0,
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
            842.0,
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
            842.0,
            tagged_runs(&["lonely"], 700.0),
            &NOTHING_DRAWN,
            &cx,
        );
        // The cells of a tag table that does not hold are free text, which the engine
        // types; a lone word may be kept as laid out (a right read).
        match laid.blocks.as_slice() {
            [Block::Paragraph { inlines, .. }] => {
                assert_eq!(rezzer_model::plain_text(inlines), "lonely")
            }
            [Block::Spatial { lines, .. }] => {
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0].pieces[0].text, "lonely");
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
        let laid = page_blocks(1, 842.0, runs, &NOTHING_DRAWN, &cx);
        match laid.blocks.as_slice() {
            [Block::List { items, .. }] => assert_eq!(items.len(), 2),
            other => panic!("{other:?}"),
        }
        assert!(laid.stop.is_some());
        assert_eq!(laid.cut, Some(Cut::Item { block: 1, item: 3 }));
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
            height: 842.0,
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
            let mut laid = page_blocks(1, 842.0, runs.clone(), &NOTHING_DRAWN, &cx);
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
            842.0,
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

    /// The page of `a_segment_limit_loses_no_read` without its grid: 70 boxes drawn one in
    /// the other and a paragraph in the innermost, tagged by `tree` or not. Whether the page
    /// was segmented: a segmentation leaves its depth issue with the page's regions.
    fn segmented(tagged: bool) -> bool {
        let boxes: Vec<[f32; 4]> = (0..70)
            .map(|k| {
                let d = 2.5 * k as f32;
                [20.0 + d, 20.0 + d, 575.0 - d, 820.0 - d]
            })
            .collect();
        let structure = crate::regions::structure_in_frame(&[], &boxes);
        let drawn = Drawn {
            structure: &structure,
            boxes: &boxes,
            ..NOTHING_DRAWN
        };
        let tree = StructTree {
            elems: vec![elem("Document", None, &[1]), elem("P", Some(0), &[])],
            roots: vec![0],
        };
        let mut runs = Vec::new();
        for (i, t) in ["Muster GmbH", "Musterweg 1"].iter().enumerate() {
            let mut r = Run {
                x: 250.0,
                ..run(t, i as u32, 420.0 - 12.0 * i as f32, 10.0)
            };
            r.mcid = kept_mcid(Some(i as u32));
            runs.push(r);
        }
        let style = Style::new(&[PageText {
            page: 1,
            height: 842.0,
            runs: &runs,
        }]);
        let budget = Budget::new(rezzer_limits::Limits::default());
        let tables = Tables::default();
        let roles = |_: u32| Some(1usize);
        let cx = Context {
            style: &style,
            tree: tagged.then_some(&tree),
            roles: &roles,
            budget: &budget,
            tables: &tables,
            in_part: false,
            links: &LinkRoom::new(u64::MAX),
        };
        let laid = page_blocks_until(1, 842.0, runs, &drawn, &cx, &[], &|| false);
        let text: Vec<String> = laid
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph { inlines, .. } => Some(rezzer_model::plain_text(inlines)),
                _ => None,
            })
            .collect();
        assert!(
            text.iter().any(|t| t.contains("Muster GmbH")),
            "{tagged}: {text:?}"
        );
        laid.regions
            .issues
            .iter()
            .any(|i| i.what.contains("nest deeper than 64 levels"))
    }

    /// The segmentation runs only where the geometric detectors look: not on a tagged page
    /// whose tag tree holds.
    #[test]
    fn a_tagged_page_is_not_segmented() {
        assert!(segmented(false), "an untagged page is segmented");
        assert!(!segmented(true), "a tagged page whose tree holds is not");
    }

    #[test]
    fn tagged_page_ids_follow_the_flow() {
        let mut elems = vec![
            elem("Document", None, &[1, 2]),
            elem("P", Some(0), &[]),
            elem("P", Some(0), &[]),
        ];
        elems[1].actual_text = Some("Replaced".into());
        let tree = StructTree {
            elems,
            roots: vec![0],
        };
        // Runs 0 and 1 are the glyph runs of the first paragraph, run 2 stands alone.
        let owner = [1usize, 1, 2];
        let mut runs = Vec::new();
        for (i, t) in ["Gl", "yphs", "Other"].iter().enumerate() {
            let mut r = run(t, i as u32, 700.0 - 20.0 * i as f32, 10.0);
            r.mcid = kept_mcid(Some(i as u32));
            runs.push(r);
        }
        let style = Style::new(&[PageText {
            page: 1,
            height: 842.0,
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
        let (kept, slots) = classified(runs, &tree, &cx);
        let flow = Flow::new(kept, slots);
        let page = engine::page_in(
            &crate::text::PageItems::default(),
            &flow.runs,
            &NOTHING_DRAWN,
            [595.0, 842.0],
        );
        // The second glyph run is dropped: two texts, with the ids the flow keeps.
        assert_eq!(page.texts.len(), 2);
        assert_eq!(flow.ids, [0, 1]);
        assert_eq!(page.texts[0].text, "Replaced");
        assert_eq!(page.texts[1].text, "Other");
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
            height: 842.0,
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
        let laid = page_blocks(1, 842.0, runs, &NOTHING_DRAWN, &cx);
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

    /// The steps of work [`a_table_over_many_pages_is_walked_once`] may take: the count measured
    /// when the bound was set (2026-11-11: 22,030,000), with half of it again as margin.
    const WALK_STEPS: u64 = 33_000_000;

    #[test]
    #[cfg_attr(
        not(debug_assertions),
        ignore = "work steps are counted in debug builds only"
    )]
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
        let before = crate::work::work_steps();
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
            let laid = page_blocks(page as u32 + 1, 842.0, runs, &NOTHING_DRAWN, &cx);
            // The table starts on page 1; later pages add rows to it.
            match (laid.blocks.as_slice(), laid.continued.as_slice()) {
                ([Block::Table { rows: r, .. }], []) if page == 0 => rows += r.len(),
                ([], [(1, r)]) if page > 0 => rows += r.len(),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(rows, ROWS);
        // The steps of work, counted in debug builds: the table's rows are read once, and
        // each page looks at its own cells. A walk of every row per page would take about
        // 2,000 times as many.
        let steps = crate::work::work_steps().zip(before).map(|(a, b)| a - b);
        eprintln!("walked once: {steps:?} steps");
        if let Some(steps) = steps {
            assert!(steps <= WALK_STEPS, "{steps} steps of work");
        }
    }
    /// A table of pair rows on one line, for [`pair_step`]: its baseline and the left edge of
    /// its label, in text of size 10 with the value 60 points after the label.
    fn pair_line(y: f32, x: f32) -> PairPlace {
        PairPlace {
            top: y,
            bottom: y,
            first_step: None,
            last_step: None,
            labels: x,
            values: Some(x + 60.0),
            x0: x,
            x1: x + 100.0,
            size: 10.0,
        }
    }

    /// Two tables of pair rows are rows of one block only from one
    /// left edge, the lower under the upper, with no drawn line between them; the bound of
    /// the edge and each sign move the answer both ways.
    #[test]
    fn rows_of_pairs_are_of_one_block_only_from_one_edge_with_no_rule_between() {
        let a = pair_line(700.0, 40.0);
        // The same left edge, within `EDGE_SIZES` of the text size and past it.
        assert!(pair_step(&a, &pair_line(688.0, 40.9), &[]).is_some());
        assert!(pair_step(&a, &pair_line(688.0, 41.1), &[]).is_none());
        // Labels set flush right share no left edge, but their values do.
        let mut right = pair_line(688.0, 30.0);
        right.values = a.values;
        assert!(pair_step(&a, &right, &[]).is_some());
        // Another column, and a table beside or over the first one.
        assert!(pair_step(&a, &pair_line(688.0, 300.0), &[]).is_none());
        assert!(pair_step(&a, &pair_line(700.0, 40.0), &[]).is_none());
        assert!(pair_step(&a, &pair_line(712.0, 40.0), &[]).is_none());
        // An edge of a box between the two baselines over their width parts them (its far
        // side is more than a row away); one beside them, or over the upper row, does not.
        // The box of one row (as high as the step of the rows, 12, within `BASELINE_NEAR`)
        // is kept as a line.
        let b = pair_line(688.0, 40.0);
        let edge = |y: f32, x0: f32, x1: f32, other: f32| Edge { y, x0, x1, other };
        assert!(pair_step(&a, &b, &[edge(694.0, 30.0, 200.0, 650.0)]).is_none());
        assert!(pair_step(&a, &b, &[edge(694.0, 300.0, 500.0, 650.0)]).is_some());
        assert!(pair_step(&a, &b, &[edge(710.0, 30.0, 200.0, 650.0)]).is_some());
        assert_eq!(pair_step(&a, &b, &[]).map(|s| s.step), Some(12.0));
        assert!(pair_step(&a, &b, &[edge(694.0, 30.0, 200.0, 680.6)]).is_some());
        assert!(pair_step(&a, &b, &[edge(694.0, 30.0, 200.0, 680.4)]).is_none());
        // A rule that lies between them as a whole is kept in the step, with the pieces
        // that go on from it; a second line at another height parts them.
        let thin = [
            edge(694.0, 30.0, 200.0, 694.5),
            edge(694.0, 200.5, 400.0, 694.5),
            edge(694.0, 420.0, 500.0, 694.5),
            edge(694.5, 30.0, 200.0, 694.0),
        ];
        let kept = pair_step(&a, &b, &thin).expect("one line between");
        let line = kept.ruled.expect("the line");
        assert_eq!((line.x0, line.x1), (30.0, 400.0));
        assert_eq!(line.far, None, "thin rules are no sides of a box");
        // A piece that goes on from the line is a piece of it only when
        // it is no higher than one row, as the pieces over the tables are: the far side
        // within the step of the rows (12) and `BASELINE_NEAR` of the text size (1.5). The
        // upper side of a tall box that starts where the rule ends is none.
        let beside = |other: f32| {
            let edges = [thin[0], edge(694.0, 200.5, 400.0, other)];
            let line = pair_step(&a, &b, &edges)
                .and_then(|s| s.ruled)
                .expect("the line");
            (line.x0, line.x1)
        };
        assert_eq!(beside(680.6), (30.0, 400.0));
        assert_eq!(beside(680.4), (30.0, 200.0));
        // The far sides of the edges of boxes over the tables are kept with the line.
        let boxed = [
            edge(694.0, 30.0, 200.0, 706.0),
            edge(694.0, 30.0, 200.0, 682.0),
        ];
        let line = pair_step(&a, &b, &boxed)
            .and_then(|s| s.ruled)
            .expect("the line");
        assert_eq!(line.far, Some((682.0, 706.0)));
        let near = layoutregions::segment::cut::BASELINE_NEAR * 10.0;
        let two = |dy: f32| [thin[0], edge(694.0 + dy, 30.0, 200.0, 694.2 + dy)];
        assert!(pair_step(&a, &b, &two(near - 0.1)).is_some());
        assert!(pair_step(&a, &b, &two(near + 0.1)).is_none());
    }

    /// A drawn line between two
    /// rows of pairs is the block's own ruling when the same line stands between every two
    /// rows of the run: one extent, at the pitch of the rows, in every step of the run. A
    /// lone line, a line of another extent, a line off the pitch, a line beside a table with
    /// rows of its own, the two sides of the box of one row, and lines that stop while the
    /// rows go on at their pitch all part the rows.
    #[test]
    fn a_line_between_every_two_rows_of_a_run_parts_nothing() {
        let line = |y: f32, x0: f32, x1: f32| RowRule {
            y,
            x0,
            x1,
            far: None,
        };
        let step = |ruled: Option<RowRule>| PairStep {
            step: 12.0,
            size: 10.0,
            own: [None, None],
            ruled,
        };
        let plain = Some(step(None));
        let this = Some(step(Some(line(694.0, 30.0, 200.0))));
        let same = |y: f32, x0: f32, x1: f32| Some(step(Some(line(y, x0, x1))));
        // No line: nothing parts. A lone line parts, alone and between two parts that are
        // no rows of pairs.
        assert_eq!(own_rulings(&[plain]), [true]);
        assert_eq!(own_rulings(&[this]), [false]);
        assert_eq!(own_rulings(&[None, this, None]), [false, false, false]);
        // The same line one pitch lower: the two steps of three rows.
        let next = same(682.0, 30.0, 200.0);
        assert_eq!(own_rulings(&[this, next]), [true, true]);
        assert_eq!(
            own_rulings(&[this, None, next]),
            [false; 3],
            "another part between"
        );
        // Another extent: the ends within `EDGE_SIZES` of the text size, and past it.
        assert_eq!(own_rulings(&[this, same(682.0, 30.9, 200.0)]), [true; 2]);
        assert_eq!(own_rulings(&[this, same(682.0, 31.1, 200.0)]), [false; 2]);
        assert_eq!(own_rulings(&[this, same(682.0, 30.0, 300.0)]), [false; 2]);
        // Off the pitch of the rows: within `BASELINE_NEAR` of the text size, and past it.
        assert_eq!(own_rulings(&[this, same(683.4, 30.0, 200.0)]), [true; 2]);
        assert_eq!(own_rulings(&[this, same(683.6, 30.0, 200.0)]), [false; 2]);
        let wide = PairStep {
            step: 20.0,
            ..step(Some(line(678.0, 30.0, 200.0)))
        };
        assert_eq!(own_rulings(&[this, Some(wide)]), [false; 2]);
        // A table with rows of its own under each other has no line between them.
        let own = this.map(|s| PairStep {
            own: [Some(12.0), None],
            ..s
        });
        assert_eq!(own_rulings(&[own, next]), [false; 2]);
        // M3: the line stands between every two rows of the run. Rows that go on at the
        // pitch with no line (a boxed row among plain rows: the lines over and under it)
        // make the lines borders; the plain steps part nothing. Rows at another pitch are
        // no rows of the run.
        assert_eq!(
            own_rulings(&[plain, this, next, plain]),
            [true, false, false, true]
        );
        assert_eq!(own_rulings(&[this, next, plain]), [false, false, true]);
        let far = Some(PairStep {
            step: 30.0,
            ..step(None)
        });
        assert_eq!(own_rulings(&[this, next, far]), [true, true, true]);
        let just = |step_: f32| {
            Some(PairStep {
                step: step_,
                ..step(None)
            })
        };
        assert_eq!(own_rulings(&[this, next, just(13.4)]), [false, false, true]);
        assert_eq!(own_rulings(&[this, next, just(13.6)]), [true, true, true]);
        // M3: the two sides of the box of one row are one frame, no line that comes again:
        // the upper line is the upper side of a box whose lower side is the lower line.
        // Within `BASELINE_NEAR` of the text size (1.5), and past it.
        let side = |y: f32, far: (f32, f32)| {
            Some(step(Some(RowRule {
                far: Some(far),
                ..line(y, 30.0, 200.0)
            })))
        };
        let (upper, lower) = (side(694.0, (682.0, 682.0)), side(682.0, (694.0, 694.0)));
        assert_eq!(own_rulings(&[upper, lower]), [false; 2]);
        assert_eq!(own_rulings(&[upper, next]), [false; 2], "one side says so");
        assert_eq!(
            own_rulings(&[side(694.0, (683.4, 683.4)), next]),
            [false; 2]
        );
        assert_eq!(own_rulings(&[side(694.0, (683.6, 683.6)), next]), [true; 2]);
        // A third line at the pitch: a band behind every other row has its two sides and
        // the upper side of the next band, lines between every two rows of four.
        let third = side(670.0, (658.0, 658.0));
        assert_eq!(own_rulings(&[upper, lower, third]), [true; 3]);
        // Boxes of one row each that touch: each line is the lower side of the box over
        // it and the upper side of the box under it, far sides both ways.
        let (upper, lower) = (side(694.0, (682.0, 706.0)), side(682.0, (670.0, 694.0)));
        assert_eq!(own_rulings(&[upper, lower]), [true; 2]);
    }

    /// A step whose drawn line is a border lends its pitch to no
    /// step beside it: how far two blocks are apart is no pitch of rows. A step with no
    /// line, or with the block's own ruling, shows the pitch.
    #[test]
    fn a_step_across_a_border_lends_no_pitch() {
        let step = |ruled: Option<RowRule>| {
            Some(PairStep {
                step: 26.0,
                size: 10.0,
                own: [None, None],
                ruled,
            })
        };
        let line = |y: f32, x1: f32| {
            Some(RowRule {
                y,
                x0: 30.0,
                x1,
                far: None,
            })
        };
        // Rows 26 apart (wider than `ITEM_PITCH`, 20): two steps show the pitch twice.
        assert_eq!(joined_steps(&[step(None)]), [false]);
        assert_eq!(joined_steps(&[step(None), step(None)]), [true, true]);
        // A lone line before two rows at that pitch: a border, and the rows after it have
        // their pitch only once.
        assert_eq!(
            joined_steps(&[step(line(687.0, 200.0)), step(None)]),
            [false, false]
        );
        // The block's own ruling shows its pitch to the steps of its run.
        assert_eq!(
            joined_steps(&[step(line(687.0, 200.0)), step(line(661.0, 200.0))]),
            [true, true]
        );
        // Two lines of two extents are two borders: no step joins, none lends.
        assert_eq!(
            joined_steps(&[
                step(line(687.0, 200.0)),
                step(line(661.0, 300.0)),
                step(None)
            ]),
            [false, false, false]
        );
        // Rows over a lone line at the pitch of the rows under it: the border lends
        // nothing, the step past it shows the pitch. A part between that is no row of
        // pairs ends the look.
        assert_eq!(
            joined_steps(&[step(None), step(line(661.0, 200.0)), step(None)]),
            [true, false, true]
        );
        assert_eq!(
            joined_steps(&[step(None), step(line(661.0, 200.0)), None, step(None)]),
            [false, false, false, false]
        );
        // The look goes past one border, not two. Rows, a rule, a row, a
        // rule of another extent, rows: the rows before the first border do not take their
        // pitch from the rows after the second one (nor after a third).
        let (first, second) = (step(line(661.0, 200.0)), step(line(635.0, 300.0)));
        assert_eq!(
            joined_steps(&[step(None), first, second, step(None)]),
            [false, false, false, false]
        );
        assert_eq!(
            joined_steps(&[step(None), first, second, first, step(None)]),
            [false; 5]
        );
        // After the second border the look starts again: the rows there show their pitch
        // to each other, and to a step one border before them.
        assert_eq!(
            joined_steps(&[step(None), first, second, step(None), step(None)]),
            [false, false, false, true, true]
        );
        assert_eq!(
            joined_steps(&[step(None), first, step(None), second, step(None)]),
            [true, false, true, false, true]
        );
    }

    /// A block kept as laid out of `lines`, each a list of pieces (column, text).
    fn laid_out(lines: &[&[(u32, &str)]]) -> Block {
        Block::Spatial {
            lines: lines
                .iter()
                .map(|l| rezzer_model::SpatialLine {
                    pieces: l
                        .iter()
                        .map(|&(col, text)| rezzer_model::SpatialPiece {
                            col,
                            text: text.into(),
                            spans: Vec::new(),
                        })
                        .collect(),
                })
                .collect(),
            links: Vec::new(),
            grid: None,
            anchor: None,
        }
    }

    /// The grid of a block kept as laid out (its column and whether it joins) and the
    /// columns of its pieces.
    type Gridded = (Option<(u32, bool)>, Vec<u32>);

    /// The grid and the piece columns of the blocks kept as laid out among `blocks`.
    fn grids(blocks: &[Block]) -> Vec<Gridded> {
        blocks
            .iter()
            .filter_map(|b| match b {
                Block::Spatial { lines, grid, .. } => Some((
                    grid.map(|g| (g.col, g.joins)),
                    lines
                        .iter()
                        .flat_map(|l| l.pieces.iter().map(|p| p.col))
                        .collect(),
                )),
                _ => None,
            })
            .collect()
    }

    /// A block kept as laid out joins the one before it only
    /// on one grid of the page, and the columns of a run are counted on that grid.
    #[test]
    fn blocks_kept_as_laid_out_join_only_on_one_grid_of_the_page() {
        // A frame of one line at `x0` on the baseline `y`, columns `adv` wide, in 10 points.
        let frame = |x0: f32, y: f32, adv: f32| SpatialFrame {
            x0,
            adv,
            first: y,
            last: y,
            size: 10.0,
            pitch: None,
        };
        let labels = || laid_out(&[&[(0, "Bank"), (20, "Paid"), (40, "Due")]]);
        let values = || laid_out(&[&[(0, "Oakfield"), (20, "30.10.2026")]]);
        let media = || Block::Media {
            asset: "a".into(),
            alt: None,
            anchor: None,
        };
        // The page: text from 40, columns 5 points wide, no line pitch shown.
        let page = Some(layoutregions::extract::spatial::PageGrid {
            left: 40.0,
            unit: 5.0,
            pitch: None,
        });
        // Right under one another from one edge, a picture between them: one run. Both
        // count their columns in 5 points from 90, so no column moves.
        let mut blocks = vec![labels(), media(), values()];
        place_spatial(
            &mut blocks,
            &[(0, frame(90.0, 700.0, 5.0)), (2, frame(90.0, 688.0, 5.0))],
            page,
        );
        assert_eq!(
            grids(&blocks),
            [
                (Some((10, false)), vec![0, 20, 40]),
                (Some((10, true)), vec![0, 20])
            ]
        );
        assert_eq!(rezzer_model::spatial_runs(&blocks), [vec![0, 2]]);
        // The values start under the second label (100 points right): another column of
        // the page, no run, and each keeps its own columns.
        let mut blocks = vec![labels(), values()];
        place_spatial(
            &mut blocks,
            &[(0, frame(90.0, 700.0, 5.0)), (1, frame(190.0, 688.0, 5.0))],
            page,
        );
        assert_eq!(
            grids(&blocks),
            [
                (Some((10, false)), vec![0, 20, 40]),
                (Some((30, false)), vec![0, 20])
            ]
        );
        assert!(rezzer_model::spatial_runs(&blocks).is_empty());
        // Seven lines further down: no run.
        let mut blocks = vec![labels(), values()];
        place_spatial(
            &mut blocks,
            &[(0, frame(90.0, 700.0, 5.0)), (1, frame(90.0, 616.0, 5.0))],
            page,
        );
        assert!(rezzer_model::spatial_runs(&blocks).is_empty());
        // Another block between them, a block of one piece, or a block whose place is not
        // known (it hosts no region): no run.
        let mut blocks = vec![labels(), Block::paragraph(Vec::new()), values()];
        place_spatial(
            &mut blocks,
            &[(0, frame(90.0, 700.0, 5.0)), (2, frame(90.0, 688.0, 5.0))],
            page,
        );
        assert!(rezzer_model::spatial_runs(&blocks).is_empty());
        let mut blocks = vec![labels(), laid_out(&[&[(0, "Oakfield")]])];
        place_spatial(
            &mut blocks,
            &[(0, frame(90.0, 700.0, 5.0)), (1, frame(90.0, 688.0, 5.0))],
            page,
        );
        assert!(rezzer_model::spatial_runs(&blocks).is_empty());
        let mut blocks = vec![labels(), values()];
        place_spatial(&mut blocks, &[(0, frame(90.0, 700.0, 5.0))], page);
        assert_eq!(
            grids(&blocks),
            [(Some((10, false)), vec![0, 20, 40]), (None, vec![0, 20])]
        );
        assert!(rezzer_model::spatial_runs(&blocks).is_empty());
        // Labels in a smaller face (4 points a character) over values in a larger one (5
        // points) that start 3 points further right: one run on the finer grid from the
        // labels' edge. The value at column 20 of its own grid stands 103 points from that
        // edge, at the label of column 25 (100 points), not under the one at column 20.
        let mut blocks = vec![
            laid_out(&[&[(0, "Bank"), (20, "Paid"), (25, "Due")]]),
            values(),
        ];
        place_spatial(
            &mut blocks,
            &[(0, frame(90.0, 700.0, 4.0)), (1, frame(93.0, 688.0, 5.0))],
            page,
        );
        assert_eq!(
            grids(&blocks),
            [
                (Some((10, false)), vec![0, 20, 25]),
                (Some((10, true)), vec![1, 26])
            ]
        );
        // A page without text has no grid: nothing is placed.
        let mut blocks = vec![labels(), values()];
        place_spatial(
            &mut blocks,
            &[(0, frame(90.0, 700.0, 5.0)), (1, frame(90.0, 688.0, 5.0))],
            None,
        );
        assert_eq!(
            grids(&blocks),
            [(None, vec![0, 20, 40]), (None, vec![0, 20])]
        );
    }

    /// Rows of pairs stand at the pitch of one block when they
    /// are as near as the lines of a block (`ITEM_PITCH`, moved both ways), or at a pitch
    /// the document shows a second time: the rows of one of the tables, or the step to the
    /// table before or after.
    #[test]
    fn rows_of_pairs_are_of_one_block_at_its_pitch() {
        let step = |step: f32| PairStep {
            step,
            size: 10.0,
            own: [None, None],
            ruled: None,
        };
        let pitch = layoutregions::typers::pairs::ITEM_PITCH * 10.0;
        assert!(step(pitch).of_one_block([None, None]));
        assert!(!step(pitch + 0.1).of_one_block([None, None]));
        // A wider pitch, shown again by the step before or after, within `BASELINE_NEAR`.
        assert!(step(26.0).of_one_block([Some(26.0), None]));
        assert!(step(26.0).of_one_block([None, Some(27.4)]));
        assert!(!step(26.0).of_one_block([Some(28.0), Some(23.0)]));
        // The rows of one of the two tables stand at that pitch.
        let own = PairStep {
            own: [None, Some(26.5)],
            ..step(26.0)
        };
        assert!(own.of_one_block([None, None]));
        // Two blocks 82 points apart with nothing like it around them stay two.
        assert!(!step(82.0).of_one_block([None, None]));
    }
}
