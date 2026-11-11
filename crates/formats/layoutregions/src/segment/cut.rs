// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The whitespace cut: the pieces of a page in reading order. Columns come one after the
//! other, and the pieces that cross a gutter (titles, notes across the columns) split the
//! columns into bands. A region without a gutter is cut at its widest band of white space
//! across, and each part is looked at again.
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
        // Whether columns or bands are looked for in this region at all.
        let looks = ids.len() > 3 && depth < COLUMN_DEPTH && ids.len() <= COLUMN_PIECES;
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
            if let Some(at) = widest_gap(p, &ids) {
                if bound {
                    return self.keep(ids, out);
                }
                let rest = ids.split_off(at);
                self.region(ids, depth + 1, out);
                out.cut(Split::Band);
                self.region(rest, depth + 1, out);
                return;
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
        // Without a gutter, the region is cut at its widest band of white space across, and
        // each part is looked at again: columns may stand above or below full-width text.
        if looks && let Some(at) = widest_gap(p, &ids) {
            if bound {
                return self.keep(ids, out);
            }
            let rest = ids.split_off(at);
            self.region(ids, depth + 1, out);
            out.cut(Split::Band);
            self.region(rest, depth + 1, out);
            return;
        }
        out.group(ids);
    }
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

/// Where to cut pieces sorted by row: after the row with the widest white space below it,
/// when that space is at least half a line high. The index is the first piece below the cut.
fn widest_gap(p: &[Piece], ids: &[usize]) -> Option<usize> {
    // Each piece is passed once, and once more as part of the row after a cut.
    work::add(2 * ids.len() as u64);
    let mut best: Option<(f32, usize)> = None;
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
        if gap >= 0.5 * prev.size.max(cur.size) && best.is_none_or(|(g, _)| gap > g) {
            best = Some((gap, k));
        }
    }
    best.map(|(_, k)| k)
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
