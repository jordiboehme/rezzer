// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typers: one function per kind of region that scores a segment from named signals, so a
//! test can ask why a region got its type. A typer gives `None` when it finds no evidence for
//! its kind (positive evidence only, never the absence of evidence for another kind). Its score
//! is the sum of its signals' weights, at most [`MAX_SCORE`]; the signals fire on rules stated
//! relative to the segment's own sizes (ruling P6-Y), so a page printed at another scale gets
//! the same scores. The arbitration picks among the kinds of a segment.
//!
//! The typers reuse the engine's rules, one copy each: the text typers read the walk of
//! [`crate::extract::text::next_block`] over a segment's lines, the heading levels of
//! [`crate::file::Levels`], and the running lines and footnote legends of [`FileFacts`].

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

use crate::extract::pairs::Style;
use crate::extract::table::area::Area;
use crate::extract::table::caption::ByY;
use crate::extract::table::{RunRange, run_ranges, texted_per_line};
use crate::extract::text::title::page_title;
use crate::file::{Band, FileFacts, Levels, RunningIndex, text_sizes, two_page_line};
use crate::input::{PageIn, PrimId, TextRun};
use crate::line::{LineRun, text_lines};
use crate::region::{Kind, Score, Signal};
use crate::segment::drawn::{DrawnStructure, Line};
use crate::segment::{Segment, Segments};

pub mod pairs;
pub mod table;
pub mod text;

pub use pairs::type_label_value;
pub use table::{type_table, type_totals};
pub use text::{
    type_footer_band, type_footnote_legend, type_header_band, type_heading, type_list,
    type_paragraph, type_title_block,
};

/// The highest score.
pub const MAX_SCORE: u16 = 1000;

/// What a typer found for its kind in a segment.
#[derive(Debug, Clone, PartialEq)]
pub struct Typed {
    pub kind: Kind,
    pub score: Score,
    pub signals: Vec<Signal>,
}

impl Typed {
    /// The kind with `signals`, scored by the sum of their weights (at most [`MAX_SCORE`]);
    /// `None` without a signal.
    pub(crate) fn of(kind: Kind, signals: Vec<Signal>) -> Option<Typed> {
        if signals.is_empty() {
            return None;
        }
        let sum: u32 = signals.iter().map(|s| u32::from(s.weight)).sum();
        let score = Score(sum.min(u32::from(MAX_SCORE)) as u16);
        Some(Typed {
            kind,
            score,
            signals,
        })
    }
}

/// A signal of `id` with `detail` and `weight`.
pub(crate) fn signal(id: &'static str, detail: String, weight: u16) -> Signal {
    Signal { id, detail, weight }
}

/// Whether `seg` is text alone: a group of the whitespace cut or of style steps, or the rest of
/// a segment's text, with no drawn part (grid, box, drawing) or given region under it. Style
/// steps part labels from values, so such a group need not be a leaf.
pub(crate) fn text_alone(view: &PageView<'_>, seg: &Segment) -> bool {
    use crate::segment::Source;
    matches!(
        seg.source,
        Source::Gutter | Source::Band | Source::Rule | Source::Style | Source::Rest
    ) && seg.children.iter().all(|c| {
        view.segs.all.get(c.0 as usize).is_some_and(|k| {
            matches!(
                k.source,
                Source::Gutter | Source::Band | Source::Rule | Source::Style | Source::Rest
            )
        })
    })
}

/// The text runs of `seg` on the page of `view`, borrowed, in the order of their ids, and
/// those ids.
pub(crate) fn seg_runs<'a>(view: &PageView<'a>, seg: &Segment) -> (Vec<&'a TextRun>, Vec<u32>) {
    let mut refs = Vec::new();
    let mut ids = Vec::new();
    for p in &seg.prims {
        if let PrimId::Text(i) = *p
            && let Some(r) = view.page.texts.get(i as usize)
        {
            refs.push(r);
            ids.push(i);
        }
    }
    (refs, ids)
}

/// Runs with ink in lines: each with its place among the runs it came from, the lines and the
/// pieces of each line.
pub(crate) struct Lined<'r> {
    pub runs: Vec<&'r TextRun>,
    pub at: Vec<usize>,
    pub ranges: Vec<RunRange>,
    pub pieces: Vec<Vec<crate::extract::table::grid::Piece>>,
}

