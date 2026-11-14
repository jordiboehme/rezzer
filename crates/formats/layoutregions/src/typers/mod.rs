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
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::extract::pairs::Style;
use crate::extract::table::area::Area;
use crate::extract::table::caption::ByY;
use crate::extract::table::{RunRange, run_ranges, texted_per_line};
use crate::extract::text::title::page_title;
use crate::file::{Band, FileFacts, Legend, Levels, RunningIndex, text_sizes, two_page_line};
use crate::input::{PageIn, PrimId, TextRun};
use crate::line::{LineRun, size_key, text_lines};
use crate::region::{Kind, Score, Signal};
use crate::segment::drawn::{DrawnStructure, Grids, Line};
use crate::segment::{SegId, Segment, Segments};

pub mod code;
pub mod figure;
pub mod pairs;
pub mod table;
pub mod text;

pub use code::type_code;
pub use figure::type_figure;
pub use pairs::type_label_value;
pub use table::{TOTALS_LINE_PAIR, type_spatial, type_table, type_totals};
pub use text::{
    type_footer_band, type_footnote_legend, type_header_band, type_heading, type_list,
    type_paragraph, type_title_block,
};

/// The highest score.
pub const MAX_SCORE: u16 = 1000;

/// How many lines of the page over and under a segment its walk looks at: the two-line look of
/// the heading rule (see [`crate::extract::text::heading_block`]).
pub(crate) const CONTEXT_LINES: usize = 2;

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
    text_alone_in(view.segs, seg)
}

/// [`text_alone`] for a segment of `segs`, before there is a view of the page.
fn text_alone_in(segs: &Segments, seg: &Segment) -> bool {
    use crate::segment::Source;
    matches!(
        seg.source,
        Source::Gutter
            | Source::Band
            | Source::Rule
            | Source::Style
            | Source::Rest
            | Source::Walk
            | Source::Line
            | Source::Strip
            | Source::Cell
    ) && seg.children.iter().all(|c| {
        segs.all.get(c.0 as usize).is_some_and(|k| {
            matches!(
                k.source,
                Source::Gutter
                    | Source::Band
                    | Source::Rule
                    | Source::Style
                    | Source::Rest
                    | Source::Walk
                    | Source::Line
                    | Source::Cell
            )
        })
    })
}

/// The runs of `page` that are values in a row of fields: in a row of labels over their
/// values as the whitespace cut found it ([`crate::segment::Source::Cell`]: labels side by
/// side, each a size step smaller than the value under it, or a row set like such a row),
/// the runs that stand under a label (see
/// [`crate::extract::pairs::blocks::labels_with_values`]). What is filled into a field is no
/// title of the page, however large and bold the form prints it: the title is looked for
/// among the other lines. Linear in the runs of the cells.
///
/// Text that is mostly digits and signs (more of them than letters: a number, an amount, a
/// date, a code) is a value wherever it stands. A value of words is one too, with one
/// exception: the largest text of the top of the page is never kept out of the title
/// search. The row is found by its geometry alone, and a page title under a small kicker
/// line stands the same way when a label over its value stands beside the two (`Department
/// of Samples` over `Annual Report`, `Form` over `12` at the right). What tells the two
/// apart is the page's own: a title is the one text set largest in the top of the page
/// (the top third, where [`page_title`] looks for it, see `title::HEAD_SHARE`), and the
/// values of a form's fields are several texts in one size. So a value of words stays in
/// the title search when it stands in the top of the page and no other text there is set
/// as large or larger (`body` names the runs the title is looked for in; sizes by
/// [`size_key`]; values of digits and signs are not counted, they are no title in any
/// size: the form number beside the title is set as large as it). No constant of its own,
/// and no count of words: a count let short titles through or long ones, never both.
fn field_values(
    page: &PageIn,
    segs: &Segments,
    height: f32,
    body: &dyn Fn(u32) -> bool,
) -> HashSet<u32> {
    // The values of the rows of fields: the runs of each, and whether it is digits and signs.
    let mut found: Vec<(Vec<u32>, bool)> = Vec::new();
    for seg in &segs.all {
        crate::work::add(1);
        if seg.source != crate::segment::Source::Cell || !text_alone_in(segs, seg) {
            continue;
        }
        let ids: Vec<u32> = seg
            .prims
            .iter()
            .filter_map(|p| match *p {
                PrimId::Text(i) if (i as usize) < page.texts.len() => Some(i),
                _ => None,
            })
            .collect();
        let refs: Vec<&TextRun> = ids.iter().map(|&i| &page.texts[i as usize]).collect();
        let Some(under) = crate::extract::pairs::blocks::labels_with_values(&refs) else {
            continue;
        };
        for u in &under {
            let value = crate::extract::pairs::words(&refs, &u.value);
            let inked = || value.chars().filter(|c| !c.is_whitespace());
            let letters = inked().filter(|c| c.is_alphabetic()).count();
            let data = inked().count() > 2 * letters;
            found.push((u.value.iter().map(|&k| ids[k]).collect(), data));
        }
    }
    let mut values: HashSet<u32> = found
        .iter()
        .filter(|f| f.1)
        .flat_map(|f| f.0.iter().copied())
        .collect();
    // The text of the top of the page, values of digits and signs aside, with its size.
    crate::work::add(page.texts.len() as u64);
    let key = |i: u32| size_key(page.texts[i as usize].size);
    let top: HashSet<u32> = (0..page.texts.len() as u32)
        .filter(|&i| {
            let r = &page.texts[i as usize];
            body(i)
                && !values.contains(&i)
                && !r.text.trim().is_empty()
                && r.y + r.h >= height * crate::extract::text::title::HEAD_SHARE
        })
        .collect();
    let largest = top.iter().map(|&i| key(i)).max();
    let set_largest = |i: &u32| top.contains(i) && Some(key(*i)) == largest;
    let runs_largest = top.iter().filter(|i| set_largest(i)).count();
    for (ids, _) in found.iter().filter(|f| !f.1) {
        let inked: Vec<&u32> = ids
            .iter()
            .filter(|&&i| !page.texts[i as usize].text.trim().is_empty())
            .collect();
        // The largest text of the top of the page: every run of it stands there in the
        // largest size, and no other text there is as large.
        let title = !inked.is_empty()
            && inked.iter().all(|i| set_largest(i))
            && inked.len() == runs_largest;
        if !title {
            values.extend(ids.iter().copied());
        }
    }
    values
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

/// Every typer, in [`Kind`] order. Spatial is what no clear evidence leaves: its typer gives
/// only signals of weight 0 that say why the table rules refused a segment (see
/// [`type_spatial`]), never a candidate that can win.
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
    (Kind::Figure, type_figure),
    (Kind::Code, type_code),
    (Kind::FootnoteLegend, type_footnote_legend),
    (Kind::Spatial, type_spatial),
];

