// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typers of text regions: paragraph, heading, list, title block, header and footer band
//! and footnote legend.
//!
//! Heading, paragraph and list read one walk of [`next_block`] over the segment's lines (see
//! [`SegText`]): a kind claims a segment only when the walk reads every line with text as
//! blocks of that kind, so the three compete only where the walk itself says two things (lines
//! of a heading size that read as a paragraph, see [`Demotion`]). Bands come from the running
//! lines of the file ([`crate::file::RunningIndex`], the two-page rule), the legend from the
//! file's footnote legends, the title block from [`crate::extract::text::title::page_title`].
//!
//! The weights are starting values for an arbitration that asks a score of at least 300 and a
//! lead of 150 over the next kind. The first signal of every kind but paragraph alone reaches
//! 450 or more. A paragraph's signals are smaller and add up: [`PARAGRAPH_TEXT_SIZE`] alone
//! reaches the least score, so a plain line of body text is a paragraph, and it never wins
//! against another kind's evidence. The kinds today's rules decide between win by 150 or more.

use crate::extract::table::grid::piece_count;
use crate::extract::text::{Demotion, TextBlock, TextCx, next_block, sentences};
use crate::file::{Band, bold_led, in_text_size, plain_size};
use crate::input::{PrimId, TextRun};
use crate::line::{ink, text_lines};
use crate::region::Kind;
use crate::segment::Segment;
use crate::typers::table::PARAGRAPH_PROSE_CELLS;
use crate::typers::{PageView, Typed, signal};

/// `heading.size_ratio` for a line under [`crate::file::HEADING_PLAIN_RATIO`] times the body
/// size (it needs bold ink to head, see [`plain_size`]) and at or over it.
pub const HEADING_SIZE: u16 = 450;
pub const HEADING_SIZE_LARGE: u16 = 550;
/// `heading.bold_lead`: the line is led by bold ink.
pub const HEADING_BOLD: u16 = 100;
/// `heading.title_lines`: [`crate::extract::text::heading_block`] keeps the block a heading:
/// none of its [`Demotion`]s applies. For one line this is the absence of the paragraph's
/// evidence rather than evidence of its own (Task 18b weighs it).
pub const HEADING_TITLE_LINES: u16 = 200;
/// `paragraph.text_size`: the lines are set in a text size of the page or the body size.
pub const PARAGRAPH_TEXT_SIZE: u16 = 300;
/// `paragraph.joins`: lines go on one another as one paragraph.
pub const PARAGRAPH_JOINS: u16 = 300;
/// `paragraph.sentences`: the lines read as sentences.
pub const PARAGRAPH_SENTENCES: u16 = 150;
/// `paragraph.lines_apart`, `.unwrapped`, `.cells`, `.address` and `.sentences_demoted`: lines
/// of a heading size read as a paragraph, for the reason the signal names.
pub const PARAGRAPH_DEMOTED: u16 = 800;
/// `list.marker`: each item opens with a list marker.
pub const LIST_MARKER: u16 = 450;
/// `list.items`: items follow one another as one list.
pub const LIST_ITEMS: u16 = 200;
/// `list.hanging`: an item goes on in lines right of its marker.
pub const LIST_HANGING: u16 = 100;
/// `header_band.running` and `footer_band.running`: running lines of the file.
pub const BAND_RUNNING: u16 = 900;
/// `footer_band.two_page`: footers of a file of two pages.
pub const BAND_TWO_PAGE: u16 = 100;
/// `footnote_legend.entries`: the runs of the page's footnote legend.
pub const LEGEND_ENTRIES: u16 = 900;
/// `footnote_legend.raised_refs`: raised reference numbers above the legend.
pub const LEGEND_RAISED: u16 = 100;
/// `title_block.largest_heading`: the largest heading in the head of page 1.
pub const TITLE_HEADING: u16 = 700;
/// `title_block.top_third`: its top stands in the top third of the page. `page_title` only
/// picks such a heading, so this is the second half of one rule's weight, not independent
/// evidence.
pub const TITLE_TOP: u16 = 300;

/// What the walk of the text lines reads one block as.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Read {
    /// A heading: its first line's size, whether that size is plain (see [`plain_size`]),
    /// whether that line is led by bold ink, and why it reads as a paragraph, if it does.
    Heading {
        size: f32,
        plain: bool,
        bold_lead: bool,
        demoted: Option<Demotion>,
    },
    /// A paragraph of `lines` lines, which read as sentences or not, and whether each of its
    /// lines is one piece (no gap of a cell between its runs).
    Paragraph {
        lines: usize,
        sentences: bool,
        whole: bool,
    },
    /// A list item that joins the item before it or not, of `lines` lines.
    Item { joins: bool, lines: usize },
}

