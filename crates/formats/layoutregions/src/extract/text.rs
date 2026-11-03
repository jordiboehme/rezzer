// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The blocks of text lines: headings by size, list items by marker and paragraphs by gaps,
//! with the line breaks a paragraph keeps.
//!
//! Until regions are typed, deciding what a line is and reading the block it opens are one
//! walk whose steps depend on each other: a marker line ends a paragraph, a heading ends an
//! item, a list item joins the one before it, and line breaks look at the lines around a
//! block. [`next_block`] is one step of that walk; the reader keeps the loop (and its stop
//! check) and builds the blocks' text from its own runs. Tasks 16 and 19 split this walk into
//! the typers' decisions and one extractor per kind.
//!
//! Every function works on the lines of the reader's own runs (see [`LineRun`]), borrowed,
//! and returns line indexes into the slice it is given.

use crate::file::{HEADING_PLAIN_RATIO, data_only};
use crate::line::{EDGE, GAP_RATIO, LABEL_STEP, LineRun, PIECE_GAP, SIZE_SLACK, TextLine, ink};

/// What the walk over a page's lines needs besides the lines.
pub struct TextCx<'a, R> {
    /// The body size of the file.
    pub body_size: f32,
    /// The heading level of a line, `None` for body text.
    pub heading_level: &'a dyn Fn(&TextLine<'_, R>) -> Option<u8>,
    /// Whether a text ends in an amount (`Gesamtbetrag 1.234,56 EUR`).
    pub ends_in_amount: &'a dyn Fn(&str) -> bool,
    /// The block of its own each line stands in, if the reader read one (a cell of a row of
    /// fields read cell by cell): a line goes on the line before it only in the same block.
    pub block: &'a [Option<usize>],
}

/// The block that a line opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextBlock {
    /// A line with no text: no block.
    Empty,
    /// Lines of a heading of `level`. A `demoted` heading reads as a paragraph (an address,
    /// sentences, lines of their own) and keeps a line break before each line `breaks` names
    /// (page, line; sorted).
    Heading {
        level: u8,
        demoted: bool,
        breaks: Vec<(u32, u32)>,
    },
    /// A list item: its marker takes the first `cut` bytes of the trimmed text of its first
    /// line. It `joins` the list of the item before it.
    Item {
        ordered: bool,
        cut: usize,
        joins: bool,
    },
    /// A paragraph with a line break before each line `breaks` names (page, line; sorted).
    Paragraph { breaks: Vec<(u32, u32)> },
}