/// What every typer of [`TYPERS`] finds in each segment of the view's tree, the segments of
/// alternative cuts included, by segment: the input of [`crate::arbitrate()`]. Each segment
/// is passed as it stands in the tree, so its text and readings are worked out once for all
/// typers. A segment no typer finds evidence in has an empty list.
pub fn type_segments(view: &PageView<'_>) -> BTreeMap<SegId, Vec<Typed>> {
    view.segs
        .all
        .iter()
        .map(|seg| {
            crate::work::add(1);
            let typed = TYPERS
                .iter()
                .filter_map(|(_, typer)| typer(view, seg))
                .collect();
            (seg.id, typed)
        })
        .collect()
}

/// The segment tree of the view with each text leaf whose walk reads blocks of more than one
/// text kind (a heading over its paragraph, list items and a paragraph) cut at the walk's block
/// boundaries: one [`Source::Walk`] child per run of blocks of one kind, so each kind is typed
/// on its own lines. Lines with no text go with the block before them. A page number of a leaf
/// that stands apart from its other lines far under them is cut off too and stands last (see
/// [`band_cut`]): a leaf of few lines is not parted by white space, so a page number far under
/// the last line of a page shares its leaf. `None` when no leaf is cut. The caller types the new tree with a new view. No segment is added past
/// `max_regions` segments: the leaves left then stand as they are, and an issue of kind
/// `Limit` says so.
///
/// [`Source::Walk`]: crate::segment::Source::Walk
pub(crate) fn walk_cut(view: &PageView<'_>, max_regions: usize) -> Option<Segments> {
    use crate::segment::Source;
    let mut cuts: Vec<(usize, Vec<Vec<PrimId>>)> = Vec::new();
    for (k, seg) in view.segs.all.iter().enumerate() {
        crate::work::add(1 + seg.prims.len() as u64);
        if !seg.children.is_empty() || !seg.alternatives.is_empty() || !text_alone(view, seg) {
            continue;
        }
        let Some(spans) = view.with_text(seg, |t| {
            let kinds: Vec<Kind> = t.blocks.iter().map(text::Read::kind).collect();
            let mixed = kinds.windows(2).any(|w| w[0] != w[1]);
            Some(mixed.then(|| (kinds, t.spans.clone())))
        }) else {
            continue;
        };
        // The segment's lines, as the walk read them.
        let mut refs: Vec<(&TextRun, u32)> = Vec::new();
        for p in &seg.prims {
            if let PrimId::Text(i) = *p
                && let Some(r) = view.page.texts.get(i as usize)
            {
                refs.push((r, i));
            }
        }
        let banded = band_cut(view, &refs);
        let Some(spans) = spans else {
            if let Some(b) = banded {
                let body: Vec<PrimId> = refs
                    .iter()
                    .filter(|(_, i)| !b.contains(i))
                    .map(|(_, i)| PrimId::Text(*i))
                    .collect();
                cuts.push((k, b.around(vec![body])));
            }
            continue;
        };
        let mut line_of: Vec<usize> = Vec::with_capacity(refs.len());
        let mut n = 0usize;
        for (j, (r, _)) in refs.iter().enumerate() {
            if j > 0 && refs[j - 1].0.line != r.line {
                n += 1;
            }
            line_of.push(n);
        }
        let (kinds, spans) = spans;
        // The groups: consecutive blocks of one kind, by their first line.
        let mut starts: Vec<usize> = Vec::new();
        for (b, &(i, _)) in spans.iter().enumerate() {
            if b == 0 || kinds[b] != kinds[b - 1] {
                starts.push(if b == 0 { 0 } else { i });
            }
        }
        let mut groups: Vec<Vec<PrimId>> = vec![Vec::new(); starts.len()];
        for (j, (_, id)) in refs.iter().enumerate() {
            let g = starts
                .partition_point(|&s| s <= line_of[j])
                .saturating_sub(1);
            groups[g].push(PrimId::Text(*id));
        }
        groups.retain(|g| !g.is_empty());
        if let Some(b) = banded {
            // The page numbers leave the walk's groups and stand last.
            let body = groups
                .into_iter()
                .map(|g| {
                    g.into_iter()
                        .filter(|p| match p {
                            PrimId::Text(i) => !b.contains(i),
                            _ => true,
                        })
                        .collect()
                })
                .collect();
            groups = b.around(body);
        }
        if groups.len() >= 2 {
            cuts.push((k, groups));
        }
    }
    if cuts.is_empty() {
        return None;
    }
    let mut segs = view.segs.clone();
    crate::work::add(segs.all.len() as u64);
    let mut over = false;
    for (k, groups) in cuts {
        // A leaf the bound leaves whole is short of its cut.
        if over || segs.all.len() + groups.len() > max_regions {
            if !over {
                segs.issues.push(crate::region::Issue {
                    what: format!(
                        "the text walk's cuts were not all made: more than {max_regions} \
                         segments"
                    ),
                    kind: crate::region::IssueKind::Limit,
                });
            }
            over = true;
            let id = segs.all[k].id;
            if let Err(at) = segs.short.binary_search(&id) {
                segs.short.insert(at, id);
            }
            continue;
        }
        let parent = segs.all[k].id;
        for mut prims in groups {
            prims.sort_unstable();
            let id = SegId(segs.all.len() as u32);
            let bounds = text_extent(view.page, &prims);
            segs.all.push(Segment {
                id,
                bounds,
                prims,
                source: Source::Walk,
                parent: Some(parent),
                children: Vec::new(),
                alternatives: Vec::new(),
            });
            segs.all[k].children.push(id);
        }
    }
    Some(segs)
}