impl Read {
    /// The text kind the block reads as: a heading that keeps its lines, a paragraph (a
    /// heading size that reads as a paragraph too), a list item.
    pub(crate) fn kind(&self) -> Kind {
        match self {
            Read::Heading { demoted: None, .. } => Kind::Heading,
            Read::Heading { .. } | Read::Paragraph { .. } => Kind::Paragraph,
            Read::Item { .. } => Kind::List,
        }
    }
}

/// The text of a segment as the typers read it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SegText {
    /// Its runs with ink, sorted (a segment's primitives are sorted).
    runs: Vec<u32>,
    /// The line numbers of its runs with ink, sorted, once each.
    lines: Vec<u32>,
    /// The blocks the walk reads its lines as, in order.
    pub(crate) blocks: Vec<Read>,
    /// The lines of each block, as a range of the segment's lines (in the order of its runs).
    pub(crate) spans: Vec<(usize, usize)>,
    /// Every line with ink is set in a text size of the page or in the body size.
    text_size: bool,
}

impl SegText {
    /// Why its blocks of a heading size read as a paragraph, in order, once per block.
    pub(crate) fn demotions(&self) -> impl Iterator<Item = Demotion> + '_ {
        self.blocks.iter().filter_map(|b| match b {
            Read::Heading { demoted, .. } => *demoted,
            _ => None,
        })
    }

    /// The text of `seg` on the page of `view`; `None` when it holds no run with ink.
    pub(crate) fn of(view: &PageView<'_>, seg: &Segment) -> Option<SegText> {
        let mut ids = Vec::new();
        let mut refs: Vec<&TextRun> = Vec::new();
        for p in &seg.prims {
            if let PrimId::Text(i) = *p
                && let Some(r) = view.page.texts.get(i as usize)
            {
                refs.push(r);
                if ink(&r.text) > 0 {
                    ids.push(i);
                }
            }
        }
        if ids.is_empty() {
            return None;
        }
        let mut lines: Vec<u32> = ids
            .iter()
            .map(|&i| view.page.texts[i as usize].line)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        let seg_lines = text_lines(&refs);
        // The page lines just over and under the segment, for the walk's look past its edges
        // (the heading rule looks at two lines over and under a block): the walk reads only
        // the segment's lines, its blocks end at the segment's last line.
        let inked_on = |line: u32| {
            refs.iter()
                .filter(|r| r.line == line && !r.text.trim().is_empty())
                .count() as u32
        };
        let (over, under) = match (seg_lines.first(), seg_lines.last()) {
            (Some(f), Some(l)) => {
                let (f, l) = (f.runs[0].line, l.runs[0].line);
                view.context((f, inked_on(f)), (l, inked_on(l)))
            }
            _ => (&[][..], &[][..]),
        };
        let texts = &view.page.texts;
        let mut ext: Vec<&TextRun> = Vec::new();
        for r in over {
            ext.extend(&texts[r.start..r.end]);
        }
        let before = text_lines(&ext).len();
        ext.extend(refs.iter().copied());
        for r in under {
            ext.extend(&texts[r.start..r.end]);
        }
        let all = text_lines(&ext);
        let end = before + seg_lines.len();
        let levels = view.levels();
        let block = vec![None; all.len()];
        let cx = TextCx {
            levels: &levels,
            block: &block,
        };
        crate::work::add((ext.len() + all.len()) as u64);
        let body = [view.file.body_size];
        let text_size = seg_lines
            .iter()
            .filter(|l| !l.text.is_empty())
            .all(|l| in_text_size(l.size, view.text_sizes()) || in_text_size(l.size, &body));
        // The walk as a reader makes it: an item leaves its list open, a line without text
        // leaves it as it is, a heading or a paragraph closes it.
        let mut blocks = Vec::new();
        let mut spans = Vec::new();
        let mut open = false;
        let mut i = before;
        while i < end {
            let (found, j) = next_block(&all, i, open, &cx);
            let j = j.min(end);
            let read = match found {
                TextBlock::Empty => None,
                TextBlock::Heading { demoted, .. } => {
                    open = false;
                    Some(Read::Heading {
                        size: all[i].size,
                        plain: plain_size(all[i].size, view.file.body_size),
                        bold_lead: bold_led(&all[i]),
                        demoted,
                    })
                }
                TextBlock::Paragraph { .. } => {
                    open = false;
                    Some(Read::Paragraph {
                        lines: j - i,
                        sentences: sentences(&all[i..j]),
                        whole: all[i..j]
                            .iter()
                            .all(|l| piece_count(l.runs, 0, l.runs.len()) <= 1),
                    })
                }
                TextBlock::Item { joins, .. } => {
                    open = true;
                    Some(Read::Item {
                        joins,
                        lines: j - i,
                    })
                }
            };
            if let Some(read) = read {
                blocks.push(read);
                spans.push((i - before, j - before));
            }
            i = j;
        }
        Some(SegText {
            runs: ids,
            lines,
            blocks,
            spans,
            text_size,
        })
    }
}