/// The runs with ink of `runs` (not turned off the line), each with its place among them,
/// their lines and the pieces of each line (see [`crate::extract::table::grid::pieces_of`]).
pub(crate) fn lined<'r>(runs: &[&'r TextRun]) -> Lined<'r> {
    let at: Vec<usize> = (0..runs.len())
        .filter(|&k| !runs[k].text.trim().is_empty() && !runs[k].off_axis)
        .collect();
    let inked: Vec<&TextRun> = at.iter().map(|&k| runs[k]).collect();
    let ranges = run_ranges(&inked);
    let pieces = ranges
        .iter()
        .map(|l| crate::extract::table::grid::pieces_of(&inked, l.start, l.end))
        .collect();
    crate::work::add((inked.len() + ranges.len()) as u64);
    Lined {
        runs: inked,
        at,
        ranges,
        pieces,
    }
}

/// A typer: the evidence for its kind in a segment of the page the view shows. Pass the
/// segment as it stands in the view's tree (`&segs.all[i]`, not a copy): the text of a segment
/// of the tree is walked once for all typers, a copy is walked again by each.
pub type Typer = fn(&PageView<'_>, &Segment) -> Option<Typed>;

/// Every typer, in [`Kind`] order. Spatial has none: it is what no clear evidence leaves.
pub const TYPERS: &[(Kind, Typer)] = &[
    (Kind::HeaderBand, type_header_band),
    (Kind::FooterBand, type_footer_band),
    (Kind::TitleBlock, type_title_block),
    (Kind::Heading, type_heading),
    (Kind::Paragraph, type_paragraph),
    (Kind::List, type_list),
    (Kind::Table, type_table),
    (Kind::LabelValue, type_label_value),
    (Kind::Totals, type_totals),
    (Kind::FootnoteLegend, type_footnote_legend),
];

/// The file facts as the typers look them up, built once per file.
#[derive(Debug, Clone, Default)]
pub struct FileIndex {
    running: RunningIndex,
    two_page: HashSet<String>,
}

impl FileIndex {
    pub fn new(file: &FileFacts) -> FileIndex {
        FileIndex {
            running: RunningIndex::new(&file.running),
            two_page: file.two_page.iter().cloned().collect(),
        }
    }
}

/// The title of page 1 as the view finds it: the line numbers of its heading, and where its
/// top stands as a share of the page height.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ViewTitle {
    pub lines: Vec<u32>,
    pub top: f32,
}

/// One page as the typers see it: the page, its drawn structure, its segments and the facts
/// of its file, with what every typer of the page needs worked out once.
pub struct PageView<'a> {
    pub page: &'a PageIn,
    pub drawn: &'a DrawnStructure,
    pub segs: &'a Segments,
    pub file: &'a FileFacts,
    pub index: &'a FileIndex,
    /// Whether a text ends in an amount: the reader's own reading of amounts and dates.
    pub ends_in_amount: &'a dyn Fn(&str) -> bool,
    /// The band of each line number that is a running header or footer.
    bands: HashMap<u32, Band>,
    /// The line numbers of the running footers of the two-page rule.
    two_page: HashSet<u32>,
    /// The runs of the page's footnote legend.
    legend: HashSet<u32>,
    /// The text sizes of the page (see [`crate::file::text_sizes`]).
    text_sizes: Vec<f32>,
    /// The title of page 1 (see [`page_title`]).
    title: Option<ViewTitle>,
    /// The text of each segment of `segs`, worked out once.
    texts: Vec<OnceCell<Option<text::SegText>>>,
    /// The page's lines (runs of one line number, see [`run_ranges`]) and their index by
    /// height, for captions and the bodies of header bands.
    lines: Vec<RunRange>,
    by_y: ByY,
    /// The page's pictures (x0, y0, x1, y1).
    images: Vec<[f32; 4]>,
    /// The page's horizontal rules, sorted by height.
    hs: Vec<Line>,
    /// How many runs with text each line of the page has (see [`texted_per_line`]).
    per_line: HashMap<u32, u32>,
    /// The table reading of each segment of `segs`, worked out once.
    readings: Vec<OnceCell<table::Find>>,
    /// The totals reading of each segment of `segs`, worked out once.
    totals: Vec<OnceCell<Option<table::TotalsRead>>>,
    /// The label styles that pairs with independent evidence show (see [`pairs::seeds`]).
    seeds: OnceCell<Vec<Style>>,
    /// The label/value candidate of each segment, worked out once.
    pairs: Vec<OnceCell<Option<Typed>>>,
    /// Whether each segment's lines read as sentences across a ruled table, worked out once.
    across: Vec<OnceCell<bool>>,
    /// The segments read as tables, by the bottom of their tables, lowest first.
    tables: OnceCell<Vec<(f32, usize)>>,
    /// For each segment, whether it is a box that touches another box of its parent (see
    /// [`pairs::touching_boxes`]).
    touching: OnceCell<Vec<bool>>,
}