/// A page number of a leaf is cut off from the leaf's other lines when the white space between
/// them is more than this many times its height: a page number far under the text, not a
/// number set close under the last line of the text.
const BAND_APART: f32 = 2.0;

/// A page number holds at most this many digits.
const PAGE_NUMBER_DIGITS: usize = 4;

/// A page number holds at most this many words that are no number: the word for the page and
/// the word between the page and the count of the pages (`Page 7 of 12`).
const PAGE_NUMBER_WORDS: usize = 2;

/// Whether a line's text has the shape of a page number: the number of the page, maybe with
/// the count of the pages after it, each of at most [`PAGE_NUMBER_DIGITS`] digits; maybe
/// between dashes (`7`, `- 7 -`), with a slash between the two numbers (`7 / 12`, `7/12`),
/// and with at most [`PAGE_NUMBER_WORDS`] words of letters, a stop or a colon after them
/// (`Page 7`, `p. 7`, `Page 7 of 12`). The words are not looked up: which words a file writes
/// there is its own. The shape alone is no proof: `Total 120` and `Amount due: 120` have it
/// too, so [`band_cut`] asks more of a line with a word or a second number (see
/// [`bare_number`]). A line with any other sign, a longer number, a third number or a third
/// word is none.
fn page_number(text: &str) -> bool {
    let t = text
        .trim()
        .trim_matches(|c: char| c == '-' || c.is_whitespace());
    let (mut numbers, mut words, mut slashes) = (0usize, 0usize, 0usize);
    for token in t.split_whitespace().flat_map(|w| w.split_inclusive('/')) {
        let (token, slash) = match token.strip_suffix('/') {
            Some(rest) => (rest, 1),
            None => (token, 0),
        };
        slashes += slash;
        // A slash stands between the two numbers only.
        if slash == 1 && (numbers + usize::from(!token.is_empty()) != 1 || slashes > 1) {
            return false;
        }
        if token.is_empty() {
            continue;
        }
        let word = token.trim_end_matches(['.', ':']);
        if token.len() <= PAGE_NUMBER_DIGITS && token.chars().all(|c| c.is_ascii_digit()) {
            numbers += 1;
        } else if !word.is_empty() && word.chars().all(char::is_alphabetic) {
            words += 1;
        } else {
            return false;
        }
    }
    (1..=2).contains(&numbers) && words <= PAGE_NUMBER_WORDS && (slashes == 0 || numbers == 2)
}