/// Whether a segment that holds `seg` (its parents up to the page) reads as a table.
fn in_a_table(view: &PageView<'_>, seg: &Segment) -> bool {
    let mut up = seg.parent;
    // The readings are cached per segment: a second look does no work.
    while let Some(p) = up.and_then(|p| view.segs.all.get(p.0 as usize)) {
        if super::table::type_table(view, p).is_some() {
            return true;
        }
        up = p.parent;
    }
    false
}

/// A stop closes an abbreviation when the word before it has at most this many letters
/// (`Nr.`, `Std.`, `Pos.`).
const ABBREVIATION_LETTERS: usize = 4;

/// Whether `text` is a line of titles, not a sentence: every word opens with a capital or a
/// digit, and every stop in it closes an abbreviation (see [`ABBREVIATION_LETTERS`]):
/// `Nr. Art Tage Std. Satz Brutto` heads columns, it says nothing.
pub(crate) fn title_words(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    !words.is_empty()
        && words.iter().all(|w| {
            let opens = w
                .chars()
                .find(|c| c.is_alphanumeric())
                .is_none_or(|c| c.is_uppercase() || c.is_ascii_digit());
            let letters = w.chars().filter(|c| c.is_alphabetic()).count();
            let stop = w.ends_with(['.', '!', '?']) || w.contains(['!', '?']);
            opens && (!stop || (w.ends_with('.') && letters <= ABBREVIATION_LETTERS))
        })
}

