// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The whitespace cut: the pieces of a page in reading order. Columns come one after the
//! other, and the pieces that cross a gutter (titles, notes across the columns) split the
//! columns into bands. A region without a gutter is cut at its widest band of white space
//! across (at every band as wide as the widest), and each part is looked at again.
//!
//! The cut is on pieces: rows of text split where a gap is wide enough to be a column gutter.
//! The reader makes the pieces; this module only orders them.

use crate::limits::Limits;
use crate::region::Bounds;
use crate::segment::drawn::Line;
use crate::work;

/// A row of text on one baseline, split where a gap is wide enough to be a column gutter.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    /// The extent of the piece; `y` grows upward. A piece of white space only has the extent
    /// of its blank string.
    pub bounds: Bounds,
    /// The row of the piece: pieces of one row share a line. Rows are numbered from the top of
    /// the page.
    pub line: u32,
    /// The baseline of the piece's first run.
    pub baseline: f32,
    /// The font size.
    pub size: f32,
    /// Every run with ink of the piece is bold (false for a piece without ink).
    pub bold: bool,
    /// A glyph with ink is in the piece: a piece of white space only has no size to align
    /// with the cell beside it, and neither fills nor crosses a band.
    pub inked: bool,
    /// The characters of the piece, padding not counted.
    pub chars: usize,
    /// The runs of text the piece is made of, 1 for a host that hands its own pieces: a
    /// region is looked at for columns and bands when it holds more than three.
    pub runs: usize,
    /// The text of the piece ends with a colon: a label that states it is one.
    pub colon: bool,
    /// The text of the piece reads as an amount or a date (see [`reads_as_value`]): a value,
    /// no label.
    pub value: bool,
    /// The piece stands in for a drawn box (the index of the box): its text is read whole
    /// where it stands, and `rows` counts its rows.
    pub boxed: Option<usize>,
    /// The rows of the box the piece stands in for; 1 for any other piece.
    pub rows: usize,
}

/// What made a cut between two groups of the order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// A column gutter: a band of white space with blocks on both sides, or a piece that
    /// crosses one.
    Gutter,
    /// A column gutter that only a drawn box makes.
    Boxed,
    /// The widest band of white space across a region.
    Band,
    /// A drawn rule across the gutter and most of the region.
    Rule,
}

/// Why a cut stopped before its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutShort {
    /// A region stood deeper than [`Limits::max_cut_depth`]: its pieces stay one group.
    Depth,
    /// The stop callback asked to stop: the regions not yet cut stay one group each.
    Stopped,
}

/// The pieces in reading order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cut {
    /// Groups of piece indices in reading order. Every piece is in exactly one group. A group
    /// is a region that was not cut further, or one piece that crosses a gutter.
    pub groups: Vec<Vec<usize>>,
    /// The cut between each group and the next: one less than `groups`.
    pub splits: Vec<Split>,
    /// The indices, ascending, of the groups that start the right side of the gutter of a
    /// lone row (a title beside the first line of a field list, see `lone_row`): the first piece
    /// there starts a line even when it shares the row of the piece before it. The pieces of a
    /// row beside other gutters stay one line, as they read: cells of a table in columns,
    /// labels of fields side by side.
    pub sides: Vec<usize>,
    /// Set when the cut stopped early. Nothing is dropped then: the pieces left stay in
    /// groups in their order. A caller that reports limits turns this into a note.
    pub short: Option<CutShort>,
    /// The indices, ascending, of the groups the depth bound or the stop kept whole where the
    /// cut would have looked into them further.
    pub kept: Vec<usize>,
}

/// The pieces in reading order. `rules` are the horizontal drawn rules in the frame of the
/// pieces (`at` is the height), which part the bands of a gutter. `limits.max_cut_depth` bounds
/// the recursion above the fixed depth of the column search; `stop` is asked before each
/// region is looked at. Where either ends the cut, the pieces of the region at hand stay one
/// group in their order ([`Cut::short`] says so): no piece is dropped.
pub fn xy_cut(pieces: &[Piece], rules: &[Line], limits: &Limits, stop: &dyn Fn() -> bool) -> Cut {
    let cutter = Cutter {
        p: pieces,
        rules,
        max_depth: limits.max_cut_depth as usize,
        stop,
    };
    let mut out = Out::default();
    cutter.region((0..pieces.len()).collect(), 0, &mut out);
    Cut {
        groups: out.groups,
        splits: out.splits,
        sides: out.sides,
        short: out.short,
        kept: out.kept,
    }
}

#[derive(Default)]
struct Out {
    groups: Vec<Vec<usize>>,
    splits: Vec<Split>,
    sides: Vec<usize>,
    short: Option<CutShort>,
    kept: Vec<usize>,
    /// The cut that the next group starts after, once there is a group before it.
    pending: Option<Split>,
}

impl Out {
    fn group(&mut self, ids: Vec<usize>) {
        if ids.is_empty() {
            return;
        }
        let pending = self.pending.take();
        if !self.groups.is_empty() {
            self.splits.push(pending.unwrap_or(Split::Gutter));
        }
        self.groups.push(ids);
    }

    /// A group the depth bound or the stop kept whole.
    fn kept(&mut self, ids: Vec<usize>) {
        if !ids.is_empty() {
            self.kept.push(self.groups.len());
        }
        self.group(ids);
    }

    /// The next group starts after a cut of this kind; the first cut named before a group
    /// stays.
    fn cut(&mut self, split: Split) {
        self.pending.get_or_insert(split);
    }
}

struct Cutter<'a> {
    p: &'a [Piece],
    rules: &'a [Line],
    max_depth: usize,
    stop: &'a dyn Fn() -> bool,
}

/// What makes a column gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
    /// Prose on both sides.
    Prose,
    /// Blocks of lines whose baselines do not line up.
    Offset,
    /// A lone row beside a block of lines of one pitch, apart from them by size or weight
    /// and baseline: a title beside a field list.
    Lone,
    /// Only a drawn box on one side.
    Boxed,
}

/// A column gutter is at least this many font sizes wide.
pub(crate) const GUTTER: f32 = 0.8;

/// Share of the pieces of a region that may cross a column gutter (titles, notes, footers
/// that span the columns).
const CROSSERS: f32 = 0.15;

/// The lines of a text column hold at least this many characters on median; shorter pieces
/// side by side are a table or a form, read row by row.
const PROSE_CHARS: usize = 15;

/// Above this many pieces in a region, columns are not looked for.
const COLUMN_PIECES: usize = 20_000;

/// Deepest nesting of column splits.
const COLUMN_DEPTH: usize = 8;

/// A rule this share of a region's width or wider, across its gutter, parts its bands.
const WIDE_RULE: f32 = 2.0 / 3.0;