/// Whether a line in the shape of a page number ([`page_number`]) is a number alone, maybe
/// between dashes (`7`, `- 7 -`): no word and no second number.
fn bare_number(text: &str) -> bool {
    let t = text.trim_matches(|c: char| c == '-' || c.is_whitespace());
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
}

/// The runs of a leaf's page numbers that stand apart from its other lines (see [`band_cut`]).
struct BandCut {
    bottom: HashSet<u32>,
}

impl BandCut {
    /// The groups of a leaf: `body` (the walk's groups without the page number runs), then the
    /// page number runs; empty groups left out.
    fn around(self, body: Vec<Vec<PrimId>>) -> Vec<Vec<PrimId>> {
        let mut bottom: Vec<PrimId> = self.bottom.into_iter().map(PrimId::Text).collect();
        bottom.sort_unstable();
        body.into_iter()
            .chain(std::iter::once(bottom))
            .filter(|g| !g.is_empty())
            .collect()
    }

    fn contains(&self, run: &u32) -> bool {
        self.bottom.contains(run)
    }
}

/// The page numbers of the leaf of runs `refs` that stand apart from its other lines: lines of
/// the leaf in the shape of a page number (see [`page_number`]) in the foot band of the page as
/// a running footer of the file (see [`PageView::band`]: a running line below all other text
/// of the page), whose gap to the lowest other line of the leaf with ink is more than
/// [`BAND_APART`] times their own height. `None` when the leaf has no such line, or no other
/// line with ink.
///
/// A line with a word or a second number (`Page 7`, `7 / 12`) is a page number only when the
/// running line counts the pages of the file ([`crate::file::Running::counts`]: its number
/// goes up by one from page to page, a count after it is the same on every page and not
/// below it). A line that repeats (`Amount due: 120` on both pages, `7 / 12` under each
/// table) or whose number changes in another way (`Total 120`, `Total 140`) is a value and
/// stays with its leaf: a value with the role of a page footer is a wrong structure. A
/// number alone ([`bare_number`]) keeps the rule it had.
fn band_cut(view: &PageView<'_>, refs: &[(&TextRun, u32)]) -> Option<BandCut> {
    crate::work::add(refs.len() as u64);
    // The text and the extent (bottom, top) of the leaf's lines with ink, by line number.
    let mut lines: HashMap<u32, (String, f32, f32)> = HashMap::new();
    for (r, _) in refs {
        if crate::line::ink(&r.text) == 0 {
            continue;
        }
        let (lo, hi) = (r.y.min(r.y + r.h), r.y.max(r.y + r.h));
        let e = lines
            .entry(r.line)
            .or_insert((String::new(), f32::MAX, f32::MIN));
        e.0.push_str(&r.text);
        e.1 = e.1.min(lo);
        e.2 = e.2.max(hi);
    }
    let number = |n: u32, t: &str| {
        view.band(n) == Some(Band::Bottom)
            && page_number(t)
            && (bare_number(t) || view.counts_pages(n))
    };
    let body_low = lines
        .iter()
        .filter(|(n, (t, _, _))| !number(**n, t))
        .map(|(_, e)| e.1)
        .fold(f32::MAX, f32::min);
    if body_low == f32::MAX {
        return None;
    }
    let cut: HashSet<u32> = lines
        .iter()
        .filter(|(n, (t, lo, hi))| number(**n, t) && body_low - hi > BAND_APART * (hi - lo))
        .map(|(n, _)| *n)
        .collect();
    if cut.is_empty() {
        return None;
    }
    let bottom = refs
        .iter()
        .filter(|(r, _)| cut.contains(&r.line))
        .map(|(_, i)| *i)
        .collect();
    Some(BandCut { bottom })
}

/// The box of the text runs `prims` of `page`.
fn text_extent(page: &PageIn, prims: &[PrimId]) -> crate::region::Bounds {
    let mut b = crate::region::Bounds {
        x0: f32::MAX,
        y0: f32::MAX,
        x1: f32::MIN,
        y1: f32::MIN,
    };
    for p in prims {
        if let PrimId::Text(i) = *p
            && let Some(r) = page.texts.get(i as usize)
        {
            b.x0 = b.x0.min(r.x.min(r.x + r.w));
            b.x1 = b.x1.max(r.x.max(r.x + r.w));
            b.y0 = b.y0.min(r.y.min(r.y + r.h));
            b.y1 = b.y1.max(r.y.max(r.y + r.h));
        }
    }
    b
}

/// The segments of a page and every typer's candidates on them: the tree `segs` with the text
/// leaves that mix kinds cut by [`walk_cut`] (within `limits.max_regions` segments), then
/// [`type_segments`] over that tree. The input of [`crate::arbitrate()`]. Where the walk cuts
/// a leaf, the page is viewed a second time for the new tree (its page-level indexes are built
/// again).
pub fn type_page(
    page: &PageIn,
    drawn: &DrawnStructure,
    segs: Segments,
    file: &FileFacts,
    index: &FileIndex,
    ends_in_amount: &dyn Fn(&str) -> bool,
    limits: &crate::Limits,
) -> (Segments, BTreeMap<SegId, Vec<Typed>>) {
    let facts = PageFacts::of(file, index, page.page);
    type_page_with(page, drawn, segs, facts, ends_in_amount, limits)
}