/// Paragraph: the walk reads every line as paragraphs, or as lines of a heading size that read
/// as a paragraph; lines in cells (pieces apart by a cell gap) only one at a time and outside
/// a segment that reads as a table, unless they read as prose across them. Signals: the lines are set in a text size of the page or the body size
/// (`paragraph.text_size`, the least evidence), lines go on one another (`paragraph.joins`),
/// they read as sentences (`paragraph.sentences`), and the reason a heading's lines read as a
/// paragraph (one signal per [`Demotion`]).
pub fn type_paragraph(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    // Lines of titles (every word capitalised, stops only after abbreviations) are no prose
    // and no sentences, wherever they stand.
    let titles = {
        let (refs, _) = super::seg_runs(view, seg);
        let lines = text_lines(&refs);
        let inked: Vec<&str> = lines
            .iter()
            .map(|l| l.text.as_str())
            .filter(|t| !t.is_empty())
            .collect();
        !inked.is_empty() && inked.iter().all(|t| title_words(t))
    };
    // What the table rules say of the segment: prose they refuse, or cells they read.
    let prose = super::table::refusal(view, seg)
        .filter(|r| r.kind == Kind::Paragraph && r.signal.weight > 0)
        .map(|r| r.signal)
        .or_else(|| {
            view.across(seg).then(|| {
                signal(
                    "paragraph.prose_cells",
                    "its lines read as sentences across the columns of a ruled table".into(),
                    PARAGRAPH_PROSE_CELLS,
                )
            })
        })
        .filter(|_| !titles);
    let in_table = super::table::type_table(view, seg).is_some();
    view.with_text(seg, |t| {
        let all = t.blocks.iter().all(|b| {
            matches!(
                b,
                Read::Paragraph { .. }
                    | Read::Heading {
                        demoted: Some(_),
                        ..
                    }
            )
        });
        if t.blocks.is_empty() || !(all || prose.is_some()) {
            return None;
        }
        // Lines that stand in pieces apart by a cell gap are cells: a paragraph never joins
        // such lines, and never reads a line of them where a segment that holds it reads as a
        // table (a row of a table whose reading is blocked keeps its cells as laid out),
        // unless they read as prose across them.
        let joins_cells = t
            .blocks
            .iter()
            .any(|b| matches!(b, Read::Paragraph { whole: false, lines, .. } if *lines >= 2));
        let row = t
            .blocks
            .iter()
            .any(|b| matches!(b, Read::Paragraph { whole: false, .. }))
            && in_a_table(view, seg);
        if (joins_cells || row) && prose.is_none() {
            return None;
        }
        let mut signals = Vec::new();
        let paragraphs = t
            .blocks
            .iter()
            .filter(|b| matches!(b, Read::Paragraph { .. }))
            .count();
        if t.text_size && paragraphs > 0 {
            signals.push(signal(
                "paragraph.text_size",
                "set in a text size of the page".into(),
                PARAGRAPH_TEXT_SIZE,
            ));
        }
        let joined: usize = t
            .blocks
            .iter()
            .filter_map(|b| match b {
                Read::Paragraph {
                    lines, whole: true, ..
                } if *lines >= 2 => Some(lines - 1),
                _ => None,
            })
            .sum();
        if joined > 0 {
            signals.push(signal(
                "paragraph.joins",
                format!("{joined} lines go on the line over them"),
                PARAGRAPH_JOINS,
            ));
        }
        if !titles
            && t.blocks.iter().any(|b| {
                matches!(
                    b,
                    Read::Paragraph {
                        sentences: true,
                        ..
                    }
                )
            })
        {
            signals.push(signal(
                "paragraph.sentences",
                "the lines read as sentences".into(),
                PARAGRAPH_SENTENCES,
            ));
        }
        // Lines of a heading size that stand apart, unwrapped or in cells inside a table's
        // reading are the table's rows: the table rule reads them, so those demotions are the
        // table's evidence, not the paragraph's. An address or sentences say what the lines
        // are, and they refuse the table (see `super::table`).
        let mut why: Vec<Demotion> = Vec::new();
        for d in t.demotions() {
            let geometric = matches!(
                d,
                Demotion::LinesApart | Demotion::Unwrapped | Demotion::Cells
            );
            if !(in_table && geometric) && !why.contains(&d) {
                why.push(d);
            }
        }
        for d in why {
            let (id, detail) = match d {
                Demotion::LinesApart => (
                    "paragraph.lines_apart",
                    "lines of a heading size are short items or fields",
                ),
                Demotion::Unwrapped => (
                    "paragraph.unwrapped",
                    "lines of a heading size do not wrap into one another",
                ),
                Demotion::Cells => (
                    "paragraph.cells",
                    "lines of a heading size stand in cells of columns",
                ),
                Demotion::Address => (
                    "paragraph.address",
                    "lines a little larger than the body are an address",
                ),
                Demotion::Sentences => (
                    "paragraph.sentences_demoted",
                    "lines a little larger than the body read as sentences",
                ),
            };
            signals.push(signal(id, detail.into(), PARAGRAPH_DEMOTED));
        }
        signals.extend(prose.clone());
        Typed::of(Kind::Paragraph, signals)
    })
}

/// Heading: the walk reads every line as headings by their size (see
/// [`crate::file::Levels::level`]). Signals: the size against the body size
/// (`heading.size_ratio`, more at [`crate::file::HEADING_PLAIN_RATIO`] and over), bold ink
/// that leads each heading (`heading.bold_lead`), and lines that stay a heading
/// (`heading.title_lines`; lines that read as a paragraph give the paragraph its evidence
/// instead).
pub fn type_heading(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    view.with_text(seg, |t| {
        let mut size = f32::MAX;
        let mut plain = false;
        let mut bold = true;
        let mut kept = true;
        for b in &t.blocks {
            let Read::Heading {
                size: s,
                plain: p,
                bold_lead,
                demoted,
            } = b
            else {
                return None;
            };
            size = size.min(*s);
            plain |= *p;
            bold &= *bold_lead;
            kept &= demoted.is_none();
        }
        if t.blocks.is_empty() {
            return None;
        }
        let weight = if plain {
            HEADING_SIZE
        } else {
            HEADING_SIZE_LARGE
        };
        let body = view.file.body_size;
        let detail = if body > 0.0 {
            format!("{:.2} times the body size", size / body)
        } else {
            "the file has no body size".into()
        };
        let mut signals = vec![signal("heading.size_ratio", detail, weight)];
        if bold {
            signals.push(signal(
                "heading.bold_lead",
                "led by bold ink".into(),
                HEADING_BOLD,
            ));
        }
        if kept {
            signals.push(signal(
                "heading.title_lines",
                "its lines stay a heading".into(),
                HEADING_TITLE_LINES,
            ));
        }
        Typed::of(Kind::Heading, signals)
    })
}