/// The block that line `i` of `lines` opens, and the line after it. `open_list` is whether
/// the block before it was a list item that is still open (a line without text leaves it
/// open; a heading or a paragraph closes it).
pub fn next_block<R: LineRun>(
    lines: &[TextLine<'_, R>],
    i: usize,
    open_list: bool,
    cx: &TextCx<'_, R>,
) -> (TextBlock, usize) {
    let l = &lines[i];
    if l.text.is_empty() {
        return (TextBlock::Empty, i + 1);
    }
    let heading_level = cx.heading_level;
    // A line goes on the line before it only in the same block of its own.
    let goes_on =
        |p: usize, c: usize| continues(&lines[p], &lines[c]) && cx.block[p] == cx.block[c];
    // A number of three digits or more that opens a line in the middle of a sentence is part
    // of the sentence ("31 CFR 209 and/or / 210. The information"), not a list item.
    let marker = |k: usize| {
        if padded_mark(&lines[k]) {
            return None;
        }
        let found = list_marker(&lines[k].text)?;
        let digits = lines[k]
            .text
            .chars()
            .take_while(char::is_ascii_digit)
            .count();
        let mid_sentence =
            k > 0 && goes_on(k - 1, k) && !lines[k - 1].text.ends_with(['.', ':', ';', '!', '?']);
        (!(found.0 && digits >= 3 && mid_sentence)).then_some(found)
    };
    if let Some(level) = heading_level(l) {
        let (j, heading) = heading_block(
            lines,
            i,
            level,
            heading_level,
            cx.body_size,
            cx.ends_in_amount,
        );
        let j = (i + 1..j)
            .find(|&k| cx.block[k] != cx.block[i])
            .unwrap_or(j);
        let breaks = if heading {
            Vec::new()
        } else {
            line_breaks(lines, i, j, cx.ends_in_amount)
        };
        return (
            TextBlock::Heading {
                level,
                demoted: !heading,
                breaks,
            },
            j,
        );
    }
    if let Some((ordered, cut)) = marker(i) {
        let mut j = i + 1;
        while j < lines.len()
            && heading_level(&lines[j]).is_none()
            && marker(j).is_none()
            && goes_on(j - 1, j)
            && lines[j].x > l.x + SIZE_SLACK
        {
            j += 1;
        }
        // An item follows the item before it when the gap is not over twice the usual.
        let joins = i > 0 && open_list && list_gap_ok(&lines[i - 1], l);
        return (
            TextBlock::Item {
                ordered,
                cut,
                joins,
            },
            j,
        );
    }
    let mut j = i + 1;
    while j < lines.len()
        && heading_level(&lines[j]).is_none()
        && marker(j).is_none()
        && goes_on(j - 1, j)
        && !bold_title_after(&lines[i..=j])
    {
        j += 1;
    }
    let breaks = line_breaks(lines, i, j, cx.ends_in_amount);
    (TextBlock::Paragraph { breaks }, j)
}

/// Whether `cur` goes on in the same paragraph as `prev`: below it, near enough and in the
/// same size, and not wholly right of it: a line that starts [`BESIDE_GAP`] sizes or more
/// after the ink of the line over it ends, and is not much wider than it, stands in another
/// column (an amount alone under a table in another column than the one over it), as text that
/// wraps goes on at the left.
pub fn continues<R: LineRun>(prev: &TextLine<'_, R>, cur: &TextLine<'_, R>) -> bool {
    let gap = prev.y - cur.y;
    let height = prev.h.max(cur.h).max(1.0);
    let ink = |l: &TextLine<'_, R>| {
        l.runs
            .iter()
            .filter(|r| !r.text().trim().is_empty())
            .fold((f32::MAX, f32::MIN), |(a, b), r| {
                (a.min(r.x()), b.max(r.x() + r.w().max(0.0)))
            })
    };
    let ((p0, end), (c0, c1)) = (ink(prev), ink(cur));
    // A line that is much wider than a short line over it is text going on after a label or
    // a marker (an indented continuation), not a cell of another column.
    // A word cut at the end of a line goes on at the left of the next, never right of where
    // the cut ends (`Page-` over a cell of the next row).
    let narrow = c1 - c0 <= BESIDE_WIDTH * (end - p0).max(prev.size) || cuts(&prev.text, &cur.text);
    let beside = narrow && c0 >= end + BESIDE_GAP * prev.size.max(cur.size).max(1.0);
    gap > 0.0 && gap <= GAP_RATIO * height && (prev.size - cur.size).abs() <= SIZE_SLACK && !beside
}

/// A line that starts this many sizes or more right of where the line over it ends goes on
/// no paragraph of it (see [`continues`]): the white space of a cell gap, far over a space.
const BESIDE_GAP: f32 = 1.0;

/// A line beside the line over it (see [`BESIDE_GAP`]) is a cell of another column only when
/// it is at most this many times as wide as that line.
const BESIDE_WIDTH: f32 = 2.0;

/// A bold line under plain lines stands this many times farther from them than their pitch
/// when it is a title of its own.
const TITLE_STEP: f32 = 1.1;

/// Line heights one line of text steps down by, for a block of one line that has no pitch of
/// its own.
const LEADING: f32 = 1.2;

/// Whether the last of `lines` is a title under the block of the others: all its ink is bold,
/// none of theirs is, and it stands farther under them than [`TITLE_STEP`] times their pitch
/// (or than the leading of one line, see [`LEADING`], under a block of one line). A bold
/// title of the size of the sender lines under them (`Rechnung` under the sender of a letter)
/// is no line of the sender's block; a bold line at the block's own pitch is.
fn bold_title_after<R: LineRun>(lines: &[TextLine<'_, R>]) -> bool {
    let [above @ .., prev, cur] = lines else {
        return false;
    };
    let inked = |l: &TextLine<'_, R>| {
        l.runs
            .iter()
            .filter(|r| !r.text().trim().is_empty())
            .map(LineRun::bold)
            .collect::<Vec<bool>>()
    };
    let (bold, plain) = (inked(cur), inked(prev));
    if bold.is_empty() || !bold.iter().all(|b| *b) || plain.iter().any(|b| *b) {
        return false;
    }
    if above.iter().any(|l| inked(l).iter().any(|b| *b)) {
        return false;
    }
    let pitch = match above.last() {
        Some(a) => a.y - prev.y,
        None => LEADING * prev.size,
    };
    pitch > 0.0 && prev.y - cur.y > TITLE_STEP * pitch
}