/// [`type_page`] with the facts of the page handed in (see [`PageFacts`]): for a host whose
/// page names its runs by other ids than the legend its file learned.
pub fn type_page_with(
    page: &PageIn,
    drawn: &DrawnStructure,
    segs: Segments,
    facts: PageFacts<'_>,
    ends_in_amount: &dyn Fn(&str) -> bool,
    limits: &crate::Limits,
) -> (Segments, BTreeMap<SegId, Vec<Typed>>) {
    // A tree the walk does not cut is typed with the view the walk read it with, so the text
    // of its segments is worked out once.
    let cut = {
        let view = PageView::with(page, drawn, &segs, facts, ends_in_amount);
        match walk_cut(&view, limits.max_regions) {
            Some(cut) => cut,
            None => {
                let typed = type_segments(&view);
                let unparted = view.strips_unparted();
                drop(view);
                let mut segs = segs;
                say_unparted(&mut segs, unparted);
                return (segs, typed);
            }
        }
    };
    let mut segs = cut;
    let (typed, unparted) = {
        let view = PageView::with(page, drawn, &segs, facts, ends_in_amount);
        (type_segments(&view), view.strips_unparted())
    };
    say_unparted(&mut segs, unparted);
    (segs, typed)
}

/// Says in the issues of `segs` that the page's strips were read without their dividers
/// (see [`pairs::Strips::unparted`]), when they were.
fn say_unparted(segs: &mut Segments, unparted: bool) {
    if unparted {
        segs.issues.push(crate::region::Issue {
            what: pairs::DIVIDERS_UNSEARCHED.to_string(),
            kind: crate::region::IssueKind::Limit,
        });
    }
}

/// The facts of a file as one page is typed with them: the facts and their index, and the
/// footnote legend of that page alone. The typers read no other page's legend, so a host
/// that renames the legend's runs for a page (see [`crate::RunOrder::legend`]) hands only
/// that one legend and copies no facts of the file.
#[derive(Debug, Clone, Copy)]
pub struct PageFacts<'a> {
    /// What the file learned across its pages (running lines, body size, heading sizes).
    pub file: &'a FileFacts,
    /// The index of the file's facts by page.
    pub index: &'a FileIndex,
    /// The legend of the page, its runs named by the ids of the page that is typed.
    pub legend: Option<&'a Legend>,
}

impl<'a> PageFacts<'a> {
    /// The facts for page `page` of the file, with the legend the file learned for it.
    pub fn of(file: &'a FileFacts, index: &'a FileIndex, page: u32) -> PageFacts<'a> {
        PageFacts {
            file,
            index,
            legend: file.legends.get(&page),
        }
    }
}

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
    /// The line numbers of the running lines that count the pages (see
    /// [`crate::file::Running::counts`]).
    counting: HashSet<u32>,
    /// The line numbers of the running footers of the two-page rule.
    two_page: HashSet<u32>,
    /// The runs of the page's footnote legend.
    legend: HashSet<u32>,
    /// The page's footnote legend (see [`PageFacts::legend`]).
    page_legend: Option<&'a Legend>,
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
    /// The place in `lines` of each line number of the page (its first range).
    line_at: HashMap<u32, usize>,
    /// The table reading of each segment of `segs`, worked out once.
    readings: Vec<OnceCell<table::Find>>,
    /// The totals reading of each segment of `segs`, worked out once.
    totals: Vec<OnceCell<Option<table::TotalsRead>>>,
    /// The label styles that pairs with independent evidence show (see [`pairs::seeds`]).
    seeds: OnceCell<Vec<Style>>,
    /// The label/value candidate of each segment, worked out once.
    pairs: Vec<OnceCell<Option<Typed>>>,
    /// The figure candidate of each segment, worked out once.
    figures: Vec<OnceCell<Option<Typed>>>,
    /// The code candidate of each segment, worked out once.
    codes: Vec<OnceCell<Option<Typed>>>,
    /// Whether each segment's lines read as sentences across a ruled table, worked out once.
    across: Vec<OnceCell<bool>>,
    /// The segments read as tables, by the bottom of their tables, lowest first.
    tables: OnceCell<Vec<(f32, usize)>>,
    /// For each segment, whether it is a box that touches another box of its parent (see
    /// [`pairs::touching_boxes`]).
    touching: OnceCell<Vec<bool>>,
    /// The strips between the page's rules (see [`pairs::strip_frames`]), found once.
    strip_frames: OnceCell<pairs::Strips>,
    /// The strip each run in a strip stands in, by run id.
    strip_of: OnceCell<HashMap<u32, usize>>,
    /// The innermost box segment of each run that stands in one, by run id.
    box_of: OnceCell<HashMap<u32, usize>>,
    /// The strip fields of each segment (see [`pairs::strip_fields`]), worked out once.
    strips: Vec<OnceCell<Option<pairs::StripFields>>>,
    /// How many boxes the runs of each segment stand in (see [`PageView::boxes_of`]).
    boxes: Vec<OnceCell<usize>>,
    /// The pieces of each of the page's lines (see [`PageView::line_pieces`]), by place in
    /// `lines`, worked out once.
    line_pieces: Vec<OnceCell<Vec<crate::extract::table::grid::Piece>>>,
    /// The pieces of all the page's lines by left edge and height (see [`PageView::edges_near`]).
    edges: OnceCell<Edges>,
    /// The frames of the page's groups of drawn lines (see [`table::Frames`]), found once.
    frames: OnceCell<table::Frames>,
}