/// List: the walk reads every line as list items (see [`crate::extract::text::list_marker`]).
/// Signals: the markers (`list.marker`), items that follow one another (`list.items`), and
/// items that go on right of their marker (`list.hanging`).
pub fn type_list(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    view.with_text(seg, |t| {
        let mut items = 0;
        let mut joined = 0;
        let mut hanging = 0;
        for b in &t.blocks {
            let Read::Item { joins, lines } = b else {
                return None;
            };
            items += 1;
            joined += usize::from(*joins);
            hanging += usize::from(*lines > 1);
        }
        if items == 0 {
            return None;
        }
        let mut signals = vec![signal(
            "list.marker",
            format!("{items} items open with a list marker"),
            LIST_MARKER,
        )];
        if joined > 0 {
            signals.push(signal(
                "list.items",
                format!("{joined} items follow the item before them"),
                LIST_ITEMS,
            ));
        }
        if hanging > 0 {
            signals.push(signal(
                "list.hanging",
                format!("{hanging} items go on right of their marker"),
                LIST_HANGING,
            ));
        }
        Typed::of(Kind::List, signals)
    })
}

/// Header band: every line with ink is a running header of the file on this page.
pub fn type_header_band(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    view.with_text(seg, |t| {
        t.lines
            .iter()
            .all(|&l| view.band(l) == Some(Band::Top))
            .then(|| {
                vec![signal(
                    "header_band.running",
                    format!("{} lines repeat at the top of the pages", t.lines.len()),
                    BAND_RUNNING,
                )]
            })
            .and_then(|s| Typed::of(Kind::HeaderBand, s))
    })
}

/// Footer band: every line with ink is a running footer of the file on this page; footers of
/// the two-page rule say so in a second signal.
pub fn type_footer_band(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    view.with_text(seg, |t| {
        if !t.lines.iter().all(|&l| view.band(l) == Some(Band::Bottom)) {
            return None;
        }
        let mut signals = vec![signal(
            "footer_band.running",
            format!("{} lines repeat at the foot of the pages", t.lines.len()),
            BAND_RUNNING,
        )];
        if t.lines.iter().all(|&l| view.two_page(l)) {
            signals.push(signal(
                "footer_band.two_page",
                "at the foot of both pages of a file of two".into(),
                BAND_TWO_PAGE,
            ));
        }
        Typed::of(Kind::FooterBand, signals)
    })
}

/// Footnote legend: every run with ink is a run of the page's footnote legend (see
/// [`crate::file::FileFacts::legends`]); raised reference numbers above it are a second
/// signal.
pub fn type_footnote_legend(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    view.with_text(seg, |t| {
        if !t.runs.iter().all(|&r| view.in_legend(r)) {
            return None;
        }
        let legend = view.file.legends.get(&view.page.page)?;
        let entries = legend
            .entries
            .iter()
            .filter(|e| e.runs.iter().any(|r| t.runs.binary_search(r).is_ok()))
            .count();
        let mut signals = vec![signal(
            "footnote_legend.entries",
            format!("{entries} footnotes of the page's legend"),
            LEGEND_ENTRIES,
        )];
        if !legend.raised.is_empty() {
            signals.push(signal(
                "footnote_legend.raised_refs",
                format!("{} raised references above it", legend.raised.len()),
                LEGEND_RAISED,
            ));
        }
        Typed::of(Kind::FootnoteLegend, signals)
    })
}

/// Title block: the lines with ink are the lines of the title of page 1, the largest heading
/// in its head (see [`crate::extract::text::title::page_title`]).
pub fn type_title_block(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    let title = view.title()?;
    view.with_text(seg, |t| {
        if t.lines != title.lines {
            return None;
        }
        let signals = vec![
            signal(
                "title_block.largest_heading",
                "the largest heading in the head of page 1".into(),
                TITLE_HEADING,
            ),
            signal(
                "title_block.top_third",
                format!(
                    "its top {:.2} of the page height from the top",
                    1.0 - title.top
                ),
                TITLE_TOP,
            ),
        ];
        Typed::of(Kind::TitleBlock, signals)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line::{LineRun, TextLine};

    /// A line of the walk is borrowed from the page: the line type takes borrowed runs.
    #[test]
    fn lines_of_borrowed_runs() {
        let r = TextRun {
            text: "Muster".into(),
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 9.0,
            size: 9.0,
            font: 0,
            font_kind: crate::input::FontKind::Unknown,
            bold: false,
            italic: false,
            color: None,
            shift: crate::input::Shift::Base,
            line: 3,
            off_axis: false,
            gaps: Vec::new(),
        };
        let refs = [&r];
        let lines: Vec<TextLine<'_, &TextRun>> = text_lines(&refs);
        assert_eq!(lines[0].text, "Muster");
        assert_eq!(lines[0].runs[0].line(), 3);
    }
}