impl Cutter<'_> {
    /// The region at the bound of the depth: its pieces, sorted, stay one group.
    fn keep(&self, mut ids: Vec<usize>, out: &mut Out) {
        let p = self.p;
        ids.sort_by(|&a, &b| {
            p[a].line
                .cmp(&p[b].line)
                .then(p[a].bounds.x0.total_cmp(&p[b].bounds.x0))
        });
        out.short.get_or_insert(CutShort::Depth);
        out.kept(ids);
    }

    /// Cuts the pieces `ids` (indices into the pieces) into groups, `depth` levels down.
    fn region(&self, mut ids: Vec<usize>, depth: usize, out: &mut Out) {
        work::add(1 + ids.len() as u64);
        let p = self.p;
        let sort = |ids: &mut Vec<usize>| {
            ids.sort_by(|&a, &b| {
                p[a].line
                    .cmp(&p[b].line)
                    .then(p[a].bounds.x0.total_cmp(&p[b].bounds.x0))
            });
        };
        // Whether columns or bands are looked for in this region at all: it holds more than
        // three runs. Counted in words or in ink characters instead, a label of a few words
        // and its value are looked at and parted (both measured in Task 31a), so the count
        // stays on runs: a region
        // of up to three runs is the one place where the cut still depends on how a line is
        // set in runs.
        let runs: usize = ids.iter().take(4).map(|&i| p[i].runs.max(1)).sum();
        let looks = runs > 3 && depth < COLUMN_DEPTH && ids.len() <= COLUMN_PIECES;
        if looks && out.short.is_none() && (self.stop)() {
            out.short = Some(CutShort::Stopped);
        }
        if looks && out.short == Some(CutShort::Stopped) {
            sort(&mut ids);
            out.kept(ids);
            return;
        }
        // At the bound of the depth a region that would be cut stays one group instead.
        let bound = depth >= self.max_depth;
        let found = if looks { gutter(p, &ids) } else { None };
        // A gutter that only a drawn box makes is taken after the bands of white space across:
        // text above or below the boxes is not part of their columns.
        if let Some((_, _, Why::Boxed, _)) = found {
            sort(&mut ids);
            let at = widest_gaps(p, &ids);
            if !at.is_empty() {
                if bound {
                    return self.keep(ids, out);
                }
                return self.bands(ids, &at, depth, out);
            }
        }
        if let Some((g0, g1, why, slack)) = found {
            if bound {
                return self.keep(ids, out);
            }
            sort(&mut ids);
            let mut band: Vec<usize> = Vec::new();
            let flush = |band: &mut Vec<usize>, out: &mut Out| {
                if band.is_empty() {
                    return;
                }
                let (left, right): (Vec<usize>, Vec<usize>) = band
                    .drain(..)
                    .partition(|&i| on_left(&p[i], (g0, g1), slack));
                out.cut(Split::Gutter);
                self.region(left, depth + 1, out);
                // A lone title and the line of the block whose row it shares are two lines.
                // The pieces of a row beside other gutters stay one line, as they read: cells
                // of a table in columns, labels of fields side by side.
                if why == Why::Lone {
                    out.sides.push(out.groups.len());
                }
                out.cut(if why == Why::Boxed {
                    Split::Boxed
                } else {
                    Split::Gutter
                });
                self.region(right, depth + 1, out);
            };
            // A drawn rule across the gutter and most of the region parts the bands too: the
            // text above it is read before the text below it (a titled block before a grid).
            let lo = ids.iter().map(|&i| p[i].bounds.x0).fold(f32::MAX, f32::min);
            let hi = ids.iter().map(|&i| p[i].bounds.x1).fold(f32::MIN, f32::max);
            let across: Vec<f32> = self
                .rules
                .iter()
                .filter(|r| r.from < g0 && r.to > g1 && r.to - r.from >= WIDE_RULE * (hi - lo))
                .map(|r| r.at)
                .collect();
            let mut low = f32::MAX;
            for id in ids {
                let ruled = across.iter().any(|&y| y < low && y > p[id].bounds.y1);
                if ruled {
                    flush(&mut band, out);
                    out.cut(Split::Rule);
                    low = f32::MAX;
                }
                if crosses(&p[id], (g0, g1), slack) {
                    flush(&mut band, out);
                    out.cut(Split::Gutter);
                    out.group(vec![id]);
                    low = f32::MAX;
                } else {
                    band.push(id);
                    low = low.min(p[id].bounds.y0);
                }
            }
            flush(&mut band, out);
            return;
        }
        sort(&mut ids);
        // Without a gutter, the region is cut at its widest bands of white space across, and
        // each part is looked at again: columns may stand above or below full-width text.
        let at = if looks {
            widest_gaps(p, &ids)
        } else {
            Vec::new()
        };
        if !at.is_empty() {
            if bound {
                return self.keep(ids, out);
            }
            return self.bands(ids, &at, depth, out);
        }
        out.group(ids);
    }

    /// Cuts the sorted pieces `ids` before each index of `at` (ascending) into bands, and
    /// looks at each band again.
    fn bands(&self, ids: Vec<usize>, at: &[usize], depth: usize, out: &mut Out) {
        let ends: Vec<usize> = at.iter().copied().chain([ids.len()]).collect();
        let mut from = 0;
        let mut k = 0;
        while k < ends.len() {
            let to = ends[k];
            if from > 0 {
                out.cut(Split::Band);
            }
            // A row of labels over the row of their values (see `labels_over_values`) is read
            // cell by cell: each label, then the value under it.
            let cells = ends.get(k + 1).and_then(|&next| {
                let before = (k > 0).then(|| &ids[if k > 1 { ends[k - 2] } else { 0 }..from]);
                let after = ends.get(k + 2).map(|&end| &ids[next..end]);
                labels_over_values(self.p, &ids[from..to], &ids[to..next], (before, after))
            });
            if let Some(cells) = cells {
                for cell in cells {
                    out.cut(Split::Gutter);
                    out.group(cell);
                }
                from = ends[k + 1];
                k += 2;
                continue;
            }
            self.region(ids[from..to].to_vec(), depth + 1, out);
            from = to;
            k += 1;
        }
    }
}

/// Whether `text` reads as an amount or a date set in figures: it holds a digit, and nothing
/// but digits, white space, signs of amounts and dates (`. , / - + : % ( )`) and currency
/// signs. A label holds letters (`1 Wages`, `Box 12a`); a date with its month in words is
/// not known here.
pub fn reads_as_value(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
        && text.chars().all(|c| {
            c.is_ascii_digit()
                || c.is_whitespace()
                || matches!(
                    c,
                    '.' | ',' | '/' | '-' | '+' | ':' | '%' | '(' | ')' | '\''
                )
                || matches!(c, '$' | '\u{20ac}' | '\u{a3}' | '\u{a5}')
        })
}