/// The pieces of a page's lines (see [`PageView::line_pieces`]) by their left edge, in buckets
/// half the page's smallest line size wide, and by the height of their line within a bucket:
/// so a look for the pieces at one left edge in a band of heights reads only those buckets.
struct Edges {
    width: f32,
    /// The bucket of the left edge, the line's height (`RunRange::y`), its place in the page's
    /// lines and the place of the piece in the line's pieces.
    keys: Vec<(i64, f32, u32, u32)>,
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
        let facts = PageFacts::of(file, index, page.page);
        PageView::with(page, drawn, segs, facts, ends_in_amount)
    }

    /// [`PageView::new`] with the facts of the page handed in: the legend is `facts.legend`,
    /// not the one the file holds for the page.
    pub fn with(
        page: &'a PageIn,
        drawn: &'a DrawnStructure,
        segs: &'a Segments,
        facts: PageFacts<'a>,
        ends_in_amount: &'a dyn Fn(&str) -> bool,
    ) -> PageView<'a> {
        let PageFacts {
            file,
            index,
            legend: page_legend,
        } = facts;
        let height = page.size[1];
        crate::work::add(page.texts.len() as u64);
        let lines = text_lines(&page.texts);
        let found = index.running.bands(&lines, height, &[]);
        let mut bands = HashMap::new();
        let mut two_page = HashSet::new();
        let mut counting = HashSet::new();
        for (l, b) in lines.iter().zip(found) {
            let Some(b) = b else { continue };
            let number = l.runs[0].line;
            bands.insert(number, b);
            if index.running.counts_pages(l, height) {
                counting.insert(number);
            }
            if two_page_line(l, height, &index.two_page) {
                two_page.insert(number);
            }
        }
        let legend: HashSet<u32> = page_legend
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
            let values = field_values(page, segs, height, &|i| {
                body(&(i as usize, &page.texts[i as usize]))
            });
            let runs: Vec<&TextRun> = page
                .texts
                .iter()
                .enumerate()
                .filter(body)
                .filter(|(i, _)| !values.contains(&(*i as u32)))
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
        let mut line_at: HashMap<u32, usize> = HashMap::new();
        for (k, l) in page_lines.iter().enumerate() {
            line_at.entry(page.texts[l.start].line).or_insert(k);
        }
        let line_count = page_lines.len();
        PageView {
            page,
            drawn,
            segs,
            file,
            index,
            ends_in_amount,
            bands,
            counting,
            two_page,
            legend,
            page_legend,
            text_sizes: sizes,
            title,
            texts: (0..n).map(|_| OnceCell::new()).collect(),
            lines: page_lines,
            by_y,
            images,
            hs,
            per_line: texted_per_line(&page.texts),
            line_at,
            readings: (0..n).map(|_| OnceCell::new()).collect(),
            totals: (0..n).map(|_| OnceCell::new()).collect(),
            seeds: OnceCell::new(),
            pairs: (0..n).map(|_| OnceCell::new()).collect(),
            figures: (0..n).map(|_| OnceCell::new()).collect(),
            codes: (0..n).map(|_| OnceCell::new()).collect(),
            across: (0..n).map(|_| OnceCell::new()).collect(),
            tables: OnceCell::new(),
            touching: OnceCell::new(),
            strip_frames: OnceCell::new(),
            strip_of: OnceCell::new(),
            box_of: OnceCell::new(),
            strips: (0..n).map(|_| OnceCell::new()).collect(),
            boxes: (0..n).map(|_| OnceCell::new()).collect(),
            line_pieces: (0..line_count).map(|_| OnceCell::new()).collect(),
            edges: OnceCell::new(),
            frames: OnceCell::new(),
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

    /// The strips between the page's rules (see [`pairs::strip_frames`]).
    pub(crate) fn strip_frames(&self) -> &[pairs::StripFrame] {
        &self
            .strip_frames
            .get_or_init(|| pairs::strip_frames(self))
            .frames
    }

    /// Whether the strips of the page were looked for and are read without their dividers,
    /// as the search for them went past its bound (see [`pairs::Strips::unparted`]).
    fn strips_unparted(&self) -> bool {
        self.strip_frames.get().is_some_and(|s| s.unparted)
    }

    /// How many box segments of the view's tree the text runs of `seg` stand in: each run
    /// counts in the innermost box that holds it (the box of most depth), a run in no box not
    /// at all.
    pub(crate) fn boxes_of(&self, seg: &Segment) -> usize {
        let at = self.box_of.get_or_init(|| {
            let all = &self.segs.all;
            let depth = |s: &Segment| {
                let (mut d, mut up) = (0usize, s.parent);
                while let Some(p) = up.and_then(|p| all.get(p.0 as usize)) {
                    crate::work::add(1);
                    d += 1;
                    up = p.parent;
                }
                d
            };
            let mut best: HashMap<u32, (usize, usize)> = HashMap::new();
            for (k, b) in all.iter().enumerate() {
                if b.source != crate::segment::Source::Box {
                    continue;
                }
                let d = depth(b);
                crate::work::add(b.prims.len() as u64);
                for p in &b.prims {
                    if let PrimId::Text(i) = *p
                        && best.get(&i).is_none_or(|&(_, e)| d > e)
                    {
                        best.insert(i, (k, d));
                    }
                }
            }
            best.into_iter().map(|(i, (k, _))| (i, k)).collect()
        });
        self.cached(
            &self.boxes,
            seg,
            || {
                crate::work::add(seg.prims.len() as u64);
                let boxes: HashSet<usize> = seg
                    .prims
                    .iter()
                    .filter_map(|p| match *p {
                        PrimId::Text(i) => at.get(&i).copied(),
                        _ => None,
                    })
                    .collect();
                boxes.len()
            },
            |n| *n,
        )
    }

    /// The strip of each run that stands in one, by run id.
    pub(crate) fn strip_of(&self) -> &HashMap<u32, usize> {
        self.strip_of.get_or_init(|| {
            let mut at = HashMap::new();
            for (k, f) in self.strip_frames().iter().enumerate() {
                for &r in &f.runs {
                    at.insert(r, k);
                }
            }
            at
        })
    }

    /// The strip fields of `seg` (see [`pairs::strip_fields`]).
    pub(crate) fn strip_fields_of(&self, seg: &Segment) -> Option<pairs::StripFields> {
        self.cached(&self.strips, seg, || pairs::strip_fields(self, seg), |t| *t)
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

    /// The figure candidate of `seg` (see [`figure::type_figure`]).
    pub(crate) fn figure_of(&self, seg: &Segment) -> Option<Typed> {
        self.cached(
            &self.figures,
            seg,
            || figure::figure(self, seg),
            Clone::clone,
        )
    }

    /// The code candidate of `seg` (see [`code::type_code`]).
    pub(crate) fn code_of(&self, seg: &Segment) -> Option<Typed> {
        self.cached(&self.codes, seg, || code::code(self, seg), Clone::clone)
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

    /// The frames of the groups of `g`, the grids of this page (`drawn.grids`), found once.
    pub(crate) fn frames(&self, g: &Grids) -> &table::Frames {
        self.frames.get_or_init(|| table::Frames::of(g))
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

    /// The page's lines (ranges of `page.texts`) just over the line `first` and just under the
    /// line `last`, [`CONTEXT_LINES`] at most each, when the segment holding `inked` runs with
    /// ink of each holds the whole of those two lines: the lines a walk over the segment looks
    /// at for its demotions, as the walk over the page does. A line cut by a gutter has no
    /// context (the page's line holds the other column too).
    pub(crate) fn context(
        &self,
        first: (u32, u32),
        last: (u32, u32),
    ) -> (&[RunRange], &[RunRange]) {
        let whole = |(line, inked): (u32, u32)| self.per_line.get(&line) == Some(&inked);
        let (Some(&a), Some(&b)) = (self.line_at.get(&first.0), self.line_at.get(&last.0)) else {
            return (&[], &[]);
        };
        if !whole(first) || !whole(last) || a > b {
            return (&[], &[]);
        }
        let over = &self.lines[a.saturating_sub(CONTEXT_LINES)..a];
        let under = &self.lines
            [(b + 1).min(self.lines.len())..(b + 1 + CONTEXT_LINES).min(self.lines.len())];
        (over, under)
    }

    /// The pieces of the page's line `k` (a place in [`PageView::lines`], see
    /// [`crate::extract::table::grid::pieces_of`]), sorted by their left edge: worked out once
    /// per line for every segment that looks at it, so a page cut into many segments reads
    /// each of its lines once.
    pub(crate) fn line_pieces(&self, k: usize) -> &[crate::extract::table::grid::Piece] {
        let Some(cell) = self.line_pieces.get(k) else {
            return &[];
        };
        cell.get_or_init(|| {
            let r = &self.lines[k];
            let mut pieces =
                crate::extract::table::grid::pieces_of(&self.page.texts, r.start, r.end);
            crate::work::add((r.end - r.start) as u64);
            pieces.sort_by(|a, b| a.x0.total_cmp(&b.x0));
            pieces
        })
    }

    /// The pieces of the page's lines whose left edge lies within `tol` of `x0` and whose line
    /// stands at a height from `low` to `high` (`RunRange::y`), each with the place of its line
    /// in [`PageView::lines`], in the order of the lines by height (then by place) and of the
    /// runs within a line. The index is built once per page, on the first look.
    pub(crate) fn edges_near(
        &self,
        x0: f32,
        tol: f32,
        (low, high): (f32, f32),
    ) -> Vec<(usize, &crate::extract::table::grid::Piece)> {
        let e = self.edges.get_or_init(|| {
            let width = 0.5
                * self
                    .lines
                    .iter()
                    .map(|r| r.size.max(1.0))
                    .fold(f32::MAX, f32::min)
                    .min(1.0e6);
            let mut keys = Vec::new();
            for (k, r) in self.lines.iter().enumerate() {
                if !r.y.is_finite() {
                    continue;
                }
                for (j, q) in self.line_pieces(k).iter().enumerate() {
                    if q.x0.is_finite() {
                        keys.push(((q.x0 / width).floor() as i64, r.y, k as u32, j as u32));
                    }
                }
            }
            crate::work::add(keys.len() as u64);
            keys.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
            Edges { width, keys }
        });
        let mut out = Vec::new();
        if !(x0.is_finite() && tol.is_finite() && low <= high) {
            return out;
        }
        let (b0, b1) = (
            ((x0 - tol) / e.width).floor() as i64,
            ((x0 + tol) / e.width).floor() as i64,
        );
        let at = |b: i64, y: f32| {
            e.keys
                .partition_point(|&(kb, ky, _, _)| kb < b || (kb == b && ky < y))
        };
        let mut i = at(b0, low);
        let mut steps = 1u64;
        while let Some(&(kb, ky, k, j)) = e.keys.get(i) {
            steps += 1;
            if kb > b1 {
                break;
            }
            if ky < low {
                i = at(kb, low);
                continue;
            }
            if ky > high {
                i = e.keys.partition_point(|&(b, _, _, _)| b <= kb);
                continue;
            }
            let q = &self.line_pieces(k as usize)[j as usize];
            if (q.x0 - x0).abs() <= tol {
                out.push((k as usize, q));
            }
            i += 1;
        }
        crate::work::add(steps);
        out.sort_by(|a, b| {
            self.lines[a.0]
                .y
                .total_cmp(&self.lines[b.0].y)
                .then(a.0.cmp(&b.0))
                .then(a.1.start.cmp(&b.1.start))
        });
        out
    }

    /// The places in [`PageView::lines`] of the page's lines just over and just under the line
    /// `line`.
    pub(crate) fn neighbours_at(&self, line: u32) -> [Option<usize>; 2] {
        match self.line_at.get(&line) {
            Some(&k) => [
                k.checked_sub(1),
                (k + 1 < self.lines.len()).then_some(k + 1),
            ],
            None => [None, None],
        }
    }

    pub(crate) fn band(&self, line: u32) -> Option<Band> {
        self.bands.get(&line).copied()
    }

    /// Whether the line is a running line of the file whose lines count the pages.
    pub(crate) fn counts_pages(&self, line: u32) -> bool {
        self.counting.contains(&line)
    }

    pub(crate) fn two_page(&self, line: u32) -> bool {
        self.two_page.contains(&line)
    }

    pub(crate) fn in_legend(&self, run: u32) -> bool {
        self.legend.contains(&run)
    }

    /// The page's footnote legend, its runs named by the ids of the page.
    pub(crate) fn legend(&self) -> Option<&Legend> {
        self.page_legend
    }

    pub(crate) fn title(&self) -> Option<&ViewTitle> {
        self.title.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::{bare_number, page_number};

    /// A number alone, maybe between dashes, against a number with a word or a count.
    #[test]
    fn a_bare_number_has_no_word_and_no_count() {
        for yes in ["7", "- 7 -", " 12 ", "9999"] {
            assert!(page_number(yes) && bare_number(yes), "{yes}");
        }
        for no in ["Page 7", "7 / 12", "7/12", "7 12", "p. 7", "Total 120"] {
            assert!(page_number(no) && !bare_number(no), "{no}");
        }
        assert!(!bare_number("") && !bare_number("- -"));
    }

    /// The shape of a page number: one or two short numbers, a slash between the two, at
    /// most two words, dashes around; each bound moved both ways.
    #[test]
    fn the_shape_of_a_page_number() {
        for yes in [
            "7",
            "- 7 -",
            "Page 7",
            "p. 7",
            "7 / 12",
            "7/12",
            "Page 7 of 12",
            "Page 7 / 12",
            "Sheet: 7",
            "9999",
        ] {
            assert!(page_number(yes), "{yes}");
        }
        for no in [
            "",
            "Page",
            // A number of five digits, a third number, a third word.
            "10000",
            "7 / 12 / 3",
            "7 12 3",
            "Page 7 of all 12",
            // A slash that does not stand between the two numbers.
            "/ 7",
            "7 /",
            "Page/7",
            // Other signs: an amount, a date, a code.
            "12.5",
            "1,234",
            "2026-11-13",
            "A7",
            "(7)",
        ] {
            assert!(!page_number(no), "{no}");
        }
    }
}
