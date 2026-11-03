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

use crate::extract::text::title::page_title;
use crate::file::{Band, FileFacts, Levels, RunningIndex, text_sizes, two_page_line};
use crate::input::{PageIn, TextRun};
use crate::line::{LineRun, text_lines};
use crate::region::{Kind, Score, Signal};
use crate::segment::drawn::DrawnStructure;
use crate::segment::{Segment, Segments};

pub mod text;

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

/// A typer: the evidence for its kind in a segment of the page the view shows.
pub type Typer = fn(&PageView<'_>, &Segment) -> Option<Typed>;

/// Every typer, in [`Kind`] order. Spatial has none: it is what no clear evidence leaves.
pub const TYPERS: &[(Kind, Typer)] = &[
    (Kind::HeaderBand, type_header_band),
    (Kind::FooterBand, type_footer_band),
    (Kind::TitleBlock, type_title_block),
    (Kind::Heading, type_heading),
    (Kind::Paragraph, type_paragraph),
    (Kind::List, type_list),
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
            texts: (0..segs.all.len()).map(|_| OnceCell::new()).collect(),
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
        let own = self
            .segs
            .all
            .get(seg.id.0 as usize)
            .filter(|s| std::ptr::eq(*s, seg))
            .and_then(|_| self.texts.get(seg.id.0 as usize));
        match own {
            Some(cell) => cell
                .get_or_init(|| text::SegText::of(self, seg))
                .as_ref()
                .and_then(f),
            None => text::SegText::of(self, seg).as_ref().and_then(f),
        }
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