/// The cells of a row of labels over the row of their values, left to right, each a label and
/// the value under it (or a label alone): `upper` and `lower` are two bands of one row each
/// that an equal band of white space parts (as it parts every row of the region), `upper`
/// holds two pieces or more and `lower` too (one piece under one of two small ones is a line
/// under a line, a name under its sender line), every piece of `lower` stands under exactly one piece of
/// `upper` (their extents overlap along the row, and under no other), no two under the same,
/// and the pieces of `upper` are set a size step ([`SIZE_STEP`]) smaller than those of
/// `lower`: the small labels of a form's boxes over what is filled in. No piece of `upper`
/// reads as an amount or a date, no piece of `lower` ends with a colon, and the band `before`
/// is no row in the size of `upper` with a piece over each of its pieces: rows of a label
/// and its value over a larger total row are rows. The band `after` them,
/// when there is one, is small text again: under the small header of a table come more rows
/// in the size of the first. Read row by row, the values would follow the last label of
/// their row, far from their own. `None` for any other two bands.
fn labels_over_values(
    p: &[Piece],
    upper: &[usize],
    lower: &[usize],
    (before, after): (Option<&[usize]>, Option<&[usize]>),
) -> Option<Vec<Vec<usize>>> {
    // The pieces of a band are sorted by row: its first and last tell whether it is one row.
    let one_row = |band: &[usize]| matches!((band.first(), band.last()), (Some(&a), Some(&b)) if p[a].line == p[b].line);
    if !one_row(upper) || !one_row(lower) {
        return None;
    }
    work::add((upper.len() + lower.len()) as u64);
    let inked = |band: &[usize]| -> Vec<usize> {
        let mut v: Vec<usize> = band.iter().copied().filter(|&i| p[i].inked).collect();
        v.sort_by(|&a, &b| p[a].bounds.x0.total_cmp(&p[b].bounds.x0));
        v
    };
    let (labels, values) = (inked(upper), inked(lower));
    if labels.len() < 2 || values.len() < 2 || values.len() > labels.len() {
        return None;
    }
    let largest = labels.iter().map(|&i| p[i].size).fold(f32::MIN, f32::max);
    let smallest = values.iter().map(|&i| p[i].size).fold(f32::MAX, f32::min);
    if smallest < SIZE_STEP * largest {
        return None;
    }
    // The upper row is labels and the lower row is what stands under them: a piece of the
    // upper row that reads as an amount or a date is a value (a row `Subtotal 120.00` over a
    // larger `Total 127.50`), and a piece of the lower row that ends with a colon is a label
    // (a row of a label and its value over a larger total row).
    if labels.iter().any(|&i| p[i].value) || values.iter().any(|&i| p[i].colon) {
        return None;
    }
    // The band over the labels is no row like them: a row in their size with a piece over
    // each of them is the row before of one block of rows, whose last row is set larger.
    let over =
        |a: usize, b: usize| p[a].bounds.x0 < p[b].bounds.x1 && p[a].bounds.x1 > p[b].bounds.x0;
    if before.is_some_and(|b| {
        let row = inked(b);
        one_row(b)
            && row
                .iter()
                .all(|&i| p[i].size < SIZE_STEP * largest && largest < SIZE_STEP * p[i].size)
            && labels.iter().all(|&l| row.iter().any(|&i| over(i, l)))
    }) {
        return None;
    }
    // The band under the values is small text again (the labels of the next boxes), or there
    // is none: under the small header of a table come more rows in the size of the first, and
    // its header and first row are no cells of labels over values.
    if after.is_some_and(|a| {
        a.iter()
            .filter(|&&i| p[i].inked)
            .any(|&i| SIZE_STEP * p[i].size > smallest)
    }) {
        return None;
    }
    let mut under: Vec<Option<usize>> = vec![None; labels.len()];
    for &v in &values {
        work::add(labels.len() as u64);
        let mut at = labels.iter().enumerate().filter(|&(_, &l)| over(l, v));
        let (k, _) = at.next()?;
        if at.next().is_some() || under[k].is_some() {
            return None;
        }
        under[k] = Some(v);
    }
    // The pieces without ink go with the row they stand in: after its last cell.
    let blank: Vec<usize> = upper
        .iter()
        .chain(lower)
        .copied()
        .filter(|&i| !p[i].inked)
        .collect();
    let mut cells: Vec<Vec<usize>> = labels
        .iter()
        .zip(&under)
        .map(|(&l, v)| std::iter::once(l).chain(*v).collect())
        .collect();
    if let Some(last) = cells.last_mut() {
        last.extend(blank);
    }
    Some(cells)
}

/// A piece reaches into a gutter when it reaches past its edge by more than this share of
/// a step of the bands (see [`BAND_STEP`]): half a point at 8 points.
const GUTTER_SLACK: f32 = 0.5 * BAND_STEP;

/// Whether the piece `p` stands on the left side of the gutter from `g0` to `g1`: its end is
/// before the gutter, within `slack`. A piece of white space only has no ink to stand by: it
/// goes with the side its start lies on.
fn on_left(p: &Piece, (g0, g1): (f32, f32), slack: f32) -> bool {
    if p.inked {
        p.bounds.x1 <= g0 + slack
    } else {
        p.bounds.x0 < (g0 + g1) / 2.0
    }
}

/// Whether the piece `p` crosses the gutter from `g0` to `g1`: a piece with ink that reaches
/// into it from both sides by more than `slack`. White space crosses nothing: a blank string
/// that a print run draws under a row of fields is no text across the columns.
fn crosses(p: &Piece, (g0, g1): (f32, f32), slack: f32) -> bool {
    p.inked && p.bounds.x1 > g0 + slack && p.bounds.x0 < g1 - slack
}

/// Bands of white space within this share of the text size of the widest one are as wide as
/// it: paragraphs set at one spacing differ by the rounding of their baselines only, and
/// which of them is the widest by a hair says nothing about the page.
const TIE: f32 = 1.0 / 8.0;

/// More equal bands of white space than this in one region are not cut all at once: they
/// part the lines or paragraphs of one long flow, and a page of very many of them must not
/// use up the regions of the page. Such a region is cut at the topmost of them only, and the
/// rest is looked at again, so the first blocks are parted down to the depth of the column
/// search and the rest reads in its order as one group.
const MANY_BANDS: usize = 512;

/// Where to cut pieces sorted by row: after each row with the widest white space below it,
/// when that space is at least half a line high. Bands as wide as the widest (see [`TIE`],
/// of the median size of the pieces) are all cut, so that the cut does not depend on which of
/// equal bands comes first or is wider by the noise of its places. The indices, ascending,
/// are the first pieces below each cut; the topmost alone when there are more than
/// [`MANY_BANDS`].
fn widest_gaps(p: &[Piece], ids: &[usize]) -> Vec<usize> {
    // Each piece is passed once, and once more as part of the row after a cut.
    work::add(2 * ids.len() as u64);
    let mut gaps: Vec<(f32, usize)> = Vec::new();
    let mut low = f32::MAX;
    for k in 1..ids.len() {
        let (prev, cur) = (&p[ids[k - 1]], &p[ids[k]]);
        low = low.min(prev.bounds.y0);
        if cur.line == prev.line {
            continue;
        }
        // The lowest point of everything above against the top of the next row.
        let top = ids[k..]
            .iter()
            .take_while(|&&i| p[i].line == cur.line)
            .map(|&i| p[i].bounds.y1)
            .fold(f32::MIN, f32::max);
        let gap = low - top;
        if gap >= 0.5 * prev.size.max(cur.size) {
            gaps.push((gap, k));
        }
    }
    let Some(widest) = gaps.iter().map(|g| g.0).reduce(f32::max) else {
        return Vec::new();
    };
    let mut sizes: Vec<f32> = ids.iter().map(|&i| p[i].size).collect();
    sizes.sort_by(f32::total_cmp);
    let tie = TIE * sizes[sizes.len() / 2];
    let at: Vec<usize> = gaps
        .iter()
        .filter(|g| g.0 >= widest - tie)
        .map(|g| g.1)
        .collect();
    if at.len() > MANY_BANDS {
        // The rule of one band at a time: the topmost of them, and the rest is looked at
        // again down to the depth of the column search.
        return at[..1].to_vec();
    }
    at
}

/// A block beside a lone row has at least this many rows: a lone cell beside fewer is a row
/// of a table, not a title.
const LONE_BLOCK: usize = 3;

/// Each step between the baselines of a block beside a lone row is within this share of
/// their median step: the block keeps one pitch, as the lines of a field list do.
const LONE_PITCH: f32 = 0.15;

/// Two text sizes differ when the larger is at least this many times the smaller (0.5 points
/// at 6 points): a ratio, so that it holds at every scale of the page.
pub const SIZE_STEP: f32 = 1.08;