impl<'a> PageView<'a> {
    /// The view of `page`. The running lines are the page's lines that
    /// [`RunningIndex::bands`] finds; the text sizes are counted over the page's text without
    /// them and without its legend, with the footers of the two-page rule (as the layout of a
    /// page counts them before it reads its tables); the title is [`page_title`] over the
    /// page's lines without running lines and legend, on page 1.
    pub fn new(
        page: &'a PageIn,
        drawn: &'a DrawnStructure,
        segs: &'a Segments,
        file: &'a FileFacts,
        index: &'a FileIndex,
        ends_in_amount: &'a dyn Fn(&str) -> bool,
    ) -> PageView<'a> {
        let height = page.size[1];
        crate::work::add(page.texts.len() as u64);
        let lines = text_lines(&page.texts);
        let found = index.running.bands(&lines, height, &[]);
        let mut bands = HashMap::new();
        let mut two_page = HashSet::new();
        for (l, b) in lines.iter().zip(found) {
            let Some(b) = b else { continue };
            let number = l.runs[0].line;
            bands.insert(number, b);
            if two_page_line(l, height, &index.two_page) {
                two_page.insert(number);
            }
        }
        let legend: HashSet<u32> = file
            .legends
            .get(&page.page)
            .map(|l| {
                l.entries
                    .iter()
                    .flat_map(|e| e.runs.iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        let body = |(i, r): &(usize, &TextRun)| {
            !legend.contains(&(*i as u32)) && !bands.contains_key(&r.line)
        };
        let sizes = text_sizes(
            page.texts
                .iter()
                .enumerate()
                .filter(|(i, r)| {
                    !legend.contains(&(*i as u32))
                        && (!bands.contains_key(&r.line) || two_page.contains(&r.line))
                })
                .map(|(_, r)| r),
        );
        let title = if page.page == 1 {
            let runs: Vec<&TextRun> = page
                .texts
                .iter()
                .enumerate()
                .filter(body)
                .map(|(_, r)| r)
                .collect();
            let lines = text_lines(&runs);
            let levels = Levels {
                body_size: file.body_size,
                heading_sizes: &file.heading_sizes,
                text_sizes: &sizes,
                ends_in_amount,
            };
            page_title(&lines, &levels, height).map(|t| ViewTitle {
                lines: lines[t.lines.clone()]
                    .iter()
                    .map(|l| l.runs[0].line())
                    .collect(),
                top: lines[t.lines.start].y + lines[t.lines.start].h,
            })
        } else {
            None
        };
        let title = title.map(|t| ViewTitle {
            top: if height > 0.0 { t.top / height } else { 0.0 },
            ..t
        });
        crate::work::add(page.texts.len() as u64);
        let page_lines = run_ranges(&page.texts);
        let by_y = ByY::new(&page.texts, &page_lines);
        let images = page
            .pictures
            .iter()
            .map(|p| [p.bounds.x0, p.bounds.y0, p.bounds.x1, p.bounds.y1])
            .collect();
        let mut hs: Vec<Line> = drawn
            .grids
            .as_ref()
            .map(|g| g.hs.clone())
            .unwrap_or_default();
        hs.sort_by(|a, b| a.at.total_cmp(&b.at));
        let n = segs.all.len();
        PageView {
            page,
            drawn,
            segs,
            file,
            index,
            ends_in_amount,
            bands,
            two_page,
            legend,
            text_sizes: sizes,
            title,
            texts: (0..n).map(|_| OnceCell::new()).collect(),
            lines: page_lines,
            by_y,
            images,
            hs,
            per_line: texted_per_line(&page.texts),
            readings: (0..n).map(|_| OnceCell::new()).collect(),
            totals: (0..n).map(|_| OnceCell::new()).collect(),
            seeds: OnceCell::new(),
            pairs: (0..n).map(|_| OnceCell::new()).collect(),
            across: (0..n).map(|_| OnceCell::new()).collect(),
            tables: OnceCell::new(),
            touching: OnceCell::new(),
        }
    }

    /// The heading levels of the page's lines.
    pub fn levels(&self) -> Levels<'_> {
        Levels {
            body_size: self.file.body_size,
            heading_sizes: &self.file.heading_sizes,
            text_sizes: &self.text_sizes,
            ends_in_amount: self.ends_in_amount,
        }
    }

    /// The text sizes of the page, smallest first.
    pub fn text_sizes(&self) -> &[f32] {
        &self.text_sizes
    }

    /// `f` of the text of `seg`; `None` when it holds no run with ink. The text of a segment of
    /// the view's tree is worked out once, that of another segment (a test's) each time.
    pub(crate) fn with_text<T>(
        &self,
        seg: &Segment,
        f: impl FnOnce(&text::SegText) -> Option<T>,
    ) -> Option<T> {
        self.cached(
            &self.texts,
            seg,
            || text::SegText::of(self, seg),
            |t| t.as_ref().and_then(f),
        )
    }

    /// `f` of what `make` works out for `seg`: once for a segment of the view's tree (kept in
    /// `cells`, one per segment), each time for another segment (a test's).
    pub(crate) fn cached<T, U>(
        &self,
        cells: &[OnceCell<T>],
        seg: &Segment,
        make: impl FnOnce() -> T,
        f: impl FnOnce(&T) -> U,
    ) -> U {
        let own = self
            .segs
            .all
            .get(seg.id.0 as usize)
            .filter(|s| std::ptr::eq(*s, seg))
            .and_then(|_| cells.get(seg.id.0 as usize));
        match own {
            Some(cell) => f(cell.get_or_init(make)),
            None => f(&make()),
        }
    }

    /// The table reading of `seg` (see [`table::Find`]).
    pub(crate) fn reading<U>(&self, seg: &Segment, f: impl FnOnce(&table::Find) -> U) -> U {
        self.cached(&self.readings, seg, || table::Find::of(self, seg), f)
    }

    /// The totals reading of `seg` (see [`table::TotalsRead`]).
    pub(crate) fn totals_of<U>(
        &self,
        seg: &Segment,
        f: impl FnOnce(Option<&table::TotalsRead>) -> U,
    ) -> U {
        self.cached(
            &self.totals,
            seg,
            || table::TotalsRead::of(self, seg),
            |t| f(t.as_ref()),
        )
    }

    /// The label/value candidate of `seg` (see [`pairs::type_label_value`]).
    pub(crate) fn pairs_of(&self, seg: &Segment) -> Option<Typed> {
        self.cached(
            &self.pairs,
            seg,
            || pairs::label_value(self, seg),
            Clone::clone,
        )
    }

    /// Whether the lines of `seg` read as sentences across a ruled table (see
    /// [`table::sentences_across`]).
    pub(crate) fn across(&self, seg: &Segment) -> bool {
        self.cached(
            &self.across,
            seg,
            || table::sentences_across(self, seg),
            |b| *b,
        )
    }

    /// The table reading of the segment `i` of the view's tree.
    pub(crate) fn reading_at(&self, i: usize) -> Option<&table::Find> {
        let seg = self.segs.all.get(i)?;
        Some(self.readings[i].get_or_init(|| table::Find::of(self, seg)))
    }

    /// The segments of the view's tree read as tables, by the bottom of their tables, lowest
    /// first: worked out once, as every table reading is worked out once.
    pub(crate) fn tables(&self) -> &[(f32, usize)] {
        self.tables.get_or_init(|| {
            let mut out: Vec<(f32, usize)> = (0..self.segs.all.len())
                .filter_map(|i| match self.reading_at(i) {
                    Some(table::Find::Table(t)) => Some((t.bounds[1], i)),
                    _ => None,
                })
                .collect();
            crate::work::add(self.segs.all.len() as u64);
            out.sort_by(|a, b| a.0.total_cmp(&b.0));
            out
        })
    }

    /// Whether the segment `i` of the view's tree is a box that touches another box of its
    /// parent (see [`pairs::touching_boxes`]).
    pub(crate) fn touches(&self, i: usize) -> bool {
        self.touching
            .get_or_init(|| pairs::touching_boxes(self))
            .get(i)
            .copied()
            .unwrap_or(false)
    }

    /// The label styles of the page that may seed weight-only pairs (see [`pairs::seeds`]).
    pub(crate) fn seeds(&self) -> &[Style] {
        self.seeds.get_or_init(|| pairs::seeds(self))
    }

    /// What the page's area says against a table (see [`Area`]).
    pub(crate) fn area(&self) -> Area<'_> {
        Area {
            width: self.page.size[0],
            height: self.page.size[1],
            images: &self.images,
            body_size: self.file.body_size,
        }
    }

    pub(crate) fn lines(&self) -> &[RunRange] {
        &self.lines
    }

    pub(crate) fn by_y(&self) -> &ByY {
        &self.by_y
    }

    pub(crate) fn hs(&self) -> &[Line] {
        &self.hs
    }

    pub(crate) fn per_line(&self) -> &HashMap<u32, u32> {
        &self.per_line
    }

    pub(crate) fn band(&self, line: u32) -> Option<Band> {
        self.bands.get(&line).copied()
    }

    pub(crate) fn two_page(&self, line: u32) -> bool {
        self.two_page.contains(&line)
    }

    pub(crate) fn in_legend(&self, run: u32) -> bool {
        self.legend.contains(&run)
    }

    pub(crate) fn title(&self) -> Option<&ViewTitle> {
        self.title.as_ref()
    }
}