/// The marker of a list line: `(ordered, bytes of marker and the space after it)`.
pub fn list_marker(text: &str) -> Option<(bool, usize)> {
    let first = text.chars().next()?;
    if first == '\u{2022}' {
        let rest = &text[first.len_utf8()..];
        return Some((false, text.len() - rest.trim_start().len()));
    }
    if matches!(first, '-' | '*' | '\u{2013}') {
        let rest = &text[first.len_utf8()..];
        // Two padding characters or more after it: a mark set into a field of a form (a
        // starred amount, a dash for none), no bullet.
        let pad = rest.chars().take_while(|c| c.is_whitespace()).count();
        // A star that a lone star later on the line closes encloses an item (`* A123456B * x/y`,
        // a code set between stars): no bullet.
        let closed = first == '*' && rest.split_whitespace().any(|w| w == "*");
        if pad == 1 && !closed {
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

/// Whether a line opens with a `*`, `-` or U+2013 run whose padding a piece gap parts from
/// the text after it: padding written as one space (a reader writes padding across a piece
/// gap as one space after the run before it), no bullet.
fn padded_mark<R: LineRun>(line: &TextLine<'_, R>) -> bool {
    let mut inked = line.runs.iter().filter(|r| ink(r.text()) > 0);
    let (Some(mark), Some(next)) = (inked.next(), inked.next()) else {
        return false;
    };
    // Padding across a piece gap is one space after the run before it; a bullet set at a tab
    // stop has its space before the text instead.
    matches!(mark.text().trim(), "*" | "-" | "\u{2013}")
        && mark.text().ends_with(char::is_whitespace)
        && next.x() - (mark.x() + mark.w()) > PIECE_GAP * mark.size().max(next.size())
}

/// The heading that starts at line `i` of `all`, a line of `level`: the end of its lines
/// (each after the first of `level` and going on the one before), and whether they are a
/// heading. Bold lines a little larger than the body (`body_size`) that read as an address
/// or as sentences are a paragraph: the address of an invoice, its closing words.
pub fn heading_block<R: LineRun>(
    all: &[TextLine<'_, R>],
    i: usize,
    level: u8,
    heading_level: &dyn Fn(&TextLine<'_, R>) -> Option<u8>,
    body_size: f32,
    ends_in_amount: &dyn Fn(&str) -> bool,
) -> (usize, bool) {
    let mut j = i + 1;
    while j < all.len() && heading_level(&all[j]) == Some(level) && continues(&all[j - 1], &all[j])
    {
        j += 1;
    }
    let first = &all[i];
    let small = first.size < HEADING_PLAIN_RATIO * body_size;
    let block = &all[i..j];
    // Lines that keep a break between them (items, fields) are no heading but a paragraph with
    // its breaks, when they are a little larger than the body or three lines or more (a column
    // of items, also where a box cuts it). Two large short lines are a title over two lines.
    // The lines of the same column over the block (a box may have cut it off them), two at
    // most: enough to make it three lines.
    let mut over = i;
    while over > 0
        && i - over < 2
        && heading_level(&all[over - 1]) == Some(level)
        && continues(&all[over - 1], &all[over])
    {
        over -= 1;
    }
    let column = &all[over..j];
    let items = block_listed(column);
    // Lines that each stand in pieces apart by a cell gap (cells of columns) are no title; lines
    // set full (justified: words equal gaps apart, both outer edges shared) are no cells.
    let full = {
        let edges = ink_edges(block);
        let tol = 0.5
            * block
                .iter()
                .map(|l| l.size)
                .fold(f32::MAX, f32::min)
                .max(1.0);
        let even = |l: &TextLine<'_, R>| {
            let mut end: Option<f32> = None;
            let gaps: Vec<f32> = l
                .runs
                .iter()
                .filter(|r| !r.text().trim().is_empty())
                .filter_map(|r| {
                    let gap = end.map(|e| r.x() - e);
                    end = Some(r.x() + r.w());
                    gap
                })
                .collect();
            let (lo, hi) = gaps
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), &g| (a.min(g), b.max(g)));
            hi - lo <= tol
        };
        block.iter().all(even)
            && edges
                .iter()
                .all(|e| (e.0 - edges[0].0).abs() <= tol && (e.1 - edges[0].1).abs() <= tol)
    };
    // A sentence that goes on over its lines (a line opening with a small letter, or a last
    // line ending with a full stop) is no row of cells either.
    let prose = block
        .iter()
        .skip(1)
        .any(|l| l.text.starts_with(char::is_lowercase))
        || block.last().is_some_and(|l| l.text.ends_with('.'));
    let pieced = |l: &TextLine<'_, R>| !full && !prose && cell_gaps(l) > 0;
    let apart = (1..block.len()).any(|k| lines_apart(block, k, items))
        || wraps(column, &ink_edges(column), ends_in_amount).contains(&false)
        || (block.len() >= 2 && block.iter().all(pieced));
    if (small || column.len() >= 3 || block.iter().all(pieced)) && apart {
        return (j, false);
    }
    let texts: Vec<&str> = block.iter().map(|l| l.text.as_str()).collect();
    let one_edge = block.iter().all(|b| (b.x - first.x).abs() <= EDGE);
    let paragraph = small && ((address_shaped(&texts) && one_edge) || sentences(block));
    (j, !paragraph)
}

/// Whether the line `k` of `block` (a paragraph or a heading) starts after a line break
/// whatever the width of the block, as no wrapped text is made of it and the line over it:
/// the block's lines are short items (an address, a column of labels, of values, or of fields
/// of one face; see [`listed`]: at most a few words each and few small words) and the two
/// lines share their left edge, or both lines are a label and its value in the same two faces
/// (see [`field_line`]) and not long lines of prose.
/// `items` is whether the block is [`listed`] (see [`block_listed`]).
fn lines_apart<R: LineRun>(block: &[TextLine<'_, R>], k: usize, items: bool) -> bool {
    let (a, b) = (&block[k - 1], &block[k]);
    let left = |l: &TextLine<'_, R>| {
        l.runs
            .iter()
            .filter(|r| !r.text().trim().is_empty())
            .map(LineRun::x)
            .fold(f32::MAX, f32::min)
    };
    let items = items && (left(a) - left(b)).abs() <= EDGE;
    let words = |l: &TextLine<'_, R>| l.text.split_whitespace().count();
    let fields = field_line(a).is_some_and(|f| {
        field_line(b).is_some_and(|g| same_face(f[0], g[0]) && same_face(f[1], g[1]))
    }) && words(a).max(words(b)) <= FIELD_WORDS;
    items || fields
}

/// Whether the lines of `block` are [`listed`]: short items.
fn block_listed<R: LineRun>(block: &[TextLine<'_, R>]) -> bool {
    listed(&block.iter().map(|l| l.text.as_str()).collect::<Vec<_>>())
}

/// A line of more words than this is a line of prose, though it starts plain and ends bold (see
/// [`lines_apart`]).
const FIELD_WORDS: usize = 8;

/// A line is short when the next first word fits after it with this share of the widest
/// line to spare.
const SHORT_ROOM: f32 = 0.15;

/// The lines of the paragraph `i..j` of `all` (by page and line, sorted) that start after a
/// line break: the first word of the line would have fit at the end of the line before
/// within the widest line with [`SHORT_ROOM`] of it to spare (and that line does not end in a
/// hyphen), or the paragraph is set flush right or centred and the two lines share a right
/// edge or a centre but not a left edge, or the
/// paragraph is a block of short lines each of which after the first starts with a capital or
/// a digit, none ends in a hyphen or a comma, the lines do not read as sentences, share their
/// left edge and have the shape of an address or a list (list markers, or at most
/// [`ADDRESS_WORDS`] words a line), or the two lines are each a label and its value in the
/// same two faces (see [`field_line`]). How wide the block is does not matter: a long name in an
/// address is still a line of it. Prose that fills its measure keeps joining, also where
/// every line starts with a capital.
fn line_breaks<R: LineRun>(
    all: &[TextLine<'_, R>],
    i: usize,
    j: usize,
    ends_in_amount: &dyn Fn(&str) -> bool,
) -> Vec<(u32, u32)> {
    let lines = &all[i..j];
    if lines.len() < 2 {
        return Vec::new();
    }
    // The column the block stands in: up to two lines over and under it that go on it but
    // for the edge of a block of its own (see [`TextCx::block`]), which a box may have cut
    // off it. Where the column shows that its lines are lines of their own, so are the
    // block's.
    let (mut lo, mut hi) = (i, j);
    while lo > 0 && i - lo < 2 && continues(&all[lo - 1], &all[lo]) {
        lo -= 1;
    }
    while hi < all.len() && hi - j < 2 && continues(&all[hi - 1], &all[hi]) {
        hi += 1;
    }
    let column = &all[lo..hi];
    let around = wraps(column, &ink_edges(column), ends_in_amount);
    let edges = ink_edges(lines);
    let right = edges.iter().map(|e| e.1).fold(f32::MIN, f32::max);
    let left = edges.iter().map(|e| e.0).fold(f32::MAX, f32::min);
    let short = lines[1..].iter().all(|l| {
        l.text
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
    }) && lines
        .iter()
        .all(|l| !word_cut(&l.text) && !l.text.ends_with(','))
        && lines.windows(2).all(|w| !cuts(&w[0].text, &w[1].text))
        && !sentences(lines)
        && edges.iter().all(|e| (e.0 - left).abs() <= EDGE)
        && listed(&lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>());
    // Lines set flush right or centred: most pairs of lines in a row, and at least two,
    // share a right edge or a centre but not a left edge. One indented first line of
    // justified prose is not that.
    let same_left = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() <= EDGE;
    let aligned_pair = |a: (f32, f32), b: (f32, f32)| {
        ((a.1 - b.1).abs() <= EDGE || ((a.0 + a.1) / 2.0 - (b.0 + b.1) / 2.0).abs() <= EDGE)
            && !same_left(a, b)
    };
    let pairs = edges
        .windows(2)
        .filter(|w| aligned_pair(w[0], w[1]))
        .count();
    // Two lines centred on one another (their right edges apart) are a centred block too: an
    // indented first line of prose shares the right edge, not the centre.
    let centred_pair = edges.len() == 2 && {
        let (a, b) = (edges[0], edges[1]);
        ((a.0 + a.1) / 2.0 - (b.0 + b.1) / 2.0).abs() <= EDGE && (a.1 - b.1).abs() > EDGE
    };
    let set_aligned = (pairs >= 2 && 2 * pairs >= edges.len() - 1) || centred_pair;
    let items = block_listed(lines);
    let wraps = wraps(lines, &edges, ends_in_amount);
    let mut out = Vec::new();
    for k in 0..lines.len() - 1 {
        let (a, b) = (edges[k], edges[k + 1]);
        let next = &lines[k + 1];
        let word = first_word(next);
        // A short line: it ends well before the widest line, so far that the next line's
        // first word and a good share of the measure more would still fit. Ragged prose
        // misses a word by a little. A word cut at the line end goes on in the next line.
        let room = SHORT_ROOM * (right - left);
        let fits =
            a.1 + 0.25 * next.size + word + room <= right && !cuts(&lines[k].text, &next.text);
        let aligned = set_aligned && aligned_pair(a, b);
        let apart = !wraps[k] || !around[i - lo + k];
        if fits || aligned || short || apart || lines_apart(lines, k + 1, items) {
            out.push((next.runs[0].page(), next.runs[0].line()));
        }
    }
    out.sort_unstable();
    out
}

/// The ends of each line's ink.
fn ink_edges<R: LineRun>(lines: &[TextLine<'_, R>]) -> Vec<(f32, f32)> {
    lines
        .iter()
        .map(|l| {
            let inked = l.runs.iter().filter(|r| !r.text().trim().is_empty());
            let x0 = inked.clone().map(LineRun::x).fold(f32::MAX, f32::min);
            let x1 = inked.map(|r| r.x() + r.w()).fold(f32::MIN, f32::max);
            (x0, x1)
        })
        .collect()
}

/// The gaps of a cell between the runs with ink of `line` (see [`PIECE_GAP`]): the white
/// space between cells of columns, far over a word space.
fn cell_gaps<R: LineRun>(line: &TextLine<'_, R>) -> usize {
    let mut end: Option<f32> = None;
    line.runs
        .iter()
        .filter(|r| !r.text().trim().is_empty())
        .filter(|r| {
            let gap = end.is_some_and(|e| r.x() - e > PIECE_GAP * r.size());
            end = Some(r.x() + r.w());
            gap
        })
        .count()
}

/// Whether the cell gaps of `line` (see [`cell_gaps`]) are all of one width, within half a
/// size: the word spaces of a line set full (justified), no gutters of cells.
fn set_full<R: LineRun>(line: &TextLine<'_, R>) -> bool {
    let mut end: Option<f32> = None;
    let gaps: Vec<f32> = line
        .runs
        .iter()
        .filter(|r| !r.text().trim().is_empty())
        .filter_map(|r| {
            let gap = end.map(|e| r.x() - e).filter(|g| *g > PIECE_GAP * r.size());
            end = Some(r.x() + r.w());
            gap
        })
        .collect();
    let (lo, hi) = gaps
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &g| (a.min(g), b.max(g)));
    !gaps.is_empty() && hi - lo <= 0.5 * line.size.max(1.0)
}

/// Whether the last run with ink of `line` stands after a cell gap (see [`cell_gaps`]): it
/// is a cell of its own.
fn last_cell<R: LineRun>(line: &TextLine<'_, R>) -> bool {
    let mut inked = line
        .runs
        .iter()
        .rev()
        .filter(|r| !r.text().trim().is_empty());
    match (inked.next(), inked.next()) {
        (Some(last), Some(before)) => {
            last.x() - (before.x() + before.w()) > PIECE_GAP * last.size()
        }
        _ => false,
    }
}

/// The width of the first word of `line`, by its share of the first run with text.
fn first_word<R: LineRun>(line: &TextLine<'_, R>) -> f32 {
    line.runs
        .iter()
        .find(|r| !r.text().trim().is_empty())
        .map_or(f32::MAX, |r| {
            let text = r.text().trim_start();
            let chars = text.chars().count().max(1) as f32;
            let first = text.split_whitespace().next().unwrap_or("");
            r.w() * first.chars().count() as f32 / chars
        })
}

/// For each line of `lines` but the last (with the ends `edges` of their ink), whether it may
/// go on in the next line as wrapped text: the positive evidence that a line was broken where
/// it is only because its measure was full (Task 15b part 2e-2, ruling P6-X: a line of its own
/// read as wrapped text mixes two fields). A line that ends in a hyphen or a comma, or whose
/// next line starts with a small letter or holds punctuation alone, goes on. Otherwise no line goes on when
/// - the lines each end with an exclamation or a question mark: sentences of their own (a
///   slogan, a greeting), or each opens with a star: starred items;
/// - the lines read as no sentences and one that ends no sentence, or the first line, would
///   have taken the next line's first word in its own face within the widest line, with
///   [`SPARE`] sizes to spare:
///   the lines were not filled to a measure, they are lines of their own (fields, items,
///   address lines) of any length;
/// - the lines read as no sentences and one ends in a number (an amount, a date, a customer
///   number) over a line that opens with a capital, or in an amount (`ends_in_amount`) over a
///   line of an amount alone: a column of values.
///
/// In lines that read as sentences, only a row of cells that ends in a number over a line
/// without cells ends there. Prose goes on, whatever its last word (a number over a noun).
fn wraps<R: LineRun>(
    lines: &[TextLine<'_, R>],
    edges: &[(f32, f32)],
    ends_in_amount: &dyn Fn(&str) -> bool,
) -> Vec<bool> {
    let n = lines.len();
    if n < 2 {
        return Vec::new();
    }
    let ends_sentence = |l: &TextLine<'_, R>| l.text.ends_with(['.', '!', '?', ':']);
    let cut = |l: &TextLine<'_, R>| word_cut(&l.text) || l.text.ends_with(',');
    let lower = |l: &TextLine<'_, R>| l.text.chars().next().is_some_and(char::is_lowercase);
    // A line of punctuation alone (a dash that ends a verse, set on a line of its own) is no
    // line of its own: it ends the line before.
    let marks = |l: &TextLine<'_, R>| !l.text.chars().any(char::is_alphanumeric);
    let goes_on: Vec<bool> = (0..n - 1)
        .map(|k| {
            cut(&lines[k])
                || cuts(&lines[k].text, &lines[k + 1].text)
                || lower(&lines[k + 1])
                || marks(&lines[k + 1])
        })
        .collect();
    let exclaims = lines.iter().all(|l| l.text.ends_with(['!', '?']));
    let starred = lines.iter().all(|l| l.text.starts_with('*'));
    let right = edges.iter().map(|e| e.1).fold(f32::MIN, f32::max);
    // A title line in bold over plain text is no line of that text.
    let one_face = |a: &TextLine<'_, R>, b: &TextLine<'_, R>| {
        let last = a.runs.iter().rev().find(|r| !r.text().trim().is_empty());
        let first = b.runs.iter().find(|r| !r.text().trim().is_empty());
        match (last, first) {
            (Some(x), Some(y)) => same_face(x, y),
            (x, y) => x.is_none() && y.is_none(),
        }
    };
    let unfilled = !sentences(lines)
        && (0..n - 1).any(|k| {
            let l = &lines[k];
            // A short line that ends a sentence ends a paragraph in prose, after a line that
            // wrapped into it; as the first line of the block it is a line of its own.
            (!ends_sentence(l) || k == 0)
                && !goes_on[k]
                && one_face(l, &lines[k + 1])
                && edges[k].1 + (0.25 + SPARE) * lines[k + 1].size + first_word(&lines[k + 1])
                    <= right
        });
    // A line that ends in a number (an amount, a date, a customer number), and no sentence,
    // over a line that opens with a capital ends a value: the next line is a field of its own,
    // and so is every line of a block that holds such a line and no sentence. So does a line
    // that ends in an amount over a line of an amount alone (a total under it).
    let value_end = |k: usize| {
        let l = &lines[k];
        let next = &lines[k + 1].text;
        let total = || {
            data_only(next)
                && ends_in_amount(next)
                && (ends_in_amount(&l.text) || signed_amount(&l.text))
        };
        !ends_sentence(l)
            && !goes_on[k]
            && (next.chars().next().is_some_and(char::is_uppercase) || total())
            && l.text.split_whitespace().next_back().is_some_and(|w| {
                w.chars().any(|c| c.is_ascii_digit())
                    && w.chars().all(|c| c.is_ascii_digit() || ".,/-".contains(c))
            })
    };
    let values = !sentences(lines) && (0..n - 1).any(value_end);
    if exclaims || starred || unfilled || values {
        goes_on
    } else {
        // In prose a line that ends in a number goes on (`mit 2` over `Prozent Skonto`), but a
        // line that ends in a row of cells (two cell gaps or more of other widths than the word
        // spaces of a line set full, the number a cell of its own after one) ends with that row
        // (`79-84 70-85` over `Preparation work`).
        (0..n - 1)
            .map(|k| {
                !(value_end(k)
                    && cell_gaps(&lines[k]) >= 2
                    && last_cell(&lines[k])
                    && !set_full(&lines[k]))
            })
            .collect()
    }
}

/// Whether the line `text` ends in a hyphen that goes on in the line `next`: a word cut (see
/// [`word_cut`]), or the hyphen of a range after an amount over a line that opens with a number
/// and holds words (`08.00-` over `18.00 Uhr`). Over a line of numbers alone the minus is the
/// sign of a credit (a total under it).
fn cuts(text: &str, next: &str) -> bool {
    word_cut(text)
        || (text.ends_with('-')
            && next.starts_with(|c: char| c.is_ascii_digit())
            && !data_only(next))
}

/// Whether `text` ends in a hyphen that cuts a word: not the minus after an amount with
/// cents (`10,00-`, `1.234,56-`, the way a deduction or a credit is written), which ends the
/// amount, not a word.
fn word_cut(text: &str) -> bool {
    text.ends_with('-') && !signed_amount(text)
}

/// Whether the last word of `text` is an amount with a decimal part of two digits and a minus
/// after it (`10,00-`, `1.234,56-`).
pub fn signed_amount(text: &str) -> bool {
    let Some(word) = text.split_whitespace().next_back() else {
        return false;
    };
    let Some(number) = word.strip_suffix('-') else {
        return false;
    };
    // Digits and marks only (all ASCII, so the byte offsets below are char bounds), a digit
    // first, and a mark before two digits last.
    let b = number.as_bytes();
    b.len() >= 4
        && b.iter()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b','))
        && b[0].is_ascii_digit()
        && matches!(b[b.len() - 3], b'.' | b',')
        && b[b.len() - 2..].iter().all(u8::is_ascii_digit)
}

/// A line has room for the next line's first word when this many sizes of the widest line
/// are left over after it and a space (see [`wraps`]): more than the error of a word's
/// width taken from its share of a run.
const SPARE: f32 = 1.0;

/// The faces of a line that is a label and its value: the face its ink starts in and the other
/// face it ends in (a plain label, then a bold value), each as weight, slant, family and size.
/// Two lines one under the other that are each a label and its value in the same two faces are
/// two fields, never one line of text going on (see [`line_breaks`]).
fn field_line<'a, R: LineRun>(l: &TextLine<'a, R>) -> Option<[&'a R; 2]> {
    let mut inked = l.runs.iter().filter(|r| !r.text().trim().is_empty());
    let first = inked.next()?;
    let last = inked.next_back()?;
    (!same_face(first, last)).then_some([first, last])
}

/// Whether two runs are set in one face: weight, slant and family, and sizes less than
/// [`LABEL_STEP`] apart (relative to the size, ruling P6-Y).
fn same_face<R: LineRun>(a: &R, b: &R) -> bool {
    let (lo, hi) = (a.size().min(b.size()), a.size().max(b.size()));
    (a.bold(), a.italic(), a.font()) == (b.bold(), b.italic(), b.font())
        && hi < LABEL_STEP * lo.max(f32::MIN_POSITIVE)
}

/// Whether the lines read as sentences: one holds a sentence end with more text after it, or
/// the last ends a sentence after a word in small letters. One pass over each line.
fn sentences<R: LineRun>(lines: &[TextLine<'_, R>]) -> bool {
    // A stop after a word of three letters or more, then a space and a capital: "ul. 18" or
    // "Nr. 5" are no sentence ends.
    let inner = lines.iter().any(|l| {
        let chars: Vec<char> = l.text.chars().collect();
        let mut letters = 0usize;
        for (k, &c) in chars.iter().enumerate() {
            if c == ' ' {
                letters = 0;
            } else if matches!(c, '.' | '!' | '?') {
                if letters >= 3
                    && chars.get(k + 1) == Some(&' ')
                    && chars.get(k + 2).is_some_and(|n| n.is_uppercase())
                {
                    return true;
                }
            } else if c.is_alphabetic() {
                letters += 1;
            }
        }
        false
    });
    let last = lines.last().map_or("", |l| l.text.as_str());
    let mut tail = last.chars().rev();
    let ends =
        matches!(tail.next(), Some('.' | '!' | '?')) && tail.next().is_some_and(char::is_lowercase);
    inner || ends
}

/// Whether the list item `cur` follows the item that ends with `prev`: below it, and the gap
/// at most twice the gap that ends a paragraph (see [`GAP_RATIO`]).
fn list_gap_ok<R: LineRun>(prev: &TextLine<'_, R>, cur: &TextLine<'_, R>) -> bool {
    let gap = prev.y - cur.y;
    gap > 0.0 && gap <= 2.0 * GAP_RATIO * prev.h.max(cur.h).max(1.0)
}

/// Lines of an address or a list of labels hold at most this many words; lines of prose in a
/// narrow column hold more.
pub const ADDRESS_WORDS: usize = 4;

/// Whether lines have the shape of an address or a list: each starts with a list marker, or
/// none holds more than [`ADDRESS_WORDS`] words and words in small letters are rare (at most
/// one for every two lines, name particles aside). Prose, also German prose whose nouns
/// start with capitals, holds many words in small letters.
fn listed(lines: &[&str]) -> bool {
    if lines.iter().all(|l| list_marker(l).is_some()) {
        return true;
    }
    let total: usize = lines.iter().map(|l| small_words(l)).sum();
    lines.iter().all(|l| word_count(l) <= ADDRESS_WORDS) && 2 * total <= lines.len()
}

/// The words of a line, a run of words that each hold a digit counted as one: an IBAN or a
/// phone number written in groups is one word (`IBAN DE00 0000 0000` is two).
pub fn word_count(text: &str) -> usize {
    let mut n = 0;
    let mut in_number = false;
    for w in text.split_whitespace() {
        let number = w.chars().any(|c| c.is_ascii_digit());
        if !(number && in_number) {
            n += 1;
        }
        in_number = number;
    }
    n
}

/// Whether lines have the shape of an address: three lines or more that are [`listed`], each
/// with a letter, and a digit on a line after the first (a street number, a postcode). Their
/// left edge is the caller's to check.
pub fn address_shaped(lines: &[&str]) -> bool {
    lines.len() >= 3
        && listed(lines)
        && lines.iter().all(|l| l.chars().any(char::is_alphabetic))
        && lines[1..]
            .iter()
            .any(|l| l.chars().any(|c| c.is_ascii_digit()))
}

/// Words that start with a small letter, not counting name particles ("von der Heide",
/// "Maria de la Cruz") before a capitalised word. German articles on their own are no
/// particles: they precede nouns in prose.
fn small_words(text: &str) -> usize {
    const PARTICLES: [&str; 17] = [
        "von", "van", "de", "del", "della", "di", "da", "du", "la", "le", "ten", "ter", "zu",
        "zum", "zur", "bin", "al",
    ];
    const ARTICLES: [&str; 4] = ["der", "den", "dem", "des"];
    const LINKS: [&str; 3] = ["von", "van", "zu"];
    let words: Vec<&str> = text.split_whitespace().collect();
    let capital =
        |w: Option<&&str>| w.is_some_and(|w| w.chars().next().is_some_and(char::is_uppercase));
    let mut count = 0;
    for (k, w) in words.iter().enumerate() {
        if !w.chars().next().is_some_and(char::is_lowercase) {
            continue;
        }
        let next = words.get(k + 1);
        let exempt = if PARTICLES.contains(w) {
            capital(next)
                || (LINKS.contains(w)
                    && next.is_some_and(|n| ARTICLES.contains(n))
                    && capital(words.get(k + 2)))
        } else {
            ARTICLES.contains(w) && k > 0 && LINKS.contains(&words[k - 1]) && capital(next)
        };
        if !exempt {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests;