/// A column gutter in a region: the widest vertical band of white space that only a few
/// pieces cross, with blocks on both sides that stand side by side: prose, lines whose
/// baselines do not line up, a lone row beside a block (see [`lone_row`]) or a drawn box; and
/// which of them makes it. Only
/// the best band is tried (see [`best_band`]): a band after it, where the best one fails, cuts
/// label and value blocks and table columns apart (ruling P6-X3). Pieces of white space only
/// are no text: they neither fill nor cross a band.
fn gutter(p: &[Piece], ids: &[usize]) -> Option<(f32, f32, Why, f32)> {
    let inked: Vec<usize> = ids.iter().copied().filter(|&i| p[i].inked).collect();
    if inked.is_empty() {
        return None;
    }
    let allowed = (CROSSERS * inked.len() as f32).floor() as i32;
    let mut sizes: Vec<f32> = inked.iter().map(|&i| p[i].size).collect();
    sizes.sort_by(f32::total_cmp);
    let size = sizes[sizes.len() / 2];
    let (g0, g1) = best_band(p, &inked, allowed, size)?;
    split_at(p, ids, (g0, g1), allowed, size).map(|s| (g0, g1, s, GUTTER_SLACK * size))
}

/// The pieces' places are measured in steps of this share of their median text size (a point
/// at 8 points): steps of the text, so that a page printed larger or smaller has the same
/// bands.
const BAND_STEP: f32 = 1.0 / 8.0;

/// The most steps a region is measured in.
const BAND_STEPS: usize = 20_000;

/// The best band of white space between the pieces `inked` (of median text size `size`) at
/// least [`GUTTER`] sizes wide with text on both sides: of those covered by the fewest pieces
/// (a ragged line end next to the gutter must not widen it), up to `allowed` pieces, the
/// widest. Places are measured in steps of [`BAND_STEP`] sizes; a piece covers the steps it
/// starts and ends in, so pieces that touch close a band.
fn best_band(p: &[Piece], inked: &[usize], allowed: i32, size: f32) -> Option<(f32, f32)> {
    let min_gap = GUTTER * size;
    let step = BAND_STEP * size;
    let lo = inked
        .iter()
        .map(|&i| p[i].bounds.x0)
        .fold(f32::MAX, f32::min);
    let hi = inked
        .iter()
        .map(|&i| p[i].bounds.x1)
        .fold(f32::MIN, f32::max);
    if !(hi - lo).is_finite() || step.is_nan() || step <= 0.0 || hi - lo < 2.0 * step {
        return None;
    }
    // The span from the leftmost to the rightmost ink in a whole number of steps of about
    // `step`, so that a page printed larger or smaller has the same steps: a thousandth of a
    // step of float noise does not add a step.
    let width = (((hi - lo) / step - 1e-3).ceil().max(1.0) as usize).min(BAND_STEPS) + 2;
    let scale = (hi - lo) / (width - 2) as f32;
    let bin = |x: f32| (((x - lo) / scale) as usize).min(width - 1);
    work::add((inked.len() + width) as u64);
    let mut diff = vec![0i32; width + 1];
    for &i in inked {
        diff[bin(p[i].bounds.x0)] += 1;
        diff[bin(p[i].bounds.x1) + 1] -= 1;
    }
    let mut cover = Vec::with_capacity(width);
    let mut c = 0;
    for d in diff.iter().take(width) {
        c += d;
        cover.push(c);
    }
    // Only the covers some white space has make bands of their own: a level between two of
    // them has the bands of the lower one.
    let mut levels: Vec<i32> = cover.iter().copied().filter(|&c| c <= allowed).collect();
    levels.sort_unstable();
    levels.dedup();
    for level in levels {
        work::add(cover.len() as u64);
        let mut best: Option<(f32, f32)> = None;
        let mut start = None;
        for (b, &c) in cover.iter().enumerate() {
            match (c <= level, start) {
                (true, None) => start = Some(b),
                (false, Some(s)) => {
                    let (g0, g1) = (lo + s as f32 * scale, lo + b as f32 * scale);
                    if s > 0 && g1 - g0 >= min_gap && best.is_none_or(|(a, z)| g1 - g0 > z - a) {
                        best = Some((g0, g1));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        if best.is_some() {
            return best;
        }
    }
    None
}

/// What makes the band from `g0` to `g1` a gutter of the pieces `ids`, if anything: at most
/// `allowed` of them cross it, the blocks on its sides stand side by side and are prose, lines
/// whose baselines do not line up, a lone row beside a block, columns in the shape of
/// addresses or a drawn box. `size` is the median size of the pieces.
fn split_at(
    p: &[Piece],
    ids: &[usize],
    (g0, g1): (f32, f32),
    allowed: i32,
    size: f32,
) -> Option<Why> {
    // The crossers, the two sides and the tests of the sides pass the pieces a few times.
    work::add(4 * ids.len() as u64);
    let slack = GUTTER_SLACK * size;
    let crossing = ids
        .iter()
        .filter(|&&i| crosses(&p[i], (g0, g1), slack))
        .count();
    if crossing as i32 > allowed {
        return None;
    }
    let left: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|&i| !crosses(&p[i], (g0, g1), slack) && on_left(&p[i], (g0, g1), slack))
        .collect();
    let right: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|&i| !crosses(&p[i], (g0, g1), slack) && !on_left(&p[i], (g0, g1), slack))
        .collect();
    let prose = |side: &[usize]| {
        let inked: Vec<usize> = side.iter().copied().filter(|&i| p[i].inked).collect();
        if inked.len() < 2 {
            return false;
        }
        let mut chars: Vec<usize> = inked.iter().map(|&i| p[i].chars).collect();
        chars.sort_unstable();
        chars[chars.len() / 2] >= PROSE_CHARS
    };
    // A side that holds a drawn box of two lines or more is a block of its own.
    let boxed = |side: &[usize]| side.iter().any(|&i| p[i].boxed.is_some() && p[i].rows >= 2);
    // The baselines and sizes of a side, line by line: those of the pieces of each row.
    let baselines = |side: &[usize]| {
        let mut rows: Vec<(u32, f32, f32, bool)> = side
            .iter()
            .map(|&i| (p[i].line, p[i].baseline, p[i].size, p[i].inked))
            .collect();
        rows.sort_by_key(|r| r.0);
        let mut out: Vec<Vec<Part>> = Vec::new();
        let mut last = None;
        for (row, y, size, inked) in rows {
            match out.last_mut() {
                Some(l) if last == Some(row) => l.push((y, size, inked)),
                _ => out.push(vec![(y, size, inked)]),
            }
            last = Some(row);
        }
        out
    };
    let apart = || offset_baselines(&baselines(&left), &baselines(&right), size);
    let split = if prose(&left) && prose(&right) {
        Why::Prose
    } else if apart() {
        Why::Offset
    } else if lone_row(p, &left, &right) || lone_row(p, &right, &left) {
        Why::Lone
    } else if !left.is_empty() && !right.is_empty() && (boxed(&left) || boxed(&right)) {
        Why::Boxed
    } else {
        return None;
    };
    // The columns stand side by side: their heights overlap.
    let span = |side: &[usize]| {
        (
            side.iter()
                .map(|&i| p[i].bounds.y0)
                .fold(f32::MAX, f32::min),
            side.iter()
                .map(|&i| p[i].bounds.y1)
                .fold(f32::MIN, f32::max),
        )
    };
    let (a, b) = (span(&left), span(&right));
    let overlap = a.1.min(b.1) - a.0.max(b.0);
    let smaller = (a.1 - a.0).min(b.1 - b.0).max(0.01);
    (overlap / smaller >= 0.3).then_some(split)
}

/// Whether the pieces `one` are a lone row beside the block `block`, a title beside a field
/// list: the pieces of `one` with ink all lie in one row; those of `block` lie in
/// [`LONE_BLOCK`] rows or more whose baselines step at one pitch (see [`LONE_PITCH`]); the
/// row is larger than the block by its median size (see [`SIZE_STEP`]), or bold beside a
/// block that is not all bold and not smaller; and the baseline of each piece of `one` lies farther
/// than [`BASELINE_NEAR`] of the smaller size from every baseline of `block`. A lone cell on
/// the baseline of a row is a cell of that row, and a smaller bold term beside the lines of
/// its definition is a cell of a glossary.
fn lone_row(p: &[Piece], one: &[usize], block: &[usize]) -> bool {
    work::add((one.len() + block.len()) as u64);
    let inked =
        |side: &[usize]| -> Vec<usize> { side.iter().copied().filter(|&i| p[i].inked).collect() };
    let (one, mut block) = (inked(one), inked(block));
    let Some(&first) = one.first() else {
        return false;
    };
    if one.iter().any(|&i| p[i].line != p[first].line) || block.len() < LONE_BLOCK {
        return false;
    }
    // The baseline of each row of the block, top down: its highest one.
    block.sort_by_key(|&i| p[i].line);
    let mut rows: Vec<(u32, f32)> = Vec::new();
    for &i in &block {
        match rows.last_mut() {
            Some(r) if r.0 == p[i].line => r.1 = r.1.max(p[i].baseline),
            _ => rows.push((p[i].line, p[i].baseline)),
        }
    }
    if rows.len() < LONE_BLOCK {
        return false;
    }
    let mut steps: Vec<f32> = rows.windows(2).map(|w| w[0].1 - w[1].1).collect();
    steps.sort_by(f32::total_cmp);
    let pitch = steps[steps.len() / 2];
    if pitch <= 0.0 || steps.iter().any(|s| (s - pitch).abs() > LONE_PITCH * pitch) {
        return false;
    }
    let median = |side: &[usize]| {
        let mut sizes: Vec<f32> = side.iter().map(|&i| p[i].size).collect();
        sizes.sort_by(f32::total_cmp);
        sizes[sizes.len() / 2]
    };
    let (a, b) = (median(&one), median(&block));
    // The row stands out as a title does: larger, or bold beside lines that are not all bold
    // (bold labels beside plain values) and not smaller than them. A smaller bold term beside
    // the lines of its definition is a cell of a row.
    let larger = a >= SIZE_STEP * b;
    let bolder =
        one.iter().all(|&i| p[i].bold) && !block.iter().all(|&i| p[i].bold) && a * SIZE_STEP > b;
    if !larger && !bolder {
        return false;
    }
    let small = a.min(b);
    let mut ys: Vec<f32> = block.iter().map(|&i| p[i].baseline).collect();
    ys.sort_by(f32::total_cmp);
    let near = BASELINE_NEAR * small;
    one.iter().all(|&i| {
        let y = p[i].baseline;
        let k = ys.partition_point(|b| *b < y - near);
        ys.get(k).is_none_or(|b| *b > y + near)
    })
}

/// Two baselines this many font sizes apart or less are one: lines side by side that share
/// most of their baselines are rows, not two blocks.
pub const BASELINE_NEAR: f32 = 0.15;

/// Lines of one block follow each other at most this many font sizes apart, on median.
pub const BLOCK_PITCH: f32 = 4.0;

/// A baseline shift between two cells of a row that their size difference explains (top,
/// centre or baseline alignment of the two sizes) is at most this share of the difference,
/// plus [`BASELINE_NEAR`] of the smaller size.
pub const ALIGN_SHIFT: f32 = 0.8;

/// One part of a line: its baseline, its font size and whether it has ink (white space
/// only has no size to align with the cell beside it).
pub type Part = (f32, f32, bool);

/// Whether two blocks of lines stand apart by their baselines (text of font size `size`).
/// Each line is given by the baselines and sizes of its parts. Each block is at least two
/// lines that follow each other (see [`BLOCK_PITCH`]), fewer than half of the lines of
/// `right` meet a line of `left`, and at most half of the lines of `left` meet one of
/// `right`. Two lines meet when a baseline of the one lies within [`BASELINE_NEAR`] of a
/// baseline of the other, or when they pair as the cells of one row (both with ink): each
/// pairs with the
/// nearest line of the other side, one to one, by a shift their size difference explains
/// (see [`ALIGN_SHIFT`]), and all pairs share one shift, so both sides keep one pitch. A
/// column of labels next to text, each label on a line of the text, is not apart; label and
/// value cells of two sizes, top-aligned or centred, are rows.
pub fn offset_baselines(left: &[Vec<Part>], right: &[Vec<Part>], size: f32) -> bool {
    work::add((left.len() + right.len()) as u64);
    if left.len() < 2 || right.len() < 2 {
        return false;
    }
    // The first part of a line: its highest baseline with ink, with its size. White space
    // beside the text does not stand for the line; a line of white space only stands for
    // itself.
    let head =
        |l: &Vec<Part>| -> Part {
            let top =
                |inked: bool| {
                    l.iter().copied().filter(|p| p.2 || !inked).fold(
                        (f32::MIN, size, false),
                        |a, p| if p.0 > a.0 { p } else { a },
                    )
                };
            if l.iter().any(|p| p.2) {
                top(true)
            } else {
                top(false)
            }
        };
    let block = |side: &[Vec<Part>]| {
        let mut t: Vec<f32> = side.iter().map(|l| head(l).0).collect();
        t.sort_by(f32::total_cmp);
        let mut gaps: Vec<f32> = t.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_by(f32::total_cmp);
        gaps[gaps.len() / 2] <= BLOCK_PITCH * size
    };
    if !block(left) || !block(right) {
        return false;
    }
    let near = BASELINE_NEAR * size;
    let all = |side: &[Vec<Part>]| -> Vec<f32> {
        let mut v: Vec<f32> = side.iter().flatten().map(|p| p.0).collect();
        v.sort_by(f32::total_cmp);
        v
    };
    let (on_left, on_right) = (all(left), all(right));
    let meets = |line: &Vec<Part>, on: &[f32]| {
        line.iter().any(|&(y, _, _)| {
            let k = on.partition_point(|l| *l < y - near);
            on.get(k).is_some_and(|l| *l <= y + near)
        })
    };
    let mut met_left: Vec<bool> = left.iter().map(|l| meets(l, &on_right)).collect();
    let mut met_right: Vec<bool> = right.iter().map(|l| meets(l, &on_left)).collect();
    // Rows of cells of two sizes: each right line with the nearest left line, when the size
    // difference explains the shift.
    let mut heads: Vec<(Part, usize)> = left
        .iter()
        .enumerate()
        .map(|(i, l)| (head(l), i))
        .filter(|(h, _)| h.2)
        .collect();
    heads.sort_by(|a, b| a.0.0.total_cmp(&b.0.0));
    // For each left line, the right line it pairs with and the shift.
    let mut best: Vec<Option<(usize, f32)>> = vec![None; left.len()];
    for (r, line) in right.iter().enumerate() {
        let (yr, sr, inked) = head(line);
        if !inked {
            continue;
        }
        let k = heads.partition_point(|h| h.0.0 < yr);
        let Some(&((yl, sl, _), i)) = [k.checked_sub(1), Some(k)]
            .into_iter()
            .flatten()
            .filter_map(|j| heads.get(j))
            .min_by(|a, b| (a.0.0 - yr).abs().total_cmp(&(b.0.0 - yr).abs()))
        else {
            continue;
        };
        let d = yr - yl;
        let explained = d.abs() <= BASELINE_NEAR * sl.min(sr) + ALIGN_SHIFT * (sl - sr).abs();
        if explained && best[i].is_none_or(|(_, e)| d.abs() < e.abs()) {
            best[i] = Some((r, d));
        }
    }
    let mut shifts: Vec<f32> = best.iter().flatten().map(|b| b.1).collect();
    if !shifts.is_empty() {
        shifts.sort_by(f32::total_cmp);
        let shift = shifts[shifts.len() / 2];
        for (i, b) in best.iter().enumerate() {
            if let Some((r, d)) = *b {
                let small = head(&left[i]).1.min(head(&right[r]).1);
                if (d - shift).abs() <= BASELINE_NEAR * small {
                    met_left[i] = true;
                    met_right[r] = true;
                }
            }
        }
    }
    let count = |v: &[bool]| v.iter().filter(|&&m| m).count();
    2 * count(&met_right) < right.len() && 2 * count(&met_left) <= left.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A piece of ink from `x0` to `x1` in row `row`, of 6 points.
    fn ink_piece(x0: f32, x1: f32, row: usize) -> Piece {
        Piece {
            bounds: Bounds {
                x0,
                y0: 700.0 - 10.0 * row as f32,
                x1,
                y1: 706.0 - 10.0 * row as f32,
            },
            line: row as u32,
            baseline: 701.5 - 10.0 * row as f32,
            size: 6.0,
            bold: false,
            inked: true,
            chars: 4,
            runs: 1,
            colon: false,
            value: false,
            boxed: None,
            rows: 1,
        }
    }

    /// Which band wins does not depend on the scale: two gaps of one line, 10 and 11.6 points
    /// wide at 8 points, are measured in steps of the text size, so the wider one wins at
    /// every scale from 0.6 to 1.6. Measured in whole points, the two swap places at some
    /// scales, and the line is cut at the other gap.
    #[test]
    fn the_same_band_wins_at_every_scale() {
        let band = |k: f32| {
            let ps = vec![
                ink_piece(20.0 * k, 100.0 * k, 0),
                ink_piece(110.0 * k, 200.0 * k, 0),
                ink_piece(211.6 * k, 300.0 * k, 0),
            ];
            let ids: Vec<usize> = (0..ps.len()).collect();
            best_band(&ps, &ids, 0, 8.0 * k)
        };
        for n in 0..=20 {
            let k = 0.6 + 0.05 * n as f32;
            let (g0, g1) = band(k).expect("a band");
            let middle = (g0 + g1) / 2.0 / k;
            assert!((middle - 205.8).abs() < 1.0, "{k}: {g0} {g1}");
        }
    }

    /// Two pieces of one line that touch close the band before them, also when their ends
    /// differ in their last bits.
    #[test]
    fn touching_pieces_close_a_band() {
        let at = |noise: f32| {
            // A line across all, a line that ends in two touching pieces, and a line with a
            // narrow gap: the widest band of the fewest pieces runs up to where the two pieces
            // touch.
            let ps = vec![
                ink_piece(0.0, 500.0, 0),
                ink_piece(0.0, 100.0, 1),
                ink_piece(140.0, 450.0, 1),
                ink_piece(450.0 + noise, 500.0, 1),
                ink_piece(0.0, 125.0, 2),
                ink_piece(130.0, 400.0, 2),
            ];
            let ids: Vec<usize> = (0..ps.len()).collect();
            best_band(&ps, &ids, 8, 12.5)
        };
        for noise in [0.0, 3e-5] {
            let (a, b) = at(noise).expect("a band");
            assert!(
                (400.0..402.0).contains(&a) && b > 448.0 && b <= 450.0,
                "{noise}: {a} {b}"
            );
        }
    }

    #[test]
    fn baselines_that_do_not_line_up() {
        let at = |ys: &[f32], size: f32| -> Vec<Vec<Part>> {
            ys.iter().map(|&y| vec![(y, size, true)]).collect()
        };
        let lines = |ys: &[f32]| at(ys, 10.0);
        let left = at(&[650.6, 639.0, 627.4, 615.8, 604.2], 8.0);
        assert!(offset_baselines(
            &left,
            &at(&[646.3, 630.4, 614.5], 8.0),
            8.0
        ));
        // All baselines meet: one block of rows.
        let left = lines(&[700.0, 686.0, 672.0]);
        assert!(!offset_baselines(
            &left,
            &lines(&[700.0, 686.0, 672.0]),
            10.0
        ));
        // A line whose parts sit at two heights meets by either.
        let right = vec![
            vec![(604.4, 7.0, true), (606.5, 7.0, true)],
            vec![(598.7, 7.0, true)],
        ];
        assert!(!offset_baselines(&at(&[606.5, 598.7], 7.0), &right, 7.0));
        // Labels in a margin, each on a line of the text beside them: not apart.
        let text: Vec<f32> = (0..12).map(|k| 700.0 - 12.0 * k as f32).collect();
        assert!(!offset_baselines(
            &lines(&[700.0, 688.0]),
            &lines(&text),
            10.0
        ));
        // Two labels far apart are no block of lines.
        assert!(!offset_baselines(
            &lines(&[703.0, 503.0]),
            &lines(&text),
            10.0
        ));
        // One line on a side is no block.
        assert!(!offset_baselines(
            &lines(&[700.0]),
            &lines(&[703.0, 690.0]),
            10.0
        ));
        // Labels of 8 points and values of 12, top-aligned: the shift (2.9 points) is what the
        // sizes explain, the same on every row: rows.
        let labels: Vec<f32> = (0..6).map(|k| 694.3 - 20.0 * k as f32).collect();
        let values: Vec<f32> = (0..6).map(|k| 691.4 - 20.0 * k as f32).collect();
        assert!(!offset_baselines(
            &at(&labels, 8.0),
            &at(&values, 12.0),
            8.0
        ));
        // A run of white space on each value line, above the value or first: the value
        // stands for its line, and the cells are rows.
        let padded: Vec<Vec<Part>> = values
            .iter()
            .map(|&y| vec![(y + 0.5, 12.0, false), (y, 12.0, true)])
            .collect();
        assert!(!offset_baselines(&at(&labels, 8.0), &padded, 8.0));
        // The same shift between lines of one size is not explained: two blocks.
        assert!(offset_baselines(
            &at(&labels, 10.0),
            &at(&values, 10.0),
            10.0
        ));
    }

    /// A piece of text on the baseline `y`, with the extent of its size.
    fn text(x0: f32, x1: f32, line: u32, y: f32, size: f32, chars: usize) -> Piece {
        Piece {
            bounds: Bounds {
                x0,
                y0: y - 0.25 * size,
                x1,
                y1: y + 0.75 * size,
            },
            line,
            baseline: y,
            size,
            bold: false,
            inked: true,
            chars,
            runs: 1,
            colon: false,
            value: false,
            boxed: None,
            rows: 1,
        }
    }

    fn cut_of(pieces: &[Piece], rules: &[Line]) -> Cut {
        xy_cut(pieces, rules, &Limits::default(), &|| false)
    }

    /// Every piece is in exactly one group.
    fn assert_whole(cut: &Cut, n: usize) {
        let mut all: Vec<usize> = cut.groups.concat();
        all.sort_unstable();
        assert_eq!(all, (0..n).collect::<Vec<_>>());
        assert_eq!(cut.splits.len(), cut.groups.len().saturating_sub(1));
    }

    /// Two columns of prose with a title and a footer across both.
    fn two_columns() -> Vec<Piece> {
        let mut ps = vec![text(100.0, 420.0, 0, 800.0, 10.0, 40)];
        for k in 0..6 {
            let y = 700.0 - 12.0 * k as f32;
            ps.push(text(50.0, 160.0, k + 1, y, 10.0, 24));
            ps.push(text(320.0, 430.0, k + 1, y, 10.0, 25));
        }
        ps.push(text(100.0, 420.0, 7, 40.0, 10.0, 45));
        ps
    }

    #[test]
    fn two_columns_with_a_title_and_a_footer() {
        let ps = two_columns();
        let cut = cut_of(&ps, &[]);
        let left: Vec<usize> = (0..6).map(|k| 1 + 2 * k).collect();
        let right: Vec<usize> = (0..6).map(|k| 2 + 2 * k).collect();
        assert_eq!(cut.groups, vec![vec![0], left, right, vec![13]]);
        assert_eq!(cut.splits, vec![Split::Gutter; 3]);
        assert!(cut.sides.is_empty());
        assert_eq!(cut.short, None);
    }

    #[test]
    fn a_drawn_rule_across_the_gutter_parts_the_bands() {
        let mut ps = Vec::new();
        for k in 0..6u32 {
            let y = 700.0 - 12.0 * k as f32;
            ps.push(text(50.0, 160.0, k, y, 10.0, 24));
            ps.push(text(320.0, 430.0, k, y, 10.0, 25));
        }
        let plain = cut_of(&ps, &[]);
        assert_eq!(plain.groups.len(), 2);
        assert_eq!(plain.splits, vec![Split::Gutter]);
        // A rule between the third and the fourth row, across the gutter and most of the width.
        let rule = Line {
            at: 672.5,
            from: 40.0,
            to: 440.0,
        };
        let cut = cut_of(&ps, &[rule]);
        let col = |side: usize, rows: std::ops::Range<usize>| -> Vec<usize> {
            rows.map(|k| 2 * k + side).collect()
        };
        assert_eq!(
            cut.groups,
            vec![col(0, 0..3), col(1, 0..3), col(0, 3..6), col(1, 3..6)]
        );
        assert_eq!(cut.splits, vec![Split::Gutter, Split::Rule, Split::Gutter]);
        // A short rule parts nothing.
        let short = Line {
            at: rule.at,
            from: 40.0,
            to: 200.0,
        };
        assert_eq!(cut_of(&ps, &[short]), plain);
    }

    /// Lines of one column in blocks, `gaps` points of white space between the blocks.
    fn blocks(gaps: &[f32]) -> Vec<Piece> {
        let mut ps = Vec::new();
        let mut y = 700.0;
        let mut line = 0;
        for block in 0..=gaps.len() {
            for _ in 0..4 {
                ps.push(text(50.0, 400.0, line, y, 10.0, 60));
                y -= 12.0;
                line += 1;
            }
            if block < gaps.len() {
                y -= gaps[block];
            }
        }
        ps
    }

    #[test]
    fn a_region_without_a_gutter_is_cut_at_its_widest_band() {
        let ps = blocks(&[30.0, 50.0, 30.0]);
        let cut = cut_of(&ps, &[]);
        let block = |k: usize| (4 * k..4 * k + 4).collect::<Vec<_>>();
        assert_eq!(cut.groups, vec![block(0), block(1), block(2), block(3)]);
        assert_eq!(cut.splits, vec![Split::Band; 3]);
    }

    /// Blocks at one spacing are all parted, whichever of the equal bands is the widest by
    /// the noise of its place: a block moved by a few hundredths of a point makes the same
    /// groups.
    #[test]
    fn equal_bands_are_all_cut_whatever_their_noise() {
        let even = cut_of(&blocks(&[30.0, 30.0, 30.0, 30.0]), &[]);
        let block = |k: usize| (4 * k..4 * k + 4).collect::<Vec<_>>();
        assert_eq!(even.groups, (0..5).map(block).collect::<Vec<_>>());
        assert_eq!(even.splits, vec![Split::Band; 4]);
        for noise in [
            [0.04, -0.03, 0.02, -0.05],
            [-0.05, 0.05, -0.02, 0.01],
            [0.0, 0.0, 0.0, 0.06],
        ] {
            let gaps: Vec<f32> = noise.iter().map(|n| 30.0 + n).collect();
            assert_eq!(cut_of(&blocks(&gaps), &[]), even, "{noise:?}");
        }
        // A band wider than the others by more than the noise is cut first, and the parts
        // are cut again: the groups are the same.
        assert_eq!(
            cut_of(&blocks(&[30.0, 30.0, 45.0, 30.0]), &[]).groups,
            even.groups
        );
    }

    /// More equal bands than [`MANY_BANDS`] are cut one at a time from the top, down to the
    /// depth of the column search, and the rest is one group in order: no page of 20,000
    /// regions. The edge is not gone, it lies a few bands higher: up to `MANY_BANDS` plus the
    /// depth of the column search less one, the bands left after each cut from the top are at
    /// most `MANY_BANDS` again and every band is cut; from one more on, the top eight are.
    #[test]
    fn very_many_equal_bands_are_cut_from_the_top() {
        let ps = blocks(&vec![30.0; MANY_BANDS]);
        assert_eq!(cut_of(&ps, &[]).groups.len(), MANY_BANDS + 1);
        let ps = blocks(&vec![30.0; MANY_BANDS + 1]);
        assert_eq!(cut_of(&ps, &[]).groups.len(), MANY_BANDS + 2);
        let ps = blocks(&vec![30.0; MANY_BANDS + 100]);
        let cut = cut_of(&ps, &[]);
        assert_eq!(cut.groups.len(), COLUMN_DEPTH + 1);
        assert_eq!(cut.groups[0], vec![0, 1, 2, 3]);
        assert_eq!(cut.groups.concat(), (0..ps.len()).collect::<Vec<_>>());
        assert_eq!(cut.short, None);
    }

    /// Fix round 1 of Task 31a (I4): boxes of a form, a small label over what is filled in,
    /// all rows at one pitch. The equal bands part every row; a row of labels and the row of
    /// their values are read cell by cell, each label with the value under it, not the labels
    /// of the row and then its values. With labels in the size of the values the rows are
    /// rows.
    #[test]
    fn small_labels_over_their_values_read_cell_by_cell() {
        // A line across, a row of three labels, the row of their values, a line across: all
        // 20 points apart. Pieces 1, 3, 5 are the labels, 2, 4, 6 the values under them.
        let form = |label_size: f32| {
            let mut ps = vec![text(50.0, 440.0, 0, 700.0, 10.0, 60)];
            for x in [50.0f32, 200.0, 350.0] {
                ps.push(text(x, x + 90.0, 1, 680.0, label_size, 20));
                ps.push(text(x + 4.0, x + 60.0, 2, 660.0, 10.0, 9));
            }
            ps.push(text(50.0, 440.0, 3, 640.0, 10.0, 60));
            ps
        };
        let ps = form(6.0);
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        assert_eq!(
            cut.groups,
            vec![vec![0], vec![1, 2], vec![3, 4], vec![5, 6], vec![7]]
        );
        // A small header over rows of a table: the row under the first row is no small text.
        let mut table = form(6.0);
        let last = table.len() - 1;
        table[last] = text(54.0, 110.0, 3, 640.0, 10.0, 9);
        assert_eq!(
            cut_of(&table, &[]).groups[1..3],
            [vec![1, 3, 5], vec![2, 4, 6]]
        );
        let rows = cut_of(&form(10.0), &[]);
        assert_eq!(
            rows.groups,
            vec![vec![0], vec![1, 3, 5], vec![2, 4, 6], vec![7]]
        );
    }

    /// A piece of text as [`text`] with the two flags of its wording: it ends with a colon,
    /// it reads as an amount or a date.
    fn worded(x0: f32, x1: f32, line: u32, y: f32, size: f32, colon: bool, value: bool) -> Piece {
        Piece {
            colon,
            value,
            ..text(x0, x1, line, y, size, 9)
        }
    }

    /// Fix round 2 of Task 31a (N1): rows of a label and its value at one pitch, the last row
    /// (a total) set larger. The larger row is no row of values under a row of labels: its
    /// first piece ends with a colon, and the row over the two is a row like the upper one.
    /// The rows read row by row, each label with its own value.
    #[test]
    fn a_larger_total_row_under_rows_of_pairs_is_a_row() {
        // `Date: Jan`, `Order: po_123` at 10 points, `Balance: 127.50` at 11.5.
        let rows = |colons: bool, lead: bool| {
            let mut ps = Vec::new();
            if lead {
                ps.push(worded(50.0, 90.0, 0, 700.0, 10.0, colons, false));
                ps.push(worded(200.0, 240.0, 0, 700.0, 10.0, false, false));
            }
            ps.push(worded(50.0, 110.0, 1, 680.0, 10.0, colons, false));
            ps.push(worded(200.0, 250.0, 1, 680.0, 10.0, false, false));
            ps.push(worded(50.0, 120.0, 2, 660.0, 11.5, colons, false));
            ps.push(worded(200.0, 245.0, 2, 660.0, 11.5, false, true));
            ps
        };
        // With colons and a row over them: both refusals hold.
        let cut = cut_of(&rows(true, true), &[]);
        assert_eq!(cut.groups, vec![vec![0, 1], vec![2, 3], vec![4, 5]]);
        // The colon alone (no row over them), and the row over them alone (no colons).
        let cut = cut_of(&rows(true, false), &[]);
        assert_eq!(cut.groups, vec![vec![0, 1], vec![2, 3]]);
        let cut = cut_of(&rows(false, true), &[]);
        assert_eq!(cut.groups, vec![vec![0, 1], vec![2, 3], vec![4, 5]]);
    }

    /// Fix round 2 of Task 31a (N1): `Subtotal 120.00` over a larger `Total 127.50`, no colon
    /// and no row over them. The upper row holds an amount: it is a label with its value, no
    /// row of labels.
    #[test]
    fn a_row_with_an_amount_is_no_row_of_labels() {
        let ps = vec![
            worded(50.0, 110.0, 0, 680.0, 10.0, false, false),
            worded(200.0, 250.0, 0, 680.0, 10.0, false, true),
            worded(50.0, 100.0, 1, 660.0, 11.5, false, false),
            worded(200.0, 255.0, 1, 660.0, 11.5, false, true),
        ];
        assert_eq!(cut_of(&ps, &[]).groups, vec![vec![0, 1], vec![2, 3]]);
    }

    /// A small header over the one row of a table reads cell by cell: each title with the
    /// value under it, the same meaning as the two rows.
    #[test]
    fn a_small_header_over_one_row_reads_cell_by_cell() {
        let mut ps = Vec::new();
        for x in [50.0f32, 200.0, 350.0] {
            ps.push(worded(x, x + 60.0, 0, 680.0, 8.0, false, false));
        }
        for x in [50.0f32, 200.0, 350.0] {
            ps.push(worded(x, x + 50.0, 1, 660.0, 10.0, false, true));
        }
        assert_eq!(
            cut_of(&ps, &[]).groups,
            vec![vec![0, 3], vec![1, 4], vec![2, 5]]
        );
    }

    #[test]
    fn a_box_beside_text_makes_a_boxed_gutter() {
        let mut stand = text(40.0, 250.0, 0, 690.0, 8.0, 80);
        stand.bounds.y0 = 600.0;
        stand.bounds.y1 = 700.0;
        stand.boxed = Some(0);
        stand.rows = 3;
        let ps = vec![
            stand,
            text(300.0, 500.0, 0, 690.0, 8.0, 30),
            text(300.0, 500.0, 1, 670.0, 8.0, 30),
            text(300.0, 500.0, 2, 650.0, 8.0, 30),
        ];
        let cut = cut_of(&ps, &[]);
        assert_eq!(cut.groups, vec![vec![0], vec![1, 2, 3]]);
        assert_eq!(cut.splits, vec![Split::Boxed]);
    }

    /// A title beside a list of field lines at 6 points, the title 1.5 points under the
    /// baseline of the first line.
    fn title_beside(ys: &[f32]) -> Vec<Piece> {
        let mut title = text(40.0, 100.0, 0, ys[0] - 1.5, 9.0, 8);
        title.bold = true;
        let mut ps = vec![title];
        for (k, &y) in ys.iter().enumerate() {
            ps.push(text(360.0, 378.0, k as u32, y, 6.0, 6));
            ps.push(text(440.0, 458.0, k as u32, y, 6.0, 6));
        }
        ps
    }

    #[test]
    fn lone_side_needs_one_pitch() {
        // The field lines step 8, 15 and 20 points: no block of one pitch, so no gutter. (The
        // region is cut at the wide steps instead.)
        let ps = title_beside(&[703.0, 695.0, 680.0, 660.0]);
        let cut = cut_of(&ps, &[]);
        assert!(cut.sides.is_empty(), "{cut:?}");
        assert_whole(&cut, ps.len());
        // At one pitch, the lone title makes the gutter, and the field lines start a line.
        let ps = title_beside(&[703.0, 695.0, 687.0, 679.0]);
        let cut = cut_of(&ps, &[]);
        assert_eq!(cut.groups, vec![vec![0], (1..9).collect::<Vec<_>>()]);
        assert_eq!(cut.sides, vec![1]);
        assert_eq!(cut.splits, vec![Split::Gutter]);
    }

    #[test]
    fn the_depth_bound_keeps_every_piece() {
        let ps = blocks(&[30.0, 50.0, 30.0]);
        let whole = cut_of(&ps, &[]);
        assert_eq!(whole.groups.len(), 4);
        // Cut two levels deep, the regions are at the bound after one cut.
        for depth in 0..=5u32 {
            let limits = Limits {
                max_cut_depth: depth,
                ..Limits::default()
            };
            let cut = xy_cut(&ps, &[], &limits, &|| false);
            assert_whole(&cut, ps.len());
            if depth >= 2 {
                assert_eq!(cut, whole, "{depth}");
            } else {
                assert_eq!(cut.short, Some(CutShort::Depth), "{depth}");
                // The regions at the bound stay one group each, in their order.
                assert_eq!(cut.groups.len(), 1 << depth, "{depth}");
                assert_eq!(cut.groups.concat(), (0..ps.len()).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn a_stop_keeps_every_piece_in_one_group() {
        let ps = two_columns();
        let cut = xy_cut(&ps, &[], &Limits::default(), &|| true);
        assert_eq!(cut.short, Some(CutShort::Stopped));
        assert_eq!(cut.groups.len(), 1);
        assert_whole(&cut, ps.len());
    }

    #[test]
    fn no_pieces_make_no_groups() {
        assert_eq!(cut_of(&[], &[]), Cut::default());
    }
}
